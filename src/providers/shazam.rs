//! Shazam, asked as its Android app asks: the fingerprint, a time, and a place, posted to its tag endpoint.

use super::{agent, failure, strings, text, Clip};
use crate::signature::Generator;
use crate::song::Song;
use serde_json::{json, Value};

const USER_AGENTS: &[&str] = &[
    "Dalvik/2.1.0 (Linux; U; Android 5.0.2; VS980 4G Build/LRX22G)",
    "Dalvik/1.6.0 (Linux; U; Android 4.4.2; SM-T210 Build/KOT49H)",
    "Dalvik/2.1.0 (Linux; U; Android 5.1.1; SM-P905V Build/LMY47X)",
    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G920F Build/MMB29K)",
    "Dalvik/2.1.0 (Linux; U; Android 7.0; SM-G930F Build/NRD90M)",
    "Dalvik/2.1.0 (Linux; U; Android 8.0.0; SM-G950F Build/R16NW)",
    "Dalvik/2.1.0 (Linux; U; Android 9; SM-G960F Build/PPR1.180610.011)",
    "Dalvik/2.1.0 (Linux; U; Android 10; Pixel 3 Build/QQ3A.200805.001)",
];

pub fn identify(clip: &Clip) -> Result<Option<Song>, String> {
    let signature = Generator::of(&clip.samples);
    if signature.peak_count() == 0 {
        return Ok(None);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let body = json!({
        "geolocation": {"altitude": 300, "latitude": 45, "longitude": 2},
        "signature": {
            "samplems": signature.milliseconds(),
            "timestamp": now,
            "uri": signature.uri(),
        },
        "timestamp": now,
        "timezone": "Europe/Paris",
    });
    let url = format!(
        "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}",
        uuid::Uuid::new_v4().to_string().to_uppercase(),
        uuid::Uuid::new_v4().to_string().to_uppercase()
    );
    let user_agent = USER_AGENTS[rand::random_range(0..USER_AGENTS.len())];
    let reply: Value = agent()
        .post(&url)
        .query("sync", "true")
        .query("webv3", "true")
        .query("sampling", "true")
        .query("connected", "")
        .query("shazamapiversion", "v3")
        .query("sharehub", "true")
        .query("video", "v3")
        .set("User-Agent", user_agent)
        .set("Content-Language", "en_US")
        .send_json(body)
        .map_err(failure)?
        .into_json()
        .map_err(|e| format!("its answer could not be read ({e})"))?;
    Ok(read(&reply))
}

/// The song in Shazam's answer, if it found one.
pub fn read(reply: &Value) -> Option<Song> {
    let matched = reply["matches"].as_array().is_some_and(|m| !m.is_empty());
    let track = &reply["track"];
    if !matched || !track.is_object() {
        return None;
    }
    let mut song = Song {
        title: text(&track["title"])?,
        artist: text(&track["subtitle"]).unwrap_or_default(),
        cover: text(&track["images"]["coverarthq"]).or_else(|| text(&track["images"]["coverart"])),
        page: text(&track["url"]),
        isrc: text(&track["isrc"]),
        genre: text(&track["genres"]["primary"]),
        ..Song::default()
    };
    for section in track["sections"].as_array().into_iter().flatten() {
        for item in section["metadata"].as_array().into_iter().flatten() {
            let value = text(&item["text"]);
            match item["title"].as_str().unwrap_or_default() {
                "Album" => song.album = song.album.take().or(value),
                "Released" => song.year = song.year.take().or(value),
                "Label" => song.label = song.label.take().or(value),
                _ => {}
            }
        }
    }
    let mut all = Vec::new();
    strings(&track["hub"], &mut all);
    for link in all {
        if link.starts_with("https://music.apple.com/") && song.apple_music.is_none() {
            song.apple_music = Some(link);
        } else if link.starts_with("spotify:track:") && song.spotify.is_none() {
            song.spotify = Some(format!(
                "https://open.spotify.com/track/{}",
                link.trim_start_matches("spotify:track:")
            ));
        } else if link.starts_with("https://open.spotify.com/track/") && song.spotify.is_none() {
            song.spotify = Some(link);
        } else if link.starts_with("https://www.deezer.com/track/") && song.deezer.is_none() {
            song.deezer = Some(link);
        } else if (link.starts_with("https://www.youtube.com/watch")
            || link.starts_with("https://youtu.be/"))
            && song.youtube.is_none()
        {
            song.youtube = Some(link);
        }
    }
    Some(song)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_with_a_match_is_read() {
        let reply = json!({
            "matches": [{"id": "1", "offset": 31.2}],
            "track": {
                "title": "Midnight City",
                "subtitle": "M83",
                "url": "https://www.shazam.com/track/54474034/midnight-city",
                "isrc": "FR6V81141061",
                "genres": {"primary": "Alternative"},
                "images": {"coverart": "https://is1.example/400x400cc.jpg", "coverarthq": "https://is1.example/800.jpg"},
                "sections": [{"type": "SONG", "metadata": [
                    {"title": "Album", "text": "Hurry Up, We're Dreaming"},
                    {"title": "Label", "text": "Naive"},
                    {"title": "Released", "text": "2011"}
                ]}],
                "hub": {
                    "options": [{"actions": [{"uri": "https://music.apple.com/us/album/x?i=1"}]}],
                    "providers": [{"type": "SPOTIFY", "actions": [{"uri": "spotify:search:Midnight%20City"}, {"uri": "spotify:track:6GyFP1nfCDB8lbD2bG0Hq9"}]}]
                }
            }
        });
        let song = read(&reply).unwrap();
        assert_eq!(song.title, "Midnight City");
        assert_eq!(song.artist, "M83");
        assert_eq!(song.album.as_deref(), Some("Hurry Up, We're Dreaming"));
        assert_eq!(song.year.as_deref(), Some("2011"));
        assert_eq!(song.label.as_deref(), Some("Naive"));
        assert_eq!(song.cover.as_deref(), Some("https://is1.example/800.jpg"));
        assert_eq!(
            song.spotify.as_deref(),
            Some("https://open.spotify.com/track/6GyFP1nfCDB8lbD2bG0Hq9")
        );
        assert!(song
            .apple_music
            .unwrap()
            .starts_with("https://music.apple.com/"));
    }

    #[test]
    fn an_answer_without_a_match_is_nothing() {
        assert!(read(&json!({"matches": [], "tagid": "x", "retryms": 12000})).is_none());
        assert!(read(&json!({})).is_none());
    }
}
