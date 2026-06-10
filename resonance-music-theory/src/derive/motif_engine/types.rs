// Shared motif-engine types: motif cells, transformations, and phrase plans.

use super::super::cadence::CadenceGoal;

/// A single note in a motif, stored as a relative interval from an anchor
/// pitch so that transposition and inversion are simple arithmetic.
#[derive(Debug, Clone, Copy)]
pub(in crate::derive) struct MotifNote {
    /// Signed interval in semitones from the motif's anchor pitch.
    pub(in crate::derive) interval: i8,
    /// Duration as a multiple of a base rhythmic unit.
    pub(in crate::derive) duration_ratio: u8,
    /// Slight velocity emphasis on this note.
    pub(in crate::derive) accent: bool,
    /// True if this entry is a rest — the per-chord cursor still advances
    /// by `duration_ratio` but no MIDI note is emitted.
    pub(in crate::derive) silent: bool,
}

/// Transformation to apply to a motif when developing it across phrases.
#[derive(Debug, Clone, Copy)]
pub(in crate::derive) enum Transform {
    Identity,
    TransposeUp(i8),
    TransposeDown(i8),
    Invert,
    Retrograde,
    Augment,
    Diminish,
    Fragment(usize),
}

/// Internal contour shape for a phrase.
#[derive(Debug, Clone, Copy)]
pub(super) enum Contour {
    Arch,
    Descending,
    Ascending,
    Wave,
}

/// Plan for a single melodic phrase.
pub(in crate::derive) struct PhrasePlan {
    pub(in crate::derive) chord_range: (usize, usize),
    pub(super) contour: Contour,
    pub(super) is_consequent: bool,
    /// Goal cadence for the phrase ending (weak for antecedents,
    /// strong for consequents with a ~10% deceptive swap). Realized by
    /// `cadence::apply_cadence_formula` on the rendered notes.
    pub(super) cadence: CadenceGoal,
}
