//! The Library overlay (drums-plugin-rework.md §6.5, §9), driven as a
//! whole editor frame by frame through the `TestEditor` hook
//! (`editor/mod.rs`; the editor module is private outside the crate).
//!
//! Every test builds its own kit library at a temp root, with fixture kits
//! written straight to disk and a download worker pointed at a closed
//! loopback port: nothing here reads the user's library or reaches the
//! network.
//!
//! The old Download Kits overlay painted its backdrop on a layer *above*
//! the panel (a near-black screen, §1.2), was not modal, and ignored Esc.
//! The checks for those carry over here, against the Library overlay.
#![cfg(feature = "editor")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use plugin_gui_core::egui;
use resonance_drums::download::{ServerIndex, ServerKit, WorkerConfig};
use resonance_drums::library::{Roots, SharedKitLibrary};
use resonance_drums::{EditorFrameProbe, ResonanceDrums, TestEditor};
use resonance_plugin::ResonancePlugin;

const SIZE: (f32, f32) = (960.0, 640.0);

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-drums-overlay-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drumkits")).unwrap();
        Self(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("drumkits")
    }

    fn library(&self) -> Arc<SharedKitLibrary> {
        SharedKitLibrary::open(Roots {
            root: Some(self.root()),
            marks_dir: Some(self.0.join("library")),
            installed_json: None,
            worker: WorkerConfig {
                index_url: "http://127.0.0.1:9/index.json".into(),
                ..WorkerConfig::default()
            },
        })
    }

    /// Write a kit `<root>/<dir>/<dir_lower>/drum_samples.json` whose
    /// `_meta.name` is `name`; returns the manifest path.
    fn kit(&self, dir: &str, name: &str, pieces: usize) -> PathBuf {
        let inner = self
            .root()
            .join(dir)
            .join(dir.to_lowercase().replace(' ', ""));
        write_kit(&inner, name, pieces)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_kit(dir: &Path, name: &str, pieces: usize) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let mut obj = serde_json::Map::new();
    for p in 0..pieces {
        obj.insert(
            format!("Piece {p} of {name}"),
            serde_json::json!({
                "01_KickIn_e901": {
                    "brand": "Sennheiser", "channel": "01", "mic": "e901",
                    "position": "KickIn",
                    "rounds": { "RR01": { "Vel01": format!("p{p}.wav") } }
                }
            }),
        );
        std::fs::write(dir.join(format!("p{p}.wav")), [0u8; 64]).unwrap();
    }
    obj.insert("_meta".into(), serde_json::json!({ "name": name }));
    let path = dir.join("drum_samples.json");
    std::fs::write(&path, serde_json::to_string_pretty(&obj).unwrap()).unwrap();
    path
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Open the overlay and let the modal settle (it knows it is on top from
/// its second frame).
fn opened(editor: &mut TestEditor) -> EditorFrameProbe {
    editor.open_library();
    editor.frame(Vec::new());
    editor.frame(Vec::new())
}

fn shape_has_text(shape: &egui::Shape, needle: &str) -> bool {
    match shape {
        egui::Shape::Text(t) => t.galley.text() == needle,
        egui::Shape::Vec(v) => v.iter().any(|s| shape_has_text(s, needle)),
        _ => false,
    }
}

/// A black, partly opaque fill covering the whole window: the backdrop
/// (whatever alpha the modal's fade-in has reached).
fn shape_is_backdrop(shape: &egui::Shape, screen: egui::Rect) -> bool {
    match shape {
        egui::Shape::Rect(r) => {
            r.fill.r() == 0
                && r.fill.g() == 0
                && r.fill.b() == 0
                && r.fill.a() > 0
                && r.rect.contains_rect(screen)
        }
        egui::Shape::Vec(v) => v.iter().any(|s| shape_is_backdrop(s, screen)),
        _ => false,
    }
}

#[test]
fn it_opens_on_plok_org_when_empty_and_on_installed_otherwise() {
    let home = Home::new("tab");
    let lib = home.library();
    let plugin = ResonanceDrums::new();

    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    assert_eq!(
        editor.library_tab(),
        "plok.org",
        "an empty library opens on plok.org"
    );

    home.kit("Alpha", "Alpha Kit", 2);
    lib.rescan().unwrap().unwrap();
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    let frame = opened(&mut editor);
    assert_eq!(editor.library_tab(), "Installed");
    assert!(frame.shows("Alpha Kit"), "{:?}", frame.strings());
}

/// §1.2: the dimming fill must be painted before the panel's own content,
/// so it sits beneath it, not over it.
#[test]
fn the_backdrop_is_painted_below_the_panel() {
    let home = Home::new("backdrop");
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, home.library(), SIZE);
    let closed = editor.frame(Vec::new());
    assert!(
        !closed
            .shapes
            .iter()
            .any(|s| shape_is_backdrop(&s.shape, closed.screen)),
        "a closed overlay must not dim the editor"
    );
    let frame = opened(&mut editor);
    let backdrop = frame
        .shapes
        .iter()
        .position(|s| shape_is_backdrop(&s.shape, frame.screen))
        .expect("the overlay dims the editor while open");
    let title = frame
        .shapes
        .iter()
        .position(|s| shape_has_text(&s.shape, "KIT LIBRARY"))
        .expect("the panel title is drawn");
    assert!(
        backdrop < title,
        "the backdrop ({backdrop}) is painted over the panel ({title})"
    );
}

/// Esc and Close close the overlay; a click on the backdrop — far more
/// often a miss than a request to dismiss — does not.
#[test]
fn escape_and_close_close_it_and_a_backdrop_click_does_not() {
    let home = Home::new("close");
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, home.library(), SIZE);

    opened(&mut editor);
    // Outside the panel's 20 px margin: on the backdrop, over the header.
    editor.click(egui::pos2(6.0, 6.0));
    assert!(editor.library_open(), "a backdrop click closed the overlay");
    editor.frame(vec![escape()]);
    assert!(!editor.library_open(), "Esc did not close the overlay");

    let frame = opened(&mut editor);
    let close = frame.text_center("Close").expect("Close is drawn");
    editor.click(close);
    assert!(!editor.library_open(), "Close did not close the overlay");
}

