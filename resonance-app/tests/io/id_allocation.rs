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
//!
//! **Clips (D-7b)** come from one app allocator, `EntityIds::clips`,
//! starting at `CLIP_ID_BASE` and never reset — not by undo, `ClearAll` or
//! a disk load. Since D-7d the engine's own clips (recordings, loop passes,
//! live MIDI) take ids from blocks of that same counter granted ahead of
//! time, which [`FakeEngine::draw`] plays back; its old counter
//! ([`FakeEngine::clip`]) allocates nothing any more.

use std::collections::HashSet;
use std::path::PathBuf;

use resonance_app::compose::ComposeMessage;
use resonance_app::message::*;
use resonance_app::project;
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::ids::{BUS_ID_BASE, CLIP_ID_BASE};
use resonance_app::{demo, Resonance, TestChain};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{ChainOwner, 
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
    /// The clip ids granted and not yet drawn (`engine/id_grant.rs`,
    /// D-7d), oldest first.
    grant: std::collections::VecDeque<std::ops::Range<u64>>,
    /// Every range ever granted, in order.
    granted: Vec<std::ops::Range<u64>>,
}

impl FakeEngine {
    fn new() -> Self {
        Self {
            next_clip: 1,
            grant: Default::default(),
            granted: Vec::new(),
        }
    }

    /// `ClipIdGrant::take` (D-7d): a recording's clip id, drawn from the
    /// front of the grant.
    fn draw(&mut self) -> Option<u64> {
        let front = self.grant.front_mut()?;
        let id = front.start;
        front.start += 1;
        if front.is_empty() {
            self.grant.pop_front();
        }
        Some(id)
    }

    fn grant_left(&self) -> u64 {
        self.grant.iter().map(|r| r.end - r.start).sum()
    }

