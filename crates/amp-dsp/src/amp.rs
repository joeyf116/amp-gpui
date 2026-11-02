use crate::{Block, DspError, db_to_gain, smooth::Smoothed};
use nam_rs::{Model, NamModel};
use std::path::Path;

/// The level NAM's reference plugin normalizes captures to, in dBFS RMS. Using
/// it means swapping models doesn't change the volume by 20 dB.
const NORMALIZED_LOUDNESS: f32 = -18.0;

/// What a model browser or block card shows about a capture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmpInfo {
    pub name: Option<String>,
    pub modeled_by: Option<String>,
    pub gear: Option<String>,
    pub tone_type: Option<String>,
    /// What was captured, as the file says, such as `amp` or `pedal`.
    pub gear_type: Option<String>,
    /// Whether the capture already has a speaker cabinet in it. A cab block
    /// after one of these would be a second cabinet.
    pub includes_cab: Option<bool>,
    /// The model type as the file names it, such as `WaveNet`.
    pub architecture: String,
    /// How many sizes the capture holds. NAM A2 captures have several, which
    /// the Quality control picks between. Everything else has one.
    pub quality_tiers: usize,
}

/// A Neural Amp Modeler capture with the gain staging `nam-rs` leaves to its
/// host. With no model loaded it passes audio through untouched.
pub struct NamAmp {
    model: Option<Model>,
    input: Smoothed,
    output: Smoothed,
    normalize: f32,
    dc_input: f32,
    dc_output: f32,
    dc_coefficient: f32,
}

impl NamAmp {
    pub fn empty(sample_rate: u32) -> Self {
        Self {
            model: None,
            input: Smoothed::new(1.0, sample_rate),
            output: Smoothed::new(1.0, sample_rate),
            normalize: 1.0,
            dc_input: 0.0,
            dc_output: 0.0,
            // A first-order high-pass around 5 Hz.
            dc_coefficient: 1.0 - std::f32::consts::TAU * 5.0 / sample_rate as f32,
        }
    }

    /// Reads what a `.nam` file says about itself, without preparing it to run.
    pub fn inspect(path: &Path) -> Result<AmpInfo, DspError> {
        let nam = NamModel::from_file(path)?;
        let quality_tiers = Model::from_nam(&nam)?
            .as_slimmable()
            .map_or(1, |slimmable| slimmable.len());
        Ok(capture_info(&nam, quality_tiers))
    }

    /// Loads a `.nam` file. Must run off the audio thread.
    pub fn load(path: &Path, sample_rate: u32) -> Result<(Self, AmpInfo), DspError> {
        Self::from_nam(&Self::read(path)?, sample_rate)
    }

    /// Parses a `.nam` file without preparing it to run. Parsing is most of
    /// the cost of a load, so a parsed file can be kept and built from again.
    pub fn read(path: &Path) -> Result<NamModel, DspError> {
        Ok(NamModel::from_file(path)?)
    }

    pub fn from_nam(nam: &NamModel, sample_rate: u32) -> Result<(Self, AmpInfo), DspError> {
        // nam-rs doesn't resample, and a model fed the wrong rate plays wrong
        // with no error of its own, so refuse here.
        let model_rate = nam.expected_sample_rate().round() as u32;
        if model_rate != sample_rate {
            return Err(DspError::SampleRateMismatch {
                model: model_rate,
                device: sample_rate,
            });
        }
        let metadata = nam.metadata_typed();
        let mut amp = Self::empty(sample_rate);
        if let Some(loudness) = metadata.loudness {
            amp.normalize = db_to_gain(NORMALIZED_LOUDNESS - loudness);
        }
        let mut model = Model::from_nam(nam)?;
        // Run the startup transient off the audio thread, so the first block
        // the listener hears is already settled.
        let mut warmup = vec![0.0; model.receptive_field().max(1)];
        model.process_buffer(&mut warmup);
        let quality_tiers = model.as_slimmable().map_or(1, |slimmable| slimmable.len());
        amp.model = Some(model);
        Ok((amp, capture_info(nam, quality_tiers)))
    }
}

