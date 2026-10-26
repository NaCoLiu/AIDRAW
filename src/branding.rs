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

pub fn transparency_grid() -> Arc<Image> {
    static GRID: OnceLock<Arc<Image>> = OnceLock::new();
    GRID.get_or_init(|| {
        let pixels = image::RgbImage::from_fn(440, 220, |x, y| {
            image::Rgb(if (x / 22 + y / 22) % 2 == 0 {
                [60, 70, 82]
            } else {
                [48, 57, 67]
            })
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("Failed to create transparency grid");
        Arc::new(Image::from_bytes(ImageFormat::Png, bytes.into_inner()))
    })
    .clone()
}