#[test]
fn installed_lists_the_kits_with_search_and_the_favourites_filter() {
    let home = Home::new("list");
    for (dir, name) in [("A", "Alpha Kit"), ("B", "Beta Kit"), ("C", "Gamma Kit")] {
        home.kit(dir, name, 2);
    }
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    let frame = opened(&mut editor);
    for name in ["Alpha Kit", "Beta Kit", "Gamma Kit"] {
        assert!(
            frame.shows(name),
            "{name} is not listed: {:?}",
            frame.strings()
        );
    }
    assert!(
        frame.strings().iter().any(|s| s.starts_with("3 kits · ")),
        "the kit count is missing: {:?}",
        frame.strings()
    );

    editor.search("beta");
    assert_eq!(editor.view_names(), vec!["Beta Kit".to_string()]);
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("Beta Kit") && !frame.shows("Alpha Kit"));

    editor.search("");
    let gamma = lib.read().find("Gamma Kit").unwrap().id.clone();
    lib.toggle_favorite(&gamma).unwrap();
    assert_eq!(
        editor.view_names().first().map(String::as_str),
        Some("Gamma Kit"),
        "favourites sort first"
    );
    editor.set_favorites_only(true);
    assert_eq!(editor.view_names(), vec!["Gamma Kit".to_string()]);
}

#[test]
fn the_detail_pane_names_the_kit_its_mics_and_who_uses_it() {
    let home = Home::new("detail");
    let manifest = home.kit("Alpha", "Alpha Kit", 3);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let other = ResonanceDrums::new();
    // Two live instances play this kit.
    *plugin.bridge.kit_path.lock() = Some(manifest.clone());
    *other.bridge.kit_path.lock() = Some(manifest);

    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    let frame = editor.frame(Vec::new());
    let all = frame.strings().join("\n");
    assert!(
        all.contains("Sennheiser e901 · KickIn"),
        "friendly mic label missing:\n{all}"
    );
    assert!(
        all.contains("Piece 0 of Alpha Kit"),
        "pieces missing:\n{all}"
    );
    assert!(frame.shows("used in 2 open drum instances"), "{all}");
    for button in ["Load", "Reveal", "Delete…"] {
        assert!(frame.shows(button), "{button} is missing:\n{all}");
    }
    assert!(
        !frame.shows("Re-download"),
        "a local kit offers Re-download"
    );
}

/// Delete asks in place first — naming the kit, its size and who still
/// plays it — then deletes on a job; Esc disarms an armed delete.
#[test]
fn delete_confirms_in_place_then_removes_the_kit() {
    let home = Home::new("delete");
    let manifest = home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha = lib.read().find("Alpha Kit").unwrap().clone();
    lib.toggle_favorite(&alpha.id).unwrap();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(manifest);

    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    let frame = editor.frame(Vec::new());
    let arm = frame.text_center("Delete…").expect("Delete… is drawn");
    let frame = editor.click(arm);
    assert!(
        editor.pending_delete().is_some(),
        "the first click must only arm"
    );
    let all = frame.strings().join("\n");
    assert!(
        all.contains("Delete \"Alpha Kit\" ("),
        "the prompt names the kit and its size:\n{all}"
    );
    assert!(all.contains("Used by 1 open drum instance"), "{all}");
    assert!(alpha.dir.exists());

    // Esc disarms (and only that); reopening starts with nothing armed.
    editor.frame(vec![escape()]);
    assert_eq!(editor.pending_delete(), None);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    let frame = editor.frame(Vec::new());
    editor.click(frame.text_center("Delete…").unwrap());
    let frame = editor.frame(Vec::new());
    let confirm = frame
        .text_center("Delete")
        .expect("the confirm button is drawn");
    editor.click(confirm);
    editor.finish_jobs();
    assert!(!alpha.dir.exists(), "the kit directory is still there");
    assert_eq!(lib.read().len(), 1);
    assert_eq!(editor.notice().as_deref(), Some("deleted \"Alpha Kit\""));
    let marks = lib.marks_of(&alpha.id);
    assert!(marks.favorite, "marks are kept for the orphan window");
    assert!(
        marks.orphaned_at.is_some(),
        "the orphan pass did not stamp the marks"
    );
}

