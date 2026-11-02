//! Measures how loud a tone runs, so a new one can be turned down before it
//! reaches the player's ears. Nobody hears a generated tone first.

use crate::{LEVEL_RANGE, Library, LibraryError, Preset, build_chain};
use amp_dsp::MAX_BLOCK;
use std::f32::consts::TAU;

const SECONDS: f32 = 1.5;
const WINDOW_SECONDS: f32 = 0.05;
/// About what a single-coil puts into an interface on a firm chord.
const PLUCK_LEVEL: f32 = 0.25;
const PLUCK_DECAY: f32 = 4.0;
/// An open E chord, which puts energy where the low and high strings do.
const PLUCK_HERTZ: [f32; 6] = [82.4, 123.5, 164.8, 207.7, 246.9, 329.6];
/// A tone this much louder than the last one starts to jump out.
const ALLOWED_RISE_DB: f32 = 2.0;

fn test_chord(sample_rate: u32) -> Vec<f32> {
    let frames = (SECONDS * sample_rate as f32) as usize;
    (0..frames)
        .map(|frame| {
            let time = frame as f32 / sample_rate as f32;
            let notes: f32 = PLUCK_HERTZ
                .iter()
                .map(|hertz| (TAU * hertz * time).sin())
                .sum();
            notes / PLUCK_HERTZ.len() as f32 * PLUCK_LEVEL * (-PLUCK_DECAY * time).exp()
        })
        .collect()
}

/// The loudest short stretch of `samples`, in dB.
fn peak_loudness(samples: &[f32], sample_rate: u32) -> f32 {
    let window = ((WINDOW_SECONDS * sample_rate as f32) as usize).max(1);
    let loudest = samples
        .chunks(window)
        .map(|chunk| chunk.iter().map(|sample| sample * sample).sum::<f32>() / chunk.len() as f32)
        .fold(0.0, f32::max);
    10.0 * loudest.max(1e-12).log10()
}

/// How loud `preset` runs on a strummed test chord, in dB. This loads every
/// model, so it belongs on a background thread.
pub fn loudness(preset: &Preset, library: &Library, sample_rate: u32) -> Result<f32, LibraryError> {
    let mut built = build_chain(preset, library, sample_rate)?;
    let mut samples = test_chord(sample_rate);
    for chunk in samples.chunks_mut(MAX_BLOCK) {
        built.chain.process(chunk);
    }
    Ok(peak_loudness(&samples, sample_rate))
}

/// Turns `preset` down if it runs well over `reference`. Returns how many dB
/// came off, and never turns anything up. Only the tone's Level moves, which
/// leaves how the amp and pedals are driven alone.
pub fn trim_to(preset: &mut Preset, measured: f32, reference: f32) -> Option<f32> {
    let excess = measured - reference;
    if excess <= ALLOWED_RISE_DB {
        return None;
    }
    let current = preset.level();
    let trimmed = (current - excess).max(LEVEL_RANGE.0);
    preset.level_db = trimmed;
    (trimmed < current).then_some(current - trimmed)
}

/// Where Match level puts a tone, in dB on the test chord. It's about the
/// middle of the factory tones, so a matched tone sits among them.
pub const MATCHED_LOUDNESS: f32 = -14.5;

/// The Level that brings `preset` to [`MATCHED_LOUDNESS`], in half dB steps.
/// This loads every model, so it belongs on a background thread.
pub fn matched_level(
    preset: &Preset,
    library: &Library,
    sample_rate: u32,
) -> Result<f32, LibraryError> {
    let mut flat = preset.clone();
    flat.level_db = 0.0;
    let measured = loudness(&flat, library, sample_rate)?;
    let level = ((MATCHED_LOUDNESS - measured) * 2.0).round() / 2.0;
    Ok(level.clamp(LEVEL_RANGE.0, LEVEL_RANGE.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockPreset;
    use amp_dsp::BlockKind;

    fn library(name: &str) -> (Library, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("amp-level-{name}-{}", std::process::id()));
        (Library::open(&directory).unwrap(), directory)
    }

    fn tone(gain: f32, master: f32) -> Preset {
        let mut preset = Preset::starter("Test");
        preset.blocks = vec![
            BlockPreset::new(BlockKind::Tube, true),
            BlockPreset::new(BlockKind::Cab, true),
        ];
        preset.blocks[0].params.insert("gain".into(), gain);
        preset.blocks[0].params.insert("master".into(), master);
        preset
    }

    #[test]
    fn a_higher_master_measures_louder() {
        let (library, directory) = library("master");
        let quiet = loudness(&tone(3.0, 2.0), &library, 48_000).unwrap();
        let loud = loudness(&tone(3.0, 8.0), &library, 48_000).unwrap();
        assert!(loud > quiet + 3.0, "{quiet} {loud}");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_hot_tone_comes_down_to_about_the_reference() {
        let (library, directory) = library("trim");
        let reference = loudness(&tone(3.0, 2.0), &library, 48_000).unwrap();
        let mut hot = tone(3.0, 8.0);
        let measured = loudness(&hot, &library, 48_000).unwrap();
        let removed = trim_to(&mut hot, measured, reference).unwrap();
        assert!(removed > 0.0);
        let after = loudness(&hot, &library, 48_000).unwrap();
        assert!((after - reference).abs() < 1.5, "{reference} {after}");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn nothing_is_ever_turned_up() {
        let mut quiet = tone(3.0, 2.0);
        assert_eq!(trim_to(&mut quiet, -30.0, -10.0), None);
        assert_eq!(trim_to(&mut quiet, -10.0, -11.0), None);
    }

    #[test]
    fn a_tone_already_at_the_bottom_stays_there() {
        let mut preset = tone(3.0, 8.0);
        preset.level_db = LEVEL_RANGE.0;
        assert_eq!(trim_to(&mut preset, 0.0, -20.0), None);
    }

    #[test]
    fn match_level_brings_tones_to_the_same_loudness() {
        let (library, directory) = library("match");
        let mut quiet = tone(2.0, 2.0);
        let mut loud = tone(7.0, 8.0);
        quiet.level_db = matched_level(&quiet, &library, 48_000).unwrap();
        loud.level_db = matched_level(&loud, &library, 48_000).unwrap();
        assert!(quiet.level_db > loud.level_db);
        for preset in [&quiet, &loud] {
            let after = loudness(preset, &library, 48_000).unwrap();
            assert!((after - MATCHED_LOUDNESS).abs() < 1.0, "{after}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_factory_tones_already_sit_at_the_matched_level() {
        let (library, directory) = library("factory");
        for tone in crate::factory_tones() {
            let level = loudness(&tone, &library, 48_000).unwrap();
            assert!(
                (level - MATCHED_LOUDNESS).abs() < 2.0,
                "{} measures {level}",
                tone.name
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
