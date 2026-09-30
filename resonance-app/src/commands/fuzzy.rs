//! The palette's fuzzy matcher.

// ===========================================================================
// Fuzzy matcher
// ===========================================================================

/// A successful fuzzy match: a relevance `score` (higher is better) and the
/// matched character ranges in the haystack as `[start, end)` half-open spans
/// over `char` indices, merged so adjacent matches form one highlight run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub ranges: Vec<(usize, usize)>,
}

/// Case-insensitive subsequence fuzzy match of `needle` against `haystack`.
///
/// Returns `None` unless every character of `needle` appears in `haystack` in
/// order. An empty needle matches everything with score `0` and no ranges.
/// Scoring rewards consecutive runs and matches at word boundaries (start of
/// string, or following a separator / case transition) and lightly penalises
/// gaps, so `"opmc"` ranks "Open MIDI Clip" above a scattered coincidental
/// hit. A leading gap is penalised only when the first match is mid-word: a
/// match that starts a word scores the same wherever the word sits, so
/// `"loop st"` finds "Playhead to Loop Start" as well as it finds "Set Loop
/// Start at Playhead".
///
/// Every occurrence of the needle's first character is tried as the start of
/// the alignment (then greedy), and the best-scoring one wins; ties go to the
/// earliest start.
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<FuzzyMatch> {
    let needle: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            ranges: Vec::new(),
        });
    }
    let hay: Vec<char> = haystack.chars().collect();
    let first = needle[0].to_ascii_lowercase();
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in 0..hay.len() {
        if hay[start].to_ascii_lowercase() != first {
            continue;
        }
        if let Some((score, matched)) = align_from(&needle, &hay, start) {
            if best.as_ref().is_none_or(|(b, _)| score > *b) {
                best = Some((score, matched));
            }
        }
    }
    let (score, matched) = best?;
    Some(FuzzyMatch {
        score,
        ranges: merge_ranges(&matched),
    })
}

/// Greedy alignment of `needle` with its first character at `hay[start]`.
fn align_from(needle: &[char], hay: &[char], start: usize) -> Option<(i32, Vec<usize>)> {
    let mut score: i32 = 0;
    let mut matched: Vec<usize> = Vec::with_capacity(needle.len());
    let mut hi = start; // index into hay
    let mut prev_match: Option<usize> = None;

    for &nc in needle {
        let target = nc.to_ascii_lowercase();
        let mut found = None;
        while hi < hay.len() {
            if hay[hi].to_ascii_lowercase() == target {
                found = Some(hi);
                break;
            }
            hi += 1;
        }
        let idx = found?;
        let boundary = is_word_boundary(hay, idx);

        // Base reward for the match.
        score += 1;
        match prev_match {
            Some(p) if p + 1 == idx => {
                // Consecutive run — strongly preferred.
                score += 5;
            }
            Some(_) => {
                // A gap between matched chars; small penalty.
                score -= 1;
            }
            None if boundary => {}
            None => {
                // A mid-word first match; penalise its distance.
                score -= idx as i32;
            }
        }
        if boundary {
            score += 3;
        }

        matched.push(idx);
        prev_match = Some(idx);
        hi = idx + 1;
    }
    Some((score, matched))
}

/// Whether `hay[idx]` begins a "word" — index 0, or preceded by a separator,
/// or a lower→upper case transition (camelCase boundary).
fn is_word_boundary(hay: &[char], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = hay[idx - 1];
    if prev == ' ' || prev == '-' || prev == '_' || prev == '/' || prev == '›' || prev == '.' {
        return true;
    }
    prev.is_lowercase() && hay[idx].is_uppercase()
}

/// Collapse a sorted list of matched indices into half-open `[start, end)`
/// ranges, merging adjacent indices.
fn merge_ranges(indices: &[usize]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &i in indices {
        match ranges.last_mut() {
            Some(last) if last.1 == i => last.1 = i + 1,
            _ => ranges.push((i, i + 1)),
        }
    }
    ranges
}
