//! How well the melody tracker follows a line through a modeled band. These are
//! sums of decaying harmonics, not recordings, so they check the method rather
//! than promise a score on a real song.

use amp_dsp::{MelodyNote, midi_hertz, read_song_map, transcribe};
use std::f32::consts::TAU;

const RATE: u32 = 44_100;

/// A plucked note added into `buffer`.
fn pluck(buffer: &mut [f32], start: f32, length: f32, midi: i32, level: f32, partials: usize) {
    let hertz = midi_hertz(midi);
    let first = (start * RATE as f32) as usize;
    let count = (length * RATE as f32) as usize;
    for offset in 0..count {
        let Some(sample) = buffer.get_mut(first + offset) else {
            return;
        };
        let time = offset as f32 / RATE as f32;
        let attack = (time / 0.004).min(1.0);
        let release = ((length - time) / 0.02).clamp(0.0, 1.0);
        let body: f32 = (1..=partials)
            .map(|partial| {
                let number = partial as f32;
                (TAU * hertz * number * time).sin() * (-time * 1.5 * number).exp() / number
            })
            .sum();
        *sample += body * attack * release * level;
    }
}

/// Noise bursts on the beat, the way drums smear across every band.
fn drums(buffer: &mut [f32], beat: f32, level: f32) {
    let mut seed = 22_222_u32;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut at = 0.0;
    while at * (RATE as f32) < buffer.len() as f32 {
        let first = (at * RATE as f32) as usize;
        for offset in 0..6_000 {
            let Some(sample) = buffer.get_mut(first + offset) else {
                break;
            };
            *sample += random() * (-(offset as f32) / 1_200.0).exp() * level;
        }
        at += beat;
    }
}

/// A sixteen-note phrase, its length in beats per note, in A minor.
const PHRASE: [(i32, f32); 16] = [
    (69, 1.0),
    (72, 0.5),
    (74, 0.5),
    (76, 1.5),
    (74, 0.5),
    (72, 1.0),
    (69, 1.0),
    (67, 0.5),
    (69, 0.5),
    (64, 1.0),
    (67, 1.0),
    (69, 1.0),
    (72, 1.0),
    (76, 0.5),
    (79, 0.5),
    (81, 2.0),
];
const BEAT: f32 = 0.5;
/// Roots of the backing and the size of each third, a bar each: Am, F, C, G.
const CHANGES: [i32; 4] = [45, 41, 48, 43];
const THIRDS: [i32; 4] = [3, 4, 4, 4];

fn melody(transpose: i32) -> Vec<MelodyNote> {
    let mut at = 0.25;
    PHRASE
        .iter()
        .map(|(midi, beats)| {
            let length = beats * BEAT;
            let note = MelodyNote {
                start: at,
                end: at + length,
                midi: midi + transpose,
            };
            at += length;
            note
        })
        .collect()
}

/// The melody, alone or over chords, bass, and drums.
fn render(notes: &[MelodyNote], band: bool, sharp: f32) -> Vec<f32> {
    let total = notes.last().map_or(0.0, |note| note.end) + 0.5;
    let mut buffer = vec![0.0; (total * RATE as f32) as usize];
    if band {
        let mut bar = 0;
        let mut at = 0.0;
        while at < total {
            let root = CHANGES[bar % CHANGES.len()];
            let third = THIRDS[bar % THIRDS.len()];
            for step in 0..4 {
                let beat = at + step as f32 * BEAT;
                pluck(&mut buffer, beat, BEAT, root - 12, 0.3, 4);
                for tone in [12, 12 + third, 19] {
                    pluck(&mut buffer, beat, BEAT, root + tone, 0.1, 6);
                }
            }
            at += 4.0 * BEAT;
            bar += 1;
        }
        drums(&mut buffer, BEAT, 0.15);
    }
    let mut lead = vec![0.0; buffer.len()];
    for note in notes {
        pluck(
            &mut lead,
            note.start,
            note.end - note.start,
            note.midi,
            0.4,
            8,
        );
    }
    // Reading the lead a little faster raises its pitch, like a sharp tape.
    let rate = 2.0_f32.powf(sharp / 12.0);
    for (index, sample) in buffer.iter_mut().enumerate() {
        let at = index as f32 * rate;
        let (from, share) = (at as usize, at.fract());
        if let (Some(a), Some(b)) = (lead.get(from), lead.get(from + 1)) {
            *sample += a + (b - a) * share;
        }
    }
    buffer
}

