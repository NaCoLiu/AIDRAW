#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

extern crate gpui_kit as gpui;

mod api;
mod branding;
mod model;
mod ui;

use gpui_kit::{
    component::{Theme, ThemeMode, set_locale},
    *,
};

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            branding::register_fonts(cx);
            set_locale("zh-CN");
            Theme::change(ThemeMode::Dark, None, cx);
            Theme::update(cx, |theme| {
                theme.font_family = branding::FONT_FAMILY.into();
                theme.mono_font_family = branding::FONT_FAMILY.into();
            });
            ui::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1380.0), px(940.0)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1060.0), px(760.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("AIDRAW · 图像批量创作工作台".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| ui::Studio::new(window, cx)),
            )
            .expect("Failed to open AIDRAW");
            cx.activate(true);
        });
}
