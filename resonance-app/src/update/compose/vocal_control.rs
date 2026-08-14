//! Vocal-lane lookups and the control-API lyric/render endpoints
//! (ba doc #265, todo #1156).
//!
//! Split out of `vocal_render` (ba todo #1259): resolving *which* lane a
//! control call addresses is a different job from producing its audio.
//! The four lookups are re-exported from `vocal_render` so the control
//! layer's existing import paths keep resolving.

use iced::Task;

use resonance_audio::types::TrackId;

use crate::compose::LaneGeneratorKind;
use crate::message::Message;

/// Whether a track has a vocal lane in any section — used by the control
/// endpoint to turn a "no lane" case into a precise error before it
/// synthesizes a mutating message.
pub(crate) fn track_has_vocal_lane(r: &crate::Resonance, track_id: TrackId) -> bool {
    r.compose.definitions.iter().any(|d| {
        matches!(
            d.lane_generators.get(&track_id).map(|c| &c.kind),
            Some(LaneGeneratorKind::Vocal(_))
        )
    })
}

/// Every section definition whose `track_id` lane is a vocal generator,
/// placed lanes first in placement order (ties broken by definition id
/// so the order is stable), then unplaced ones in creation order. A
/// definition placed several times appears once — a lane renders once
/// and its audio fans out to every placement.
///
/// This is the whole track: `vocal.render` with no `section_id` renders
/// all of it. Addressing only the head of this list is what left every
/// lane but the first frozen at its previous audio (ba doc #271 V2).
pub(crate) fn vocal_definitions_for_track(
    r: &crate::Resonance,
    track_id: TrackId,
) -> Vec<u64> {
    let is_vocal_lane = |definition_id: u64| {
        matches!(
            r.compose
                .find_definition(definition_id)
                .and_then(|d| d.lane_generators.get(&track_id))
                .map(|c| &c.kind),
            Some(LaneGeneratorKind::Vocal(_))
        )
    };

    let mut placed: Vec<(u32, u64)> = r
        .compose
        .placements
        .iter()
        .filter(|p| is_vocal_lane(p.definition_id))
        .map(|p| (p.start_bar, p.definition_id))
        .collect();
    placed.sort_by_key(|&(bar, def)| (bar, def));

    let mut out: Vec<u64> = Vec::new();
    for (_, def) in placed {
        if !out.contains(&def) {
            out.push(def);
        }
    }
    for def in &r.compose.definitions {
        if is_vocal_lane(def.id) && !out.contains(&def.id) {
            out.push(def.id);
        }
    }
    out
}

/// Every `(definition_id, track_id)` vocal lane in the project, ordered
/// by placement then track id. Backs a `vocal.render` that names no
/// track at all — "every vocal track".
pub(crate) fn all_vocal_lanes(r: &crate::Resonance) -> Vec<(u64, TrackId)> {
    let mut tracks: Vec<TrackId> = Vec::new();
    for def in &r.compose.definitions {
        for (track_id, cfg) in &def.lane_generators {
            if matches!(cfg.kind, LaneGeneratorKind::Vocal(_)) && !tracks.contains(track_id) {
                tracks.push(*track_id);
            }
        }
    }
    tracks.sort_unstable();
    tracks
        .into_iter()
        .flat_map(|track_id| {
            vocal_definitions_for_track(r, track_id)
                .into_iter()
                .map(move |def| (def, track_id))
        })
        .collect()
}

/// The first section (in placement order, then creation order) whose
/// `track_id` lane is a vocal generator. The control lyric methods
/// address a track; this picks the lane they act on when the caller
/// didn't (couldn't) name a section.
pub(crate) fn first_vocal_definition(r: &crate::Resonance, track_id: TrackId) -> Option<u64> {
    vocal_definitions_for_track(r, track_id).first().copied()
}

/// `vocal.set_lyrics`: replace the lane's whole draft from bulk text.
pub(crate) fn control_set_lyrics(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    text: &str,
) {
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        super::vocal_lyrics::rebuild_draft_from_bulk(p, text);
    });
    super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
}

/// `vocal.set_line`: replace one 0-based lyric line. Returns `false`
/// (leaving state untouched) when the index is out of range.
pub(crate) fn control_set_line(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    line_index: usize,
    text: &str,
) -> bool {
    let in_range = r
        .compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            LaneGeneratorKind::Vocal(p) => Some(line_index < p.draft.len()),
            _ => None,
        })
        .unwrap_or(false);
    if !in_range {
        return false;
    }
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        if let Some(line) = p.draft.get_mut(line_index) {
            line.text = text.to_owned();
            line.syllables =
                resonance_music_theory::count_syllables(text).min(255) as u8;
            // A hand-set line is locked so a later re-roll preserves it,
            // matching the per-line editor's behaviour.
            line.locked = true;
        }
    });
    super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
    true
}

/// `vocal.render`: set the lane's voicebank and synthesise the notes
/// currently in the lane's MIDI clip. Returns the async render `Task`
/// (or `Task::none()` when the lane can't render yet — the caller has
/// already validated the lane exists; a `Task::none()` here means no
/// generated clip / no notes, surfaced as `compose.last_error`).
///
/// **Renders, never generates.** This drives `rerender_vocal_audio`
/// (notes-only), not `roll_vocal_melody` (full regenerate) — the same
/// split the GUI exposes as "Re-render audio" versus "Generate". Wired
/// to the generate path, `vocal.render` silently discarded whatever the
/// client had authored into the clip and re-derived a melody from the
/// lane's seed, so three different authored note sets rendered to
/// byte-identical audio while every call reported success (ba doc #271
/// V1). Melody generation belongs to `vocal.generate`, which exists for
/// exactly that (doc #269 FR-2).
///
/// Lyrics still come from the lane's live draft: the render path
/// resolves them out of `VocalParams::draft`, and the clip's
/// `clip_lyrics` entry is only a per-note annotation overlay — so a
/// `vocal.set_lyrics` between generate and render is picked up here.
pub(crate) fn control_render(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    voicebank: resonance_music_theory::VocalVoicebank,
) -> Task<Message> {
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        p.voicebank = voicebank;
    });
    super::vocal_render::rerender_vocal_audio(r, definition_id, track_id)
}
