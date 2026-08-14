//! Vocal-lane projections: which clip a lane sings, how many notes it
//! holds, whether those notes can actually be articulated, and whether
//! the SVS render has run.

use crate::Resonance;
use resonance_control::methods::song::VocalRenderState;

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
/// (`override > project-dict > CMU-auto`, then the voicebank's phoneme
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
    let empty = std::collections::HashMap::new();
    let overrides = app
        .compose
        .pronunciation
        .clip_overrides(clip.id)
        .unwrap_or(&empty);
    let annotations = app
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .cloned()
        .unwrap_or_else(|| vec![String::new(); clip.notes.len()]);
    let resolved = crate::compose::vocal_svs::resolve_clip_pronunciation(
        &params.draft,
        &annotations,
        clip.notes.len(),
        overrides,
        &app.compose.pronunciation.project_dictionary,
        &[],
    );
    let assigned = crate::compose::vocal_svs::validate_for_voicebank(&resolved, params.voicebank)
        .unwrap_or(resolved);
    crate::compose::vocal_svs::articulation_report(
        &clip.notes,
        &assigned,
        resonance_audio::types::TICKS_PER_QUARTER_NOTE as u32,
        // The render path reads the transport tempo the same way
        // (`vocal_render::rerender_vocal_audio`), so the durations
        // reported here are the ones the segment builder will divide up.
        app.transport.bpm,
        params.voicebank,
    )
}

/// SVS render state of a vocal track, from the vocal-audio registry:
/// an installed render for any of the track's lanes -> `rendered`; a
/// queued render epoch with nothing installed yet -> `rendering`; else
/// `not_rendered`. (Staleness/error tracking has no persistent app
/// state to read yet.)
pub(in crate::update::control) fn vocal_render_state(
    app: &Resonance,
    track: resonance_audio::types::TrackId,
) -> VocalRenderState {
    let installed = app
        .compose
        .vocal_audio
        .clips
        .keys()
        .any(|(_, _, t)| *t == track);
    if installed {
        return VocalRenderState::Rendered;
    }
    let queued = app
        .compose
        .vocal_audio
        .render_epoch
        .keys()
        .any(|(_, t)| *t == track);
    if queued {
        VocalRenderState::Rendering
    } else {
        VocalRenderState::NotRendered
    }
}
