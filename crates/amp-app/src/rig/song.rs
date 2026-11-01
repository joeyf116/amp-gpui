//! A loaded song and what was read from it. The sound plays through a worker in
//! the engine, so this only says how it should play, and reads back how far it
//! has got. What's on screen follows that position. `song_view.rs` draws it.

use super::{
    Rig,
    fretboard::{finger_line, positions},
    song_export::{ascii_tab, midi_file},
};
use crate::settings::data_dir;
use amp_dsp::{MelodyNote, Pitch, SongChord, SongKey, SongMap, read_song_map};
use amp_engine::{Command, ENGINE_RATE, SongError, SongFeed, SongPlan, read_song};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

const SPEED_RANGE: (f32, f32) = (0.25, 1.0);
const SPEED_STEP: f32 = 0.05;
const SEMITONE_RANGE: i32 = 6;
/// How close to a beat a loop point snaps to it, in seconds.
const BEAT_SNAP: f32 = 0.25;
/// How long the tuner takes to name a note after it sounds, in seconds.
const HEARING_LAG: f32 = 0.12;
/// Bump this when the reading of a song improves, so saved ones are read again.
const READING_VERSION: u32 = 1;

/// One melody note, with its place on the neck.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TabNote {
    pub(super) start: f32,
    pub(super) end: f32,
    pub(super) midi: i32,
    pub(super) string: usize,
    pub(super) fret: i32,
    /// The string the player chose, which the fingering has to keep.
    pub(super) pinned: Option<usize>,
    /// Whether the guitar played it on this pass.
    pub(super) hit: bool,
}

/// A song as remembered between runs, with the player's fixes in it.
#[derive(Deserialize, Serialize)]
struct SavedSong {
    version: u32,
    /// Start, end, MIDI note, and any string the player chose.
    notes: Vec<(f32, f32, i32, Option<usize>)>,
    beats: Vec<f32>,
    /// Start, end, and name.
    chords: Vec<(f32, f32, String)>,
    /// Root, and whether it's minor.
    key: Option<(usize, bool)>,
}

/// A loop being marked out, or in use, in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Repeat {
    #[default]
    Off,
    Marking(f32),
    On(f32, f32),
}

pub(super) struct Song {
    pub(super) name: SharedString,
    audio: Arc<[f32]>,
    pub(super) notes: Vec<TabNote>,
    pub(super) beats: Vec<f32>,
    pub(super) chords: Vec<SongChord>,
    pub(super) key: Option<SongKey>,
    /// How far it has played, in seconds.
    pub(super) position: f32,
    pub(super) playing: bool,
    pub(super) speed: f32,
    pub(super) semitones: i32,
    pub(super) repeat: Repeat,
    /// Raises the speed a step after each pass of a loop.
    pub(super) trainer: bool,
    pub(super) selected: Option<usize>,
    /// The share of notes played on the last full pass.
    pub(super) last_pass: Option<f32>,
    /// Where the current pass of scoring began, in seconds.
    pass_start: f32,
    /// Set after a jump, as the engine's position lags it by a moment.
    moved: bool,
    saved_at: PathBuf,
    feed: Option<SongFeed>,
}

impl Song {
    pub(super) fn length(&self) -> f32 {
        self.audio.len() as f32 / ENGINE_RATE as f32
    }

    /// The note sounding now, and the one after it.
    pub(super) fn now_and_next(&self) -> (Option<&TabNote>, Option<&TabNote>) {
        let at = self
            .notes
            .partition_point(|note| note.start <= self.position);
        let now = at
            .checked_sub(1)
            .map(|index| &self.notes[index])
            .filter(|note| self.position < note.end);
        (now, self.notes.get(at))
    }

    fn plan(&self) -> SongPlan {
        let samples = |seconds: f32| (seconds * ENGINE_RATE as f32) as usize;
        SongPlan {
            position: samples(self.position),
            speed: self.speed,
            semitones: self.semitones,
            repeat: match self.repeat {
                Repeat::On(start, end) => Some((samples(start), samples(end))),
                _ => None,
            },
        }
    }

    /// Tells the worker how to play from here on.
    fn replan(&mut self) {
        self.moved = true;
        if let Some(feed) = &self.feed {
            feed.set(self.plan());
        }
    }

    /// Clears the score for a fresh pass from the position.
    fn start_pass(&mut self) {
        self.notes.iter_mut().for_each(|note| note.hit = false);
        self.pass_start = self.position;
    }

