//! Source-level guards for the editor's honesty rules.
//!
//! The plugin editors have no snapshot coverage (they need a live Wayland
//! surface), so these tests assert over the editor sources themselves.
//! That is coarse, but it does catch the exact regressions the audit found
//! — a placeholder tab denying a feature that ships (ba todo #1327), a
//! routing control claiming a mode the DSP has no notion of (#1270).

const APP: &str = include_str!("../src/editor/app.rs");
const CHROME: &str = include_str!("../src/editor/chrome.rs");
const PAD_INSPECTOR: &str = include_str!("../src/editor/pad_inspector.rs");

/// Mics and Articulations ship inside the pad inspector, and Mod / FX do
/// not exist at all. No tab may claim otherwise.
#[test]
fn no_tab_says_coming_soon() {
    for (name, src) in [("app.rs", APP), ("chrome.rs", CHROME)] {
        assert!(
            !src.to_lowercase().contains("coming soon"),
            "{name} still advertises a 'coming soon' tab"
        );
    }
}

/// The tab strip lists only views that exist. `Pads` is the only body the
/// app draws, so it is the only label the strip may carry.
#[test]
fn tab_strip_lists_only_the_pads_view() {
    for absent in ["\"Mics\"", "\"Articulations\"", "\"Mod\"", "\"FX\""] {
        assert!(
            !CHROME.contains(absent),
            "tab strip still offers a {absent} tab with no view behind it"
        );
    }
    assert!(
        CHROME.contains("&[\"Pads\"]"),
        "the tab strip should list exactly the Pads view"
    );
}

/// A user who wants mic selection must be told where it lives, since the
/// tab that used to (falsely) promise it is gone.
#[test]
fn tab_bar_points_at_the_pad_inspector() {
    assert!(
        CHROME.contains("Mic and articulation pickers live in each pad's inspector"),
        "removing the Mics/Articulations tabs must leave a pointer behind"
    );
    // …and the pickers really are there.
    assert!(PAD_INSPECTOR.contains("CLOSE MICS"));
    assert!(PAD_INSPECTOR.contains("ARTICULATIONS"));
}
