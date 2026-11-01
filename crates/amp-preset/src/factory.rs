use crate::{BlockPreset, Library, LibraryError, Preset, SCHEMA_VERSION, library::write_json};
use amp_dsp::BlockKind;
use std::{fs, io::ErrorKind};

fn block(kind: BlockKind, enabled: bool, params: &[(&str, f32)]) -> BlockPreset {
    let mut block = BlockPreset::new(kind, enabled);
    for (id, value) in params {
        block.params.insert((*id).to_owned(), *value);
    }
    block
}

fn tone(name: &str, tags: &[&str], notes: &str, blocks: Vec<BlockPreset>) -> Preset {
    Preset {
        schema: SCHEMA_VERSION,
        name: name.to_owned(),
        author: "Amp GPUI".to_owned(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        notes: notes.to_owned(),
        blocks,
    }
}

fn amp(params: &[(&str, f32)]) -> BlockPreset {
    block(BlockKind::Tube, true, params)
}

fn cab(level: f32) -> BlockPreset {
    block(BlockKind::Cab, true, &[("level", level)])
}

/// The tones a new library starts with. They use only built-in blocks, so they
/// play with no captures installed and can be shared freely.
pub fn factory_tones() -> Vec<Preset> {
    use BlockKind::{Chorus, Compressor, Delay, Drive, Gate, Reverb};
    vec![
        tone(
            "Moss Clean",
            &["clean"],
            "A warm, even clean for chords and fingerstyle.",
            vec![
                block(
                    Compressor,
                    true,
                    &[("threshold", -28.0), ("ratio", 3.0), ("level", 4.0)],
                ),
                amp(&[
                    ("gain", 1.6),
                    ("bass", 5.5),
                    ("mid", 4.5),
                    ("treble", 5.5),
                    ("presence", 3.0),
                    ("master", 5.9),
                ]),
                cab(0.0),
                block(
                    Reverb,
                    true,
                    &[("size", 45.0), ("damping", 45.0), ("mix", 16.0)],
                ),
            ],
        ),
        tone(
            "Glass Creek",
            &["clean", "chorus"],
            "Bright clean with a slow shimmer and a short slap.",
            vec![
                block(
                    Compressor,
                    true,
                    &[("threshold", -30.0), ("ratio", 4.0), ("level", 5.0)],
                ),
                amp(&[
                    ("gain", 1.2),
                    ("bass", 4.5),
                    ("mid", 4.5),
                    ("treble", 7.0),
                    ("presence", 5.0),
                    ("master", 6.4),
                ]),
                cab(0.0),
                block(
                    Chorus,
                    true,
                    &[("rate", 0.7), ("depth", 45.0), ("mix", 40.0)],
                ),
                block(
                    Delay,
                    true,
                    &[("time", 340.0), ("feedback", 22.0), ("mix", 16.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 55.0), ("damping", 35.0), ("mix", 20.0)],
                ),
            ],
        ),
        tone(
            "Fern Crunch",
            &["crunch"],
            "Edge of breakup. Dig in to bite, or ease off to clean up.",
            vec![
                amp(&[
                    ("gain", 4.6),
                    ("bass", 5.0),
                    ("mid", 6.0),
                    ("treble", 5.5),
                    ("presence", 3.5),
                    ("master", 4.0),
                ]),
                cab(0.0),
                block(
                    Reverb,
                    true,
                    &[("size", 35.0), ("damping", 50.0), ("mix", 10.0)],
                ),
            ],
        ),
        tone(
            "Bark Rhythm",
            &["high gain", "rhythm"],
            "Tight palm mutes. A drive pedal trims the lows going into the amp.",
            vec![
                block(Drive, true, &[("drive", 4.0), ("level", 0.0)]),
                amp(&[
                    ("gain", 7.2),
                    ("bass", 5.5),
                    ("mid", 3.6),
                    ("treble", 6.0),
                    ("presence", 4.0),
                    ("master", 2.8),
                ]),
                block(Gate, true, &[("threshold", -58.0), ("release", 80.0)]),
                cab(0.0),
            ],
        ),
        tone(
            "Peat Lead",
            &["high gain", "lead"],
            "Singing mids for solos, with a delay trailing behind the notes.",
            vec![
                block(Drive, true, &[("drive", 8.0), ("level", -2.0)]),
                amp(&[
                    ("gain", 8.0),
                    ("bass", 4.8),
                    ("mid", 6.6),
                    ("treble", 5.2),
                    ("presence", 3.0),
                    ("master", 2.6),
                ]),
                block(Gate, true, &[("threshold", -62.0), ("release", 160.0)]),
                cab(0.0),
                block(
                    Delay,
                    true,
                    &[("time", 420.0), ("feedback", 32.0), ("mix", 18.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 50.0), ("damping", 50.0), ("mix", 14.0)],
                ),
            ],
        ),
        tone(
            "Ember Fuzz",
            &["fuzz"],
            "A woolly wall of sustain, driving a mostly clean amp.",
            vec![
                block(Drive, true, &[("drive", 34.0), ("level", -8.0)]),
                amp(&[
                    ("gain", 3.0),
                    ("bass", 6.0),
                    ("mid", 4.5),
                    ("treble", 4.5),
                    ("presence", 2.0),
                    ("master", 1.7),
                ]),
                cab(0.0),
                block(
                    Reverb,
                    true,
                    &[("size", 40.0), ("damping", 60.0), ("mix", 10.0)],
                ),
            ],
        ),
        tone(
            "Hollow Fog",
            &["ambient"],
            "Long washes for swells and slow chords.",
            vec![
                block(
                    Compressor,
                    true,
                    &[("threshold", -32.0), ("ratio", 4.0), ("level", 5.0)],
                ),
                amp(&[
                    ("gain", 2.0),
                    ("bass", 5.0),
                    ("mid", 4.0),
                    ("treble", 5.0),
                    ("presence", 2.0),
                    ("master", 6.6),
                ]),
                cab(0.0),
                block(
                    Chorus,
                    true,
                    &[("rate", 0.3), ("depth", 55.0), ("mix", 30.0)],
                ),
                block(
                    Delay,
                    true,
                    &[("time", 520.0), ("feedback", 48.0), ("mix", 30.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 90.0), ("damping", 30.0), ("mix", 42.0)],
                ),
            ],
        ),
        tone(
            "Basalt Drive",
            &["crunch", "rhythm"],
            "Medium gain with weight, for big open chords and riffs.",
            vec![
                block(Drive, true, &[("drive", 6.0), ("level", -3.0)]),
                amp(&[
                    ("gain", 5.8),
                    ("bass", 6.0),
                    ("mid", 5.0),
                    ("treble", 5.0),
                    ("presence", 3.5),
                    ("master", 3.0),
                ]),
                block(Gate, true, &[("threshold", -64.0), ("release", 140.0)]),
                cab(0.0),
                block(
                    Reverb,
                    true,
                    &[("size", 40.0), ("damping", 50.0), ("mix", 8.0)],
                ),
            ],
        ),
        tone(
            "Silver Hollow",
            &["clean", "blues"],
            "A glassy, round clean with light compression and a plate-style reverb.",
            vec![
                block(
                    Compressor,
                    true,
                    &[
                        ("threshold", -26.0),
                        ("ratio", 2.5),
                        ("release", 220.0),
                        ("level", 3.0),
                    ],
                ),
                amp(&[
                    ("gain", 2.6),
                    ("bass", 5.5),
                    ("mid", 4.4),
                    ("treble", 6.4),
                    ("presence", 4.5),
                    ("master", 5.5),
                ]),
                cab(0.0),
                block(
                    Reverb,
                    true,
                    &[("size", 50.0), ("damping", 30.0), ("mix", 20.0)],
                ),
            ],
        ),
        tone(
            "Slow Thaw",
            &["blues", "edge of breakup"],
            "Warm edge of breakup, with a light drive left on as a gentle push.",
            vec![
                block(
                    Compressor,
                    true,
                    &[
                        ("threshold", -24.0),
                        ("ratio", 2.0),
                        ("release", 240.0),
                        ("level", 2.0),
                    ],
                ),
                block(Drive, true, &[("drive", 6.0), ("level", -1.0)]),
                amp(&[
                    ("gain", 3.6),
                    ("bass", 5.2),
                    ("mid", 5.4),
                    ("treble", 5.8),
                    ("presence", 3.5),
                    ("master", 3.3),
                ]),
                cab(0.0),
                block(
                    Delay,
                    true,
                    &[("time", 300.0), ("feedback", 15.0), ("mix", 10.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 45.0), ("damping", 40.0), ("mix", 18.0)],
                ),
            ],
        ),
        tone(
            "Amber Lead",
            &["blues", "lead"],
            "A vocal lead with a mid push, for long bends over a soft delay.",
            vec![
                block(
                    Compressor,
                    true,
                    &[("threshold", -24.0), ("ratio", 2.5), ("level", 2.0)],
                ),
                block(Drive, true, &[("drive", 10.0), ("level", -3.0)]),
                amp(&[
                    ("gain", 5.0),
                    ("bass", 4.8),
                    ("mid", 6.2),
                    ("treble", 5.6),
                    ("presence", 3.5),
                    ("master", 2.4),
                ]),
                cab(0.0),
                block(
                    Delay,
                    true,
                    &[("time", 380.0), ("feedback", 24.0), ("mix", 16.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 50.0), ("damping", 40.0), ("mix", 18.0)],
                ),
            ],
        ),
        tone(
            "Sunlit Neo",
            &["clean", "neo-soul"],
            "A shimmering neo-soul clean for chord melodies and fills.",
            vec![
                block(
                    Compressor,
                    true,
                    &[
                        ("threshold", -30.0),
                        ("ratio", 4.0),
                        ("release", 180.0),
                        ("level", 5.0),
                    ],
                ),
                amp(&[
                    ("gain", 1.4),
                    ("bass", 4.6),
                    ("mid", 4.2),
                    ("treble", 6.6),
                    ("presence", 4.5),
                    ("master", 6.3),
                ]),
                cab(0.0),
                block(
                    Chorus,
                    true,
                    &[("rate", 0.9), ("depth", 30.0), ("mix", 28.0)],
                ),
                block(
                    Delay,
                    true,
                    &[("time", 250.0), ("feedback", 18.0), ("mix", 12.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 60.0), ("damping", 30.0), ("mix", 24.0)],
                ),
            ],
        ),
        tone(
            "Velvet Pocket",
            &["clean", "neo-soul", "rhythm"],
            "A tight, percussive clean for funky rhythm and double-stops.",
            vec![
                block(
                    Compressor,
                    true,
                    &[
                        ("threshold", -32.0),
                        ("ratio", 5.0),
                        ("release", 120.0),
                        ("level", 6.0),
                    ],
                ),
                amp(&[
                    ("gain", 1.8),
                    ("bass", 4.4),
                    ("mid", 5.0),
                    ("treble", 5.4),
                    ("presence", 3.5),
                    ("master", 6.0),
                ]),
                cab(0.0),
                block(
                    Reverb,
                    true,
                    &[("size", 30.0), ("damping", 45.0), ("mix", 12.0)],
                ),
            ],
        ),
        tone(
            "Dew Line",
            &["neo-soul", "lead"],
            "A singing, airy lead with a light chorus and a lush tail.",
            vec![
                block(
                    Compressor,
                    true,
                    &[("threshold", -28.0), ("ratio", 3.0), ("level", 3.0)],
                ),
                block(Drive, true, &[("drive", 8.0), ("level", -2.0)]),
                amp(&[
                    ("gain", 4.2),
                    ("bass", 4.6),
                    ("mid", 5.6),
                    ("treble", 6.0),
                    ("presence", 4.0),
                    ("master", 3.2),
                ]),
                cab(0.0),
                block(
                    Chorus,
                    true,
                    &[("rate", 0.6), ("depth", 20.0), ("mix", 16.0)],
                ),
                block(
                    Delay,
                    true,
                    &[("time", 430.0), ("feedback", 26.0), ("mix", 18.0)],
                ),
                block(
                    Reverb,
                    true,
                    &[("size", 60.0), ("damping", 35.0), ("mix", 26.0)],
                ),
            ],
        ),
    ]
}

/// Remembers which factory tones a library has been given, so a tone added in a
/// later version arrives once, and one the user removed stays gone.
const SEEDED: &str = "factory.json";

impl Library {
    /// Gives a new library the factory set, and an existing factory library any
    /// tones added since. A library of someone's own tones is left as it is.
    pub fn seed_factory_tones(&self) -> Result<(), LibraryError> {
        let (entries, errors) = self.presets()?;
        if !errors.is_empty() {
            return Ok(());
        }
        let tones = factory_tones();
        let path = self.root().join(SEEDED);
        let existing = |name: &str| entries.iter().any(|entry| entry.preset.name == name);
        let mut seeded: Vec<String> = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|source| LibraryError::Json {
                path: path.clone(),
                source,
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if !entries.is_empty() && !tones.iter().any(|tone| existing(&tone.name)) {
                    return Ok(());
                }
                Vec::new()
            }
            Err(error) => return Err(error.into()),
        };
        for tone in tones {
            if !seeded.contains(&tone.name) {
                if !existing(&tone.name) {
                    self.save_preset(&tone)?;
                }
                seeded.push(tone.name);
            }
        }
        write_json(&path, &seeded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_chain;
    use amp_dsp::Chain;

    const RATE: usize = 48_000;

    /// A plucked string, so tones are judged on something shaped like a guitar:
    /// a sharp attack, then a decay rich in harmonics. Two notes ring together.
    fn pluck() -> Vec<f32> {
        let mut noise = 0x2545_f491_u32;
        let mut string = |period: usize| -> Vec<f32> {
            let mut line: Vec<f32> = (0..period)
                .map(|_| {
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    noise as f32 / u32::MAX as f32 - 0.5
                })
                .collect();
            let mut previous = 0.0;
            (0..RATE * 2)
                .map(|index| {
                    let slot = index % period;
                    let sample = line[slot];
                    line[slot] = (sample + previous) * 0.4985;
                    previous = sample;
                    sample
                })
                .collect()
        };
        let low = string(RATE / 110);
        let high = string(RATE / 165);
        low.iter().zip(&high).map(|(a, b)| (a + b) * 0.4).collect()
    }

    fn render(mut chain: Chain) -> Vec<f32> {
        let mut audio = pluck();
        for block in audio.chunks_mut(64) {
            chain.process(block);
        }
        audio
    }

    fn rms_db(samples: &[f32]) -> f32 {
        let power = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
        10.0 * power.max(1e-12).log10()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
    }

    fn rendered() -> Vec<(String, Vec<f32>)> {
        let directory = std::env::temp_dir().join(format!("amp-factory-{}", std::process::id()));
        let library = Library::open(&directory).unwrap();
        let tones = factory_tones()
            .into_iter()
            .map(|tone| {
                let built = build_chain(&tone, &library, RATE as u32).unwrap();
                (tone.name, render(built.chain))
            })
            .collect();
        std::fs::remove_dir_all(directory).unwrap();
        tones
    }

    #[test]
    fn every_tone_plays_clean_of_clipping_and_close_in_level() {
        let tones = rendered();
        let levels: Vec<f32> = tones
            .iter()
            .map(|(name, audio)| {
                let level = rms_db(&audio[..RATE]);
                println!("{name:14} {level:6.1} dB rms  {:.2} peak", peak(audio));
                level
            })
            .collect();
        for (name, audio) in &tones {
            assert!(audio.iter().all(|s| s.is_finite()), "{name} went unstable");
            assert!(peak(audio) < 1.0, "{name} clips at {}", peak(audio));
        }
        let loudest = levels.iter().copied().fold(f32::MIN, f32::max);
        let quietest = levels.iter().copied().fold(f32::MAX, f32::min);
        assert!(loudest - quietest < 6.0, "{quietest} to {loudest} dB");
    }

    #[test]
    fn tone_names_are_unique_and_every_tone_saves() {
        let directory = std::env::temp_dir().join(format!("amp-seed-{}", std::process::id()));
        let library = Library::open(&directory).unwrap();
        library.seed_factory_tones().unwrap();
        assert_eq!(library.presets().unwrap().0.len(), factory_tones().len());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_factory_library_gets_new_tones_once() {
        let directory = std::env::temp_dir().join(format!("amp-seed-new-{}", std::process::id()));
        let library = Library::open(&directory).unwrap();
        library.save_preset(&factory_tones()[0]).unwrap();

        library.seed_factory_tones().unwrap();
        let count = library.presets().unwrap().0.len();
        assert_eq!(count, factory_tones().len());

        let removed = directory.join("presets").join("silver-hollow.json");
        fs::remove_file(removed).unwrap();
        library.seed_factory_tones().unwrap();
        assert_eq!(library.presets().unwrap().0.len(), count - 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn seeding_leaves_a_library_with_tones_alone() {
        let directory = std::env::temp_dir().join(format!("amp-seed-kept-{}", std::process::id()));
        let library = Library::open(&directory).unwrap();
        library.save_preset(&Preset::starter("Mine")).unwrap();
        library.seed_factory_tones().unwrap();
        assert_eq!(library.presets().unwrap().0.len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
