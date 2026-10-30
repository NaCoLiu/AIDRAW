use crate::model::{ImageAsset, Mode, Settings, Status, atomic_write};
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use std::{
    io::{Cursor, Read},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Job {
    pub drawing_id: Uuid,
    pub settings: Settings,
    pub prompt: String,
    pub mode: Mode,
    pub references: Vec<PathBuf>,
    pub resume_task: Option<String>,
    pub image_dir: PathBuf,
}

pub enum Event {
    Submitted(String),
    Progress(Status, u8),
    Completed(Vec<ImageAsset>),
    Failed(String),
}

pub struct Api {
    client: Client,
    poll_interval: Duration,
    timeout: Duration,
}

impl Api {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(200))
                .timeout(Duration::from_secs(900))
                .user_agent("AIDRAW/0.1")
                .build()?,
            poll_interval: Duration::from_secs(4),
            timeout: Duration::from_secs(600),
        })
    }

    pub fn run(
        &self,
        job: &Job,
        key: &str,
        canceled: &AtomicBool,
        mut emit: impl FnMut(Event) -> bool,
    ) -> Result<()> {
        job.settings.validate()?;
        if key.trim().is_empty() {
            bail!("请先在设置中填写 RightAPI API Key");
        }
        if canceled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut value = if let Some(task_id) = &job.resume_task {
            json!({"task_id": task_id, "status": "processing"})
        } else {
            self.submit(job, key)
                .context("提交失败；为避免重复扣费，客户端不会自动重新提交，请先检查服务端任务")?
        };

        let mut task_id = value
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(id) = &task_id {
            if id.is_empty() {
                bail!("接口返回了空的 task_id");
            }
            if !emit(Event::Submitted(id.clone())) {
                return Ok(());
            }
        }
        let started = Instant::now();
        let mut failures = 0;
        loop {
            if canceled.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Some(urls) = image_urls(&value)? {
                if !emit(Event::Progress(Status::Downloading, 100)) {
                    return Ok(());
                }
                let mut images: Vec<ImageAsset> = Vec::new();
                for url in urls {
                    if canceled.load(Ordering::Relaxed) {
                        for image in &images {
                            let _ = std::fs::remove_file(&image.path);
                        }
                        return Ok(());
                    }
                    match self.download(&url, job) {
                        Ok(image) => images.push(image),
                        Err(error) => {
                            for image in &images {
                                let _ = std::fs::remove_file(&image.path);
                            }
                            return Err(error);
                        }
                    }
                }
                if canceled.load(Ordering::Relaxed) {
                    for image in &images {
                        let _ = std::fs::remove_file(&image.path);
                    }
                    return Ok(());
                }
                emit(Event::Completed(images));
                return Ok(());
            }
            let (status, progress) = task_progress(&value)?;
            if !emit(Event::Progress(status, progress)) {
                return Ok(());
            }
            let id = task_id.as_ref().context("响应缺少 task_id 和图片结果")?;
            if started.elapsed() >= self.timeout {
                bail!("任务等待已超过 10 分钟。可稍后点击「继续查询」，无需重复生成。");
            }
            let mut url = reqwest::Url::parse(&job.settings.endpoint("tasks/"))?;
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("API 地址无效"))?
                .pop_if_empty()
                .push(id);
            loop {
                if !wait(self.poll_interval, canceled) {
                    return Ok(());
                }
                match self
                    .client
                    .get(url.clone())
                    .bearer_auth(key)
                    .timeout(Duration::from_secs(30))
                    .send()
                {
                    Ok(response)
                        if response.status().is_server_error()
                            || response.status().as_u16() == 429 =>
                    {
                        failures += 1;
                        if failures >= 5 {
                            return response_json(response)
                                .context("任务查询服务暂不可用，请稍后继续查询")
                                .map(|_| ());
                        }
                    }
                    Ok(response) => {
                        value = response_json(response).context("查询任务失败")?;
                        failures = 0;
                        break;
                    }
                    Err(_) => {
                        failures += 1;
                        if failures >= 5 {
                            bail!("查询任务时网络连接中断。请稍后点击「继续查询」。");
                        }
                    }
                }
                if started.elapsed() >= self.timeout {
                    bail!("任务查询超时，请稍后继续查询。");
                }
            }
            if let Some(id) = value.get("task_id").and_then(Value::as_str) {
                task_id = Some(id.into());
            }
        }
    }

    fn submit(&self, job: &Job, key: &str) -> Result<Value> {
        let settings = &job.settings;
        settings.validate_for(job.mode)?;
        let response = match job.mode {
            Mode::Generate => self.client.post(settings.endpoint("images/generations")).bearer_auth(key)
                .json(&json!({"model": settings.model, "prompt": job.prompt, "async": true,
                    "n": 1, "size": settings.size, "quality": settings.quality, "output_format": settings.format}))
                .send().context("无法连接 RightAPI")?,
            Mode::Edit | Mode::Batch => {
                if job.mode == Mode::Batch && job.references.len() != 1 {
                    bail!("批量任务必须且只能包含一张原图");
                }
                if job.references.is_empty() { bail!("图片编辑至少需要一张参考图"); }
                let mut images = Vec::with_capacity(job.references.len());
                for path in &job.references {
                    let mut bytes = Vec::new();
                    std::fs::File::open(path).context("参考图已丢失，请重新选择")?
                        .take(50 * 1024 * 1024 + 1).read_to_end(&mut bytes)
                        .context("无法读取参考图片")?;
                    if bytes.len() > 50 * 1024 * 1024 { bail!("单张参考图请控制在 50 MB 内"); }
                    let mime = match image::guess_format(&bytes).context("无法识别参考图片内容")? {
                        image::ImageFormat::Png => "image/png",
                        image::ImageFormat::Jpeg => "image/jpeg",
                        image::ImageFormat::WebP => "image/webp",
                        _ => bail!("参考图片仅支持 PNG、JPEG 或 WebP"),
                    };
                    images.push(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
                }
                // Use the documented JSON data URL form: bool/number fields retain their
                // types, and upload handling no longer depends on filenames or multipart parsing.
                self.client.post(settings.endpoint("images/edits")).bearer_auth(key)
                    .json(&json!({"model": settings.model, "prompt": job.prompt, "image": images,
                        "async": true, "n": 1, "size": settings.size, "quality": settings.quality,
                        "output_format": settings.format}))
                    .send().context("无法提交图片编辑请求")?
            }
        };
        response_json(response).context(match job.mode {
            Mode::Generate => "文生图接口提交失败",
            Mode::Edit | Mode::Batch => "图片编辑接口提交失败",
        })
    }

    fn download(&self, url: &str, job: &Job) -> Result<ImageAsset> {
        let parsed = reqwest::Url::parse(url).context("服务端返回了无效图片地址")?;
        if !matches!(parsed.scheme(), "https" | "http")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            bail!("图片地址必须是 HTTP(S) 地址");
        }
        // CDN requests intentionally carry no API key.
        let response = self
            .client
            .get(parsed)
            .send()
            .context("图片下载失败，可继续查询以重试保存")?;
        if !response.status().is_success() {
            bail!(
                "图片下载失败（HTTP {}），可继续查询重试",
                response.status().as_u16()
            );
        }
        let mut bytes = Vec::new();
        response
            .take(100 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 100 * 1024 * 1024 {
            bail!("返回图片超过 100 MB，已停止下载");
        }
        let format = image::guess_format(&bytes).context("下载内容不是受支持的图片")?;
        let extension = match format {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpeg",
            image::ImageFormat::WebP => "webp",
            _ => bail!("服务端返回的图片不是 PNG、JPEG 或 WebP"),
        };
        let (w, h) = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()?
            .into_dimensions()?;
        if w > 16384 || h > 16384 || u64::from(w) * u64::from(h) > 100_000_000 {
            bail!("返回图片的分辨率过大");
        }
        image::load_from_memory(&bytes).context("返回图片已损坏")?;
        let id = Uuid::new_v4();
        let path = job
            .image_dir
            .join(format!("{}-{}.{}", job.drawing_id, id, extension));
        atomic_write(&path, &bytes)?;
        Ok(ImageAsset {
            id,
            path,
            url: url.into(),
        })
    }
}

