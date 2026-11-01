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
}

fn data_dir() -> PathBuf {
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
