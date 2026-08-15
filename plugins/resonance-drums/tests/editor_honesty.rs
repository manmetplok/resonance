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

/// No control in the body may throw its interaction away (ba todo
/// #1326). `let _ = widgets::slider…` / `widgets::segmented…` is the
/// exact shape the audit found four times in this file: a control that
/// looks live, moves under the pointer, and changes nothing. Either it
/// writes a parameter or it should not be drawn.
///
/// The one legitimate discard is in `chrome.rs` — the tab strip has a
/// single tab, so its click has nowhere to go — and this test does not
/// cover that file.
#[test]
fn no_control_in_the_body_discards_its_interaction() {
    for line in APP.lines() {
        let trimmed = line.trim_start();
        assert!(
            !trimmed.starts_with("let _ = widgets::"),
            "a drawn control discards its interaction: {trimmed}"
        );
    }
}

/// "preview" was the badge the GLOBAL card wore to admit its controls
/// did nothing. The controls work now, so the badge must not survive —
/// a working card labelled "preview" is its own kind of lie.
#[test]
fn no_card_is_labelled_preview() {
    assert!(
        !APP.contains("\"preview\""),
        "a card still advertises itself as a preview"
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
