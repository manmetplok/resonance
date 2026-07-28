//! Unit tests for the pure pieces of the PipeWire graph-rate assertion
//! (ba todo #1101): the assert-rate candidate choice and the
//! `pw-metadata` output parsing the engage/verify round trip relies on.
//!
//! The metadata write/verify/clear itself talks to a live PipeWire
//! daemon and is exercised manually (`pw-metadata -n settings`); these
//! tests pin the decision logic around it.

use resonance_audio::__test_support::{
    choose_assert_rate, parse_pw_metadata_value, CANONICAL_RATE,
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
