//! Audio clip handlers: move/trim/delete, mmap-backed load from a WAV
//! file on disk (project load, imports go through the pool — see
//! `import_pool.rs`), and ensure-all-clips-have-wav-files (project save).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use hound::{SampleFormat, WavSpec, WavWriter};
use parking_lot::RwLock;
use thiserror::Error;

use crate::types::*;

use resonance_dsp::tempo::{detect_tempo_default, TempoEstimate};

use super::internal::{EngineInternal, LoadedClip};
use super::thread::{HandlerCtx, HandlerState};

/// How long a clip edit waits for its clip to finish loading before it
/// is given up on. Loading is an mmap plus a waveform decimation on a
/// worker thread — milliseconds for a normal clip, seconds for a very
/// long one on a busy queue. Ten seconds is far past either, and a
/// command that waits that long has lost its clip for good.
const DEFERRED_CLIP_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A clip edit parked until its clip exists (ba doc #276 BUG 1).
pub struct DeferredClipCommand {
    pub clip_id: ClipId,
    pub command: AudioCommand,
    pub parked_at: std::time::Instant,
}

/// Raise the clip-id allocator past every `audio/clip_{id}.wav` in a
/// project dir the engine is pointed at, so a new clip never overwrites
/// the WAV of a clip deleted before the last save — one a versioned
/// backup can still reference (code review STATE-08). Synchronous; the
/// `SetProjectDir` handler uses [`start_clip_id_scan`] instead. A missing
/// dir reserves nothing.
pub(crate) fn reserve_clip_ids_in_project_dir(state: &mut HandlerState, dir: &Path) {
    reserve_clip_ids_up_to(state, highest_clip_id_on_disk(dir));
}

fn highest_clip_id_on_disk(dir: &Path) -> Option<ClipId> {
    std::fs::read_dir(dir.join("audio"))
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            name.strip_prefix("clip_")?.strip_suffix(".wav")?.parse::<ClipId>().ok()
        })
        .filter(|id| *id < DERIVED_CLIP_ID_BASE)
        .max()
}

fn reserve_clip_ids_up_to(state: &mut HandlerState, highest: Option<ClipId>) {
    if let Some(id) = highest {
        reserve_clip_id(&mut state.next_clip_id, id);
    }
}

/// Keep the engine's clip-id counter above `id` — a clip it was handed
/// or found on disk — unless `id` is app-owned ([`DERIVED_CLIP_ID_BASE`]
/// and up): the app allocates those itself, and bumping past one would
/// put the next engine allocation onto the id the app hands out next
/// (code review FU-A6a). The same rule the track, bus and send hints
/// follow.
pub(crate) fn reserve_clip_id(next_clip_id: &mut ClipId, id: ClipId) {
    if id < DERIVED_CLIP_ID_BASE {
        *next_clip_id = (*next_clip_id).max(id + 1);
    }
}

/// `SetProjectDir`: run the STATE-08 folder scan on a worker (FU-M12b) —
/// a project on a slow or network disk with thousands of WAVs used to
/// stall the engine command thread. The reservation lands through
/// [`settle_clip_id_scan`]: from the engine loop once the scan is done,
/// or — waiting for it — from any clip-id allocation that comes first,
/// so an id is never issued before the scan could reserve it.
pub(crate) fn start_clip_id_scan(state: &mut HandlerState, dir: &Path) {
    settle_clip_id_scan(state, true);
    let owned = dir.to_path_buf();
    match std::thread::Builder::new()
        .name("clip-id-scan".into())
        .spawn(move || highest_clip_id_on_disk(&owned))
    {
        Ok(handle) => state.clip_id_scan = Some(handle),
        Err(_) => reserve_clip_ids_in_project_dir(state, dir),
    }
}

/// Apply a finished [`start_clip_id_scan`]. With `wait`, join one still
/// running first — every clip-id allocation site calls this before it
/// allocates; the engine loop polls without waiting.
pub(crate) fn settle_clip_id_scan(state: &mut HandlerState, wait: bool) {
    let Some(handle) = state.clip_id_scan.take_if(|h| wait || h.is_finished()) else {
        return;
    };
    // A panicked scan reserves nothing, like a missing dir.
    let highest = handle.join().ok().flatten();
    reserve_clip_ids_up_to(state, highest);
}

/// Park `command` until `clip_id` shows up, instead of dropping it.
///
/// The engine's convention everywhere else is that a command naming
/// something that does not exist is a silent no-op — right for a
/// deleted target, wrong for one that has not finished loading yet,
/// which is every clip for the first few milliseconds of its life.
pub(crate) fn defer_clip_command(state: &mut HandlerState, clip_id: ClipId, command: AudioCommand) {
    state.deferred_clip_commands.push(DeferredClipCommand {
        clip_id,
        command,
        parked_at: std::time::Instant::now(),
    });
}

/// True when `clip_id` is already in the engine's clip list.
pub(crate) fn clip_exists(ctx: &HandlerCtx, clip_id: ClipId) -> bool {
    ctx.clips.read().iter().any(|c| c.id == clip_id)
}

