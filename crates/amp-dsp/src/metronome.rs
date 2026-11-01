use std::f32::consts::TAU;

const ACCENT_HZ: f32 = 1_760.0;
const BEAT_HZ: f32 = 1_175.0;
/// A beat sits a little under the accent, so the one stands out.
const BEAT_LEVEL: f32 = 0.7;
/// How long a click takes to fall by about 60 dB, in seconds.
const RING: f32 = 0.035;

/// What the metronome should be doing, sent whole from the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub on: bool,
    pub bpm: f32,
    pub beats: u32,
    /// Linear gain of the click, independent of the Output level.
    pub level: f32,
}

/// A click mixed in after the amp. It counts samples on the audio thread, so it
/// holds time however busy the UI is.
pub struct Metronome {
    rate: f32,
    click: Click,
    /// Samples until the next click. Fractional, so tempos that don't divide
    /// evenly into samples don't drift.
    until_click: f64,
    beat: u32,
    phase: f32,
    step: f32,
    ring: f32,
    fall: f32,
}

impl Metronome {
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            click: Click {
                on: false,
                bpm: 120.0,
                beats: 4,
                level: 0.0,
            },
            until_click: 0.0,
            beat: 0,
            phase: 0.0,
            step: 0.0,
            ring: 0.0,
            fall: (-6.9 / (RING * rate)).exp(),
        }
    }

    pub fn set(&mut self, click: Click) {
        let starting = click.on && !self.click.on;
        self.click = click;
        if starting {
            self.until_click = 0.0;
            self.beat = click.beats.saturating_sub(1);
        }
        // A slower tempo can't strand the next click, and a faster one lands
        // on time rather than after the old, longer wait.
        self.until_click = self.until_click.min(self.period());
    }

    fn period(&self) -> f64 {
        f64::from(self.rate) * 60.0 / f64::from(self.click.bpm.max(1.0))
    }

    /// The beat now sounding, counted from 1, or 0 while stopped.
    pub fn beat(&self) -> u32 {
        if self.click.on { self.beat + 1 } else { 0 }
    }

    pub fn mix(&mut self, buffer: &mut [f32]) {
        if !self.click.on && self.ring < 1e-4 {
            return;
        }
        for sample in buffer.iter_mut() {
            if self.click.on {
                if self.until_click < 1.0 {
                    self.until_click += self.period();
                    self.beat = (self.beat + 1) % self.click.beats.max(1);
                    let accent = self.beat == 0;
                    let hertz = if accent { ACCENT_HZ } else { BEAT_HZ };
                    self.step = TAU * hertz / self.rate;
                    self.phase = 0.0;
                    self.ring = if accent { 1.0 } else { BEAT_LEVEL };
                }
                self.until_click -= 1.0;
            }
            *sample += self.phase.sin() * self.ring * self.click.level;
            self.phase = (self.phase + self.step) % TAU;
            self.ring *= self.fall;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Click, Metronome};

    const RATE: f32 = 48_000.0;

    fn click(bpm: f32, beats: u32) -> Click {
        Click {
            on: true,
            bpm,
            beats,
            level: 1.0,
        }
    }

    /// Where each click starts in `length` samples of output.
    fn onsets(metronome: &mut Metronome, length: usize) -> Vec<usize> {
        let mut buffer = vec![0.0; length];
        metronome.mix(&mut buffer);
        let mut quiet = true;
        let mut found = Vec::new();
        for (index, chunk) in buffer.chunks(64).enumerate() {
            let loud = chunk.iter().any(|sample| sample.abs() > 0.05);
            if loud && quiet {
                found.push(index * 64);
            }
            quiet = !loud;
        }
        found
    }

    #[test]
    fn stays_silent_until_switched_on() {
        let mut metronome = Metronome::new(RATE);
        let mut buffer = [0.25; 256];
        metronome.mix(&mut buffer);
        assert_eq!(buffer, [0.25; 256]);
        assert_eq!(metronome.beat(), 0);
    }

    #[test]
    fn clicks_land_on_the_beat_from_the_first_sample() {
        let mut metronome = Metronome::new(RATE);
        metronome.set(click(120.0, 4));
        assert_eq!(onsets(&mut metronome, 96_000), [0, 24_000, 48_000, 72_000]);
    }

    #[test]
    fn counts_through_the_bar_and_wraps() {
        let mut metronome = Metronome::new(RATE);
        metronome.set(click(120.0, 3));
        let mut buffer = vec![0.0; 24_000];
        let mut beats = Vec::new();
        for _ in 0..4 {
            metronome.mix(&mut buffer);
            beats.push(metronome.beat());
        }
        assert_eq!(beats, [1, 2, 3, 1]);
    }

    #[test]
    fn the_first_beat_is_the_loudest() {
        let mut metronome = Metronome::new(RATE);
        metronome.set(click(120.0, 2));
        let mut buffer = vec![0.0; 48_000];
        metronome.mix(&mut buffer);
        let peak = |part: &[f32]| part.iter().fold(0.0_f32, |peak, sample| peak.max(*sample));
        assert!(peak(&buffer[..24_000]) > peak(&buffer[24_000..]) * 1.2);
    }

    #[test]
    fn a_tempo_that_divides_unevenly_does_not_drift() {
        let mut metronome = Metronome::new(RATE);
        metronome.set(click(7.0, 4));
        let found = onsets(&mut metronome, 2_100_000);
        let expected = 48_000.0 * 60.0 / 7.0 * 5.0;
        assert!((found[5] as f64 - expected).abs() < 64.0);
    }
}
