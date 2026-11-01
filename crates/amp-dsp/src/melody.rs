//! Finds the melody of a finished recording, as notes with times. This reads a
//! whole song on a background thread, so unlike the live ear in `listen` it can
//! look ahead and take the line that makes the most sense from start to finish.
//!
//! Each moment gets a pitch salience: how strongly the spectrum's peaks line up
//! as the harmonics of each pitch. A Viterbi pass then follows the path through
//! those moments that stays strong and doesn't jump around, which is what keeps
//! it on a lead line with a band playing underneath. The path becomes notes once
//! the recording's own tuning is taken out.

use crate::listen::fft;
use std::f32::consts::PI;

/// The rate the analysis runs at. Melody harmonics above 6 kHz add nothing.
pub(crate) const ANALYSIS_RATE: u32 = 12_000;
const WINDOW: usize = 1_024;
const FFT_SIZE: usize = 4_096;
/// One frame of analysis every 10 ms.
const HOP: usize = 120;
const FRAME_SECONDS: f32 = HOP as f32 / ANALYSIS_RATE as f32;
/// The lowest and highest pitches followed, as MIDI notes: a guitar's range.
const LOWEST_MIDI: f32 = 40.0;
const HIGHEST_MIDI: f32 = 91.0;
/// Pitches are followed in steps of a fifth of a semitone.
const STEPS_PER_NOTE: usize = 5;
const PITCHES: usize = (HIGHEST_MIDI - LOWEST_MIDI) as usize * STEPS_PER_NOTE + 1;
const SILENT: usize = PITCHES;
const PEAKS: usize = 40;
const HARMONICS: usize = 8;
/// Each harmonic counts for this much of the one before.
const HARMONIC_FALL: f32 = 0.8;
/// How far a peak can sit from a harmonic and still count, in semitones.
const HARMONIC_REACH: f32 = 0.5;
/// Below this frequency the spectrum is turned down, so the bass doesn't take
/// the line just by being loud.
const BASS_CORNER: f32 = 220.0;
/// The salience that counts as full strength, as a share of the song's frames.
const STRONG_SHARE: f32 = 0.95;
/// What the silent path scores in a frame, against 1 for a strong pitch.
const SILENT_SCORE: f32 = 0.3;
/// What moving costs per frame: per semitone of smooth glide, for a leap, and
/// for starting or stopping.
const GLIDE_COST: f32 = 0.12;
const LEAP_COST: f32 = 0.9;
const VOICE_COST: f32 = 0.5;
/// How far a glide reaches in one frame, in steps. Past that it's a leap.
const GLIDE_REACH: usize = 2 * STEPS_PER_NOTE;
/// Frames either side in the median that takes the wobble out of the path.
const SMOOTHING: usize = 3;
/// How far the pitch has to leave a note to end it, in semitones. More than half
/// a semitone, so vibrato stays one note.
const NOTE_REACH: f32 = 0.7;
/// Frames the pitch has to stay away before the note really has ended.
const LEAVE_FRAMES: usize = 4;
/// The shortest note kept, in seconds.
const SHORTEST_NOTE: f32 = 0.07;
/// Gaps shorter than this inside one pitch are the same note, in seconds.
const JOIN_GAP: f32 = 0.05;
/// A repeated note shows as the level falling away, then jumping back up by this
/// much within `PICK_FRAMES`. The first fall matters: a note's own attack also rises.
const PICK_RISE: f32 = 1.6;
const PICK_FRAMES: usize = 6;
/// The fewest frames either side of a split in a held note.
const PICK_SPACING: usize = 7;

/// One note of a melody, with times in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MelodyNote {
    pub start: f32,
    pub end: f32,
    pub midi: i32,
}

/// Steps `samples` down to the analysis rate. A windowed sinc takes out what
/// wouldn't fit, then a straight interpolation picks the new samples.
pub(crate) fn resample(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate == ANALYSIS_RATE {
        return samples.to_vec();
    }
    let ratio = rate as f32 / ANALYSIS_RATE as f32;
    let reach = (8.0 * ratio).ceil() as isize;
    let cutoff = 0.45 / ratio;
    let taps: Vec<f32> = (-reach..=reach)
        .map(|index| {
            let at = index as f32;
            let sinc = if index == 0 {
                1.0
            } else {
                (2.0 * PI * cutoff * at).sin() / (2.0 * PI * cutoff * at)
            };
            let shape = 0.5 + 0.5 * (PI * at / (reach + 1) as f32).cos();
            2.0 * cutoff * sinc * shape
        })
        .collect();
    let length = (samples.len() as f32 / ratio) as usize;
    (0..length)
        .map(|index| {
            let center = (index as f32 * ratio) as isize;
            taps.iter()
                .zip(-reach..=reach)
                .map(|(tap, offset)| {
                    let at = center + offset;
                    if at < 0 {
                        0.0
                    } else {
                        samples.get(at as usize).copied().unwrap_or(0.0) * tap
                    }
                })
                .sum()
        })
        .collect()
}

