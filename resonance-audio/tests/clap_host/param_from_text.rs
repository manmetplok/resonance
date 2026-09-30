//! `ClapInstance::param_from_text`: the host asks a plugin which value a
//! text names (CLAP `text_to_value`), the mirror of `param_text`
//! (nam-model-library.md §9.2). Driven against the real built amp, whose
//! gains parse "-6 dB" and whose model selector parses "slot N" and model
//! names — the path an agent's label takes when a parameter has no
//! enumerated choices.

use resonance_audio::test_support::ClapBundle;

use crate::plugin_binaries::plugin_binary;

#[test]
fn a_first_party_plugin_turns_its_display_text_back_into_a_value() {
    let Some(path) = plugin_binary("resonance-amp") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the amp bundle should load");
    let id = bundle
        .descriptors()
        .first()
        .map(|d| d.id.clone())
        .expect("the amp bundle exposes its plugin");
    let instance = bundle.create_instance(&id, 48_000).expect("create_instance");
    let params = instance.query_params();
    let by_name = |name: &str| {
        params
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("the amp declares {name}"))
            .id
    };

    let output = by_name("Output Gain");
    let half = instance
        .param_from_text(output, "-6.02 dB")
        .expect("a gain parses its own display text");
    assert!((half - 0.5).abs() < 1e-3, "-6.02 dB is a gain of 0.5, got {half}");
    // And the round trip: the text the plugin prints parses back.
    let text = instance.param_text(output, 0.25).expect("value_to_text");
    let back = instance.param_from_text(output, &text).expect("its own text");
    assert!((back - 0.25).abs() < 1e-3, "{text:?} -> {back}");

    let select = by_name("Model Select");
    assert_eq!(instance.param_from_text(select, "slot 7"), Some(7.0));
    assert_eq!(instance.param_from_text(select, "12"), Some(12.0));
    assert_eq!(
        instance.param_from_text(select, "surely no model is called this"),
        None,
        "a text the plugin does not accept is None, not a guess"
    );
    assert_eq!(instance.param_from_text(select, "nul\0inside"), None);
    drop(instance);
    drop(bundle);
}
