//! Voice presets in wire shape (`vocal.voice_presets`,
//! `vocal.save_voice_preset`).

use resonance_control::methods::vocal as wire;

use super::vocal_params::{timbre_to_wire, voice_to_wire};
use crate::compose::vocal_svs::voice_presets::VoicePreset;
use crate::compose::vocal_svs::VoicebankRegistry;

/// `preset` as `vocal.voice_presets` lists it; `installed` says whether
/// its bank is in `registry`.
pub(in crate::update::control) fn voice_preset_view(
    preset: &VoicePreset,
    registry: &VoicebankRegistry,
) -> wire::VoicePresetView {
    let s = &preset.settings;
    wire::VoicePresetView {
        id: preset.id.clone(),
        name: preset.name.clone(),
        description: preset.description.clone(),
        tags: preset.tags.clone(),
        score: preset.score,
        builtin: preset.builtin,
        voicebank: s.voicebank.to_string(),
        speaker: s.speaker.clone(),
        installed: registry.get(s.voicebank.as_str()).is_some(),
        settings: wire::VoicePresetSettings {
            voicebank: s.voicebank.to_string(),
            speaker: s.speaker.clone(),
            voice: voice_to_wire(s.voice),
            range: wire::VocalRange {
                low: s.range.0,
                high: s.range.1,
            },
            timbre: timbre_to_wire(s.timbre),
            tension: s.tension,
            tension_velocity_amount: s.tension_velocity_amount,
            tension_contour_amount: s.tension_contour_amount,
            vibrato: s.vibrato,
            vibrato_rate: s.vibrato_rate,
            portamento_ms: s.portamento_ms,
            articulation: s.articulation,
            consonant_emphasis: s.consonant_emphasis,
            velocity_dynamics: s.velocity_dynamics,
            rest_threshold_beats: s.rest_threshold_beats,
            bank_timing: s.bank_timing,
            bank_pitch: s.bank_pitch,
        },
    }
}
