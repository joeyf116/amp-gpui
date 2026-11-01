mod art;
mod knob;
mod rig;
mod settings;
mod style;

use gpui_kit::{
    component::{Theme, ThemeMode, ThemeSet},
    *,
};
use std::rc::Rc;

const THEME: &str = include_str!("../assets/theme.json");
use rig::Rig;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        rig::bind_keys(cx);
        let themes: ThemeSet = serde_json::from_str(THEME).expect("the bundled theme is valid");
        Theme::update(cx, |theme| {
            for config in themes.themes {
                theme.dark_theme = Rc::new(config);
            }
        });
        Theme::change(ThemeMode::Dark, None, cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Amp GPUI".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::centered(size(px(1360.), px(840.)), cx)),
            window_min_size: Some(size(px(1080.), px(640.))),
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        };
        if let Err(error) =
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Rig::new(window, cx)))
        {
            eprintln!("Unable to open the window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
