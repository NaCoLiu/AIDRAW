use crate::{
    api::{self, Event, Job},
    model::{
        self, Drawing, FORMATS, ImageAsset, Library, MODELS, Mode, QUALITIES, SIZES, Session,
        Status, Store,
    },
};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon, IndexPath, Selectable, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        progress::Progress,
        select::{Select, SelectEvent, SelectState},
        spinner::Spinner,
    },
    prelude::*,
    *,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

gpui_kit::actions!(aidraw, [Generate, NewSession, OpenSettings]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", Generate, Some("AIDRAW")),
        KeyBinding::new("ctrl-n", NewSession, Some("AIDRAW")),
        KeyBinding::new("ctrl-,", OpenSettings, Some("AIDRAW")),
    ]);
}

type Choice = Entity<SelectState<Vec<SharedString>>>;
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteTarget {
    Session(Uuid),
    Drawing(Uuid),
}

pub struct Studio {
    library: Library,
    store: Option<Store>,
    storage_error: Option<String>,
    notice: Option<(bool, String)>,
    prompt: Entity<TextareaState>,
    title: Entity<InputState>,
    base_url: Entity<InputState>,
    key_input: Entity<InputState>,
    api_key: String,
    model: Choice,
    quality: Choice,
    size: Choice,
    custom_size: bool,
    custom_width: Entity<InputState>,
    custom_height: Entity<InputState>,
    format: Choice,
    mode: Mode,
    references: Vec<PathBuf>,
    settings_open: bool,
    renaming: bool,
    deleting: Option<DeleteTarget>,
    importing: bool,
    running: HashMap<Uuid, Arc<AtomicBool>>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    save_task: Option<Task<()>>,
}

fn choice(values: &[&str], value: &str, window: &mut Window, cx: &mut App) -> Choice {
    let index = values.iter().position(|s| *s == value).unwrap_or(0);
    cx.new(|cx| {
        SelectState::new(
            values
                .iter()
                .map(|s| SharedString::from(s.to_string()))
                .collect(),
            Some(IndexPath::new(index)),
            window,
            cx,
        )
    })
}
fn hint(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(rgb(0x929ba9)).child(text.into())
}

