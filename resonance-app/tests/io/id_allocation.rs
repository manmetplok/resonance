//! Entity ids never collide across the app's allocators, or with the
//! engine's own remaining ones (ARCH-04 A4-1, updated for D-1).
//!
//! The engine allocates the ids of everything the GUI adds without a
//! hint (tracks, busses, sends); the app allocates the ids it needs
//! synchronously (control-API adds, FX returns, sub-tracks, track
//! groups) from the bases named in `resonance_app::state::ids`. This
//! pins the partition that keeps the two apart today, so the epic that
//! moves ownership to the app (A4-4) can migrate one space at a time
//! against a test: the demo project gets one of each entity added
//! through the GUI path *and* through the control path, is saved,
//! reloaded, and gets the same adds again; every id space must still be
//! a set afterwards, and every app counter must sit above every id its
//! space holds.
//!
//! **Plugins landed D-1**: there is no more engine-vs-app split for
//! `PluginInstanceId`. Every add — GUI or control, track/bus/master —
//! now allocates through `Resonance::allocate_plugin_id`, and
//! [`FakeEngine::plugin`] just echoes the id it was given straight back,
//! the way the real `handle_add_plugin`/`handle_add_plugin_to_bus`/
//! `handle_add_plugin_to_master` do (no counter, no hint, no base). The
//! remaining `FakeEngine` allocators (track, bus, send) still follow the
//! allocation rules in `resonance-audio/src/engine/{tracks,busses}.rs` to
//! the letter — including the one that matters: a hint at or above the
//! app's base is taken but never moves the engine's counter. Before that
//! rule covered tracks (it was plugin-only) this test failed on its
//! second round: the engine, dragged to 1e9+2 by the control adds, handed
//! a GUI "Add track" the id of the group the app had created — a group
//! the engine never hears about.

use std::collections::HashSet;
use std::path::PathBuf;

use resonance_app::compose::ComposeMessage;
use resonance_app::message::*;
use resonance_app::project;
use resonance_app::state::ids::{
    CONTROL_SEND_ID_BASE, DERIVED_CLIP_ID_BASE, RETURN_BUS_ID_BASE, SUB_TRACK_ID_BASE,
};
use resonance_app::{demo, Resonance, TestChain};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ClipId, MidiNote, ScannedPlugin, SendSource, TrackType,
};

use crate::common::call;

// ---------------------------------------------------------------------------
// The engine's allocators, as the engine implements them
// ---------------------------------------------------------------------------

/// The engine-side counters (`HandlerState::next_*_id`) and the hint
/// rules each space applies, so the echoes this test plays back carry
/// exactly the ids the live engine would.
struct FakeEngine {
    next_track: u64,
    next_bus: u64,
    next_send: u64,
    next_clip: u64,
}

impl FakeEngine {
    fn new() -> Self {
        Self {
            next_track: 1,
            next_bus: 1,
            next_send: 1,
            next_clip: 1,
        }
    }

    /// `engine/tracks.rs`, `engine/midi/clips.rs`, `engine/busses.rs`
    /// (busses and sends): honour a hint, and bump the counter past it
    /// only when it is below the app's base for that space.
    fn allocate(next: &mut u64, hint: Option<u64>, app_base: u64) -> u64 {
        match hint {
            Some(h) => {
                if h < app_base {
                    *next = (*next).max(h + 1);
                }
                h
            }
            None => {
                let i = *next;
                *next += 1;
                i
            }
        }
    }

    fn track(&mut self, hint: Option<u64>) -> u64 {
        Self::allocate(&mut self.next_track, hint, SUB_TRACK_ID_BASE)
    }

    fn bus(&mut self, hint: Option<u64>) -> u64 {
        Self::allocate(&mut self.next_bus, hint, RETURN_BUS_ID_BASE)
    }

    fn send(&mut self, hint: Option<u64>) -> u64 {
        Self::allocate(&mut self.next_send, hint, CONTROL_SEND_ID_BASE)
    }

    /// `engine/midi/clips.rs`, `engine/clips.rs` (FU-A6a): a clip id
    /// handed in (`LoadMidiClipDirect`, `LoadClipFromWav`) raises the
    /// counter only when it is below the app's derived-clip base.
    fn clip(&mut self, id: Option<u64>) -> u64 {
        Self::allocate(&mut self.next_clip, id, DERIVED_CLIP_ID_BASE)
    }

