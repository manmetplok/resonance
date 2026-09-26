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

use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::ComposeMessage;
use resonance_app::demo;
use resonance_app::message::{GroupMessage, MarkerMessage, Message, TrackMessage};
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

// ---------------------------------------------------------------------------
// Drum patterns
// ---------------------------------------------------------------------------

fn drum(msg: DrumGroupsMessage) -> Message {
    Message::Compose(ComposeMessage::DrumGroups(msg))
}

fn group_count(app: &Resonance) -> usize {
    app.compose_state()
        .drum_patterns
        .iter()
        .map(|p| p.groups.len())
        .sum()
}

#[test]
fn adding_a_drum_pattern_undoes_through_the_diff_path() {
    let mut f = fixture("drum-pattern");
    let before = f.app.test_snapshot_for_undo();
    let added = edit(&mut f, drum(DrumGroupsMessage::AddPattern));
    assert_eq!(
        added.project.file.drum_patterns.len(),
        before.project.file.drum_patterns.len() + 1,
        "the edit added a pattern"
    );
    // Undo removes the pattern the drum-roll focus still names (the diff
    // path leaves the focus alone), redo brings it back.
    undo_redo_over(&mut f, &before, &added, "drum pattern add");
    step_lands_on(&mut f, Message::Undo, &before, "undo drum pattern add again");

    // A stale focus is resolved, not trusted: a group add after the undo
    // lands in a pattern that exists.
    let groups = group_count(&f.app);
    let _ = f.app.update(drum(DrumGroupsMessage::AddGroup));
    assert_eq!(
        group_count(&f.app),
        groups + 1,
        "a group add after undoing the focused pattern's add lands in a live pattern"
    );
}

/// `DrumPatterns`' diff arm clears the bank when the target has none
/// (`clear_on_empty`). A-13c noted this was dead on the diff path while
/// the gate forced equal pattern sets; it is live now, and it is the rule
/// that keeps the fixed point (the full path instead keeps the live bank,
/// a disk-load rule for projects that predate drum patterns). No edit can
/// empty the bank (the last pattern refuses to delete), so the snapshot is
/// made by hand.
#[test]
fn a_diff_restore_to_an_empty_drum_bank_clears_it() {
    let mut f = fixture("drum-bank-empty");
    let mut target = f.app.test_snapshot_for_undo();
    assert!(
        !target.project.file.drum_patterns.is_empty(),
        "the demo seeds a drum bank, or this test is vacuous"
    );
    target.project.file.drum_patterns.clear();
    for d in &mut target.project.file.section_definitions {
        d.arrangement.clear();
    }
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(target.clone());
    assert!(
        !drain(&f.rx).iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an emptied bank takes the diff path"
    );
    assert!(f.app.compose_state().drum_patterns.is_empty());
    assert_eq!(f.app.compose_state().default_drum_pattern_id, None);
    assert_eq!(
        f.app.test_build_project_file().drum_patterns,
        target.project.file.drum_patterns
    );
}

/// A drum group add (inside a pattern — the bank's shape since epic #38).
/// Snapshots carry the legacy flat `drum_groups` list empty, so it can
/// never be what differs between two of them.
#[test]
fn adding_a_drum_group_undoes_through_the_diff_path() {
    let mut f = fixture("drum-group");
    let before = f.app.test_snapshot_for_undo();
    let groups = group_count(&f.app);
    let added = edit(&mut f, drum(DrumGroupsMessage::AddGroup));
    assert_eq!(group_count(&f.app), groups + 1, "the edit added a group");
    assert!(
        before.project.file.drum_groups.is_empty() && added.project.file.drum_groups.is_empty(),
        "snapshots write the legacy drum_groups list empty"
    );
    undo_redo_over(&mut f, &before, &added, "drum group add");
}

// ---------------------------------------------------------------------------
// Section placements
// ---------------------------------------------------------------------------

fn compose(msg: ComposeMessage) -> Message {
    Message::Compose(msg)
}