    /// The share of notes played since the pass began, once any have gone by.
    pub(super) fn score(&self) -> Option<f32> {
        let gone_by = self
            .notes
            .iter()
            .filter(|note| note.start >= self.pass_start && note.end < self.position);
        let (total, hits) = gone_by.fold((0, 0), |(total, hits), note| {
            (total + 1, hits + usize::from(note.hit))
        });
        (total > 0).then(|| hits as f32 / total as f32)
    }

    /// Credits the note that was sounding when the tuner heard `midi`. Octaves
    /// count, as a line played an octave off is still the right line.
    fn hear(&mut self, midi: i32) {
        let then = self.position - HEARING_LAG * self.speed;
        let at = self.notes.partition_point(|note| note.start <= then);
        if let Some(note) = at.checked_sub(1).map(|index| &mut self.notes[index])
            && then < note.end
            && (note.midi - midi).rem_euclid(12) == 0
        {
            note.hit = true;
        }
    }

    /// The nearest beat if one is close, so loops land cleanly.
    fn snapped(&self, time: f32) -> f32 {
        let at = self.beats.partition_point(|beat| *beat < time);
        let around = at.checked_sub(1).into_iter().chain([at]);
        around
            .filter_map(|index| self.beats.get(index))
            .copied()
            .filter(|beat| (beat - time).abs() <= BEAT_SNAP)
            .min_by(|a, b| (a - time).abs().total_cmp(&(b - time).abs()))
            .unwrap_or(time)
    }

    /// Works the frets out again, keeping any strings the player chose.
    fn refinger(&mut self) {
        let melody: Vec<MelodyNote> = self
            .notes
            .iter()
            .map(|note| MelodyNote {
                start: note.start,
                end: note.end,
                midi: note.midi,
            })
            .collect();
        let pins: Vec<Option<usize>> = self.notes.iter().map(|note| note.pinned).collect();
        for (note, (string, fret)) in self.notes.iter_mut().zip(finger_line(&melody, &pins)) {
            (note.string, note.fret) = (string, fret);
        }
    }

    /// Remembers the song with the player's fixes, in concert pitch.
    fn save(&self) -> Result<(), String> {
        let saved = SavedSong {
            version: READING_VERSION,
            notes: self
                .notes
                .iter()
                .map(|note| {
                    let midi = note.midi - self.semitones;
                    (note.start, note.end, midi, note.pinned)
                })
                .collect(),
            beats: self.beats.clone(),
            chords: self
                .chords
                .iter()
                .map(|chord| (chord.start, chord.end, chord.name.clone()))
                .collect(),
            key: self.key.map(|key| (key.root, key.minor)),
        };
        let write = || {
            if let Some(parent) = self.saved_at.parent() {
                fs::create_dir_all(parent)?;
            }
            let temporary = self.saved_at.with_extension("json.tmp");
            fs::write(&temporary, serde_json::to_string(&saved)?)?;
            fs::rename(temporary, &self.saved_at)
        };
        write().map_err(|error: std::io::Error| error.to_string())
    }
}

/// Where the song panel is at.
#[derive(Default)]
pub(super) enum Deck {
    #[default]
    Empty,
    Reading {
        name: SharedString,
        /// The share done so far, as the bits of a float.
        progress: Arc<AtomicU32>,
    },
    Ready(Box<Song>),
    Failed(SharedString),
}

/// The song panel, and what it needs between frames.
#[derive(Default)]
pub(super) struct SongPanel {
    pub(super) deck: Deck,
    /// Counts loads, so a slow one can't replace a song picked after it.
    load: u64,
    pub(super) seek_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Puts the tuner and metronome away, so the neck and tab fit together.
    pub(super) compact: bool,
}

impl SongPanel {
    pub(super) fn song(&self) -> Option<&Song> {
        match &self.deck {
            Deck::Ready(song) => Some(song),
            _ => None,
        }
    }

    fn song_mut(&mut self) -> Option<&mut Song> {
        match &mut self.deck {
            Deck::Ready(song) => Some(song),
            _ => None,
        }
    }
}

/// A fingerprint of a song's sound, so a renamed file finds its saved reading.
fn fingerprint(audio: &[f32]) -> u64 {
    let start = 0xcbf2_9ce4_8422_2325_u64 ^ audio.len() as u64;
    audio.iter().step_by(61).fold(start, |hash, sample| {
        (hash ^ u64::from(sample.to_bits())).wrapping_mul(0x0100_0000_01b3)
    })
}

