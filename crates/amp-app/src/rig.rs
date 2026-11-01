mod board;
mod practice;

use crate::{
    knob::Knob,
    settings::Settings,
    style::{fog, hairline, pane},
};
use amp_dsp::AmpInfo;
use amp_engine::{
    Command, DeviceList, DeviceOption, ENGINE_RATE, Engine, EngineConfig, Latency, LatencyMode,
};
use amp_preset::{
    AssetKind, CaptureEntry, Library, LibraryError, Preset, PresetEntry, build_chain,
};
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
use practice::Practice;
use std::{path::PathBuf, time::Duration};

const METER_INTERVAL: Duration = Duration::from_millis(50);
const METER_FLOOR_DB: f32 = -60.0;
/// The Quality param, which only A2 captures respond to.
const AMP_QUALITY: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Board,
    Practice,
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

/// The main window: the devices, the board, and the tone library.
pub struct Rig {
    settings: Settings,
    library: Library,
    presets: Vec<PresetEntry>,
    catalog: Vec<CaptureEntry>,
    preset: Preset,
    preset_name: Entity<InputState>,
    /// What each block's capture says about itself, in chain order.
    captures: Vec<Option<AmpInfo>>,
    /// Counts chain builds, so a slow one can't overwrite a newer edit.
    build: u64,
    view: View,
    practice: Practice,

