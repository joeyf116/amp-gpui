//! Draws the song panel: its controls, the scrolling tab, and the seek bar.

use super::{
    Rig,
    song::{Deck, Repeat, Song},
};
use crate::{
    art::PRINT_FONT,
    knob::Knob,
    style::{fog, hairline, moss},
};
use amp_dsp::ParamSpec;
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::sync::atomic::Ordering;

/// Seconds of song the tab shows at once.
const TAB_SPAN: f32 = 6.0;
/// How far along the tab the playhead sits.
const TAB_PLAYHEAD: f32 = 0.12;
const TAB_LINE: f32 = 15.0;
const TAB_STRINGS: usize = 6;
const CHORD_LANE: f32 = 18.0;
const SEEK_HEIGHT: f32 = 6.0;
const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const SONG_LEVEL: ParamSpec = ParamSpec {
    id: "song_level",
    name: "Song",
    unit: "dB",
    min: -40.0,
    max: 0.0,
    default: -12.0,
};

fn clock(seconds: f32) -> String {
    let whole = seconds.max(0.0) as u32;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// How far along the tab a moment falls, with `position` at the playhead.
fn tab_share(time: f32, position: f32) -> f32 {
    TAB_PLAYHEAD + (time - position) / TAB_SPAN
}

/// A small control with a value between two step buttons.
fn stepper(
    id: &'static str,
    label: &'static str,
    value: String,
    cx: &mut Context<Rig>,
    step: impl Fn(&mut Rig, f32, &mut Context<Rig>) + Copy + 'static,
) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(div().text_xs().text_color(fog()).child(label))
        .child(
            Button::new((id, 0_usize))
                .small()
                .ghost()
                .label("\u{2212}")
                .on_click(cx.listener(move |this, _, _, cx| step(this, -1.0, cx))),
        )
        .child(div().w(px(38.)).text_center().text_sm().child(value))
        .child(
            Button::new((id, 1_usize))
                .small()
                .ghost()
                .label("+")
                .on_click(cx.listener(move |this, _, _, cx| step(this, 1.0, cx))),
        )
}

