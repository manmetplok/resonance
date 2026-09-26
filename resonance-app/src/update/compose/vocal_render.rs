//! Vocal-lane render orchestration. Splits cleanly from the
//! message-routing `lane_inspector` module: this file owns the
//! side-effects of producing audible vocals — deriving the MIDI clip,
//! queuing the off-thread SVS render, installing the resulting WAV at
//! every section placement, and the lifecycle bookkeeping (epochs,
//! tear-downs) that keeps back-to-back regen presses from stacking
//! clips.
//!
//! Collaborators carry the pieces that aren't orchestration
//! (ba todo #1259):
//!
//! - [`vocal_render_plan`](super::vocal_render_plan) — the pure decision
//!   step: pronunciation resolution + voicebank gate, timeline placement
//!   maths, the render epoch.
//! - [`vocal_audio_io`](super::vocal_audio_io) — everything that touches
//!   the disk: destination directory, WAV write, superseded-file unlink.
//! - [`vocal_midi_install`](super::vocal_midi_install) /
//!   [`vocal_audio_install`](super::vocal_audio_install) — installing the
//!   derived MIDI clip and the rendered audio at every placement.
//! - [`vocal_control`](super::vocal_control) — which lane a control-API
//!   call addresses.

use iced::Task;

use resonance_audio::types::TrackId;

use crate::compose::{ComposeMessage, LaneGeneratorKind};
use crate::message::Message;

use super::vocal_audio_install::tear_down_old_vocal_audio;
use super::vocal_audio_io;
use super::vocal_midi_install::VocalMidiInstall;
use super::vocal_render_plan::{self as plan, VocalRenderPlan};

/// Vocal-lane lookups, re-exported so the control layer's existing
/// `update::compose::vocal_render::*` paths keep resolving after the
/// split (ba todo #1259). They live in
/// [`vocal_control`](super::vocal_control) now.
pub(crate) use super::vocal_control::{
    all_vocal_lanes, first_vocal_definition, track_has_vocal_lane, vocal_definitions_for_track,
};

/// Roll a fresh lyric draft for the vocal lane. Bumps the seed first so
/// repeated presses don't produce the same draft. Locked lines stay put
/// — `generate_lyrics` preserves them and anchors the rhyme pattern to
/// their bucket.
pub(super) fn roll_vocal_lyrics(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    seed_mix: u64,
) {
    let Some(def) = r.compose.find_definition_mut(definition_id) else {
        return;
    };
    let Some(cfg) = def.lane_generators.get_mut(&track_id) else {
        return;
    };
    let LaneGeneratorKind::Vocal(params) = &mut cfg.kind else {
        return;
    };
    cfg.seed = crate::util::bump_seed(cfg.seed, seed_mix);
    let seed = cfg.seed;
    params.draft = resonance_music_theory::generate_lyrics(params, seed);
    r.compose.last_error = None;
}