/// Refuse a mandatory-id create that collides with a live clip, audio or
/// MIDI — one shared clip-id space (D-6 §1d/§3), so a `CreateMidiClip`
/// must not overwrite an audio clip's id or vice versa. Returns `true`
/// (and emits `EngineErrorKind::Internal`) when the id is already taken,
/// the same "refuse rather than replace the live entity" shape as
/// `engine::tracks::reject_if_track_id_in_use`. The app is the only
/// allocator for this space (`ComposeState::fresh_derived_clip_id`), so a
/// collision here means a caller bug, not a race to recover from.
pub(crate) fn reject_if_clip_id_in_use(ctx: &HandlerCtx, clip_id: ClipId) -> bool {
    let taken = ctx.clips.read().iter().any(|c| c.id == clip_id)
        || ctx.shared.graph.load().midi_clip(clip_id).is_some();
    if taken {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(format!(
            "clip id {clip_id} is already in use; refusing the create rather than replacing the live clip"
        ))));
    }
    taken
}

/// Engine-loop hook: apply parked clip edits whose clip has landed, and
/// give up on ones whose clip never did.
///
/// Order is preserved per clip — the queue is scanned front to back, so
/// a trim followed by a fade lands in that order once the clip appears.
pub(crate) fn poll_deferred_clip_commands(ctx: &HandlerCtx, state: &mut HandlerState) {
    if state.deferred_clip_commands.is_empty() {
        return;
    }
    // A parked delete waits for a split, not for a load, so it neither
    // runs when its clip lands nor expires: it is resolved below, once no
    // split that would create its clip is parked any more.
    let (parked_deletes, rest): (Vec<_>, Vec<_>) =
        std::mem::take(&mut state.deferred_clip_commands)
            .into_iter()
            .partition(|d| matches!(d.command, AudioCommand::DeleteClip { .. }));
    state.deferred_clip_commands = rest;
    let (ready, expired) = {
        let clips = ctx.clips.read();
        let landed = |clip_id: ClipId| clips.iter().any(|c| c.id == clip_id);
        partition_deferred_clip_commands(
            &mut state.deferred_clip_commands,
            landed,
            std::time::Instant::now(),
            DEFERRED_CLIP_COMMAND_TIMEOUT,
        )
    };
    for clip_id in expired {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::not_found(format!(
            "clip {clip_id} never finished loading; an edit aimed at it was dropped"
        ))));
    }
    for command in ready {
        apply_clip_command(ctx, state, command);
    }
    state.deferred_clip_commands.extend(parked_deletes);
    resolve_orphaned_deletes(ctx, state);
}

/// Split the parked queue into commands whose clip has landed (to run
/// now, in the order they were parked) and clips that timed out.
/// Everything else stays parked.
///
/// Pure so the queue's ordering and expiry can be tested without an
/// engine thread; `landed` answers "is this clip in the engine yet".
pub fn partition_deferred_clip_commands(
    deferred: &mut Vec<DeferredClipCommand>,
    landed: impl Fn(ClipId) -> bool,
    now: std::time::Instant,
    timeout: std::time::Duration,
) -> (Vec<AudioCommand>, Vec<ClipId>) {
    let mut ready = Vec::new();
    let mut expired = Vec::new();
    deferred.retain(|d| {
        if landed(d.clip_id) {
            ready.push(d.command.clone());
            return false;
        }
        if now.duration_since(d.parked_at) >= timeout {
            expired.push(d.clip_id);
            return false;
        }
        true
    });
    (ready, expired)
}

/// Run one previously-parked clip command now that its clip exists.
///
/// Only the commands `dispatch_clips` can park are listed; anything else
/// reaching here would be a parking bug rather than a client error.
fn apply_clip_command(ctx: &HandlerCtx, state: &mut HandlerState, command: AudioCommand) {
    match command {
        AudioCommand::MoveClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => handle_move_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimClip {
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        } => handle_trim_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::SplitClip {
            clip_id,
            new_clip_id,
            at_sample,
        } => handle_split_clip(ctx, clip_id, new_clip_id, at_sample),
        AudioCommand::DeleteClip { clip_id } => handle_delete_clip(ctx, state, clip_id),
        AudioCommand::SetClipFade {
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        } => handle_set_clip_fade(
            ctx,
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        ),
        AudioCommand::SetClipGain { clip_id, gain_db } => {
            handle_set_clip_gain(ctx, clip_id, gain_db)
        }
        _ => {}
    }
}

pub(crate) fn handle_move_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    new_track_id: TrackId,
) {
    let mut clips = ctx.clips.write();
    if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
        let _ = ctx.event_tx.send(AudioEvent::ClipMoved {
            clip_id,
            new_start_sample,
            new_track_id,
        });
    }
}

pub(crate) fn handle_trim_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_frames: u64,
    trim_end_frames: u64,
) {
    let mut clips = ctx.clips.write();
    if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_frames = trim_start_frames;
        clip.trim_end_frames = trim_end_frames;
        let _ = ctx.event_tx.send(AudioEvent::ClipTrimmed {
            clip_id,
            new_start_sample,
            new_duration_samples: clip.duration_frames(),
            trim_start_frames,
            trim_end_frames,
        });
    }
}

