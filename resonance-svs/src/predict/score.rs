//! What the bank's own duration, pitch and variance models are fed, and
//! how their output is laid back onto the timeline. Pure functions of a
//! [`BankScore`] — no ONNX here — so the alignment and input shapes are
//! testable without a bank.
//!
//! The layout follows OpenUtau's DiffSinger renderer
//! (`DiffSingerBasePhonemizer.ProcessPart`, `DsPitch.Process`,
//! `DsVariance.Process`):
//!
//! * The duration model works on **groups** ("words" in the model's
//!   `word_div` / `word_dur` inputs). A group starts at an *anchor* — the
//!   moment a note's vowel lands on its beat, or a rest or breath begins —
//!   and holds everything sung until the next anchor: the note's vowel, its
//!   coda, and the **next** syllable's onset consonants, which are sung
//!   before that syllable's beat. Group 0 is the leading pad and the first
//!   syllable's onset.
//! * The model predicts each phoneme's frames; each group is then
//!   stretched to fill its anchor-to-anchor span exactly, so every vowel
//!   still lands on its beat. Group 0's consonants keep their predicted
//!   length (capped by [`BankScore::lead_max_sec`]) and end on the first
//!   anchor; the pad's silence takes what is left.
//! * The pitch model sees the notes (held notes are notes of their own, so
//!   it slides between them) padded with rest notes over the leading and
//!   trailing pads and every gap.

/// One phoneme the bank models see.
#[derive(Debug, Clone, PartialEq)]
pub struct ScorePhone {
    /// The symbol as the bank's dictionaries spell it (`L`, `en/ah`, `SP`).
    pub symbol: String,
    /// Its language id (0 where the bank has none).
    pub language: i64,
    /// The group (index into [`BankScore::groups`]) it belongs to.
    pub group: usize,
    /// Its duration under the rule-based layout, in seconds — what the
    /// render uses when the bank's duration model is not run.
    pub rule_sec: f64,
}

/// A run of phonemes starting at one anchor (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreGroup {
    /// Seconds from the segment start. Group 0 starts at 0.
    pub start_sec: f64,
    /// The MIDI note its phonemes are sung on (the duration model's
    /// `ph_midi`).
    pub midi: i64,
}

/// One written note, for the pitch model.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreNote {
    /// Seconds from the segment start: its beat.
    pub start_sec: f64,
    /// Where it stops sounding: the next note's beat when sung legato,
    /// else its written end.
    pub end_sec: f64,
    /// Its pitch, fractional MIDI.
    pub midi: f32,
    /// A held (`_`) note: no new syllable, the previous vowel carries on.
    pub slur: bool,
    /// It sings a vowel (a note of consonants only, or a bracketed `[AP]`,
    /// is a rest to the pitch model).
    pub voiced: bool,
}

/// A render unit as the bank models see it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BankScore {
    pub phones: Vec<ScorePhone>,
    pub groups: Vec<ScoreGroup>,
    pub notes: Vec<ScoreNote>,
    /// The unit's whole length, pads included — what the phonemes'
    /// durations always add up to.
    pub total_sec: f64,
    /// The most the first syllable's onset may take of the leading pad.
    pub lead_max_sec: f64,
}

impl BankScore {
    /// The rule-based durations, in phoneme order.
    pub fn rule_durations(&self) -> Vec<f64> {
        self.phones.iter().map(|p| p.rule_sec).collect()
    }

    /// Phonemes per group: the duration model's `word_div`.
    pub fn word_div(&self) -> Vec<i64> {
        let mut div = vec![0i64; self.groups.len()];
        for p in &self.phones {
            if let Some(d) = div.get_mut(p.group) {
                *d += 1;
            }
        }
        div
    }

