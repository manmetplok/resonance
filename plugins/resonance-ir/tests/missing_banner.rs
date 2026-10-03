//! A failed IR load draws as a banner, not as a filename (code review
//! PUX-09). `loader.rs::load_into`'s only error channel is `ir_name`
//! itself (`"Error: {e}"`); this proves the editor turns that into a
//! WARM banner with the real message and a `Locate…` action, and stops
//! showing it as if it were a loaded file's name.
#![cfg(feature = "editor")]

use resonance_ir::editor::headless_editor;
use resonance_ir::ResonanceIr;
use resonance_plugin::ResonancePlugin;

const SIZE: (f32, f32) = (880.0, 540.0);

#[test]
fn no_error_shows_the_empty_state_not_a_banner() {
    let plugin = ResonanceIr::new();
    let mut editor = headless_editor(&plugin, SIZE);
    let probe = editor.settled();

    assert!(probe.shows("(no IR loaded)"));
    assert!(!probe.shows("⚠ Could not load the impulse response"));
}

#[test]
fn a_load_error_draws_the_warm_banner_with_its_message_and_a_locate_button() {
    let plugin = ResonanceIr::new();
    plugin.test_set_ir_name("Error: No such file or directory (os error 2)");
    let mut editor = headless_editor(&plugin, SIZE);
    let probe = editor.settled();

    assert!(
        probe.shows("⚠ Could not load the impulse response"),
        "expected the banner's headline; painted texts: {:#?}",
        probe.texts
    );
    assert!(
        probe.shows("No such file or directory (os error 2)"),
        "expected the banner to show the error with the \"Error: \" prefix stripped"
    );
    assert!(
        probe.shows("Locate…"),
        "expected a Locate… action on the banner"
    );

    // The filename slot in the header must not show the raw "Error: "
    // string as though it were a loaded file's name.
    assert!(!probe.shows("Error: No such file or directory (os error 2)"));
}

#[test]
fn a_real_filename_that_happens_to_be_empty_is_still_the_empty_state() {
    // Guards `load_error`'s prefix match against false positives: an
    // empty ir_name (nothing loaded, not an error) must not trip it.
    let plugin = ResonanceIr::new();
    let mut editor = headless_editor(&plugin, SIZE);
    let probe = editor.settled();
    assert!(!probe.shows("⚠ Could not load the impulse response"));
}
