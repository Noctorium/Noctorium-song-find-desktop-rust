//! AcoustID: a Chromaprint fingerprint, made here, looked up in AcoustID and named from MusicBrainz.

use super::{agent, failure, text, Clip};
use crate::song::Song;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use serde_json::Value;

/// The fingerprint as fpcalc prints it: compressed, then URL-safe base64 with no padding.
pub fn fingerprint(clip: &Clip) -> Result<String, String> {
    use base64::Engine;
    // From the start of the song where it is known, as AcoustID's own fpcalc does: two minutes of it.
    let samples: &[i16] = match &clip.whole {
        Some(whole) => {
            &whole[..whole
                .len()
                .min(crate::signature::SAMPLE_RATE as usize * 120)]
        }
        None => &clip.samples,
    };
    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(crate::signature::SAMPLE_RATE, 1)
        .map_err(|e| format!("the fingerprint could not be made ({e:?})"))?;
    printer.consume(samples);
    printer.finish();
    let raw = printer.fingerprint();
    if raw.is_empty() {
        return Err("the sound was too short to fingerprint".into());
    }
    let compressed = FingerprintCompressor::from(&config).compress(raw);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(compressed))
}

pub fn identify(clip: &Clip, key: &str) -> Result<Option<Song>, String> {
    let fingerprint = fingerprint(clip)?;
    let reply: Value = agent()
        .post("https://api.acoustid.org/v2/lookup")
        .send_form(&[
            ("client", key.trim()),
            ("meta", "recordings releasegroups"),
            ("duration", &duration(clip).to_string()),
            ("fingerprint", &fingerprint),
        ])
        .map_err(failure)?
        .into_json()
        .map_err(|e| format!("its answer could not be read ({e})"))?;
    read(&reply)
}

/// How long the song is, in whole seconds: the file's length where there is one.
fn duration(clip: &Clip) -> u32 {
    let samples = clip.whole.as_ref().map_or(clip.samples.len(), |w| w.len());
    ((samples as f32 / crate::signature::SAMPLE_RATE as f32).round() as u32).max(1)
}

pub fn read(reply: &Value) -> Result<Option<Song>, String> {
    if reply["status"].as_str() != Some("ok") {
        return Err(
            text(&reply["error"]["message"]).unwrap_or_else(|| "an unexpected answer".into())
        );
    }
    let mut results: Vec<&Value> = reply["results"].as_array().into_iter().flatten().collect();
    results.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
    });
    for result in results {
        if result["score"].as_f64().unwrap_or(0.0) < 0.5 {
            continue;
        }
        for recording in result["recordings"].as_array().into_iter().flatten() {
            let Some(title) = text(&recording["title"]) else {
                continue;
            };
            let artists: Vec<String> = recording["artists"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| text(&a["name"]))
                .collect();
            let group = &recording["releasegroups"][0];
            let id = text(&recording["id"]);
            return Ok(Some(Song {
                title,
                artist: artists.join(", "),
                album: text(&group["title"]),
                cover: text(&group["id"])
                    .map(|g| format!("https://coverartarchive.org/release-group/{g}/front-500")),
                page: id.map(|id| format!("https://musicbrainz.org/recording/{id}")),
                ..Song::default()
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tune_has_a_fingerprint() {
        let clip = Clip {
            samples: crate::signature::tests_support::tune(8),
            whole: None,
        };
        let print = fingerprint(&clip).unwrap();
        assert!(print.len() > 20);
        assert!(!print.contains('=') && !print.contains('+') && !print.contains('/'));
    }

    #[test]
    fn the_best_recording_is_read() {
        let reply = json!({"status": "ok", "results": [
            {"score": 0.3, "recordings": [{"id": "a", "title": "Wrong"}]},
            {"score": 0.94, "recordings": [{"id": "b", "title": "Right", "artists": [{"name": "Band"}],
                "releasegroups": [{"id": "g", "title": "Album"}]}]}
        ]});
        let song = read(&reply).unwrap().unwrap();
        assert_eq!(song.title, "Right");
        assert_eq!(
            song.page.as_deref(),
            Some("https://musicbrainz.org/recording/b")
        );
        assert_eq!(read(&json!({"status": "ok", "results": []})), Ok(None));
        assert_eq!(
            read(&json!({"status": "error", "error": {"message": "invalid API key"}})),
            Err("invalid API key".into())
        );
    }
}