/// The first bar after every placement, where a section of the first
/// definition's length fits.
fn free_bar(app: &Resonance) -> (u64, u32) {
    let c = app.compose_state();
    let def = c.definitions.first().expect("the demo has a section");
    let end = c
        .placements
        .iter()
        .filter_map(|p| {
            c.definitions
                .iter()
                .find(|d| d.id == p.definition_id)
                .map(|d| p.start_bar + d.length_bars)
        })
        .max()
        .unwrap_or(0);
    (def.id, end + 1)
}

fn placement_ids(app: &Resonance) -> HashSet<u64> {
    app.compose_state().placements.iter().map(|p| p.id).collect()
}

/// A fresh placement has no derived clips (nothing is generated until the
/// user asks), so placing and removing one changes only the placement set.
#[test]
fn placing_and_removing_a_section_undoes_through_the_diff_path() {
    let mut f = fixture("placement");
    let before = f.app.test_snapshot_for_undo();
    let ids = placement_ids(&f.app);
    let (definition_id, start_bar) = free_bar(&f.app);

    let placed = edit(
        &mut f,
        compose(ComposeMessage::PlaceSection {
            definition_id,
            start_bar,
        }),
    );
    let placement_id = placement_ids(&f.app)
        .into_iter()
        .find(|id| !ids.contains(id))
        .expect("the edit placed a section");
    assert_eq!(
        placed.project.file.midi_clips.len(),
        before.project.file.midi_clips.len(),
        "a fresh placement generates no clips, or the gate still sees a clip change"
    );
    let removed = edit(
        &mut f,
        compose(ComposeMessage::DeleteSectionPlacement { placement_id }),
    );

    step_lands_on(&mut f, Message::Undo, &placed, "undo placement delete");
    undo_redo_over(&mut f, &before, &placed, "placement add");
    step_lands_on(&mut f, Message::Redo, &removed, "redo placement delete");
}

// ---------------------------------------------------------------------------
// Section definitions
// ---------------------------------------------------------------------------

fn definition_ids(app: &Resonance) -> HashSet<u64> {
    app.compose_state().definitions.iter().map(|d| d.id).collect()
}

/// A definition created unplaced (`section.create` without `place`), then
/// deleted: only the definition set changes.
#[test]
fn creating_and_deleting_a_section_undoes_through_the_diff_path() {
    let mut f = fixture("definition");
    let before = f.app.test_snapshot_for_undo();
    let ids = definition_ids(&f.app);

    let created = edit(
        &mut f,
        compose(ComposeMessage::CreateSection {
            name: "Bridge".into(),
            length_bars: 4,
            color: [10, 20, 30],
            place: false,
        }),
    );
    let definition_id = definition_ids(&f.app)
        .into_iter()
        .find(|id| !ids.contains(id))
        .expect("the edit created a section");
    let deleted = edit(
        &mut f,
        compose(ComposeMessage::DeleteSectionDefinition { definition_id }),
    );

    step_lands_on(&mut f, Message::Undo, &created, "undo section delete");
    undo_redo_over(&mut f, &before, &created, "section create");
    step_lands_on(&mut f, Message::Redo, &deleted, "redo section delete");
}

/// The GUI's create: a new definition and its placement in one edit.
#[test]
fn creating_a_placed_section_undoes_through_the_diff_path() {
    let mut f = fixture("definition-placed");
    let before = f.app.test_snapshot_for_undo();
    let created = edit(
        &mut f,
        compose(ComposeMessage::CreateSection {
            name: "Outro".into(),
            length_bars: 4,
            color: [30, 20, 10],
            place: true,
        }),
    );
    assert_eq!(
        (
            created.project.file.section_definitions.len(),
            created.project.file.section_placements.len(),
        ),
        (
            before.project.file.section_definitions.len() + 1,
            before.project.file.section_placements.len() + 1,
        ),
        "the edit created and placed a section"
    );
    undo_redo_over(&mut f, &before, &created, "placed section create");
}

