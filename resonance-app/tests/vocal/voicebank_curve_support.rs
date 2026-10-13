//! Voicebank × expression-curve support matrix (doc #154, todo #333;
//! `vocal-mcp-control-and-quality.md` slices 2 and 4).
//!
//! [`BankProfile::curve_supported`] is the single source of truth that
//! both the vocal-roll Expression dock (the `n/a` rail badge) and the SVS
//! segment builder (cheap no-op for unsupported curves) consume. An
//! installed bank answers from its acoustic `dsconfig.yaml`
//! (`use_*_embed`); a known bank that is not installed answers from the
//! built-in record of the shipped configs. The two agree, so these tests
//! pin the same matrix with or without models on disk:
//!
//! - **PitchBend** is supported everywhere (a pre-synthesis f0 edit).
//! - **Dynamics** (energy) and **Breathiness** are supported nowhere: no
//!   shipped acoustic model has those inputs.
//! - **Tension** is accepted by Lilia and Meiji, not by TIGER.
//!
//! With `RESONANCE_SVS_MODELS_DIR` pointing at the real models, the last
//! two tests also check that the three shipped layouts register under
//! their fixed ids with the right speakers and curve flags.

use resonance_app::compose::vocal_svs::{
    BankProfile, CurveKind, VoicebankRegistry, VoicebankRoots,
};
use resonance_music_theory::VoicebankId;

const SHIPPED: [&str; 3] = ["tiger", "lilia", "meiji"];

#[test]
fn pitch_bend_supported_everywhere() {
    for vb in SHIPPED {
        assert!(
            BankProfile::not_installed(vb).curve_supported(CurveKind::PitchBend),
            "PitchBend must be supported on {vb}"
        );
    }
    // Even on a bank nothing is known about.
    assert!(BankProfile::not_installed("nobody").curve_supported(CurveKind::PitchBend));
}

/// No shipped acoustic model takes `energy` or `breathiness`, so neither
/// curve may claim to work — that was the overstatement the research
/// found.
#[test]
fn dynamics_and_breathiness_supported_nowhere() {
    for vb in SHIPPED {
        let bank = BankProfile::not_installed(vb);
        assert!(!bank.curve_supported(CurveKind::Dynamics), "{vb}");
        assert!(!bank.curve_supported(CurveKind::Breathiness), "{vb}");
    }
}

/// Tension keeps its historic per-voicebank behaviour: TIGER has no
/// `tension` input; Lilia and Meiji do.
#[test]
fn tension_matrix_per_voicebank() {
    assert!(!BankProfile::not_installed("tiger").curve_supported(CurveKind::Tension));
    assert!(BankProfile::not_installed("lilia").curve_supported(CurveKind::Tension));
    assert!(BankProfile::not_installed("meiji").curve_supported(CurveKind::Tension));
}

/// The models root a developer opted into, if any.
fn real_models() -> Option<VoicebankRegistry> {
    let dir = std::env::var_os("RESONANCE_SVS_MODELS_DIR").filter(|v| !v.is_empty())?;
    Some(VoicebankRegistry::scan(VoicebankRoots {
        dev_models: vec![dir.into()],
        ..VoicebankRoots::default()
    }))
}

/// The built-in record is the shipped configs, so an installed bank and a
/// missing one answer the same.
#[test]
fn installed_flags_match_the_shipped_record() {
    let Some(registry) = real_models() else {
        eprintln!("RESONANCE_SVS_MODELS_DIR unset; nothing installed to compare");
        return;
    };
    for vb in SHIPPED {
        let Some(bank) = registry.get(vb) else {
            continue;
        };
        let installed = registry.profile(&VoicebankId::new(vb));
        assert_eq!(
            installed.curve_support(),
            BankProfile::not_installed(vb).curve_support(),
            "{vb}"
        );
        assert_eq!(
            installed.missing_phonemes(),
            BankProfile::not_installed(vb).missing_phonemes(),
            "{vb}: the scanned inventory's gaps match the built-in record"
        );
        assert!(installed.english(), "{vb} sings English");
        assert!(bank.vocoder_config.is_some(), "{vb} has its vocoder");
    }
}

