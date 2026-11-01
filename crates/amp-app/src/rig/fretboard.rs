//! A fretboard that shows a key and scale, and lights up what's being played.
//! Notes come from the tuner, which hears one note at a time, so it shows the
//! exact pitch at every place it can be fretted, with a short trail behind it.

use super::{Choice, Rig};
use crate::style::{fog, glow, hairline, moss};
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, Sizable, StyledExt,
        button::Button,
        h_flex,
        select::{Select, SelectEvent, SelectState},
        v_flex,
    },
    *,
};

/// Open strings as MIDI notes, high E first, the way a player looks down at them.
const TUNING: [i32; 6] = [64, 59, 55, 50, 45, 40];
const FRETS: i32 = 15;
const INLAYS: [i32; 6] = [3, 5, 7, 9, 12, 15];
const STRING_GAP: f32 = 28.0;
const NUT_SHARE: f32 = 0.05;
const DOT: f32 = 22.0;
/// How long a played note stays lit, in seconds.
const TRAIL: f32 = 1.4;
const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const INTERVALS: [&str; 12] = [
    "1", "b2", "2", "b3", "3", "4", "b5", "5", "b6", "6", "b7", "7",
];
const SCALES: [(&str, &[i32]); 12] = [
    ("Minor pentatonic", &[0, 3, 5, 7, 10]),
    ("Major pentatonic", &[0, 2, 4, 7, 9]),
    ("Blues", &[0, 3, 5, 6, 7, 10]),
    ("Major (Ionian)", &[0, 2, 4, 5, 7, 9, 11]),
    ("Natural minor (Aeolian)", &[0, 2, 3, 5, 7, 8, 10]),
    ("Harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("Melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("Lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("Locrian", &[0, 1, 3, 5, 6, 8, 10]),
];

/// How far up the neck a fret sits, from 0 at the nut to 1 at the last fret.
/// Spaced like a real neck, with frets closing up toward the body.
fn fret_share(fret: f32) -> f32 {
    let along = |fret: f32| 1.0 - 2.0_f32.powf(-fret / 12.0);
    along(fret) / along(FRETS as f32)
}

/// Every place a pitch can be fretted, as string and fret.
fn positions(midi: i32) -> impl Iterator<Item = (usize, i32)> {
    TUNING
        .iter()
        .enumerate()
        .map(move |(string, open)| (string, midi - open))
        .filter(|(_, fret)| (0..=FRETS).contains(fret))
}

/// How far `midi` sits above the key's root, within an octave.
fn degree(midi: i32, key: usize) -> usize {
    (midi - key as i32).rem_euclid(12) as usize
}

fn scale_index(name: &str) -> usize {
    SCALES
        .iter()
        .position(|(scale, _)| *scale == name)
        .unwrap_or(0)
}

/// The MIDI note nearest a frequency.
fn midi_note(hertz: f32) -> i32 {
    (12.0 * (hertz / 440.0).log2() + 69.0).round() as i32
}

fn in_scale(scale: &[i32], degree: usize) -> bool {
    scale.contains(&(degree as i32))
}

struct Played {
    midi: i32,
    at: f32,
}

/// The fretboard's pickers and the trail of recent notes.
pub(super) struct Fretboard {
    key_select: Entity<SelectState<Vec<Choice>>>,
    scale_select: Entity<SelectState<Vec<Choice>>>,
    played: Vec<Played>,
}

impl Fretboard {
    pub(super) fn new(
        key: usize,
        scale: &str,
        window: &mut Window,
        cx: &mut Context<Rig>,
    ) -> (Self, Vec<Subscription>) {
        let choices = |names: Vec<&str>| {
            names
                .into_iter()
                .map(|name| Choice {
                    value: name.to_owned(),
                    title: name.to_owned().into(),
                })
                .collect::<Vec<_>>()
        };
        let keys = choices(NOTES.to_vec());
        let scales = choices(SCALES.iter().map(|(name, _)| *name).collect());
        let key_select =
            cx.new(|cx| SelectState::new(keys, Some(IndexPath::new(key % 12)), window, cx));
        let scale = scale_index(scale);
        let scale_select =
            cx.new(|cx| SelectState::new(scales, Some(IndexPath::new(scale)), window, cx));
        let subscriptions = vec![
            cx.subscribe(
                &key_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        this.settings.key = NOTES.iter().position(|note| note == name).unwrap_or(0);
                        this.save_settings(cx);
                    }
                },
            ),
            cx.subscribe(
                &scale_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        this.settings.scale = name.clone();
                        this.save_settings(cx);
                    }
                },
            ),
        ];
        let board = Self {
            key_select,
            scale_select,
            played: Vec::new(),
        };
        (board, subscriptions)
    }

    /// Lights `hertz`, or keeps it lit if it's still sounding.
    pub(super) fn play(&mut self, hertz: f32, now: f32) {
        let midi = midi_note(hertz);
        self.played.retain(|note| note.midi != midi);
        self.played.push(Played { midi, at: now });
    }

    pub(super) fn fade(&mut self, now: f32) {
        self.played.retain(|note| now - note.at < TRAIL);
    }
}