    selected: Option<usize>,
    settings_open: bool,
    hover_gap: Option<usize>,
    smoke_phase: f32,

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
        let captures = vec![None; preset.blocks.len()];
        let mut rig = Self {
            settings,
            library,
            presets: Vec::new(),
            catalog: Vec::new(),
            preset,
            preset_name,
            captures,
            build: 0,
            view: View::Board,
            practice: Practice::default(),
            selected: None,
            settings_open: false,
            hover_gap: None,
            smoke_phase: 0.0,
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
        rig.refresh_sheets();
        rig.refresh_catalog();
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
        self.smoke_phase += METER_INTERVAL.as_secs_f32();
        if let Some(engine) = &mut self.engine {
            engine.drop_retired();
            let meters = engine.meters();
            // Fall back slowly, so a pick attack stays readable on the meter.
            self.input_level = meters.take_input_peak().max(self.input_level * 0.8);
            self.output_level = meters.take_output_peak().max(self.output_level * 0.8);
            self.latency = meters.latency();
            self.glitches = meters.xruns();
        }
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

    fn engine_config(&self, mode: LatencyMode) -> EngineConfig {
        EngineConfig {
            input: self.settings.input.clone(),
            output: self.settings.output.clone(),
            input_channel: self.settings.input_channel,
            mode,
        }
    }

    fn start_monitoring(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        let built = match build_chain(&self.preset, &self.library, ENGINE_RATE) {
            Ok(built) => built,
            Err(error) => return self.set_error(error, cx),
        };
        if !self.settings.shared_audio {
            let config = self.engine_config(LatencyMode::Exclusive);
            match Engine::start(Box::new(built.chain), &config) {
                Ok(engine) => {
                    self.engine = Some(engine);
                    return cx.notify();
                }
                // Something else may hold the device, and playing with more
                // latency beats not playing. The chain went with the failed
                // start, so build another.
                Err(error) => self.error = Some(format!("{error}. Using shared audio.").into()),
            }
        } else {
            return self.start_shared(built.chain, cx);
        }
        match build_chain(&self.preset, &self.library, ENGINE_RATE) {
            Ok(built) => self.start_shared(built.chain, cx),
            Err(error) => self.set_error(error, cx),
        }
    }

    fn start_shared(&mut self, chain: amp_dsp::Chain, cx: &mut Context<Self>) {
        let config = self.engine_config(LatencyMode::Shared);
        match Engine::start(Box::new(chain), &config) {
            Ok(engine) => self.engine = Some(engine),
            Err(error) => self.set_error(error, cx),
        }
        cx.notify();
    }

    fn toggle_shared_audio(&mut self, cx: &mut Context<Self>) {
        self.settings.shared_audio = !self.settings.shared_audio;
        self.devices_changed(cx);
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

    /// Rebuilds the running chain in the background after a board edit. Only
    /// the newest build counts, so quick drags can't arrive out of order.
    fn rebuild_chain(&mut self, cx: &mut Context<Self>) {
        self.build += 1;
        let build = self.build;
        let preset = self.preset.clone();
        let library = self.library.clone();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { build_chain(&preset, &library, ENGINE_RATE) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.build != build {
                    return;
                }
                match result {
                    Ok(built) => this.adopt_chain(built, cx),
                    Err(error) => this.set_error(error, cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn adopt_chain(&mut self, built: amp_preset::BuiltChain, cx: &mut Context<Self>) {
        self.captures = built.captures;
        self.error = None;
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetChain(Box::new(built.chain)));
            // Knobs may have moved while the chain was building.
            for (slot, block) in self.preset.blocks.iter().enumerate() {
                for param in 0..block.kind.params().len() {
                    engine.send(Command::SetParam {
                        slot,
                        param,
                        value: block.param(param),
                    });
                }
            }
        }
        cx.notify();
    }

    /// Builds `preset` in the background and only adopts it if every model
    /// loads, so a bad file leaves the tone playing as it was.
    fn load_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        self.build += 1;
        let build = self.build;
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let built = build_chain(&preset, &library, ENGINE_RATE)?;
                Ok::<_, LibraryError>((preset, built))
            });
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.build != build {
                    return;
                }
                match result {
                    Ok((preset, built)) => {
                        this.preset_name.update(cx, |input, cx| {
                            input.set_value(preset.name.clone(), window, cx);
                        });
                        this.settings.last_tone = Some(preset.name.clone());
                        this.save_settings(cx);
                        this.preset = preset;
                        this.selected = None;
                        this.settings_open = false;
                        this.adopt_chain(built, cx);
                    }
                    Err(error) => this.set_error(error, cx),
                }
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

    /// Adds `.nam` files to the rack. One bad file shouldn't stop the rest of a
    /// batch, so the first error is shown after everything else is in.
    fn add_captures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add captures".into()),
        });
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let task = cx.background_executor().spawn(async move {
                let mut first_error = None;
                for path in paths {
                    if let Err(error) = library.add_capture(&path) {
                        first_error.get_or_insert(error.to_string());
                    }
                }
                first_error
            });
            let error = task.await;
            let _ = this.update(cx, |this, cx| {
                this.refresh_catalog();
                if let Some(error) = error {
                    this.error = Some(error.into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_cab_ir(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(
            false,
            "Load",
            window,
            cx,
            move |this, path, _, cx| match this
                .library
                .import_asset(AssetKind::ImpulseResponse, &path)
            {
                Ok(asset) => {
                    this.preset.blocks[slot].asset = Some(asset);
                    this.rebuild_chain(cx);
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

    fn refresh_catalog(&mut self) {
        match self.library.captures() {
            Ok(catalog) => self.catalog = catalog,
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
                    this.refresh_sheets();
                    this.refresh_catalog();
                    this.save_settings(cx);
                }
                Err(error) => this.error = Some(error.to_string().into()),
            }
            cx.notify();
        });
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
                    .h(px(4.))
                    .rounded_sm()
                    .bg(hairline())
                    .child(div().h_full().rounded_sm().w(relative(fill)).bg(color)),
            )
    }

    fn render_device_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live = self.engine.is_some();
        let picker = |label: &'static str, select: AnyElement| {
            v_flex()
                .gap_1()
                .min_w_0()
                .child(div().text_xs().text_color(fog()).child(label))
                .child(select)
        };
        h_flex()
            .gap_3()
            .p_3()
            .items_end()
            .border_b_1()
            .border_color(hairline())
            .bg(pane())
            .child(picker(
                "Guitar in",
                Select::new(&self.input_select)
                    .placeholder("Choose your interface")
                    .w(px(260.))
                    .into_any_element(),
            ))
            .child(picker(
                "Channel",
                Select::new(&self.channel_select)
                    .w(px(100.))
                    .into_any_element(),
            ))
            .child(picker(
                "Speakers out",
                Select::new(&self.output_select)
                    .placeholder("Choose an output")
                    .w(px(260.))
                    .into_any_element(),
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
            .child(
                Button::new("latency-mode")
                    .outline()
                    .tooltip("Low latency takes over your audio devices, so other apps go silent on them while monitoring")
                    .label(if self.settings.shared_audio {
                        "Shared audio"
                    } else {
                        "Low latency"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_shared_audio(cx))),
            )
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap_1()
                    .child(self.render_meter("In", self.input_level, cx))
                    .child(self.render_meter("Out", self.output_level, cx))
                    .when(live, |column| {
                        column.child(div().text_xs().text_color(fog()).child(format!(
                            "{:.0} ms latency ({}), {} glitches",
                            self.latency.total_ms(),
                            self.engine.as_ref().map_or("off", |engine| {
                                match engine.info().mode {
                                    LatencyMode::Exclusive => "exclusive",
                                    LatencyMode::Shared => "shared",
                                }
                            }),
                            self.glitches
                        )))
                    }),
            )
    }

    fn render_tones(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(hairline())
            .child(div().font_semibold().child("Tones"))
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
                            .ghost()
                            .label("Export")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_pack(window, cx);
                            })),
                    )
                    .child(
                        Button::new("import")
                            .small()
                            .ghost()
                            .label("Import")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_pack(window, cx);
                            })),
                    )
                    .child(
                        Button::new("folder")
                            .small()
                            .ghost()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.view {
            View::Board => self.render_board_view(window, cx).into_any_element(),
            View::Practice => self.render_practice_view(cx).into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .bg(hsla(0.42, 0.3, 0.035, 0.82))
            .text_color(cx.theme().foreground)
            .child(self.render_backdrop())
            .child(
                v_flex()
                    .relative()
                    .size_full()
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
                            .child(
                                v_flex()
                                    .w(px(232.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .gap_3()
                                    .p_3()
                                    .border_r_1()
                                    .border_color(hairline())
                                    .bg(pane())
                                    .child(self.render_rack(cx))
                                    .child(self.render_tones(cx)),
                            )
                            .child(
                                v_flex()
                                    .id("main")
                                    .flex_1()
                                    .h_full()
                                    .overflow_y_scroll()
                                    .gap_4()
                                    .p_4()
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(self.render_tab(
                                                "tab-board",
                                                "Board",
                                                View::Board,
                                                cx,
                                            ))
                                            .child(self.render_tab(
                                                "tab-practice",
                                                "Practice",
                                                View::Practice,
                                                cx,
                                            )),
                                    )
                                    .child(body)
                                    .child(div().text_xs().text_color(fog()).child(
                                        "Hearing a clean signal under the amp? Turn off direct \
                                         monitoring on your interface, so only this app reaches \
                                         your speakers.",
                                    )),
                            ),
                    ),
            )
    }
}