/// Cut a clip in two at an absolute timeline position (ba doc #275 P2).
///
/// The geometry lives in [`AudioClip::split_tail`] /
/// [`AudioClip::split_head_trim_end`]; this shortens the original to the
/// head in place and appends the tail, echoing a `ClipTrimmed` for the
/// first and a `ClipImported` for the second so the app mirrors both
/// through the paths it already has. A split at or outside either edge
/// is a no-op.
pub(crate) fn handle_split_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_clip_id: ClipId,
    at_sample: u64,
) {
    let mut clips = ctx.clips.write();
    let Some(index) = clips.iter().position(|c| c.id == clip_id) else {
        return;
    };
    let (head_trim_end, tail) = {
        let clip = &clips[index];
        let Some(tail) = clip.split_tail(new_clip_id, at_sample) else {
            return;
        };
        (clip.split_head_trim_end(at_sample), tail)
    };

    let head = &mut clips[index];
    head.trim_end_frames = head_trim_end;
    head.fade_out_frames = 0;
    let head_start = head.start_sample;
    let head_trim_start = head.trim_start_frames;
    let head_duration = head.duration_frames();
    let _ = ctx.event_tx.send(AudioEvent::ClipTrimmed {
        clip_id,
        new_start_sample: head_start,
        new_duration_samples: head_duration,
        trim_start_frames: head_trim_start,
        trim_end_frames: head_trim_end,
    });

    let (track_id, start_sample, duration_samples, name) = (
        tail.track_id,
        tail.start_sample,
        tail.duration_frames(),
        tail.name.clone(),
    );
    let waveform_peaks = crate::types::compute_waveform_peaks(tail.source.as_frames());
    clips.push(tail);
    drop(clips);
    let _ = ctx.event_tx.send(AudioEvent::ClipImported {
        clip_id: new_clip_id,
        track_id,
        start_sample,
        duration_samples,
        name,
        waveform_peaks,
    });
}

/// `DeleteClip`: remove the clip, cancel every load of its id still in
/// flight, and echo `ClipDeleted` — at once, whether or not the clip ever
/// landed (code review FU-A13e, FU-A13f).
///
/// It used to be parked like any other edit of a clip not in the list yet
/// (ba doc #276 BUG 1), which is wrong for a delete twice over:
///
/// * **FU-A13e.** An undo/redo burst over a clip add sends
///   `LoadClipFromWav(id)`, `DeleteClip(id)`, `LoadClipFromWav(id)` inside
///   one load's latency. The parked delete ran against whichever load
///   landed first — possibly the *second* one — and the first load, if it
///   landed later, survived it or was dropped by the second's duplicate
///   check. The engine ended with no clip, or with the wrong load's. Now
///   the delete withdraws the id's load ticket
///   ([`ClipLoadTickets`](super::clip_loads::ClipLoadTickets)) on the
///   engine thread, where every finished load is also applied
///   ([`apply_clip_loaded`]), so every load submitted before it is
///   cancelled and every load after it is not.
/// * **FU-A13f.** A clip whose load never succeeds (missing media) never
///   lands, so its parked delete timed out without an echo, and the app's
///   `RestoreEchoes` ledger kept owing it forever. Now a delete of an id
///   the engine does not hold — never loaded, failed, or cancelled — still
///   echoes, exactly once per `DeleteClip`.
///
/// Edits parked for the id are dropped: they were aimed at the instance
/// being deleted, and replaying them onto a re-load under the same id
/// would edit the wrong clip. The one delete that still waits is one of a
/// clip a parked `SplitClip` is about to create — it runs after the split,
/// in order, as before.
pub(crate) fn handle_delete_clip(ctx: &HandlerCtx, state: &mut HandlerState, clip_id: ClipId) {
    if split_parked_for(state, clip_id) {
        defer_clip_command(state, clip_id, AudioCommand::DeleteClip { clip_id });
        return;
    }
    state.deferred_clip_commands.retain(|d| d.clip_id != clip_id);
    state.clip_load_tickets.withdraw(clip_id);
    ctx.clips.write().retain(|c| c.id != clip_id);
    let _ = ctx.event_tx.send(AudioEvent::ClipDeleted { clip_id });
    // A delete parked behind a split of this clip lost its split with the
    // retain above.
    resolve_orphaned_deletes(ctx, state);
}

/// Run every parked delete whose split is no longer parked — it ran and
/// created the clip, it expired, or its parent was deleted — so a delete
/// never waits on a split that is not coming and always echoes.
fn resolve_orphaned_deletes(ctx: &HandlerCtx, state: &mut HandlerState) {
    let orphaned: Vec<ClipId> = state
        .deferred_clip_commands
        .iter()
        .filter(|d| matches!(d.command, AudioCommand::DeleteClip { .. }))
        .map(|d| d.clip_id)
        .filter(|&id| !split_parked_for(state, id))
        .collect();
    for id in orphaned {
        handle_delete_clip(ctx, state, id);
    }
}

/// A parked `SplitClip` will create `clip_id` once its parent lands.
fn split_parked_for(state: &HandlerState, clip_id: ClipId) -> bool {
    state.deferred_clip_commands.iter().any(|d| {
        matches!(d.command, AudioCommand::SplitClip { new_clip_id, .. } if new_clip_id == clip_id)
    })
}

/// Sane bounds for per-clip gain, in decibels. `-60` dB is effectively
/// silent and `+24` dB a generous boost ceiling; values outside this
/// range from the command are clamped before being stored or emitted.
pub const MIN_CLIP_GAIN_DB: f32 = -60.0;
pub const MAX_CLIP_GAIN_DB: f32 = 24.0;

pub(crate) fn handle_set_clip_fade(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    fade_in_frames: u64,
    fade_in_curve: FadeCurve,
    fade_out_frames: u64,
    fade_out_curve: FadeCurve,
) {
    set_clip_fade_in_place(
        ctx.clips,
        ctx.event_tx,
        clip_id,
        fade_in_frames,
        fade_in_curve,
        fade_out_frames,
        fade_out_curve,
    );
}

pub(crate) fn handle_set_clip_gain(ctx: &HandlerCtx, clip_id: ClipId, gain_db: f32) {
    set_clip_gain_in_place(ctx.clips, ctx.event_tx, clip_id, gain_db);
}

