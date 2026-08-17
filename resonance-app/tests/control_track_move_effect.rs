//! `track.move_effect` — reordering a track's insert chain over the
//! control API (ba doc #273, todo #1225).
//!
//! `track.add_effect` only ever appends, so before this the order of a
//! chain was whatever order it was built in, and correcting it meant
//! tearing the chain down and rebuilding it — losing every parameter set
//! along the way. Order is audible (an EQ before a compressor is a
//! different sound from an EQ after it), so this is the last piece of
//! the chain surface, not a convenience.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, ScannedPlugin, TrackType};
use resonance_control::methods::track::PluginParamsView;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-move-effect.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/compressor.clap".to_owned(),
                clap_plugin_id: "com.resonance.compressor".to_owned(),
                name: "Resonance Compressor".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/mastering.clap".to_owned(),
                clap_plugin_id: "com.resonance.mastering".to_owned(),
                name: "Resonance Mastering".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
        ],
    });
    app
}

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

fn add(app: &mut Resonance, plugin_id: &str) {
    let _: serde_json::Value = call(
        app,
        "track.add_effect",
        serde_json::json!({"track_id": TRACK, "plugin_id": plugin_id}),
    )
    .result()
    .expect("track.add_effect succeeds");
}

fn add_instrument(app: &mut Resonance) {
    let _: serde_json::Value = call(
        app,
        "track.add_instrument",
        serde_json::json!({"track_id": TRACK, "plugin_id": "com.resonance.wavetable"}),
    )
    .result()
    .expect("track.add_instrument succeeds");
}

fn order(app: &mut Resonance) -> Vec<String> {
    let view: PluginParamsView = call(
        app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params succeeds");
    // Slots must always be a dense ascending run, whatever the order.
    let slots: Vec<u32> = view.plugins.iter().map(|p| p.slot).collect();
    assert_eq!(slots, (0..slots.len() as u32).collect::<Vec<_>>());
    view.plugins.into_iter().map(|p| p.plugin_id).collect()
}

fn mv(app: &mut Resonance, params: serde_json::Value) -> Response {
    let mut p = params;
    p["track_id"] = serde_json::json!(TRACK);
    call(app, "track.move_effect", p)
}

/// The short name after the last dot, so assertions read as
/// `["eq", "compressor"]`.
fn short(ids: &[String]) -> Vec<&str> {
    ids.iter()
        .map(|s| s.rsplit('.').next().unwrap_or(s))
        .collect()
}

#[test]
fn moving_a_compressor_in_front_of_an_eq_reorders_the_chain() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");
    assert_eq!(short(&order(&mut app)), vec!["eq", "compressor"]);

    let rx = app.test_capture_engine();
    let _: MutationAck = mv(
        &mut app,
        serde_json::json!({"plugin_id": "com.resonance.compressor", "to_slot": 0}),
    )
    .result()
    .expect("track.move_effect succeeds");

    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::MovePlugin { to_index: 0, .. })),
        "the engine must be told to reorder its own chain too"
    );
    assert_eq!(
        short(&order(&mut app)),
        vec!["compressor", "eq"],
        "and the app reads back the new order in the same cycle"
    );
}

#[test]
fn the_engine_echo_replays_the_same_move_without_disturbing_it() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");
    let rx = app.test_capture_engine();
    let _: MutationAck = mv(&mut app, serde_json::json!({"slot": 1, "to_slot": 0}))
        .result()
        .expect("succeeds");
    let instance_id = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::MovePlugin { instance_id, .. } => Some(instance_id),
            _ => None,
        })
        .expect("a MovePlugin reached the engine");

    app.test_apply_engine_event(AudioEvent::PluginMoved {
        track_id: TRACK,
        instance_id,
        to_index: 0,
    });
    assert_eq!(
        short(&order(&mut app)),
        vec!["compressor", "eq"],
        "the echo replays a move already applied and must be a no-op"
    );
}

