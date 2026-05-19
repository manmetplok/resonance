//! Handlers for the per-track lane inspector messages: switch generator
//! kind, edit Bass/Melody/Pad/Vocal parameters, and regenerate just this
//! lane (which bumps the lane's own seed — section-shared motif identity
//! is only touched by the chord-inspector's Regenerate motif button).
//!
//! Each generator family has its own submodule under
//! `update/compose/lane_inspector/`:
//!
//! - [`bass_params`]   — `SetBass*` setters.
//! - [`melody_params`] — `SetMelody*` / `ToggleMelody*` setters.
//! - [`pad_params`]    — `SetPad*` setters.
//! - [`vocal_params`]  — every `*Vocal*` arm (lyrics, melody, voice &
//!   delivery, generate / re-render actions).
//! - [`common`]        — shared `update_lane_gen` / `update_vocal` /
//!   `bump_lane_seed` helpers used by every per-generator file.
//!
//! `SetGenerator` and `Regenerate` (the cross-cutting arms) stay in this
//! dispatcher because they touch state outside any single generator family.

use iced::Task;

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassParams, MelodyParams, PadParams, VocalParams};

use super::regenerate::regenerate_lane;
use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag};
use crate::message::Message;

mod bass_params;
mod common;
mod melody_params;
mod pad_params;
mod vocal_params;

