mod rig;
mod settings;

use gpui_kit::{
    component::{Theme, ThemeMode},
    *,
};
use rig::Rig;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
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
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            window_min_size: Some(size(px(960.), px(600.))),
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