/// The plok.org tab, from a fixture index: `Installed` when the manifest
/// hash is in the library, `Update` when only the name of a kit from
/// plok.org is, `Download`
/// otherwise. A fresh index is not fetched again on opening the tab.
#[test]
fn the_plok_tab_shows_installed_update_and_download() {
    let home = Home::new("plok");
    home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
    // Only a kit that came from plok.org is offered as an Update.
    resonance_common::drumkit_library::write_sidecar(
        &home.root().join("Beta"),
        &resonance_common::drumkit_library::Sidecar {
            source: resonance_common::drumkit_library::SOURCE_PLOK.into(),
            index_name: Some("Beta Kit".into()),
            index_file: Some("beta.zip".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha_id = lib.read().find("Alpha Kit").unwrap().id.clone();
    {
        let mut s = lib.download().state.lock();
        s.index = Some(ServerIndex {
            drumkits: vec![
                ServerKit {
                    manifest_sha256: Some(alpha_id),
                    description: Some("The first kit".into()),
                    ..ServerKit::new("Alpha Kit", "alpha.zip")
                },
                ServerKit {
                    manifest_sha256: Some("f".repeat(64)),
                    ..ServerKit::new("Beta Kit", "beta.zip")
                },
                ServerKit {
                    bytes: Some(5_000_000),
                    tags: vec!["electronic".into()],
                    ..ServerKit::new("Delta Kit", "delta.zip")
                },
            ],
        });
        s.index_fetched_at = Some(Instant::now());
    }
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.show_tab(true);
    assert!(
        !lib.download().is_running(),
        "a fresh index was fetched again on opening the tab"
    );
    let frame = editor.frame(Vec::new());
    for probe in [
        "plok.Alpha Kit.installed",
        "plok.Beta Kit.update",
        "plok.Delta Kit.download",
    ] {
        assert!(
            frame.widget(probe).is_some(),
            "{probe} missing: {:?}",
            frame.strings()
        );
    }
    assert!(frame.shows("The first kit"));
    assert!(frame.shows("electronic"));

    let download = frame
        .widget("plok.Delta Kit.download")
        .unwrap()
        .rect
        .center();
    editor.click(download);
    assert_eq!(editor.my_downloads(), vec!["Delta Kit".to_string()]);
    assert!(
        lib.download().is_running(),
        "Download sent nothing to the worker"
    );
}

/// §6.1: the header's kit dropdown shows the library's name for the loaded
/// kit (its `_meta.name`), not the manifest's directory, with ☆/★ beside
/// it.
#[test]
fn the_header_dropdown_shows_the_library_name_of_the_loaded_kit() {
    let home = Home::new("header");
    let manifest = home.kit("fancy-dir", "Fancy Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(manifest);
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("Fancy Kit"), "{:?}", frame.strings());
    assert!(!frame.shows("fancydir") && !frame.shows("fancy-dir"));
    assert!(
        frame.widget("kit.star").is_some(),
        "no ☆/★ for the loaded kit"
    );
    assert!(frame.shows("1 / 1 in view"));
}

// ---------------------------------------------------------------------------
// Review fixes (fix2-ui): keyboard, fit, races, parity with the amp
// ---------------------------------------------------------------------------

fn key_ev(k: egui::Key) -> egui::Event {
    egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn text_ev(s: &str) -> egui::Event {
    egui::Event::Text(s.to_string())
}

/// The plugin as a host has activated it: a load actually starts the
/// loader (and so records a pick) instead of refusing for want of a
/// sample rate.
fn activated() -> ResonanceDrums {
    let plugin = ResonanceDrums::new();
    plugin
        .bridge
        .sample_rate
        .store(48_000.0f32.to_bits(), std::sync::atomic::Ordering::Release);
    plugin
}

/// Antialiasing and glyph bounds can poke a fraction of a pixel past a
/// clip that still shows the whole thing.
const TOLERANCE: f32 = 1.0;

/// `rect` is wholly visible: inside the clip it was drawn under and the
/// window.
fn fully_visible(rect: egui::Rect, clip: egui::Rect, screen: egui::Rect) -> Result<(), String> {
    let v = rect.intersect(clip).intersect(screen);
    if !(v.width() > 0.0 && v.height() > 0.0) {
        return Err(format!("not visible: rect {rect:?}, clip {clip:?}"));
    }
    if !clip.expand(TOLERANCE).contains_rect(rect) {
        return Err(format!("clipped: rect {rect:?} outside its clip {clip:?}"));
    }
    if !screen.expand(TOLERANCE).contains_rect(rect) {
        return Err(format!("off-window: rect {rect:?}"));
    }
    Ok(())
}

fn assert_text_fully_visible(frame: &EditorFrameProbe, needle: &str, at: &str) {
    let hits: Vec<_> = frame.texts.iter().filter(|t| t.text == needle).collect();
    assert!(!hits.is_empty(), "{needle:?} is not drawn at all ({at})");
    let errs: Vec<String> = hits
        .iter()
        .filter_map(|t| fully_visible(t.rect, t.clip, frame.screen).err())
        .collect();
    assert!(
        errs.len() < hits.len(),
        "{needle:?} is not visible ({at}): {errs:?}"
    );
}

fn assert_widget_fully_visible(frame: &EditorFrameProbe, name: &str, at: &str) {
    let w = frame
        .widget(name)
        .unwrap_or_else(|| panic!("{name} was not laid out ({at})"));
    if let Err(e) = fully_visible(w.rect, w.clip, frame.screen) {
        panic!("{name} is not visible ({at}): {e}");
    }
}

fn widget_center(frame: &EditorFrameProbe, name: &str) -> egui::Pos2 {
    frame
        .widget(name)
        .unwrap_or_else(|| panic!("{name} was not laid out: {:?}", frame.strings()))
        .rect
        .center()
}

const LONG_NAME: &str =
    "The Extraordinarily Long-Named Thirty-Five Piece Fourteen Mic Studio Kit (2026 Remaster)";

/// Drummica's shape: 35 pieces, each recorded through 14 mic setups.
fn big_kit(home: &Home, dir: &str, name: &str) -> PathBuf {
    let inner = home.root().join(dir).join("kit");
    std::fs::create_dir_all(&inner).unwrap();
    let positions = [
        "KickIn", "KickOut", "SnTop", "SnBot", "HatClose", "Tom1", "Tom2", "Tom3", "OHL", "OHR",
        "RoomL", "RoomR", "Mono", "Crush",
    ];
    let mut obj = serde_json::Map::new();
    for p in 0..35 {
        let mut mics = serde_json::Map::new();
        for (m, pos) in positions.iter().enumerate() {
            let file = format!("p{p}_m{m}.wav");
            std::fs::write(inner.join(&file), [0u8; 64]).unwrap();
            mics.insert(
                format!("{:02}_{pos}_mic{m}", m + 1),
                serde_json::json!({
                    "brand": "Brand", "channel": format!("{:02}", m + 1),
                    "mic": format!("Mic{m}"), "position": pos,
                    "rounds": { "RR01": { "Vel01": file } }
                }),
            );
        }
        obj.insert(format!("Piece number {p}"), serde_json::Value::Object(mics));
    }
    obj.insert("_meta".into(), serde_json::json!({ "name": name }));
    let path = inner.join("drum_samples.json");
    std::fs::write(&path, serde_json::to_string_pretty(&obj).unwrap()).unwrap();
    path
}

/// §6.1 / review #3, #4: at both declared window sizes, with a library of
/// a dozen kits and a long-named 35-piece, 14-setup kit selected, the
/// action row is on screen — not below the fold of the detail's scroll —
/// and so are the confirm's buttons once Delete… is armed, the long name
/// notwithstanding.
#[test]
fn a_big_kits_actions_and_delete_confirm_stay_in_view() {
    let home = Home::new("fit");
    for i in 0..12 {
        home.kit(&format!("K{i:02}"), &format!("Kit number {i}"), 2);
    }
    big_kit(&home, "Big", LONG_NAME);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    for size in [(780.0, 520.0), (960.0, 640.0)] {
        let at = format!("{size:?}");
        let mut editor = TestEditor::new(&plugin, lib.clone(), size);
        opened(&mut editor);
        assert!(editor.select(LONG_NAME));
        editor.finish_jobs();
        editor.frame(Vec::new());
        let frame = editor.frame(Vec::new());
        let panel = frame.widget("library.panel").unwrap();
        assert!(
            frame.screen.expand(TOLERANCE).contains_rect(panel.rect),
            "the panel overflows at {at}: {:?}",
            panel.rect
        );
        for name in [
            "library.action.load",
            "library.action.reveal",
            "library.action.delete",
            "library.rescan",
        ] {
            assert_widget_fully_visible(&frame, name, &at);
        }

        editor.click(widget_center(&frame, "library.action.delete"));
        assert!(
            editor.pending_delete().is_some(),
            "Delete… did not arm at {at}"
        );
        let frame = editor.frame(Vec::new());
        for text in ["Delete", "Cancel"] {
            assert_text_fully_visible(&frame, text, &at);
        }
        let prompt = frame
            .texts
            .iter()
            .find(|t| t.text.starts_with("Delete \"The Extraordinarily"))
            .unwrap_or_else(|| panic!("no prompt at {at}: {:?}", frame.strings()));
        let seen = prompt.rect.intersect(prompt.clip);
        assert!(
            panel.rect.expand(TOLERANCE).contains_rect(seen) && seen.width() > 0.0,
            "the prompt runs past the panel at {at}: {:?}",
            prompt.rect
        );
        let panel = frame.widget("library.panel").unwrap();
        assert!(
            frame.screen.expand(TOLERANCE).contains_rect(panel.rect),
            "arming the delete pushed the panel past the window at {at}: {:?}",
            panel.rect
        );
    }
}

/// Click the search field and type `text` into it.
fn type_in_search(editor: &mut TestEditor, text: &str) {
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.search"));
    editor.frame(vec![text_ev(text)]);
}

/// Review #1: Enter in the search box ends the edit. It used to load the
/// selected kit — even when the search had just filtered it out of view.
#[test]
fn enter_in_the_search_field_loads_nothing() {
    let home = Home::new("enter");
    home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha = lib.read().find("Alpha Kit").unwrap().id.clone();
    let beta = lib.read().find("Beta Kit").unwrap().id.clone();
    let plugin = activated();
    let generation = || {
        plugin
            .bridge
            .load_generation
            .load(std::sync::atomic::Ordering::Acquire)
    };
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    let before = generation();
    type_in_search(&mut editor, "beta");
    assert_eq!(editor.view_names(), vec!["Beta Kit".to_string()]);
    editor.frame(vec![key_ev(egui::Key::Enter)]);
    editor.frame(Vec::new());
    assert_eq!(
        generation(),
        before,
        "Enter in the search field started a load"
    );
    assert!(lib.marks_of(&alpha).last_used.is_none());
    assert!(lib.marks_of(&beta).last_used.is_none());
    assert!(editor.library_open());
}

/// Review #2: Esc while typing in the search or the tag field leaves the
/// field; it used to close the whole overlay (egui drops the field's focus
/// before the overlay's check ran). A second Esc closes.
#[test]
fn escape_while_typing_only_leaves_the_field() {
    let home = Home::new("esc-typing");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    opened(&mut editor);

    type_in_search(&mut editor, "alp");
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    assert!(
        editor.library_open(),
        "Esc in the search field closed the overlay"
    );
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    assert!(!editor.library_open(), "a second Esc did not close it");

    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(frame.text_center("+ tag").expect("the tag field is drawn"));
    editor.frame(vec![text_ev("ro")]);
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    assert!(
        editor.library_open(),
        "Esc in the tag field closed the overlay"
    );
}

/// Esc with a delete armed backs out of the delete; only the next Esc
/// closes the overlay.
#[test]
fn escape_disarms_a_delete_before_it_closes() {
    let home = Home::new("esc-delete");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.action.delete"));
    assert!(editor.pending_delete().is_some());
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    assert_eq!(editor.pending_delete(), None, "Esc did not disarm");
    assert!(
        editor.library_open(),
        "Esc closed the overlay with a delete armed"
    );
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    assert!(!editor.library_open());
}

/// The search and ★ filters, driven through the widgets themselves:
/// typing into the field and clicking the chip.
#[test]
fn search_and_the_favourites_chip_filter_through_the_widgets() {
    let home = Home::new("filters");
    for (dir, name) in [("A", "Alpha Kit"), ("B", "Beta Kit"), ("C", "Gamma Kit")] {
        home.kit(dir, name, 2);
    }
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let gamma = lib.read().find("Gamma Kit").unwrap().id.clone();
    lib.toggle_favorite(&gamma).unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    opened(&mut editor);

    type_in_search(&mut editor, "beta");
    let frame = editor.frame(Vec::new());
    assert_eq!(editor.view_names(), vec!["Beta Kit".to_string()]);
    assert!(frame.shows("Beta Kit") && !frame.shows("Alpha Kit"));
    // Clear the field the way a user does.
    editor.frame(vec![
        key_ev(egui::Key::Backspace),
        key_ev(egui::Key::Backspace),
        key_ev(egui::Key::Backspace),
        key_ev(egui::Key::Backspace),
    ]);
    assert_eq!(editor.view_names().len(), 3);

    let frame = editor.frame(Vec::new());
    editor.click(frame.text_center("★ only").expect("the ★ chip is drawn"));
    assert_eq!(editor.view_names(), vec!["Gamma Kit".to_string()]);
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("Gamma Kit") && !frame.shows("Beta Kit"));
}

/// The tag editor: type a tag and Enter adds it; the pill's × removes it.
#[test]
fn the_tag_editor_adds_and_removes_tags() {
    let home = Home::new("tags");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha = lib.read().find("Alpha Kit").unwrap().id.clone();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(frame.text_center("+ tag").expect("the tag field is drawn"));
    editor.frame(vec![text_ev("rock")]);
    editor.frame(vec![key_ev(egui::Key::Enter)]);
    assert_eq!(lib.marks_of(&alpha).tags, vec!["rock".to_string()]);

    let frame = editor.frame(Vec::new());
    let pill = frame
        .texts
        .iter()
        .find(|t| t.text == "rock")
        .expect("the tag pill is drawn")
        .rect;
    // The × sits right of the label: its 7 px padding, then a 16 px box.
    editor.click(egui::pos2(pill.max.x + 7.0 + 8.0, pill.center().y));
    assert!(
        lib.marks_of(&alpha).tags.is_empty(),
        "× did not remove the tag"
    );
}

/// The list's ☆ toggles that row's favourite without selecting it; the
/// header's ☆ toggles the loaded kit's.
#[test]
fn the_list_and_header_stars_toggle_favourites() {
    let home = Home::new("stars");
    let manifest = home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha = lib.read().find("Alpha Kit").unwrap().id.clone();
    let beta = lib.read().find("Beta Kit").unwrap().id.clone();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(manifest);
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);

    // Header, overlay closed.
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "kit.star"));
    assert!(
        lib.marks_of(&alpha).favorite,
        "the header ☆ did not favourite the kit"
    );

    // The list row's star: 14 px left of its title.
    let frame = opened(&mut editor);
    let title = frame
        .texts
        .iter()
        .find(|t| t.text == "Beta Kit" && t.clip.intersects(t.rect))
        .expect("Beta Kit is listed")
        .rect;
    editor.click(egui::pos2(title.min.x - 14.0, title.center().y));
    assert!(
        lib.marks_of(&beta).favorite,
        "the list ☆ did not favourite the row"
    );
    assert_ne!(
        editor.selected_name().as_deref(),
        Some("Beta Kit"),
        "the star click selected the row"
    );
}

