//! The real-time signal chain. Nothing here knows about audio devices, files
//! on disk beyond loading a model or IR, or the UI.
//!
//! Everything reachable from [`Chain::process`] runs on the audio thread, so it
//! must not allocate, lock, or block. Construction happens off that thread.

mod amp;
mod block;
mod cab;
mod chain;
mod delay;
mod drive;
mod effects;
mod filter;
mod gate;
mod listen;
mod melody;
mod metronome;
mod smooth;
mod song_map;
mod trim;
mod tube;
mod tuner;

pub use amp::{AmpInfo, NamAmp};
pub use block::{Block, BlockKind, ParamSpec};
pub use cab::IrCab;
pub use chain::Chain;
pub use delay::Delay;
pub use drive::Drive;
pub use effects::{Chorus, Compressor, Reverb};
pub use gate::Gate;
pub use listen::{Spectrum, chord_name, midi_hertz};
pub use melody::{MelodyNote, transcribe};
pub use metronome::{Click, Metronome};
/// A parsed `.nam` file, ready to build any number of [`NamAmp`]s from.
pub use nam_rs::NamModel;
pub use song_map::{SongChord, SongKey, SongMap, read_song_map};
pub use trim::Trim;
pub use tube::TubeAmp;
pub use tuner::{Pitch, detect_pitch};

/// The longest slice a block is ever handed in one call. [`Chain::process`]
/// splits anything longer, so blocks can size their scratch space to this.
pub const MAX_BLOCK: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum DspError {
    #[error("could not load the NAM model: {0}")]
    Nam(#[from] nam_rs::Error),
    #[error("could not read the impulse response: {0}")]
    Wav(#[from] hound::Error),
    #[error("the impulse response has no samples")]
    EmptyImpulse,
    #[error("could not set up the convolver: {0}")]
    Convolver(String),
    #[error("this model was captured at {model} Hz but the audio device runs at {device} Hz")]
    SampleRateMismatch { model: u32, device: u32 },
}

pub(crate) fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}