fn capture_info(nam: &NamModel, quality_tiers: usize) -> AmpInfo {
    let metadata = nam.metadata_typed();
    let gear = [
        metadata.gear_make.as_deref(),
        metadata.gear_model.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    AmpInfo {
        name: metadata.name.clone(),
        modeled_by: metadata.modeled_by.clone(),
        gear: (!gear.is_empty()).then_some(gear),
        tone_type: metadata.tone_type.clone(),
        gear_type: metadata.gear_type.clone(),
        includes_cab: nam.includes_cab(),
        architecture: nam.architecture.clone(),
        quality_tiers,
    }
}

impl Block for NamAmp {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.input.set(db_to_gain(value)),
            1 => self.output.set(db_to_gain(value)),
            2 => {
                if let Some(slimmable) = self.model.as_mut().and_then(Model::as_slimmable_mut) {
                    slimmable.set_slim_size(value * 0.01);
                }
            }
            _ => {}
        }
    }

    fn process(&mut self, buffer: &mut [f32]) {
        let Some(model) = &mut self.model else {
            return;
        };
        for sample in buffer.iter_mut() {
            *sample *= self.input.next();
        }
        model.process_buffer(buffer);
        for sample in buffer {
            let input = *sample;
            self.dc_output = input - self.dc_input + self.dc_coefficient * self.dc_output;
            self.dc_input = input;
            *sample = self.dc_output * self.normalize * self.output.next();
        }
    }

    fn reset(&mut self) {
        if let Some(model) = &mut self.model {
            model.reset();
        }
        self.dc_input = 0.0;
        self.dc_output = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest valid model: a single one-cell LSTM layer, so tests run
    /// real inference without shipping a capture.
    fn tiny_lstm_json(sample_rate: u32, loudness: f32, weight: f32) -> String {
        const CELL_AND_INITIAL_STATE: usize = 14;
        const HEAD: usize = 2;
        let mut weights = vec![weight; CELL_AND_INITIAL_STATE + HEAD];
        // The head's bias would only add DC.
        *weights.last_mut().unwrap() = 0.0;
        format!(
            r#"{{
                "version": "0.5.2",
                "architecture": "LSTM",
                "config": {{ "input_size": 1, "hidden_size": 1, "num_layers": 1 }},
                "metadata": {{ "loudness": {loudness}, "name": "Test amp", "gear_type": "amp" }},
                "weights": {weights:?},
                "sample_rate": {sample_rate}
            }}"#
        )
    }

    fn tiny_lstm(sample_rate: u32, loudness: f32) -> NamModel {
        NamModel::from_json_str(&tiny_lstm_json(sample_rate, loudness, 0.1))
            .expect("the test model parses")
    }

    /// A NAM A2 style container with a small and a full model that sound
    /// different, so a test can hear which one is active.
    fn tiny_a2() -> NamModel {
        let small = tiny_lstm_json(48_000, -18.0, 0.1);
        let full = tiny_lstm_json(48_000, -18.0, 0.4);
        let json = format!(
            r#"{{
                "version": "0.7.0",
                "architecture": "SlimmableContainer",
                "config": {{ "submodels": [
                    {{ "max_value": 0.5, "model": {small} }},
                    {{ "max_value": 1.0, "model": {full} }}
                ] }},
                "metadata": {{ "name": "Test A2" }},
                "weights": [],
                "sample_rate": 48000
            }}"#
        );
        NamModel::from_json_str(&json).expect("the test container parses")
    }

    fn sine() -> Vec<f32> {
        (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect()
    }

    #[test]
    fn reports_the_quality_tiers_of_an_a2_capture() {
        let (_, a2) = NamAmp::from_nam(&tiny_a2(), 48_000).unwrap();
        assert_eq!(a2.quality_tiers, 2);
        assert_eq!(a2.architecture, "SlimmableContainer");

        let (_, single) = NamAmp::from_nam(&tiny_lstm(48_000, -18.0), 48_000).unwrap();
        assert_eq!(single.quality_tiers, 1);
    }

    #[test]
    fn quality_switches_between_the_sizes_of_an_a2_capture() {
        let (mut full, _) = NamAmp::from_nam(&tiny_a2(), 48_000).unwrap();
        let (mut small, _) = NamAmp::from_nam(&tiny_a2(), 48_000).unwrap();
        small.set_param(2, 25.0);

        let mut full_output = sine();
        let mut small_output = sine();
        full.process(&mut full_output);
        small.process(&mut small_output);
        assert_ne!(full_output, small_output);

        let (mut restored, _) = NamAmp::from_nam(&tiny_a2(), 48_000).unwrap();
        restored.set_param(2, 25.0);
        restored.set_param(2, 100.0);
        let mut restored_output = sine();
        restored.process(&mut restored_output);
        assert_eq!(full_output, restored_output);
    }

    #[test]
    fn quality_does_nothing_on_a_single_size_capture() {
        let (mut plain, _) = NamAmp::from_nam(&tiny_lstm(48_000, -18.0), 48_000).unwrap();
        let (mut dialed, _) = NamAmp::from_nam(&tiny_lstm(48_000, -18.0), 48_000).unwrap();
        dialed.set_param(2, 10.0);
        let mut plain_output = sine();
        let mut dialed_output = sine();
        plain.process(&mut plain_output);
        dialed.process(&mut dialed_output);
        assert_eq!(plain_output, dialed_output);
    }

    #[test]
    fn refuses_a_model_at_another_sample_rate() {
        let error = NamAmp::from_nam(&tiny_lstm(44_100, -18.0), 48_000).err();
        assert!(matches!(
            error,
            Some(DspError::SampleRateMismatch {
                model: 44_100,
                device: 48_000
            })
        ));
    }

    #[test]
    fn runs_a_model_and_reports_its_metadata() {
        let (mut amp, info) = NamAmp::from_nam(&tiny_lstm(48_000, -18.0), 48_000).unwrap();
        assert_eq!(info.name.as_deref(), Some("Test amp"));
        let mut buffer: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let dry = buffer.clone();
        amp.process(&mut buffer);
        assert!(buffer.iter().all(|sample| sample.is_finite()));
        assert_ne!(buffer, dry);
    }

    #[test]
    fn normalizes_a_loud_model_down() {
        let run = |loudness| {
            let (mut amp, _) = NamAmp::from_nam(&tiny_lstm(48_000, loudness), 48_000).unwrap();
            let mut buffer: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            amp.process(&mut buffer);
            buffer.iter().fold(0.0, |peak: f32, s| peak.max(s.abs()))
        };
        let quiet = run(-18.0);
        let loud = run(-6.0);
        assert!((quiet / loud - 4.0).abs() < 0.1, "{quiet} / {loud}");
    }

    #[test]
    fn empty_amp_passes_audio_through() {
        let mut amp = NamAmp::empty(48_000);
        let mut buffer = [0.25, -0.5];
        amp.process(&mut buffer);
        assert_eq!(buffer, [0.25, -0.5]);
    }
}
