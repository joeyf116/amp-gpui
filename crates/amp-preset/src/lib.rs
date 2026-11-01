//! Presets and the tone library.
//!
//! A library is a plain folder the user owns. Presets are JSON, and models and
//! IRs are stored under the SHA-256 of their contents, so a preset points at
//! exactly one file wherever it travels. The folder works as is in git, a
//! synced drive, or a network share, and a tone pack is the same layout cut
//! down to one preset.

mod build;
mod catalog;
mod library;
mod practice;
mod preset;

pub use build::{BuiltChain, build_chain};
pub use catalog::{CaptureEntry, CaptureKind};
pub use library::{Library, LibraryError, PresetEntry};
pub use practice::{PracticeSection, PracticeSheet};
pub use preset::{AssetKind, AssetRef, BlockPreset, Preset, SCHEMA_VERSION};
