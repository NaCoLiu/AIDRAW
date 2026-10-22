use gpui_kit::{App, Image, ImageFormat};
use std::{
    borrow::Cow,
    sync::{Arc, OnceLock},
};

pub const FONT_FAMILY: &str = "Maple Mono CN";

pub fn register_fonts(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/fonts/MapleMono-CN-Regular.ttf")),
            Cow::Borrowed(include_bytes!("../assets/fonts/MapleMono-CN-Bold.ttf")),
        ])
        .expect("Failed to load embedded Maple Mono CN fonts");
}

pub fn app_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/icons/aidraw.png").to_vec(),
        ))
    })
    .clone()
}
