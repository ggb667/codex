use super::*;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[test]
fn finds_only_active_pony_wavs_in_installed_assets() {
    let temp = TempDir::new().expect("temporary directory");
    let voice_dir = temp.path().join("voices/askingForHelp");
    fs::create_dir_all(&voice_dir).expect("create voice directory");
    let twilight_0 = voice_dir.join("twilight_sparkle_annoyed_variation0.wav");
    let twilight_1 = voice_dir.join("Twilight Sparkle concerned variation1.WAV");
    fs::write(&twilight_0, b"test wav").expect("write sound");
    fs::write(&twilight_1, b"test wav").expect("write sound");
    fs::write(voice_dir.join("rainbow_dash_annoyed.wav"), b"test wav")
        .expect("write other pony sound");
    fs::write(voice_dir.join("twilight_sparkle_notes.txt"), b"not a wav").expect("write non-sound");

    let sounds = resolve_sound_paths(
        "Twilight Sparkle",
        Some(temp.path().as_os_str().to_owned()),
        /*project_root*/ None,
    )
    .expect("resolve sounds");

    assert_eq!(sounds, vec![twilight_1, twilight_0]);
}

#[test]
fn falls_back_to_project_root_assets() {
    let temp = TempDir::new().expect("temporary directory");
    let voice_dir = temp.path().join("pony/assets/voices/askingForHelp");
    fs::create_dir_all(&voice_dir).expect("create voice directory");
    let sound = voice_dir.join("twilight_sparkle_help.wav");
    fs::write(&sound, b"test wav").expect("write sound");

    assert_eq!(
        resolve_sound_paths(
            "twilight-sparkle",
            Some(OsString::from(temp.path().join("missing"))),
            Some(temp.path().as_os_str().to_owned()),
        )
        .expect("resolve fallback sound"),
        vec![sound]
    );
}

#[test]
fn missing_or_unmatched_assets_return_not_found() {
    let temp = TempDir::new().expect("temporary directory");

    assert_eq!(
        resolve_sound_paths(
            "Twilight Sparkle",
            /*assets_dir*/ None,
            Some(temp.path().as_os_str().to_owned()),
        )
        .expect_err("missing assets should fail")
        .kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn fallback_bell_is_harmless_and_audible() {
    let mut output = Vec::new();

    ring_bell(&mut output).expect("write bell");

    assert_eq!(output, b"\x07");
}
