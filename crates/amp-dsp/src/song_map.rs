//! The rest of a song around its melody: where the beats fall, what the chords
//! are, and what key it's in. Like the melody, this reads a whole song on a
//! background thread, and uses plain signal processing with no trained model.
//!
//! Beats come from Ellis's dynamic programming tracker: find the tempo, then the
//! run of beats that lands on strong onsets while keeping to it. Chords are chroma
//! between beats matched against chord shapes, with a Viterbi pass so they don't
//! flicker. Key is the summed chroma against the Krumhansl-Kessler profiles.

use crate::{
    MelodyNote,
    listen::fft,
    melody::{ANALYSIS_RATE, melody_notes, resample},
};
use std::f32::consts::PI;

const ONSET_WINDOW: usize = 512;
/// An onset reading every 10 ms.
const ONSET_HOP: usize = 120;
const ONSET_SECONDS: f32 = ONSET_HOP as f32 / ANALYSIS_RATE as f32;
/// The tempos considered, as frames per beat: 40 to 240 BPM.
const BEAT_FRAMES: (usize, usize) = (25, 150);
/// Tempos near this many frames are expected, as most music sits near 120 BPM.
const USUAL_BEAT: f32 = 50.0;
/// How far from `USUAL_BEAT` a tempo can sit without losing much, in octaves.
const TEMPO_SPREAD: f32 = 1.4;
/// How strictly beats keep to the tempo, against landing on an onset.
const BEAT_TIGHTNESS: f32 = 100.0;
const CHROMA_WINDOW: usize = 4_096;
const CHROMA_FFT: usize = 8_192;
/// A chroma reading every 100 ms.
const CHROMA_HOP: usize = 1_200;
const CHROMA_RANGE: (f32, f32) = (40.0, 1_760.0);
/// Below this the bass plays the root, so those bins count for more. A melody
/// on top of a chord otherwise pulls it toward its relative minor or major.
const BASS_TOP: f32 = 220.0;
const BASS_WEIGHT: f32 = 3.0;
/// Splits a song into this many seconds when no beat could be found.
const CHORD_STEP: f32 = 0.5;
/// How much a chord change has to win by, against a typical match near 1.
const CHORD_CHANGE: f32 = 0.35;
/// What "no chord" scores, so silence and drum breaks don't get names.
const NO_CHORD: f32 = 0.55;
const CHORD_STATES: usize = 24;
const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// A chord over a stretch of the song, with times in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct SongChord {
    pub start: f32,
    pub end: f32,
    pub name: String,
}

/// A song's key, with `root` as a semitone above C.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongKey {
    pub root: usize,
    pub minor: bool,
}

impl SongKey {
    pub fn name(&self) -> String {
        let mode = if self.minor { "minor" } else { "major" };
        format!("{} {mode}", NOTES[self.root])
    }
}

/// Everything read from a song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongMap {
    pub notes: Vec<MelodyNote>,
    /// In seconds.
    pub beats: Vec<f32>,
    pub chords: Vec<SongChord>,
    pub key: Option<SongKey>,
}

/// How strongly something new starts in each 10 ms of `audio`.
fn onsets(audio: &[f32]) -> Vec<f32> {
    let shape: Vec<f32> = (0..ONSET_WINDOW)
        .map(|index| (PI * index as f32 / ONSET_WINDOW as f32).sin().powi(2))
        .collect();
    let mut re = vec![0.0; ONSET_WINDOW];
    let mut im = vec![0.0; ONSET_WINDOW];
    let mut before = vec![0.0_f32; ONSET_WINDOW / 2];
    let mut found = Vec::with_capacity(audio.len() / ONSET_HOP);
    for part in audio.windows(ONSET_WINDOW).step_by(ONSET_HOP) {
        for ((re, sample), shape) in re.iter_mut().zip(part).zip(&shape) {
            *re = sample * shape;
        }
        im.fill(0.0);
        fft(&mut re, &mut im);
        let mut rise = 0.0;
        for (bin, last) in before.iter_mut().enumerate() {
            let level = (1.0 + 100.0 * re[bin].hypot(im[bin])).ln();
            rise += (level - *last).max(0.0);
            *last = level;
        }
        found.push(rise);
    }
    found
}

