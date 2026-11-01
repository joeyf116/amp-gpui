use crate::{Block, db_to_gain, filter::Biquad, smooth::Smoothed};

const OVERSAMPLE: f32 = 2.0;
const MAX_GAIN_DB: f32 = 46.0;
/// Output trimmed per step of the gain knob, so raising the gain adds dirt
/// instead of jumping in volume.
const GAIN_TRIM_DB: f32 = 1.2;

/// An original amp model that needs no capture: two clipping gain stages run
/// at double the sample rate to hold down aliasing, then a tone stack and a
/// gentle power stage. It gives presets a sound that ships with the app.
pub struct TubeAmp {
    rate: f32,
    tight: Biquad,
    interpolate: Biquad,
    decimate: Biquad,
    coupling: Biquad,
    bass: Biquad,
    mid: Biquad,
    treble: Biquad,
    presence: Biquad,
    drive: Smoothed,
    master: Smoothed,
    previous: f32,
    bias: f32,
    gain_value: f32,
    master_value: f32,
}

/// Maps a 0 to 10 tone knob to a boost or cut, with 5 as flat.
fn tone_db(value: f32) -> f32 {
    (value - 5.0) * 2.4
}

/// Clips harder on one side, which adds the even harmonics of a real stage.
#[inline]
fn stage(input: f32, bias: f32) -> f32 {
    (input + bias).tanh() - bias.tanh()
}

impl TubeAmp {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        let fast = rate * OVERSAMPLE;
        let mut amp = Self {
            rate,
            tight: Biquad::high_pass(rate, 95.0, 0.7),
            interpolate: Biquad::low_pass(fast, rate * 0.42, 0.7),
            decimate: Biquad::low_pass(fast, rate * 0.42, 0.7),
            coupling: Biquad::high_pass(rate, 30.0, 0.7),
            bass: Biquad::default(),
            mid: Biquad::default(),
            treble: Biquad::default(),
            presence: Biquad::default(),
            drive: Smoothed::new(1.0, sample_rate),
            master: Smoothed::new(1.0, sample_rate),
            previous: 0.0,
            bias: 0.08,
            gain_value: 5.0,
            master_value: 5.0,
        };
        for index in 0..6 {
            amp.set_param(index, 5.0);
        }
        amp
    }
}

impl TubeAmp {
    fn update_master(&mut self) {
        let trim = self.gain_value * GAIN_TRIM_DB;
        self.master
            .set(db_to_gain((self.master_value - 5.0) * 3.0 - trim));
    }
}

impl Block for TubeAmp {
    fn set_param(&mut self, index: usize, value: f32) {
        let rate = self.rate;
        match index {
            0 => {
                self.drive.set(db_to_gain(value * 0.1 * MAX_GAIN_DB));
                // More gain pushes the stage off center, as a hotter tube would.
                self.bias = 0.05 + value * 0.012;
                self.gain_value = value;
                self.update_master();
            }
            1 => self
                .bass
                .retune(Biquad::low_shelf(rate, 140.0, tone_db(value))),
            2 => self
                .mid
                .retune(Biquad::peak(rate, 700.0, 0.7, tone_db(value))),
            3 => self
                .treble
                .retune(Biquad::high_shelf(rate, 2_600.0, tone_db(value))),
            4 => self
                .presence
                .retune(Biquad::high_shelf(rate, 4_800.0, value * 0.9)),
            5 => {
                self.master_value = value;
                self.update_master();
            }
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            let tight = self.tight.process(*sample);
            let drive = self.drive.next();
            let split = drive.sqrt();
            let mut output = 0.0;
            // Two passes per input sample: halfway to this one, then this one.
            for step in [0.5 * (self.previous + tight), tight] {
                let smooth = self.interpolate.process(step);
                let first = stage(smooth * split, self.bias);
                let second = stage(first * split * 0.7, self.bias * 0.5);
                output = self.decimate.process(second);
            }
            self.previous = tight;

            let shaped = self.coupling.process(output);
            let shaped = self.bass.process(shaped);
            let shaped = self.mid.process(shaped);
            let shaped = self.treble.process(shaped);
            let shaped = self.presence.process(shaped);
            // The power stage only rounds off peaks that reach it.
            *sample = (shaped * 0.8).tanh() * self.master.next();
        }
    }

    fn reset(&mut self) {
        for filter in [
            &mut self.tight,
            &mut self.interpolate,
            &mut self.decimate,
            &mut self.coupling,
            &mut self.bass,
            &mut self.mid,
            &mut self.treble,
            &mut self.presence,
        ] {
            filter.reset();
        }
        self.previous = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine_through(amp: &mut TubeAmp) -> Vec<f32> {
        let mut buffer: Vec<f32> = (0..24_000)
            .map(|i| (TAU * 220.0 * i as f32 / 48_000.0).sin() * 0.2)
            .collect();
        for chunk in buffer.chunks_mut(64) {
            amp.process(chunk);
        }
        buffer.split_off(12_000)
    }

    fn crest(samples: &[f32]) -> f32 {
        let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        let power = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
        peak / power.sqrt()
    }

    #[test]
    fn more_gain_squares_the_wave_off() {
        let mut clean = TubeAmp::new(48_000);
        clean.set_param(0, 1.0);
        let mut heavy = TubeAmp::new(48_000);
        heavy.set_param(0, 9.0);
        assert!(crest(&sine_through(&mut heavy)) < crest(&sine_through(&mut clean)) * 0.85);
    }

    #[test]
    fn stays_bounded_and_finite_at_extremes() {
        let mut amp = TubeAmp::new(48_000);
        for index in 0..6 {
            amp.set_param(index, 10.0);
        }
        let output = sine_through(&mut amp);
        assert!(output.iter().all(|s| s.is_finite() && s.abs() < 8.0));
    }

    #[test]
    fn silence_stays_silent() {
        let mut amp = TubeAmp::new(48_000);
        let mut buffer = vec![0.0; 4_800];
        amp.process(&mut buffer);
        assert!(buffer.iter().all(|s| s.abs() < 1e-6));
    }
}
