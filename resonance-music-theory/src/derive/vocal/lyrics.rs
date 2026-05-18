//! Lyric data + generator. The corpus is a small mood-tagged set of
//! pre-syllabified, pre-rhyme-tagged template lines; `generate_lyrics`
//! samples them according to the requested mood / scheme / syllable
//! range and writes the result back into `VocalParams::draft`.

use serde::{Deserialize, Serialize};

use crate::rng::XorShift;

use super::params::{VocalMood, VocalParams, VocalRhymeScheme};

/// One generated lyric line in the draft preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    /// 1-based line number.
    pub n: u8,
    /// Rhyme tag — 'A', 'B', 'C', etc. matching the chosen scheme.
    pub rhyme: char,
    /// Syllable count.
    pub syllables: u8,
    /// The line itself, with `·` used as a syllable separator (matches
    /// the prototype's typographic convention).
    pub text: String,
    /// Locked lines are not replaced by the next re-roll.
    pub locked: bool,
}

// ===========================================================================
// Lyric corpus + generator
// ===========================================================================

/// One template line in the bundled lyric corpus.
struct CorpusLine {
    mood: VocalMood,
    /// Rhyme bucket — lines that share this tag end on the same vowel
    /// sound. Same tag = same rhyme.
    rhyme: u8,
    /// Total syllable count of `text` (the syllable separator `·` is
    /// significant — `text` is already pre-broken).
    syllables: u8,
    /// Line, with `·` between syllables.
    text: &'static str,
}

/// Tiny mood-tagged corpus. Each line is pre-syllabified and pre-tagged
/// with a rhyme bucket so the generator can build any A/B/C scheme by
/// matching tags rather than running a phonetic rhyme matcher at runtime.
///
/// Rhyme bucket numbering is local and only meaningful within this file
/// — buckets in different moods aren't interchangeable.
const CORPUS: &[CorpusLine] = &[
    // ---- Yearning (rhyme 1: -ay/-ey, 2: -one/-ome, 3: -ember/-er) ----
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 9, text: "I hold the days I can·not say" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 10, text: "the morn·ing leaves us no·thing to weigh" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 8, text: "ev·ery·thing I meant to say" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 9, text: "an emp·ty house an emp·ty day" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 9, text: "ev·ry stone we threw on the way" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 10, text: "we walk the hall·ways all a·lone" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 9, text: "a qui·et house that feels like home" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 9, text: "I made a bed of glass and stone" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 3, syllables: 11, text: "Glass hou·ses don't break, they just re·mem·ber" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 3, syllables: 11, text: "the way the light bends as you en·ter the room" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 3, syllables: 10, text: "a name I have not learned to fore·get" },

    // ---- Defiant (1: -own, 2: -ire/-ind, 3: -ash/-ack) ----
    CorpusLine { mood: VocalMood::Defiant, rhyme: 1, syllables: 8, text: "I won't lay this an·ger down" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 1, syllables: 9, text: "I am not the one who broke the crown" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 1, syllables: 8, text: "burn the maps that brought us down" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 2, syllables: 9, text: "I walk a·gainst the lev·el wind" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 2, syllables: 9, text: "I will not bend, I will not bind" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 2, syllables: 10, text: "the room is full of bor·rowed fire" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 3, syllables: 8, text: "I let the qui·et turn to ash" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 3, syllables: 9, text: "I'll meet your an·ger with my own back" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 3, syllables: 8, text: "noth·ing breaks me, noth·ing cracks" },

    // ---- Hopeful (1: -ight, 2: -orn/-orning, 3: -ound) ----
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 1, syllables: 9, text: "and ev·ery cor·ner finds the light" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 1, syllables: 8, text: "a thou·sand suns be·hind the night" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 1, syllables: 9, text: "I learn to read the dark·er signs" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 2, syllables: 9, text: "I'll meet you in the qui·et morn·ing" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 2, syllables: 9, text: "the world is gen·tle when it's born" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 2, syllables: 10, text: "a low and stead·y wind across the lawn" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 3, syllables: 8, text: "ev·ry·thing I lost is found" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 3, syllables: 9, text: "I press my ear against the ground" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 3, syllables: 9, text: "the soft and pa·tient turn·ing sound" },

    // ---- Reflective (1: -ow/-ow, 2: -ile, 3: -ear/-eel) ----
    CorpusLine { mood: VocalMood::Reflective, rhyme: 1, syllables: 8, text: "I watch the riv·er find its flow" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 1, syllables: 9, text: "we used to know each oth·er though" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 1, syllables: 10, text: "I car·ry on the way the rains do go" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 2, syllables: 9, text: "I count the lights, I sit a while" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 2, syllables: 9, text: "an old, fa·mil·iar morn·ing smile" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 2, syllables: 8, text: "the streets that ran on for a mile" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 3, syllables: 9, text: "I think of you and feel quite near" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 3, syllables: 8, text: "I bend my head and hear, and hear" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 3, syllables: 9, text: "ev·ery still·ness makes it clear" },

    // ---- Joyful (1: -ee, 2: -ay, 3: -ound) ----
    CorpusLine { mood: VocalMood::Joyful, rhyme: 1, syllables: 8, text: "you laugh, the whole room laughs with me" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 1, syllables: 9, text: "we run and run, the world goes free" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 1, syllables: 9, text: "I read the sky like po·et·ry" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 2, syllables: 8, text: "we won't be quiet, won't go a·way" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 2, syllables: 9, text: "the kind of morn·ing made for play" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 2, syllables: 8, text: "the kind of word·s I want to say" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 3, syllables: 8, text: "your laugh·ter spins us all around" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 3, syllables: 9, text: "we shake the win·dows with the sound" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 3, syllables: 9, text: "the kind of joy that won't stay bound" },

    // ---- Melancholy (1: -ain, 2: -ear/-ere, 3: -one) ----
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 1, syllables: 9, text: "I think of you and feel the rain" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 1, syllables: 8, text: "the qui·et stays, the streets re·main" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 1, syllables: 9, text: "the win·ter holds its old re·frain" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 2, syllables: 8, text: "I should have brought you clos·er here" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 2, syllables: 9, text: "you said the words but I'd not hear" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 2, syllables: 9, text: "the dis·tance be·tween then and here" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 3, syllables: 8, text: "I count the things that I have done" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 3, syllables: 9, text: "I leave the house and I am gone" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 3, syllables: 9, text: "the long, the qui·et, the un·done" },
];