pub(crate) fn handle_set_clip_warp(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    warp_enabled: bool,
    original_bpm: Option<f32>,
    transpose_semitones: f32,
    warp_algorithm: WarpAlgorithm,
) {
    set_clip_warp_in_place(
        ctx.clips,
        ctx.event_tx,
        clip_id,
        warp_enabled,
        original_bpm,
        transpose_semitones,
        warp_algorithm,
    );
}

pub(crate) fn handle_set_clip_warp_markers(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    markers: Vec<WarpMarker>,
) {
    set_clip_warp_markers_in_place(ctx.clips, ctx.event_tx, clip_id, markers);
}

/// Handle `AudioCommand::DetectClipTempo`. Thin wrapper over
/// [`detect_clip_tempo_in_place`], passing the engine's project sample
/// rate through. See that helper for the behaviour contract.
pub(crate) fn handle_detect_clip_tempo(ctx: &HandlerCtx, clip_id: ClipId) {
    detect_clip_tempo_in_place(ctx.clips, ctx.event_tx, ctx.sample_rate, clip_id);
}

/// Apply fade lengths/curves to the audio clip with `clip_id` and emit
/// `ClipFadeChanged`. Each fade length is clamped to the clip's visible
/// duration so a fade can never run past the audible region. Both the
/// mutation and the event live inside the `if let Some(clip)` branch, so
/// a missing-clip lookup never emits a ghost event (mirroring
/// [`handle_move_clip`] / the MIDI clip handlers). The clamped values are
/// what gets stored and emitted, keeping the app mirror in sync.
#[allow(clippy::too_many_arguments)]
pub fn set_clip_fade_in_place(
    clips: &RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    fade_in_frames: u64,
    fade_in_curve: FadeCurve,
    fade_out_frames: u64,
    fade_out_curve: FadeCurve,
) {
    let mut guard = clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        // A fade can't be longer than the clip is audible.
        let max = clip.duration_frames();
        let fade_in_frames = fade_in_frames.min(max);
        let fade_out_frames = fade_out_frames.min(max);
        clip.fade_in_frames = fade_in_frames;
        clip.fade_in_curve = fade_in_curve;
        clip.fade_out_frames = fade_out_frames;
        clip.fade_out_curve = fade_out_curve;
        let _ = event_tx.send(AudioEvent::ClipFadeChanged {
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        });
    }
}

/// Apply a per-clip gain to the audio clip with `clip_id` and emit
/// `ClipGainChanged`. The value is clamped to
/// `[MIN_CLIP_GAIN_DB, MAX_CLIP_GAIN_DB]` (a `NaN` is treated as unity)
/// before being stored/emitted. Same missing-clip invariant as
/// [`set_clip_fade_in_place`].
pub fn set_clip_gain_in_place(
    clips: &RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    gain_db: f32,
) {
    let mut guard = clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        let gain_db = if gain_db.is_nan() {
            0.0
        } else {
            gain_db.clamp(MIN_CLIP_GAIN_DB, MAX_CLIP_GAIN_DB)
        };
        clip.gain_db = gain_db;
        let _ = event_tx.send(AudioEvent::ClipGainChanged { clip_id, gain_db });
    }
}

/// Apply warp ("follow tempo") parameters to the audio clip with
/// `clip_id` and emit `ClipWarpChanged`. The original [`ClipSource`] PCM
/// is never mutated — warp is non-destructive and only changes how the
/// source is read on the render path. Same missing-clip invariant as
/// [`set_clip_fade_in_place`]: a lookup miss emits no ghost event.
pub fn set_clip_warp_in_place(
    clips: &RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    warp_enabled: bool,
    original_bpm: Option<f32>,
    transpose_semitones: f32,
    warp_algorithm: WarpAlgorithm,
) {
    let mut guard = clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.warp_enabled = warp_enabled;
        clip.original_bpm = original_bpm;
        clip.transpose_semitones = transpose_semitones;
        clip.warp_algorithm = warp_algorithm;
        let _ = event_tx.send(AudioEvent::ClipWarpChanged {
            clip_id,
            warp_enabled,
            original_bpm,
            transpose_semitones,
            warp_algorithm,
        });
    }
}

/// Replace the full warp-marker set of the audio clip with `clip_id`
/// and emit `ClipWarpMarkersChanged`. The incoming markers are sorted by
/// `timeline_beat` ascending before being stored so the [`WarpMarker`]
/// invariant the warp-mapping math relies on always holds, regardless of
/// the order the caller built them in. The sorted set is what's both
/// stored and emitted. Same missing-clip invariant as
/// [`set_clip_fade_in_place`].
pub fn set_clip_warp_markers_in_place(
    clips: &RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
    clip_id: ClipId,
    mut markers: Vec<WarpMarker>,
) {
    let mut guard = clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        markers.sort_by(|a, b| a.timeline_beat.total_cmp(&b.timeline_beat));
        clip.warp_markers = markers.clone();
        let _ = event_tx.send(AudioEvent::ClipWarpMarkersChanged { clip_id, markers });
    }
}

