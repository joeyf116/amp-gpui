/// The lowest pitch worth looking for: a low E tuned down a fifth.
const LOWEST_HZ: f32 = 55.0;
const HIGHEST_HZ: f32 = 1_400.0;
/// How clearly a repeat has to stand out for the pitch to be trusted.
const CONFIDENCE: f32 = 0.15;
const SILENCE: f32 = 1e-3;
const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A tuner reading: the closest note and how far off it the string is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pitch {
    pub note: &'static str,
    pub octave: i32,
    pub cents: f32,
    pub hertz: f32,
}

impl Pitch {
    pub fn from_hertz(hertz: f32) -> Self {
        let semitones = 12.0 * (hertz / 440.0).log2() + 69.0;
        let nearest = semitones.round();
        let midi = nearest as i32;
        Self {
            note: NOTES[midi.rem_euclid(12) as usize],
            octave: midi.div_euclid(12) - 1,
            cents: (semitones - nearest) * 100.0,
            hertz,
        }
    }
}

/// Finds the pitch of `samples` using the YIN method, which compares the signal
/// with delayed copies of itself. Guitars need this over a plain spectrum,
/// since the strongest harmonic of a low string is often not its fundamental.
pub fn detect_pitch(samples: &[f32], sample_rate: u32) -> Option<Pitch> {
    let rate = sample_rate as f32;
    let shortest = (rate / HIGHEST_HZ) as usize;
    let longest = (rate / LOWEST_HZ) as usize;
    let window = longest * 2;
    if samples.len() < window + longest {
        return None;
    }
    let samples = &samples[samples.len() - window - longest..];
    if samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs())) < SILENCE {
        return None;
    }

    let mut differences = vec![1.0_f32; longest + 1];
    let mut running = 0.0;
    for lag in 1..=longest {
        let difference: f32 = (0..window)
            .map(|index| {
                let delta = samples[index] - samples[index + lag];
                delta * delta
            })
            .sum();
        running += difference;
        differences[lag] = if running > 0.0 {
            difference * lag as f32 / running
        } else {
            1.0
        };
    }

    let mut lag = shortest.max(2);
    while lag < longest {
        if differences[lag] < CONFIDENCE {
            while lag + 1 < longest && differences[lag + 1] < differences[lag] {
                lag += 1;
            }
            // Fit a curve through the dip to land between whole samples.
            let (before, at, after) =
                (differences[lag - 1], differences[lag], differences[lag + 1]);
            let curve = before + after - 2.0 * at;
            let offset = if curve.abs() > 1e-9 {
                0.5 * (before - after) / curve
            } else {
                0.0
            };
            return Some(Pitch::from_hertz(rate / (lag as f32 + offset)));
        }
        lag += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: u32 = 48_000;

    /// A note with strong harmonics, as a guitar string has.
    fn string(hertz: f32) -> Vec<f32> {
        (0..8_192)
            .map(|index| {
                let time = index as f32 / RATE as f32;
                (1..=6)
                    .map(|harmonic| {
                        let harmonic = harmonic as f32;
                        (TAU * hertz * harmonic * time).sin() / harmonic
                    })
                    .sum::<f32>()
                    * 0.2
            })
            .collect()
    }

    #[test]
    fn names_every_open_string_to_within_two_cents() {
        for (hertz, note, octave) in [
            (82.41, "E", 2),
            (110.0, "A", 2),
            (146.83, "D", 3),
            (196.0, "G", 3),
            (246.94, "B", 3),
            (329.63, "E", 4),
        ] {
            let pitch = detect_pitch(&string(hertz), RATE).expect("a clear note");
            assert_eq!((pitch.note, pitch.octave), (note, octave));
            assert!(pitch.cents.abs() < 2.0, "{note}: {} cents", pitch.cents);
        }
    }

    #[test]
    fn reports_how_far_sharp_a_string_is() {
        let sharp = 110.0 * 2.0_f32.powf(20.0 / 1200.0);
        let pitch = detect_pitch(&string(sharp), RATE).unwrap();
        assert_eq!(pitch.note, "A");
        assert!((pitch.cents - 20.0).abs() < 2.0, "{} cents", pitch.cents);
    }

    #[test]
    fn stays_quiet_on_silence_and_noise() {
        assert!(detect_pitch(&vec![0.0; 8_192], RATE).is_none());
        let mut seed = 0x2545_f491_u32;
        let noise: Vec<f32> = (0..8_192)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as f32 / u32::MAX as f32 - 0.5
            })
            .collect();
        assert!(detect_pitch(&noise, RATE).is_none());
    }
}
