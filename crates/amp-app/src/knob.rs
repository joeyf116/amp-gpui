use crate::{
    art,
    style::{moss, moss_glow},
};
use amp_dsp::ParamSpec;
use gpui_kit::{component::ActiveTheme, prelude::FluentBuilder, *};
use std::{f32::consts::PI, rc::Rc};

const SWEEP: f32 = 1.5 * PI;
const START_ANGLE: f32 = 0.75 * PI;
const ARC_STEPS: usize = 48;
/// Pixels of vertical drag that cover the knob's whole range.
const DRAG_RANGE: f32 = 180.0;
/// Holding Shift makes a drag or scroll this much finer.
const FINE: f32 = 0.2;
/// How far one notch of a scroll wheel turns the knob.
const WHEEL_STEP: f32 = 0.02;

/// How much of a faceplate knob's box the knob itself fills, leaving a printed
/// scale around it.
const CAP_SHARE: f32 = 0.84;
const SCALE_TICKS: usize = 11;

type OnChange = Rc<dyn Fn(&f32, &mut Window, &mut App)>;

/// What a knob is drawn as.
#[derive(Clone, Copy, PartialEq)]
pub enum KnobLook {
    /// A flat ring, for the window's own level controls.
    Ring,
    /// A fluted amp knob with a spun metal cap.
    Amp,
    /// A small knurled pedal knob.
    Pedal,
}

#[derive(Clone, Copy)]
struct Drag {
    last_y: Pixels,
    amount: f32,
}

/// A round control. Drag up or down to turn it, hold Shift for fine moves,
/// scroll to nudge it, and double-click to reset.
#[derive(IntoElement)]
pub struct Knob {
    id: ElementId,
    spec: ParamSpec,
    value: f32,
    size: Pixels,
    look: KnobLook,
    ink: Option<Hsla>,
    on_change: OnChange,
}

impl Knob {
    pub fn new(
        id: impl Into<ElementId>,
        spec: ParamSpec,
        value: f32,
        size: Pixels,
        on_change: impl Fn(&f32, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            spec,
            value,
            size,
            look: KnobLook::Ring,
            ink: None,
            on_change: Rc::new(on_change),
        }
    }

    /// Sets it into a faceplate, with its text in that plate's print color.
    pub fn on_faceplate(mut self, look: KnobLook, ink: Hsla) -> Self {
        self.look = look;
        self.ink = Some(ink);
        self
    }
}

fn point_at(center: Point<Pixels>, radius: Pixels, angle: f32) -> Point<Pixels> {
    point(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    )
}

fn arc(
    center: Point<Pixels>,
    radius: Pixels,
    from: f32,
    to: f32,
    width: Pixels,
) -> Option<Path<Pixels>> {
    let mut builder = PathBuilder::stroke(width);
    builder.move_to(point_at(center, radius, from));
    for step in 1..=ARC_STEPS {
        let angle = from + (to - from) * step as f32 / ARC_STEPS as f32;
        builder.line_to(point_at(center, radius, angle));
    }
    builder.build().ok()
}

pub fn disc(center: Point<Pixels>, radius: Pixels) -> Option<Path<Pixels>> {
    let points: Vec<_> = (0..ARC_STEPS)
        .map(|step| point_at(center, radius, step as f32 * 2.0 * PI / ARC_STEPS as f32))
        .collect();
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    builder.build().ok()
}

