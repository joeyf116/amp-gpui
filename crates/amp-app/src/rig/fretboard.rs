//! A fretboard that shows a key and scale, and lights up what's being played.
//!
//! A pitch alone can't say where it was fretted, since most notes exist in
//! several places. So a single note is placed by two clues: where the hand has
//! been, and how stiff the string sounds, which differs between two places for
//! the same pitch. A chord is placed by finding a shape one hand could hold.

use super::{Choice, Rig, song::SongPanel};
use crate::{
    art::PRINT_FONT,
    style::{fog, glow, hairline, moss},
};
use amp_dsp::{MelodyNote, Pitch, SongKey, Spectrum, chord_name};
use amp_engine::ENGINE_RATE;
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, Sizable, StyledExt,
        button::Button,
        h_flex,
        select::{Select, SelectEvent, SelectState},
        v_flex,
    },
    prelude::FluentBuilder,
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
const TRAIL: f32 = 0.8;
/// How stiff each open string is on a typical 10-46 set, in the same order as
/// `TUNING`. Learning a guitar replaces these.
const STRING_STIFFNESS: [f32; 6] = [1.3e-5, 3.9e-5, 1.05e-4, 8.9e-5, 1.9e-4, 4.0e-4];
/// How far up the neck the hand reaches without moving, either side of its center.
const HAND_REACH: f32 = 1.5;
/// How heavily a stiffness mismatch counts against a place, in frets of hand
/// movement. A learned guitar earns much more trust than book values.
const STIFFNESS_WEIGHT: f32 = 3.0;
const LEARNED_WEIGHT: f32 = 8.0;
/// How quickly the hand estimate follows new notes.
const HAND_FOLLOW: f32 = 0.4;
/// A note has to ring this long before its stiffness can be read.
const SETTLE: f32 = 0.3;
/// The widest stretch a chord shape can use, in frets.
const CHORD_SPAN: i32 = 4;
/// What crossing to another string costs a melody, in frets of hand movement.
const STRING_CHANGE: f32 = 0.15;
/// What each fret up the neck costs, as players stay low until a line takes them up.
const NECK_HEIGHT: f32 = 0.12;
/// What an open string costs a melody, as players mostly fret a line.
const OPEN_STRING: f32 = 0.2;
/// How many frets past the first finger the hand reaches without moving.
const HAND_SPAN: i32 = 3;
/// How quickly a gap between notes eases a hand move, per second.
const MOVE_TIME: f32 = 3.0;
/// Clean readings of each open string needed to learn it.
const LEARN_READINGS: usize = 5;
const STRING_NAMES: [&str; 6] = ["high E", "B", "G", "D", "A", "low E"];
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
pub(super) fn positions(midi: i32) -> impl Iterator<Item = (usize, i32)> {
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

/// Picks where a single note was played. Stays near the hand, and lets a
/// stiffness reading pull it to another string when one is available.
fn place_note(
    midi: i32,
    hand: Option<f32>,
    stiffness: Option<f32>,
    strings: &[f32; 6],
    learned: bool,
) -> Option<(usize, i32)> {
    let weight = if learned {
        LEARNED_WEIGHT
    } else {
        STIFFNESS_WEIGHT
    };
    let cost = |(string, fret): &(usize, i32)| {
        let reach = match hand {
            _ if *fret == 0 => 0.5,
            Some(hand) => ((*fret as f32 - hand).abs() - HAND_REACH).max(0.0),
            // With nothing to go on, players start low on the neck.
            None => *fret as f32 * 0.2,
        };
        let sound = stiffness.map_or(0.0, |measured| {
            let expected = strings[*string] * 2.0_f32.powf(*fret as f32 / 6.0);
            (measured / expected).ln().abs() * weight
        });
        reach + sound
    };
    positions(midi).min_by(|a, b| cost(a).total_cmp(&cost(b)))
}

/// Finds the shape one hand could hold for `notes`, low to high, with each note
/// on its own string and the low notes on the low strings.
fn place_chord(notes: &[i32], hand: Option<f32>) -> Option<Vec<(usize, i32)>> {
    fn search(
        notes: &[i32],
        strings: std::ops::Range<usize>,
        chosen: &mut Vec<(usize, i32)>,
        hand: Option<f32>,
        best: &mut Option<(f32, Vec<(usize, i32)>)>,
    ) {
        let Some((note, rest)) = notes.split_first() else {
            let fretted = || {
                chosen
                    .iter()
                    .map(|(_, fret)| *fret)
                    .filter(|fret| *fret > 0)
            };
            let low = fretted().min().unwrap_or(0);
            let high = fretted().max().unwrap_or(0);
            if high - low > CHORD_SPAN {
                return;
            }
            let opens = chosen.iter().filter(|(_, fret)| *fret == 0).count();
            let center = (low + high) as f32 / 2.0;
            let away = hand.map_or(center * 0.3, |hand| (center - hand).abs() * 0.3);
            let cost = (high - low) as f32 + away - opens as f32 * 0.5;
            if best.as_ref().is_none_or(|(lowest, _)| cost < *lowest) {
                *best = Some((cost, chosen.clone()));
            }
            return;
        };
        // Strings run high to low, so a higher note takes a lower index.
        for string in strings.clone().rev() {
            let fret = note - TUNING[string];
            if (0..=FRETS).contains(&fret) {
                chosen.push((string, fret));
                search(rest, 0..string, chosen, hand, best);
                chosen.pop();
            }
        }
    }
    let mut best = None;
    search(notes, 0..TUNING.len(), &mut Vec::new(), hand, &mut best);
    best.map(|(_, shape)| shape)
}

/// Fills strings skipped inside a shape. An octave hides in the overtones of the
/// note below it, and a strum nearly always hits the strings in between, so each
/// gap takes the chord tone that sits under the hand.
fn fill_gaps(shape: &mut Vec<(usize, i32)>, notes: &[i32]) {
    let classes: Vec<i32> = notes.iter().map(|note| note.rem_euclid(12)).collect();
    let fretted = || shape.iter().map(|(_, fret)| *fret).filter(|fret| *fret > 0);
    let low = fretted().min().unwrap_or(0);
    let high = fretted().max().unwrap_or(0);
    let top = shape.iter().map(|(string, _)| *string).min().unwrap_or(0);
    let bottom = shape.iter().map(|(string, _)| *string).max().unwrap_or(0);
    let center = (low + high) as f32 / 2.0;
    let mut gaps = Vec::new();
    for (string, open) in TUNING.iter().enumerate().take(bottom).skip(top) {
        if shape.iter().any(|(taken, _)| *taken == string) {
            continue;
        }
        let in_reach = |fret: &i32| *fret == 0 || (low..=low.max(1) + CHORD_SPAN).contains(fret);
        let tone = (0..=FRETS)
            .filter(in_reach)
            .filter(|fret| classes.contains(&(open + fret).rem_euclid(12)))
            .min_by(|a, b| {
                // Down by the nut, players let open strings ring.
                let away = |fret: &i32| match fret {
                    0 if high <= CHORD_SPAN => 0.0,
                    _ => (*fret as f32 - center).abs() + 0.5,
                };
                away(a).total_cmp(&away(b))
            });
        gaps.extend(tone.map(|fret| (string, fret)));
    }
    shape.extend(gaps);
}

/// Brings a pitch onto the neck by octaves, so a line that runs past the
/// guitar's range keeps its shape.
fn fold_onto_neck(midi: i32) -> i32 {
    let (lowest, highest) = (TUNING[5], TUNING[0] + FRETS);
    let mut midi = midi;
    while midi < lowest {
        midi += 12;
    }
    while midi > highest {
        midi -= 12;
    }
    midi
}

/// Places a whole melody on the neck. With the full line known up front, the
/// answer can be the one that moves the hand least from first note to last,
/// rather than the best guess for each note as it comes. Each step weighs every
/// hand position that could reach the note, so a run settles into one position
/// instead of creeping up the neck a fret at a time.
pub(super) fn finger_line(notes: &[MelodyNote], pins: &[Option<usize>]) -> Vec<(usize, i32)> {
    /// A hand position, as the fret under the first finger, and a place it plays.
    type Spot = (i32, usize, i32);
    let spots: Vec<Vec<Spot>> = notes
        .iter()
        .zip(pins)
        .map(|(note, pin)| {
            let midi = fold_onto_neck(note.midi);
            let pinned = |(string, _): &(usize, i32)| pin.is_none_or(|pin| pin == *string);
            let any_pin_fits = positions(midi).any(|place| pinned(&place));
            positions(midi)
                .filter(move |place| !any_pin_fits || pinned(place))
                .flat_map(|(string, fret)| {
                    let hands = match fret {
                        // An open string leaves the hand free, wherever it is.
                        0 => 1..=FRETS - HAND_SPAN,
                        _ => (fret - HAND_SPAN).max(1)..=fret.min(FRETS - HAND_SPAN),
                    };
                    hands.map(move |hand| (hand, string, fret))
                })
                .collect()
        })
        .collect();
    let standing = |(hand, _, fret): &Spot| {
        *hand as f32 * NECK_HEIGHT + if *fret == 0 { OPEN_STRING } else { 0.0 }
    };
    let mut links: Vec<Vec<usize>> = Vec::with_capacity(notes.len());
    let mut costs: Vec<f32> = Vec::new();
    for (index, here) in spots.iter().enumerate() {
        if index == 0 {
            costs = here.iter().map(standing).collect();
            links.push(vec![0; here.len()]);
            continue;
        }
        // A gap gives the hand time to get there, so moving costs less.
        let gap = (notes[index].start - notes[index - 1].end).max(0.0);
        let scale = 1.0 / (1.0 + gap * MOVE_TIME);
        let mut next = Vec::with_capacity(here.len());
        let mut link = Vec::with_capacity(here.len());
        for spot in here {
            let (from, cost) = spots[index - 1]
                .iter()
                .zip(&costs)
                .map(|(before, cost)| {
                    let hand = spot.0.abs_diff(before.0) as f32 * scale;
                    cost + hand + spot.1.abs_diff(before.1) as f32 * STRING_CHANGE
                })
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((0, 0.0));
            next.push(cost + standing(spot));
            link.push(from);
        }
        costs = next;
        links.push(link);
    }
    let mut at = costs
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(index, _)| index);
    let mut line = vec![(0, 0); notes.len()];
    for index in (0..notes.len()).rev() {
        let (_, string, fret) = spots[index][at];
        line[index] = (string, fret);
        at = links[index][at];
    }
    line
}

