use crate::{AssetKind, AssetRef, Preset, SCHEMA_VERSION};
use amp_dsp::DspError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const PRESETS: &str = "presets";
const ASSETS: &str = "assets";
pub(crate) const PRACTICE: &str = "practice";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{path} could not be read: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Dsp(#[from] DspError),
    #[error("\"{0}\" needs a newer version of this app")]
    NewerSchema(String),
    #[error("the asset reference {0:?} is not a SHA-256 hash")]
    InvalidHash(String),
    #[error("{name} is missing from the library")]
    MissingAsset { name: String },
    #[error("{name} does not match the hash in its preset, so it was not imported")]
    HashMismatch { name: String },
    #[error("a preset name needs at least one letter or number")]
    InvalidName,
}

/// One preset found in a library folder.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetEntry {
    pub path: PathBuf,
    pub preset: Preset,
}

/// A tone library rooted at a folder the user chose.
#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, LibraryError> {
        let root = root.into();
        fs::create_dir_all(root.join(PRESETS))?;
        fs::create_dir_all(root.join(ASSETS))?;
        fs::create_dir_all(root.join(PRACTICE))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every readable preset, sorted by name, plus the errors for the rest. A
    /// broken file in a shared folder shouldn't hide all the others.
    pub fn presets(&self) -> Result<(Vec<PresetEntry>, Vec<LibraryError>), LibraryError> {
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        for file in fs::read_dir(self.root.join(PRESETS))? {
            let path = file?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                match read_preset(&path) {
                    Ok(preset) => entries.push(PresetEntry { path, preset }),
                    Err(error) => errors.push(error),
                }
            }
        }
        entries.sort_by_key(|entry| entry.preset.name.to_lowercase());
        Ok((entries, errors))
    }

    /// Writes `preset` under a file name derived from its name, replacing any
    /// preset with the same name.
    pub fn save_preset(&self, preset: &Preset) -> Result<PathBuf, LibraryError> {
        let path = self.preset_path(&preset.name)?;
        write_json(&path, preset)?;
        Ok(path)
    }

    fn preset_path(&self, name: &str) -> Result<PathBuf, LibraryError> {
        let slug = slug(name).ok_or(LibraryError::InvalidName)?;
        Ok(self.root.join(PRESETS).join(format!("{slug}.json")))
    }

    /// Copies a model or IR into the library and returns its reference.
    pub fn import_asset(&self, kind: AssetKind, source: &Path) -> Result<AssetRef, LibraryError> {
        let sha256 = hash_file(source)?;
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| sha256.clone());
        let asset = AssetRef { kind, sha256, name };
        let destination = self.stored_path(&asset)?;
        if !destination.exists() {
            fs::copy(source, destination)?;
        }
        Ok(asset)
    }

    /// Where `asset` lives in this library, if it is there.
    pub fn asset_path(&self, asset: &AssetRef) -> Result<PathBuf, LibraryError> {
        let path = self.stored_path(asset)?;
        if path.exists() {
            Ok(path)
        } else {
            Err(LibraryError::MissingAsset {
                name: asset.name.clone(),
            })
        }
    }

    /// Preset files come from other people, so the hash is checked before it
    /// goes anywhere near a path. Nothing else in the name is used.
    fn stored_path(&self, asset: &AssetRef) -> Result<PathBuf, LibraryError> {
        let hash = &asset.sha256;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(LibraryError::InvalidHash(hash.clone()));
        }
        let file = format!("{}.{}", hash.to_lowercase(), asset.kind.extension());
        Ok(self.root.join(ASSETS).join(file))
    }

    /// Writes a tone pack for `preset` to `destination`: a small library with
    /// just this preset and the files it needs. TONE3000's terms don't allow
    /// passing its files on, so those stay behind and the preset keeps a
    /// reference, which each person's own account downloads again.
    pub fn export_pack(&self, preset: &Preset, destination: &Path) -> Result<(), LibraryError> {
        let pack = Self::open(destination)?;
        for asset in preset
            .blocks
            .iter()
            .filter(|block| block.origin.is_none())
            .filter_map(|block| block.asset.as_ref())
        {
            fs::copy(self.asset_path(asset)?, pack.stored_path(asset)?)?;
        }
        pack.save_preset(preset)?;
        Ok(())
    }

    /// Adds every preset in the pack at `source` to this library. Each file
    /// is re-hashed first, so a pack can't swap in something else.
    pub fn import_pack(&self, source: &Path) -> Result<Vec<Preset>, LibraryError> {
        let pack = Self {
            root: source.into(),
        };
        let (entries, errors) = pack.presets()?;
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        let presets: Vec<Preset> = entries.into_iter().map(|entry| entry.preset).collect();
        let assets = || {
            presets
                .iter()
                .flat_map(|preset| &preset.blocks)
                .filter(|block| block.origin.is_none())
                .filter_map(|block| block.asset.as_ref())
        };
        for asset in assets() {
            let path = pack.asset_path(asset)?;
            if !hash_file(&path)?.eq_ignore_ascii_case(&asset.sha256) {
                return Err(LibraryError::HashMismatch {
                    name: asset.name.clone(),
                });
            }
        }
        for asset in assets() {
            let destination = self.stored_path(asset)?;
            if !destination.exists() {
                fs::copy(pack.stored_path(asset)?, destination)?;
            }
        }
        for preset in &presets {
            self.save_preset(preset)?;
        }
        Ok(presets)
    }
}