impl Rig {
    fn render_seek(&self, song: &Song, cx: &mut Context<Self>) -> impl IntoElement {
        let length = song.length().max(0.001);
        let share = song.position / length;
        let span = match song.repeat {
            Repeat::On(start, end) => Some((start / length, end / length)),
            _ => None,
        };
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
                if let Some((start, end)) = span {
                    let mut looped = frame;
                    looped.origin.x += frame.size.width * start;
                    looped.size.width = frame.size.width * (end - start);
                    window.paint_quad(fill(looped, filled.opacity(0.35)));
                }
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

    fn render_tab(&self, song: &Song, cx: &mut Context<Self>) -> impl IntoElement {
        let position = song.position;
        let shown = |time: f32| (0.0..=1.0).contains(&tab_share(time, position));
        let beats: Vec<f32> = song
            .beats
            .iter()
            .copied()
            .filter(|beat| shown(*beat))
            .map(|beat| tab_share(beat, position))
            .collect();
        let line = hairline();
        let head = moss();
        let lines = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let height = bounds.size.height;
                for beat in &beats {
                    let x = bounds.left() + bounds.size.width * *beat;
                    window.paint_quad(fill(
                        Bounds::new(point(x, bounds.top()), size(px(1.), height)),
                        line.opacity(0.4),
                    ));
                }
                for string in 0..TAB_STRINGS {
                    let y = bounds.top() + px(CHORD_LANE + TAB_LINE * (string as f32 + 0.5));
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.))),
                        line,
                    ));
                }
                let x = bounds.left() + bounds.size.width * TAB_PLAYHEAD;
                window.paint_quad(fill(
                    Bounds::new(point(x, bounds.top()), size(px(2.), height)),
                    head,
                ));
            },
        )
        .absolute()
        .size_full();
        let chords = song
            .chords
            .iter()
            .filter(|chord| {
                tab_share(chord.end, position) > 0.0 && tab_share(chord.start, position) < 1.0
            })
            .map(|chord| {
                div()
                    .absolute()
                    .left(relative(tab_share(chord.start, position).max(0.0)))
                    .top_0()
                    .pl(px(3.))
                    .text_size(px(12.))
                    .font_family(PRINT_FONT)
                    .font_semibold()
                    .child(chord.name.clone())
            });
        let first = song
            .notes
            .partition_point(|note| note.end < position - TAB_SPAN * TAB_PLAYHEAD);
        let last = song
            .notes
            .partition_point(|note| note.start < position + TAB_SPAN);
        let background = cx.theme().background;
        let picked = cx.theme().warning;
        let now_start = song.now_and_next().0.map(|note| note.start);
        let notes: Vec<_> = song.notes[first..last.max(first)]
            .iter()
            .enumerate()
            .map(|(offset, note)| {
                let index = first + offset;
                let lit = now_start == Some(note.start);
                let selected = song.selected == Some(index);
                let color = if selected {
                    picked
                } else if lit || note.hit {
                    moss()
                } else {
                    fog()
                };
                div()
                    .id(("tab-note", index))
                    .absolute()
                    .left(relative(tab_share(note.start, position)))
                    .top(px(CHORD_LANE + TAB_LINE * note.string as f32))
                    .w(relative((note.end - note.start) / TAB_SPAN))
                    .h(px(TAB_LINE))
                    .cursor_pointer()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(TAB_LINE / 2.0 - 1.0))
                            .h(px(3.))
                            .bg(color.opacity(if lit || selected { 0.7 } else { 0.3 })),
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
                            .text_color(color)
                            .child(note.fret.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_song_note(index, cx);
                    }))
            })
            .collect();
        div()
            .relative()
            .w_full()
            .h(px(CHORD_LANE + TAB_LINE * TAB_STRINGS as f32))
            .overflow_hidden()
            .child(lines)
            .children(chords)
            .children(notes)
    }

    /// Fixes for the note picked on the tab.
    fn render_note_fixes(&self, song: &Song, cx: &mut Context<Self>) -> impl IntoElement {
        let note = song.selected.and_then(|index| song.notes.get(index));
        h_flex()
            .gap_2()
            .items_center()
            .when_some(note, |row, note| {
                let name = NOTES[note.midi.rem_euclid(12) as usize];
                let octave = note.midi.div_euclid(12) - 1;
                row.child(
                    div()
                        .text_sm()
                        .child(format!("{name}{octave}, fret {}", note.fret)),
                )
                .child(
                    Button::new("note-down")
                        .small()
                        .ghost()
                        .label("Lower")
                        .on_click(cx.listener(|this, _, _, cx| this.nudge_song_note(-1, cx))),
                )
                .child(
                    Button::new("note-up")
                        .small()
                        .ghost()
                        .label("Raise")
                        .on_click(cx.listener(|this, _, _, cx| this.nudge_song_note(1, cx))),
                )
                .child(
                    Button::new("note-string")
                        .small()
                        .ghost()
                        .label("Other string")
                        .on_click(cx.listener(|this, _, _, cx| this.cycle_song_string(cx))),
                )
                .child(
                    Button::new("note-delete")
                        .small()
                        .ghost()
                        .label("Delete")
                        .on_click(cx.listener(|this, _, _, cx| this.delete_song_note(cx))),
                )
            })
    }

    fn render_song_practice(&self, song: &Song, cx: &mut Context<Self>) -> impl IntoElement {
        let loop_label = match song.repeat {
            Repeat::Off => "Loop from here",
            Repeat::Marking(_) => "Loop to here",
            Repeat::On(..) => "Clear loop",
        };
        let looping = song.repeat != Repeat::Off;
        let key_label = match song.semitones {
            0 => "0".to_owned(),
            semitones => format!("{semitones:+}"),
        };
        let song_key = song.key;
        h_flex()
            .gap_3()
            .items_center()
            .child(stepper(
                "song-speed",
                "Speed",
                format!("{:.0}%", song.speed * 100.0),
                cx,
                |this, step, cx| this.step_song_speed(step, cx),
            ))
            .child(stepper(
                "song-key",
                "Key",
                key_label,
                cx,
                |this, step, cx| this.step_song_key(step as i32, cx),
            ))
            .child(
                Button::new("song-loop")
                    .small()
                    .outline()
                    .label(loop_label)
                    .tooltip("Marks where the loop starts and ends, snapped to the beat")
                    .on_click(cx.listener(|this, _, _, cx| this.mark_song_loop(cx))),
            )
            .child(
                Button::new("song-trainer")
                    .small()
                    .label("Speed up each pass")
                    .tooltip("Raises the speed by 5% after each pass of the loop")
                    .when(song.trainer, |button| button.outline())
                    .when(!song.trainer, |button| button.ghost())
                    .disabled(!looping)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_song_trainer(cx))),
            )
            .child(div().flex_1())
            .when_some(song_key, |row, key| {
                row.child(div().text_sm().text_color(fog()).child(key.name()))
                    .child(
                        Button::new("song-use-key")
                            .small()
                            .ghost()
                            .label("Use on neck")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.use_song_key(key, window, cx);
                            })),
                    )
            })
            .child(
                Button::new("song-export")
                    .small()
                    .ghost()
                    .label("Save tab")
                    .tooltip("Writes text tab and a MIDI file to a folder")
                    .on_click(cx.listener(|this, _, window, cx| this.export_song(window, cx))),
            )
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
        let score = song
            .and_then(|song| song.score().or(song.last_pass))
            .map(|share| format!("You played {:.0}%", share * 100.0));
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
                    .when_some(score, |row, score| {
                        row.child(div().text_sm().text_color(moss()).child(score))
                    })
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
                    .child(self.render_song_practice(song, cx))
                    .when(!song.notes.is_empty(), |panel| {
                        panel.child(self.render_tab(song, cx))
                    })
                    .child(self.render_seek(song, cx))
                    .when(song.selected.is_some(), |panel| {
                        panel.child(self.render_note_fixes(song, cx))
                    })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{TAB_PLAYHEAD, clock, tab_share};

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