fn pitch_midi(step: usize) -> f32 {
    LOWEST_MIDI + step as f32 / STEPS_PER_NOTE as f32
}

fn hertz_midi(hertz: f32) -> f32 {
    69.0 + 12.0 * (hertz / 440.0).log2()
}

/// Reads one frame's spectrum into the strength of every pitch.
struct Frames {
    shape: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    level: Vec<f32>,
    peaks: Vec<(f32, f32)>,
}

impl Frames {
    fn new() -> Self {
        Self {
            shape: (0..WINDOW)
                .map(|index| (PI * index as f32 / WINDOW as f32).sin().powi(2))
                .collect(),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            level: vec![0.0; FFT_SIZE / 2],
            peaks: Vec::with_capacity(PEAKS * 2),
        }
    }

    fn salience(&mut self, samples: &[f32], out: &mut [f32]) {
        out.fill(0.0);
        self.re.fill(0.0);
        self.im.fill(0.0);
        for ((value, sample), shape) in self.re.iter_mut().zip(samples).zip(&self.shape) {
            *value = sample * shape;
        }
        fft(&mut self.re, &mut self.im);
        let hz_per_bin = ANALYSIS_RATE as f32 / FFT_SIZE as f32;
        for (bin, level) in self.level.iter_mut().enumerate() {
            let hertz = bin as f32 * hz_per_bin;
            *level = self.re[bin].hypot(self.im[bin]) * (hertz / (hertz + BASS_CORNER));
        }
        self.peaks.clear();
        for bin in 2..self.level.len() - 1 {
            let (left, middle, right) = (self.level[bin - 1], self.level[bin], self.level[bin + 1]);
            if middle > left && middle >= right && middle > 1e-4 {
                let (a, b, c) = (left.ln(), middle.ln(), right.ln());
                let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
                self.peaks
                    .push(((bin as f32 + shift) * hz_per_bin, middle.sqrt()));
            }
        }
        self.peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.peaks.truncate(PEAKS);
        for &(hertz, level) in &self.peaks {
            let midi = hertz_midi(hertz);
            let mut weight = level;
            for harmonic in 1..=HARMONICS {
                let root = midi - 12.0 * (harmonic as f32).log2();
                let center = (root - LOWEST_MIDI) * STEPS_PER_NOTE as f32;
                let reach = HARMONIC_REACH * STEPS_PER_NOTE as f32;
                let low = (center - reach).ceil().max(0.0) as usize;
                let high = (center + reach).floor().min((PITCHES - 1) as f32);
                if high >= 0.0 {
                    for (step, value) in
                        out.iter_mut().enumerate().take(high as usize + 1).skip(low)
                    {
                        let away = (step as f32 - center) / reach;
                        *value += weight * (0.5 * PI * away).cos().powi(2);
                    }
                }
                weight *= HARMONIC_FALL;
            }
        }
    }
}