    /// Frames each group spans: the duration model's `word_dur`, from the
    /// anchors rounded to frames cumulatively (so they add up to the
    /// unit's frame count).
    pub fn word_dur(&self, frame_sec: f64) -> Vec<i64> {
        let mut bounds: Vec<f64> = self.groups.iter().map(|g| g.start_sec).collect();
        bounds.push(self.total_sec);
        bounds
            .windows(2)
            .map(|w| to_frame(w[1], frame_sec) - to_frame(w[0], frame_sec))
            .collect()
    }

    /// Each phoneme's group pitch: the duration model's `ph_midi`.
    pub fn ph_midi(&self) -> Vec<i64> {
        self.phones
            .iter()
            .map(|p| self.groups.get(p.group).map_or(60, |g| g.midi))
            .collect()
    }

    /// Check the score is well formed: groups in time order starting at 0,
    /// every group non-empty and its phonemes contiguous.
    pub fn validate(&self) -> Result<(), String> {
        if self.phones.is_empty() || self.groups.is_empty() {
            return Err("empty score".into());
        }
        if self.groups[0].start_sec != 0.0 {
            return Err("group 0 must start at 0".into());
        }
        for w in self.groups.windows(2) {
            if w[1].start_sec < w[0].start_sec {
                return Err("groups out of time order".into());
            }
        }
        if self.groups.last().is_some_and(|g| g.start_sec > self.total_sec) {
            return Err("a group starts after the unit ends".into());
        }
        let mut expected = 0usize;
        for p in &self.phones {
            if p.group == expected + 1 {
                expected += 1;
            } else if p.group != expected {
                return Err(format!("phoneme {:?} skips to group {}", p.symbol, p.group));
            }
        }
        if expected + 1 != self.groups.len() {
            return Err("a group has no phonemes".into());
        }
        Ok(())
    }
}

/// Seconds → frame index, rounded (the pipeline's cumulative rounding).
pub fn to_frame(sec: f64, frame_sec: f64) -> i64 {
    (sec / frame_sec).round() as i64
}

/// Shortest a phoneme may come out of the alignment.
const MIN_PHONE_SEC: f64 = 0.01;

/// Least silence the leading pad keeps before the first onset.
const LEAD_SILENCE_MIN_SEC: f64 = 0.05;

/// Lay the duration model's prediction (`predicted`, frames per phoneme)
/// onto the score's anchors; seconds per phoneme, adding up to
/// [`BankScore::total_sec`].
///
/// Group 0 is the leading pad (its first phoneme, silence) and the first
/// onset: the onset phonemes keep their predicted length, shrunk
/// together if they would take more than [`BankScore::lead_max_sec`],
/// and end on the first anchor; the silence takes the rest. Every later
/// group is stretched (or squeezed) proportionally to fill its span.
pub fn align_durations(score: &BankScore, predicted: &[f32], frame_sec: f64) -> Vec<f64> {
    let n = score.phones.len();
    let mut out = vec![0.0; n];
    let ends: Vec<f64> = score
        .groups
        .iter()
        .skip(1)
        .map(|g| g.start_sec)
        .chain(std::iter::once(score.total_sec))
        .collect();
    let mut start = 0usize;
    for (g, group) in score.groups.iter().enumerate() {
        let len = score.phones[start..]
            .iter()
            .take_while(|p| p.group == g)
            .count();
        let idx = start..start + len;
        start += len;
        let span = (ends[g] - group.start_sec).max(0.0);
        let pred: Vec<f64> = idx
            .clone()
            .map(|i| (predicted.get(i).copied().unwrap_or(0.0) as f64 * frame_sec).max(MIN_PHONE_SEC))
            .collect();
        if g == 0 && len > 1 {
            let room = (span - LEAD_SILENCE_MIN_SEC).clamp(0.0, score.lead_max_sec);
            let onset: f64 = pred[1..].iter().sum();
            let scale = if onset > room { room / onset } else { 1.0 };
            let mut used = 0.0;
            for (k, i) in idx.clone().enumerate().skip(1) {
                out[i] = pred[k] * scale;
                used += out[i];
            }
            out[idx.start] = span - used;
            continue;
        }
        let total: f64 = pred.iter().sum();
        for (k, i) in idx.enumerate() {
            out[i] = if total > 0.0 { span * pred[k] / total } else { span / len as f64 };
        }
    }
    out
}