// ---------------------------------------------------------------------------
// Group macro solo / mute: effective re-derivation on restore (FU-A13a)
// ---------------------------------------------------------------------------
//
// The group handlers (`update::group::toggle_macro_mute`/`toggle_macro_solo`)
// send each member's *effective* solo/mute — its own flag OR the group's
// macro — to the engine. Design doc §12 "found, not fixed": no restore ever
// re-derived that; the entity domain (`Tracks`) only ever restores a
// member's *own* flag, and only when it changed. So undoing a macro toggle
// (a scalar change, no track added/removed — always the diff path) used to
// leave the engine holding the stale effective value even though the GUI
// showed the toggle undone. These pin the fix: `TrackGroups`'s reconcile
// now re-sends every member's effective solo/mute wherever it may have
// changed, on both restore paths.

/// Group the first two top-level demo tracks and return `(group_id, a, b)`.
fn group_first_two_tracks(f: &mut Fixture) -> (u64, u64, u64) {
    let tracks: Vec<u64> = f
        .app
        .test_registry()
        .tracks
        .iter()
        .filter(|t| t.sub_track.is_none())
        .map(|t| t.id)
        .take(2)
        .collect();
    assert_eq!(tracks.len(), 2, "the demo has two top-level tracks to group");
    f.app.test_set_selected_tracks(tracks.clone());
    let _ = edit(f, Message::Group(GroupMessage::CreateGroupFromSelection));
    let group_id = f
        .app
        .test_track_groups()
        .get_all_groups()
        .into_iter()
        .map(|g| g.id)
        .next()
        .expect("the group was created");
    (group_id, tracks[0], tracks[1])
}

/// The last `SetTrackMute` this app sent for `track_id`, across `cmds`.
fn last_mute(cmds: &[AudioCommand], track_id: u64) -> Option<bool> {
    cmds.iter().rev().find_map(|c| match c {
        AudioCommand::SetTrackMute { track_id: t, muted } if *t == track_id => Some(*muted),
        _ => None,
    })
}

/// The last `SetTrackSolo` this app sent for `track_id`, across `cmds`.
fn last_solo(cmds: &[AudioCommand], track_id: u64) -> Option<bool> {
    cmds.iter().rev().find_map(|c| match c {
        AudioCommand::SetTrackSolo { track_id: t, soloed } if *t == track_id => Some(*soloed),
        _ => None,
    })
}

/// Like [`edit`], but returns the commands the app sent while applying
/// `msg` instead of discarding them — `edit` routes everything through
/// `echo_midi_clip_loads`, which drains and discards every non-clip-load
/// command, so it cannot be used where the test needs to see a
/// `SetTrackMute`/`SetTrackSolo`. None of the group-macro messages issue a
/// `LoadMidiClipDirect`, so there is nothing to echo here.
fn edit_capturing(f: &mut Fixture, msg: Message) -> Vec<AudioCommand> {
    let depth = f.app.test_undo_history().undo_len();
    let _ = drain(&f.rx);
    let _ = f.app.update(msg);
    let cmds = drain(&f.rx);
    assert_eq!(
        f.app.test_undo_history().undo_len(),
        depth + 1,
        "the edit must record one undo entry"
    );
    cmds
}

#[test]
fn undoing_a_group_macro_mute_toggle_resends_effective_member_mute() {
    let mut f = fixture("group-macro-mute");
    let (group_id, a, b) = group_first_two_tracks(&mut f);

    // Engage the macro mute: both members get the effective (true) mute.
    let cmds = edit_capturing(&mut f, Message::Group(GroupMessage::ToggleMacroMute(group_id)));
    for t in [a, b] {
        assert_eq!(
            last_mute(&cmds, t),
            Some(true),
            "engaging the macro mute must send member {t}'s effective mute"
        );
    }

    // Undo (diff path — no track or group was added/removed): both
    // members' effective mute must drop back to false.
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Undo);
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "undoing a macro toggle must take the diff path"
    );
    for t in [a, b] {
        assert_eq!(
            last_mute(&cmds, t),
            Some(false),
            "undoing the macro mute must resend member {t}'s effective mute (FU-A13a)"
        );
    }

    // Redo re-engages it.
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Redo);
    let cmds = drain(&f.rx);
    for t in [a, b] {
        assert_eq!(
            last_mute(&cmds, t),
            Some(true),
            "redoing the macro mute must resend member {t}'s effective mute"
        );
    }
}

