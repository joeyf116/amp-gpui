use crate::settings::Settings;
use amp_engine::{Command, DeviceList, DeviceOption, ENGINE_RATE, Engine, EngineConfig, Latency};
use amp_preset::{AssetKind, Library, Preset, PresetEntry, build_chain};
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputState},
        select::{Select, SelectEvent, SelectItem, SelectState},
        slider::{Slider, SliderEvent, SliderState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{path::PathBuf, time::Duration};

const METER_INTERVAL: Duration = Duration::from_millis(50);
const METER_FLOOR_DB: f32 = -60.0;

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

/// The main window: the devices, the signal chain, and the preset library.
pub struct Rig {
    settings: Settings,
    library: Library,
    presets: Vec<PresetEntry>,
    preset: Preset,
    preset_name: Entity<InputState>,
    sliders: Vec<Vec<Entity<SliderState>>>,
    slider_subscriptions: Vec<Subscription>,

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
            sliders: Vec::new(),
            slider_subscriptions: Vec::new(),
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
        rig.rebuild_sliders(cx);
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
        if let Err(error) = self.settings.save() {
            self.set_error(error, cx);
        }
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
        let chain = match build_chain(&self.preset, &self.library, ENGINE_RATE) {
            Ok(chain) => chain,
            Err(error) => return self.set_error(error, cx),
        };
        match Engine::start(Box::new(chain), &self.engine_config()) {
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

    fn rebuild_sliders(&mut self, cx: &mut Context<Self>) {
        self.sliders.clear();
        self.slider_subscriptions.clear();
        for (slot, block) in self.preset.blocks.iter().enumerate() {
            let mut row = Vec::new();
            for (param, spec) in block.kind.params().iter().enumerate() {
                let value = block.param(param);
                let state = cx.new(|_| {
                    SliderState::new()
                        .min(spec.min)
                        .max(spec.max)
                        .step((spec.max - spec.min) / 200.0)
                        .default_value(value)
                });
                self.slider_subscriptions.push(cx.subscribe(
                    &state,
                    move |this, _, event: &SliderEvent, cx| {
                        if let SliderEvent::Change(value) = event {
                            this.set_param(slot, param, value.end(), cx);
                        }
                    },
                ));
                row.push(state);
            }
            self.sliders.push(row);
        }
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
                let chain = build_chain(&preset, &library, ENGINE_RATE)?;
                Ok::<_, amp_preset::LibraryError>((preset, chain))
            });
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((preset, chain)) => {
                    this.preset_name.update(cx, |input, cx| {
                        input.set_value(preset.name.clone(), window, cx);
                    });
                    this.preset = preset;
                    this.rebuild_sliders(cx);
                    this.error = None;
                    if let Some(engine) = &mut this.engine {
                        engine.send(Command::SetChain(Box::new(chain)));
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
            Ok(_) => self.refresh_presets(),
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
                    if let Err(error) = this.settings.save() {
                        this.error = Some(error.to_string().into());
                    }
                }
                Err(error) => this.error = Some(error.to_string().into()),
            }
            cx.notify();
        });
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
        let asset_kind = AssetKind::for_block(block.kind);
        let asset_label = block
            .asset
            .as_ref()
            .map(|asset| asset.name.clone())
            .unwrap_or_else(|| match asset_kind {
                Some(AssetKind::NamModel) => "No model loaded".to_owned(),
                _ => "No cab IR loaded".to_owned(),
            });
        v_flex()
            .id(("block", slot))
            .w(px(220.))
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if enabled {
                cx.theme().primary
            } else {
                cx.theme().border
            })
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_semibold().child(block.kind.title()))
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
            .children(block.kind.params().iter().enumerate().map(|(param, spec)| {
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(spec.name)
                            .child(format!("{:.1} {}", block.param(param), spec.unit)),
                    )
                    .child(Slider::new(&self.sliders[slot][param]).accessibility_label(spec.name))
            }))
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
}

impl Render for Rig {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let blocks: Vec<_> = (0..self.preset.blocks.len())
            .map(|slot| self.render_block(slot, cx).into_any_element())
            .collect();
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
                            .id("chain")
                            .flex_1()
                            .h_full()
                            .overflow_y_scroll()
                            .gap_3()
                            .p_3()
                            .child(h_flex().flex_wrap().items_start().gap_3().children(blocks))
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
