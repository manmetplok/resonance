//! `vocal.voicebanks`: the voicebank registry in wire shape
//! (vocal-mcp-control-and-quality.md §4.5).

use resonance_control::methods::vocal as wire;
use resonance_music_theory::VoicebankId;

use crate::compose::vocal_svs::registry::{ComfortableRange, VoicebankRegistry};
use crate::compose::vocal_svs::{BankProfile, CurveKind, PhonemeFate, VoicebankPhonemes};

/// Every installed bank, by id. `available` stays empty until the plok.org
/// index lists voicebanks (spec slice 7).
pub(in crate::update::control) fn voicebank_list(
    registry: &VoicebankRegistry,
) -> wire::VoicebankList {
    wire::VoicebankList {
        installed: registry
            .installed()
            .map(|bank| voicebank_entry(&registry.profile(&VoicebankId::new(&bank.id))))
            .collect(),
        available: Vec::new(),
    }
}

fn voicebank_entry(profile: &BankProfile) -> wire::VoicebankEntry {
    let bank = profile.installed();
    let builtin = profile.builtin();
    let phonemes = VoicebankPhonemes::new(profile);
    let missing = profile.missing_phonemes();
    let substitutions = missing
        .iter()
        .filter_map(|&ph| match phonemes.resolve(ph) {
            PhonemeFate::Substituted(sub) => Some(wire::PhonemeSubstitution {
                phoneme: ph.to_owned(),
                sung_as: sub.to_owned(),
            }),
            PhonemeFate::Direct | PhonemeFate::Unsupported => None,
        })
        .collect();
    let speakers = profile
        .speakers()
        .into_iter()
        .map(|s| wire::VoicebankSpeaker {
            comfortable_range: range_view(profile.comfortable_range(Some(&s.id))),
            id: s.id,
            name: s.name,
        })
        .collect();
    wire::VoicebankEntry {
        id: profile.id().to_string(),
        name: profile.name(),
        version: bank.and_then(|b| b.version.clone()),
        english: profile.english(),
        speakers,
        default_speaker: profile.default_speaker(resonance_music_theory::VoiceType::Alto),
        comfortable_range: range_view(profile.comfortable_range(None)),
        missing_phonemes: missing.iter().map(|p| (*p).to_owned()).collect(),
        substitutions,
        curves: wire::VoicebankCurves {
            dynamics: profile.curve_supported(CurveKind::Dynamics),
            tension: profile.curve_supported(CurveKind::Tension),
            breathiness: profile.curve_supported(CurveKind::Breathiness),
            pitch_bend: profile.curve_supported(CurveKind::PitchBend),
        },
        credit: builtin.and_then(|b| b.credit).map(str::to_owned),
        terms_url: builtin.and_then(|b| b.terms_url).map(str::to_owned),
        commercial: builtin.and_then(|b| b.commercial).map(str::to_owned),
        vocoder: bank.and_then(|b| b.vocoder_name.clone()),
        vocoder_installed: bank.is_some_and(|b| b.vocoder_config.is_some()),
    }
}

fn range_view(range: ComfortableRange) -> wire::VoicebankRange {
    wire::VoicebankRange {
        low: range.low,
        high: range.high,
        range_source: range.source.as_str().to_owned(),
    }
}
