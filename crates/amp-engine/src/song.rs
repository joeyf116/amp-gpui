//! Plays a song along with the rig, at any speed and in any key. Stretching audio
//! is too heavy to risk on the audio thread, where it would take time from the
//! guitar, so a worker thread does it and queues the result. The audio thread only
//! pops finished samples, and each one says where in the song it came from.

use crate::pump::ENGINE_RATE;
use rtrb::{Consumer, Producer, RingBuffer};
use signalsmith_stretch::Stretch;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::Duration,
};

/// Half a second of song waiting to play. A change discards it, so depth costs
/// nothing in response and buys room if the worker is held up.
const QUEUE: usize = 24_000;
const BLOCK: usize = 1_024;
const IDLE: Duration = Duration::from_millis(5);
/// How much of the way the level moves per sample. About 5 ms, to avoid clicks.
const FADE: f32 = 0.004;
/// Quieter than this, a fade has finished.
const FADED: f32 = 1e-3;
/// Marks the end of the song in place of a position.
const END: u32 = u32::MAX;

/// One queued sample, where it came from, and which plan it belongs to.
pub(crate) type SongSample = (f32, u32, u32);

/// How the song should play, sent whole from the UI, from `position` on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongPlan {
    /// In samples from the start of the song.
    pub position: usize,
    /// 1 is the recorded speed.
    pub speed: f32,
    pub semitones: i32,
    /// A stretch to repeat, in samples from the start of the song.
    pub repeat: Option<(usize, usize)>,
}

/// The audio thread's end of a [`SongFeed`].
pub struct SongOutlet {
    samples: Consumer<SongSample>,
    plan: Arc<AtomicU32>,
}

/// The UI's handle on a worker thread. Dropping it ends the worker.
pub struct SongFeed {
    plans: Sender<SongPlan>,
}

impl SongFeed {
    /// Starts a worker over `track`, returning the end to give the audio thread.
    pub fn start(track: Arc<[f32]>, plan: SongPlan) -> (Self, SongOutlet) {
        let (samples, reader) = RingBuffer::new(QUEUE);
        let shared = Arc::new(AtomicU32::new(0));
        let (plans, orders) = mpsc::channel();
        let mut worker = Worker::new(track, samples, shared.clone());
        worker.follow(plan);
        thread::spawn(move || worker.run(&orders));
        let outlet = SongOutlet {
            samples: reader,
            plan: shared,
        };
        (Self { plans }, outlet)
    }

    pub fn set(&self, plan: SongPlan) {
        // A finished worker has nothing left to change.
        let _ = self.plans.send(plan);
    }
}

struct Worker {
    track: Arc<[f32]>,
    samples: Producer<SongSample>,
    /// Which plan is current, shared with the audio thread.
    shared: Arc<AtomicU32>,
    plan_id: u32,
    plan: SongPlan,
    stretch: Stretch,
    /// The next sample of the track to read.
    cursor: usize,
    /// Input samples owed, as a block rarely divides evenly at a slow speed.
    owed: f64,
    input: Vec<f32>,
    output: Vec<f32>,
    ended: bool,
}

impl Worker {
    fn new(track: Arc<[f32]>, samples: Producer<SongSample>, shared: Arc<AtomicU32>) -> Self {
        Self {
            track,
            samples,
            shared,
            plan_id: 0,
            plan: SongPlan {
                position: 0,
                speed: 1.0,
                semitones: 0,
                repeat: None,
            },
            stretch: Stretch::preset_default(1, ENGINE_RATE),
            cursor: 0,
            owed: 0.0,
            input: Vec::new(),
            output: vec![0.0; BLOCK],
            ended: false,
        }
    }

    fn run(mut self, orders: &Receiver<SongPlan>) {
        loop {
            match orders.try_recv() {
                Ok(plan) => self.follow(plan),
                Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {
                    if !self.fill() {
                        thread::sleep(IDLE);
                    }
                }
            }
        }
    }

    /// Whether this plan can skip the stretcher, which colors the sound a little.
    fn untouched(&self) -> bool {
        self.plan.speed == 1.0 && self.plan.semitones == 0 && self.plan.repeat.is_none()
    }

