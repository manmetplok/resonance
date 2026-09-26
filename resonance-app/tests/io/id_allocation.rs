//! Entity ids never collide across the app's allocators, or with the
//! engine's own remaining ones (ARCH-04 A4-1, updated for D-1/D-2/D-3/D-4).
//!
//! The app allocates the ids it needs synchronously (control-API adds, FX
//! returns, sub-tracks, track groups) from the bases named in
//! `resonance_app::state::ids`. This pins the partition that once kept an
//! app range and an engine range apart, so the epic that moved ownership
//! to the app (A4-4) could migrate one space at a time against a test:
//! the demo project gets one of each entity added through the GUI path
//! *and* through the control path, is saved, reloaded, and gets the same
//! adds again; every id space must still be a set afterwards, and every
//! app counter must sit above every id its space holds.
//!
//! **Plugins (D-1), sends (D-2), busses (D-3) and tracks (D-4) landed**:
//! there is no more engine-vs-app split for any of the four. Every add —
//! GUI or control, track/bus/master for plugins — now allocates through
//! `Resonance::allocate_plugin_id` / `AuxSendState::allocate_send_id` /
//! `TrackRegistry::allocate_bus_id` / `Resonance::allocate_track_id`, and
//! [`FakeEngine::plugin`] / [`FakeEngine::bus`] / [`FakeEngine::send`] /
//! [`FakeEngine::track`] just echo the id they were given straight back,
//! the way the real handlers do — no engine counter, no hint left
//! anywhere. Busses are the one exception with a base at all: `BUS_ID_BASE`
//! stays, but it is no longer an engine-agreed range — the engine has
//! nothing left to keep it clear of. It exists because `song.summary` /
//! `song.tracks` list tracks and busses in ONE id-addressed sequence
//! (`state/ids.rs` has the story), which is also why
//! `Resonance::allocate_track_id` still `debug_assert`s every track id it
//! hands out stays below it, even though tracks have no base of their own
//! any more.

use std::collections::HashSet;
use std::path::PathBuf;

use resonance_app::compose::ComposeMessage;
use resonance_app::message::*;
use resonance_app::project;
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::ids::{BUS_ID_BASE, DERIVED_CLIP_ID_BASE};
use resonance_app::{demo, Resonance, TestChain};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ClipId, MidiNote, ScannedPlugin, SendSource, TrackType,
};

use crate::common::call;

// ---------------------------------------------------------------------------
// The engine's allocators, as the engine implements them
// ---------------------------------------------------------------------------

/// The engine-side counter and hint rule the one remaining space
/// (clips) still applies, plus the pass-through "echo" for every space
/// that has already lost its engine-side counter, so the echoes this
/// test plays back carry exactly the ids the live engine would.
struct FakeEngine {
    next_clip: u64,
}

impl FakeEngine {
    fn new() -> Self {
        Self { next_clip: 1 }
    }

    /// `engine/midi/clips.rs`, `engine/clips.rs` (FU-A6a): a clip id
    /// handed in (`LoadMidiClipDirect`, `LoadClipFromWav`) raises the
    /// counter only when it is below the app's derived-clip base.
    fn clip(&mut self, id: Option<u64>) -> u64 {
        match id {
            Some(h) => {
                if h < DERIVED_CLIP_ID_BASE {
                    self.next_clip = self.next_clip.max(h + 1);
                }
                h
            }
            None => {
                let i = self.next_clip;
                self.next_clip += 1;
                i
            }
        }
    }

    /// D-1: the engine has no plugin-id counter left. It honours
    /// whatever id `AddPlugin`/`AddPluginToBus`/`AddPluginToMaster`
    /// carries, full stop — this just plays that back as the echo.
    fn plugin(&mut self, id: u64) -> u64 {
        id
    }

    /// D-3: the engine has no bus-id counter left. It honours whatever id
    /// `AddBus` carries, full stop.
    fn bus(&mut self, id: u64) -> u64 {
        id
    }

    /// D-2: the engine has no send-id counter left. It honours whatever
    /// id `AddAuxSend`/`SetAuxSend` carries, full stop.
    fn send(&mut self, id: u64) -> u64 {
        id
    }

    /// D-4: the engine has no track-id counter left. It honours whatever
    /// id `AddTrack`/`AddInstrumentTrack`/`AddVocalTrack` carries, full
    /// stop.
    fn track(&mut self, id: u64) -> u64 {
        id
    }
}