#[test]
fn undoing_a_group_macro_solo_toggle_resends_effective_member_solo() {
    let mut f = fixture("group-macro-solo");
    let (group_id, a, b) = group_first_two_tracks(&mut f);

    let cmds = edit_capturing(&mut f, Message::Group(GroupMessage::ToggleMacroSolo(group_id)));
    for t in [a, b] {
        assert_eq!(
            last_solo(&cmds, t),
            Some(true),
            "engaging the macro solo must send member {t}'s effective solo"
        );
    }

    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Undo);
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "undoing a macro toggle must take the diff path"
    );
    for t in [a, b] {
        assert_eq!(
            last_solo(&cmds, t),
            Some(false),
            "undoing the macro solo must resend member {t}'s effective solo (FU-A13a)"
        );
    }
}

/// A member's own mute changes while the group macro mute holds: the
/// engine's effective mute must stay `true` throughout — including after
/// undoing the member's own toggle, which the `Tracks` entity domain alone
/// would resend as the member's bare (now `false`) own flag.
#[test]
fn undoing_a_member_mute_while_group_macro_holds_keeps_effective_mute() {
    let mut f = fixture("group-macro-member-mute");
    let (group_id, a, _b) = group_first_two_tracks(&mut f);
    let _ = edit_capturing(&mut f, Message::Group(GroupMessage::ToggleMacroMute(group_id)));

    // The member mutes itself too — effective mute was already true, and
    // stays true.
    let cmds = edit_capturing(&mut f, Message::Track(TrackMessage::ToggleMute(a)));
    assert_eq!(
        last_mute(&cmds, a),
        Some(true),
        "the member's own mute composes with the still-active macro mute"
    );

    // Undo the member's own toggle (diff path): `Tracks` alone would send
    // the member's bare own flag (false); the effective mute — still held
    // up by the group macro — must be what actually reaches the engine.
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Undo);
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "undoing a member's own mute must take the diff path"
    );
    assert_eq!(
        last_mute(&cmds, a),
        Some(true),
        "undoing the member's own mute must not clear the still-active group's effective mute (FU-A13a)"
    );
}

/// A saved project with an engaged group macro mute (a disk load, or an
/// undo's structural `ClearAll` fallback — both share `Tracks`' full-replay
/// arm, which sends only each track's bare own flag): loading it must still
/// bring the engine up on the *effective* mute, not the bare saved flag.
#[test]
fn loading_a_saved_group_macro_mute_sends_effective_member_mute() {
    use resonance_app::project::ProjectTrack;
    use resonance_app::state::{InstrumentIcon, InstrumentType};
    use resonance_common::group_identity::GroupIdentityColor;
    use resonance_common::track_group::TrackGroup;

    fn track(id: u64) -> ProjectTrack {
        ProjectTrack {
            id,
            name: format!("T{id}"),
            order: id as usize,
            volume: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            fx_bypassed: false,
            record_armed: false,
            monitor_enabled: false,
            playback_source: resonance_common::PlaybackSource::Live,
            mono: true,
            input_device_name: None,
            input_port_index: Some(0),
            plugins: Vec::new(),
            track_type: "audio".to_string(),
            output_bus: None,
            instrument_type: InstrumentType::default(),
            instrument_icon: InstrumentIcon::default(),
            role: None,
            sub_track: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: None,
            midi_output_channel: None,
            freeze: resonance_common::TrackFreezeState::unfrozen(),
            external_instrument: None,
        }
    }

    let mut group = TrackGroup::new(100, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![10, 11];
    group.macro_mute = true;

    let file = ProjectFile {
        tracks: vec![track(10), track(11)],
        track_groups: vec![group],
        ..ProjectFile::default()
    };

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_replay_loaded_project(file);
    let cmds = drain(&rx);
    for t in [10u64, 11u64] {
        assert_eq!(
            last_mute(&cmds, t),
            Some(true),
            "loading a project with an engaged group macro mute must bring up member {t} muted"
        );
    }
}
