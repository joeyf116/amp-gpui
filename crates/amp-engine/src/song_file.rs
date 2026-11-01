//! Reads a song from disk into mono at the engine's rate. This is slow work for
//! a background thread, never the audio thread.

use crate::pump::ENGINE_RATE;
use std::{fs::File, path::Path};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_NULL, DecoderOptions},
    errors::Error,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

#[derive(Debug, thiserror::Error)]
pub enum SongError {
    #[error("could not open the song: {0}")]
    Open(#[from] std::io::Error),
    #[error("could not read this file as audio: {0}")]
    Decode(#[from] Error),
    #[error("this file has no audio in it")]
    Empty,
}

/// Decodes the audio file at `path`, mixing it to mono.
pub fn read_song(path: &Path) -> Result<Vec<f32>, SongError> {
    let source = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        source,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(SongError::Empty)?;
    let id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut mono: Vec<f32> = Vec::new();
    let mut rate = 0;
    let mut interleaved: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // The format reports the end of a file as a read error.
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // One damaged frame shouldn't lose the whole song.
            Err(Error::DecodeError(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count();
        let buffer =
            interleaved.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        buffer.copy_interleaved_ref(decoded);
        mono.extend(
            buffer
                .samples()
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
    }
    if mono.is_empty() {
        return Err(SongError::Empty);
    }
    Ok(to_engine_rate(&mono, rate))
}

/// Moves `samples` to the engine's rate by straight interpolation. It's a backing
/// track for practice, and its top octave barely matters beside the keeping of time.
fn to_engine_rate(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate == ENGINE_RATE {
        return samples.to_vec();
    }
    let step = f64::from(rate) / f64::from(ENGINE_RATE);
    let length = (samples.len() as f64 / step) as usize;
    (0..length)
        .map(|index| {
            let at = index as f64 * step;
            let (from, share) = (at as usize, at.fract() as f32);
            let first = samples[from];
            let second = samples.get(from + 1).copied().unwrap_or(first);
            first + (second - first) * share
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{SongError, read_song, to_engine_rate};

    fn write_wav(path: &std::path::Path, rate: u32, channels: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for frame in 0..rate / 10 {
            for _ in 0..channels {
                writer.write_sample((frame % 100) as i16 * 100).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("amp-song-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_stereo_file_comes_back_mono_at_the_engine_rate() {
        let path = scratch("stereo.wav");
        write_wav(&path, 44_100, 2);
        let song = read_song(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(song.len().abs_diff(4_800) <= 1, "{}", song.len());
        assert!(song.iter().any(|sample| *sample > 0.1));
    }

    #[test]
    fn an_mp3_decodes() {
        // Silent frames of 128 kbps stereo at 44.1 kHz: a header and no audio data.
        let mut frame = vec![0_u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        let path = scratch("silent.mp3");
        std::fs::write(&path, frame.repeat(40)).unwrap();
        let song = read_song(&path);
        std::fs::remove_file(&path).unwrap();
        let song = song.unwrap();
        // 40 frames of 1,152 samples, give or take the decoder's own delay.
        let expected = 40.0 * 1_152.0 * 48_000.0 / 44_100.0;
        assert!(
            (song.len() as f32 / expected - 1.0).abs() < 0.1,
            "{}",
            song.len()
        );
        assert!(song.iter().all(|sample| sample.abs() < 1e-3));
    }

    #[test]
    fn something_that_isnt_audio_says_so() {
        let path = scratch("notes.txt");
        std::fs::write(&path, "not a song at all, sorry").unwrap();
        let result = read_song(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(result, Err(SongError::Decode(_))));
    }

    #[test]
    fn a_missing_file_says_so() {
        assert!(matches!(
            read_song(&scratch("gone.mp3")),
            Err(SongError::Open(_))
        ));
    }

    #[test]
    fn rate_changes_keep_the_length_in_time() {
        assert_eq!(to_engine_rate(&[0.5; 24_000], 24_000).len(), 48_000);
        assert_eq!(to_engine_rate(&[0.5; 100], 48_000).len(), 100);
    }
}