/// Generate a fresh melody MIDI clip for the vocal lane and queue the
/// SVS audio render off-thread. The MIDI side is installed synchronously
/// so the staff updates immediately; the WAV arrives later via the
/// `VocalAudioReady` message dispatched by the returned `Task`.
///
/// Uses the lane config's *current* seed — callers that want a fresh
/// random surface must call `bump_lane_seed` beforehand. This split
/// avoids the previous double-bump where `Regenerate → regenerate_lane
/// → roll_vocal_melody` all bumped the seed in turn.
pub(super) fn roll_vocal_melody(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};

    if !super::track_exists(r, track_id) {
        return Task::none();
    }
    let Some(mut def) = r.compose.find_definition(definition_id).cloned() else {
        return Task::none();
    };
    let Some(cfg) = def.lane_generators.get(&track_id).cloned() else {
        return Task::none();
    };
    let LaneGeneratorKind::Vocal(params) = cfg.kind else {
        return Task::none();
    };
    if let Err(e) = params.validate() {
        r.compose.last_error = Some(format!("Vocal params invalid: {e}"));
        return Task::none();
    }
    if def.chords.is_empty() || params.draft.is_empty() {
        return Task::none();
    }

    // Constrain the vocal melody to the user's pinned chord-track harmony
    // (doc #168, todo #445), mirroring the instrument lanes in
    // `regenerate_lane`.
    super::regenerate::apply_chord_track_harmony(r, definition_id, &mut def);

    let timed = crate::compose::generate::to_timed_chords(&def.chords);
    let meter = super::section_meter(r, definition_id);
    let beats_per_bar = meter.numerator as u32;
    let motif_intervals: Vec<i8> = timed
        .first()
        .map(|first| {
            resonance_music_theory::motif_intervals(
                &def.motif_source,
                first.chord,
                def.scale,
            )
        })
        .unwrap_or_default();
    let notes = resonance_music_theory::derive_vocal_with_motif(
        &timed,
        &params,
        TICKS_PER_QUARTER_NOTE as u32,
        beats_per_bar,
        Some(&motif_intervals),
        cfg.seed,
    );
    if notes.is_empty() {
        return Task::none();
    }

    let duration_ticks = def.length_bars as u64 * meter.numerator as u64 * TICKS_PER_QUARTER_NOTE;

    let track_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("Vocal");
    let name = format!("{} \u{00B7} {}", def.name, track_name);

    let midi_notes: Vec<MidiNote> = notes
        .iter()
        .map(|n| MidiNote {
            note: n.note,
            velocity: n.velocity,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect();

    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();

    let placement_starts: Vec<(u64, u64)> = placements
        .iter()
        .map(|(pid, start_bar)| (*pid, r.tempo_map.bar_to_sample(*start_bar)))
        .collect();
    let initial_lyrics: Vec<String> = vec![String::new(); midi_notes.len()];
    VocalMidiInstall {
        definition_id,
        track_id,
        placements: &placement_starts,
        duration_ticks,
        midi_notes: &midi_notes,
        lyrics: &initial_lyrics,
        name: &name,
    }
    .install(r);
    enqueue_vocal_render(
        r,
        VocalRenderRequest {
            definition_id,
            track_id,
            midi_notes,
            lyrics: initial_lyrics,
            params,
            placement_starts,
            clip_name: name,
            // A fresh roll has no edited clip yet, so no per-syllable
            // overrides apply — only the project dictionary + auto path.
            clip_id: None,
        },
    )
}

/// One lane's request to render: what to sing, and where its audio
/// belongs. Bundled rather than passed as nine parallel arguments —
/// [`enqueue_vocal_render`] hands most of it straight to the planner and
/// the off-thread job.
struct VocalRenderRequest {
    definition_id: u64,
    track_id: TrackId,
    midi_notes: Vec<resonance_audio::types::MidiNote>,
    /// Per-note lyric annotations overlaying the lane's draft.
    lyrics: Vec<String>,
    params: resonance_music_theory::VocalParams,
    /// `(placement_id, section_start_sample)` for every placement.
    placement_starts: Vec<(u64, u64)>,
    clip_name: String,
    /// The clip being re-rendered, if any — the source of per-syllable
    /// pronunciation overrides.
    clip_id: Option<resonance_audio::types::ClipId>,
}

/// Shared off-thread vocal render path: plan, tear down the prior audio,
/// bump the in-flight epoch (stale-result protection against back-to-back
/// presses), then spawn the SVS pipeline on a blocking thread. The two
/// callers — `roll_vocal_melody` (full regenerate) and
/// `rerender_vocal_audio` (notes-only) — differ only in how they produce
/// `midi_notes` and `lyrics`; everything after that is identical, so it
/// lives here.
fn enqueue_vocal_render(r: &mut crate::Resonance, req: VocalRenderRequest) -> Task<Message> {
    // Resolve pronunciation and gate every phoneme through the active
    // voicebank *before* touching the existing audio. A blocked phoneme
    // aborts the render with a report and leaves the current clip intact
    // rather than corrupting the segment. (#494)
    let plan = match plan_render(r, &req) {
        Ok(plan) => plan,
        Err(message) => {
            r.compose.last_error = Some(message);
            return Task::none();
        }
    };

    tear_down_old_vocal_audio(r, req.definition_id, req.track_id);
    let render_epoch = bump_render_epoch(r, req.definition_id, req.track_id);
    r.compose.last_error = None;

    let job = vocal_audio_io::VocalRenderJob {
        assigned: plan.assigned,
        // The lane's editable expression overlay (dynamics/tension/
        // breathiness/pitch bend, doc #154). Cloned out of compose state
        // before the render moves off-thread; an un-edited lane has no
        // entry, so a default (all-`Auto`) bundle reproduces the
        // pre-overlay audio exactly.
        curves: r
            .compose
            .expression_curves(req.definition_id, req.track_id)
            .cloned()
            .unwrap_or_default(),
        // The section's tempo, not the one under the playhead (VIEW-13).
        bpm: super::section_meter(r, req.definition_id).bpm,
        engine_sample_rate: r.sample_rate,
        dest_dir: vocal_audio_io::vocal_audio_dir(r.io.project_path.as_deref()),
        render_cache: render_cache_for(r, req.definition_id, req.track_id),
        midi_notes: req.midi_notes,
        params: req.params,
    };
    spawn_render(
        job,
        req.definition_id,
        req.track_id,
        req.clip_name,
        plan.audio_starts,
        render_epoch,
    )
}

/// Adapter from app state to the pure planner: pulls the lane's
/// pronunciation overrides and project dictionary, the tempo map and the
/// engine rate out of `r` and hands them over.
fn plan_render(
    r: &crate::Resonance,
    req: &VocalRenderRequest,
) -> Result<VocalRenderPlan, String> {
    let empty = std::collections::HashMap::new();
    let overrides = req
        .clip_id
        .and_then(|c| r.compose.pronunciation.clip_overrides(c))
        .unwrap_or(&empty);
    plan::plan_vocal_render(plan::VocalRenderPlanInputs {
        tempo_map: &r.tempo_map,
        engine_sample_rate: r.sample_rate,
        params: &req.params,
        annotations: &req.lyrics,
        midi_notes: &req.midi_notes,
        placement_starts: &req.placement_starts,
        overrides,
        project_dictionary: &r.compose.pronunciation.project_dictionary,
    })
}

/// Advance the lane's render epoch and return the value the queued
/// render carries. A result arriving with any other epoch is stale and
/// gets dropped by
/// [`handle_vocal_audio_ready`](super::vocal_audio_install::handle_vocal_audio_ready).
fn bump_render_epoch(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> u64 {
    let entry = r
        .compose
        .vocal_audio
        .render_epoch
        .entry((definition_id, track_id))
        .or_insert(0);
    *entry = plan::next_render_epoch(Some(*entry));
    *entry
}

/// The lane's per-clip content-addressed render cache: an edit only
/// re-renders the sub-clip segments it touched. Shared into the blocking
/// render thread via `Arc<Mutex>`; the cache's `last_plan` is read back
/// afterwards for the "N of M segments changed" overlay (#495).
fn render_cache_for(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> std::sync::Arc<std::sync::Mutex<crate::compose::vocal_svs::SvsRenderCache>> {
    r.compose
        .vocal_audio
        .render_cache
        .entry((definition_id, track_id))
        .or_default()
        .clone()
}

/// Run `job` on a blocking thread and map its outcome onto the message
/// that installs (or reports) the result.
fn spawn_render(
    job: vocal_audio_io::VocalRenderJob,
    definition_id: u64,
    track_id: TrackId,
    clip_name: String,
    audio_starts: Vec<(u64, u64)>,
    render_epoch: u64,
) -> Task<Message> {
    use crate::compose::messages::VocalAudioReadyData;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || job.run())
                .await
                .unwrap_or_else(|join_err| Err(format!("vocal render task join: {join_err}")))
        },
        move |result| match result {
            Ok(Some((wav_path, trim_start, trim_end))) => Message::Compose(
                ComposeMessage::VocalAudioReady(Box::new(VocalAudioReadyData {
                    definition_id,
                    track_id,
                    wav_path,
                    placements: audio_starts.clone(),
                    clip_name: clip_name.clone(),
                    trim_start_frames: trim_start,
                    trim_end_frames: trim_end,
                    render_epoch,
                })),
            ),
            Ok(None) => Message::Tick,
            Err(error) => Message::Compose(ComposeMessage::VocalAudioFailed {
                definition_id,
                track_id,
                render_epoch,
                error,
            }),
        },
    )
}

