//! Original gear art. Each picture is rendered by `tools/gear-art/render.py` and
//! suggests a type of pedal, amp, or cab, with nothing taken from a real product.
//! Knobs, lamps, and names stay out of the pictures and are drawn here, so they
//! always show real values. A capture's own name is printed on it, as the file
//! supplies it.

use crate::style::{glow, lamp_glow};
use amp_dsp::BlockKind;
use gpui_kit::{component::StyledExt, prelude::FluentBuilder, *};
use std::sync::{Arc, OnceLock};

/// The type face printed on gear. Windows has shipped it since Windows 10, and
/// text falls back to the UI font without it.
pub const PRINT_FONT: &str = "Bahnschrift";

/// Picture sizes in the chain strip, which are half the size of the files.
const PEDAL_THUMB: (f32, f32) = (70.0, 112.5);
const HEAD_THUMB: (f32, f32) = (187.5, 87.5);
const CAB_THUMB: (f32, f32) = (112.5, 107.5);
/// Picture sizes on the stage, at a scale of 1.
const PEDAL_STAGE: (f32, f32) = (280.0, 450.0);
const HEAD_STAGE: (f32, f32) = (750.0, 350.0);
const CAB_STAGE: (f32, f32) = (450.0, 430.0);

/// Where the pedal's lamp socket and footswitch sit, as shares of the picture.
const PEDAL_LED: (f32, f32) = (0.5, 0.645);
const PEDAL_FOOTSWITCH: (f32, f32) = (0.5, 0.8);
const HEAD_LAMP: (f32, f32) = (0.9067, 0.7086);
const HEAD_BADGE: (f32, f32, f32, f32) = (0.375, 0.255, 0.25, 0.126);
const CAB_BADGE: (f32, f32, f32, f32) = (0.687, 0.82, 0.213, 0.06);

/// The height a stage picture is laid out for, to scale it to the window.
pub const STAGE_HEIGHT: f32 = PEDAL_STAGE.1;

macro_rules! picture {
    ($file:expr, $format:ident) => {{
        static PICTURE: OnceLock<Arc<Image>> = OnceLock::new();
        PICTURE
            .get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::$format,
                    include_bytes!(concat!("../assets/gear/", $file)).to_vec(),
                ))
            })
            .clone()
    }};
}

macro_rules! gear {
    ($name:literal, $small:expr) => {
        if $small {
            picture!(concat!($name, "_s.webp"), Webp)
        } else {
            picture!(concat!($name, ".webp"), Webp)
        }
    };
}

pub fn logo() -> Arc<Image> {
    picture!("logo.png", Png)
}

pub fn knob_amp() -> Arc<Image> {
    picture!("knob_amp.png", Png)
}

pub fn knob_pedal() -> Arc<Image> {
    picture!("knob_pedal.png", Png)
}

pub fn stage_backdrop() -> Arc<Image> {
    picture!("stage.webp", Webp)
}

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
            Self::Gate => "noise gate",
            Self::Compressor => "compressor",
            Self::Boost => "boost",
            Self::Overdrive => "overdrive",
            Self::Distortion => "distortion",
            Self::Fuzz => "fuzz",
            Self::Modulation => "modulation",
            Self::Delay => "delay",
            Self::Reverb => "reverb",
            Self::Capture => "pedal capture",
        }
    }

    fn picture(self, small: bool) -> Arc<Image> {
        match self {
            Self::Gate => gear!("pedal_gate", small),
            Self::Compressor => gear!("pedal_compressor", small),
            Self::Boost => gear!("pedal_boost", small),
            Self::Overdrive => gear!("pedal_overdrive", small),
            Self::Distortion => gear!("pedal_distortion", small),
            Self::Fuzz => gear!("pedal_fuzz", small),
            Self::Modulation => gear!("pedal_modulation", small),
            Self::Delay => gear!("pedal_delay", small),
            Self::Reverb => gear!("pedal_reverb", small),
            Self::Capture => gear!("pedal_capture", small),
        }
    }

    /// The print color on this pedal's paint, matched to its screen printing.
    fn ink(self) -> Hsla {
        match self {
            Self::Boost => hsla(0.0, 0.0, 0.08, 1.0),
            Self::Capture => hsla(0.4, 0.1, 0.07, 1.0),
            Self::Fuzz => hsla(0.12, 0.62, 0.62, 1.0),
            _ => hsla(0.12, 0.3, 0.88, 1.0),
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
    /// `capture` marks a NAM capture, rather than the built-in amp.
    Head {
        capture: bool,
    },
    /// `open` shows bare speakers, for a cabinet running a loaded IR.
    Cabinet {
        open: bool,
    },
}

/// What a piece of gear looks like and what it's called. The type always shows
/// beside the name, so an original name never hides what a unit does.
#[derive(Clone, Debug, PartialEq)]
pub struct GearArt {
    pub shape: Shape,
    pub title: SharedString,
    pub type_label: SharedString,
}

/// The name printed on built-in gear.
pub fn model_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Amp => "No capture",
        BlockKind::Pedal => "No capture",
        BlockKind::Tube => "Understory",
        BlockKind::Cab => "Deadfall 412",
        BlockKind::Compressor => "Slow Sap",
        BlockKind::Drive => "Cinder",
        BlockKind::Gate => "Stillwater",
        BlockKind::Chorus => "Mycelium",
        BlockKind::Delay => "Long Ridge",
        BlockKind::Reverb => "Fog Bank",
    }
}

