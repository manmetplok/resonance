//! The host reads a first-party plugin's factory bank out of its binary
//! (ba todo #1333).
//!
//! CLAP has no way to enumerate presets compiled into a plugin, so our own
//! plugins export `resonance_factory_presets` and the scan reads it from
//! the library it already has open. This drives the real path against a
//! real built plugin — the encode/decode pair itself is unit-tested in
//! `resonance-common`, and what is worth proving here is that the symbol
//! survives the trip through `cdylib` export, `dlopen` and `dlsym`.
//!
//! Needs a built plugin binary; `plugin_binaries` finds it, and a missing
//! one fails the test unless `RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES` is
//! set — the contract every test under `tests/clap_host/` that needs a
//! built plugin follows.

use resonance_audio::test_support::ClapBundle;

use crate::plugin_binaries::plugin_binary;

/// The EQ ships eleven factory presets. Reading them must not require
/// instantiating the plugin — the host lists them while scanning.
#[test]
fn a_first_party_bundle_publishes_its_factory_bank() {
    let Some(path) = plugin_binary("resonance-eq") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the EQ bundle should load");

    let presets = bundle.factory_presets();
    assert!(
        !presets.is_empty(),
        "the EQ ships factory presets, so the exported symbol should carry them"
    );

    // Every entry must be a loadable state document, not a name with an
    // empty or double-encoded body.
    for (name, json) in presets {
        assert!(!name.is_empty(), "a factory preset with no name");
        let value: serde_json::Value =
            serde_json::from_str(json).unwrap_or_else(|e| panic!("preset '{name}' is not JSON: {e}"));
        assert!(
            value.get("params").and_then(|p| p.as_object()).is_some(),
            "preset '{name}' carries no params object: {json}"
        );
    }
}

/// A plugin that ships no factory presets reports an empty bank rather
/// than failing to load, and so does anything that is not one of ours.
#[test]
fn a_bundle_without_a_bank_loads_and_reports_nothing() {
    // The amp ships no factory presets.
    let Some(path) = plugin_binary("resonance-amp") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the amp bundle should load");
    assert!(
        bundle.factory_presets().is_empty(),
        "the amp ships no factory presets"
    );
}