/// The share of the true melody's time the transcription has the right note.
fn accuracy(truth: &[MelodyNote], found: &[MelodyNote]) -> f32 {
    let mut right = 0.0;
    let mut total = 0.0;
    for note in truth {
        total += note.end - note.start;
        for heard in found.iter().filter(|heard| heard.midi == note.midi) {
            right += (note.end.min(heard.end) - note.start.max(heard.start)).max(0.0);
        }
    }
    right / total
}

fn score(truth: &[MelodyNote], audio: &[f32]) -> f32 {
    let found = transcribe(audio, RATE, |_| {});
    let score = accuracy(truth, &found);
    eprintln!("{} true, {} found, {score:.3}", truth.len(), found.len());
    score
}

#[test]
fn a_lone_line_comes_back_note_for_note() {
    let truth = melody(0);
    let found = transcribe(&render(&truth, false, 0.0), RATE, |_| {});
    let pitches: Vec<i32> = found.iter().map(|note| note.midi).collect();
    let expected: Vec<i32> = truth.iter().map(|note| note.midi).collect();
    assert_eq!(pitches, expected);
    assert!(accuracy(&truth, &found) > 0.9);
}

#[test]
fn a_line_stands_out_over_a_band() {
    let truth = melody(0);
    assert!(score(&truth, &render(&truth, true, 0.0)) > 0.8);
}

#[test]
fn a_line_low_on_the_neck_stands_out_too() {
    let truth = melody(-12);
    assert!(score(&truth, &render(&truth, true, 0.0)) > 0.7);
}

#[test]
fn a_recording_a_little_sharp_lands_on_the_right_notes() {
    let truth = melody(0);
    let at_tempo: Vec<MelodyNote> = truth
        .iter()
        .map(|note| {
            let rate = 2.0_f32.powf(0.4 / 12.0);
            MelodyNote {
                start: note.start / rate,
                end: note.end / rate,
                midi: note.midi,
            }
        })
        .collect();
    assert!(score(&at_tempo, &render(&truth, false, 0.4)) > 0.9);
}

#[test]
fn the_beats_of_a_band_fall_on_its_pulse() {
    let map = read_song_map(&render(&melody(0), true, 0.0), RATE, |_| {});
    let gaps: Vec<f32> = map.beats.windows(2).map(|pair| pair[1] - pair[0]).collect();
    eprintln!("{} beats, gaps {gaps:?}", map.beats.len());
    let right = gaps
        .iter()
        .filter(|gap| (**gap - BEAT).abs() < 0.03)
        .count();
    assert!(right * 10 >= gaps.len() * 8, "{gaps:?}");
    let on_pulse = map.beats.iter().filter(|beat| {
        let off = (*beat / BEAT).fract();
        off.min(1.0 - off) * BEAT < 0.06
    });
    assert!(
        on_pulse.count() * 10 >= map.beats.len() * 8,
        "{:?}",
        map.beats
    );
}

#[test]
fn the_chords_of_a_band_follow_its_changes() {
    let map = read_song_map(&render(&melody(0), true, 0.0), RATE, |_| {});
    eprintln!("{:?} key {:?}", map.chords, map.key);
    let expected = ["Am", "F", "C", "G"];
    let times: Vec<f32> = (0..15).map(|beat| (beat as f32 + 0.5) * BEAT).collect();
    let right = times.iter().filter(|time| {
        let bar = (**time / (4.0 * BEAT)) as usize;
        map.chords.iter().any(|chord| {
            (chord.start..chord.end).contains(*time) && chord.name == expected[bar % 4]
        })
    });
    assert!(right.count() >= 10, "{:?}", map.chords);
}

/// Prints how long a four minute song takes. Run with `--ignored --nocapture`.
#[test]
#[ignore = "timing only"]
fn a_whole_song_takes_seconds() {
    let truth = melody(0);
    let bars = render(&truth, true, 0.0);
    let song: Vec<f32> = bars
        .iter()
        .copied()
        .cycle()
        .take(240 * RATE as usize)
        .collect();
    let started = std::time::Instant::now();
    let found = transcribe(&song, RATE, |_| {});
    eprintln!("{} notes in {:?}", found.len(), started.elapsed());
}

#[test]
fn silence_has_no_melody() {
    assert!(transcribe(&vec![0.0; 44_100], RATE, |_| {}).is_empty());
    assert!(transcribe(&[0.0; 16], RATE, |_| {}).is_empty());
}