    /// How far behind the read cursor the sound leaving the stretcher is.
    fn lag(&self) -> usize {
        let output = self.stretch.output_latency() as f32 * self.plan.speed;
        self.stretch.input_latency() + output as usize
    }

    fn follow(&mut self, plan: SongPlan) {
        self.plan = plan;
        self.plan_id = self.plan_id.wrapping_add(1);
        self.shared.store(self.plan_id, Ordering::Release);
        self.ended = false;
        self.owed = 0.0;
        self.cursor = plan.position.min(self.track.len());
        if self.untouched() {
            return;
        }
        self.stretch.reset();
        self.stretch
            .set_transpose_factor_semitones(plan.semitones as f32, None);
        // A run-up, so the first thing out of the stretcher is `position`.
        let lag = self.lag();
        let from = self.cursor.saturating_sub(lag);
        self.input.clear();
        self.input.extend_from_slice(&self.track[from..self.cursor]);
        self.gather(lag);
        self.stretch.seek(&self.input, f64::from(plan.speed));
    }

    /// Adds `count` samples of the song to `input`, going around any repeat.
    fn gather(&mut self, count: usize) {
        for _ in 0..count {
            self.input
                .push(self.track.get(self.cursor).copied().unwrap_or(0.0));
            self.cursor += 1;
            if let Some((start, end)) = self.plan.repeat
                && self.cursor >= end
            {
                self.cursor = start;
            }
        }
    }

    /// Where the sound `behind` samples before the read cursor came from.
    fn origin(&self, behind: usize) -> usize {
        match self.plan.repeat {
            Some((start, end)) if self.cursor < start + behind => {
                end.saturating_sub(start + behind - self.cursor)
            }
            _ => self.cursor.saturating_sub(behind),
        }
    }

    /// Queues one block, if there's room and song left to play.
    fn fill(&mut self) -> bool {
        if self.ended || self.samples.slots() < BLOCK + 1 {
            return false;
        }
        let lag = if self.untouched() { 0 } else { self.lag() };
        let from = self.origin(lag);
        if from >= self.track.len() {
            self.ended = true;
            let _ = self.samples.push((0.0, END, self.plan_id));
            return true;
        }
        let speed = f64::from(self.plan.speed);
        self.owed += BLOCK as f64 * speed;
        let count = self.owed as usize;
        self.owed -= count as f64;
        self.input.clear();
        self.gather(count);
        if self.untouched() {
            self.output.clear();
            self.output.extend_from_slice(&self.input);
        } else {
            self.output.resize(BLOCK, 0.0);
            self.stretch.process(&self.input, &mut self.output);
        }
        let step = count as f32 / self.output.len() as f32;
        for (index, sample) in self.output.iter().enumerate() {
            let mut at = from + (index as f32 * step) as usize;
            if let Some((start, end)) = self.plan.repeat
                && at >= end
            {
                at = at - end + start;
            }
            let _ = self.samples.push((*sample, at as u32, self.plan_id));
        }
        true
    }
}

/// The song on the audio thread, mixed in after the amp like the metronome.
#[derive(Default)]
pub(crate) struct SongPlayer {
    outlet: Option<SongOutlet>,
    plan_id: u32,
    /// Fading down to change over to a newer plan.
    changing: bool,
    position: usize,
    playing: bool,
    ended: bool,
    level: f32,
    gain: f32,
}

impl SongPlayer {
    /// Swaps in a new song, handing back the old one. The caller keeps it off
    /// the audio thread, where freeing it isn't allowed.
    pub(crate) fn load(&mut self, outlet: SongOutlet) -> Option<SongOutlet> {
        self.plan_id = outlet.plan.load(Ordering::Acquire);
        self.changing = false;
        self.gain = 0.0;
        self.ended = false;
        self.outlet.replace(outlet)
    }

    pub(crate) fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Sets the song's linear gain, independent of the Output level.
    pub(crate) fn set_level(&mut self, level: f32) {
        self.level = level;
    }

    pub(crate) fn position(&self) -> usize {
        self.position
    }

