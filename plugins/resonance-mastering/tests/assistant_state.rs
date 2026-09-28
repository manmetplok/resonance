//! The assistant's target choice — mode, genre, reference file — is saved
//! with the plugin state and restored from it (warmth-width-depth.md §7.4,
//! the plugin-audit finding that it was GUI-only and lost on reload).

use std::time::{Duration, Instant};

use resonance_mastering::assistant::{AssistantSettings, Genre, Target, TargetMode, STATE_KEY};
use resonance_mastering::ResonanceMastering;
use resonance_plugin::ResonancePlugin;

fn state_json(plugin: &ResonanceMastering) -> serde_json::Value {
    serde_json::from_slice(&plugin.save_state()).expect("state is JSON")
}

#[test]
fn an_untouched_assistant_saves_no_entry() {
    let plugin = ResonanceMastering::new();
    assert!(state_json(&plugin).get(STATE_KEY).is_none());
}

#[test]
fn genre_and_mode_round_trip_through_the_state() {
    let plugin = ResonanceMastering::new();
    plugin.viz().assistant.set_genre(Genre::Acoustic);
    plugin.viz().assistant.set_mode(TargetMode::Genre);
    let blob = plugin.save_state();
    let saved = state_json(&plugin);
    assert_eq!(saved[STATE_KEY]["genre"], "acoustic");
    assert_eq!(saved[STATE_KEY]["mode"], "genre");

    let mut restored = ResonanceMastering::new();
    assert!(restored.load_state(&blob));
    assert_eq!(
        restored.viz().assistant.settings(),
        AssistantSettings {
            mode: TargetMode::Genre,
            genre: Genre::Acoustic,
            reference_path: String::new(),
        }
    );
}

/// Loading a state replaces the whole choice: a state without the entry
/// resets an assistant that had one.
#[test]
fn loading_a_state_without_the_entry_resets_the_choice() {
    let mut plugin = ResonanceMastering::new();
    plugin.viz().assistant.set_genre(Genre::Pop);
    let fresh = ResonanceMastering::new().save_state();
    assert!(plugin.load_state(&fresh));
    assert_eq!(plugin.viz().assistant.settings(), AssistantSettings::default());
}

#[test]
fn unknown_values_fall_back_to_the_defaults() {
    let mut plugin = ResonanceMastering::new();
    let blob = br#"{"version":1,"params":{},"assistant":{"mode":"telepathy","genre":"polka"}}"#;
    assert!(plugin.load_state(blob));
    assert_eq!(plugin.viz().assistant.settings(), AssistantSettings::default());
}

/// Write a short stereo 16-bit WAV of a 1 kHz tone.
fn write_wav(path: &std::path::Path, seconds: f32) {
    let rate = 48_000u32;
    let frames = (seconds * rate as f32) as usize;
    let mut data = Vec::with_capacity(frames * 4);
    for n in 0..frames {
        let s = (0.25 * (std::f32::consts::TAU * 1_000.0 * n as f32 / rate as f32).sin()
            * i16::MAX as f32) as i16;
        data.extend_from_slice(&s.to_le_bytes());
        data.extend_from_slice(&s.to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).expect("write wav");
}

/// A saved reference path comes back and is decoded again in the
/// background, so reopening the project shows the reference loaded.
#[test]
fn the_reference_path_round_trips_and_is_reloaded() {
    let dir = std::env::temp_dir().join(format!("mastering-ref-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reference.wav");
    write_wav(&path, 3.0);
    let path = path.to_string_lossy().into_owned();

    let plugin = ResonanceMastering::new();
    plugin.viz().assistant.set_mode(TargetMode::Reference);
    plugin.viz().assistant.load_reference(&path).expect("reference loads");
    let blob = plugin.save_state();
    assert_eq!(state_json(&plugin)[STATE_KEY]["reference_path"], path.as_str());

    let mut restored = ResonanceMastering::new();
    assert!(restored.load_state(&blob));
    let settings = restored.viz().assistant.settings();
    assert_eq!(settings.mode, TargetMode::Reference);
    assert_eq!(settings.reference_path, path);

    let deadline = Instant::now() + Duration::from_secs(20);
    while restored.viz().assistant.reference().is_none() {
        assert!(
            restored.viz().assistant.reference_error().is_none(),
            "reload failed: {:?}",
            restored.viz().assistant.reference_error()
        );
        assert!(Instant::now() < deadline, "the reference never reloaded");
        std::thread::sleep(Duration::from_millis(10));
    }
    let reference = restored.viz().assistant.reference().unwrap();
    assert_eq!(reference.display_name, "reference.wav");
    assert!(matches!(
        restored.viz().assistant.current_target(),
        Target::Reference(_)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A reference file that is gone surfaces as the panel's error, not as a
/// failed state load.
#[test]
fn a_missing_reference_file_is_an_error_not_a_failed_load() {
    let mut plugin = ResonanceMastering::new();
    let blob = br#"{"version":1,"params":{},"assistant":{"mode":"reference","genre":"rock","reference_path":"/nonexistent/gone.wav"}}"#;
    assert!(plugin.load_state(blob));
    let deadline = Instant::now() + Duration::from_secs(20);
    while plugin.viz().assistant.reference_error().is_none() {
        assert!(Instant::now() < deadline, "no error surfaced");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(plugin.viz().assistant.reference().is_none());
    // Reference mode with nothing loaded analyses against the genre.
    assert!(matches!(
        plugin.viz().assistant.current_target(),
        Target::Genre(Genre::Rock)
    ));
}