/// Drain the captured commands and play back the echo the engine would
/// send for every add, so the mirror sees engine-allocated ids exactly
/// as it does live.
fn echo(app: &mut Resonance, rx: &Receiver<AudioCommand>, engine: &mut FakeEngine) {
    let commands: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    for cmd in commands {
        match cmd {
            AudioCommand::AddTrack { id, .. } => {
                let track_id = engine.track(id);
                app.test_apply_engine_event(AudioEvent::TrackAdded { track_id });
            }
            AudioCommand::AddInstrumentTrack { id, .. } => {
                let track_id = engine.track(id);
                app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id });
            }
            AudioCommand::AddVocalTrack { id, .. } => {
                let track_id = engine.track(id);
                app.test_apply_engine_event(AudioEvent::VocalTrackAdded { track_id });
            }
            AudioCommand::CreateSubTrack { .. } => {
                // No echo; the mirror names sub-tracks itself, and (D-4)
                // the engine has no counter left to learn the id for.
            }
            AudioCommand::AddBus { id, name } => {
                let bus_id = engine.bus(id);
                app.test_apply_engine_event(AudioEvent::BusAdded {
                    bus_id,
                    name: name.unwrap_or_else(|| format!("Bus {bus_id}")),
                });
            }
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
            } => {
                let send_id = engine.send(id);
                app.test_apply_engine_event(AudioEvent::AuxSendChanged {
                    send_id,
                    source,
                    dest,
                    level_db,
                    pre_fader,
                    enabled,
                });
            }
            AudioCommand::AddPlugin {
                track_id,
                clap_file_path,
                clap_plugin_id,
                id,
                ..
            } => {
                let instance_id = engine.plugin(id);
                app.test_apply_engine_event(AudioEvent::PluginAdded {
                    track_id,
                    instance_id,
                    plugin_name: clap_plugin_id.clone(),
                    clap_plugin_id,
                    clap_file_path,
                    params: Vec::new(),
                    has_gui: false,
                    has_sidechain_input: false,
                    output_port_count: 1,
                    output_port_names: vec!["Main".to_owned()],
                });
            }
            AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path,
                clap_plugin_id,
                id,
            } => {
                let instance_id = engine.plugin(id);
                app.test_apply_engine_event(AudioEvent::BusPluginAdded {
                    bus_id,
                    instance_id,
                    plugin_name: clap_plugin_id.clone(),
                    clap_plugin_id,
                    clap_file_path,
                    params: Vec::new(),
                    has_gui: false,
                    has_sidechain_input: false,
                });
            }
            AudioCommand::AddPluginToMaster {
                clap_file_path,
                clap_plugin_id,
                id,
            } => {
                let instance_id = engine.plugin(id);
                app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
                    instance_id,
                    plugin_name: clap_plugin_id.clone(),
                    clap_plugin_id,
                    clap_file_path,
                    params: Vec::new(),
                    has_gui: false,
                    has_sidechain_input: false,
                });
            }
            AudioCommand::CreateMidiClip {
                track_id,
                start_sample,
                duration_ticks,
                name,
            } => {
                let clip_id = engine.clip(None);
                app.test_apply_engine_event(AudioEvent::MidiClipCreated {
                    clip_id,
                    track_id,
                    start_sample,
                    duration_ticks,
                    name,
                    notes: Vec::new(),
                    trim_start_ticks: 0,
                    trim_end_ticks: 0,
                });
            }
            AudioCommand::LoadMidiClipDirect {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                notes,
                name,
                trim_start_ticks,
                trim_end_ticks,
            } => {
                let clip_id = engine.clip(Some(clip_id));
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
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    engine: FakeEngine,
    project: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.project.parent().unwrap());
    }
}

const EQ: &str = "com.resonance.eq";

/// The demo project in a capturing app with a saved path (so control
/// adds are accepted and undo records), a plugin catalog holding one
/// effect for `track.add_effect`, and the engine's counters advanced
/// past everything the demo seeded — as they would be after the demo's
/// own adds went through the engine.
fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "resonance-id-allocation-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("fixture.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("create project dir");

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    demo::seed_demo_content(&mut app);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: EQ.to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        }],
    });
    let mut engine = FakeEngine::new();
    echo(&mut app, &rx, &mut engine);
    engine.next_clip = app
        .test_midi_clips()
        .iter()
        .map(|c| c.id)
        .filter(|id| *id < DERIVED_CLIP_ID_BASE)
        .max()
        .unwrap_or(0)
        + 1;
    // No `next_plugin`/`next_bus`/`next_send`/`next_track` to advance:
    // D-1/D-3/D-2/D-4 deleted the engine's counters for all four, so
    // `FakeEngine::plugin` / `::bus` / `::send` / `::track` have nothing
    // to seed.
    app.test_set_active_project(true);
    app.test_set_project_path(project.clone());
    Fixture {
        app,
        rx,
        engine,
        project,
    }
}

// ---------------------------------------------------------------------------
// Every id the app holds, per space
// ---------------------------------------------------------------------------

/// The ids in each space as the mirror holds them, in slot order, with
/// duplicates preserved so a collision shows up as a repeated entry.
struct LiveIds {
    tracks: Vec<u64>,
    groups: Vec<u64>,
    busses: Vec<u64>,
    sends: Vec<u64>,
    plugins: Vec<u64>,
    markers: Vec<u64>,
}

fn live_ids(app: &Resonance) -> LiveIds {
    let registry = app.test_registry();
    let tracks: Vec<u64> = registry.tracks.iter().map(|t| t.id).collect();
    let busses: Vec<u64> = registry.busses.iter().map(|b| b.id).collect();
    let mut plugins: Vec<u64> = Vec::new();
    for t in &tracks {
        plugins.extend(app.test_chain_slots(TestChain::Track(*t)).iter().map(|s| s.0));
    }
    for b in &busses {
        plugins.extend(app.test_chain_slots(TestChain::Bus(*b)).iter().map(|s| s.0));
    }
    plugins.extend(app.test_chain_slots(TestChain::Master).iter().map(|s| s.0));
    LiveIds {
        tracks,
        groups: app.test_track_groups().get_all_groups().iter().map(|g| g.id).collect(),
        busses,
        sends: app.test_aux_sends().iter().map(|s| s.id).collect(),
        plugins,
        markers: app.test_markers().markers.iter().map(|m| m.id).collect(),
    }
}

fn assert_set(space: &str, ids: &[u64]) {
    let unique: HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "{space} ids repeat: {ids:?}");
}

