//! The footer and the Audio setup panel. Devices are set once and left alone,
//! so they fold away, and the footer reports how the audio path is doing.

use super::{FOOTER_HEIGHT, Overlay, Rig, close_button};
use crate::style::{bar, fog, glow, hairline, moss, sheet};
use amp_engine::{DeviceOption, ENGINE_RATE, Engine, EngineConfig, LatencyMode};
use amp_preset::build_chain;
use gpui_kit::{
    component::{
        IndexPath, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        select::{Select, SelectItem},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

const PANEL_WIDTH: f32 = 380.0;
const WINDOWS_AUDIO: &str = "Windows audio";

#[derive(Clone)]
pub(super) struct Choice {
    pub(super) value: String,
    pub(super) title: SharedString,
}

impl SelectItem for Choice {
    type Value = String;
    fn value(&self) -> &String {
        &self.value
    }
    fn title(&self) -> SharedString {
        self.title.clone()
    }
}

pub(super) fn device_choices(devices: &[DeviceOption]) -> Vec<Choice> {
    devices
        .iter()
        .map(|device| Choice {
            value: device.id.clone(),
            title: device.name.clone().into(),
        })
        .collect()
}

pub(super) fn driver_choices(asio: &[DeviceOption]) -> Vec<Choice> {
    let mut choices = vec![Choice {
        value: WINDOWS_AUDIO.to_owned(),
        title: WINDOWS_AUDIO.into(),
    }];
    choices.extend(device_choices(asio));
    choices
}

pub(super) fn channel_choices(channels: u16) -> Vec<Choice> {
    (0..channels.max(1))
        .map(|channel| Choice {
            value: channel.to_string(),
            title: format!("Input {}", channel + 1).into(),
        })
        .collect()
}

pub(super) fn position(devices: &[DeviceOption], id: Option<&str>) -> Option<IndexPath> {
    let id = id?;
    devices
        .iter()
        .position(|device| device.id == id)
        .map(IndexPath::new)
}
impl Rig {
    pub(super) fn scan_asio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.asio_devices = Engine::asio_devices();
        let selected = self
            .settings
            .asio_driver
            .as_deref()
            .and_then(|driver| position(&self.asio_devices, Some(driver)))
            .map_or(0, |index| index.row + 1);
        let choices = driver_choices(&self.asio_devices);
        self.driver_select.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_index(Some(IndexPath::new(selected)), window, cx);
        });
        if self.settings.asio_driver.is_some() {
            self.show_channels(window, cx);
        }
        cx.notify();
    }

    pub(super) fn select_driver(
        &mut self,
        driver: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.asio_driver = (driver != WINDOWS_AUDIO).then_some(driver);
        self.settings.input_channel = 0;
        self.show_channels(window, cx);
        self.devices_changed(cx);
    }

    /// Lists the inputs of whichever device the guitar is coming through.
    pub(super) fn show_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let device = match &self.settings.asio_driver {
            Some(driver) => self.asio_devices.iter().find(|device| &device.id == driver),
            None => {
                let input = self.settings.input.as_ref();
                self.devices
                    .inputs
                    .iter()
                    .find(|device| Some(&device.id) == input)
            }
        };
        let channels = device.map_or(1, |device| device.channels);
        let selected = self.settings.input_channel.min(usize::from(channels) - 1);
        self.channel_select.update(cx, |select, cx| {
            select.set_items(channel_choices(channels), window, cx);
            select.set_selected_index(Some(IndexPath::new(selected)), window, cx);
        });
    }

    pub(super) fn select_input(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let channels = self
            .devices
            .inputs
            .iter()
            .find(|device| device.id == id)
            .map_or(1, |device| device.channels);
        self.settings.input = Some(id);
        self.settings.input_channel = 0;
        self.channel_select.update(cx, |select, cx| {
            select.set_items(channel_choices(channels), window, cx);
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.devices_changed(cx);
    }

    /// Saves the choice, and moves live monitoring over to the new devices.
    pub(super) fn devices_changed(&mut self, cx: &mut Context<Self>) {
        self.save_settings(cx);
        if self.engine.is_some() {
            self.engine = None;
            self.start_monitoring(cx);
        }
        cx.notify();
    }

    pub(super) fn engine_config(&self, mode: LatencyMode) -> EngineConfig {
        EngineConfig {
            input: self.settings.input.clone(),
            output: self.settings.output.clone(),
            input_channel: self.settings.input_channel,
            mode,
            asio_driver: self.settings.asio_driver.clone(),
        }
    }

    /// Starts on the fastest route available. Something else may hold a device,
    /// and playing with more latency beats not playing, so each slower route
    /// is tried in turn.
    pub(super) fn start_monitoring(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        let mut modes = Vec::new();
        #[cfg(feature = "asio")]
        if self.settings.asio_driver.is_some() {
            modes.push(LatencyMode::Asio);
        }
        if !self.settings.shared_audio {
            modes.push(LatencyMode::Exclusive);
        }
        modes.push(LatencyMode::Shared);

        let mut first_error = None;
        for mode in modes {
            // A failed start takes its chain with it, so each try needs one.
            let built = match build_chain(&self.preset, &self.library, ENGINE_RATE) {
                Ok(built) => built,
                Err(error) => return self.set_error(error, cx),
            };
            match Engine::start(Box::new(built.chain), &self.engine_config(mode)) {
                Ok(engine) => {
                    self.engine = Some(engine);
                    self.apply_levels();
                    self.send_song();
                    if let Some(error) = first_error {
                        self.error = Some(format!("{error}. Using a slower route.").into());
                    }
                    return cx.notify();
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_error {
            self.set_error(error, cx);
        }
    }

    pub(super) fn toggle_shared_audio(&mut self, cx: &mut Context<Self>) {
        self.settings.shared_audio = !self.settings.shared_audio;
        self.devices_changed(cx);
    }

    pub(super) fn toggle_monitoring(&mut self, cx: &mut Context<Self>) {
        if self.engine.take().is_none() {
            self.start_monitoring(cx);
        }
        self.input_level = 0.0;
        self.output_level = 0.0;
        cx.notify();
    }

    pub(super) fn input_is_microphone(&self) -> bool {
        if self.settings.asio_driver.is_some() {
            return false;
        }
        let input = self.settings.input.as_ref();
        self.devices
            .inputs
            .iter()
            .find(|device| Some(&device.id) == input)
            .is_some_and(|device| {
                let name = device.name.to_lowercase();
                name.contains("microphone") || name.contains("webcam")
            })
    }
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
            .children(self.render_update(cx))
            .child(div().flex_1())
            .when(self.metronome_on, |bar| {
                bar.child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .when(self.beat == 1, |lamp| {
                            lamp.bg(moss()).shadow(vec![glow(0.7, 8.0)])
                        })
                        .when(self.beat != 1, |lamp| lamp.bg(hairline())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(fog())
                        .whitespace_nowrap()
                        .child(format!("{:.0} BPM", self.settings.bpm)),
                )
            })
            .child(
                Button::new("tools")
                    .small()
                    .label("Practice tools")
                    .when(self.tuner_open, |button| button.outline())
                    .when(!self.tuner_open, |button| button.ghost())
                    .tooltip("Press T")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_tools(window, cx))),
            )
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
                    .child(close_button("close-audio", cx)),
            )
            .child(picker(
                "Audio driver",
                Select::new(&self.driver_select).w_full().into_any_element(),
            ))
            .when(cfg!(feature = "asio"), |panel| {
                panel.child(
                    div().text_xs().text_color(fog()).child(
                        "ASIO is a registered trademark of Steinberg Media Technologies GmbH.",
                    ),
                )
            })
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
