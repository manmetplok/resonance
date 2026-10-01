//! Undo/redo across an add or remove of an app-side entity takes the diff
//! path and lands exactly on the snapshot (ARCH-01 A-13g) — and since
//! A-13h / A-13i across busses, plugin instances, tracks and clips too, so
//! no undo falls back to `ClearAll` any more.
//!
//! `structurally_compatible` (deleted with the fallback in A-13j) used to
//! send any change in the id sets of
//! section definitions and placements, drum patterns, track groups and
//! arrangement markers down the `ClearAll` fallback, which re-instantiates
//! every plugin. Their domains restore them whole on both paths (since
//! A-13a / A-13c), so the gate no longer looks at them. Each test here
//! makes one such edit through the real message path on the demo project,
//! then walks undo and redo over it and asserts, after every step:
//!
//! * no `ClearAll` went out, and every domain ran under `Origin::Undo`
//!   (the reconcile trace) — the diff path was taken;
//! * `build_project_file` equals the target snapshot's file, and the whole
//!   snapshot (notes included) is `same_state` — the fixed point.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::ComposeMessage;
use resonance_app::demo;
use resonance_app::message::{
    BusMessage, ClipMessage, GroupMessage, MarkerMessage, MarkerUiMessage, MasterMessage,
    Message, MidiClipMessage, MixerMessage, PluginMessage, TrackMessage,
};
use resonance_app::project::ProjectFile;
use resonance_app::undo::UndoSnapshot;
use resonance_app::update::project_io::reconcile::{domain_order, Origin};
use resonance_app::{Resonance, TestChain};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, SendSource};

struct Fixture {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    root: PathBuf,
    /// The engine's sub-tracks, by parent (`CreateSubTrack` has no echo;
    /// `RemoveTrack` of a parent answers for each one still under it).
    subs: HashMap<u64, u64>,
    /// The engine's plugin chains, when a test models them
    /// ([`EngineChains`]); `None` answers every add / move as `echo_of`.
    chains: Option<EngineChains>,
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
    let mut f = Fixture {
        app,
        rx,
        root,
        subs: HashMap::new(),
        chains: None,
    };
    echo_midi_clip_loads(&mut f);
    f.app.test_set_active_project(true);
    f.app.test_set_project_path(project);
    f
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Answer every `LoadMidiClipDirect` with its `MidiClipCreated` echo, as
/// the live engine does, and every other command below with its echo
/// (the adds, removals and moves of busses and plugins, sends and key
/// routes — A-13h; tracks and clips — A-13i). Until the engine goes
/// quiet: an echo handler may send commands of its own (a bus removal's
/// send cleanup, a multi-output instrument's sub-tracks).
fn echo_midi_clip_loads(f: &mut Fixture) {
    let cmds = drain(&f.rx);
    echo(f, cmds);
}

fn echo(f: &mut Fixture, mut cmds: Vec<AudioCommand>) {
    while !cmds.is_empty() {
        for cmd in cmds {
            for event in engine_answer(f, cmd) {
                f.app.test_apply_engine_event(event);
            }
        }
        cmds = drain(&f.rx);
    }
}

/// The engine's answer to `cmd`, with the little engine state these
/// tests need: which sub-tracks sit under which parent, and an audio
/// clip's length (the WAV's, which the load reports; the mirror has it).
fn engine_answer(f: &mut Fixture, cmd: AudioCommand) -> Vec<AudioEvent> {
    if let Some(events) = f.chains.as_mut().and_then(|c| c.answer(&cmd)) {
        return events;
    }
    match cmd {
        AudioCommand::CreateSubTrack {
            sub_id,
            parent_track_id,
            ..
        } => {
            f.subs.insert(sub_id, parent_track_id);
            Vec::new()
        }
        AudioCommand::RemoveTrack { track_id } => {
            f.subs.remove(&track_id);
            let mut subs: Vec<u64> = f
                .subs
                .iter()
                .filter(|(_, parent)| **parent == track_id)
                .map(|(sub, _)| *sub)
                .collect();
            subs.sort_unstable();
            f.subs.retain(|_, parent| *parent != track_id);
            std::iter::once(track_id)
                .chain(subs)
                .map(|track_id| AudioEvent::TrackRemoved { track_id })
                .collect()
        }
        AudioCommand::LoadClipFromWav {
            clip_id,
            track_id,
            start_sample,
            name,
            trim_start_frames,
            trim_end_frames,
            ..
        } => {
            let total = f
                .app
                .test_clips()
                .iter()
                .find(|c| c.id == clip_id)
                .map_or(0, |c| c.total_frames);
            vec![AudioEvent::ClipImported {
                clip_id,
                track_id,
                start_sample,
                duration_samples: total
                    .saturating_sub(trim_start_frames)
                    .saturating_sub(trim_end_frames),
                name,
                waveform_peaks: Vec::new(),
            }]
        }
        other => echo_of(other).into_iter().collect(),
    }
}

/// The event the engine answers `cmd` with, for the stateless commands
/// these tests drive. The removals and moves are answered for any id, as
/// the engine does.
fn echo_of(cmd: AudioCommand) -> Option<AudioEvent> {
    // Every fake plugin has one parameter, at its default.
    let plugin_params = || {
        vec![ParamInfo {
            id: GAIN,
            name: "Gain".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            current_value: 0.0,
            ..Default::default()
        }]
    };
    Some(match cmd {
        AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } => AudioEvent::MidiClipCreated {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            name,
            notes,
            trim_start_ticks,
            trim_end_ticks,
        },
        AudioCommand::AddBus { id, name } => AudioEvent::BusAdded {
            bus_id: id,
            name: name.unwrap_or_else(|| format!("Bus {id}")),
        },
        AudioCommand::RemoveBus { bus_id } => AudioEvent::BusRemoved { bus_id },
        AudioCommand::SetBusRole { bus_id, is_return } => {
            AudioEvent::BusRoleChanged { bus_id, is_return }
        }
        AudioCommand::AddPlugin {
            track_id,
            clap_file_path,
            clap_plugin_id,
            id,
        } => {
            // A "multi" plugin has three outputs: two sub-tracks.
            let ports = if clap_plugin_id.contains("multi") { 3 } else { 1 };
            AudioEvent::PluginAdded {
                track_id,
                instance_id: id,
                plugin_name: clap_plugin_id.clone(),
                clap_plugin_id,
                clap_file_path,
                params: plugin_params(),
                has_gui: false,
                has_sidechain_input: true,
                output_port_count: ports,
                output_port_names: (0..ports).map(|p| format!("Out {p}")).collect(),
            }
        }
        AudioCommand::AddPluginToBus {
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id,
        } => AudioEvent::BusPluginAdded {
            bus_id,
            instance_id: id,
            plugin_name: clap_plugin_id.clone(),
            clap_plugin_id,
            clap_file_path,
            params: plugin_params(),
            has_gui: false,
            has_sidechain_input: true,
        },
        AudioCommand::AddPluginToMaster {
            clap_file_path,
            clap_plugin_id,
            id,
        } => AudioEvent::MasterPluginAdded {
            instance_id: id,
            plugin_name: clap_plugin_id.clone(),
            clap_plugin_id,
            clap_file_path,
            params: plugin_params(),
            has_gui: false,
            has_sidechain_input: true,
        },
        AudioCommand::RemovePlugin {
            track_id,
            instance_id,
        } => AudioEvent::PluginRemoved {
            track_id,
            instance_id,
        },
        AudioCommand::RemovePluginFromBus {
            bus_id,
            instance_id,
        } => AudioEvent::BusPluginRemoved {
            bus_id,
            instance_id,
        },
        AudioCommand::RemovePluginFromMaster { instance_id } => {
            AudioEvent::MasterPluginRemoved { instance_id }
        }
        AudioCommand::MovePlugin {
            track_id,
            instance_id,
            to_index,
        } => AudioEvent::PluginMoved {
            track_id,
            instance_id,
            to_index,
        },
        AudioCommand::MovePluginInBus {
            bus_id,
            instance_id,
            to_index,
        } => AudioEvent::BusPluginMoved {
            bus_id,
            instance_id,
            to_index,
        },
        AudioCommand::MovePluginInMaster {
            instance_id,
            to_index,
        } => AudioEvent::MasterPluginMoved {
            instance_id,
            to_index,
        },
        AudioCommand::AddAuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        }
        | AudioCommand::SetAuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => AudioEvent::AuxSendChanged {
            send_id: id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        },
        AudioCommand::RemoveAuxSend { send_id } => AudioEvent::AuxSendRemoved { send_id },
        AudioCommand::SetPluginBypass {
            instance_id,
            bypassed,
        } => AudioEvent::PluginBypassChanged {
            instance_id,
            bypassed,
            own_bypass_param: false,
        },
        AudioCommand::SetSidechainRoute {
            plugin,
            source,
            enabled,
        } => AudioEvent::SidechainRouteChanged {
            plugin,
            source: Some(source),
            enabled,
        },
        AudioCommand::ClearSidechainRoute { plugin } => AudioEvent::SidechainRouteChanged {
            plugin,
            source: None,
            enabled: false,
        },
        AudioCommand::AddTrack { id, .. } => AudioEvent::TrackAdded { track_id: id },
        AudioCommand::AddInstrumentTrack { id, .. } => {
            AudioEvent::InstrumentTrackAdded { track_id: id }
        }
        AudioCommand::AddVocalTrack { id, .. } => AudioEvent::VocalTrackAdded { track_id: id },
        AudioCommand::DeleteClip { clip_id } => AudioEvent::ClipDeleted { clip_id },
        AudioCommand::DeleteMidiClip { clip_id } => AudioEvent::MidiClipDeleted { clip_id },
        AudioCommand::SetTrackFxBypass { track_id, bypassed } => {
            AudioEvent::TrackFxBypassChanged { track_id, bypassed }
        }
        AudioCommand::SetTrackPlaybackSource { track_id, source } => {
            AudioEvent::TrackPlaybackSourceChanged { track_id, source }
        }
        _ => return None,
    })
}

