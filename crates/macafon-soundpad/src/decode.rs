//! Offline decoding. Multichannel files use their first two channels; mono is duplicated.
//! Chained containers stop at ResetRequired (the end of the current logical stream).
use crate::{Error, library::MAX_FILE_BYTES};
use macafon_dsp::SAMPLE_RATE_HZ;
use std::{fs::File, path::Path};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::CODEC_TYPE_NULL,
    errors::Error as DecodeError,
    formats::{FormatReader, Track},
    io::MediaSourceStream,
    probe::Hint,
};

pub const MAX_DURATION_S: u32 = 600;
#[derive(Debug)]
pub struct Sound {
    samples: Vec<f32>,
    peak_lin: f32,
    rms_dbfs: f32,
}
impl Sound {
    /// Sanitizes non-finite samples and discards an incomplete final stereo frame.
    pub fn from_interleaved_stereo(mut samples: Vec<f32>) -> Self {
        samples.truncate(samples.len() / 2 * 2);
        let mut peak_lin = 0.0_f32;
        for sample in &mut samples {
            *sample = sanitize(*sample);
            peak_lin = peak_lin.max(sample.abs());
        }
        let mut power = 0.0_f64;
        let mut count = 0;
        for block in samples.chunks(SAMPLE_RATE_HZ as usize * 2 * 400 / 1000) {
            let sum = block.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
            if sum / block.len() as f64 > 1e-6 {
                power += sum;
                count += block.len();
            }
        }
        let rms_dbfs = if count == 0 {
            f32::NEG_INFINITY
        } else {
            (10.0 * (power / count as f64).log10()) as f32
        };
        Self {
            samples,
            peak_lin,
            rms_dbfs,
        }
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn duration_ms(&self) -> u32 {
        ((self.frames() as u64 * 1000 / u64::from(SAMPLE_RATE_HZ)).min(u64::from(u32::MAX))) as u32
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
    pub fn peak_lin(&self) -> f32 {
        self.peak_lin
    }
    pub fn rms_dbfs(&self) -> f32 {
        self.rms_dbfs
    }
    pub fn byte_size(&self) -> usize {
        self.samples.capacity() * size_of::<f32>()
    }
}
pub(crate) fn sanitize(sample: f32) -> f32 {
    if sample.is_finite() && !sample.is_subnormal() {
        sample
    } else {
        0.0
    }
}
fn open(path: &Path) -> Result<Box<dyn FormatReader>, Error> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .format(&hint, stream, &Default::default(), &Default::default())
        .map(|p| p.format)
        .map_err(decode_error)
}
fn decode_error(error: DecodeError) -> Error {
    Error::Decode(error.to_string())
}
fn audio_track(format: &dyn FormatReader) -> Result<&Track, Error> {
    format
        .default_track()
        .or_else(|| {
            format
                .tracks()
                .iter()
                .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        })
        .ok_or(Error::UnsupportedFormat)
}

pub fn probe_duration_ms(path: &Path) -> Result<Option<u32>, Error> {
    let format = open(path)?;
    let track = audio_track(format.as_ref())?;
    Ok(track
        .codec_params
        .n_frames
        .zip(track.codec_params.sample_rate)
        .and_then(|(n, rate)| {
            if rate == 0 {
                None
            } else {
                Some((u128::from(n) * 1000 / u128::from(rate)).min(u128::from(u32::MAX)) as u32)
            }
        }))
}
pub fn decode_file(path: &Path) -> Result<Sound, Error> {
    let mut format = open(path)?;
    let track = audio_track(format.as_ref())?;
    if track
        .codec_params
        .n_frames
        .zip(track.codec_params.sample_rate)
        .is_some_and(|(n, rate)| {
            rate != 0 && u128::from(n) > u128::from(rate) * u128::from(MAX_DURATION_S)
        })
    {
        return Err(Error::TooLong);
    }
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &Default::default())
        .map_err(decode_error)?;
    let mut source = Vec::new();
    let mut samples = Vec::new();
    let mut source_rate_hz = 0;
    let mut duration_s = 0.0_f64;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(DecodeError::ResetRequired) => break,
            Err(DecodeError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(decode_error(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(DecodeError::DecodeError(_)) => continue,
            Err(DecodeError::ResetRequired) => break,
            Err(DecodeError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(decode_error(e)),
        };
        let rate_hz = decoded.spec().rate;
        let channels = decoded.spec().channels.count();
        if rate_hz == 0 || channels == 0 {
            return Err(Error::Decode("invalid audio dimensions".into()));
        }
        duration_s += decoded.frames() as f64 / f64::from(rate_hz);
        if duration_s > f64::from(MAX_DURATION_S) + 1e-9 {
            return Err(Error::TooLong);
        }
        if source_rate_hz != 0 && source_rate_hz != rate_hz {
            samples.extend(resample(&source, source_rate_hz));
            source.clear();
        }
        source_rate_hz = rate_hz;
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks_exact(channels) {
            source.push(sanitize(frame[0]));
            source.push(sanitize(frame[if channels == 1 { 0 } else { 1 }]));
        }
    }
    if source_rate_hz == 0 {
        return Err(Error::Decode("no decoded audio frames".into()));
    }
    samples.extend(resample(&source, source_rate_hz));
    if samples.len() / 2 > SAMPLE_RATE_HZ as usize * MAX_DURATION_S as usize {
        return Err(Error::TooLong);
    }
    Ok(Sound::from_interleaved_stereo(samples))
}

/// 64-tap Blackman-windowed sinc, 1024 fractional phases, normalized DC gain.
/// Cutoff follows the lower Nyquist rate, suppressing aliasing when downsampling.
fn resample(input: &[f32], rate_hz: u32) -> Vec<f32> {
    if rate_hz == SAMPLE_RATE_HZ {
        return input.to_vec();
    }
    const TAPS: usize = 64;
    const PHASES: usize = 1024;
    let cutoff = (f64::from(SAMPLE_RATE_HZ) / f64::from(rate_hz)).min(1.0) * 0.94;
    let mut table = vec![[0.0_f64; TAPS]; PHASES];
    for (phase, weights) in table.iter_mut().enumerate() {
        let fraction = phase as f64 / PHASES as f64;
        for (tap, weight) in weights.iter_mut().enumerate() {
            let x = tap as f64 - 31.0 - fraction;
            let arg = std::f64::consts::PI * x * cutoff;
            let sinc = if arg.abs() < 1e-12 {
                1.0
            } else {
                arg.sin() / arg
            };
            let angle = std::f64::consts::PI * x / 32.0;
            *weight = sinc * cutoff * (0.42 + 0.5 * angle.cos() + 0.08 * (2.0 * angle).cos());
        }
        let sum: f64 = weights.iter().sum();
        for weight in weights {
            *weight /= sum;
        }
    }
    let frames = input.len() / 2;
    let output_frames =
        (frames as u64 * u64::from(SAMPLE_RATE_HZ) + u64::from(rate_hz) / 2) / u64::from(rate_hz);
    let mut output = Vec::with_capacity(output_frames as usize * 2);
    for frame in 0..output_frames {
        let position = frame as f64 * f64::from(rate_hz) / f64::from(SAMPLE_RATE_HZ);
        let center = position.floor() as i64;
        let phase = ((position.fract() * PHASES as f64) as usize).min(PHASES - 1);
        let mut value = [0.0_f64; 2];
        for (tap, weight) in table[phase].iter().enumerate() {
            let index = center + tap as i64 - 31;
            if index >= 0 && (index as usize) < frames {
                value[0] += f64::from(input[index as usize * 2]) * weight;
                value[1] += f64::from(input[index as usize * 2 + 1]) * weight;
            }
        }
        output.extend(value.map(|s| s as f32));
    }
    output
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    pub(crate) fn wav(path: &Path, rate_hz: u32, frames: u32) {
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(rate_hz.to_le_bytes());
        bytes.extend((rate_hz * 2).to_le_bytes());
        bytes.extend(2_u16.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for i in 0..frames {
            let value = ((f64::from(i) * 1000.0 * std::f64::consts::TAU / f64::from(rate_hz)).sin()
                * 16384.0)
                .round() as i16;
            bytes.extend(value.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }
    #[test]
    fn mono_wav_resampled_and_measured() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tone.wav");
        wav(&path, 44_100, 44_100);
        assert_eq!(probe_duration_ms(&path).unwrap(), Some(1000));
        let sound = decode_file(&path).unwrap();
        assert!((i64::from(sound.duration_ms()) - 1000).abs() <= 1);
        assert_eq!(sound.frames(), 48_000);
        assert!(sound.samples().chunks_exact(2).all(|f| f[0] == f[1]));
        let crossings = sound
            .samples()
            .chunks_exact(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|f| f[0] <= 0.0 && f[1] > 0.0)
            .count();
        assert!((crossings as i32 - 1000).abs() <= 1);
        assert!((sound.peak_lin() - 0.5).abs() < 0.02);
        assert!((sound.rms_dbfs() + 9.03).abs() < 0.1);
        let silent = Sound::from_interleaved_stereo(vec![f32::NAN, f32::INFINITY, 0.0, 0.0]);
        assert_eq!(silent.rms_dbfs(), f32::NEG_INFINITY);
        assert_eq!(silent.peak_lin(), 0.0);
    }
    #[test]
    fn duration_limit_is_checked_before_decoding_pcm() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("long.wav");
        wav(&path, 48_000, 1);
        let mut bytes = std::fs::read(&path).unwrap();
        let data_bytes = (MAX_DURATION_S + 1) * 48_000 * 2;
        bytes[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes[40..44].copy_from_slice(&data_bytes.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(decode_file(&path), Err(Error::TooLong)));
    }
    #[test]
    fn downsampling_rejects_aliases() {
        let input = |freq_hz: f64| {
            (0..9600)
                .flat_map(|i| {
                    let s = (i as f64 * freq_hz * std::f64::consts::TAU / 96_000.0).sin() as f32;
                    [s, s]
                })
                .collect::<Vec<_>>()
        };
        let low = Sound::from_interleaved_stereo(resample(&input(1000.0), 96_000));
        let high = Sound::from_interleaved_stereo(resample(&input(30_000.0), 96_000));
        assert!(low.rms_dbfs() > -4.0);
        assert!(high.rms_dbfs() < -35.0);
    }
}