    /// `engine/midi/clips.rs`, `engine/clips.rs` (FU-A6a): a clip id
    /// handed in (`LoadMidiClipDirect`, `LoadClipFromWav`) raises the
    /// counter only when it is below the app's derived-clip base.
    /// `CreateMidiClip` (D-7c) is now id-hinted the same way — it no
    /// longer allocates a fresh id of its own.
    fn clip(&mut self, id: Option<u64>) -> u64 {
        match id {
            Some(h) => {
                if h < CLIP_ID_BASE {
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
    /// whatever id `AddPlugin` carries, for any chain owner, full stop — this just plays that back as the echo.
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
                owner: ChainOwner::Track(track_id),
                clap_file_path,
                clap_plugin_id,
                id,
                ..
            } => {
                let instance_id = engine.plugin(id);
                app.test_apply_engine_event(AudioEvent::PluginAdded {
                    owner: ChainOwner::Track(track_id),
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
            AudioCommand::AddPlugin {
                owner: ChainOwner::Bus(bus_id),
                clap_file_path,
                clap_plugin_id,
                id,
            } => {
                let instance_id = engine.plugin(id);
                app.test_apply_engine_event(AudioEvent::PluginAdded {
                    owner: ChainOwner::Bus(bus_id),
                    instance_id,
                    plugin_name: clap_plugin_id.clone(),
                    clap_plugin_id,
                    clap_file_path,
                    params: Vec::new(),
                    has_gui: false,
                    has_sidechain_input: false,
                    output_port_count: 1,
                    output_port_names: Vec::new(),
                });
            }
            AudioCommand::AddPlugin {
                owner: ChainOwner::Master,
                clap_file_path,
                clap_plugin_id,
                id,
            } => {
                let instance_id = engine.plugin(id);
                app.test_apply_engine_event(AudioEvent::PluginAdded {
                    owner: ChainOwner::Master,
                    instance_id,
                    plugin_name: clap_plugin_id.clone(),
                    clap_plugin_id,
                    clap_file_path,
                    params: Vec::new(),
                    has_gui: false,
                    has_sidechain_input: false,
                    output_port_count: 1,
                    output_port_names: Vec::new(),
                });
            }
            AudioCommand::CreateMidiClip {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                name,
            } => {
                // D-7c: mandatory app-allocated id, honoured verbatim —
                // same echo shape as `LoadMidiClipDirect` below.
                let clip_id = engine.clip(Some(clip_id));
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
            // D-7d: the engine appends every grant to the ids it draws
            // recordings from; `ClearAll` revokes them.
            AudioCommand::GrantIds(blocks) => {
                engine.granted.push(blocks.clips.clone());
                engine.grant.push_back(blocks.clips);
            }
            AudioCommand::ClearAll => engine.grant.clear(),
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
        .filter(|id| *id < CLIP_ID_BASE)
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
    // D-7b: the app's one clip allocator starts at its base and sits
    // above every app-allocated clip id the mirror holds.
    let clip_ids: Vec<u64> = app
        .test_midi_clips()
        .iter()
        .map(|c| c.id)
        .chain(app.test_clips().iter().map(|c| c.id))
        .collect();
    assert_set(&format!("{when}: clip"), &clip_ids);
    above(app.test_next_clip_id(), CLIP_ID_BASE, &clip_ids, "clip");
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
        .map(|mc| (mc.id, mc.notes.as_ref().clone()))
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

/// D-7c (formerly FU-A6a): a GUI-drawn MIDI clip and two app-derived ones
/// (`notes.create_clip` draws from `EntityIds::clips`) all come from
/// the SAME app allocator now — `AudioCommand::CreateMidiClip` carries a
/// mandatory id since D-7c, so the engine has nothing left to allocate for
/// a drawn clip. Before D-7c the engine allocated the drawn clip's id
/// itself; the original FU-A6a bug was the engine's counter chasing the
/// app's derived range and handing out an id `notes.create_clip` had
/// already claimed. Also pins the STATE-08 shape every other allocator in
/// this file gets: the id stays unique across an undo of the draw and a
/// save/reload.
#[test]
fn a_drawn_clip_never_shares_an_id_with_a_derived_clip_including_across_undo_and_reload() {
    let mut f = fixture("clips");
    let track_id = f
        .app
        .test_registry()
        .tracks
        .iter()
        .find(|t| matches!(t.track_type, TrackType::Instrument) && t.sub_track.is_none())
        .expect("the demo has an instrument track")
        .id;
    let derive = |f: &mut Fixture, bar: u32| -> ClipId {
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
    // Identified by set difference (same reasoning as `gui_bus` in
    // `add_round`): a drawn clip's id no longer falls in a range of its
    // own that would tell it apart from a derived one.
    let draw = |f: &mut Fixture, start_sample: u64| -> ClipId {
        let before: HashSet<ClipId> = f.app.test_midi_clips().iter().map(|c| c.id).collect();
        let _ = f.app.update(Message::Compose(ComposeMessage::CreateMidiClipInSection {
            track_id,
            start_sample,
            length_bars: 1,
        }));
        echo(&mut f.app, &f.rx, &mut f.engine);
        *f.app
            .test_midi_clips()
            .iter()
            .map(|c| c.id)
            .find(|id| !before.contains(id))
            .as_ref()
            .expect("the drawn clip landed")
    };

    let first = derive(&mut f, 40);
    assert!(first >= CLIP_ID_BASE, "control clips come from the app's clip allocator");
    let drawn = draw(&mut f, 0);
    assert!(
        drawn >= CLIP_ID_BASE,
        "D-7c: the drawn clip now comes from the same app allocator, got {drawn}"
    );
    assert_set("midi clip", &[first, drawn]);

    // STATE-08: undo the draw (since A-13i through the diff path, which
    // deletes the clip itself) *immediately*, so it's the draw's own
    // snapshot that's undone rather than whatever came after it, and draw
    // again: the allocator must not rewind and reuse the undone id.
    let _ = f.app.update(Message::Undo);
    let undo: Vec<AudioCommand> = std::iter::from_fn(|| f.rx.try_recv().ok()).collect();
    assert!(
        !undo.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a clip draw undoes through the diff path (A-13i)"
    );
    assert!(
        undo.iter()
            .any(|c| matches!(c, AudioCommand::DeleteMidiClip { clip_id } if *clip_id == drawn)),
        "undo must find the draw and delete its clip"
    );
    assert!(
        !f.app.test_midi_clips().iter().any(|c| c.id == drawn),
        "undo removed the clip the draw created"
    );
    let redrawn = draw(&mut f, 0);
    assert_ne!(
        redrawn, drawn,
        "undo must not rewind the allocator: the post-undo draw reused the \
         id the undone draw held"
    );
    let second = derive(&mut f, 44);
    assert_set("midi clip", &[first, second, redrawn]);

    // A reload must not let a further draw or derive collide with
    // anything the save carried over.
    save_and_reload(&mut f);
    let after_reload: HashSet<ClipId> = f.app.test_midi_clips().iter().map(|c| c.id).collect();
    for id in [first, second, redrawn] {
        assert!(after_reload.contains(&id), "reload kept clip {id}");
    }

    let post_reload_derive = derive(&mut f, 48);
    let post_reload_drawn = draw(&mut f, 960);
    assert!(!after_reload.contains(&post_reload_derive), "a post-reload derive must not reuse a restored id");
    assert!(!after_reload.contains(&post_reload_drawn), "a post-reload draw must not reuse a restored id");
    assert_ne!(post_reload_derive, post_reload_drawn);
}

/// The one remaining base is named in one place and keeps its order
/// relative to its neighbour; a base that moves onto that neighbour fails
/// here (and at compile time in `ids.rs`).
#[test]
fn the_app_id_bases_are_ordered_and_disjoint() {
    assert!(BUS_ID_BASE < CLIP_ID_BASE);
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
    // D-7b: the one clip allocator starts at its base.
    assert_eq!(app.test_next_clip_id(), CLIP_ID_BASE);
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
    // Since A-13h a plugin add undoes on the diff path: `RemovePlugin`
    // for that instance, synchronously inside `update()`.
    assert!(
        std::iter::from_fn(|| f.rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::RemovePlugin { instance_id, .. } if instance_id == first_id
        )),
        "undo must find the add and remove the instance it created"
    );
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
/// id undo just freed. Since A-13h a bus add undoes through the diff path
/// (`RemoveBus`, no `ClearAll`), synchronously inside `update()`.
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
        std::iter::from_fn(|| f.rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::RemoveBus { bus_id } if bus_id == first_id)),
        "undo must find the add and remove the bus it created, on the diff path"
    );
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
/// `RoutingRemovals` / `Sends` reconcile a send add on an undo, which
/// never sends `ClearAll` (A-13j) —
/// `RemoveAuxSend` lands directly, and the mirror drops the
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
/// the id undo just freed. Since A-13i the undo takes the diff path (no
/// `ClearAll`): it removes the track itself, and re-adding a track never
/// lowers the allocator either.
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
    let undo: Vec<AudioCommand> = std::iter::from_fn(|| f.rx.try_recv().ok()).collect();
    assert!(
        !undo.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a track add undoes through the diff path (A-13i)"
    );
    assert!(
        undo.iter()
            .any(|c| matches!(c, AudioCommand::RemoveTrack { track_id } if *track_id == first_id)),
        "undo must find the add and remove its track"
    );
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
/// no engine counter), so this test
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

    // Undo the second load: an undo restores in place
    // (`reconcile_references`), never through `ClearAll` (A-13j).
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

    // Undo the import: an undo restores in place, never through
    // `ClearAll` (A-13j) — and it leaves the (session-monotonic) counter
    // alone.
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

// ---------------------------------------------------------------------------
// D-7b: one session-monotonic clip allocator
// ---------------------------------------------------------------------------

/// The demo's first top-level instrument track, where the clip tests below
/// create their clips.
fn instrument_track(app: &Resonance) -> u64 {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| matches!(t.track_type, TrackType::Instrument) && t.sub_track.is_none())
        .expect("the demo has an instrument track")
        .id
}

/// One `notes.create_clip` — an app-allocated clip id, read back from the
/// reply — echoed, so the mirror holds the clip.
fn create_clip(f: &mut Fixture, bar: u32) -> ClipId {
    let track_id = instrument_track(&f.app);
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
}

/// Save the live project to `dir` (a bundle other than the fixture's own).
fn save_to(app: &Resonance, dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("audio")).expect("bundle dir");
    let file = app.test_build_project_file();
    let midi_clips: Vec<(ClipId, Vec<MidiNote>)> = app
        .test_midi_clips()
        .iter()
        .map(|mc| (mc.id, mc.notes.as_ref().clone()))
        .collect();
    project::save_project(dir, &file, &[], &midi_clips).expect("save");
}

/// Replay `loaded` into the fixture's app the way a disk open does, then
/// point the session at `dir`.
fn open(f: &mut Fixture, loaded: project::LoadedProject, dir: &std::path::Path) {
    f.app.test_replay_loaded_project_from(loaded);
    echo(&mut f.app, &f.rx, &mut f.engine);
    f.app.test_set_active_project(true);
    f.app.test_set_project_path(dir.to_path_buf());
}

/// D-7b (design doc D-6 §4.1, decision §7a.2): the clip allocator is
/// session-monotonic — no undo, redo, second project load or Save As ever
/// lowers it, so every clip id this session hands out is strictly larger
/// than the one before.
///
/// The second-load step is the behaviour change: before D-7b a disk load
/// reset the counter to the base and reserved past the loaded clips only,
/// so opening an earlier save of the same project handed out ids this
/// session had already used (and whose `clip_<id>.wav` a vocal render may
/// still hold).
#[test]
fn clip_ids_strictly_increase_across_undo_redo_a_second_load_and_save_as() {
    let mut f = fixture("clip-monotonic");
    let root = f.project.parent().unwrap().to_path_buf();
    let mut issued: Vec<ClipId> = Vec::new();
    let assert_next = |issued: &mut Vec<ClipId>, id: ClipId, when: &str| {
        if let Some(last) = issued.last() {
            assert!(
                id > *last,
                "{when}: clip id {id} is not above the last one issued ({last}); all: {issued:?}"
            );
        }
        issued.push(id);
    };

    let a = create_clip(&mut f, 40);
    assert_next(&mut issued, a, "first clip");
    // Project B: this project as it stood after one clip.
    let bundle_b = root.join("earlier.rproj");
    save_to(&f.app, &bundle_b);

    let b = create_clip(&mut f, 44);
    assert_next(&mut issued, b, "second clip");
    let _ = f.app.update(Message::Undo);
    echo(&mut f.app, &f.rx, &mut f.engine);
    assert!(!f.app.test_midi_clips().iter().any(|c| c.id == b), "undo removed clip {b}");
    let _ = f.app.update(Message::Redo);
    echo(&mut f.app, &f.rx, &mut f.engine);
    assert!(f.app.test_midi_clips().iter().any(|c| c.id == b), "redo restored clip {b}");
    let c = create_clip(&mut f, 48);
    assert_next(&mut issued, c, "after undo + redo");
    let _ = f.app.update(Message::Undo);
    echo(&mut f.app, &f.rx, &mut f.engine);
    let d = create_clip(&mut f, 48);
    assert_next(&mut issued, d, "after undoing a create");

    // A second project in the same session, whose clips all sit below
    // what this session has already issued: the counter is not reset.
    let loaded = project::load_project(&bundle_b).expect("load B");
    open(&mut f, loaded, &bundle_b);
    assert!(f.app.test_midi_clips().iter().any(|mc| mc.id == a), "B holds clip {a}");
    let e = create_clip(&mut f, 52);
    assert_next(&mut issued, e, "after opening a second project");

    // Save As into a bundle that already holds a clip WAV above the
    // counter: the session writes clip WAVs there from now on.
    let bundle_c = root.join("existing.rproj");
    std::fs::create_dir_all(bundle_c.join("audio")).expect("bundle dir");
    let orphan = e + 100;
    std::fs::write(bundle_c.join(format!("audio/clip_{orphan}.wav")), b"orphan")
        .expect("orphan");
    let _ = f.app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(
        bundle_c.to_string_lossy().into_owned(),
    ))));
    let g = create_clip(&mut f, 56);
    assert!(g > orphan, "the clip after Save As ({g}) would re-issue clip_{orphan}.wav");
    assert_next(&mut issued, g, "after Save As");
}

/// D-7b: a loaded project raises the clip counter past every clip id it
/// names — a MIDI clip, an audio take's `clip_ref` (which no mirrored clip
/// carries), and an `audio/clip_<id>.wav` on disk that nothing names any
/// more. Each step's id sits above the previous one's, so every step has to
/// raise the counter on its own.
#[test]
fn a_loaded_projects_clips_takes_and_wavs_raise_the_clip_counter() {
    use std::collections::HashMap;
    use std::sync::Arc;

    let mut f = fixture("clip-seed");
    let dir = f.project.clone();
    let loaded_with = |file: project::ProjectFile, midi_notes: HashMap<ClipId, Arc<Vec<MidiNote>>>| {
        project::LoadedProject {
            file,
            project_dir: dir.clone(),
            midi_notes,
            plugin_states: HashMap::new(),
        }
    };

    // A MIDI clip far above the counter.
    let high_midi = CLIP_ID_BASE + 1_000;
    let low = create_clip(&mut f, 40);
    let mut file = f.app.test_build_project_file();
    let clip = file.midi_clips.iter_mut().find(|c| c.id == low).expect("the clip is saved");
    clip.id = high_midi;
    let notes = [(high_midi, Arc::new(Vec::new()))].into_iter().collect();
    open(&mut f, loaded_with(file, notes), &dir);
    assert!(f.app.test_midi_clips().iter().any(|c| c.id == high_midi), "the clip loaded");
    let next = create_clip(&mut f, 44);
    assert!(next > high_midi, "clip id {next} is not above the loaded MIDI clip {high_midi}");

    // An audio take whose recording is `clip_<ref>.wav`, missing from the
    // bundle (kept and flagged, not dropped) — nothing but the take names
    // the id.
    let high_take = CLIP_ID_BASE + 2_000;
    let audio_track = f
        .app
        .test_registry()
        .tracks
        .iter()
        .find(|t| matches!(t.track_type, TrackType::Audio))
        .expect("the demo has an audio track")
        .id;
    let slot = resonance_common::TimelineRange { start: 0, length: 96_000 };
    f.app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: 1,
        take_id: 0,
        track_id: audio_track,
        slot,
        pass_index: 0,
        extent: slot,
        content: resonance_common::TakeContent::Audio { clip_ref: high_take },
    });
    let file = f.app.test_build_project_file();
    assert!(
        file.take_groups.iter().flat_map(|g| &g.takes).any(|t| matches!(
            t.content,
            resonance_common::TakeContent::Audio { clip_ref } if clip_ref == high_take
        )),
        "the take is saved"
    );
    open(&mut f, loaded_with(file, HashMap::new()), &dir);
    let next = create_clip(&mut f, 48);
    assert!(next > high_take, "clip id {next} is not above the loaded take's clip_ref {high_take}");

