mod audio;
mod board;
mod explorer;
mod header;
mod practice;
mod tone3000;
mod tuner;

use crate::{knob::Knob, settings::Settings};
use amp_dsp::{AmpInfo, BlockKind, ParamSpec, Pitch};
use amp_engine::{
    Command, DeviceList, DeviceOption, ENGINE_RATE, Engine, EngineConfig, Latency, LatencyMode,
};
use amp_preset::{
    AssetKind, BlockPreset, CaptureEntry, Library, LibraryError, Preset, PresetEntry, build_chain,
};
use explorer::Explorer;
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::InputState,
        select::{SelectEvent, SelectItem, SelectState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use practice::Practice;
use std::{path::PathBuf, time::Duration};
use tone3000::Tone3000;

const METER_INTERVAL: Duration = Duration::from_millis(50);
const METER_FLOOR_DB: f32 = -60.0;
/// How many edits Undo can step back through.
const HISTORY: usize = 64;
/// Waits this long after a level knob stops before writing settings to disk.
const SETTINGS_SAVE_DELAY: f32 = 1.0;
const FIRST_TONE: &str = "Moss Clean";
const HEADER_HEIGHT: f32 = 72.0;
const FOOTER_HEIGHT: f32 = 36.0;
const KNOB_INPUT: ParamSpec = ParamSpec {
    id: "input_level",
    name: "Input",
    unit: "dB",
    min: -24.0,
    max: 24.0,
    default: 0.0,
};
const KNOB_OUTPUT: ParamSpec = ParamSpec {
    id: "output_level",
    name: "Output",
    unit: "dB",
    min: -40.0,
    max: 12.0,
    default: 0.0,
};

actions!(
    rig,
    [
        Undo,
        Redo,
        RemoveUnit,
        BypassUnit,
        OpenUnit,
        CloseUnit,
        PreviousUnit,
        NextUnit,
        MoveUnitLeft,
        MoveUnitRight,
        PreviousTone,
        NextTone,
        ToggleMonitoring,
        ToggleTuner,
        ToggleTones,
        ToggleGear
    ]
);

/// Single letters that act on the rig. Text fields sit inside the rig, so these
/// are switched off in them, or typing a tone's name would bypass a pedal.
const LETTER_KEYS: [&str; 4] = ["b", "g", "m", "t"];

/// Keys that act on the board.
pub fn bind_keys(cx: &mut App) {
    let rig = Some("Rig");
    cx.bind_keys([
        KeyBinding::new("ctrl-z", Undo, rig),
        KeyBinding::new("ctrl-y", Redo, rig),
        KeyBinding::new("ctrl-shift-z", Redo, rig),
        KeyBinding::new("delete", RemoveUnit, rig),
        KeyBinding::new("b", BypassUnit, rig),
        KeyBinding::new("enter", OpenUnit, rig),
        KeyBinding::new("escape", CloseUnit, rig),
        KeyBinding::new("left", PreviousUnit, rig),
        KeyBinding::new("right", NextUnit, rig),
        KeyBinding::new("alt-left", MoveUnitLeft, rig),
        KeyBinding::new("alt-right", MoveUnitRight, rig),
        KeyBinding::new("pageup", PreviousTone, rig),
        KeyBinding::new("pagedown", NextTone, rig),
        KeyBinding::new("m", ToggleMonitoring, rig),
        KeyBinding::new("t", ToggleTuner, rig),
        KeyBinding::new("ctrl-o", ToggleTones, rig),
        KeyBinding::new("g", ToggleGear, rig),
    ]);
    cx.bind_keys(
        LETTER_KEYS
            .iter()
            .map(|key| KeyBinding::new(key, NoAction, Some("Input"))),
    );
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}
const ASIO_SCAN_DELAY: Duration = Duration::from_millis(300);
const WINDOWS_AUDIO: &str = "Windows audio";
/// The Quality param, which only A2 captures respond to.
const AMP_QUALITY: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Board,
    Practice,
}

/// A panel open over the main view.
#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    None,
    Tones,
    Audio,
    Tone3000,
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