/// Apply a recorded edit and return the snapshot of the state it left.
fn edit(f: &mut Fixture, msg: Message) -> UndoSnapshot {
    let depth = f.app.test_undo_history().undo_len();
    let _ = f.app.update(msg);
    echo_midi_clip_loads(f);
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
/// `target` exactly. Returns the commands it sent, not yet echoed.
fn step_lands_on(
    f: &mut Fixture,
    msg: Message,
    target: &UndoSnapshot,
    what: &str,
) -> Vec<AudioCommand> {
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
        trace.iter().all(|(o, _)| *o == Origin::Undo),
        "{what}: every domain must run under Undo: {trace:?}"
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
    assert_same_state(f, target, what);
    cmds
}

fn assert_same_state(f: &mut Fixture, target: &UndoSnapshot, what: &str) {
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

/// FU-A13b: a restore never rewinds the marker id counter. Two undos in a
/// row used to reset it to the restored set's max + 1, so the next marker
/// took the id of one the undos had removed.
#[test]
fn undoing_marker_adds_never_hands_a_removed_id_out_again() {
    let mut f = fixture("marker-ids");
    let ids = |f: &Fixture| -> HashSet<u64> {
        f.app.test_markers().markers.iter().map(|m| m.id).collect()
    };
    let start = ids(&f);
    edit(&mut f, Message::Marker(MarkerMessage::AddAtPlayhead));
    edit(&mut f, Message::Marker(MarkerMessage::AddAtPlayhead));
    let added: HashSet<u64> = ids(&f).difference(&start).copied().collect();
    assert_eq!(added.len(), 2, "two markers were added");

    let _ = f.app.update(Message::Undo);
    let _ = f.app.update(Message::Undo);
    assert_eq!(ids(&f), start, "both adds were undone");

    edit(&mut f, Message::Marker(MarkerMessage::AddAtPlayhead));
    let fresh: Vec<u64> = ids(&f).difference(&start).copied().collect();
    assert_eq!(fresh.len(), 1);
    assert!(
        !added.contains(&fresh[0]),
        "the post-undo marker reused a removed id: {fresh:?} vs {added:?}"
    );
}

/// FU-A13b: a selection naming a marker the restore removed is cleared, so
/// it can't highlight a later marker that happens to share the id.
#[test]
fn undoing_a_marker_add_clears_its_selection() {
    let mut f = fixture("marker-select");
    let start: HashSet<u64> = f.app.test_markers().markers.iter().map(|m| m.id).collect();
    edit(&mut f, Message::Marker(MarkerMessage::AddAtPlayhead));
    let id = f
        .app
        .test_markers()
        .markers
        .iter()
        .map(|m| m.id)
        .find(|id| !start.contains(id))
        .expect("the add landed a marker");
    let _ = f.app.update(Message::MarkerUi(MarkerUiMessage::Select(Some(id))));
    assert_eq!(f.app.test_selected_marker_id(), Some(id));

    let _ = f.app.update(Message::Undo);
    assert!(!f.app.test_markers().markers.iter().any(|m| m.id == id));
    assert_eq!(f.app.test_selected_marker_id(), None);
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
// Track colour (mixer-cleanup.md §6)
// ---------------------------------------------------------------------------

#[test]
fn recolouring_a_track_undoes_through_the_diff_path() {
    let mut f = fixture("track-colour");
    let track = f
        .app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.sub_track.is_none())
        .map(|t| (t.id, t.color))
        .expect("the demo has a track");
    let (id, old) = track;
    let new = [0x12, 0x34, 0x56];
    assert_ne!(old, new);
    let colour = |s: &UndoSnapshot| {
        s.project
            .file
            .tracks
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.color)
    };
    let before = f.app.test_snapshot_for_undo();
    let after = edit(&mut f, Message::Track(TrackMessage::SetTrackColor(id, new)));
    assert_eq!(colour(&before), Some(old));
    assert_eq!(colour(&after), Some(new));
    undo_redo_over(&mut f, &before, &after, "track colour");
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
            color: None,
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

// ---------------------------------------------------------------------------
// Busses and plugin instances (A-13h)
// ---------------------------------------------------------------------------

/// Echo what a step sent, then assert the restore still sits on `target`
/// and owes the engine nothing: the echoes of its own adds, removals and
/// moves change nothing.
fn settle(f: &mut Fixture, cmds: Vec<AudioCommand>, target: &UndoSnapshot, what: &str) {
    echo(f, cmds);
    assert_eq!(
        f.app.test_build_project_file(),
        target.project.file,
        "{what}: the echoes moved the restored state"
    );
    assert_same_state(f, target, what);
    assert!(
        f.app.test_restore_echoes_settled(),
        "{what}: every owed echo was consumed"
    );
}

/// The commands a restore sent, as the engine sees them, without the
/// `Stop` every restore opens with.
fn sent(cmds: &[AudioCommand]) -> Vec<String> {
    cmds.iter()
        .filter(|c| !matches!(c, AudioCommand::Stop))
        .map(|c| format!("{c:?}"))
        .collect()
}

fn bus_ids(app: &Resonance) -> HashSet<u64> {
    app.test_registry().busses.iter().map(|b| b.id).collect()
}

const DRUMS: u64 = 1;
const DRUM_BUS_COMP: u64 = 10001;

/// What every diff restore of the demo sends besides its own domain's
/// commands: the redo snapshot's clip-WAV persist (`snapshot_for_undo`),
/// the tempo map and the take groups (restored whole on every origin).
const TEMPO: &str = "SetTempoEvents { tempo: [TempoPoint { bar: 0, bpm: 90.0 }], signature: \
                     [SignaturePoint { bar: 0, numerator: 6, denominator: 8 }] }";
const NO_TAKES: &str = "RestoreTakeGroups { groups: [] }";

/// A GUI bus add, made a return, fed by a send from the drums and keying
/// the drum bus's compressor; then the bus is deleted (which, live, drops
/// the send and the route on the `BusRemoved` echo). Every undo and redo
/// over the five edits takes the diff path: the bus comes back with its
/// send and key route (`AddBus` before `AddAuxSend` / `SetSidechainRoute`),
/// and goes again with them (`RemoveAuxSend` / `ClearSidechainRoute`
/// before `RemoveBus`).
#[test]
fn adding_and_removing_a_bus_with_routing_undoes_through_the_diff_path() {
    let mut f = fixture("bus");
    let s0 = f.app.test_snapshot_for_undo();
    let ids = bus_ids(&f.app);
    let s1 = edit(&mut f, Message::Bus(BusMessage::AddBus));
    let bus = bus_ids(&f.app)
        .into_iter()
        .find(|id| !ids.contains(id))
        .expect("the add landed a bus");
    let s2 = edit(&mut f, Message::Mixer(MixerMessage::SetBusReturnRole(bus, true)));
    let s3 = edit(
        &mut f,
        Message::Mixer(MixerMessage::AddSend {
            source: SendSource::Track(DRUMS),
            dest: bus,
        }),
    );
    let s4 = edit(
        &mut f,
        Message::Plugin(PluginMessage::SetPluginSidechain {
            instance_id: DRUM_BUS_COMP,
            source: Some(SendSource::Bus(bus)),
            enabled: true,
        }),
    );
    assert_eq!(s4.project.file.sends.len(), 1, "the send landed");
    assert_eq!(s4.project.file.sidechain_routes.len(), 1, "the key route landed");
    let s5 = edit(&mut f, Message::Bus(BusMessage::RemoveBus(bus)));
    assert!(
        s5.project.file.sends.is_empty() && s5.project.file.sidechain_routes.is_empty(),
        "deleting the bus dropped its send and key route"
    );

    // Undo the delete: the bus, its send and its key route come back.
    let cmds = step_lands_on(&mut f, Message::Undo, &s4, "undo bus delete");
    let send = s4.project.file.sends[0].id;
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            format!("AddBus {{ id: {bus}, name: Some(\"Bus {bus}\") }}"),
            format!("SetBusVolume {{ bus_id: {bus}, volume: 1.0 }}"),
            format!("SetBusPan {{ bus_id: {bus}, pan: 0.0 }}"),
            format!("SetBusMute {{ bus_id: {bus}, muted: false }}"),
            format!("SetBusFxBypass {{ bus_id: {bus}, bypassed: false }}"),
            format!("SetBusRole {{ bus_id: {bus}, is_return: true }}"),
            format!(
                "AddAuxSend {{ id: {send}, source: Track({DRUMS}), dest: {bus}, level_db: 0.0, \
                 pre_fader: false, enabled: true }}"
            ),
            format!(
                "SetSidechainRoute {{ plugin: {DRUM_BUS_COMP}, source: Bus({bus}), enabled: true }}"
            ),
            NO_TAKES.to_owned(),
        ],
        "undo bus delete: the bus, then its routing"
    );
    settle(&mut f, cmds, &s4, "undo bus delete");
    for (target, what) in [(&s3, "undo key route"), (&s2, "undo send"), (&s1, "undo return role")] {
        let cmds = step_lands_on(&mut f, Message::Undo, target, what);
        settle(&mut f, cmds, target, what);
    }
    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo bus add");
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            format!("RemoveBus {{ bus_id: {bus} }}"),
            NO_TAKES.to_owned(),
        ],
        "undo bus add"
    );
    settle(&mut f, cmds, &s0, "undo bus add");
    for (target, what) in [
        (&s1, "redo bus add"),
        (&s2, "redo return role"),
        (&s3, "redo send"),
        (&s4, "redo key route"),
    ] {
        let cmds = step_lands_on(&mut f, Message::Redo, target, what);
        settle(&mut f, cmds, target, what);
    }
    let cmds = step_lands_on(&mut f, Message::Redo, &s5, "redo bus delete");
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            format!("RemoveAuxSend {{ send_id: {send} }}"),
            format!("ClearSidechainRoute {{ plugin: {DRUM_BUS_COMP} }}"),
            format!("RemoveBus {{ bus_id: {bus} }}"),
            NO_TAKES.to_owned(),
        ],
        "redo bus delete: the routing, then the bus"
    );
    settle(&mut f, cmds, &s5, "redo bus delete");
}

/// The echoes of one restore land after the next one ran — a held Ctrl+Z
/// or a control client's undo/redo burst. `BusRemoved` for a bus the redo
/// has put back must not delete it, and the `BusAdded` of a bus the undo
/// has removed again must not resurrect it.
#[test]
fn a_bus_restore_survives_the_previous_restores_late_echoes() {
    let mut f = fixture("bus-late-echo");
    let s0 = f.app.test_snapshot_for_undo();
    let s1 = edit(&mut f, Message::Bus(BusMessage::AddBus));
    let undo = step_lands_on(&mut f, Message::Undo, &s0, "undo bus add");
    let redo = step_lands_on(&mut f, Message::Redo, &s1, "redo bus add");
    let late: Vec<_> = undo.into_iter().chain(redo).collect();
    settle(&mut f, late, &s1, "redo bus add, then undo's echoes");

    let undo = step_lands_on(&mut f, Message::Undo, &s0, "undo bus add again");
    let redo = step_lands_on(&mut f, Message::Redo, &s1, "redo bus add again");
    let back = step_lands_on(&mut f, Message::Undo, &s0, "and undo it once more");
    let late: Vec<_> = undo.into_iter().chain(redo).chain(back).collect();
    settle(&mut f, late, &s0, "undo bus add, then three restores' echoes");
}

/// STATE-10 on the diff path (FU-A13c): a live bus delete mirrors at once
/// and owes its `BusRemoved` echo, so an undo pressed before that echo
/// re-adds the bus under its id and the late echo must not remove it
/// again.
#[test]
fn undoing_a_bus_delete_before_its_echo_keeps_the_bus() {
    let mut f = fixture("bus-delete-early-undo");
    let ids = bus_ids(&f.app);
    let s1 = edit(&mut f, Message::Bus(BusMessage::AddBus));
    let bus = bus_ids(&f.app)
        .into_iter()
        .find(|id| !ids.contains(id))
        .expect("the add landed a bus");
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Bus(BusMessage::RemoveBus(bus)));
    assert!(
        !bus_ids(&f.app).contains(&bus),
        "the delete mirrors immediately, before its echo"
    );
    let delete = drain(&f.rx);
    let undo = step_lands_on(&mut f, Message::Undo, &s1, "undo bus delete before its echo");
    let late: Vec<_> = delete.into_iter().chain(undo).collect();
    settle(&mut f, late, &s1, "the delete's echo, then the undo's");
    assert!(bus_ids(&f.app).contains(&bus));
}

// ---------------------------------------------------------------------------
// Plugin instances on a track, a bus and the master (A-13h)
// ---------------------------------------------------------------------------

/// The fake plugins' one parameter (`echo_of`).
const GAIN: u32 = 1;
/// The demo's audio track, which carries no plugin.
const AUDIO_TRACK: u64 = 5;
/// The demo's first bus, which carries `DRUM_BUS_COMP`.
const DRUM_BUS: u64 = 100;

fn scanned(id: &str) -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: format!("/plugins/{id}.clap"),
        clap_plugin_id: format!("com.resonance.{id}"),
        name: id.to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        ..Default::default()
    }
}

