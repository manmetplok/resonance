//! Per-lane vocal render state and the debounced GUI re-render
//! (vocal-mcp-control-and-quality.md §7, D19–D22).
//!
//! - **Staleness** compares the unit content keys of the take a lane
//!   plays (recorded per installed clip when it lands) with the keys its
//!   current content plans to. Planning builds every unit's segment, so
//!   the expected keys are memoised by a **fingerprint** of everything
//!   the plan reads — the clip's notes and lyrics, the lane's params,
//!   pronunciation overrides, the project dictionary, expression curves,
//!   the section tempo and the bank that renders it — and rebuilt only
//!   when that changes.
//! - **Origin.** [`settle`] runs once after every outermost
//!   `Resonance::update`; the caller says whether that update came from
//!   the GUI or from a control request (`Message::Control`, which wraps
//!   every MCP edit, nested `run_via_update` dispatches included). A lane
//!   whose content a **GUI** update changed, and that has audio to bring
//!   up to date (or a failed render to retry), gets a re-render armed
//!   [`AUTO_RENDER_DEBOUNCE`] after the last such edit; the render cache
//!   re-synthesises only the units that changed. A **control** edit only
//!   makes the lane `stale` — an agent batches its edits and calls
//!   `vocal.render` once.
//! - **Errors** are the lane's last render failure, kept with the
//!   fingerprint it failed on: a successful render or an edit that
//!   changes the lane retires it.

use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use iced::Task;
use resonance_audio::types::{TrackId, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::VocalParams;

use crate::compose::vocal_render_status::{ExpectedUnits, VocalLaneError};
use crate::compose::{
    ComposeMessage, LaneGeneratorKind, VocalLaneErrorKind, VocalLaneKey, VocalLaneRenderState,
    VocalLaneRenderStatus,
};
use crate::message::Message;

use super::vocal_render_plan as plan;

/// How long after the last GUI edit of a lane its re-render starts.
pub(crate) const AUTO_RENDER_DEBOUNCE: std::time::Duration =
    std::time::Duration::from_millis(750);

/// Every vocal lane, in definition order.
fn vocal_lanes(r: &crate::Resonance) -> Vec<VocalLaneKey> {
    r.compose
        .definitions
        .iter()
        .flat_map(|def| {
            def.lane_generators
                .iter()
                .filter(|(_, cfg)| matches!(cfg.kind, LaneGeneratorKind::Vocal(_)))
                .map(move |(track_id, _)| (def.id, *track_id))
        })
        .collect()
}

fn lane_params(
    r: &crate::Resonance,
    (definition_id, track_id): VocalLaneKey,
) -> Option<&VocalParams> {
    let cfg = r
        .compose
        .find_definition(definition_id)?
        .lane_generators
        .get(&track_id)?;
    match &cfg.kind {
        LaneGeneratorKind::Vocal(params) => Some(params),
        _ => None,
    }
}

/// The lane's MIDI clip, resolved the way a re-render resolves it.
fn lane_clip(
    r: &crate::Resonance,
    (definition_id, track_id): VocalLaneKey,
) -> Option<&crate::state::MidiClipState> {
    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();
    let clip_id = super::vocal_render::lane_midi_clip(r, definition_id, track_id, &placements)?;
    r.midi_clips.iter().find(|c| c.id == clip_id)
}

/// Feeds `Debug` output into a hasher without building the string.
struct DebugHasher<'a, H: Hasher>(&'a mut H);

impl<H: Hasher> std::fmt::Write for DebugHasher<'_, H> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}