#[test]
fn addressing_by_slot_and_by_occurrence_pick_the_same_plugin() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.mastering");

    // The SECOND EQ, moved to the end.
    let _: MutationAck = mv(
        &mut app,
        serde_json::json!({"plugin_id": "com.resonance.eq", "occurrence": 1, "to_slot": 2}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(short(&order(&mut app)), vec!["eq", "mastering", "eq"]);

    // ...and back to slot 1 by its new slot.
    let _: MutationAck = mv(&mut app, serde_json::json!({"slot": 2, "to_slot": 1}))
        .result()
        .expect("succeeds");
    assert_eq!(short(&order(&mut app)), vec!["eq", "eq", "mastering"]);
}

#[test]
fn a_to_slot_past_the_end_clamps_rather_than_erroring() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");
    add(&mut app, "com.resonance.mastering");

    let _: MutationAck = mv(&mut app, serde_json::json!({"slot": 0, "to_slot": 99}))
        .result()
        .expect("an out-of-range destination means 'the end', not an error");
    assert_eq!(short(&order(&mut app)), vec!["compressor", "mastering", "eq"]);
}

#[test]
fn moving_an_effect_to_where_it_already_sits_is_an_accepted_no_op() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");
    let before = app.revision();

    let rx = app.test_capture_engine();
    let _: MutationAck = mv(&mut app, serde_json::json!({"slot": 1, "to_slot": 1}))
        .result()
        .expect("succeeds");
    assert_eq!(app.revision(), before, "no undo entry for a no-op");
    assert!(
        !std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::MovePlugin { .. })),
        "and no engine command"
    );
    assert_eq!(short(&order(&mut app)), vec!["eq", "compressor"]);
}

#[test]
fn the_instrument_can_neither_be_moved_nor_displaced() {
    let mut app = app();
    add_instrument(&mut app);
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");
    assert_eq!(
        short(&order(&mut app)),
        vec!["wavetable", "eq", "compressor"]
    );

    // Moving the instrument itself.
    for params in [
        serde_json::json!({"slot": 0, "to_slot": 2}),
        serde_json::json!({"plugin_id": "com.resonance.wavetable", "to_slot": 2}),
    ] {
        let error = mv(&mut app, params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} must be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
        assert!(
            error.message.contains("INSTRUMENT"),
            "the error must say why: {}",
            error.message
        );
    }

    // Moving an effect ON TOP of the instrument.
    let error = mv(&mut app, serde_json::json!({"slot": 2, "to_slot": 0}))
        .error
        .expect("an effect cannot displace the instrument");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("to_slot must be at least 1"),
        "and must say where effects may go: {}",
        error.message
    );

    assert_eq!(
        short(&order(&mut app)),
        vec!["wavetable", "eq", "compressor"],
        "nothing moved"
    );
}

#[test]
fn addressing_must_be_unambiguous_and_misses_are_not_found() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");

    for params in [
        serde_json::json!({"to_slot": 0}),
        serde_json::json!({"slot": 0, "plugin_id": "com.resonance.eq", "to_slot": 0}),
    ] {
        let error = mv(&mut app, params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} must be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }
    // The "which effect?" message must name this verb, not removal.
    let error = mv(&mut app, serde_json::json!({"to_slot": 0}))
        .error
        .expect("rejected");
    assert!(
        error.message.contains("to move"),
        "the rejection is worded for a move: {}",
        error.message
    );

    let error = mv(&mut app, serde_json::json!({"slot": 9, "to_slot": 0}))
        .error
        .expect("missing slot rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    let error = mv(
        &mut app,
        serde_json::json!({"plugin_id": "com.resonance.reverb", "to_slot": 0}),
    )
    .error
    .expect("effect not on the track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    let error = call(
        &mut app,
        "track.move_effect",
        serde_json::json!({"track_id": 4242, "slot": 0, "to_slot": 1}),
    )
    .error
    .expect("unknown track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn the_move_is_a_committed_undoable_edit() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");

    let before = app.revision();
    let _: MutationAck = mv(&mut app, serde_json::json!({"slot": 1, "to_slot": 0}))
        .result()
        .expect("succeeds");
    assert_eq!(app.revision(), before + 1, "the move is a committed edit");

    // Chain order is structural, so undo takes the ClearAll -> replay
    // path (the same one `track.remove_effect`'s undo takes); the engine
    // round-trip that finishes it is asynchronous, so assert the restore
    // actually starts.
    let rx = app.test_capture_engine();
    let _ = app.update(Message::Undo);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find the move and start restoring the pre-move snapshot"
    );
}

