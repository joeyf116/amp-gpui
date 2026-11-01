//! The tuner and metronome, side by side in one panel. The tuner reads like a
//! needle gauge, with the note under it and the six strings below. The click
//! itself runs on the audio thread, so this only sends it settings.

use super::{Overlay, Rig};
use crate::{
    art::PRINT_FONT,
    knob::{Knob, disc},
    style::{fog, glow, hairline, moss, sheet},
};
use amp_dsp::{Click, ParamSpec, detect_pitch};
use amp_engine::{Command, ENGINE_RATE};
use gpui_kit::{
    component::{
        ActiveTheme, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{f32::consts::PI, time::Instant};

/// Enough audio to catch the lowest note the tuner looks for.
const TUNER_FRAMES: usize = 6_000;
/// The longer stretch kept for picking out chords and reading strings.
const LISTEN_FRAMES: usize = 16_384;
const IN_TUNE_CENTS: f32 = 3.0;
const SCALE_CENTS: f32 = 50.0;
/// How far the needle swings either side of straight up, in radians.
const NEEDLE_SWING: f32 = 0.42 * PI;
/// How much of the way the needle moves to its target on each meter tick.
const NEEDLE_EASE: f32 = 0.45;
const GAUGE_SIZE: (f32, f32) = (300.0, 104.0);
const STRINGS: [(&str, i32); 6] = [("E", 2), ("A", 2), ("D", 3), ("G", 3), ("B", 3), ("E", 4)];
const BPM_RANGE: (f32, f32) = (30.0, 300.0);
const BEATS_RANGE: (u32, u32) = (1, 12);
/// Tapping again after this long starts a new count.
const TAP_RESET: f32 = 2.0;
/// How many of the latest taps set the tempo.
const TAPS: usize = 6;
const CLICK_LEVEL: ParamSpec = ParamSpec {
    id: "click_level",
    name: "Level",
    unit: "dB",
    min: -40.0,
    max: 0.0,
    default: -12.0,
};

/// The tempo a run of taps implies, from the time of each tap in seconds.
fn tapped_bpm(taps: &[f32]) -> Option<f32> {
    let (first, last) = (taps.first()?, taps.last()?);
    let span = last - first;
    (taps.len() >= 2 && span > 0.0).then(|| 60.0 * (taps.len() - 1) as f32 / span)
}

/// What to do about a reading, in words.
fn tuning_hint(cents: f32) -> &'static str {
    if cents.abs() <= IN_TUNE_CENTS {
        "In tune"
    } else if cents < 0.0 {
        "Flat, tune up"
    } else {
        "Sharp, tune down"
    }
}

impl Rig {
    /// Opens or closes the panel. The tuner listens only while it's open.
    pub(super) fn toggle_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::Tools {
            self.close_overlay(window, cx);
        } else {
            self.overlay = Overlay::Tools;
            cx.notify();
        }
    }

    /// Follows the panel opening or closing, however that happened.
    pub(super) fn sync_tuner(&mut self) {
        let open = self.overlay == Overlay::Tools;
        if open != self.tuner_open {
            self.tuner_open = open;
            self.pitch = None;
            self.tuner_samples.clear();
            self.apply_levels();
        }
    }

    pub(super) fn read_tuner(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.read_tuner(&mut self.tuner_samples);
            let extra = self.tuner_samples.len().saturating_sub(LISTEN_FRAMES);
            self.tuner_samples.drain(..extra);
            let recent = self.tuner_samples.len().saturating_sub(TUNER_FRAMES);
            let heard = detect_pitch(&self.tuner_samples[recent..], ENGINE_RATE);
            // Hold the last reading through a gap, so the needle doesn't flicker
            // as a note decays.
            if heard.is_some() {
                self.pitch = heard;
            }
            self.follow_song(heard);
            let (strings, learned) = self.string_stiffness();
            let finished = self.fretboard.hearing.hear(
                &self.tuner_samples,
                heard,
                self.smoke_phase,
                &strings,
                learned,
            );
            if finished.is_some() {
                self.settings.string_stiffness = finished;
                self.settings_save_at = Some(self.smoke_phase);
            }
        }
        let target = self.pitch.map_or(0.0, |pitch| pitch.cents);
        self.needle += (target - self.needle) * NEEDLE_EASE;
    }

    pub(super) fn click(&self) -> Click {
        Click {
            on: self.metronome_on,
            bpm: self.settings.bpm,
            beats: self.settings.beats,
            level: 10.0_f32.powf(self.settings.click_db / 20.0),
        }
    }

    pub(super) fn send_click(&mut self, cx: &mut Context<Self>) {
        let click = self.click();
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetClick(click));
        }
        self.settings_save_at = Some(self.smoke_phase + super::SETTINGS_SAVE_DELAY);
        cx.notify();
    }

    pub(super) fn toggle_metronome(&mut self, cx: &mut Context<Self>) {
        self.metronome_on = !self.metronome_on;
        self.send_click(cx);
    }

    fn set_bpm(&mut self, bpm: f32, cx: &mut Context<Self>) {
        self.settings.bpm = bpm.round().clamp(BPM_RANGE.0, BPM_RANGE.1);
        self.send_click(cx);
    }

    fn step_beats(&mut self, step: i32, cx: &mut Context<Self>) {
        self.settings.beats = self
            .settings
            .beats
            .saturating_add_signed(step)
            .clamp(BEATS_RANGE.0, BEATS_RANGE.1);
        self.send_click(cx);
    }

    fn tap(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self
            .taps
            .last()
            .is_some_and(|last| (now - *last).as_secs_f32() > TAP_RESET)
        {
            self.taps.clear();
        }
        self.taps.push(now);
        let extra = self.taps.len().saturating_sub(TAPS);
        self.taps.drain(..extra);
        let first = self.taps[0];
        let times: Vec<f32> = self
            .taps
            .iter()
            .map(|tap| (*tap - first).as_secs_f32())
            .collect();
        if let Some(bpm) = tapped_bpm(&times) {
            self.set_bpm(bpm, cx);
        }
    }

    fn render_gauge(&self, in_tune: bool, cx: &App) -> impl IntoElement {
        let reading = self.pitch.is_some();
        let cents = self.needle.clamp(-SCALE_CENTS, SCALE_CENTS);
        let color = if in_tune { moss() } else { cx.theme().warning };
        let ink = fog();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let pivot = point(bounds.center().x, bounds.bottom() - px(8.0));
                let reach = bounds.size.height - px(26.0);
                let at = |angle: f32, length: Pixels| {
                    point(
                        pivot.x + length * angle.sin(),
                        pivot.y - length * angle.cos(),
                    )
                };
                let line = |from, to, width: f32, color: Hsla, window: &mut Window| {
                    let mut path = PathBuilder::stroke(px(width));
                    path.move_to(from);
                    path.line_to(to);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                };
                for tick in -5_i32..=5 {
                    let angle = tick as f32 / 5.0 * NEEDLE_SWING;
                    let major = tick % 5 == 0;
                    let inner = reach - px(if major { 16.0 } else { 9.0 });
                    let tone = if tick == 0 { moss() } else { ink };
                    line(
                        at(angle, inner),
                        at(angle, reach),
                        if major { 2.0 } else { 1.0 },
                        tone,
                        window,
                    );
                }
                if reading {
                    let angle = cents / SCALE_CENTS * NEEDLE_SWING;
                    line(pivot, at(angle, reach - px(4.0)), 2.5, color, window);
                    if let Some(hub) = disc(pivot, px(6.0)) {
                        window.paint_path(hub, color);
                    }
                }
            },
        )
        .w(px(GAUGE_SIZE.0))
        .h(px(GAUGE_SIZE.1))
    }

    fn render_tuner_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let pitch = self.pitch;
        let cents = pitch.map_or(0.0, |pitch| pitch.cents);
        let in_tune = pitch.is_some() && cents.abs() <= IN_TUNE_CENTS;
        let color = match pitch {
            None => fog(),
            Some(_) if in_tune => moss(),
            Some(_) => cx.theme().warning,
        };
        let hint = match pitch {
            Some(_) => tuning_hint(cents).to_owned(),
            None if live => "Play one open string".to_owned(),
            None => "Start monitoring to tune".to_owned(),
        };
        let detail = pitch.map_or(String::new(), |pitch| {
            format!("{:+.0} cents, {:.1} Hz", pitch.cents, pitch.hertz)
        });
        let strings = STRINGS.iter().map(|(note, octave)| {
            let lit = pitch.is_some_and(|pitch| pitch.note == *note && pitch.octave == *octave);
            div()
                .size(px(34.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(if lit { color } else { hairline() })
                .text_sm()
                .text_color(if lit { color } else { fog() })
                .child(*note)
        });
        let muted = self.settings.tuner_mute;
        v_flex()
            .flex_1()
            .items_center()
            .gap_2()
            .p_4()
            .border_r_1()
            .border_color(hairline())
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(div().font_semibold().child("Tuner"))
                    .child(
                        Button::new("tuner-mute")
                            .small()
                            .label("Mute while tuning")
                            .when(muted, |button| button.outline())
                            .when(!muted, |button| button.ghost())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.tuner_mute = !this.settings.tuner_mute;
                                this.apply_levels();
                                this.save_settings(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .child(self.render_gauge(in_tune, cx))
                    .child(
                        h_flex()
                            .absolute()
                            .bottom(px(-4.))
                            .left_0()
                            .right_0()
                            .h(px(10.))
                            .justify_between()
                            .text_xs()
                            .text_color(fog())
                            .child("Flat")
                            .child("Sharp"),
                    ),
            )
            .child(
                h_flex()
                    .items_baseline()
                    .gap_1()
                    .child(
                        div()
                            .font_family(PRINT_FONT)
                            .font_semibold()
                            .text_size(px(40.))
                            .line_height(px(44.))
                            .text_color(color)
                            .child(pitch.map_or("--", |pitch| pitch.note)),
                    )
                    .when_some(pitch, |row, pitch| {
                        row.child(
                            div()
                                .text_sm()
                                .text_color(fog())
                                .child(pitch.octave.to_string()),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .h(px(18.))
                    .child(div().text_sm().text_color(color).child(hint))
                    .child(div().text_xs().text_color(fog()).child(detail)),
            )
            .child(h_flex().gap_2().children(strings))
    }

    fn render_metronome_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let on = self.metronome_on;
        let (bpm, beats) = (self.settings.bpm, self.settings.beats);
        let lights = (1..=beats).map(|beat| {
            let lit = self.beat == beat;
            let size = if beat == 1 { 16.0 } else { 12.0 };
            div()
                .size(px(size))
                .rounded_full()
                .border_1()
                .border_color(hairline())
                .when(lit, |light| light.bg(moss()).shadow(vec![glow(0.7, 10.0)]))
        });
        let wheel = cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
            let notches = event.delta.pixel_delta(px(20.0)).y / px(20.0);
            this.set_bpm(this.settings.bpm + notches, cx);
        });
        v_flex()
            .w(px(330.))
            .flex_shrink_0()
            .gap_3()
            .p_4()
            .child(div().font_semibold().child("Metronome"))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        Button::new("bpm-down").label("\u{2212}").ghost().on_click(
                            cx.listener(move |this, _, _, cx| this.set_bpm(bpm - 1.0, cx)),
                        ),
                    )
                    .child(
                        v_flex()
                            .id("bpm")
                            .items_center()
                            .on_scroll_wheel(wheel)
                            .child(
                                div()
                                    .font_family(PRINT_FONT)
                                    .font_semibold()
                                    .text_size(px(52.))
                                    .line_height(px(56.))
                                    .child(format!("{bpm:.0}")),
                            )
                            .child(div().text_xs().text_color(fog()).child("BPM")),
                    )
                    .child(
                        Button::new("bpm-up").label("+").ghost().on_click(
                            cx.listener(move |this, _, _, cx| this.set_bpm(bpm + 1.0, cx)),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .justify_center()
                    .items_center()
                    .gap_2()
                    .h(px(20.))
                    .children(lights),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div().flex_1().child(
                            Button::new("metronome")
                                .w_full()
                                .label(if on { "Stop" } else { "Start" })
                                .tooltip("Press K")
                                .when(!on, |button| button.primary())
                                .when(on, |button| button.outline())
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_metronome(cx))),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            Button::new("tap")
                                .w_full()
                                .outline()
                                .label("Tap tempo")
                                .on_click(cx.listener(|this, _, _, cx| this.tap(cx))),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("beats-down")
                                    .small()
                                    .ghost()
                                    .label("\u{2212}")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.step_beats(-1, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .w(px(112.))
                                    .text_center()
                                    .whitespace_nowrap()
                                    .child(format!("{beats} beats per bar")),
                            )
                            .child(Button::new("beats-up").small().ghost().label("+").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.step_beats(1, cx);
                                }),
                            )),
                    )
                    .child(Knob::new(
                        "click-level",
                        CLICK_LEVEL,
                        self.settings.click_db,
                        px(30.),
                        cx.listener(|this, value: &f32, _, cx| {
                            this.settings.click_db = *value;
                            this.send_click(cx);
                        }),
                    )),
            )
            .when(!live, |card| {
                card.child(
                    div()
                        .text_xs()
                        .text_color(fog())
                        .child("Start monitoring to hear the click."),
                )
            })
    }

    pub(super) fn render_tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = self.fretboard.song.compact;
        v_flex()
            .id("tools")
            .occlude()
            .absolute()
            .top(px(8.))
            .bottom(px(8.))
            .left(px(16.))
            .right(px(16.))
            .overflow_y_scroll()
            .rounded_lg()
            .border_1()
            .border_color(hairline())
            .bg(sheet())
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(hairline())
                    .child(div().font_semibold().child("Practice tools"))
                    .child(
                        Button::new("toggle-tuner")
                            .small()
                            .ghost()
                            .label(if compact {
                                "Show tuner and metronome"
                            } else {
                                "Hide tuner and metronome"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.fretboard.song.compact = !this.fretboard.song.compact;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("close-tools")
                            .small()
                            .ghost()
                            .label("Close")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_overlay(window, cx);
                            })),
                    ),
            )
            .when(!compact, |panel| {
                panel.child(
                    h_flex()
                        .flex_shrink_0()
                        .items_start()
                        .child(self.render_tuner_card(cx))
                        .child(self.render_metronome_card(cx)),
                )
            })
            .child(self.render_fretboard(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{tapped_bpm, tuning_hint};

    #[test]
    fn taps_average_out_to_a_tempo() {
        assert_eq!(tapped_bpm(&[0.0, 0.5, 1.0, 1.5]), Some(120.0));
        assert_eq!(tapped_bpm(&[0.0, 1.0]), Some(60.0));
    }

    #[test]
    fn one_tap_says_nothing_about_tempo() {
        assert_eq!(tapped_bpm(&[0.0]), None);
        assert_eq!(tapped_bpm(&[]), None);
    }

    #[test]
    fn the_hint_says_which_way_to_turn_the_peg() {
        assert_eq!(tuning_hint(-20.0), "Flat, tune up");
        assert_eq!(tuning_hint(20.0), "Sharp, tune down");
        assert_eq!(tuning_hint(1.0), "In tune");
    }
}
