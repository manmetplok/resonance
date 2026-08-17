//! The mixer inspector's SENDS block (ba todo #1310, doc #172; finding
//! P3 of the capability-vs-exposure audit, doc #275).
//!
//! Aux sends shipped backend-first and stayed there. The engine model,
//! the tap-and-sum, the `MixerMessage` variants, undo, the mirror,
//! persistence and `track.add_send` / `set_send` / `remove_send` were all
//! done — but the ROUTING group drew two hardcoded read-only rows,
//! `Send A -> (none)` and `Send B -> (none)`, and nothing in the entire
//! view tree raised a single one of those messages. An agent could build
//! a reverb send; a human could not.
//!
//! **The acceptance criterion is two-way.** A send made over the control
//! API must appear in the panel, and a send made in the panel must read
//! back over the control API — because both are supposed to be the same
//! graph seen from two ends. So these tests always drive one side and
//! assert the other:
//!
//! * `test_send_affordances` / `test_add_send_options` return exactly
//!   what the ROUTING group renders, together with the messages its
//!   controls raise, so pressing them here is pressing the real panel.
//! * `song.tracks` is read back through the control socket, the same
//!   round trip an MCP client makes.
//!
//! The engine is the single writer of the send graph, so — as in
//! `control_sends.rs` and `aux_send_persistence.rs` — the tests pump the
//! `AuxSendChanged` / `AuxSendRemoved` / `BusAdded` / `BusRoleChanged`
//! echoes the real engine would emit.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, SendSlotAffordances, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, SendSource, TrackType};
use resonance_control::methods::bus::CreateResult;
use resonance_control::methods::song::{SendView, TracksView};
use resonance_control::methods::track::AddSendResult;
use resonance_control::{MutationAck, Request, Response};

const GUITAR: u64 = 1;
const KEYS: u64 = 2;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/inspector-sends-test.rprj"));
    app.test_add_track(GUITAR, TrackType::Instrument);
    app.test_add_track(KEYS, TrackType::Instrument);
    app
}

// ---------------------------------------------------------------------------
// Control-socket plumbing — the same round trip an MCP client makes.
// ---------------------------------------------------------------------------

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