/// A pick — Load, a double-click — counts as a use for Recent; stepping
/// with ◀/▶ does not (it would re-sort a Recent view under the stepping).
#[test]
fn loading_records_a_use_and_stepping_does_not() {
    let home = Home::new("recent");
    for (dir, name) in [("A", "Alpha Kit"), ("B", "Beta Kit"), ("C", "Gamma Kit")] {
        home.kit(dir, name, 2);
    }
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let id = |name: &str| lib.read().find(name).unwrap().id.clone();
    let plugin = activated();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);

    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.action.load"));
    assert!(
        lib.marks_of(&id("Alpha Kit")).last_used.is_some(),
        "Load recorded no use"
    );

    let frame = editor.frame(Vec::new());
    let beta = frame.text_center("Beta Kit").expect("Beta Kit is listed");
    // egui counts a third click within 0.6 s of the first as a triple
    // click: let the Load click age out (frames advance 1/60 s each).
    for _ in 0..40 {
        editor.frame(Vec::new());
    }
    editor.click(beta);
    editor.click(beta);
    assert!(
        lib.marks_of(&id("Beta Kit")).last_used.is_some(),
        "a double-click recorded no use"
    );

    // ◀/▶ from the header, overlay closed: Beta → Gamma is browsing.
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    assert!(!editor.library_open());
    let frame = editor.frame(Vec::new());
    let generation = || {
        plugin
            .bridge
            .load_generation
            .load(std::sync::atomic::Ordering::Acquire)
    };
    let before = generation();
    editor.click(widget_center(&frame, "kit.next"));
    editor.frame(Vec::new());
    assert!(generation() > before, "▶ loaded nothing");
    assert!(
        lib.marks_of(&id("Gamma Kit")).last_used.is_none(),
        "▶ recorded a use"
    );
}

