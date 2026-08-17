//! Every plugin in the fleet reaches the shared preset surface (ba todo
//! #1358, audit findings X1 and X2).
//!
//! `resonance-plugin` shipping `PresetBank` / `PresetSession` /
//! `preset_bar` does nothing on its own — the finding was "0 of 11 plugins
//! can save a preset", and it stays true until each editor adopts them. So
//! this asserts the adoption itself rather than the machinery, which is
//! covered in `tests/presets.rs`.
//!
//! The sources are `include_str!`d rather than read at runtime, so the
//! guard is compiled from the same tree it checks and cannot drift from
//! what actually builds. It is a source-text check, which is a blunt
//! instrument — but the alternative is a dependency from this crate onto
//! all eleven plugins that depend on it, which is a cycle.
//!
//! **Adding a plugin crate means adding it to [`FLEET`].** A new plugin
//! that skips the preset surface is exactly the regression this exists to
//! catch, and it can only catch what it is told about.

/// Every plugin crate, with its `lib.rs` and the editor source that draws
/// its chrome.
const FLEET: &[(&str, &str, &str)] = &[
    (
        "resonance-amp",
        include_str!("../../plugins/resonance-amp/src/lib.rs"),
        include_str!("../../plugins/resonance-amp/src/editor/header.rs"),
    ),
    (
        "resonance-compressor",
        include_str!("../../plugins/resonance-compressor/src/lib.rs"),
        include_str!("../../plugins/resonance-compressor/src/editor/app.rs"),
    ),
    (
        "resonance-delay",
        include_str!("../../plugins/resonance-delay/src/lib.rs"),
        include_str!("../../plugins/resonance-delay/src/editor/app.rs"),
    ),
    (
        "resonance-drums",
        include_str!("../../plugins/resonance-drums/src/lib.rs"),
        include_str!("../../plugins/resonance-drums/src/editor/chrome.rs"),
    ),
    (
        "resonance-eq",
        include_str!("../../plugins/resonance-eq/src/lib.rs"),
        include_str!("../../plugins/resonance-eq/src/editor/app.rs"),
    ),
    (
        "resonance-gate",
        include_str!("../../plugins/resonance-gate/src/lib.rs"),
        include_str!("../../plugins/resonance-gate/src/editor/mod.rs"),
    ),
    (
        "resonance-granular-delay",
        include_str!("../../plugins/resonance-granular-delay/src/lib.rs"),
        include_str!("../../plugins/resonance-granular-delay/src/editor/app.rs"),
    ),
    (
        "resonance-ir",
        include_str!("../../plugins/resonance-ir/src/lib.rs"),
        include_str!("../../plugins/resonance-ir/src/editor/header.rs"),
    ),
    (
        "resonance-mastering",
        include_str!("../../plugins/resonance-mastering/src/lib.rs"),
        include_str!("../../plugins/resonance-mastering/src/editor/header.rs"),
    ),
    (
        "resonance-reverb",
        include_str!("../../plugins/resonance-reverb/src/lib.rs"),
        include_str!("../../plugins/resonance-reverb/src/editor/mod.rs"),
    ),
    (
        "resonance-wavetable",
        include_str!("../../plugins/resonance-wavetable/src/lib.rs"),
        include_str!("../../plugins/resonance-wavetable/src/editor/chrome.rs"),
    ),
];

/// The audit counted eleven plugins. If that number moves, the list above
/// has to move with it — otherwise this file silently stops covering the
/// newcomer.
#[test]
fn the_fleet_is_the_size_the_audit_counted() {
    assert_eq!(
        FLEET.len(),
        11,
        "the audit's X1 finding is about 11 plugins; add the new crate to FLEET"
    );
}

/// Every plugin hands its `PresetSession` to the bridge, which is what
/// makes the loaded-preset identity survive closing the window (finding
/// X2, the persistence half).
#[test]
fn every_plugin_publishes_its_preset_session_as_extra_state() {
    for (crate_name, lib_rs, _editor) in FLEET {
        assert!(
            lib_rs.contains("fn extra_state_saver"),
            "{crate_name}: no extra_state_saver, so the loaded preset cannot \
             survive the window closing"
        );
        assert!(
            lib_rs.contains("Some(self.presets.clone())"),
            "{crate_name}: extra_state_saver must return the shared \
             PresetSession. A plugin with a saver of its own chains it with \
             PresetSession::with_extra rather than choosing between the two"
        );
    }
}

/// Every editor draws the shared bar, rather than a private preset combo.
/// This is the half a user can see: save, rename, delete, and what is
/// loaded right now.
#[test]
fn every_editor_draws_the_shared_preset_bar() {
    for (crate_name, _lib_rs, editor) in FLEET {
        assert!(
            editor.contains("preset_bar("),
            "{crate_name}: its editor chrome does not draw preset_bar, so \
             this plugin still cannot save a preset"
        );
    }
}

/// No editor keeps its own idea of which preset is loaded.
///
/// Gate, granular and wavetable each grew a private index into their
/// factory list. All three were display-only, none could represent a user
/// preset, and none survived the window closing — `PresetSession` is the
/// one place that answer lives now (ba todo #1280).
#[test]
fn no_editor_tracks_the_loaded_preset_itself() {
    for (crate_name, _lib_rs, editor) in FLEET {
        for line in editor.lines() {
            // Skip prose: the doc comments here explain what these fields
            // *used* to be, which is worth keeping and is not a field.
            if line.trim_start().starts_with("//") {
                continue;
            }
            for stale in ["selected_preset", "preset_idx"] {
                assert!(
                    !line.contains(stale),
                    "{crate_name}: `{stale}` is a private copy of what \
                     PresetSession already tracks — found in `{}`",
                    line.trim()
                );
            }
        }
    }
}