fn read_preset(path: &Path) -> Result<Preset, LibraryError> {
    let text = fs::read_to_string(path)?;
    let preset: Preset = serde_json::from_str(&text).map_err(|source| LibraryError::Json {
        path: path.to_owned(),
        source,
    })?;
    if preset.schema > SCHEMA_VERSION {
        return Err(LibraryError::NewerSchema(preset.name));
    }
    Ok(preset)
}

/// Writes through a temporary file, so a crash mid-save can't leave a library
/// that syncs half a file to everyone else.
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<(), LibraryError> {
    let json = serde_json::to_string_pretty(value).map_err(|source| LibraryError::Json {
        path: path.to_owned(),
        source,
    })?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, json)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, LibraryError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn slug(name: &str) -> Option<String> {
    let mut slug = String::new();
    for character in name.to_lowercase().chars() {
        if character.is_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    (!slug.is_empty()).then_some(slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToneOrigin;
    use amp_dsp::BlockKind;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("amp-library-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn preset_with_asset(library: &Library, directory: &Path) -> Preset {
        let source = directory.join("Plexi Crunch.nam");
        fs::write(&source, b"model bytes").unwrap();
        let mut preset = Preset::starter("Crunch: Lead #2");
        let amp = preset
            .blocks
            .iter_mut()
            .find(|block| block.kind == BlockKind::Amp)
            .unwrap();
        amp.asset = Some(library.import_asset(AssetKind::NamModel, &source).unwrap());
        preset
    }

    #[test]
    fn saves_and_lists_presets_by_name() {
        let directory = TempDir::new();
        let library = Library::open(&directory.0).unwrap();
        library.save_preset(&Preset::starter("Zebra")).unwrap();
        library.save_preset(&Preset::starter("alpha")).unwrap();
        let (entries, errors) = library.presets().unwrap();
        assert!(errors.is_empty());
        let names: Vec<_> = entries.iter().map(|e| e.preset.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zebra"]);
    }

    #[test]
    fn one_broken_preset_does_not_hide_the_rest() {
        let directory = TempDir::new();
        let library = Library::open(&directory.0).unwrap();
        library.save_preset(&Preset::starter("Good")).unwrap();
        fs::write(directory.0.join(PRESETS).join("bad.json"), "{").unwrap();
        let (entries, errors) = library.presets().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn imports_the_same_file_once_by_content() {
        let directory = TempDir::new();
        let library = Library::open(directory.0.join("library")).unwrap();
        let first = directory.0.join("a.nam");
        let second = directory.0.join("b.nam");
        fs::write(&first, b"same").unwrap();
        fs::write(&second, b"same").unwrap();
        let first = library.import_asset(AssetKind::NamModel, &first).unwrap();
        let second = library.import_asset(AssetKind::NamModel, &second).unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(
            fs::read_dir(library.root().join(ASSETS)).unwrap().count(),
            1
        );
    }

    #[test]
    fn a_pack_moves_a_preset_and_its_assets_between_libraries() {
        let directory = TempDir::new();
        let mine = Library::open(directory.0.join("mine")).unwrap();
        let theirs = Library::open(directory.0.join("theirs")).unwrap();
        let preset = preset_with_asset(&mine, &directory.0);
        let pack = directory.0.join("pack");
        mine.export_pack(&preset, &pack).unwrap();

        assert_eq!(
            theirs.import_pack(&pack).unwrap(),
            std::slice::from_ref(&preset)
        );
        let asset = preset.blocks[2].asset.as_ref().unwrap();
        assert_eq!(
            fs::read(theirs.asset_path(asset).unwrap()).unwrap(),
            b"model bytes"
        );
    }

    #[test]
    fn a_pack_leaves_tone3000_files_behind_but_keeps_the_reference() {
        let directory = TempDir::new();
        let mine = Library::open(directory.0.join("mine")).unwrap();
        let theirs = Library::open(directory.0.join("theirs")).unwrap();
        let mut preset = preset_with_asset(&mine, &directory.0);
        let amp = &mut preset.blocks[2];
        amp.origin = Some(ToneOrigin {
            tone_id: 7,
            ..ToneOrigin::default()
        });
        let asset = amp.asset.clone().unwrap();
        let pack = directory.0.join("pack");
        mine.export_pack(&preset, &pack).unwrap();

        assert_eq!(fs::read_dir(pack.join(ASSETS)).unwrap().count(), 0);
        assert_eq!(
            theirs.import_pack(&pack).unwrap(),
            std::slice::from_ref(&preset)
        );
        assert!(matches!(
            theirs.asset_path(&asset),
            Err(LibraryError::MissingAsset { .. })
        ));
    }

    #[test]
    fn rejects_a_pack_whose_asset_was_swapped() {
        let directory = TempDir::new();
        let mine = Library::open(directory.0.join("mine")).unwrap();
        let theirs = Library::open(directory.0.join("theirs")).unwrap();
        let preset = preset_with_asset(&mine, &directory.0);
        let pack = directory.0.join("pack");
        mine.export_pack(&preset, &pack).unwrap();
        let asset = preset.blocks[2].asset.as_ref().unwrap();
        let packed = Library::open(&pack).unwrap().asset_path(asset).unwrap();
        fs::write(packed, b"something else").unwrap();

        assert!(matches!(
            theirs.import_pack(&pack),
            Err(LibraryError::HashMismatch { .. })
        ));
        assert!(theirs.presets().unwrap().0.is_empty());
        assert!(matches!(
            theirs.asset_path(asset),
            Err(LibraryError::MissingAsset { .. })
        ));
    }

    #[test]
    fn rejects_a_hash_that_tries_to_leave_the_library() {
        let directory = TempDir::new();
        let library = Library::open(&directory.0).unwrap();
        let asset = AssetRef {
            kind: AssetKind::NamModel,
            sha256: "../../secrets".into(),
            name: "x".into(),
        };
        assert!(matches!(
            library.asset_path(&asset),
            Err(LibraryError::InvalidHash(_))
        ));
    }

    #[test]
    fn slugs_keep_names_safe_for_a_file() {
        assert_eq!(slug("Crunch: Lead #2").as_deref(), Some("crunch-lead-2"));
        assert_eq!(slug("../etc").as_deref(), Some("etc"));
        assert_eq!(slug("!!!"), None);
    }
}
