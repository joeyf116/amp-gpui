use std::f32::consts::TAU;

/// A second-order filter, using the standard audio EQ cookbook shapes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from_coefficients(b: [f32; 3], a: [f32; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Takes on another shape's response while keeping its running state, so a
    /// knob can move without a click.
    pub(crate) fn retune(&mut self, shape: Self) {
        *self = Self {
            z1: self.z1,
            z2: self.z2,
            ..shape
        };
    }

    pub(crate) fn low_pass(rate: f32, frequency: f32, q: f32) -> Self {
        let (cos, alpha) = terms(rate, frequency, q);
        let b = (1.0 - cos) / 2.0;
        Self::from_coefficients([b, 2.0 * b, b], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    pub(crate) fn high_pass(rate: f32, frequency: f32, q: f32) -> Self {
        let (cos, alpha) = terms(rate, frequency, q);
        let b = (1.0 + cos) / 2.0;
        Self::from_coefficients([b, -2.0 * b, b], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    pub(crate) fn peak(rate: f32, frequency: f32, q: f32, gain_db: f32) -> Self {
        let (cos, alpha) = terms(rate, frequency, q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        Self::from_coefficients(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    pub(crate) fn low_shelf(rate: f32, frequency: f32, gain_db: f32) -> Self {
        let (cos, alpha) = terms(rate, frequency, std::f32::consts::FRAC_1_SQRT_2);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let root = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            [
                a * ((a + 1.0) - (a - 1.0) * cos + root),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - root),
            ],
            [
                (a + 1.0) + (a - 1.0) * cos + root,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - root,
            ],
        )
    }

    pub(crate) fn high_shelf(rate: f32, frequency: f32, gain_db: f32) -> Self {
        let (cos, alpha) = terms(rate, frequency, std::f32::consts::FRAC_1_SQRT_2);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let root = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            [
                a * ((a + 1.0) + (a - 1.0) * cos + root),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - root),
            ],
            [
                (a + 1.0) - (a - 1.0) * cos + root,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - root,
            ],
        )
    }

    #[inline]
    pub(crate) fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }

    pub(crate) fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

fn terms(rate: f32, frequency: f32, q: f32) -> (f32, f32) {
    let omega = TAU * frequency / rate;
    (omega.cos(), omega.sin() / (2.0 * q))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn level_at(filter: &mut Biquad, frequency: f32) -> f32 {
        let sine = |i: usize| (TAU * frequency * i as f32 / RATE).sin();
        for i in 0..24_000 {
            filter.process(sine(i));
        }
        (24_000..30_000)
            .map(|i| filter.process(sine(i)).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn low_pass_keeps_lows_and_cuts_highs() {
        let shape = Biquad::low_pass(RATE, 1_000.0, 0.7);
        assert!(level_at(&mut shape.clone(), 100.0) > 0.95);
        assert!(level_at(&mut shape.clone(), 10_000.0) < 0.05);
    }

    #[test]
    fn high_pass_keeps_highs_and_cuts_lows() {
        let shape = Biquad::high_pass(RATE, 1_000.0, 0.7);
        assert!(level_at(&mut shape.clone(), 10_000.0) > 0.95);
        assert!(level_at(&mut shape.clone(), 50.0) < 0.05);
    }

    #[test]
    fn peak_and_shelves_apply_their_gain() {
        let two = 10.0_f32.powf(6.0 / 20.0);
        let near = |level: f32| (level - two).abs() < 0.08;
        assert!(near(level_at(
            &mut Biquad::peak(RATE, 1_000.0, 1.0, 6.0),
            1_000.0
        )));
        assert!(near(level_at(
            &mut Biquad::low_shelf(RATE, 200.0, 6.0),
            30.0
        )));
        assert!(near(level_at(
            &mut Biquad::high_shelf(RATE, 2_000.0, 6.0),
            15_000.0
        )));
    }
}
