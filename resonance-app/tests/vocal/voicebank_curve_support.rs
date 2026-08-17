//! Voicebank × expression-curve support matrix (doc #154, todo #333).
//!
//! `curve_supported` is the single source of truth that both the
//! vocal-roll Expression dock (the `n/a` rail badge) and the SVS
//! segment builder (cheap no-op for unsupported curves) consume. These
//! tests pin the matrix so a regression in either consumer is caught:
//!
//! - **Dynamics** and **PitchBend** are supported on every voicebank.
//! - **Tension** and **Breathiness** are TIGER-unsupported but accepted
//!   by Lilia and Meiji.
//!
//! In particular, `Tension` must keep its historic behaviour exactly
//! (TIGER = false; Lilia/Meiji = true) because the tension curve builder
//! migrated from the old `voicebank_supports_tension` predicate.

use resonance_app::compose::vocal_svs::{curve_supported, CurveKind};
use resonance_music_theory::VocalVoicebank;

/// Universally-supported curves are accepted on every voicebank.
#[test]
fn dynamics_and_pitch_bend_supported_everywhere() {
    for vb in VocalVoicebank::ALL.iter().copied() {
        assert!(
            curve_supported(vb, CurveKind::Dynamics),
            "Dynamics must be supported on {vb:?}"
        );
        assert!(
            curve_supported(vb, CurveKind::PitchBend),
            "PitchBend must be supported on {vb:?}"
        );
    }
}

/// Tension keeps its historic per-voicebank behaviour exactly: TIGER has
/// no `tension` input; Lilia and Meiji do.
#[test]
fn tension_matrix_matches_legacy_behaviour() {
    assert!(!curve_supported(VocalVoicebank::Tiger, CurveKind::Tension));
    assert!(curve_supported(VocalVoicebank::Lilia, CurveKind::Tension));
    assert!(curve_supported(VocalVoicebank::Meiji, CurveKind::Tension));
}

/// Breathiness tracks the same capability split as tension: TIGER's
/// acoustic model exposes no `breathiness` input; Lilia and Meiji do.
#[test]
fn breathiness_matrix_per_voicebank() {
    assert!(!curve_supported(VocalVoicebank::Tiger, CurveKind::Breathiness));
    assert!(curve_supported(VocalVoicebank::Lilia, CurveKind::Breathiness));
    assert!(curve_supported(VocalVoicebank::Meiji, CurveKind::Breathiness));
}

/// `CurveKind::ALL` covers exactly the four curve kinds, each distinct,
/// so consumers that iterate the rail never miss or duplicate a curve.
#[test]
fn curve_kind_all_is_exhaustive_and_distinct() {
    assert_eq!(CurveKind::ALL.len(), 4);
    for (i, a) in CurveKind::ALL.iter().enumerate() {
        for b in CurveKind::ALL.iter().skip(i + 1) {
            assert_ne!(a, b, "CurveKind::ALL must not contain duplicates");
        }
    }
    // Spot-check membership.
    assert!(CurveKind::ALL.contains(&CurveKind::Dynamics));
    assert!(CurveKind::ALL.contains(&CurveKind::Tension));
    assert!(CurveKind::ALL.contains(&CurveKind::Breathiness));
    assert!(CurveKind::ALL.contains(&CurveKind::PitchBend));
}
