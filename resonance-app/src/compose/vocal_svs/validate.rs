//! Voicebank validation gate for the resolved pronunciation (todo #494).
//!
//! After the #493 resolver folds overrides + dictionaries + CMU-auto into
//! a per-note [`AssignedSyllable`] stream, every phoneme must clear two
//! gates before it reaches the SVS model:
//!
//! 1. **Valid ARPAbet** (#491): the symbol must canonicalise through
//!    [`g2p::canonical_phoneme`]. The upstream layers already canonicalise
//!    their input, but this is the final guarantee that no garbage token
//!    reaches the model as a PAD / token-0 and silently corrupts the
//!    segment.
//! 2. **Singable by the active voicebank** (#492): the symbol is checked
//!    against [`VoicebankPhonemes`]. One the bank covers is kept as-is;
//!    one it lacks but can substitute (e.g. Lilia `v` → `f`) is rewritten
//!    to the substitute; one with no acceptable substitute blocks the
//!    render.
//!
//! On success the returned syllables carry the *effective* (substituted)
//! phonemes — exactly what the segment builder feeds the model — with
//! every other field (label, slur, stress, [`PhonemeProvenance`]) carried
//! through, so downstream re-render scoping and the phoneme strip keep the
//! provenance / affected state. On failure the offending syllables are
//! reported rather than corrupting the segment.
//!
//! [`PhonemeProvenance`]: super::PhonemeProvenance

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::{self, AssignedSyllable};

use super::phonemes::{min_articulation_sec, PhonemeFate, VoicebankPhonemes};
use super::rests::RestRule;
use super::registry::BankProfile;

/// Why one phoneme failed the voicebank gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidPhonemeReason {
    /// Not a valid ARPAbet symbol (#491) — [`g2p::canonical_phoneme`]
    /// rejected it, so it would reach the model as an unknown token.
    NotArpabet,
    /// Valid ARPAbet, but the active voicebank can neither sing it nor map
    /// it onto anything it has (#492 — [`PhonemeFate::Unsupported`]).
    UnsupportedByVoicebank,
}

impl InvalidPhonemeReason {
    /// A short human-readable tag for error messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            InvalidPhonemeReason::NotArpabet => "not ARPAbet",
            InvalidPhonemeReason::UnsupportedByVoicebank => "unsupported by voicebank",
        }
    }
}

/// One syllable that blocks the render, with the offending phoneme and
/// why. `note_index` is the position in the clip's note list (so the UI
/// can badge the right note); `label` is the syllable's surface glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSyllable {
    /// Index of the offending note in the clip's note list.
    pub note_index: usize,
    /// The syllable's surface label (glyphs to display).
    pub label: String,
    /// The whole word the syllable belongs to (its labels joined), so an
    /// error can name the word that cannot be sung.
    pub word: String,
    /// The exact symbol that failed the gate.
    pub phoneme: String,
    /// Whether it failed the ARPAbet or the voicebank gate.
    pub reason: InvalidPhonemeReason,
}

/// Validate and substitute every phoneme in the resolved per-note stream
/// against `bank`. Returns the syllables with their *effective*
/// (substituted) phonemes on success — every other field carried through
/// unchanged so provenance survives — or every blocking syllable on
/// failure.
///
/// All invalid phonemes across all syllables are collected before
/// returning, so the caller can report every problem at once rather than
/// one-per-render-press. Empty phoneme lists (e.g. a note the draft never
/// reached) pass through untouched; the segment builder handles them with
/// its own vowel fallback.
pub fn validate_for_voicebank(
    assigned: &[AssignedSyllable],
    bank: &BankProfile,
) -> Result<Vec<AssignedSyllable>, Vec<InvalidSyllable>> {
    let phonemes = VoicebankPhonemes::new(bank);
    let words = word_labels(assigned);
    let mut out = Vec::with_capacity(assigned.len());
    let mut invalid = Vec::new();

    for (note_index, syl) in assigned.iter().enumerate() {
        let mut effective = Vec::with_capacity(syl.phonemes.len());
        let mut fail = |ph: &str, reason| {
            invalid.push(InvalidSyllable {
                note_index,
                label: syl.label.clone(),
                word: words[note_index].clone(),
                phoneme: ph.to_string(),
                reason,
            })
        };
        for &ph in &syl.phonemes {
            // Gate 1: must be a real ARPAbet symbol. `canonical_phoneme`
            // also accepts the `AP`/`SP` silence markers, but those are
            // never part of a syllable's phoneme list (the builder inserts
            // them separately), so a hit here means a genuine phone.
            if g2p::canonical_phoneme(ph).is_none() {
                fail(ph, InvalidPhonemeReason::NotArpabet);
                continue;
            }
            // Gate 2: the active voicebank must sing it directly or map it
            // onto a phone it has. `Substituted` rewrites to the singable
            // form; `Unsupported` blocks.
            match phonemes.resolve(ph) {
                PhonemeFate::Direct => effective.push(ph),
                PhonemeFate::Substituted(sub) => effective.push(sub),
                PhonemeFate::Unsupported => fail(ph, InvalidPhonemeReason::UnsupportedByVoicebank),
            }
        }
        let mut s = syl.clone();
        s.phonemes = effective;
        out.push(s);
    }

    if invalid.is_empty() {
        Ok(out)
    } else {
        Err(invalid)
    }
}

