use crate::{
    knob::{Knob, accent},
    settings::Settings,
};
use amp_dsp::{AmpInfo, BlockKind};
use amp_engine::{Command, DeviceList, DeviceOption, ENGINE_RATE, Engine, EngineConfig, Latency};
use amp_preset::{AssetKind, Library, Preset, PresetEntry, build_chain};
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputState},
        select::{Select, SelectEvent, SelectItem, SelectState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{path::PathBuf, time::Duration};

const METER_INTERVAL: Duration = Duration::from_millis(50);
const METER_FLOOR_DB: f32 = -60.0;
const CARD_KNOB: Pixels = px(52.);
const AMP_KNOB: Pixels = px(96.);
/// The Quality param, which only A2 captures respond to.
const AMP_QUALITY: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Amp,
    Rig,
}

#[derive(Clone)]
struct Choice {
    value: String,
    title: SharedString,
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

fn device_choices(devices: &[DeviceOption]) -> Vec<Choice> {
    devices
        .iter()
        .map(|device| Choice {
            value: device.id.clone(),
            title: device.name.clone().into(),
        })
        .collect()
}

fn channel_choices(channels: u16) -> Vec<Choice> {
    (0..channels.max(1))
        .map(|channel| Choice {
            value: channel.to_string(),
            title: format!("Input {}", channel + 1).into(),
        })
        .collect()
}

fn position(devices: &[DeviceOption], id: Option<&str>) -> Option<IndexPath> {
    let id = id?;
    devices
        .iter()
        .position(|device| device.id == id)
        .map(IndexPath::new)
}

/// The main window: the devices, the amp, the chain, and the tone library.
pub struct Rig {
    settings: Settings,
    library: Library,
    presets: Vec<PresetEntry>,
    preset: Preset,
    preset_name: Entity<InputState>,
    amp_info: Option<AmpInfo>,
    view: View,

    devices: DeviceList,
    input_select: Entity<SelectState<Vec<Choice>>>,
    channel_select: Entity<SelectState<Vec<Choice>>>,
    output_select: Entity<SelectState<Vec<Choice>>>,

    engine: Option<Engine>,
    input_level: f32,
    output_level: f32,
    latency: Latency,
    glitches: u64,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
    _meter_timer: Task<()>,
}

impl Rig {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut settings = Settings::load();
        let mut error = None;
        let devices = DeviceList::scan().unwrap_or_else(|scan_error| {
            error = Some(scan_error.to_string().into());
            DeviceList::default()
        });
        if settings.input.is_none() {
            settings.input = devices.default_input.clone();
        }
        if settings.output.is_none() {
            settings.output = devices.default_output.clone();
        }
        let library =
            Library::open(settings.library_root()).expect("the app data folder should be writable");

        let selected = position(&devices.inputs, settings.input.as_deref());
        let input_select = cx.new(|cx| {
            SelectState::new(device_choices(&devices.inputs), selected, window, cx).searchable(true)
        });
        let channels = selected.map_or(1, |index| devices.inputs[index.row].channels);
        let channel = Some(IndexPath::new(
            settings.input_channel.min(usize::from(channels) - 1),
        ));
        let channel_select =
            cx.new(|cx| SelectState::new(channel_choices(channels), channel, window, cx));
        let selected = position(&devices.outputs, settings.output.as_deref());
        let output_select = cx.new(|cx| {
            SelectState::new(device_choices(&devices.outputs), selected, window, cx)
                .searchable(true)
        });