/// ◀/▶ are disabled at the ends of the view.
#[test]
fn the_step_arrows_are_disabled_at_the_view_ends() {
    let home = Home::new("ends");
    let first = home.kit("A", "Alpha Kit", 2);
    home.kit("B", "Beta Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(first);
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert_eq!(editor.view_names()[0], "Alpha Kit");
    assert!(
        frame.widget("kit.prev.enabled").is_none(),
        "◀ is live at the first kit"
    );
    assert!(
        frame.widget("kit.next.enabled").is_some(),
        "▶ is dead with a kit after"
    );
}

/// The plok.org tab's live states: a download's progress bar and Cancel,
/// a queued one, a worker error, and Load beside Installed.
#[test]
fn the_plok_tab_shows_progress_queued_error_and_load() {
    let home = Home::new("plok-states");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha = lib.read().find("Alpha Kit").unwrap().id.clone();
    {
        let mut s = lib.download().state.lock();
        s.index = Some(ServerIndex {
            drumkits: vec![
                ServerKit {
                    manifest_sha256: Some(alpha.clone()),
                    ..ServerKit::new("Alpha Kit", "alpha.zip")
                },
                ServerKit::new("Delta Kit", "delta.zip"),
                ServerKit::new("Echo Kit", "echo.zip"),
            ],
        });
        s.index_fetched_at = Some(Instant::now());
        s.status = resonance_drums::download::Status::Downloading {
            name: "Delta Kit".into(),
            downloaded_bytes: 1_000_000,
            total_bytes: 4_000_000,
            bytes_per_sec: 500_000.0,
            eta_secs: Some(6.0),
            resumed_from: 0,
        };
        s.queued = vec!["Echo Kit".into()];
    }
    let plugin = activated();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.show_tab(true);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert_widget_fully_visible(&frame, "plok.Delta Kit.progress", "progress");
    assert_widget_fully_visible(&frame, "plok.Echo Kit.queued", "queued");
    let cancels = frame.texts.iter().filter(|t| t.text == "Cancel").count();
    assert_eq!(cancels, 2, "Cancel for the running and the queued download");
    assert_widget_fully_visible(&frame, "plok.Alpha Kit.load", "load");

    editor.click(widget_center(&frame, "plok.Alpha Kit.load"));
    assert!(
        lib.marks_of(&alpha).last_used.is_some(),
        "Load on the plok.org tab loaded nothing"
    );

    lib.download().state.lock().status =
        resonance_drums::download::Status::Error("connection reset".into());
    let frame = editor.frame(Vec::new());
    assert!(
        frame.shows("Error: connection reset"),
        "{:?}",
        frame.strings()
    );
}

/// The "Fetching…" line and the Refresh button's disabled state come from
/// `fetching_index`, not `Status::FetchingIndex`: an index fetch runs on
/// its own thread beside a download, which then owns `status`, so a
/// status-based check never fired while a download was active (ba
/// review).
#[test]
fn the_plok_tab_shows_fetching_alongside_a_running_download() {
    let home = Home::new("plok-fetching-alongside-download");
    let lib = home.library();
    {
        let mut s = lib.download().state.lock();
        s.index = Some(ServerIndex {
            drumkits: vec![ServerKit::new("Delta Kit", "delta.zip")],
        });
        s.index_fetched_at = Some(Instant::now());
        s.fetching_index = true;
        s.status = resonance_drums::download::Status::Downloading {
            name: "Delta Kit".into(),
            downloaded_bytes: 1_000_000,
            total_bytes: 4_000_000,
            bytes_per_sec: 500_000.0,
            eta_secs: Some(6.0),
            resumed_from: 0,
        };
    }
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.show_tab(true);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(
        frame.shows("Fetching the kit index from plok.org…"),
        "{:?}",
        frame.strings()
    );
    // The running download's own progress still shows through: the two
    // are independent.
    assert_widget_fully_visible(&frame, "plok.Delta Kit.progress", "progress");

    // Refresh is disabled while the fetch it would start is already
    // running: clicking it starts no second one.
    let fetches_before = lib.download().fetches_started();
    let refresh = frame.text_center("Refresh").expect("Refresh is drawn");
    editor.click(refresh);
    editor.frame(Vec::new());
    assert_eq!(
        lib.download().fetches_started(),
        fetches_before,
        "Refresh fired a fetch while one was already running"
    );
}

/// `index_error` alone drives "Could not reach plok.org" — not `status`:
/// an index fetch can fail while a download is running and owns `status`,
/// and the tab must still say so (ba review: the old check read
/// `Status::Error(_)`, which a concurrent download's own status hid this
/// behind).
#[test]
fn the_plok_tab_reads_the_index_error_field_not_status() {
    let home = Home::new("plok-index-error-field");
    let lib = home.library();
    {
        let mut s = lib.download().state.lock();
        s.index_error = Some("connection refused".into());
        s.status = resonance_drums::download::Status::Downloading {
            name: "Echo Kit".into(),
            downloaded_bytes: 10,
            total_bytes: 100,
            bytes_per_sec: 0.0,
            eta_secs: None,
            resumed_from: 0,
        };
    }
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.show_tab(true);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(
        frame.shows("Could not reach plok.org. Refresh to try again."),
        "{:?}",
        frame.strings()
    );
}

/// With no index and the server unreachable, the tab says so.
#[test]
fn the_plok_tab_says_when_plok_org_is_unreachable() {
    let home = Home::new("plok-offline");
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, home.library(), SIZE);
    opened(&mut editor);
    assert_eq!(editor.library_tab(), "plok.org");
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let frame = editor.frame(Vec::new());
        if frame.shows("Could not reach plok.org. Refresh to try again.") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no offline state: {:?}",
            frame.strings()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Review #5: a download this editor asked for that fails (or is
/// cancelled) is reported, and stops being "mine".
#[test]
fn a_failed_download_is_reported() {
    let home = Home::new("dl-fail");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);

    editor.add_my_download("Alpha Kit");
    lib.download().state.lock().status =
        resonance_drums::download::Status::Error("sha256 mismatch".into());
    let frame = editor.frame(Vec::new());
    assert_eq!(
        editor.notice().as_deref(),
        Some("could not download \"Alpha Kit\": sha256 mismatch")
    );
    assert!(editor.my_downloads().is_empty());
    assert_widget_fully_visible(&frame, "library.notice", "notice");

    editor.add_my_download("Alpha Kit");
    lib.download().state.lock().status =
        resonance_drums::download::Status::Cancelled("Alpha Kit".into());
    editor.frame(Vec::new());
    assert_eq!(
        editor.notice().as_deref(),
        Some("the download of \"Alpha Kit\" was cancelled")
    );
}

