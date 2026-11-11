//! What is kept between runs, and where: the settings and the history, as JSON in the system's own place
//! for an application's data. Nothing leaves the computer but what is sent to the providers to ask them.

use crate::providers::Provider;
use crate::song::Song;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the files are: `NOCTORIUM_SONG_FIND_HOME` if it is set, or the system's data folder.
#[derive(Debug, Clone)]
pub struct Places {
    pub folder: PathBuf,
}

impl Places {
    pub fn system() -> Places {
        let folder = std::env::var_os("NOCTORIUM_SONG_FIND_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|d| d.join("Noctorium Song Find")))
            .unwrap_or_else(|| PathBuf::from(".noctorium-song-find"));
        Places { folder }
    }

    pub fn settings(&self) -> PathBuf {
        self.folder.join("settings.json")
    }

    pub fn history(&self) -> PathBuf {
        self.folder.join("history.json")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Source {
    #[default]
    Microphone,
    /// What the computer is playing.
    System,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Microphone => "Microphone",
            Source::System => "System audio",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub source: Source,
    /// How long to listen for, in seconds.
    pub seconds: u32,
    /// The providers, in the order they are asked, and whether each is on.
    pub providers: Vec<(Provider, bool)>,
    /// Ask every provider rather than stopping at the first that knows the song.
    pub ask_everyone: bool,
    /// A microphone of the listener's choosing, by name; None for the system's default.
    pub microphone: Option<String>,
    pub keep_history: bool,
    /// A notification when a search ends while the window is behind another.
    pub notify_when_done: bool,
    pub audd_token: String,
    pub acrcloud_host: String,
    pub acrcloud_key: String,
    pub acrcloud_secret: String,
    pub acoustid_key: String,
    pub check_for_updates: bool,
    /// Install an update by itself, where it can be (Windows).
    pub install_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            source: Source::Microphone,
            seconds: 10,
            providers: Provider::ALL.iter().map(|&p| (p, true)).collect(),
            ask_everyone: false,
            microphone: None,
            keep_history: true,
            notify_when_done: true,
            audd_token: String::new(),
            acrcloud_host: String::new(),
            acrcloud_key: String::new(),
            acrcloud_secret: String::new(),
            acoustid_key: String::new(),
            check_for_updates: true,
            install_updates: true,
        }
    }
}

pub const SECONDS: [u32; 4] = [5, 8, 10, 15];

impl Settings {
    pub fn load(places: &Places) -> Settings {
        let mut settings: Settings = read(&places.settings()).unwrap_or_default();
        settings.tidy();
        settings
    }

    pub fn save(&self, places: &Places) -> Result<(), String> {
        write(&places.settings(), self)
    }

    /// Every provider once, whatever an older or hand-edited file said, and a length that is one of the
    /// lengths offered.
    pub fn tidy(&mut self) {
        let mut seen = Vec::new();
        self.providers.retain(|(p, _)| {
            let new = !seen.contains(p);
            seen.push(*p);
            new
        });
        for provider in Provider::ALL {
            if !seen.contains(&provider) {
                self.providers.push((provider, true));
            }
        }
        self.seconds = self.seconds.clamp(3, 20);
    }

    pub fn providers_in_order(&self) -> Vec<Provider> {
        self.providers
            .iter()
            .filter(|(_, on)| *on)
            .map(|(p, _)| *p)
            .collect()
    }
}

/// One song found, and when, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub song: Song,
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: String,
}

pub const HISTORY_MOST: usize = 500;

pub fn load_history(places: &Places) -> Vec<Found> {
    read(&places.history()).unwrap_or_default()
}

pub fn save_history(places: &Places, history: &[Found]) -> Result<(), String> {
    write(&places.history(), &history)
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Written beside and renamed into place, so a crash half way through leaves the last good file.
fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    let partial = path.with_extension("json.partial");
    let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&partial, json).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        let folder = std::env::temp_dir().join(format!(
            "noctorium-song-find-test-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        Places { folder }
    }

    #[test]
    fn settings_and_history_survive_a_restart() {
        let places = places();
        let mut settings = Settings {
            seconds: 15,
            audd_token: "t".into(),
            ..Settings::default()
        };
        settings.providers.swap(0, 2);
        settings.save(&places).unwrap();
        assert_eq!(Settings::load(&places), settings);

        let history = vec![Found {
            song: Song {
                title: "A".into(),
                ..Song::default()
            },
            at: chrono::Utc::now(),
            source: "Microphone".into(),
        }];
        save_history(&places, &history).unwrap();
        assert_eq!(load_history(&places), history);
        let _ = std::fs::remove_dir_all(&places.folder);
    }

    #[test]
    fn an_odd_file_is_tidied() {
        let mut settings = Settings {
            providers: vec![(Provider::AudD, false), (Provider::AudD, true)],
            seconds: 999,
            ..Settings::default()
        };
        settings.tidy();
        assert_eq!(settings.providers.len(), 4);
        assert_eq!(settings.providers[0], (Provider::AudD, false));
        assert_eq!(settings.seconds, 20);
        assert_eq!(settings.providers_in_order().len(), 3);
    }
}
