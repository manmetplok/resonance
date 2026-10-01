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
const PAD_GRID: &str = include_str!("../src/editor/pad_grid.rs");
const DOWNLOAD_PANEL: &str = include_str!("../src/editor/download_panel.rs");

/// Mics and Articulations ship inside the pad inspector, and Mod / FX do
/// not exist at all. No tab may claim otherwise.
#[test]
fn no_tab_says_coming_soon() {
    for (name, src) in [
        ("app.rs", APP),
        ("chrome.rs", CHROME),
        ("pad_grid.rs", PAD_GRID),
        ("download_panel.rs", DOWNLOAD_PANEL),
    ] {
        assert!(
            !src.to_lowercase().contains("coming soon"),
            "{name} still advertises a 'coming soon' tab"
        );
    }
}

/// The editor has exactly one view (Pads), so there is nothing to switch
/// it with: K0 removed the single-option `Pads` segmented control along
/// with the rest of the chrome that looked interactive and did nothing
/// (ba drums-plugin-rework.md §10). What's left to guard is that chrome
/// never claims a view that does not exist.
#[test]
fn chrome_claims_no_view_that_does_not_exist() {
    for absent in ["\"Mics\"", "\"Articulations\"", "\"Mod\"", "\"FX\"", "\"Pads\""] {
        assert!(
            !CHROME.contains(absent),
            "chrome still offers a {absent} tab — there is only one view, and it is not switched"
        );
    }
}

/// No control in the body may throw its interaction away (ba todo
/// #1326). `let _ = widgets::slider…` / `widgets::segmented…` is the
/// exact shape the audit found four times in this file: a control that
/// looks live, moves under the pointer, and changes nothing. Either it
/// writes a parameter or it should not be drawn.
///
/// `chrome.rs` used to be exempt — its tab strip had a single tab, so its
/// click had nowhere to go — but that discarded control is gone (K0), so
/// every editor file is covered now.
#[test]
fn no_control_in_the_body_discards_its_interaction() {
    for (name, src) in [
        ("app.rs", APP),
        ("chrome.rs", CHROME),
        ("pad_grid.rs", PAD_GRID),
        ("download_panel.rs", DOWNLOAD_PANEL),
    ] {
        for line in src.lines() {
            let trimmed = line.trim_start();
            assert!(
                !trimmed.starts_with("let _ = widgets::"),
                "{name}: a drawn control discards its interaction: {trimmed}"
            );
        }
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

/// Audition is a live button again (ba todo #1328): it hands the pad's
/// note to the audio thread. The disabled placeholder and its "not
/// wired up yet" apology must both be gone — a control that works and
/// still apologises is as misleading as one that does not.
#[test]
fn audition_is_live_and_no_longer_apologises() {
    assert!(
        PAD_INSPECTOR.contains("bridge.audition(mapping.note)"),
        "the Audition button must trigger the pad"
    );
    assert!(
        !PAD_INSPECTOR.contains("not wired up yet"),
        "Audition still tells the user it does nothing"
    );
}

/// The tab bar's "Mic and articulation pickers live in each pad's
/// inspector →" hint is gone (K0 — one more piece of chrome that pointed
/// at a problem the removed Mics/Articulations tabs created, rather than
/// doing anything itself). What has to stay true is the thing it used to
/// point at: the pickers really are in the inspector.
#[test]
fn the_mic_and_articulation_pickers_still_live_in_the_inspector() {
    assert!(PAD_INSPECTOR.contains("CLOSE MICS"));
    assert!(PAD_INSPECTOR.contains("ARTICULATIONS"));
}

/// The ghost `Browse` button and the `Load kit` button next to it used to
/// live in the pad-list kit card — `Browse` opened an overlay you then
/// couldn't see (§1.2), and neither label said what it did. Both moved to
/// the header (`chrome.rs`), clearly labelled, and neither the buttons
/// nor the code that opened them belongs in `pad_grid.rs` any more.
#[test]
fn pad_grid_no_longer_hides_the_kit_actions() {
    for gone in ["\"Browse\"", "\"Load kit\"", "download_panel", "kit_browser"] {
        assert!(
            !PAD_GRID.contains(gone),
            "pad_grid.rs still references {gone} — the kit actions should live in chrome.rs now"
        );
    }
}

/// The overlay's backdrop must be painted on the same layer as the panel
/// it dims, not above it (ba drums-plugin-rework.md §1.2) — the exact
/// defect that made opening "Download Kits" show a near-black screen.
/// `egui::Modal` keeps both on `Order::Foreground`; `Order::Tooltip`,
/// which drew above it, must not come back.
#[test]
fn the_download_overlay_backdrop_cannot_be_drawn_above_the_panel() {
    assert!(
        !DOWNLOAD_PANEL.contains("Order::Tooltip"),
        "the backdrop must not go back to painting on a layer above the panel"
    );
    assert!(
        DOWNLOAD_PANEL.contains("egui::Modal"),
        "the overlay should be a Modal — it is also how it stays click-blocking and Esc-closing"
    );
}

/// The doc comment used to describe a `Download Kits` button that did not
/// exist — the real entry point was a ghost `Browse` button elsewhere
/// (§1.2). Now that the real button is back, the doc comment has to name
/// it correctly.
#[test]
fn the_module_doc_names_the_real_entry_point() {
    assert!(
        DOWNLOAD_PANEL.contains("Download kits…"),
        "the doc comment should point at the real header button, not a stale name"
    );
}
