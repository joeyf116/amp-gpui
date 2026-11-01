use amp_dsp::ParamSpec;
use gpui_kit::{component::ActiveTheme, *};
use std::{f32::consts::PI, rc::Rc};

/// The amber that marks anything live in the rig.
pub fn accent() -> Hsla {
    hsla(0.095, 0.92, 0.56, 1.0)
}

const SWEEP: f32 = 1.5 * PI;
const START_ANGLE: f32 = 0.75 * PI;
const ARC_STEPS: usize = 48;
/// Pixels of vertical drag that cover the knob's whole range.
const DRAG_RANGE: f32 = 180.0;

type OnChange = Rc<dyn Fn(&f32, &mut Window, &mut App)>;

#[derive(Clone, Copy)]
struct Drag {
    start_y: Pixels,
    start: f32,
}

/// A round control. Drag up or down to turn it, and double-click to reset.
#[derive(IntoElement)]
pub struct Knob {
    id: ElementId,
    spec: ParamSpec,
    value: f32,
    size: Pixels,
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
            on_change: Rc::new(on_change),
        }
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

fn disc(center: Point<Pixels>, radius: Pixels) -> Option<Path<Pixels>> {
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
            on_change,
        } = self;
        let drag = window.use_keyed_state(id.clone(), cx, |_, _| None::<Drag>);
        let dragging = drag.read(cx).is_some();
        let amount = (value - spec.min) / (spec.max - spec.min);
        let track = cx.theme().muted;
        let body = cx.theme().background;
        let rim = cx.theme().border;
        let label_color = cx.theme().muted_foreground;
        let ink = cx.theme().foreground;

        let press_state = drag.clone();
        let reset = on_change.clone();
        let paint_state = drag.clone();
        let change = on_change.clone();

        let dial = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center = bounds.center();
                let outer = bounds.size.width / 2.0 - px(3.0);
                let end = START_ANGLE + SWEEP;
                let needle = START_ANGLE + SWEEP * amount;
                if let Some(path) = arc(center, outer, START_ANGLE, end, px(4.0)) {
                    window.paint_path(path, track);
                }
                if amount > 0.0
                    && let Some(path) = arc(center, outer, START_ANGLE, needle, px(4.0))
                {
                    window.paint_path(path, accent());
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

                // Registered on the window so a drag keeps working once the
                // pointer leaves the knob, which it nearly always does.
                if dragging {
                    let moved_state = paint_state.clone();
                    let change = change.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let Some(drag) = *moved_state.read(cx) else {
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
                        let moved = (drag.start_y - event.position.y) / px(DRAG_RANGE);
                        let next = (drag.start + moved).clamp(0.0, 1.0);
                        let value = spec.min + next * (spec.max - spec.min);
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
                    .size(size)
                    .cursor_ns_resize()
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        if event.click_count == 2 {
                            reset(&spec.default, window, cx);
                            return;
                        }
                        press_state.update(cx, |state, cx| {
                            *state = Some(Drag {
                                start_y: event.position.y,
                                start: amount,
                            });
                            cx.notify();
                        });
                        window.refresh();
                    })
                    .child(dial),
            )
            .child(div().text_xs().child(format!("{:.1} {}", value, spec.unit)))
    }
}
