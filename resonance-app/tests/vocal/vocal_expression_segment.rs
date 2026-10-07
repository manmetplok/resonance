//! Feeding edited expression curves into `DsSegment` building (todo #334).
//!
//! Covers the contract from doc #154 / epic #17: dynamics (→ energy),
//! tension and breathiness are honoured only on voicebanks whose acoustic
//! model accepts them — no shipped bank takes energy or breathiness,
//! Lilia and Meiji take tension; a pitch-bend overlay shifts the f0 curve;
//! and a curve left in its `Auto` state leaves the auto-derived result
//! untouched (baseline only).

use resonance_app::compose::vocal_svs::{build_segment, CurveKind};
use resonance_app::compose::ExpressionCurves;
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::g2p::{
    AssignedSyllable, G2pSource, PhonemeProvenance, SyllableStress,
};
use resonance_app::compose::vocal_svs::BankProfile;
use resonance_music_theory::{VocalParams, VoicebankId};

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
const BPM: f32 = 120.0;

fn params(vb: &str) -> VocalParams {
    VocalParams {
        voicebank: VoicebankId::new(vb),
        // A non-zero baseline tension so the auto-derived tension curve is
        // present (non-default) on supporting voicebanks.
        tension: 0.3,
        ..VocalParams::default()
    }
}

