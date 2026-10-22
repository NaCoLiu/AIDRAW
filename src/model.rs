use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const MODELS: &[&str] = &["gpt-image-2", "gpt-image-2-vip", "gpt-image-2.5"];
pub const QUALITIES: &[&str] = &["auto", "low", "medium", "high", "xhigh", "max"];
pub const SIZES: &[&str] = &[
    "auto",
    "1024x1024",
    "1536x1024",
    "1024x1536",
    "2048x2048",
    "2048x1152",
    "3840x2160",
    "2160x3840",
];
pub const FORMATS: &[&str] = &["png", "jpeg", "webp"];

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub base_url: String,
    pub model: String,
    pub quality: String,
    pub size: String,
    pub format: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: "https://rightapi.ai/v1".into(),
            model: MODELS[0].into(),
            quality: "auto".into(),
            size: "1024x1024".into(),
            format: "png".into(),
        }
    }
}

impl Settings {
    pub fn normalize_model(&mut self) {
        if !MODELS.contains(&self.model.as_str()) {
            self.model = if self.model.starts_with("gpt-image-2.5-") {
                "gpt-image-2.5"
            } else {
                MODELS[0]
            }
            .into();
        }
        if self.model != "gpt-image-2.5" && matches!(self.quality.as_str(), "xhigh" | "max") {
            self.quality = "high".into();
        }
    }

    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.base_url).context("API 地址格式不正确")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("请输入完整的 HTTP(S) API 地址，例如 https://rightapi.ai/v1");
        }
        if !MODELS.contains(&self.model.as_str()) {
            bail!("请选择支持的模型");
        }
        if !QUALITIES.contains(&self.quality.as_str()) {
            bail!("请选择支持的质量档位");
        }
        if self.model != "gpt-image-2.5" && matches!(self.quality.as_str(), "xhigh" | "max") {
            bail!("当前模型最高支持 high，请选择 gpt-image-2.5 或降低质量");
        }
        validate_size(&self.size)?;
        if !FORMATS.contains(&self.format.as_str()) {
            bail!("图片格式须为 png、jpeg 或 webp");
        }
        Ok(())
    }

    pub fn endpoint(&self, suffix: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    }
}