    pub(crate) fn ended(&self) -> bool {
        self.ended
    }

    pub(crate) fn mix(&mut self, buffer: &mut [f32]) {
        let Some(outlet) = &mut self.outlet else {
            return;
        };
        let newest = outlet.plan.load(Ordering::Acquire);
        self.changing |= newest != self.plan_id;
        for sample in buffer.iter_mut() {
            let wanted = if self.playing && !self.changing {
                self.level
            } else {
                0.0
            };
            self.gain += (wanted - self.gain) * FADE;
            if self.gain < FADED {
                if self.changing {
                    // Everything queued under the old plan goes unheard.
                    self.plan_id = newest;
                    self.changing = false;
                    self.ended = false;
                    while outlet.samples.peek().is_ok_and(|queued| queued.2 != newest) {
                        let _ = outlet.samples.pop();
                    }
                }
                if !self.playing {
                    continue;
                }
            }
            let Ok((value, at, _)) = outlet.samples.pop() else {
                continue;
            };
            if at == END {
                self.ended = true;
                self.playing = false;
                continue;
            }
            self.position = at as usize;
            *sample += value * self.gain;
        }
    }
}

#[cfg(test)]
pub(crate) fn test_outlet() -> (Producer<SongSample>, SongOutlet) {
    let (samples, reader) = RingBuffer::new(QUEUE);
    let outlet = SongOutlet {
        samples: reader,
        plan: Arc::new(AtomicU32::new(0)),
    };
    (samples, outlet)
}

#[cfg(test)]
mod tests {
    use super::{BLOCK, SongOutlet, SongPlan, SongPlayer, Worker};
    use rtrb::RingBuffer;
    use std::sync::{Arc, atomic::AtomicU32};

    const RATE: f32 = 48_000.0;

    fn plan(position: usize, speed: f32) -> SongPlan {
        SongPlan {
            position,
            speed,
            semitones: 0,
            repeat: None,
        }
    }

    fn worker(track: Vec<f32>, depth: usize) -> (Worker, SongOutlet) {
        let (samples, reader) = RingBuffer::new(depth);
        let shared = Arc::new(AtomicU32::new(0));
        let worker = Worker::new(track.into(), samples, shared.clone());
        let outlet = SongOutlet {
            samples: reader,
            plan: shared,
        };
        (worker, outlet)
    }

    /// Everything the worker can queue for one plan.
    fn drain(worker: &mut Worker, outlet: &mut SongOutlet) -> Vec<(f32, u32, u32)> {
        let mut found = Vec::new();
        for _ in 0..1_000 {
            if !worker.fill() {
                break;
            }
            while let Ok(item) = outlet.samples.pop() {
                found.push(item);
            }
        }
        found
    }

    fn tone(hertz: f32, length: usize) -> Vec<f32> {
        (0..length)
            .map(|index| (std::f32::consts::TAU * hertz * index as f32 / RATE).sin() * 0.5)
            .collect()
    }

    /// The pitch of a stretch of sound, by counting upward zero crossings.
    fn hertz(samples: &[f32]) -> f32 {
        let rising = samples.windows(2).filter(|p| p[0] < 0.0 && p[1] >= 0.0);
        rising.count() as f32 * RATE / samples.len() as f32
    }

    #[test]
    fn at_full_speed_the_track_comes_through_untouched() {
        let track: Vec<f32> = (0..4_096).map(|index| index as f32 / 8_192.0).collect();
        let (mut worker, mut outlet) = worker(track.clone(), 8_192);
        worker.follow(plan(100, 1.0));
        let queued = drain(&mut worker, &mut outlet);
        assert_eq!(queued[0].0, track[100]);
        assert_eq!(queued[0].1, 100);
        assert_eq!(queued.last().unwrap().1, super::END);
    }

    #[test]
    fn half_speed_takes_twice_as_long_at_the_same_pitch() {
        let (mut worker, mut outlet) = worker(tone(440.0, 48_000), 8_192);
        worker.follow(plan(0, 0.5));
        let queued = drain(&mut worker, &mut outlet);
        assert!(queued.len().abs_diff(96_000) < 16_000, "{}", queued.len());
        let sound: Vec<f32> = queued[20_000..60_000].iter().map(|item| item.0).collect();
        assert!((hertz(&sound) - 440.0).abs() < 5.0, "{}", hertz(&sound));
    }

