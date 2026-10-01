//! The Download Kits overlay's modal mechanics (drums-plugin-rework.md
//! §1.2, §6.5, §9).
//!
//! The backdrop used to be painted on an `Order::Tooltip` layer, which in
//! egui draws *above* `Order::Foreground` — the panel's own layer — so
//! opening the overlay gave a near-black screen (§1.2). It also was not
//! modal: clicks fell through to the pads behind it, and Esc did nothing.
//! `download_panel.rs` is now an `egui::Modal`, which keeps the backdrop
//! and the panel on the same layer and makes the backdrop (and the panel
//! itself) sense clicks so they stop short of the pads.
//!
//! `editor_honesty.rs` covers the `Order::Tooltip` regression as a source
//! guard; this file proves the paint order and the Esc behaviour at
//! runtime, through the `test_run_download_panel_frame` and
//! `TestDownloadPanel` hooks (`editor/mod.rs` — `download_panel` is a
//! private module outside this crate).
//!
//! Nothing here reaches the network: the single-frame hook never sends
//! its worker a command, and every test that opens the panel the way the
//! header button does hands it a worker pointed at a closed loopback
//! port.
#![cfg(feature = "editor")]

use std::sync::Arc;

use plugin_gui_core::egui;
use resonance_drums::download::{self, Status, WorkerHandle};
use resonance_drums::{ResonanceDrums, TestDownloadPanel};
use resonance_plugin::ResonancePlugin;

fn shape_contains_text(shape: &egui::Shape, needle: &str) -> bool {
    match shape {
        egui::Shape::Text(t) => t.galley.text() == needle,
        egui::Shape::Vec(v) => v.iter().any(|s| shape_contains_text(s, needle)),
        _ => false,
    }
}

/// A black, partially-opaque fill covering (at least) the whole screen —
/// the backdrop, whatever its exact alpha. `download_panel::draw` asks
/// for a fixed alpha of 180, matching the amp's own overlay
/// (`resonance-amp/src/editor/library_panel.rs`), but `egui::Modal`
/// fades its backdrop in, so a freshly-opened panel's first settled
/// frame can report less than that — the shape to look for is "black,
/// not fully transparent, full-screen", not an exact colour.
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

fn escape_event() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The exact regression from §1.2: the dimming rect must come *before*
/// the panel's own content in the frame's shape list — shapes paint in
/// list order, so anything found earlier sits visually underneath
/// anything found later. A backdrop found after the header text would be
/// the bug coming back.
#[test]
fn the_backdrop_is_painted_below_the_panel_not_above_it() {
    let plugin = ResonanceDrums::new();
    let size = (960.0, 640.0);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let (_, shapes) = resonance_drums::test_run_download_panel_frame(&plugin, size, vec![], true);

    let backdrop = shapes
        .iter()
        .position(|s| shape_is_backdrop(&s.shape, screen))
        .expect("the overlay must dim the background while open");
    let header = shapes
        .iter()
        .position(|s| shape_contains_text(&s.shape, "DOWNLOAD KITS"))
        .expect("the panel header must be drawn while open");

    assert!(
        backdrop < header,
        "the backdrop (shape {backdrop}) must be painted before — and so sit below — the \
         panel's own content (shape {header}); it was found after it instead"
    );
}

#[test]
fn escape_closes_the_overlay() {
    let plugin = ResonanceDrums::new();
    let (open_after, _) = resonance_drums::test_run_download_panel_frame(
        &plugin,
        (960.0, 640.0),
        vec![escape_event()],
        true,
    );
    assert!(!open_after, "Esc should close the Download Kits overlay");
}

/// The negative case: without Esc, nothing closes the panel from under
/// the user.
#[test]
fn without_escape_the_overlay_stays_open() {
    let plugin = ResonanceDrums::new();
    let (open_after, _) =
        resonance_drums::test_run_download_panel_frame(&plugin, (960.0, 640.0), vec![], true);
    assert!(open_after, "the overlay closed on its own with no input");
}

/// A closed panel draws nothing — in particular, no leftover backdrop
/// darkening the editor behind it.
#[test]
fn a_closed_panel_draws_no_backdrop() {
    let plugin = ResonanceDrums::new();
    let size = (960.0, 640.0);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let (open_after, shapes) =
        resonance_drums::test_run_download_panel_frame(&plugin, size, vec![], false);
    assert!(!open_after);
    assert!(
        !shapes.iter().any(|s| shape_is_backdrop(&s.shape, screen)),
        "a closed overlay must not dim the editor behind it"
    );
}