/// Every id space is a set, tracks and groups together (they share one
/// space), and every app-side counter sits above every id in its range.
fn assert_partition_holds(app: &Resonance, when: &str) {
    let ids = live_ids(app);
    assert_set(&format!("{when}: track"), &ids.tracks);
    assert_set(&format!("{when}: group"), &ids.groups);
    let tracks_and_groups: Vec<u64> = ids.tracks.iter().chain(&ids.groups).copied().collect();
    assert_set(&format!("{when}: track+group"), &tracks_and_groups);
    assert_set(&format!("{when}: bus"), &ids.busses);
    assert_set(&format!("{when}: send"), &ids.sends);
    assert_set(&format!("{when}: plugin"), &ids.plugins);
    assert_set(&format!("{when}: marker"), &ids.markers);

    let registry = app.test_registry();
    let above = |counter: u64, base: u64, held: &[u64], space: &str| {
        assert!(counter >= base, "{when}: {space} counter {counter} below its base {base}");
        for id in held.iter().filter(|id| **id >= base) {
            assert!(
                counter > *id,
                "{when}: {space} counter {counter} would re-issue held id {id}"
            );
        }
    };
    // Tracks (ARCH-04 D-4) have no base of their own any more, but
    // `demo::seed_demo_content` bumps `next_track_id` past every
    // hand-picked track/group id it seeds (unlike the plugin/send demo
    // fixtures below), so the "counter sits above every held id" check
    // still holds — for every held id, not just ones above some range,
    // hence `base = 0`.
    above(registry.next_track_id, 0, &tracks_and_groups, "track");
    // Busses kept their base (ARCH-04 D-3) even though the engine no
    // longer agrees to it — see `state/ids.rs`: it is the control API's
    // combined track+bus listing, not the engine, that needs bus ids kept
    // off track ids. So the same "counter sits above every held id in its
    // range" check as tracks still applies.
    above(registry.next_bus_id, BUS_ID_BASE, &ids.busses, "bus");
    // D-1/D-2: no base and no "counter stays above every held id"
    // invariant for plugins or sends any more — the app is the sole
    // allocator for each, but an id can also land in the mirror without
    // ever passing through the counter (`demo::seed_demo_content`'s
    // fixture ids, a project loaded with ids from a session that ran
    // ahead of this one's counter). What `allocate_plugin_id` /
    // `allocate_send_id` actually guarantee is that their OWN in-use scan
    // never hands out an id already live, regardless of the counter's
    // position — which is exactly what the `assert_set` above is for: any
    // real collision shows up there as a repeated id the moment a
    // scan-protected allocation coexists with an out-of-band one, which
    // is precisely what `add_round`'s GUI + control adds onto the demo's
    // seeded content already drives.
    assert!(
        app.compose_state().next_derived_clip_id >= DERIVED_CLIP_ID_BASE,
        "{when}: derived-clip counter below its base"
    );
}

// ---------------------------------------------------------------------------
// One round of adds: each entity once through the GUI, once via control
// ---------------------------------------------------------------------------

fn add_round(f: &mut Fixture, round: &str) {
    let Fixture {
        app, rx, engine, ..
    } = f;

    // Tracks: GUI and control — both draw from the SAME app allocator now
    // (ARCH-04 D-4), so the id spaces mixing here is the point; before
    // D-4 the GUI ones would have come from the engine's own counter
    // instead. Identified by set difference, same reasoning as `gui_bus`
    // below: there is no more range to distinguish a GUI track from a
    // control one by value alone.
    let tracks_before_gui_add: HashSet<u64> =
        app.test_registry().tracks.iter().map(|t| t.id).collect();
    app.test_dispatch(Message::Track(TrackMessage::AddTrack));
    app.test_dispatch(Message::Track(TrackMessage::AddInstrumentTrack));
    echo(app, rx, engine);
    let gui_tracks: Vec<u64> = app
        .test_registry()
        .tracks
        .iter()
        .map(|t| t.id)
        .filter(|id| !tracks_before_gui_add.contains(id))
        .collect();
    assert_eq!(gui_tracks.len(), 2, "both GUI track adds landed: {gui_tracks:?}");
    let control_track = call(
        app,
        "track.add",
        serde_json::json!({ "kind": "audio", "name": format!("{round} control audio") }),
    )
    .result::<serde_json::Value>()
    .expect("track.add succeeds")["track_id"]
        .as_u64()
        .expect("track.add returns the id");
    echo(app, rx, engine);
    let _: serde_json::Value = call(
        app,
        "track.add",
        serde_json::json!({ "kind": "instrument", "name": format!("{round} control synth") }),
    )
    .result()
    .expect("track.add (instrument) succeeds");
    echo(app, rx, engine);

    // Busses: GUI and control (ARCH-04 D-3) — both draw from the SAME app
    // allocator now, seeded at `BUS_ID_BASE`, so the id spaces mixing
    // here is the point; before D-3 the GUI one would have come from the
    // engine's own counter instead. Identified by set difference rather
    // than `.max()` — belt and braces against the demo's hand-picked, far
    // lower bus ids (100, 101) ever being mistaken for a fresh one, the
    // way relying on `.max()` here once did for a `next_bus_id` that
    // briefly started at 1 (ARCH-04 D-3's first cut, before `BUS_ID_BASE`
    // came back).
    let busses_before_gui_add: HashSet<u64> =
        app.test_registry().busses.iter().map(|b| b.id).collect();
    app.test_dispatch(Message::Bus(BusMessage::AddBus));
    echo(app, rx, engine);
    let gui_bus = app
        .test_registry()
        .busses
        .iter()
        .map(|b| b.id)
        .find(|id| !busses_before_gui_add.contains(id))
        .expect("the GUI add landed a new bus");
    let control_bus = call(
        app,
        "bus.create",
        serde_json::json!({ "name": format!("{round} control bus") }),
    )
    .result::<serde_json::Value>()
    .expect("bus.create succeeds")["bus_id"]
        .as_u64()
        .expect("bus.create returns the id");
    assert!(control_bus >= BUS_ID_BASE, "control busses come from the app range");
    echo(app, rx, engine);

    // Plugins: GUI (engine-allocated) onto the control track, control
    // (app-allocated) onto the same track.
    app.test_dispatch(Message::Plugin(PluginMessage::AddPluginToTrack(
        control_track,
        ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: EQ.to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
    )));
    echo(app, rx, engine);
    let _: serde_json::Value = call(
        app,
        "track.add_effect",
        serde_json::json!({ "track_id": control_track, "plugin_id": EQ }),
    )
    .result()
    .expect("track.add_effect succeeds");
    echo(app, rx, engine);

    // Plugins on a bus and on master, GUI and control (ARCH-04 D-1):
    // every one of these now draws from the SAME app allocator as the
    // track adds above, so the id spaces mixing here is the point —
    // before D-1 the GUI ones would have come from the engine's own
    // counter instead.
    app.test_dispatch(Message::Bus(BusMessage::AddPluginToBus(
        gui_bus,
        ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: EQ.to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
    )));
    echo(app, rx, engine);
    let _: serde_json::Value = call(
        app,
        "bus.add_effect",
        serde_json::json!({ "bus_id": control_bus, "plugin_id": EQ }),
    )
    .result()
    .expect("bus.add_effect succeeds");
    echo(app, rx, engine);
    app.test_dispatch(Message::Master(MasterMessage::AddPluginToMaster(ScannedPlugin {
        clap_file_path: "/plugins/eq.clap".to_owned(),
        clap_plugin_id: EQ.to_owned(),
        name: "Resonance EQ".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        ..Default::default()
    })));
    echo(app, rx, engine);
    let _: serde_json::Value = call(app, "master.add_effect", serde_json::json!({ "plugin_id": EQ }))
        .result()
        .expect("master.add_effect succeeds");
    echo(app, rx, engine);

    // Sends: GUI and control (ARCH-04 D-2) — both draw from the SAME app
    // allocator now, from the control track into the two new busses.
    app.test_dispatch(Message::Mixer(MixerMessage::AddSend {
        source: SendSource::Track(control_track),
        dest: gui_bus,
    }));
    echo(app, rx, engine);
    let _: serde_json::Value = call(
        app,
        "track.add_send",
        serde_json::json!({ "track_id": control_track, "to_bus": control_bus }),
    )
    .result()
    .expect("track.add_send succeeds");
    echo(app, rx, engine);

    // A track group from the two GUI-added tracks (captured above, before
    // any control track/bus adds could be mistaken for them) and a
    // marker (app-only space).
    app.test_set_selected_tracks(gui_tracks);
    app.test_dispatch(Message::Group(GroupMessage::CreateGroupFromSelection));
    app.test_dispatch(Message::Marker(MarkerMessage::AddAtPlayhead));
    echo(app, rx, engine);
}

