//! Measures the melody tracker against a hand-checked pitch track, so changes to
//! it can be judged on real recordings. Labels are lines of `seconds,hertz`, with
//! 0 hertz where no melody sounds, as vocadito and MedleyDB's melody files use.
//!
//! cargo run --release -p amp-engine --example melody_score -- song.mp3 labels.csv

use amp_dsp::{MelodyNote, transcribe};
use amp_engine::{ENGINE_RATE, read_song};
use std::{env, error::Error, fs};

/// How near a pitch has to be to count, in semitones. The standard half semitone.
const TOLERANCE: f32 = 0.5;

fn midi(hertz: f32) -> f32 {
    69.0 + 12.0 * (hertz / 440.0).log2()
}

fn sounding(notes: &[MelodyNote], time: f32) -> Option<i32> {
    let at = notes.partition_point(|note| note.start <= time);
    at.checked_sub(1)
        .map(|index| &notes[index])
        .filter(|note| time < note.end)
        .map(|note| note.midi)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let (Some(song), Some(labels)) = (arguments.next(), arguments.next()) else {
        return Err("usage: melody_score <song> <labels.csv>".into());
    };
    let notes = transcribe(&read_song(song.as_ref())?, ENGINE_RATE, |_| {});
    let (mut voiced, mut pitch_right, mut chroma_right) = (0, 0, 0);
    let (mut frames, mut all_right) = (0, 0);
    for line in fs::read_to_string(labels)?.lines() {
        let mut fields = line
            .split([',', '\t', ' '])
            .filter(|field| !field.is_empty());
        let (Some(time), Some(hertz)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(time), Ok(hertz)) = (time.parse::<f32>(), hertz.parse::<f32>()) else {
            continue;
        };
        let heard = sounding(&notes, time);
        frames += 1;
        if hertz <= 0.0 {
            all_right += usize::from(heard.is_none());
            continue;
        }
        voiced += 1;
        let Some(heard) = heard else {
            continue;
        };
        let away = heard as f32 - midi(hertz);
        let wrapped = away - 12.0 * (away / 12.0).round();
        let right = away.abs() <= TOLERANCE;
        pitch_right += usize::from(right);
        chroma_right += usize::from(wrapped.abs() <= TOLERANCE);
        all_right += usize::from(right);
    }
    let share = |part: usize, whole: usize| 100.0 * part as f32 / whole.max(1) as f32;
    println!("{} notes found", notes.len());
    println!("raw pitch accuracy   {:5.1}%", share(pitch_right, voiced));
    println!("raw chroma accuracy  {:5.1}%", share(chroma_right, voiced));
    println!("overall accuracy     {:5.1}%", share(all_right, frames));
    Ok(())
}
