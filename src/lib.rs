//! Noctorium Song Find: which song is playing, heard through the microphone or taken from the computer's
//! own sound, asked of Shazam, AudD, ACRCloud and AcoustID.
//!
//! [audio] listens, [signature] makes Shazam's fingerprint, [providers] asks, [song] is what comes back,
//! [settings] is what is kept (the history included, which never leaves the computer), [update] whether a
//! newer release is out, and [gui] the window.

pub mod audio;
pub mod gui;
pub mod providers;
pub mod settings;
pub mod signature;
pub mod song;
pub mod update;

/// This build's version: the release tag's, which the release build passes in as NOCTORIUM_VERSION, or
/// Cargo's for a build of one's own.
pub fn version() -> &'static str {
    const FROM_TAG: Option<&str> = option_env!("NOCTORIUM_VERSION");
    match FROM_TAG
        .map(|v| v.trim().trim_start_matches('v'))
        .filter(|v| !v.is_empty())
    {
        Some(version) => version,
        None => env!("CARGO_PKG_VERSION"),
    }
}
