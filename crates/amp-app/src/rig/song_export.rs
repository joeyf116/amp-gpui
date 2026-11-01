//! Takes a song's tab out of the app, as plain text and as MIDI.

use super::song::{Song, TabNote};

const STRING_LABELS: [&str; 6] = ["e", "B", "G", "D", "A", "E"];
/// How wide a row of text tab runs, in characters.
const ROW_WIDTH: usize = 72;
/// 480 ticks to the quarter note at 120 BPM, the MIDI default.
const TICKS_PER_QUARTER: u16 = 480;
const TICKS_PER_SECOND: f32 = 960.0;
const VELOCITY: u8 = 96;

/// Writes the melody as text tab, with chords over the notes they start on.
pub(super) fn ascii_tab(song: &Song) -> String {
    let mut text = format!("{}\n\n", song.name);
    if let Some(key) = song.key {
        text.push_str(&format!("Key: {}\n\n", key.name()));
    }
    for row in rows(song).chunks(ROW_WIDTH) {
        let chords: String = row.iter().map(|column| column.chord).collect();
        text.push_str(chords.trim_end());
        text.push('\n');
        for (string, label) in STRING_LABELS.iter().enumerate() {
            let line: String = row.iter().map(|column| column.strings[string]).collect();
            text.push_str(&format!("{label}|{line}|\n"));
        }
        text.push('\n');
    }
    text
}

/// One character across the tab.
struct Column {
    chord: char,
    strings: [char; 6],
}

fn rows(song: &Song) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    let mut chords = song.chords.iter().peekable();
    for note in &song.notes {
        let mut chord: Vec<char> = Vec::new();
        while let Some(next) = chords.next_if(|next| next.start <= note.start) {
            chord = next.name.chars().collect();
        }
        let fret: Vec<char> = note.fret.to_string().chars().collect();
        let width = fret.len().max(chord.len()) + 1;
        for index in 0..width {
            let mut strings = ['-'; 6];
            strings[note.string] = fret.get(index).copied().unwrap_or('-');
            columns.push(Column {
                chord: chord.get(index).copied().unwrap_or(' '),
                strings,
            });
        }
    }
    columns
}

/// Adds a variable length number, as MIDI writes time.
fn push_length(bytes: &mut Vec<u8>, mut value: u32) {
    let mut groups = vec![(value & 0x7f) as u8];
    value >>= 7;
    while value > 0 {
        groups.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    bytes.extend(groups.iter().rev());
}

/// Writes the melody as a one track MIDI file.
pub(super) fn midi_file(notes: &[TabNote]) -> Vec<u8> {
    let ticks = |seconds: f32| (seconds * TICKS_PER_SECOND) as u32;
    let mut events: Vec<(u32, u8, u8)> = Vec::with_capacity(notes.len() * 2);
    for note in notes {
        let pitch = note.midi.clamp(0, 127) as u8;
        events.push((ticks(note.start), 0x90, pitch));
        events.push((ticks(note.end), 0x80, pitch));
    }
    // A note off sorts first, so a repeated pitch isn't cut short by its own end.
    events.sort_by_key(|(at, kind, _)| (*at, *kind));
    let mut track = Vec::new();
    let mut last = 0;
    for (at, kind, pitch) in events {
        push_length(&mut track, at - last);
        track.extend([kind, pitch, VELOCITY]);
        last = at;
    }
    track.extend([0x00, 0xff, 0x2f, 0x00]);
    let mut file = b"MThd".to_vec();
    file.extend(6_u32.to_be_bytes());
    file.extend(0_u16.to_be_bytes());
    file.extend(1_u16.to_be_bytes());
    file.extend(TICKS_PER_QUARTER.to_be_bytes());
    file.extend(b"MTrk");
    file.extend((track.len() as u32).to_be_bytes());
    file.extend(track);
    file
}

#[cfg(test)]
mod tests {
    use super::{midi_file, push_length};
    use crate::rig::song::TabNote;

    #[test]
    fn long_times_take_more_bytes() {
        let encoded = |value| {
            let mut bytes = Vec::new();
            push_length(&mut bytes, value);
            bytes
        };
        assert_eq!(encoded(0), [0x00]);
        assert_eq!(encoded(0x7f), [0x7f]);
        assert_eq!(encoded(0x80), [0x81, 0x00]);
        assert_eq!(encoded(0x2000), [0xc0, 0x00]);
    }

    #[test]
    fn a_note_becomes_an_on_and_an_off_at_its_times() {
        let note = TabNote {
            start: 0.0,
            end: 0.5,
            midi: 60,
            string: 2,
            fret: 5,
            pinned: None,
            hit: false,
        };
        let file = midi_file(&[note]);
        assert_eq!(&file[..4], b"MThd");
        assert_eq!(&file[14..18], b"MTrk");
        let track = &file[22..];
        assert_eq!(&track[..4], [0x00, 0x90, 60, 96]);
        // 480 ticks later, in two bytes.
        assert_eq!(&track[4..9], [0x83, 0x60, 0x80, 60, 96]);
        assert_eq!(&track[9..], [0x00, 0xff, 0x2f, 0x00]);
    }
}
