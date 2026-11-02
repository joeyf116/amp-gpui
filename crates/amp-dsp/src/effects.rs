use crate::{Block, db_to_gain, filter::Biquad, smooth::Smoothed};
use std::f32::consts::TAU;

/// A feed-forward compressor for evening out picking dynamics.
pub struct Compressor {
    rate: f32,
    threshold_db: f32,
    ratio: f32,
    envelope: f32,
    attack: f32,
    release: f32,
    level: Smoothed,
}

impl Compressor {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        Self {
            rate,
            threshold_db: -24.0,
            ratio: 3.0,
            envelope: 0.0,
            attack: (-1.0 / (0.012 * rate)).exp(),
            release: (-1.0 / (0.16 * rate)).exp(),
            level: Smoothed::new(1.0, sample_rate),
        }
    }
}

impl Block for Compressor {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.threshold_db = value,
            1 => self.ratio = value,
            2 => self.release = (-1.0 / (value * 0.001 * self.rate)).exp(),
            3 => self.level.set(db_to_gain(value)),
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            let level = sample.abs();
            let coefficient = if level > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = level + (self.envelope - level) * coefficient;
            let over = 20.0 * self.envelope.max(1e-6).log10() - self.threshold_db;
            let reduction = over.max(0.0) * (1.0 / self.ratio - 1.0);
            *sample *= db_to_gain(reduction) * self.level.next();
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }
}

/// One modulated delay line, mixed with the dry signal.
pub struct Chorus {
    rate: f32,
    line: Vec<f32>,
    write: usize,
    phase: f32,
    step: f32,
    depth: f32,
    mix: f32,
}

const CHORUS_CENTER_SECONDS: f32 = 0.014;
const CHORUS_MAX_DEPTH_SECONDS: f32 = 0.006;

impl Chorus {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        let seconds = CHORUS_CENTER_SECONDS + CHORUS_MAX_DEPTH_SECONDS * 2.0;
        Self {
            rate,
            line: vec![0.0; (rate * seconds) as usize],
            write: 0,
            phase: 0.0,
            step: 0.8 / rate,
            depth: 0.4,
            mix: 0.4,
        }
    }
}

impl Block for Chorus {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.step = value / self.rate,
            1 => self.depth = value * 0.01,
            2 => self.mix = value * 0.01,
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        let length = self.line.len();
        let center = CHORUS_CENTER_SECONDS * self.rate;
        let swing = CHORUS_MAX_DEPTH_SECONDS * self.rate * self.depth;
        for sample in buffer {
            self.line[self.write] = *sample;
            let delay = center + swing * (self.phase * TAU).sin();
            let whole = delay as usize;
            let fraction = delay - whole as f32;
            let newer = self.line[(self.write + length - whole) % length];
            let older = self.line[(self.write + 2 * length - whole - 1) % length];
            let wet = newer + (older - newer) * fraction;
            *sample = *sample * (1.0 - self.mix * 0.5) + wet * self.mix;
            self.write = (self.write + 1) % length;
            self.phase = (self.phase + self.step).fract();
        }
    }

    fn reset(&mut self) {
        self.line.fill(0.0);
    }
}

struct Comb {
    line: Vec<f32>,
    index: usize,
    damped: f32,
}

struct AllPass {
    line: Vec<f32>,
    index: usize,
}

/// Comb lengths at 48 kHz. Their prime spacing keeps echoes from lining up.
const COMBS: [usize; 6] = [1_687, 1_777, 1_949, 2_053, 2_213, 2_341];
const ALL_PASSES: [usize; 3] = [241, 601, 809];

/// A plate-like reverb: parallel damped combs into a series of all-passes.
pub struct Reverb {
    combs: Vec<Comb>,
    all_passes: Vec<AllPass>,
    feedback: f32,
    damping: f32,
    mix: f32,
    tone: Biquad,
}