    // A clip WAV in the bundle that nothing in the file names.
    let high_wav = CLIP_ID_BASE + 3_000;
    std::fs::write(dir.join(format!("audio/clip_{high_wav}.wav")), b"orphan").expect("orphan");
    let file = f.app.test_build_project_file();
    open(&mut f, loaded_with(file, HashMap::new()), &dir);
    let next = create_clip(&mut f, 52);
    assert!(next > high_wav, "clip id {next} would re-issue clip_{high_wav}.wav on disk");
}

// ---------------------------------------------------------------------------
// D-7d: the engine's clip ids come from grants of the same counter
// ---------------------------------------------------------------------------

fn grants(cmds: &[AudioCommand]) -> Vec<std::ops::Range<u64>> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::GrantIds(b) => Some(b.clips.clone()),
            _ => None,
        })
        .collect()
}

/// D-7d (design doc D-6 §4.2 / §4.4): a disk load's replay ends by
/// granting the engine a fresh block — its `ClearAll` revoked the old one —
/// taken from the counter after the load seeded it, so the block sits above
/// every id the loaded project holds and no app clip is ever allocated
/// inside it. An undo (no `ClearAll`) grants nothing.
#[test]
fn a_disk_load_replay_ends_by_granting_ids_above_everything_it_loaded() {
    use std::collections::HashMap;
    use std::sync::Arc;

    let mut f = fixture("grant-replay");
    let dir = f.project.clone();

    // An undo takes the diff path and keeps the engine's grant.
    let low = create_clip(&mut f, 40);
    let _: Vec<AudioCommand> = f.rx.try_iter().collect();
    let _ = f.app.update(Message::Undo);
    let cmds: Vec<AudioCommand> = f.rx.try_iter().collect();
    assert!(grants(&cmds).is_empty(), "an undo grants nothing: {cmds:?}");
    let _ = f.app.update(Message::Redo);
    echo(&mut f.app, &f.rx, &mut f.engine);

    let high = CLIP_ID_BASE + 50_000;
    let mut file = f.app.test_build_project_file();
    file.midi_clips.iter_mut().find(|c| c.id == low).expect("saved").id = high;
    let notes = [(high, Arc::new(Vec::new()))].into_iter().collect();
    f.app.test_replay_loaded_project_from(project::LoadedProject {
        file,
        project_dir: dir,
        midi_notes: notes,
        plugin_states: HashMap::new(),
    });
    let cmds: Vec<AudioCommand> = f.rx.try_iter().collect();
    let granted = grants(&cmds);
    assert_eq!(granted.len(), 1, "one grant per replay: {granted:?}");
    assert!(
        matches!(cmds.last(), Some(AudioCommand::GrantIds(_))),
        "the grant is the replay's last command"
    );
    let block = &granted[0];
    assert_eq!(block.end - block.start, resonance_audio::types::CLIP_GRANT_SIZE);
    assert!(block.start > high, "grant {block:?} overlaps the loaded clip {high}");
    assert_eq!(f.app.test_next_clip_id(), block.end, "the granted ids count as issued");
}