fn deviation(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    let spread = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>();
    (spread / values.len().max(1) as f32).sqrt()
}

/// The likeliest beat length in onset frames, from how the onsets repeat.
fn beat_length(onsets: &[f32]) -> f32 {
    let (low, high) = BEAT_FRAMES;
    (low..=high.min(onsets.len().saturating_sub(1)))
        .map(|lag| {
            let matched: f32 = onsets
                .iter()
                .zip(&onsets[lag..])
                .map(|(first, second)| first * second)
                .sum();
            let octaves = (lag as f32 / USUAL_BEAT).log2() / TEMPO_SPREAD;
            (
                lag,
                matched / onsets.len() as f32 * (-0.5 * octaves * octaves).exp(),
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(USUAL_BEAT, |(lag, _)| lag as f32)
}

/// Picks the run of beats through `onsets` that best lands on them at `length`.
fn follow_beats(onsets: &[f32], length: f32) -> Vec<f32> {
    let frames = onsets.len();
    let mut scores = onsets.to_vec();
    let mut before = vec![usize::MAX; frames];
    let reach = (length * 2.0) as usize;
    for frame in 1..frames {
        let earliest = frame.saturating_sub(reach);
        let latest = frame.saturating_sub((length / 2.0) as usize);
        let best = (earliest..latest)
            .map(|from| {
                let stretch = ((frame - from) as f32 / length).ln();
                (from, scores[from] - BEAT_TIGHTNESS * stretch * stretch)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((from, score)) = best
            && score > 0.0
        {
            scores[frame] += score;
            before[frame] = from;
        }
    }
    // The last strong ending, as a fade-out shouldn't invent beats past the music.
    let top = scores.iter().copied().fold(0.0, f32::max);
    let tail = frames.saturating_sub(reach);
    let mut at = (tail..frames)
        .filter(|frame| scores[*frame] > top * 0.5)
        .max_by(|a, b| scores[*a].total_cmp(&scores[*b]));
    let mut beats = Vec::new();
    while let Some(frame) = at {
        beats.push(frame as f32 * ONSET_SECONDS + ONSET_WINDOW as f32 / 2.0 / ANALYSIS_RATE as f32);
        at = (before[frame] != usize::MAX).then_some(before[frame]);
    }
    beats.reverse();
    beats
}

/// Finds the beats of `audio`, in seconds.
fn find_beats(audio: &[f32]) -> Vec<f32> {
    let mut strengths = onsets(audio);
    let spread = deviation(&strengths);
    if spread <= 0.0 {
        return Vec::new();
    }
    for value in &mut strengths {
        *value /= spread;
    }
    follow_beats(&strengths, beat_length(&strengths))
}

/// The twelve pitch classes' strengths in each 100 ms of `audio`, from C up,
/// with the recording's own `tune` in semitones taken out.
fn chroma(audio: &[f32], tune: f32) -> Vec<[f32; 12]> {
    let shape: Vec<f32> = (0..CHROMA_WINDOW)
        .map(|index| (PI * index as f32 / CHROMA_WINDOW as f32).sin().powi(2))
        .collect();
    let hz_per_bin = ANALYSIS_RATE as f32 / CHROMA_FFT as f32;
    let first = (CHROMA_RANGE.0 / hz_per_bin) as usize;
    let last = (CHROMA_RANGE.1 / hz_per_bin) as usize;
    let classes: Vec<(usize, f32)> = (first..last)
        .map(|bin| {
            let midi = 69.0 + 12.0 * (bin as f32 * hz_per_bin / 440.0).log2() - tune;
            let nearest = midi.round();
            let hertz = bin as f32 * hz_per_bin;
            let bass = if hertz < BASS_TOP { BASS_WEIGHT } else { 1.0 };
            let weight = (PI * (midi - nearest)).cos().powi(2) * bass;
            ((nearest as i32).rem_euclid(12) as usize, weight)
        })
        .collect();
    let mut re = vec![0.0; CHROMA_FFT];
    let mut im = vec![0.0; CHROMA_FFT];
    audio
        .windows(CHROMA_WINDOW)
        .step_by(CHROMA_HOP)
        .map(|part| {
            re.fill(0.0);
            im.fill(0.0);
            for ((re, sample), shape) in re.iter_mut().zip(part).zip(&shape) {
                *re = sample * shape;
            }
            fft(&mut re, &mut im);
            let mut frame = [0.0; 12];
            for (offset, (class, weight)) in classes.iter().enumerate() {
                let bin = first + offset;
                frame[*class] += re[bin].hypot(im[bin]).sqrt() * weight;
            }
            frame
        })
        .collect()
}

fn normalized(mut values: [f32; 12]) -> [f32; 12] {
    let length = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        for value in &mut values {
            *value /= length;
        }
    }
    values
}

/// A chord shape to match against, with state 0 to 11 major and 12 to 23 minor.
fn chord_shape(state: usize) -> [f32; 12] {
    let root = state % 12;
    let third = if state < 12 { 4 } else { 3 };
    let mut shape = [0.0; 12];
    shape[root] = 1.0;
    shape[(root + third) % 12] = 0.8;
    shape[(root + 7) % 12] = 0.9;
    normalized(shape)
}

fn chord_label(state: usize) -> String {
    let suffix = if state < 12 { "" } else { "m" };
    format!("{}{suffix}", NOTES[state % 12])
}

/// The chroma of each stretch between `edges`, in seconds.
fn chroma_between(frames: &[[f32; 12]], edges: &[f32]) -> Vec<[f32; 12]> {
    let seconds = CHROMA_HOP as f32 / ANALYSIS_RATE as f32;
    let center = CHROMA_WINDOW as f32 / 2.0 / ANALYSIS_RATE as f32;
    let frame_at = |time: f32| (((time - center) / seconds).max(0.0) as usize).min(frames.len());
    edges
        .windows(2)
        .map(|pair| {
            let (from, to) = (frame_at(pair[0]), frame_at(pair[1]));
            let mut sum = [0.0; 12];
            for frame in &frames[from..to.max(from + 1).min(frames.len())] {
                for (total, value) in sum.iter_mut().zip(frame) {
                    *total += value;
                }
            }
            normalized(sum)
        })
        .collect()
}

/// Picks a chord for each stretch, favoring staying put. The last state is no chord.
fn follow_chords(stretches: &[[f32; 12]]) -> Vec<usize> {
    let shapes: Vec<[f32; 12]> = (0..CHORD_STATES).map(chord_shape).collect();
    let states = CHORD_STATES + 1;
    let mut scores = vec![0.0_f32; states];
    let mut from = vec![0_u8; stretches.len() * states];
    for (index, stretch) in stretches.iter().enumerate() {
        let (best_at, best) = scores
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, 0.0));
        let links = &mut from[index * states..(index + 1) * states];
        let mut next = vec![0.0; states];
        for state in 0..states {
            let fit = match shapes.get(state) {
                Some(shape) => shape.iter().zip(stretch).map(|(a, b)| a * b).sum(),
                None => NO_CHORD,
            };
            let (origin, carried) = if scores[state] >= best - CHORD_CHANGE {
                (state, scores[state])
            } else {
                (best_at, best - CHORD_CHANGE)
            };
            next[state] = carried + fit;
            links[state] = origin as u8;
        }
        scores = next;
    }
    let mut state = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(CHORD_STATES, |(state, _)| state);
    let mut path = vec![CHORD_STATES; stretches.len()];
    for index in (0..stretches.len()).rev() {
        path[index] = state;
        state = usize::from(from[index * states + state]);
    }
    path
}

/// Names the chords over a song, joining repeats of one chord.
fn find_chords(frames: &[[f32; 12]], beats: &[f32], length: f32) -> Vec<SongChord> {
    let mut edges = beats.to_vec();
    if edges.len() < 4 {
        edges = (0..(length / CHORD_STEP) as usize)
            .map(|step| step as f32 * CHORD_STEP)
            .collect();
    }
    let path = follow_chords(&chroma_between(frames, &edges));
    let mut chords: Vec<SongChord> = Vec::new();
    let mut last = CHORD_STATES;
    for (pair, state) in edges.windows(2).zip(path) {
        if state == CHORD_STATES {
            last = state;
            continue;
        }
        match chords.last_mut() {
            Some(chord) if state == last => chord.end = pair[1],
            _ => chords.push(SongChord {
                start: pair[0],
                end: pair[1],
                name: chord_label(state),
            }),
        }
        last = state;
    }
    chords
}

/// The key whose profile best fits how much each pitch class sounds.
fn find_key(frames: &[[f32; 12]]) -> Option<SongKey> {
    let mut sum = [0.0; 12];
    for frame in frames {
        for (total, value) in sum.iter_mut().zip(frame) {
            *total += value;
        }
    }
    if sum.iter().all(|value| *value <= 0.0) {
        return None;
    }
    let center = |values: [f32; 12]| {
        let mean = values.iter().sum::<f32>() / 12.0;
        normalized(values.map(|value| value - mean))
    };
    let heard = center(sum);
    let keys = (0..12).flat_map(|root| [(root, false), (root, true)]);
    keys.map(|(root, minor)| {
        let profile = center(if minor { MINOR_PROFILE } else { MAJOR_PROFILE });
        let fit: f32 = (0..12)
            .map(|class| heard[(class + root) % 12] * profile[class])
            .sum();
        (SongKey { root, minor }, fit)
    })
    .max_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(key, _)| key)
}

/// Reads mono `samples` for everything in a [`SongMap`]. `progress` hears a
/// share from 0 to 1.
pub fn read_song_map(samples: &[f32], rate: u32, mut progress: impl FnMut(f32)) -> SongMap {
    let audio = resample(samples, rate);
    let (notes, tune) = melody_notes(&audio, |share| progress(share * 0.8));
    if audio.len() < CHROMA_WINDOW {
        return SongMap::default();
    }
    let beats = find_beats(&audio);
    progress(0.9);
    let frames = chroma(&audio, tune);
    let length = audio.len() as f32 / ANALYSIS_RATE as f32;
    let chords = find_chords(&frames, &beats, length);
    let key = find_key(&frames);
    progress(1.0);
    SongMap {
        notes,
        beats,
        chords,
        key,
    }
}

#[cfg(test)]
mod tests {
    use super::{SongKey, chord_label, chord_shape, find_key, follow_chords};

    #[test]
    fn chord_states_name_major_then_minor() {
        assert_eq!(chord_label(9), "A");
        assert_eq!(chord_label(21), "Am");
    }

    #[test]
    fn a_stray_beat_does_not_change_the_chord() {
        // C major, one beat of something else, then C major again.
        let mut stretches = vec![chord_shape(0); 9];
        stretches[4] = chord_shape(19);
        let path = follow_chords(&stretches);
        assert!(path.iter().all(|state| *state == 0), "{path:?}");
    }

    #[test]
    fn a_held_new_chord_takes_over() {
        let mut stretches = vec![chord_shape(0); 8];
        stretches.extend(vec![chord_shape(21); 8]);
        let path = follow_chords(&stretches);
        assert_eq!(path[1], 0);
        assert_eq!(path[14], 21);
    }

    #[test]
    fn a_major_scale_reads_as_its_key() {
        // Tonic and fifth lean heaviest, as they do in a real tune.
        let mut frame = [0.0; 12];
        for (step, weight) in [
            (0, 3.0),
            (2, 1.0),
            (4, 2.0),
            (5, 1.0),
            (7, 2.5),
            (9, 1.0),
            (11, 1.0),
        ] {
            frame[(7 + step) % 12] = weight;
        }
        let key = find_key(&[frame]);
        assert_eq!(
            key,
            Some(SongKey {
                root: 7,
                minor: false
            })
        );
        assert_eq!(key.map(|key| key.name()).as_deref(), Some("G major"));
    }

    #[test]
    fn a_minor_scale_reads_as_its_key() {
        let mut frame = [0.0; 12];
        for (step, weight) in [
            (0, 3.0),
            (2, 1.0),
            (3, 2.0),
            (5, 1.0),
            (7, 2.5),
            (8, 1.0),
            (10, 1.0),
        ] {
            frame[(9 + step) % 12] = weight;
        }
        assert_eq!(
            find_key(&[frame]),
            Some(SongKey {
                root: 9,
                minor: true
            })
        );
    }

    #[test]
    fn silence_has_no_key() {
        assert_eq!(find_key(&[[0.0; 12]; 4]), None);
    }
}
