//! Original drawings of gear. Each one suggests a type of pedal, amp, or cab
//! through its shape and color. Nothing here copies a real product's look, name,
//! or logo. A capture's own name is printed on it, as the file supplies it.

use crate::{
    knob::disc,
    style::{glow, moss},
};
use amp_dsp::BlockKind;
use gpui_kit::{component::StyledExt, *};
use std::f32::consts::PI;

pub const UNIT_HEIGHT: f32 = 176.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PedalLook {
    Gate,
    Compressor,
    Boost,
    Overdrive,
    Distortion,
    Fuzz,
    Modulation,
    Delay,
    Reverb,
    Capture,
}

impl PedalLook {
    /// Reads a pedal's type out of its name or tone description.
    pub fn from_text(text: &str) -> Self {
        let text = text.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| text.contains(word));
        if has(&["fuzz"]) {
            Self::Fuzz
        } else if has(&["dist", "metal", "hi_gain", "hi-gain"]) {
            Self::Distortion
        } else if has(&["drive", "screamer", " od", "crunch"]) {
            Self::Overdrive
        } else if has(&["boost"]) {
            Self::Boost
        } else if has(&["comp"]) {
            Self::Compressor
        } else if has(&["delay", "echo"]) {
            Self::Delay
        } else if has(&["reverb", "verb"]) {
            Self::Reverb
        } else if has(&["chorus", "phase", "flang", "trem", "vibe"]) {
            Self::Modulation
        } else if has(&["gate"]) {
            Self::Gate
        } else {
            Self::Capture
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::Compressor => "compressor",
            Self::Boost => "boost",
            Self::Overdrive => "overdrive",
            Self::Distortion => "distortion",
            Self::Fuzz => "fuzz",
            Self::Modulation => "modulation",
            Self::Delay => "delay",
            Self::Reverb => "reverb",
            Self::Capture => "pedal",
        }
    }

    /// Muted enclosure paint, so a board reads by type at a glance without any
    /// one pedal shouting over the rest.
    fn hue(self) -> (f32, f32, f32) {
        match self {
            Self::Gate => (0.56, 0.1, 0.3),
            Self::Compressor => (0.58, 0.28, 0.36),
            Self::Boost => (0.12, 0.42, 0.5),
            Self::Overdrive => (0.27, 0.34, 0.33),
            Self::Distortion => (0.03, 0.5, 0.36),
            Self::Fuzz => (0.92, 0.36, 0.34),
            Self::Modulation => (0.75, 0.28, 0.4),
            Self::Delay => (0.49, 0.36, 0.3),
            Self::Reverb => (0.62, 0.3, 0.42),
            Self::Capture => (0.42, 0.18, 0.3),
        }
    }

    /// Where this type usually sits, front of the chain first. Compression and
    /// lower gain come early, time-based effects after the amp, and a gate
    /// after the drives whose noise it is there to catch.
    fn rank(self) -> u8 {
        match self {
            Self::Compressor => 1,
            Self::Fuzz => 2,
            Self::Boost | Self::Capture => 3,
            Self::Overdrive => 4,
            Self::Distortion => 5,
            Self::Gate => 6,
            Self::Modulation => 20,
            Self::Delay => 21,
            Self::Reverb => 22,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Stomp(PedalLook),
    Head,
    Cabinet,
}

/// A drawing of one piece of gear. `knobs` are real settings, each from 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct GearArt {
    pub shape: Shape,
    pub title: SharedString,
    pub knobs: Vec<f32>,
}

impl Shape {
    pub fn for_kind(kind: BlockKind, name: &str) -> Self {
        match kind {
            BlockKind::Amp => Self::Head,
            BlockKind::Cab => Self::Cabinet,
            BlockKind::Gate => Self::Stomp(PedalLook::Gate),
            BlockKind::Drive => Self::Stomp(PedalLook::Overdrive),
            BlockKind::Delay => Self::Stomp(PedalLook::Delay),
            BlockKind::Pedal => Self::Stomp(PedalLook::from_text(name)),
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            Self::Stomp(look) => look.rank(),
            Self::Head => 10,
            Self::Cabinet => 11,
        }
    }
}

fn dial(amount: f32, size: f32, body: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center = bounds.center();
            let radius = bounds.size.width / 2.0;
            if let Some(path) = disc(center, radius) {
                window.paint_path(path, hsla(0.0, 0.0, 0.0, 0.55));
            }
            if let Some(path) = disc(center, radius - px(1.5)) {
                window.paint_path(path, body);
            }
            let angle = 0.75 * PI + 1.5 * PI * amount;
            let mut pointer = PathBuilder::stroke(px(1.6));
            pointer.move_to(point(
                center.x + radius * 0.25 * angle.cos(),
                center.y + radius * 0.25 * angle.sin(),
            ));
            pointer.line_to(point(
                center.x + radius * 0.8 * angle.cos(),
                center.y + radius * 0.8 * angle.sin(),
            ));
            if let Ok(path) = pointer.build() {
                window.paint_path(path, hsla(0.12, 0.3, 0.88, 0.95));
            }
        },
    )
    .size(px(size))
}

fn led(enabled: bool) -> impl IntoElement {
    div()
        .size(px(7.))
        .rounded_full()
        .bg(if enabled {
            moss()
        } else {
            hsla(0.42, 0.1, 0.16, 1.0)
        })
        .when_glow(enabled)
}

trait WhenGlow: Styled + Sized {
    fn when_glow(self, enabled: bool) -> Self {
        if enabled {
            self.shadow(vec![glow(0.75, 10.0)])
        } else {
            self
        }
    }
}

