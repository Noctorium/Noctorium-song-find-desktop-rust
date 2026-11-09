//! AudD: a few seconds of the sound, as a WAV file, posted with a token if there is one.

use super::{agent, text, Clip, Form};
use crate::song::Song;
use serde_json::Value;

pub fn identify(clip: &Clip, token: &str) -> Result<Option<Song>, String> {
    let mut form = Form::new().text("return", "apple_music,spotify,deezer");
    if !token.trim().is_empty() {
        form = form.text("api_token", token.trim());
    }
    let form = form.file("file", "clip.wav", "audio/wav", &clip.wav(12));
    let reply: Value = form
        .send(agent().post("https://api.audd.io/"))?
        .into_json()
        .map_err(|e| format!("its answer could not be read ({e})"))?;
    read(&reply)
}

pub fn read(reply: &Value) -> Result<Option<Song>, String> {
    if reply["status"].as_str() == Some("error") {
        let message = text(&reply["error"]["error_message"]).unwrap_or_else(|| "an error".into());
        return Err(message);
    }
    let result = &reply["result"];
    if !result.is_object() {
        return Ok(None);
    }
    let Some(title) = text(&result["title"]) else {
        return Ok(None);
    };
    let apple = &result["apple_music"];
    let spotify = &result["spotify"];
    let deezer = &result["deezer"];
    let cover = text(&apple["artwork"]["url"])
        .map(|u| u.replace("{w}", "600").replace("{h}", "600"))
        .or_else(|| text(&spotify["album"]["images"][0]["url"]))
        .or_else(|| text(&deezer["album"]["cover_xl"]));
    Ok(Some(Song {
        title,
        artist: text(&result["artist"]).unwrap_or_default(),
        album: text(&result["album"]),
        year: text(&result["release_date"]).map(|d| d.chars().take(4).collect()),
        label: text(&result["label"]),
        genre: text(&apple["genreNames"][0]),
        isrc: text(&apple["isrc"]).or_else(|| text(&spotify["external_ids"]["isrc"])),
        cover,
        page: text(&result["song_link"]),
        spotify: text(&spotify["external_urls"]["spotify"]),
        apple_music: text(&apple["url"]),
        deezer: text(&deezer["link"]),
        youtube: None,
        provider: String::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_result_is_read() {
        let reply = json!({"status": "success", "result": {
            "artist": "Imagine Dragons", "title": "Warriors", "album": "Warriors",
            "release_date": "2014-09-18", "label": "Universal", "song_link": "https://lis.tn/Warriors",
            "apple_music": {"url": "https://music.apple.com/us/album/1", "artwork": {"url": "https://a/{w}x{h}bb.jpg"}, "genreNames": ["Alternative"]},
            "spotify": {"external_urls": {"spotify": "https://open.spotify.com/track/1"}}
        }});
        let song = read(&reply).unwrap().unwrap();
        assert_eq!(song.title, "Warriors");
        assert_eq!(song.year.as_deref(), Some("2014"));
        assert_eq!(song.cover.as_deref(), Some("https://a/600x600bb.jpg"));
        assert_eq!(song.genre.as_deref(), Some("Alternative"));
    }

    #[test]
    fn nothing_and_errors_are_told_apart() {
        assert_eq!(
            read(&json!({"status": "success", "result": null})),
            Ok(None)
        );
        assert_eq!(
            read(
                &json!({"status": "error", "error": {"error_code": 901, "error_message": "Recognition failed: limit reached"}})
            ),
            Err("Recognition failed: limit reached".into())
        );
    }
}
