//! Per-lane vocal render state (vocal-mcp-control-and-quality.md §7): is
//! the audio a lane plays the audio its current notes, lyrics and settings
//! would render, is a render running, or did the last one fail?
//!
//! Staleness is content-addressed, like the render cache it builds on
//! (`vocal_svs::render_cache`): every render records the unit content
//! keys of the take it installed, per installed audio clip; the lane's
//! current plan is hashed the same way; equal key lists mean `rendered`.
//! The keys travel with the audio clip — saved on its `ProjectClip`,
//! restored by the `vocal_audio_clips` reconcile — so a reopened project
//! (or an undo that brings an older take back) compares against the take
//! it actually plays.
//!
//! These are the app-side types; `update::compose::vocal_render_state`
//! computes them and the control layer maps them onto the wire.

use std::collections::HashMap;
use std::sync::Arc;

use resonance_audio::types::{ClipId, TrackId};

use super::vocal_svs::VocalRenderErrorKind;

/// A vocal lane: `(section definition, track)`.
pub type VocalLaneKey = (u64, TrackId);

/// Where a vocal lane's audio stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VocalLaneRenderState {
    /// No audio is installed and no render has failed.
    NotRendered,
    /// A render is queued or running. The previous take, if any, keeps
    /// playing until it lands.
    Rendering,
    /// The installed audio is what the lane's content renders to.
    Rendered,
    /// The lane changed since its audio was rendered; the old take keeps
    /// playing until a render replaces it.
    Stale,
    /// The last render failed, and nothing relevant changed since.
    Error,
}

impl VocalLaneRenderState {
    /// How much a lane in this state needs attention, for the track-level
    /// summary: `error > rendering > stale > not_rendered > rendered`. A
    /// failure needs a fix before anything else; a running render needs
    /// waiting on before the state means anything; stale audio plays the
    /// wrong thing; a lane with nothing rendered plays nothing (often a
    /// lane with no notes yet), and `rendered` is the only "done".
    pub fn severity(self) -> u8 {
        match self {
            Self::Rendered => 0,
            Self::NotRendered => 1,
            Self::Stale => 2,
            Self::Rendering => 3,
            Self::Error => 4,
        }
    }

    /// The state that needs the most attention of `states`;
    /// `NotRendered` for none.
    pub fn worst(states: impl IntoIterator<Item = Self>) -> Self {
        states
            .into_iter()
            .max_by_key(|s| s.severity())
            .unwrap_or(Self::NotRendered)
    }
}

/// Why a lane's last render failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VocalLaneErrorKind {
    /// The lane's bank, or its vocoder, is not installed.
    VoicebankMissing,
    /// Every unit resolved to no phonemes.
    NothingToSing,
    /// The lane has no clip, or its clip has no notes.
    NoNotes,
    /// A syllable has a phoneme the bank cannot sing.
    UnsupportedPhonemes,
    /// The lane's settings do not validate.
    InvalidParams,
    /// The SVS pipeline or the WAV write failed.
    RenderFailed,
}

impl VocalLaneErrorKind {
    /// The snake_case name the wire and the badge use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VoicebankMissing => "voicebank_missing",
            Self::NothingToSing => "nothing_to_sing",
            Self::NoNotes => "no_notes",
            Self::UnsupportedPhonemes => "unsupported_phonemes",
            Self::InvalidParams => "invalid_params",
            Self::RenderFailed => "render_failed",
        }
    }
}

impl From<VocalRenderErrorKind> for VocalLaneErrorKind {
    fn from(kind: VocalRenderErrorKind) -> Self {
        match kind {
            VocalRenderErrorKind::VoicebankMissing => Self::VoicebankMissing,
            VocalRenderErrorKind::NothingToSing => Self::NothingToSing,
            VocalRenderErrorKind::Failed => Self::RenderFailed,
        }
    }
}

/// A lane's last render failure, and the lane content it failed on: an
/// edit that changes the content (the lane's fingerprint) retires it.
#[derive(Debug, Clone, PartialEq)]
pub struct VocalLaneError {
    pub kind: VocalLaneErrorKind,
    pub message: String,
    pub fingerprint: u64,
}

/// The unit keys a lane's current content plans to, memoised by the
/// content's fingerprint. `keys` is `None` when no plan can be built (no
/// clip or notes, a phoneme the bank cannot sing).
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedUnits {
    pub fingerprint: u64,
    pub keys: Option<Arc<[u64]>>,
}

/// One lane's render state as the views and `song.vocal` show it.
#[derive(Debug, Clone, PartialEq)]
pub struct VocalLaneRenderStatus {
    pub state: VocalLaneRenderState,
    /// Units of the current plan whose audio is not in the installed take
    /// — what the next render synthesises. `0` unless `state` is `Stale`
    /// (and `0` there too when the edit only removed units).
    pub stale_units: usize,
    /// Units in the current plan (`0` when none can be built).
    pub total_units: usize,
    /// The failure behind `Error`.
    pub error: Option<(VocalLaneErrorKind, String)>,
}

/// The render-state side table, kept in `VocalAudioRegistry::status`.
#[derive(Debug, Default)]
pub struct VocalRenderStatus {
    /// Unit content keys of the take each installed vocal audio clip
    /// plays. Saved as `ProjectClip::vocal_render_units`.
    pub clip_unit_keys: HashMap<ClipId, Arc<[u64]>>,
    /// Each lane's last render failure.
    pub errors: HashMap<VocalLaneKey, VocalLaneError>,
    /// The expected-keys memo. Survives an undo (it is keyed by content,
    /// so a stale entry only costs a recompute) so a GUI undo is seen as
    /// the edit it is; reset by a disk load.
    pub expected: HashMap<VocalLaneKey, ExpectedUnits>,
    /// The lane fingerprint a queued render was planned from, while it is
    /// in flight: an edit made while it runs is not covered by it.
    pub queued_fingerprint: HashMap<VocalLaneKey, u64>,
    /// Lanes with a debounced GUI re-render pending, by the generation
    /// the pending `VocalAutoRender` message carries; a newer edit bumps
    /// it, so only the last edit's timer renders.
    pub auto_render: HashMap<VocalLaneKey, u64>,
    pub next_generation: u64,
    /// Every vocal lane's status as of the end of the last update, for
    /// the views (read per frame, never computed there).
    pub lanes: HashMap<VocalLaneKey, VocalLaneRenderStatus>,
}

impl VocalRenderStatus {
    /// What a restore (undo, redo, load) invalidates: the per-clip keys
    /// are rebuilt from the restored file, failures and queued renders
    /// belong to the state being left. The memo and the settled view
    /// survive (see [`Self::expected`]).
    pub fn clear_for_restore(&mut self) {
        self.clip_unit_keys.clear();
        self.errors.clear();
        self.queued_fingerprint.clear();
        self.auto_render.clear();
    }

    /// A disk load: nothing of the previous project applies, so the first
    /// settle after it sees no edits.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The unit keys of the take the lane plays, from any of its installed
    /// clips (one render fans out to every placement, so they agree).
    pub fn installed_keys<'a>(
        &'a self,
        clips: impl IntoIterator<Item = &'a ClipId>,
    ) -> Option<&'a Arc<[u64]>> {
        clips.into_iter().find_map(|id| self.clip_unit_keys.get(id))
    }
}
