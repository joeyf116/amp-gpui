use crate::{Library, LibraryError, Preset};
use amp_dsp::{
    AmpInfo, Block, BlockKind, Chain, Chorus, Compressor, Delay, Drive, Gate, IrCab, NamAmp,
    Reverb, TubeAmp,
};

/// A chain ready for the engine, and what the UI should show about it.
pub struct BuiltChain {
    pub chain: Chain,
    /// One entry per block, set for the blocks that run a capture.
    pub captures: Vec<Option<AmpInfo>>,
}

/// Builds the chain for `preset`. This reads model files, so it belongs on a
/// background thread, never the audio thread.
pub fn build_chain(
    preset: &Preset,
    library: &Library,
    sample_rate: u32,
) -> Result<BuiltChain, LibraryError> {
    let mut chain = Chain::default();
    let mut captures = Vec::with_capacity(preset.blocks.len());
    for block_preset in &preset.blocks {
        let path = block_preset
            .asset
            .as_ref()
            .map(|asset| library.asset_path(asset))
            .transpose()?;
        let mut capture = None;
        let mut block: Box<dyn Block> = match (block_preset.kind, path) {
            (BlockKind::Gate, _) => Box::new(Gate::new(sample_rate)),
            (BlockKind::Drive, _) => Box::new(Drive::new(sample_rate)),
            (BlockKind::Delay, _) => Box::new(Delay::new(sample_rate)),
            (BlockKind::Amp | BlockKind::Pedal, Some(path)) => {
                let (block, info) = NamAmp::load(&path, sample_rate)?;
                capture = Some(info);
                Box::new(block)
            }
            (BlockKind::Amp | BlockKind::Pedal, None) => Box::new(NamAmp::empty(sample_rate)),
            (BlockKind::Cab, Some(path)) => Box::new(IrCab::load(&path, sample_rate)?),
            (BlockKind::Cab, None) => Box::new(IrCab::built_in(sample_rate)?),
            (BlockKind::Tube, _) => Box::new(TubeAmp::new(sample_rate)),
            (BlockKind::Compressor, _) => Box::new(Compressor::new(sample_rate)),
            (BlockKind::Chorus, _) => Box::new(Chorus::new(sample_rate)),
            (BlockKind::Reverb, _) => Box::new(Reverb::new(sample_rate)),
        };
        for index in 0..block_preset.kind.params().len() {
            block.set_param(index, block_preset.param(index));
        }
        chain.push(block, !block_preset.enabled);
        captures.push(capture);
    }
    chain.settle(sample_rate);
    Ok(BuiltChain { chain, captures })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_every_block_of_the_starter_preset() {
        let directory = std::env::temp_dir().join("amp-preset-build-test");
        let library = Library::open(&directory).unwrap();
        let built = build_chain(&Preset::starter("Test"), &library, 48_000).unwrap();
        assert_eq!(built.chain.len(), 5);
        assert!(built.captures.iter().all(Option::is_none));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
