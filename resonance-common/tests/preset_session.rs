//! The `com.resonance.preset-session` wire (slice P5): what the host makes
//! of a report. A malformed one is dropped, never read as "nothing loaded"
//! (which would clear the identity the host holds).

use resonance_common::preset_session::{ignored_params_json, parse_ignored_params, IdentityReport};

#[test]
fn reports_parse_to_an_identity_nothing_or_are_dropped() {
    let full = IdentityReport {
        source: "factory".into(),
        id: "warm".into(),
        name: "Warm".into(),
        modified: true,
    };
    assert_eq!(IdentityReport::parse_report(&full.to_json()), Some(Some(full)));
    assert_eq!(IdentityReport::parse_report("{}"), Some(None), "nothing loaded");
    for bad in ["", "not json", "[]", r#"{"id": "x"}"#, r#"{"source": 3, "name": "a"}"#] {
        assert_eq!(IdentityReport::parse_report(bad), None, "{bad:?} is dropped");
    }
}

#[test]
fn ignored_params_round_trip_and_garbage_is_none() {
    assert_eq!(parse_ignored_params(&ignored_params_json(&[3, 7])), vec![3, 7]);
    assert!(parse_ignored_params("nope").is_empty());
}