/// The real shipped banks register under their fixed ids, with TIGER
/// offering only its own seven modes and Meiji its four.
#[test]
fn real_banks_register_with_their_ids_and_speakers() {
    let Some(registry) = real_models() else {
        eprintln!("RESONANCE_SVS_MODELS_DIR unset; skipping");
        return;
    };
    let ids: Vec<String> = registry.installed().map(|b| b.id.clone()).collect();
    eprintln!("installed: {ids:?}");
    let speakers = |id: &str| -> Vec<String> {
        registry
            .profile(&VoicebankId::new(id))
            .speakers()
            .into_iter()
            .map(|s| s.id)
            .collect()
    };
    if registry.get("tiger").is_some() {
        assert_eq!(
            speakers("tiger"),
            ["glam", "fresh", "disco", "royal", "electric", "mystic", "vinyl"]
        );
        let tiger = registry.profile(&VoicebankId::new("tiger"));
        let target = tiger
            .render_target(Some("vinyl"), resonance_music_theory::VoiceType::Alto, None)
            .expect("TIGER renders");
        assert!(target.speaker.unwrap().ends_with("tiger_vinyl"));
    }
    if registry.get("lilia").is_some() {
        assert!(speakers("lilia").is_empty(), "Lilia is single-voice");
        let lilia = registry.profile(&VoicebankId::new("lilia"));
        assert!(lilia.dictionary().is_some(), "Lilia ships dsdict-en.yaml");
        assert_eq!(
            lilia.language_id("ah"),
            None,
            "Lilia takes no language input"
        );
    }
    if registry.get("meiji").is_some() {
        assert_eq!(
            speakers("meiji"),
            ["standard", "hunter", "lilith", "phantom"]
        );
        let meiji = registry.profile(&VoicebankId::new("meiji"));
        assert_eq!(meiji.phoneme_name("ah"), "en/ah");
        assert_eq!(meiji.phoneme_name("hh"), "hh", "Meiji's universal bucket");
        assert_eq!(meiji.language_id("ah"), Some(3), "Meiji's id for en");
        assert_eq!(meiji.language_id("AP"), Some(0));
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

/// The LUNAI English singers as they unpack from their v170 zips, when a
/// developer points `RESONANCE_VOICEBANK_DIR` at a folder holding them
/// (opt-in, like `RESONANCE_SVS_MODELS_DIR` above; the hermetic fixture
/// version is `voicebank_registry::lunai_v170_banks_register_…`): each
/// registers under its folder id with its modes, sings English through
/// `en/` and language id 3, has all three of its own models and its
/// bundled vocoder, and loads its 384-wide speaker embeddings.
#[test]
fn real_lunai_banks_register() {
    let Some(dir) = std::env::var_os("RESONANCE_VOICEBANK_DIR").filter(|v| !v.is_empty()) else {
        eprintln!("RESONANCE_VOICEBANK_DIR unset; skipping");
        return;
    };
    let registry = VoicebankRegistry::scan(VoicebankRoots {
        banks: Some(dir.into()),
        ..VoicebankRoots::default()
    });
    for (id, modes) in [
        ("katyusha", 4),
        ("keiro_revenant", 10),
        ("liam_thorne", 10),
        ("allen_crow", 5),
    ] {
        let Some(bank) = registry.get(id) else {
            eprintln!("{id} not installed there; skipped");
            continue;
        };
        let profile = registry.profile(&VoicebankId::new(id));
        assert!(profile.english(), "{id}");
        assert!(profile.missing_phonemes().is_empty(), "{id}");
        assert_eq!(profile.speakers().len(), modes, "{id}");
        assert_eq!(profile.phoneme_name("ah"), "en/ah", "{id}");
        assert_eq!(profile.language_id("ah"), Some(3), "{id}");
        assert!(profile.dictionary().is_some(), "{id}");
        let models = profile.bank_models();
        assert!(
            models.duration.is_some() && models.pitch.is_some() && models.variance.is_some(),
            "{id}"
        );
        assert!(bank.vocoder_config.is_some(), "{id} has its vocoder");
        let embeds = resonance_svs::pipeline::speaker_embeddings(&bank.acoustic_config)
            .unwrap_or_else(|e| panic!("{id}: {e:#}"));
        assert_eq!(embeds.len(), modes, "{id}");
        assert!(embeds.values().all(|e| e.len() == 384), "{id}");
    }
}