/// Every send `song.tracks` reports for a track — the API's side of the
/// two-way check.
fn api_sends(app: &mut Resonance, track_id: u64) -> Vec<SendView> {
    let view: TracksView = roundtrip(app, Request::without_params(99, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    view.tracks
        .into_iter()
        .find(|t| t.summary.id.0 == track_id)
        .expect("track in song.tracks")
        .sends
}

/// Every slot the mixer inspector's ROUTING group renders for a track —
/// the GUI's side.
fn panel_sends(app: &Resonance, track_id: u64) -> Vec<SendSlotAffordances> {
    app.test_send_affordances(track_id)
}

// ---------------------------------------------------------------------------
// Engine echoes. The engine owns the graph; nothing lands in the mirror
// (and therefore in the panel) until it says so.
// ---------------------------------------------------------------------------

fn echo_send(
    app: &mut Resonance,
    send_id: u64,
    track_id: u64,
    dest: u64,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id,
        source: SendSource::Track(track_id),
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

/// Replay the `SetAuxSend` / `RemoveAuxSend` / `AddBus` / `SetBusRole`
/// commands a GUI gesture emitted back as the events the engine would
/// answer with. `allocated` is the id the engine hands to a send created
/// without a hint. Returns the send ids that were touched.
fn echo_engine(app: &mut Resonance, cmds: &[AudioCommand], allocated: u64) -> Vec<u64> {
    let mut touched = Vec::new();
    for cmd in cmds {
        match cmd {
            AudioCommand::AddBus { id_hint, name } => {
                app.test_apply_engine_event(AudioEvent::BusAdded {
                    bus_id: id_hint.expect("the GUI always hints a bus id"),
                    name: name.clone().unwrap_or_default(),
                });
            }
            AudioCommand::SetBusRole { bus_id, is_return } => {
                app.test_apply_engine_event(AudioEvent::BusRoleChanged {
                    bus_id: *bus_id,
                    is_return: *is_return,
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
                let send_id = id_hint.unwrap_or(allocated);
                app.test_apply_engine_event(AudioEvent::AuxSendChanged {
                    send_id,
                    source: *source,
                    dest: *dest,
                    level_db: *level_db,
                    pre_fader: *pre_fader,
                    enabled: *enabled,
                });
                touched.push(send_id);
            }
            AudioCommand::RemoveAuxSend { send_id } => {
                app.test_apply_engine_event(AudioEvent::AuxSendRemoved {
                    send_id: *send_id,
                });
                touched.push(*send_id);
            }
            _ => {}
        }
    }
    touched
}

/// Press one panel affordance and pump the engine's answer, returning
/// the commands it produced.
fn press(app: &mut Resonance, message: Message, allocated: u64) -> Vec<AudioCommand> {
    let rx = app.test_capture_engine();
    let _ = app.update(message);
    let cmds: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    echo_engine(app, &cmds, allocated);
    cmds
}

fn create_return_bus(app: &mut Resonance, name: &str) -> u64 {
    let result: CreateResult = call(app, "bus.create", serde_json::json!({"name": name}))
        .result()
        .expect("bus.create succeeds");
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: result.bus_id.0,
        name: name.to_owned(),
    });
    app.test_apply_engine_event(AudioEvent::BusRoleChanged {
        bus_id: result.bus_id.0,
        is_return: true,
    });
    result.bus_id.0
}

fn api_add_send(app: &mut Resonance, track_id: u64, to_bus: u64, level_db: f32) -> u64 {
    let result: AddSendResult = call(
        app,
        "track.add_send",
        serde_json::json!({"track_id": track_id, "to_bus": to_bus, "level_db": level_db}),
    )
    .result()
    .expect("track.add_send succeeds");
    echo_send(app, result.send_id.0, track_id, to_bus, level_db, false, true);
    result.send_id.0
}

// ---------------------------------------------------------------------------
// The finding itself.
// ---------------------------------------------------------------------------

/// The regression the whole todo exists for: a track with no sends used
/// to render `Send A -> (none)` and `Send B -> (none)` no matter what
/// the graph said, and there was no way to add one. Now an empty track
/// renders no slots but always offers the add affordance.
#[test]
fn an_empty_track_renders_no_slots_but_can_still_grow_one() {
    let app = app();
    assert!(
        panel_sends(&app, GUITAR).is_empty(),
        "no sends means no slots — not two dead placeholders"
    );

    // No busses at all, yet the picker still leads somewhere: the
    // "New FX return…" entry creates the return and the send together.
    let options = app.test_add_send_options(GUITAR);
    assert_eq!(options.len(), 1, "only the create-a-return entry");
    assert!(
        options[0].0.starts_with("New FX return"),
        "unexpected add option: {}",
        options[0].0
    );
}

// ---------------------------------------------------------------------------
// Direction 1: control API -> GUI.
// ---------------------------------------------------------------------------

/// A send an agent builds over `track.add_send` shows up in the panel,
/// with its destination, level, tap point and enable state intact.
#[test]
fn a_send_made_over_the_control_api_appears_in_the_panel() {
    let mut app = app();
    let reverb = create_return_bus(&mut app, "Reverb");
    let send_id = api_add_send(&mut app, GUITAR, reverb, -6.0);

    let slots = panel_sends(&app, GUITAR);
    assert_eq!(slots.len(), 1, "the panel lists the API's send");
    assert_eq!(slots[0].send_id, send_id);
    assert_eq!(slots[0].dest_bus, reverb);
    assert!(
        slots[0].dest_label.contains("Reverb"),
        "the picker names the bus, not its id: {}",
        slots[0].dest_label
    );
    assert_eq!(slots[0].level_readout, "-6.0 dB");
    assert_eq!(slots[0].tap_label, "POST");
    assert!(slots[0].enabled);

    // …and only on the track that owns it.
    assert!(panel_sends(&app, KEYS).is_empty());

    // A later API edit reaches the panel too, since both read the one
    // engine-written mirror.
    let _: MutationAck = call(
        &mut app,
        "track.set_send",
        serde_json::json!({"send_id": send_id, "level_db": -3.5, "pre_fader": true, "enabled": false}),
    )
    .result()
    .expect("track.set_send succeeds");
    echo_send(&mut app, send_id, GUITAR, reverb, -3.5, true, false);

    let slots = panel_sends(&app, GUITAR);
    assert_eq!(slots[0].level_readout, "-3.5 dB");
    assert_eq!(slots[0].tap_label, "PRE");
    assert!(!slots[0].enabled);
}

/// A return bus the API created is offered as a destination; a plain bus
/// is not (an aux send targets returns — a plain bus is reached through
/// the OUTPUT picker instead).
#[test]
fn the_add_picker_offers_return_busses_only() {
    let mut app = app();
    let reverb = create_return_bus(&mut app, "Reverb");
    // A plain bus: created, never flagged as a return.
    let plain: CreateResult = call(&mut app, "bus.create", serde_json::json!({"name": "Drums"}))
        .result()
        .expect("bus.create succeeds");
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: plain.bus_id.0,
        name: "Drums".to_string(),
    });

    let labels: Vec<String> = app
        .test_add_send_options(GUITAR)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    assert!(labels.iter().any(|l| l.contains("Reverb")), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("Drums")), "{labels:?}");

    // Once the track already feeds the reverb, the picker stops offering
    // a second send into it.
    api_add_send(&mut app, GUITAR, reverb, 0.0);
    let labels: Vec<String> = app
        .test_add_send_options(GUITAR)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    assert!(!labels.iter().any(|l| l.contains("Reverb")), "{labels:?}");
    assert_eq!(labels.len(), 1, "only New FX return… is left: {labels:?}");
}

