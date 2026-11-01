use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// What the app remembers between runs. Device choices matter most: the
/// default input on most machines is a webcam, not the guitar.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Settings {
    pub input: Option<String>,
    pub output: Option<String>,
    #[serde(default)]
    pub input_channel: usize,
    pub library: Option<PathBuf>,
    /// The tone to reopen, by name, so the app starts ready to play.
    pub last_tone: Option<String>,
    /// Set to use the Windows mixer, so other apps keep playing while
    /// monitoring, at the cost of much higher latency.
    #[serde(default)]
    pub shared_audio: bool,
    #[serde(default)]
    pub input_db: f32,
    #[serde(default)]
    pub output_db: f32,
    /// Set to run through this interface's ASIO driver.
    #[serde(default)]
    pub asio_driver: Option<String>,
    /// Tones marked with a star, by name.
    #[serde(default)]
    pub favorites: Vec<String>,
    /// Tones loaded lately, by name, newest first.
    #[serde(default)]
    pub recent: Vec<String>,
}

/// `AMP_GPUI_DATA` points the app at another folder, which keeps test runs away
/// from a real library.
pub fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("AMP_GPUI_DATA") {
        return path.into();
    }
    ProjectDirs::from("", "", "amp-gpui")
        .map(|dirs| dirs.data_dir().to_owned())
        .unwrap_or_else(|| PathBuf::from("amp-gpui-data"))
}

impl Settings {
    fn path() -> PathBuf {
        data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)
    }

    pub fn library_root(&self) -> PathBuf {
        self.library
            .clone()
            .unwrap_or_else(|| data_dir().join("library"))
    }
}
