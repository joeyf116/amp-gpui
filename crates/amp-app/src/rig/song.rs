//! A loaded song and its melody, shown as tab under the fretboard. The song plays
//! on the audio thread after the rig, so this only sends it orders and reads back
//! how far it has got. Everything on screen follows that position.

use super::{Rig, fretboard::finger_line};
use crate::{
    art::PRINT_FONT,
    knob::Knob,
    style::{fog, hairline, moss},
};
use amp_dsp::{MelodyNote, ParamSpec, transcribe};
use amp_engine::{Command, ENGINE_RATE, SongError, read_song};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

/// Seconds of song the tab shows at once.
const TAB_SPAN: f32 = 6.0;
/// How far along the tab the playhead sits.
const TAB_PLAYHEAD: f32 = 0.12;
const TAB_LINE: f32 = 15.0;
const TAB_STRINGS: usize = 6;
const SEEK_HEIGHT: f32 = 6.0;
const SONG_LEVEL: ParamSpec = ParamSpec {
    id: "song_level",
    name: "Song",
    unit: "dB",
    min: -40.0,
    max: 0.0,
    default: -12.0,
};

/// One melody note, with its place on the neck.
pub(super) struct TabNote {
    pub(super) start: f32,
    pub(super) end: f32,
    pub(super) midi: i32,
    pub(super) string: usize,
    pub(super) fret: i32,
}

pub(super) struct Song {
    name: SharedString,
    audio: Arc<[f32]>,
    notes: Vec<TabNote>,
    /// How far it has played, in seconds.
    position: f32,
    playing: bool,
}

impl Song {
    fn length(&self) -> f32 {
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
    Ready(Song),
    Failed(SharedString),
}

/// The song panel, and what it needs between frames.
#[derive(Default)]
pub(super) struct SongPanel {
    pub(super) deck: Deck,
    /// Counts loads, so a slow one can't replace a song picked after it.
    load: u64,
    seek_bounds: Rc<Cell<Bounds<Pixels>>>,
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

/// Places a melody on the neck.
fn tab_notes(melody: Vec<MelodyNote>) -> Vec<TabNote> {
    let spots = finger_line(&melody);
    melody
        .into_iter()
        .zip(spots)
        .map(|(note, (string, fret))| TabNote {
            start: note.start,
            end: note.end,
            midi: note.midi,
            string,
            fret,
        })
        .collect()
}

fn clock(seconds: f32) -> String {
    let whole = seconds.max(0.0) as u32;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// How far along the tab a moment falls, with `position` at the playhead.
fn tab_share(time: f32, position: f32) -> f32 {
    TAB_PLAYHEAD + (time - position) / TAB_SPAN
}

impl Rig {
    fn load_song(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                let melody = transcribe(&audio, ENGINE_RATE, |share| {
                    progress.store(share.to_bits(), Ordering::Relaxed);
                });
                Ok::<_, SongError>((Arc::<[f32]>::from(audio), tab_notes(melody)))
            });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fretboard.song.load != load {
                    return;
                }
                this.fretboard.song.deck = match result {
                    Ok((audio, notes)) => Deck::Ready(Song {
                        name,
                        audio,
                        notes,
                        position: 0.0,
                        playing: false,
                    }),
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
        let Some(song) = self.fretboard.song.song() else {
            return;
        };
        let commands = [
            Command::LoadSong(song.audio.clone()),
            Command::SetSongLevel(level),
            Command::SeekSong((song.position * ENGINE_RATE as f32) as usize),
            Command::SetSongPlaying(song.playing),
        ];
        if let Some(engine) = &mut self.engine {
            commands
                .into_iter()
                .for_each(|command| engine.send(command));
        }
    }

    /// Reads back how far the song has got.
    pub(super) fn follow_song(&mut self) {
        let Some(song) = self.fretboard.song.song_mut() else {
            return;
        };
        let Some(engine) = &mut self.engine else {
            song.playing = false;
            return;
        };
        let position = engine.meters().song_position();
        song.position = position as f32 / ENGINE_RATE as f32;
        if song.playing && position >= song.audio.len() {
            song.playing = false;
            song.position = 0.0;
            engine.send(Command::SetSongPlaying(false));
            engine.send(Command::SeekSong(0));
        }
    }

    fn toggle_song(&mut self, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            song.playing = !song.playing;
            let playing = song.playing;
            if let Some(engine) = &mut self.engine {
                engine.send(Command::SetSongPlaying(playing));
            }
        }
        cx.notify();
    }

    fn seek_song(&mut self, share: f32, cx: &mut Context<Self>) {
        if let Some(song) = self.fretboard.song.song_mut() {
            song.position = share.clamp(0.0, 1.0) * song.length();
            let position = (song.position * ENGINE_RATE as f32) as usize;
            if let Some(engine) = &mut self.engine {
                engine.send(Command::SeekSong(position));
            }
        }
        cx.notify();
    }

