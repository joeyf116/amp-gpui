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

/// Writes panics to a file beside the settings. The installed app has no
/// console, so this is the only trace a crash leaves.
fn log_panics() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = format!("Amp GPUI {}: {info}\n", env!("CARGO_PKG_VERSION"));
        let path = settings::data_dir().join("crash.log");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, report.as_bytes()));
        default_hook(info);
    }));
}

fn main() {
    log_panics();
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