impl Studio {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (store, library, storage_error) = match Store::open() {
            Ok(store) => match store.load() {
                Ok(library) => (Some(store), library, None),
                Err(error) => (Some(store), Library::default(), Some(format!("{error:#}"))),
            },
            Err(error) => (None, Library::default(), Some(format!("{error:#}"))),
        };
        let settings = library.settings.clone();
        let api_key = model::load_api_key();
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("描述你想创作的画面：主体、场景、光线、构图和风格……")
                .default_value(library.active().draft.clone())
                .rows(3)
        });
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("会话标题")
                .default_value(library.active().title.clone())
        });
        let base_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://rightapi.ai/v1")
                .default_value(settings.base_url.clone())
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("sk-…")
                .masked(true)
                .default_value(api_key.clone())
        });
        let model = choice(MODELS, &settings.model, window, cx);
        let quality = choice(QUALITIES, &settings.quality, window, cx);
        let custom_size = !SIZES.contains(&settings.size.as_str());
        let mut size_choices = SIZES.to_vec();
        size_choices.push("自定义");
        let size = choice(
            &size_choices,
            if custom_size {
                "自定义"
            } else {
                &settings.size
            },
            window,
            cx,
        );
        let (width, height) = settings.size.split_once('x').unwrap_or(("1024", "1024"));
        let custom_width = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("宽度")
                .default_value(width)
        });
        let custom_height = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("高度")
                .default_value(height)
        });
        let format = choice(FORMATS, &settings.format, window, cx);
        let mut subscriptions = vec![cx.subscribe_in(&prompt, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.library.active_mut().draft = this.prompt.read(cx).value().to_string();
                this.schedule_save(cx);
            }
        })];
        subscriptions.push(
            cx.subscribe_in(&model, window, |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.library.settings.model = value.to_string();
                    let old = this
                        .quality
                        .read(cx)
                        .selected_value()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    if value.as_ref() != "gpt-image-2.5" && matches!(old.as_str(), "xhigh" | "max")
                    {
                        this.quality.update(cx, |s, cx| {
                            s.set_selected_value(&SharedString::from("high"), window, cx)
                        });
                        this.library.settings.quality = "high".into();
                    }
                    this.persist();
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(&quality, window, |this, _, event, _, cx| {
            if let SelectEvent::Confirm(Some(v)) = event {
                this.library.settings.quality = v.to_string();
                this.persist();
                cx.notify();
            }
        }));
        subscriptions.push(cx.subscribe_in(&size, window, |this, _, event, _, cx| {
            if let SelectEvent::Confirm(Some(v)) = event {
                this.custom_size = v.as_ref() == "自定义";
                if this.custom_size {
                    this.sync_custom_size(cx);
                } else {
                    this.library.settings.size = v.to_string();
                }
                this.persist();
                cx.notify();
            }
        }));
        for input in [&custom_width, &custom_height] {
            subscriptions.push(cx.subscribe_in(input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) && this.custom_size {
                    this.sync_custom_size(cx);
                    this.schedule_save(cx);
                    cx.notify();
                }
            }));
        }
        subscriptions.push(cx.subscribe_in(&format, window, |this, _, event, _, cx| {
            if let SelectEvent::Confirm(Some(v)) = event {
                this.library.settings.format = v.to_string();
                this.persist();
                cx.notify();
            }
        }));
        Self {
            library,
            store,
            storage_error,
            notice: None,
            prompt,
            title,
            base_url,
            key_input,
            api_key,
            model,
            quality,
            size,
            custom_size,
            custom_width,
            custom_height,
            format,
            mode: Mode::Generate,
            references: vec![],
            settings_open: false,
            renaming: false,
            deleting: None,
            importing: false,
            running: HashMap::new(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
            save_task: None,
        }
    }

    fn sync_custom_size(&mut self, cx: &App) {
        self.library.settings.size = format!(
            "{}x{}",
            self.custom_width.read(cx).value().trim(),
            self.custom_height.read(cx).value().trim()
        );
    }

    fn persist(&mut self) -> bool {
        if self.storage_error.is_some() {
            return false;
        }
        if let Some(store) = &self.store {
            if let Err(error) = store.save(&self.library) {
                self.notice = Some((true, format!("会话保存失败：{error:#}")));
                return false;
            }
            return true;
        }
        false
    }
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |this, _| {
                this.persist();
            });
        }));
    }
    fn message(&mut self, error: bool, text: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some((error, text.into()));
        cx.notify();
    }
    fn select_session(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.library.active_session {
            return;
        }
        self.library.active_mut().draft = self.prompt.read(cx).value().to_string();
        self.library.active_session = id;
        let draft = self.library.active().draft.clone();
        let title = self.library.active().title.clone();
        self.prompt
            .update(cx, |s, cx| s.set_value(draft, window, cx));
        self.title
            .update(cx, |s, cx| s.set_value(title, window, cx));
        self.renaming = false;
        self.settings_open = false;
        self.deleting = None;
        self.references.clear();
        self.mode = Mode::Generate;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.persist();
        cx.notify();
    }
    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.library.active_mut().draft = self.prompt.read(cx).value().to_string();
        let session = Session::default();
        let id = session.id;
        self.library.sessions.insert(0, session);
        self.select_session(id, window, cx);
        self.prompt.update(cx, |s, cx| s.focus(window, cx));
    }
    fn save_title(&mut self, cx: &mut Context<Self>) {
        let value = self
            .title
            .read(cx)
            .value()
            .trim()
            .chars()
            .take(80)
            .collect::<String>();
        if value.is_empty() {
            self.message(true, "请填写会话标题", cx);
            return;
        }
        let session = self.library.active_mut();
        session.title = value;
        session.named = true;
        self.renaming = false;
        self.persist();
        cx.notify();
    }
    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.library.settings.clone();
        settings.base_url = self
            .base_url
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_string();
        if let Err(error) = settings.validate() {
            self.message(true, error.to_string(), cx);
            return;
        }
        let key = self.key_input.read(cx).value().trim().to_string();
        let result = model::credential().and_then(|entry| {
            if key.is_empty() {
                match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(e.into()),
                }
            } else {
                entry.set_password(&key).map_err(Into::into)
            }
        });
        if let Err(error) = result {
            self.message(
                true,
                format!("密钥未能写入系统凭据：{error:#}。可设置 RIGHTAPI_API_KEY 环境变量。"),
                cx,
            );
            return;
        }
        self.library.settings = settings;
        self.api_key = key;
        self.persist();
        self.settings_open = false;
        self.message(false, "连接设置已保存", cx);
    }
    fn add_references(&mut self, cx: &mut Context<Self>) {
        if self.importing {
            return;
        }
        let Some(store) = self.store.clone() else {
            self.message(true, "本地数据目录不可用", cx);
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("选择参考图片（PNG / JPEG / WebP）".into()),
        });
        let session_id = self.library.active_session;
        self.importing = true;
        cx.notify();
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        paths
                            .iter()
                            .map(|path| store.import_reference(path))
                            .collect::<anyhow::Result<Vec<_>>>()
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.importing = false;
                    match result {
                        Ok(paths) => {
                            if this.library.active_session != session_id {
                                this.message(false, "会话已切换，请在目标会话重新添加参考图", cx);
                                return;
                            }
                            this.references.extend(paths);
                            this.mode = Mode::Edit;
                            cx.notify();
                        }
                        Err(e) => this.message(true, format!("导入失败：{e:#}"), cx),
                    }
                });
            }
            result => {
                let _ = this.update(cx, |this, cx| {
                    this.importing = false;
                    match result {
                        Ok(Err(e)) => this.message(true, format!("无法打开文件选择器：{e:#}"), cx),
                        Err(e) => this.message(true, format!("文件选择器已关闭：{e}"), cx),
                        _ => cx.notify(),
                    }
                });
            }
        })
        .detach();
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.storage_error.is_some() {
            self.message(true, "请先修复本地数据目录，原会话文件已保留。", cx);
            return;
        }
        if self.session_busy() {
            return;
        }
        let prompt = self.prompt.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            self.message(true, "先写下你的创作提示词", cx);
            self.prompt.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        if self.api_key.trim().is_empty() {
            self.settings_open = true;
            self.message(true, "请填写 RightAPI API Key 并保存设置", cx);
            return;
        }
        if self.mode == Mode::Edit && self.references.is_empty() {
            self.message(true, "请先添加参考图，或点击已有图片的「编辑此图」", cx);
            return;
        }
        if let Err(e) = self.library.settings.validate() {
            self.message(true, e.to_string(), cx);
            return;
        }
        let references = if self.mode == Mode::Edit {
            self.references.clone()
        } else {
            vec![]
        };
        let drawing = Drawing {
            id: Uuid::new_v4(),
            prompt: prompt.clone(),
            mode: self.mode,
            settings: self.library.settings.clone(),
            references,
            created_at: model::now(),
            status: Status::Submitting,
            progress: 0,
            task_id: None,
            images: vec![],
            error: None,
        };
        let session = self.library.active_mut();
        if !session.named && session.drawings.is_empty() {
            session.title = prompt.chars().take(22).collect();
        }
        let job = self.job(&drawing);
        self.library.active_mut().drawings.push(drawing);
        self.notice = None;
        let saved = self.persist();
        if !saved {
            if let Some(drawing) = self.library.active_mut().drawings.last_mut() {
                drawing.status = Status::Failed;
                drawing.error = Some("本地记录保存失败，尚未提交到服务端".into());
            }
        } else if let Some(job) = job {
            self.start(job, cx);
        }
        cx.notify();
    }
    fn job(&self, drawing: &Drawing) -> Option<Job> {
        let mut settings = drawing.settings.clone();
        // Polling old tasks does not send a model; keep historical records intact.
        if drawing.task_id.is_some() {
            settings.normalize_model();
        }
        Some(Job {
            drawing_id: drawing.id,
            settings,
            prompt: drawing.prompt.clone(),
            mode: drawing.mode,
            references: drawing.references.clone(),
            resume_task: drawing.task_id.clone(),
            image_dir: self.store.as_ref()?.root.join("images"),
        })
    }
    fn start(&mut self, job: Job, cx: &mut Context<Self>) {
        let id = job.drawing_id;
        if self.running.contains_key(&id) {
            return;
        }
        let canceled = Arc::new(AtomicBool::new(false));
        self.running.insert(id, canceled.clone());
        let (sender, receiver) = async_channel::unbounded();
        api::spawn(job, self.api_key.clone(), canceled, sender);
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let canceled = this
                            .running
                            .get(&id)
                            .is_some_and(|cancel| cancel.load(Ordering::Relaxed));
                        if let Some(drawing) = this.library.drawing_mut(id) {
                            match event {
                                Event::Submitted(task_id) => drawing.task_id = Some(task_id),
                                Event::Progress(status, progress) if !canceled => {
                                    drawing.status = status;
                                    drawing.progress = progress;
                                }
                                Event::Completed(images) => {
                                    drawing.images = images;
                                    drawing.status = Status::Completed;
                                    drawing.progress = 100;
                                    drawing.error = None;
                                }
                                Event::Failed(error) if !canceled => {
                                    drawing.status = Status::Failed;
                                    drawing.error = Some(error);
                                }
                                _ => {}
                            }
                            this.persist();
                            cx.notify();
                        } else if let Event::Completed(images) = event {
                            if let Some(store) = &this.store {
                                for image in &images {
                                    let _ = store.remove_generated(image);
                                }
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.running.remove(&id);
                cx.notify();
            });
        })
        .detach();
    }
    fn resume(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.storage_error.is_some() {
            return;
        }
        if self.api_key.is_empty() {
            self.settings_open = true;
            self.message(true, "请先保存 API Key", cx);
            return;
        }
        let drawing = self
            .library
            .sessions
            .iter()
            .flat_map(|s| &s.drawings)
            .find(|d| d.id == id)
            .cloned();
        if let Some(drawing) = drawing {
            if drawing.task_id.is_none() || self.running.contains_key(&id) {
                return;
            }
            if let Some(job) = self.job(&drawing) {
                if let Some(d) = self.library.drawing_mut(id) {
                    d.status = Status::Processing;
                    d.error = None;
                }
                self.persist();
                self.start(job, cx);
                cx.notify();
            }
        }
    }
    fn pause(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(cancel) = self.running.get(&id) {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(d) = self.library.drawing_mut(id) {
            d.status = Status::Interrupted;
            d.error = Some("已暂停本地查询，服务端任务仍可能运行。可继续查询以获取结果。".into());
        }
        self.persist();
        cx.notify();
    }
    fn reuse(&mut self, drawing: Drawing, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt.update(cx, |s, cx| {
            s.set_value(drawing.prompt, window, cx);
            s.focus(window, cx);
        });
        self.mode = drawing.mode;
        self.references = drawing.references;
        cx.notify();
    }
    fn edit_image(&mut self, image: ImageAsset, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        match store.import_reference(&image.path) {
            Ok(path) => {
                self.references = vec![path];
                self.mode = Mode::Edit;
                self.prompt.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                    s.focus(window, cx);
                });
                self.message(false, "已添加原图，请描述需要保留的内容和想修改的部分", cx);
            }
            Err(e) => self.message(true, format!("无法使用这张图片：{e:#}"), cx),
        }
    }
    fn export_image(&mut self, image: ImageAsset, cx: &mut Context<Self>) {
        let directory = dirs::download_dir().unwrap_or_else(|| {
            self.store
                .as_ref()
                .map(|s| s.root.clone())
                .unwrap_or_default()
        });
        let filename = format!(
            "AIDRAW-{}.{}",
            image.id,
            image
                .path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("png")
        );
        let picker = cx.prompt_for_new_path(&directory, Some(&filename));
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(path))) => {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::read(&image.path)
                            .map_err(anyhow::Error::from)
                            .and_then(|bytes| model::atomic_write(&path, &bytes))
                    })
                    .await;
                let _ = this.update(cx, |this, cx| match result {
                    Ok(()) => this.message(false, "图片已保存", cx),
                    Err(e) => this.message(true, format!("保存失败：{e:#}"), cx),
                });
            }
            Ok(Err(e)) => {
                let _ = this.update(cx, |this, cx| {
                    this.message(true, format!("无法打开保存窗口：{e:#}"), cx)
                });
            }
            _ => {}
        })
        .detach();
    }
    fn delete_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.deleting.take() else {
            return;
        };
        let before = self.library.clone();
        let drawings = match target {
            DeleteTarget::Drawing(id) => {
                let session = self.library.active_mut();
                let index = session.drawings.iter().position(|d| d.id == id);
                index
                    .map(|i| vec![session.drawings.remove(i)])
                    .unwrap_or_default()
            }
            DeleteTarget::Session(id) => {
                let index = self
                    .library
                    .sessions
                    .iter()
                    .position(|s| s.id == id)
                    .unwrap();
                let session = self.library.sessions.remove(index);
                if self.library.sessions.is_empty() {
                    self.library.sessions.push(Session::default());
                }
                self.library.active_session = self.library.sessions[0].id;
                session.drawings
            }
        };
        if !self.persist() {
            self.library = before;
            cx.notify();
            return;
        }
        for drawing in &drawings {
            if let Some(cancel) = self.running.get(&drawing.id) {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        if matches!(target, DeleteTarget::Session(_)) {
            self.prompt.update(cx, |s, cx| {
                s.set_value(self.library.active().draft.clone(), window, cx)
            });
            self.title.update(cx, |s, cx| {
                s.set_value(self.library.active().title.clone(), window, cx)
            });
            self.references.clear();
            self.mode = Mode::Generate;
            self.renaming = false;
        }
        if let Some(store) = self.store.clone() {
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        for drawing in drawings {
                            for image in drawing.images {
                                store.remove_generated(&image)?;
                            }
                        }
                        anyhow::Ok(())
                    })
                    .await;
                if let Err(e) = result {
                    let _ = this.update(cx, |this, cx| {
                        this.message(true, format!("记录已删除，但部分缓存无法清理：{e:#}"), cx)
                    });
                }
            })
            .detach();
        }
        cx.notify();
    }
    fn session_busy(&self) -> bool {
        self.library
            .active()
            .drawings
            .iter()
            .any(|d| self.running.contains_key(&d.id))
    }
}

