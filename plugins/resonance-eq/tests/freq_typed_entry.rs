//! FU-P2b: a band's `freq` param displays "1.00 kHz" above 1 kHz
//! (`format_hz`), but until now had no `string_to_value` of its own —
//! `Param::parse`'s fallback only strips the declared unit (" Hz"), so
//! it read "1200 Hz" but choked on the "kHz" form its own display
//! produces above 1 kHz, and any shorthand a user actually types.
use resonance_eq::params::EqParams;
use resonance_plugin::Param;

const EPS: f64 = 1e-3;

fn assert_parses_to(text: &str, expected: f64) {
    let params = EqParams::default();
    let freq = &params.bands[0].freq;
    let parsed = freq.parse(text);
    assert!(
        parsed.is_some_and(|v| (v - expected).abs() < EPS),
        "parsing {text:?}: expected {expected}, got {parsed:?}"
    );
}

#[test]
fn a_plain_number_is_hz() {
    assert_parses_to("1200", 1200.0);
}

#[test]
fn an_explicit_hz_unit_is_hz() {
    assert_parses_to("440 Hz", 440.0);
    assert_parses_to("440Hz", 440.0);
}

#[test]
fn khz_shorthand_and_full_unit_both_scale_to_hz() {
    assert_parses_to("1.2k", 1200.0);
    assert_parses_to("1.2K", 1200.0);
    assert_parses_to("1.2 kHz", 1200.0);
    assert_parses_to("1.2kHz", 1200.0);
}

#[test]
fn a_typed_value_reads_back_through_the_param_as_its_own_display_produces() {
    // What a user would actually do: type what the control shows, and
    // land exactly where the display says — the round trip this whole
    // follow-up exists for.
    let params = EqParams::default();
    let freq = &params.bands[0].freq;
    freq.set_plain(9000.0);
    let shown = freq.display(freq.get_plain());
    assert_eq!(shown, "9.00 kHz");
    assert_eq!(freq.parse(&shown), Some(9000.0));
}

#[test]
fn garbage_is_refused() {
    let params = EqParams::default();
    let freq = &params.bands[0].freq;
    assert_eq!(freq.parse("not a number"), None);
}