    /// D-1: the engine has no plugin-id counter left. It honours
    /// whatever id `AddPlugin`/`AddPluginToBus`/`AddPluginToMaster`
    /// carries, full stop — this just plays that back as the echo.
    fn plugin(&mut self, id: u64) -> u64 {
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
            AudioCommand::AddTrack { id_hint, .. } => {
                let track_id = engine.track(id_hint);
                app.test_apply_engine_event(AudioEvent::TrackAdded { track_id });
            }
            AudioCommand::AddInstrumentTrack { id_hint, .. } => {
                let track_id = engine.track(id_hint);
                app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id });
            }
            AudioCommand::AddVocalTrack { id_hint, .. } => {
                let track_id = engine.track(id_hint);
                app.test_apply_engine_event(AudioEvent::VocalTrackAdded { track_id });
            }
            AudioCommand::CreateSubTrack { sub_id, .. } => {
                // No echo; the mirror names sub-tracks itself. The
                // engine still learns the id.
                engine.track(Some(sub_id));
            }
            AudioCommand::AddBus { id_hint, name } => {
                let bus_id = engine.bus(id_hint);
                app.test_apply_engine_event(AudioEvent::BusAdded {
                    bus_id,
                    name: name.unwrap_or_else(|| format!("Bus {bus_id}")),
                });
            }
            AudioCommand::SetAuxSend {
                id_hint,
                source,
                dest,
                level_db,
                pre_fader,
                enabled,
            } => {
                let send_id = engine.send(id_hint);
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
    let ids = live_ids(&app);
    engine.next_track = ids.tracks.iter().copied().max().unwrap_or(0) + 1;
    engine.next_bus = ids.busses.iter().copied().max().unwrap_or(0) + 1;
    engine.next_send = ids.sends.iter().copied().max().unwrap_or(0) + 1;
    engine.next_clip = app
        .test_midi_clips()
        .iter()
        .map(|c| c.id)
        .filter(|id| *id < DERIVED_CLIP_ID_BASE)
        .max()
        .unwrap_or(0)
        + 1;
    // No `next_plugin` to advance: D-1 deleted the engine's plugin
    // counter, so `FakeEngine::plugin` has nothing to seed.
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
    above(registry.next_sub_track_id, SUB_TRACK_ID_BASE, &tracks_and_groups, "track");
    above(registry.next_return_bus_id, RETURN_BUS_ID_BASE, &ids.busses, "bus");
    // The send counter is seeded lazily (`0` until the first control add).
    if app.test_aux_next_control_send_id() != 0 {
        above(app.test_aux_next_control_send_id(), CONTROL_SEND_ID_BASE, &ids.sends, "send");
    }
    // D-1: no base and no "counter stays above every held id" invariant
    // for plugins any more — the app is the sole allocator, but plugin
    // ids can also land in the mirror without ever passing through it
    // (`demo::seed_demo_content`'s fixture ids, a project loaded with
    // ids from a session that ran ahead of this one's counter). What
    // `allocate_plugin_id` actually guarantees is that its OWN in-use
    // scan never hands out an id already live, regardless of the
    // counter's position — which is exactly what the `assert_set` above
    // is for: any real collision shows up there as a repeated id the
    // moment a scan-protected allocation coexists with an out-of-band
    // one, which is precisely what `add_round`'s GUI + control adds onto
    // the demo's seeded content already drives.
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

    // Tracks: GUI (engine-allocated) and control (app-allocated).
    app.test_dispatch(Message::Track(TrackMessage::AddTrack));
    app.test_dispatch(Message::Track(TrackMessage::AddInstrumentTrack));
    echo(app, rx, engine);
    let control_track = call(
        app,
        "track.add",
        serde_json::json!({ "kind": "audio", "name": format!("{round} control audio") }),
    )
    .result::<serde_json::Value>()
    .expect("track.add succeeds")["track_id"]
        .as_u64()
        .expect("track.add returns the id");
    assert!(control_track >= SUB_TRACK_ID_BASE, "control tracks come from the app range");
    echo(app, rx, engine);
    let _: serde_json::Value = call(
        app,
        "track.add",
        serde_json::json!({ "kind": "instrument", "name": format!("{round} control synth") }),
    )
    .result()
    .expect("track.add (instrument) succeeds");
    echo(app, rx, engine);

    // Busses: GUI and control.
    app.test_dispatch(Message::Bus(BusMessage::AddBus));
    echo(app, rx, engine);
    let control_bus = call(
        app,
        "bus.create",
        serde_json::json!({ "name": format!("{round} control bus") }),
    )
    .result::<serde_json::Value>()
    .expect("bus.create succeeds")["bus_id"]
        .as_u64()
        .expect("bus.create returns the id");
    assert!(control_bus >= RETURN_BUS_ID_BASE, "control busses come from the app range");
    echo(app, rx, engine);
    let gui_bus = app.test_registry().busses.iter().map(|b| b.id).max().unwrap();

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

    // Sends: GUI (engine-allocated) and control (app-allocated), from
    // the control track into the two new busses.
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

    // A track group from the two GUI-added tracks (app-allocated, in the
    // track space) and a marker (app-only space).
    let gui_tracks: Vec<u64> = app
        .test_registry()
        .tracks
        .iter()
        .map(|t| t.id)
        .filter(|id| *id < SUB_TRACK_ID_BASE)
        .rev()
        .take(2)
        .collect();
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
    assert!(group_id >= SUB_TRACK_ID_BASE, "groups come from the app range");

    // Wind the counter back onto the group's id, as a project whose
    // groups were restored without the bump would have it.
    f.app.test_registry_mut().next_sub_track_id = group_id;
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

/// The bases are named in one place and keep their order; a base that
/// moves onto a neighbour fails here (and at compile time in `ids.rs`).
#[test]
fn the_app_id_bases_are_ordered_and_disjoint() {
    assert!(SUB_TRACK_ID_BASE < RETURN_BUS_ID_BASE);
    assert_eq!(RETURN_BUS_ID_BASE, CONTROL_SEND_ID_BASE);
    assert!(CONTROL_SEND_ID_BASE < DERIVED_CLIP_ID_BASE);
    // A fresh app seeds its counters from the bases.
    let (app, _task) = Resonance::new_for_test();
    assert_eq!(app.test_registry().next_sub_track_id, SUB_TRACK_ID_BASE);
    assert_eq!(app.test_registry().next_return_bus_id, RETURN_BUS_ID_BASE);
    // D-1: plugins have no base — the app is the only allocator left, so
    // it starts at 1 like the engine's counters used to.
    assert_eq!(app.test_next_plugin_id(), 1);
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