pub(crate) use common::update_vocal;

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    msg: LaneInspectorMsg,
) -> Task<Message> {
    match msg {
        LaneInspectorMsg::SetGenerator(tag) => set_generator(r, definition_id, track_id, tag),

        // Bass parameter updates
        LaneInspectorMsg::SetBassStyle(style) => {
            bass_params::set_style(r, definition_id, track_id, style)
        }
        LaneInspectorMsg::SetBassBaseNote(note) => {
            bass_params::set_base_note(r, definition_id, track_id, note)
        }
        LaneInspectorMsg::SetBassVelocity(v) => {
            bass_params::set_velocity(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetBassMotifMode(mode) => {
            bass_params::set_motif_mode(r, definition_id, track_id, mode)
        }
        LaneInspectorMsg::SetBassMotifPhrase(phrase) => {
            bass_params::set_motif_phrase(r, definition_id, track_id, phrase)
        }

        // Melody parameter updates
        LaneInspectorMsg::SetMelodyStyle(style) => {
            melody_params::set_style(r, definition_id, track_id, style)
        }
        LaneInspectorMsg::SetMelodyRegisterLow(note) => {
            melody_params::set_register_low(r, definition_id, track_id, note)
        }
        LaneInspectorMsg::SetMelodyRegisterHigh(note) => {
            melody_params::set_register_high(r, definition_id, track_id, note)
        }
        LaneInspectorMsg::SetMelodyNoteValue(ticks) => {
            melody_params::set_note_value(r, definition_id, track_id, ticks)
        }
        LaneInspectorMsg::SetMelodyRestDensity(d) => {
            melody_params::set_rest_density(r, definition_id, track_id, d)
        }
        LaneInspectorMsg::SetMelodyVelocity(v) => {
            melody_params::set_velocity(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetMelodyArticulation(a) => {
            melody_params::set_articulation(r, definition_id, track_id, a)
        }
        LaneInspectorMsg::SetMelodyContour(contour) => {
            melody_params::set_contour(r, definition_id, track_id, contour)
        }
        LaneInspectorMsg::SetMelodyPhraseLen(len) => {
            melody_params::set_phrase_len(r, definition_id, track_id, len)
        }
        LaneInspectorMsg::ToggleMelodyFillVocalGaps => {
            melody_params::toggle_fill_vocal_gaps(r, definition_id, track_id)
        }

        // Pad parameter updates
        LaneInspectorMsg::SetPadRegisterLow(note) => {
            pad_params::set_register_low(r, definition_id, track_id, note)
        }
        LaneInspectorMsg::SetPadRegisterHigh(note) => {
            pad_params::set_register_high(r, definition_id, track_id, note)
        }
        LaneInspectorMsg::SetPadVelocity(v) => {
            pad_params::set_velocity(r, definition_id, track_id, v)
        }

        // Vocal lyrics
        LaneInspectorMsg::SetVocalTheme(text) => {
            vocal_params::set_theme(r, definition_id, track_id, text)
        }
        LaneInspectorMsg::SetVocalMood(m) => {
            vocal_params::set_mood(r, definition_id, track_id, m)
        }
        LaneInspectorMsg::SetVocalPov(pov) => {
            vocal_params::set_pov(r, definition_id, track_id, pov)
        }
        LaneInspectorMsg::SetVocalRhyme(rhyme) => {
            vocal_params::set_rhyme(r, definition_id, track_id, rhyme)
        }
        LaneInspectorMsg::SetVocalLines(n) => {
            vocal_params::set_lines(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::SetVocalSyllablesMin(n) => {
            vocal_params::set_syllables_min(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::SetVocalSyllablesMax(n) => {
            vocal_params::set_syllables_max(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::ToggleVocalMatchSyllables => {
            vocal_params::toggle_match_syllables(r, definition_id, track_id)
        }
        LaneInspectorMsg::ToggleVocalAvoidCliches => {
            vocal_params::toggle_avoid_cliches(r, definition_id, track_id)
        }
        LaneInspectorMsg::ToggleVocalLockLine(n) => {
            vocal_params::toggle_lock_line(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::SetVocalLineText(n, text) => {
            vocal_params::set_line_text(r, definition_id, track_id, n, text)
        }
        LaneInspectorMsg::VocalBulkLyricsAction(action) => {
            super::vocal_lyrics::handle_bulk_lyrics_action(r, definition_id, track_id, action);
        }
        LaneInspectorMsg::RerollUnlockedLyrics => {
            vocal_params::reroll_unlocked_lyrics(r, definition_id, track_id)
        }
        LaneInspectorMsg::AutoSyllabifyLyrics => {
            vocal_params::auto_syllabify_lyrics(r, definition_id, track_id)
        }

        // Vocal melody
        LaneInspectorMsg::SetVocalVoiceType(v) => {
            vocal_params::set_voice_type(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalRangeLow(n) => {
            vocal_params::set_range_low(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::SetVocalRangeHigh(n) => {
            vocal_params::set_range_high(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::SetVocalStyle(s) => {
            vocal_params::set_style(r, definition_id, track_id, s)
        }
        LaneInspectorMsg::SetVocalContour(c) => {
            vocal_params::set_contour(r, definition_id, track_id, c)
        }
        LaneInspectorMsg::SetVocalSyllableMode(m) => {
            vocal_params::set_syllable_mode(r, definition_id, track_id, m)
        }
        LaneInspectorMsg::SetVocalChordToneAnchor(v) => {
            vocal_params::set_chord_tone_anchor(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalLeapRange(v) => {
            vocal_params::set_leap_range(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalPhraseLength(n) => {
            vocal_params::set_phrase_length(r, definition_id, track_id, n)
        }
        LaneInspectorMsg::SetVocalBreath(v) => {
            vocal_params::set_breath(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::ToggleVocalStayInScale => {
            vocal_params::toggle_stay_in_scale(r, definition_id, track_id)
        }
        LaneInspectorMsg::ToggleVocalAvoidClashes => {
            vocal_params::toggle_avoid_clashes(r, definition_id, track_id)
        }
        LaneInspectorMsg::ToggleVocalUseSectionMotif => {
            vocal_params::toggle_use_section_motif(r, definition_id, track_id)
        }

        // Vocal voice & delivery
        LaneInspectorMsg::SetVocalTimbre(t) => {
            vocal_params::set_timbre(r, definition_id, track_id, t)
        }
        LaneInspectorMsg::SetVocalVoicebank(v) => {
            vocal_params::set_voicebank(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalSinger(s) => {
            vocal_params::set_singer(r, definition_id, track_id, s)
        }
        LaneInspectorMsg::SetVocalSingerMeiji(s) => {
            vocal_params::set_singer_meiji(r, definition_id, track_id, s)
        }
        LaneInspectorMsg::SetVocalVibrato(v) => {
            vocal_params::set_vibrato(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalVibratoRate(v) => {
            vocal_params::set_vibrato_rate(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalTension(v) => {
            vocal_params::set_tension(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalTensionVelocityAmount(v) => {
            vocal_params::set_tension_velocity_amount(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalTensionContourAmount(v) => {
            vocal_params::set_tension_contour_amount(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalPortamentoMs(v) => {
            vocal_params::set_portamento_ms(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalArticulation(v) => {
            vocal_params::set_articulation(r, definition_id, track_id, v)
        }
        LaneInspectorMsg::SetVocalConsonantEmphasis(v) => {
            vocal_params::set_consonant_emphasis(r, definition_id, track_id, v)
        }

        // Vocal generate actions
        LaneInspectorMsg::GenerateVocalLyricsOnly => {
            vocal_params::generate_lyrics_only(r, definition_id, track_id)
        }
        LaneInspectorMsg::GenerateVocalMelodyOnly => {
            return vocal_params::generate_melody_only(r, definition_id, track_id);
        }
        LaneInspectorMsg::GenerateVocalAll => {
            return vocal_params::generate_all(r, definition_id, track_id);
        }
        LaneInspectorMsg::RerenderVocalAudio => {
            return vocal_params::rerender_audio(r, definition_id, track_id);
        }

        LaneInspectorMsg::Regenerate => {
            // Bump the lane's own seed and re-derive only this lane. For
            // Motif lanes this varies the per-lane surface (phrase
            // contours, rest density holes) without touching the
            // section-shared motif — those identity bits only change via
            // the chord inspector's "Regenerate motif" button.
            common::bump_lane_seed(r, definition_id, track_id, 0x9E3779B97F4A7C15);
            return regenerate_lane(r, definition_id, track_id);
        }
    }
    Task::none()
}

/// Switch the generator type for a lane. Manual removes the entry; the
/// other tags install a default-params generator with a deterministic
/// per-tag seed derived from the section's definition id.
fn set_generator(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    tag: LaneGeneratorKindTag,
) {
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
