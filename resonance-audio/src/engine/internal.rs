//! Worker → engine-thread messages (code review ARCH-02 B-5 / A2-8).
//!
//! The clip list used to be the one render-graph map worker threads
//! wrote: the clip-load worker pushed its finished clip, the pitch
//! analyser stored its contour, the offline renderers attached their
//! retune caches and the bounce-in-place worker pushed its bounced clip —
//! each under the clip list's write lock, from its own thread. Since B-5 only the
//! engine control thread publishes the render graph, so a worker does its
//! heavy work (mmap, decimation, f0 detection, FFT resynthesis, the
//! render) on its own thread and hands the *result* to the engine thread
//! as an [`EngineInternal`] message on [`SharedState::inbox`]; the engine
//! loop applies it — checks, edit, publish and echo in one engine-thread
//! step.
//!
//! Why an inbox on `SharedState` rather than engine-internal
//! `AudioCommand` variants (B-3's `BounceTargetCancelled`): every worker
//! already holds an `Arc<SharedState>` (the offline entry points included,
//! which have no command sender), the payloads carry clips that have no
//! business in the public command enum, and the engine loop `select!`s on
//! both channels, so a message wakes it as promptly as a command does.
//! Messages are not ordered against commands — and need not be: every
//! check that decides a message's fate (load ticket, `ClearAll`
//! generation, duplicate id, take-park claim, clip still present) reads
//! engine-thread state *when the message is applied*, and every command
//! that could change the answer runs on the same thread, either wholly
//! before or wholly after. That is exactly the atomicity the shared write
//! lock used to provide; see each handler for the per-message argument.
//!
//! [`SharedState::inbox`]: super::SharedState::inbox

use crossbeam_channel::{Receiver, Sender};

use crate::types::{AudioClip, AudioEvent, ClipId, F0Frame, NoteBlob};

use super::clips::ClipLoadEcho;
use super::thread::{HandlerCtx, HandlerState};
use super::vocal_render::TuningCaches;

/// A clip-load worker's finished clip (`clips::submit_clip_load`).
#[derive(Debug)]
pub(crate) struct LoadedClip {
    pub clip: AudioClip,
    /// The load ticket issued at submit (FU-A13e).
    pub ticket: u64,
    /// `HandlerState::clear_generation` at submit (FU-D7c).
    pub generation: u64,
    pub echo: ClipLoadEcho,
    pub duration_samples: u64,
    pub waveform_peaks: Vec<(f32, f32)>,
}

/// One worker result for the engine thread to apply. See the module docs.
#[derive(Debug)]
pub(crate) enum EngineInternal {
    /// A `LoadClipFromWav` / `LoadTakeClipFromWav` worker decoded its
    /// clip ([`super::clips::apply_clip_loaded`]).
    ClipLoaded(Box<LoadedClip>),
    /// A clip-load worker could not open its WAV
    /// ([`super::clips::apply_clip_load_failed`]).
    ClipLoadFailed {
        clip_id: ClipId,
        ticket: u64,
        error: String,
    },
    /// `AnalyzeClipPitch`'s worker detected the clip's f0 contour and note
    /// blobs ([`super::vocal_analysis::apply_pitch_analysis`]).
    PitchAnalysed {
        clip_id: ClipId,
        contour: Vec<F0Frame>,
        notes: Vec<NoteBlob>,
    },
    /// An offline renderer (bounce / export / stem / freeze) built the
    /// vocal-tuning retune caches it renders with; attach them to the live
    /// clips too ([`super::vocal_render::apply_tuning_caches`]).
    TuningCachesBuilt(TuningCaches),
    /// The offline bounce-in-place finished: add its clip to the timeline
    /// and report `TrackBounceCompleted`
    /// ([`super::bounce::apply_bounced_clip`]).
    BouncedClip {
        clip: Box<AudioClip>,
        completed: Box<AudioEvent>,
    },
}

/// The worker → engine channel on [`SharedState`](super::SharedState).
/// Unbounded: a worker never blocks on the engine, and the engine drains
/// it on every loop pass.
pub(crate) struct EngineInbox {
    tx: Sender<EngineInternal>,
    rx: Receiver<EngineInternal>,
}

impl Default for EngineInbox {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { tx, rx }
    }
}

impl std::fmt::Debug for EngineInbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineInbox").field("len", &self.rx.len()).finish()
    }
}

impl EngineInbox {
    /// Hand `msg` to the engine thread. Any thread.
    pub(crate) fn post(&self, msg: EngineInternal) {
        // The receiver lives beside the sender, so this cannot fail.
        let _ = self.tx.send(msg);
    }

    /// The engine loop's end, for its `select!`.
    pub(crate) fn receiver(&self) -> &Receiver<EngineInternal> {
        &self.rx
    }

    /// Every message posted so far, in order. Engine thread.
    pub(crate) fn drain(&self) -> Vec<EngineInternal> {
        self.rx.try_iter().collect()
    }
}

/// Apply one worker result. Engine thread only.
pub(crate) fn dispatch_internal(ctx: &HandlerCtx, state: &mut HandlerState, msg: EngineInternal) {
    match msg {
        EngineInternal::ClipLoaded(loaded) => super::clips::apply_clip_loaded(ctx, state, *loaded),
        EngineInternal::ClipLoadFailed {
            clip_id,
            ticket,
            error,
        } => super::clips::apply_clip_load_failed(ctx, state, clip_id, ticket, error),
        EngineInternal::PitchAnalysed {
            clip_id,
            contour,
            notes,
        } => super::vocal_analysis::apply_pitch_analysis(ctx, clip_id, contour, notes),
        EngineInternal::TuningCachesBuilt(caches) => {
            super::vocal_render::apply_tuning_caches(ctx, &caches)
        }
        EngineInternal::BouncedClip { clip, completed } => {
            super::bounce::apply_bounced_clip(ctx, *clip, *completed)
        }
    }
}

/// Apply every message waiting in the inbox, in order. Engine thread.
pub(crate) fn drain_internal(ctx: &HandlerCtx, state: &mut HandlerState) {
    for msg in ctx.shared.inbox.drain() {
        dispatch_internal(ctx, state, msg);
    }
}
