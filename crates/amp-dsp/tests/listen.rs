//! Measures note, chord, and string detection on synthesized plucks. These are
//! modeled strings made deliberately untidy: an unknown string set, sloppy
//! intonation, a pickup that thins the fundamental, other strings ringing, and
//! noise. They still aren't recordings of a guitar, so treat the numbers as an
//! upper bound. Run with `--nocapture` to see them.

use amp_dsp::{Spectrum, chord_name, midi_hertz};

const RATE: u32 = 48_000;
const FRAMES: usize = 16_384;
/// Open strings as MIDI notes, low E first, with the stiffness of a 10-46 set.
const OPEN: [i32; 6] = [40, 45, 50, 55, 59, 64];
const STIFFNESS: [f32; 6] = [4.0e-4, 1.9e-4, 8.9e-5, 1.05e-4, 3.9e-5, 1.3e-5];

/// A small repeatable random source, so results don't change between runs.
struct Dice(u32);

impl Dice {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1 << 24) as f32
    }
}

/// One plucked string at a fret, added into `buffer`.
fn pluck(buffer: &mut [f32], string: usize, fret: i32, dice: &mut Dice) {
    let midi = OPEN[string] + fret;
    let cents = (dice.next() - 0.5) * 30.0;
    let root = midi_hertz(midi) * 2.0_f32.powf(cents / 1200.0);
    // A real set differs from the book values by gauge, brand, and age.
    let set = 0.7 + dice.next() * 0.6;
    let stiffness = set * STIFFNESS[string] * 2.0_f32.powf(fret as f32 / 6.0);
    let spot = 0.1 + dice.next() * 0.12;
    let level = 0.6 + dice.next() * 0.4;
    let age = 0.08 + dice.next() * 0.12;
    for partial in 1..=30 {
        let number = partial as f32;
        let hertz = number * root * (1.0 + stiffness * number * number).sqrt();
        if hertz > 12_000.0 {
            break;
        }
        // Pickups hear little of a low string's fundamental.
        let pickup = (hertz / 300.0).clamp(0.2, 1.0);
        let height = pickup * level * (std::f32::consts::PI * number * spot).sin().abs() / number;
        let phase = dice.next() * std::f32::consts::TAU;
        let decay = 1.2 + number * 0.35;
        for (index, sample) in buffer.iter_mut().enumerate() {
            let time = index as f32 / RATE as f32 + age;
            let angle = std::f32::consts::TAU * hertz * time + phase;
            *sample += height * (-decay * time).exp() * angle.sin() * 0.08;
        }
    }
}

/// Noise, plus an open string the hand didn't quite damp.
fn hiss(buffer: &mut [f32], dice: &mut Dice) {
    let string = (dice.next() * 6.0) as usize;
    let mut ringing = vec![0.0; buffer.len()];
    pluck(&mut ringing, string, 0, dice);
    for (sample, ring) in buffer.iter_mut().zip(&ringing) {
        *sample += ring * 0.05 + (dice.next() - 0.5) * 0.006;
    }
}

/// A chord as a fret per string, low E first, with -1 for a muted string.
const SHAPES: [(&str, [i32; 6]); 24] = [
    ("Em", [0, 2, 2, 0, 0, 0]),
    ("E", [0, 2, 2, 1, 0, 0]),
    ("Am", [-1, 0, 2, 2, 1, 0]),
    ("A", [-1, 0, 2, 2, 2, 0]),
    ("C", [-1, 3, 2, 0, 1, 0]),
    ("G", [3, 2, 0, 0, 0, 3]),
    ("D", [-1, -1, 0, 2, 3, 2]),
    ("Dm", [-1, -1, 0, 2, 3, 1]),
    ("F", [1, 3, 3, 2, 1, 1]),
    ("Bm", [-1, 2, 4, 4, 3, 2]),
    ("B", [-1, 2, 4, 4, 4, 2]),
    ("Fm", [1, 3, 3, 1, 1, 1]),
    ("Gm", [3, 5, 5, 3, 3, 3]),
    ("A7", [-1, 0, 2, 0, 2, 0]),
    ("E7", [0, 2, 0, 1, 0, 0]),
    ("D7", [-1, -1, 0, 2, 1, 2]),
    ("G7", [3, 2, 0, 0, 0, 1]),
    ("Am7", [-1, 0, 2, 0, 1, 0]),
    ("Em7", [0, 2, 0, 0, 0, 0]),
    ("Cmaj7", [-1, 3, 2, 0, 0, 0]),
    ("Fmaj7", [-1, -1, 3, 2, 1, 0]),
    ("Asus4", [-1, 0, 2, 2, 3, 0]),
    ("Dsus2", [-1, -1, 0, 2, 3, 0]),
    ("E5", [0, 2, 2, -1, -1, -1]),
];

