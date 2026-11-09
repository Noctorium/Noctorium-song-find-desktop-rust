//! A song that was found, whoever found it, and where it can be listened to.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub year: Option<String>,
    pub label: Option<String>,
    pub genre: Option<String>,
    pub isrc: Option<String>,
    /// A picture of the cover, as an address.
    pub cover: Option<String>,
    /// The provider's own page for it: Shazam's, AudD's song.link, MusicBrainz'.
    pub page: Option<String>,
    pub spotify: Option<String>,
    pub apple_music: Option<String>,
    pub deezer: Option<String>,
    pub youtube: Option<String>,
    /// Which provider found it, by name.
    pub provider: String,
}

/// Where a song can be heard, as (name, address): the provider's own links first, then searches by title
/// and artist on the services Noctorium plays from.
pub fn links(song: &Song) -> Vec<(&'static str, String)> {
    let query = format!("{} {}", song.artist, song.title);
    let q = encode(&query);
    let mut out = Vec::new();
    out.push((
        "Spotify",
        song.spotify
            .clone()
            .unwrap_or_else(|| format!("https://open.spotify.com/search/{q}")),
    ));
    out.push((
        "YouTube Music",
        song.youtube
            .clone()
            .unwrap_or_else(|| format!("https://music.youtube.com/search?q={q}")),
    ));
    out.push((
        "Apple Music",
        song.apple_music
            .clone()
            .unwrap_or_else(|| format!("https://music.apple.com/search?term={q}")),
    ));
    out.push((
        "Deezer",
        song.deezer
            .clone()
            .unwrap_or_else(|| format!("https://www.deezer.com/search/{q}")),
    ));
    out.push(("SoundCloud", format!("https://soundcloud.com/search?q={q}")));
    out.push((
        "Lyrics",
        format!(
            "https://www.google.com/search?q={}",
            encode(&format!("{query} lyrics"))
        ),
    ));
    if let Some(page) = &song.page {
        out.push(("Provider's page", page.clone()));
    }
    out
}

/// Percent-encoding for a query: everything but the unreserved characters.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_are_encoded() {
        assert_eq!(encode("AC/DC & Me"), "AC%2FDC%20%26%20Me");
        let song = Song {
            title: "Song".into(),
            artist: "Band".into(),
            spotify: Some("https://open.spotify.com/track/x".into()),
            ..Song::default()
        };
        let links = links(&song);
        assert_eq!(links[0].1, "https://open.spotify.com/track/x");
        assert_eq!(links[1].1, "https://music.youtube.com/search?q=Band%20Song");
    }
}
