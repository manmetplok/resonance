//! Why a vocal render produced no audio, by kind, so the control API can
//! report `voicebank_missing` apart from any other failure
//! (vocal-mcp-control-and-quality.md §4.6).

use super::registry::VoicebankMissing;

/// A vocal render that failed.
#[derive(Debug, Clone, PartialEq)]
pub enum VocalRenderError {
    /// The lane's bank, or the vocoder it names, is not installed.
    VoicebankMissing(VoicebankMissing),
    /// Every render unit came out with no phonemes: there is nothing to
    /// sing (lyrics that resolve to nothing, notes the draft never
    /// reaches).
    NothingToSing,
    /// The SVS pipeline or the WAV write failed.
    Failed(String),
}

impl VocalRenderError {
    /// The app-side kind, for the control layer to map onto its wire kind.
    pub fn kind(&self) -> VocalRenderErrorKind {
        match self {
            VocalRenderError::VoicebankMissing(_) => VocalRenderErrorKind::VoicebankMissing,
            VocalRenderError::NothingToSing => VocalRenderErrorKind::NothingToSing,
            VocalRenderError::Failed(_) => VocalRenderErrorKind::Failed,
        }
    }
}

impl std::fmt::Display for VocalRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VocalRenderError::VoicebankMissing(m) => m.fmt(f),
            VocalRenderError::NothingToSing => f.write_str(
                "the vocal lane resolved to no phonemes to sing; check its lyrics \
                 (song_vocal shows each note's phonemes)",
            ),
            VocalRenderError::Failed(e) => f.write_str(e),
        }
    }
}

impl From<String> for VocalRenderError {
    fn from(e: String) -> Self {
        VocalRenderError::Failed(e)
    }
}

/// [`VocalRenderError`] without its payload: what a render-failure message
/// carries to the job board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocalRenderErrorKind {
    VoicebankMissing,
    NothingToSing,
    Failed,
}
