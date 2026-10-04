//! FU-P1a: every first-party plugin opts into
//! `ParamTextSource::live_value` for all of its params, not just the
//! handful a plugin moves off the audio thread on its own — so a
//! third-party host that never calls `params.flush` between blocks
//! still reads the real value of an edit made while the transport is
//! stopped, instead of the bridge's shared mirror (which only the
//! per-block push-back refreshes).
//!
//! `resonance-plugin/tests/clap_bridge_params_state.rs` pins the
//! mechanism generically, across the real CLAP C ABI, with a synthetic
//! test plugin. This pins *this* plugin's wiring onto it: every
//! declared parameter, read straight off `ParamTextSource`, follows a
//! value set directly on the shared params (what an editor edit is),
//! with no `process()` block and no `params.flush` in between.
use resonance_gate::params::PARAM_COUNT;
use resonance_gate::ResonanceGate;
use resonance_plugin::ResonancePlugin;

#[test]
fn every_declared_param_has_a_live_value_that_follows_a_direct_write() {
    let plugin = ResonanceGate::new();
    let source = plugin
        .param_text_source()
        .expect("the gate opts into ParamTextSource for live values (FU-P1a)");

    assert_eq!(plugin.param_count(), PARAM_COUNT, "test and plugin agree on the param count");

    for i in 0..plugin.param_count() {
        let p = plugin.param(i);
        let restore = p.get_plain();

        // A value this parameter's own range actually adopts and that
        // differs from whatever it started at — the sweep in
        // `tests/state.rs::off_default` does the same thing for the
        // same reason (a stepped/boolean param can't just take `+1.0`).
        let (min, max) = (p.min_plain(), p.max_plain());
        let mut moved = None;
        for k in 0..16 {
            let frac = (k + 1) as f64 / 17.0;
            p.set_plain(min + (max - min) * frac);
            if p.get_plain() != restore {
                moved = Some(p.get_plain());
                break;
            }
        }
        let Some(expected) = moved else {
            // Only a parameter whose whole range collapses to one value
            // could land here; the gate declares none (ba todo #1340's
            // param-count assertion already guards the declared set).
            panic!("param {i} (`{}`) has no in-range value to move to", p.id());
        };

        assert_eq!(
            source.live_value(i),
            Some(expected),
            "param {i} (`{}`): live_value should follow the direct write, not read a mirror",
            p.id()
        );

        p.set_plain(restore);
    }

    // Out of range: no opinion, so the bridge keeps using its mirror
    // rather than reading garbage.
    assert_eq!(source.live_value(PARAM_COUNT), None);
}