/// Pattern of rhyme keys for a given scheme. Letters share keys; "free"
/// returns an empty pattern (no rhyme constraint).
fn rhyme_pattern(scheme: VocalRhymeScheme) -> &'static [u8] {
    match scheme {
        VocalRhymeScheme::Aabb => &[0, 0, 1, 1],
        VocalRhymeScheme::Abab => &[0, 1, 0, 1],
        VocalRhymeScheme::Abcb => &[0, 1, 2, 1],
        VocalRhymeScheme::Abba => &[0, 1, 1, 0],
        VocalRhymeScheme::Free => &[],
    }
}

/// Generate (or regenerate) a `params.draft` of lyric lines satisfying
/// the requested mood, rhyme scheme, line count, and syllable range.
///
/// Locked lines keep their current text/rhyme/syllables; the rhyme
/// pattern is *anchored* to any locked line so re-rolls preserve the
/// established A/B groupings.
pub fn generate_lyrics(params: &VocalParams, seed: u64) -> Vec<LyricLine> {
    let mut rng = XorShift::new(seed.max(1));
    let lines = params.lines.max(1) as usize;
    let pattern = rhyme_pattern(params.rhyme);

    // Letter assigned to each unique rhyme key (0 -> 'A', 1 -> 'B', ...).
    let letter_for = |k: u8| -> char { (b'A' + (k % 26)) as char };

    // Filter corpus by mood. Fall back to all moods if the bucket is empty.
    let mood_pool: Vec<&CorpusLine> =
        CORPUS.iter().filter(|l| l.mood == params.mood).collect();
    let fallback_pool: Vec<&CorpusLine> = CORPUS.iter().collect();
    let active_pool = if mood_pool.is_empty() {
        &fallback_pool
    } else {
        &mood_pool
    };

    // Distinct rhyme keys present in the active pool.
    let mut available_keys: Vec<u8> = active_pool.iter().map(|l| l.rhyme).collect();
    available_keys.sort_unstable();
    available_keys.dedup();
    if available_keys.is_empty() {
        return params.draft.clone();
    }

    // Anchor any rhyme-pattern slot whose corresponding line is locked
    // to the locked line's rhyme bucket. The bucket is recovered by
    // matching the locked line's rhyme letter back through the pattern.
    // Pattern entries with no corresponding locked line get a freshly
    // picked bucket (without repeating already-used buckets when
    // possible).
    let mut pattern_keys: std::collections::BTreeMap<u8, u8> = Default::default();
    if !pattern.is_empty() {
        for (i, slot) in pattern.iter().enumerate().take(lines) {
            if let Some(line) = params.draft.get(i) {
                if line.locked {
                    // Recover the bucket from the locked line's text by
                    // re-matching it in the corpus.
                    let bucket = CORPUS
                        .iter()
                        .find(|c| c.text == line.text)
                        .map(|c| c.rhyme)
                        .unwrap_or(*slot);
                    pattern_keys.insert(*slot, bucket);
                }
            }
        }
        for slot in pattern.iter().take(lines) {
            if pattern_keys.contains_key(slot) {
                continue;
            }
            let used: std::collections::BTreeSet<u8> =
                pattern_keys.values().copied().collect();
            // Pick an unused key when possible.
            let unused: Vec<u8> = available_keys
                .iter()
                .copied()
                .filter(|k| !used.contains(k))
                .collect();
            let key = if !unused.is_empty() {
                unused[rng.next_range(unused.len())]
            } else {
                available_keys[rng.next_range(available_keys.len())]
            };
            pattern_keys.insert(*slot, key);
        }
    }

    // For each output line: pick from the pool that matches its bucket
    // (or any line if scheme is Free), syllable count in range.
    let mut out = Vec::with_capacity(lines);
    let mut used_texts: std::collections::BTreeSet<String> = Default::default();
    for i in 0..lines {
        let n = (i + 1) as u8;

        // Preserve a locked line if present.
        if let Some(existing) = params.draft.get(i) {
            if existing.locked {
                used_texts.insert(existing.text.clone());
                out.push(existing.clone());
                continue;
            }
        }

        let bucket = if pattern.is_empty() {
            // Free scheme — any rhyme bucket goes.
            available_keys[rng.next_range(available_keys.len())]
        } else {
            *pattern_keys
                .get(&pattern[i % pattern.len()])
                .unwrap_or(&available_keys[0])
        };

        // Candidate pool: bucket match + syllable range + not already used.
        let mut candidates: Vec<&CorpusLine> = active_pool
            .iter()
            .filter(|l| {
                l.rhyme == bucket
                    && l.syllables >= params.syllables_min
                    && l.syllables <= params.syllables_max
                    && !used_texts.contains(l.text)
            })
            .copied()
            .collect();
        // Fall back to bucket match without syllable constraint, then
        // any bucket. Keeps the generator from returning the placeholder
        // line on niche corpora.
        if candidates.is_empty() {
            candidates = active_pool
                .iter()
                .filter(|l| l.rhyme == bucket && !used_texts.contains(l.text))
                .copied()
                .collect();
        }
        if candidates.is_empty() {
            candidates = active_pool
                .iter()
                .filter(|l| !used_texts.contains(l.text))
                .copied()
                .collect();
        }
        if candidates.is_empty() {
            // Should be unreachable given we always have one corpus
            // line; emit a sentinel so the UI still has something.
            out.push(LyricLine {
                n,
                rhyme: letter_for(bucket),
                syllables: 0,
                text: String::from("\u{2014}"),
                locked: false,
            });
            continue;
        }
        let pick = candidates[rng.next_range(candidates.len())];
        used_texts.insert(pick.text.to_string());

        let letter = if pattern.is_empty() {
            'F' // "Free"
        } else {
            letter_for(pattern[i % pattern.len()])
        };

        out.push(LyricLine {
            n,
            rhyme: letter,
            syllables: pick.syllables,
            text: pick.text.to_string(),
            locked: false,
        });
    }
    out
}
