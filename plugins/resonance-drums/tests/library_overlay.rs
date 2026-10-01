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

    // Esc disarms (and closes); reopening starts with nothing armed.
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
/// hash is in the library, `Update` when only the name is, `Download`
/// otherwise. A fresh index is not fetched again on opening the tab.
#[test]
fn the_plok_tab_shows_installed_update_and_download() {
    let home = Home::new("plok");
    home.kit("Alpha", "Alpha Kit", 2);
    home.kit("Beta", "Beta Kit", 2);
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