/// The word each note's syllable belongs to: the labels of the notes from
/// the word's first syllable to its `is_word_end`, joined. Slur notes
/// (`+`) are left out of the spelling.
fn word_labels(assigned: &[AssignedSyllable]) -> Vec<String> {
    let mut out = vec![String::new(); assigned.len()];
    let mut start = 0;
    for (i, syl) in assigned.iter().enumerate() {
        if syl.is_word_end || i + 1 == assigned.len() {
            let word: String = assigned[start..=i]
                .iter()
                .filter(|s| !s.is_slur)
                .map(|s| s.label.as_str())
                .collect();
            for slot in &mut out[start..=i] {
                slot.clone_from(&word);
            }
            start = i + 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Intelligibility report
// ---------------------------------------------------------------------------
//
// `validate_for_voicebank` above answers "can this render at all?".  The
// report below answers the question that actually decides whether a
// listener understands the words: "is each note long enough, and pitched
// where the bank sings clearly?"  Both failures are silent — the render
// succeeds and sounds wrong — so a caller driving the app over the
// control API had no way to self-correct short of bouncing audio and
// listening.  `song.vocal` surfaces this per note.

/// One note's articulation budget: what it has to sing, how long it has,
/// and whether that is enough.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteArticulation {
    /// Index into the clip's note list.
    pub note_index: usize,
    /// The syllable's surface label (`"+"` for a slur).
    pub label: String,
    /// Effective phonemes this note sings.
    pub phonemes: Vec<&'static str>,
    /// The note's singing time in seconds — time until the next note,
    /// capped at its own length when a genuine rest follows. This is the
    /// same figure the segment builder divides up, so the two agree by
    /// construction.
    pub duration_sec: f64,
    /// Shortest duration that can articulate `phonemes` without pushing
    /// any of them below its audibility floor
    /// ([`super::phonemes::min_articulation_sec`]).
    pub min_duration_sec: f64,
    /// `duration_sec < min_duration_sec` — the note *will* smear. Fix by
    /// lengthening it, or by splitting the word over more notes so each
    /// note carries fewer phonemes.
    pub too_short: bool,
    /// MIDI pitch (60 = C4).
    pub pitch: u8,
    /// Outside the voicebank's comfortable range
    /// ([`BankProfile::comfortable_range`]) — the model extrapolates formants
    /// there and consonants blur even on a long note.
    pub out_of_range: bool,
}

/// Per-note articulation report for one clip.
///
/// `assigned` is the resolved (ideally voicebank-validated) syllable
/// stream — one entry per note, exactly what
/// [`super::build_segment`] will sing. Notes and syllables are zipped by
/// index; extra notes beyond `assigned` are skipped.
///
/// The duration model mirrors the segment builder's slot rule: a note
/// sings until the next note starts, unless the gap is long enough to be
/// a genuine rest, in which case it sings for its own written length.
/// (The builder additionally pulls a syllable's onset consonants back
/// across the previous boundary; that moves time between neighbours
/// without changing either note's phoneme budget materially, so it is not
/// modelled here.)
///
/// `rest_threshold_beats` is the lane's, so the report bridges exactly
/// the gaps the segment builder sings legato. `speaker` is the lane's, for
/// its comfortable range.
#[allow(clippy::too_many_arguments)]
pub fn articulation_report_for(
    notes: &[MidiNote],
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
    bank: &BankProfile,
    speaker: Option<&str>,
    rest_threshold_beats: f32,
) -> Vec<NoteArticulation> {
    let rest_rule = RestRule::from_beats(rest_threshold_beats, bpm);
    let seconds_per_tick = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter.max(1) as f64);
    let range = bank.comfortable_range(speaker);
    let (lo, hi) = (range.low, range.high);
    let mut out = Vec::with_capacity(notes.len().min(assigned.len()));
    for (i, n) in notes.iter().enumerate() {
        let Some(syl) = assigned.get(i) else { break };
        let next_start_tick = notes
            .get(i + 1)
            .map(|nx| nx.start_tick)
            .unwrap_or(n.start_tick + n.duration_ticks);
        let slot_sec =
            (next_start_tick.saturating_sub(n.start_tick) as f64 * seconds_per_tick).max(0.05);
        let own_sec = (n.duration_ticks as f64 * seconds_per_tick).max(0.05);
        let duration_sec = if rest_rule.is_rest(slot_sec, own_sec) {
            own_sec
        } else {
            slot_sec
        };
        let min_duration_sec = min_articulation_sec(&syl.phonemes);
        out.push(NoteArticulation {
            note_index: i,
            label: syl.label.clone(),
            phonemes: syl.phonemes.clone(),
            duration_sec,
            min_duration_sec,
            too_short: min_duration_sec > duration_sec,
            pitch: n.note,
            out_of_range: n.note < lo || n.note > hi,
        });
    }
    out
}
