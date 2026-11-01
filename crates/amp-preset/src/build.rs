use crate::{Library, LibraryError, Preset};
use amp_dsp::{Block, BlockKind, Chain, Delay, Drive, Gate, IrCab, NamAmp};

/// Builds the chain for `preset`. This reads model files, so it belongs on a
/// background thread, never the audio thread.
pub fn build_chain(
    preset: &Preset,
    library: &Library,
    sample_rate: u32,
) -> Result<Chain, LibraryError> {
    let mut chain = Chain::default();
    for block_preset in &preset.blocks {
        let path = block_preset
            .asset
            .as_ref()
            .map(|asset| library.asset_path(asset))
            .transpose()?;
        let mut block: Box<dyn Block> = match (block_preset.kind, path) {
            (BlockKind::Gate, _) => Box::new(Gate::new(sample_rate)),
            (BlockKind::Drive, _) => Box::new(Drive::new(sample_rate)),
            (BlockKind::Delay, _) => Box::new(Delay::new(sample_rate)),
            (BlockKind::Amp, Some(path)) => Box::new(NamAmp::load(&path, sample_rate)?.0),
            (BlockKind::Amp, None) => Box::new(NamAmp::empty(sample_rate)),
            (BlockKind::Cab, Some(path)) => Box::new(IrCab::load(&path, sample_rate)?),
            (BlockKind::Cab, None) => Box::new(IrCab::empty(sample_rate)),
        };
        for index in 0..block_preset.kind.params().len() {
            block.set_param(index, block_preset.param(index));
        }
        chain.push(block, !block_preset.enabled);
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_every_block_of_the_starter_preset() {
        let directory = std::env::temp_dir().join("amp-preset-build-test");
        let library = Library::open(&directory).unwrap();
        let chain = build_chain(&Preset::starter("Test"), &library, 48_000).unwrap();
        assert_eq!(chain.len(), 5);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