/// Save the project the way a manual save writes it and open it again
/// through the real replay, echoing the engine's part.
fn save_and_reload(f: &mut Fixture) {
    let file = f.app.test_build_project_file();
    let midi_clips: Vec<(ClipId, Vec<MidiNote>)> = f
        .app
        .test_midi_clips()
        .iter()
        .map(|mc| (mc.id, mc.notes.clone()))
        .collect();
    project::save_project(&f.project, &file, &[], &midi_clips).expect("save");
    let loaded = project::load_project(&f.project).expect("reload");
    f.app.test_replay_loaded_project_from(loaded);
    echo(&mut f.app, &f.rx, &mut f.engine);
    f.app.test_set_active_project(true);
    f.app.test_set_project_path(f.project.clone());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn ids_stay_unique_across_gui_and_control_adds_and_a_reload() {
    let mut f = fixture("rounds");
    assert_partition_holds(&f.app, "demo");

    add_round(&mut f, "first");
    assert_partition_holds(&f.app, "after the first round");
    let before_reload = live_ids(&f.app);

    save_and_reload(&mut f);
    assert_partition_holds(&f.app, "after reload");
    let after_reload = live_ids(&f.app);
    assert_eq!(
        after_reload.tracks.iter().copied().collect::<HashSet<_>>(),
        before_reload.tracks.iter().copied().collect::<HashSet<_>>(),
        "a reload keeps every track id"
    );
    assert_eq!(
        after_reload.groups.iter().copied().collect::<HashSet<_>>(),
        before_reload.groups.iter().copied().collect::<HashSet<_>>(),
        "a reload keeps every group id"
    );

    add_round(&mut f, "second");
    assert_partition_holds(&f.app, "after the second round");
    let ids = live_ids(&f.app);
    assert!(
        ids.tracks.len() >= before_reload.tracks.len() + 4,
        "the second round added its tracks"
    );
    assert_eq!(ids.groups.len(), 2, "one group per round");
}

/// FU-A1c: a track allocation must skip a group's id, not only a
/// track's — tracks and groups share one id space, and until A4-3 the
/// load-time counter bump was the only thing keeping them apart.
#[test]
fn a_new_track_never_takes_a_group_id() {
    let mut f = fixture("group");
    add_round(&mut f, "first");
    let group_id = f.app.test_track_groups().get_all_groups()[0].id;

    // Wind the counter back onto the group's id, as a project whose
    // groups were restored without the bump would have it.
    f.app.test_registry_mut().next_track_id = group_id;
    let track_id = call(&mut f.app, "track.add", serde_json::json!({ "kind": "audio" }))
        .result::<serde_json::Value>()
        .expect("track.add succeeds")["track_id"]
        .as_u64()
        .unwrap();
    assert_ne!(track_id, group_id, "the allocator skipped the group's id");
    echo(&mut f.app, &f.rx, &mut f.engine);
    assert_partition_holds(&f.app, "after allocating over a group id");
}

/// FU-A6a: a GUI-drawn MIDI clip (engine-allocated) between two
/// app-derived ones (`notes.create_clip` draws from
/// `fresh_derived_clip_id`) must not share an id with either. Before the
/// engine stopped bumping its counter past app-range ids, the first
/// derived clip at `base` moved the engine to `base + 1`, the drawn clip
/// took it, and the next derived clip was handed `base + 1` as well.
#[test]
fn a_drawn_clip_never_takes_a_derived_clip_id() {
    let mut f = fixture("clips");
    let track_id = f
        .app
        .test_registry()
        .tracks
        .iter()
        .find(|t| matches!(t.track_type, TrackType::Instrument) && t.sub_track.is_none())
        .expect("the demo has an instrument track")
        .id;
    let seeded: HashSet<ClipId> = f.app.test_midi_clips().iter().map(|c| c.id).collect();
    let derive = |f: &mut Fixture, bar: u32| {
        let clip_id = call(
            &mut f.app,
            "notes.create_clip",
            serde_json::json!({ "track_id": track_id, "start_bar": bar, "length_beats": 4.0 }),
        )
        .result::<serde_json::Value>()
        .expect("notes.create_clip succeeds")["clip_id"]
            .as_u64()
            .expect("notes.create_clip returns the id");
        echo(&mut f.app, &f.rx, &mut f.engine);
        clip_id
    };

    let first = derive(&mut f, 40);
    assert!(first >= DERIVED_CLIP_ID_BASE, "control clips come from the derived range");
    f.app.test_dispatch(Message::Compose(ComposeMessage::CreateMidiClipInSection {
        track_id,
        start_sample: 0,
        length_bars: 1,
    }));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let second = derive(&mut f, 44);

    let ids: Vec<ClipId> = f.app.test_midi_clips().iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), seeded.len() + 3, "three clips were added: {ids:?}");
    assert_set("midi clip", &ids);
    let drawn = *ids
        .iter()
        .find(|id| **id != first && **id != second && !seeded.contains(id))
        .expect("the drawn clip landed");
    assert!(drawn < DERIVED_CLIP_ID_BASE, "the drawn clip took an engine id, got {drawn}");
}

