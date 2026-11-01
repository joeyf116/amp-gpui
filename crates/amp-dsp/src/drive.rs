use crate::{Block, db_to_gain, smooth::Smoothed};

/// A simple asymmetric soft clipper for pushing the front of an amp model.
pub struct Drive {
    drive: Smoothed,
    level: Smoothed,
}

impl Drive {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            drive: Smoothed::new(db_to_gain(12.0), sample_rate),
            level: Smoothed::new(db_to_gain(-6.0), sample_rate),
        }
    }
}

impl Block for Drive {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.drive.set(db_to_gain(value)),
            1 => self.level.set(db_to_gain(value)),
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        // The bias adds the even harmonics of a real gain stage. Subtracting its
        // own clipped value keeps silence at zero.
        const BIAS: f32 = 0.15;
        let rest = BIAS.tanh();
        for sample in buffer {
            let driven = *sample * self.drive.next();
            *sample = ((driven + BIAS).tanh() - rest) * self.level.next();
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_bounded_and_keeps_silence_silent() {
        let mut drive = Drive::new(48_000);
        drive.set_param(0, 36.0);
        drive.set_param(1, 0.0);
        let mut loud = vec![1.0; 4800];
        drive.process(&mut loud);
        assert!(loud.iter().all(|sample| sample.abs() <= 2.0));

        let mut silence = vec![0.0; 16];
        drive.process(&mut silence);
        assert!(silence.iter().all(|sample| *sample == 0.0));
    }
}