/// D-7d: `IdGrantLow` tops the engine's grant up with the next block of
/// the counter — except while a load is in flight: a low report raised
/// before that load's `ClearAll` would grant from a counter not yet seeded
/// past the incoming project. The replay's own closing grant covers it.
#[test]
fn id_grant_low_refills_the_engine_unless_a_load_is_in_flight() {
    use std::collections::HashMap;

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let low = AudioEvent::IdGrantLow { clips_left: 3 };
    app.test_apply_engine_event(low.clone());
    let first = grants(&rx.try_iter().collect::<Vec<_>>());
    let size = resonance_audio::types::CLIP_GRANT_SIZE;
    assert_eq!(first, vec![CLIP_ID_BASE..CLIP_ID_BASE + size]);

    let dir = std::env::temp_dir().join(format!("resonance-grant-load-{}", std::process::id()));
    let loaded = project::LoadedProject {
        file: project::ProjectFile::default(),
        project_dir: dir,
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    };
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(Box::new(loaded)))));
    let cmds: Vec<AudioCommand> = rx.try_iter().collect();
    assert!(cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)));
    app.test_apply_engine_event(low);
    assert!(
        grants(&rx.try_iter().collect::<Vec<_>>()).is_empty(),
        "no grant while the load is in flight"
    );
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let replay = grants(&rx.try_iter().collect::<Vec<_>>());
    assert_eq!(replay.len(), 1, "the replay's own grant: {replay:?}");
    assert!(replay[0].start >= first[0].end, "never below an earlier grant");
}

