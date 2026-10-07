//! Voicebank × expression-curve support matrix (doc #154, todo #333;
//! `vocal-mcp-control-and-quality.md` slice 2).
//!
//! `curve_supported` is the single source of truth that both the
//! vocal-roll Expression dock (the `n/a` rail badge) and the SVS segment
//! builder (cheap no-op for unsupported curves) consume. It answers from
//! each bank's acoustic `dsconfig.yaml` (`use_*_embed`), falling back to a
//! table equal to the shipped configs when the bank is not installed. The
//! two agree, so these tests pin the same matrix with or without models
//! on disk:
//!
//! - **PitchBend** is supported everywhere (a pre-synthesis f0 edit).
//! - **Dynamics** (energy) and **Breathiness** are supported nowhere: no
//!   shipped acoustic model has those inputs.
//! - **Tension** is accepted by Lilia and Meiji, not by TIGER.

use resonance_app::compose::vocal_svs::paths::{
    bank_curve_support, curve_supported, shipped_curve_support, CurveKind,
};
use resonance_music_theory::VocalVoicebank;

#[test]
fn pitch_bend_supported_everywhere() {
    for vb in VocalVoicebank::ALL.iter().copied() {
        assert!(
            curve_supported(vb, CurveKind::PitchBend),
            "PitchBend must be supported on {vb:?}"
        );
    }
}

/// No shipped acoustic model takes `energy` or `breathiness`, so neither
/// curve may claim to work — that was the overstatement the research
/// found.
#[test]
fn dynamics_and_breathiness_supported_nowhere() {
    for vb in VocalVoicebank::ALL.iter().copied() {
        assert!(
            !curve_supported(vb, CurveKind::Dynamics),
            "Dynamics must not be supported on {vb:?}"
        );
        assert!(
            !curve_supported(vb, CurveKind::Breathiness),
            "Breathiness must not be supported on {vb:?}"
        );
    }
}

/// Tension keeps its historic per-voicebank behaviour: TIGER has no
/// `tension` input; Lilia and Meiji do.
#[test]
fn tension_matrix_per_voicebank() {
    assert!(!curve_supported(VocalVoicebank::Tiger, CurveKind::Tension));
    assert!(curve_supported(VocalVoicebank::Lilia, CurveKind::Tension));
    assert!(curve_supported(VocalVoicebank::Meiji, CurveKind::Tension));
}

/// The fallback table is the shipped configs, so an installed bank and a
/// missing one answer the same. (With models installed this compares the
/// parsed YAML against the table; without, it is trivially the table.)
#[test]
fn installed_flags_match_the_shipped_fallback() {
    for vb in VocalVoicebank::ALL.iter().copied() {
        assert_eq!(bank_curve_support(vb), shipped_curve_support(vb), "{vb:?}");
    }
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
