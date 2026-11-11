//! Telling the listener a search is done when they are looking at something else: a notification from the
//! system, with the song's cover where there is one, that brings the window back when it is clicked.
//!
//! On Windows it is a toast through WinRT, under Song Find's own name and mark -- registered for this user
//! under HKCU\Software\Classes\AppUserModelId, as Windows asks of a program that is not a Store package --
//! and never through a helper process. Elsewhere it goes to the desktop's notification service.

use crate::settings::Places;
use std::path::{Path, PathBuf};

/// Song Find's name to the notification service.
pub const APP_ID: &str = "Noctorium.SongFind";

/// What to say.
#[derive(Debug, Clone)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// A picture to show beside it: the song's cover, by its address.
    pub cover: Option<String>,
}

/// Shows `note`, on a thread of its own, and calls `clicked` if the listener clicks it.
pub fn tell(places: &Places, note: Note, clicked: impl FnOnce() + Send + 'static) {
    let folder = places.folder.clone();
    std::thread::spawn(move || {
        let cover = note
            .cover
            .as_deref()
            .and_then(|url| fetch_cover(&folder, url));
        show(&folder, &note, cover.as_deref(), clicked);
    });
}

/// The cover, saved beside the settings for the notification service to read; None if it cannot be had.
fn fetch_cover(folder: &Path, url: &str) -> Option<PathBuf> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let response = crate::providers::agent().get(url).call().ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(response.into_reader(), 8 << 20),
        &mut bytes,
    )
    .ok()?;
    let format = image::guess_format(&bytes).ok()?;
    let extension = format.extensions_str().first().copied().unwrap_or("img");
    std::fs::create_dir_all(folder).ok()?;
    let path = folder.join(format!("notification-cover.{extension}"));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Song Find's mark as a PNG beside the settings, for the notification to carry.
fn mark_file(folder: &Path) -> Option<PathBuf> {
    let path = folder.join("song-find.png");
    if !path.exists() {
        std::fs::create_dir_all(folder).ok()?;
        image::save_buffer(
            &path,
            crate::gui::MARK,
            crate::gui::MARK_SIDE as u32,
            crate::gui::MARK_SIDE as u32,
            image::ColorType::Rgba8,
        )
        .ok()?;
    }
    Some(path)
}

#[cfg(windows)]
fn show(folder: &Path, note: &Note, cover: Option<&Path>, clicked: impl FnOnce() + Send + 'static) {
    use tauri_winrt_notification::{IconCrop, Toast};
    let mark = mark_file(folder);
    register(mark.as_deref());
    let mut toast = Toast::new(APP_ID).title(&note.title).text1(&note.body);
    if let Some(cover) = cover {
        toast = toast.icon(cover, IconCrop::Square, "");
    }
    let clicked = std::sync::Mutex::new(Some(clicked));
    let toast = toast.on_activated(move |_| {
        if let Some(clicked) = clicked.lock().ok().and_then(|mut c| c.take()) {
            clicked();
        }
        Ok(())
    });
    if let Err(why) = toast.show() {
        eprintln!("The notification could not be shown: {why:?}");
    }
    // The toast's events come to this thread's toast object; keep it a while so a click is still heard.
    std::thread::sleep(std::time::Duration::from_secs(30));
}

/// Windows shows a toast under the name and picture registered for its app id.
#[cfg(windows)]
fn register(icon: Option<&Path>) {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key_path = wide(&format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
    let mut key: HKEY = std::ptr::null_mut();
    let opened = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if opened != 0 {
        return;
    }
    let set = |name: &str, value: &str| {
        let (name, value) = (wide(name), wide(value));
        unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                (value.len() * 2) as u32,
            );
        }
    };
    set("DisplayName", crate::gui::TITLE);
    if let Some(icon) = icon {
        set("IconUri", &icon.to_string_lossy());
    }
    set("IconBackgroundColor", "0");
    unsafe {
        RegCloseKey(key);
    }
}

#[cfg(not(windows))]
fn show(folder: &Path, note: &Note, cover: Option<&Path>, clicked: impl FnOnce() + Send + 'static) {
    let mut notification = notify_rust::Notification::new();
    notification
        .appname(crate::gui::TITLE)
        .summary(&note.title)
        .body(&note.body);
    match cover.map(Path::to_path_buf).or_else(|| mark_file(folder)) {
        Some(picture) => {
            notification.icon(&picture.to_string_lossy());
        }
        None => {
            notification.icon("noctorium-song-find");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        notification.action("default", "Show");
        if let Ok(handle) = notification.show() {
            handle.wait_for_action(|action| {
                if action == "default" {
                    clicked();
                }
            });
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = clicked;
        let _ = notification.show();
    }
}