impl Shape {
    pub fn for_kind(kind: BlockKind, name: &str, has_asset: bool) -> Self {
        match kind {
            BlockKind::Amp => Self::Head { capture: true },
            BlockKind::Tube => Self::Head { capture: false },
            BlockKind::Cab => Self::Cabinet { open: has_asset },
            BlockKind::Gate => Self::Stomp(PedalLook::Gate),
            BlockKind::Drive => Self::Stomp(PedalLook::Overdrive),
            BlockKind::Delay => Self::Stomp(PedalLook::Delay),
            BlockKind::Compressor => Self::Stomp(PedalLook::Compressor),
            BlockKind::Chorus => Self::Stomp(PedalLook::Modulation),
            BlockKind::Reverb => Self::Stomp(PedalLook::Reverb),
            BlockKind::Pedal => Self::Stomp(PedalLook::from_text(name)),
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            Self::Stomp(look) => look.rank(),
            Self::Head { .. } => 10,
            Self::Cabinet { .. } => 11,
        }
    }

    pub fn type_label(&self) -> &'static str {
        match self {
            Self::Stomp(look) => look.label(),
            Self::Head { capture: true } => "amp capture",
            Self::Head { capture: false } => "tube head",
            Self::Cabinet { open: true } => "cabinet IR",
            Self::Cabinet { open: false } => "4x12 cabinet",
        }
    }

    fn picture(&self, small: bool) -> Arc<Image> {
        match self {
            Self::Stomp(look) => look.picture(small),
            Self::Head { capture: true } => gear!("head_capture", small),
            Self::Head { capture: false } => gear!("head_understory", small),
            Self::Cabinet { open: true } => gear!("cab_open", small),
            Self::Cabinet { open: false } => gear!("cab_closed", small),
        }
    }

    fn thumb_size(&self) -> (f32, f32) {
        match self {
            Self::Stomp(_) => PEDAL_THUMB,
            Self::Head { .. } => HEAD_THUMB,
            Self::Cabinet { .. } => CAB_THUMB,
        }
    }

    pub fn stage_size(&self) -> (f32, f32) {
        match self {
            Self::Stomp(_) => PEDAL_STAGE,
            Self::Head { .. } => HEAD_STAGE,
            Self::Cabinet { .. } => CAB_STAGE,
        }
    }

    pub fn is_stomp(&self) -> bool {
        matches!(self, Self::Stomp(_))
    }
}

/// A small lit lamp, centered on a share of its parent. Bypassed units keep the
/// dark socket that is already in the picture.
fn lamp(at: (f32, f32), size: f32, color: Hsla, halo: BoxShadow) -> Div {
    div()
        .absolute()
        .left(relative(at.0))
        .top(relative(at.1))
        .ml(px(-size / 2.0))
        .mt(px(-size / 2.0))
        .size(px(size))
        .rounded_full()
        .bg(color)
        .shadow(vec![halo])
}

fn unit_lamp(shape: &Shape, scale: f32) -> Option<Div> {
    match shape {
        Shape::Stomp(_) => Some(lamp(
            PEDAL_LED,
            10.0 * scale,
            hsla(0.41, 0.72, 0.68, 1.0),
            glow(0.85, 14.0 * scale),
        )),
        Shape::Head { .. } => Some(lamp(
            HEAD_LAMP,
            15.0 * scale,
            hsla(0.08, 0.92, 0.58, 0.5),
            lamp_glow(0.75, 22.0 * scale),
        )),
        Shape::Cabinet { .. } => None,
    }
}

