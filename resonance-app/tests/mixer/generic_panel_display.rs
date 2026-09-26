//! What the host's generic parameter panel prints (ba todo #1290,
//! finding X8's GUI half).
//!
//! The panel used to render every parameter as `{:.2}`, so a filter type
//! read `2.00` and a mix read `0.40` — in the same app whose plugins
//! declare `with_value_to_string` in 100+ places. The layout needs a
//! GUI to test; the decision does not, and the decision is the finding.
//!
//! The panel is host-side code (`resonance_app::plugin_ui`); it moved out
//! of the plugin SDK with ARCH-08.

use resonance_app::plugin_ui::{param_display, UiParam};

fn param(current: f64, text: &str) -> UiParam {
    UiParam {
        id: 1,
        name: "Mix".to_owned(),
        min_value: 0.0,
        max_value: 1.0,
        default_value: 0.35,
        current_value: current,
        text: text.to_owned(),
        stepped: false,
    }
}

#[test]
fn the_plugins_own_formatting_wins() {
    assert_eq!(param_display(&param(0.4, "40 %")), "40 %");
    assert_eq!(param_display(&param(2.0, "Low-pass")), "Low-pass");
    assert_eq!(param_display(&param(8.0, "1/8D")), "1/8D");
}

#[test]
fn a_plugin_that_declares_no_formatting_still_gets_its_number() {
    // The old behaviour, kept for parameters with no `value_to_text`.
    assert_eq!(param_display(&param(0.4, "")), "0.40");
    assert_eq!(param_display(&param(0.4, "   ")), "0.40");
}
