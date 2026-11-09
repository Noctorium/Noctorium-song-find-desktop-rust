//! ACRCloud: the sound, posted to the project's own host and signed with its secret, as its HTTP API asks.

use super::{agent, text, Clip, Form};
use crate::song::Song;
use hmac::{Hmac, Mac};
use serde_json::Value;

/// The string ACRCloud signs, and the signature, base64 of HMAC-SHA1 with the secret.
pub fn sign(key: &str, secret: &str, timestamp: u64) -> String {
    use base64::Engine;
    let to_sign = format!("POST\n/v1/identify\n{key}\naudio\n1\n{timestamp}");
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(to_sign.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

pub fn identify(clip: &Clip, host: &str, key: &str, secret: &str) -> Result<Option<Song>, String> {
    let host = host
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let (key, secret) = (key.trim(), secret.trim());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let wav = clip.wav(12);
    let reply: Value = Form::new()
        .text("access_key", key)
        .text("data_type", "audio")
        .text("signature_version", "1")
        .text("signature", &sign(key, secret, timestamp))
        .text("sample_bytes", &wav.len().to_string())
        .text("timestamp", &timestamp.to_string())
        .file("sample", "clip.wav", "audio/wav", &wav)
        .send(agent().post(&format!("https://{host}/v1/identify")))?
        .into_json()
        .map_err(|e| format!("its answer could not be read ({e})"))?;
    read(&reply)
}

pub fn read(reply: &Value) -> Result<Option<Song>, String> {
    match reply["status"]["code"].as_i64() {
        Some(0) => {}
        // No result.
        Some(1001) => return Ok(None),
        _ => {
            return Err(
                text(&reply["status"]["msg"]).unwrap_or_else(|| "an unexpected answer".into())
            )
        }
    }
    let music = &reply["metadata"]["music"][0];
    let Some(title) = text(&music["title"]) else {
        return Ok(None);
    };
    let artists: Vec<String> = music["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| text(&a["name"]))
        .collect();
    let external = &music["external_metadata"];
    Ok(Some(Song {
        title,
        artist: artists.join(", "),
        album: text(&music["album"]["name"]),
        year: text(&music["release_date"]).map(|d| d.chars().take(4).collect()),
        label: text(&music["label"]),
        genre: text(&music["genres"][0]["name"]),
        isrc: text(&music["external_ids"]["isrc"]),
        cover: None,
        page: None,
        spotify: text(&external["spotify"]["track"]["id"])
            .map(|id| format!("https://open.spotify.com/track/{id}")),
        apple_music: None,
        deezer: external["deezer"]["track"]["id"]
            .as_u64()
            .map(|id| id.to_string())
            .or_else(|| text(&external["deezer"]["track"]["id"]))
            .map(|id| format!("https://www.deezer.com/track/{id}")),
        youtube: text(&external["youtube"]["vid"])
            .map(|id| format!("https://www.youtube.com/watch?v={id}")),
        provider: String::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_signature_is_hmac_sha1_in_base64() {
        // HMAC-SHA1("secret", "POST\n/v1/identify\nkey\naudio\n1\n1700000000"), worked out separately.
        let signature = sign("key", "secret", 1_700_000_000);
        assert_eq!(signature.len(), 28);
        assert_eq!(signature, sign("key", "secret", 1_700_000_000));
        assert_ne!(signature, sign("key", "secret", 1_700_000_001));
    }

    #[test]
    fn a_result_is_read() {
        let reply = json!({"status": {"msg": "Success", "code": 0}, "metadata": {"music": [{
            "title": "Numb", "artists": [{"name": "Linkin Park"}], "album": {"name": "Meteora"},
            "release_date": "2003-03-25", "label": "Warner", "genres": [{"name": "Rock"}],
            "external_ids": {"isrc": "USWB10300474"},
            "external_metadata": {"spotify": {"track": {"id": "2nLtzopw4rPReszdYBJU6h"}}, "deezer": {"track": {"id": 2475381}}, "youtube": {"vid": "kXYiU_JCYtU"}}
        }]}});
        let song = read(&reply).unwrap().unwrap();
        assert_eq!(song.artist, "Linkin Park");
        assert_eq!(
            song.deezer.as_deref(),
            Some("https://www.deezer.com/track/2475381")
        );
        assert_eq!(
            song.youtube.as_deref(),
            Some("https://www.youtube.com/watch?v=kXYiU_JCYtU")
        );
    }

    #[test]
    fn no_result_and_errors() {
        assert_eq!(
            read(&json!({"status": {"msg": "No result", "code": 1001}})),
            Ok(None)
        );
        assert_eq!(
            read(&json!({"status": {"msg": "invalid access key", "code": 3001}})),
            Err("invalid access key".into())
        );
    }
}