impl Studio {
    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(248.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(0x14171d))
            .border_r_1()
            .border_color(rgb(0x2a303a))
            .p_4()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .child(
                        div()
                            .flex()
                            .size_10()
                            .rounded_lg()
                            .bg(rgb(0x23463f))
                            .items_center()
                            .justify_center()
                            .text_color(rgb(0x9de4cb))
                            .child(img(crate::branding::app_icon()).size_10()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_lg()
                                    .child("AIDRAW"),
                            )
                            .child(hint("三只猫专用图像工作室")),
                    ),
            )
            .child(
                Button::new("new-session")
                    .primary()
                    .icon(IconName::Plus)
                    .label("新建会话")
                    .w_full()
                    .on_click(cx.listener(|this, _, w, cx| this.new_session(w, cx))),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(hint("创作会话"))
                    .child(hint(format!("{}", self.library.sessions.len()))),
            )
            .child(
                div()
                    .id("sessions")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .overflow_y_scroll()
                    .children(self.library.sessions.iter().map(|session| {
                        let id = session.id;
                        let selected = id == self.library.active_session;
                        let busy = session
                            .drawings
                            .iter()
                            .any(|d| self.running.contains_key(&d.id));
                        div()
                            .id(SharedString::from(id.to_string()))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_3()
                            .rounded_lg()
                            .cursor_pointer()
                            .bg(if selected {
                                rgb(0x293b37)
                            } else {
                                rgb(0x14171d)
                            })
                            .hover(|s| s.bg(rgb(0x25302f)))
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.select_session(id, w, cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::MessageSquare).small())
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .truncate()
                                            .child(session.title.clone()),
                                    )
                                    .when(busy, |el| el.child(Spinner::new().small())),
                            )
                            .child(hint(format!("{} 幅绘画", session.drawings.len())))
                    })),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(0x2a303a))
                    .pt_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        Button::new("settings")
                            .ghost()
                            .icon(IconName::Settings)
                            .label("连接设置")
                            .w_full()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = !this.settings_open;
                                this.notice = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size_2()
                                    .rounded_full()
                                    .bg(if self.api_key.is_empty() {
                                        rgb(0xf0b86e)
                                    } else {
                                        rgb(0x7dd3b0)
                                    }),
                            )
                            .child(hint(if self.api_key.is_empty() {
                                "尚未配置 API Key"
                            } else {
                                "RightAPI · 已配置密钥"
                            })),
                    )
                    .child(hint("Ctrl+N 新会话 · Ctrl+Enter 生成")),
            )
    }
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.library.active();
        let id = active.id;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .h(px(84.))
            .px_7()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(0x2a303a))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .child(hint("WORKSPACE / 创作空间"))
                    .when(!self.renaming, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(active.title.clone()),
                                )
                                .child(
                                    Button::new("rename")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Pencil)
                                        .tooltip("修改会话标题")
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            let title = this.library.active().title.clone();
                                            this.title.update(cx, |s, cx| {
                                                s.set_value(title, w, cx);
                                                s.focus(w, cx);
                                            });
                                            this.renaming = true;
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .when(self.renaming, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(Input::new(&self.title).w(px(320.)))
                                .child(
                                    Button::new("save-title")
                                        .primary()
                                        .small()
                                        .label("保存")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.save_title(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("cancel-title")
                                        .ghost()
                                        .small()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.renaming = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                    }),
            )
            .child(hint(format!("{} 幅绘画", active.drawings.len())))
            .child(
                Button::new("delete-session")
                    .ghost()
                    .small()
                    .icon(IconName::Trash)
                    .tooltip("删除当前会话")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.deleting = Some(DeleteTarget::Session(id));
                        cx.notify();
                    })),
            )
    }
    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("settings-page").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().p_8().gap_6()
            .child(div().text_2xl().font_weight(FontWeight::SEMIBOLD).child("连接你的创作引擎"))
            .child(hint("接入 RightAPI，使用 GPT Image 系列生成和编辑图片。"))
            .child(div().flex().flex_col().max_w(px(700.)).p_6().gap_5().rounded_xl().bg(rgb(0x20242c)).border_1().border_color(rgb(0x343b46))
                .child(div().flex().flex_col().gap_2().child("API 地址").child(Input::new(&self.base_url)).child(hint("默认 https://rightapi.ai/v1，地址需包含 /v1。")))
                .child(div().flex().flex_col().gap_2().child("API Key").child(Input::new(&self.key_input)).child(hint("密钥保存在系统凭据库（Windows 凭据管理器 / macOS 钥匙串），不写入会话文件。留空保存可清除密钥。")))
                .child(div().flex().gap_3()
                    .child(Button::new("save-settings").primary().icon(IconName::Check).label("保存设置").on_click(cx.listener(|this, _, _, cx| this.apply_settings(cx))))
                    .child(Button::new("back-to-work").outline().label("返回创作").on_click(cx.listener(|this, _, _, cx| { this.settings_open = false; cx.notify(); })))))
            .child(div().flex().flex_col().gap_3().max_w(px(700.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("本地资料库"))
                .child(hint(self.store.as_ref().map(|s| s.root.display().to_string()).unwrap_or_else(|| "数据目录不可用".into())))
                .child(hint("会话、提示词和生成图片自动保存在本机。图片编辑支持 PNG、JPEG、WebP 多图参考。"))
                .child(Button::new("open-library").ghost().icon(IconName::FolderOpen).label("打开资料目录").on_click(cx.listener(|this, _, _, cx| { if let Some(store) = &this.store { cx.open_with_system(&store.root); } })))
                .child(Button::new("api-docs").ghost().icon(IconName::ExternalLink).label("查看 RightAPI 接口文档").on_click(|_, _, cx| cx.open_url("https://docs.rightapi.ai/docs/rc_draw/openai.html"))))
    }
    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let examples = [
            (
                "商业摄影",
                "一瓶透明香水，放在湿润的黑色岩石上，商业棚拍风格，柔和侧光，浅景深。顶部居中写有优雅的英文标题 AURORA。",
            ),
            (
                "插画灵感",
                "一座漂浮在云海之上的微型花园，柔和晨光，精致的植物与玻璃温室，等距视角，温暖的手绘插画。",
            ),
            (
                "产品设计",
                "极简无线耳机的产品概念图，哑光奶油白材质，柔和阴影，干净的浅灰背景，工业设计摄影。",
            ),
        ];
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_5()
            .p_8()
            .child(
                div()
                    .flex()
                    .size_16()
                    .rounded_2xl()
                    .bg(rgb(0x23463f))
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0x9de4cb))
                    .child(Icon::new(IconName::Sparkles).size_8()),
            )
            .child(
                div()
                    .text_3xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("让想象，成为画面。"),
            )
            .child(
                div()
                    .text_color(rgb(0xa4aebc))
                    .child("写下灵感，或上传一张图片开始新的创作。"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_3()
                    .mt_4()
                    .children(
                        examples
                            .into_iter()
                            .enumerate()
                            .map(|(index, (label, prompt))| {
                                Button::new(("example", index))
                                    .outline()
                                    .icon(IconName::ArrowUpRight)
                                    .label(label)
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.mode = Mode::Generate;
                                        this.prompt.update(cx, |s, cx| {
                                            s.set_value(prompt, w, cx);
                                            s.focus(w, cx);
                                        });
                                        cx.notify();
                                    }))
                            }),
                    ),
            )
            .when(self.api_key.is_empty(), |el| {
                el.child(
                    Button::new("setup-first")
                        .ghost()
                        .label("先配置 API Key →")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            cx.notify();
                        })),
                )
            })
    }
}