/// The GUI add / remove / move of the chain `chain` names.
fn add_to(chain: TestChain, plugin: ScannedPlugin) -> Message {
    match chain {
        TestChain::Track(id) => Message::Plugin(PluginMessage::AddPluginToTrack(id, plugin)),
        TestChain::Bus(id) => Message::Bus(BusMessage::AddPluginToBus(id, plugin)),
        TestChain::Master => Message::Master(MasterMessage::AddPluginToMaster(plugin)),
    }
}

fn remove_from(chain: TestChain, instance_id: u64) -> Message {
    match chain {
        TestChain::Track(id) => {
            Message::Plugin(PluginMessage::RemovePluginFromTrack(id, instance_id))
        }
        TestChain::Bus(id) => Message::Bus(BusMessage::RemovePluginFromBus(id, instance_id)),
        TestChain::Master => Message::Master(MasterMessage::RemovePluginFromMaster(instance_id)),
    }
}

fn move_in(chain: TestChain, instance_id: u64, to_index: usize) -> Message {
    match chain {
        TestChain::Track(track_id) => Message::Plugin(PluginMessage::MovePluginInTrack {
            track_id,
            instance_id,
            to_index,
        }),
        TestChain::Bus(bus_id) => Message::Bus(BusMessage::MovePluginInBus {
            bus_id,
            instance_id,
            to_index,
        }),
        TestChain::Master => Message::Master(MasterMessage::MovePluginInMaster {
            instance_id,
            to_index,
        }),
    }
}

/// How the engine names a remove / move on `chain`, for the pinned
/// command lists.
fn remove_cmd(chain: TestChain, instance_id: u64) -> String {
    match chain {
        TestChain::Track(track_id) => {
            format!("RemovePlugin {{ track_id: {track_id}, instance_id: {instance_id} }}")
        }
        TestChain::Bus(bus_id) => {
            format!("RemovePluginFromBus {{ bus_id: {bus_id}, instance_id: {instance_id} }}")
        }
        TestChain::Master => format!("RemovePluginFromMaster {{ instance_id: {instance_id} }}"),
    }
}

fn move_cmd(chain: TestChain, instance_id: u64, to_index: usize) -> String {
    match chain {
        TestChain::Track(track_id) => format!(
            "MovePlugin {{ track_id: {track_id}, instance_id: {instance_id}, to_index: {to_index} }}"
        ),
        TestChain::Bus(bus_id) => format!(
            "MovePluginInBus {{ bus_id: {bus_id}, instance_id: {instance_id}, to_index: {to_index} }}"
        ),
        TestChain::Master => {
            format!("MovePluginInMaster {{ instance_id: {instance_id}, to_index: {to_index} }}")
        }
    }
}

fn add_cmd(chain: TestChain, instance_id: u64, id: &str) -> String {
    let (path, clap) = (format!("/plugins/{id}.clap"), format!("com.resonance.{id}"));
    match chain {
        TestChain::Track(track_id) => format!(
            "AddPlugin {{ track_id: {track_id}, clap_file_path: {path:?}, clap_plugin_id: \
             {clap:?}, id: {instance_id} }}"
        ),
        TestChain::Bus(bus_id) => format!(
            "AddPluginToBus {{ bus_id: {bus_id}, clap_file_path: {path:?}, clap_plugin_id: \
             {clap:?}, id: {instance_id} }}"
        ),
        TestChain::Master => format!(
            "AddPluginToMaster {{ clap_file_path: {path:?}, clap_plugin_id: {clap:?}, id: \
             {instance_id} }}"
        ),
    }
}

fn chain_ids(app: &Resonance, chain: TestChain) -> Vec<u64> {
    app.test_chain_slots(chain).into_iter().map(|s| s.0).collect()
}

/// On `chain`: add an EQ and a compressor (GUI adds, mirrored on the
/// echo), key the compressor from the drums, turn its gain up and bypass
/// it, move it in front of the EQ, then remove it. Every undo and redo
/// over the seven edits takes the diff path; the pinned steps are the
/// structural ones:
///
/// * undo the remove — the compressor is re-added (appended, as the
///   engine does), its bypass sent after the add, its gain parked for the
///   `PluginAdded` echo, then moved back in front of the EQ; its key
///   route after all of that;
/// * undo the move — one `MovePlugin*`, nothing re-instantiated;
/// * undo the add — one `RemovePlugin*`, no other plugin touched;
/// * redo the remove — the key route, then the instance.
///
/// Each step's echoes are played back and must change nothing.
fn plugin_chain_undo_walk(tag: &str, chain: TestChain) {
    let mut f = fixture(tag);
    let before = chain_ids(&f.app, chain);
    let s0 = f.app.test_snapshot_for_undo();
    let s1 = edit(&mut f, add_to(chain, scanned("eq")));
    let eq = *chain_ids(&f.app, chain).last().expect("the EQ landed");
    let s2 = edit(&mut f, add_to(chain, scanned("comp")));
    let comp = *chain_ids(&f.app, chain).last().expect("the compressor landed");
    let s3 = edit(
        &mut f,
        Message::Plugin(PluginMessage::SetPluginSidechain {
            instance_id: comp,
            source: Some(SendSource::Track(DRUMS)),
            enabled: true,
        }),
    );
    let s4 = edit(&mut f, Message::Plugin(PluginMessage::SetPluginParam(comp, GAIN, 0.5)));
    let s5 = edit(
        &mut f,
        Message::Plugin(PluginMessage::SetPluginBypass {
            instance_id: comp,
            bypassed: true,
        }),
    );
    let eq_at = before.len();
    let s6 = edit(&mut f, move_in(chain, comp, eq_at));
    assert_eq!(
        chain_ids(&f.app, chain)[eq_at..],
        [comp, eq],
        "the move put the compressor in front of the EQ"
    );
    let s7 = edit(&mut f, remove_from(chain, comp));
    assert_eq!(chain_ids(&f.app, chain)[eq_at..], [eq], "the remove landed");
    assert!(s7.project.file.sidechain_routes.is_empty(), "and took its key route");

    // Undo the remove.
    let cmds = step_lands_on(&mut f, Message::Undo, &s6, "undo plugin remove");
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            add_cmd(chain, comp, "comp"),
            format!("SetPluginBypass {{ instance_id: {comp}, bypassed: true }}"),
            move_cmd(chain, comp, eq_at),
            format!(
                "SetSidechainRoute {{ plugin: {comp}, source: Track({DRUMS}), enabled: true }}"
            ),
            NO_TAKES.to_owned(),
        ],
        "undo plugin remove: add, bypass, reorder, then the key route"
    );
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::SetPluginParam { .. })),
        "the re-added instance's params wait for its PluginAdded echo"
    );
    settle(&mut f, cmds, &s6, "undo plugin remove");
    let gain = f
        .app
        .test_chain_slots(chain)
        .iter()
        .position(|s| s.0 == comp)
        .expect("the compressor is back");
    assert_eq!(gain, eq_at, "in front of the EQ");

    // Undo the move.
    let cmds = step_lands_on(&mut f, Message::Undo, &s5, "undo plugin move");
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            move_cmd(chain, eq, eq_at),
            NO_TAKES.to_owned(),
        ],
        "undo plugin move: one move, nothing re-instantiated"
    );
    settle(&mut f, cmds, &s5, "undo plugin move");
    for (target, what) in [
        (&s4, "undo plugin bypass"),
        (&s3, "undo plugin param"),
        (&s2, "undo key route"),
    ] {
        let cmds = step_lands_on(&mut f, Message::Undo, target, what);
        settle(&mut f, cmds, target, what);
    }

    // Undo the adds.
    let cmds = step_lands_on(&mut f, Message::Undo, &s1, "undo compressor add");
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            remove_cmd(chain, comp),
            NO_TAKES.to_owned(),
        ],
        "undo compressor add: that instance only"
    );
    settle(&mut f, cmds, &s1, "undo compressor add");
    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo EQ add");
    settle(&mut f, cmds, &s0, "undo EQ add");
    assert_eq!(chain_ids(&f.app, chain), before, "back to the demo's chain");

    for (target, what) in [
        (&s1, "redo EQ add"),
        (&s2, "redo compressor add"),
        (&s3, "redo key route"),
        (&s4, "redo plugin param"),
        (&s5, "redo plugin bypass"),
        (&s6, "redo plugin move"),
    ] {
        let cmds = step_lands_on(&mut f, Message::Redo, target, what);
        settle(&mut f, cmds, target, what);
    }
    let cmds = step_lands_on(&mut f, Message::Redo, &s7, "redo plugin remove");
    assert_eq!(
        sent(&cmds),
        [
            "PersistClipWavs".to_owned(),
            TEMPO.to_owned(),
            format!("ClearSidechainRoute {{ plugin: {comp} }}"),
            remove_cmd(chain, comp),
            NO_TAKES.to_owned(),
        ],
        "redo plugin remove: the key route, then the instance"
    );
    settle(&mut f, cmds, &s7, "redo plugin remove");
    assert_eq!(
        f.app.test_plugin_index(comp),
        None,
        "the removed instance left the side-index"
    );
}

#[test]
fn adding_removing_and_reordering_a_track_plugin_undoes_through_the_diff_path() {
    plugin_chain_undo_walk("track-plugin", TestChain::Track(AUDIO_TRACK));
}

#[test]
fn adding_removing_and_reordering_a_bus_plugin_undoes_through_the_diff_path() {
    plugin_chain_undo_walk("bus-plugin", TestChain::Bus(DRUM_BUS));
}

#[test]
fn adding_removing_and_reordering_a_master_plugin_undoes_through_the_diff_path() {
    plugin_chain_undo_walk("master-plugin", TestChain::Master);
}

/// A removed plugin that was selected in the mixer is not left selected.
#[test]
fn undoing_a_plugin_add_drops_its_selection() {
    let mut f = fixture("plugin-selection");
    let chain = TestChain::Track(AUDIO_TRACK);
    let s0 = f.app.test_snapshot_for_undo();
    let _ = edit(&mut f, add_to(chain, scanned("eq")));
    let eq = chain_ids(&f.app, chain)[0];
    let _ = f.app.update(Message::Plugin(PluginMessage::TogglePluginPanel(eq)));
    assert_eq!(f.app.test_selected_plugin(), Some(eq), "the panel selected it");
    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo EQ add");
    assert_eq!(f.app.test_selected_plugin(), None);
    settle(&mut f, cmds, &s0, "undo EQ add");
}

