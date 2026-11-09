//! Whether a newer Noctorium Song Find is out: the newest release in Noctorium-Installer, where every
//! Noctorium is published, asked once at start and compared by version, as Noctorium Rust does it. Off with
//! `NOCTORIUM_NO_UPDATE_CHECK` or in Settings. On Windows an update installs itself unless that is switched
//! off: the release's own zip, checked against its published SHA-256 and swapped in place of the program,
//! ready at the next start. On a Mac and Linux the release's terminal installer does it, once this window
//! has closed.

use std::time::Duration;

pub const RELEASES: &str = "https://github.com/Noctorium/Noctorium-Installer/releases/latest";
const LATEST: &str = "https://api.github.com/repos/Noctorium/Noctorium-Installer/releases/latest";

#[derive(Clone, Debug, PartialEq)]
pub struct Newer {
    pub version: String,
    pub url: String,
    /// The installer for this computer among the release's files, by its name and address.
    pub installer: Option<(String, String)>,
    /// Noctorium Song Find itself for this computer, with the checksum GitHub publishes for it: what the update is
    /// made from where it can be swapped in place (Windows).
    pub package: Option<Package>,
}

/// One file of a release: its name, its address, and its SHA-256 as GitHub gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

/// The zip of Noctorium Song Find for this computer, when the release says what its SHA-256 is. Only on Windows,
/// where the zip is the program and nothing else; elsewhere the installer puts its files in their places.
pub fn package_for(assets: &[(String, String, Option<String>)]) -> Option<Package> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return None;
    }
    assets
        .iter()
        .find(|(name, _, digest)| {
            name.starts_with("noctorium-song-find-")
                && name.ends_with("-windows-x64.zip")
                && digest.is_some()
        })
        .map(|(name, url, digest)| Package {
            name: name.clone(),
            url: url.clone(),
            sha256: digest.clone().unwrap_or_default(),
        })
}

/// The release's terminal installer for this computer, which installs Noctorium Song Find for whoever runs
/// it. None when the release has no Noctorium Song Find for this computer, or no installer.
pub fn installer_for(assets: &[(String, String)]) -> Option<(String, String)> {
    let (ours, installer) = if cfg!(windows) {
        (
            "-windows-x64.zip",
            "noctorium-installer-cli-windows-x64.exe",
        )
    } else if cfg!(target_os = "macos") {
        (
            if cfg!(target_arch = "aarch64") {
                "-macos-arm64.zip"
            } else {
                "-macos-x64.zip"
            },
            "noctorium-installer-cli-macos",
        )
    } else {
        ("-linux-x64.tar.gz", "noctorium-installer-cli-linux-x64")
    };
    assets
        .iter()
        .find(|(name, _)| name.starts_with("noctorium-song-find-") && name.ends_with(ours))?;
    assets.iter().find(|(name, _)| name == installer).cloned()
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let reply = ureq::get(url)
        .set(
            "User-Agent",
            concat!("Noctorium-Song-Find/", env!("CARGO_PKG_VERSION")),
        )
        .timeout(Duration::from_secs(600))
        .call()
        .map_err(|e| format!("The download failed: {e}"))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(reply.into_reader(), 400 * 1024 * 1024),
        &mut bytes,
    )
    .map_err(|e| format!("The download failed: {e}"))?;
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The program's own file, and the two names it is swapped through: `.new` while it is written, `.old` for
/// the copy that was running, removed at the next start.
fn program_files() -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    let program = std::env::current_exe()
        .map_err(|e| format!("Where Noctorium Song Find is could not be found: {e}"))?;
    let name = program
        .file_name()
        .ok_or("Where Noctorium Song Find is could not be found.")?
        .to_string_lossy()
        .into_owned();
    let folder = program
        .parent()
        .ok_or("Where Noctorium Song Find is could not be found.")?
        .to_path_buf();
    Ok((
        program.clone(),
        folder.join(format!("{name}.new")),
        folder.join(format!("{name}.old")),
    ))
}

/// Downloads the release's own zip, checks it against the SHA-256 GitHub publishes for it, and puts the new
/// program in place of this one: Windows lets a running program's file be renamed, though not overwritten,
/// so this one steps aside to `.old` and the new one takes its name. The next start is the new version.
///
/// Nothing else is run: no installer, no command prompt, nothing from the temporary folder. A program that
/// downloads another and starts it through cmd from %TEMP% is what a dropper looks like, and Windows
/// Defender flagged the update that did that.
pub fn stage(package: &Package) -> Result<(), String> {
    let bytes = download(&package.url)?;
    if !sha256_hex(&bytes).eq_ignore_ascii_case(package.sha256.trim()) {
        return Err(
            "The download did not match the checksum the release publishes, so it was not used."
                .into(),
        );
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("The download is not a zip: {e}"))?;
    let index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .map(|f| {
                    f.is_file()
                        && f.name()
                            .is_ok_and(|n| n.to_ascii_lowercase().ends_with(".exe"))
                })
                .unwrap_or(false)
        })
        .ok_or("The release's zip has no program in it.")?;
    let (program, new, old) = program_files()?;
    {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(&new).map_err(|e| format!("Noctorium Song Find's folder cannot be written to ({e}); download the update from its page."))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(&program, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!(
            "Noctorium Song Find could not step aside for the new version: {e}"
        ));
    }
    if let Err(e) = std::fs::rename(&new, &program) {
        // Put things back as they were, so there is still a program to start.
        let _ = std::fs::rename(&old, &program);
        let _ = std::fs::remove_file(&new);
        return Err(format!("The new version could not be put in place: {e}"));
    }
    Ok(())
}

