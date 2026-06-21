//! Non-destructive vocal-tuning (graphical pitch & timing correction)
//! data model attached to an [`AudioClip`](super::AudioClip).
//!
//! A clip carries `Option<VocalTuning>`: `None` means untuned (the
//! existing behaviour, zero overhead), `Some(_)` holds the analysis
//! cache (detected f0 contour + note blobs) plus the user's per-note and
//! global edits. **All of this is data only** — the original
//! [`ClipSource`](super::ClipSource) PCM is never mutated. The render and
//! bounce paths (todo #358) read this model live to resynthesise corrected
//! audio; analysis (todo #357) fills in `contour`/`notes`.
//!
//! This module is deliberately self-contained and dependency-free so it is
//! trivially serializable for project persistence (todo #363) and so the
//! engine crate need not depend on `resonance-music-theory` just to hold
//! the data. The scale-snap logic that consumes [`TuningScale`] lives in
//! `resonance-music-theory`; [`TuningScale::intervals`] mirrors its mode
//! interval tables so the two stay in lock-step.

/// Musical scale used to snap corrected pitches to in-key scale degrees.
///
/// `Chromatic` performs no scale snapping (every semitone is allowed);
/// the remaining variants mirror `resonance_music_theory::scale::Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TuningScale {
    /// No scale snapping — all twelve semitones are valid targets.
    #[default]
    Chromatic,
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    HarmonicMinor,
    MelodicMinor,
}

impl TuningScale {
    /// Semitone offsets of the scale degrees above the key root, or `None`
    /// for [`TuningScale::Chromatic`] (which admits every semitone). The
    /// non-chromatic tables match `resonance_music_theory::scale::Mode`.
    pub fn intervals(self) -> Option<&'static [u8]> {
        Some(match self {
            TuningScale::Chromatic => return None,
            TuningScale::Major => &[0, 2, 4, 5, 7, 9, 11],
            TuningScale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            TuningScale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            TuningScale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            TuningScale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            TuningScale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            TuningScale::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            TuningScale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            TuningScale::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
        })
    }
}

/// One analysis frame of the detected fundamental-frequency (f0) contour,
/// produced by the monophonic pitch detector (todo #352). Frames are at a
/// fixed analysis hop; `frame` anchors each one to the clip's PCM so the
/// contour survives trims and timeline moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Frame {
    /// Time of this frame as an offset, in stereo sample frames, from the
    /// start of the clip's audio data (not the timeline).
    pub frame: u64,
    /// Detected fundamental frequency in Hz. `0.0` when unvoiced.
    pub f0_hz: f32,
    /// Detection confidence in `[0, 1]`.
    pub confidence: f32,
    /// Whether this frame is voiced (carries a pitched fundamental).
    pub voiced: bool,
}

/// The non-destructive edit state for a single detected note. Every field
/// at its default leaves the note exactly as detected — no pitch pull, no
/// timing shift — so an analysed-but-untouched clip renders bit-identical
/// to the original.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEdit {
    /// Target pitch offset from the note's detected mean, in semitones.
    /// Dragging a note up/down in the editor sets this; fractional values
    /// allow fine retune.
    pub semitone_offset: f32,
    /// Correction strength in `[0, 1]`: how strongly the note is pulled
    /// toward its (snapped) target pitch. `0.0` leaves it as detected,
    /// `1.0` lands it fully on target.
    pub correction_strength: f32,
    /// Drift / transition amount in `[0, 1]`: how much of the note's
    /// natural pitch drift and note-to-note glide is preserved (`1.0`) vs
    /// flattened toward a steady tone (`0.0`).
    pub drift: f32,
    /// Timing nudge in sample frames (signed): shifts this note earlier
    /// (negative) or later (positive) without moving its neighbours.
    pub timing_nudge_frames: i64,
}

impl Default for NoteEdit {
    fn default() -> Self {
        // Identity edit: on-pitch pull disabled, full natural drift kept,
        // no timing nudge -> renders as detected.
        NoteEdit {
            semitone_offset: 0.0,
            correction_strength: 0.0,
            drift: 1.0,
            timing_nudge_frames: 0,
        }
    }
}

impl NoteEdit {
    /// True when this edit would not change the detected note at all
    /// (no pitch pull and no timing shift). Used to skip resynthesis for
    /// untouched notes on the render path.
    pub fn is_identity(&self) -> bool {
        self.correction_strength == 0.0 && self.timing_nudge_frames == 0
    }
}

/// A detected note "blob": a contiguous run of voiced analysis frames
/// grouped by the segmentation pass (todo #354), carrying its detected
/// geometry plus the user's [`NoteEdit`]. Re-analysis replaces the
/// geometry; the edit travels with the blob.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteBlob {
    /// Onset, in stereo sample frames from the start of the clip's audio
    /// data.
    pub start_frame: u64,
    /// Offset (exclusive), in stereo sample frames from the start of the
    /// clip's audio data. Invariant: `end_frame >= start_frame`.
    pub end_frame: u64,
    /// Mean detected pitch over the blob, as a fractional MIDI note number
    /// (e.g. `69.0` = A4 / 440 Hz).
    pub mean_pitch_midi: f32,
    /// Per-analysis-frame deviation from `mean_pitch_midi`, in cents. This
    /// captures vibrato and drift within the note; an empty vector is
    /// valid (deviation unknown / flat).
    pub cents_contour: Vec<f32>,
    /// Non-destructive edit applied to this note.
    pub edit: NoteEdit,
}