fn strum(shape: &[i32; 6], dice: &mut Dice) -> (Vec<f32>, Vec<i32>) {
    let mut buffer = vec![0.0; FRAMES];
    let mut notes = Vec::new();
    for (string, fret) in shape.iter().enumerate() {
        if *fret >= 0 {
            pluck(&mut buffer, string, *fret, dice);
            notes.push(OPEN[string] + fret);
        }
    }
    hiss(&mut buffer, dice);
    (buffer, notes)
}

#[test]
fn single_notes_read_as_one_note() {
    let mut dice = Dice(7);
    let (mut right, mut total) = (0, 0);
    for (string, open) in OPEN.iter().enumerate() {
        for fret in 0..=12 {
            let mut buffer = vec![0.0; FRAMES];
            pluck(&mut buffer, string, fret, &mut dice);
            hiss(&mut buffer, &mut dice);
            let heard = Spectrum::new(&buffer, RATE).unwrap().notes();
            right += usize::from(heard == [open + fret]);
            total += 1;
        }
    }
    let share = right as f32 / total as f32;
    println!(
        "single notes exact: {right}/{total} ({:.0}%)",
        share * 100.0
    );
    assert!(share > 0.85, "only {share}");
}

#[test]
fn chords_are_named_from_their_sound() {
    let mut dice = Dice(11);
    let (mut named, mut found, mut wanted, mut extra, mut total) = (0, 0, 0, 0, 0);
    for _ in 0..4 {
        for (name, shape) in &SHAPES {
            let (buffer, notes) = strum(shape, &mut dice);
            let heard = Spectrum::new(&buffer, RATE).unwrap().notes();
            let classes = |list: &[i32]| {
                let mut classes: Vec<i32> = list.iter().map(|note| note % 12).collect();
                classes.sort_unstable();
                classes.dedup();
                classes
            };
            let (want, got) = (classes(&notes), classes(&heard));
            found += want.iter().filter(|class| got.contains(class)).count();
            wanted += want.len();
            extra += got.iter().filter(|class| !want.contains(class)).count();
            let result = chord_name(&heard);
            named += usize::from(result.as_deref() == Some(*name));
            total += 1;
            if result.as_deref() != Some(*name) {
                println!("{name}: heard {heard:?} as {result:?}");
            }
        }
    }
    let share = named as f32 / total as f32;
    println!(
        "chords named: {named}/{total} ({:.0}%), notes found {found}/{wanted}, extra {extra}",
        share * 100.0
    );
    assert!(share > 0.7, "only {share}");
}

#[test]
fn stiffness_tells_a_pitch_on_two_strings_apart() {
    let mut dice = Dice(3);
    let (mut right, mut total, mut read) = (0, 0, 0);
    for (string, open) in OPEN.iter().enumerate().take(5) {
        for fret in [5, 7, 9, 10, 12] {
            let midi = open + fret;
            let alternatives: Vec<(usize, i32)> = (0..6)
                .map(|other| (other, midi - OPEN[other]))
                .filter(|(_, fret)| (0..=15).contains(fret))
                .collect();
            let mut buffer = vec![0.0; FRAMES];
            pluck(&mut buffer, string, fret, &mut dice);
            hiss(&mut buffer, &mut dice);
            total += 1;
            let Some(measured) = Spectrum::new(&buffer, RATE)
                .unwrap()
                .stiffness(midi_hertz(midi))
            else {
                continue;
            };
            read += 1;
            let guess = alternatives.iter().min_by(|a, b| {
                let miss = |(string, fret): &(usize, i32)| {
                    let expected = STIFFNESS[*string] * 2.0_f32.powf(*fret as f32 / 6.0);
                    (measured / expected).ln().abs()
                };
                miss(a).total_cmp(&miss(b))
            });
            right += usize::from(guess == Some(&(string, fret)));
        }
    }
    println!("string from stiffness: {right}/{read} read, {total} tried");
    assert!(read * 10 >= total * 7, "read {read} of {total}");
    assert!(right * 10 >= read * 7, "right {right} of {read}");
}

#[test]
fn silence_hears_nothing() {
    assert!(Spectrum::new(&vec![0.0; FRAMES], RATE).is_none());
}
