//! Handlers for the per-track lane inspector messages: switch generator
//! kind, edit Bass/Melody/Pad/Drum parameters, and regenerate just this
//! lane (which bumps the lane's own seed — section-shared motif identity
//! is only touched by the chord-inspector's Regenerate motif button).

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassParams, MelodyParams, PadParams};

use super::regenerate::regenerate_lane;
use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag};

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    msg: LaneInspectorMsg,
) {
    match msg {
        LaneInspectorMsg::SetGenerator(tag) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                match tag {
                    LaneGeneratorKindTag::Manual => {
                        def.lane_generators.remove(&track_id);
                    }
                    LaneGeneratorKindTag::Bass => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Bass(BassParams::default()),
                                seed: definition_id.wrapping_mul(0x9E3779B97F4A7C15),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Melody => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Melody(MelodyParams::default()),
                                seed: definition_id.wrapping_mul(0x517CC1B727220A95),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Pad => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Pad(PadParams::default()),
                                seed: definition_id.wrapping_mul(0x6C62272E07BB0142),
                            },
                        );
                    }
                }
                r.compose.last_error = None;
            }
        }

        // Bass parameter updates
        LaneInspectorMsg::SetBassStyle(style) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.style = style;
                }
            });
        }
        LaneInspectorMsg::SetBassBaseNote(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.base_note = note;
                }
            });
        }
        LaneInspectorMsg::SetBassVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.velocity = v;
                }
            });
        }
        LaneInspectorMsg::SetBassMotifMode(mode) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.motif_mode = mode;
                }
            });
        }
        LaneInspectorMsg::SetBassMotifPhrase(phrase) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.motif_phrase = phrase;
                }
            });
        }

        // Melody parameter updates
        LaneInspectorMsg::SetMelodyStyle(style) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.style = style;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRegisterLow(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.register.0 = note;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRegisterHigh(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.register.1 = note;
                }
            });
        }
        LaneInspectorMsg::SetMelodyNoteValue(ticks) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.note_value_ticks = ticks;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRestDensity(d) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.rest_density = d;
                }
            });
        }
        LaneInspectorMsg::SetMelodyVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.velocity = v;
                }
            });
        }
        LaneInspectorMsg::SetMelodyArticulation(a) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.articulation = a;
                }
            });
        }
        LaneInspectorMsg::SetMelodyContour(contour) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.contour = contour;
                }
            });
        }
        LaneInspectorMsg::SetMelodyPhraseLen(len) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.phrase_len = len;
                }
            });
        }

        // Pad parameter updates
        LaneInspectorMsg::SetPadRegisterLow(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.register.0 = note;
                }
            });
        }
        LaneInspectorMsg::SetPadRegisterHigh(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.register.1 = note;
                }
            });
        }
        LaneInspectorMsg::SetPadVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.velocity = v;
                }
            });
        }

        // Drum voice mode
        LaneInspectorMsg::SetDrumVoiceMode { pad_index, mode } => {
            ensure_drum_config(r, definition_id, track_id);
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Drum(dc) = kind {
                    dc.voices.insert(pad_index, mode);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidSteps { pad_index, steps } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { steps: s, .. } = mode {
                    *s = steps.max(1);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidHits { pad_index, hits } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { hits: h, steps, .. } = mode {
                    *h = hits.min(*steps);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidRotation {
            pad_index,
            rotation,
        } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { rotation: rot, .. } = mode {
                    *rot = rotation;
                }
            });
        }

        LaneInspectorMsg::Regenerate => {
            // Bump the lane's own seed and re-derive only this lane. For Motif
            // lanes this varies the per-lane surface (phrase contours, rest
            // density holes) without touching the section-shared motif —
            // those identity bits only change via the chord inspector's
            // "Regenerate motif" button.
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
                    cfg.seed = cfg.seed.wrapping_add(0x9E3779B97F4A7C15).wrapping_add(1);
                }
            }
            regenerate_lane(r, definition_id, track_id);
        }
    }
}

/// Mutate a lane's generator kind in-place.
fn update_lane_gen(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut LaneGeneratorKind),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            f(&mut cfg.kind);
        }
        r.compose.last_error = None;
    }
}

/// Ensure a drum lane config exists for the given track.
fn ensure_drum_config(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.lane_generators
            .entry(track_id)
            .or_insert_with(|| LaneGeneratorConfig {
                kind: LaneGeneratorKind::Drum(crate::compose::DrumLaneConfig::default()),
                seed: 0,
            });
    }
}

/// Mutate a specific drum voice's mode in-place.
fn update_drum_voice(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    pad_index: usize,
    f: impl FnOnce(&mut crate::compose::DrumVoiceMode),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            if let LaneGeneratorKind::Drum(dc) = &mut cfg.kind {
                if let Some(mode) = dc.voices.get_mut(&pad_index) {
                    f(mode);
                }
            }
        }
        r.compose.last_error = None;
    }
}
