# Noctorium Song Find for the desktop

Which song is playing? Noctorium Song Find listens and tells you — like Shazam, as part of Noctorium. It is
the desktop's half of a pair; the phone's is
[Noctorium-song-find-mobile-dart](https://github.com/Noctorium/Noctorium-song-find-mobile-dart).

Written in Rust with egui, for Windows, Linux and macOS.

## Two ways to listen

- **Microphone** — a song playing in the room, on a speaker, in a shop.
- **System audio** — what this computer itself is playing: a video, a stream, a game, another app. Nothing
  has to be held up to a microphone.
  - **Windows:** WASAPI loopback of the default output.
  - **macOS 14.2 and later:** a Core Audio process tap of the default output; macOS asks for permission the
    first time.
  - **Linux:** the monitor of the default PulseAudio or PipeWire output.

Or **drop an audio file** onto the window (MP3, FLAC, WAV, Ogg, M4A, ALAC).

## Providers

Asked in the order you put them in Settings, until one knows the song — or all of them, if you prefer:

| Provider | What it needs | What is sent |
| --- | --- | --- |
| **Shazam** | nothing | a fingerprint of the sound, never the sound itself |
| **AudD** | nothing for a few tries a day; an API token from [audd.io](https://dashboard.audd.io/) for more | a few seconds of the sound |
| **ACRCloud** | a project's host, access key and secret, free at [console.acrcloud.com](https://console.acrcloud.com/) | a few seconds of the sound |
| **AcoustID** + MusicBrainz | a free application key from [acoustid.org](https://acoustid.org/new-application) | a Chromaprint fingerprint; best with whole songs from a file |

Shazam is asked the way its own Android app asks, with the fingerprint algorithm
[SongRec](https://github.com/marin-m/SongRec) worked out (GPL-3.0, like this). It is not an official,
documented service, so it may change without warning; the other providers are there for that day.

Every song found links to Spotify, YouTube Music, Apple Music, Deezer and SoundCloud, and to its lyrics.

## History

Every song found is kept in a history **on this computer only**, searchable, with each entry removable and
a button to clear it all. Settings can turn it off. The history and settings are JSON files in the system's
data folder (`%APPDATA%\Noctorium Song Find`, `~/.local/share/Noctorium Song Find`,
`~/Library/Application Support/Noctorium Song Find`), or in `NOCTORIUM_SONG_FIND_HOME` if that is set.

## Updates

At start it asks GitHub whether a newer Noctorium release is out, as every Noctorium does. On Windows the
update installs itself: the release's own zip, checked against the SHA-256 GitHub publishes for it, swapped
in place of the program and ready at the next start. On a Mac and Linux it offers to run the release's
terminal installer once the window closes. Settings › Updates turns either off, and
`NOCTORIUM_NO_UPDATE_CHECK` does too.

## Getting it

Every Noctorium release carries `noctorium-song-find-<version>-windows-x64.zip`, `-linux-x64.tar.gz`,
`-macos-arm64.zip` and `-macos-x64.zip` on
[the releases page](https://github.com/Noctorium/Noctorium-Installer/releases/latest), and every Noctorium
installer offers it.

## From the terminal

```bash
noctorium-song-find --identify song.mp3     # which song a file is
noctorium-song-find --listen system         # listen to the computer for the length Settings gives
noctorium-song-find --listen mic
```

## Building

With Rust 1.88 or newer. On Linux, the ALSA, X11 and Wayland headers too
(`libasound2-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev`).
On Windows, build from PowerShell or a developer prompt with MSVC.

```bash
cargo test
cargo run
packaging/package.sh 0.28.0 windows-x64     # or linux-x64, macos-arm64, macos-x64: what a release ships
```

`NOCTORIUM_SONG_FIND_CAPTURE=<folder>` draws each page, saves it as a PNG there and closes.

## Releasing

Noctorium-Installer's release workflow builds this repository at the release tag `vX.Y.Z` with
`packaging/package.sh`, as it does Noctorium Stats and Noctorium Rust.

## Licence

GPL-3.0-only, as the rest of Noctorium.