/// Run the `resonance-dsp` tempo detector over the audio clip with
/// `clip_id` and emit [`AudioEvent::ClipTempoDetected`] with the
/// estimated BPM and confidence. The clip's stereo-interleaved source
/// is downmixed to mono (`(l + r) * 0.5`) for the detector, which works
/// on a single channel. `sample_rate` is the engine's project rate.
///
/// This is analysis only: the clip is never mutated. The app decides
/// whether to act on the estimate (e.g. via `AudioCommand::SetClipWarp`
/// to set `original_bpm`). Same missing-clip invariant as
/// [`set_clip_warp_in_place`]: a lookup miss emits no ghost event. The
/// clip read lock is released before the event is sent.
pub fn detect_clip_tempo_in_place(
    clips: &RwLock<Vec<AudioClip>>,
    event_tx: &Sender<AudioEvent>,
    sample_rate: u32,
    clip_id: ClipId,
) {
    let mono = {
        let guard = clips.read();
        match guard.iter().find(|c| c.id == clip_id) {
            Some(clip) => clip
                .source
                .as_frames()
                .chunks_exact(2)
                .map(|frame| (frame[0] + frame[1]) * 0.5)
                .collect::<Vec<f32>>(),
            None => return,
        }
    };

    let TempoEstimate { bpm, confidence } = detect_tempo_default(&mono, sample_rate as f32);
    let _ = event_tx.send(AudioEvent::ClipTempoDetected {
        clip_id,
        bpm,
        confidence,
    });
}

/// What a finished clip load owes the app.
///
/// The mmap, the peak decimation and the publish are identical for a
/// timeline clip and a restored take clip; only this differs, and it is
/// not cosmetic. `ClipImported` is what makes a clip appear in
/// `Resonance::clips`, and a take clip must not (ba todo #1396) — see
/// [`AudioCommand::LoadTakeClipFromWav`](crate::types::AudioCommand::LoadTakeClipFromWav).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipLoadEcho {
    /// Emit `AudioEvent::ClipImported`, so the app mirrors a timeline clip.
    Timeline,
    /// Emit nothing. The take-group restore is sender and mirror both.
    SilentTake,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_load_clip_from_wav(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    path: PathBuf,
    name: String,
    trim_start_frames: u64,
    trim_end_frames: u64,
) {
    submit_clip_load(
        ctx,
        state,
        clip_id,
        track_id,
        start_sample,
        path,
        name,
        trim_start_frames,
        trim_end_frames,
        ClipLoadEcho::Timeline,
    );
}

/// Put a restored take clip's recorded WAV back into the engine's clip
/// list (ba todo #1402): the audio half of a project-load take-lane
/// restore, whose group half is `RestoreTakeGroups`.
///
/// Silent, untrimmed, and idempotent — see
/// [`AudioCommand::LoadTakeClipFromWav`](crate::types::AudioCommand::LoadTakeClipFromWav)
/// for why each of those is load-bearing.
///
/// The early return here is only an optimisation: it spares the mmap and
/// the O(n) peak decimation on the undo/redo replay, where every take clip
/// is already loaded. It cannot be the guarantee, because the load it
/// guards is asynchronous — two restores in quick succession would both
/// look at a list the first one's load has not landed in yet. The binding
/// check is the one in [`apply_clip_loaded`], in the same engine-thread
/// step as the push.
pub(crate) fn handle_load_take_clip_from_wav(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    path: PathBuf,
    name: String,
) {
    if ctx.clips.read().iter().any(|c| c.id == clip_id) {
        // Still raise the allocator: the reservation must hold whether or
        // not this particular load had anything left to do.
        reserve_clip_id(&mut state.next_clip_id, clip_id);
        return;
    }
    submit_clip_load(
        ctx,
        state,
        clip_id,
        track_id,
        start_sample,
        path,
        name,
        0,
        0,
        ClipLoadEcho::SilentTake,
    );
}