/// A hash of everything the lane's render plan reads. Equal fingerprints
/// plan to equal unit keys; the converse need not hold (a lyric-generator
/// setting changes the fingerprint and no key), which only costs a
/// recompute.
pub(crate) fn lane_fingerprint(r: &crate::Resonance, lane: VocalLaneKey) -> u64 {
    let (definition_id, track_id) = lane;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let Some(params) = lane_params(r, lane) else {
        return 0;
    };
    let _ = write!(DebugHasher(&mut h), "{params:?}");
    let bank = r.voicebanks.profile(&params.voicebank);
    crate::compose::vocal_svs::model_identity(params, &bank).hash(&mut h);
    if let Some(installed) = bank.installed() {
        installed.stamp.hash(&mut h);
    }
    match lane_clip(r, lane) {
        Some(clip) => {
            clip.id.hash(&mut h);
            for n in clip.notes.iter() {
                (n.note, n.velocity.to_bits(), n.start_tick, n.duration_ticks).hash(&mut h);
            }
            r.compose.vocal_audio.clip_lyrics.get(&clip.id).hash(&mut h);
        }
        None => u64::MAX.hash(&mut h),
    }
    if let Some(overrides) = r.compose.pronunciation.lane_overrides(definition_id, track_id) {
        let mut sorted: Vec<_> = overrides.iter().collect();
        sorted.sort_unstable_by_key(|(index, _)| **index);
        let _ = write!(DebugHasher(&mut h), "{sorted:?}");
    }
    let _ = write!(
        DebugHasher(&mut h),
        "{:?}|{:?}",
        r.compose.pronunciation.project_dictionary,
        r.compose.expression_curves(definition_id, track_id),
    );
    super::section_meter(r, definition_id).bpm.to_bits().hash(&mut h);
    h.finish()
}

/// The unit keys the lane's content renders to now, in unit order —
/// built exactly as `enqueue_vocal_render` plans the render that would
/// produce them. `None` when no plan can be built: no clip or notes, or a
/// phoneme the bank cannot sing.
pub(crate) fn expected_unit_keys(r: &crate::Resonance, lane: VocalLaneKey) -> Option<Arc<[u64]>> {
    let (definition_id, track_id) = lane;
    let params = lane_params(r, lane)?;
    let clip = lane_clip(r, lane)?;
    if clip.notes.is_empty() {
        return None;
    }
    let lyrics = r
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .cloned()
        .unwrap_or_else(|| vec![String::new(); clip.notes.len()]);
    let bank = r.voicebanks.profile(&params.voicebank);
    let empty = std::collections::HashMap::new();
    let overrides = r
        .compose
        .pronunciation
        .lane_overrides(definition_id, track_id)
        .unwrap_or(&empty);
    let planned = plan::plan_vocal_render(plan::VocalRenderPlanInputs {
        tempo_map: &r.tempo_map,
        engine_sample_rate: r.sample_rate,
        params,
        bank: &bank,
        annotations: &lyrics,
        midi_notes: &clip.notes,
        placement_starts: &[],
        overrides,
        project_dictionary: &r.compose.pronunciation.project_dictionary,
    })
    .ok()?;
    let curves = r
        .compose
        .expression_curves(definition_id, track_id)
        .cloned()
        .unwrap_or_default();
    Some(
        crate::compose::vocal_svs::plan_unit_keys(
            &clip.notes,
            params,
            &planned.assigned,
            &curves,
            TICKS_PER_QUARTER_NOTE as u32,
            super::section_meter(r, definition_id).bpm,
            &bank,
        )
        .into(),
    )
}

/// The installed take's keys, `None` for a lane with no audio, and
/// `Some(None)` for audio whose keys are unknown (saved before render
/// state existed, or a test stand-in).
fn installed_keys(r: &crate::Resonance, lane: VocalLaneKey) -> Option<Option<Arc<[u64]>>> {
    let audio = &r.compose.vocal_audio;
    let clips: Vec<_> = audio
        .clips
        .iter()
        .filter(|((d, _, t), _)| (*d, *t) == lane)
        .map(|(_, (clip_id, _))| clip_id)
        .collect();
    if clips.is_empty() {
        return None;
    }
    Some(audio.status.installed_keys(clips).cloned())
}