/// The one remaining base is named in one place and keeps its order
/// relative to its neighbour; a base that moves onto that neighbour fails
/// here (and at compile time in `ids.rs`).
#[test]
fn the_app_id_bases_are_ordered_and_disjoint() {
    assert!(BUS_ID_BASE < DERIVED_CLIP_ID_BASE);
    // A fresh app seeds its counters from the bases.
    let (app, _task) = Resonance::new_for_test();
    // ARCH-04 D-3: busses kept a base — not an engine-agreed one any
    // more, but one `song.summary` / `song.tracks` still needs (see
    // `state/ids.rs`).
    assert_eq!(app.test_registry().next_bus_id, BUS_ID_BASE);
    // D-1/D-2/D-4: plugins, sends and tracks have no base at all — the
    // app is the only allocator left for any of the three, so all three
    // start at 1 like the engine's counters used to (tracks still stay
    // `debug_assert`-below `BUS_ID_BASE`, but that's a runtime guard in
    // `allocate_track_id`, not a seed value).
    assert_eq!(app.test_registry().next_track_id, 1);
    assert_eq!(app.test_next_plugin_id(), 1);
    assert_eq!(app.test_next_send_id(), 1);
}

/// ARCH-04 D-1: every plugin add — GUI or control, on a track, a bus, or
/// master — now goes through the same app allocator
/// (`Resonance::allocate_plugin_id`), so a GUI add on a track and a
/// control add on a bus can no longer land on the same id by construction
/// (before D-1 they came from disjoint ranges; now they come from the
/// same counter's in-use scan). This drives every one of those six add
/// paths, plus undo of one of them, and checks the plugin space stays a
/// set throughout — including that undoing an add does not let the next
/// add reuse the id undo just freed (the same STATE-08-style monotonicity
/// `ids_stay_unique_across_gui_and_control_adds_and_a_reload` pins for
/// tracks and busses across a reload).
#[test]
fn every_plugin_add_path_gets_a_unique_app_id_including_across_undo() {
    let mut f = fixture("plugin-paths");
    add_round(&mut f, "first");
    assert_partition_holds(&f.app, "after one round of track/bus/master adds");
    let ids_before = live_ids(&f.app);
    assert!(
        ids_before.plugins.len() >= 6,
        "add_round adds a plugin via both GUI and control on the track, \
         the bus, and master: {:?}",
        ids_before.plugins
    );

    // A fresh, self-contained add-then-undo-then-add, so this doesn't
    // depend on which of `add_round`'s several undo-recorded actions
    // happens to be last. Add a plugin, undo it, add again: if undo
    // rewound the allocator instead of merely removing the mirrored
    // slot, the second add would reuse the id the first one held.
    //
    // Uses `update()`, not `test_dispatch` (which bypasses undo
    // bookkeeping on purpose — see its doc comment): `record_undo` has to
    // run before the add dispatches to snapshot the pre-add state, or
    // `Message::Undo` (also only wired up inside `update()`) has nothing
    // to restore.
    let target_track = f.app.test_registry().tracks[0].id;
    let add_eq = || PluginMessage::AddPluginToTrack(
        target_track,
        ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: EQ.to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
    );
    let _ = f.app.update(Message::Plugin(add_eq()));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let first_id = f
        .app
        .test_chain_slots(TestChain::Track(target_track))
        .last()
        .expect("the first add landed")
        .0;

    let _ = f.app.update(Message::Undo);
    // A plugin add is a structural change, so undo takes the
    // ClearAll -> replay path (same as
    // `control_track_remove_effect::the_removal_is_recorded_on_the_undo_stack`)
    // rather than the fast fader/knob diff-replay: it only STARTS the
    // restore, asynchronously, by sending `ClearAll` and stashing the
    // pre-add snapshot in `io.pending_load`. Play the engine's
    // `AllCleared` echo back to actually finish it.
    assert!(
        std::iter::from_fn(|| f.rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find the add and start restoring the pre-add snapshot"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(
        f.app
            .test_chain_slots(TestChain::Track(target_track))
            .iter()
            .all(|s| s.0 != first_id),
        "undo removed the slot the first add created"
    );

    let _ = f.app.update(Message::Plugin(add_eq()));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let second_id = f
        .app
        .test_chain_slots(TestChain::Track(target_track))
        .last()
        .expect("the second add landed")
        .0;

    assert_ne!(
        second_id, first_id,
        "undo must not rewind the allocator: the post-undo add reused the \
         id the undone add held"
    );
    assert_partition_holds(&f.app, "after undo + a fresh add");
}

/// ARCH-04 D-3: every bus add path — the plain GUI "Add bus", `bus.create`,
/// and the bus half of `CreateReturnFromSend` — now goes through the same
/// app allocator (`TrackRegistry::allocate_bus_id`). `add_round` already
/// drives the GUI and control paths every round; this adds the
/// undo-monotonicity half `every_plugin_add_path_gets_a_unique_app_id_including_across_undo`
/// pins for plugins: undoing a bus add must not let the next add reuse the
/// id undo just freed. A bus add is structural (`bus_set_matches` is part
/// of `structurally_compatible`), so undo takes the same `ClearAll` ->
/// replay path as a plugin add.
#[test]
fn every_bus_add_path_gets_a_unique_app_id_including_across_undo() {
    let mut f = fixture("bus-paths");
    add_round(&mut f, "first");
    assert_partition_holds(&f.app, "after one round of bus adds");
    let ids_before = live_ids(&f.app);
    assert!(
        ids_before.busses.len() >= 4,
        "add_round adds a bus via the GUI and via control, plus demo's two \
         seeded busses: {:?}",
        ids_before.busses
    );

    // Identified by set difference, not `.max()` — see the same note on
    // `gui_bus` in `add_round`.
    let before_first: HashSet<u64> = f.app.test_registry().busses.iter().map(|b| b.id).collect();
    let _ = f.app.update(Message::Bus(BusMessage::AddBus));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let first_id = f
        .app
        .test_registry()
        .busses
        .iter()
        .map(|b| b.id)
        .find(|id| !before_first.contains(id))
        .expect("the first add landed a new bus");

    let _ = f.app.update(Message::Undo);
    assert!(
        std::iter::from_fn(|| f.rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find the add and start restoring the pre-add snapshot"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(
        !f.app.test_registry().busses.iter().any(|b| b.id == first_id),
        "undo removed the bus the first add created"
    );

    let before_second: HashSet<u64> = f.app.test_registry().busses.iter().map(|b| b.id).collect();
    let _ = f.app.update(Message::Bus(BusMessage::AddBus));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let second_id = f
        .app
        .test_registry()
        .busses
        .iter()
        .map(|b| b.id)
        .find(|id| !before_second.contains(id))
        .expect("the second add landed a new bus");

    assert_ne!(
        second_id, first_id,
        "undo must not rewind the allocator: the post-undo add reused the \
         id the undone add held"
    );
    assert_partition_holds(&f.app, "after undo + a fresh bus add");
}

/// ARCH-04 D-2: every send add path — the plain GUI "Add send",
/// `track.add_send`, and the send half of `CreateReturnFromSend` — now
/// goes through the same app allocator (`AuxSendState::allocate_send_id`).
/// Unlike a plugin or bus add, a send add is NOT structural (`apply_sends`
/// reconciles it on the fast diff-replay path, `structurally_compatible`
/// never looks at `ProjectFile::sends`), so undo here never sends
/// `ClearAll` — `RemoveAuxSend` lands directly, and the mirror drops the
/// send synchronously inside `update()`.
#[test]
fn every_send_add_path_gets_a_unique_app_id_including_across_undo() {
    let mut f = fixture("send-paths");
    add_round(&mut f, "first");
    assert_partition_holds(&f.app, "after one round of send adds");
    let ids_before = live_ids(&f.app);
    assert!(
        ids_before.sends.len() >= 2,
        "add_round adds a send via the GUI and via control: {:?}",
        ids_before.sends
    );

    let track_id = f.app.test_registry().tracks[0].id;
    let bus_id = f.app.test_registry().busses[0].id;
    let _ = f.app.update(Message::Mixer(MixerMessage::AddSend {
        source: SendSource::Track(track_id),
        dest: bus_id,
    }));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let first_id = f.app.test_aux_sends().iter().map(|s| s.id).max().unwrap();

    let _ = f.app.update(Message::Undo);
    assert!(
        !std::iter::from_fn(|| f.rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "a send-only undo must take the fast diff-replay path, not ClearAll"
    );
    assert!(
        !f.app.test_aux_sends().iter().any(|s| s.id == first_id),
        "undo removed the send the first add created"
    );

    let _ = f.app.update(Message::Mixer(MixerMessage::AddSend {
        source: SendSource::Track(track_id),
        dest: bus_id,
    }));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let second_id = f.app.test_aux_sends().iter().map(|s| s.id).max().unwrap();

    assert_ne!(
        second_id, first_id,
        "undo must not rewind the allocator: the post-undo add reused the \
         id the undone add held"
    );
    assert_partition_holds(&f.app, "after undo + a fresh send add");
}

/// ARCH-04 D-4: every track add path — the plain GUI "Add Track" and its
/// instrument/vocal/external siblings, `track.add`, sub-tracks, bounce
/// targets and track groups — now goes through the same app allocator
/// (`Resonance::allocate_track_id`), with no engine counter left to fall
/// back on. `add_round` already drives the GUI and control paths every
/// round, below `BUS_ID_BASE` and never colliding with a group id
/// (`a_new_track_never_takes_a_group_id` covers that specifically); this
/// adds the undo-monotonicity half `every_bus_add_path_gets_a_unique_app_id_including_across_undo`
/// pins for busses: undoing a track add must not let the next add reuse
/// the id undo just freed. A track add is structural
/// (`replay_diff::added_track_forces_fallback`), so undo takes the same
/// `ClearAll` -> replay path as a plugin or bus add.
#[test]
fn every_track_add_path_gets_a_unique_app_id_including_across_undo() {
    let mut f = fixture("track-paths");
    add_round(&mut f, "first");
    assert_partition_holds(&f.app, "after one round of track adds");
    let ids_before = live_ids(&f.app);
    assert!(
        ids_before.tracks.len() >= 6,
        "add_round adds a track via the GUI (twice) and via control (twice), \
         plus demo's seeded tracks: {:?}",
        ids_before.tracks
    );
    for id in &ids_before.tracks {
        assert!(*id < BUS_ID_BASE, "track id {id} grew into the bus range");
    }

    let before_first: HashSet<u64> = f.app.test_registry().tracks.iter().map(|t| t.id).collect();
    let _ = f.app.update(Message::Track(TrackMessage::AddTrack));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let first_id = f
        .app
        .test_registry()
        .tracks
        .iter()
        .map(|t| t.id)
        .find(|id| !before_first.contains(id))
        .expect("the first add landed a new track");

    let _ = f.app.update(Message::Undo);
    assert!(
        std::iter::from_fn(|| f.rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find the add and start restoring the pre-add snapshot"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(
        !f.app.test_registry().tracks.iter().any(|t| t.id == first_id),
        "undo removed the track the first add created"
    );

    let before_second: HashSet<u64> = f.app.test_registry().tracks.iter().map(|t| t.id).collect();
    let _ = f.app.update(Message::Track(TrackMessage::AddTrack));
    echo(&mut f.app, &f.rx, &mut f.engine);
    let second_id = f
        .app
        .test_registry()
        .tracks
        .iter()
        .map(|t| t.id)
        .find(|id| !before_second.contains(id))
        .expect("the second add landed a new track");

    assert_ne!(
        second_id, first_id,
        "undo must not rewind the allocator: the post-undo add reused the \
         id the undone add held"
    );
    assert_partition_holds(&f.app, "after undo + a fresh track add");
}

// ---------------------------------------------------------------------------
// References (ARCH-04 D-5)
// ---------------------------------------------------------------------------

/// ARCH-04 D-5: `LoadReferenceTrack`'s `id` is now mandatory — the app is
/// the only allocator left for reference ids
/// (`ReferenceState::alloc_engine_id`), and the engine refuses a collision
/// (`EngineErrorKind::Internal`) rather than inventing one. References sit
/// outside the `Fixture`/`FakeEngine`/`add_round` machinery above (no base,
/// no engine counter, not part of `structurally_compatible`), so this test
/// is self-contained: it drives the real `ReferenceMessage::LoadRequested`
/// path through an add, an undo, a fresh add, and a save/reload, and checks
/// every id handed to `LoadReferenceTrack` is a set throughout — the same
/// undo-monotonicity and reload-uniqueness shape
/// `every_send_add_path_gets_a_unique_app_id_including_across_undo` and
/// `ids_stay_unique_across_gui_and_control_adds_and_a_reload` pin for sends
/// and tracks.
#[test]
fn every_reference_load_gets_a_unique_app_id_including_across_undo_and_reload() {
    let root = std::env::temp_dir().join(format!(
        "resonance-id-allocation-reference-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("fixture.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("create project dir");

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(project.clone());

    // Real (if empty) files: `restore_references` only re-issues
    // `LoadReferenceTrack` for an entry whose path still exists on disk —
    // otherwise it is seeded as `Missing` and never sent to the engine.
    let ref_path = |name: &str| -> PathBuf {
        let p = root.join(name);
        std::fs::write(&p, b"").expect("create stand-in reference file");
        p
    };

    let load = |app: &mut Resonance, path: PathBuf| {
        let _ = app.update(Message::Reference(ReferenceMessage::LoadRequested(path)));
    };
    let sent_id = |rx: &Receiver<AudioCommand>| -> u32 {
        std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|c| match c {
                AudioCommand::LoadReferenceTrack { id, .. } => Some(id.0),
                _ => None,
            })
            .expect("LoadRequested sends a LoadReferenceTrack")
    };

    load(&mut app, ref_path("ref-a.wav"));
    let first_id = sent_id(&rx);

    load(&mut app, ref_path("ref-b.wav"));
    let second_id = sent_id(&rx);
    assert_ne!(first_id, second_id, "two loads must not share an id");

    // Undo the second load: a reference add is not structural
    // (`structurally_compatible` never looks at `ProjectFile::references`),
    // so this takes the fast diff-replay path (`reconcile_references`), not
    // `ClearAll` + full restore.
    let _ = app.update(Message::Undo);
    assert!(
        !std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "a reference-only undo must take the fast diff-replay path, not ClearAll"
    );
    assert!(
        !app.test_reference().entries.iter().any(|e| e.id.0 == second_id),
        "undo removed the reference the second load created"
    );

    load(&mut app, ref_path("ref-c.wav"));
    let third_id = sent_id(&rx);
    assert_ne!(
        third_id, second_id,
        "undo must not rewind the allocator: the post-undo load reused the \
         id the undone load held"
    );

    let ids_before_reload: Vec<u32> =
        app.test_reference().entries.iter().map(|e| e.id.0).collect();
    assert_eq!(
        ids_before_reload.iter().collect::<HashSet<_>>().len(),
        ids_before_reload.len(),
        "pre-reload reference ids are a set: {ids_before_reload:?}"
    );

    // Save + reload: `restore_references` re-issues `LoadReferenceTrack` for
    // every saved entry under a fresh id from the app's own allocator
    // (ARCH-04 D-5 — the engine keeps none of its own to restart at 1).
    let file = app.test_build_project_file();
    project::save_project(&project, &file, &[], &[]).expect("save");
    let loaded = project::load_project(&project).expect("reload");
    app.test_replay_loaded_project_from(loaded);
    let reload_ids: Vec<u32> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|c| match c {
            AudioCommand::LoadReferenceTrack { id, .. } => Some(id.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        reload_ids.len(),
        ids_before_reload.len(),
        "the reload re-issues one LoadReferenceTrack per saved reference"
    );
    assert_eq!(
        reload_ids.iter().collect::<HashSet<_>>().len(),
        reload_ids.len(),
        "reload ids are a set: {reload_ids:?}"
    );

    // A further load after the reload must not collide with a restored id.
    load(&mut app, ref_path("ref-d.wav"));
    let fourth_id = sent_id(&rx);
    assert!(
        !reload_ids.contains(&fourth_id),
        "a post-reload load must not reuse a restored id"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Pool assets (D-7a)
// ---------------------------------------------------------------------------

/// D-7a: the app is the pool's only asset-id allocator now — session-
/// monotonic (never rewound by undo), and seeded past both a loaded
/// project's assets AND any `audio/asset_<id>.wav` a disk scan finds.
/// Self-contained (no base, no engine counter, not part of
/// `Fixture`/`add_round`), the same shape as
/// [`every_reference_load_gets_a_unique_app_id_including_across_undo_and_reload`]:
/// an import, an undo of it (checking the next import does not reuse its
/// id), a save + reload with an ORPHANED asset file sitting ABOVE the
/// pool's own max id (one an undone import left behind before the save),
/// and a further import — checking the new id lands above both.
#[test]
fn asset_ids_stay_unique_across_undo_and_above_an_orphaned_wav_after_reload() {
    let root = std::env::temp_dir().join(format!(
        "resonance-id-allocation-assets-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("fixture.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("create project dir");

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(project.clone());

    let asset_imported = |id: u64, path: &str| resonance_audio::types::AudioEvent::AssetImported {
        asset_id: id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: path.to_string(),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 48_000,
        duration_frames: 4_800,
        peaks: vec![(-0.1, 0.1)],
    };
    let sent_asset_id = |rx: &Receiver<AudioCommand>| -> u64 {
        std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|c| match c {
                AudioCommand::ImportAudioToPool { files } => files.first().map(|f| f.asset_id),
                _ => None,
            })
            .expect("ImportAudioToPool sent")
    };

    // First import (via `update()`, not `test_dispatch`, so it records an
    // undo entry): a fresh app's counter starts at 1.
    let _ = app.update(Message::Pool(PoolMessage::ImportFilesToPool(vec![
        PathBuf::from("/imports/first.wav"),
    ])));
    let first_id = sent_asset_id(&rx);
    assert_eq!(first_id, 1);
    app.test_apply_engine_event(asset_imported(first_id, "/imports/first.wav"));
    assert_eq!(app.test_pool().max_asset_id(), Some(1));

    // Undo the import: a pool asset add/remove is not structural
    // (`structurally_compatible` never looks at `pool_assets`), so this
    // takes the fast diff-replay path, not `ClearAll` + full restore — and
    // the diff path leaves the (session-monotonic) counter alone.
    let _ = app.update(Message::Undo);
    assert!(app.test_pool().assets.is_empty(), "undo removed the imported asset");

    // A further import must not reuse the undone import's id.
    let _ = app.update(Message::Pool(PoolMessage::ImportFilesToPool(vec![
        PathBuf::from("/imports/second.wav"),
    ])));
    let second_id = sent_asset_id(&rx);
    assert_ne!(
        second_id, first_id,
        "undo must not rewind the allocator: the post-undo import reused \
         the id the undone import held"
    );
    app.test_apply_engine_event(asset_imported(second_id, "/imports/second.wav"));
    assert_eq!(app.test_pool().max_asset_id(), Some(second_id));

    // An orphaned asset WAV above the pool's own max id: an import undone
    // (or otherwise never saved into `pool_assets`) before the save below,
    // whose file a backup or a stale worker still left on disk.
    let orphan_id = second_id + 10;
    std::fs::write(
        project.join(format!("audio/asset_{orphan_id}.wav")),
        b"orphaned",
    )
    .expect("write orphan");

    // Save + reload: `restore_pool_assets` seeds past both the pool's own
    // max id and the disk scan.
    let file = app.test_build_project_file();
    project::save_project(&project, &file, &[], &[]).expect("save");
    let loaded = project::load_project(&project).expect("reload");
    app.test_replay_loaded_project_from(loaded);
    app.test_set_active_project(true);
    app.test_set_project_path(project.clone());

    // A further import after the reload must not reuse the orphan's id.
    let rx = app.test_capture_engine();
    app.test_dispatch(Message::Pool(PoolMessage::ImportFilesToPool(vec![
        PathBuf::from("/imports/third.wav"),
    ])));
    let third_id = sent_asset_id(&rx);
    assert!(
        third_id > orphan_id,
        "the post-reload import id {third_id} must be above the orphaned \
         asset_{orphan_id}.wav"
    );
    app.test_apply_engine_event(asset_imported(third_id, "/imports/third.wav"));
    assert_eq!(
        app.test_pool().max_asset_id(),
        Some(third_id),
        "the new asset landed at the id the app allocated for it"
    );

    let _ = std::fs::remove_dir_all(&root);
}
