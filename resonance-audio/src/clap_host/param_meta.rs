//! Reading a parameter's *meaning* out of a CLAP plugin (ba todo #1290,
//! finding X8).
//!
//! CLAP describes a parameter with a number and a range, and hands the
//! rendering of that number back through `value_to_text`. There is no
//! unit field and no enum list, so a host that wants "40 %", "Hz" or
//! "Low-pass / Band-pass / High-pass" has to ask the plugin to format
//! values and read the answers. That is what this module does, once per
//! parameter at query time:
//!
//! - [`unit_from_text`] takes the unit off a formatted value, because a
//!   reader that wants to label a fader needs `"dB"` on its own rather
//!   than `"-6.0 dB"`;
//! - [`choice_labels`] walks a stepped parameter's steps and keeps the
//!   labels only when they say something the number does not.
//!
//! Both are pure and cover third-party plugins as well as our own: they
//! are built on the standard extension, not on `resonance-plugin`.

/// The largest stepped range we enumerate labels for.
///
/// Choice parameters are short — filter types, LFO shapes, cabinet
/// lists. A stepped parameter with hundreds of steps is a count (MIDI
/// note, voice number), where per-step labels would be a wall of numbers
/// and one `value_to_text` call per step is real work on the engine
/// thread for every plugin load.
pub const MAX_CHOICE_STEPS: i64 = 64;

/// The longest a unit suffix may be, in bytes.
///
/// Units are symbols, not sentences: the longest ones in the fleet are
/// `"semitones"` and `"dB/oct"`. A long tail after a number is prose
/// that happens to follow one ("3 voices unison"), and reporting it as
/// a unit would put it on every fader label.
const MAX_UNIT_LEN: usize = 12;

/// The unit suffix of a formatted value, or `""` when it has none.
///
/// The plugin renders `"-6.0 dB"`; a control label wants `"dB"`. The
/// number is stripped from the front — sign, digits, decimal point,
/// exponent — and what remains, trimmed, is the unit.
///
/// Text that does not *start* with a number has no unit: `"Low-pass"` is
/// a choice name, and calling `"Low-pass"` the unit of a filter-type
/// parameter would be worse than admitting there is none. Neither does a
/// bare number: `"3"` is unitless.
///
/// Nor does a value that merely *begins* with a number without being
/// one. `"1/8D"` is a note division, and reporting `"/8D"` as its unit
/// would be an invented fact about the parameter — so a remainder
/// carrying a digit, or one too long to be a unit, is rejected.
pub fn unit_from_text(text: &str) -> &str {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start {
        // No leading number at all: a choice name, or "-inf".
        return "";
    }
    // An exponent belongs to the number, not to the unit.
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'-' || bytes[j] == b'+') {
            j += 1;
        }
        let exp_digits = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_digits {
            i = j;
        }
    }
    let rest = trimmed[i..].trim();
    // A unit is a short symbol without digits in it: "dB", "%", "Hz",
    // "dB/oct". Anything else means the leading number was part of a
    // larger token ("1/8D", "2 of 4") rather than a measured value.
    if rest.len() > MAX_UNIT_LEN || rest.bytes().any(|b| b.is_ascii_digit()) {
        return "";
    }
    rest
}

/// Whether a step's label carries information the step number does not.
///
/// A "voices 1..16" parameter formats step 3 as `"3"`; listing that as a
/// choice label would dress a count up as an enumeration. A label counts
/// as informative when it is not simply the number itself.
fn label_adds_meaning(label: &str, value: i64) -> bool {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return false;
    }
    match trimmed.parse::<f64>() {
        Ok(parsed) => parsed != value as f64,
        Err(_) => true,
    }
}

/// The labels of a stepped parameter, from `min` to `max` inclusive, or
/// `None` when this parameter is not an enumeration.
///
/// `display` is the plugin's formatter — in practice a `value_to_text`
/// call. The whole point is to ask the plugin what it calls each step,
/// so an agent can send `"Low-pass"` instead of `3` and a reader can
/// show a name instead of `2.0`.
///
/// `None` when: the range is not stepped or is inverted; it is longer
/// than [`MAX_CHOICE_STEPS`]; the plugin formats a step as nothing; or
/// *every* label is just the number (a count, not a choice).
pub fn choice_labels(
    min: f64,
    max: f64,
    mut display: impl FnMut(f64) -> Option<String>,
) -> Option<Vec<String>> {
    if !min.is_finite() || !max.is_finite() || max < min {
        return None;
    }
    let lo = min.round() as i64;
    let hi = max.round() as i64;
    let count = hi.checked_sub(lo)?.checked_add(1)?;
    if count <= 1 || count > MAX_CHOICE_STEPS {
        return None;
    }

    let mut labels = Vec::with_capacity(count as usize);
    let mut informative = false;
    for value in lo..=hi {
        let label = display(value as f64)?;
        if label.trim().is_empty() {
            return None;
        }
        informative |= label_adds_meaning(&label, value);
        labels.push(label.trim().to_string());
    }
    if !informative {
        return None;
    }
    Some(labels)
}

/// Whether a plugin's answer to `text_to_value(input)` is a real match:
/// the text the plugin displays for that value (`display`) says what the
/// caller typed (nam-model-library.md §9.2).
///
/// Lenient plugins parse anything: JUCE's `getValueForText` turns `"loud"`
/// into 0. So the answer only counts when it round-trips:
///
/// - the display equals the input, ignoring case, accents and outer space;
/// - or the display starts with the input (3+ characters: a name prefix,
///   `"friedman"` for `"Friedman BE-100 · standard"`);
/// - or both begin with a number and they agree to the input's precision
///   (`"-6 dB"` against `"-6.02 dB"`).
pub fn label_round_trips(input: &str, display: &str) -> bool {
    let fold = |s: &str| resonance_common::library_marks::vocab::fold(s.trim());
    let (i, d) = (fold(input), fold(display));
    if i.is_empty() {
        return false;
    }
    if i == d || (i.chars().count() >= 3 && d.starts_with(&i)) {
        return true;
    }
    match (leading_number(&i), leading_number(&d)) {
        (Some((a, decimals)), Some((b, _))) => {
            let tol = 0.5 * 10f64.powi(-(decimals as i32));
            (a - b).abs() <= tol + f64::EPSILON * b.abs().max(1.0)
        }
        _ => false,
    }
}

/// The number a text starts with, and how many decimals it was written
/// with (`"-6.02 dB"` → `(-6.02, 2)`).
fn leading_number(s: &str) -> Option<(f64, usize)> {
    let s = s.trim_start();
    let end = s
        .char_indices()
        .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+')))
        .map(|(i, c)| i + c.len_utf8())
        .last()?;
    let num = &s[..end];
    let v: f64 = num.parse().ok()?;
    let decimals = num.split_once('.').map(|(_, f)| f.len()).unwrap_or(0);
    Some((v, decimals))
}
