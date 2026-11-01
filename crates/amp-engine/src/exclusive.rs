//! WASAPI exclusive mode, opened directly because `cpal` only does shared mode.
//!
//! Shared mode goes through the Windows mixer, which can't run faster than
//! 10 ms a period. Exclusive mode talks to the device on its own terms, at the
//! cost of silencing everything else on that device while it runs.

use crate::{Meters, pump::Pump};
use rtrb::Producer;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        Media::{
            Audio::*,
            KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE},
            Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
        },
        System::{Com::*, Threading::*},
    },
    core::{GUID, HSTRING, w},
};

const SCRATCH_FRAMES: usize = 8_192;
/// A device that goes this long without waking has stopped.
const WAIT_MS: u32 = 500;
const HUNDRED_NS_PER_SECOND: i64 = 10_000_000;

#[derive(Debug, thiserror::Error)]
pub enum ExclusiveError {
    #[error("Windows audio error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("the device doesn't offer a format exclusive mode can use")]
    NoFormat,
    #[error("the audio thread stopped while starting")]
    ThreadGone,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sample {
    F32,
    I32,
    I24,
    I16,
}

impl Sample {
    fn bytes(self) -> usize {
        match self {
            Self::F32 | Self::I32 => 4,
            Self::I24 => 3,
            Self::I16 => 2,
        }
    }

    fn subformat(self) -> GUID {
        match self {
            Self::F32 => KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
            _ => KSDATAFORMAT_SUBTYPE_PCM,
        }
    }

    fn read(self, bytes: &[u8]) -> f32 {
        match self {
            Self::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            Self::I32 => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32
                    / 2_147_483_648.0
            }
            Self::I24 => {
                i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) as f32 / 2_147_483_648.0
            }
            Self::I16 => i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0,
        }
    }

    fn write(self, value: f32, bytes: &mut [u8]) {
        let value = value.clamp(-1.0, 1.0);
        match self {
            Self::F32 => bytes.copy_from_slice(&value.to_le_bytes()),
            Self::I32 => {
                let sample = (f64::from(value) * 2_147_483_647.0) as i32;
                bytes.copy_from_slice(&sample.to_le_bytes());
            }
            Self::I24 => {
                let sample = (f64::from(value) * 2_147_483_647.0) as i32;
                bytes.copy_from_slice(&sample.to_le_bytes()[1..]);
            }
            Self::I16 => {
                let sample = (value * 32_767.0) as i16;
                bytes.copy_from_slice(&sample.to_le_bytes());
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Format {
    rate: u32,
    channels: u16,
    sample: Sample,
}

impl Format {
    fn frame_bytes(self) -> usize {
        self.sample.bytes() * usize::from(self.channels)
    }

    fn wave(self) -> WAVEFORMATEXTENSIBLE {
        let bits = self.sample.bytes() as u16 * 8;
        let block = self.channels * self.sample.bytes() as u16;
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
                nChannels: self.channels,
                nSamplesPerSec: self.rate,
                nAvgBytesPerSec: self.rate * u32::from(block),
                nBlockAlign: block,
                wBitsPerSample: bits,
                cbSize: 22,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: bits,
            },
            dwChannelMask: match self.channels {
                1 => 0x4,
                2 => 0x3,
                _ => 0,
            },
            SubFormat: self.sample.subformat(),
        }
    }
}

/// What a device agreed to, reported back to the thread that started it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opened {
    pub(crate) rate: u32,
    pub(crate) period_frames: usize,
    pub(crate) latency: Duration,
}

struct Stream {
    client: IAudioClient,
    event: HANDLE,
    format: Format,
    frames: u32,
    latency: Duration,
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

fn find_format(client: &IAudioClient, mix: &WAVEFORMATEX) -> Option<Format> {
    let mix_channels = mix.nChannels;
    for rate in [48_000, 44_100, 96_000, mix.nSamplesPerSec] {
        for channels in [mix_channels, 2, 1] {
            for sample in [Sample::F32, Sample::I32, Sample::I24, Sample::I16] {
                let format = Format {
                    rate,
                    channels,
                    sample,
                };
                let wave = format.wave();
                let supported = unsafe {
                    client.IsFormatSupported(
                        AUDCLNT_SHAREMODE_EXCLUSIVE,
                        (&raw const wave).cast(),
                        None,
                    )
                };
                if supported.is_ok() && supported.0 == 0 {
                    return Some(format);
                }
            }
        }
    }
    None
}

fn device(
    enumerator: &IMMDeviceEnumerator,
    id: Option<&str>,
    flow: EDataFlow,
) -> Result<IMMDevice, ExclusiveError> {
    unsafe {
        Ok(match id {
            // cpal's ids are the Windows endpoint id with a host prefix.
            Some(id) => enumerator.GetDevice(&HSTRING::from(id.trim_start_matches("wasapi:")))?,
            None => enumerator.GetDefaultAudioEndpoint(flow, eConsole)?,
        })
    }
}

fn open(id: Option<&str>, flow: EDataFlow) -> Result<Stream, ExclusiveError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = device(&enumerator, id, flow)?;
        let mut client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        let mix = client.GetMixFormat()?;
        let format = find_format(&client, &*mix);
        CoTaskMemFree(Some(mix.cast()));
        let format = format.ok_or(ExclusiveError::NoFormat)?;
        let wave = format.wave();