fn median(readings: &mut [f32]) -> f32 {
    readings.sort_by(f32::total_cmp);
    readings[readings.len() / 2]
}

struct Played {
    string: usize,
    fret: i32,
    midi: i32,
    at: f32,
}

/// Learning a guitar's strings, one open string at a time, low E first.
struct Learn {
    string: usize,
    readings: Vec<f32>,
    strings: [f32; 6],
}

/// The fretboard's pickers, and what it has heard lately.
pub(super) struct Fretboard {
    key_select: Entity<SelectState<Vec<Choice>>>,
    scale_select: Entity<SelectState<Vec<Choice>>>,
    pub(super) hearing: Hearing,
    pub(super) song: SongPanel,
}

/// What the fretboard has heard lately.
#[derive(Default)]
pub(super) struct Hearing {
    played: Vec<Played>,
    /// Where the fretting hand seems to be, as a fret.
    hand: Option<f32>,
    chord: Option<(String, f32)>,
    /// The note set from the last look, as a chord has to hold for two.
    last_heard: Vec<i32>,
    /// The note ringing now, and when it started.
    ringing: Option<(i32, f32)>,
    measured: bool,
    learn: Option<Learn>,
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
            hearing: Hearing::default(),
            song: SongPanel::default(),
        };
        (board, subscriptions)
    }
}

