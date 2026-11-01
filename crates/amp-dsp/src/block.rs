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
    /// The built-in amp model, which needs no capture.
    Tube,
    Compressor,
    Chorus,
    Reverb,
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
            Self::Tube => "Understory amp",
            Self::Compressor => "Compressor",
            Self::Chorus => "Chorus",
            Self::Reverb => "Reverb",
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
        const TUBE: &[ParamSpec] = &[
            param("gain", "Gain", "", 0.0, 10.0, 5.0),
            param("bass", "Bass", "", 0.0, 10.0, 5.0),
            param("mid", "Mid", "", 0.0, 10.0, 5.0),
            param("treble", "Treble", "", 0.0, 10.0, 5.0),
            param("presence", "Presence", "", 0.0, 10.0, 3.0),
            param("master", "Master", "", 0.0, 10.0, 5.0),
        ];
        const COMPRESSOR: &[ParamSpec] = &[
            param("threshold", "Threshold", "dB", -50.0, 0.0, -24.0),
            param("ratio", "Ratio", ":1", 1.0, 20.0, 3.0),
            param("release", "Release", "ms", 30.0, 600.0, 160.0),
            param("level", "Level", "dB", -12.0, 18.0, 3.0),
        ];
        const CHORUS: &[ParamSpec] = &[
            param("rate", "Rate", "Hz", 0.1, 6.0, 0.8),
            param("depth", "Depth", "%", 0.0, 100.0, 40.0),
            param("mix", "Mix", "%", 0.0, 100.0, 40.0),
        ];
        const REVERB: &[ParamSpec] = &[
            param("size", "Size", "%", 0.0, 100.0, 50.0),
            param("damping", "Damping", "%", 0.0, 100.0, 40.0),
            param("mix", "Mix", "%", 0.0, 100.0, 20.0),
        ];
        match self {
            Self::Gate => GATE,
            Self::Drive => DRIVE,
            Self::Amp | Self::Pedal => AMP,
            Self::Cab => CAB,
            Self::Delay => DELAY,
            Self::Tube => TUBE,
            Self::Compressor => COMPRESSOR,
            Self::Chorus => CHORUS,
            Self::Reverb => REVERB,
        }
    }

    pub fn is_amp(self) -> bool {
        matches!(self, Self::Amp | Self::Tube)
    }

    /// Whether this block needs a file from the library to make sound.
    pub fn uses_asset(self) -> bool {
        matches!(self, Self::Amp | Self::Pedal | Self::Cab)
    }
}
