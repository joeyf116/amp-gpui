use crate::pump::ENGINE_RATE;
use crate::song::{SongOutlet, SongPlayer};
use amp_dsp::{Chain, Click, Metronome};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

const COMMAND_CAPACITY: usize = 256;
const RETIRED_CAPACITY: usize = 16;
/// How long a chain swap crossfades, in samples. Long enough that editing the
/// chain mid-note doesn't click, short enough to feel instant.
const FADE_FRAMES: usize = 240;
/// The longest block that gets a crossfade. The engine sends far less.
const FADE_SCRATCH: usize = 4_096;
const TUNER_CAPACITY: usize = 16_384;
/// How quickly level changes land, per sample. About 10 ms, to avoid zipper noise.
const GAIN_SMOOTHING: f32 = 0.002;
/// Where the safety limiter starts to lean on the signal.
const LIMIT_KNEE: f32 = 0.9;

/// A change from the UI, applied between audio blocks.
pub enum Command {
    SetParam {
        slot: usize,
        param: usize,
        value: f32,
    },
    SetBypassed {
        slot: usize,
        bypassed: bool,
    },
    /// The level going into the chain, as a linear gain.
    SetInputGain(f32),
    /// The level leaving the chain, as a linear gain.
    SetOutputGain(f32),
    /// Replaces the whole chain. It is boxed because building one allocates
    /// and loads files, and all of that has to stay off the audio thread.
    SetChain(Box<Chain>),
    /// Starts, stops, or changes the metronome.
    SetClick(Click),
    /// Loads a song to play along with, from its start.
    LoadSong(SongOutlet),
    SetSongPlaying(bool),
    /// The song's level, as a linear gain.
    SetSongLevel(f32),
}

/// Levels and health the audio thread publishes for the UI to poll.
#[derive(Default)]
pub struct Meters {
    input_peak: AtomicU32,
    output_peak: AtomicU32,
    xruns: AtomicU64,
    input_device_us: AtomicU32,
    queue_us: AtomicU32,
    output_device_us: AtomicU32,
    tuner_on: AtomicBool,
    beat: AtomicU32,
    song_position: AtomicUsize,
    song_ended: AtomicBool,
}

/// Where the delay between string and speaker goes. Only what the drivers
/// report is counted, so the true total can be higher, especially on Bluetooth.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Latency {
    pub input_device_ms: f32,
    pub queue_ms: f32,
    pub output_device_ms: f32,
}

impl Latency {
    pub fn total_ms(&self) -> f32 {
        self.input_device_ms + self.queue_ms + self.output_device_ms
    }
}

impl Meters {
    /// Tells the audio thread whether to copy the guitar to the tuner.
    pub fn set_tuner(&self, on: bool) {
        self.tuner_on.store(on, Ordering::Relaxed);
    }

    /// The metronome's current beat, counted from 1, or 0 while it's stopped.
    pub fn beat(&self) -> u32 {
        self.beat.load(Ordering::Relaxed)
    }

    /// How far the song has played, in samples from its start.
    pub fn song_position(&self) -> usize {
        self.song_position.load(Ordering::Relaxed)
    }

    /// Whether the song has played through to its end.
    pub fn song_ended(&self) -> bool {
        self.song_ended.load(Ordering::Relaxed)
    }

    pub fn latency(&self) -> Latency {
        let read = |value: &AtomicU32| value.load(Ordering::Relaxed) as f32 / 1_000.0;
        Latency {
            input_device_ms: read(&self.input_device_us),
            queue_ms: read(&self.queue_us),
            output_device_ms: read(&self.output_device_us),
        }
    }

