//! The seeded metadata vocabulary (plugin-preset-library.md §4.4).
//!
//! **Integration seam.** The spec puts this table in
//! `resonance_common::library_marks::vocab`, shared with the NAM model
//! library. That module is being built on another branch; until it lands
//! this is the preset library's local copy, and round 2 replaces the
//! body of this file with a re-export. Nothing outside `presets` names
//! these constants directly except through [`normalize_facet`] and
//! [`canonical_category`].
//!
//! Values outside the seeded lists are **accepted and kept**, lowercased
//! and slugged, so an agent can still write `"shoegaze"`.

/// `category` values for instrument plugins.
pub const INSTRUMENT_CATEGORIES: &[&str] = &[
    "Bass", "Lead", "Pad", "Pluck", "Keys", "Arp", "Brass", "Strings", "Drone", "FX", "Drums",
    "Init",
];

/// `category` values for effect plugins.
pub const EFFECT_CATEGORIES: &[&str] = &["Utility", "Track", "Bus", "Master", "Creative"];

/// "What is it for": the source an effect suits, or what an instrument is.
pub const INSTRUMENT: &[&str] = &[
    "vocal", "lead-vocal", "backing-vocal", "guitar", "electric-guitar", "acoustic-guitar",
    "bass", "synth-bass", "drums", "kick", "snare", "hats", "room", "keys", "piano", "synth",
    "strings", "violin", "mix-bus", "drum-bus", "master", "full-mix",
];

/// Seeded genres: a superset of the mastering assistant's genre targets
/// and the agent plugin's genre skills.
pub const GENRES: &[&str] = &[
    "ambient", "americana", "cinematic", "drum-and-bass", "electronic", "folk", "hip-hop",
    "house", "indie", "industrial", "jazz", "metal", "pop", "post-metal", "rock",
    "singer-songwriter", "techno",
];

/// Timbre words.
pub const CHARACTER: &[&str] = &[
    "warm", "bright", "dark", "clean", "gritty", "saturated", "punchy", "soft", "wide", "narrow",
    "lush", "dry", "subtle", "aggressive", "vintage", "modern", "evolving", "static", "metallic",
    "airy",
];

/// Longest free tag, in bytes.
pub const MAX_TAG_LEN: usize = 32;

/// A facet value (`instrument`, `genres`, `character`, `tags`) in its
/// stored spelling: lowercase ASCII `[a-z0-9-]`, accents folded, runs of
/// anything else collapsed to one `-`, at most [`MAX_TAG_LEN`] bytes.
/// `None` when nothing usable is left.
pub fn normalize_facet(value: &str) -> Option<String> {
    let folded = super::query::fold(value);
    let mut out = String::with_capacity(folded.len());
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() > MAX_TAG_LEN {
        out.truncate(MAX_TAG_LEN);
        while out.ends_with('-') {
            out.pop();
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The vocabulary spelling of a category (`"bass"` → `"Bass"`), or the
/// trimmed input when it is not a seeded category. `None` for blank.
pub fn canonical_category(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let seeded = INSTRUMENT_CATEGORIES
        .iter()
        .chain(EFFECT_CATEGORIES)
        .find(|c| c.eq_ignore_ascii_case(trimmed));
    Some(seeded.map(|c| c.to_string()).unwrap_or_else(|| trimmed.to_string()))
}

/// The seeded category a legacy `"<X> — <Y>"` / `"<X> - <Y>"` /
/// `"<X>___<Y>"` name starts with, if any (§13).
pub fn category_from_name(name: &str) -> Option<&'static str> {
    let head = ["—", " - ", "___", "–"]
        .iter()
        .filter_map(|sep| name.split_once(sep).map(|(head, _)| head.trim()))
        .min_by_key(|head| head.len())?;
    INSTRUMENT_CATEGORIES
        .iter()
        .chain(EFFECT_CATEGORIES)
        .find(|c| c.eq_ignore_ascii_case(head))
        .copied()
}