impl<T: Styled> WhenGlow for T {}

fn footswitch() -> impl IntoElement {
    div()
        .size(px(30.))
        .rounded_full()
        .border_2()
        .border_color(hsla(0.0, 0.0, 0.0, 0.5))
        .bg(linear_gradient(
            160.,
            linear_color_stop(hsla(0.0, 0.0, 0.78, 1.0), 0.),
            linear_color_stop(hsla(0.0, 0.0, 0.42, 1.0), 1.),
        ))
}

fn stomp(art: &GearArt, look: PedalLook, enabled: bool) -> Div {
    let (hue, saturation, lightness) = look.hue();
    let paint = hsla(hue, saturation, lightness, 1.0);
    let shade = hsla(hue, saturation, lightness * 0.68, 1.0);
    let knob = hsla(hue, 0.12, 0.12, 1.0);
    let row: Vec<_> = art
        .knobs
        .iter()
        .take(3)
        .map(|amount| dial(*amount, 24.0, knob))
        .collect();
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_between()
        .w(px(100.))
        .h(px(UNIT_HEIGHT))
        .p_2()
        .rounded_lg()
        .border_1()
        .border_color(hsla(0.0, 0.0, 1.0, 0.14))
        .bg(linear_gradient(
            165.,
            linear_color_stop(paint, 0.),
            linear_color_stop(shade, 1.),
        ))
        .child(div().flex().gap_1().justify_center().children(row))
        .child(led(enabled))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .text_xs()
                        .font_semibold()
                        .truncate()
                        .text_color(hsla(0.12, 0.2, 0.93, 1.0))
                        .child(art.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .text_color(hsla(0.12, 0.1, 0.8, 0.7))
                        .child(look.label()),
                ),
        )
        .child(footswitch())
}

fn head(art: &GearArt, enabled: bool) -> Div {
    let knob = hsla(0.1, 0.1, 0.14, 1.0);
    let row: Vec<_> = art
        .knobs
        .iter()
        .take(4)
        .map(|amount| dial(*amount, 26.0, knob))
        .collect();
    div()
        .flex()
        .flex_col()
        .w(px(190.))
        .h(px(UNIT_HEIGHT))
        .p_2()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(hsla(0.0, 0.0, 1.0, 0.1))
        .bg(hsla(0.3, 0.06, 0.1, 1.0))
        .child(
            div()
                .flex_1()
                .rounded_md()
                .border_1()
                .border_color(hsla(0.0, 0.0, 0.0, 0.5))
                .bg(pattern_slash(hsla(0.3, 0.1, 0.24, 0.5), 1.0, 5.0))
                .flex()
                .items_center()
                .justify_center()
                .px_2()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .truncate()
                        .text_color(hsla(0.12, 0.2, 0.9, 1.0))
                        .child(art.title.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(hsla(0.12, 0.12, 0.22, 1.0))
                .child(div().flex().gap_2().children(row))
                .child(led(enabled)),
        )
}

fn cabinet(art: &GearArt, enabled: bool) -> Div {
    let speaker = || {
        canvas(
            |_, _, _| {},
            |bounds, _, window, _| {
                let center = bounds.center();
                let radius = bounds.size.width / 2.0;
                for (share, color) in [
                    (1.0, hsla(0.0, 0.0, 0.03, 1.0)),
                    (0.88, hsla(0.3, 0.05, 0.13, 1.0)),
                    (0.36, hsla(0.3, 0.05, 0.22, 1.0)),
                ] {
                    if let Some(path) = disc(center, radius * share) {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size(px(50.))
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_between()
        .w(px(124.))
        .h(px(UNIT_HEIGHT))
        .p_2()
        .rounded_lg()
        .border_1()
        .border_color(hsla(0.0, 0.0, 1.0, 0.1))
        .bg(hsla(0.3, 0.05, 0.11, 1.0))
        .child(speaker())
        .child(speaker())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .w_full()
                .child(led(enabled))
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .truncate()
                        .text_color(hsla(0.12, 0.15, 0.85, 1.0))
                        .child(art.title.clone()),
                ),
        )
}

/// Draws `art` at full size. A bypassed unit stays visible but drops back.
pub fn draw(art: &GearArt, enabled: bool) -> Div {
    let unit = match art.shape {
        Shape::Stomp(look) => stomp(art, look, enabled),
        Shape::Head => head(art, enabled),
        Shape::Cabinet => cabinet(art, enabled),
    };
    unit.opacity(if enabled { 1.0 } else { 0.55 })
}

#[cfg(test)]
mod tests {
    use super::{PedalLook, Shape};

    #[test]
    fn reads_the_type_of_a_pedal_from_its_name() {
        assert_eq!(
            PedalLook::from_text("Green Drive 808"),
            PedalLook::Overdrive
        );
        assert_eq!(PedalLook::from_text("Big FUZZ"), PedalLook::Fuzz);
        assert_eq!(PedalLook::from_text("tape echo"), PedalLook::Delay);
        assert_eq!(PedalLook::from_text("mystery box"), PedalLook::Capture);
    }

    #[test]
    fn the_usual_order_runs_from_dynamics_through_the_amp_to_time_effects() {
        let order = [
            Shape::Stomp(PedalLook::Compressor),
            Shape::Stomp(PedalLook::Overdrive),
            Shape::Stomp(PedalLook::Gate),
            Shape::Head,
            Shape::Cabinet,
            Shape::Stomp(PedalLook::Delay),
            Shape::Stomp(PedalLook::Reverb),
        ];
        assert!(order.windows(2).all(|pair| pair[0].rank() < pair[1].rank()));
    }
}
