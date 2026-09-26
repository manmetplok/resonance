//! Undo/redo across an add or remove of an app-side entity takes the diff
//! path and lands exactly on the snapshot (ARCH-01 A-13g).
//!
//! `structurally_compatible` used to send any change in the id sets of
//! section definitions and placements, drum patterns, track groups and
//! arrangement markers down the `ClearAll` fallback, which re-instantiates
//! every plugin. Their domains restore them whole on both paths (since
//! A-13a / A-13c), so the gate no longer looks at them. Each test here
//! makes one such edit through the real message path on the demo project,
//! then walks undo and redo over it and asserts, after every step:
//!
//! * no `ClearAll` went out, and every domain ran under `Origin::UndoDiff`
//!   (the reconcile trace) — the diff path was taken;
//! * `build_project_file` equals the target snapshot's file, and the whole
//!   snapshot (notes included) is `same_state` — the fixed point.

use std::collections::HashSet;
use std::path::PathBuf;

use resonance_app::demo;
use resonance_app::message::{GroupMessage, MarkerMessage, Message};
use resonance_app::project::ProjectFile;
use resonance_app::undo::UndoSnapshot;
use resonance_app::update::project_io::reconcile::{domain_order, Origin};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent};

struct Fixture {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The demo project, its MIDI clip loads echoed, with an active saved
/// project so edits record undo entries.
fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "resonance-undo-diff-shape-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("fixture.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("create project dir");

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    demo::seed_demo_content(&mut app);
    echo_midi_clip_loads(&mut app, &rx);
    app.test_set_active_project(true);
    app.test_set_project_path(project);
    Fixture { app, rx, root }
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Answer every `LoadMidiClipDirect` with its `MidiClipCreated` echo, as
/// the live engine does.
fn echo_midi_clip_loads(app: &mut Resonance, rx: &Receiver<AudioCommand>) {
    for cmd in drain(rx) {
        if let AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } = cmd
        {
            app.test_apply_engine_event(AudioEvent::MidiClipCreated {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                name,
                notes,
                trim_start_ticks,
                trim_end_ticks,
            });
        }
    }
}

/// Apply a recorded edit and return the snapshot of the state it left.
fn edit(f: &mut Fixture, msg: Message) -> UndoSnapshot {
    let depth = f.app.test_undo_history().undo_len();
    let _ = f.app.update(msg);
    echo_midi_clip_loads(&mut f.app, &f.rx);
    assert_eq!(
        f.app.test_undo_history().undo_len(),
        depth + 1,
        "the edit must record one undo entry"
    );
    f.app.test_snapshot_for_undo()
}

fn pretty(file: &ProjectFile) -> String {
    serde_json::to_string_pretty(file).expect("ProjectFile serializes")
}

/// Run `Undo` / `Redo` and assert it took the diff path and landed on
/// `target` exactly.
fn step_lands_on(f: &mut Fixture, msg: Message, target: &UndoSnapshot, what: &str) {
    let _ = drain(&f.rx);
    let _ = f.app.update(msg);
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "{what}: must take the diff path, not ClearAll"
    );
    let trace = f.app.test_reconcile_trace();
    assert_eq!(
        trace.len(),
        domain_order().len(),
        "{what}: every domain must have run"
    );
    assert!(
        trace.iter().all(|(o, _)| *o == Origin::UndoDiff),
        "{what}: every domain must run under UndoDiff: {trace:?}"
    );
    let restored = f.app.test_build_project_file();
    if restored != target.project.file {
        let (a, b) = (pretty(&restored), pretty(&target.project.file));
        let first = a
            .lines()
            .zip(b.lines())
            .position(|(x, y)| x != y)
            .unwrap_or(0);
        let ctx = |s: &str| {
            s.lines()
                .skip(first.saturating_sub(4))
                .take(10)
                .collect::<Vec<_>>()
                .join("\n")
        };
        panic!(
            "{what}: restore != snapshot, first difference at line {}\n--- restored:\n{}\n--- snapshot:\n{}",
            first + 1,
            ctx(&a),
            ctx(&b)
        );
    }
    let after = f.app.test_snapshot_for_undo();
    assert!(
        Resonance::test_snapshot_same_state(&after, target),
        "{what}: the file matches but the snapshot does not (notes?)"
    );
}

/// `before` → `edit` → `after`; then undo lands on `before`, redo on
/// `after`, both through the diff path.
fn undo_redo_over(f: &mut Fixture, before: &UndoSnapshot, after: &UndoSnapshot, what: &str) {
    assert!(
        !Resonance::test_snapshot_same_state(before, after),
        "{what}: the edit must change the snapshot, or the test is vacuous"
    );
    step_lands_on(f, Message::Undo, before, &format!("undo {what}"));
    step_lands_on(f, Message::Redo, after, &format!("redo {what}"));
}

// ---------------------------------------------------------------------------
// Arrangement markers
// ---------------------------------------------------------------------------

#[test]
fn adding_and_removing_a_marker_undoes_through_the_diff_path() {
    let mut f = fixture("marker");
    let before = f.app.test_snapshot_for_undo();
    let ids: HashSet<u64> = f.app.test_markers().markers.iter().map(|m| m.id).collect();

    let added = edit(&mut f, Message::Marker(MarkerMessage::AddAtPlayhead));
    let new_id = f
        .app
        .test_markers()
        .markers
        .iter()
        .map(|m| m.id)
        .find(|id| !ids.contains(id))
        .expect("the add landed a marker");
    let removed = edit(&mut f, Message::Marker(MarkerMessage::Delete(new_id)));

    // Undo the remove (the marker comes back), then the add (it goes).
    step_lands_on(&mut f, Message::Undo, &added, "undo marker delete");
    undo_redo_over(&mut f, &before, &added, "marker add");
    step_lands_on(&mut f, Message::Redo, &removed, "redo marker delete");
}

// ---------------------------------------------------------------------------
// Track groups
// ---------------------------------------------------------------------------

#[test]
fn creating_a_track_group_undoes_through_the_diff_path() {
    let mut f = fixture("track-group");
    let tracks: Vec<_> = f
        .app
        .test_registry()
        .tracks
        .iter()
        .filter(|t| t.sub_track.is_none())
        .map(|t| t.id)
        .take(2)
        .collect();
    assert_eq!(tracks.len(), 2, "the demo has two top-level tracks to group");
    let before = f.app.test_snapshot_for_undo();
    f.app.test_set_selected_tracks(tracks);
    let added = edit(&mut f, Message::Group(GroupMessage::CreateGroupFromSelection));
    assert_eq!(
        added.project.file.track_groups.len(),
        before.project.file.track_groups.len() + 1,
        "the edit created a group"
    );
    // Undo removes the group, redo brings it back.
    undo_redo_over(&mut f, &before, &added, "track group create");
}
