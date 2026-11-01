mod assistant;
mod assistant_view;
mod audio;
mod board;
mod explorer;
mod fretboard;
mod header;
mod keys;
mod song;
mod song_export;
mod song_view;
mod tone3000;
mod tones;
mod tools;
mod update;

use crate::{
    knob::Knob,
    settings::{Settings, data_dir},
    style,
};
use amp_dsp::{AmpInfo, ParamSpec, Pitch};
use amp_engine::{Command, DeviceList, DeviceOption, ENGINE_RATE, Engine, Latency};
use amp_preset::{BlockPreset, CaptureEntry, Library, Preset, PresetEntry};
use assistant::Assistant;
use audio::{Choice, channel_choices, device_choices, driver_choices, position};
use explorer::Explorer;
use fretboard::Fretboard;
use gpui_kit::{
    component::{
        ActiveTheme, IndexPath, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::InputState,
        select::{SelectEvent, SelectState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
pub use keys::bind_keys;
use keys::*;
use std::time::{Duration, Instant};
use tone3000::Tone3000;
use update::Update;

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

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}
const ASIO_SCAN_DELAY: Duration = Duration::from_millis(300);
/// The Quality param, which only A2 captures respond to.
const AMP_QUALITY: usize = 2;

/// A panel open over the main view.
#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    None,
    Tones,
    Audio,
    Tools,
    Tone3000,
}

/// A panel's Close button.
fn close_button(id: &'static str, cx: &mut Context<Rig>) -> Button {
    Button::new(id)
        .small()
        .ghost()
        .label("Close")
        .on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
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
    /// Set until the first tone loads, which isn't an edit to undo.
    starting: bool,
    history: Vec<Preset>,
    future: Vec<Preset>,
    /// The knob being turned, so one long turn is one step of undo.
    last_param: Option<(usize, usize)>,
    focus: FocusHandle,
    settings_save_at: Option<f32>,
    tuner_open: bool,
    tuner_samples: Vec<f32>,
    /// Where the tuner's needle has eased to, in cents.
    needle: f32,
    metronome_on: bool,
    /// The metronome's current beat, counted from 1.
    beat: u32,
    taps: Vec<Instant>,
    pitch: Option<Pitch>,
    overlay: Overlay,
    explorer: Explorer,
    fretboard: Fretboard,
    tone3000: Tone3000,
    update: Update,
    assistant: Assistant,
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
        // A library on a drive that's gone shouldn't stop the app opening.
        let library = Library::open(settings.library_root()).unwrap_or_else(|open_error| {
            error = Some(format!("Using a temporary library: {open_error}").into());
            settings.library = None;
            Library::open(data_dir().join("library"))
                .or_else(|_| Library::open(std::env::temp_dir().join("amp-gpui-library")))
                .expect("the temporary folder should be writable")
        });
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

        let mut subscriptions = vec![
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
        let (fretboard, fretboard_subscriptions) =
            Fretboard::new(settings.key, &settings.scale, window, cx);
        subscriptions.extend(fretboard_subscriptions);
        let (assistant, assistant_subscription) = Assistant::new(window, cx);
        subscriptions.push(assistant_subscription);
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
            starting: true,
            history: Vec::new(),
            future: Vec::new(),
            last_param: None,
            focus: focus.clone(),
            settings_save_at: None,
            tuner_open: false,
            tuner_samples: Vec::new(),
            needle: 0.0,
            metronome_on: false,
            beat: 0,
            taps: Vec::new(),
            pitch: None,
            overlay: Overlay::None,
            explorer,
            fretboard,
            tone3000: Tone3000::new(),
            update: Update::default(),
            assistant,
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
        rig.check_for_update(cx);
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
        self.pump_assistant(cx);
        self.sync_tuner();
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
            self.beat = meters.beat();
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

    fn apply_levels(&mut self) {
        let tuner_open = self.tuner_open;
        let input = db_to_gain(self.settings.input_db);
        let muted = tuner_open && self.settings.tuner_mute;
        let output = if muted {
            0.0
        } else {
            db_to_gain(self.settings.output_db)
        };
        let click = self.click();
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetInputGain(input));
            engine.send(Command::SetOutputGain(output));
            engine.send(Command::SetClick(click));
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
}

impl Render for Rig {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .key_context("Rig")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
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
            .on_action(cx.listener(|this, _: &ToggleTools, window, cx| {
                this.toggle_tools(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMetronome, _, cx| {
                this.toggle_metronome(cx);
            }))
            .on_action(cx.listener(|this, _: &TonePrevious, window, cx| {
                this.step_explorer(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToneNext, window, cx| {
                this.step_explorer(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTones, window, cx| {
                this.toggle_tones(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleGear, _, cx| this.toggle_gear(cx)))
            .on_action(cx.listener(|this, _: &ToggleAssistant, window, cx| {
                this.toggle_assistant(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CompareTrial, window, cx| {
                this.compare_trial(window, cx);
            }))
            .bg(style::window())
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
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_start()
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(self.render_board_view(window, cx))
                                    .when(self.overlay == Overlay::Tones, |main| {
                                        main.child(self.render_explorer(cx))
                                    })
                                    .when(self.overlay == Overlay::Audio, |main| {
                                        main.child(self.render_audio_panel(cx))
                                    })
                                    .when(self.overlay == Overlay::Tools, |main| {
                                        main.child(self.render_tools(cx))
                                    })
                                    .when(self.overlay == Overlay::Tone3000, |main| {
                                        main.child(self.render_tone3000(cx))
                                    }),
                            )
                            .when(self.assistant.open, |row| {
                                row.child(self.render_assistant(cx))
                            }),
                    )
                    .child(self.render_footer(cx)),
            )
    }
}
