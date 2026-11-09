//! Listening: the microphone, or the computer's own sound, recorded for a few seconds and brought down to
//! what the providers want -- 16 kHz in one channel. And audio files, decoded the same way.
//!
//! The computer's own sound is recorded as each system offers it: on Windows by WASAPI loopback of the
//! default output; on a Mac by a Core Audio process tap of the default output (macOS 14.2 and later), which
//! asks for permission the first time; on Linux from the monitor of the default PulseAudio or PipeWire
//! output.

use crate::providers::Clip;
use crate::settings::Source;
use crate::signature::SAMPLE_RATE;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The microphones there are, by name, for Settings to offer.
pub fn microphones() -> Vec<String> {
    let host = cpal::default_host();
    let mut names: Vec<String> = host
        .input_devices()
        .map(|devices| {
            devices
                .filter(|d| !is_monitor(d))
                .map(|d| d.to_string())
                .collect()
        })
        .unwrap_or_default();
    names.dedup();
    names
}

/// A PulseAudio monitor is an input that is really an output; it is not a microphone.
fn is_monitor(device: &cpal::Device) -> bool {
    device
        .id()
        .map(|id| id.id().ends_with(".monitor"))
        .unwrap_or(false)
}

fn microphone(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(name) = name {
        if let Ok(mut devices) = host.input_devices() {
            if let Some(device) = devices.find(|d| d.to_string() == name) {
                return Ok(device);
            }
        }
    }
    host.default_input_device()
        .ok_or_else(|| "There is no microphone. Plug one in, or allow this app to use it.".into())
}

#[cfg(target_os = "linux")]
fn system_output() -> Result<cpal::Device, String> {
    let host = cpal::host_from_id(cpal::HostId::PulseAudio).map_err(|_| {
        "The computer's sound is recorded through PulseAudio or PipeWire, and neither is running."
            .to_string()
    })?;
    let default_sink = host
        .default_output_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.id().to_string());
    let sources: Vec<cpal::Device> = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .filter(is_monitor)
        .collect();
    let wanted = default_sink.map(|sink| format!("{sink}.monitor"));
    sources
        .iter()
        .find(|d| {
            d.id()
                .map(|id| Some(id.id().to_string()) == wanted)
                .unwrap_or(false)
        })
        .or_else(|| sources.first())
        .cloned()
        .ok_or_else(|| "No output to listen to was found.".into())
}

#[cfg(not(target_os = "linux"))]
fn system_output() -> Result<cpal::Device, String> {
    cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "There is no sound output to listen to.".into())
}

/// Recording as it goes: how loud it is now, and how far along.
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub level: f32,
    pub fraction: f32,
}

/// Records for `seconds`, or until `stop` is set, telling `progress` along the way.
pub fn record(
    source: Source,
    microphone_name: Option<&str>,
    seconds: u32,
    stop: Arc<AtomicBool>,
    progress: impl Fn(Progress),
) -> Result<Clip, String> {
    let device = match source {
        Source::Microphone => microphone(microphone_name)?,
        Source::System => system_output()?,
    };
    let loopback = source == Source::System && !device.supports_input();
    let supported = if loopback {
        device.default_output_config()
    } else {
        device.default_input_config()
    }
    .map_err(|e| format!("{} could not be opened ({e}).", source.name()))?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config = supported.config();

    let gathered: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let level = Arc::new(Mutex::new(0f32));
    let failed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    macro_rules! stream_of {
        ($t:ty) => {{
            let gathered = gathered.clone();
            let level = level.clone();
            let failed = failed.clone();
            device.build_input_stream::<$t, _, _>(
                config.clone(),
                move |data: &[$t], _| {
                    let mut out = gathered.lock().unwrap();
                    let mut peak = 0f32;
                    for frame in data.chunks(channels) {
                        let sum: f32 = frame
                            .iter()
                            .map(|&s| <f32 as cpal::FromSample<$t>>::from_sample_(s))
                            .sum();
                        let mono = sum / channels as f32;
                        peak = peak.max(mono.abs());
                        out.push(mono);
                    }
                    let mut l = level.lock().unwrap();
                    *l = (*l * 0.7).max(peak);
                },
                move |e: cpal::Error| {
                    // A sample or two dropped or doubled is a glitch, not a failure: the recording goes on.
                    if e.kind() != cpal::ErrorKind::Xrun {
                        *failed.lock().unwrap() = Some(e.to_string());
                    }
                },
                Some(Duration::from_secs(5)),
            )
        }};
    }
    let stream = match format {
        cpal::SampleFormat::F32 => stream_of!(f32),
        cpal::SampleFormat::I16 => stream_of!(i16),
        cpal::SampleFormat::I32 => stream_of!(i32),
        cpal::SampleFormat::U16 => stream_of!(u16),
        cpal::SampleFormat::U8 => stream_of!(u8),
        cpal::SampleFormat::F64 => stream_of!(f64),
        other => {
            return Err(format!(
                "{} gives sound as {other}, which is not read here.",
                source.name()
            ))
        }
    }
    .map_err(|e| explain(source, e.to_string()))?;
    stream.play().map_err(|e| explain(source, e.to_string()))?;

    let start = Instant::now();
    let length = Duration::from_secs(seconds as u64);
    while start.elapsed() < length && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(40));
        if let Some(why) = failed.lock().unwrap().take() {
            return Err(explain(source, why));
        }
        let now = {
            let mut l = level.lock().unwrap();
            let now = *l;
            *l *= 0.85;
            now
        };
        progress(Progress {
            level: now.min(1.0),
            fraction: (start.elapsed().as_secs_f32() / length.as_secs_f32()).min(1.0),
        });
    }
    drop(stream);
    let samples = std::mem::take(&mut *gathered.lock().unwrap());
    if samples.is_empty() && source == Source::System {
        // WASAPI loopback hands over nothing at all while nothing is playing.
        return Ok(Clip::default());
    }
    Ok(Clip {
        samples: to_16k(&samples, rate),
        whole: None,
    })
}

