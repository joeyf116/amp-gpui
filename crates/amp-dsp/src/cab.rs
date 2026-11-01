use crate::{Block, DspError, MAX_BLOCK, db_to_gain, smooth::Smoothed};
use fft_convolver::FFTConvolver;
use std::path::Path;

/// Cabinet IRs hold their useful energy in the first half second. Anything
/// longer is a room, and costs CPU on every block.
const MAX_IR_SECONDS: f32 = 0.5;
const PARTITION: usize = 256;

/// Convolves with a speaker cabinet impulse response. With no IR loaded it
/// passes audio through untouched.
pub struct IrCab {
    convolver: Option<FFTConvolver<f32>>,
    dry: Vec<f32>,
    level: Smoothed,
}

impl IrCab {
    pub fn empty(sample_rate: u32) -> Self {
        Self {
            convolver: None,
            dry: vec![0.0; MAX_BLOCK],
            level: Smoothed::new(1.0, sample_rate),
        }
    }

    /// Loads a WAV impulse response. Must run off the audio thread.
    pub fn load(path: &Path, sample_rate: u32) -> Result<Self, DspError> {
        let (samples, file_rate) = read_mono(path)?;
        Self::from_samples(&samples, file_rate, sample_rate)
    }

    pub fn from_samples(
        samples: &[f32],
        file_rate: u32,
        sample_rate: u32,
    ) -> Result<Self, DspError> {
        if samples.is_empty() {
            return Err(DspError::EmptyImpulse);
        }
        let mut impulse = resample_linear(samples, file_rate, sample_rate);
        impulse.truncate((MAX_IR_SECONDS * sample_rate as f32) as usize);
        normalize_energy(&mut impulse);

        let mut convolver = FFTConvolver::default();
        convolver
            .init(PARTITION, &impulse)
            .map_err(|error| DspError::Convolver(error.to_string()))?;
        let mut cab = Self::empty(sample_rate);
        cab.convolver = Some(convolver);
        Ok(cab)
    }
}

impl Block for IrCab {
    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.level.set(db_to_gain(value));
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        let Some(convolver) = &mut self.convolver else {
            return;
        };
        let dry = &mut self.dry[..buffer.len()];
        dry.copy_from_slice(buffer);
        if convolver.process(dry, buffer).is_err() {
            buffer.fill(0.0);
            return;
        }
        for sample in buffer {
            *sample *= self.level.next();
        }
    }

    fn reset(&mut self) {
        if let Some(convolver) = &mut self.convolver {
            convolver.reset();
        }
    }
}

fn read_mono(path: &Path) -> Result<(Vec<f32>, u32), DspError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|sample| sample as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    // Stereo IRs are usually two mics on one cab, so averaging keeps both.
    let channels = usize::from(spec.channels).max(1);
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

/// Linear interpolation dulls the top octave slightly, which is acceptable for
/// a cab IR and far better than playing it at the wrong pitch.
fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let length = (samples.len() as f64 / ratio).ceil() as usize;
    (0..length)
        .map(|index| {
            let position = index as f64 * ratio;
            let whole = position as usize;
            let fraction = (position - whole as f64) as f32;
            let first = samples[whole.min(samples.len() - 1)];
            let second = samples[(whole + 1).min(samples.len() - 1)];
            first + (second - first) * fraction
        })
        .collect()
}

/// Scales to unit energy so swapping IRs keeps roughly the same volume.
fn normalize_energy(samples: &mut [f32]) {
    let energy: f32 = samples.iter().map(|sample| sample * sample).sum();
    if energy > 0.0 {
        let gain = energy.sqrt().recip();
        samples.iter_mut().for_each(|sample| *sample *= gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convolves_with_the_impulse_response() {
        // An echo at sample 3, half the level of the direct sound.
        let mut cab = IrCab::from_samples(&[1.0, 0.0, 0.0, 0.5], 48_000, 48_000).unwrap();
        let mut buffer = vec![0.0; 512];
        buffer[0] = 1.0;
        cab.process(&mut buffer);
        let ratio = buffer[3] / buffer[0];
        assert!((ratio - 0.5).abs() < 1e-3, "ratio was {ratio}");
    }

    #[test]
    fn resampling_changes_length_by_the_rate_ratio() {
        let resampled = resample_linear(&[0.0; 441], 44_100, 48_000);
        assert_eq!(resampled.len(), 480);
    }

    #[test]
    fn rejects_an_empty_impulse() {
        assert!(matches!(
            IrCab::from_samples(&[], 48_000, 48_000),
            Err(DspError::EmptyImpulse)
        ));
    }

    #[test]
    fn empty_cab_passes_audio_through() {
        let mut cab = IrCab::empty(48_000);
        let mut buffer = [0.25, -0.5];
        cab.process(&mut buffer);
        assert_eq!(buffer, [0.25, -0.5]);
    }
}
