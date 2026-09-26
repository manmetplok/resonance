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
use resonance_app::message::{
    BusMessage, GroupMessage, MarkerMessage, MasterMessage, Message, MixerMessage, PluginMessage,
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
/// the live engine does, and every other command below with its echo
/// (the adds, removals and moves of busses and plugins, sends and key
/// routes — A-13h). Until the engine goes quiet: an echo handler may send
/// commands of its own (a bus removal's send cleanup).
fn echo_midi_clip_loads(app: &mut Resonance, rx: &Receiver<AudioCommand>) {
    echo(app, rx, drain(rx));
}

fn echo(app: &mut Resonance, rx: &Receiver<AudioCommand>, mut cmds: Vec<AudioCommand>) {
    while !cmds.is_empty() {
        for cmd in cmds {
            if let Some(event) = echo_of(cmd) {
                app.test_apply_engine_event(event);
            }
        }
        cmds = drain(rx);
    }
}

/// The event the engine answers `cmd` with, for the commands these tests
/// drive. The removals and moves are answered for any id, as the engine
/// does.
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
        } => AudioEvent::PluginAdded {
            track_id,
            instance_id: id,
            plugin_name: clap_plugin_id.clone(),
            clap_plugin_id,
            clap_file_path,
            params: plugin_params(),
            has_gui: false,
            has_sidechain_input: true,
            output_port_count: 1,
            output_port_names: vec!["Main".to_owned()],
        },
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
        _ => return None,
    })
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
// Busses and plugin instances (A-13h)
// ---------------------------------------------------------------------------

/// Echo what a step sent, then assert the restore still sits on `target`
/// and owes the engine nothing: the echoes of its own adds, removals and
/// moves change nothing.
fn settle(f: &mut Fixture, cmds: Vec<AudioCommand>, target: &UndoSnapshot, what: &str) {
    echo(&mut f.app, &f.rx, cmds);
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