/// One marker on the neck, centered where a finger would go.
fn marker(
    string: usize,
    fret: i32,
    label: SharedString,
    background: Hsla,
    border: Hsla,
    ink: Hsla,
) -> Div {
    let x = if fret == 0 {
        NUT_SHARE / 2.0
    } else {
        NUT_SHARE + (1.0 - NUT_SHARE) * (fret_share(fret as f32 - 0.5))
    };
    div()
        .absolute()
        .left(relative(x))
        .top(px(STRING_GAP * string as f32 + (STRING_GAP - DOT) / 2.0))
        .ml(px(-DOT / 2.0))
        .size(px(DOT))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(background)
        .text_size(px(10.))
        .text_color(ink)
        .font_semibold()
        .child(label)
}

impl Rig {
    fn fret_label(&self, midi: i32) -> SharedString {
        let degree = degree(midi, self.settings.key);
        if self.settings.fret_intervals {
            INTERVALS[degree].into()
        } else {
            NOTES[midi.rem_euclid(12) as usize].into()
        }
    }

    fn render_neck(&self, cx: &App) -> impl IntoElement {
        let key = self.settings.key;
        let (_, scale) = SCALES[scale_index(&self.settings.scale)];
        let height = STRING_GAP * TUNING.len() as f32;
        let root = moss();
        let surface = hsla(0.42, 0.04, 0.11, 1.0);
        let on_root = cx.theme().primary_foreground;
        let tone = hsla(0.42, 0.05, 0.3, 1.0);
        let text = cx.theme().foreground;
        let out_of_key = cx.theme().warning;
        let neck = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let left = bounds.left();
                let width = bounds.size.width;
                let top = bounds.top();
                let nut = width * NUT_SHARE;
                let room = width - nut;
                let x_of = |fret: f32| left + nut + room * fret_share(fret);
                window.paint_quad(fill(
                    Bounds::new(point(left + nut, top), size(room, bounds.size.height)),
                    surface,
                ));
                window.paint_quad(fill(
                    Bounds::new(
                        point(left + nut - px(4.), top),
                        size(px(4.), bounds.size.height),
                    ),
                    hsla(0.42, 0.04, 0.72, 1.0),
                ));
                for fret in 1..=FRETS {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(x_of(fret as f32) - px(1.), top),
                            size(px(2.), bounds.size.height),
                        ),
                        hsla(0.42, 0.04, 0.4, 1.0),
                    ));
                }
                let middle = top + bounds.size.height / 2.0;
                for fret in INLAYS {
                    let x = x_of(fret as f32 - 0.5);
                    let spots: &[f32] = if fret == 12 {
                        &[-STRING_GAP, STRING_GAP]
                    } else {
                        &[0.0]
                    };
                    for spot in spots {
                        let center = point(x, middle + px(*spot));
                        if let Some(path) = crate::knob::disc(center, px(5.0)) {
                            window.paint_path(path, hsla(0.42, 0.04, 0.2, 1.0));
                        }
                    }
                }
                for string in 0..TUNING.len() {
                    let y = top + px(STRING_GAP * (string as f32 + 0.5));
                    let thick = 1.0 + string as f32 * 0.25;
                    window.paint_quad(fill(
                        Bounds::new(point(left + nut, y), size(room, px(thick))),
                        hsla(0.42, 0.03, 0.6, 1.0),
                    ));
                }
            },
        )
        .absolute()
        .size_full();

        let mut dots: Vec<Div> = Vec::new();
        for (string, open) in TUNING.iter().enumerate() {
            for fret in 0..=FRETS {
                let midi = open + fret;
                let step = degree(midi, key);
                if in_scale(scale, step) {
                    let is_root = step == 0;
                    dots.push(marker(
                        string,
                        fret,
                        self.fret_label(midi),
                        if is_root { root } else { tone },
                        if is_root { root } else { hairline() },
                        if is_root { on_root } else { text },
                    ));
                }
            }
        }
        let now = self.smoke_phase;
        let lit = self.fretboard.played.iter().flat_map(|note| {
            let fading = (1.0 - (now - note.at) / TRAIL).clamp(0.0, 1.0);
            let color = if in_scale(scale, degree(note.midi, key)) {
                hsla(0.4, 0.26, 0.93, 1.0)
            } else {
                out_of_key
            };
            let label = self.fret_label(note.midi);
            positions(note.midi).map(move |(string, fret)| {
                marker(
                    string,
                    fret,
                    label.clone(),
                    color.opacity(fading),
                    color,
                    on_root.opacity(fading),
                )
                .shadow(vec![glow(0.8 * fading, 14.0)])
            })
        });
        let dots: Vec<Div> = dots.into_iter().chain(lit).collect();
        let numbers: Vec<_> = INLAYS
            .iter()
            .map(|fret| {
                div()
                    .absolute()
                    .left(relative(
                        NUT_SHARE + (1.0 - NUT_SHARE) * fret_share(*fret as f32 - 0.5),
                    ))
                    .ml(px(-10.))
                    .w(px(20.))
                    .text_center()
                    .text_xs()
                    .text_color(fog())
                    .child(fret.to_string())
            })
            .collect();
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(height))
                    .child(neck)
                    .children(dots),
            )
            .child(div().relative().w_full().h(px(16.)).children(numbers))
    }

    pub(super) fn render_fretboard(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let intervals = self.settings.fret_intervals;
        let (_, scale) = SCALES[scale_index(&self.settings.scale)];
        let spelled = scale
            .iter()
            .map(|step| {
                if intervals {
                    INTERVALS[*step as usize]
                } else {
                    NOTES[(self.settings.key + *step as usize) % 12]
                }
            })
            .collect::<Vec<_>>()
            .join("   ");
        v_flex()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(hairline())
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(div().font_semibold().child("Fretboard"))
                    .child(
                        div()
                            .w(px(84.))
                            .child(Select::new(&self.fretboard.key_select).small()),
                    )
                    .child(
                        div()
                            .w(px(220.))
                            .child(Select::new(&self.fretboard.scale_select).small()),
                    )
                    .child(
                        Button::new("fret-labels")
                            .small()
                            .outline()
                            .label(if intervals {
                                "Showing intervals"
                            } else {
                                "Showing notes"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.fret_intervals = !this.settings.fret_intervals;
                                this.save_settings(cx);
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(fog()).child(spelled)),
            )
            .child(self.render_neck(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{SCALES, degree, fret_share, in_scale, midi_note, positions, scale_index};

    #[test]
    fn a_pitch_shows_everywhere_it_can_be_fretted() {
        // Middle A: 5th fret on the high E, 10th on the B, and so on down.
        let found: Vec<_> = positions(69).collect();
        assert_eq!(found, [(0, 5), (1, 10), (2, 14)]);
    }

    #[test]
    fn notes_off_the_neck_show_nowhere() {
        assert_eq!(positions(30).count(), 0);
        assert_eq!(positions(100).count(), 0);
    }

    #[test]
    fn degrees_count_up_from_the_key() {
        assert_eq!(degree(69, 9), 0);
        assert_eq!(degree(72, 9), 3);
        assert_eq!(degree(57, 0), 9);
    }

    #[test]
    fn the_a_minor_pentatonic_holds_c_but_not_c_sharp() {
        let (_, scale) = SCALES[scale_index("Minor pentatonic")];
        assert!(in_scale(scale, degree(72, 9)));
        assert!(!in_scale(scale, degree(73, 9)));
    }

    #[test]
    fn an_unknown_scale_falls_back_to_the_first() {
        assert_eq!(scale_index("no such scale"), 0);
    }

    #[test]
    fn frets_close_up_toward_the_body() {
        let first = fret_share(1.0) - fret_share(0.0);
        let last = fret_share(15.0) - fret_share(14.0);
        assert!(first > last);
        assert_eq!(fret_share(15.0), 1.0);
    }

    #[test]
    fn frequencies_land_on_the_nearest_note() {
        assert_eq!(midi_note(440.0), 69);
        assert_eq!(midi_note(82.41), 40);
        assert_eq!(midi_note(445.0), 69);
    }
}
