//! The render unit as the bank's own duration / pitch / variance models
//! see it ([`BankScore`], see `resonance_svs::predict::score`), built from
//! the rule-based phoneme track so the two always describe the same
//! phonemes and the rule durations stay the fallback.
//!
//! * **Groups** start at anchors: a note's first vowel — or, in a
//!   consonant–glide–vowel onset (`b r ay`), the glide, as OpenUtau does —
//!   and every rest and written breath. A syllable's onset consonants sit
//!   in the group before its anchor, sung before the beat. The anchor time
//!   is where the rule layout starts that phoneme: the beat, or a little
//!   after when the onset could not be fully anticipated.
//! * **Held notes** (`_`) are merged the way DiffSinger was trained on
//!   them: the held vowel is one phoneme spanning both notes (the rule
//!   track's repeated vowel folds into it) and the hold's coda consonants
//!   close the group; the pitch model still sees the held note as a note,
//!   so it slides to it.

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::{self, AssignedSyllable};
use resonance_svs::predict::{BankScore, ScoreGroup, ScoreNote, ScorePhone};

use super::duration::{EntryRole, PhonemeTrack};

/// Most of the leading pad the first syllable's onset may take under the
/// bank's duration model, keeping 50 ms of silence (the rules allow 0.2 s,
/// `PAD_ONSET_MAX_SEC`).
pub(crate) const MODEL_LEAD_MAX_SEC: f64 = super::super::SEGMENT_PAD_SEC - 0.05;

/// ARPAbet glides: a consonant–glide–vowel onset starts its note on the
/// glide (OpenUtau's `semivowel` / `liquid` types).
fn is_glide(ph: &str) -> bool {
    matches!(ph, "w" | "y" | "l" | "r")
}

/// Build the bank models' score of a unit from its rule track.
pub(super) fn build_bank_score(
    track: &PhonemeTrack,
    notes: &[MidiNote],
    assigned: &[AssignedSyllable],
) -> BankScore {
    let starts: Vec<f64> = track
        .ph_dur
        .iter()
        .scan(0.0, |t, d| {
            let s = *t;
            *t += d;
            Some(s)
        })
        .collect();
    let total_sec: f64 = track.ph_dur.iter().sum();
    let lang = |k: usize| track.languages.get(k).copied().unwrap_or(0);
    let midi_of = |i: usize| notes.get(i).map_or(60, |n| n.note as i64);
    // The anchor phoneme of each note: its first vowel (or the glide
    // before it after a consonant), else its first phoneme.
    let anchor_index = |phones: &[&str]| -> usize {
        match phones.iter().position(|p| !g2p::is_consonant(p)) {
            Some(v) if v >= 2 && is_glide(phones[v - 1]) && g2p::is_consonant(phones[v - 2]) => v - 1,
            Some(v) => v,
            None => 0,
        }
    };

    let mut score = BankScore {
        total_sec,
        lead_max_sec: MODEL_LEAD_MAX_SEC,
        ..BankScore::default()
    };
    score.groups.push(ScoreGroup {
        start_sec: 0.0,
        midi: midi_of(0),
    });
    // Per note: where it starts (its anchor; a held note's first entry),
    // whether it sings a vowel.
    let mut note_start: Vec<Option<f64>> = vec![None; notes.len()];
    let mut note_voiced = vec![false; notes.len()];
    // Per note: the gap (rest or breath) that follows it, if any.
    let mut gap_after: Vec<Option<f64>> = vec![None; notes.len()];

    let mut k = 0;
    let n_entries = track.ph_seq.len();
    let mut last_note: Option<usize> = None;
    while k < n_entries {
        match track.roles.get(k).copied().unwrap_or(EntryRole::Extra) {
            EntryRole::Pad if k == 0 => {
                score.phones.push(phone(track, k, lang(k), 0));
                k += 1;
            }
            EntryRole::Pad | EntryRole::Rest | EntryRole::Breath => {
                if let Some(i) = last_note {
                    gap_after[i].get_or_insert(starts[k]);
                }
                let next = last_note.map_or(0, |i| i + 1).min(notes.len().saturating_sub(1));
                let midi = if k + 1 == n_entries {
                    last_note.map_or(midi_of(0), midi_of)
                } else {
                    midi_of(next)
                };
                score.groups.push(ScoreGroup {
                    start_sec: starts[k],
                    midi,
                });
                let g = score.groups.len() - 1;
                score.phones.push(phone(track, k, lang(k), g));
                k += 1;
            }
            EntryRole::Extra => {
                let g = score.groups.len() - 1;
                score.phones.push(phone(track, k, lang(k), g));
                k += 1;
            }
            EntryRole::Phone { note, .. } => {
                // The note's whole run of entries.
                let mut end = k;
                let mut arpa: Vec<&'static str> = Vec::new();
                while end < n_entries {
                    match track.roles[end] {
                        EntryRole::Phone { note: n, arpabet } if n == note => {
                            arpa.push(arpabet);
                            end += 1;
                        }
                        _ => break,
                    }
                }
                let slur = assigned.get(note).is_some_and(|a| a.is_slur);
                let has_vowel = arpa.iter().any(|p| !g2p::is_consonant(p));
                // A hold straight after its note (no rest or breath between)
                // continues that note's vowel.
                let holds_previous = slur && last_note.is_some_and(|i| gap_after[i].is_none());
                if holds_previous {
                    // The held vowel(s) fold into the phoneme before; the
                    // coda joins the current group.
                    note_start[note] = Some(starts[k]);
                    note_voiced[note] = last_note.is_some_and(|i| note_voiced[i]);
                    let vowels = arpa.iter().take_while(|p| !g2p::is_consonant(p)).count();
                    for j in k..k + vowels {
                        if let Some(prev) = score.phones.last_mut() {
                            prev.rule_sec += track.ph_dur[j];
                        }
                    }
                    let g = score.groups.len() - 1;
                    for j in k + vowels..end {
                        score.phones.push(phone(track, j, lang(j), g));
                    }
                } else {
                    note_voiced[note] = has_vowel;
                    let anchor = k + anchor_index(&arpa);
                    for j in k..end {
                        if j == anchor {
                            note_start[note] = Some(starts[j]);
                            score.groups.push(ScoreGroup {
                                start_sec: starts[j],
                                midi: midi_of(note),
                            });
                        }
                        let g = score.groups.len() - 1;
                        score.phones.push(phone(track, j, lang(j), g));
                    }
                }
                last_note = Some(note);
                k = end;
            }
        }
    }

    // The notes, for the pitch model: from the beat (its first phoneme
    // that is not an onset consonant — the anchor) to the next note, or
    // to the rest / breath after it.
    let anchors = note_start;
    for (i, n) in notes.iter().enumerate() {
        let Some(start) = anchors[i] else {
            continue;
        };
        let next = anchors.iter().skip(i + 1).flatten().next().copied();
        let end = gap_after[i].or(next).unwrap_or(total_sec).max(start);
        score.notes.push(ScoreNote {
            start_sec: start,
            end_sec: end,
            midi: n.note as f32,
            slur: assigned.get(i).is_some_and(|a| a.is_slur),
            voiced: note_voiced[i],
        });
    }
    score
}

fn phone(track: &PhonemeTrack, k: usize, language: i64, group: usize) -> ScorePhone {
    ScorePhone {
        symbol: track.ph_seq[k].clone(),
        language,
        group,
        rule_sec: track.ph_dur[k],
    }
}
