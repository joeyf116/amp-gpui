use super::Rig;
use crate::style::{fog, hairline, moss, pane};
use amp_dsp::detect_pitch;
use amp_engine::ENGINE_RATE;
use gpui_kit::{
    component::{ActiveTheme, StyledExt, h_flex, v_flex},
    *,
};

/// Enough audio to catch the lowest note the tuner looks for.
const TUNER_FRAMES: usize = 6_000;
const IN_TUNE_CENTS: f32 = 3.0;
const SCALE_CENTS: f32 = 50.0;

impl Rig {
    pub(super) fn toggle_tuner(&mut self, cx: &mut Context<Self>) {
        self.tuner_open = !self.tuner_open;
        self.pitch = None;
        self.tuner_samples.clear();
        if let Some(engine) = &self.engine {
            engine.meters().set_tuner(self.tuner_open);
        }
        cx.notify();
    }

    pub(super) fn read_tuner(&mut self) {
        let Some(engine) = &mut self.engine else {
            return;
        };
        engine.read_tuner(&mut self.tuner_samples);
        let extra = self.tuner_samples.len().saturating_sub(TUNER_FRAMES);
        self.tuner_samples.drain(..extra);
        // Hold the last reading through a gap, so the needle doesn't flicker
        // as a note decays.
        if let Some(pitch) = detect_pitch(&self.tuner_samples, ENGINE_RATE) {
            self.pitch = Some(pitch);
        }
    }

    pub(super) fn render_tuner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let pitch = self.pitch;
        let cents = pitch.map_or(0.0, |pitch| pitch.cents);
        let in_tune = pitch.is_some() && cents.abs() <= IN_TUNE_CENTS;
        let color = if in_tune { moss() } else { cx.theme().warning };
        let note = pitch.map_or_else(|| "--".to_owned(), |pitch| pitch.note.to_owned());
        let detail = match pitch {
            Some(pitch) => format!("{:.1} Hz, {:+.0} cents", pitch.hertz, pitch.cents),
            None if live => "Play one open string".to_owned(),
            None => "Start monitoring to tune".to_owned(),
        };
        let position = ((cents + SCALE_CENTS) / (2.0 * SCALE_CENTS)).clamp(0.0, 1.0);
        h_flex()
            .gap_4()
            .items_center()
            .px_4()
            .py_3()
            .rounded_lg()
            .border_1()
            .border_color(hairline())
            .bg(pane())
            .child(
                div()
                    .w(px(64.))
                    .text_2xl()
                    .font_semibold()
                    .text_color(if pitch.is_some() { color } else { fog() })
                    .child(note),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .relative()
                            .h(px(16.))
                            .child(
                                div()
                                    .absolute()
                                    .top(px(7.))
                                    .left_0()
                                    .right_0()
                                    .h(px(2.))
                                    .bg(hairline()),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left(relative(0.5))
                                    .w(px(1.))
                                    .h(px(16.))
                                    .bg(fog()),
                            )
                            .when_marker(pitch.is_some(), position, color),
                    )
                    .child(div().text_xs().text_color(fog()).child(detail)),
            )
    }
}

trait TunerMarker: ParentElement + Sized {
    fn when_marker(self, show: bool, position: f32, color: Hsla) -> Self {
        if show {
            self.child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(relative(position))
                    .size(px(12.))
                    .rounded_full()
                    .bg(color),
            )
        } else {
            self
        }
    }
}

impl<T: ParentElement> TunerMarker for T {}