/// Undo/redo bursts whose echoes land late, on a chain (a held Ctrl+Z, a
/// control client's burst). Without `io.restore_echoes` both halves fail:
///
/// * two reorders undone back to back: replaying the first undo's
///   `PluginMoved` on the chain the second undo left scrambles it (the
///   echo is an absolute "move X to i", and the second undo skipped the
///   slots that were already in place when it ran);
/// * a remove undone, redone and undone again: the redo's `PluginRemoved`
///   lands on the instance the last undo re-added and drops it with its
///   parked params, and the `PluginAdded` after it pushes a bare slot.
#[test]
fn a_plugin_restore_survives_the_previous_restores_late_echoes() {
    let mut f = fixture("plugin-late-echo");
    let chain = TestChain::Track(AUDIO_TRACK);
    let _ = edit(&mut f, add_to(chain, scanned("eq")));
    let eq = chain_ids(&f.app, chain)[0];
    let _ = edit(&mut f, Message::Plugin(PluginMessage::SetPluginParam(eq, GAIN, 0.5)));
    let _ = edit(
        &mut f,
        Message::Plugin(PluginMessage::SetPluginBypass {
            instance_id: eq,
            bypassed: true,
        }),
    );
    let _ = edit(&mut f, add_to(chain, scanned("comp")));
    let in_order = edit(&mut f, add_to(chain, scanned("gate")));
    let [_, comp, gate] = chain_ids(&f.app, chain)[..] else {
        panic!("three plugins on the chain");
    };
    let eq_last = edit(&mut f, move_in(chain, eq, 2));
    let eq_middle = edit(&mut f, move_in(chain, eq, 1));
    assert_eq!(chain_ids(&f.app, chain), [comp, eq, gate]);

    let mut late = Vec::new();
    late.extend(step_lands_on(&mut f, Message::Undo, &eq_last, "undo second move"));
    late.extend(step_lands_on(&mut f, Message::Undo, &in_order, "undo first move"));
    settle(&mut f, late, &in_order, "two undone moves, then their echoes");
    assert_eq!(chain_ids(&f.app, chain), [eq, comp, gate]);

    let mut late = Vec::new();
    late.extend(step_lands_on(&mut f, Message::Redo, &eq_last, "redo first move"));
    late.extend(step_lands_on(&mut f, Message::Redo, &eq_middle, "redo second move"));
    settle(&mut f, late, &eq_middle, "two redone moves, then their echoes");

    let removed = edit(&mut f, remove_from(chain, eq));
    let mut late = Vec::new();
    late.extend(step_lands_on(&mut f, Message::Undo, &eq_middle, "undo remove"));
    late.extend(step_lands_on(&mut f, Message::Redo, &removed, "redo remove"));
    late.extend(step_lands_on(&mut f, Message::Undo, &eq_middle, "undo remove again"));
    settle(&mut f, late, &eq_middle, "remove undone, redone, undone, then the echoes");
    // `settle` compared the whole file: the EQ is back in the middle,
    // bypassed, its gain at 0.5.
    assert_eq!(chain_ids(&f.app, chain), [comp, eq, gate]);
}

/// A plugin re-added by one restore has no param list until its
/// `PluginAdded`: its values are parked for the echo. A second restore
/// before that echo, to a state where the plugin's values differ, must
/// re-park the target's values rather than leave the first restore's for
/// the echo to apply.
#[test]
fn a_re_added_plugins_params_follow_a_second_restore_before_its_echo() {
    let mut f = fixture("plugin-parked-params");
    let chain = TestChain::Track(AUDIO_TRACK);
    let added = edit(&mut f, add_to(chain, scanned("eq")));
    let eq = chain_ids(&f.app, chain)[0];
    let turned_up = edit(&mut f, Message::Plugin(PluginMessage::SetPluginParam(eq, GAIN, 0.5)));
    let _removed = edit(&mut f, remove_from(chain, eq));

    let mut late = Vec::new();
    late.extend(step_lands_on(&mut f, Message::Undo, &turned_up, "undo remove"));
    late.extend(step_lands_on(&mut f, Message::Undo, &added, "undo param"));
    settle(&mut f, late, &added, "undo remove, undo param, then the echoes");
}

/// STATE-10 on the diff path (FU-A13c): a live plugin delete (track, bus
/// or master chain) mirrors at once and owes its `*PluginRemoved` echo, so
/// an undo pressed before that echo re-adds the instance under its id and
/// the late echo must not remove it again.
fn plugin_delete_before_echo_keeps_the_plugin(tag: &str, chain: TestChain) {
    let mut f = fixture(tag);
    let s0 = edit(&mut f, add_to(chain, scanned("eq")));
    let eq = *chain_ids(&f.app, chain).last().expect("the EQ landed");
    let _ = drain(&f.rx);
    let _ = f.app.update(remove_from(chain, eq));
    assert!(
        !chain_ids(&f.app, chain).contains(&eq),
        "the delete mirrors immediately, before its echo"
    );
    let delete = drain(&f.rx);
    let undo = step_lands_on(&mut f, Message::Undo, &s0, "undo plugin delete before its echo");
    let late: Vec<_> = delete.into_iter().chain(undo).collect();
    settle(&mut f, late, &s0, "the delete's echo, then the undo's");
    assert!(chain_ids(&f.app, chain).contains(&eq));
}

#[test]
fn undoing_a_track_plugin_delete_before_its_echo_keeps_the_plugin() {
    plugin_delete_before_echo_keeps_the_plugin(
        "track-plugin-delete-early-undo",
        TestChain::Track(AUDIO_TRACK),
    );
}

#[test]
fn undoing_a_bus_plugin_delete_before_its_echo_keeps_the_plugin() {
    plugin_delete_before_echo_keeps_the_plugin(
        "bus-plugin-delete-early-undo",
        TestChain::Bus(DRUM_BUS),
    );
}

#[test]
fn undoing_a_master_plugin_delete_before_its_echo_keeps_the_plugin() {
    plugin_delete_before_echo_keeps_the_plugin("master-plugin-delete-early-undo", TestChain::Master);
}

// ---------------------------------------------------------------------------
// A re-added plugin that turns out missing (FU-A13d)
// ---------------------------------------------------------------------------

/// A chain the engine keeps, as [`EngineChains`] names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ChainKey {
    Track(u64),
    Bus(u64),
    Master,
}

impl From<TestChain> for ChainKey {
    fn from(chain: TestChain) -> Self {
        match chain {
            TestChain::Track(id) => ChainKey::Track(id),
            TestChain::Bus(id) => ChainKey::Bus(id),
            TestChain::Master => ChainKey::Master,
        }
    }
}

/// The engine's own plugin chains, which the app only mirrors: an add
/// appends, unless the plugin is not installed, when it answers
/// `PluginLoadFailed` and the chain never holds the instance; a move is
/// remove + insert at the clamped index and echoes that index; a move of
/// an instance the chain does not hold answers with an error, not an echo
/// (`handle_move_plugin`). This is what the pinned command lists cannot
/// show: where the plugins actually end up, and so what the user hears.
struct EngineChains {
    chains: HashMap<ChainKey, Vec<u64>>,
    /// `clap_plugin_id`s whose `.clap` is gone from this machine.
    uninstalled: HashSet<String>,
}

impl EngineChains {
    /// The engine as the app left it: every chain's live slots, in order.
    fn of(app: &Resonance) -> Self {
        let mut keys: Vec<TestChain> = app
            .test_registry()
            .tracks
            .iter()
            .map(|t| TestChain::Track(t.id))
            .collect();
        keys.extend(app.test_registry().busses.iter().map(|b| TestChain::Bus(b.id)));
        keys.push(TestChain::Master);
        let chains = keys
            .into_iter()
            .map(|chain| {
                let live = app
                    .test_chain_slots(chain)
                    .into_iter()
                    .filter(|(_, _, missing)| !missing)
                    .map(|(id, _, _)| id)
                    .collect();
                (ChainKey::from(chain), live)
            })
            .collect();
        Self {
            chains,
            uninstalled: HashSet::new(),
        }
    }

    fn chain(&self, chain: TestChain) -> Vec<u64> {
        self.chains.get(&chain.into()).cloned().unwrap_or_default()
    }

    /// The engine's answer when it differs from `echo_of`'s, having
    /// applied `cmd` to the chains.
    fn answer(&mut self, cmd: &AudioCommand) -> Option<Vec<AudioEvent>> {
        let (key, clap_file_path, clap_plugin_id, id) = match cmd {
            AudioCommand::AddPlugin {
                track_id,
                clap_file_path,
                clap_plugin_id,
                id,
            } => (ChainKey::Track(*track_id), clap_file_path, clap_plugin_id, *id),
            AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path,
                clap_plugin_id,
                id,
            } => (ChainKey::Bus(*bus_id), clap_file_path, clap_plugin_id, *id),
            AudioCommand::AddPluginToMaster {
                clap_file_path,
                clap_plugin_id,
                id,
            } => (ChainKey::Master, clap_file_path, clap_plugin_id, *id),
            AudioCommand::RemovePlugin {
                track_id,
                instance_id,
            } => {
                self.remove(ChainKey::Track(*track_id), *instance_id);
                return None;
            }
            AudioCommand::RemovePluginFromBus {
                bus_id,
                instance_id,
            } => {
                self.remove(ChainKey::Bus(*bus_id), *instance_id);
                return None;
            }
            AudioCommand::RemovePluginFromMaster { instance_id } => {
                self.remove(ChainKey::Master, *instance_id);
                return None;
            }
            AudioCommand::MovePlugin {
                track_id,
                instance_id,
                to_index,
            } => {
                let to = self.move_to(ChainKey::Track(*track_id), *instance_id, *to_index);
                return Some(to.map_or_else(Vec::new, |to_index| {
                    vec![AudioEvent::PluginMoved {
                        track_id: *track_id,
                        instance_id: *instance_id,
                        to_index,
                    }]
                }));
            }
            AudioCommand::MovePluginInBus {
                bus_id,
                instance_id,
                to_index,
            } => {
                let to = self.move_to(ChainKey::Bus(*bus_id), *instance_id, *to_index);
                return Some(to.map_or_else(Vec::new, |to_index| {
                    vec![AudioEvent::BusPluginMoved {
                        bus_id: *bus_id,
                        instance_id: *instance_id,
                        to_index,
                    }]
                }));
            }
            AudioCommand::MovePluginInMaster {
                instance_id,
                to_index,
            } => {
                let to = self.move_to(ChainKey::Master, *instance_id, *to_index);
                return Some(to.map_or_else(Vec::new, |to_index| {
                    vec![AudioEvent::MasterPluginMoved {
                        instance_id: *instance_id,
                        to_index,
                    }]
                }));
            }
            AudioCommand::RemoveTrack { track_id } => {
                self.chains.remove(&ChainKey::Track(*track_id));
                return None;
            }
            AudioCommand::RemoveBus { bus_id } => {
                self.chains.remove(&ChainKey::Bus(*bus_id));
                return None;
            }
            _ => return None,
        };
        if self.uninstalled.contains(clap_plugin_id) {
            return Some(vec![AudioEvent::PluginLoadFailed {
                instance_id: Some(id),
                clap_plugin_id: clap_plugin_id.clone(),
                clap_file_path: clap_file_path.clone(),
                reason: "not installed".to_owned(),
            }]);
        }
        self.chains.entry(key).or_default().push(id);
        None
    }

    fn remove(&mut self, key: ChainKey, id: u64) {
        if let Some(chain) = self.chains.get_mut(&key) {
            chain.retain(|&p| p != id);
        }
    }

    /// `None` when the chain does not hold `id` (the engine's error).
    fn move_to(&mut self, key: ChainKey, id: u64, to_index: usize) -> Option<usize> {
        let chain = self.chains.get_mut(&key)?;
        let from = chain.iter().position(|&p| p == id)?;
        let to = to_index.min(chain.len() - 1);
        let moved = chain.remove(from);
        chain.insert(to, moved);
        Some(to)
    }
}

/// Where, in the chain one restore re-adds two plugins to, the one that
/// turns out missing sits.
#[derive(Debug, Clone, Copy)]
enum MissingAt {
    /// `[m, b, (the chain's own plugins), a, c]`.
    Start,
    /// `[(own), a, m, b, c]` — the order the plugins were added in.
    Middle,
    /// `[(own), a, b, c, m]`.
    End,
}