impl Hearing {
    /// Takes in the latest audio, and returns a guitar's strings once learned.
    pub(super) fn hear(
        &mut self,
        samples: &[f32],
        pitch: Option<Pitch>,
        now: f32,
        strings: &[f32; 6],
        learned: bool,
    ) -> Option<[f32; 6]> {
        let spectrum = Spectrum::new(samples, ENGINE_RATE);
        let mut finished = None;
        if self.learn.is_some() {
            finished = self.learn_string(spectrum.as_ref(), pitch);
        } else if let Some(spectrum) = &spectrum {
            let heard = spectrum.notes();
            let chord = chord_name(&heard);
            match chord {
                Some(name) if heard == self.last_heard => {
                    self.show_chord(name, &heard, now);
                }
                Some(_) => self.last_heard = heard,
                None => {
                    self.last_heard.clear();
                    if let Some(pitch) = pitch {
                        self.show_note(spectrum, pitch, now, strings, learned);
                    }
                }
            }
        }
        if spectrum.is_none() {
            self.ringing = None;
        }
        self.played.retain(|note| now - note.at < TRAIL);
        if self.chord.as_ref().is_some_and(|(_, at)| now - at > TRAIL) {
            self.chord = None;
        }
        finished
    }

    fn follow_hand(&mut self, frets: impl Iterator<Item = i32>) {
        let frets: Vec<f32> = frets
            .filter(|fret| *fret > 0)
            .map(|fret| fret as f32)
            .collect();
        if frets.is_empty() {
            return;
        }
        let center = frets.iter().sum::<f32>() / frets.len() as f32;
        self.hand = Some(
            self.hand
                .map_or(center, |hand| hand + (center - hand) * HAND_FOLLOW),
        );
    }

