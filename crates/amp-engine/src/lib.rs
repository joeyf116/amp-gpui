//! The bridge between the signal chain, the audio devices, and the UI.
//!
//! The job is live monitoring: guitar into an audio interface, amp tone out of
//! any output, with the two allowed to be separate devices. [`Processor`] and
//! `Pump` are the audio thread's half and have no device code in them, so tests
//! can run them directly. [`Engine`] is the UI thread's half and owns the
//! device streams.

mod engine;
#[cfg(windows)]
mod exclusive;
mod processor;
mod pump;
mod resampler;
mod song;
mod song_file;

pub use engine::{
    DeviceList, DeviceOption, Engine, EngineConfig, EngineError, EngineInfo, LatencyMode,
};
pub use processor::{Command, Controller, Latency, Meters, Processor, processor};
pub use pump::ENGINE_RATE;
pub use song::{SongFeed, SongOutlet, SongPlan};
pub use song_file::{SongError, read_song};