fn syl(phonemes: &[&'static str], syllable_index: usize) -> AssignedSyllable {
    AssignedSyllable {
        label: phonemes.concat(),
        phonemes: phonemes.to_vec(),
        is_slur: false,
        is_word_end: true,
        syllable_index,
        stress: SyllableStress::None,
        provenance: PhonemeProvenance::Auto,
        source: G2pSource::Cmu,
        breath_before: false,
        syllable_warning: false,
    }
}

/// One sustained note — long enough that the f0 grid has plenty of frames.
fn one_note() -> (Vec<MidiNote>, Vec<AssignedSyllable>) {
    let notes = vec![MidiNote {
        note: 64,
        velocity: 0.8,
        start_tick: 0,
        duration_ticks: TICKS_PER_QUARTER_NOTE * 2,
    }];
    let assigned = vec![syl(&["s", "ow"], 0)];
    (notes, assigned)
}

/// A bundle with one curve carrying a single, flat overlay breakpoint at
/// `value` (held constant across the whole clip).
fn flat_overlay(kind: CurveKind, value: f32) -> ExpressionCurves {
    let mut curves = ExpressionCurves::default();
    curves.curve_mut(kind).add_breakpoint(0.0, value);
    curves
}

fn build(vb: &str, curves: &ExpressionCurves) -> resonance_svs::ds::DsSegment {
    let (notes, assigned) = one_note();
    build_segment(&notes, &params(vb), &assigned, curves, TPQ, BPM, &BankProfile::not_installed(vb))
}

// ---------------------------------------------------------------------------
// Baseline-only (Auto) leaves the auto-derived result untouched
// ---------------------------------------------------------------------------

#[test]
fn auto_curves_leave_energy_and_breathiness_default() {
    // No overlays: dynamics/breathiness contribute nothing, so their
    // per-frame curves stay empty (the pre-#334 default).
    let seg = build("lilia", &ExpressionCurves::default());
    assert!(seg.energy.is_empty(), "energy should be default when Auto");
    assert!(
        seg.breathiness.is_empty(),
        "breathiness should be default when Auto"
    );
}

#[test]
fn auto_tension_matches_auto_derived_baseline() {
    // The tension field for an Auto curve must equal the builder's own
    // auto-derived tension (driven by params.tension), unchanged by the
    // overlay feature.
    let seg_default = build("lilia", &ExpressionCurves::default());
    // Lilia accepts tension and params.tension is non-zero, so it's present.
    assert!(
        !seg_default.tension.is_empty(),
        "auto tension should be present on a supporting voicebank"
    );

    // An edited *dynamics* overlay must not perturb the tension or f0 curve
    // — the curves stay frame-independent.
    let dyn_edited = build("lilia", &flat_overlay(CurveKind::Dynamics, 0.9));
    assert_eq!(
        seg_default.tension.samples, dyn_edited.tension.samples,
        "editing dynamics must not change tension"
    );
    assert_eq!(
        seg_default.f0.samples, dyn_edited.f0.samples,
        "editing dynamics must not change f0"
    );
}

// ---------------------------------------------------------------------------
// Dynamics / Breathiness: no shipped acoustic model takes them
// ---------------------------------------------------------------------------

#[test]
fn dynamics_overlay_is_a_no_op_on_every_shipped_voicebank() {
    // No shipped bank's acoustic config sets `use_energy_embed`, so an
    // edited dynamics overlay must not emit an energy curve the model
    // would ignore (or, worse, claim to honour).
    for vb in ["tiger", "lilia", "meiji"] {
        let seg = build(vb, &flat_overlay(CurveKind::Dynamics, 1.0));
        assert!(seg.energy.is_empty(), "{vb:?} must leave energy default");
    }
}

#[test]
fn breathiness_overlay_is_a_no_op_on_every_shipped_voicebank() {
    for vb in ["tiger", "lilia", "meiji"] {
        let seg = build(vb, &flat_overlay(CurveKind::Breathiness, 1.0));
        assert!(
            seg.breathiness.is_empty(),
            "{vb:?} must leave breathiness default"
        );
    }
}

// ---------------------------------------------------------------------------
// Tension honoured only on supporting voicebanks; value mapping
// ---------------------------------------------------------------------------

#[test]
fn tension_overlay_honoured_on_supporting_voicebank() {
    for vb in ["lilia", "meiji"] {
        let seg = build(vb, &flat_overlay(CurveKind::Tension, 1.0));
        assert!(!seg.tension.is_empty(), "{vb:?}");
        assert_eq!(seg.tension.samples.len(), seg.f0.samples.len());
        // The edited overlay replaces the auto-derived tension; a flat 1.0
        // envelope maps to the model's +1.0 extreme.
        for v in &seg.tension.samples {
            assert!((*v - 1.0).abs() < 1e-6, "tension frame {v} should map to +1.0");
        }
    }
}

#[test]
fn tension_tracks_overlay_value() {
    // Midpoint of the envelope (0.5) maps to model-neutral (0.0); the same
    // mapping serves energy and breathiness on a bank that takes them.
    let seg = build("lilia", &flat_overlay(CurveKind::Tension, 0.5));
    assert!(!seg.tension.is_empty());
    for v in &seg.tension.samples {
        assert!((*v).abs() < 1e-6, "tension frame {v} should map to 0.0");
    }
}

#[test]
fn unsupported_tension_overlay_leaves_field_default() {
    // TIGER's acoustic model has no tension input, so even an edited
    // overlay is a clean no-op (field stays default), not an error.
    let seg = build("tiger", &flat_overlay(CurveKind::Tension, 1.0));
    assert!(
        seg.tension.is_empty(),
        "TIGER must leave tension default even when the overlay is edited"
    );
}

// ---------------------------------------------------------------------------
// Pitch bend → additive cents offset on f0
// ---------------------------------------------------------------------------

#[test]
fn pitch_bend_overlay_shifts_f0_up() {
    let baseline = build("lilia", &ExpressionCurves::default());
    // +50 cents constant: every voiced frame should rise by ~2^(50/1200).
    let bent = build("lilia", &flat_overlay(CurveKind::PitchBend, 50.0));

    assert_eq!(baseline.f0.samples.len(), bent.f0.samples.len());
    let factor = 2.0_f64.powf(50.0 / 1200.0);
    let mut shifted = 0;
    for (b, e) in baseline.f0.samples.iter().zip(&bent.f0.samples) {
        if *b > 0.0 {
            assert!(*e > *b, "bent f0 {e} should exceed baseline {b}");
            assert!((e / b - factor).abs() < 1e-6, "expected ~{factor}x shift");
            shifted += 1;
        }
    }
    assert!(shifted > 0, "expected some voiced frames to shift");
}

#[test]
fn pitch_bend_overlay_shifts_f0_down() {
    let baseline = build("lilia", &ExpressionCurves::default());
    let bent = build("lilia", &flat_overlay(CurveKind::PitchBend, -50.0));
    for (b, e) in baseline.f0.samples.iter().zip(&bent.f0.samples) {
        if *b > 0.0 {
            assert!(*e < *b, "negative bend should lower f0");
        }
    }
}

#[test]
fn auto_pitch_bend_leaves_f0_identical() {
    // PitchBend is universally supported, but an Auto (un-edited) curve
    // must leave the f0 curve byte-for-byte identical.
    let baseline = build("tiger", &ExpressionCurves::default());
    let mut curves = ExpressionCurves::default();
    // Touch a *different* curve so the bundle isn't all-default but pitch
    // bend stays Auto.
    curves.curve_mut(CurveKind::Dynamics).add_breakpoint(0.0, 0.5);
    let other = build("tiger", &curves);
    assert_eq!(
        baseline.f0.samples, other.f0.samples,
        "an Auto pitch-bend curve must not touch f0"
    );
}
