//! VIEW-26: the plugin parameter panel sits in a `lazy` region keyed on
//! `plugin_params_fingerprint`, so the Mixer's fast meter tick reuses the
//! built rows. Every field the rows draw must move the key (or the panel
//! freezes on screen); nothing else may (or the cache never hits).

use resonance_app::state::PluginSlotState;
use resonance_app::Resonance;
use resonance_audio::types::ParamInfo;

fn param(id: u32, name: &str, value: f64) -> ParamInfo {
    ParamInfo {
        id,
        name: name.to_owned(),
        min_value: 0.0,
        max_value: 1.0,
        default_value: 0.5,
        current_value: value,
        text: format!("{:.0} %", value * 100.0),
        ..ParamInfo::default()
    }
}

fn slot() -> PluginSlotState {
    PluginSlotState::new(
        10,
        "Synth".into(),
        "com.example.synth".into(),
        "/plugins/synth.clap".into(),
        (0..1000).map(|i| param(i, &format!("P{i}"), 0.25)).collect(),
        true,
    )
}

fn fp(slot: &PluginSlotState) -> u64 {
    Resonance::test_plugin_params_fingerprint(slot)
}

#[test]
fn unchanged_slot_keeps_the_key() {
    assert_eq!(fp(&slot()), fp(&slot()));
}

#[test]
fn every_drawn_param_field_moves_the_key() {
    let base = fp(&slot());
    let edits: [(&str, fn(&mut ParamInfo)); 7] = [
        ("current_value", |p| p.current_value = 0.9),
        ("text", |p| p.text = "90 %".into()),
        ("name", |p| p.name = "Cutoff".into()),
        ("min_value", |p| p.min_value = -1.0),
        ("max_value", |p| p.max_value = 2.0),
        ("stepped", |p| p.stepped = true),
        ("hidden", |p| p.hidden = true),
    ];
    for (field, edit) in edits {
        let mut s = slot();
        edit(&mut s.params[500]);
        assert_ne!(fp(&s), base, "{field} is drawn and must move the key");
    }
    let mut other = slot();
    other.instance_id = 11;
    assert_ne!(fp(&other), base, "another instance is another panel");
}

#[test]
fn undrawn_slot_state_keeps_the_key() {
    let base = fp(&slot());
    let mut s = slot();
    // Header-only state is rendered outside the lazy region.
    s.editor_open = true;
    s.bypassed = true;
    s.plugin_name = "Renamed".into();
    // Hidden params are never drawn.
    s.params[3].hidden = true;
    let hidden_base = fp(&s);
    s.params[3].current_value = 0.8;
    assert_eq!(fp(&s), hidden_base, "a hidden param's value is not drawn");
    s.params[3].hidden = false;
    s.params[3].current_value = 0.25;
    assert_eq!(fp(&s), base, "header-only fields must not rebuild the rows");
}