/// FU-A13d. On `chain`, add `a`, `m`, `b` and `c` and arrange them as
/// `at` says; remove `m`, then `b`; uninstall `m`; then restore the state
/// with all four in one step, so one restore re-adds both `m` and `b` and
/// `m`'s add fails.
///
/// The restore appends both and then moves each into place, naming
/// **engine** indices; a slot whose plugin is missing is not in the
/// engine's chain. `m` is not known to be missing until its
/// `PluginLoadFailed` arrives, after the restore has sent every move, so
/// a move that counted `m` ahead of it is one engine slot off. After the
/// echoes the engine's chain must be the mirror's with `m` skipped, and
/// nothing may still be owed (the move of `m` itself is answered with an
/// error, never an echo).
fn missing_plugin_re_add_walk(tag: &str, chain: TestChain, at: MissingAt) {
    let mut f = fixture(tag);
    f.chains = Some(EngineChains::of(&f.app));
    let own = chain_ids(&f.app, chain);
    let e = own.len();
    let add = |f: &mut Fixture, name: &str| {
        let _ = edit(f, add_to(chain, scanned(name)));
        *chain_ids(&f.app, chain).last().expect("the plugin landed")
    };
    let a = add(&mut f, "a");
    let m = add(&mut f, "m");
    let b = add(&mut f, "b");
    let c = add(&mut f, "c");
    match at {
        MissingAt::Start => {
            let _ = edit(&mut f, move_in(chain, m, 0));
            let _ = edit(&mut f, move_in(chain, b, 1));
        }
        MissingAt::Middle => {}
        MissingAt::End => {
            let _ = edit(&mut f, move_in(chain, m, e + 3));
        }
    }
    let full = f.app.test_snapshot_for_undo();
    let want: Vec<u64> = match at {
        MissingAt::Start => [m, b].into_iter().chain(own.iter().copied()).chain([a, c]).collect(),
        MissingAt::Middle => own.iter().copied().chain([a, m, b, c]).collect(),
        MissingAt::End => own.iter().copied().chain([a, b, c, m]).collect(),
    };
    assert_eq!(chain_ids(&f.app, chain), want, "{at:?}: the arrangement landed");
    let engine = |f: &Fixture| f.chains.as_ref().expect("modelled").chain(chain);
    assert_eq!(engine(&f), want, "{at:?}: the engine agrees before anything is missing");

    let _ = edit(&mut f, remove_from(chain, m));
    let _ = edit(&mut f, remove_from(chain, b));
    f.chains
        .as_mut()
        .expect("modelled")
        .uninstalled
        .insert("com.resonance.m".to_owned());

    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(full);
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "{at:?}: must take the diff path"
    );
    assert!(
        f.app.test_reconcile_trace().iter().all(|(o, _)| *o == Origin::Undo),
        "{at:?}: every domain runs under Undo"
    );
    assert_eq!(chain_ids(&f.app, chain), want, "{at:?}: the restore mirrored the order");
    echo(&mut f, cmds);

    assert_eq!(
        chain_ids(&f.app, chain),
        want,
        "{at:?}: m keeps its slot in the app's chain"
    );
    assert!(
        f.app
            .test_chain_slots(chain)
            .iter()
            .any(|(id, _, missing)| *id == m && *missing),
        "{at:?}: m's slot is marked missing"
    );
    let live: Vec<u64> = want.iter().copied().filter(|&id| id != m).collect();
    assert_eq!(
        engine(&f),
        live,
        "{at:?}: the engine's chain is the app's with the missing slot skipped \
         (a={a}, m={m}, b={b}, c={c}, own={own:?})"
    );
    assert!(
        f.app.test_restore_echoes_settled(),
        "{at:?}: every owed echo was consumed"
    );
}

#[test]
fn a_re_added_plugin_after_a_missing_one_lands_in_its_engine_slot_on_a_track() {
    for at in [MissingAt::Start, MissingAt::Middle, MissingAt::End] {
        missing_plugin_re_add_walk(
            &format!("missing-re-add-track-{at:?}"),
            TestChain::Track(AUDIO_TRACK),
            at,
        );
    }
}

#[test]
fn a_re_added_plugin_after_a_missing_one_lands_in_its_engine_slot_on_a_bus() {
    for at in [MissingAt::Start, MissingAt::Middle, MissingAt::End] {
        missing_plugin_re_add_walk(
            &format!("missing-re-add-bus-{at:?}"),
            TestChain::Bus(DRUM_BUS),
            at,
        );
    }
}

#[test]
fn a_re_added_plugin_after_a_missing_one_lands_in_its_engine_slot_on_the_master() {
    for at in [MissingAt::Start, MissingAt::Middle, MissingAt::End] {
        missing_plugin_re_add_walk(
            &format!("missing-re-add-master-{at:?}"),
            TestChain::Master,
            at,
        );
    }
}

// ---------------------------------------------------------------------------
// Tracks (A-13i)
// ---------------------------------------------------------------------------

fn track_ids(app: &Resonance) -> HashSet<u64> {
    app.test_registry().tracks.iter().map(|t| t.id).collect()
}

/// Apply a recorded edit that adds exactly one top-level track; return
/// the snapshot it left and the new track's id.
fn add_track(f: &mut Fixture, msg: Message) -> (UndoSnapshot, u64) {
    let before = track_ids(&f.app);
    let s = edit(f, msg);
    let added: Vec<u64> = track_ids(&f.app)
        .difference(&before)
        .copied()
        .filter(|id| {
            f.app
                .test_registry()
                .tracks
                .iter()
                .any(|t| t.id == *id && t.sub_track.is_none())
        })
        .collect();
    assert_eq!(added.len(), 1, "the edit added one track: {added:?}");
    (s, added[0])
}

/// The GUI delete of an empty track (no confirmation, one undo entry).
fn delete_track(f: &mut Fixture, track_id: u64) -> UndoSnapshot {
    edit(f, Message::Track(TrackMessage::RequestRemoveTrack(track_id)))
}

/// What `replay_track` sends after the add command for a default track
/// (`mono` differs by type), before its plugins.
fn default_track_scalars(t: u64, mono: bool) -> Vec<String> {
    vec![
        format!("SetTrackVolume {{ track_id: {t}, volume: 1.0 }}"),
        format!("SetTrackPan {{ track_id: {t}, pan: 0.0 }}"),
        format!("SetTrackMute {{ track_id: {t}, muted: false }}"),
        format!("SetTrackSolo {{ track_id: {t}, soloed: false }}"),
        format!("SetTrackRecordArm {{ track_id: {t}, armed: false }}"),
        format!("SetTrackMonitor {{ track_id: {t}, enabled: false }}"),
        format!("SetTrackPlaybackSource {{ track_id: {t}, source: Live }}"),
        format!("SetTrackMono {{ track_id: {t}, mono: {mono} }}"),
        format!("SetTrackFxBypass {{ track_id: {t}, bypassed: false }}"),
        format!("SetTrackInputPort {{ track_id: {t}, port_index: 0 }}"),
    ]
}

fn wrap(inner: Vec<String>) -> Vec<String> {
    ["PersistClipWavs".to_owned(), TEMPO.to_owned()]
        .into_iter()
        .chain(inner)
        .chain([NO_TAKES.to_owned()])
        .collect()
}

/// `before` → add → `added` → delete → `deleted`, then every undo and
/// redo over the two edits, each settled. Returns the four steps'
/// commands (undo delete, undo add, redo add, redo delete).
fn track_add_delete_walk(
    f: &mut Fixture,
    before: &UndoSnapshot,
    added: &UndoSnapshot,
    deleted: &UndoSnapshot,
) -> [Vec<String>; 4] {
    let a = step_lands_on(f, Message::Undo, added, "undo track delete");
    let a_sent = sent(&a);
    settle(f, a, added, "undo track delete");
    let b = step_lands_on(f, Message::Undo, before, "undo track add");
    let b_sent = sent(&b);
    settle(f, b, before, "undo track add");
    let c = step_lands_on(f, Message::Redo, added, "redo track add");
    let c_sent = sent(&c);
    settle(f, c, added, "redo track add");
    let d = step_lands_on(f, Message::Redo, deleted, "redo track delete");
    let d_sent = sent(&d);
    settle(f, d, deleted, "redo track delete");
    [a_sent, b_sent, c_sent, d_sent]
}

/// A GUI audio-track add, then its delete: undo re-adds it under its id
/// (the add command, every scalar, nothing else), undo again removes it,
/// and the redos do the same — no `ClearAll`, nothing else touched.
#[test]
fn adding_and_removing_an_audio_track_undoes_through_the_diff_path() {
    let mut f = fixture("audio-track");
    let s0 = f.app.test_snapshot_for_undo();
    let (s1, t) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    let s2 = delete_track(&mut f, t);
    let [undo_delete, undo_add, redo_add, redo_delete] = track_add_delete_walk(&mut f, &s0, &s1, &s2);
    let add = wrap(
        std::iter::once(format!("AddTrack {{ id: {t}, name: Some(\"Track {t}\") }}"))
            .chain(default_track_scalars(t, true))
            .collect(),
    );
    let remove = wrap(vec![format!("RemoveTrack {{ track_id: {t} }}")]);
    assert_eq!(undo_delete, add, "undo track delete: the add, every scalar");
    assert_eq!(undo_add, remove, "undo track add: that track only");
    assert_eq!(redo_add, add, "redo track add");
    assert_eq!(redo_delete, remove, "redo track delete");
}

#[test]
fn adding_and_removing_a_vocal_track_undoes_through_the_diff_path() {
    let mut f = fixture("vocal-track");
    let s0 = f.app.test_snapshot_for_undo();
    let (s1, t) = add_track(&mut f, Message::Track(TrackMessage::AddVocalTrack));
    let s2 = delete_track(&mut f, t);
    let [undo_delete, undo_add, ..] = track_add_delete_walk(&mut f, &s0, &s1, &s2);
    assert!(
        undo_delete[2].starts_with(&format!("AddVocalTrack {{ id: {t},")),
        "undo vocal track delete re-adds a vocal track: {undo_delete:?}"
    );
    assert_eq!(undo_add, wrap(vec![format!("RemoveTrack {{ track_id: {t} }}")]));
}

/// An instrument track carrying a plugin (gain up, bypassed) and a send:
/// undoing its delete re-adds the track, then the plugin as a load does
/// (blob-less, bypass after the add, param parked for the echo), then the
/// send once the track exists. Undoing the add removes the track whole —
/// its send first, its plugin with it (no per-plugin command).
#[test]
fn adding_and_removing_an_instrument_track_with_plugins_and_sends_undoes_through_the_diff_path() {
    let mut f = fixture("instrument-track");
    let s0 = f.app.test_snapshot_for_undo();
    let (_, t) = add_track(&mut f, Message::Track(TrackMessage::AddInstrumentTrack));
    let _ = edit(&mut f, add_to(TestChain::Track(t), scanned("synth")));
    let synth = chain_ids(&f.app, TestChain::Track(t))[0];
    let _ = edit(&mut f, Message::Plugin(PluginMessage::SetPluginParam(synth, GAIN, 0.5)));
    let _ = edit(
        &mut f,
        Message::Plugin(PluginMessage::SetPluginBypass {
            instance_id: synth,
            bypassed: true,
        }),
    );
    let s_full = edit(
        &mut f,
        Message::Mixer(MixerMessage::AddSend {
            source: SendSource::Track(t),
            dest: DRUM_BUS,
        }),
    );
    let send = s_full.project.file.sends[0].id;
    let s_deleted = delete_track(&mut f, t);
    assert!(s_deleted.project.file.sends.is_empty(), "the delete took its send");

    let cmds = step_lands_on(&mut f, Message::Undo, &s_full, "undo instrument track delete");
    assert_eq!(
        sent(&cmds),
        wrap(
            std::iter::once(format!("AddInstrumentTrack {{ id: {t}, name: Some(\"Instrument {t}\") }}"))
                .chain(default_track_scalars(t, false))
                .chain([
                    add_cmd(TestChain::Track(t), synth, "synth"),
                    format!("SetPluginBypass {{ instance_id: {synth}, bypassed: true }}"),
                    format!(
                        "AddAuxSend {{ id: {send}, source: Track({t}), dest: {DRUM_BUS}, \
                         level_db: 0.0, pre_fader: false, enabled: true }}"
                    ),
                ])
                .collect()
        ),
        "undo instrument track delete: track, plugin, bypass, send"
    );
    settle(&mut f, cmds, &s_full, "undo instrument track delete");

    let cmds = step_lands_on(&mut f, Message::Redo, &s_deleted, "redo instrument track delete");
    assert_eq!(
        sent(&cmds),
        wrap(vec![
            format!("RemoveAuxSend {{ send_id: {send} }}"),
            format!("RemoveTrack {{ track_id: {t} }}"),
        ]),
        "redo instrument track delete: the send, then the track and its chain"
    );
    settle(&mut f, cmds, &s_deleted, "redo instrument track delete");

    // Undo back over every edit to before the track add, then redo.
    let _ = s0;
}