impl Studio {
    fn render_drawing(&self, drawing: &Drawing, cx: &mut Context<Self>) -> impl IntoElement {
        let id = drawing.id;
        let reuse = drawing.clone();
        let busy = self.running.contains_key(&id);
        let resume = !busy && drawing.task_id.is_some() && drawing.status != Status::Completed;
        let status_color = match drawing.status {
            Status::Completed => rgb(0x93d9b7),
            Status::Failed => rgb(0xf4a6a6),
            _ => rgb(0xb3c5df),
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .flex_shrink_0()
            .gap_4()
            .p_5()
            .rounded_xl()
            .bg(rgb(0x20242c))
            .border_1()
            .border_color(rgb(0x333b46))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x2c343e))
                            .child(drawing.mode.label()),
                    )
                    .child(hint(drawing.settings.model.clone()))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(status_color)
                            .child(drawing.status.label()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip("删除这幅绘画")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.deleting = Some(DeleteTarget::Drawing(id));
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .line_height(rems(1.5))
                    .child(drawing.prompt.clone()),
            )
            .when(!drawing.references.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(hint("参考图"))
                        .children(drawing.references.iter().map(|path| {
                            img(path.clone())
                                .size_12()
                                .object_fit(ObjectFit::Contain)
                                .rounded_md()
                        })),
                )
            })
            .when(drawing.status.active(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_6()
                        .rounded_lg()
                        .bg(rgb(0x181c23))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(Spinner::new().small())
                                .child(format!(
                                    "{} · {}%",
                                    drawing.status.label(),
                                    drawing.progress
                                ))
                                .child(div().flex_1())
                                .child(
                                    Button::new(SharedString::from(format!("pause-{id}")))
                                        .ghost()
                                        .small()
                                        .label("暂停查询")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| this.pause(id, cx)),
                                        ),
                                ),
                        )
                        .child(
                            Progress::new(SharedString::from(format!("progress-{id}")))
                                .value(drawing.progress as f32)
                                .loading(drawing.progress == 0),
                        )
                        .child(hint("结果会自动保存，你可以切换会话继续创作。")),
                )
            })
            .children(drawing.images.iter().map(|image| {
                let edit = image.clone();
                let export = image.clone();
                let open = image.path.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .id(SharedString::from(format!("image-{}", image.id)))
                            .flex()
                            .items_start()
                            .w(px(300.))
                            .h(px(300.))
                            .flex_shrink_0()
                            .rounded_lg()
                            .bg(rgb(0x13161c))
                            .overflow_hidden()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| cx.open_with_system(&open))
                            .child(
                                img(image.path.clone())
                                    .size_full()
                                    .object_fit(ObjectFit::Cover)
                                    .with_fallback(|| {
                                        hint("图片文件不存在或无法读取").into_any_element()
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(hint("300 × 300 预览 · 点击查看原图"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "edit-{}",
                                            image.id
                                        )))
                                        .outline()
                                        .small()
                                        .icon(IconName::Pencil)
                                        .label("编辑此图")
                                        .on_click(
                                            cx.listener(move |this, _, w, cx| {
                                                this.edit_image(edit.clone(), w, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "export-{}",
                                            image.id
                                        )))
                                        .outline()
                                        .small()
                                        .icon(IconName::Download)
                                        .label("另存为")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.export_image(export.clone(), cx)
                                            }),
                                        ),
                                    ),
                            ),
                    )
            }))
            .when_some(drawing.error.clone(), |el, error| {
                el.child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(0x382a2b))
                        .text_sm()
                        .text_color(rgb(0xf1b7b7))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(hint(format!(
                        "{} · {} · {}",
                        drawing.settings.size,
                        drawing.settings.quality,
                        drawing.settings.format.to_uppercase()
                    )))
                    .child(div().flex_1())
                    .when(resume, |el| {
                        el.child(
                            Button::new(SharedString::from(format!("resume-{id}")))
                                .outline()
                                .small()
                                .icon(IconName::RefreshCw)
                                .label("继续查询")
                                .on_click(cx.listener(move |this, _, _, cx| this.resume(id, cx))),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("reuse-{id}")))
                            .ghost()
                            .small()
                            .label("复用提示词")
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.reuse(reuse.clone(), w, cx)),
                            ),
                    ),
            )
    }
    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.session_busy();
        div().flex().flex_col().flex_shrink_0().gap_3().p_5().border_t_1().border_color(rgb(0x2a303a)).bg(rgb(0x1b1f26))
            .child(div().flex().items_center().gap_2()
                .child(Button::new("mode-generate").ghost().small().icon(IconName::Sparkles).label("文生图").selected(self.mode == Mode::Generate)
                    .on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Generate; cx.notify(); })))
                .child(Button::new("mode-edit").ghost().small().icon(IconName::Image).label("图片编辑").selected(self.mode == Mode::Edit)
                    .on_click(cx.listener(|this, _, _, cx| { this.mode = Mode::Edit; cx.notify(); })))
                .child(div().flex_1())
                .child(Button::new("add-reference").ghost().small().icon(IconName::Paperclip).label("添加参考图").loading(self.importing)
                    .on_click(cx.listener(|this, _, _, cx| this.add_references(cx)))))
            .when(self.mode == Mode::Edit, |el| el.child(div().flex().flex_wrap().items_center().gap_2()
                .when(self.references.is_empty(), |el| el.child(hint("添加图片后写下修改指令。支持多图融合；局部修改请直接在提示词中描述。")))
                .children(self.references.iter().enumerate().map(|(index, path)| {
                    div().flex().items_center().gap_1().p_1().rounded_md().bg(rgb(0x2c343e))
                        .child(img(path.clone()).size_10().object_fit(ObjectFit::Contain).rounded_md()).child(hint(format!("图 {}", index + 1)))
                        .child(Button::new(("remove-reference", index)).ghost().small().icon(IconName::X).tooltip("移除参考图")
                            .on_click(cx.listener(move |this, _, _, cx| { if index < this.references.len() { this.references.remove(index); } cx.notify(); })))
                }))))
            .child(Textarea::new(&self.prompt).h(px(94.)).w_full())
            .child(div().flex().items_end().gap_3()
                .child(div().flex().flex_col().gap_1().w(px(222.)).child(hint("模型")).child(Select::new(&self.model).w_full()))
                .child(div().flex().flex_col().gap_1().w(px(120.)).child(hint("画面尺寸")).child(Select::new(&self.size).w_full()))
                .child(div().flex().flex_col().gap_1().w(px(95.)).child(hint("质量")).child(Select::new(&self.quality).w_full()))
                .child(div().flex().flex_col().gap_1().w(px(90.)).child(hint("格式")).child(Select::new(&self.format).w_full()))
                .child(div().flex_1())
                .child(Button::new("generate").primary().icon(IconName::ArrowUp).label(if busy { "正在创作" } else if self.mode == Mode::Edit { "开始编辑" } else { "生成图片" })
                    .disabled(busy || self.importing || self.storage_error.is_some()).on_click(cx.listener(|this, _, w, cx| this.submit(w, cx)))))
            .when(self.custom_size, |el| el.child(div().flex().flex_col().gap_2()
                .child(div().flex().flex_wrap().items_center().gap_2()
                    .child(hint("宽度"))
                    .child(Input::new(&self.custom_width).w(px(100.)))
                    .child(hint("× 高度"))
                    .child(Input::new(&self.custom_height).w(px(100.)))
                    .child(hint("像素 · 每边为 16 的倍数，最大 3840")))
                .child(match model::validate_size(&self.library.settings.size) {
                    Ok(()) => hint("宽高比不超过 3:1，总像素为 655360–8294400").into_any_element(),
                    Err(error) => div().text_xs().text_color(rgb(0xf4a6a6)).child(error.to_string()).into_any_element(),
                })))
            .child(hint("每次生成 1 张 · Ctrl+Enter 发送 · 高质量与大尺寸可能需要更长时间"))
    }
    fn render_delete(&self, target: DeleteTarget, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .m_5()
            .p_5()
            .rounded_lg()
            .border_1()
            .border_color(rgb(0x684343))
            .bg(rgb(0x382a2b))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(match target {
                DeleteTarget::Session(_) => "删除整个会话？",
                DeleteTarget::Drawing(_) => "删除这幅绘画？",
            }))
            .child(
                div()
                    .text_sm()
                    .child("对应的本地记录和生成图片将被删除。已导出的图片会保留。"),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("confirm-delete")
                            .danger()
                            .label("确认删除")
                            .on_click(cx.listener(|this, _, w, cx| this.delete_confirmed(w, cx))),
                    )
                    .child(
                        Button::new("cancel-delete")
                            .outline()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.deleting = None;
                                cx.notify();
                            })),
                    ),
            )
    }
}