fn driver_choices(asio: &[DeviceOption]) -> Vec<Choice> {
    let mut choices = vec![Choice {
        value: WINDOWS_AUDIO.to_owned(),
        title: WINDOWS_AUDIO.into(),
    }];
    choices.extend(device_choices(asio));
    choices
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
    /// The board as last saved or loaded, to tell when there are unsaved edits.
    saved_blocks: Vec<BlockPreset>,
    history: Vec<Preset>,
    future: Vec<Preset>,
    /// The knob being turned, so one long turn is one step of undo.
    last_param: Option<(usize, usize)>,
    focus: FocusHandle,
    settings_save_at: Option<f32>,
    tuner_open: bool,
    tuner_samples: Vec<f32>,
    pitch: Option<Pitch>,
    view: View,
    practice: Practice,
    overlay: Overlay,
    explorer: Explorer,
    tone3000: Tone3000,
    gear_open: bool,

    selected: Option<usize>,
    hover_gap: Option<usize>,
    smoke_phase: f32,

    devices: DeviceList,
    asio_devices: Vec<DeviceOption>,
    driver_select: Entity<SelectState<Vec<Choice>>>,
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
        if let Err(seed_error) = library.seed_factory_tones() {
            error = Some(seed_error.to_string().into());
        }

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

        let driver_select =
            cx.new(|cx| SelectState::new(driver_choices(&[]), Some(IndexPath::new(0)), window, cx));

        let subscriptions = vec![
            cx.subscribe_in(
                &driver_select,
                window,
                |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(driver)) = event {
                        this.select_driver(driver.clone(), window, cx);
                    }
                },
            ),
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
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let explorer = Explorer::new(window, cx);
        let mut rig = Self {
            settings,
            library,
            presets: Vec::new(),
            catalog: Vec::new(),
            preset,
            preset_name,
            captures,
            build: 0,
            saved_blocks: Vec::new(),
            history: Vec::new(),
            future: Vec::new(),
            last_param: None,
            focus: focus.clone(),
            settings_save_at: None,
            tuner_open: false,
            tuner_samples: Vec::new(),
            pitch: None,
            view: View::Board,
            practice: Practice::default(),
            overlay: Overlay::None,
            explorer,
            tone3000: Tone3000::new(),
            gear_open: false,
            selected: None,
            hover_gap: None,
            smoke_phase: 0.0,
            devices,
            asio_devices: Vec::new(),
            driver_select,
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
            .or(rig
                .presets
                .iter()
                .find(|entry| entry.preset.name == FIRST_TONE))
            .or(rig.presets.first())
            .map(|entry| entry.preset.clone());
        if let Some(preset) = last_tone {
            rig.load_preset(preset, window, cx);
        }
        // Loading ASIO drivers takes a moment, so it waits for the first frame.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(ASIO_SCAN_DELAY).await;
            let _ = this.update_in(cx, |this, window, cx| this.scan_asio(window, cx));
        })
        .detach();
        rig
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.smoke_phase += METER_INTERVAL.as_secs_f32();
        if self
            .settings_save_at
            .is_some_and(|at| self.smoke_phase >= at)
        {
            self.settings_save_at = None;
            self.save_settings(cx);
        }
        if self.tuner_open {
            self.read_tuner();
        }
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

    fn scan_asio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn select_driver(&mut self, driver: String, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.asio_driver = (driver != WINDOWS_AUDIO).then_some(driver);
        self.settings.input_channel = 0;
        self.show_channels(window, cx);
        self.devices_changed(cx);
    }

    /// Lists the inputs of whichever device the guitar is coming through.
    fn show_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            asio_driver: self.settings.asio_driver.clone(),
        }
    }

    /// Starts on the fastest route available. Something else may hold a device,
    /// and playing with more latency beats not playing, so each slower route
    /// is tried in turn.
    fn start_monitoring(&mut self, cx: &mut Context<Self>) {
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

    fn apply_levels(&mut self) {
        let tuner_open = self.tuner_open;
        let input = db_to_gain(self.settings.input_db);
        let output = db_to_gain(self.settings.output_db);
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetInputGain(input));
            engine.send(Command::SetOutputGain(output));
            engine.meters().set_tuner(tuner_open);
        }
    }

    fn set_level(&mut self, input: Option<f32>, output: Option<f32>, cx: &mut Context<Self>) {
        if let Some(db) = input {
            self.settings.input_db = db;
        }
        if let Some(db) = output {
            self.settings.output_db = db;
        }
        self.settings_save_at = Some(self.smoke_phase + SETTINGS_SAVE_DELAY);
        self.apply_levels();
        cx.notify();
    }

    /// Keeps the board as it is now, for Undo.
    fn remember(&mut self) {
        self.history.push(self.preset.clone());
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.future.clear();
        self.last_param = None;
    }

    fn restore(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.captures = vec![None; preset.blocks.len()];
        // Stay on the unit being worked on, if it's still there.
        self.selected = self.selected.filter(|slot| *slot < preset.blocks.len());
        self.preset = preset;
        self.last_param = None;
        self.rebuild_chain(cx);
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(preset) = self.history.pop() {
            self.future
                .push(std::mem::replace(&mut self.preset, preset.clone()));
            self.restore(preset, cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(preset) = self.future.pop() {
            self.history
                .push(std::mem::replace(&mut self.preset, preset.clone()));
            self.restore(preset, cx);
        }
    }

    fn step_tone(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.presets.len() as isize;
        if count == 0 {
            return;
        }
        let current = self
            .presets
            .iter()
            .position(|entry| entry.preset.name == self.preset.name)
            .map_or(0, |index| index as isize);
        let next = (current + step).rem_euclid(count) as usize;
        let preset = self.presets[next].preset.clone();
        self.load_preset(preset, window, cx);
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
        if self.last_param != Some((slot, param)) {
            self.remember();
            self.last_param = Some((slot, param));
        }
        let block = &mut self.preset.blocks[slot];
        let id = block.kind.params()[param].id;
        block.params.insert(id.to_owned(), value);
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetParam { slot, param, value });
        }
        cx.notify();
    }

    fn toggle_block(&mut self, slot: usize, cx: &mut Context<Self>) {
        self.remember();
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
        self.want_block_images(cx);
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
        if self.restore_tone_models(&preset, window, cx) {
            return;
        }
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
                        this.note_recent(&preset.name);
                        this.save_settings(cx);
                        this.saved_blocks = preset.blocks.clone();
                        this.history.clear();
                        this.future.clear();
                        this.last_param = None;
                        // Open on the amp, the first thing most people reach for.
                        this.selected = preset.blocks.iter().position(|block| {
                            matches!(block.kind, BlockKind::Amp | BlockKind::Tube)
                        });
                        this.preset = preset;
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
                    this.remember();
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
                self.saved_blocks = self.preset.blocks.clone();
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

    fn input_is_microphone(&self) -> bool {
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
}

impl Render for Rig {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.view {
            View::Board => self.render_board_view(window, cx).into_any_element(),
            View::Practice => v_flex()
                .id("practice")
                .size_full()
                .overflow_y_scroll()
                .gap_4()
                .p_4()
                .when(self.tuner_open, |column| {
                    column.child(self.render_tuner(cx))
                })
                .child(self.render_practice_view(cx))
                .into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .key_context("Rig")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.redo(cx)))
            .on_action(cx.listener(|this, _: &RemoveUnit, _, cx| this.remove_selected(cx)))
            .on_action(cx.listener(|this, _: &BypassUnit, _, cx| this.bypass_selected(cx)))
            .on_action(cx.listener(|this, _: &OpenUnit, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &CloseUnit, window, cx| {
                this.close_selected(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousUnit, _, cx| this.select_step(-1, cx)))
            .on_action(cx.listener(|this, _: &NextUnit, _, cx| this.select_step(1, cx)))
            .on_action(cx.listener(|this, _: &MoveUnitLeft, _, cx| this.move_selected(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveUnitRight, _, cx| this.move_selected(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTone, window, cx| {
                this.step_tone(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTone, window, cx| {
                this.step_tone(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMonitoring, _, cx| {
                this.toggle_monitoring(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTuner, _, cx| this.toggle_tuner(cx)))
            .on_action(cx.listener(|this, _: &ToggleTones, window, cx| {
                this.toggle_tones(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleGear, _, cx| this.toggle_gear(cx)))
            .bg(hsla(0.42, 0.3, 0.035, 1.0))
            .text_color(cx.theme().foreground)
            .child(self.render_backdrop())
            .child(
                v_flex()
                    .relative()
                    .size_full()
                    .child(self.render_header(cx))
                    .when_some(self.error.clone(), |column, error| {
                        column.child(
                            h_flex()
                                .px_3()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .justify_between()
                                .child(div().text_sm().text_color(cx.theme().danger).child(error))
                                .child(
                                    Button::new("dismiss-error")
                                        .small()
                                        .ghost()
                                        .label("Dismiss")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.error = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .when(self.input_is_microphone(), |column| {
                        column.child(
                            div()
                                .px_3()
                                .py_1()
                                .text_sm()
                                .text_color(cx.theme().warning)
                                .child(
                                    "Guitar in is set to a microphone, which can feed back through \
                                     speakers. Choose your interface in Audio setup first.",
                                ),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .child(body)
                            .when(self.overlay == Overlay::Tones, |main| {
                                main.child(self.render_explorer(cx))
                            })
                            .when(self.overlay == Overlay::Audio, |main| {
                                main.child(self.render_audio_panel(cx))
                            })
                            .when(self.overlay == Overlay::Tone3000, |main| {
                                main.child(self.render_tone3000(cx))
                            }),
                    )
                    .child(self.render_footer(cx)),
            )
    }
}