// ---------------------------------------------------------------------------
// Direction 2: GUI -> control API.
// ---------------------------------------------------------------------------

/// Picking "New FX return…" out of the panel's add picker builds the
/// return bus *and* the send, and the result reads back over
/// `song.tracks` exactly as `track.add_send` would have produced it.
#[test]
fn a_send_made_in_the_panel_reads_back_over_the_control_api() {
    let mut app = app();
    let (label, add) = app
        .test_add_send_options(GUITAR)
        .into_iter()
        .next()
        .expect("the create-a-return entry is always offered");
    assert!(label.starts_with("New FX return"));

    const ENGINE_SEND_ID: u64 = 77;
    let cmds = press(&mut app, add, ENGINE_SEND_ID);

    // The gesture is three ordered commands: make the bus, flag it a
    // return, route into it.
    assert!(
        cmds.iter()
            .any(|c| matches!(c, AudioCommand::AddBus { id_hint: Some(_), .. })),
        "{cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::SetBusRole { is_return: true, .. }
        )),
        "{cmds:?}"
    );

    // The API sees the send the GUI just made.
    let sends = api_sends(&mut app, GUITAR);
    assert_eq!(sends.len(), 1, "song.tracks reports the panel's send");
    assert_eq!(sends[0].send_id.0, ENGINE_SEND_ID);
    assert!(
        (sends[0].level_db - 0.0).abs() < 1e-4,
        "a fresh send is at unity"
    );
    assert!(!sends[0].pre_fader, "and post-fader");
    assert!(sends[0].enabled);

    // And the panel and the API agree on where it goes.
    let slots = panel_sends(&app, GUITAR);
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].dest_bus, sends[0].to_bus.0);
    assert!(
        slots[0].dest_label.contains("FX Return 1"),
        "the new return is named, not numbered: {}",
        slots[0].dest_label
    );
}