/// The strongest path through `salience`, a frame of `PITCHES` values at a time.
/// Every entry is a step, or `SILENT`.
fn follow(salience: &[f32], strong: f32) -> Vec<usize> {
    let frames = salience.len() / PITCHES;
    let glide = GLIDE_COST / STEPS_PER_NOTE as f32;
    let mut scores = vec![0.0_f32; PITCHES + 1];
    let mut next = vec![0.0_f32; PITCHES + 1];
    // Where each state came from. A frame of these is cheap beside the audio.
    let mut from = vec![0_u16; frames * (PITCHES + 1)];
    for frame in 0..frames {
        let strengths = &salience[frame * PITCHES..(frame + 1) * PITCHES];
        let links = &mut from[frame * (PITCHES + 1)..(frame + 1) * (PITCHES + 1)];
        let (mut best, mut best_at) = (scores[0], 0);
        for (step, score) in scores.iter().enumerate().take(PITCHES) {
            if *score > best {
                (best, best_at) = (*score, step);
            }
        }
        for step in 0..PITCHES {
            let (mut top, mut top_at) = (best - LEAP_COST, best_at);
            let start = step.saturating_sub(GLIDE_REACH);
            let end = (step + GLIDE_REACH).min(PITCHES - 1);
            for (offset, score) in scores[start..=end].iter().enumerate() {
                let before = start + offset;
                let score = score - step.abs_diff(before) as f32 * glide;
                if score > top {
                    (top, top_at) = (score, before);
                }
            }
            let voiced = scores[SILENT] - VOICE_COST;
            if voiced > top {
                (top, top_at) = (voiced, SILENT);
            }
            next[step] = top + (strengths[step] / strong).min(1.5);
            links[step] = top_at as u16;
        }
        let (stay, leave) = (scores[SILENT], best - VOICE_COST);
        next[SILENT] = stay.max(leave) + SILENT_SCORE;
        links[SILENT] = if stay >= leave { SILENT } else { best_at } as u16;
        // Keeps the scores near zero, so a long song doesn't wear out the float.
        let top = next.iter().copied().fold(f32::MIN, f32::max);
        for (score, value) in scores.iter_mut().zip(&next) {
            *score = value - top;
        }
    }
    let mut path = vec![SILENT; frames];
    let mut state = (0..=PITCHES)
        .max_by(|a, b| scores[*a].total_cmp(&scores[*b]))
        .unwrap_or(SILENT);
    for frame in (0..frames).rev() {
        path[frame] = state;
        state = usize::from(from[frame * (PITCHES + 1) + state]);
    }
    path
}

/// The middle of each stretch of `path`, ignoring silence.
fn smooth(path: &[usize]) -> Vec<Option<f32>> {
    (0..path.len())
        .map(|frame| {
            if path[frame] == SILENT {
                return None;
            }
            let start = frame.saturating_sub(SMOOTHING);
            let end = (frame + SMOOTHING + 1).min(path.len());
            let mut near: Vec<usize> = path[start..end]
                .iter()
                .copied()
                .filter(|step| *step != SILENT)
                .collect();
            near.sort_unstable();
            Some(pitch_midi(near[near.len() / 2]))
        })
        .collect()
}

/// How far the recording sits from concert pitch, in semitones. Bands tune by
/// ear and tape runs fast, so each pitch votes for its offset from a semitone.
fn tuning(pitches: &[Option<f32>]) -> f32 {
    let (mut x, mut y) = (0.0, 0.0);
    for pitch in pitches.iter().flatten() {
        let angle = 2.0 * PI * pitch.fract();
        x += angle.cos();
        y += angle.sin();
    }
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let offset = y.atan2(x) / (2.0 * PI);
    // A fifth of a semitone is as fine as the path is, so don't read past it.
    (offset * STEPS_PER_NOTE as f32).round() / STEPS_PER_NOTE as f32
}

/// Cuts a smoothed pitch path into notes. `levels` splits repeated notes.
fn segment(pitches: &[Option<f32>], levels: &[f32], tune: f32) -> Vec<MelodyNote> {
    let seconds = |frame: usize| frame as f32 * FRAME_SECONDS;
    let mut notes: Vec<MelodyNote> = Vec::new();
    let mut current: Option<(i32, usize)> = None;
    let mut away = 0;
    let mut end = |current: &mut Option<(i32, usize)>, frame: usize| {
        if let Some((midi, start)) = current.take()
            && seconds(frame - start) >= SHORTEST_NOTE
        {
            notes.push(MelodyNote {
                start: seconds(start),
                end: seconds(frame),
                midi,
            });
        }
    };
    for (frame, pitch) in pitches.iter().enumerate() {
        let tuned = pitch.map(|pitch| pitch - tune);
        let Some(tuned) = tuned else {
            end(&mut current, frame);
            away = 0;
            continue;
        };
        match current {
            None => current = Some((tuned.round() as i32, frame)),
            Some((midi, start)) => {
                if (tuned - midi as f32).abs() > NOTE_REACH {
                    away += 1;
                    if away >= LEAVE_FRAMES {
                        end(&mut current, frame + 1 - away);
                        current = Some((tuned.round() as i32, frame + 1 - away));
                        away = 0;
                    }
                } else {
                    away = 0;
                    let picked = frame - start >= PICK_SPACING && {
                        let dip = levels[frame - PICK_FRAMES];
                        let held = levels[start..frame - PICK_FRAMES]
                            .iter()
                            .copied()
                            .fold(0.0, f32::max);
                        levels[frame] > PICK_RISE * dip
                            && held > PICK_RISE * dip
                            && levels[frame] >= levels[frame - 1]
                    };
                    if picked {
                        end(&mut current, frame);
                        current = Some((midi, frame));
                    }
                }
            }
        }
    }
    end(&mut current, pitches.len());
    let mut joined: Vec<MelodyNote> = Vec::with_capacity(notes.len());
    for note in notes {
        match joined.last_mut() {
            // No gap at all means a new pick split them, so they stay apart.
            Some(last)
                if last.midi == note.midi
                    && note.start > last.end
                    && note.start - last.end < JOIN_GAP =>
            {
                last.end = note.end;
            }
            _ => joined.push(note),
        }
    }
    joined
}

