//! Best-effort audible notifications for Pony attention requests.

use rand::seq::IndexedRandom;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const PONY_ASSETS_DIR_ENV: &str = "AGENIC_PROJECT_PONY_ASSETS_DIR";
const PROJECT_ROOT_ENV: &str = "AGENIC_PROJECT_ROOT";

/// Starts an alert without waiting for asset discovery or audio playback to finish.
pub(crate) fn notify_attention(pony_name: &str) {
    let pony_name = pony_name.to_string();
    let spawn_result = std::thread::Builder::new()
        .name("pony-notification".to_string())
        .spawn(move || {
            if play_random_sound(&pony_name).is_err() {
                let _ = ring_bell(&mut io::stderr());
            }
        });
    if spawn_result.is_err() {
        let _ = ring_bell(&mut io::stderr());
    }
}

fn play_random_sound(pony_name: &str) -> io::Result<()> {
    let sound_paths = resolve_sound_paths(
        pony_name,
        std::env::var_os(PONY_ASSETS_DIR_ENV),
        std::env::var_os(PROJECT_ROOT_ENV),
    )?;
    let sound_path = sound_paths
        .choose(&mut rand::rng())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no matching Pony WAV files"))?;
    play_sound(sound_path)
}

fn resolve_sound_paths(
    pony_name: &str,
    assets_dir: Option<OsString>,
    project_root: Option<OsString>,
) -> io::Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    if let Some(assets_dir) = assets_dir.filter(|value| !value.is_empty()) {
        directories.push(PathBuf::from(assets_dir).join("voices/askingForHelp"));
    }
    if let Some(project_root) = project_root.filter(|value| !value.is_empty()) {
        let fallback = PathBuf::from(project_root).join("pony/assets/voices/askingForHelp");
        if !directories.contains(&fallback) {
            directories.push(fallback);
        }
    }

    let pony_identity = pony_name
        .rsplit_once(':')
        .map_or(pony_name, |(_, identity)| identity);
    let identity_prefix = format!("{}_", normalize_name(pony_identity));
    for directory in directories {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        let mut sounds = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("wav")
                }) && path.file_stem().is_some_and(|stem| {
                    normalize_name(&stem.to_string_lossy()).starts_with(&identity_prefix)
                })
            })
            .collect::<Vec<_>>();
        sounds.sort();
        if !sounds.is_empty() {
            return Ok(sounds);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no matching Pony WAV files in the installed voice library",
    ))
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn play_sound(sound_path: &Path) -> io::Result<()> {
    let windows_path = windows_sound_path(sound_path)?;
    let escaped_path = windows_path.replace('\'', "''");
    let script = format!(
        "$player = New-Object System.Media.SoundPlayer '{escaped_path}'; $player.PlaySync()"
    );
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "PowerShell SoundPlayer exited with {status}"
        )))
    }
}

fn windows_sound_path(sound_path: &Path) -> io::Result<String> {
    if cfg!(windows) {
        return Ok(sound_path.to_string_lossy().into_owned());
    }
    if !is_wsl() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Pony WAV playback is supported on Windows and WSL",
        ));
    }
    let output = Command::new("wslpath").arg("-w").arg(sound_path).output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            "wslpath failed to translate Pony WAV path",
        ));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Err(io::Error::other("wslpath returned an empty Pony WAV path"))
    } else {
        Ok(path)
    }
}

fn is_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

fn ring_bell(output: &mut impl Write) -> io::Result<()> {
    output.write_all(b"\x07")?;
    output.flush()
}

#[cfg(test)]
#[path = "pony_notification_tests.rs"]
mod tests;
