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
/// leading and intermediate gaps, so `"opmc"` ranks "Open MIDI Clip" above a
/// scattered coincidental hit.
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<FuzzyMatch> {
    let needle: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            ranges: Vec::new(),
        });
    }
    let hay: Vec<char> = haystack.chars().collect();

    let mut score: i32 = 0;
    let mut matched: Vec<usize> = Vec::with_capacity(needle.len());
    let mut hi = 0usize; // index into hay
    let mut prev_match: Option<usize> = None;

    for &nc in &needle {
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
            None => {
                // Leading gap before the first match; penalise distance.
                score -= idx as i32;
            }
        }
        if is_word_boundary(&hay, idx) {
            score += 3;
        }

        matched.push(idx);
        prev_match = Some(idx);
        hi = idx + 1;
    }

    Some(FuzzyMatch {
        score,
        ranges: merge_ranges(&matched),
    })
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
