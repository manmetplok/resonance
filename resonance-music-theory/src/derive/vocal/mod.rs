//! Vocal generator: parameters, lyric corpus, and `derive_vocal` melody
//! synthesis.
//!
//! Lyric generation is template-based. A small mood-tagged corpus of
//! lines is bundled below; each line is pre-marked with a rhyme group
//! and a syllable count. The generator picks lines that satisfy the
//! requested mood, rhyme scheme, and syllable range, then writes them
//! back into `VocalParams::draft`.
//!
//! Melody generation walks the section's chord progression once per
//! syllable, picking a pitch that respects `range`, the active `contour`,
//! the `chord_tone_anchor` probability, the `leap_range` step-vs-leap
//! bias, and (optionally) the section's scale.

mod lyrics;
mod melody;
mod params;
mod style;

pub use lyrics::{generate_lyrics, LyricLine};
pub use melody::{count_syllables, vocal_phrase_spans};
pub use params::{
    SyllableMode, VocalContour, VocalMood, VocalParams, VocalParamsError, VocalPov,
    VocalRhymeScheme, VocalSinger, VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank,
    VoiceType,
};

use crate::scale::Scale;

use super::{GeneratedNote, TimedChord};
use melody::{apply_motif_pitches, enforce_no_overlap, scale_from_chords, total_beats, MotifPitchContext};
use style::derive_with_profile;

/// Derive MIDI notes for a vocal line. Dispatches to the per-style
/// generator chosen by `params.style`; each style is its own per-
/// syllable walk with distinct pitch and rhythm strategies.
///
/// All styles share the same one-note-per-syllable invariant — the SVS
/// pipeline indexes syllable phonemes by note position, so a melismatic
/// style would need pipeline changes too (see vocal_svs.rs).
pub fn derive_vocal(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    // Default to 4/4 when the caller doesn't know the meter — keeps
    // older / test call sites working unchanged.
    derive_vocal_with_meter(chords, params, ticks_per_beat, 4, seed)
}

/// Same as [`derive_vocal`] but also takes the section's
/// `beats_per_bar` so the generators know where bar boundaries fall.
/// 4 = 4/4, 3 = 3/4 / waltz, 6 = 6/8 (compound), 2 = 2/4 / cut. The
/// app should pass its current transport time-signature numerator
/// here. Affects beat-strength accents and per-line phrase-start
/// offsets (pickup, anacrusis, off-beat starts).
pub fn derive_vocal_with_meter(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    beats_per_bar: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    derive_vocal_with_motif(chords, params, ticks_per_beat, beats_per_bar, None, seed)
}

/// Same as [`derive_vocal_with_meter`] but optionally re-skins the
/// per-syllable pitches with a shared motif. Pass `motif_intervals` as
/// a non-empty slice of signed semitone offsets relative to the chord
/// root; the style's rhythm + dynamics still drive the surface, only
/// pitches are overridden. The final syllable of each line keeps its
/// style cadence landing so phrases still resolve.
///
/// When `params.use_section_motif` is `false` or `motif_intervals` is
/// empty, the motif is ignored — equivalent to plain
/// [`derive_vocal_with_meter`].
pub fn derive_vocal_with_motif(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    beats_per_bar: u32,
    motif_intervals: Option<&[i8]>,
    seed: u64,
) -> Vec<GeneratedNote> {
    let ctx = match VocalContext::build(chords, params, ticks_per_beat, beats_per_bar, seed) {
        Some(ctx) => ctx,
        None => return Vec::new(),
    };
    let mut notes = derive_with_profile(&ctx);
    if params.use_section_motif {
        if let Some(intervals) = motif_intervals {
            if !intervals.is_empty() {
                apply_motif_pitches(
                    &mut notes,
                    &MotifPitchContext {
                        motif_intervals: intervals,
                        line_syllables: &ctx.line_syllables,
                        chords: ctx.chords,
                        section_beats: ctx.section_beats,
                        scale: ctx.scale,
                        range: (ctx.lo, ctx.hi),
                        tpb: ctx.tpb,
                    },
                );
            }
        }
    }
    enforce_no_overlap(&mut notes, ctx.tpb);
    notes
}

/// Bundle of validated inputs every per-style generator needs.
/// Building it once up front lets each style stay focused on the
/// musical decisions instead of repeating the same boilerplate.
pub(in crate::derive::vocal) struct VocalContext<'a> {
    pub(in crate::derive::vocal) chords: &'a [TimedChord],
    pub(in crate::derive::vocal) params: &'a VocalParams,
    pub(in crate::derive::vocal) tpb: u64,
    pub(in crate::derive::vocal) section_beats: u32,
    /// Section time-signature numerator. 4 = 4/4 (default), 3 = 3/4,
    /// 6 = 6/8, 2 = 2/4. Drives `beat_strength` and the per-line
    /// `phrase_start_offset`.
    pub(in crate::derive::vocal) beats_per_bar: u32,
    pub(in crate::derive::vocal) lo: u8,
    pub(in crate::derive::vocal) hi: u8,
    /// One entry per draft line, in order, holding the line's
    /// mechanical syllable count (matches what the SVS pipeline walks).
    pub(in crate::derive::vocal) line_syllables: Vec<u32>,
    pub(in crate::derive::vocal) total_syl: u32,
    pub(in crate::derive::vocal) seed: u64,
    pub(in crate::derive::vocal) scale: Option<Scale>,
}

impl<'a> VocalContext<'a> {
    fn build(
        chords: &'a [TimedChord],
        params: &'a VocalParams,
        ticks_per_beat: u32,
        beats_per_bar: u32,
        seed: u64,
    ) -> Option<Self> {
        if chords.is_empty() || params.draft.is_empty() {
            return None;
        }
        let section_beats = total_beats(chords);
        if section_beats == 0 {
            return None;
        }
        let (lo, hi) = params.range;
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        if lo == hi {
            return None;
        }
        // We use `count_syllables` (mechanical count of `·`-separators
        // + whitespace) rather than the corpus's `syllables` field
        // because the SVS render path walks words/syllables with the
        // same mechanical count — trusting the corpus field can leave
        // dangling notes that surface as phantom "ah" syllables.
        let line_syllables: Vec<u32> = params
            .draft
            .iter()
            .map(|l| count_syllables(&l.text))
            .collect();
        let total_syl: u32 = line_syllables.iter().sum();
        if total_syl == 0 {
            return None;
        }
        let scale = if params.stay_in_scale {
            scale_from_chords(chords)
        } else {
            None
        };
        Some(Self {
            chords,
            params,
            tpb: ticks_per_beat as u64,
            section_beats,
            beats_per_bar: beats_per_bar.max(1),
            lo,
            hi,
            line_syllables,
            total_syl,
            seed,
            scale,
        })
    }
}
