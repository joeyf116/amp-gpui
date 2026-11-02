use crate::{Block, smooth::Smoothed};

const MAX_SECONDS: f32 = 1.6;

pub struct Delay {
    sample_rate: f32,
    line: Vec<f32>,
    write: usize,
    /// In samples. Smoothed so moving Time glides rather than clicks.
    time: Smoothed,
    feedback: f32,
    mix: f32,
}

impl Delay {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        Self {
            sample_rate: rate,
            line: vec![0.0; (rate * MAX_SECONDS) as usize],
            write: 0,
            time: Smoothed::new(0.38 * rate, sample_rate),
            feedback: 0.3,
            mix: 0.2,
        }
    }

    fn read(&self, delay: f32) -> f32 {
        let length = self.line.len();
        // Split the delay before adding the write position, since an f32 sum
        // that large would round the fraction away.
        let whole = delay as usize;
        let fraction = delay - whole as f32;
        let newer = self.line[(self.write + length - whole) % length];
        let older = self.line[(self.write + 2 * length - whole - 1) % length];
        newer + (older - newer) * fraction
    }
}

impl Block for Delay {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.time.set(value * 0.001 * self.sample_rate),
            1 => self.feedback = value * 0.01,
            2 => self.mix = value * 0.01,
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            let time = self.time.next();
            let delayed = self.read(time);
            self.line[self.write] = *sample + delayed * self.feedback;
            self.write = (self.write + 1) % self.line.len();
            *sample += delayed * self.mix;
        }
    }

    fn has_tail(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        self.line.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_an_impulse_after_the_delay_time() {
        let mut delay = Delay::new(48_000);
        delay.set_param(0, 100.0);
        // Let the smoothed time settle on 4800 samples.
        delay.process(&mut vec![0.0; 48_000]);
        delay.set_param(1, 0.0);
        delay.set_param(2, 100.0);

        let mut buffer = vec![0.0; 6000];
        buffer[0] = 1.0;
        delay.process(&mut buffer);
        assert_eq!(buffer[0], 1.0);
        let (echo_at, echo) = buffer[1..]
            .iter()
            .enumerate()
            .fold((0, 0.0_f32), |best, (i, s)| {
                if s.abs() > best.1 {
                    (i + 1, s.abs())
                } else {
                    best
                }
            });
        assert_eq!(echo_at, 4800);
        assert!(echo > 0.9, "echo was {echo}");
    }
}
