//! Hears what's being played: which notes are sounding, what chord they make,
//! and how stiff the string under a single note is. This runs on the UI thread
//! from the tuner's copy of the guitar, never on the audio thread.
//!
//! Notes are found by iterative harmonic cancellation: take the note whose
//! harmonics best explain the spectrum, remove what it accounts for, and repeat
//! until nothing strong is left.

use std::f32::consts::PI;

const WINDOW: usize = 16_384;
const FFT_SIZE: usize = 32_768;
const LOWEST_MIDI: i32 = 40;
const HIGHEST_MIDI: i32 = 88;
const MAX_NOTES: usize = 6;
const HARMONICS: usize = 12;
const TOP_HZ: f32 = 5_000.0;
/// How far a harmonic can sit from where it should, in semitones.
const HARMONIC_SLACK: f32 = 0.3;
/// A later note has to stand this tall against the first one found.
const NEXT_NOTE: f32 = 0.3;
/// Quieter than this, there's nothing to listen to.
const SILENCE_RMS: f32 = 5e-4;
/// Bins either side of a harmonic that cancelling removes.
const CANCEL_BINS: isize = 6;
/// Stiffness needs at least this many clean partials to fit a line.
const FIT_PARTIALS: usize = 6;
const FIT_HARMONICS: usize = 24;
/// The most inharmonic a guitar string can plausibly be.
const STIFFNESS_LIMIT: f32 = 0.004;

const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// Chord shapes as semitones above the root. Earlier entries win a tie.
const CHORDS: [(&str, &[i32]); 16] = [
    ("", &[0, 4, 7]),
    ("m", &[0, 3, 7]),
    ("5", &[0, 7]),
    ("7", &[0, 4, 7, 10]),
    ("m7", &[0, 3, 7, 10]),
    ("maj7", &[0, 4, 7, 11]),
    ("sus4", &[0, 5, 7]),
    ("sus2", &[0, 2, 7]),
    ("6", &[0, 4, 7, 9]),
    ("m6", &[0, 3, 7, 9]),
    ("add9", &[0, 2, 4, 7]),
    ("9", &[0, 2, 4, 7, 10]),
    ("dim", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("dim7", &[0, 3, 6, 9]),
    ("m7b5", &[0, 3, 6, 10]),
];

pub fn midi_hertz(midi: i32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69) as f32 / 12.0)
}

/// An in-place radix-2 FFT. `re` and `im` must be a power of two long.
pub(crate) fn fft(re: &mut [f32], im: &mut [f32]) {
    let size = re.len();
    let mut target = 0;
    for index in 1..size {
        let mut bit = size >> 1;
        while target & bit != 0 {
            target ^= bit;
            bit >>= 1;
        }
        target |= bit;
        if index < target {
            re.swap(index, target);
            im.swap(index, target);
        }
    }
    let mut span = 2;
    while span <= size {
        let angle = -2.0 * std::f64::consts::PI / span as f64;
        for start in (0..size).step_by(span) {
            for offset in 0..span / 2 {
                let turn = angle * offset as f64;
                let (w_re, w_im) = (turn.cos() as f32, turn.sin() as f32);
                let (near, far) = (start + offset, start + offset + span / 2);
                let t_re = re[far] * w_re - im[far] * w_im;
                let t_im = re[far] * w_im + im[far] * w_re;
                re[far] = re[near] - t_re;
                im[far] = im[near] - t_im;
                re[near] += t_re;
                im[near] += t_im;
            }
        }
        span <<= 1;
    }
}

/// The spectrum of the latest audio, up to the top of the useful range.
pub struct Spectrum {
    magnitude: Vec<f32>,
    hz_per_bin: f32,
}

impl Spectrum {
    /// Reads the end of `samples`. Returns `None` for too little audio, or silence.
    pub fn new(samples: &[f32], sample_rate: u32) -> Option<Self> {
        if samples.len() < WINDOW / 2 {
            return None;
        }
        let recent = &samples[samples.len().saturating_sub(WINDOW)..];
        let rms =
            (recent.iter().map(|sample| sample * sample).sum::<f32>() / recent.len() as f32).sqrt();
        if rms < SILENCE_RMS {
            return None;
        }
        let mut re = vec![0.0; FFT_SIZE];
        let mut im = vec![0.0; FFT_SIZE];
        for (index, sample) in recent.iter().enumerate() {
            let shape = (PI * index as f32 / recent.len() as f32).sin();
            re[index] = sample * shape * shape;
        }
        fft(&mut re, &mut im);
        let hz_per_bin = sample_rate as f32 / FFT_SIZE as f32;
        let top = ((TOP_HZ / hz_per_bin) as usize).min(FFT_SIZE / 2);
        let magnitude = re[..top]
            .iter()
            .zip(&im[..top])
            .map(|(re, im)| re.hypot(*im))
            .collect();
        Some(Self {
            magnitude,
            hz_per_bin,
        })
    }