/// Every slot control raises the pre-existing `MixerMessage` that drives
/// the backend — level, tap point, enable, re-route and remove — and the
/// control API reads each edit back.
#[test]
fn every_slot_control_edits_the_send_the_api_can_see() {
    let mut app = app();
    let reverb = create_return_bus(&mut app, "Reverb");
    let delay = create_return_bus(&mut app, "Delay");
    let send_id = api_add_send(&mut app, GUITAR, reverb, 0.0);

    // Level slider.
    let slot = panel_sends(&app, GUITAR).remove(0);
    let cmds = press(&mut app, slot.set_level(-9.0), send_id);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::SetAuxSend { id_hint: Some(id), level_db, .. }
                if *id == send_id && (*level_db - -9.0).abs() < 1e-4
        )),
        "the slider must upsert the send's level: {cmds:?}"
    );
    assert!((api_sends(&mut app, GUITAR)[0].level_db - -9.0).abs() < 1e-4);
    assert_eq!(panel_sends(&app, GUITAR)[0].level_readout, "-9.0 dB");

    // PRE/POST toggle.
    let slot = panel_sends(&app, GUITAR).remove(0);
    assert_eq!(slot.tap_label, "POST");
    press(&mut app, slot.toggle_tap(), send_id);
    assert!(api_sends(&mut app, GUITAR)[0].pre_fader);
    assert_eq!(panel_sends(&app, GUITAR)[0].tap_label, "PRE");

    // ON toggle.
    let slot = panel_sends(&app, GUITAR).remove(0);
    press(&mut app, slot.toggle_enabled(), send_id);
    assert!(!api_sends(&mut app, GUITAR)[0].enabled);
    assert!(!panel_sends(&app, GUITAR)[0].enabled);

    // Destination picker — both returns are offered, and picking the
    // other one re-routes.
    let slot = panel_sends(&app, GUITAR).remove(0);
    let offered: Vec<u64> = slot.dest_options.iter().map(|(id, _)| *id).collect();
    assert!(offered.contains(&reverb) && offered.contains(&delay), "{offered:?}");
    press(&mut app, slot.reroute_to(delay), send_id);
    assert_eq!(api_sends(&mut app, GUITAR)[0].to_bus.0, delay);
    assert_eq!(panel_sends(&app, GUITAR)[0].dest_bus, delay);

    // Trash affordance.
    let slot = panel_sends(&app, GUITAR).remove(0);
    let cmds = press(&mut app, slot.remove(), send_id);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, AudioCommand::RemoveAuxSend { send_id: id } if *id == send_id)),
        "{cmds:?}"
    );
    assert!(api_sends(&mut app, GUITAR).is_empty());
    assert!(panel_sends(&app, GUITAR).is_empty());
}

/// The bus inspector's AUX RETURN toggle promotes an existing bus into a
/// send destination. Without it a bus made with `bus.create` could never
/// be picked in the panel at all.
#[test]
fn the_bus_return_toggle_makes_a_plain_bus_a_send_destination() {
    let mut app = app();
    let plain: CreateResult = call(&mut app, "bus.create", serde_json::json!({"name": "Plate"}))
        .result()
        .expect("bus.create succeeds");
    let bus_id = plain.bus_id.0;
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id,
        name: "Plate".to_string(),
    });

    let labels: Vec<String> = app
        .test_add_send_options(GUITAR)
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert!(!labels.iter().any(|l| l.contains("Plate")), "{labels:?}");

    // What the bus inspector's toggle raises.
    press(
        &mut app,
        Message::Mixer(resonance_app::message::MixerMessage::SetBusReturnRole(
            bus_id, true,
        )),
        0,
    );

    let labels: Vec<String> = app
        .test_add_send_options(GUITAR)
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert!(labels.iter().any(|l| l.contains("Plate")), "{labels:?}");
}

// ---------------------------------------------------------------------------
// Undo + persistence: the two things the panel must not break.
// ---------------------------------------------------------------------------

/// Panel edits go through the same `update` dispatch as everything else,
/// so the existing undo classification applies: a create and a remove
/// each record one entry, and a slider drag coalesces into one.
#[test]
fn panel_edits_record_undo_entries() {
    let mut app = app();
    let reverb = create_return_bus(&mut app, "Reverb");
    let send_id = api_add_send(&mut app, GUITAR, reverb, 0.0);
    let baseline = app.test_undo_history().test_undo_entries().len();

    let slot = panel_sends(&app, GUITAR).remove(0);
    press(&mut app, slot.toggle_tap(), send_id);
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        baseline + 1,
        "flipping the tap point is undoable"
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("send edit"));

    // A drag is many messages and exactly one entry (CoalesceKey::SendLevel).
    let before = app.test_undo_history().test_undo_entries().len();
    for db in [-1.0f32, -2.0, -3.0, -4.0] {
        let slot = panel_sends(&app, GUITAR).remove(0);
        press(&mut app, slot.set_level(db), send_id);
    }
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        before + 1,
        "a slider drag is one undo entry, not four"
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("send level"));

    let before = app.test_undo_history().test_undo_entries().len();
    let slot = panel_sends(&app, GUITAR).remove(0);
    press(&mut app, slot.remove(), send_id);
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        before + 1
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("remove send"));
}