        let mut default_period = 0;
        let mut minimum_period = 0;
        client.GetDevicePeriod(Some(&mut default_period), Some(&mut minimum_period))?;

        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        let pointer = (&raw const wave).cast();
        let mut period = minimum_period;
        let result = client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            flags,
            period,
            period,
            pointer,
            None,
        );
        if let Err(error) = result {
            if error.code() != AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
                return Err(error.into());
            }
            // The device told us the nearest size it can do, and a failed
            // client can't be reused, so start over with that size.
            let frames = client.GetBufferSize()?;
            period = (HUNDRED_NS_PER_SECOND * i64::from(frames) + i64::from(format.rate) / 2)
                / i64::from(format.rate);
            client = device.Activate(CLSCTX_ALL, None)?;
            client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                flags,
                period,
                period,
                pointer,
                None,
            )?;
        }

        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event)?;
        let frames = client.GetBufferSize()?;
        let latency = client.GetStreamLatency().unwrap_or(0).max(0) as u64 * 100;
        Ok(Stream {
            client,
            event,
            format,
            frames,
            latency: Duration::from_nanos(latency),
        })
    }
}

impl Stream {
    fn opened(&self) -> Opened {
        Opened {
            rate: self.format.rate,
            period_frames: self.frames as usize,
            latency: Duration::from_secs_f64(f64::from(self.frames) / f64::from(self.format.rate))
                + self.latency,
        }
    }

    fn wait(&self) -> bool {
        unsafe { WaitForSingleObject(self.event, WAIT_MS) == WAIT_OBJECT_0 }
    }
}

/// Asks Windows to schedule this thread the way pro audio apps do. Without it
/// a short period glitches whenever anything else on the machine gets busy.
fn raise_priority() {
    unsafe {
        let mut task = 0;
        if let Ok(handle) = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) {
            let _ = AvSetMmThreadPriority(handle, AVRT_PRIORITY_CRITICAL);
        }
    }
}

fn capture_loop(
    stream: &Stream,
    channel: usize,
    queue: &mut Producer<f32>,
    meters: &Meters,
    stop: &AtomicBool,
) -> Result<(), ExclusiveError> {
    let capture: IAudioCaptureClient = unsafe { stream.client.GetService()? };
    let format = stream.format;
    let bytes = format.sample.bytes();
    let frame_bytes = format.frame_bytes();
    let offset = channel.min(usize::from(format.channels) - 1) * bytes;
    unsafe { stream.client.Start()? };
    while !stop.load(Ordering::Relaxed) {
        if !stream.wait() {
            meters.add_xrun();
            continue;
        }
        unsafe {
            while capture.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                let packet = std::slice::from_raw_parts(data, frames as usize * frame_bytes);
                for frame in packet.chunks_exact(frame_bytes) {
                    let sample = if silent {
                        0.0
                    } else {
                        format.sample.read(&frame[offset..offset + bytes])
                    };
                    if queue.push(sample).is_err() {
                        meters.add_xrun();
                        break;
                    }
                }
                capture.ReleaseBuffer(frames)?;
            }
        }
    }
    Ok(())
}

