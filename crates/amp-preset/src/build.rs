use crate::{BlockPreset, Library, LibraryError, Preset};
use amp_dsp::{
    AmpInfo, Block, BlockKind, Chain, Chorus, Compressor, Delay, Drive, Gate, IrCab, NamAmp,
    Reverb, Trim, TubeAmp,
};

/// A chain ready for the engine, and what the UI should show about it.
///
/// The chain has one slot per block of the preset, in order, and then the
/// tone's Level at [`level_slot`].
pub struct BuiltChain {
    pub chain: Chain,
    /// One entry per block, set for the blocks that run a capture.
    pub captures: Vec<Option<AmpInfo>>,
    /// One entry per block, set for a block whose file couldn't be used. That
    /// block passes the signal through, or plays the built-in cabinet if it's
    /// a cab, so the rest of the tone still plays.
    pub problems: Vec<Option<String>>,
}

/// The chain slot of the tone's Level, which comes after every block.
pub fn level_slot(preset: &Preset) -> usize {
    preset.blocks.len()
}

/// A built block, what its capture says about itself, and what went wrong.
type Built = (Box<dyn Block>, Option<AmpInfo>, Option<String>);

fn build_block(block: &BlockPreset, library: &Library, sample_rate: u32) -> Built {
    let plain = |block: Box<dyn Block>| (block, None, None);
    match block.kind {
        BlockKind::Gate => plain(Box::new(Gate::new(sample_rate))),
        BlockKind::Drive => plain(Box::new(Drive::new(sample_rate))),
        BlockKind::Delay => plain(Box::new(Delay::new(sample_rate))),
        BlockKind::Tube => plain(Box::new(TubeAmp::new(sample_rate))),
        BlockKind::Compressor => plain(Box::new(Compressor::new(sample_rate))),
        BlockKind::Chorus => plain(Box::new(Chorus::new(sample_rate))),
        BlockKind::Reverb => plain(Box::new(Reverb::new(sample_rate))),
        BlockKind::Amp | BlockKind::Pedal => {
            let Some(asset) = &block.asset else {
                return plain(Box::new(NamAmp::empty(sample_rate)));
            };
            let loaded = library.capture(asset).and_then(|model| {
                NamAmp::from_nam(&model, sample_rate).map_err(LibraryError::from)
            });
            match loaded {
                Ok((amp, info)) => (Box::new(amp), Some(info), None),
                Err(error) => (
                    Box::new(NamAmp::empty(sample_rate)),
                    None,
                    Some(error.to_string()),
                ),
            }
        }
        BlockKind::Cab => {
            let loaded = block.asset.as_ref().map(|asset| {
                library.impulse(asset).and_then(|impulse| {
                    IrCab::from_samples(&impulse.0, impulse.1, sample_rate)
                        .map_err(LibraryError::from)
                })
            });
            match loaded {
                Some(Ok(cab)) => plain(Box::new(cab)),
                // A cab is what tames an amp's fizz, so a missing IR falls
                // back to the built-in voice rather than nothing at all.
                Some(Err(error)) => (built_in_cab(sample_rate), None, Some(error.to_string())),
                None => plain(built_in_cab(sample_rate)),
            }
        }
    }
}

fn built_in_cab(sample_rate: u32) -> Box<dyn Block> {
    match IrCab::built_in(sample_rate) {
        Ok(cab) => Box::new(cab),
        Err(_) => Box::new(IrCab::empty(sample_rate)),
    }
}

/// Builds the chain for `preset`. This reads model files, so it belongs on a
/// background thread, never the audio thread. A file that's missing or won't
/// run is reported in [`BuiltChain::problems`] instead of failing the tone.
pub fn build_chain(
    preset: &Preset,
    library: &Library,
    sample_rate: u32,
) -> Result<BuiltChain, LibraryError> {
    let mut chain = Chain::default();
    let mut captures = Vec::with_capacity(preset.blocks.len());
    let mut problems = Vec::with_capacity(preset.blocks.len());
    for block_preset in &preset.blocks {
        let (mut block, capture, problem) = build_block(block_preset, library, sample_rate);
        for index in 0..block_preset.kind.params().len() {
            block.set_param(index, block_preset.param(index));
        }
        chain.push(block, !block_preset.enabled);
        captures.push(capture);
        problems.push(problem);
    }
    let mut level = Trim::new(sample_rate);
    level.set_param(0, preset.level());
    chain.push(Box::new(level), false);
    chain.settle(sample_rate);
    Ok(BuiltChain {
        chain,
        captures,
        problems,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetKind, AssetRef};

    fn library(name: &str) -> (Library, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("amp-preset-build-{name}-{}", std::process::id()));
        (Library::open(&directory).unwrap(), directory)
    }

    #[test]
    fn builds_every_block_of_the_starter_preset_and_the_level() {
        let (library, directory) = library("starter");
        let built = build_chain(&Preset::starter("Test"), &library, 48_000).unwrap();
        assert_eq!(built.chain.len(), 6);
        assert!(built.captures.iter().all(Option::is_none));
        assert!(built.problems.iter().all(Option::is_none));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_missing_file_is_reported_and_the_rest_still_builds() {
        let (library, directory) = library("missing");
        let mut preset = Preset::starter("Test");
        let missing = |kind| AssetRef {
            kind,
            sha256: "a".repeat(64),
            name: "Gone.wav".into(),
        };
        preset.blocks[2].asset = Some(missing(AssetKind::NamModel));
        preset.blocks[3].asset = Some(missing(AssetKind::ImpulseResponse));
        let built = build_chain(&preset, &library, 48_000).unwrap();
        assert_eq!(built.chain.len(), 6);
        assert!(built.problems[2].as_deref().unwrap().contains("Gone.wav"));
        assert!(built.problems[3].is_some());
        assert!(built.problems[0].is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_capture_is_read_once_and_built_from_again() {
        let (library, directory) = library("cache");
        let file = directory.join("tiny.nam");
        std::fs::write(
            &file,
            r#"{"version":"0.5.2","architecture":"LSTM",
            "config":{"input_size":1,"hidden_size":1,"num_layers":1},"metadata":{},
            "weights":[0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.0],
            "sample_rate":48000}"#,
        )
        .unwrap();
        let asset = library.import_asset(AssetKind::NamModel, &file).unwrap();
        let mut preset = Preset::new("Test", vec![BlockPreset::new(BlockKind::Amp, true)]);
        preset.blocks[0].asset = Some(asset.clone());
        assert!(build_chain(&preset, &library, 48_000).unwrap().captures[0].is_some());

        // With the stored file gone, only the cache can supply it.
        std::fs::remove_file(library.asset_path(&asset).unwrap()).unwrap();
        let built = build_chain(&preset, &library.clone(), 48_000).unwrap();
        assert!(built.captures[0].is_some());
        assert!(built.problems[0].is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_level_turns_the_whole_tone_down() {
        let (library, directory) = library("level");
        let mut preset = Preset::new("Test", vec![BlockPreset::new(BlockKind::Drive, true)]);
        let run = |preset: &Preset| {
            let mut built = build_chain(preset, &library, 48_000).unwrap();
            let mut buffer: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.05).sin() * 0.1).collect();
            built.chain.process(&mut buffer);
            buffer
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
        };
        let loud = run(&preset);
        preset.level_db = -12.0;
        let quiet = run(&preset);
        assert!((quiet / loud - 0.25).abs() < 0.02, "{loud} {quiet}");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
