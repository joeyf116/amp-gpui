use crate::{Block, db_to_gain, smooth::Smoothed};

/// A volume control in dB. Every tone ends in one, so tones and scenes can be
/// set to play at the same loudness without touching how the amp is driven.
pub struct Trim {
    gain: Smoothed,
}

impl Trim {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            gain: Smoothed::new(1.0, sample_rate),
        }
    }
}

impl Block for Trim {
    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.gain.set(db_to_gain(value));
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            *sample *= self.gain.next();
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_on_its_level() {
        let mut trim = Trim::new(48_000);
        trim.set_param(0, -6.0);
        let mut buffer = vec![1.0; 4_800];
        trim.process(&mut buffer);
        assert!((buffer[4_799] - 0.501).abs() < 0.01, "{}", buffer[4_799]);
    }
}