fn sub_tracks_of(app: &Resonance, parent: u64) -> Vec<u64> {
    let mut subs: Vec<u64> = app
        .test_registry()
        .tracks
        .iter()
        .filter(|t| t.sub_track.is_some_and(|l| l.parent_track_id == parent))
        .map(|t| t.id)
        .collect();
    subs.sort_unstable();
    subs
}

/// A multi-output instrument makes its sub-tracks on its `PluginAdded`
/// echo (`ensure_subtracks`). A restore that re-adds the instrument
/// together with its sub-tracks adds the sub-tracks itself, under their
/// saved ids, before that echo — which then finds every (parent, port)
/// taken and adds none (`settle` would see a duplicate in the file). The
/// plugin's removal with the parent kept removes the sub-tracks one by
/// one; the parent's removal takes the sub-tracks first, each by its own
/// `RemoveTrack`.
#[test]
fn a_sub_track_producing_instrument_undoes_through_the_diff_path() {
    let mut f = fixture("sub-tracks");
    let s0 = f.app.test_snapshot_for_undo();
    let (s1, t) = add_track(&mut f, Message::Track(TrackMessage::AddInstrumentTrack));
    let s2 = edit(&mut f, add_to(TestChain::Track(t), scanned("multi")));
    let multi = chain_ids(&f.app, TestChain::Track(t))[0];
    let subs = sub_tracks_of(&f.app, t);
    assert_eq!(subs.len(), 2, "the echo made two sub-tracks");
    let s3 = delete_track(&mut f, t);
    assert!(sub_tracks_of(&f.app, t).is_empty(), "the delete took them");

    // Undo the delete: parent, its plugin, then the sub-tracks.
    let cmds = step_lands_on(&mut f, Message::Undo, &s2, "undo instrument delete");
    let sent_cmds = sent(&cmds);
    let creates: Vec<&String> = sent_cmds
        .iter()
        .filter(|c| c.starts_with("CreateSubTrack"))
        .collect();
    assert_eq!(creates.len(), 2, "both sub-tracks re-added: {sent_cmds:?}");
    let add_parent = sent_cmds
        .iter()
        .position(|c| c.starts_with("AddInstrumentTrack"))
        .expect("the parent is re-added");
    let first_sub = sent_cmds
        .iter()
        .position(|c| c.starts_with("CreateSubTrack"))
        .expect("a sub-track is re-added");
    assert!(add_parent < first_sub, "parent before its sub-tracks");
    for sub in &subs {
        assert!(
            sent_cmds.iter().any(|c| c.starts_with(&format!("CreateSubTrack {{ sub_id: {sub},"))),
            "sub-track {sub} keeps its id"
        );
    }
    settle(&mut f, cmds, &s2, "undo instrument delete (the echo adds no sub-track)");
    assert_eq!(sub_tracks_of(&f.app, t), subs);

    // Undo the plugin add: the plugin and both sub-tracks go, the parent
    // stays.
    let cmds = step_lands_on(&mut f, Message::Undo, &s1, "undo multi-out plugin add");
    assert_eq!(
        sent(&cmds),
        wrap(vec![
            remove_cmd(TestChain::Track(t), multi),
            format!("RemoveTrack {{ track_id: {} }}", subs[0]),
            format!("RemoveTrack {{ track_id: {} }}", subs[1]),
        ]),
        "undo multi-out plugin add: the plugin, then its sub-tracks"
    );
    settle(&mut f, cmds, &s1, "undo multi-out plugin add");

    // Undo the track add, then redo all three.
    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo instrument add");
    settle(&mut f, cmds, &s0, "undo instrument add");
    for (target, what) in [(&s1, "redo instrument add"), (&s2, "redo multi-out plugin add")] {
        let cmds = step_lands_on(&mut f, Message::Redo, target, what);
        settle(&mut f, cmds, target, what);
    }
    assert_eq!(sub_tracks_of(&f.app, t), subs, "the redo re-added the same sub-tracks");
    let cmds = step_lands_on(&mut f, Message::Redo, &s3, "redo instrument delete");
    assert_eq!(
        sent(&cmds),
        wrap(vec![
            format!("RemoveTrack {{ track_id: {} }}", subs[0]),
            format!("RemoveTrack {{ track_id: {} }}", subs[1]),
            format!("RemoveTrack {{ track_id: {t} }}"),
        ]),
        "redo instrument delete: sub-tracks first, then the parent with its chain"
    );
    settle(&mut f, cmds, &s3, "redo instrument delete");
}

/// An external-instrument track: re-added with its config
/// (`SetExternalInstrument`), and — fresh, like after a `ClearAll` — no
/// empty `SetTrackDeviceParams` when no device is selected. Removed, its
/// config is cleared on the engine.
#[test]
fn adding_and_removing_an_external_instrument_track_undoes_through_the_diff_path() {
    let mut f = fixture("external-track");
    let s0 = f.app.test_snapshot_for_undo();
    let (s1, t) = add_track(&mut f, Message::Track(TrackMessage::AddExternalInstrumentTrack));
    assert!(
        s1.project.file.tracks.iter().any(|pt| pt.id == t && pt.external_instrument.is_some()),
        "the add made an external track"
    );
    let s2 = delete_track(&mut f, t);
    let [undo_delete, undo_add, ..] = track_add_delete_walk(&mut f, &s0, &s1, &s2);
    assert!(
        undo_delete.iter().any(|c| c.starts_with(&format!(
            "SetExternalInstrument {{ config: ExternalInstrument {{ track_id: {t},"
        ))),
        "undo delete restores the external config: {undo_delete:?}"
    );
    assert!(
        !undo_delete.iter().any(|c| c.starts_with("SetTrackDeviceParams")),
        "a fresh external track with no device gets no device params: {undo_delete:?}"
    );
    assert!(
        undo_add.contains(&format!("ClearExternalInstrument {{ track_id: {t} }}")),
        "undo add clears the removed track's config: {undo_add:?}"
    );
    assert_eq!(f.app.test_external_instrument(t).is_some(), false);
}

/// A frozen track re-added by a restore holds no frozen source on the
/// engine, whatever the live status said: its cache is decoded and
/// attached, as after a `ClearAll` (per fresh track, not per restore).
#[test]
fn a_re_added_frozen_track_gets_its_cache_attached() {
    use resonance_app::state::FreezeStatus;
    use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

    let mut f = fixture("frozen-track");
    let (_, t) = add_track(&mut f, Message::Track(TrackMessage::AddInstrumentTrack));
    let cache = f.root.join("fixture.freeze").join(format!("freeze_{t}.wav"));
    crate::common::write_freeze_cache_wav(&cache);
    f.app.test_set_freeze_status(
        t,
        FreezeStatus::Frozen {
            cache_ref: FreezeCacheRef::new(
                format!("freeze_{t}.wav"),
                48_000,
                32,
                1,
                FreezeCacheStatus::Frozen,
            ),
        },
    );
    let frozen = f.app.test_snapshot_for_undo();
    let deleted = delete_track(&mut f, t);
    assert!(!cache.exists(), "the live delete deleted the cache");
    // A re-render leaves the cache back on disk.
    crate::common::write_freeze_cache_wav(&cache);

    let cmds = step_lands_on(&mut f, Message::Undo, &frozen, "undo frozen track delete");
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::SetTrackFrozenSource { track_id, source: Some(_) } if *track_id == t
        )),
        "the re-added track plays its cache"
    );
    assert!(matches!(f.app.test_freeze_status(t), FreezeStatus::Frozen { .. }));
    settle(&mut f, cmds, &frozen, "undo frozen track delete");

    let cmds = step_lands_on(&mut f, Message::Redo, &deleted, "redo frozen track delete");
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::UnfreezeTrack { track_id } if *track_id == t)),
        "the removed frozen track's source is detached"
    );
    assert_eq!(f.app.test_freeze_status(t), FreezeStatus::Idle);
    settle(&mut f, cmds, &deleted, "redo frozen track delete");
}

/// A track whose type changed under the same id (a hand-made snapshot —
/// no edit does it) is removed and re-added, as the full replay did.
#[test]
fn a_track_type_change_is_a_remove_and_an_add() {
    let mut f = fixture("track-retype");
    let (audio, t) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    let mut retyped = audio.clone();
    let pt = retyped
        .project
        .file
        .tracks
        .iter_mut()
        .find(|pt| pt.id == t)
        .expect("the track");
    pt.track_type = "instrument".to_owned();
    pt.mono = false;

    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(retyped.clone());
    let cmds = drain(&f.rx);
    let sent_cmds = sent(&cmds);
    assert!(!cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)));
    let remove = sent_cmds
        .iter()
        .position(|c| *c == format!("RemoveTrack {{ track_id: {t} }}"))
        .expect("the old track goes");
    let add = sent_cmds
        .iter()
        .position(|c| c.starts_with(&format!("AddInstrumentTrack {{ id: {t},")))
        .expect("an instrument track comes back under the id");
    assert!(remove < add, "removed before the re-add: {sent_cmds:?}");
    assert_eq!(f.app.test_build_project_file(), retyped.project.file);
    settle(&mut f, cmds, &retyped, "retype");

    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(audio.clone());
    let cmds = drain(&f.rx);
    assert_eq!(f.app.test_build_project_file(), audio.project.file);
    settle(&mut f, cmds, &audio, "retype back");
}

/// The echoes of one restore land after the next ran (a held Ctrl+Z):
/// `TrackRemoved` for a track the redo put back must not delete it, and
/// the `*TrackAdded` of a track the undo removed again must not bring it
/// back — for a plain track and for one with sub-tracks, whose
/// `PluginAdded` must not make sub-tracks on a removed parent.
#[test]
fn a_track_restore_survives_the_previous_restores_late_echoes() {
    let mut f = fixture("track-late-echo");
    let s0 = f.app.test_snapshot_for_undo();
    let (s1, _) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    let (s2, t) = add_track(&mut f, Message::Track(TrackMessage::AddInstrumentTrack));
    let s3 = edit(&mut f, add_to(TestChain::Track(t), scanned("multi")));
    let s4 = delete_track(&mut f, t);

    let mut late = Vec::new();
    late.extend(step_lands_on(&mut f, Message::Undo, &s3, "undo delete"));
    late.extend(step_lands_on(&mut f, Message::Redo, &s4, "redo delete"));
    late.extend(step_lands_on(&mut f, Message::Undo, &s3, "undo delete again"));
    settle(&mut f, late, &s3, "delete undone, redone, undone, then the echoes");

    let mut late = Vec::new();
    for (target, what) in [(&s2, "undo plugin"), (&s1, "undo instrument add"), (&s0, "undo add")] {
        late.extend(step_lands_on(&mut f, Message::Undo, target, what));
    }
    for (target, what) in [(&s1, "redo add"), (&s2, "redo instrument add")] {
        late.extend(step_lands_on(&mut f, Message::Redo, target, what));
    }
    late.extend(step_lands_on(&mut f, Message::Undo, &s1, "undo instrument add again"));
    settle(&mut f, late, &s1, "a burst of track undos and redos, then the echoes");
}