fn saved_path(audio: &[f32]) -> PathBuf {
    data_dir()
        .join("songs")
        .join(format!("{:016x}.json", fingerprint(audio)))
}

fn load_saved(path: &Path) -> Option<SavedSong> {
    let text = fs::read_to_string(path).ok()?;
    let saved: SavedSong = serde_json::from_str(&text).ok()?;
    (saved.version == READING_VERSION).then_some(saved)
}

/// What a fresh reading of a song leaves to remember.
fn saved_from(map: SongMap) -> SavedSong {
    SavedSong {
        version: READING_VERSION,
        notes: map
            .notes
            .iter()
            .map(|note| (note.start, note.end, note.midi, None))
            .collect(),
        beats: map.beats,
        chords: map
            .chords
            .into_iter()
            .map(|chord| (chord.start, chord.end, chord.name))
            .collect(),
        key: map.key.map(|key| (key.root, key.minor)),
    }
}

fn song_from(name: SharedString, audio: Arc<[f32]>, saved: SavedSong, at: PathBuf) -> Box<Song> {
    let notes = saved
        .notes
        .into_iter()
        .map(|(start, end, midi, pinned)| TabNote {
            start,
            end,
            midi,
            string: 0,
            fret: 0,
            pinned,
            hit: false,
        });
    let mut song = Song {
        name,
        audio,
        notes: notes.collect(),
        beats: saved.beats,
        chords: saved
            .chords
            .into_iter()
            .map(|(start, end, name)| SongChord { start, end, name })
            .collect(),
        key: saved.key.map(|(root, minor)| SongKey { root, minor }),
        position: 0.0,
        playing: false,
        speed: 1.0,
        semitones: 0,
        repeat: Repeat::Off,
        trainer: false,
        selected: None,
        last_pass: None,
        pass_start: 0.0,
        moved: false,
        saved_at: at,
        feed: None,
    };
    song.refinger();
    Box::new(song)
}

impl Rig {
    pub(super) fn load_song(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(false, "Load song", window, cx, |this, path, _, cx| {
            this.read_song(path, cx);
        });
    }