        let subscriptions = vec![
            cx.subscribe_in(
                &input_select,
                window,
                |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(id)) = event {
                        this.select_input(id.clone(), window, cx);
                    }
                },
            ),
            cx.subscribe(
                &channel_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(channel)) = event {
                        this.settings.input_channel = channel.parse().unwrap_or(0);
                        this.devices_changed(cx);
                    }
                },
            ),
            cx.subscribe(
                &output_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(id)) = event {
                        this.settings.output = Some(id.clone());
                        this.devices_changed(cx);
                    }
                },
            ),
        ];

        let meter_timer = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(METER_INTERVAL).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });

        let preset = Preset::starter("New tone");
        let name = preset.name.clone();
        let preset_name = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let mut rig = Self {
            settings,
            library,
            presets: Vec::new(),
            preset,
            preset_name,
            amp_info: None,
            view: View::Amp,
            devices,
            input_select,
            channel_select,
            output_select,
            engine: None,
            input_level: 0.0,
            output_level: 0.0,
            latency: Latency::default(),
            glitches: 0,
            error,
            _subscriptions: subscriptions,
            _meter_timer: meter_timer,
        };
        rig.refresh_presets();
        let last_tone = rig.settings.last_tone.as_deref();
        let last_tone = rig
            .presets
            .iter()
            .find(|entry| Some(entry.preset.name.as_str()) == last_tone)
            .map(|entry| entry.preset.clone());
        if let Some(preset) = last_tone {
            rig.load_preset(preset, window, cx);
        }
        rig
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = &mut self.engine else {
            return;
        };
        engine.drop_retired();
        let meters = engine.meters();
        // Fall back slowly, so a pick attack stays readable on the meter.
        self.input_level = meters.take_input_peak().max(self.input_level * 0.8);
        self.output_level = meters.take_output_peak().max(self.output_level * 0.8);
        self.latency = meters.latency();
        self.glitches = meters.xruns();
        cx.notify();
    }

    fn set_error(&mut self, error: impl ToString, cx: &mut Context<Self>) {
        self.error = Some(error.to_string().into());
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.settings.save() {
            self.set_error(error, cx);
        }
    }

    fn select_input(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
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
    fn devices_changed(&mut self, cx: &mut Context<Self>) {
        self.save_settings(cx);
        if self.engine.is_some() {
            self.engine = None;
            self.start_monitoring(cx);
        }
        cx.notify();
    }

    fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            input: self.settings.input.clone(),
            output: self.settings.output.clone(),
            input_channel: self.settings.input_channel,
        }
    }

    fn start_monitoring(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        let built = match build_chain(&self.preset, &self.library, ENGINE_RATE) {
            Ok(built) => built,
            Err(error) => return self.set_error(error, cx),
        };
        match Engine::start(Box::new(built.chain), &self.engine_config()) {
            Ok(engine) => self.engine = Some(engine),
            Err(error) => self.set_error(error, cx),
        }
        cx.notify();
    }

    fn toggle_monitoring(&mut self, cx: &mut Context<Self>) {
        if self.engine.take().is_none() {
            self.start_monitoring(cx);
        }
        self.input_level = 0.0;
        self.output_level = 0.0;
        cx.notify();
    }

    fn set_param(&mut self, slot: usize, param: usize, value: f32, cx: &mut Context<Self>) {
        let block = &mut self.preset.blocks[slot];
        let id = block.kind.params()[param].id;
        block.params.insert(id.to_owned(), value);
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetParam { slot, param, value });
        }
        cx.notify();
    }

    fn toggle_block(&mut self, slot: usize, cx: &mut Context<Self>) {
        let block = &mut self.preset.blocks[slot];
        block.enabled = !block.enabled;
        let bypassed = !block.enabled;
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetBypassed { slot, bypassed });
        }
        cx.notify();
    }

    /// Builds `preset` in the background and only adopts it if every model
    /// loads, so a bad file leaves the tone playing as it was.
    fn load_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let built = build_chain(&preset, &library, ENGINE_RATE)?;
                Ok::<_, amp_preset::LibraryError>((preset, built))
            });
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((preset, built)) => {
                    this.preset_name.update(cx, |input, cx| {
                        input.set_value(preset.name.clone(), window, cx);
                    });
                    this.settings.last_tone = Some(preset.name.clone());
                    this.save_settings(cx);
                    this.preset = preset;
                    this.amp_info = built.amp;
                    this.error = None;
                    if let Some(engine) = &mut this.engine {
                        engine.send(Command::SetChain(Box::new(built.chain)));
                    }
                    cx.notify();
                }
                Err(error) => this.set_error(error, cx),
            });
        })
        .detach();
    }

    fn pick_path(
        &self,
        directories: bool,
        prompt: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update_in(cx, |this, window, cx| then(this, path, window, cx));
            }
        })
        .detach();
    }

    fn load_asset(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = AssetKind::for_block(self.preset.blocks[slot].kind) else {
            return;
        };
        self.pick_path(
            false,
            "Load",
            window,
            cx,
            move |this, path, window, cx| match this.library.import_asset(kind, &path) {
                Ok(asset) => {
                    let mut preset = this.preset.clone();
                    preset.blocks[slot].asset = Some(asset);
                    this.load_preset(preset, window, cx);
                }
                Err(error) => this.set_error(error, cx),
            },
        );
    }

    fn save_preset(&mut self, cx: &mut Context<Self>) {
        self.preset.name = self.preset_name.read(cx).value().to_string();
        match self.library.save_preset(&self.preset) {
            Ok(_) => {
                self.settings.last_tone = Some(self.preset.name.clone());
                self.save_settings(cx);
                self.refresh_presets();
            }
            Err(error) => self.set_error(error, cx),
        }
        cx.notify();
    }

    fn refresh_presets(&mut self) {
        match self.library.presets() {
            Ok((entries, errors)) => {
                self.presets = entries;
                if let Some(error) = errors.first() {
                    self.error = Some(error.to_string().into());
                }
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
    }

    fn export_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Export here", window, cx, |this, path, _, cx| {
            if let Err(error) = this.library.export_pack(&this.preset, &path) {
                this.set_error(error, cx);
            }
        });
    }

    fn import_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Import", window, cx, |this, path, _, cx| {
            match this.library.import_pack(&path) {
                Ok(_) => this.refresh_presets(),
                Err(error) => this.error = Some(error.to_string().into()),
            }
            cx.notify();
        });
    }

    fn choose_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Use this folder", window, cx, |this, path, _, cx| {
            match Library::open(path.clone()) {
                Ok(library) => {
                    this.library = library;
                    this.settings.library = Some(path);
                    this.refresh_presets();
                    this.save_settings(cx);
                }
                Err(error) => this.error = Some(error.to_string().into()),
            }
            cx.notify();
        });
    }

    fn slot_of(&self, kind: BlockKind) -> Option<usize> {
        self.preset
            .blocks
            .iter()
            .position(|block| block.kind == kind)
    }

    fn knob(&self, slot: usize, param: usize, size: Pixels, cx: &mut Context<Self>) -> Knob {
        let block = &self.preset.blocks[slot];
        Knob::new(
            SharedString::from(format!("knob-{slot}-{param}")),
            block.kind.params()[param],
            block.param(param),
            size,
            cx.listener(move |this, value: &f32, _, cx| {
                this.set_param(slot, param, *value, cx);
            }),
        )
    }

    fn render_meter(&self, label: &'static str, peak: f32, cx: &App) -> impl IntoElement {
        let db = 20.0 * peak.max(1e-6).log10();
        let fill = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
        let color = if peak >= 0.99 {
            cx.theme().danger
        } else {
            cx.theme().success
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(div().text_xs().w(px(26.)).child(label))
            .child(
                div()
                    .w(px(110.))
                    .h(px(6.))
                    .rounded_sm()
                    .bg(cx.theme().muted)
                    .child(div().h_full().rounded_sm().w(relative(fill)).bg(color)),
            )
    }

    fn render_device_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let picker = |label: &'static str, select: AnyElement, cx: &App| {
            v_flex()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(select)
        };
        h_flex()
            .gap_3()
            .p_3()
            .items_end()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(picker(
                "Guitar in",
                Select::new(&self.input_select)
                    .placeholder("Choose your interface")
                    .w(px(280.))
                    .into_any_element(),
                cx,
            ))
            .child(picker(
                "Channel",
                Select::new(&self.channel_select)
                    .w(px(110.))
                    .into_any_element(),
                cx,
            ))
            .child(picker(
                "Speakers out",
                Select::new(&self.output_select)
                    .placeholder("Choose an output")
                    .w(px(280.))
                    .into_any_element(),
                cx,
            ))
            .child(
                Button::new("monitor")
                    .label(if live {
                        "Stop monitoring"
                    } else {
                        "Start monitoring"
                    })
                    .when(!live, |button| button.primary())
                    .when(live, |button| button.danger())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_monitoring(cx))),
            )
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap_1()
                    .child(self.render_meter("In", self.input_level, cx))
                    .child(self.render_meter("Out", self.output_level, cx))
                    .when(live, |column| {
                        column.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{:.0} ms latency, {} glitches",
                                    self.latency.total_ms(),
                                    self.glitches
                                )),
                        )
                    }),
            )
    }

    fn render_block(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let enabled = block.enabled;
        let kind = block.kind;
        let asset_kind = AssetKind::for_block(kind);
        let asset_label = block
            .asset
            .as_ref()
            .map(|asset| asset.name.clone())
            .unwrap_or_else(|| match asset_kind {
                Some(AssetKind::NamModel) => "No capture loaded".to_owned(),
                _ => "No cab IR loaded".to_owned(),
            });
        let params = kind.params().len();
        let knobs: Vec<_> = (0..params)
            // A capture with one size has nothing for Quality to choose.
            .filter(|&param| {
                kind != BlockKind::Amp
                    || param != AMP_QUALITY
                    || self
                        .amp_info
                        .as_ref()
                        .is_some_and(|amp| amp.quality_tiers > 1)
            })
            .map(|param| self.knob(slot, param, CARD_KNOB, cx))
            .collect();
        v_flex()
            .id(("block", slot))
            .w(px(250.))
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if enabled { accent() } else { cx.theme().border })
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_semibold().child(kind.title()))
                    .child(
                        Button::new(("toggle", slot))
                            .small()
                            .label(if enabled { "On" } else { "Off" })
                            .when(enabled, |button| button.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_block(slot, cx);
                            })),
                    ),
            )
            .when(asset_kind.is_some(), |card| {
                card.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .truncate()
                                .text_color(cx.theme().muted_foreground)
                                .child(asset_label),
                        )
                        .child(
                            Button::new(("load", slot))
                                .small()
                                .outline()
                                .label(match asset_kind {
                                    Some(AssetKind::NamModel) => "Load .nam capture",
                                    _ => "Load cab IR",
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.load_asset(slot, window, cx);
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_around()
                    .children(knobs),
            )
    }

    fn render_chip(&self, text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
        div()
            .px_2()
            .py(px(2.))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
    }

    fn render_amp_panel(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let info = self.amp_info.as_ref();
        let asset_name = block.asset.as_ref().map(|asset| asset.name.clone());
        let title = info
            .and_then(|info| info.name.clone().or_else(|| info.gear.clone()))
            .or(asset_name)
            .unwrap_or_else(|| "No amp loaded".to_owned());
        let byline = info.and_then(|info| match (&info.gear, &info.modeled_by) {
            (Some(gear), Some(by)) => Some(format!("{gear}, captured by {by}")),
            (Some(gear), None) => Some(gear.clone()),
            (None, Some(by)) => Some(format!("Captured by {by}")),
            (None, None) => None,
        });
        let mut chips: Vec<AnyElement> = Vec::new();
        if let Some(info) = info {
            let kind = if info.quality_tiers > 1 {
                format!("NAM A2, {} quality sizes", info.quality_tiers)
            } else {
                format!("NAM {}", info.architecture)
            };
            chips.push(self.render_chip(kind, cx).into_any_element());
            if let Some(tone) = &info.tone_type {
                chips.push(self.render_chip(tone.clone(), cx).into_any_element());
            }
            if info.includes_cab == Some(true) {
                chips.push(self.render_chip("Cab in capture", cx).into_any_element());
            }
        }
        let has_quality = info.is_some_and(|info| info.quality_tiers > 1);
        let loaded = info.is_some();
        let knobs: Vec<_> = (0..AMP_QUALITY + usize::from(has_quality))
            .map(|param| self.knob(slot, param, AMP_KNOB, cx))
            .collect();

        v_flex()
            .id("amp-panel")
            .gap_4()
            .p_5()
            .rounded_lg()
            .border_1()
            .border_color(if block.enabled {
                accent()
            } else {
                cx.theme().border
            })
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .min_w_0()
                            .child(div().text_2xl().font_semibold().child(title))
                            .when_some(byline, |column, byline| {
                                column.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(byline),
                                )
                            }),
                    )
                    .child(
                        Button::new("load-capture")
                            .outline()
                            .label(if loaded { "Change capture" } else { "Load .nam capture" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.load_asset(slot, window, cx);
                            })),
                    ),
            )
            .when(!chips.is_empty(), |panel| {
                panel.child(h_flex().flex_wrap().gap_2().children(chips))
            })
            .when(!loaded, |panel| {
                panel.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Load a capture to hear an amp. Until then the guitar passes through clean."),
                )
            })
            .child(h_flex().flex_wrap().gap_6().justify_center().children(knobs))
    }

    fn render_signal_path(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut path: Vec<AnyElement> = Vec::new();
        for (slot, block) in self.preset.blocks.iter().enumerate() {
            if slot > 0 {
                path.push(
                    div()
                        .w(px(16.))
                        .h(px(1.))
                        .bg(cx.theme().border)
                        .into_any_element(),
                );
            }
            path.push(
                Button::new(("path", slot))
                    .small()
                    .label(block.kind.title())
                    .when(block.enabled, |button| button.primary())
                    .when(!block.enabled, |button| button.outline())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_block(slot, cx);
                    }))
                    .into_any_element(),
            );
        }
        h_flex().flex_wrap().items_center().gap_1().children(path)
    }

    fn render_amp_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let amp = self.slot_of(BlockKind::Amp);
        let cab = self.slot_of(BlockKind::Cab);
        let gate = self.slot_of(BlockKind::Gate);
        let cab_doubled = cab.is_some_and(|cab| {
            let cab = &self.preset.blocks[cab];
            cab.enabled && cab.asset.is_some()
        }) && self
            .amp_info
            .as_ref()
            .is_some_and(|info| info.includes_cab == Some(true));

        v_flex()
            .gap_3()
            .child(self.render_signal_path(cx))
            .when_some(amp, |column, amp| {
                column.child(self.render_amp_panel(amp, cx))
            })
            .when(cab_doubled, |column| {
                column.child(
                    div()
                        .text_sm()
                        .text_color(accent())
                        .child("This capture already includes a cabinet, so you are hearing two. Turn off the Cabinet block."),
                )
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .items_start()
                    .gap_3()
                    .when_some(gate, |row, gate| row.child(self.render_block(gate, cx)))
                    .when_some(cab, |row, cab| row.child(self.render_block(cab, cx))),
            )
    }

    fn render_rig_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let blocks: Vec<_> = (0..self.preset.blocks.len())
            .map(|slot| self.render_block(slot, cx).into_any_element())
            .collect();
        v_flex()
            .gap_3()
            .child(self.render_signal_path(cx))
            .child(h_flex().flex_wrap().items_start().gap_3().children(blocks))
    }

    fn render_library(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .gap_2()
            .p_3()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(div().font_semibold().child("Tone library"))
            .child(Input::new(&self.preset_name))
            .child(
                Button::new("save")
                    .small()
                    .primary()
                    .label("Save tone")
                    .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx))),
            )
            .child(
                v_flex()
                    .id("presets")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_1()
                    .children(self.presets.iter().enumerate().map(|(index, entry)| {
                        let preset = entry.preset.clone();
                        Button::new(("preset", index))
                            .small()
                            .ghost()
                            .label(entry.preset.name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.load_preset(preset.clone(), window, cx);
                            }))
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("export")
                            .small()
                            .outline()
                            .label("Export")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_pack(window, cx);
                            })),
                    )
                    .child(
                        Button::new("import")
                            .small()
                            .outline()
                            .label("Import")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_pack(window, cx);
                            })),
                    )
                    .child(
                        Button::new("folder")
                            .small()
                            .outline()
                            .label("Folder")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_library(window, cx);
                            })),
                    ),
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
            .when(self.view == view, |button| button.primary())
            .when(self.view != view, |button| button.ghost())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.view = view;
                cx.notify();
            }))
    }
}

impl Render for Rig {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.view {
            View::Amp => self.render_amp_view(cx).into_any_element(),
            View::Rig => self.render_rig_view(cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_device_bar(cx))
            .when_some(self.error.clone(), |column, error| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(self.render_library(cx))
                    .child(
                        v_flex()
                            .id("main")
                            .flex_1()
                            .h_full()
                            .overflow_y_scroll()
                            .gap_3()
                            .p_3()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(self.render_tab("tab-amp", "Amp", View::Amp, cx))
                                    .child(self.render_tab("tab-rig", "Full rig", View::Rig, cx)),
                            )
                            .child(body)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        "Hearing a clean signal under the amp? Turn off direct \
                                         monitoring on your interface, so only this app reaches \
                                         your speakers.",
                                    ),
                            ),
                    ),
            )
    }
}
