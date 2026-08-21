//! Unit tests for the pure pieces of the PipeWire graph-rate assertion
//! (ba todo #1101): the assert-rate candidate choice and the
//! `pw-metadata` output parsing the engage/verify round trip relies on.
//!
//! The metadata write/verify/clear itself talks to a live PipeWire
//! daemon and is exercised manually (`pw-metadata -n settings`); these
//! tests pin the decision logic around it.

use resonance_audio::__test_support::{
    choose_assert_rate, force_release_target, needs_reassert, parse_pw_metadata_value,
    reassert_source_key, CANONICAL_RATE,
};

// --- choose_assert_rate -------------------------------------------------

#[test]
fn prefers_canonical_48k_when_device_supports_it() {
    // Even when another client dragged the graph to 44.1k, the engine
    // asserts 48k on a device that can do it.
    let rate = choose_assert_rate(Some(44_100), Some(44_100), |r| r == 48_000 || r == 44_100);
    assert_eq!(rate, Some(CANONICAL_RATE));
}

#[test]
fn falls_back_to_graph_rate_when_48k_unsupported() {
    let rate = choose_assert_rate(Some(44_100), Some(96_000), |r| r != 48_000);
    assert_eq!(rate, Some(44_100));
}

#[test]
fn falls_back_to_sink_rate_when_graph_rate_unknown() {
    let rate = choose_assert_rate(None, Some(96_000), |r| r == 96_000);
    assert_eq!(rate, Some(96_000));
}

#[test]
fn none_when_no_candidate_supported() {
    // No PipeWire and a device that supports none of the candidates:
    // the caller keeps today's follow-the-graph behaviour.
    let rate = choose_assert_rate(None, None, |r| r == 44_100);
    assert_eq!(rate, None);
}

#[test]
fn canonical_rate_still_wins_when_graph_already_there() {
    // Asserting the rate the graph already runs at is deliberate: it
    // pins the graph so a later client can't drag it away mid-session.
    let rate = choose_assert_rate(Some(48_000), None, |r| r == 48_000);
    assert_eq!(rate, Some(CANONICAL_RATE));
}

// --- parse_pw_metadata_value --------------------------------------------

#[test]
fn parses_settings_update_line() {
    let out = "Found \"settings\" metadata 31\nupdate: id:0 key:'clock.force-rate' value:'48000' type:''\n";
    assert_eq!(parse_pw_metadata_value(out), Some("48000"));
}

#[test]
fn parses_cleared_force_rate() {
    let out = "update: id:0 key:'clock.force-rate' value:'0' type:''";
    assert_eq!(parse_pw_metadata_value(out), Some("0"));
}

#[test]
fn parses_first_value_when_multiple_lines() {
    let out = "update: id:0 key:'clock.rate' value:'48000' type:''\n\
               update: id:0 key:'clock.force-rate' value:'44100' type:''";
    assert_eq!(parse_pw_metadata_value(out), Some("48000"));
}

#[test]
fn rejects_output_without_value_field() {
    assert_eq!(parse_pw_metadata_value("Found \"settings\" metadata 31\n"), None);
    assert_eq!(parse_pw_metadata_value(""), None);
}

#[test]
fn rejects_unterminated_value() {
    assert_eq!(parse_pw_metadata_value("update: id:0 key:'x' value:'48000"), None);
}

#[test]
fn parses_non_numeric_value_as_string() {
    // The array-typed settings entries also flow through this parser.
    let out = "update: id:0 key:'clock.allowed-rates' value:'[ 48000 ]' type:''";
    assert_eq!(parse_pw_metadata_value(out), Some("[ 48000 ]"));
}

// --- force_release_target ------------------------------------------------

#[test]
fn release_restores_the_unforced_state_we_found() {
    // The common case: nothing was forced before us, so letting go
    // writes 0 and hands the graph back to its configured rate.
    assert_eq!(force_release_target(Some(48_000), 48_000, 0), Some(0));
}

#[test]
fn release_restores_a_pre_existing_foreign_force() {
    // Someone had forced 96k before the DAW started; we forced 48k.
    // Releasing must put *their* value back, not clear the force.
    assert_eq!(
        force_release_target(Some(48_000), 48_000, 96_000),
        Some(96_000)
    );
}

#[test]
fn release_leaves_an_identical_pre_existing_force_alone() {
    // The user already had our exact rate forced. `current == ours`
    // reads as "still ours", but clearing it would take away a setting
    // we never made — write nothing.
    assert_eq!(force_release_target(Some(48_000), 48_000, 48_000), None);
}

#[test]
fn release_leaves_a_later_foreign_force_alone() {
    // Another client re-forced the graph after we engaged; clobbering
    // it on our way out would be rude and wrong.
    assert_eq!(force_release_target(Some(44_100), 48_000, 0), None);
}

#[test]
fn release_writes_nothing_when_the_metadata_is_unreadable() {
    // A `None` readback means pw-metadata is gone or its output no
    // longer parses; we can't tell whose value is in there.
    assert_eq!(force_release_target(None, 48_000, 0), None);
}

// --- needs_reassert ------------------------------------------------------

#[test]
fn first_input_stream_reasserts() {
    assert!(needs_reassert(None, Some("alsa_input.usb-Focusrite")));
    assert!(needs_reassert(None, None));
}

#[test]
fn rebuilding_the_same_source_does_not_respawn_pw_metadata() {
    // Count-in -> record rebuilds the input stream on the engine
    // thread with the same device; a subprocess there is a dropout
    // risk and cannot change the graph rate anyway.
    let last = reassert_source_key(Some("alsa_input.usb-Focusrite"));
    assert!(!needs_reassert(
        Some(&last),
        Some("alsa_input.usb-Focusrite")
    ));
}

#[test]
fn switching_source_reasserts() {
    let last = reassert_source_key(Some("alsa_input.usb-Focusrite"));
    assert!(needs_reassert(Some(&last), Some("alsa_input.pci-hdmi")));
    // ...including switching back to the default device.
    assert!(needs_reassert(Some(&last), None));
}

#[test]
fn default_source_is_distinguishable_from_never_asserted() {
    // `None` (default device) must not collide with "no re-assert has
    // happened yet", or the very first build would be skipped.
    let last = reassert_source_key(None);
    assert!(!needs_reassert(Some(&last), None));
    assert!(needs_reassert(None, None));
}
