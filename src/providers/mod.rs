//! Who is asked what the song is.
//!
//! Four services, each asked in its own way:
//!
//! - **Shazam**, through the same fingerprint its own apps send ([crate::signature]). Needs nothing set up,
//!   and is the one that knows the most songs; it is not an official, documented service, so it may change.
//! - **AudD** (audd.io), sent a few seconds of the sound itself. Works without a token for a handful of
//!   tries a day; a token of one's own from audd.io lifts that.
//! - **ACRCloud**, sent the sound and signed with a project's access key and secret, made for free at
//!   console.acrcloud.com.
//! - **AcoustID** with MusicBrainz, sent a Chromaprint fingerprint. Free with an application key from
//!   acoustid.org, and good at whole songs -- an audio file -- rather than a few seconds from the middle.
//!
//! Every one of them is asked in the order Settings gives, and the first to know the song answers -- or,
//! if Settings says so, every one is asked and every answer shown.

pub mod acoustid;
pub mod acrcloud;
pub mod audd;
pub mod shazam;

use crate::settings::Settings;
use crate::song::Song;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Sound to identify: 16 kHz, one channel, as the fingerprint wants it and every service takes.
#[derive(Debug, Clone, Default)]
pub struct Clip {
    pub samples: Vec<i16>,
    /// The whole song from its start, when it came from a file: what AcoustID wants.
    pub whole: Option<std::sync::Arc<Vec<i16>>>,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / crate::signature::SAMPLE_RATE as f32
    }

    /// How loud it is at its loudest stretch, from 0 to 1: a clip that is silence is not worth sending.
    pub fn level(&self) -> f32 {
        self.samples
            .chunks(1600)
            .map(|c| {
                let sum: f64 = c.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
                (sum / c.len().max(1) as f64).sqrt() as f32
            })
            .fold(0.0, f32::max)
    }

    /// The clip as a 16-bit WAV file, for the services that are sent the sound.
    pub fn wav(&self, most_seconds: u32) -> Vec<u8> {
        let rate = crate::signature::SAMPLE_RATE;
        let samples = &self.samples[..self.samples.len().min((rate * most_seconds) as usize)];
        let data = (samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // one channel
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Shazam,
    AudD,
    AcrCloud,
    AcoustId,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Shazam,
        Provider::AudD,
        Provider::AcrCloud,
        Provider::AcoustId,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Shazam => "Shazam",
            Provider::AudD => "AudD",
            Provider::AcrCloud => "ACRCloud",
            Provider::AcoustId => "AcoustID",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Provider::Shazam => "Knows the most songs. Needs nothing set up. Only a fingerprint is sent, never the sound.",
            Provider::AudD => "Sent a few seconds of the sound. A few tries a day without a token; a token from audd.io lifts that.",
            Provider::AcrCloud => "Sent a few seconds of the sound. Needs a project's host, access key and secret, free at console.acrcloud.com.",
            Provider::AcoustId => "A Chromaprint fingerprint, looked up in MusicBrainz. Needs a free application key from acoustid.org. Best with whole songs, from a file.",
        }
    }

    /// Whether Settings has what this provider needs to be asked at all.
    pub fn ready(self, settings: &Settings) -> Result<(), &'static str> {
        let filled = |s: &str| !s.trim().is_empty();
        match self {
            Provider::Shazam | Provider::AudD => Ok(()),
            Provider::AcrCloud
                if filled(&settings.acrcloud_host)
                    && filled(&settings.acrcloud_key)
                    && filled(&settings.acrcloud_secret) =>
            {
                Ok(())
            }
            Provider::AcrCloud => Err("needs a host, access key and secret in Settings"),
            Provider::AcoustId if filled(&settings.acoustid_key) => Ok(()),
            Provider::AcoustId => Err("needs an application key in Settings"),
        }
    }
}

/// What one provider said.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Answer {
    Found(Song),
    /// It listened, and did not know the song.
    Unknown,
    /// It could not be asked, or did not answer sensibly; why, in words.
    Failed(String),
}

