/// A cubic Hermite interpolator. It holds only four samples, so it adds about
/// two samples of latency where a filter-based resampler would add dozens.
/// That is the right trade here: every sample of delay is felt under the
/// fingers, and a guitar has little energy near the top of the band.
#[derive(Default)]
pub(crate) struct Resampler {
    history: [f32; 4],
    phase: f32,
}

impl Resampler {
    /// Fills `output`, pulling from `next` at `ratio` input samples per output
    /// sample. A ratio of 1 is a clean two-sample delay.
    pub(crate) fn process(
        &mut self,
        ratio: f32,
        output: &mut [f32],
        mut next: impl FnMut() -> f32,
    ) {
        for sample in output {
            while self.phase >= 1.0 {
                self.history.rotate_left(1);
                self.history[3] = next();
                self.phase -= 1.0;
            }
            let [before, from, to, after] = self.history;
            let t = self.phase;
            let a = -0.5 * before + 1.5 * from - 1.5 * to + 0.5 * after;
            let b = before - 2.5 * from + 2.0 * to - 0.5 * after;
            let c = -0.5 * before + 0.5 * to;
            *sample = ((a * t + b) * t + c) * t + from;
            self.phase += ratio;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_ratio_is_a_plain_delay() {
        let mut resampler = Resampler::default();
        let mut source = (1..).map(|i| i as f32);
        let mut output = [0.0; 6];
        // Starting phase is 0, so the first output reads history before any pull.
        resampler.phase = 1.0;
        resampler.process(1.0, &mut output, || source.next().unwrap());
        assert_eq!(output, [0.0, 0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn consumes_input_at_the_ratio() {
        let mut resampler = Resampler::default();
        let mut pulled = 0_i32;
        let mut output = vec![0.0; 48_000];
        resampler.process(44_100.0 / 48_000.0, &mut output, || {
            pulled += 1;
            0.0
        });
        assert!((pulled - 44_100).abs() <= 2, "pulled {pulled}");
    }
}