#[allow(clippy::too_many_arguments)]
fn submit_clip_load(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    path: PathBuf,
    name: String,
    trim_start_frames: u64,
    trim_end_frames: u64,
    echo: ClipLoadEcho,
) {
    // Bump the engine-thread-local id counter immediately so that any
    // subsequent recording (C3/C4/C5, still auto-allocated until D-7d)
    // issued before the worker thread completes still allocates a unique
    // id. The worker captures `clip_id` by move, so this update only
    // affects future allocations.
    reserve_clip_id(&mut state.next_clip_id, clip_id);

    // The heavy work — `ClipSource::open_wav` (which pre-touches every
    // page of the mmap) and `compute_waveform_peaks` (an O(n) decimation
    // across the whole sample buffer) — used to run synchronously on the
    // engine thread. Project load fires one `LoadClipFromWav` per audio
    // clip, so on a project with many large clips the engine command
    // queue stalled for hundreds of milliseconds. A worker does it now and
    // posts the finished clip back ([`EngineInternal::ClipLoaded`]); the
    // engine thread applies it in [`apply_clip_loaded`] — every check that
    // decides whether it lands, the publish and the echo, as one
    // engine-thread step (code review ARCH-02 B-5: only this thread edits
    // the render graph).
    //
    // Concurrency is bounded by `MAX_CONCURRENT_IMPORTS` worker threads
    // in `state.imports` (shared with the import path). Requests past
    // that bound *queue*: this handler used to reject them with an error
    // event, which meant any project with more than
    // `MAX_CONCURRENT_IMPORTS` audio clips silently lost the excess —
    // those clips never reached the engine, played back silent, and were
    // then missing from the bundle written by the next save. `submit`
    // only enqueues (unbounded channel, lazy worker spawn), so the
    // engine thread still returns immediately regardless of backlog.
    let shared = Arc::clone(ctx.shared);
    let engine_rate = ctx.sample_rate;
    // Project fence (FU-D7c): `handle_clear_all` bumps `clear_generation`.
    // The load carries the generation it was submitted under, and
    // `apply_clip_loaded` compares it on this thread — see there.
    let generation = state.clear_generation.load(Ordering::SeqCst);
    // This load's ticket (FU-A13e): superseded by a later load of the
    // id, withdrawn by a `DeleteClip` — either way it must not publish.
    let ticket = state.clip_load_tickets.issue(clip_id);

    let submit_result = state.imports.submit(move || {
        // `open_wav_at_rate` resamples to the engine rate when the
        // project's WAV was written under a different device rate,
        // so the clip can't play back pitched/sped.
        match ClipSource::open_wav_at_rate(&path, engine_rate) {
            Ok(source) => {
                let total_frames = source.frame_count();
                let waveform_peaks = compute_waveform_peaks(source.as_frames());
                let duration_samples = total_frames
                    .saturating_sub(trim_start_frames)
                    .saturating_sub(trim_end_frames);

                let clip = AudioClip {
                    id: clip_id,
                    track_id,
                    start_sample,
                    source,
                    name,
                    trim_start_frames,
                    trim_end_frames,
                    fade_in_frames: 0,
                    fade_in_curve: FadeCurve::default(),
                    fade_out_frames: 0,
                    fade_out_curve: FadeCurve::default(),
                    gain_db: 0.0,
                    vocal_tuning: None,
                    warp_enabled: false,
                    original_bpm: None,
                    transpose_semitones: 0.0,
                    warp_algorithm: Default::default(),
                    warp_markers: Vec::new(),
                    tuning_render_cache: None,
                };
                shared.inbox.post(EngineInternal::ClipLoaded(Box::new(LoadedClip {
                    clip,
                    ticket,
                    generation,
                    echo,
                    duration_samples,
                    waveform_peaks,
                })));
            }
            Err(e) => shared.inbox.post(EngineInternal::ClipLoadFailed {
                clip_id,
                ticket,
                error: e.to_string(),
            }),
        }
    });
    if let Err(e) = submit_result {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::io(format!(
            "Failed to spawn clip-load thread: {}",
            e
        ))));
    }
}

/// Apply a clip-load worker's finished clip ([`EngineInternal::ClipLoaded`]).
/// Engine thread.
///
/// Every check that decides whether the load lands runs here, in one
/// engine-thread step with the publish and the echo. Until ARCH-02 B-5 the
/// worker made them itself while holding `ctx.clips.write()`, and each
/// ordering guarantee below came from that lock; now it comes from the
/// fact that every command that could change the answer (`DeleteClip`,
/// `ClearAll`, `RemoveTake`, a second load's submit) also runs on this
/// thread, so it is applied wholly before or wholly after this, never in
/// the middle:
///
/// * **Ticket (FU-A13e).** Redeemed before any early return, so a load
///   dropped for another reason still retires its ticket. A `DeleteClip`
///   applied before this withdrew the ticket → dropped, and the delete
///   already echoed; one applied after it removes the clip this pushed. A
///   later load of the id superseded the ticket at *its* submit → this
///   one is dropped and the later one lands — last submitted wins, in
///   whatever order the workers finish.
/// * **`ClearAll` fence (FU-D7c).** A `ClearAll` applied before this bumped
///   `clear_generation` → dropped, no echo. One applied after it drains
///   the clip this pushed, and its `AllCleared` goes out after this
///   `ClipImported` (both sent from this thread, in order) — so the app
///   never keeps a clip, an echo or a take-park delivery from the project
///   the load was submitted against.
/// * **Duplicate id.** Two loads of one id both in flight never get this
///   far (the later ticket supersedes the earlier), so this only catches a
///   load of an id whose clip already landed; it is dropped, echo and all.
///   The app's reload-in-place is a `DeleteClip` first (A-13i).
///   Pinned by `loop_record_takes.rs::two_take_clip_loads_racing_each_other_still_leave_one_clip`.
/// * **Take park (ba todo #1403).** A `RemoveTake` applied before this,
///   while the load was in flight, left a claim; the clip goes to the park
///   rather than the render's input (an ungoverned take clip is the
///   "deleting a take makes it louder" bug, ba doc #292). Unconditional,
///   not gated on `echo`: only take clips are ever claimed, so it is a
///   no-op on the timeline path, and no future caller can route a load
///   around it.
pub(crate) fn apply_clip_loaded(ctx: &HandlerCtx, state: &mut HandlerState, loaded: LoadedClip) {
    let LoadedClip {
        clip,
        ticket,
        generation,
        echo,
        duration_samples,
        waveform_peaks,
    } = loaded;
    let clip_id = clip.id;
    let live = state.clip_load_tickets.redeem(clip_id, ticket);
    if state.clear_generation.load(Ordering::SeqCst) != generation || !live {
        return;
    }
    if clip_exists(ctx, clip_id) {
        return;
    }
    let Some(clip) = state.take_clip_park.deliver(clip) else {
        return;
    };
    let (track_id, start_sample, name) = (clip.track_id, clip.start_sample, clip.name.clone());
    ctx.clips.write().push(clip);
    if echo == ClipLoadEcho::Timeline {
        let _ = ctx.event_tx.send(AudioEvent::ClipImported {
            clip_id,
            track_id,
            start_sample,
            duration_samples,
            name,
            waveform_peaks,
        });
    }
}