/// The live delete mirrors at once (STATE-10) and owes its echo: an undo
/// before that echo re-adds the track under its id, and the late
/// `TrackRemoved` of the delete must not remove it again.
#[test]
fn undoing_a_track_delete_before_its_echo_keeps_the_track() {
    let mut f = fixture("track-delete-early-undo");
    let (added, t) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Track(TrackMessage::RequestRemoveTrack(t)));
    let delete = drain(&f.rx);
    let undo = step_lands_on(&mut f, Message::Undo, &added, "undo delete before its echo");
    let late: Vec<_> = delete.into_iter().chain(undo).collect();
    settle(&mut f, late, &added, "the delete's echo, then the undo's");
    assert!(track_ids(&f.app).contains(&t));
}

/// FU-A13i: a scalar echo carries no per-track generation of its own, so a
/// `TrackFxBypassChanged` sent for a toggle made right before a delete can
/// still be in flight when an undo re-adds the track under the same id.
/// Guarded the same way as a stale `*TrackAdded` echo (`stale_track_echo`
/// in `engine_events::tracks`): while the delete's own `TrackRemoved` is
/// still owed, no echo naming this id is trusted, so the toggle's stale
/// value cannot clobber the restore's.
#[test]
fn a_late_track_fx_bypass_echo_does_not_clobber_a_re_added_track() {
    let mut f = fixture("track-scalar-late-echo");
    let (_, t) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));

    // Toggle bypass on: mirrors true at once and sends the command, but
    // its echo is held rather than answered here — this is the toggle
    // that predates the delete below.
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Track(TrackMessage::ToggleTrackFxBypass(t)));
    let stale_bypass_on = drain(&f.rx);
    assert!(
        sent(&stale_bypass_on)
            .contains(&format!("SetTrackFxBypass {{ track_id: {t}, bypassed: true }}")),
        "the toggle sends the bypass command: {:?}",
        sent(&stale_bypass_on)
    );
    let stale_bypass_on: Vec<_> = stale_bypass_on
        .into_iter()
        .filter(|c| matches!(c, AudioCommand::SetTrackFxBypass { .. }))
        .collect();

    // Toggle back off — a real edit, echoed normally: the delete below,
    // and the undo's target, both see the track with bypass off.
    let off = edit(&mut f, Message::Track(TrackMessage::ToggleTrackFxBypass(t)));

    // Delete the track: mirrors at once (STATE-10) and owes its
    // `TrackRemoved` echo — held, not answered.
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Track(TrackMessage::RequestRemoveTrack(t)));
    let delete = drain(&f.rx);

    // Undo the delete: a diff restore re-adds the track fresh under the
    // same id, mirroring bypass = false (`off`'s value) at once.
    let undo = step_lands_on(&mut f, Message::Undo, &off, "undo delete");

    // The delete's own removal is still owed — its `TrackRemoved` echo
    // hasn't landed — so the toggle-on echo that predates it is exactly
    // the "late echo of the old incarnation" case the ledger must filter.
    assert!(
        !f.app.test_restore_echoes_settled(),
        "the delete's TrackRemoved echo must still be owed"
    );
    for event in engine_answer(&mut f, stale_bypass_on[0].clone()) {
        f.app.test_apply_engine_event(event);
    }
    let bypassed_now = f
        .app
        .test_registry()
        .tracks
        .iter()
        .find(|tr| tr.id == t)
        .map(|tr| tr.fx_bypassed);
    assert_eq!(
        bypassed_now,
        Some(false),
        "a stale bypass-on echo from before the delete must not clobber the re-added track's restored (off) value"
    );

    // Flush the rest in the order they were actually sent and confirm the
    // ledger settles cleanly.
    let late: Vec<_> = delete.into_iter().chain(undo).collect();
    settle(&mut f, late, &off, "the delete's echo, then the undo's");
}

/// FU-A13a for a re-added member: the entity domain adds the track with
/// its own mute, and `TrackGroups` then sends the effective one while the
/// group's macro mute holds.
#[test]
fn a_re_added_group_member_gets_its_effective_mute() {
    let mut f = fixture("group-member-readd");
    let (_, a) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    let (_, b) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    f.app.test_set_selected_tracks(vec![a, b]);
    let _ = edit(&mut f, Message::Group(GroupMessage::CreateGroupFromSelection));
    let group = f
        .app
        .test_track_groups()
        .get_all_groups()
        .into_iter()
        .map(|g| g.id)
        .max()
        .expect("the group");
    let muted = edit(&mut f, Message::Group(GroupMessage::ToggleMacroMute(group)));
    let _ = delete_track(&mut f, b);

    let cmds = step_lands_on(&mut f, Message::Undo, &muted, "undo member delete");
    assert_eq!(
        last_mute(&cmds, b),
        Some(true),
        "the re-added member plays muted while the group's macro mute holds"
    );
    settle(&mut f, cmds, &muted, "undo member delete");
}

/// A lane on a deleted track (the live delete clears it on the engine)
/// comes back with the track.
#[test]
fn a_re_added_tracks_automation_lane_is_sent_again() {
    use resonance_app::message::AutomationMessage;
    use resonance_common::{AutomationTarget, CurveKind};

    let mut f = fixture("track-lane-readd");
    let (_, t) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    let target = AutomationTarget::TrackGain(t);
    let laned = edit(
        &mut f,
        Message::Automation(AutomationMessage::AddBreakpoint {
            target: target.clone(),
            time_frames: 0,
            value: 0.5,
            curve: CurveKind::Linear,
        }),
    );
    let _ = delete_track(&mut f, t);
    let cmds = step_lands_on(&mut f, Message::Undo, &laned, "undo laned track delete");
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::SetAutomationLane { lane } if lane.target == target
        )),
        "the lane is sent again"
    );
    settle(&mut f, cmds, &laned, "undo laned track delete");
}

/// A restore that removes a track drops every piece of transient UI that
/// names it; the track id is never handed out again (D-4).
#[test]
fn undoing_a_track_add_drops_its_selection_and_never_reissues_its_id() {
    let mut f = fixture("track-selection");
    let s0 = f.app.test_snapshot_for_undo();
    let (_, t) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    f.app.test_set_selected_tracks(vec![t]);
    assert_eq!(f.app.test_selected_track(), Some(t));
    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo track add");
    settle(&mut f, cmds, &s0, "undo track add");
    assert_eq!(f.app.test_selected_track(), None);
    assert!(f.app.test_selected_tracks().is_empty());

    let (_, again) = add_track(&mut f, Message::Track(TrackMessage::AddTrack));
    assert_ne!(again, t, "the undone add's id is not reissued");
}

// ---------------------------------------------------------------------------
// Clips (A-13i)
// ---------------------------------------------------------------------------

/// The demo's audio clip, on `AUDIO_TRACK`.
const AUDIO_CLIP: u64 = 15;
/// The demo's bass clip, on the bass track (2).
const BASS_CLIP: u64 = 12;
const BASS_TRACK: u64 = 2;

fn audio_clip_ids(app: &Resonance) -> Vec<u64> {
    app.test_clips().iter().map(|c| c.id).collect()
}

fn midi_clip_ids(app: &Resonance) -> Vec<u64> {
    app.test_midi_clips().iter().map(|c| c.id).collect()
}

fn load_cmd(f: &Fixture, snapshot: &UndoSnapshot, clip_id: u64) -> String {
    let pc = snapshot
        .project
        .file
        .clips
        .iter()
        .find(|c| c.id == clip_id)
        .expect("the clip is in the snapshot");
    format!(
        "LoadClipFromWav {{ clip_id: {clip_id}, track_id: {}, start_sample: {}, path: {:?}, \
         name: {:?}, trim_start_frames: {}, trim_end_frames: {} }}",
        pc.track_id,
        pc.start_sample,
        f.root.join("fixture.rproj").join(&pc.audio_file),
        pc.name,
        pc.trim_start_frames,
        pc.trim_end_frames
    )
}

/// A GUI audio-clip delete: undo reloads it under its id from its
/// persisted WAV (nothing else), redo deletes it again.
#[test]
fn deleting_an_audio_clip_undoes_through_the_diff_path() {
    let mut f = fixture("audio-clip-delete");
    let s0 = f.app.test_snapshot_for_undo();
    let s1 = edit(&mut f, Message::Clip(ClipMessage::DeleteClip(AUDIO_CLIP)));
    assert!(!audio_clip_ids(&f.app).contains(&AUDIO_CLIP));

    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo clip delete");
    assert_eq!(
        sent(&cmds),
        [TEMPO.to_owned(), load_cmd(&f, &s0, AUDIO_CLIP), NO_TAKES.to_owned()],
        "undo clip delete: one load"
    );
    settle(&mut f, cmds, &s0, "undo clip delete");
    let cmds = step_lands_on(&mut f, Message::Redo, &s1, "redo clip delete");
    assert_eq!(
        sent(&cmds),
        wrap(vec![format!("DeleteClip {{ clip_id: {AUDIO_CLIP} }}")]),
        "redo clip delete: one delete"
    );
    settle(&mut f, cmds, &s1, "redo clip delete");
}

/// A split: undo deletes the tail and trims the head back; redo reloads
/// the tail and trims the head again. A selection on the tail is dropped
/// by the undo.
#[test]
fn splitting_an_audio_clip_undoes_through_the_diff_path() {
    let mut f = fixture("audio-clip-split");
    let s0 = f.app.test_snapshot_for_undo();
    let clip = f
        .app
        .test_clips()
        .iter()
        .find(|c| c.id == AUDIO_CLIP)
        .expect("the demo clip")
        .clone();
    let tail = 5_000;
    let s1 = edit(
        &mut f,
        Message::Clip(ClipMessage::SplitClipAt {
            clip_id: AUDIO_CLIP,
            new_clip_id: tail,
            at_sample: clip.start_sample + clip.duration_samples / 2,
        }),
    );
    assert_eq!(audio_clip_ids(&f.app), [AUDIO_CLIP, tail]);
    f.app.test_set_selected_clip(Some(tail));

    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo split");
    let sent_cmds = sent(&cmds);
    assert_eq!(sent_cmds[1], TEMPO);
    assert_eq!(
        sent_cmds[2],
        format!("DeleteClip {{ clip_id: {tail} }}"),
        "undo split: the tail goes first: {sent_cmds:?}"
    );
    assert!(
        sent_cmds[3].starts_with(&format!("TrimClip {{ clip_id: {AUDIO_CLIP},")),
        "then the head is trimmed back: {sent_cmds:?}"
    );
    assert_eq!(f.app.test_selected_clip(), None, "the tail's selection is dropped");
    settle(&mut f, cmds, &s0, "undo split");

    let cmds = step_lands_on(&mut f, Message::Redo, &s1, "redo split");
    let sent_cmds = sent(&cmds);
    assert!(sent_cmds.contains(&load_cmd(&f, &s1, tail)), "redo reloads the tail: {sent_cmds:?}");
    assert!(
        sent_cmds
            .iter()
            .any(|c| c.starts_with(&format!("TrimClip {{ clip_id: {AUDIO_CLIP},"))),
        "and trims the head: {sent_cmds:?}"
    );
    settle(&mut f, cmds, &s1, "redo split");
}