/// Review #6: a notice does not outlive the visit it belongs to.
#[test]
fn reopening_clears_the_last_notice() {
    let home = Home::new("notice");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.add_my_download("Alpha Kit");
    lib.download().state.lock().status = resonance_drums::download::Status::Error("x".repeat(400));
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(editor.notice().is_some());
    let panel = frame.widget("library.panel").unwrap().rect;
    let notice = frame
        .widget("library.notice")
        .expect("the notice is drawn")
        .rect;
    assert!(
        panel.expand(TOLERANCE).contains_rect(notice),
        "a long notice runs out of the panel: {notice:?}"
    );
    editor.frame(vec![key_ev(egui::Key::Escape)]);
    opened(&mut editor);
    assert_eq!(editor.notice(), None);
}

/// Review #7: a delete confirmed while a background job holds the library
/// waits for it (the confirm says why) instead of being dropped as
/// "busy"; a folder picked for import while a job runs is queued.
#[test]
fn delete_and_import_wait_for_a_running_job() {
    let home = Home::new("race");
    home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
    let outside = home.0.join("outside").join("Gamma");
    write_kit(&outside, "Gamma Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let alpha = lib.read().find("Alpha Kit").unwrap().clone();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();

    let release = editor.hold_job_slot();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.action.delete"));
    assert!(
        editor.pending_delete().is_some(),
        "Delete… must arm while a job runs"
    );
    let frame = editor.frame(Vec::new());
    assert!(
        frame.shows("waiting for scanning to finish"),
        "the confirm does not say why it waits: {:?}",
        frame.strings()
    );
    editor.click(frame.text_center("Delete").unwrap());
    assert!(
        editor.pending_delete().is_some(),
        "a blocked confirm was spent"
    );
    assert!(alpha.dir.exists());

    editor.picked_for_import(outside.clone());
    assert_eq!(
        editor.queued_actions(),
        1,
        "the picked import was not queued"
    );

    release.store(true, std::sync::atomic::Ordering::SeqCst);
    editor.finish_jobs();
    assert!(
        lib.read().find("Gamma Kit").is_some(),
        "the queued import never ran"
    );
    // The import selected its kit, which disarmed the delete: arm again.
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.action.delete"));
    let frame = editor.frame(Vec::new());
    editor.click(frame.text_center("Delete").unwrap());
    editor.finish_jobs();
    assert!(
        !alpha.dir.exists(),
        "the delete did not run once the job finished"
    );
}

/// Review #7: the editor's own delete does not wake the freshness poll
/// into a rescan of its own (which then held the job slot).
#[test]
fn the_editors_own_delete_does_not_trigger_a_rescan() {
    let home = Home::new("rebaseline");
    home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.action.delete"));
    let frame = editor.frame(Vec::new());
    editor.click(frame.text_center("Delete").unwrap());
    editor.finish_jobs();
    // Past the open overlay's 500 ms poll interval.
    std::thread::sleep(std::time::Duration::from_millis(700));
    editor.frame(Vec::new());
    assert_ne!(
        editor.job_running().as_deref(),
        Some("scanning…"),
        "the delete's own writes started a rescan"
    );
}

/// Review #8: a Rescan the user clicked that loses the library to another
/// writer says so.
#[test]
fn a_rescan_that_loses_the_library_says_so() {
    let home = Home::new("rescan-busy");
    home.kit("Alpha", "Alpha Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.finish_jobs();

    // Another writer (standing in for a download installing) holds it.
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let writer = {
        let lib = lib.clone();
        std::thread::spawn(move || {
            lib.mutate(|_| {
                held_tx.send(()).unwrap();
                let _ = go_rx.recv();
            })
        })
    };
    held_rx.recv().unwrap();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.rescan"));
    editor.finish_jobs();
    go_tx.send(()).unwrap();
    writer.join().unwrap();
    let notice = editor.notice().unwrap_or_default();
    assert!(
        notice.starts_with("rescan skipped: the library is busy"),
        "the lost rescan was not reported: {notice:?}"
    );
}

/// Review #10: the selected kit's (and the loaded kit's) sample files are
/// checked on a job; a missing file shows in the detail and the header.
#[test]
fn missing_sample_files_show_in_the_detail_and_the_header() {
    let home = Home::new("missing");
    let manifest = home.kit("Alpha", "Alpha Kit", 3);
    std::fs::remove_file(manifest.parent().unwrap().join("p1.wav")).unwrap();
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(manifest);
    let mut editor = TestEditor::new(&plugin, lib, SIZE);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("1 file missing"), "{:?}", frame.strings());
    opened(&mut editor);
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    assert!(
        frame.shows("1 sample file missing"),
        "{:?}",
        frame.strings()
    );
}

/// Review #11 (E6): samples the loader could not read are counted in the
/// header.
#[test]
fn unreadable_samples_show_in_the_header() {
    let home = Home::new("unreadable");
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_status.lock() = resonance_drums::kit_loader::KitStatus::Loaded {
        name: "Alpha".into(),
        num_pads: 30,
        unreadable: 3,
        unreadable_paths: vec![PathBuf::from("/kits/a.wav"), PathBuf::from("/kits/b.wav")],
    };
    let mut editor = TestEditor::new(&plugin, home.library(), SIZE);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("3 samples unreadable"), "{:?}", frame.strings());
    assert!(frame.widget("kit.unreadable").is_some());
}

