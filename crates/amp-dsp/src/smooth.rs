/// One-pole smoothing for gain changes, so moving a slider doesn't click.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Smoothed {
    current: f32,
    target: f32,
    coefficient: f32,
}

impl Smoothed {
    pub(crate) fn new(value: f32, sample_rate: u32) -> Self {
        const TIME_SECONDS: f32 = 0.01;
        Self {
            current: value,
            target: value,
            coefficient: (-1.0 / (TIME_SECONDS * sample_rate as f32)).exp(),
        }
    }

    pub(crate) fn set(&mut self, value: f32) {
        self.target = value;
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        // Near the target the step rounds to nothing and the value stalls short
        // of it, so finish the last 0.01% in one go.
        const SNAP: f32 = 1e-4;
        self.current = self.target + (self.current - self.target) * self.coefficient;
        if (self.current - self.target).abs() <= self.target.abs() * SNAP {
            self.current = self.target;
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaches_the_target_exactly() {
        let mut smoothed = Smoothed::new(18_240.0, 48_000);
        smoothed.set(4_800.0);
        for _ in 0..48_000 {
            smoothed.next();
        }
        assert_eq!(smoothed.next(), 4_800.0);
    }
}
