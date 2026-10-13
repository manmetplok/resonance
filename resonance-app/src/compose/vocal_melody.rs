//! What happens to a vocal lane's melody when its section's chords or
//! length change (vocal-melody-on-chord-change, BUG-70): the per-lane
//! flags, the project's remembered choice, the GUI question and its
//! messages. The behaviour lives in `update::compose::vocal_melody`.

use resonance_audio::types::TrackId;
use serde::{Deserialize, Serialize};

use super::VocalLaneKey;

/// What a chord or section-length edit does to a vocal lane's melody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocalMelodyChoice {
    /// Leave the notes as they are and flag the lane
    /// [`LaneMelodyFlags::melody_predates_chords`].
    Keep,
    /// Move each pitch to the nearest note that fits the new chords;
    /// rhythm, note count, lyrics and holds stay
    /// (`resonance_music_theory::refit_melody`).
    Refit,
    /// Re-derive the melody from the lane's generator (what every chord
    /// edit did before).
    Regenerate,
}

impl VocalMelodyChoice {
    pub const ALL: [VocalMelodyChoice; 3] = [Self::Keep, Self::Refit, Self::Regenerate];

    /// The modal's button label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "Keep melody",
            Self::Refit => "Re-fit pitches",
            Self::Regenerate => "Regenerate",
        }
    }
}

/// A vocal lane's melody provenance, persisted with the lane
/// ([`super::LaneGeneratorConfig::melody`]) so it rides undo and save.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LaneMelodyFlags {
    /// The lane's notes changed by something other than the generator —
    /// a vocal-roll edit, `notes.*`, a MIDI import into its clip — since
    /// it was last generated. Cleared by a (re)generate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hand_edited: bool,
    /// A chord or section-length change kept this lane's melody: its
    /// notes were written against chords the section no longer has.
    /// Cleared by a regenerate, a re-fit, or the next note edit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub melody_predates_chords: bool,
}

impl LaneMelodyFlags {
    /// Nothing to record — left out of the project file.
    pub fn is_clear(&self) -> bool {
        !self.hand_edited && !self.melody_predates_chords
    }
}

/// One lane a chord or length edit touched, and what became of it — the
/// control API's `vocal_lanes_affected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffectedVocalLane {
    pub definition_id: u64,
    pub track_id: TrackId,
    pub outcome: VocalMelodyChoice,
}

/// One row of the "what about the vocal melody?" question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocalMelodyPromptRow {
    pub definition_id: u64,
    pub track_id: TrackId,
    pub section_name: String,
    pub track_name: String,
    pub hand_edited: bool,
    pub choice: VocalMelodyChoice,
}

/// The open question. The chord edit that raised it has already
/// happened, with every listed lane kept and flagged; Apply carries out
/// the rows' choices inside that edit's undo entry, Cancel undoes the
/// edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocalMelodyPrompt {
    pub rows: Vec<VocalMelodyPromptRow>,
    /// "Remember for this project" — needs one choice for every row.
    pub remember: bool,
    /// The undo history's generation once the edit was recorded: the
    /// question is about that entry, and is dropped when the history
    /// moves on without it (an undo, a control edit, a load).
    pub history_generation: u64,
    /// Whether the edit has an undo entry Cancel can take back.
    pub undoable: bool,
    /// The undo stack's depth when the question opened, to tell an undo
    /// that closed it from other history moves.
    pub undo_len: usize,
}

impl VocalMelodyPrompt {
    /// The one choice every row shares, if they share one.
    pub fn uniform_choice(&self) -> Option<VocalMelodyChoice> {
        let first = self.rows.first()?.choice;
        self.rows.iter().all(|r| r.choice == first).then_some(first)
    }
}

/// Runtime state of the chord-change flow ([`super::ComposeState::vocal_melody`]).
#[derive(Debug, Clone, Default)]
pub struct VocalMelodyState {
    /// "Remember for this project": applied to every chord or length
    /// edit that names no choice of its own, GUI and control alike.
    /// Persisted as `ProjectFile::vocal_melody_choice` (project state,
    /// rides undo).
    pub remembered: Option<VocalMelodyChoice>,
    /// The choice the edit being dispatched asked for (a control call's
    /// `vocal_melody`), set only for the length of that dispatch.
    pub requested: Option<VocalMelodyChoice>,
    /// What the edits dispatched since the caller last took it did to
    /// each lane, for the control reply.
    pub affected: Vec<AffectedVocalLane>,
    /// Lanes kept because nothing chose for them, waiting for the GUI
    /// question (a control edit leaves them kept and flagged).
    pub undecided: Vec<VocalLaneKey>,
    /// The open question, when one is showing.
    pub prompt: Option<VocalMelodyPrompt>,
    /// The undecided lanes came from a coalesced control (a slider drag):
    /// the question waits for the control's release.
    pub parked_run: bool,
    /// Lanes a chord or length change reached while a Begin…Commit
    /// gesture was open, with the choice that applied (`None`: ask). They
    /// are only flagged until the gesture commits, then get their outcome
    /// once, at the final length.
    pub gesture_parked: Vec<(VocalLaneKey, Option<VocalMelodyChoice>)>,
}

/// Messages of the chord-change flow and the lane badge.
#[derive(Debug, Clone)]
pub enum VocalMelodyMsg {
    /// Re-fit a lane's pitches to its section's chords now (the badge's
    /// Re-fit button, `vocal.refit_melody`).
    Refit { definition_id: u64, track_id: TrackId },
    /// Re-derive a lane's melody from its generator, keeping its seed
    /// (the badge's Regenerate button).
    Regenerate { definition_id: u64, track_id: TrackId },
    /// Pick one row's choice in the open question.
    SetRowChoice { row: usize, choice: VocalMelodyChoice },
    /// "Apply to all lanes".
    SetAllChoices(VocalMelodyChoice),
    /// Toggle "Remember for this project".
    ToggleRemember,
    /// Carry out the rows' choices, inside the chord edit's undo entry.
    Apply,
    /// Esc / Cancel: undo the chord edit itself.
    Cancel,
    /// Settings › Compose "Ask again about vocal melodies": forget the
    /// project's remembered choice.
    ForgetRememberedChoice,
    /// A continuous control that can change chords (the schema
    /// substitution slider) was released: a question its drag raised
    /// opens now, not under the user's held mouse.
    ControlReleased,
}

impl VocalMelodyMsg {
    /// How this message interacts with the undo history. Exhaustive on
    /// purpose (ARCH-06 A6-4).
    ///
    /// `Apply` mutates the project but records nothing of its own: its
    /// outcome belongs to the chord edit that raised the question, whose
    /// entry snapshots the state before that edit, so one undo takes back
    /// both (the handler marks the project edited without a new entry).
    /// `Cancel` undoes that entry itself.
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            Self::Refit { .. } | Self::Regenerate { .. } | Self::ForgetRememberedChoice => {
                UndoAction::Record
            }
            Self::SetRowChoice { .. }
            | Self::SetAllChoices(_)
            | Self::ToggleRemember
            | Self::Apply
            | Self::Cancel
            | Self::ControlReleased => UndoAction::Skip,
        }
    }
}
