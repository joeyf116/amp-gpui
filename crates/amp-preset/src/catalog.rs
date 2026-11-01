use crate::{AssetKind, AssetRef, Library, LibraryError, ToneOrigin, library::write_json};
use amp_dsp::{AmpInfo, NamAmp};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const CATALOG: &str = "captures.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Amp,
    Pedal,
}

/// A capture in the gear rack. These details live in a small catalog because
/// reading them from the `.nam` means parsing megabytes of weights.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CaptureEntry {
    pub asset: AssetRef,
    pub title: String,
    pub kind: CaptureKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone_type: Option<String>,
    #[serde(default)]
    pub quality_tiers: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ToneOrigin>,
}

impl CaptureEntry {
    fn new(asset: AssetRef, info: AmpInfo) -> Self {
        let kind = match info.gear_type.as_deref() {
            Some(gear_type) if gear_type.to_lowercase().contains("pedal") => CaptureKind::Pedal,
            _ => CaptureKind::Amp,
        };
        let file_stem = asset
            .name
            .rsplit_once('.')
            .map_or(asset.name.as_str(), |(stem, _)| stem);
        Self {
            title: info
                .name
                .clone()
                .or_else(|| info.gear.clone())
                .unwrap_or_else(|| file_stem.to_owned()),
            asset,
            kind,
            gear: info.gear,
            tone_type: info.tone_type,
            quality_tiers: info.quality_tiers,
            origin: None,
        }
    }
}

impl Library {
    /// The captures in this library, sorted by title.
    pub fn captures(&self) -> Result<Vec<CaptureEntry>, LibraryError> {
        let path = self.root().join(CATALOG);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        serde_json::from_str(&text).map_err(|source| LibraryError::Json { path, source })
    }

    /// Copies a `.nam` file in and lists it in the rack. It is parsed before it
    /// is stored, so a broken file never reaches the library.
    pub fn add_capture(&self, source: &Path) -> Result<CaptureEntry, LibraryError> {
        let info = NamAmp::inspect(source)?;
        let asset = self.import_asset(AssetKind::NamModel, source)?;
        self.list_capture(CaptureEntry::new(asset, info))
    }

    /// Adds a downloaded model, titled and credited as it was published rather
    /// than by what the file says about itself. `kind` comes from the tone too,
    /// which knows better than a file with no metadata.
    pub fn add_tone_capture(
        &self,
        source: &Path,
        kind: CaptureKind,
        origin: ToneOrigin,
    ) -> Result<CaptureEntry, LibraryError> {
        let info = NamAmp::inspect(source)?;
        let asset = self.import_asset(AssetKind::NamModel, source)?;
        let mut entry = CaptureEntry::new(asset, info);
        entry.title = origin.title.clone();
        entry.kind = kind;
        entry.origin = Some(origin);
        self.list_capture(entry)
    }

    fn list_capture(&self, entry: CaptureEntry) -> Result<CaptureEntry, LibraryError> {
        let mut entries = self.captures()?;
        entries.retain(|existing| existing.asset.sha256 != entry.asset.sha256);
        entries.push(entry.clone());
        entries.sort_by_key(|entry| entry.title.to_lowercase());
        write_json(&self.root().join(CATALOG), &entries)?;
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn weights() -> &'static str {
        "[0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.0]"
    }

    fn capture_json(metadata: &str) -> String {
        format!(
            r#"{{"version":"0.5.2","architecture":"LSTM",
            "config":{{"input_size":1,"hidden_size":1,"num_layers":1}},
            "metadata":{metadata},"weights":{},"sample_rate":48000}}"#,
            weights()
        )
    }

    #[test]
    fn sorts_captures_into_amps_and_pedals_by_their_metadata() {
        let directory = std::env::temp_dir().join(format!("amp-catalog-{}", std::process::id()));
        let library = Library::open(directory.join("library")).unwrap();
        let pedal = directory.join("drive.nam");
        let amp = directory.join("head.nam");
        fs::write(
            &pedal,
            capture_json(r#"{"name":"Mid Drive","gear_type":"pedal"}"#),
        )
        .unwrap();
        fs::write(
            &amp,
            capture_json(r#"{"gear_make":"Test","gear_model":"Head"}"#),
        )
        .unwrap();

        let added = library.add_capture(&pedal).unwrap();
        assert_eq!(added.kind, CaptureKind::Pedal);
        assert_eq!(added.title, "Mid Drive");
        library.add_capture(&amp).unwrap();
        library.add_capture(&pedal).unwrap();

        let entries = library.captures().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Mid Drive");
        assert_eq!(entries[1].kind, CaptureKind::Amp);
        assert_eq!(entries[1].title, "Test Head");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_downloaded_capture_keeps_its_published_title_and_credit() {
        let directory =
            std::env::temp_dir().join(format!("amp-catalog-tone-{}", std::process::id()));
        let library = Library::open(directory.join("library")).unwrap();
        let file = directory.join("model.nam");
        fs::write(&file, capture_json(r#"{"name":"Whatever the file says"}"#)).unwrap();
        let origin = ToneOrigin {
            tone_id: 7,
            title: "Plexi 51".into(),
            creator: "brucew".into(),
            ..ToneOrigin::default()
        };

        let entry = library
            .add_tone_capture(&file, CaptureKind::Pedal, origin.clone())
            .unwrap();
        assert_eq!(entry.title, "Plexi 51");
        assert_eq!(entry.kind, CaptureKind::Pedal);
        assert_eq!(library.captures().unwrap()[0].origin, Some(origin));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_broken_file_is_not_added() {
        let directory =
            std::env::temp_dir().join(format!("amp-catalog-bad-{}", std::process::id()));
        let library = Library::open(directory.join("library")).unwrap();
        let broken = directory.join("broken.nam");
        fs::write(&broken, "not a model").unwrap();
        assert!(library.add_capture(&broken).is_err());
        assert!(library.captures().unwrap().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }
}