    fn bin(&self, hertz: f32) -> f32 {
        hertz / self.hz_per_bin
    }

    /// The strongest bin near `hertz` in `values`.
    fn peak_near(values: &[f32], center: f32, reach: f32) -> Option<(usize, f32)> {
        let low = (center - reach).floor().max(1.0) as usize;
        let high = ((center + reach).ceil() as usize).min(values.len().checked_sub(2)?);
        (low..=high)
            .map(|bin| (bin, values[bin]))
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// What each harmonic of `midi` finds in `values`, with its bin.
    fn harmonics(&self, values: &[f32], midi: i32) -> Vec<(usize, f32)> {
        let root = midi_hertz(midi);
        let slack = 2.0_f32.powf(HARMONIC_SLACK / 12.0) - 1.0;
        (1..=HARMONICS)
            .map_while(|harmonic| {
                let hertz = root * harmonic as f32;
                (hertz < TOP_HZ).then(|| {
                    let reach = (self.bin(hertz) * slack).max(1.0);
                    Self::peak_near(values, self.bin(hertz), reach).unwrap_or((0, 0.0))
                })
            })
            .collect()
    }

    /// How much of the spectrum `midi` could explain. Low harmonics count most,
    /// so a note outranks anything built from its own overtones.
    fn salience(&self, values: &[f32], midi: i32) -> f32 {
        self.harmonics(values, midi)
            .iter()
            .enumerate()
            .map(|(index, (_, level))| level / ((index + 1) as f32).sqrt())
            .sum()
    }

    /// Removes what `midi` accounts for. A partial two notes share is only part
    /// `midi`'s, so each is capped at the level of its neighbours, since a real
    /// string's harmonics fall away smoothly.
    fn cancel(&self, values: &mut [f32], midi: i32) {
        let found = self.harmonics(values, midi);
        for (index, (bin, level)) in found.iter().enumerate() {
            let before = index.checked_sub(1).map(|at| found[at].1);
            let after = found.get(index + 1).map(|(_, level)| *level);
            let smooth = match (before, after) {
                (Some(before), Some(after)) => (before + after) * 0.5,
                _ => *level,
            };
            let amount = level.min(smooth.max(level * 0.5));
            for offset in -CANCEL_BINS..=CANCEL_BINS {
                let at = *bin as isize + offset;
                if let Some(value) = usize::try_from(at).ok().and_then(|at| values.get_mut(at)) {
                    let shape = 1.0 - (offset as f32 / (CANCEL_BINS + 1) as f32).abs();
                    *value = (*value - amount * shape).max(0.0);
                }
            }
        }
    }

    /// The notes sounding, as MIDI numbers from low to high.
    pub fn notes(&self) -> Vec<i32> {
        let mut values: Vec<f32> = self.magnitude.iter().map(|level| level.sqrt()).collect();
        let mut found: Vec<i32> = Vec::new();
        let mut first = 0.0;
        while found.len() < MAX_NOTES {
            let best = (LOWEST_MIDI..=HIGHEST_MIDI)
                .filter(|midi| !found.contains(midi))
                .map(|midi| (midi, self.salience(&values, midi)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let Some((midi, salience)) = best else {
                break;
            };
            if found.is_empty() {
                first = salience;
            } else if salience < first * NEXT_NOTE {
                break;
            }
            found.push(midi);
            self.cancel(&mut values, midi);
        }
        found.sort_unstable();
        found
    }

    /// The exact frequency of the peak near `hertz`, read between bins.
    fn partial(&self, hertz: f32, reach_hz: f32) -> Option<f32> {
        let reach = reach_hz / self.hz_per_bin;
        let (bin, level) = Self::peak_near(&self.magnitude, self.bin(hertz), reach)?;
        let (left, right) = (self.magnitude[bin - 1], self.magnitude[bin + 1]);
        if level <= left || level <= right {
            return None;
        }
        let (left, middle, right) = (left.ln(), level.ln(), right.ln());
        let shift = 0.5 * (left - right) / (left - 2.0 * middle + right);
        Some((bin as f32 + shift) * self.hz_per_bin)
    }

    /// The string's stiffness, as the inharmonicity B in
    /// `partial(n) = n * f0 * sqrt(1 + B * n * n)`. Thicker and shorter strings
    /// have more of it, which is what tells two places for a pitch apart.
    /// Returns `None` if too few partials can be read cleanly.
    pub fn stiffness(&self, hertz: f32) -> Option<f32> {
        let mut points: Vec<(f64, f64)> = Vec::new();
        let mut estimate = 0.0;
        for partial in 2..=FIT_HARMONICS {
            let number = partial as f32;
            let expected = number * hertz * (1.0 + estimate * number * number).sqrt();
            if expected >= TOP_HZ {
                break;
            }
            let Some(found) = self.partial(expected, hertz * 0.12) else {
                continue;
            };
            let per = f64::from(found) / f64::from(number);
            points.push((f64::from(number * number), per * per));
            // Later partials wander further, so refit to know where to look.
            if let Some(stiffness) = fit(&points) {
                estimate = stiffness.clamp(0.0, STIFFNESS_LIMIT);
            }
        }
        if points.len() < FIT_PARTIALS {
            return None;
        }
        fit(&points).filter(|stiffness| (0.0..STIFFNESS_LIMIT).contains(stiffness))
    }
}

/// Fits `y = a + b * x`, where `b / a` is the stiffness.
fn fit(points: &[(f64, f64)]) -> Option<f32> {
    if points.len() < 3 {
        return None;
    }
    let count = points.len() as f64;
    let mean_x = points.iter().map(|point| point.0).sum::<f64>() / count;
    let mean_y = points.iter().map(|point| point.1).sum::<f64>() / count;
    let spread: f64 = points.iter().map(|point| (point.0 - mean_x).powi(2)).sum();
    let shared: f64 = points
        .iter()
        .map(|point| (point.0 - mean_x) * (point.1 - mean_y))
        .sum();
    let slope = shared / spread;
    let base = mean_y - slope * mean_x;
    (base > 0.0).then(|| (slope / base) as f32)
}

/// Names the chord a set of notes makes, with the bass after a slash when it
/// isn't the root. Needs the whole chord: anything it can't match is `None`.
pub fn chord_name(notes: &[i32]) -> Option<String> {
    let bass = notes.first()?.rem_euclid(12);
    let mut classes: Vec<i32> = notes.iter().map(|note| note.rem_euclid(12)).collect();
    classes.sort_unstable();
    classes.dedup();
    if classes.len() < 2 {
        return None;
    }
    let mut roots = vec![bass];
    roots.extend(classes.iter().filter(|class| **class != bass));
    for root in roots {
        let mut steps: Vec<i32> = classes
            .iter()
            .map(|class| (class - root).rem_euclid(12))
            .collect();
        steps.sort_unstable();
        for (suffix, shape) in CHORDS {
            // Guitar voicings often drop the fifth from sevenths and up.
            let without_fifth: Vec<i32> = shape.iter().copied().filter(|step| *step != 7).collect();
            if steps == shape || (shape.len() >= 4 && steps == without_fifth) {
                let name = format!("{}{suffix}", NOTES[root as usize]);
                return Some(if root == bass {
                    name
                } else {
                    format!("{name}/{}", NOTES[bass as usize])
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{chord_name, fft};

    #[test]
    fn the_fft_finds_a_sine_in_its_bin() {
        let size = 64;
        let mut re: Vec<f32> = (0..size)
            .map(|index| (std::f32::consts::TAU * 5.0 * index as f32 / size as f32).sin())
            .collect();
        let mut im = vec![0.0; size];
        fft(&mut re, &mut im);
        let loudest = (1..size / 2)
            .max_by(|a, b| re[*a].hypot(im[*a]).total_cmp(&re[*b].hypot(im[*b])))
            .unwrap();
        assert_eq!(loudest, 5);
    }

    #[test]
    fn names_open_chords_by_their_notes() {
        assert_eq!(chord_name(&[40, 47, 52, 55, 59, 64]).as_deref(), Some("Em"));
        assert_eq!(chord_name(&[45, 52, 57, 61, 64]).as_deref(), Some("A"));
        assert_eq!(chord_name(&[48, 52, 55, 60, 64]).as_deref(), Some("C"));
        assert_eq!(chord_name(&[45, 52, 55, 60, 64]).as_deref(), Some("Am7"));
    }

    #[test]
    fn a_bass_that_isnt_the_root_goes_after_a_slash() {
        assert_eq!(chord_name(&[40, 48, 55, 60]).as_deref(), Some("C/E"));
    }

    #[test]
    fn a_seventh_without_its_fifth_still_names() {
        assert_eq!(chord_name(&[43, 53, 59]).as_deref(), Some("G7"));
    }

    #[test]
    fn a_power_chord_and_a_single_note() {
        assert_eq!(chord_name(&[40, 47, 52]).as_deref(), Some("E5"));
        assert_eq!(chord_name(&[40]), None);
        assert_eq!(chord_name(&[40, 52]), None);
    }

    #[test]
    fn an_unknown_cluster_gets_no_name() {
        assert_eq!(chord_name(&[60, 61, 62, 63]), None);
    }
}