impl NoteBlob {
    /// Length of the detected note in stereo sample frames.
    pub fn duration_frames(&self) -> u64 {
        self.end_frame.saturating_sub(self.start_frame)
    }

    /// The note's target pitch as a fractional MIDI note number, i.e. the
    /// detected mean shifted by the edit's semitone offset. This is the
    /// *pre-snap* target; scale snapping (music-theory) is applied on the
    /// render path.
    pub fn target_pitch_midi(&self) -> f32 {
        self.mean_pitch_midi + self.edit.semitone_offset
    }
}

/// Global tuning parameters that apply across every note in a clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalTuning {
    /// Key root as a pitch class, `0` = C through `11` = B. Combined with
    /// [`GlobalTuning::scale`] to decide which pitches notes snap to.
    pub key: u8,
    /// Scale used for snap. [`TuningScale::Chromatic`] disables snapping.
    pub scale: TuningScale,
    /// Overall correction amount in `[0, 1]`: `0.0` = natural (subtle),
    /// `1.0` = hard (full auto-tune). Multiplies each note's own
    /// [`NoteEdit::correction_strength`] on the render path.
    pub correction_amount: f32,
}

impl Default for GlobalTuning {
    fn default() -> Self {
        GlobalTuning {
            key: 0,
            scale: TuningScale::Chromatic,
            correction_amount: 0.0,
        }
    }
}

/// The complete non-destructive vocal-tuning model attached to a clip.
///
/// `default()` is the empty model (no analysis, no edits, natural global
/// settings) — attaching it to a clip changes nothing until analysis fills
/// in [`Self::contour`] / [`Self::notes`] and the user makes edits.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VocalTuning {
    /// Cached f0 contour from the most recent analysis. Empty before the
    /// clip has been analysed.
    pub contour: Vec<F0Frame>,
    /// Detected notes (in onset order) with their per-note edits.
    pub notes: Vec<NoteBlob>,
    /// Global key / scale / correction-amount parameters.
    pub global: GlobalTuning,
}

impl VocalTuning {
    /// Mutable access to the note blob at `index`, or `None` if out of
    /// range.
    pub fn note_mut(&mut self, index: usize) -> Option<&mut NoteBlob> {
        self.notes.get_mut(index)
    }

    /// Set the target semitone offset of the note at `index` (dragging it
    /// vertically in the editor). Returns `false` if the index is out of
    /// range.
    pub fn set_note_semitone_offset(&mut self, index: usize, semitones: f32) -> bool {
        match self.notes.get_mut(index) {
            Some(note) => {
                note.edit.semitone_offset = semitones;
                true
            }
            None => false,
        }
    }

    /// Set the per-note correction strength (clamped to `[0, 1]`) of the
    /// note at `index`. Returns `false` if the index is out of range.
    pub fn set_note_correction_strength(&mut self, index: usize, strength: f32) -> bool {
        match self.notes.get_mut(index) {
            Some(note) => {
                note.edit.correction_strength = strength.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }

    /// Set the per-note drift / transition amount (clamped to `[0, 1]`) of
    /// the note at `index`. Returns `false` if the index is out of range.
    pub fn set_note_drift(&mut self, index: usize, drift: f32) -> bool {
        match self.notes.get_mut(index) {
            Some(note) => {
                note.edit.drift = drift.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }

    /// Add `delta_frames` to the timing nudge of the note at `index`
    /// (dragging it horizontally). Saturates on overflow. Returns `false`
    /// if the index is out of range.
    pub fn nudge_note_timing(&mut self, index: usize, delta_frames: i64) -> bool {
        match self.notes.get_mut(index) {
            Some(note) => {
                note.edit.timing_nudge_frames =
                    note.edit.timing_nudge_frames.saturating_add(delta_frames);
                true
            }
            None => false,
        }
    }

    /// Set the global key (pitch class) and scale for snap.
    pub fn set_key_scale(&mut self, key: u8, scale: TuningScale) {
        self.global.key = key % 12;
        self.global.scale = scale;
    }

    /// Set the global overall correction amount, clamped to `[0, 1]`.
    pub fn set_correction_amount(&mut self, amount: f32) {
        self.global.correction_amount = amount.clamp(0.0, 1.0);
    }

    /// Reset every per-note edit and the global correction amount back to
    /// their identity defaults while keeping the detected analysis
    /// (`contour` and note geometry) intact. Key/scale are preserved.
    pub fn reset_edits(&mut self) {
        for note in &mut self.notes {
            note.edit = NoteEdit::default();
        }
        self.global.correction_amount = 0.0;
    }

    /// True when any note carries a non-identity edit or the global
    /// correction amount is engaged — i.e. the clip would render
    /// differently from the untouched detection.
    pub fn has_edits(&self) -> bool {
        self.global.correction_amount != 0.0
            || self.notes.iter().any(|n| !n.edit.is_identity())
    }
}