    #[test]
    fn a_shift_changes_pitch_and_keeps_time() {
        let (mut worker, mut outlet) = worker(tone(440.0, 48_000), 8_192);
        let mut up = plan(0, 1.0);
        up.semitones = 12;
        worker.follow(up);
        let queued = drain(&mut worker, &mut outlet);
        assert!(queued.len().abs_diff(48_000) < 8_000, "{}", queued.len());
        let sound: Vec<f32> = queued[10_000..30_000].iter().map(|item| item.0).collect();
        assert!((hertz(&sound) - 880.0).abs() < 10.0, "{}", hertz(&sound));
    }

    #[test]
    fn positions_stay_with_the_sound_while_stretched() {
        // A click at one second comes out labeled close to one second.
        let mut track = vec![0.0; 96_000];
        track[48_000..48_040].fill(0.9);
        let (mut worker, mut outlet) = worker(track, 8_192);
        worker.follow(plan(24_000, 0.5));
        let queued = drain(&mut worker, &mut outlet);
        let loudest = queued
            .iter()
            .max_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
            .unwrap();
        assert!(loudest.1.abs_diff(48_000) < 2_400, "{}", loudest.1);
    }

    #[test]
    fn a_repeat_stays_inside_its_stretch() {
        let (mut worker, mut outlet) = worker(tone(220.0, 96_000), 8_192);
        let mut repeat = plan(24_000, 1.0);
        repeat.repeat = Some((24_000, 36_000));
        worker.follow(repeat);
        let mut found = Vec::new();
        for _ in 0..40 {
            assert!(worker.fill());
            while let Ok(item) = outlet.samples.pop() {
                found.push(item.1);
            }
        }
        assert!(found.len() >= 40 * BLOCK);
        assert!(found.iter().all(|at| (20_000..36_000).contains(at)));
        assert!(found.windows(2).any(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn a_new_plan_fades_and_drops_what_was_queued() {
        let (mut worker, outlet) = worker(vec![0.5; 96_000], 16_384);
        let mut player = SongPlayer::default();
        player.load(outlet);
        player.set_level(1.0);
        player.set_playing(true);
        worker.follow(plan(0, 1.0));
        worker.fill();
        worker.fill();
        player.mix(&mut [0.0; 512]);
        worker.follow(plan(48_000, 1.0));
        for _ in 0..8 {
            worker.fill();
        }
        let mut buffer = vec![0.0; 4_096];
        player.mix(&mut buffer);
        let largest_step = buffer
            .windows(2)
            .fold(0.0_f32, |step, pair| step.max((pair[0] - pair[1]).abs()));
        assert!(largest_step < 0.01, "jumped by {largest_step}");
        assert!(player.position() >= 48_000);
    }

    #[test]
    fn a_pause_holds_its_place() {
        let (mut worker, outlet) = worker(vec![0.5; 96_000], 16_384);
        let mut player = SongPlayer::default();
        player.load(outlet);
        player.set_level(1.0);
        player.set_playing(true);
        worker.follow(plan(0, 1.0));
        worker.fill();
        worker.fill();
        player.mix(&mut [0.0; 512]);
        player.set_playing(false);
        for _ in 0..8 {
            worker.fill();
        }
        player.mix(&mut [0.0; 4_096]);
        let held = player.position();
        player.mix(&mut [0.0; 512]);
        assert_eq!(player.position(), held);
    }

    #[test]
    fn the_end_of_the_song_is_reported() {
        let (mut worker, outlet) = worker(vec![0.5; BLOCK], 8_192);
        let mut player = SongPlayer::default();
        player.load(outlet);
        player.set_level(1.0);
        player.set_playing(true);
        worker.follow(plan(0, 1.0));
        worker.fill();
        worker.fill();
        player.mix(&mut vec![0.0; BLOCK + 8]);
        assert!(player.ended());
    }
}