/// A unit in the chain strip. A bypassed unit stays visible but drops back.
pub fn thumb(art: &GearArt, enabled: bool) -> Div {
    let (width, height) = art.shape.thumb_size();
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .child(img(art.shape.picture(true)).size_full())
        .when_some(
            unit_lamp(&art.shape, 0.5).filter(|_| enabled),
            |unit, lamp| unit.child(lamp),
        )
        .opacity(if enabled { 1.0 } else { 0.5 })
}

fn printed(text: SharedString, size: f32, ink: Hsla) -> Div {
    div()
        .font_family(PRINT_FONT)
        .text_size(px(size))
        .text_color(ink)
        .whitespace_nowrap()
        .child(text)
}

fn plate(area: (f32, f32, f32, f32), title: SharedString, size: f32) -> Div {
    div()
        .absolute()
        .left(relative(area.0))
        .top(relative(area.1))
        .w(relative(area.2))
        .h(relative(area.3))
        .flex()
        .items_center()
        .justify_center()
        .px(px(size * 0.7))
        .child(
            printed(title, size, hsla(0.11, 0.42, 0.72, 1.0))
                .font_semibold()
                .truncate(),
        )
}

/// How big a stage knob should be for this unit, before scaling.
pub fn stage_knob(shape: &Shape, knobs: usize) -> f32 {
    match shape {
        Shape::Stomp(_) if knobs >= 4 => 42.0,
        Shape::Stomp(_) => 58.0,
        Shape::Head { .. } if knobs >= 5 => 54.0,
        Shape::Head { .. } => 66.0,
        Shape::Cabinet { .. } => 66.0,
    }
}

/// The print color for a knob's label on this unit's faceplate.
pub fn faceplate_ink(shape: &Shape) -> Hsla {
    match shape {
        Shape::Stomp(look) => look.ink(),
        _ => hsla(0.12, 0.14, 0.86, 1.0),
    }
}

/// Draws a unit at stage size with `knobs` set into its faceplate.
pub fn stage(art: &GearArt, enabled: bool, scale: f32, knobs: Vec<AnyElement>) -> Div {
    let (width, height) = art.shape.stage_size();
    let unit = div()
        .relative()
        .flex_shrink_0()
        .w(px(width * scale))
        .h(px(height * scale))
        .child(img(art.shape.picture(false)).size_full())
        .when_some(
            unit_lamp(&art.shape, scale).filter(|_| enabled),
            |unit, lamp| unit.child(lamp),
        );
    let row = div().absolute().flex().items_start().justify_around();
    match &art.shape {
        Shape::Stomp(look) => unit
            .child(
                row.left(relative(0.13))
                    .right(relative(0.13))
                    .top(relative(0.075))
                    .children(knobs),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(0.13))
                    .right(relative(0.13))
                    .top(relative(0.335))
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        printed(art.title.clone(), 19.0 * scale, look.ink())
                            .font_semibold()
                            .truncate(),
                    )
                    .child(printed(
                        art.type_label.clone(),
                        10.5 * scale,
                        look.ink().opacity(0.72),
                    )),
            ),
        Shape::Head { .. } => unit
            .child(
                row.left(relative(0.16))
                    .right(relative(0.2))
                    .top(relative(0.545))
                    .children(knobs),
            )
            .child(plate(HEAD_BADGE, art.title.clone(), 21.0 * scale)),
        Shape::Cabinet { open: false } => {
            unit.child(plate(CAB_BADGE, art.title.clone(), 12.0 * scale))
        }
        Shape::Cabinet { open: true } => unit,
    }
}

/// The share of a pedal picture to press for the footswitch, as left, top,
/// width, and height.
pub const PEDAL_SWITCH_AREA: (f32, f32, f32, f32) = (
    PEDAL_FOOTSWITCH.0 - 0.2,
    PEDAL_FOOTSWITCH.1 - 0.11,
    0.4,
    0.22,
);

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
            Shape::Head { capture: false },
            Shape::Cabinet { open: false },
            Shape::Stomp(PedalLook::Delay),
            Shape::Stomp(PedalLook::Reverb),
        ];
        assert!(order.windows(2).all(|pair| pair[0].rank() < pair[1].rank()));
    }

    #[test]
    fn a_cabinet_opens_up_once_it_has_an_ir() {
        use amp_dsp::BlockKind;
        assert_eq!(
            Shape::for_kind(BlockKind::Cab, "room", true),
            Shape::Cabinet { open: true }
        );
        assert_eq!(
            Shape::for_kind(BlockKind::Cab, "Deadfall 412", false),
            Shape::Cabinet { open: false }
        );
    }
}
