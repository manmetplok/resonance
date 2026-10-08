//! Vocal-lane projections: which clip a lane sings, how many notes it
//! holds, whether those notes can actually be articulated, and whether
//! its SVS audio is current.

use crate::Resonance;
use resonance_control::methods::song::{
    VocalRenderErrorKind, VocalRenderErrorView, VocalRenderState,
};

/// Notes in a vocal lane's derived clip — what the SVS render actually
/// sings (ba doc #269 FR-7). A lane is derived once per placement of its
/// section, and every placement carries the same material, so the first
/// entry found for `(definition, track)` is the lane's note count. `0`
/// means the lane has not been generated yet.
pub(in crate::update::control) fn lane_note_count(
    app: &Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
) -> usize {
    // Prefer the derived-clip map, but do not trust it as the only
    // answer: a lane whose map entry is missing or points at a clip that
    // is no longer in `midi_clips` reported 0 notes while `song.notes`
    // on that lane's clip plainly returned some, which reads as "not
    // generated" and silenced the mismatch flag (ba doc #271).
    lane_clip(app, definition_id, track_id).map_or(0, |clip| clip.notes.len())
}

/// The MIDI clip a vocal lane sings from: the derived-clip map first,
/// then the placement-start fallback that `rebuild_derived_clips` uses to
/// recover the mapping after a load (a MIDI clip on this track starting
/// at one of the section's placement bars is this lane's clip).
pub(in crate::update::control) fn lane_clip<'a>(
    app: &'a Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
) -> Option<&'a crate::state::MidiClipState> {
    let mapped = app
        .compose
        .derived_clips
        .iter()
        .filter(|((def, _, track), _)| *def == definition_id && *track == track_id)
        .find_map(|(_, clip_id)| app.midi_clips.iter().find(|c| c.id == *clip_id));
    if mapped.is_some() {
        return mapped;
    }
    app.compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .find_map(|p| {
            let start = app.tempo_map.bar_to_sample(p.start_bar);
            app.midi_clips
                .iter()
                .find(|c| c.track_id == track_id && c.start_sample == start)
        })
}

/// Per-note articulation report for one vocal lane — the data behind
/// `song.vocal`'s `too_short` / `out_of_range` flags.
///
/// Resolves the lane's pronunciation the same way the render does
/// (`override > project-dict > bank-dict > CMU-auto`, then the voicebank's phoneme
/// substitutions) so the phonemes reported are the ones that will be
/// sung. A lane whose phonemes fail the voicebank gate outright reports
/// on the unsubstituted stream rather than nothing — the render will
/// refuse with its own precise error, and the durations are still true.
pub(in crate::update::control) fn lane_articulation(
    app: &Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    params: &resonance_music_theory::VocalParams,
) -> Vec<crate::compose::vocal_svs::NoteArticulation> {
    let Some(clip) = lane_clip(app, definition_id, track_id) else {
        return Vec::new();
    };
    if clip.notes.is_empty() {
        return Vec::new();
    }
    let annotations = app
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .cloned()
        .unwrap_or_else(|| vec![String::new(); clip.notes.len()]);
    let bank = app.voicebanks.profile(&params.voicebank);
    let bank_dictionary = bank.dictionary();
    let resolved = app.compose.pronunciation.resolve_lane(
        definition_id,
        track_id,
        &params.draft,
        &annotations,
        clip.notes.len(),
        &[],
        bank_dictionary.as_deref(),
    );
    let assigned = crate::compose::vocal_svs::validate_for_voicebank(&resolved, &bank)
        .unwrap_or(resolved);
    crate::compose::vocal_svs::articulation_report_for(
        &clip.notes,
        &assigned,
        resonance_audio::types::TICKS_PER_QUARTER_NOTE as u32,
        // The render path reads the section's tempo the same way
        // (`vocal_render::enqueue_vocal_render`), so the durations
        // reported here are the ones the segment builder will divide up.
        crate::update::compose::section_meter(app, definition_id).bpm,
        &bank,
        params.speaker.as_deref(),
        params.voice,
        params.rest_threshold_beats,
    )
}

/// The lane's render state on the wire: state, stale-unit count and the
/// last failure (vocal-mcp-control-and-quality.md §7). Computed by
/// `update::compose::vocal_render_state`, which compares the installed
/// take's unit keys with the lane's current plan.
pub(in crate::update::control) fn lane_render_view(
    app: &Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
) -> (VocalRenderState, usize, Option<VocalRenderErrorView>) {
    use crate::compose::VocalLaneRenderState as S;
    let status = crate::update::compose::vocal_render_state::lane_render_status(
        app,
        (definition_id, track_id),
    );
    let state = match status.state {
        S::NotRendered => VocalRenderState::NotRendered,
        S::Rendering => VocalRenderState::Rendering,
        S::Rendered => VocalRenderState::Rendered,
        S::Stale => VocalRenderState::Stale,
        S::Error => VocalRenderState::Error,
    };
    let error = status.error.map(|(kind, message)| VocalRenderErrorView {
        kind: error_kind(kind),
        message,
    });
    (state, status.stale_units, error)
}

fn error_kind(kind: crate::compose::VocalLaneErrorKind) -> VocalRenderErrorKind {
    use crate::compose::VocalLaneErrorKind as K;
    match kind {
        K::VoicebankMissing => VocalRenderErrorKind::VoicebankMissing,
        K::NothingToSing => VocalRenderErrorKind::NothingToSing,
        K::NoNotes => VocalRenderErrorKind::NoNotes,
        K::UnsupportedPhonemes => VocalRenderErrorKind::UnsupportedPhonemes,
        K::InvalidParams => VocalRenderErrorKind::InvalidParams,
        K::RenderFailed => VocalRenderErrorKind::RenderFailed,
    }
}

/// A track's render state: the worst of its lanes'
/// (`VocalLaneRenderState::severity`: `error > rendering > stale >
/// not_rendered > rendered`); `not_rendered` for a track with no lanes.
pub(in crate::update::control) fn track_render_state(
    lanes: impl IntoIterator<Item = VocalRenderState>,
) -> VocalRenderState {
    let severity = |s: &VocalRenderState| match s {
        VocalRenderState::Rendered => 0,
        VocalRenderState::NotRendered => 1,
        VocalRenderState::Stale => 2,
        VocalRenderState::Rendering => 3,
        VocalRenderState::Error => 4,
    };
    lanes
        .into_iter()
        .max_by_key(severity)
        .unwrap_or(VocalRenderState::NotRendered)
}
