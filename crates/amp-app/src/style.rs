//! The look of the app: near-black and near-white neutrals carrying a trace of
//! green, with the logo's green marking what is live and its off-white for text.
//! Behind them, a pine forest in smoke, a faint survey grid, and drifting mist.

use crate::{art, knob::disc};
use gpui_kit::*;

pub fn moss() -> Hsla {
    hsla(0.41, 0.52, 0.62, 1.0)
}

pub fn moss_glow(alpha: f32) -> Hsla {
    hsla(0.41, 0.6, 0.62, alpha)
}

pub fn pane() -> Hsla {
    hsla(0.42, 0.05, 0.075, 0.78)
}

/// The bars at the top and bottom of the window, which sit over the forest.
pub fn bar() -> Hsla {
    hsla(0.42, 0.05, 0.068, 0.94)
}

/// Panels that open over everything else.
pub fn sheet() -> Hsla {
    hsla(0.42, 0.05, 0.088, 0.985)
}

pub fn hairline() -> Hsla {
    hsla(0.42, 0.06, 0.92, 0.13)
}

pub fn fog() -> Hsla {
    hsla(0.42, 0.04, 0.66, 1.0)
}

/// A soft halo in moss light.
pub fn glow(alpha: f32, radius: f32) -> BoxShadow {
    halo(moss_glow(alpha), radius)
}

/// The warm halo of a pilot lamp.
pub fn lamp_glow(alpha: f32, radius: f32) -> BoxShadow {
    halo(hsla(0.08, 0.95, 0.55, alpha), radius)
}

fn halo(color: Hsla, radius: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(radius),
        spread_radius: px(0.),
        inset: false,
    }
}

const GRID: f32 = 36.0;
const SMOKE_LAYERS: usize = 9;
/// Where each bank of smoke sits, as a share of the window, its size, and how
/// fast it drifts.
const SMOKE: [(f32, f32, f32, f32); 5] = [
    (0.18, 0.28, 0.46, 0.9),
    (0.72, 0.18, 0.38, -0.7),
    (0.5, 0.72, 0.56, 0.5),
    (0.9, 0.8, 0.42, -1.1),
    (0.08, 0.9, 0.34, 0.8),
];

/// Forest, grid, and smoke, painted behind everything. `phase` is a slowly
/// rising clock.
pub fn backdrop(phase: f32) -> impl IntoElement {
    div()
        .absolute()
        .size_full()
        .child(
            img(art::stage_backdrop())
                .absolute()
                .size_full()
                .object_fit(ObjectFit::Cover),
        )
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    for (x, y, size, speed) in SMOKE {
                        let center = point(
                            bounds.origin.x
                                + bounds.size.width * (x + (phase * speed * 0.03).sin() * 0.06),
                            bounds.origin.y
                                + bounds.size.height * (y + (phase * speed * 0.02).cos() * 0.05),
                        );
                        let reach = bounds.size.width * size;
                        for layer in 0..SMOKE_LAYERS {
                            let share = 1.0 - layer as f32 / SMOKE_LAYERS as f32;
                            if let Some(path) = disc(center, reach * share) {
                                window.paint_path(path, hsla(0.42, 0.03, 0.7, 0.0065));
                            }
                        }
                    }

                    let line = hsla(0.42, 0.05, 0.85, 0.024);
                    let mut x = bounds.origin.x;
                    while x < bounds.right() {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(x, bounds.origin.y),
                                size(px(1.), bounds.size.height),
                            ),
                            line,
                        ));
                        x += px(GRID);
                    }
                    let mut y = bounds.origin.y;
                    while y < bounds.bottom() {
                        window.paint_quad(fill(
                            Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, px(1.))),
                            line,
                        ));
                        y += px(GRID);
                    }
                },
            )
            .absolute()
            .size_full(),
        )
}
