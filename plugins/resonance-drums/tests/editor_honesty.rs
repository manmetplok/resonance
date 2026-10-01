//! Guards for the editor's honesty rules: nothing drawn that does
//! nothing, claims a feature that does not exist, or denies one that does.
//!
//! Most of these assert over the editor sources. That is coarse, but it
//! catches the exact regressions the audit found — a placeholder tab
//! denying a feature that ships (ba todo #1327), a control whose click is
//! thrown away (#1326). Where a headless frame can answer the question
//! instead (`test_render_editor_frame`), it does: what is on screen is
//! less brittle to check than what a comment says.

const APP: &str = include_str!("../src/editor/app.rs");
const CHROME: &str = include_str!("../src/editor/chrome.rs");
const PAD_INSPECTOR: &str = include_str!("../src/editor/pad_inspector.rs");
const PAD_GRID: &str = include_str!("../src/editor/pad_grid.rs");
const LIBRARY_PANEL: &str = include_str!("../src/editor/library_panel.rs");
const PLOK_PANEL: &str = include_str!("../src/editor/plok_panel.rs");
const KIT_BROWSER: &str = include_str!("../src/editor/kit_browser.rs");
const JOBS: &str = include_str!("../src/editor/jobs.rs");
const MISSING_KIT: &str = include_str!("../src/editor/missing_kit.rs");
const DOWNLOAD: &str = include_str!("../src/download.rs");
const LIBRARY: &str = include_str!("../src/library.rs");

/// Mics and Articulations ship inside the pad inspector, and Mod / FX do
/// not exist at all. No tab may claim otherwise.
#[test]
fn no_tab_says_coming_soon() {
    for (name, src) in [
        ("app.rs", APP),
        ("chrome.rs", CHROME),
        ("pad_grid.rs", PAD_GRID),
        ("library_panel.rs", LIBRARY_PANEL),
        ("plok_panel.rs", PLOK_PANEL),
        ("missing_kit.rs", MISSING_KIT),
    ] {
        assert!(
            !src.to_lowercase().contains("coming soon"),
            "{name} still advertises a 'coming soon' tab"
        );
    }
}

/// Mics and Articulations are pickers inside the pad inspector, and Mod
/// and FX do not exist; chrome must never offer any of them as a view.
///
/// `"Pads"` is deliberately not on this list. The single-option `Pads`
/// segmented control K0 removed was wrong because its click was thrown
/// away, not because of its label — K5's `[Pads | Mix | Setup]` strip is
/// a real view switch. That defect is what
/// `no_control_discards_its_interaction` below catches, in any file.
#[test]
fn chrome_claims_no_view_that_does_not_exist() {
    for absent in ["\"Mics\"", "\"Articulations\"", "\"Mod\"", "\"FX\""] {
        assert!(
            !CHROME.contains(absent),
            "chrome still offers a {absent} tab — that view does not exist"
        );
    }
}