/// The pitch model's note layout: `(midi, rest, frames)` per note, the
/// written notes padded with rest notes over the leading pad, every gap
/// and the trailing pad, the frames adding up to `total_frames`.
///
/// Rest notes take the pitch of their nearest sung neighbour, split at the
/// middle of a run of rests (OpenUtau's rule); a held note is voiced when
/// the note it holds is.
pub fn pitch_notes(score: &BankScore, frame_sec: f64, total_frames: i64) -> Vec<(f32, bool, i64)> {
    // (start, end, midi, rest)
    let mut notes: Vec<(f64, f64, f32, bool)> = Vec::new();
    let mut cursor = 0.0;
    for n in &score.notes {
        if n.start_sec > cursor {
            notes.push((cursor, n.start_sec, n.midi, true));
        }
        let rest = if n.slur {
            notes.last().map_or(!n.voiced, |prev| prev.3)
        } else {
            !n.voiced
        };
        let end = n.end_sec.max(n.start_sec);
        notes.push((n.start_sec, end, n.midi, rest));
        cursor = end;
    }
    let last_midi = score.notes.last().map_or(60.0, |n| n.midi);
    if score.total_sec > cursor || notes.is_empty() {
        notes.push((cursor, score.total_sec.max(cursor), last_midi, true));
    }

    // Rest pitches from the neighbours.
    let mut midi: Vec<f32> = notes.iter().map(|n| n.2).collect();
    let rest: Vec<bool> = notes.iter().map(|n| n.3).collect();
    if rest.iter().all(|r| *r) {
        midi.iter_mut().for_each(|m| *m = 60.0);
    } else {
        let mut i = 0;
        while i < rest.len() {
            if !rest[i] {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < rest.len() && rest[j] {
                j += 1;
            }
            if i == 0 {
                let v = midi[j];
                midi[..j].iter_mut().for_each(|m| *m = v);
            } else if j == rest.len() {
                let v = midi[i - 1];
                midi[i..].iter_mut().for_each(|m| *m = v);
            } else {
                let mid = (i + j + 1) / 2;
                let (before, after) = (midi[i - 1], midi[j]);
                midi[i..mid].iter_mut().for_each(|m| *m = before);
                midi[mid..j].iter_mut().for_each(|m| *m = after);
            }
            i = j;
        }
    }

    // Frames by cumulative rounding of the boundaries, fitted to the
    // phonemes' frame total.
    let mut frames: Vec<i64> = notes
        .iter()
        .map(|n| to_frame(n.1, frame_sec) - to_frame(n.0, frame_sec))
        .collect();
    fit_frames(&mut frames, total_frames);
    notes
        .iter()
        .zip(midi)
        .zip(frames)
        .map(|((n, m), f)| (m, n.3, f))
        .collect()
}

/// Make `frames` add up to `total`: the difference goes on the last entry,
/// borrowing backwards when that would go negative (OpenUtau's
/// `FitDurationSum`).
pub fn fit_frames(frames: &mut [i64], total: i64) {
    let Some(last) = frames.len().checked_sub(1) else {
        return;
    };
    let delta = total - frames.iter().sum::<i64>();
    frames[last] += delta;
    let mut deficit = 0;
    if frames[last] < 0 {
        deficit = -frames[last];
        frames[last] = 0;
    }
    for f in frames[..last].iter_mut().rev() {
        if deficit == 0 {
            break;
        }
        let take = (*f).min(deficit);
        *f -= take;
        deficit -= take;
    }
}

/// Fractional MIDI → Hz.
pub fn midi_to_hz(midi: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0) / 12.0)
}

/// Hz → fractional MIDI.
pub fn hz_to_midi(hz: f64) -> f64 {
    69.0 + 12.0 * (hz.max(1e-6) / 440.0).log2()
}