pub fn validate_size(value: &str) -> Result<()> {
    if value == "auto" {
        return Ok(());
    }
    let (w, h) = value
        .split_once('x')
        .context("尺寸格式应为 宽x高，如 1024x1024")?;
    let w: u64 = w.parse().context("图片宽度必须是正整数")?;
    let h: u64 = h.parse().context("图片高度必须是正整数")?;
    if w == 0
        || h == 0
        || w > 3840
        || h > 3840
        || w % 16 != 0
        || h % 16 != 0
        || w.max(h) > 3 * w.min(h)
        || !(655_360..=8_294_400).contains(&(w * h))
    {
        bail!("尺寸须为 16 的倍数、每边不超过 3840、比例不超过 3:1，总像素为 655360–8294400");
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Generate,
    Edit,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Generate => "文生图",
            Self::Edit => "图片编辑",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Submitting,
    Queued,
    Processing,
    Downloading,
    Completed,
    Failed,
    Interrupted,
}

impl Status {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Submitting | Self::Queued | Self::Processing | Self::Downloading
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Submitting => "正在提交",
            Self::Queued => "排队中",
            Self::Processing => "正在绘画",
            Self::Downloading => "正在保存图片",
            Self::Completed => "已完成",
            Self::Failed => "生成失败",
            Self::Interrupted => "已暂停",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ImageAsset {
    pub id: Uuid,
    pub path: PathBuf,
    pub url: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Drawing {
    pub id: Uuid,
    pub prompt: String,
    pub mode: Mode,
    pub settings: Settings,
    pub references: Vec<PathBuf>,
    pub created_at: u64,
    pub status: Status,
    pub progress: u8,
    pub task_id: Option<String>,
    pub images: Vec<ImageAsset>,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub title: String,
    pub named: bool,
    pub created_at: u64,
    #[serde(default)]
    pub draft: String,
    pub drawings: Vec<Drawing>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            title: "未命名会话".into(),
            named: false,
            created_at: now(),
            draft: String::new(),
            drawings: vec![],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Library {
    pub version: u32,
    pub settings: Settings,
    pub active_session: Uuid,
    pub sessions: Vec<Session>,
}

impl Default for Library {
    fn default() -> Self {
        let session = Session::default();
        Self {
            version: 1,
            settings: Settings::default(),
            active_session: session.id,
            sessions: vec![session],
        }
    }
}

impl Library {
    pub fn active(&self) -> &Session {
        self.sessions
            .iter()
            .find(|s| s.id == self.active_session)
            .unwrap_or(&self.sessions[0])
    }
    pub fn active_mut(&mut self) -> &mut Session {
        let index = self
            .sessions
            .iter()
            .position(|s| s.id == self.active_session)
            .unwrap_or(0);
        &mut self.sessions[index]
    }
    pub fn drawing_mut(&mut self, id: Uuid) -> Option<&mut Drawing> {
        self.sessions
            .iter_mut()
            .flat_map(|s| &mut s.drawings)
            .find(|d| d.id == id)
    }
    pub fn normalize(&mut self) {
        self.settings.normalize_model();
        if self.sessions.is_empty() {
            self.sessions.push(Session::default());
        }
        if !self.sessions.iter().any(|s| s.id == self.active_session) {
            self.active_session = self.sessions[0].id;
        }
        for drawing in self.sessions.iter_mut().flat_map(|s| &mut s.drawings) {
            if drawing.status.active() {
                drawing.status = Status::Interrupted;
                drawing.error = Some(if drawing.task_id.is_some() {
                    "应用已关闭。点击「继续查询」可找回任务结果，不会重复提交。".into()
                } else {
                    "上次提交未确认。请先到服务端检查任务，再决定是否重新生成。".into()
                });
            }
        }
    }
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        let root = match std::env::var_os("AIDRAW_DATA_DIR") {
            Some(value) => PathBuf::from(value),
            None => dirs::data_local_dir()
                .context("无法找到应用数据目录")?
                .join("AIDRAW"),
        };
        Self::at(root)
    }
    pub fn at(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("images")).context("无法创建图片目录")?;
        fs::create_dir_all(root.join("references")).context("无法创建参考图目录")?;
        Ok(Self { root })
    }
    pub fn load(&self) -> Result<Library> {
        let path = self.root.join("library.json");
        if !path.exists() {
            return Ok(Library::default());
        }
        let mut library: Library = serde_json::from_slice(&fs::read(&path)?)
            .context("会话文件无法读取，请检查 library.json；原文件未修改")?;
        if library.version != 1 {
            bail!("会话文件版本不受支持；原文件未修改");
        }
        library.normalize();
        Ok(library)
    }
    pub fn save(&self, library: &Library) -> Result<()> {
        atomic_write(
            &self.root.join("library.json"),
            &serde_json::to_vec_pretty(library)?,
        )
    }
    pub fn import_reference(&self, source: &Path) -> Result<PathBuf> {
        let extension = source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            bail!("参考图仅支持 PNG、JPEG 和 WebP");
        }
        if fs::metadata(source)?.len() > 50 * 1024 * 1024 {
            bail!("单张参考图请控制在 50 MB 内");
        }
        image::ImageReader::open(source)?
            .with_guessed_format()?
            .into_dimensions()
            .context("无法识别参考图片")?;
        let target = self
            .root
            .join("references")
            .join(format!("{}.{}", Uuid::new_v4(), extension));
        fs::copy(source, &target).context("无法导入参考图")?;
        Ok(target)
    }
    pub fn remove_generated(&self, image: &ImageAsset) -> Result<()> {
        // Only remove files owned by the cache, never imported user originals.
        if image.path.parent() == Some(self.root.join("images").as_path()) && image.path.exists() {
            fs::remove_file(&image.path)?;
        }
        Ok(())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("目标文件没有父目录")?)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|e| e.error)
        .context("无法保存文件，请检查目录权限")?;
    Ok(())
}