impl RenderOnce for Knob {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            spec,
            value,
            size,
            look,
            ink,
            on_change,
        } = self;
        let drag = window.use_keyed_state(id.clone(), cx, |_, _| None::<Drag>);
        let dragging = drag.read(cx).is_some();
        let amount = (value - spec.min) / (spec.max - spec.min);
        let track = cx.theme().muted;
        let body = cx.theme().background;
        let rim = cx.theme().border;
        let label_color = ink.unwrap_or(cx.theme().muted_foreground);
        let value_color = ink.map_or(cx.theme().foreground, |ink| ink.opacity(0.78));
        let ink = cx.theme().foreground;

        let press_state = drag.clone();
        let reset = on_change.clone();
        let wheel = on_change.clone();
        let paint_state = drag.clone();
        let change = on_change.clone();

        let dial = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center = bounds.center();
                let needle = START_ANGLE + SWEEP * amount;
                if look == KnobLook::Ring {
                    let outer = bounds.size.width / 2.0 - px(3.0);
                    let end = START_ANGLE + SWEEP;
                    if let Some(path) = arc(center, outer, START_ANGLE, end, px(4.0)) {
                        window.paint_path(path, track);
                    }
                    if amount > 0.0
                        && let Some(path) = arc(center, outer, START_ANGLE, needle, px(4.0))
                    {
                        window.paint_path(path, moss());
                    }
                    if amount > 0.0
                        && let Some(path) = arc(center, outer, START_ANGLE, needle, px(8.0))
                    {
                        window.paint_path(path, moss_glow(0.14));
                    }
                    let inner = outer - px(8.0);
                    if let Some(path) = disc(center, inner) {
                        window.paint_path(path, rim);
                    }
                    if let Some(path) = disc(center, inner - px(1.5)) {
                        window.paint_path(path, body);
                    }
                    let mut pointer = PathBuilder::stroke(px(3.0));
                    pointer.move_to(point_at(center, inner * 0.35, needle));
                    pointer.line_to(point_at(center, inner * 0.88, needle));
                    if let Ok(path) = pointer.build() {
                        window.paint_path(path, ink);
                    }
                } else {
                    // The knob below is a fixed picture, lit from one side, so
                    // only the pointer and printed scale are painted.
                    let reach = bounds.size.width / 2.0;
                    let cap = reach * CAP_SHARE;
                    for tick in 0..SCALE_TICKS {
                        let angle = START_ANGLE + SWEEP * tick as f32 / (SCALE_TICKS - 1) as f32;
                        let mut mark = PathBuilder::stroke(px(1.2));
                        mark.move_to(point_at(center, reach * 0.9, angle));
                        mark.line_to(point_at(center, reach, angle));
                        if let Ok(path) = mark.build() {
                            window.paint_path(path, label_color.opacity(0.55));
                        }
                    }
                    let (from, width) = match look {
                        KnobLook::Amp => (0.55, 2.4),
                        _ => (0.3, 2.6),
                    };
                    let mut pointer = PathBuilder::stroke(px(width));
                    pointer.move_to(point_at(center, cap * from, needle));
                    pointer.line_to(point_at(center, cap * 0.92, needle));
                    if let Ok(path) = pointer.build() {
                        window.paint_path(path, hsla(0.12, 0.25, 0.92, 1.0));
                    }
                }

                // Registered on the window so a drag keeps working once the
                // pointer leaves the knob, which it nearly always does.
                if dragging {
                    let moved_state = paint_state.clone();
                    let change = change.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let Some(mut drag) = *moved_state.read(cx) else {
                            return;
                        };
                        if event.pressed_button != Some(MouseButton::Left) {
                            moved_state.update(cx, |state, cx| {
                                *state = None;
                                cx.notify();
                            });
                            window.refresh();
                            return;
                        }
                        // Measured step by step, so pressing Shift mid-drag doesn't jump.
                        let scale = if event.modifiers.shift { FINE } else { 1.0 };
                        let moved = (drag.last_y - event.position.y) / px(DRAG_RANGE);
                        drag.amount = (drag.amount + moved * scale).clamp(0.0, 1.0);
                        drag.last_y = event.position.y;
                        moved_state.update(cx, |state, _| *state = Some(drag));
                        let value = spec.min + drag.amount * (spec.max - spec.min);
                        change(&value, window, cx);
                    });
                    let released_state = paint_state.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble {
                            released_state.update(cx, |state, cx| {
                                *state = None;
                                cx.notify();
                            });
                            window.refresh();
                        }
                    });
                }
            },
        )
        .absolute()
        .size_full();

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .w(size + px(20.0))
            .child(div().text_xs().text_color(label_color).child(spec.name))
            .child(
                div()
                    .id(id)
                    .relative()
                    .size(size)
                    .cursor_ns_resize()
                    .on_scroll_wheel(move |event, window, cx| {
                        let notches = event.delta.pixel_delta(px(20.0)).y / px(20.0);
                        let scale = if event.modifiers.shift { FINE } else { 1.0 };
                        let next = (amount + notches * WHEEL_STEP * scale).clamp(0.0, 1.0);
                        wheel(&(spec.min + next * (spec.max - spec.min)), window, cx);
                    })
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        if event.click_count == 2 {
                            reset(&spec.default, window, cx);
                            return;
                        }
                        press_state.update(cx, |state, cx| {
                            *state = Some(Drag {
                                last_y: event.position.y,
                                amount,
                            });
                            cx.notify();
                        });
                        window.refresh();
                    })
                    .when(look != KnobLook::Ring, |knob| {
                        let cap = size * CAP_SHARE;
                        let inset = (size - cap) / 2.0;
                        knob.child(
                            img(if look == KnobLook::Amp {
                                art::knob_amp()
                            } else {
                                art::knob_pedal()
                            })
                            .absolute()
                            .left(inset)
                            .top(inset)
                            .size(cap),
                        )
                    })
                    .child(dial),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(value_color)
                    .child(format!("{:.1} {}", value, spec.unit)),
            )
    }
}
