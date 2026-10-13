//! `vocal.voicebanks`: the voicebank registry in wire shape
//! (vocal-mcp-control-and-quality.md §4.5).

use resonance_control::methods::vocal as wire;
use resonance_music_theory::VoicebankId;

use crate::compose::vocal_svs::registry::{
    BankTerms, ComfortableRange, PlokIndex, PlokVoicebank, VoicebankRegistry,
};
use crate::compose::vocal_svs::{BankProfile, CurveKind, PhonemeFate, VoicebankPhonemes};

/// What the plok.org index looked like when `vocal.voicebanks` answered.
pub(in crate::update::control) struct IndexView<'a> {
    pub index: Option<&'a PlokIndex>,
    /// `fresh`, `cached`, `fetching` or `unavailable`.
    pub status: &'static str,
    pub error: Option<String>,
}

/// Every installed bank, by id, and the index's banks that are not
/// installed (spec §4.5).
pub(in crate::update::control) fn voicebank_list(
    registry: &VoicebankRegistry,
    index: IndexView<'_>,
) -> wire::VoicebankList {
    let available = index
        .index
        .map(|i| {
            i.voicebanks
                .iter()
                .filter(|b| registry.get(&b.id).is_none())
                .map(available_entry)
                .collect()
        })
        .unwrap_or_default();
    wire::VoicebankList {
        installed: registry
            .installed()
            .map(|bank| {
                voicebank_entry(
                    &registry.profile(&VoicebankId::new(&bank.id)),
                    registry.terms(&bank.id),
                )
            })
            .collect(),
        available,
        index_status: Some(index.status.to_owned()),
        index_error: index.error,
    }
}

fn available_entry(b: &PlokVoicebank) -> wire::AvailableVoicebank {
    wire::AvailableVoicebank {
        id: b.id.clone(),
        name: b.name.clone(),
        version: b.version.clone(),
        terms_url: b.terms_url.clone(),
        credit: b.credit.clone(),
        commercial: b.commercial.clone(),
        languages: b.languages.clone(),
        speakers: b.speakers.clone(),
        size: b.size_text(),
        vocoder: b.vocoder.clone(),
        url: Some(b.url.clone()),
    }
}

fn voicebank_entry(profile: &BankProfile, terms: BankTerms) -> wire::VoicebankEntry {
    let bank = profile.installed();
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
            effect: s.effect,
        })
        .collect();
    wire::VoicebankEntry {
        id: profile.id().to_string(),
        name: profile.name(),
        version: bank.and_then(|b| {
            b.version
                .clone()
                .or_else(|| b.plok.as_ref().and_then(|p| p.version.clone()))
        }),
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
        models: {
            let m = profile.bank_models();
            wire::VoicebankModels {
                duration: m.duration.is_some(),
                pitch: m.pitch.is_some(),
                variance: m.variance.is_some(),
            }
        },
        // What a plok.org install recorded (the terms the user accepted),
        // trusted as `VoicebankRegistry::terms` says.
        credit: terms.credit,
        terms_url: terms.terms_url,
        commercial: terms.commercial,
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