pub fn credential() -> Result<keyring::Entry> {
    keyring::Entry::new("AIDRAW.RightAPI", "api-key").context("无法访问系统凭据管理器")
}

pub fn load_api_key() -> String {
    std::env::var("RIGHTAPI_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| credential().ok()?.get_password().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_retired_model_choices_and_accepts_requested_models() {
        let mut settings = Settings {
            model: "gpt-image-2.5-flare".into(),
            quality: "max".into(),
            ..Settings::default()
        };
        settings.normalize_model();
        assert_eq!(settings.model, "gpt-image-2.5");
        settings.validate().unwrap();
        for model in MODELS {
            settings.model = (*model).into();
            settings.quality = "auto".into();
            settings.validate().unwrap();
        }
        settings.model = "gpt-image-2.5-sunburst".into();
        assert!(settings.validate().is_err());
        settings.normalize_model();
        assert_eq!(settings.model, "gpt-image-2.5");
    }

    #[test]
    fn validates_documented_size_and_quality_rules() {
        for size in SIZES {
            validate_size(size).unwrap();
        }
        for size in [
            "0x1024",
            "1025x1024",
            "3840x2560",
            "4096x1024",
            "3072x512",
            "256x256",
            "18446744073709551615x16",
        ] {
            assert!(validate_size(size).is_err(), "{size}");
        }
        let mut settings = Settings {
            model: "gpt-image-2".into(),
            quality: "max".into(),
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        settings.model = "gpt-image-2.5".into();
        settings.validate().unwrap();
        settings.base_url = "https://user:secret@example.test/v1".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn round_trip_preserves_sessions_and_recovers_pending_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().into()).unwrap();
        let mut library = Library::default();
        library.active_mut().title = "海报设计".into();
        library.active_mut().draft = "下一幅画".into();
        library.active_mut().drawings.push(Drawing {
            id: Uuid::new_v4(),
            prompt: "测试".into(),
            mode: Mode::Generate,
            settings: Settings::default(),
            references: vec![],
            created_at: now(),
            status: Status::Processing,
            progress: 45,
            task_id: Some("task_123".into()),
            images: vec![],
            error: None,
        });
        store.save(&library).unwrap();
        library.active_mut().title = "修改后的标题".into();
        store.save(&library).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.active().title, "修改后的标题");
        assert_eq!(loaded.active().draft, "下一幅画");
        assert_eq!(
            loaded.active().drawings[0].task_id.as_deref(),
            Some("task_123")
        );
        assert!(loaded.active().drawings[0].status == Status::Interrupted);
        assert!(
            !fs::read_to_string(directory.path().join("library.json"))
                .unwrap()
                .contains("api_key")
        );
    }

    #[test]
    fn deletion_only_removes_owned_generated_images() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().join("library")).unwrap();
        let original = directory.path().join("original.png");
        image::DynamicImage::new_rgb8(2, 2).save(&original).unwrap();
        let reference = store.import_reference(&original).unwrap();
        let generated = store.root.join("images/result.png");
        fs::copy(&original, &generated).unwrap();
        for path in [&original, &reference, &generated] {
            store
                .remove_generated(&ImageAsset {
                    id: Uuid::new_v4(),
                    path: path.clone(),
                    url: String::new(),
                })
                .unwrap();
        }
        assert!(original.exists());
        assert!(reference.exists());
        assert!(!generated.exists());
    }

    #[test]
    fn corrupt_library_is_never_silently_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().into()).unwrap();
        let path = directory.path().join("library.json");
        fs::write(&path, b"broken").unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(path).unwrap(), b"broken");
    }
}