/// A clip-load worker could not open its WAV
/// ([`EngineInternal::ClipLoadFailed`]). Engine thread.
///
/// Retires the ticket either way; the failure is reported only while the
/// load is still wanted — one already cancelled by its `DeleteClip` (which
/// echoed) or superseded by a later load has nobody left to report to.
pub(crate) fn apply_clip_load_failed(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    ticket: u64,
    error: String,
) {
    if state.clip_load_tickets.redeem(clip_id, ticket) {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::io(format!(
            "Failed to load clip WAV: {error}"
        ))));
    }
}

/// Guarantee that every in-engine audio clip has a WAV file on disk
/// at `{project_dir}/audio/clip_{id}.wav`. Recorded and imported
/// clips are already `ClipSource::Mapped` and just need their path
/// returned; any remaining `ClipSource::Memory` clips get transcoded.
pub(crate) fn handle_save_clips_to_project_dir(ctx: &HandlerCtx, state: &mut HandlerState) {
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(
                "Cannot save clips: no project directory set.",
            )));
            return;
        }
    };

    // Collect the per-clip work list while holding only a read
    // lock so in-memory or cross-directory clips we need to
    // transcode/copy don't block the mixer any longer than
    // necessary.
    //
    // For each clip we categorize as:
    //   Ready   — already at the target path, no work
    //   Copy    — `Mapped` at a different path (save-as case)
    //   Encode  — `Memory` (transient imports)
    enum Action {
        Ready,
        Copy(PathBuf),
        Encode(std::sync::Arc<[f32]>),
    }
    let mut entries: Vec<(ClipId, String, Action)> = Vec::new();
    {
        let clips_guard = ctx.clips.read();
        for clip in clips_guard.iter() {
            let rel = format!("audio/clip_{}.wav", clip.id);
            let target = project_dir.join(&rel);
            let action = match &clip.source {
                ClipSource::Mapped { path, .. } => {
                    if path == &target {
                        Action::Ready
                    } else {
                        Action::Copy(path.clone())
                    }
                }
                ClipSource::Memory(v) => Action::Encode(v.clone()),
            };
            entries.push((clip.id, rel, action));
        }
    }

    let sr = ctx.sample_rate;
    let mut needs_remap: Vec<ClipId> = Vec::new();
    for (clip_id, _rel, action) in &entries {
        let target = project_dir
            .join("audio")
            .join(format!("clip_{clip_id}.wav"));
        match action {
            Action::Ready => {}
            Action::Copy(src_path) => {
                if let Some(parent) = target.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        let _ = ctx
                            .event_tx
                            .send(AudioEvent::Error(EngineError::io(format!("Create audio dir: {e}"))));
                        return;
                    }
                }
                if let Err(e) = std::fs::copy(src_path, &target) {
                    let _ = ctx
                        .event_tx
                        .send(AudioEvent::Error(EngineError::io(format!("Copy clip {clip_id} WAV: {e}"))));
                    return;
                }
                needs_remap.push(*clip_id);
            }
            Action::Encode(samples) => {
                if let Err(e) = transcode_to_wav(&target, samples, sr) {
                    let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::io(format!(
                        "Transcode clip {clip_id} to WAV: {e}"
                    ))));
                    return;
                }
                needs_remap.push(*clip_id);
            }
        }
    }

    // Re-open the mmap for each clip whose backing file we just
    // wrote, so playback reads from the file inside the new project
    // dir and future saves are no-ops.
    for clip_id in needs_remap {
        let target = project_dir
            .join("audio")
            .join(format!("clip_{clip_id}.wav"));
        if let Ok(source) = ClipSource::open_wav(&target) {
            let mut clips_guard = ctx.clips.write();
            if let Some(clip) = clips_guard.iter_mut().find(|c| c.id == clip_id) {
                clip.source = source;
            }
        }
    }

    let clip_files: Vec<(ClipId, String)> =
        entries.into_iter().map(|(id, rel, _)| (id, rel)).collect();
    let _ = ctx
        .event_tx
        .send(AudioEvent::ClipsSavedToProjectDir { clip_files });
}

/// Give every in-engine audio clip its `{project_dir}/audio/clip_{id}.wav`
/// now rather than at the next save (code review FU-V5b).
///
/// An undo snapshot names a clip's audio only by that path, and the
/// slow-path (full-reload) restore reloads it from there — so a clip that
/// lived only in RAM or in a render's `vocal_*.wav` came back silent when
/// undone before a save. The app sends this whenever it captures an undo
/// snapshot, ahead of the edit's own commands, so every clip a snapshot
/// can reference has its file before anything can remove the clip.
///
/// Silent (no event, unlike [`handle_save_clips_to_project_dir`]); a
/// no-op without a project dir. Per clip, only when the file is missing:
/// hard-link a mapped source inside the project dir, else encode the PCM
/// the engine already holds (so a source file unlinked meanwhile still
/// persists), write-then-rename. The clip is then remapped onto the new
/// file, which keeps the next save from copying over a hard link it
/// shares an inode with and frees an in-RAM clip's buffer. An existing
/// `clip_{id}.wav` is never touched — ids are never reused (STATE-08).
pub(crate) fn handle_persist_clip_wavs(ctx: &HandlerCtx, state: &HandlerState) {
    let Some(project_dir) = state.project_dir.clone() else {
        return;
    };
    let audio_dir = project_dir.join("audio");
    let pending: Vec<(ClipId, PathBuf, ClipSource)> = {
        let clips = ctx.clips.read();
        clips
            .iter()
            .filter_map(|clip| {
                let target = audio_dir.join(format!("clip_{}.wav", clip.id));
                let ready = clip.source.mapped_path() == Some(target.as_path());
                (!ready && !target.exists()).then(|| (clip.id, target, clip.source.share()))
            })
            .collect()
    };
    if pending.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&audio_dir) {
        tracing::warn!("[clips] persist clip WAVs: create {}: {e}", audio_dir.display());
        return;
    }
    for (clip_id, target, source) in pending {
        let linked = match &source {
            ClipSource::Mapped { path, .. } if path.starts_with(&project_dir) => {
                std::fs::hard_link(path, &target).is_ok()
            }
            _ => false,
        };
        if !linked {
            let tmp = target.with_extension("wav.tmp");
            let written = transcode_to_wav(&tmp, source.as_frames(), ctx.sample_rate)
                .map_err(|e| e.to_string())
                .and_then(|()| std::fs::rename(&tmp, &target).map_err(|e| e.to_string()));
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                tracing::warn!("[clips] persist clip {clip_id} WAV: {e}");
                continue;
            }
        }
        drop(source);
        match ClipSource::open_wav(&target) {
            Ok(mapped) => {
                let mut clips = ctx.clips.write();
                if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.source = mapped;
                }
            }
            Err(e) => {
                // Never leave a file the clip isn't mapped to: a later
                // save would copy the old source over it in place.
                let _ = std::fs::remove_file(&target);
                tracing::warn!("[clips] map persisted clip {clip_id} WAV: {e}");
            }
        }
    }
}

