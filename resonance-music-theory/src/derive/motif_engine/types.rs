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
    /// Straight syncopation (Open Music Theory, rhythm in pop music):
    /// halve the first duration and shift every later onset earlier by
    /// that half, extending the final note to keep the cell's span.
    /// Operates at half the pattern's base division — eighth-level for
    /// quarter-based patterns, sixteenth-level for eighth-based ones.
    Syncopate,
}

/// Internal contour shape for a phrase.
#[derive(Debug, Clone, Copy)]
pub(super) enum Contour {
    Arch,
    Descending,
    Ascending,
    Wave,
}

/// Grammatical role of one phrase inside its form group (Open Music
/// Theory v2, phrase archetypes). Phrases are planned in groups —
/// a *sentence* (basic idea, varied repeat, continuation, cadential
/// continuation) or a *period* (antecedent, consequent) — instead of
/// drawing each phrase's treatment independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseGrammarRole {
    /// Sentence presentation, first statement of the basic idea.
    /// Tonic-prolonging; no cadence.
    BasicIdea,
    /// Sentence presentation, varied repeat of the basic idea (small
    /// transposition or exact repeat). No cadence.
    VariedRepeat,
    /// Sentence continuation: fragmentation of the idea's head motive
    /// at doubled surface-rhythm density. No cadence yet — the drive
    /// continues into the cadential phrase.
    Continuation,
    /// Final continuation phrase: keeps the fragmented head + dense
    /// surface, and carries the sentence's one real (strong) cadence.
    ContinuationCadence,
    /// Period opening: ends weak (HC, sometimes IAC).
    Antecedent,
    /// Period close: reuses the antecedent's opening (same transform)
    /// and swaps the ending weak→strong (PAC, ~10% deceptive).
    Consequent,
}

impl PhraseGrammarRole {
    /// Does this phrase close its group (strong cadence + root-snap
    /// baseline + descending contour bias)?
    pub fn closes(self) -> bool {
        matches!(
            self,
            PhraseGrammarRole::Consequent | PhraseGrammarRole::ContinuationCadence
        )
    }

    /// Is this phrase part of a sentence continuation? Continuations
    /// tile their fragmented head motive at an accelerated rate — the
    /// realizer doubles the surface-rhythm density relative to the
    /// presentation (OMT's "faster surface rhythm" drive toward the
    /// cadence).
    pub(super) fn is_continuation(self) -> bool {
        matches!(
            self,
            PhraseGrammarRole::Continuation | PhraseGrammarRole::ContinuationCadence
        )
    }
}

/// Plan for a single melodic phrase.
pub(in crate::derive) struct PhrasePlan {
    pub(in crate::derive) chord_range: (usize, usize),
    pub(super) contour: Contour,
    /// Grammatical role within the planned sentence/period group.
    pub(super) role: PhraseGrammarRole,
    /// Goal cadence for the phrase ending, realized by
    /// `cadence::apply_cadence_formula` on the rendered notes. `None`
    /// for sentence presentation/continuation phrases — they prolong
    /// without cadencing; only the group's closing phrase (consequent
    /// or cadential continuation, plus standalone antecedents) carries
    /// a goal.
    pub(super) cadence: Option<CadenceGoal>,
}
