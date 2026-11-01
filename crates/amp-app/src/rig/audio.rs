//! The footer and the Audio setup panel. Devices are set once and left alone,
//! so they fold away, and the footer reports how the audio path is doing.

use super::{FOOTER_HEIGHT, Overlay, Rig, View};
use crate::style::{bar, fog, glow, hairline, moss, sheet};
use amp_engine::LatencyMode;
use gpui_kit::{
    component::{
        Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        select::Select,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

const PANEL_WIDTH: f32 = 380.0;

impl Rig {
    fn audio_status(&self) -> String {
        let Some(engine) = &self.engine else {
            return "Not monitoring".to_owned();
        };
        let route = match engine.info().mode {
            LatencyMode::Exclusive => "exclusive",
            LatencyMode::Shared => "shared",
            #[cfg(feature = "asio")]
            LatencyMode::Asio => "ASIO",
        };
        format!(
            "{:.0} ms latency ({route}), {} glitches",
            self.latency.total_ms(),
            self.glitches
        )
    }

    fn render_tab(
        &self,
        id: &'static str,
        label: &'static str,
        view: View,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(id)
            .small()
            .label(label)
            .when(self.view == view, |button| button.outline())
            .when(self.view != view, |button| button.ghost())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.view = view;
                cx.notify();
            }))
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        h_flex()
            .flex_shrink_0()
            .h(px(FOOTER_HEIGHT))
            .px_3()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(hairline())
            .bg(bar())
            .child(
                div()
                    .size(px(8.))
                    .rounded_full()
                    .bg(if live { moss() } else { hairline() })
                    .when(live, |lamp| lamp.shadow(vec![glow(0.7, 8.0)])),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(fog())
                    .whitespace_nowrap()
                    .child(self.audio_status()),
            )
            .child(
                Button::new("audio-setup")
                    .small()
                    .ghost()
                    .label("Audio setup")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.overlay == Overlay::Audio {
                            this.close_overlay(window, cx);
                        } else {
                            this.overlay = Overlay::Audio;
                            cx.notify();
                        }
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("tuner")
                    .small()
                    .label("Tuner")
                    .when(self.tuner_open, |button| button.outline())
                    .when(!self.tuner_open, |button| button.ghost())
                    .tooltip("Press T")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_tuner(cx))),
            )
            .child(self.render_tab("tab-board", "Rig", View::Board, cx))
            .child(self.render_tab("tab-practice", "Practice", View::Practice, cx))
    }

    pub(super) fn render_audio_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let asio = self.settings.asio_driver.is_some();
        let picker = |label: &'static str, select: AnyElement| {
            v_flex()
                .gap_1()
                .min_w_0()
                .child(div().text_xs().text_color(fog()).child(label))
                .child(select)
        };
        v_flex()
            .id("audio-panel")
            .occlude()
            .absolute()
            .left(px(12.))
            .bottom(px(12.))
            .w(px(PANEL_WIDTH))
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(hairline())
            .bg(sheet())
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_semibold().child("Audio setup"))
                    .child(
                        Button::new("close-audio")
                            .small()
                            .ghost()
                            .label("Close")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_overlay(window, cx);
                            })),
                    ),
            )
            .child(picker(
                "Audio driver",
                Select::new(&self.driver_select).w_full().into_any_element(),
            ))
            .when(!asio, |panel| {
                panel.child(picker(
                    "Guitar in",
                    Select::new(&self.input_select)
                        .placeholder("Choose your interface")
                        .w_full()
                        .into_any_element(),
                ))
            })
            .child(picker(
                "Channel",
                Select::new(&self.channel_select)
                    .w_full()
                    .into_any_element(),
            ))
            .when(!asio, |panel| {
                panel
                    .child(picker(
                        "Speakers out",
                        Select::new(&self.output_select)
                            .placeholder("Choose an output")
                            .w_full()
                            .into_any_element(),
                    ))
                    .child(
                        Button::new("latency-mode")
                            .outline()
                            .tooltip(
                                "Low latency takes over your audio devices, so other apps go \
                                 silent on them while monitoring",
                            )
                            .label(if self.settings.shared_audio {
                                "Shared audio"
                            } else {
                                "Low latency"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_shared_audio(cx))),
                    )
            })
            .child(div().text_xs().text_color(fog()).child(
                "Hearing a clean signal under the amp? Turn off direct monitoring on your \
                 interface, so only this app reaches your speakers.",
            ))
    }
}