    fn set_song_level(&mut self, db: f32, cx: &mut Context<Self>) {
        self.settings.song_db = db;
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetSongLevel(10.0_f32.powf(db / 20.0)));
        }
        self.settings_save_at = Some(self.smoke_phase + super::SETTINGS_SAVE_DELAY);
        cx.notify();
    }

    fn render_seek(&self, song: &Song, cx: &mut Context<Self>) -> impl IntoElement {
        let share = song.position / song.length().max(0.001);
        let bounds = self.fretboard.song.seek_bounds.clone();
        let track = hairline();
        let filled = moss();
        let bar = canvas(
            move |frame, _, _| bounds.set(frame),
            move |frame, _, window, _| {
                window.paint_quad(fill(frame, track));
                let mut played = frame;
                played.size.width = frame.size.width * share;
                window.paint_quad(fill(played, filled));
            },
        )
        .size_full();
        div()
            .id("song-seek")
            .w_full()
            .h(px(SEEK_HEIGHT))
            .cursor_pointer()
            .child(bar)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    let bounds = this.fretboard.song.seek_bounds.get();
                    let along = (event.position.x - bounds.left()) / bounds.size.width.max(px(1.));
                    this.seek_song(along, cx);
                }),
            )
    }

    fn render_tab(&self, song: &Song, cx: &App) -> impl IntoElement {
        let line = hairline();
        let head = moss();
        let position = song.position;
        let (now, _) = song.now_and_next();
        let now_start = now.map(|note| note.start);
        let lines = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for string in 0..TAB_STRINGS {
                    let y = bounds.top() + px(TAB_LINE * (string as f32 + 0.5));
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.))),
                        line,
                    ));
                }
                let x = bounds.left() + bounds.size.width * TAB_PLAYHEAD;
                window.paint_quad(fill(
                    Bounds::new(point(x, bounds.top()), size(px(2.), bounds.size.height)),
                    head,
                ));
            },
        )
        .absolute()
        .size_full();
        let first = song
            .notes
            .partition_point(|note| note.end < position - TAB_SPAN * TAB_PLAYHEAD);
        let last = song
            .notes
            .partition_point(|note| note.start < position + TAB_SPAN);
        let background = cx.theme().background;
        let notes = song.notes[first..last.max(first)].iter().map(|note| {
            let lit = now_start == Some(note.start);
            let held = (note.end - note.start) / TAB_SPAN;
            let y = TAB_LINE * note.string as f32;
            div()
                .absolute()
                .left(relative(tab_share(note.start, position)))
                .top(px(y))
                .w(relative(held))
                .h(px(TAB_LINE))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(TAB_LINE / 2.0 - 1.0))
                        .h(px(3.))
                        .bg(moss().opacity(if lit { 0.7 } else { 0.3 })),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(-2.))
                        .h(px(TAB_LINE))
                        .px(px(2.))
                        .bg(background)
                        .text_size(px(11.))
                        .line_height(px(TAB_LINE))
                        .font_family(PRINT_FONT)
                        .font_semibold()
                        .text_color(if lit { moss() } else { fog() })
                        .child(note.fret.to_string()),
                )
        });
        div()
            .relative()
            .w_full()
            .h(px(TAB_LINE * TAB_STRINGS as f32))
            .overflow_hidden()
            .child(lines)
            .children(notes)
    }

    pub(super) fn render_song(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let song = self.fretboard.song.song();
        let playing = song.is_some_and(|song| song.playing);
        let status = match &self.fretboard.song.deck {
            Deck::Empty => "Load an MP3 to see its melody on the neck".to_owned(),
            Deck::Reading { name, progress } => {
                let share = f32::from_bits(progress.load(Ordering::Relaxed));
                format!("Listening to {name}, {:.0}%", share * 100.0)
            }
            Deck::Ready(song) if song.notes.is_empty() => {
                format!("{}: no clear melody found", song.name)
            }
            Deck::Ready(song) => format!(
                "{}, {} of {}, {} notes",
                song.name,
                clock(song.position),
                clock(song.length()),
                song.notes.len()
            ),
            Deck::Failed(error) => error.to_string(),
        };
        let failed = matches!(self.fretboard.song.deck, Deck::Failed(_));
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        Button::new("load-song")
                            .small()
                            .outline()
                            .label("Load song")
                            .tooltip("MP3, WAV, FLAC, or Ogg")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.load_song(window, cx)),
                            ),
                    )
                    .when_some(song, |row, _| {
                        row.child(
                            Button::new("play-song")
                                .small()
                                .when(playing, |button| button.outline())
                                .when(!playing, |button| button.primary())
                                .label(if playing { "Pause" } else { "Play" })
                                .disabled(!live)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_song(cx))),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(if failed { cx.theme().warning } else { fog() })
                            .child(status),
                    )
                    .when(song.is_some() && !live, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(fog())
                                .child("Start monitoring to hear the song."),
                        )
                    })
                    .when_some(song, |row, _| {
                        row.child(Knob::new(
                            "song-level",
                            SONG_LEVEL,
                            self.settings.song_db,
                            px(30.),
                            cx.listener(|this, value: &f32, _, cx| {
                                this.set_song_level(*value, cx);
                            }),
                        ))
                    }),
            )
            .when_some(song, |panel, song| {
                panel
                    .when(!song.notes.is_empty(), |panel| {
                        panel.child(self.render_tab(song, cx))
                    })
                    .child(self.render_seek(song, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{TAB_PLAYHEAD, TabNote, clock, tab_share};
    use std::sync::Arc;

    fn song(position: f32) -> super::Song {
        let note = |start, end, midi| TabNote {
            start,
            end,
            midi,
            string: 0,
            fret: 0,
        };
        super::Song {
            name: "Test".into(),
            audio: Arc::from(vec![0.0; 16]),
            notes: vec![note(1.0, 1.5, 64), note(2.0, 2.5, 67)],
            position,
            playing: false,
        }
    }

    fn pitches(song: &super::Song) -> (Option<i32>, Option<i32>) {
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
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(65.9), "1:05");
    }

    #[test]
    fn notes_reach_the_playhead_as_they_sound() {
        assert_eq!(tab_share(10.0, 10.0), TAB_PLAYHEAD);
        assert!(tab_share(11.0, 10.0) > TAB_PLAYHEAD);
    }
}