fn explain(source: Source, why: String) -> String {
    let lower = why.to_lowercase();
    if lower.contains("permission") || lower.contains("denied") || lower.contains("access") {
        match source {
            Source::Microphone => format!("This app may not use the microphone ({why}). Allow it in the system's privacy settings, then try again."),
            Source::System => format!("This app may not record the computer's sound ({why}). Allow it in the system's privacy settings, then try again."),
        }
    } else {
        format!("{} could not be recorded ({why}).", source.name())
    }
}

/// Mono samples at `rate`, filtered below 7.6 kHz and resampled to 16 kHz.
pub fn to_16k(samples: &[f32], rate: u32) -> Vec<i16> {
    let filtered = if rate > SAMPLE_RATE {
        low_pass(samples, 7_600.0 / rate as f32)
    } else {
        samples.to_vec()
    };
    let step = rate as f64 / SAMPLE_RATE as f64;
    let count = (filtered.len() as f64 / step) as usize;
    (0..count)
        .map(|i| {
            let at = i as f64 * step;
            let low = at.floor() as usize;
            let high = (low + 1).min(filtered.len() - 1);
            let t = (at - low as f64) as f32;
            let v = filtered[low] * (1.0 - t) + filtered[high] * t;
            (v.clamp(-1.0, 1.0) * 32767.0) as i16
        })
        .collect()
}

/// A windowed-sinc low-pass with its cut-off as a fraction of the sample rate.
fn low_pass(samples: &[f32], cutoff: f32) -> Vec<f32> {
    const TAPS: usize = 63;
    let middle = (TAPS / 2) as f32;
    let mut kernel: Vec<f32> = (0..TAPS)
        .map(|i| {
            let x = i as f32 - middle;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
            };
            let blackman = 0.42
                - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (TAPS - 1) as f32).cos()
                + 0.08 * (4.0 * std::f32::consts::PI * i as f32 / (TAPS - 1) as f32).cos();
            sinc * blackman
        })
        .collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);
    let half = TAPS / 2;
    (0..samples.len())
        .map(|i| {
            let mut acc = 0f32;
            for (k, weight) in kernel.iter().enumerate() {
                let j = i as isize + k as isize - half as isize;
                if j >= 0 && (j as usize) < samples.len() {
                    acc += samples[j as usize] * weight;
                }
            }
            acc
        })
        .collect()
}

/// An audio file, decoded: twelve seconds from a third of the way in for the providers that want a
/// snippet, and the whole of it from the start for AcoustID.
pub fn decode_file(path: &std::path::Path) -> Result<Clip, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file =
        std::fs::File::open(path).map_err(|e| format!("The file could not be opened ({e})."))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| {
            "That file is not audio this app can read (MP3, FLAC, WAV, Ogg, M4A and ALAC are)."
                .to_string()
        })?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("The file has no sound in it.")?;
    let id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or("The file does not say its sample rate.")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("The file's sound could not be decoded ({e})."))?;

    // At most ten minutes: plenty for any song, and a long mix stays a few tens of megabytes.
    let most = rate as usize * 600;
    let mut mono: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(_)) | Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("The file could not be read to the end ({e}).")),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("The file's sound could not be decoded ({e}).")),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }
        if mono.len() >= most {
            break;
        }
    }
    if mono.is_empty() {
        return Err("The file has no sound in it.".into());
    }
    let whole = to_16k(&mono, rate);
    let snippet = (SAMPLE_RATE * 12) as usize;
    let start = if whole.len() > snippet * 2 {
        whole.len() / 3
    } else {
        0
    };
    let end = (start + snippet).min(whole.len());
    Ok(Clip {
        samples: whole[start..end].to_vec(),
        whole: Some(Arc::new(whole)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forty_eight_kilohertz_comes_down_to_sixteen() {
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let out = to_16k(&tone, 48_000);
        assert!((out.len() as i32 - 16_000).abs() <= 1);
        let peak = out.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((15_000..17_500).contains(&peak), "{peak}");
    }

    #[test]
    fn what_is_too_high_to_keep_is_filtered_out() {
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 12_000.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let out = to_16k(&tone, 48_000);
        let peak = out[100..out.len() - 100]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap();
        assert!(peak < 1_500, "{peak}");
    }

    #[test]
    fn a_wav_file_is_decoded() {
        let clip = Clip {
            samples: crate::signature::tests_support::tune(30),
            whole: None,
        };
        let path =
            std::env::temp_dir().join(format!("noctorium-song-find-{}.wav", std::process::id()));
        std::fs::write(&path, clip.wav(60)).unwrap();
        let decoded = decode_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(decoded.samples.len(), 16_000 * 12);
        assert_eq!(decoded.whole.as_ref().unwrap().len(), 16_000 * 30);
    }
}
