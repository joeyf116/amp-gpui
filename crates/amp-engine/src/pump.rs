use crate::{Meters, Processor, resampler::Resampler};
use rtrb::Consumer;
use std::sync::Arc;

/// The rate the chain always runs at, whatever the devices use. Captures are
/// almost always 48 kHz, and keeping the chain fixed means the two devices can
/// differ without anything downstream noticing.
pub const ENGINE_RATE: u32 = 48_000;

/// Frames the chain processes at a time. This is a latency floor of its own,
/// so it stays small.
const ENGINE_BLOCK: usize = 64;

/// Input frames to keep queued. The two devices wake on separate clocks, so
/// an interface feeding the built-in speakers needs some cushion, and any more
/// than necessary is heard as delay. It starts low and grows only on underrun.
pub(crate) const START_TARGET: usize = 240;
const TARGET_STEP: f32 = 48.0;
/// A burst of underruns comes from one hiccup, so the queue grows at most once
/// in this many seconds. Otherwise a single stall buys a lot of lasting delay.
const TARGET_HOLD_SECONDS: f32 = 0.1;
const MAX_TARGET: f32 = 4_800.0;

/// How hard to lean on the input rate to pull the queue back to target: 0.5%
/// for a full 480-frame error, which is well under anything audible on a
/// guitar and still drains that error in a few seconds.
const CORRECTION_PER_FRAME: f32 = 0.005 / 480.0;
const MAX_CORRECTION: f32 = 0.01;
const AVERAGING: f32 = 0.05;

/// Feeds the output device. Pulls guitar samples from the input queue, runs
/// the chain at [`ENGINE_RATE`], and converts to the output device's rate.
///
/// Two independent devices never run at exactly the same speed. Left alone,
/// the queue between them slowly empties (clicks) or fills (latency that keeps
/// growing), so the input ratio is continually nudged to hold it at target.
pub(crate) struct Pump {
    input: Consumer<f32>,
    processor: Processor,
    meters: Arc<Meters>,
    input_resampler: Resampler,
    output_resampler: Resampler,
    input_ratio: f32,
    output_ratio: f32,
    block: [f32; ENGINE_BLOCK],
    position: usize,
    target: f32,
    average: f32,
    /// False until the first input arrives. The output device usually starts
    /// first, and running dry then says nothing about the queue being too small.
    input_started: bool,
    frames_since_growth: usize,
    hold_frames: usize,
}

impl Pump {
    pub(crate) fn new(
        input: Consumer<f32>,
        processor: Processor,
        meters: Arc<Meters>,
        input_rate: u32,
        output_rate: u32,
        start_target: usize,
    ) -> Self {
        Self {
            input,
            processor,
            meters,
            input_resampler: Resampler::default(),
            output_resampler: Resampler::default(),
            input_ratio: input_rate as f32 / ENGINE_RATE as f32,
            output_ratio: ENGINE_RATE as f32 / output_rate as f32,
            block: [0.0; ENGINE_BLOCK],
            position: ENGINE_BLOCK,
            target: start_target as f32,
            average: start_target as f32,
            input_started: false,
            frames_since_growth: 0,
            hold_frames: (output_rate as f32 * TARGET_HOLD_SECONDS) as usize,
        }
    }

    /// Frames currently held back, for the latency readout.
    pub(crate) fn queued_frames(&self) -> f32 {
        self.average
    }

