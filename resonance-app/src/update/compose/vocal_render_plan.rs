//! The decision half of a vocal render: what will be sung and where the
//! resulting audio goes on the timeline.
//!
//! Split out of `vocal_render::enqueue_vocal_render` (ba todo #1259).
//! Every function here is pure — no [`crate::Resonance`], no engine, no
//! disk — so the pronunciation gate, the placement maths and the render
//! epoch can be pinned directly from `tests/`. The handler stays
//! responsible for *applying* the plan (tear-down, epoch bump, cache
//! hand-off, task spawn).

use std::collections::HashMap;

use resonance_audio::types::{MidiNote, TempoMap};
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;

use crate::compose::vocal_svs::{self, DictionaryEntry, InvalidSyllable, SyllableOverride};

/// A validated, placed vocal render, ready for the handler to execute.
#[derive(Debug)]
pub struct VocalRenderPlan {
    /// The per-note syllable stream after pronunciation resolution and
    /// the voicebank gate — exactly what the SVS pipeline will sing.
    pub assigned: Vec<AssignedSyllable>,
    /// `(placement_id, start_sample)` for the rendered audio at every
    /// placement of the section.
    pub audio_starts: Vec<(u64, u64)>,
    /// Tick of the lane's first note; the audio's lead-in offset.
    pub lead_ticks: u64,
}

/// Everything the planner reads. Borrowed straight out of app state by
/// the handler's thin adapter, or hand-built in tests.
pub struct VocalRenderPlanInputs<'a> {
    pub tempo_map: &'a TempoMap,
    pub engine_sample_rate: u32,
    /// Lane params — the planner reads `draft` (lyrics) and `voicebank`
    /// (the phoneme gate).
    pub params: &'a VocalParams,
    /// Per-note lyric annotations overlaying the draft.
    pub annotations: &'a [String],
    /// Notes to be sung.
    pub midi_notes: &'a [MidiNote],
    /// `(placement_id, section_start_sample)` for every placement.
    pub placement_starts: &'a [(u64, u64)],
    /// The edited clip's per-syllable pronunciation overrides. Empty for
    /// a fresh roll, which has no clip to have edited yet.
    pub overrides: &'a HashMap<usize, SyllableOverride>,
    pub project_dictionary: &'a [DictionaryEntry],
}

/// Resolve + gate the pronunciation and place the audio.
///
/// `Err` carries the user-facing status-bar line for a draft the active
/// voicebank can't sing; the handler surfaces it and leaves the existing
/// audio untouched rather than corrupting the segment (#494).
pub fn plan_vocal_render(inputs: VocalRenderPlanInputs<'_>) -> Result<VocalRenderPlan, String> {
    let assigned = resolve_and_validate(
        inputs.params,
        inputs.annotations,
        inputs.midi_notes.len(),
        inputs.overrides,
        inputs.project_dictionary,
    )?;
    let lead = lead_ticks(inputs.midi_notes);
    let audio_starts = placement_audio_starts(
        inputs.tempo_map,
        inputs.placement_starts,
        lead,
        inputs.engine_sample_rate,
    );
    Ok(VocalRenderPlan {
        assigned,
        audio_starts,
        lead_ticks: lead,
    })
}

/// The lane's lead-in: the tick of its first note, or 0 for an empty
/// lane.
pub fn lead_ticks(midi_notes: &[MidiNote]) -> u64 {
    midi_notes.first().map(|n| n.start_tick).unwrap_or(0)
}

/// Where the rendered audio actually goes on the timeline — each
/// section start advanced by the lane's first note. See
/// [`vocal_audio_start`](crate::compose::vocal_svs::vocal_audio_start)
/// for why the offset is needed at all (ba doc #272 V-1). Only the audio
/// moves: the lane's MIDI clip still starts at the section boundary and
/// carries its own per-note ticks.
pub fn placement_audio_starts(
    tempo_map: &TempoMap,
    placement_starts: &[(u64, u64)],
    lead_ticks: u64,
    engine_sample_rate: u32,
) -> Vec<(u64, u64)> {
    placement_starts
        .iter()
        .map(|&(placement_id, section_start)| {
            (
                placement_id,
                vocal_svs::vocal_audio_start(
                    tempo_map,
                    section_start,
                    lead_ticks,
                    engine_sample_rate,
                ),
            )
        })
        .collect()
}

/// The epoch a render about to be queued should carry, given the lane's
/// current one (`None` for a lane that has never rendered).
///
/// Stale-result protection: a result whose epoch no longer matches the
/// lane's is discarded on arrival, so back-to-back presses can't stack
/// clips. Wrapping keeps that a pure counter comparison — a lane would
/// have to render 2^64 times for a stale epoch to alias the live one.
pub fn next_render_epoch(current: Option<u64>) -> u64 {
    current.unwrap_or(0).wrapping_add(1)
}

/// Resolve the clip's per-note pronunciation and gate every phoneme
/// through the active voicebank, returning the substituted syllable
/// stream ready for the render or a user-facing error listing the
/// syllables that can't be sung. (#494)
///
/// Precedence is `override > project-dict > global-dict > CMU-auto`: the
/// clip's overrides (only present for a re-render of an edited clip) win,
/// then the project dictionary. The global (user-config) dictionary has
/// no on-disk store yet — a later todo wires it; until then it is empty.
fn resolve_and_validate(
    params: &VocalParams,
    annotations: &[String],
    note_count: usize,
    overrides: &HashMap<usize, SyllableOverride>,
    project_dictionary: &[DictionaryEntry],
) -> Result<Vec<AssignedSyllable>, String> {
    let resolved = vocal_svs::resolve_clip_pronunciation(
        &params.draft,
        annotations,
        note_count,
        overrides,
        project_dictionary,
        &[],
    );
    vocal_svs::validate_for_voicebank(&resolved, params.voicebank)
        .map_err(|invalid| describe_invalid(&invalid))
}

/// Render the blocked-phoneme report into a single status-bar line. Caps
/// the number of syllables spelled out so a wholesale-bad draft doesn't
/// produce a wall of text.
pub fn describe_invalid(invalid: &[InvalidSyllable]) -> String {
    const MAX_SHOWN: usize = 6;
    let shown = invalid.len().min(MAX_SHOWN);
    let mut parts: Vec<String> = invalid
        .iter()
        .take(shown)
        .map(|s| {
            let label = if s.label.is_empty() {
                "?".to_string()
            } else {
                s.label.clone()
            };
            format!(
                "note {} \u{201c}{}\u{201d}: {} ({})",
                s.note_index + 1,
                label,
                s.phoneme,
                s.reason.as_str()
            )
        })
        .collect();
    if invalid.len() > shown {
        parts.push(format!("+{} more", invalid.len() - shown));
    }
    format!(
        "Can\u{2019}t render vocals \u{2014} {} phoneme(s) the voicebank can\u{2019}t sing: {}",
        invalid.len(),
        parts.join("; ")
    )
}