/// Failure writing a stereo-interleaved f32 buffer to a 32-bit float WAV
/// ([`transcode_to_wav`]). Message text matches the historical
/// `format!()` strings.
#[derive(Debug, Error)]
pub enum TranscodeError {
    #[error("create {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("create {path}: {source}")]
    Create {
        path: String,
        #[source]
        source: hound::Error,
    },
    #[error("write sample: {source}")]
    WriteSample {
        #[source]
        source: hound::Error,
    },
    #[error("finalize wav: {source}")]
    Finalize {
        #[source]
        source: hound::Error,
    },
    /// [`transcode_to_wav_new`] refused to overwrite an existing file
    /// (D-7a): the app is the pool's only asset-id allocator and never
    /// hands out an id twice, so an `asset_<id>.wav` already on disk means
    /// either a stale/orphaned file the app hasn't seeded past, or a
    /// duplicate id — either way, silently overwriting it would risk
    /// destroying audio a clip or a backup still names.
    #[error("asset id in use: {path} already exists")]
    AssetIdInUse { path: String },
    /// A non-collision failure opening the file for
    /// [`transcode_to_wav_new`] (permissions, a missing/unwritable parent
    /// after the `create_dir_all`, …).
    #[error("create {path}: {source}")]
    OpenNew {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl From<TranscodeError> for EngineError {
    fn from(e: TranscodeError) -> Self {
        EngineError::new(EngineErrorKind::Io, e.to_string())
    }
}

fn wav_spec(sample_rate: u32) -> WavSpec {
    WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    }
}

/// Write every sample to an already-opened writer and finalize it. Shared
/// tail of [`transcode_to_wav`] and [`transcode_to_wav_new`], which differ
/// only in how the underlying file is opened.
fn write_wav_samples<W: std::io::Write + std::io::Seek>(
    mut writer: WavWriter<W>,
    samples: &[f32],
) -> Result<(), TranscodeError> {
    for &s in samples {
        writer
            .write_sample(s)
            .map_err(|e| TranscodeError::WriteSample { source: e })?;
    }
    writer
        .finalize()
        .map_err(|e| TranscodeError::Finalize { source: e })?;
    Ok(())
}

/// Write a stereo-interleaved f32 buffer to a 32-bit float WAV, creating
/// or overwriting `path`. Creates the target directory if needed. Used by
/// both the save-time fallback for in-RAM clips and clip-load paths that
/// deliberately re-lay a file (e.g. a persisted take being remapped).
pub fn transcode_to_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), TranscodeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TranscodeError::CreateDir {
            path: parent.display().to_string(),
            source: e,
        })?;
    }
    let writer = WavWriter::create(path, wav_spec(sample_rate)).map_err(|e| TranscodeError::Create {
        path: path.display().to_string(),
        source: e,
    })?;
    write_wav_samples(writer, samples)
}

/// Write a stereo-interleaved f32 buffer to a FRESH 32-bit float WAV,
/// refusing (`TranscodeError::AssetIdInUse`) if `path` already exists
/// (`O_EXCL` via `create_new`). Creates the target directory if needed.
///
/// Used only for pool-asset writes (`asset_<id>.wav`, D-7a): unlike
/// [`transcode_to_wav`]'s overwrite semantics, an asset name must never be
/// silently reused out from under a file that's already there — the app is
/// the pool's only id allocator and never means to hand out the same id
/// twice, so a collision here is either a stale orphaned file or a bug,
/// and either way the existing bytes are worth keeping.
pub fn transcode_to_wav_new(
    path: &Path,
    samples: &[f32],
    sample_rate: u32,
) -> Result<(), TranscodeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TranscodeError::CreateDir {
            path: parent.display().to_string(),
            source: e,
        })?;
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                TranscodeError::AssetIdInUse {
                    path: path.display().to_string(),
                }
            } else {
                TranscodeError::OpenNew {
                    path: path.display().to_string(),
                    source: e,
                }
            }
        })?;
    let writer = WavWriter::new(std::io::BufWriter::new(file), wav_spec(sample_rate))
        .map_err(|e| TranscodeError::Create {
            path: path.display().to_string(),
            source: e,
        })?;
    write_wav_samples(writer, samples)
}