/// D-7d hammer: app-allocated clips (`notes.create_clip`) and the engine's
/// recordings — drawn in order from the grants, refilled on `IdGrantLow`,
/// revoked at a reload's `ClearAll` — interleaved with undos and reloads.
/// No id is ever issued twice, no app clip lands inside a granted block,
/// no two grants overlap, and the engine's ids strictly increase for the
/// whole session (STATE-08: a record after a full undo or a reload gets a
/// larger id than any before it).
#[test]
fn engine_granted_and_app_allocated_clip_ids_never_collide() {
    let mut f = fixture("grant-hammer");
    let audio_track = f
        .app
        .test_registry()
        .tracks
        .iter()
        .find(|t| matches!(t.track_type, TrackType::Audio) && t.sub_track.is_none())
        .expect("the demo has an audio track")
        .id;
    // The startup grant.
    f.app.test_apply_engine_event(AudioEvent::IdGrantLow { clips_left: 0 });
    echo(&mut f.app, &f.rx, &mut f.engine);

    let mut app_ids: Vec<ClipId> = Vec::new();
    let mut engine_ids: Vec<ClipId> = Vec::new();
    for round in 0..60u32 {
        app_ids.push(create_clip(&mut f, 40 + round * 4));
        // A few recordings: each draws from the grant, asking for more
        // below the mark as the engine does.
        for _ in 0..(round % 4) * 150 {
            let id = f.engine.draw().expect("the grant is kept topped up");
            if f.engine.grant_left() == resonance_audio::types::CLIP_GRANT_LOW_WATER - 1 {
                f.app.test_apply_engine_event(AudioEvent::IdGrantLow {
                    clips_left: f.engine.grant_left(),
                });
                echo(&mut f.app, &f.rx, &mut f.engine);
            }
            engine_ids.push(id);
        }
        if let Some(&id) = engine_ids.last() {
            f.app.test_apply_engine_event(AudioEvent::RecordingFinished {
                clip_id: id,
                track_id: audio_track,
                start_sample: 0,
                duration_samples: 480,
                name: "Recording".into(),
                waveform_peaks: Vec::new(),
            });
        }
        if round % 7 == 3 {
            let _ = f.app.update(Message::Undo);
            echo(&mut f.app, &f.rx, &mut f.engine);
        }
        if round % 17 == 9 {
            // What the reload's `ClearAll` does to the engine's grant.
            f.engine.grant.clear();
            save_and_reload(&mut f);
        }
    }

    assert!(
        engine_ids.windows(2).all(|w| w[0] < w[1]),
        "the engine's clip ids strictly increase across undos and reloads"
    );
    let mut all: Vec<ClipId> = app_ids.iter().chain(&engine_ids).copied().collect();
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), n, "an id was issued twice");
    let granted = &f.engine.granted;
    assert!(granted.len() > 3, "the run refilled and reloaded: {granted:?}");
    for id in &app_ids {
        assert!(
            !granted.iter().any(|g| g.contains(id)),
            "app clip {id} lies inside a grant"
        );
    }
    let mut sorted = granted.clone();
    sorted.sort_by_key(|r| r.start);
    assert!(sorted.windows(2).all(|w| w[0].end <= w[1].start), "grants overlap: {sorted:?}");
}