    pub(crate) fn set_input_device_latency(&self, duration: Duration) {
        self.input_device_us
            .store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    pub(crate) fn set_queue_latency(&self, duration: Duration) {
        self.queue_us
            .store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    pub(crate) fn set_output_device_latency(&self, duration: Duration) {
        self.output_device_us
            .store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    /// Reads and clears the peak since the last call, so a short transient
    /// still shows up on a meter drawn 20 times a second.
    pub fn take_input_peak(&self) -> f32 {
        f32::from_bits(self.input_peak.swap(0, Ordering::Relaxed))
    }

    pub fn take_output_peak(&self) -> f32 {
        f32::from_bits(self.output_peak.swap(0, Ordering::Relaxed))
    }

    pub fn xruns(&self) -> u64 {
        self.xruns.load(Ordering::Relaxed)
    }

    pub(crate) fn add_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }

    fn raise(peak: &AtomicU32, value: f32) {
        if value > f32::from_bits(peak.load(Ordering::Relaxed)) {
            peak.store(value.to_bits(), Ordering::Relaxed);
        }
    }
}

/// The UI thread's handle on a [`Processor`].
pub struct Controller {
    commands: Producer<Command>,
    retired: Consumer<Box<Chain>>,
    retired_songs: Consumer<SongOutlet>,
    tuner: Consumer<f32>,
    pub meters: Arc<Meters>,
}

impl Controller {
    /// Queues a command, handing it back if the audio side has stopped
    /// draining the queue.
    pub fn send(&mut self, command: Command) -> Result<(), Command> {
        self.commands.push(command).map_err(|error| match error {
            rtrb::PushError::Full(command) => command,
        })
    }

    /// Moves the guitar samples the audio thread has copied so far into into.
    pub fn read_tuner(&mut self, into: &mut Vec<f32>) {
        while let Ok(sample) = self.tuner.pop() {
            into.push(sample);
        }
    }

    /// Frees chains and songs the audio thread has finished with. Call this
    /// regularly from the UI thread.
    pub fn drop_retired(&mut self) {
        while self.retired.pop().is_ok() {}
        while self.retired_songs.pop().is_ok() {}
    }
}

/// Runs the chain. Everything in here happens on the audio thread.
pub struct Processor {
    chain: Box<Chain>,
    commands: Consumer<Command>,
    retired: Producer<Box<Chain>>,
    /// A chain waiting for room in `retired`. Dropping it here would free
    /// memory on the audio thread, so it waits until the queue has space.
    pending_retire: Option<Box<Chain>>,
    retired_songs: Producer<SongOutlet>,
    /// A song waiting for room, for the same reason as `pending_retire`.
    pending_song: Option<SongOutlet>,
    fade: Option<Fade>,
    fade_scratch: Vec<f32>,
    tuner: Producer<f32>,
    input_gain: Ramp,
    output_gain: Ramp,
    metronome: Metronome,
    song: SongPlayer,
    meters: Arc<Meters>,
}

/// A gain that glides to its target.
struct Ramp {
    current: f32,
    target: f32,
}

impl Ramp {
    fn new() -> Self {
        Self {
            current: 1.0,
            target: 1.0,
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        self.current += (self.target - self.current) * GAIN_SMOOTHING;
        self.current
    }
}

/// Leaves normal levels alone, and bends peaks into full scale rather than
/// flattening them, so a stray transient can't hard-clip at the speakers.
#[inline]
fn limit(sample: f32) -> f32 {
    let level = sample.abs();
    if level <= LIMIT_KNEE {
        return sample;
    }
    let room = 1.0 - LIMIT_KNEE;
    (LIMIT_KNEE + room * ((level - LIMIT_KNEE) / room).tanh()).copysign(sample)
}

/// The outgoing chain, still running while the new one fades in.
struct Fade {
    chain: Box<Chain>,
    position: usize,
}

pub fn processor(chain: Box<Chain>) -> (Processor, Controller) {
    let (command_tx, command_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (retired_tx, retired_rx) = RingBuffer::new(RETIRED_CAPACITY);
    let (songs_tx, songs_rx) = RingBuffer::new(RETIRED_CAPACITY);
    let (tuner_tx, tuner_rx) = RingBuffer::new(TUNER_CAPACITY);
    let meters = Arc::new(Meters::default());
    (
        Processor {
            chain,
            commands: command_rx,
            retired: retired_tx,
            pending_retire: None,
            retired_songs: songs_tx,
            pending_song: None,
            fade: None,
            fade_scratch: vec![0.0; FADE_SCRATCH],
            tuner: tuner_tx,
            input_gain: Ramp::new(),
            output_gain: Ramp::new(),
            metronome: Metronome::new(ENGINE_RATE as f32),
            song: SongPlayer::default(),
            meters: meters.clone(),
        },
        Controller {
            commands: command_tx,
            retired: retired_rx,
            retired_songs: songs_rx,
            tuner: tuner_rx,
            meters,
        },
    )
}

impl Processor {
    pub fn process(&mut self, buffer: &mut [f32]) {
        self.apply_commands();
        // Tapped before the trim, so the tuner works with the input turned down.
        if self.meters.tuner_on.load(Ordering::Relaxed) {
            for sample in buffer.iter() {
                let _ = self.tuner.push(*sample);
            }
        }
        for sample in buffer.iter_mut() {
            *sample *= self.input_gain.next();
        }
        Meters::raise(&self.meters.input_peak, peak(buffer));
        let faded = buffer.len().min(FADE_SCRATCH);
        if self.fade.is_some() {
            self.fade_scratch[..faded].copy_from_slice(&buffer[..faded]);
        }
        self.chain.process(buffer);
        self.mix_in_fade(&mut buffer[..faded]);
        for sample in buffer.iter_mut() {
            *sample *= self.output_gain.next();
        }
        // After the Output level, so turning the rig down leaves these alone.
        self.metronome.mix(buffer);
        self.song.mix(buffer);
        self.meters
            .beat
            .store(self.metronome.beat(), Ordering::Relaxed);
        self.meters
            .song_position
            .store(self.song.position(), Ordering::Relaxed);
        self.meters
            .song_ended
            .store(self.song.ended(), Ordering::Relaxed);
        for sample in buffer.iter_mut() {
            *sample = limit(*sample);
        }
        Meters::raise(&self.meters.output_peak, peak(buffer));
    }

    fn apply_commands(&mut self) {
        if let Some(chain) = self.pending_retire.take() {
            self.retire(chain);
        }
        if let Some(song) = self.pending_song.take() {
            self.retire_song(song);
        }
        // Only take a new chain or song once the old one has somewhere to go.
        while self.pending_retire.is_none() && self.pending_song.is_none() {
            let Ok(command) = self.commands.pop() else {
                break;
            };
            match command {
                Command::SetParam { slot, param, value } => {
                    self.chain.set_param(slot, param, value);
                }
                Command::SetBypassed { slot, bypassed } => {
                    self.chain.set_bypassed(slot, bypassed);
                }
                Command::SetInputGain(gain) => self.input_gain.target = gain,
                Command::SetOutputGain(gain) => self.output_gain.target = gain,
                Command::SetClick(click) => self.metronome.set(click),
                Command::LoadSong(track) => {
                    if let Some(old) = self.song.load(track) {
                        self.retire_song(old);
                    }
                }
                Command::SetSongPlaying(playing) => self.song.set_playing(playing),
                Command::SetSongLevel(level) => self.song.set_level(level),
                Command::SetChain(chain) => {
                    let old = std::mem::replace(&mut self.chain, chain);
                    if let Some(fade) = self.fade.take() {
                        self.retire(fade.chain);
                    }
                    self.fade = Some(Fade {
                        chain: old,
                        position: 0,
                    });
                }
            }
        }
    }

    fn mix_in_fade(&mut self, buffer: &mut [f32]) {
        let Some(fade) = &mut self.fade else {
            return;
        };
        let old = &mut self.fade_scratch[..buffer.len()];
        fade.chain.process(old);
        for (sample, old) in buffer.iter_mut().zip(old.iter()) {
            let new_gain = (fade.position as f32 / FADE_FRAMES as f32).min(1.0);
            *sample = *sample * new_gain + *old * (1.0 - new_gain);
            fade.position += 1;
        }
        if fade.position >= FADE_FRAMES
            && let Some(fade) = self.fade.take()
        {
            self.retire(fade.chain);
        }
    }

    fn retire(&mut self, chain: Box<Chain>) {
        if let Err(rtrb::PushError::Full(chain)) = self.retired.push(chain) {
            self.pending_retire = Some(chain);
        }
    }

    fn retire_song(&mut self, song: SongOutlet) {
        if let Err(rtrb::PushError::Full(song)) = self.retired_songs.push(song) {
            self.pending_song = Some(song);
        }
    }
}

fn peak(buffer: &[f32]) -> f32 {
    buffer
        .iter()
        .fold(0.0, |peak, sample| peak.max(sample.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use amp_dsp::{Block, Chain};

    struct Scale(f32);

    impl Block for Scale {
        fn set_param(&mut self, _: usize, value: f32) {
            self.0 = value;
        }
        fn process(&mut self, buffer: &mut [f32]) {
            buffer.iter_mut().for_each(|sample| *sample *= self.0);
        }
        fn reset(&mut self) {}
    }

    fn scale_chain(gain: f32) -> Box<Chain> {
        let mut chain = Chain::default();
        chain.push(Box::new(Scale(gain)), false);
        Box::new(chain)
    }

    #[test]
    fn applies_params_between_blocks() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(
            controller
                .send(Command::SetParam {
                    slot: 0,
                    param: 0,
                    value: 0.5
                })
                .is_ok()
        );
        let mut buffer = [0.5];
        processor.process(&mut buffer);
        assert_eq!(buffer, [0.25]);
    }

    #[test]
    fn hands_the_old_chain_back_to_the_ui_thread() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(controller.send(Command::SetChain(scale_chain(0.5))).is_ok());
        processor.process(&mut [0.0; FADE_FRAMES]);
        assert_eq!(controller.retired.slots(), 1);
        controller.drop_retired();
        assert_eq!(controller.retired.slots(), 0);
    }

    #[test]
    fn holds_a_chain_swap_until_retired_space_frees_up() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        for _ in 0..RETIRED_CAPACITY + 3 {
            assert!(controller.send(Command::SetChain(scale_chain(0.5))).is_ok());
        }
        processor.process(&mut [0.0]);
        assert!(processor.pending_retire.is_some());
        for _ in 0..4 {
            controller.drop_retired();
            processor.process(&mut [0.0; FADE_FRAMES]);
        }
        assert!(processor.pending_retire.is_none());
        assert!(processor.fade.is_none());
        assert_eq!(processor.commands.slots(), 0);
    }

    #[test]
    fn normal_levels_pass_the_limiter_untouched() {
        assert_eq!(limit(0.5), 0.5);
        assert_eq!(limit(-0.9), -0.9);
        assert!(limit(50.0) <= 1.0);
        assert!(limit(1.2) > limit(1.0));
    }

    #[test]
    fn level_changes_glide_to_their_target() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(controller.send(Command::SetOutputGain(0.25)).is_ok());
        let mut buffer = [0.5; 8_192];
        processor.process(&mut buffer);
        assert!(buffer[0] > 0.49);
        assert!((buffer[8_191] - 0.125).abs() < 0.001);
    }

    #[test]
    fn the_tuner_hears_the_guitar_only_while_it_is_open() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        let mut heard = Vec::new();
        processor.process(&mut [0.25; 16]);
        controller.read_tuner(&mut heard);
        assert!(heard.is_empty());

        controller.meters.set_tuner(true);
        processor.process(&mut [0.25; 16]);
        controller.read_tuner(&mut heard);
        assert_eq!(heard, [0.25; 16]);
    }

    #[test]
    fn a_chain_swap_fades_instead_of_jumping() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(controller.send(Command::SetChain(scale_chain(0.0))).is_ok());
        let mut buffer = [0.5; FADE_FRAMES + 8];
        processor.process(&mut buffer);
        assert_eq!(buffer[0], 0.5);
        let largest_step = buffer
            .windows(2)
            .fold(0.0_f32, |step, pair| step.max((pair[0] - pair[1]).abs()));
        assert!(largest_step < 0.01, "jumped by {largest_step}");
        assert_eq!(buffer[FADE_FRAMES + 7], 0.0);
    }