/// What an update left behind: the copy that was running before it, and a download cut off half way. At start.
pub fn clean_up() {
    if let Ok((_, new, old)) = program_files() {
        let _ = std::fs::remove_file(old);
        let _ = std::fs::remove_file(new);
    }
}

/// What a relaunch is started with: wait for the copy before to close rather than bring it to the front.
pub const RELAUNCH_ARGUMENT: &str = "--relaunch";

/// Starts the program again -- the new version, now in this one's place -- told to wait for this one to go.
pub fn relaunch() -> Result<(), String> {
    let (program, _, _) = program_files()?;
    std::process::Command::new(program)
        .arg(RELAUNCH_ARGUMENT)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Noctorium Song Find could not start again: {e}"))
}

/// Where there is no package to swap in -- a Mac, Linux -- the release's terminal installer, started once
/// this window has closed so it can replace the program.
#[cfg(not(windows))]
pub fn install(newer: &Newer) -> Result<(), String> {
    let (name, url) = newer
        .installer
        .clone()
        .ok_or("This release has no Noctorium Song Find for this computer: see its page.")?;
    let folder = std::env::temp_dir().join(format!("noctorium-update-{}", newer.version));
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(name.replace(['/', '\\'], "_"));
    std::fs::write(&path, download(&url)?).map_err(|e| e.to_string())?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    std::process::Command::new("sh")
        .arg("-c")
        .arg("sleep 4; \"$0\" --product song-find --yes")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("The installer would not start: {e}"))
}

/// On Windows an update is the release's own zip, checked and swapped in place, never an installer started
/// behind the window; a release without its checksum is left to the listener, on its page.
#[cfg(windows)]
pub fn install(newer: &Newer) -> Result<(), String> {
    match &newer.package {
        Some(package) => stage(package),
        None => Err("This release cannot be checked, so it is not installed by itself: download it from its page.".into()),
    }
}

/// `v1.2.3`, `1.2`, `1.2.3-beta` as numbers to compare; None for anything else.
pub fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

/// The newest release if it is newer than this one. Quiet about every failure: no release yet, no network.
pub fn check() -> Option<Newer> {
    if std::env::var_os("NOCTORIUM_NO_UPDATE_CHECK").is_some() {
        return None;
    }
    let body: serde_json::Value = ureq::get(LATEST)
        .set(
            "User-Agent",
            concat!("Noctorium-Song-Find/", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let tag = body["tag_name"].as_str()?;
    let assets: Vec<(String, String, Option<String>)> = body["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            // GitHub gives each file's checksum as `sha256:<hex>`.
            let digest = a["digest"]
                .as_str()
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(String::from);
            Some((
                a["name"].as_str()?.to_string(),
                a["browser_download_url"].as_str()?.to_string(),
                digest,
            ))
        })
        .collect();
    let named: Vec<(String, String)> = assets
        .iter()
        .map(|(name, url, _)| (name.clone(), url.clone()))
        .collect();
    is_newer(tag, crate::version()).then(|| Newer {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: body["html_url"].as_str().unwrap_or(RELEASES).to_string(),
        installer: installer_for(&named),
        package: package_for(&assets),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.99.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("nonsense", "0.1.0"));
        assert_eq!(parse("v2.3.4-beta.1"), Some((2, 3, 4)));
    }

    #[test]
    fn the_installer_for_this_computer_is_picked() {
        let named = |names: &[&str]| {
            names
                .iter()
                .map(|n| (n.to_string(), format!("https://x/{n}")))
                .collect::<Vec<_>>()
        };
        let release = named(&[
            "noctorium-song-find-0.27.0-windows-x64.zip",
            "noctorium-song-find-0.27.0-linux-x64.tar.gz",
            "noctorium-song-find-0.27.0-macos-arm64.zip",
            "noctorium-song-find-0.27.0-macos-x64.zip",
            "noctorium-installer-cli-windows-x64.exe",
            "noctorium-installer-cli-linux-x64",
            "noctorium-installer-cli-macos",
            "Noctorium-0.21.0-windows-x64.msi",
        ]);
        let picked = installer_for(&release).unwrap().0;
        assert!(picked.starts_with("noctorium-installer-cli-"), "{picked}");
        // A release from before Noctorium Song Find was published has nothing to update to.
        assert!(installer_for(&named(&[
            "noctorium-installer-cli-windows-x64.exe",
            "noctorium-installer-cli-linux-x64",
            "noctorium-installer-cli-macos"
        ]))
        .is_none());
    }
}
