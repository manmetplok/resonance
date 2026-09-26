//! The arrange scroll position on load and on an undo replay (code review
//! FU-V3a).
//!
//! Every replay reset `viewport.scroll_offset = 0` without touching the
//! outer `Scrollable`, so state claimed "at the start" while the view sat
//! wherever it was until its next scroll report — and follow, culling and
//! drop placement all read the wrong offset meanwhile. The same reset also
//! ran on a slow-path undo, which has no business moving the view. Now a
//! disk load issues a real `scroll_to(0)` (state and widget agree), and an
//! undo replay leaves the scroll alone.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::message::{Message, ProjectIoMessage, ViewportMessage};
use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;

fn app_scrolled_to(offset_x: f32) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let _rx = app.test_capture_engine();
    app.test_set_active_project(true);
    app.test_dispatch(Message::Viewport(ViewportMessage::ArrangeScrolled {
        offset_x,
        visible_width: 1000.0,
        content_width: 20_000.0,
    }));
    app
}

#[test]
fn a_disk_load_scrolls_the_view_to_the_start() {
    let mut app = app_scrolled_to(3000.0);
    let loaded = LoadedProject {
        file: ProjectFile::default(),
        project_dir: PathBuf::from("/tmp/load-scroll-reset.rproj"),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    };
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(Box::new(
        loaded,
    )))));
    let task = app.test_engine_event_task(AudioEvent::AllCleared);
    assert!(task.units() > 0, "the load must scroll the real Scrollable");
    assert_eq!(app.test_arrange_scroll_x(), 0.0);
}

#[test]
fn an_undo_replay_leaves_the_scroll_alone() {
    let mut app = app_scrolled_to(3000.0);
    app.test_replay_loaded_project(ProjectFile::default());
    assert_eq!(app.test_arrange_scroll_x(), 3000.0);
}
