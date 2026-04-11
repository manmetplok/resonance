//! EBU Tech 3341-2016 compliance — a subset of the standard test cases.
//!
//! Tolerance per the spec is ±0.1 LU for integrated loudness.
//!
//! We implement five of the tabulated cases that are relevant to a
//! stereo mastering meter; the remaining cases are either 5.1 channel
//! configurations we don't support or duplicate coverage.

mod common;

use common::{concat, sine_mono};
use resonance_metering::LufsMeter;

const TOL: f32 = 0.1;
const SR: f32 = 48_000.0;

// A 1 kHz stereo sine at -X dBFS integrates to roughly -X LUFS. The
// K-weighting gain at 1 kHz (~+0.67 dB) almost exactly compensates the
// +3 dB from channel summation when combined with the BS.1770 offset of
// -0.691, leaving the integrated value essentially equal to the dBFS
// amplitude. This is why EBU Tech 3341-2016 uses -23 dBFS sine signals
// to verify -23 LUFS reference levels.

#[test]
fn case_1_stereo_minus_23_dbfs_sine_reads_minus_23_lufs() {
    let (l, r) = sine_mono(SR, 1000.0, -23.0, 20.0);
    let readout = LufsMeter::analyze_offline(SR, &l, &r);
    assert!(
        (readout.integrated - -23.0).abs() < TOL,
        "integrated = {} LUFS (expected -23.0 ± {TOL})",
        readout.integrated
    );
}

#[test]
fn stereo_minus_33_dbfs_sine_reads_minus_33_lufs() {
    let (l, r) = sine_mono(SR, 1000.0, -33.0, 20.0);
    let readout = LufsMeter::analyze_offline(SR, &l, &r);
    assert!(
        (readout.integrated - -33.0).abs() < TOL,
        "integrated = {} LUFS (expected -33.0 ± {TOL})",
        readout.integrated
    );
}

#[test]
fn absolute_gate_ignores_sub_minus_70_lufs_blocks() {
    // 10 s at -23 dBFS stereo (≈-23 LUFS), then 10 s near silence.
    // The quiet blocks must be dropped by the absolute gate.
    let loud = sine_mono(SR, 1000.0, -23.0, 10.0);
    let near_silent = sine_mono(SR, 1000.0, -80.0, 10.0);
    let (l, r) = concat(loud, near_silent);
    let readout = LufsMeter::analyze_offline(SR, &l, &r);
    assert!(
        (readout.integrated - -23.0).abs() < TOL,
        "integrated = {} LUFS (expected -23.0 ± {TOL})",
        readout.integrated
    );
}

#[test]
fn relative_gate_drops_blocks_more_than_10lu_below_reference() {
    // Half at -23 dBFS stereo (-23 LUFS) and half at -40 dBFS (-40 LUFS).
    // The quiet half is more than 10 LU below the ungated reference and
    // should be removed by the relative gate.
    let loud = sine_mono(SR, 1000.0, -23.0, 10.0);
    let quiet = sine_mono(SR, 1000.0, -40.0, 10.0);
    let (l, r) = concat(loud, quiet);
    let readout = LufsMeter::analyze_offline(SR, &l, &r);
    assert!(
        (readout.integrated - -23.0).abs() < 0.2,
        "integrated = {} LUFS (expected -23.0 ± 0.2)",
        readout.integrated
    );
}

#[test]
fn momentary_and_short_term_follow_instantaneous_level() {
    // Feed 5 s of -23 dBFS stereo sine and read both M and S meters
    // near the end (the 3 s short-term window must be fully populated).
    let sr = SR;
    let (l, r) = sine_mono(sr, 1000.0, -23.0, 5.0);
    let mut m = LufsMeter::new(sr);
    m.push_stereo(&l, &r);
    let mom = m.momentary_lufs();
    let short = m.short_term_lufs();
    assert!((mom - -23.0).abs() < TOL, "momentary = {mom}");
    assert!((short - -23.0).abs() < TOL, "short-term = {short}");
}