/// Parity with the amp: a playing kit deleted from the library says
/// "(deleted)" in the header; the confirm says what projects will see; a
/// kit no instance plays has no "used in 0" line; the row after the
/// deleted one takes the selection.
#[test]
fn after_a_delete_the_header_and_selection_follow() {
    let home = Home::new("deleted");
    let manifest = home.kit("A", "Alpha Kit", 2);
    home.kit("B", "Beta Kit", 2);
    home.kit("C", "Gamma Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(manifest);
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    let names = editor.view_names();
    let pos = names.iter().position(|n| n == "Alpha Kit").unwrap();
    assert!(editor.select("Beta Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    assert!(
        !frame.strings().iter().any(|s| s.starts_with("used in 0")),
        "a kit nobody plays says \"used in 0\""
    );
    assert!(editor.select("Alpha Kit"));
    editor.finish_jobs();
    let frame = editor.frame(Vec::new());
    editor.click(widget_center(&frame, "library.action.delete"));
    let frame = editor.frame(Vec::new());
    assert!(
        frame
            .strings()
            .iter()
            .any(|s| s.contains("Projects that use it will show it as missing")),
        "{:?}",
        frame.strings()
    );
    editor.click(frame.text_center("Delete").unwrap());
    editor.finish_jobs();
    assert_eq!(
        editor.selected_name().as_deref(),
        Some(names[pos + 1].as_str()),
        "the next row did not take the selection"
    );

    editor.frame(vec![key_ev(egui::Key::Escape)]);
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("Alpha Kit (deleted)"), "{:?}", frame.strings());
}

/// Parity: importing one kit loads it.
#[test]
fn importing_a_kit_loads_it() {
    let home = Home::new("import-load");
    home.kit("Alpha", "Alpha Kit", 2);
    let outside = home.0.join("outside").join("Gamma");
    write_kit(&outside, "Gamma Kit", 2);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = activated();
    let mut editor = TestEditor::new(&plugin, lib.clone(), SIZE);
    opened(&mut editor);
    editor.picked_for_import(outside);
    editor.finish_jobs();
    let gamma = lib.read().find("Gamma Kit").expect("imported").id.clone();
    assert!(
        lib.marks_of(&gamma).last_used.is_some(),
        "the imported kit was not loaded: {:?}",
        editor.notice()
    );
    assert_eq!(editor.selected_name().as_deref(), Some("Gamma Kit"));
}

/// Parity: a list title too long for its column is elided with "…" (and
/// carries the full name on hover), not cut off mid-letter.
#[test]
fn a_long_title_is_elided_in_the_list() {
    let home = Home::new("elide");
    big_kit(&home, "Big", LONG_NAME);
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    let mut editor = TestEditor::new(&plugin, lib, (780.0, 520.0));
    let frame = opened(&mut editor);
    fn elided(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.text() == LONG_NAME && t.galley.elided,
            egui::Shape::Vec(v) => v.iter().any(elided),
            _ => false,
        }
    }
    assert!(
        frame.shapes.iter().any(|s| elided(&s.shape)),
        "the long title is not elided in the list"
    );
}
