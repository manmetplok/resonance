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

/// Melodic-sequence pattern (Open Music Theory v2, sequences): a
/// *model* restated as transposed copies at a fixed interval per copy.
/// The engine works in semitones relative to a per-chord anchor; the
/// downstream harmony alignment (`align_to_harmony`) snaps each note to
/// the chord/scale, which is what realizes the transposition
/// *diatonically* — the per-copy step here is the chromatic
/// approximation that alignment then corrects per scale degree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceKind {
    /// Each copy a fifth below the previous (the descending-fifths
    /// sequence — model, −5th, −5th…).
    DescendingFifths,
    /// Each copy a third below the previous. Encoded as a minor third:
    /// the diatonic third below most scale degrees is three semitones,
    /// and alignment re-snaps the rest.
    DescendingThirds,
    /// Each copy a step above the previous — the classic rising
    /// (ascending 5–6) sequence.
    Ascending56,
}

impl SequenceKind {
    /// Per-copy transposition in semitones (diatonicized downstream by
    /// the harmony alignment).
    pub fn step_semitones(self) -> i8 {
        match self {
            SequenceKind::DescendingFifths => -7,
            SequenceKind::DescendingThirds => -3,
            SequenceKind::Ascending56 => 2,
        }
    }

    /// Anchor-relative offset of each of `statements` statements
    /// (model + copies), stepping by `step_semitones` per statement.
    /// The run is centered on the anchor — a descending-fifths run
    /// starting *at* the anchor would spend its whole length below it
    /// and be flattened against the register floor at render time.
    pub fn offsets(self, statements: usize) -> Vec<i8> {
        let step = i16::from(self.step_semitones());
        let n = statements.max(1) as i16;
        let start = -step * (n - 1) / 2;
        (0..n)
            .map(|s| (start + step * s).clamp(-64, 64) as i8)
            .collect()
    }
}

/// Transformation to apply to a motif when developing it across phrases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
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
    /// Melodic sequence (OMT v2): the motif's leading `model_len` notes
    /// form the model, restated as `copies` transposed copies at
    /// `kind`'s per-copy interval. The realized cell is the
    /// concatenation model + copies, centered on the anchor, so one
    /// pass through the cell sounds the whole sequence; harmony
    /// alignment makes each copy diatonic.
    Sequence {
        kind: SequenceKind,
        /// Transposed copies after the model (2–3 typical).
        copies: u8,
        /// Length of the model (the motif's head), in notes.
        model_len: usize,
    },
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
    /// Contour-amplitude multiplier from the section climax plan: the
    /// designated climax-carrier phrase (and the consequent paired
    /// with it) keeps the full contour swing (1.0); secondary phrases
    /// draw their contours at reduced amplitude so their peaks sit
    /// naturally below the carrier's and the post-realization section
    /// pass rarely has to demote.
    pub(super) peak_scale: f32,
}