fn wait(duration: Duration, canceled: &AtomicBool) -> bool {
    let started = Instant::now();
    while started.elapsed() < duration {
        if canceled.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(
            Duration::from_millis(100).min(duration.saturating_sub(started.elapsed())),
        );
    }
    !canceled.load(Ordering::Relaxed)
}

fn response_json(mut response: Response) -> Result<Value> {
    let status = response.status();
    let request_id = ["x-request-id", "request-id", "x-correlation-id", "cf-ray"]
        .iter()
        .find_map(|name| response.headers().get(*name)?.to_str().ok())
        .map(|id| format!("；请求 ID：{}", error_excerpt(id, 128)))
        .unwrap_or_default();
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .context("无法读取 API 响应")?;
    let text = String::from_utf8_lossy(&bytes);
    let parsed: Option<Value> = serde_json::from_str(&text).ok();
    if !status.is_success() {
        let detail = parsed
            .as_ref()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.get("message"))
                    .or_else(|| v.get("error"))
            })
            .and_then(Value::as_str)
            .unwrap_or(&text);
        let detail = error_excerpt(detail, 600);
        let detail = if detail.is_empty() {
            "服务端未返回错误详情"
        } else {
            &detail
        };
        let hint = if status.as_u16() == 502 {
            "；网关或上游服务异常。若提交时未取得任务 ID，无法确认是否已受理，请先核对服务端任务"
        } else {
            ""
        };
        bail!("HTTP {}：{}{request_id}{hint}", status.as_u16(), detail);
    }
    parsed.with_context(|| {
        format!(
            "API 返回的内容不是有效 JSON{request_id}：{}",
            error_excerpt(&text, 600)
        )
    })
}