/// A download worker pointed at a closed local port. Opening the panel
/// sends its worker a `FetchIndex`; this one fails fast on loopback
/// instead of reaching the real server.
fn local_worker() -> Arc<WorkerHandle> {
    Arc::new(download::spawn_with_index(
        "http://127.0.0.1:9/index.json".to_string(),
    ))
}

fn pointer_events(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

/// Press and release at `pos` over two frames.
fn click(session: &mut TestDownloadPanel, pos: egui::Pos2) {
    session.frame(pointer_events(pos, true));
    session.frame(pointer_events(pos, false));
}

/// Open the panel and let the modal settle.
fn settled_open(size: (f32, f32)) -> TestDownloadPanel {
    let mut session = TestDownloadPanel::new(local_worker(), size);
    session.open();
    session.frame(Vec::new());
    session.frame(Vec::new());
    session
}

fn text_center(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
    fn walk(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == needle => {
                Some(t.visual_bounding_rect().center())
            }
            egui::Shape::Vec(v) => v.iter().find_map(|s| walk(s, needle)),
            _ => None,
        }
    }
    shapes.iter().find_map(|s| walk(&s.shape, needle))
}

/// A click that misses the panel lands on the backdrop. That is far more
/// often a stray click than a request to dismiss a panel this size, so it
/// must not close it — `egui::Modal`'s own `should_close` would.
#[test]
fn a_backdrop_click_does_not_close_the_overlay() {
    let mut session = settled_open((960.0, 640.0));
    // Inside the window, outside the panel's 48 px margin.
    click(&mut session, egui::pos2(10.0, 10.0));
    assert!(session.is_open(), "a click on the backdrop closed the overlay");
}

/// The positive control for the test above: the same press/release
/// sequence on the Close button does close it, so the backdrop click
/// really was delivered.
#[test]
fn the_close_button_closes_the_overlay() {
    let mut session = settled_open((960.0, 640.0));
    let shapes = session.frame(Vec::new());
    let close = text_center(&shapes, "Close").expect("the Close button is drawn");
    click(&mut session, close);
    assert!(!session.is_open(), "clicking Close left the overlay open");
}

/// A delete armed ("Confirm?") and then walked away from must not still
/// be armed when the panel comes back: one click would then delete a kit.
#[test]
fn closing_the_overlay_disarms_a_pending_delete() {
    // Esc.
    let mut session = settled_open((960.0, 640.0));
    session.arm_delete("Drummica");
    session.frame(vec![escape_event()]);
    assert!(!session.is_open());
    assert_eq!(session.pending_delete(), None, "Esc left a delete armed");

    // The Close button.
    let mut session = settled_open((960.0, 640.0));
    session.arm_delete("Drummica");
    let shapes = session.frame(Vec::new());
    let close = text_center(&shapes, "Close").expect("the Close button is drawn");
    click(&mut session, close);
    assert!(!session.is_open());
    assert_eq!(session.pending_delete(), None, "Close left a delete armed");
}

/// However the panel was closed, opening it starts with nothing armed.
#[test]
fn opening_the_overlay_disarms_a_pending_delete() {
    let mut session = TestDownloadPanel::new(local_worker(), (960.0, 640.0));
    session.arm_delete("Drummica");
    session.open();
    assert_eq!(session.pending_delete(), None);
}

/// Opening the panel refetches the index — unless the worker is busy. A
/// fetch queued behind a running download would replace its "Download
/// complete" the moment it finished.
#[test]
fn opening_mid_download_does_not_queue_a_refetch() {
    let worker = local_worker();

    let mut session = TestDownloadPanel::new(worker.clone(), (960.0, 640.0));
    session.open();
    assert!(
        !session.did_initial_fetch(),
        "an idle worker should get a fresh index fetch on open"
    );

    worker.state.lock().status = Status::Downloading {
        name: "Drummica".to_string(),
        downloaded_bytes: 1,
        total_bytes: 2,
    };
    let mut session = TestDownloadPanel::new(worker, (960.0, 640.0));
    session.open();
    assert!(
        session.did_initial_fetch(),
        "opening mid-download queued an index fetch behind it"
    );
}
