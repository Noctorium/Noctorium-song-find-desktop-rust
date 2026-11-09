//! Noctorium Song Find: which song is playing.
//!
//! A window program on Windows, so that opening it from the Start menu does not flash a console up first.
//! Debug builds keep the console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use noctorium_song_find::providers::{self, Answer};
use noctorium_song_find::settings::{Places, Settings, Source};
use std::process::ExitCode;

const USAGE: &str = "\
Noctorium Song Find

Which song is playing: from the microphone, or from what the computer itself is playing, asked of
Shazam, AudD, ACRCloud and AcoustID. Every song found is kept in a history on this computer only.

  (nothing)              open the window
  --identify <file>      say which song an audio file is, and quit
  --listen [mic|system]  listen for the length Settings gives, say which song it is, and quit
  --version              the version
  --help                 this

  NOCTORIUM_SONG_FIND_HOME   keep settings and the history in this folder
  NOCTORIUM_NO_UPDATE_CHECK  do not ask GitHub whether a newer release is out
";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let said = |flags: &[&str]| arguments.iter().any(|a| flags.contains(&a.as_str()));
    let after = |flag: &str| {
        arguments
            .iter()
            .position(|a| a == flag)
            .and_then(|i| arguments.get(i + 1))
            .cloned()
    };

    if said(&["--version", "-V"]) {
        println!("Noctorium Song Find {}", noctorium_song_find::version());
        return ExitCode::SUCCESS;
    }
    if said(&["--help", "-h"]) {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if said(&["--identify"]) {
        let Some(file) = after("--identify") else {
            eprintln!("--identify wants a file.");
            return ExitCode::from(2);
        };
        return match noctorium_song_find::audio::decode_file(std::path::Path::new(&file)) {
            Ok(clip) => tell(&clip),
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        };
    }
    if said(&["--listen"]) {
        let source = match after("--listen").as_deref() {
            Some("system") => Source::System,
            _ => Source::Microphone,
        };
        let settings = Settings::load(&Places::system());
        eprintln!(
            "Listening to {} for {} seconds...",
            source.name().to_lowercase(),
            settings.seconds
        );
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        return match noctorium_song_find::audio::record(
            source,
            settings.microphone.as_deref(),
            settings.seconds,
            stop,
            |_| {},
        ) {
            Ok(clip) => tell(&clip),
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        };
    }

    if said(&[noctorium_song_find::update::RELAUNCH_ARGUMENT]) {
        // The copy before is closing; give it a moment to let go of its window.
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
    noctorium_song_find::update::clean_up();

    match noctorium_song_find::gui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            complain(&format!(
                "Noctorium Song Find could not open its window.\n\n{why}"
            ));
            ExitCode::FAILURE
        }
    }
}

/// What the providers said about a clip, on standard output.
fn tell(clip: &providers::Clip) -> ExitCode {
    let settings = Settings::load(&Places::system());
    let answers = providers::identify(clip, &settings, |provider, answer| {
        if answer.is_none() {
            eprintln!("Asking {}...", provider.name());
        }
    });
    let mut found = false;
    for (provider, answer) in answers {
        match answer {
            Answer::Found(song) => {
                found = true;
                println!("{}: {} -- {}", provider.name(), song.artist, song.title);
                if let Some(album) = song.album {
                    println!("  album: {album}");
                }
                if let Some(page) = song.page {
                    println!("  {page}");
                }
            }
            Answer::Unknown => eprintln!("{}: did not know it", provider.name()),
            Answer::Failed(why) => eprintln!("{why}"),
        }
    }
    if found {
        ExitCode::SUCCESS
    } else {
        eprintln!("No song was found.");
        ExitCode::FAILURE
    }
}

#[cfg(windows)]
fn complain(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Noctorium Song Find"));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn complain(text: &str) {
    eprintln!("{text}");
}