// Display bounded plain text from gateway pages, omitting tags and script/style contents.
fn error_excerpt(source: &str, limit: usize) -> String {
    let mut rest = source;
    let mut visible = String::new();
    let mut suppressed: Option<&str> = None;
    while let Some(start) = rest.find('<') {
        if suppressed.is_none() {
            visible.push_str(&rest[..start]);
            visible.push(' ');
        }
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let name = tag.split_whitespace().next().unwrap_or("");
        if matches!(name, "script" | "style") {
            suppressed = Some(if name == "script" { "script" } else { "style" });
        } else if suppressed.is_some_and(|name| tag == format!("/{name}")) {
            suppressed = None;
        }
        rest = &rest[start + end + 1..];
    }
    if suppressed.is_none() {
        visible.push_str(rest);
    }
    visible
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|ch| !ch.is_control())
        .take(limit)
        .collect()
}

fn image_urls(value: &Value) -> Result<Option<Vec<String>>> {
    let Some(data) = value.get("data") else {
        return Ok(None);
    };
    let items = data.as_array().context("图片结果 data 不是数组")?;
    if items.is_empty() {
        bail!("生成任务返回了空的图片列表");
    }
    let urls = items
        .iter()
        .map(|item| {
            item.get("url")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .context("图片结果缺少 url 字段")
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(urls))
}

fn task_progress(value: &Value) -> Result<(Status, u8)> {
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    if matches!(status, "failed" | "expired" | "canceled" | "cancelled")
        || value.get("error").is_some_and(|error| !error.is_null())
    {
        let error = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("任务失败、过期或已取消");
        bail!("{}", error);
    }
    let stage = match status {
        "queued" => Status::Queued,
        "processing" | "in_progress" => Status::Processing,
        _ => bail!("未知任务响应：未找到图片结果或有效任务状态"),
    };
    let progress = value
        .get("progress")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 100.0) as u8;
    Ok((stage, progress))
}