impl Render for Studio {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        div()
            .id("studio")
            .key_context("AIDRAW")
            .flex()
            .size_full()
            .bg(background)
            .text_color(rgb(0xe7ecf3))
            .font_family(crate::branding::FONT_FAMILY)
            .text_sm()
            .on_action(cx.listener(|this, _: &Generate, w, cx| this.submit(w, cx)))
            .on_action(cx.listener(|this, _: &NewSession, w, cx| this.new_session(w, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.settings_open = !this.settings_open;
                cx.notify();
            }))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(rgb(0x1b1f26))
                    .child(self.render_header(cx))
                    .when_some(self.storage_error.clone(), |el, error| {
                        el.child(
                            div()
                                .px_5()
                                .py_3()
                                .bg(rgb(0x4a2e2e))
                                .text_color(rgb(0xffb8b8))
                                .child(error),
                        )
                    })
                    .when_some(self.notice.clone(), |el, (error, message)| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_5()
                                .py_2()
                                .bg(if error { rgb(0x382a2b) } else { rgb(0x233e36) })
                                .child(div().flex_1().text_sm().child(message))
                                .child(
                                    Button::new("dismiss-notice")
                                        .ghost()
                                        .small()
                                        .icon(IconName::X)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.notice = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .when_some(self.deleting, |el, target| {
                        el.child(self.render_delete(target, cx))
                    })
                    .when(self.settings_open, |el| el.child(self.render_settings(cx)))
                    .when(!self.settings_open, |el| {
                        el.when(self.library.active().drawings.is_empty(), |el| {
                            el.child(self.render_empty(cx))
                        })
                        .when(!self.library.active().drawings.is_empty(), |el| {
                            el.child(
                                div()
                                    .id("drawings")
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h_0()
                                    .p_6()
                                    .gap_5()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .children(self.library.active().drawings.iter().rev().map(
                                        |drawing| {
                                            self.render_drawing(drawing, cx).into_any_element()
                                        },
                                    )),
                            )
                        })
                        .child(self.render_composer(cx))
                    }),
            )
    }
}

impl Drop for Studio {
    fn drop(&mut self) {
        for cancel in self.running.values() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.persist();
    }
}