    pub(crate) fn render(&mut self, output: &mut [f32]) {
        let backlog = self.input.slots() as f32;
        self.input_started |= backlog > 0.0;
        self.frames_since_growth += output.len();
        self.average += (backlog - self.average) * AVERAGING;

        // Too far behind to catch up smoothly, for example after the machine
        // stalled. A glitch now beats a long delay under every note after it.
        if backlog > (self.target * 4.0).max(self.target + 960.0) {
            for _ in 0..(backlog - self.target) as usize {
                let _ = self.input.pop();
            }
            self.average = self.target;
            self.meters.add_xrun();
        }

        let correction = ((self.average - self.target) * CORRECTION_PER_FRAME)
            .clamp(-MAX_CORRECTION, MAX_CORRECTION);
        let input_ratio = self.input_ratio * (1.0 + correction);

        let Self {
            input,
            processor,
            input_resampler,
            output_resampler,
            output_ratio,
            block,
            position,
            ..
        } = self;
        let mut starved = false;
        output_resampler.process(*output_ratio, output, || {
            if *position == ENGINE_BLOCK {
                input_resampler.process(input_ratio, block, || {
                    input.pop().unwrap_or_else(|_| {
                        starved = true;
                        0.0
                    })
                });
                processor.process(block);
                *position = 0;
            }
            let sample = block[*position];
            *position += 1;
            sample
        });

        if starved && self.input_started {
            self.meters.add_xrun();
            if self.frames_since_growth >= self.hold_frames {
                self.target = (self.target + TARGET_STEP).min(MAX_TARGET);
                self.frames_since_growth = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor;
    use rtrb::{Producer, RingBuffer};

    struct Rig {
        pump: Pump,
        input: Producer<f32>,
        meters: Arc<Meters>,
        phase: f64,
    }

    fn rig(input_rate: u32, output_rate: u32) -> Rig {
        let (mut tx, rx) = RingBuffer::new(32_768);
        for _ in 0..START_TARGET {
            tx.push(0.0).unwrap();
        }
        let (processor, controller) = processor(Box::default());
        let meters = controller.meters.clone();
        let pump = Pump::new(
            rx,
            processor,
            meters.clone(),
            input_rate,
            output_rate,
            START_TARGET,
        );
        std::mem::forget(controller);
        Rig {
            pump,
            input: tx,
            meters,
            phase: 0.0,
        }
    }

    impl Rig {
        /// Simulates `seconds` of two devices on 10 ms periods, with the input
        /// really running at `actual_input_rate`, and returns the output.
        fn run(&mut self, seconds: u32, actual_input_rate: f64, output_rate: u32) -> Vec<f32> {
            let mut output = Vec::new();
            let mut owed = 0.0;
            for _ in 0..seconds * 100 {
                owed += actual_input_rate / 100.0;
                for _ in 0..owed as usize {
                    let sample = (self.phase * std::f64::consts::TAU).sin() as f32 * 0.5;
                    self.phase = (self.phase + 440.0 / actual_input_rate).fract();
                    let _ = self.input.push(sample);
                }
                owed = owed.fract();
                let mut period = vec![0.0; output_rate as usize / 100];
                self.pump.render(&mut period);
                output.extend_from_slice(&period);
            }
            output
        }
    }

    fn frequency(samples: &[f32], rate: u32) -> f32 {
        let rising = samples
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        rising as f32 * rate as f32 / samples.len() as f32
    }

    #[test]
    fn keeps_pitch_between_devices_at_different_rates() {
        let mut rig = rig(44_100, 48_000);
        let output = rig.run(10, 44_100.0, 48_000);
        let tail = &output[output.len() / 2..];
        let pitch = frequency(tail, 48_000);
        assert!((pitch - 440.0).abs() < 1.0, "pitch was {pitch}");
    }

    #[test]
    fn works_when_the_output_is_the_slower_device() {
        let mut rig = rig(48_000, 44_100);
        let output = rig.run(10, 48_000.0, 44_100);
        let tail = &output[output.len() / 2..];
        let pitch = frequency(tail, 44_100);
        assert!((pitch - 440.0).abs() < 1.0, "pitch was {pitch}");
    }

    #[test]
    fn holds_latency_steady_when_the_clocks_drift_apart() {
        // 200 ppm is worse than most consumer hardware, in both directions.
        for actual in [48_009.6, 47_990.4] {
            let mut rig = rig(48_000, 48_000);
            rig.run(120, actual, 48_000);
            let queued = rig.pump.queued_frames();
            let settled = rig.meters.xruns();
            assert!(queued < rig.pump.target + 240.0, "queue grew to {queued}");

            rig.run(60, actual, 48_000);
            assert_eq!(rig.meters.xruns(), settled, "still glitching at {actual}");
        }
    }

    #[test]
    fn waiting_for_the_input_to_start_is_not_a_glitch() {
        let mut rig = rig(48_000, 48_000);
        while rig.pump.input.pop().is_ok() {}
        rig.pump.average = 0.0;
        let mut period = vec![0.0; 144];
        for _ in 0..100 {
            rig.pump.render(&mut period);
        }
        assert_eq!(rig.meters.xruns(), 0);
        assert_eq!(rig.pump.target, START_TARGET as f32);
    }

    #[test]
    fn a_burst_of_underruns_grows_the_queue_once() {
        let mut rig = rig(48_000, 48_000);
        rig.input.push(0.0).unwrap();
        let mut period = vec![0.0; 6_000];
        rig.pump.render(&mut period);
        let grown = rig.pump.target;
        assert_eq!(grown, START_TARGET as f32 + TARGET_STEP);

        let mut short = vec![0.0; 144];
        for _ in 0..10 {
            rig.pump.render(&mut short);
        }
        assert_eq!(rig.pump.target, grown);
        assert!(rig.meters.xruns() > 1);
    }

    #[test]
    fn recovers_after_a_stall_without_keeping_the_delay() {
        let mut rig = rig(48_000, 48_000);
        rig.run(2, 48_000.0, 48_000);
        for _ in 0..12_000 {
            rig.input.push(0.0).unwrap();
        }
        rig.run(1, 48_000.0, 48_000);
        assert!(rig.pump.input.slots() < 1_000);
    }
}