    fn read_song(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetSongPlaying(false));
        }
        self.fretboard.song.load += 1;
        let load = self.fretboard.song.load;
        let name: SharedString = path
            .file_stem()
            .map_or("Song".to_owned(), |stem| {
                stem.to_string_lossy().into_owned()
            })
            .into();
        let progress = Arc::new(AtomicU32::new(0));
        self.fretboard.song.deck = Deck::Reading {
            name: name.clone(),
            progress: progress.clone(),
        };
        cx.spawn(async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let audio = read_song(&path)?;
                let saved_at = saved_path(&audio);
                let saved = load_saved(&saved_at).unwrap_or_else(|| {
                    saved_from(read_song_map(&audio, ENGINE_RATE, |share| {
                        progress.store(share.to_bits(), Ordering::Relaxed);
                    }))
                });
                Ok::<_, SongError>(song_from(name, audio.into(), saved, saved_at))
            });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fretboard.song.load != load {
                    return;
                }
                this.fretboard.song.deck = match result {
                    Ok(song) => {
                        // A failed save only costs the next load some time.
                        let _ = song.save();
                        this.fretboard.song.compact = true;
                        Deck::Ready(song)
                    }
                    Err(error) => Deck::Failed(error.to_string().into()),
                };
                this.send_song();
                cx.notify();
            });
        })
        .detach();
    }

    /// Hands the engine the song. A restarted engine starts empty, so this also
    /// picks up where the song had got to.
    pub(super) fn send_song(&mut self) {
        let level = 10.0_f32.powf(self.settings.song_db / 20.0);
        let (Some(engine), Some(song)) = (&mut self.engine, self.fretboard.song.song_mut()) else {
            return;
        };
        let (feed, outlet) = SongFeed::start(song.audio.clone(), song.plan());
        song.feed = Some(feed);
        engine.send(Command::LoadSong(outlet));
        engine.send(Command::SetSongLevel(level));
        engine.send(Command::SetSongPlaying(song.playing));
    }

    /// Reads back how far the song has got, and listens for the note it's on.
    pub(super) fn follow_song(&mut self, heard: Option<Pitch>) {
        let Some(song) = self.fretboard.song.song_mut() else {
            return;
        };
        let Some(engine) = &mut self.engine else {
            return;
        };
        if !song.playing {
            return;
        }
        let meters = engine.meters();
        if meters.song_ended() {
            song.playing = false;
            song.last_pass = song.score();
            song.position = 0.0;
            song.start_pass();
            song.replan();
            return;
        }
        let position = meters.song_position() as f32 / ENGINE_RATE as f32;
        let moved = std::mem::take(&mut song.moved);
        let wrapped = !moved && position < song.position;
        if !moved {
            song.position = position;
        }
        if wrapped && matches!(song.repeat, Repeat::On(..)) {
            song.last_pass = song.score();
            song.start_pass();
            if song.trainer && song.speed < SPEED_RANGE.1 {
                song.speed = (song.speed + SPEED_STEP).min(SPEED_RANGE.1);
                song.replan();
            }
        }
        if let Some(pitch) = heard {
            song.hear((12.0 * (pitch.hertz / 440.0).log2() + 69.0).round() as i32);
        }
    }

    pub(super) fn toggle_song(&mut self, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            song.playing = !song.playing;
            let playing = song.playing;
            if let Some(engine) = &mut self.engine {
                engine.send(Command::SetSongPlaying(playing));
            }
        }
        cx.notify();
    }

    pub(super) fn seek_song(&mut self, share: f32, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            song.position = share.clamp(0.0, 1.0) * song.length();
            song.start_pass();
            song.replan();
        }
        cx.notify();
    }

    pub(super) fn set_song_level(&mut self, db: f32, cx: &mut Context<Self>) {
        self.settings.song_db = db;
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetSongLevel(10.0_f32.powf(db / 20.0)));
        }
        self.settings_save_at = Some(self.smoke_phase + super::SETTINGS_SAVE_DELAY);
        cx.notify();
    }

    pub(super) fn step_song_speed(&mut self, steps: f32, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            let speed = song.speed + steps * SPEED_STEP;
            song.speed = speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1);
            song.replan();
        }
        cx.notify();
    }

    /// Moves the song up or down, and the tab with it, so it can meet a guitar
    /// in another tuning.
    pub(super) fn step_song_key(&mut self, step: i32, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            let semitones = (song.semitones + step).clamp(-SEMITONE_RANGE, SEMITONE_RANGE);
            let change = semitones - song.semitones;
            song.semitones = semitones;
            song.notes.iter_mut().for_each(|note| note.midi += change);
            song.refinger();
            song.replan();
        }
        cx.notify();
    }

    /// Walks through marking a loop: its start, its end, then clearing it.
    pub(super) fn mark_song_loop(&mut self, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            let here = song.snapped(song.position);
            song.repeat = match song.repeat {
                Repeat::Off => Repeat::Marking(here),
                Repeat::Marking(start) if here > start => Repeat::On(start, here),
                Repeat::Marking(start) => Repeat::Marking(start),
                Repeat::On(..) => Repeat::Off,
            };
            match song.repeat {
                Repeat::On(start, _) => {
                    song.position = start;
                    song.start_pass();
                    song.replan();
                }
                Repeat::Off => song.replan(),
                // Marking changes nothing yet, so the sound carries on untouched.
                Repeat::Marking(_) => {}
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_song_trainer(&mut self, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            song.trainer = !song.trainer;
        }
        cx.notify();
    }

    pub(super) fn select_song_note(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            song.selected = (song.selected != Some(index)).then_some(index);
        }
        cx.notify();
    }

    /// Runs a fix on the selected note, then settles and remembers the result.
    fn fix_note(&mut self, cx: &mut Context<Self>, fix: impl FnOnce(&mut Song, usize)) {
        if let Some(song) = self.fretboard.song.song_mut()
            && let Some(index) = song.selected
        {
            fix(song, index);
            song.refinger();
            if let Err(error) = song.save() {
                self.set_error(error, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn nudge_song_note(&mut self, step: i32, cx: &mut Context<Self>) {
        self.fix_note(cx, |song, index| {
            let note = &mut song.notes[index];
            note.midi += step;
            note.pinned = None;
        });
    }

    /// Moves the selected note to the next string that has its pitch.
    pub(super) fn cycle_song_string(&mut self, cx: &mut Context<Self>) {
        self.fix_note(cx, |song, index| {
            let note = &mut song.notes[index];
            let strings: Vec<usize> = positions(note.midi).map(|(string, _)| string).collect();
            let at = strings.iter().position(|string| *string == note.string);
            note.pinned = at.map(|at| strings[(at + 1) % strings.len()]);
        });
    }

    pub(super) fn delete_song_note(&mut self, cx: &mut Context<Self>) {
        self.fix_note(cx, |song, index| {
            song.notes.remove(index);
            song.selected = None;
        });
    }

    /// Writes the tab and a MIDI file into a folder the player picks.
    pub(super) fn export_song(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Save tab here", window, cx, |this, folder, _, cx| {
            let Some(song) = this.fretboard.song.song() else {
                return;
            };
            let tab = folder.join(format!("{} tab.txt", song.name));
            let midi = folder.join(format!("{}.mid", song.name));
            let saved = fs::write(tab, ascii_tab(song))
                .and_then(|()| fs::write(midi, midi_file(&song.notes)));
            if let Err(error) = saved {
                this.set_error(error, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Repeat, SavedSong, Song, TabNote, fingerprint, song_from};
    use std::{path::PathBuf, sync::Arc};

    fn note(start: f32, end: f32, midi: i32) -> (f32, f32, i32, Option<usize>) {
        (start, end, midi, None)
    }

    fn song(position: f32) -> Box<Song> {
        let saved = SavedSong {
            version: 1,
            notes: vec![note(1.0, 1.5, 64), note(2.0, 2.5, 67)],
            beats: vec![1.0, 2.0, 3.0],
            chords: Vec::new(),
            key: None,
        };
        let audio = Arc::from(vec![0.0; 240_000]);
        let mut song = song_from("Test".into(), audio, saved, PathBuf::new());
        song.position = position;
        song
    }

    fn pitches(song: &Song) -> (Option<i32>, Option<i32>) {
        let (now, next) = song.now_and_next();
        (now.map(|note| note.midi), next.map(|note| note.midi))
    }

    #[test]
    fn the_note_now_and_the_one_coming_follow_the_song() {
        assert_eq!(pitches(&song(0.0)), (None, Some(64)));
        assert_eq!(pitches(&song(1.2)), (Some(64), Some(67)));
        assert_eq!(pitches(&song(1.7)), (None, Some(67)));
        assert_eq!(pitches(&song(2.2)), (Some(67), None));
        assert_eq!(pitches(&song(9.0)), (None, None));
    }

    #[test]
    fn loops_land_on_a_near_beat_and_stay_put_otherwise() {
        let song = song(0.0);
        assert_eq!(song.snapped(1.9), 2.0);
        assert_eq!(song.snapped(2.1), 2.0);
        assert_eq!(song.snapped(4.5), 4.5);
        assert_eq!(song.snapped(2.5), 2.5);
    }

    #[test]
    fn a_note_heard_while_it_sounds_counts_in_any_octave() {
        let mut song = song(1.3);
        song.hear(76);
        assert!(song.notes[0].hit);
        let mut song = self::song(1.3);
        song.hear(65);
        assert!(!song.notes[0].hit);
    }

    #[test]
    fn the_score_counts_notes_gone_by_since_the_pass_began() {
        let mut song = song(1.3);
        assert_eq!(song.score(), None);
        song.hear(64);
        song.position = 3.0;
        assert_eq!(song.score(), Some(0.5));
        song.start_pass();
        assert_eq!(song.score(), None);
    }

    #[test]
    fn a_chosen_string_survives_new_fingering() {
        let mut song = song(0.0);
        song.notes[1].pinned = Some(1);
        song.refinger();
        let TabNote { string, fret, .. } = song.notes[1];
        assert_eq!((string, fret), (1, 8));
    }

    #[test]
    fn a_loop_marks_its_start_then_its_end() {
        assert_ne!(Repeat::Marking(1.0), Repeat::On(1.0, 2.0));
        let mut song = song(1.0);
        song.repeat = Repeat::On(1.0, 2.0);
        assert_eq!(song.plan().repeat, Some((48_000, 96_000)));
        assert_eq!(song.plan().position, 48_000);
    }

    #[test]
    fn the_same_sound_gets_the_same_fingerprint() {
        let sound = vec![0.25; 4_096];
        assert_eq!(fingerprint(&sound), fingerprint(&sound.clone()));
        assert_ne!(fingerprint(&sound), fingerprint(&vec![0.5; 4_096]));
        assert_ne!(fingerprint(&sound), fingerprint(&vec![0.25; 8_192]));
    }
}