/// A send built in the panel survives save + reload and comes back as
/// the same slot (the graph is persisted by ba todo #1269 — this pins
/// that the panel's own gesture feeds that path).
#[test]
fn a_panel_made_send_survives_save_and_reload() {
    let mut app = app();
    let (_, add) = app
        .test_add_send_options(GUITAR)
        .into_iter()
        .next()
        .expect("create-a-return entry");
    press(&mut app, add, 42);
    let slot = panel_sends(&app, GUITAR).remove(0);
    press(&mut app, slot.set_level(-4.5), 42);
    let slot = panel_sends(&app, GUITAR).remove(0);
    press(&mut app, slot.toggle_tap(), 42);

    let before = panel_sends(&app, GUITAR).remove(0);
    assert_eq!(before.level_readout, "-4.5 dB");
    assert_eq!(before.tap_label, "PRE");

    // Through the actual on-disk JSON hop.
    let file = app.test_build_project_file();
    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let back: resonance_app::project::ProjectFile =
        serde_json::from_str(&json).expect("deserialize");

    let (mut reloaded, _task) = Resonance::new();
    reloaded.test_replay_loaded_project(back);

    let after = panel_sends(&reloaded, GUITAR);
    assert_eq!(after.len(), 1, "the reloaded panel shows the send again");
    assert_eq!(after[0].dest_bus, before.dest_bus);
    assert_eq!(after[0].level_readout, "-4.5 dB");
    assert_eq!(after[0].tap_label, "PRE");
    assert!(
        after[0].dest_label.contains("FX Return 1"),
        "the return bus and its role came back too: {}",
        after[0].dest_label
    );
}

// ---------------------------------------------------------------------------
// The lazy region.
// ---------------------------------------------------------------------------

/// The SENDS block renders *inside* the inspector's `lazy(fp, …)`
/// region, so every field a slot draws has to be in the fingerprint —
/// otherwise the retained tree survives the engine's echo and the panel
/// shows a stale send. Same class of bug as ba todo #459.
#[test]
fn the_lazy_fingerprint_tracks_the_send_graph() {
    let mut app = app();
    let reverb = create_return_bus(&mut app, "Reverb");
    let empty = app.test_inspector_fingerprint(GUITAR).expect("track exists");

    let send_id = api_add_send(&mut app, GUITAR, reverb, 0.0);
    let created = app.test_inspector_fingerprint(GUITAR).expect("track exists");
    assert_ne!(empty, created, "a new send must redraw the routing group");

    let mut seen = vec![empty, created];
    for (label, edit) in [
        ("level", -9.0f32),
        ("level again", -12.0),
    ] {
        let slot = panel_sends(&app, GUITAR).remove(0);
        press(&mut app, slot.set_level(edit), send_id);
        let fp = app.test_inspector_fingerprint(GUITAR).expect("track exists");
        assert!(!seen.contains(&fp), "{label} must change the fingerprint");
        seen.push(fp);
    }

    for label in ["tap", "enable"] {
        let slot = panel_sends(&app, GUITAR).remove(0);
        let msg = if label == "tap" {
            slot.toggle_tap()
        } else {
            slot.toggle_enabled()
        };
        press(&mut app, msg, send_id);
        let fp = app.test_inspector_fingerprint(GUITAR).expect("track exists");
        assert!(!seen.contains(&fp), "{label} must change the fingerprint");
        seen.push(fp);
    }

    // A rejected route draws an inline note, so it has to redraw too.
    app.test_apply_engine_event(AudioEvent::AuxSendRejected {
        source: SendSource::Track(GUITAR),
        dest: reverb,
        reason: "that would feed back".to_string(),
    });
    let rejected = app.test_inspector_fingerprint(GUITAR).expect("track exists");
    assert!(
        !seen.contains(&rejected),
        "a rejection note must redraw the routing group"
    );
}