/// Finds the melody in mono `samples`. `progress` hears a share from 0 to 1.
pub fn transcribe(samples: &[f32], rate: u32, progress: impl FnMut(f32)) -> Vec<MelodyNote> {
    melody_notes(&resample(samples, rate), progress).0
}

/// Finds the melody in `audio` at the analysis rate, and the recording's tuning.
pub(crate) fn melody_notes(audio: &[f32], mut progress: impl FnMut(f32)) -> (Vec<MelodyNote>, f32) {
    if audio.len() < WINDOW {
        return (Vec::new(), 0.0);
    }
    let frames = (audio.len() - WINDOW) / HOP + 1;
    let mut salience = vec![0.0; frames * PITCHES];
    let mut reader = Frames::new();
    for (frame, out) in salience.as_chunks_mut::<PITCHES>().0.iter_mut().enumerate() {
        reader.salience(&audio[frame * HOP..frame * HOP + WINDOW], out);
        if frame % 256 == 0 {
            progress(0.9 * frame as f32 / frames as f32);
        }
    }
    let mut tops: Vec<f32> = salience
        .as_chunks::<PITCHES>()
        .0
        .iter()
        .map(|frame| frame.iter().copied().fold(0.0, f32::max))
        .collect();
    tops.sort_by(f32::total_cmp);
    let strong = tops[((frames - 1) as f32 * STRONG_SHARE) as usize];
    if strong <= 0.0 {
        return (Vec::new(), 0.0);
    }
    let path = follow(&salience, strong);
    progress(0.97);
    let levels: Vec<f32> = path
        .iter()
        .enumerate()
        .map(|(frame, step)| match *step {
            SILENT => 0.0,
            step => salience[frame * PITCHES + step],
        })
        .collect();
    let pitches = smooth(&path);
    let tune = tuning(&pitches);
    let notes = segment(&pitches, &levels, tune);
    progress(1.0);
    // Analysis frames are stamped at their start; a note sounds at their middle.
    let center = WINDOW as f32 / 2.0 / ANALYSIS_RATE as f32;
    let notes = notes
        .into_iter()
        .map(|note| MelodyNote {
            start: note.start + center,
            end: note.end + center,
            ..note
        })
        .collect();
    (notes, tune)
}

#[cfg(test)]
mod tests {
    use super::{resample, segment, tuning};

    #[test]
    fn resampling_keeps_a_tone_and_drops_what_cant_fit() {
        let tone = |hertz: f32| -> Vec<f32> {
            (0..48_000)
                .map(|index| (std::f32::consts::TAU * hertz * index as f32 / 48_000.0).sin())
                .collect()
        };
        let peak = |samples: &[f32]| samples[100..].iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!((peak(&resample(&tone(440.0), 48_000)) - 1.0).abs() < 0.05);
        assert!(peak(&resample(&tone(11_000.0), 48_000)) < 0.05);
    }

    #[test]
    fn tuning_finds_a_recording_that_runs_sharp() {
        let pitches: Vec<_> = [60.4, 62.4, 64.4, 65.4].map(Some).to_vec();
        assert!((tuning(&pitches) - 0.4).abs() < 1e-4);
    }

    #[test]
    fn vibrato_stays_one_note() {
        let pitches: Vec<_> = (0..60)
            .map(|frame| Some(60.0 + 0.5 * (frame as f32 * 0.5).sin()))
            .collect();
        let notes = segment(&pitches, &[1.0; 60], 0.0);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].midi, 60);
    }

    #[test]
    fn a_blip_is_not_a_note() {
        let mut pitches = vec![Some(60.0); 40];
        pitches.extend([Some(72.0); 3]);
        pitches.extend([Some(60.0); 40]);
        let notes = segment(&pitches, &[1.0; 83], 0.0);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn a_new_pick_splits_a_held_pitch() {
        let pitches = vec![Some(60.0); 60];
        let mut levels = vec![1.0; 60];
        levels[..30].fill(1.0);
        levels[24..30].fill(0.3);
        levels[30..].fill(1.0);
        let notes = segment(&pitches, &levels, 0.0);
        assert_eq!(notes.len(), 2);
    }
}