pub fn spawn(
    job: Job,
    key: String,
    canceled: Arc<AtomicBool>,
    sender: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = Api::new().and_then(|api| {
            api.run(&job, &key, &canceled, |event| {
                sender.send_blocking(event).is_ok()
            })
        });
        if let Err(error) = result {
            let mut message = format!("{error:#}");
            if !key.is_empty() {
                message = message.replace(&key, "[已隐藏 API Key]");
            }
            let _ = sender.send_blocking(Event::Failed(message));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    fn server(
        responses: Vec<(u16, String, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = vec![];
            for (code, kind, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")?
                            .trim()
                            .parse::<usize>()
                            .ok()
                    })
                    .unwrap_or(0);
                let mut request_body = vec![0; length];
                reader.read_exact(&mut request_body).unwrap();
                requests.push(format!(
                    "{}\r\n{}",
                    headers,
                    String::from_utf8_lossy(&request_body)
                ));
                write!(stream, "HTTP/1.1 {code} OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (base, handle)
    }

    fn json_response(body: Value) -> (u16, String, Vec<u8>) {
        (
            200,
            "application/json".into(),
            body.to_string().into_bytes(),
        )
    }

    fn source_image(format: image::ImageFormat, shade: u8) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([shade, 40, 80]),
        ))
        .write_to(&mut bytes, format)
        .unwrap();
        bytes.into_inner()
    }

    fn payload(request: &str) -> Value {
        assert!(
            request
                .to_lowercase()
                .contains("content-type: application/json")
        );
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
    fn job(base: &str, directory: &std::path::Path) -> Job {
        Job {
            drawing_id: Uuid::new_v4(),
            settings: Settings {
                base_url: format!("{base}/v1"),
                ..Settings::default()
            },
            prompt: "测试海报".into(),
            mode: Mode::Generate,
            references: vec![],
            resume_task: None,
            image_dir: directory.into(),
        }
    }

    #[test]
    fn generation_submits_async_and_polls_statusless_result() {
        assert_async_roundtrip(Mode::Generate);
    }

    #[test]
    fn edit_submits_async_and_downloads_result() {
        assert_async_roundtrip(Mode::Edit);
    }

    fn assert_async_roundtrip(mode: Mode) {
        // Bind first to construct the CDN URL using this test server's port.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let address = base.clone();
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let handle = std::thread::spawn(move || {
            let bodies = [
                json_response(json!({"task_id":"task_abc","status":"processing"})),
                json_response(json!({"status":"in_progress","progress":45})),
                json_response(json!({"data":[{"url":format!("{address}/result.png")}]})),
                (200, "image/png".into(), png.into_inner()),
            ];
            let mut requests = vec![];
            for (code, kind, body) in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let n = headers
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")?
                            .trim()
                            .parse::<usize>()
                            .ok()
                    })
                    .unwrap_or(0);
                let mut data = vec![0; n];
                reader.read_exact(&mut data).unwrap();
                requests.push(format!("{headers}\r\n{}", String::from_utf8_lossy(&data)));
                write!(stream,"HTTP/1.1 {code} OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        let directory = tempfile::tempdir().unwrap();
        let mut api = Api::new().unwrap();
        api.poll_interval = Duration::ZERO;
        let mut completed = false;
        let mut request = job(&base, directory.path());
        request.mode = mode;
        if mode == Mode::Edit {
            let source = directory.path().join("source.png");
            std::fs::write(&source, source_image(image::ImageFormat::Png, 1)).unwrap();
            request.references.push(source);
        }
        request.settings.size = "1280x768".into();
        api.run(&request, "test-key", &AtomicBool::new(false), |event| {
            if let Event::Completed(images) = event {
                assert_eq!(images.len(), 1);
                assert!(images[0].path.exists());
                completed = true;
            }
            true
        })
        .unwrap();
        assert!(completed);
        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with(if mode == Mode::Generate {
            "POST /v1/images/generations"
        } else {
            "POST /v1/images/edits"
        }));
        let payload: Value =
            serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["async"], true);
        assert_eq!(payload["n"], 1);
        assert_eq!(payload["size"], "1280x768");
        assert!(requests[1].starts_with("GET /v1/tasks/task_abc"));
        assert!(
            requests[0]
                .to_lowercase()
                .contains("authorization: bearer test-key")
        );
        assert!(!requests[3].to_lowercase().contains("authorization:"));
    }

    #[test]
    fn edits_upload_ordered_data_urls_with_typed_fields_and_propagate_failure() {
        let (base, handle) = server(vec![
            json_response(json!({"task_id":"task_edit","status":"processing"})),
            json_response(json!({"status":"failed","error":{"message":"上游生成失败"}})),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let sources = [
            ("原图.png", image::ImageFormat::Jpeg, "image/jpeg"),
            ("two.jpeg", image::ImageFormat::WebP, "image/webp"),
            ("three.webp", image::ImageFormat::Png, "image/png"),
        ];
        let paths: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(index, (name, format, _))| {
                let path = directory.path().join(name);
                std::fs::write(&path, source_image(*format, index as u8)).unwrap();
                path
            })
            .collect();
        let mut request = job(&base, directory.path());
        request.mode = Mode::Edit;
        request.references = paths.clone();
        request.settings.model = "gpt-image-2.5-sunburst".into();
        request.settings.quality = "max".into();
        let mut api = Api::new().unwrap();
        api.poll_interval = Duration::ZERO;
        let error = api
            .run(&request, "test-key", &AtomicBool::new(false), |_| true)
            .unwrap_err();
        assert!(error.to_string().contains("上游生成失败"));
        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("POST /v1/images/edits"));
        let body = payload(&requests[0]);
        assert_eq!(body["async"], true);
        assert_eq!(body["n"], 1);
        assert_eq!(body["model"], "gpt-image-2.5-sunburst");
        assert_eq!(body["quality"], "max");
        assert_eq!(body["output_format"], "png");
        assert!(body.get("background").is_none());
        assert_eq!(body["image"].as_array().unwrap().len(), 3);
        for (index, (_, _, mime)) in sources.iter().enumerate() {
            let encoded = body["image"][index]
                .as_str()
                .unwrap()
                .strip_prefix(&format!("data:{mime};base64,"))
                .unwrap();
            assert_eq!(
                STANDARD.decode(encoded).unwrap(),
                std::fs::read(&paths[index]).unwrap()
            );
        }
    }

    #[test]
    fn batch_sends_42_separate_edits_with_custom_instruction_and_survives_one_error() {
        let responses = (0..42)
            .map(|index| {
                if index == 5 {
                    (
                        400,
                        "application/json".into(),
                        br#"{"error":{"message":"invalid source"}}"#.to_vec(),
                    )
                } else {
                    json_response(json!({"task_id": format!("batch-{index}"), "status": "queued"}))
                }
            })
            .collect();
        let (base, handle) = server(responses);
        let directory = tempfile::tempdir().unwrap();
        let mut library = crate::model::Library::default();
        library.settings.base_url = format!("{base}/v1");
        let sources: Vec<_> = (0..42)
            .map(|index| {
                let path = directory.path().join(format!("product-{index}.png"));
                std::fs::write(&path, source_image(image::ImageFormat::Png, index)).unwrap();
                path
            })
            .collect();
        library
            .enqueue("将背景替换为蓝色渐变，保留主体", Mode::Batch, &sources)
            .unwrap();
        let api = Api::new().unwrap();
        let mut accepted = 0;
        for drawing in &library.active().drawings {
            let request = Job {
                drawing_id: drawing.id,
                settings: drawing.settings.clone(),
                prompt: drawing.prompt.clone(),
                mode: drawing.mode,
                references: drawing.references.clone(),
                resume_task: None,
                image_dir: directory.path().into(),
            };
            accepted += usize::from(api.submit(&request, "test-key").is_ok());
        }
        assert_eq!(accepted, 41);
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 42);
        for (index, request) in requests.iter().enumerate() {
            assert!(request.starts_with("POST /v1/images/edits"));
            let body = payload(request);
            assert_eq!(body["image"].as_array().unwrap().len(), 1);
            assert_eq!(body["prompt"], "将背景替换为蓝色渐变，保留主体");
            let encoded = body["image"][0]
                .as_str()
                .unwrap()
                .strip_prefix("data:image/png;base64,")
                .unwrap();
            assert_eq!(
                STANDARD.decode(encoded).unwrap(),
                std::fs::read(&sources[index]).unwrap()
            );
        }
    }

    #[test]
    fn batch_rejects_combined_sources_before_sending() {
        let directory = tempfile::tempdir().unwrap();
        let mut request = job("http://127.0.0.1:1", directory.path());
        request.mode = Mode::Batch;
        request.references = vec!["one.png".into(), "two.png".into()];
        let error = Api::new()
            .unwrap()
            .submit(&request, "test-key")
            .unwrap_err();
        assert!(error.to_string().contains("只能包含一张原图"));
    }

    #[test]
    fn resume_never_submits_a_new_generation() {
        let (base, handle) = server(vec![json_response(json!({"status":"expired"}))]);
        let directory = tempfile::tempdir().unwrap();
        let mut request = job(&base, directory.path());
        request.mode = Mode::Edit;
        request.settings.model = "gpt-image-2-vip".into();
        request.resume_task = Some("previous_task".into());
        let mut api = Api::new().unwrap();
        api.poll_interval = Duration::ZERO;
        assert!(
            api.run(&request, "test-key", &AtomicBool::new(false), |_| true)
                .is_err()
        );
        assert!(handle.join().unwrap()[0].starts_with("GET /v1/tasks/previous_task"));
    }

    #[test]
    fn edits_reject_missing_invalid_and_unsupported_sources_before_network() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        let mut request = job("http://127.0.0.1:1", directory.path());
        request.mode = Mode::Edit;
        let api = Api::new().unwrap();
        assert!(
            api.submit(&request, "test-key")
                .unwrap_err()
                .to_string()
                .contains("至少需要")
        );
        request.references = vec![source.clone()];
        assert!(
            api.submit(&request, "test-key")
                .unwrap_err()
                .to_string()
                .contains("已丢失")
        );
        std::fs::write(&source, b"not an image").unwrap();
        assert!(
            api.submit(&request, "test-key")
                .unwrap_err()
                .to_string()
                .contains("无法识别")
        );
        std::fs::write(&source, b"GIF89a").unwrap();
        assert!(
            api.submit(&request, "test-key")
                .unwrap_err()
                .to_string()
                .contains("仅支持")
        );
    }

    #[test]
    fn gateway_failure_preserves_safe_details_and_never_reposts() {
        for body in [
            "<html><head><style>hidden-style</style></head><body><h1>502 Bad Gateway</h1><p>upstream timed out</p><script>hidden-script</script></body></html>",
            "upstream timed out",
        ] {
            let (base, handle) = server(vec![(
                502,
                "text/html\r\nx-request-id: req-edit-502".into(),
                body.as_bytes().to_vec(),
            )]);
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("source.png");
            std::fs::write(&source, source_image(image::ImageFormat::Png, 0)).unwrap();
            let mut request = job(&base, directory.path());
            request.mode = Mode::Edit;
            request.references.push(source);
            let error = Api::new()
                .unwrap()
                .run(&request, "test-key", &AtomicBool::new(false), |_| true)
                .unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("图片编辑接口提交失败"));
            assert!(message.contains("HTTP 502"));
            assert!(message.contains("upstream timed out"));
            assert!(message.contains("req-edit-502"));
            assert!(message.contains("不会自动重新提交"));
            assert!(!message.contains("hidden-script"));
            assert!(!message.contains("hidden-style"));
            assert!(!message.contains("<html>"));
            let requests = handle.join().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].starts_with("POST /v1/images/edits"));
        }
        assert_eq!(error_excerpt(&"错".repeat(1000), 600).chars().count(), 600);
    }

    #[test]
    fn rejects_malformed_success_and_handles_auth_error() {
        assert!(image_urls(&json!({"data":[]})).is_err());
        assert!(image_urls(&json!({"data":[{}]})).is_err());
        let (base, handle) = server(vec![(
            401,
            "application/json".into(),
            br#"{"error":{"message":"invalid key"}}"#.to_vec(),
        )]);
        let directory = tempfile::tempdir().unwrap();
        let error = Api::new()
            .unwrap()
            .run(
                &job(&base, directory.path()),
                "test-key",
                &AtomicBool::new(false),
                |_| true,
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("401"));
        assert_eq!(handle.join().unwrap().len(), 1);
    }
}
