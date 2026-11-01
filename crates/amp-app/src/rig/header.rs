//! The header: which tone is loaded, and the controls that matter while
//! playing, which stay in reach on every view.

use super::{HEADER_HEIGHT, KNOB_INPUT, KNOB_OUTPUT, METER_FLOOR_DB, Rig};
use crate::{
    art::{self, PRINT_FONT},
    knob::Knob,
    style::{bar, fog, glow, hairline, moss},
};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};

const METER_WIDTH: f32 = 64.0;
const LEVEL_KNOB: Pixels = px(30.);

impl Rig {
    fn render_meter(&self, peak: f32, cx: &App) -> impl IntoElement {
        let db = 20.0 * peak.max(1e-6).log10();
        let fill = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
        let color = if peak >= 0.99 {
            cx.theme().danger
        } else {
            cx.theme().success
        };
        div()
            .w(px(METER_WIDTH))
            .h(px(4.))
            .rounded_sm()
            .bg(hairline())
            .child(div().h_full().rounded_sm().w(relative(fill)).bg(color))
    }

    fn render_tone_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let edited = self.preset.blocks != self.saved_blocks;
        let name = self.preset.name.clone();
        let favorite = self.is_favorite(&name);
        let title = if edited {
            format!("{name} *")
        } else {
            name.clone()
        };
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("previous-tone")
                    .ghost()
                    .label("\u{2039}")
                    .tooltip("Previous tone (Page Up)")
                    .on_click(cx.listener(|this, _, window, cx| this.step_tone(-1, window, cx))),
            )
            .child(
                v_flex()
                    .id("tone-name")
                    .w(px(236.))
                    .h(px(44.))
                    .justify_center()
                    .px_3()
                    .rounded_md()
                    .border_1()
                    .border_color(hairline())
                    .bg(hsla(0.42, 0.3, 0.02, 0.7))
                    .cursor_pointer()
                    .hover(|tone| tone.border_color(moss()))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(PRINT_FONT)
                                    .font_semibold()
                                    .truncate()
                                    .child(title),
                            )
                            .child(div().text_xs().text_color(fog()).child("\u{25BE}")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(fog())
                            .truncate()
                            .child(self.preset.tags.join("  /  ")),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_tones(window, cx))),
            )
            .child(
                Button::new("next-tone")
                    .ghost()
                    .label("\u{203A}")
                    .tooltip("Next tone (Page Down)")
                    .on_click(cx.listener(|this, _, window, cx| this.step_tone(1, window, cx))),
            )
            .child(
                div()
                    .id("favorite")
                    .px_1()
                    .cursor_pointer()
                    .text_color(if favorite { moss() } else { fog() })
                    .child(if favorite { "\u{2605}" } else { "\u{2606}" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_favorite(&name, cx);
                    })),
            )
    }

    fn render_level(&self, knob: Knob, peak: f32, cx: &App) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(knob)
            .child(self.render_meter(peak, cx))
    }

    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let edited = self.preset.blocks != self.saved_blocks;
        let input = Knob::new(
            "level-input",
            KNOB_INPUT,
            self.settings.input_db,
            LEVEL_KNOB,
            cx.listener(|this, value: &f32, _, cx| this.set_level(Some(*value), None, cx)),
        );
        let output = Knob::new(
            "level-output",
            KNOB_OUTPUT,
            self.settings.output_db,
            LEVEL_KNOB,
            cx.listener(|this, value: &f32, _, cx| this.set_level(None, Some(*value), cx)),
        );
        h_flex()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .px_3()
            .gap_4()
            .items_center()
            .border_b_1()
            .border_color(hairline())
            .bg(bar())
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(img(art::logo()).size(px(38.)))
                    .child(
                        div()
                            .font_family(PRINT_FONT)
                            .font_semibold()
                            .text_sm()
                            .whitespace_nowrap()
                            .child("AMP GPUI"),
                    ),
            )
            .child(self.render_tone_picker(cx))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("save")
                            .small()
                            .when(edited, |button| button.primary())
                            .when(!edited, |button| button.ghost())
                            .label("Save")
                            .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx))),
                    )
                    .child(
                        Button::new("undo")
                            .small()
                            .ghost()
                            .label("Undo")
                            .disabled(self.history.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                    )
                    .child(
                        Button::new("redo")
                            .small()
                            .ghost()
                            .label("Redo")
                            .disabled(self.future.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
                    ),
            )
            .child(div().flex_1())
            .child(self.render_level(input, self.input_level, cx))
            .child(self.render_level(output, self.output_level, cx))
            .child(
                div()
                    .rounded_md()
                    .when(live, |button| button.shadow(vec![glow(0.4, 12.0)]))
                    .child(
                        Button::new("monitor")
                            .label(if live {
                                "Stop monitoring"
                            } else {
                                "Start monitoring"
                            })
                            .tooltip("Press M")
                            .when(!live, |button| button.primary())
                            .when(live, |button| button.danger())
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_monitoring(cx))),
                    ),
            )
    }
}