    #[test]
    fn the_click_plays_over_a_silenced_rig_and_reports_its_beat() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        assert!(controller.send(Command::SetOutputGain(0.0)).is_ok());
        assert!(
            controller
                .send(Command::SetClick(Click {
                    on: true,
                    bpm: 120.0,
                    beats: 4,
                    level: 0.5,
                }))
                .is_ok()
        );
        let mut buffer = [0.0; 1_024];
        processor.process(&mut buffer);
        assert!(buffer.iter().any(|sample| sample.abs() > 0.1));
        assert_eq!(controller.meters.beat(), 1);
    }

    #[test]
    fn a_song_plays_over_a_silenced_rig_and_reports_where_it_is() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        let (mut samples, outlet) = crate::song::test_outlet();
        for frame in 0..4_096 {
            samples.push((0.5, frame, 0)).unwrap();
        }
        for command in [
            Command::SetOutputGain(0.0),
            Command::LoadSong(outlet),
            Command::SetSongLevel(1.0),
            Command::SetSongPlaying(true),
        ] {
            assert!(controller.send(command).is_ok());
        }
        let mut buffer = [0.0; 4_000];
        processor.process(&mut buffer);
        assert!(buffer[3_999] > 0.4);
        assert_eq!(controller.meters.song_position(), 3_999);
    }

    #[test]
    fn an_old_song_goes_back_to_the_ui_thread() {
        let (mut processor, mut controller) = processor(scale_chain(1.0));
        let (first, outlet) = crate::song::test_outlet();
        assert!(controller.send(Command::LoadSong(outlet)).is_ok());
        let (_second, outlet) = crate::song::test_outlet();
        assert!(controller.send(Command::LoadSong(outlet)).is_ok());
        processor.process(&mut [0.0]);
        assert!(!first.is_abandoned());
        controller.drop_retired();
        assert!(first.is_abandoned());
    }

    #[test]
    fn limits_output_and_reports_peaks() {
        let (mut processor, controller) = processor(scale_chain(4.0));
        let mut buffer = [0.5, -0.25];
        processor.process(&mut buffer);
        assert!(buffer[0] > 0.9 && buffer[0] <= 1.0);
        assert!(buffer[1] < -0.9 && buffer[1] >= -1.0);
        assert_eq!(controller.meters.take_input_peak(), 0.5);
        assert!(controller.meters.take_output_peak() <= 1.0);
        assert_eq!(controller.meters.take_output_peak(), 0.0);
    }
}
