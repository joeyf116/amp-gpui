use crate::{Block, db_to_gain};

/// Hold the gate open this far below the threshold once it has opened, so a
/// note decaying through the threshold doesn't chatter.
const HYSTERESIS: f32 = 0.5;

pub struct Gate {
    sample_rate: f32,
    threshold: f32,
    envelope: f32,
    envelope_release: f32,
    gain: f32,
    gain_attack: f32,
    gain_release: f32,
    open: bool,
}

impl Gate {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate as f32;
        let mut gate = Self {
            sample_rate,
            threshold: db_to_gain(-65.0),
            envelope: 0.0,
            envelope_release: (-1.0 / (0.02 * sample_rate)).exp(),
            gain: 0.0,
            gain_attack: (-1.0 / (0.001 * sample_rate)).exp(),
            gain_release: 0.0,
            open: false,
        };
        gate.set_param(1, 120.0);
        gate
    }
}

impl Block for Gate {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.threshold = db_to_gain(value),
            1 => self.gain_release = (-1.0 / (value * 0.001 * self.sample_rate)).exp(),
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            let level = sample.abs();
            self.envelope = level.max(self.envelope * self.envelope_release);
            if self.envelope > self.threshold {
                self.open = true;
            } else if self.envelope < self.threshold * HYSTERESIS {
                self.open = false;
            }
            let (target, coefficient) = if self.open {
                (1.0, self.gain_attack)
            } else {
                (0.0, self.gain_release)
            };
            self.gain = target + (self.gain - target) * coefficient;
            *sample *= self.gain;
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain = 0.0;
        self.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tail_peak(gate: &mut Gate, amplitude: f32) -> f32 {
        let mut buffer = vec![amplitude; RATE as usize / 2];
        for sample in buffer.iter_mut().skip(1).step_by(2) {
            *sample = -*sample;
        }
        for chunk in buffer.chunks_mut(512) {
            gate.process(chunk);
        }
        let tail = &buffer[buffer.len() - 512..];
        tail.iter().fold(0.0, |peak: f32, s| peak.max(s.abs()))
    }

    #[test]
    fn passes_a_note_and_mutes_hiss() {
        let mut gate = Gate::new(RATE);
        assert!(tail_peak(&mut gate, 0.1) > 0.099);
        // Half a second in, the 120 ms release has taken 20 dB off the hiss.
        assert!(tail_peak(&mut gate, 0.00001) < 0.000001);
    }
}
