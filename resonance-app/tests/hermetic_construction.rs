//! `Resonance::new_for_test()` must not touch the machine (ba doc #285, todo
//! #1365).
//!
//! `Resonance::new()` opens a real PipeWire output stream, probes audio devices
//! with `pw-metadata`/`pactl` subprocesses, scans `~/.clap` and `/usr/lib/clap`
//! and loads every plugin it finds there, and reads the user's recents,
//! settings and saved track presets. A test built on that inherits all of it —
//! which is how a third-party plugin's broken teardown came to abort test
//! processes at random (doc #285 §1). These tests pin the hermetic constructor
//! so that cannot come back.

use resonance_app::Resonance;

/// The give-away that a real engine was booted is the startup burst of
/// commands `new()` sends: device lists and, critically, `ScanPlugins` — the
/// one that dlopens every `.clap` on the machine. Construction must emit
/// nothing at all.
#[test]
fn construction_emits_no_engine_commands() {
    let (_app, _task, cmd_rx) = Resonance::new_for_test_with_capture();

    let queued: Vec<_> = cmd_rx.try_iter().collect();
    assert!(
        queued.is_empty(),
        "new_for_test() must not talk to the engine, but queued: {queued:?}"
    );
}

/// No read of `~/.config`: recents, saved track presets and settings all start
/// empty, so a test's result cannot depend on the developer's config — nor be
/// changed by another test that writes there.
#[test]
fn reads_none_of_the_users_config() {
    let (app, _task) = Resonance::new_for_test();

    assert!(
        app.test_recent_projects().is_empty(),
        "recent projects must start empty, got {:?}",
        app.test_recent_projects()
    );
    assert!(
        app.test_user_presets().is_empty(),
        "user presets must start empty, got {} entries",
        app.test_user_presets().len()
    );
    assert_eq!(
        app.test_settings(),
        &resonance_app::settings::AppSettings::default(),
        "settings must start at their defaults"
    );
}

/// The bundled device definitions are embedded at compile time rather than read
/// from disk, so they are hermetic and must still be there — a constructor that
/// dropped them would quietly change what the External Instrument inspector can
/// offer.
#[test]
fn keeps_the_compiled_in_device_definitions() {
    let (app, _task) = Resonance::new_for_test();

    assert!(
        !app.test_device_definitions().is_empty(),
        "bundled device definitions are embedded, not read from disk, and \
         should survive hermetic construction"
    );
}
