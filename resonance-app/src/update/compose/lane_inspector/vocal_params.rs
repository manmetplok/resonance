//! Handlers for the `LaneInspectorMsg::*Vocal*` arms that carry real
//! logic — clamping, coupled fields, draft-line edits with side effects,
//! plus the generate / re-render actions (which bump the lane seed and
//! may return a long-running `Task<Message>` for the SVS pipeline).
//! Plain field assignments and toggles are inlined in the dispatcher
//! (`lane_inspector::handle`) as direct `update_vocal` calls.

use iced::Task;

use resonance_audio::types::TrackId;
use resonance_music_theory::VoiceType;

use super::common::{bump_lane_seed, update_vocal};
use crate::message::Message;

// ---------------------------------------------------------------------------
// Vocal lyrics
// ---------------------------------------------------------------------------

pub(super) fn set_theme(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    text: String,
) {
    update_vocal(r, definition_id, track_id, |p| {
        // Mirror the prototype's 240-char cap.
        p.theme = text.chars().take(240).collect();
    });
}

pub(super) fn set_lines(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.lines = n.clamp(1, 16);
    });
}

pub(super) fn set_syllables_min(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.syllables_min = n.clamp(1, p.syllables_max.max(1));
    });
}

pub(super) fn set_syllables_max(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.syllables_max = n.clamp(p.syllables_min, 24);
    });
}

pub(super) fn toggle_lock_line(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        if let Some(line) = p.draft.iter_mut().find(|l| l.n == n) {
            line.locked = !line.locked;
        }
    });
}

pub(super) fn set_line_text(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
    text: String,
) {
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
    super::super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
}

pub(super) fn reroll_unlocked_lyrics(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    super::super::vocal_render::roll_vocal_lyrics(
        r,
        definition_id,
        track_id,
        0x9E3779B97F4A7C15,
    );
    super::super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
}

pub(super) fn auto_syllabify_lyrics(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    update_vocal(r, definition_id, track_id, |p| {
        for line in p.draft.iter_mut() {
            let new_text = resonance_music_theory::g2p::auto_syllabify_text(&line.text);
            if new_text != line.text {
                // Refresh the corpus-stored syllable count to match the
                // dotted text so downstream consumers (note allocator,
                // SVS pipeline) see the higher syllable count too.
                line.syllables =
                    resonance_music_theory::count_syllables(&new_text).min(255) as u8;
                line.text = new_text;
            }
        }
    });
    super::super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
}

// ---------------------------------------------------------------------------
// Vocal melody
// ---------------------------------------------------------------------------

pub(super) fn set_voice_type(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: VoiceType,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.voice = v;
        p.range = v.default_range();
    });
}

pub(super) fn set_range_low(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.range.0 = n.min(p.range.1);
    });
}

pub(super) fn set_range_high(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.range.1 = n.max(p.range.0);
    });
}

pub(super) fn set_chord_tone_anchor(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.chord_tone_anchor = v.clamp(0.0, 1.0);
    });
}

pub(super) fn set_leap_range(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.leap_range = v.clamp(0.0, 1.0);
    });
}

pub(super) fn set_phrase_length(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    n: u8,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.phrase_length_bars = n.clamp(1, 8);
    });
}

pub(super) fn set_breath(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.breath = v.clamp(0.0, 1.0);
    });
}

// ---------------------------------------------------------------------------
// Vocal voice & delivery
// ---------------------------------------------------------------------------

pub(super) fn set_vibrato(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.vibrato = v.clamp(0.0, 1.0);
    });
}

pub(super) fn set_vibrato_rate(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.vibrato_rate = v.clamp(2.0, 10.0);
    });
}

pub(super) fn set_tension(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.tension = v.clamp(-1.0, 1.0);
    });
}

pub(super) fn set_tension_velocity_amount(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.tension_velocity_amount = v.clamp(0.0, 1.0);
    });
}

pub(super) fn set_tension_contour_amount(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.tension_contour_amount = v.clamp(0.0, 1.0);
    });
}

pub(super) fn set_portamento_ms(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.portamento_ms = v.clamp(0.0, 250.0);
    });
}

pub(super) fn set_articulation(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.articulation = v.clamp(0.0, 1.0);
    });
}

pub(super) fn set_consonant_emphasis(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_vocal(r, definition_id, track_id, |p| {
        p.consonant_emphasis = v.clamp(0.0, 1.0);
    });
}

// ---------------------------------------------------------------------------
// Vocal generate actions
// ---------------------------------------------------------------------------

pub(super) fn generate_lyrics_only(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    super::super::vocal_render::roll_vocal_lyrics(
        r,
        definition_id,
        track_id,
        0xBF58476D1CE4E5B9,
    );
    super::super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
}

pub(super) fn generate_melody_only(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    bump_lane_seed(r, definition_id, track_id, 0x94D049BB133111EB);
    super::super::vocal_render::roll_vocal_melody(r, definition_id, track_id)
}

pub(super) fn generate_all(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    super::super::vocal_render::roll_vocal_lyrics(
        r,
        definition_id,
        track_id,
        0xBF58476D1CE4E5B9,
    );
    super::super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
    bump_lane_seed(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
    super::super::vocal_render::roll_vocal_melody(r, definition_id, track_id)
}

pub(super) fn rerender_audio(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    super::super::vocal_render::rerender_vocal_audio(r, definition_id, track_id)
}
