use amp_assistant::ModelChoice;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// What the app remembers between runs. Device choices matter most: the
/// default input on most machines is a webcam, not the guitar.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
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
    #[serde(default = "default_bpm")]
    pub bpm: f32,
    #[serde(default = "default_beats")]
    pub beats: u32,
    #[serde(default = "default_click_db")]
    pub click_db: f32,
    #[serde(default = "default_song_db")]
    pub song_db: f32,
    /// Silences the rig while the tuner is open.
    #[serde(default)]
    pub tuner_mute: bool,
    /// The fretboard's key, as a semitone above C.
    #[serde(default = "default_key")]
    pub key: usize,
    pub scale: String,
    /// Set to label the fretboard with intervals rather than note names.
    pub fret_intervals: bool,
    /// How stiff each open string of this guitar is, once it has been learned.
    pub string_stiffness: Option<[f32; 6]>,
    /// Which Claude model the tone assistant uses.
    pub assistant_model: ModelChoice,
}

/// A, the key most guitar players reach for first.
fn default_key() -> usize {
    9
}

fn default_bpm() -> f32 {
    120.0
}

fn default_beats() -> u32 {
    4
}

fn default_click_db() -> f32 {
    -12.0
}

fn default_song_db() -> f32 {
    -12.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            input_channel: 0,
            library: None,
            last_tone: None,
            shared_audio: false,
            input_db: 0.0,
            output_db: 0.0,
            asio_driver: None,
            favorites: Vec::new(),
            recent: Vec::new(),
            bpm: default_bpm(),
            beats: default_beats(),
            click_db: default_click_db(),
            song_db: default_song_db(),
            tuner_mute: false,
            key: default_key(),
            scale: String::new(),
            fret_intervals: false,
            string_stiffness: None,
            assistant_model: ModelChoice::default(),
        }
    }
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

    /// Damaged settings are set aside rather than overwritten, so the app still
    /// starts and the old file can be looked at.
    pub fn load() -> Self {
        let path = Self::path();
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| {
            let _ = fs::rename(&path, path.with_extension("json.bad"));
            Self::default()
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Through a temporary file, so a crash can't leave half of it behind.
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_string_pretty(self)?)?;
        fs::rename(temporary, path)
    }

    pub fn library_root(&self) -> PathBuf {
        self.library
            .clone()
            .unwrap_or_else(|| data_dir().join("library"))
    }
}