impl Reverb {
    pub fn new(sample_rate: u32) -> Self {
        let scale = sample_rate as f32 / 48_000.0;
        let length = |samples: usize| ((samples as f32 * scale) as usize).max(1);
        Self {
            combs: COMBS
                .iter()
                .map(|samples| Comb {
                    line: vec![0.0; length(*samples)],
                    index: 0,
                    damped: 0.0,
                })
                .collect(),
            all_passes: ALL_PASSES
                .iter()
                .map(|samples| AllPass {
                    line: vec![0.0; length(*samples)],
                    index: 0,
                })
                .collect(),
            feedback: 0.82,
            damping: 0.3,
            mix: 0.2,
            tone: Biquad::high_pass(sample_rate as f32, 180.0, 0.7),
        }
    }
}

impl Block for Reverb {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.feedback = 0.7 + value * 0.0028,
            1 => self.damping = value * 0.008,
            2 => self.mix = value * 0.01,
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            // Keeping lows out of the tail stops it turning to mud.
            let input = self.tone.process(*sample) * 0.12;
            let mut wet = 0.0;
            for comb in &mut self.combs {
                let delayed = comb.line[comb.index];
                comb.damped = delayed + (comb.damped - delayed) * self.damping;
                comb.line[comb.index] = input + comb.damped * self.feedback;
                comb.index = (comb.index + 1) % comb.line.len();
                wet += delayed;
            }
            for all_pass in &mut self.all_passes {
                let delayed = all_pass.line[all_pass.index];
                all_pass.line[all_pass.index] = wet + delayed * 0.5;
                all_pass.index = (all_pass.index + 1) % all_pass.line.len();
                wet = delayed - wet * 0.5;
            }
            *sample = *sample * (1.0 - self.mix * 0.4) + wet * self.mix;
        }
    }

    fn has_tail(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        for comb in &mut self.combs {
            comb.line.fill(0.0);
            comb.damped = 0.0;
        }
        for all_pass in &mut self.all_passes {
            all_pass.line.fill(0.0);
        }
        self.tone.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
    }

    #[test]
    fn compressor_turns_loud_passages_down_more_than_quiet_ones() {
        let mut compressor = Compressor::new(48_000);
        compressor.set_param(0, -30.0);
        compressor.set_param(1, 8.0);
        let mut quiet = vec![0.01; 24_000];
        let mut loud = vec![0.5; 24_000];
        compressor.process(&mut quiet);
        compressor.reset();
        compressor.process(&mut loud);
        let quiet_gain = quiet[23_999] / 0.01;
        let loud_gain = loud[23_999] / 0.5;
        assert!(loud_gain < quiet_gain * 0.25, "{quiet_gain} {loud_gain}");
    }

    #[test]
    fn chorus_stays_near_unity() {
        let mut chorus = Chorus::new(48_000);
        let mut buffer: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.03).sin() * 0.5).collect();
        chorus.process(&mut buffer);
        assert!(peak(&buffer) < 0.9);
        assert!(peak(&buffer[24_000..]) > 0.3);
    }

    #[test]
    fn reverb_rings_after_the_note_and_then_dies() {
        let mut reverb = Reverb::new(48_000);
        reverb.set_param(0, 50.0);
        reverb.set_param(2, 100.0);
        let mut buffer = vec![0.0; 240_000];
        for sample in &mut buffer[..480] {
            *sample = 0.5;
        }
        for chunk in buffer.chunks_mut(64) {
            reverb.process(chunk);
        }
        assert!(peak(&buffer[24_000..48_000]) > 0.002);
        assert!(peak(&buffer[216_000..]) < peak(&buffer[24_000..48_000]) * 0.2);
        assert!(buffer.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn reverb_stays_stable_at_its_longest_setting() {
        let mut reverb = Reverb::new(48_000);
        reverb.set_param(0, 100.0);
        reverb.set_param(1, 0.0);
        reverb.set_param(2, 100.0);
        let mut buffer = vec![0.3; 480_000];
        reverb.process(&mut buffer);
        assert!(peak(&buffer) < 12.0, "peak was {}", peak(&buffer));
    }
}