/// Every `widgets::…(…)` call in `src` whose result is thrown away:
/// either `let _ = widgets::…` or a bare `widgets::…(…);` statement.
/// A call that is the tail expression of a closure (`|ui| { widgets::…(…) }`)
/// hands its result on, and is not flagged.
fn discarded_widget_calls(src: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = src.as_bytes();
    let mut from = 0;
    while let Some(off) = src[from..].find("widgets::") {
        let at = from + off;
        from = at + "widgets::".len();
        let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
        let line = &src[line_start..];
        let line = &line[..line.find('\n').unwrap_or(line.len())];
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("use ") {
            continue;
        }
        let before = src[..at].trim_end();
        let let_discard = before.ends_with("let _ =");
        let statement_start =
            before.ends_with(';') || before.ends_with('{') || before.ends_with('}');
        if !(let_discard || statement_start) {
            continue;
        }
        // Find the call's closing paren and look at what follows it.
        let Some(open) = src[at..].find('(').map(|i| at + i) else {
            continue;
        };
        let mut depth = 0usize;
        let mut close = None;
        for (i, &b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { continue };
        if let_discard || src[close + 1..].trim_start().starts_with(';') {
            found.push(line.trim().to_string());
        }
    }
    found
}

/// No control in the editor may throw its interaction away (ba todo
/// #1326). `let _ = widgets::slider…` / a bare `widgets::segmented(…);`
/// is the exact shape the audit found four times in `app.rs` — a control
/// that looks live, moves under the pointer, and changes nothing — and
/// the single-option `Pads` tab in `chrome.rs` was the same thing.
/// Either it writes something or it should not be drawn.
#[test]
fn no_control_discards_its_interaction() {
    for (name, src) in [
        ("app.rs", APP),
        ("chrome.rs", CHROME),
        ("pad_grid.rs", PAD_GRID),
        ("pad_inspector.rs", PAD_INSPECTOR),
        ("library_panel.rs", LIBRARY_PANEL),
        ("plok_panel.rs", PLOK_PANEL),
        ("missing_kit.rs", MISSING_KIT),
    ] {
        let discarded = discarded_widget_calls(src);
        assert!(
            discarded.is_empty(),
            "{name}: a drawn control discards its interaction: {discarded:?}"
        );
    }
}

/// The checker above has to be able to fail.
#[test]
fn the_discarded_control_check_catches_both_shapes() {
    let src = "fn f(ui: &mut Ui) {\n    let _ = widgets::slider_unipolar(ui, 1.0, 0.5);\n    \
               widgets::segmented(\n        ui,\n        &[\"Pads\"],\n        0,\n    );\n}\n";
    assert_eq!(discarded_widget_calls(src).len(), 2, "{src}");
    let kept = "fn f(ui: &mut Ui) {\n    if let Some(v) = widgets::slider_unipolar(ui, 1.0, 0.5) {}\n    \
                probed(ui, \"x\", |ui| {\n        widgets::segmented(ui, &[\"A\"], 0)\n    });\n}\n";
    assert!(discarded_widget_calls(kept).is_empty(), "{kept}");
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
/// which drew above it, must not come back. (`library_overlay.rs` checks
/// the paint order at runtime.)
#[test]
fn the_library_overlay_backdrop_cannot_be_drawn_above_the_panel() {
    assert!(
        !LIBRARY_PANEL.contains("Order::Tooltip"),
        "the backdrop must not go back to painting on a layer above the panel"
    );
    assert!(
        LIBRARY_PANEL.contains("egui::Modal"),
        "the overlay should be a Modal — it is also how it stays click-blocking and Esc-closing"
    );
}

/// The Library overlay has a real, visible entry point: the header's
/// `Library…` button, which replaced "Download kits…" and "Open kit
/// file…" (§6.1). The first entry point was a ghost `Browse` button whose
/// overlay you then could not see (§1.2).
#[cfg(feature = "editor")]
#[test]
fn the_library_overlay_has_a_visible_entry_point() {
    use resonance_plugin::ResonancePlugin;
    let plugin = resonance_drums::ResonanceDrums::new();
    for size in [(960.0, 640.0), (780.0, 520.0)] {
        let frame = resonance_drums::test_render_editor_frame(&plugin, size);
        let button = frame
            .texts
            .iter()
            .find(|t| t.text == "Library…")
            .unwrap_or_else(|| panic!("no Library… button at {size:?}"));
        assert!(
            button.clip.contains_rect(button.rect) && frame.screen.contains_rect(button.rect),
            "the Library… button is not fully visible at {size:?}: {:?} in {:?}",
            button.rect,
            button.clip
        );
        for gone in ["Download kits…", "Open kit file…"] {
            assert!(!frame.shows(gone), "{gone} is back in the header at {size:?}");
        }
    }
}

/// A file dialog opens a modal run loop: never on the editor thread
/// (§6.5, E13). Only `jobs.rs`'s picker thread may name `rfd`.
#[test]
fn no_file_dialog_runs_on_the_editor_thread() {
    for (name, src) in [
        ("app.rs", APP),
        ("chrome.rs", CHROME),
        ("kit_browser.rs", KIT_BROWSER),
        ("library_panel.rs", LIBRARY_PANEL),
        ("plok_panel.rs", PLOK_PANEL),
        ("missing_kit.rs", MISSING_KIT),
    ] {
        assert!(!src.contains("rfd::"), "{name} opens a file dialog on the UI thread");
    }
    assert!(JOBS.contains("rfd::FileDialog"), "the picker moved; update this guard");
}

/// D3: drums no longer reads or writes `installed.json` — the kit
/// library is the one index, and migrates it once.
#[test]
fn drums_no_longer_keeps_installed_json() {
    for (name, src) in [
        ("download.rs", DOWNLOAD),
        ("kit_browser.rs", KIT_BROWSER),
        ("chrome.rs", CHROME),
        ("app.rs", APP),
        ("library_panel.rs", LIBRARY_PANEL),
    ] {
        for call in ["mark_installed", "remove_installed", "list_installed"] {
            assert!(!src.contains(call), "{name} still calls registry::{call}");
        }
    }
    assert!(
        LIBRARY.contains("with_installed_json"),
        "the library must migrate installed.json"
    );
}

/// §6.1: the `DRUMS` label and the "N lit" PADS badge are decoration,
/// and are gone.
#[cfg(feature = "editor")]
#[test]
fn the_tab_bar_carries_no_decoration() {
    use resonance_plugin::ResonancePlugin;
    let plugin = resonance_drums::ResonanceDrums::new();
    let frame = resonance_drums::test_render_editor_frame(&plugin, (960.0, 640.0));
    for t in &frame.texts {
        assert_ne!(t.text, "DRUMS", "the DRUMS label is back");
        assert!(!t.text.ends_with(" lit"), "the lit badge is back: {:?}", t.text);
    }
}

/// The KIT pill's ◀/▶ step from the kit on its way, not the one it
/// replaces. `kit_path` is written only when a load succeeds, so stepping
/// from it while a load ran made two quick ▶ clicks land on the same kit.
#[cfg(feature = "editor")]
#[test]
fn kit_stepping_follows_the_load_in_flight() {
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    use resonance_drums::kit_loader::KitStatus;
    use resonance_plugin::ResonancePlugin;

    let plugin = resonance_drums::ResonanceDrums::new();
    let bridge = &plugin.bridge;
    let a = PathBuf::from("/kits/A/drum_samples.json");
    let b = PathBuf::from("/kits/B/drum_samples.json");
    let step =
        |req: Option<(PathBuf, u64)>| resonance_drums::test_kit_path_for_stepping(bridge, req);

    *bridge.kit_path.lock() = Some(a.clone());
    *bridge.kit_status.lock() = KitStatus::Loaded {
        name: "A".to_string(),
        num_pads: 30,
        unreadable: 0,
        unreadable_paths: Vec::new(),
    };
    assert_eq!(step(None), Some(a.clone()), "settled: the loaded kit");

    // The editor asked for B; the loader has not published anything yet.
    let generation = bridge.load_generation.load(Ordering::Acquire);
    assert_eq!(
        step(Some((b.clone(), generation))),
        Some(b.clone()),
        "B was just requested"
    );

    // A newer load (not this editor's) superseded the request.
    assert_eq!(
        step(Some((b.clone(), generation + 1))),
        Some(a.clone()),
        "stale request"
    );

    // The loader says what it is loading, whoever asked.
    *bridge.kit_status.lock() = KitStatus::Loading { path: b.clone() };
    assert_eq!(step(None), Some(b.clone()), "B is loading");

    // B failed: A is still the kit in place.
    *bridge.kit_status.lock() = KitStatus::Error {
        message: "boom".to_string(),
    };
    assert_eq!(step(Some((b, generation))), Some(a), "B failed to load");
}