    fn show_chord(&mut self, name: String, notes: &[i32], now: f32) {
        self.chord = Some((name, now));
        self.ringing = None;
        if let Some(mut shape) = place_chord(notes, self.hand) {
            fill_gaps(&mut shape, notes);
            self.follow_hand(shape.iter().map(|(_, fret)| *fret));
            self.played.clear();
            self.played
                .extend(shape.into_iter().map(|(string, fret)| Played {
                    string,
                    fret,
                    midi: TUNING[string] + fret,
                    at: now,
                }));
        }
    }

    fn show_note(
        &mut self,
        spectrum: &Spectrum,
        pitch: Pitch,
        now: f32,
        strings: &[f32; 6],
        learned: bool,
    ) {
        let midi = midi_note(pitch.hertz);
        let since = match self.ringing {
            Some((note, since)) if note == midi => since,
            _ => {
                self.ringing = Some((midi, now));
                self.measured = false;
                now
            }
        };
        let current = self.played.iter_mut().find(|note| note.midi == midi);
        // Stiffness takes a clean stretch of the note, so it gets one look, once
        // the note has settled, and can move the first guess.
        let settled = now - since >= SETTLE && !self.measured;
        if let (Some(note), false) = (current, settled) {
            note.at = now;
            return;
        }
        let stiffness = settled.then(|| spectrum.stiffness(pitch.hertz)).flatten();
        self.measured |= settled;
        if let Some((string, fret)) = place_note(midi, self.hand, stiffness, strings, learned) {
            self.played.retain(|note| note.midi != midi);
            self.played.push(Played {
                string,
                fret,
                midi,
                at: now,
            });
            if now == since {
                self.follow_hand(std::iter::once(fret));
            }
        }
    }