/// The instrument-floor rule belongs to the chain, not to the control
/// API that happens to be its only caller today (ba todo #1261).
///
/// `PluginMessage::MovePluginInTrack` is the domain message every
/// reorder path goes through — the control handler delegates to it, and
/// a future mixer drag-to-reorder would emit it directly. Sending it
/// straight, bypassing `track.move_effect` and its wire validation, must
/// still leave the instrument in slot 0: otherwise the first GUI caller
/// silently displaces the track's sound source and serializes the wrong
/// order into the project.
#[test]
fn the_domain_message_alone_cannot_displace_the_instrument() {
    let mut app = app();
    add_instrument(&mut app);
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");
    assert_eq!(
        short(&order(&mut app)),
        vec!["wavetable", "eq", "compressor"]
    );

    // The compressor's instance id, addressed the way a GUI would: by
    // what sits in the chain, not by a wire parameter.
    let instance_id = app
        .test_track_plugin_instance_ids(TRACK)
        .into_iter()
        .nth(2)
        .expect("three plugins on the chain");

    let _ = app.update(Message::Plugin(
        resonance_app::message::PluginMessage::MovePluginInTrack {
            track_id: TRACK,
            instance_id,
            to_index: 0,
        },
    ));

    assert_eq!(
        short(&order(&mut app)),
        vec!["wavetable", "eq", "compressor"],
        "an effect moved to slot 0 must not displace the instrument"
    );

    // The same message with a legal destination still works, so the
    // guard refuses the invalid move rather than disabling the path.
    let _ = app.update(Message::Plugin(
        resonance_app::message::PluginMessage::MovePluginInTrack {
            track_id: TRACK,
            instance_id,
            to_index: 1,
        },
    ));
    assert_eq!(
        short(&order(&mut app)),
        vec!["wavetable", "compressor", "eq"],
        "a move that respects the floor still reorders"
    );
}

/// The rule is "the instrument stays put", not merely "nothing lands
/// below the floor" (ba todo #1261, review follow-up).
///
/// Checking only the destination lets the INSTRUMENT walk *down* its own
/// chain: sending it to the last slot clears a floor of 1, and once it
/// sits at slot 2 the floor becomes 3 while the last slot is 2 — so it
/// can never be moved back. The chain is then permanently wrong and no
/// further move can repair it.
#[test]
fn the_instrument_cannot_walk_down_its_own_chain() {
    let mut app = app();
    add_instrument(&mut app);
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");

    let instrument_id = app
        .test_track_plugin_instance_ids(TRACK)
        .into_iter()
        .next()
        .expect("the instrument is in slot 0");

    let _ = app.update(Message::Plugin(
        resonance_app::message::PluginMessage::MovePluginInTrack {
            track_id: TRACK,
            instance_id: instrument_id,
            to_index: 2,
        },
    ));

    assert_eq!(
        short(&order(&mut app)),
        vec!["wavetable", "eq", "compressor"],
        "the instrument must not be movable to the end of its own chain"
    );
}

/// A refused move must not spend an undo entry or bump the revision
/// (ba todo #1261, review follow-up).
///
/// `update_inner` records undo and bumps the revision BEFORE dispatch,
/// so refusing inside the handler would leave a phantom edit behind: a
/// control client polling `revision` sees a concurrent-edit signal that
/// did not happen, and the user's next undo is consumed restoring an
/// identical snapshot. The refusal therefore has to gate pre-dispatch.
#[test]
fn a_refused_move_costs_neither_a_revision_nor_an_undo_entry() {
    let mut app = app();
    add_instrument(&mut app);
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.compressor");

    let compressor = app
        .test_track_plugin_instance_ids(TRACK)
        .into_iter()
        .nth(2)
        .expect("three plugins on the chain");

    let revision_before = app.revision();
    let undo_label_before = app
        .test_undo_history()
        .undo_label()
        .map(|s| s.to_owned());

    let _ = app.update(Message::Plugin(
        resonance_app::message::PluginMessage::MovePluginInTrack {
            track_id: TRACK,
            instance_id: compressor,
            to_index: 0,
        },
    ));

    assert_eq!(
        app.revision(),
        revision_before,
        "a refused move must not bump the revision"
    );
    assert_eq!(
        app.test_undo_history().undo_label().map(|s| s.to_owned()),
        undo_label_before,
        "a refused move must not push an undo entry — the top of the \
         stack must still be whatever the last real edit was"
    );
}
