//! Handlers for the per-track lane inspector messages: switch generator
//! kind, edit Bass/Melody/Pad/Drum parameters, and regenerate just this
//! lane (which bumps the lane's own seed — section-shared motif identity
//! is only touched by the chord-inspector's Regenerate motif button).

use iced::Task;

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassParams, MelodyParams, PadParams, VocalParams};

use super::regenerate::regenerate_lane;
use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag};
use crate::message::Message;

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    msg: LaneInspectorMsg,
) -> Task<Message> {
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
                    LaneGeneratorKindTag::Vocal => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Vocal(VocalParams::default()),
                                seed: definition_id.wrapping_mul(0xBF58476D1CE4E5B9),
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
        LaneInspectorMsg::ToggleMelodyFillVocalGaps => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.fill_vocal_gaps = !p.fill_vocal_gaps;
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

        // ------------------------------------------------------------------
        // Vocal lyrics
        // ------------------------------------------------------------------
        LaneInspectorMsg::SetVocalTheme(text) => {
            update_vocal(r, definition_id, track_id, |p| {
                // Mirror the prototype's 240-char cap.
                p.theme = text.chars().take(240).collect();
            });
        }
        LaneInspectorMsg::SetVocalMood(m) => {
            update_vocal(r, definition_id, track_id, |p| p.mood = m);
        }
        LaneInspectorMsg::SetVocalPov(pov) => {
            update_vocal(r, definition_id, track_id, |p| p.pov = pov);
        }
        LaneInspectorMsg::SetVocalRhyme(rhyme) => {
            update_vocal(r, definition_id, track_id, |p| p.rhyme = rhyme);
        }
        LaneInspectorMsg::SetVocalLines(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.lines = n.clamp(1, 16);
            });
        }
        LaneInspectorMsg::SetVocalSyllablesMin(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.syllables_min = n.clamp(1, p.syllables_max.max(1));
            });
        }
        LaneInspectorMsg::SetVocalSyllablesMax(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.syllables_max = n.clamp(p.syllables_min, 24);
            });
        }
        LaneInspectorMsg::ToggleVocalMatchSyllables => {
            update_vocal(r, definition_id, track_id, |p| {
                p.match_syllables_to_melody = !p.match_syllables_to_melody;
            });
        }
        LaneInspectorMsg::ToggleVocalAvoidCliches => {
            update_vocal(r, definition_id, track_id, |p| {
                p.avoid_cliches = !p.avoid_cliches;
            });
        }
        LaneInspectorMsg::ToggleVocalLockLine(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                if let Some(line) = p.draft.iter_mut().find(|l| l.n == n) {
                    line.locked = !line.locked;
                }
            });
        }
        LaneInspectorMsg::SetVocalLineText(n, text) => {
            update_vocal(r, definition_id, track_id, |p| {
                if let Some(line) = p.draft.iter_mut().find(|l| l.n == n) {
                    line.text = text;
                    line.syllables =
                        resonance_music_theory::count_syllables(&line.text).min(255) as u8;
                    // Edited lines are implicitly the user's authored version,
                    // so lock them to keep re-roll from clobbering the text.
                    line.locked = true;
                }
            });
            super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
        }
        LaneInspectorMsg::VocalBulkLyricsAction(action) => {
            super::vocal_lyrics::handle_bulk_lyrics_action(r, definition_id, track_id, action);
        }
        LaneInspectorMsg::RerollUnlockedLyrics => {
            super::vocal_render::roll_vocal_lyrics(r, definition_id, track_id, 0x9E3779B97F4A7C15);
            super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
        }
        LaneInspectorMsg::AutoSyllabifyLyrics => {
            update_vocal(r, definition_id, track_id, |p| {
                for line in p.draft.iter_mut() {
                    let new_text =
                        resonance_music_theory::g2p::auto_syllabify_text(&line.text);
                    if new_text != line.text {
                        // Refresh the corpus-stored syllable count to
                        // match the dotted text so downstream consumers
                        // (note allocator, SVS pipeline) see the
                        // higher syllable count too.
                        line.syllables = resonance_music_theory::count_syllables(&new_text)
                            .min(255) as u8;
                        line.text = new_text;
                    }
                }
            });
            super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
        }

        // ------------------------------------------------------------------
        // Vocal melody
        // ------------------------------------------------------------------
        LaneInspectorMsg::SetVocalVoiceType(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.voice = v;
                p.range = v.default_range();
            });
        }
        LaneInspectorMsg::SetVocalRangeLow(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.range.0 = n.min(p.range.1);
            });
        }
        LaneInspectorMsg::SetVocalRangeHigh(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.range.1 = n.max(p.range.0);
            });
        }
        LaneInspectorMsg::SetVocalStyle(s) => {
            update_vocal(r, definition_id, track_id, |p| p.style = s);
        }
        LaneInspectorMsg::SetVocalContour(c) => {
            update_vocal(r, definition_id, track_id, |p| p.contour = c);
        }
        LaneInspectorMsg::SetVocalSyllableMode(m) => {
            update_vocal(r, definition_id, track_id, |p| p.syllable_mode = m);
        }
        LaneInspectorMsg::SetVocalChordToneAnchor(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.chord_tone_anchor = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalLeapRange(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.leap_range = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalPhraseLength(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.phrase_length_bars = n.clamp(1, 8);
            });
        }
        LaneInspectorMsg::SetVocalBreath(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.breath = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::ToggleVocalStayInScale => {
            update_vocal(r, definition_id, track_id, |p| {
                p.stay_in_scale = !p.stay_in_scale;
            });
        }
        LaneInspectorMsg::ToggleVocalAvoidClashes => {
            update_vocal(r, definition_id, track_id, |p| {
                p.avoid_clashes = !p.avoid_clashes;
            });
        }
        LaneInspectorMsg::ToggleVocalUseSectionMotif => {
            update_vocal(r, definition_id, track_id, |p| {
                p.use_section_motif = !p.use_section_motif;
            });
        }

        // ------------------------------------------------------------------
        // Vocal voice & delivery
        // ------------------------------------------------------------------
        LaneInspectorMsg::SetVocalTimbre(t) => {
            update_vocal(r, definition_id, track_id, |p| p.timbre = t);
        }
        LaneInspectorMsg::SetVocalVoicebank(v) => {
            update_vocal(r, definition_id, track_id, |p| p.voicebank = v);
        }
        LaneInspectorMsg::SetVocalSinger(s) => {
            update_vocal(r, definition_id, track_id, |p| p.singer = s);
        }
        LaneInspectorMsg::SetVocalSingerMeiji(s) => {
            update_vocal(r, definition_id, track_id, |p| p.singer_meiji = s);
        }
        LaneInspectorMsg::SetVocalVibrato(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.vibrato = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalVibratoRate(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.vibrato_rate = v.clamp(2.0, 10.0);
            });
        }
        LaneInspectorMsg::SetVocalTension(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.tension = v.clamp(-1.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalTensionVelocityAmount(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.tension_velocity_amount = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalTensionContourAmount(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.tension_contour_amount = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalPortamentoMs(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.portamento_ms = v.clamp(0.0, 250.0);
            });
        }
        LaneInspectorMsg::SetVocalArticulation(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.articulation = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalConsonantEmphasis(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.consonant_emphasis = v.clamp(0.0, 1.0);
            });
        }

        // ------------------------------------------------------------------
        // Vocal generate actions
        // ------------------------------------------------------------------
        LaneInspectorMsg::GenerateVocalLyricsOnly => {
            super::vocal_render::roll_vocal_lyrics(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
            super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
        }
        LaneInspectorMsg::GenerateVocalMelodyOnly => {
            bump_lane_seed(r, definition_id, track_id, 0x94D049BB133111EB);
            return super::vocal_render::roll_vocal_melody(r, definition_id, track_id);
        }
        LaneInspectorMsg::GenerateVocalAll => {
            super::vocal_render::roll_vocal_lyrics(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
            super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
            bump_lane_seed(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
            return super::vocal_render::roll_vocal_melody(r, definition_id, track_id);
        }
        LaneInspectorMsg::RerenderVocalAudio => {
            return super::vocal_render::rerender_vocal_audio(r, definition_id, track_id);
        }

        LaneInspectorMsg::Regenerate => {
            // Bump the lane's own seed and re-derive only this lane. For Motif
            // lanes this varies the per-lane surface (phrase contours, rest
            // density holes) without touching the section-shared motif —
            // those identity bits only change via the chord inspector's
            // "Regenerate motif" button.
            bump_lane_seed(r, definition_id, track_id, 0x9E3779B97F4A7C15);
            return regenerate_lane(r, definition_id, track_id);
        }
    }
    Task::none()
}

/// Bump a lane's seed by `seed_mix + 1`. Centralised so the vocal regen
/// path (which dispatches through `regenerate_lane`) can reuse the same
/// seed without double-bumping in `roll_vocal_melody`.
fn bump_lane_seed(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    seed_mix: u64,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            cfg.seed = cfg.seed.wrapping_add(seed_mix).wrapping_add(1);
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

/// Mutate the vocal params of a lane in-place. Skips silently when the
/// lane has a different generator kind installed.
pub(super) fn update_vocal(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut VocalParams),
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Vocal(p) = kind {
            f(p);
        }
    });
}

