//! CLAP plugin feature strings — the words that decide where a plugin
//! lands in a host's browser (ba todo #1298, finding X6).
//!
//! [`crate::plugin::ResonancePlugin::FEATURES`] is a `&[&CStr]`, which is
//! what CLAP itself wants, so whatever a plugin declares reaches the host
//! verbatim. It used to be `&[&str]` that the bridge translated through a
//! hand-written whitelist, `filter_map`ping away everything it did not
//! recognise: `compressor`, `equalizer`, `delay`, `mastering`,
//! `analyzer` and `gate` are all standard CLAP constants that never
//! reached a host, and the IR plugin's `cabinet_simulator` (underscore)
//! was dropped for a typo. Seven of eleven plugins showed up in the wrong
//! browser category, silently.
//!
//! Declare features from the constants below — a misspelled *name* is a
//! compile error, where a misspelled *string* was a silent drop.
//!
//! # Categories
//!
//! Every plugin must declare at least one of the five main categories
//! ([`AUDIO_EFFECT`], [`INSTRUMENT`], [`NOTE_EFFECT`], [`NOTE_DETECTOR`],
//! [`ANALYZER`]); the bridge enforces that at compile time via
//! [`has_category`]. Add a sub-category too ([`REVERB`], [`COMPRESSOR`],
//! …) or the plugin lands in a host's "uncategorized" bucket, which is
//! exactly the symptom X6 describes. The `mono`/`stereo` entries are
//! audio capabilities, not categories.

use std::ffi::CStr;

// The standard set clack exposes: the five main categories, the
// sub-categories, and the audio capabilities.
pub use clack_plugin::plugin::features::*;

/// `"gate"` — standard CLAP, missing from clack's re-export.
pub const GATE: &CStr = c"gate";
/// `"expander"` — standard CLAP, missing from clack's re-export.
pub const EXPANDER: &CStr = c"expander";
/// `"note-detector"` — standard CLAP, missing from clack's re-export.
pub const NOTE_DETECTOR: &CStr = c"note-detector";

/// `"resonance:cabinet-simulator"` — a speaker-cabinet impulse loader.
///
/// CLAP has no cabinet-simulator feature, and its spec says a
/// non-standard one must be namespaced `"$namespace:$feature"`. The IR
/// plugin previously declared a bare, misspelled `cabinet_simulator`,
/// which the old whitelist dropped on the floor.
pub const CABINET_SIMULATOR: &CStr = c"resonance:cabinet-simulator";

/// The five CLAP main categories. A plugin that declares none of them
/// tells the host nothing about what it is.
pub const MAIN_CATEGORIES: &[&CStr] = &[
    AUDIO_EFFECT,
    INSTRUMENT,
    NOTE_EFFECT,
    NOTE_DETECTOR,
    ANALYZER,
];

/// Whether `features` contains at least one CLAP main category.
///
/// `const` on purpose: the bridge evaluates it for every plugin it
/// exports, so a plugin with no category fails to build rather than
/// shipping and turning up nowhere useful in a browser.
pub const fn has_category(features: &[&CStr]) -> bool {
    let mut i = 0;
    while i < features.len() {
        let mut j = 0;
        while j < MAIN_CATEGORIES.len() {
            if cstr_eq(features[i], MAIN_CATEGORIES[j]) {
                return true;
            }
            j += 1;
        }
        i += 1;
    }
    false
}

/// Byte-wise `CStr` equality, usable in a `const` context (the standard
/// `PartialEq` is not).
pub const fn cstr_eq(a: &CStr, b: &CStr) -> bool {
    let (a, b) = (a.to_bytes(), b.to_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