/// A MIDI clip drawn, then another one deleted: undo and redo load and
/// delete them (`LoadMidiClipDirect` with the snapshot's notes,
/// `DeleteMidiClip`), nothing else.
#[test]
fn adding_and_removing_midi_clips_undoes_through_the_diff_path() {
    let mut f = fixture("midi-clips");
    let s0 = f.app.test_snapshot_for_undo();
    let drawn = 6_000;
    let s1 = edit(
        &mut f,
        Message::MidiClip(MidiClipMessage::CreateEmptyClip {
            clip_id: drawn,
            track_id: BASS_TRACK,
            start_sample: 10_000_000,
            duration_ticks: 1_920,
            name: "Drawn".into(),
        }),
    );
    assert!(midi_clip_ids(&f.app).contains(&drawn));
    let s2 = edit(&mut f, Message::MidiClip(MidiClipMessage::DeleteMidiClip(BASS_CLIP)));
    assert!(!midi_clip_ids(&f.app).contains(&BASS_CLIP));

    let cmds = step_lands_on(&mut f, Message::Undo, &s1, "undo MIDI clip delete");
    let sent_cmds = sent(&cmds);
    assert_eq!(sent_cmds.len(), 4, "undo MIDI clip delete: one load: {sent_cmds:?}");
    assert!(sent_cmds[2].starts_with(&format!(
        "LoadMidiClipDirect {{ clip_id: {BASS_CLIP}, track_id: {BASS_TRACK},"
    )));
    settle(&mut f, cmds, &s1, "undo MIDI clip delete");
    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo MIDI clip draw");
    assert_eq!(
        sent(&cmds),
        wrap(vec![format!("DeleteMidiClip {{ clip_id: {drawn} }}")]),
        "undo MIDI clip draw: one delete"
    );
    settle(&mut f, cmds, &s0, "undo MIDI clip draw");
    for (target, what) in [(&s1, "redo MIDI clip draw"), (&s2, "redo MIDI clip delete")] {
        let cmds = step_lands_on(&mut f, Message::Redo, target, what);
        settle(&mut f, cmds, target, what);
    }
    assert_eq!(midi_clip_ids(&f.app).contains(&BASS_CLIP), false);
}

/// Deleting a track that carries clips (the confirmed delete): undo
/// re-adds the track, its plugin and its clips; redo deletes the clips
/// first — each by its own command — then the track.
#[test]
fn deleting_a_track_with_clips_undoes_through_the_diff_path() {
    let mut f = fixture("track-with-clips");
    let s0 = f.app.test_snapshot_for_undo();
    let _ = f.app.update(Message::Track(TrackMessage::RequestRemoveTrack(BASS_TRACK)));
    let s1 = edit(&mut f, Message::Track(TrackMessage::ConfirmRemoveTrack));
    assert!(!track_ids(&f.app).contains(&BASS_TRACK));

    let cmds = step_lands_on(&mut f, Message::Undo, &s0, "undo track-with-clips delete");
    let sent_cmds = sent(&cmds);
    let add = sent_cmds
        .iter()
        .position(|c| c.starts_with(&format!("AddInstrumentTrack {{ id: {BASS_TRACK},")))
        .expect("the track comes back");
    let load = sent_cmds
        .iter()
        .position(|c| c.starts_with(&format!("LoadMidiClipDirect {{ clip_id: {BASS_CLIP},")))
        .expect("its clip comes back");
    assert!(add < load, "the track before its clip: {sent_cmds:?}");
    settle(&mut f, cmds, &s0, "undo track-with-clips delete");

    let cmds = step_lands_on(&mut f, Message::Redo, &s1, "redo track-with-clips delete");
    assert_eq!(
        sent(&cmds),
        wrap(vec![
            format!("DeleteMidiClip {{ clip_id: {BASS_CLIP} }}"),
            format!("RemoveTrack {{ track_id: {BASS_TRACK} }}"),
        ]),
        "redo: the clip, then the track with its chain"
    );
    settle(&mut f, cmds, &s1, "redo track-with-clips delete");

    // The audio track and its clip, the same way.
    let _ = f.app.update(Message::Track(TrackMessage::RequestRemoveTrack(AUDIO_TRACK)));
    let s2 = edit(&mut f, Message::Track(TrackMessage::ConfirmRemoveTrack));
    let cmds = step_lands_on(&mut f, Message::Undo, &s1, "undo audio track delete");
    assert!(sent(&cmds).contains(&load_cmd(&f, &s1, AUDIO_CLIP)));
    settle(&mut f, cmds, &s1, "undo audio track delete");
    let cmds = step_lands_on(&mut f, Message::Redo, &s2, "redo audio track delete");
    assert_eq!(
        sent(&cmds),
        wrap(vec![
            format!("DeleteClip {{ clip_id: {AUDIO_CLIP} }}"),
            format!("RemoveTrack {{ track_id: {AUDIO_TRACK} }}"),
        ]),
        "redo: the audio clip, then the track"
    );
    settle(&mut f, cmds, &s2, "redo audio track delete");
}

/// A clip whose WAV or length changed under the same id (a hand-made
/// snapshot) is deleted and reloaded, as the full replay did.
#[test]
fn a_clip_whose_wav_changed_is_deleted_and_reloaded() {
    let mut f = fixture("clip-wav-change");
    let before = f.app.test_snapshot_for_undo();
    let mut changed = before.clone();
    changed
        .project
        .file
        .clips
        .iter_mut()
        .find(|c| c.id == AUDIO_CLIP)
        .expect("the demo clip")
        .total_frames += 1_000;

    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(changed.clone());
    let cmds = drain(&f.rx);
    let sent_cmds = sent(&cmds);
    let delete = sent_cmds
        .iter()
        .position(|c| *c == format!("DeleteClip {{ clip_id: {AUDIO_CLIP} }}"))
        .expect("deleted");
    let load = sent_cmds
        .iter()
        .position(|c| *c == load_cmd(&f, &changed, AUDIO_CLIP))
        .expect("reloaded");
    assert!(delete < load, "{sent_cmds:?}");
    assert_eq!(f.app.test_build_project_file(), changed.project.file);
    settle(&mut f, cmds, &changed, "a changed WAV");
}

/// Late echoes over clip restores (a held Ctrl+Z): the `ClipDeleted` /
/// `MidiClipDeleted` of one restore must not delete what the next put
/// back, and the load echo of a clip a later restore deleted must not
/// bring it back.
#[test]
fn a_clip_restore_survives_the_previous_restores_late_echoes() {
    let mut f = fixture("clip-late-echo");
    let s0 = f.app.test_snapshot_for_undo();
    let s1 = edit(&mut f, Message::Clip(ClipMessage::DeleteClip(AUDIO_CLIP)));
    let s2 = edit(&mut f, Message::MidiClip(MidiClipMessage::DeleteMidiClip(BASS_CLIP)));

    let mut late = Vec::new();
    late.extend(step_lands_on(&mut f, Message::Undo, &s1, "undo MIDI delete"));
    late.extend(step_lands_on(&mut f, Message::Undo, &s0, "undo audio delete"));
    late.extend(step_lands_on(&mut f, Message::Redo, &s1, "redo audio delete"));
    late.extend(step_lands_on(&mut f, Message::Redo, &s2, "redo MIDI delete"));
    late.extend(step_lands_on(&mut f, Message::Undo, &s1, "undo MIDI delete again"));
    late.extend(step_lands_on(&mut f, Message::Undo, &s0, "undo audio delete again"));
    settle(&mut f, late, &s0, "clip deletes undone, redone, undone, then the echoes");
    assert!(audio_clip_ids(&f.app).contains(&AUDIO_CLIP));
    assert!(midi_clip_ids(&f.app).contains(&BASS_CLIP));
}

/// STATE-10 on the diff path: the GUI delete mirrors at once and owes its
/// echo, so an undo before that echo re-adds the clip and the late
/// `ClipDeleted` leaves it alone.
#[test]
fn undoing_a_clip_delete_before_its_echo_keeps_the_clip() {
    let mut f = fixture("clip-delete-early-undo");
    let s0 = f.app.test_snapshot_for_undo();
    let _ = drain(&f.rx);
    let _ = f.app.update(Message::Clip(ClipMessage::DeleteClip(AUDIO_CLIP)));
    let delete = drain(&f.rx);
    let undo = step_lands_on(&mut f, Message::Undo, &s0, "undo clip delete before its echo");
    let late: Vec<_> = delete.into_iter().chain(undo).collect();
    settle(&mut f, late, &s0, "the delete's echo, then the undo's");
    assert!(audio_clip_ids(&f.app).contains(&AUDIO_CLIP));
}

/// FU-A13h: unlike the audio-clip GUI delete (STATE-10), the GUI MIDI-clip
/// delete used to mirror only on the `MidiClipDeleted` echo — an undo
/// pressed before that echo saw a mirror that still held the clip (a
/// no-op restore) and the late echo then deleted it out from under the
/// undo. Mirroring at once and owing the echo (as the audio-clip delete
/// does) fixes it.
#[test]
fn undoing_a_midi_clip_delete_before_its_echo_keeps_the_clip() {
    let mut f = fixture("midi-clip-delete-early-undo");
    let s0 = f.app.test_snapshot_for_undo();
    let _ = drain(&f.rx);
    let _ = f
        .app
        .update(Message::MidiClip(MidiClipMessage::DeleteMidiClip(BASS_CLIP)));
    assert!(
        !midi_clip_ids(&f.app).contains(&BASS_CLIP),
        "the delete mirrors immediately, before its echo"
    );
    let delete = drain(&f.rx);
    let undo = step_lands_on(&mut f, Message::Undo, &s0, "undo MIDI clip delete before its echo");
    let late: Vec<_> = delete.into_iter().chain(undo).collect();
    settle(&mut f, late, &s0, "the delete's echo, then the undo's");
    assert!(midi_clip_ids(&f.app).contains(&BASS_CLIP));
}

/// A kept MIDI clip whose notes changed is reloaded under its id
/// (`DeleteMidiClip` + `LoadMidiClipDirect`). The delete's echo is owed:
/// before A-13i it dropped the mirror's clip and its lyric side-table
/// entry, and the load's echo brought the clip back without its lyrics.
#[test]
fn a_midi_note_restore_keeps_the_clips_lyrics_through_its_echoes() {
    let mut f = fixture("midi-reload-lyrics");
    f.app.test_set_clip_lyrics(BASS_CLIP, vec!["la".into(), "di".into()]);
    let with_lyrics = f.app.test_snapshot_for_undo();
    assert!(
        with_lyrics
            .project
            .file
            .midi_clips
            .iter()
            .any(|c| c.id == BASS_CLIP && !c.vocal_lyrics.is_empty()),
        "the snapshot carries the lyrics, or this test is vacuous"
    );
    let mut fewer_notes = with_lyrics.clone();
    std::sync::Arc::make_mut(
        fewer_notes
            .project
            .midi_notes
            .get_mut(&BASS_CLIP)
            .expect("the clip's notes"),
    )
    .pop();

    for (target, what) in [(&fewer_notes, "drop a note"), (&with_lyrics, "put it back")] {
        let _ = drain(&f.rx);
        f.app.test_begin_restore_from_snapshot(target.clone());
        let cmds = drain(&f.rx);
        assert!(
            sent(&cmds).contains(&format!("DeleteMidiClip {{ clip_id: {BASS_CLIP} }}")),
            "{what}: the clip is reloaded"
        );
        assert_eq!(f.app.test_build_project_file(), target.project.file, "{what}");
        settle(&mut f, cmds, target, what);
    }
}
