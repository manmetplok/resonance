//! `util::format_db_signed` — the one dB formatter for the view layer
//! (code review UX-17). Before this, `util::format_db` gave `"0.0"` /
//! `"-inf"` (the fader label) while sends, bus members, automation, clip
//! gain and the reference offset/trim each hand-rolled `"{:+.1} dB"` —
//! three different formats for the same value. This locks the merged
//! behaviour: always signed, the floor spelled out as `"−∞"` rather than
//! a literal `"-60.0"`, near-zero collapsed to `"+0.0"` so a `-0.0` never
//! prints, and the unit droppable for the fader label while the sign
//! stays.

use resonance_app::util::format_db_signed;

#[test]
fn signed_with_unit() {
    assert_eq!(format_db_signed(3.0, true), "+3.0 dB");
    assert_eq!(format_db_signed(-6.0, true), "-6.0 dB");
    assert_eq!(format_db_signed(12.34, true), "+12.3 dB");
}

#[test]
fn near_zero_collapses_to_positive_zero() {
    assert_eq!(format_db_signed(0.0, true), "+0.0 dB");
    assert_eq!(format_db_signed(-0.01, true), "+0.0 dB");
    assert_eq!(format_db_signed(0.04, true), "+0.0 dB");
}

#[test]
fn floor_reads_as_signed_infinity() {
    assert_eq!(format_db_signed(-60.0, true), "\u{2212}\u{221e} dB");
    assert_eq!(format_db_signed(-90.0, true), "\u{2212}\u{221e} dB");
}

#[test]
fn fader_drops_the_unit_but_keeps_the_sign() {
    assert_eq!(format_db_signed(-6.0, false), "-6.0");
    assert_eq!(format_db_signed(0.0, false), "+0.0");
    assert_eq!(format_db_signed(-60.0, false), "\u{2212}\u{221e}");
}