/// Re-run the SVS render on the *existing* MIDI clip for this vocal
/// lane, without re-deriving notes or rolling lyrics. Used when the
/// user has hand-edited notes in the vocal roll and wants to hear
/// what those edits sound like.
pub(super) fn rerender_vocal_audio(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    use resonance_audio::types::MidiNote;

    if !super::track_exists(r, track_id) {
        return Task::none();
    }
    let Some(def) = r.compose.find_definition(definition_id).cloned() else {
        return Task::none();
    };
    let Some(cfg) = def.lane_generators.get(&track_id).cloned() else {
        return Task::none();
    };
    let LaneGeneratorKind::Vocal(params) = cfg.kind else {
        return Task::none();
    };

    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();
    if placements.is_empty() {
        r.compose.last_error =
            Some("Place this section before re-rendering vocals.".to_string());
        return Task::none();
    }

    let Some(clip_id) = lane_midi_clip(r, definition_id, track_id, &placements) else {
        r.compose.last_error =
            Some("Generate a vocal first \u{2014} no MIDI clip to render.".to_string());
        return Task::none();
    };
    let (midi_notes, clip_name) = {
        let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
            r.compose.last_error =
                Some("Vocal MIDI clip vanished \u{2014} regenerate the melody.".to_string());
            return Task::none();
        };
        if clip.notes.is_empty() {
            r.compose.last_error = Some(
                "Vocal MIDI clip has no notes \u{2014} draw or generate before rendering."
                    .to_string(),
            );
            return Task::none();
        }
        let notes: Vec<MidiNote> = clip.notes.clone();
        (notes, clip.name.clone())
    };
    let lyrics = r
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip_id)
        .cloned()
        .unwrap_or_else(|| vec![String::new(); midi_notes.len()]);

    let placement_starts: Vec<(u64, u64)> = placements
        .iter()
        .map(|(pid, start_bar)| (*pid, r.tempo_map.bar_to_sample(*start_bar)))
        .collect();

    enqueue_vocal_render(
        r,
        VocalRenderRequest {
            definition_id,
            track_id,
            midi_notes,
            lyrics,
            params,
            placement_starts,
            clip_name,
            // Re-rendering an existing clip: its per-syllable pronunciation
            // overrides (if any) apply on top of the dictionary + auto path.
            clip_id: Some(clip_id),
        },
    )
}

/// The MIDI clip a vocal lane renders from.
///
/// Prefers the derived-clip mapping, but falls back to matching a clip by
/// the placement's start sample. The mapping is not always populated for
/// a lane loaded from disk, and without this fallback such a lane is
/// permanently unrenderable while `song.vocal` cheerfully reports it as
/// having notes — the two must agree. `song.vocal`'s `lane_clip` and the
/// `lane_has_notes` pre-flight resolve it the same way.
fn lane_midi_clip(
    r: &crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    placements: &[(u64, u32)],
) -> Option<resonance_audio::types::ClipId> {
    placements
        .iter()
        .find_map(|(pid, _)| {
            r.compose
                .derived_clips
                .get(&(definition_id, *pid, track_id))
                .copied()
        })
        .or_else(|| {
            placements.iter().find_map(|(_, start_bar)| {
                let start = r.tempo_map.bar_to_sample(*start_bar);
                r.midi_clips
                    .iter()
                    .find(|c| {
                        c.track_id == track_id
                            && c.start_sample == start
                            // Never adopt a clip another lane already owns.
                            && !r.compose.derived_clips.values().any(|id| *id == c.id)
                    })
                    .map(|c| c.id)
            })
        })
}