/// The lane's state from its fingerprint and expected keys.
fn status_from(
    r: &crate::Resonance,
    lane: VocalLaneKey,
    fingerprint: u64,
    expected: Option<&Arc<[u64]>>,
) -> VocalLaneRenderStatus {
    let audio = &r.compose.vocal_audio;
    let total_units = expected.map_or(0, |k| k.len());
    let mut status = VocalLaneRenderStatus {
        state: VocalLaneRenderState::NotRendered,
        stale_units: 0,
        total_units,
        error: None,
    };
    if audio.in_flight_render.contains_key(&lane) {
        status.state = VocalLaneRenderState::Rendering;
        return status;
    }
    if let Some(error) = audio.status.errors.get(&lane) {
        if error.fingerprint == fingerprint {
            status.state = VocalLaneRenderState::Error;
            status.error = Some((error.kind, error.message.clone()));
            return status;
        }
    }
    let Some(installed) = installed_keys(r, lane) else {
        return status;
    };
    match (installed, expected) {
        (Some(installed), Some(expected)) if installed[..] == expected[..] => {
            status.state = VocalLaneRenderState::Rendered;
        }
        (Some(installed), Some(expected)) => {
            status.state = VocalLaneRenderState::Stale;
            status.stale_units = expected.iter().filter(|k| !installed.contains(k)).count();
        }
        (None, Some(expected)) => {
            status.state = VocalLaneRenderState::Stale;
            status.stale_units = expected.len();
        }
        (_, None) => status.state = VocalLaneRenderState::Stale,
    }
    status
}

/// The lane's render state now. Reads the memo when the lane has not
/// changed since the last [`settle`], and plans afresh otherwise (a
/// control read in the middle of an update).
pub(crate) fn lane_render_status(
    r: &crate::Resonance,
    lane: VocalLaneKey,
) -> VocalLaneRenderStatus {
    let fingerprint = lane_fingerprint(r, lane);
    match r.compose.vocal_audio.status.expected.get(&lane) {
        Some(memo) if memo.fingerprint == fingerprint => {
            status_from(r, lane, fingerprint, memo.keys.as_ref())
        }
        _ => {
            let expected = expected_unit_keys(r, lane);
            status_from(r, lane, fingerprint, expected.as_ref())
        }
    }
}

/// Bring every lane's memo and settled status up to date after an
/// update, and arm a debounced re-render for each lane a GUI update
/// changed (see the module doc). `gui` is false for a control request.
pub(crate) fn settle(r: &mut crate::Resonance, gui: bool) -> Task<Message> {
    let lanes = vocal_lanes(r);
    let mut tasks = Vec::new();
    for &lane in &lanes {
        let fingerprint = lane_fingerprint(r, lane);
        let memo = r.compose.vocal_audio.status.expected.get(&lane);
        let previous = memo.map(|m| m.fingerprint);
        let keys = match memo {
            Some(m) if m.fingerprint == fingerprint => m.keys.clone(),
            _ => {
                let keys = expected_unit_keys(r, lane);
                r.compose.vocal_audio.status.expected.insert(
                    lane,
                    ExpectedUnits {
                        fingerprint,
                        keys: keys.clone(),
                    },
                );
                keys
            }
        };
        // `previous` is `None` on the first sight of a lane (startup, a
        // load, a new lane): nothing was edited.
        let edited = previous.is_some_and(|p| p != fingerprint);
        if gui && edited && wants_auto_render(r, lane, fingerprint, keys.as_ref()) {
            tasks.push(arm_auto_render(r, lane));
        }
        let status = status_from(r, lane, fingerprint, keys.as_ref());
        r.compose.vocal_audio.status.lanes.insert(lane, status);
    }
    let status = &mut r.compose.vocal_audio.status;
    status.expected.retain(|lane, _| lanes.contains(lane));
    status.lanes.retain(|lane, _| lanes.contains(lane));
    status.errors.retain(|lane, _| lanes.contains(lane));
    status.auto_render.retain(|lane, _| lanes.contains(lane));
    Task::batch(tasks)
}

