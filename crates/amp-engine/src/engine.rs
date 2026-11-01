use crate::{
    Command, Controller, Meters, processor,
    pump::{Pump, START_TARGET},
};
use amp_dsp::Chain;
use cpal::{
    BufferSize, Device, DeviceId, Host, SampleFormat, SampleRate, Stream, StreamConfig,
    SupportedBufferSize, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::RingBuffer;
use std::{sync::Arc, time::Duration};

/// The device buffer to ask for. Drivers clamp this to what they can do, and
/// on Windows shared mode they mostly impose their own period regardless.
const PREFERRED_BUFFER_FRAMES: u32 = 128;
/// ASIO drivers really do honor small buffers, unlike Windows shared mode.
#[cfg(feature = "asio")]
const ASIO_BUFFER_FRAMES: u32 = 64;
const QUEUE_FRAMES: usize = 16_384;
const SCRATCH_FRAMES: usize = 2_048;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no audio input device was found")]
    NoInputDevice,
    #[error("no audio output device was found")]
    NoOutputDevice,
    #[error("the audio device \"{0}\" is no longer available")]
    DeviceGone(String),
    #[error("audio device error: {0}")]
    Device(#[from] cpal::Error),
    #[error("the {0} device uses {1} samples, and only 32-bit float is supported so far")]
    UnsupportedFormat(&'static str, SampleFormat),
    #[cfg(windows)]
    #[error("exclusive mode failed: {0}")]
    Exclusive(#[from] crate::exclusive::ExclusiveError),
}

/// How the devices are opened.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LatencyMode {
    /// Shares the devices with other apps through the Windows mixer.
    #[default]
    Shared,
    /// Takes the devices over, which cuts latency and silences other apps on
    /// them while monitoring runs.
    #[cfg(windows)]
    Exclusive,
    /// Runs through the interface's own driver, the fastest route there is.
    /// Input and output have to be on that one interface.
    #[cfg(feature = "asio")]
    Asio,
}

/// An audio device someone can pick.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceOption {
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub sample_rate: SampleRate,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceList {
    pub inputs: Vec<DeviceOption>,
    pub outputs: Vec<DeviceOption>,
    pub default_input: Option<String>,
    pub default_output: Option<String>,
}

impl DeviceList {
    /// The names of the devices config points at, for showing to the user.
    #[cfg(windows)]
    fn names(config: &EngineConfig) -> (String, String) {
        let devices = Self::scan().unwrap_or_default();
        let name = |options: &[DeviceOption], id: &Option<String>, default: &Option<String>| {
            let id = id.as_ref().or(default.as_ref());
            options
                .iter()
                .find(|device| Some(&device.id) == id)
                .map_or_else(|| "Unknown device".to_owned(), |device| device.name.clone())
        };
        (
            name(&devices.inputs, &config.input, &devices.default_input),
            name(&devices.outputs, &config.output, &devices.default_output),
        )
    }

    pub fn scan() -> Result<Self, EngineError> {
        let host = cpal::default_host();
        let option = |device: Device, config: Result<SupportedStreamConfig, cpal::Error>| {
            let config = config.ok()?;
            Some(DeviceOption {
                id: device.id().ok()?.to_string(),
                name: device.description().ok()?.name().to_owned(),
                channels: config.channels(),
                sample_rate: config.sample_rate(),
            })
        };
        let id = |device: Option<Device>| device?.id().ok().map(|id| id.to_string());
        Ok(Self {
            inputs: host
                .input_devices()?
                .filter_map(|device| {
                    let config = device.default_input_config();
                    option(device, config)
                })
                .collect(),
            outputs: host
                .output_devices()?
                .filter_map(|device| {
                    let config = device.default_output_config();
                    option(device, config)
                })
                .collect(),
            default_input: id(host.default_input_device()),
            default_output: id(host.default_output_device()),
        })
    }
}

/// Which devices to use. `None` means the system default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineConfig {
    pub input: Option<String>,
    pub output: Option<String>,
    /// The zero-based input channel the guitar is plugged into.
    pub input_channel: usize,
    pub mode: LatencyMode,
    /// The ASIO driver to use in `LatencyMode::Asio`, by name.
    pub asio_driver: Option<String>,
}

/// What the running streams settled on.
#[derive(Clone, Debug)]
pub struct EngineInfo {
    pub input_name: String,
    pub output_name: String,
    pub input_rate: SampleRate,
    pub output_rate: SampleRate,
    pub mode: LatencyMode,
}

/// Live monitoring. Dropping this stops the streams.
pub struct Engine {
    controller: Controller,
    info: EngineInfo,
    _streams: Streams,
}

// Only one exists per engine, so the size difference costs nothing.
#[allow(clippy::large_enum_variant)]
enum Streams {
    Shared {
        _input: Stream,
        _output: Stream,
    },
    #[cfg(windows)]
    Exclusive {
        _streams: crate::exclusive::ExclusiveStreams,
    },
}

fn find_device(host: &Host, id: Option<&str>, input: bool) -> Result<Device, EngineError> {
    let Some(id) = id else {
        let device = if input {
            host.default_input_device()
                .ok_or(EngineError::NoInputDevice)?
        } else {
            host.default_output_device()
                .ok_or(EngineError::NoOutputDevice)?
        };
        return Ok(device);
    };
    let gone = || EngineError::DeviceGone(id.to_owned());
    let device_id: DeviceId = id.parse().map_err(|_| gone())?;
    host.device_by_id(&device_id).ok_or_else(gone)
}

fn stream_config(config: &SupportedStreamConfig, preferred: u32) -> StreamConfig {
    let buffer_size = match *config.buffer_size() {
        SupportedBufferSize::Range { min, max } => BufferSize::Fixed(preferred.clamp(min, max)),
        SupportedBufferSize::Unknown => BufferSize::Default,
    };
    StreamConfig {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        buffer_size,
    }
}

impl Engine {
    /// Opens the devices and starts processing `chain`, which must have been
    /// built for [`ENGINE_RATE`](crate::ENGINE_RATE).
    pub fn start(chain: Box<Chain>, config: &EngineConfig) -> Result<Self, EngineError> {
        match config.mode {
            LatencyMode::Shared => Self::start_shared(chain, config),
            #[cfg(windows)]
            LatencyMode::Exclusive => Self::start_exclusive(chain, config),
            #[cfg(feature = "asio")]
            LatencyMode::Asio => Self::start_asio(chain, config),
        }
    }

    /// The ASIO interfaces on this machine. Each driver has to be loaded to
    /// list it, so this takes a moment and belongs on one thread.
    #[cfg(feature = "asio")]
    pub fn asio_devices() -> Vec<DeviceOption> {
        let Ok(host) = cpal::host_from_id(cpal::HostId::Asio) else {
            return Vec::new();
        };
        let Ok(devices) = host.output_devices() else {
            return Vec::new();
        };
        devices
            .filter_map(|device| {
                let name = device.description().ok()?.name().to_owned();
                let config = device.default_input_config().ok()?;
                Some(DeviceOption {
                    id: name.clone(),
                    name,
                    channels: config.channels(),
                    sample_rate: config.sample_rate(),
                })
            })
            .collect()
    }

    #[cfg(not(feature = "asio"))]
    pub fn asio_devices() -> Vec<DeviceOption> {
        Vec::new()
    }

    #[cfg(feature = "asio")]
    fn start_asio(chain: Box<Chain>, config: &EngineConfig) -> Result<Self, EngineError> {
        let host = cpal::host_from_id(cpal::HostId::Asio)?;
        let wanted = config.asio_driver.as_deref();
        let device = host
            .output_devices()?
            .find(|device| {
                let name = device.description().ok();
                let name = name.as_ref().map(|description| description.name());
                wanted.is_none_or(|wanted| name == Some(wanted))
            })
            .ok_or(EngineError::NoOutputDevice)?;
        let input_config = device.default_input_config()?;
        let output_config = device.default_output_config()?;
        let format = output_config.sample_format();
        match format {
            SampleFormat::F32 => {
                Self::run_asio::<f32>(chain, config, &device, &input_config, &output_config)
            }
            SampleFormat::I32 => {
                Self::run_asio::<i32>(chain, config, &device, &input_config, &output_config)
            }
            SampleFormat::I24 => {
                Self::run_asio::<cpal::I24>(chain, config, &device, &input_config, &output_config)
            }
            SampleFormat::I16 => {
                Self::run_asio::<i16>(chain, config, &device, &input_config, &output_config)
            }
            other => Err(EngineError::UnsupportedFormat("ASIO", other)),
        }
    }

    #[cfg(feature = "asio")]
    fn run_asio<T>(
        chain: Box<Chain>,
        config: &EngineConfig,
        device: &Device,
        input_config: &SupportedStreamConfig,
        output_config: &SupportedStreamConfig,
    ) -> Result<Self, EngineError>
    where
        T: cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
        f32: cpal::FromSample<T>,
    {
        use cpal::Sample;

        let name = device.description()?.name().to_owned();
        let input_stream_config = stream_config(input_config, ASIO_BUFFER_FRAMES);
        let output_stream_config = stream_config(output_config, ASIO_BUFFER_FRAMES);
        // Both callbacks run in the same driver pass, one buffer apart at most.
        let start_target = match output_stream_config.buffer_size {
            BufferSize::Fixed(frames) => frames as usize,
            BufferSize::Default => START_TARGET,
        };
        let input_channels = usize::from(input_config.channels()).max(1);
        let output_channels = usize::from(output_config.channels()).max(1);
        let input_channel = config.input_channel.min(input_channels - 1);

        let (processor, controller) = processor(chain);
        let meters = controller.meters.clone();
        let (mut queue_tx, queue_rx) = RingBuffer::<f32>::new(QUEUE_FRAMES);
        for _ in 0..start_target {
            let _ = queue_tx.push(0.0);
        }
        let input_rate = input_config.sample_rate();
        let output_rate = output_config.sample_rate();
        let mut pump = Pump::new(
            queue_rx,
            processor,
            meters.clone(),
            input_rate,
            output_rate,
            start_target,
        );

        let input_meters = meters.clone();
        let input = device.build_input_stream(
            input_stream_config,
            move |data: &[T], callback: &cpal::InputCallbackInfo| {
                let timestamp = callback.timestamp();
                input_meters
                    .set_input_device_latency(timestamp.callback.duration_since(timestamp.capture));
                for frame in data.chunks_exact(input_channels) {
                    if queue_tx
                        .push(frame[input_channel].to_sample::<f32>())
                        .is_err()
                    {
                        input_meters.add_xrun();
                        return;
                    }
                }
            },
            log_stream_error(&meters),
            None,
        )?;

        let output_meters = meters.clone();
        let mut scratch = vec![0.0_f32; SCRATCH_FRAMES];
        let queue_rate = input_rate as f32;
        let output = device.build_output_stream(
            output_stream_config,
            move |data: &mut [T], callback: &cpal::OutputCallbackInfo| {
                for frames in data.chunks_mut(SCRATCH_FRAMES * output_channels) {
                    let mono = &mut scratch[..frames.len() / output_channels];
                    pump.render(mono);
                    for (frame, sample) in frames.chunks_mut(output_channels).zip(mono.iter()) {
                        frame.fill(sample.to_sample::<T>());
                    }
                }
                let timestamp = callback.timestamp();
                output_meters.set_output_device_latency(
                    timestamp.playback.duration_since(timestamp.callback),
                );
                output_meters
                    .set_queue_latency(Duration::from_secs_f32(pump.queued_frames() / queue_rate));
            },
            log_stream_error(&meters),
            None,
        )?;

        input.play()?;
        output.play()?;
        Ok(Self {
            controller,
            info: EngineInfo {
                input_name: name.clone(),
                output_name: name,
                input_rate,
                output_rate,
                mode: LatencyMode::Asio,
            },
            _streams: Streams::Shared {
                _input: input,
                _output: output,
            },
        })
    }

    #[cfg(windows)]
    fn start_exclusive(chain: Box<Chain>, config: &EngineConfig) -> Result<Self, EngineError> {
        let (processor, controller) = processor(chain);
        let meters = controller.meters.clone();
        let (queue_tx, queue_rx) = RingBuffer::<f32>::new(QUEUE_FRAMES);
        let pump_meters = meters.clone();
        let (streams, input, output) = crate::exclusive::start(
            config.input.clone(),
            config.output.clone(),
            config.input_channel,
            queue_tx,
            meters.clone(),
            move |input, output| {
                // One device period is the least that can keep the output fed.
                Pump::new(
                    queue_rx,
                    processor,
                    pump_meters,
                    input.rate,
                    output.rate,
                    input.period_frames,
                )
            },
        )?;
        meters.set_input_device_latency(input.latency);
        meters.set_output_device_latency(output.latency);
        let (input_name, output_name) = DeviceList::names(config);
        Ok(Self {
            controller,
            info: EngineInfo {
                input_name,
                output_name,
                input_rate: input.rate,
                output_rate: output.rate,
                mode: LatencyMode::Exclusive,
            },
            _streams: Streams::Exclusive { _streams: streams },
        })
    }

    fn start_shared(chain: Box<Chain>, config: &EngineConfig) -> Result<Self, EngineError> {
        let host = cpal::default_host();
        let input_device = find_device(&host, config.input.as_deref(), true)?;
        let output_device = find_device(&host, config.output.as_deref(), false)?;
        let input_config = input_device.default_input_config()?;
        let output_config = output_device.default_output_config()?;

        if input_config.sample_format() != SampleFormat::F32 {
            return Err(EngineError::UnsupportedFormat(
                "input",
                input_config.sample_format(),
            ));
        }
        if output_config.sample_format() != SampleFormat::F32 {
            return Err(EngineError::UnsupportedFormat(
                "output",
                output_config.sample_format(),
            ));
        }

        let info = EngineInfo {
            input_name: input_device.description()?.name().to_owned(),
            output_name: output_device.description()?.name().to_owned(),
            input_rate: input_config.sample_rate(),
            output_rate: output_config.sample_rate(),
            mode: LatencyMode::Shared,
        };
        let input_channels = usize::from(input_config.channels()).max(1);
        let output_channels = usize::from(output_config.channels()).max(1);
        let input_channel = config.input_channel.min(input_channels - 1);

        let (processor, controller) = processor(chain);
        let meters = controller.meters.clone();
        let (mut queue_tx, queue_rx) = RingBuffer::<f32>::new(QUEUE_FRAMES);
        for _ in 0..START_TARGET {
            let _ = queue_tx.push(0.0);
        }
        let mut pump = Pump::new(
            queue_rx,
            processor,
            meters.clone(),
            info.input_rate,
            info.output_rate,
            START_TARGET,
        );

        let input_meters = meters.clone();
        let input = input_device.build_input_stream(
            stream_config(&input_config, PREFERRED_BUFFER_FRAMES),
            move |data: &[f32], callback: &cpal::InputCallbackInfo| {
                let timestamp = callback.timestamp();
                input_meters
                    .set_input_device_latency(timestamp.callback.duration_since(timestamp.capture));
                for frame in data.chunks_exact(input_channels) {
                    if queue_tx.push(frame[input_channel]).is_err() {
                        input_meters.add_xrun();
                        return;
                    }
                }
            },
            log_stream_error(&meters),
            None,
        )?;

        let input_rate = info.input_rate as f32;
        let output_meters = meters.clone();
        let mut scratch = vec![0.0_f32; SCRATCH_FRAMES];
        let output = output_device.build_output_stream(
            stream_config(&output_config, PREFERRED_BUFFER_FRAMES),
            move |data: &mut [f32], callback: &cpal::OutputCallbackInfo| {
                for frames in data.chunks_mut(SCRATCH_FRAMES * output_channels) {
                    let mono = &mut scratch[..frames.len() / output_channels];
                    pump.render(mono);
                    for (frame, sample) in frames.chunks_mut(output_channels).zip(mono.iter()) {
                        frame.fill(*sample);
                    }
                }
                let timestamp = callback.timestamp();
                output_meters.set_output_device_latency(
                    timestamp.playback.duration_since(timestamp.callback),
                );
                output_meters
                    .set_queue_latency(Duration::from_secs_f32(pump.queued_frames() / input_rate));
            },
            log_stream_error(&meters),
            None,
        )?;

        input.play()?;
        output.play()?;
        Ok(Self {
            controller,
            info,
            _streams: Streams::Shared {
                _input: input,
                _output: output,
            },
        })
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    pub fn meters(&self) -> &Arc<Meters> {
        &self.controller.meters
    }

    /// Queues a change. A full queue means the audio thread has stalled, so
    /// the change is dropped rather than blocking the UI.
    pub fn send(&mut self, command: Command) {
        let _ = self.controller.send(command);
    }

    /// Collects guitar samples for the tuner, once it's switched on in the meters.
    pub fn read_tuner(&mut self, into: &mut Vec<f32>) {
        self.controller.read_tuner(into);
    }

    /// Frees chains the audio thread has replaced. Call from a UI timer.
    pub fn drop_retired(&mut self) {
        self.controller.drop_retired();
    }
}

fn log_stream_error(meters: &Arc<Meters>) -> impl FnMut(cpal::Error) + Send + 'static {
    let meters = meters.clone();
    move |error| {
        meters.add_xrun();
        eprintln!("audio stream error: {error}");
    }
}
