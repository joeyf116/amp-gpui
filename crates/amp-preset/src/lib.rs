//! Presets and the tone library.
//!
//! A library is a plain folder the user owns. Presets are JSON, and models and
//! IRs are stored under the SHA-256 of their contents, so a preset points at
//! exactly one file wherever it travels. The folder works as is in git, a
//! synced drive, or a network share, and a tone pack is the same layout cut
//! down to one preset.

mod build;
mod cache;
mod catalog;
mod factory;
mod level;
mod library;
mod preset;

pub use build::{BuiltChain, build_chain, level_slot};
pub use catalog::{CaptureEntry, CaptureKind};
pub use factory::factory_tones;
pub use level::{MATCHED_LOUDNESS, loudness, matched_level, trim_to};
pub use library::{Library, LibraryError, PresetEntry};
pub use preset::{
    AssetKind, AssetRef, BlockPreset, LEVEL_RANGE, MAX_SCENES, Preset, SCHEMA_VERSION, Scene,
    SceneBlock, ToneOrigin,
};