/// Whether a GUI edit that changed the lane should re-render it: the lane
/// has audio, a render running, or a failed render to bring up to date —
/// a lane never rendered waits for Generate / Re-render — and its content
/// no longer matches that (a setting that renders the same units, like
/// the lyric generator's theme, changes nothing audible).
fn wants_auto_render(
    r: &crate::Resonance,
    lane: VocalLaneKey,
    fingerprint: u64,
    expected: Option<&Arc<[u64]>>,
) -> bool {
    let audio = &r.compose.vocal_audio;
    if audio.in_flight_render.contains_key(&lane) {
        // A render queued by this very edit (Generate) already covers it.
        return audio.status.queued_fingerprint.get(&lane) != Some(&fingerprint);
    }
    match installed_keys(r, lane) {
        Some(Some(installed)) => expected.is_none_or(|e| installed[..] != e[..]),
        Some(None) => true,
        None => audio.status.errors.contains_key(&lane),
    }
}

/// Arm (or re-arm) the lane's debounce timer: only the newest generation
/// renders when its message arrives.
fn arm_auto_render(r: &mut crate::Resonance, lane: VocalLaneKey) -> Task<Message> {
    let status = &mut r.compose.vocal_audio.status;
    status.next_generation = status.next_generation.wrapping_add(1);
    let generation = status.next_generation;
    status.auto_render.insert(lane, generation);
    let (definition_id, track_id) = lane;
    // The timer is created when the task first runs, on iced's runtime.
    Task::perform(async { tokio::time::sleep(AUTO_RENDER_DEBOUNCE).await }, move |()| {
        Message::Compose(ComposeMessage::VocalAutoRender {
            definition_id,
            track_id,
            generation,
        })
    })
}

/// A lane's debounce timer ran out: re-render it if this is still its
/// newest timer and the lane still needs it.
pub(super) fn auto_render_due(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    generation: u64,
) -> Task<Message> {
    let lane = (definition_id, track_id);
    let status = &mut r.compose.vocal_audio.status;
    if status.auto_render.get(&lane) != Some(&generation) {
        return Task::none();
    }
    status.auto_render.remove(&lane);
    let fingerprint = lane_fingerprint(r, lane);
    let expected = expected_unit_keys(r, lane);
    if !wants_auto_render(r, lane, fingerprint, expected.as_ref()) {
        return Task::none();
    }
    super::vocal_render::rerender_vocal_audio(r, definition_id, track_id)
}

/// A render was queued for the lane: remember the content it plans from.
pub(super) fn note_render_queued(r: &mut crate::Resonance, lane: VocalLaneKey) {
    let fingerprint = lane_fingerprint(r, lane);
    r.compose
        .vocal_audio
        .status
        .queued_fingerprint
        .insert(lane, fingerprint);
}

/// The lane's render landed: its failure, if any, is over.
pub(super) fn note_render_installed(r: &mut crate::Resonance, lane: VocalLaneKey) {
    let status = &mut r.compose.vocal_audio.status;
    status.errors.remove(&lane);
    status.queued_fingerprint.remove(&lane);
}

/// The lane's render failed before it was queued, on its content now.
pub(super) fn record_lane_error(
    r: &mut crate::Resonance,
    lane: VocalLaneKey,
    kind: VocalLaneErrorKind,
    message: String,
) {
    let fingerprint = lane_fingerprint(r, lane);
    r.compose.vocal_audio.status.errors.insert(
        lane,
        VocalLaneError {
            kind,
            message,
            fingerprint,
        },
    );
}

/// The lane's queued render failed: the failure belongs to the content it
/// was planned from, so an edit made while it ran is not marked failed.
pub(super) fn record_render_failure(
    r: &mut crate::Resonance,
    lane: VocalLaneKey,
    kind: VocalLaneErrorKind,
    message: String,
) {
    let fingerprint = match r.compose.vocal_audio.status.queued_fingerprint.remove(&lane) {
        Some(fingerprint) => fingerprint,
        None => lane_fingerprint(r, lane),
    };
    r.compose.vocal_audio.status.errors.insert(
        lane,
        VocalLaneError {
            kind,
            message,
            fingerprint,
        },
    );
}
