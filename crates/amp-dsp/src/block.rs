use serde::{Deserialize, Serialize};

/// One processor in the chain. `process` runs on the audio thread.
pub trait Block: Send {
    /// Sets the parameter at `index` in this kind's [`BlockKind::params`].
    /// Values arrive already clamped to the spec's range.
    fn set_param(&mut self, index: usize, value: f32);

    /// Processes `buffer` in place. It is never longer than
    /// [`MAX_BLOCK`](crate::MAX_BLOCK).
    fn process(&mut self, buffer: &mut [f32]);

    /// Returns to silence, dropping tails and history.
    fn reset(&mut self);
}

/// A control a block exposes to presets and the UI.
///
/// `id` is what presets store, so renaming one breaks saved presets. `name` and
/// the order of the list can change freely.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

const fn param(
    id: &'static str,
    name: &'static str,
    unit: &'static str,
    min: f32,
    max: f32,
    default: f32,
) -> ParamSpec {
    ParamSpec {
        id,
        name,
        unit,
        min,
        max,
        default,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Gate,
    Drive,
    Amp,
    /// A NAM capture of a pedal. It runs like an amp capture and sits anywhere.
    Pedal,
    Cab,
    Delay,
}

impl BlockKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Gate => "Noise gate",
            Self::Drive => "Drive",
            Self::Amp => "Amp",
            Self::Pedal => "Pedal",
            Self::Cab => "Cabinet",
            Self::Delay => "Delay",
        }
    }

    pub fn params(self) -> &'static [ParamSpec] {
        const GATE: &[ParamSpec] = &[
            param("threshold", "Threshold", "dB", -90.0, -20.0, -65.0),
            param("release", "Release", "ms", 10.0, 500.0, 120.0),
        ];
        const DRIVE: &[ParamSpec] = &[
            param("drive", "Drive", "dB", 0.0, 36.0, 12.0),
            param("level", "Level", "dB", -24.0, 6.0, -6.0),
        ];
        const AMP: &[ParamSpec] = &[
            param("input", "Input", "dB", -24.0, 24.0, 0.0),
            param("output", "Output", "dB", -24.0, 24.0, 0.0),
            param("quality", "Quality", "%", 0.0, 100.0, 100.0),
        ];
        const CAB: &[ParamSpec] = &[param("level", "Level", "dB", -24.0, 12.0, 0.0)];
        const DELAY: &[ParamSpec] = &[
            param("time", "Time", "ms", 20.0, 1500.0, 380.0),
            param("feedback", "Feedback", "%", 0.0, 90.0, 30.0),
            param("mix", "Mix", "%", 0.0, 100.0, 20.0),
        ];
        match self {
            Self::Gate => GATE,
            Self::Drive => DRIVE,
            Self::Amp | Self::Pedal => AMP,
            Self::Cab => CAB,
            Self::Delay => DELAY,
        }
    }

    /// Whether this block needs a file from the library to make sound.
    pub fn uses_asset(self) -> bool {
        matches!(self, Self::Amp | Self::Pedal | Self::Cab)
    }
}
