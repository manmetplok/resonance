//! CLAP feature declarations (ba todo #1298, finding X6).
//!
//! `ResonancePlugin::FEATURES` is a `&[&CStr]` that the bridge hands to
//! the host verbatim. It used to be `&[&str]` translated through a
//! hand-written whitelist that `filter_map`ped away anything unlisted:
//! `compressor`, `equalizer`, `delay`, `mastering`, `analyzer` and
//! `gate` — all standard CLAP — never reached a host, and the IR
//! plugin's `cabinet_simulator` was dropped for an underscore. Seven of
//! eleven plugins landed in the wrong browser category.
//!
//! The "every plugin declares a category" rule is enforced at **compile
//! time**, not here: `ClapBridge::HAS_CATEGORY` is a const assertion
//! evaluated for each exported plugin, so a plugin without one fails to
//! build. Verified by hand by dropping `AUDIO_EFFECT` from the fixture
//! below, which turns the build into:
//!
//! ```text
//! error[E0080]: evaluation panicked: FEATURES must declare at least one
//! CLAP main category (features::AUDIO_EFFECT, INSTRUMENT, NOTE_EFFECT,
//! NOTE_DETECTOR or ANALYZER)
//!   = note: evaluation of `ClapBridge::<FeaturePlugin>::HAS_CATEGORY`
//!     failed here
//! ```
//!
//! This file covers the predicate behind that assertion, and proves that
//! what a plugin declares is what a host reads, across the real CLAP
//! boundary.

use std::ffi::CStr;

use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;
use resonance_plugin::features::{self, has_category};
use resonance_plugin::{
    ClapBridge, EventIterator, FloatParam, FloatRange, OutputBuffer, Param, ResonancePlugin,
    TempoInfo,
};

// ---------------------------------------------------------------------------
// The predicate behind the compile-time check
// ---------------------------------------------------------------------------

#[test]
fn a_main_category_is_recognised_wherever_it_sits_in_the_list() {
    assert!(has_category(&[features::AUDIO_EFFECT]));
    assert!(has_category(&[features::INSTRUMENT, features::STEREO]));
    assert!(has_category(&[
        features::STEREO,
        features::REVERB,
        features::AUDIO_EFFECT
    ]));
    assert!(has_category(&[features::NOTE_EFFECT]));
    assert!(has_category(&[features::NOTE_DETECTOR]));
    assert!(has_category(&[features::ANALYZER]));
}

#[test]
fn sub_categories_and_capabilities_are_not_categories() {
    // This is the case the compile-time assertion exists to reject: a
    // plugin that says what it does but never what it *is*.
    assert!(!has_category(&[]));
    assert!(!has_category(&[features::STEREO, features::MONO]));
    assert!(!has_category(&[features::REVERB, features::STEREO]));
    assert!(!has_category(&[features::CABINET_SIMULATOR]));
}

#[test]
fn has_category_is_usable_in_a_const_context() {
    // The whole point: the bridge evaluates it at compile time, so this
    // has to hold as a const — if it stopped being const-evaluable this
    // would not compile, which is the assertion.
    const { assert!(has_category(&[features::STEREO, features::INSTRUMENT])) };
    const { assert!(!has_category(&[features::STEREO])) };
}

#[test]
fn cstr_eq_compares_content_not_identity() {
    assert!(features::cstr_eq(c"gate", c"gate"));
    assert!(!features::cstr_eq(c"gate", c"expander"));
    assert!(!features::cstr_eq(c"gate", c"gates"));
    assert!(!features::cstr_eq(c"", c"gate"));
    assert!(features::cstr_eq(c"", c""));
}

#[test]
fn the_named_constants_carry_the_strings_clap_defines() {
    // A misspelled constant *name* is a compile error; these assertions
    // pin the handful this crate defines itself, because clack does not.
    assert_eq!(features::GATE, c"gate");
    assert_eq!(features::EXPANDER, c"expander");
    assert_eq!(features::NOTE_DETECTOR, c"note-detector");
    // Non-standard features must be namespaced, per the CLAP spec.
    assert_eq!(features::CABINET_SIMULATOR, c"resonance:cabinet-simulator");
    // And a few of the ones that used to be dropped by the whitelist.
    assert_eq!(features::COMPRESSOR, c"compressor");
    assert_eq!(features::EQUALIZER, c"equalizer");
    assert_eq!(features::DELAY, c"delay");
    assert_eq!(features::MASTERING, c"mastering");
    assert_eq!(features::ANALYZER, c"analyzer");
}

// ---------------------------------------------------------------------------
// End to end: what a plugin declares is what a host reads
// ---------------------------------------------------------------------------

/// A plugin whose feature list mixes a main category, two sub-categories
/// (one of them non-standard) and a capability — i.e. everything the old
/// whitelist would have thrown away except the first entry.
struct FeaturePlugin {
    mix: FloatParam,
}

impl ResonancePlugin for FeaturePlugin {
    const CLAP_ID: &'static str = "test.features";
    const NAME: &'static str = "Features";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static CStr] = &[
        features::AUDIO_EFFECT,
        features::MASTERING,
        features::CABINET_SIMULATOR,
        features::STEREO,
    ];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            mix: FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
        }
    }
    fn param_count(&self) -> usize {
        1
    }
    fn param(&self, _index: usize) -> &dyn Param {
        &self.mix
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
}

#[test]
fn every_declared_feature_reaches_the_host_verbatim() {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<FeaturePlugin>>>(
        c"resonance-test-features.clap",
    )
    .expect("bundle entry init");

    let factory = entry
        .get_plugin_factory()
        .expect("the bridge must expose a plugin factory");
    let descriptor = factory
        .plugin_descriptor(0)
        .expect("one plugin in the factory");

    let declared: Vec<&CStr> = FeaturePlugin::FEATURES.to_vec();
    let seen: Vec<&CStr> = descriptor.features().collect();

    assert_eq!(
        seen, declared,
        "the host must see exactly what the plugin declared, in order"
    );
    assert!(
        has_category(&seen),
        "and the list a host sees must still carry a category"
    );
}