fn render_loop(
    stream: &Stream,
    pump: &mut Pump,
    input_rate: f32,
    meters: &Meters,
    stop: &AtomicBool,
) -> Result<(), ExclusiveError> {
    let render: IAudioRenderClient = unsafe { stream.client.GetService()? };
    let format = stream.format;
    let bytes = format.sample.bytes();
    let frame_bytes = format.frame_bytes();
    let frames = stream.frames as usize;
    let mut mono = vec![0.0_f32; frames.min(SCRATCH_FRAMES)];

    unsafe {
        // The buffer must hold valid audio before the stream starts.
        render.GetBuffer(stream.frames)?;
        render.ReleaseBuffer(stream.frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        stream.client.Start()?;
    }
    while !stop.load(Ordering::Relaxed) {
        if !stream.wait() {
            meters.add_xrun();
            continue;
        }
        pump.render(&mut mono);
        unsafe {
            let data = render.GetBuffer(stream.frames)?;
            let output = std::slice::from_raw_parts_mut(data, frames * frame_bytes);
            for (frame, sample) in output.chunks_exact_mut(frame_bytes).zip(mono.iter()) {
                for slot in frame.chunks_exact_mut(bytes) {
                    format.sample.write(*sample, slot);
                }
            }
            render.ReleaseBuffer(stream.frames, 0)?;
        }
        meters.set_queue_latency(Duration::from_secs_f32(pump.queued_frames() / input_rate));
    }
    Ok(())
}

/// The running capture and render threads. Dropping this stops both.
pub(crate) struct ExclusiveStreams {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for ExclusiveStreams {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn report_failure(error: ExclusiveError) {
    eprintln!("exclusive audio stopped: {error}");
}

pub(crate) fn spawn_input(
    id: Option<String>,
    channel: usize,
    mut queue: Producer<f32>,
    meters: Arc<Meters>,
    stop: Arc<AtomicBool>,
    opened: Sender<Result<Opened, ExclusiveError>>,
    go: Receiver<()>,
) -> JoinHandle<()> {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        raise_priority();
        let stream = match open(id.as_deref(), eCapture) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = opened.send(Err(error));
                return;
            }
        };
        let _ = opened.send(Ok(stream.opened()));
        if go.recv().is_ok()
            && let Err(error) = capture_loop(&stream, channel, &mut queue, &meters, &stop)
        {
            meters.add_xrun();
            report_failure(error);
        }
    })
}

pub(crate) fn spawn_output(
    id: Option<String>,
    meters: Arc<Meters>,
    stop: Arc<AtomicBool>,
    opened: Sender<Result<Opened, ExclusiveError>>,
    pump: Receiver<(Pump, f32)>,
) -> JoinHandle<()> {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        raise_priority();
        let stream = match open(id.as_deref(), eRender) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = opened.send(Err(error));
                return;
            }
        };
        let _ = opened.send(Ok(stream.opened()));
        if let Ok((mut pump, input_rate)) = pump.recv()
            && let Err(error) = render_loop(&stream, &mut pump, input_rate, &meters, &stop)
        {
            meters.add_xrun();
            report_failure(error);
        }
    })
}

/// Opens both devices, then starts them together.
pub(crate) fn start(
    input: Option<String>,
    output: Option<String>,
    channel: usize,
    queue: Producer<f32>,
    meters: Arc<Meters>,
    build_pump: impl FnOnce(&Opened, &Opened) -> Pump,
) -> Result<(ExclusiveStreams, Opened, Opened), ExclusiveError> {
    let stop = Arc::new(AtomicBool::new(false));
    let (input_ready, input_opened) = mpsc::channel();
    let (output_ready, output_opened) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel();
    let (pump_tx, pump_rx) = mpsc::channel();
    let threads = vec![
        spawn_input(
            input,
            channel,
            queue,
            meters.clone(),
            stop.clone(),
            input_ready,
            go_rx,
        ),
        spawn_output(output, meters, stop.clone(), output_ready, pump_rx),
    ];
    // Dropped on any early return, which stops whichever side did open.
    let streams = ExclusiveStreams { stop, threads };

    let opened_input = input_opened
        .recv()
        .map_err(|_| ExclusiveError::ThreadGone)??;
    let opened_output = output_opened
        .recv()
        .map_err(|_| ExclusiveError::ThreadGone)??;
    let pump = build_pump(&opened_input, &opened_output);
    pump_tx
        .send((pump, opened_input.rate as f32))
        .map_err(|_| ExclusiveError::ThreadGone)?;
    go_tx.send(()).map_err(|_| ExclusiveError::ThreadGone)?;
    Ok((streams, opened_input, opened_output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_round_trip_through_every_format() {
        for sample in [Sample::F32, Sample::I32, Sample::I24, Sample::I16] {
            for value in [0.0, 0.5, -0.5, 0.999] {
                let mut bytes = [0_u8; 4];
                let slot = &mut bytes[..sample.bytes()];
                sample.write(value, slot);
                let read = sample.read(slot);
                assert!((read - value).abs() < 1e-3, "{sample:?}: {value} -> {read}");
            }
        }
    }

    #[test]
    fn writing_clamps_instead_of_wrapping() {
        let mut bytes = [0_u8; 2];
        Sample::I16.write(2.0, &mut bytes);
        assert!(Sample::I16.read(&bytes) > 0.99);
    }
}