pub fn ask(provider: Provider, clip: &Clip, settings: &Settings) -> Answer {
    if let Err(why) = provider.ready(settings) {
        return Answer::Failed(format!("{} {why}.", provider.name()));
    }
    let answer = match provider {
        Provider::Shazam => shazam::identify(clip),
        Provider::AudD => audd::identify(clip, &settings.audd_token),
        Provider::AcrCloud => acrcloud::identify(
            clip,
            &settings.acrcloud_host,
            &settings.acrcloud_key,
            &settings.acrcloud_secret,
        ),
        Provider::AcoustId => acoustid::identify(clip, &settings.acoustid_key),
    };
    match answer {
        Ok(Some(mut song)) => {
            song.provider = provider.name().to_string();
            Answer::Found(song)
        }
        Ok(None) => Answer::Unknown,
        Err(why) => Answer::Failed(format!("{}: {why}", provider.name())),
    }
}

/// The providers Settings has switched on, in its order, each asked in turn until one knows the song; or,
/// with `everyone`, all of them. `told` hears each as it is asked and answers.
pub fn identify(
    clip: &Clip,
    settings: &Settings,
    mut told: impl FnMut(Provider, Option<&Answer>),
) -> Vec<(Provider, Answer)> {
    let mut answers = Vec::new();
    for provider in settings.providers_in_order() {
        if provider.ready(settings).is_err() {
            continue;
        }
        told(provider, None);
        let answer = ask(provider, clip, settings);
        told(provider, Some(&answer));
        let found = matches!(answer, Answer::Found(_));
        answers.push((provider, answer));
        if found && !settings.ask_everyone {
            break;
        }
    }
    answers
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(25))
        .user_agent(&format!("NoctoriumSongFind/{}", crate::version()))
        .build()
}

/// What an HTTP failure says, in words: the service's own message where it gave one.
pub(crate) fn failure(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let body: String = body.chars().take(200).collect();
            format!("the service answered {code} {}", body.trim())
        }
        ureq::Error::Transport(t) => format!("could not reach it ({t})"),
    }
}

/// A multipart/form-data body, which ureq 2 does not make by itself.
pub(crate) struct Form {
    boundary: String,
    body: Vec<u8>,
}

impl Form {
    pub fn new() -> Self {
        Form {
            boundary: format!("----NoctoriumSongFind{:016x}", rand::random::<u64>()),
            body: Vec::new(),
        }
    }

    pub fn text(mut self, name: &str, value: &str) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self
    }

    pub fn file(mut self, name: &str, file_name: &str, mime: &str, bytes: &[u8]) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {mime}\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    pub fn send(mut self, request: ureq::Request) -> Result<ureq::Response, String> {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        request
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={}", self.boundary),
            )
            .send_bytes(&self.body)
            .map_err(failure)
    }
}

/// Every string anywhere in a JSON value, for picking links out of answers whose shape varies.
pub(crate) fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

pub(crate) fn text(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_is_a_wav() {
        let clip = Clip {
            samples: vec![1000; 16_000 * 3],
            whole: None,
        };
        let wav = clip.wav(2);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 16_000 * 2 * 2);
        assert!((clip.seconds() - 3.0).abs() < 0.001);
        assert!(clip.level() > 0.02);
        assert_eq!(
            Clip {
                samples: vec![0; 1600],
                whole: None
            }
            .level(),
            0.0
        );
    }

    #[test]
    fn providers_that_need_keys_say_so() {
        let settings = Settings::default();
        assert!(Provider::Shazam.ready(&settings).is_ok());
        assert!(Provider::AudD.ready(&settings).is_ok());
        assert!(Provider::AcrCloud.ready(&settings).is_err());
        assert!(Provider::AcoustId.ready(&settings).is_err());
        assert!(matches!(
            ask(Provider::AcoustId, &Clip::default(), &settings),
            Answer::Failed(_)
        ));
    }
}