    fn learn_string(
        &mut self,
        spectrum: Option<&Spectrum>,
        pitch: Option<Pitch>,
    ) -> Option<[f32; 6]> {
        let learn = self.learn.as_mut()?;
        let open = TUNING[learn.string];
        let reading = pitch
            .filter(|pitch| midi_note(pitch.hertz) == open)
            .and_then(|pitch| spectrum?.stiffness(pitch.hertz));
        learn.readings.extend(reading);
        if learn.readings.len() < LEARN_READINGS {
            return None;
        }
        learn.strings[learn.string] = median(&mut learn.readings);
        learn.readings.clear();
        if learn.string == 0 {
            let strings = learn.strings;
            self.learn = None;
            return Some(strings);
        }
        learn.string -= 1;
        None
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
    pub(super) fn string_stiffness(&self) -> ([f32; 6], bool) {
        match self.settings.string_stiffness {
            Some(strings) => (strings, true),
            None => (STRING_STIFFNESS, false),
        }
    }

    /// Sets the neck to a song's key, with the full scale of its mode.
    pub(super) fn use_song_key(
        &mut self,
        key: SongKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scale = if key.minor {
            "Natural minor (Aeolian)"
        } else {
            "Major (Ionian)"
        };
        self.settings.key = key.root;
        self.settings.scale = scale.to_owned();
        self.fretboard.key_select.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::new(key.root)), window, cx);
        });
        self.fretboard.scale_select.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::new(scale_index(scale))), window, cx);
        });
        self.save_settings(cx);
        cx.notify();
    }

    fn toggle_learning(&mut self, cx: &mut Context<Self>) {
        let learning = self.fretboard.hearing.learn.is_some();
        self.fretboard.hearing.learn = (!learning).then(|| Learn {
            string: TUNING.len() - 1,
            readings: Vec::new(),
            strings: STRING_STIFFNESS,
        });
        cx.notify();
    }

    fn fret_label(&self, midi: i32) -> SharedString {
        let degree = degree(midi, self.settings.key);
        if self.settings.fret_intervals {
            INTERVALS[degree].into()
        } else {
            NOTES[midi.rem_euclid(12) as usize].into()
        }
    }

    fn render_neck(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.settings.key;
        let (_, scale) = SCALES[scale_index(&self.settings.scale)];
        let height = STRING_GAP * TUNING.len() as f32;
        let root = moss();
        let surface = hsla(0.42, 0.04, 0.11, 1.0);
        let on_root = cx.theme().primary_foreground;
        // Scale notes step back while something is lit, so the played note leads.
        let song = self.fretboard.song.song();
        let (song_now, song_next) = song.map_or((None, None), |song| song.now_and_next());
        let quiet = !self.fretboard.hearing.played.is_empty() || song_now.is_some();
        let tone = hsla(0.42, 0.05, if quiet { 0.2 } else { 0.3 }, 1.0);
        let text = cx.theme().foreground.opacity(if quiet { 0.5 } else { 1.0 });
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
        let lit = self.fretboard.hearing.played.iter().map(|note| {
            let fading = (1.0 - (now - note.at) / TRAIL).clamp(0.0, 1.0);
            let color = if in_scale(scale, degree(note.midi, key)) {
                hsla(0.4, 0.26, 0.93, 1.0)
            } else {
                out_of_key
            };
            marker(
                note.string,
                note.fret,
                self.fret_label(note.midi),
                color.opacity(fading),
                color,
                on_root.opacity(fading),
            )
            .shadow(vec![glow(0.8 * fading, 14.0)])
        });
        // The song's next note is an outline, so the eye can get there first.
        let coming = song_next.map(|note| {
            marker(
                note.string,
                note.fret,
                self.fret_label(note.midi),
                surface,
                moss(),
                moss(),
            )
        });
        let sounding = song_now.map(|note| {
            let color = if in_scale(scale, degree(note.midi, key)) {
                moss()
            } else {
                out_of_key
            };
            marker(
                note.string,
                note.fret,
                self.fret_label(note.midi),
                color,
                color,
                on_root,
            )
            .shadow(vec![glow(0.8, 14.0)])
        });
        let dots: Vec<Div> = dots
            .into_iter()
            .chain(coming)
            .chain(sounding)
            .chain(lit)
            .collect();
        let numbers: Vec<_> = INLAYS
            .iter()
            .map(|fret| {
                let fret = *fret;
                div()
                    .id(("fret-number", fret as usize))
                    .absolute()
                    .left(relative(
                        NUT_SHARE + (1.0 - NUT_SHARE) * fret_share(fret as f32 - 0.5),
                    ))
                    .ml(px(-12.))
                    .w(px(24.))
                    .text_center()
                    .text_xs()
                    .text_color(fog())
                    .cursor_pointer()
                    .hover(|number| number.text_color(moss()))
                    .child(fret.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.fretboard.hearing.hand = Some(fret as f32);
                        cx.notify();
                    }))
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
        let chord = self
            .fretboard
            .hearing
            .chord
            .as_ref()
            .map(|(name, _)| name.clone());
        let learning = self.fretboard.hearing.learn.as_ref().map(|learn| {
            format!(
                "Pluck the open {} string until this moves on",
                STRING_NAMES[learn.string]
            )
        });
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
                    .child(
                        Button::new("learn-guitar")
                            .small()
                            .outline()
                            .label(if learning.is_some() {
                                "Stop learning"
                            } else if self.settings.string_stiffness.is_some() {
                                "Relearn my guitar"
                            } else {
                                "Learn my guitar"
                            })
                            .tooltip("Pluck each open string, so it can tell your strings apart")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_learning(cx))),
                    )
                    .when_some(learning, |row, prompt| {
                        row.child(div().text_sm().text_color(moss()).child(prompt))
                    })
                    .child(div().flex_1())
                    .when_some(chord, |row, chord| {
                        row.child(
                            div()
                                .font_family(PRINT_FONT)
                                .font_semibold()
                                .text_lg()
                                .text_color(moss())
                                .child(chord),
                        )
                    })
                    .child(div().text_sm().text_color(fog()).child(spelled)),
            )
            .child(self.render_neck(cx))
            .child(self.render_song(cx))
            .child(div().text_xs().text_color(fog()).child(
                "Shows where each note most likely is, from your hand's position and the sound of \
                 the string. Click a fret number to say where your hand is.",
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Hearing, SCALES, STRING_STIFFNESS, degree, finger_line, fold_onto_neck, fret_share,
        in_scale, median, midi_note, place_chord, place_note, positions, scale_index,
    };
    use amp_dsp::MelodyNote;

    /// A run of notes, a fifth of a second each with no rests.
    fn line(pitches: &[i32]) -> Vec<MelodyNote> {
        pitches
            .iter()
            .enumerate()
            .map(|(index, midi)| MelodyNote {
                start: index as f32 * 0.2,
                end: (index + 1) as f32 * 0.2,
                midi: *midi,
            })
            .collect()
    }

    fn finger(pitches: &[i32]) -> Vec<(usize, i32)> {
        finger_line(&line(pitches), &vec![None; pitches.len()])
    }

    #[test]
    fn a_chosen_string_holds_and_the_line_moves_to_meet_it() {
        let spots = finger_line(&line(&[69, 71, 72]), &[None, Some(1), None]);
        assert_eq!(spots[1], (1, 12));
        assert!(spots[0].1 >= 5, "{spots:?}");
    }

    #[test]
    fn a_scale_run_stays_in_one_position() {
        // A minor pentatonic in fifth position, across three strings.
        let spots = finger(&[60, 62, 64, 67, 69, 72, 74]);
        let frets: Vec<i32> = spots.iter().map(|(_, fret)| *fret).collect();
        let (low, high) = (frets.iter().min().unwrap(), frets.iter().max().unwrap());
        assert!(high - low <= 5, "{spots:?}");
    }

    #[test]
    fn a_line_looks_ahead_to_where_it_is_going() {
        // Ending high on the neck pulls the start of the line up to meet it.
        let spots = finger(&[65, 76, 79, 76, 79]);
        assert!(spots[0].1 >= 5, "{spots:?}");
    }

    #[test]
    fn every_note_of_a_line_lands_on_its_own_pitch() {
        let pitches = [52, 55, 57, 60, 64, 69, 67, 64, 62, 60];
        for (spot, midi) in finger(&pitches).iter().zip(pitches) {
            assert_eq!(super::TUNING[spot.0] + spot.1, midi);
        }
    }

    #[test]
    fn notes_past_the_neck_come_back_by_octaves() {
        assert_eq!(fold_onto_neck(28), 40);
        assert_eq!(fold_onto_neck(91), 79);
        assert_eq!(fold_onto_neck(60), 60);
        assert!(finger(&[]).is_empty());
    }

    #[test]
    fn a_pitch_shows_everywhere_it_can_be_fretted() {
        // Middle A: 5th fret on the high E, 10th on the B, and so on down.
        let found: Vec<_> = positions(69).collect();
        assert_eq!(found, [(0, 5), (1, 10), (2, 14)]);
    }

    const RATE: f32 = 48_000.0;
    const FRAMES: usize = 16_384;
    const LOOK: f32 = 0.05;

    /// Plucks a fret on each listed string, as `(string, fret)`.
    fn sound(spots: &[(usize, i32)]) -> Vec<f32> {
        let mut buffer = vec![0.0; FRAMES];
        for (string, fret) in spots {
            let root = amp_dsp::midi_hertz(super::TUNING[*string] + fret);
            let bend = STRING_STIFFNESS[*string] * 2.0_f32.powf(*fret as f32 / 6.0);
            for partial in 1..=24 {
                let number = partial as f32;
                let hertz = number * root * (1.0 + bend * number * number).sqrt();
                if hertz > 12_000.0 {
                    break;
                }
                for (index, sample) in buffer.iter_mut().enumerate() {
                    let time = index as f32 / RATE;
                    *sample += (std::f32::consts::TAU * hertz * time).sin() * 0.08 / number;
                }
            }
        }
        buffer
    }

    /// Feeds the same sound in for `looks` meter ticks.
    fn listen(hearing: &mut Hearing, spots: &[(usize, i32)], looks: usize, now: &mut f32) {
        let buffer = sound(spots);
        let hertz = amp_dsp::midi_hertz(super::TUNING[spots[0].0] + spots[0].1);
        let pitch = (spots.len() == 1).then(|| amp_dsp::Pitch::from_hertz(hertz));
        for _ in 0..looks {
            hearing.hear(&buffer, pitch, *now, &STRING_STIFFNESS, false);
            *now += LOOK;
        }
    }

    #[test]
    fn a_single_note_lights_one_spot() {
        let mut hearing = Hearing {
            hand: Some(5.0),
            ..Hearing::default()
        };
        let mut now = 0.0;
        listen(&mut hearing, &[(0, 5)], 3, &mut now);
        assert_eq!(hearing.played.len(), 1);
        assert_eq!((hearing.played[0].string, hearing.played[0].fret), (0, 5));
        assert!(hearing.chord.is_none());
    }

    #[test]
    fn a_chord_gets_its_name_and_one_spot_per_string() {
        let mut hearing = Hearing::default();
        let mut now = 0.0;
        let shape = [(5, 0), (4, 2), (3, 2), (2, 1), (1, 0), (0, 0)];
        listen(&mut hearing, &shape, 3, &mut now);
        let name = hearing.chord.as_ref().map(|(name, _)| name.as_str());
        assert_eq!(name, Some("E"));
        let mut lit: Vec<_> = hearing
            .played
            .iter()
            .map(|note| (note.string, note.fret))
            .collect();
        lit.sort_unstable();
        assert_eq!(lit, [(0, 0), (1, 0), (2, 1), (3, 2), (4, 2), (5, 0)]);
    }

    #[test]
    fn gaps_take_a_chord_tone_under_the_hand() {
        // G major, with the B and top G hidden in the overtones of the notes below.
        let mut shape = vec![(5, 3), (4, 2), (3, 0), (0, 3)];
        super::fill_gaps(&mut shape, &[43, 47, 50, 67]);
        shape.sort_unstable();
        assert_eq!(shape, [(0, 3), (1, 0), (2, 0), (3, 0), (4, 2), (5, 3)]);
    }

    #[test]
    fn a_held_note_can_move_once_its_string_is_heard() {
        // A3 on the A string's 12th fret, with the hand down at the 7th.
        let mut hearing = Hearing {
            hand: Some(7.0),
            ..Hearing::default()
        };
        let mut now = 0.0;
        listen(&mut hearing, &[(4, 12)], 1, &mut now);
        assert_eq!(hearing.played[0].string, 3);
        listen(&mut hearing, &[(4, 12)], 8, &mut now);
        assert_eq!((hearing.played[0].string, hearing.played[0].fret), (4, 12));
    }

    #[test]
    fn learning_measures_each_open_string_in_turn() {
        let mut hearing = Hearing {
            learn: Some(super::Learn {
                string: 5,
                readings: Vec::new(),
                strings: [0.0; 6],
            }),
            ..Hearing::default()
        };
        let mut learned = None;
        for string in (0..6).rev() {
            let buffer = sound(&[(string, 0)]);
            let hertz = amp_dsp::midi_hertz(super::TUNING[string]);
            let pitch = Some(amp_dsp::Pitch::from_hertz(hertz));
            for _ in 0..super::LEARN_READINGS {
                learned = learned.or(hearing.hear(&buffer, pitch, 0.0, &STRING_STIFFNESS, false));
            }
        }
        let learned = learned.expect("every string was heard");
        for (found, truth) in learned.iter().zip(STRING_STIFFNESS) {
            assert!((found / truth).ln().abs() < 0.4, "{found} against {truth}");
        }
        assert!(hearing.learn.is_none());
    }

    fn note(midi: i32, hand: Option<f32>, stiffness: Option<f32>) -> Option<(usize, i32)> {
        place_note(midi, hand, stiffness, &STRING_STIFFNESS, false)
    }

    #[test]
    fn a_note_lands_once_near_the_hand() {
        // Middle A, in three hand positions.
        assert_eq!(note(69, Some(5.0), None), Some((0, 5)));
        assert_eq!(note(69, Some(10.0), None), Some((1, 10)));
        assert_eq!(note(69, Some(14.0), None), Some((2, 14)));
    }

    #[test]
    fn with_no_hand_yet_it_starts_low_on_the_neck() {
        assert_eq!(note(57, None, None), Some((2, 2)));
    }

    #[test]
    fn a_stiff_sound_pulls_a_note_to_the_thicker_string() {
        // A3 near the 7th fret could be the D string, but it sounds like the
        // low E at the 17th's neighbor: here, the A string at the 12th.
        let on_a_string = STRING_STIFFNESS[4] * 2.0_f32.powf(12.0 / 6.0);
        assert_eq!(note(57, Some(7.0), None), Some((3, 7)));
        assert_eq!(note(57, Some(7.0), Some(on_a_string)), Some((4, 12)));
    }

    #[test]
    fn an_open_e_chord_lands_on_its_shape() {
        let mut shape = place_chord(&[40, 47, 52, 56, 59, 64], None).unwrap();
        shape.sort_unstable();
        assert_eq!(shape, [(0, 0), (1, 0), (2, 1), (3, 2), (4, 2), (5, 0)]);
    }

    #[test]
    fn a_barre_chord_stays_within_one_hand() {
        let shape = place_chord(&[41, 48, 53, 57, 60, 65], Some(1.0)).unwrap();
        let frets: Vec<i32> = shape.iter().map(|(_, fret)| *fret).collect();
        assert_eq!(frets, [1, 3, 3, 2, 1, 1]);
    }

    #[test]
    fn notes_no_hand_could_reach_have_no_shape() {
        assert_eq!(place_chord(&[40, 41, 42, 43, 44, 45, 46], None), None);
    }

    #[test]
    fn the_median_ignores_a_stray_reading() {
        assert_eq!(median(&mut [3.0, 1.0, 900.0, 2.0, 2.5]), 2.5);
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
