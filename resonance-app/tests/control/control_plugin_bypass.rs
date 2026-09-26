//! Per-slot and whole-chain bypass over the control API (ba doc #275
//! finding X3, todo #1305).
//!
//! Two capabilities that share a word and are independent:
//!
//! * `<surface>.set_fx_bypass` mutes a whole chain. `bus` and `master`
//!   have had it; `track` had the mixer button and NO wire method at all,
//!   which is the audit's one inverse parity gap (ba doc #276 section 0).
//! * `<surface>.set_plugin_bypass` takes ONE slot out of the path. The
//!   inspector's "BYP" was a dead label until this todo.
//!
//! What these tests are really pinning is that the wire is a SET and not
//! a toggle. The app's own messages toggle — right for a button, wrong
//! for a wire, because a client that retries a request whose reply it
//! never saw would flip the state back.

use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent};
use resonance_control::methods::track::{AddResult, PluginParamsView};
use resonance_control::{ErrorKind, MutationAck};
use crate::common::call;

const PLUGIN: &str = "com.resonance.eq";
const OTHER: &str = "com.resonance.compressor";

fn app() -> (
    Resonance,
    resonance_audio::test_support::Receiver<AudioCommand>,
) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    (app, cmd_rx)
}

/// A bare audio track, created the way a client would.
fn add_track(app: &mut Resonance) -> u64 {
    let result: AddResult = call(app, "track.add", serde_json::json!({ "kind": "audio" }))
        .result()
        .expect("track.add succeeds");
    let id = u64::from(result.track_id);
    app.test_apply_engine_event(AudioEvent::TrackAdded { track_id: id });
    id
}

/// A track with two effects on it, mirrored as the engine would.
fn track_with_two_effects(app: &mut Resonance) -> (u64, u64, u64) {
    let track_id = add_track(app);
    for (instance_id, clap) in [(10u64, PLUGIN), (11u64, OTHER)] {
        app.test_apply_engine_event(AudioEvent::PluginAdded {
            track_id,
            instance_id,
            plugin_name: clap.to_owned(),
            clap_plugin_id: clap.to_owned(),
            clap_file_path: format!("/plugins/{clap}.clap"),
            params: Vec::new(),
            has_gui: false,
            has_sidechain_input: false,
            output_port_count: 1,
            output_port_names: vec!["Main".to_owned()],
        });
    }
    (track_id, 10, 11)
}

fn bypass_commands(
    rx: &resonance_audio::test_support::Receiver<AudioCommand>,
) -> Vec<(u64, bool)> {
    rx.try_iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginBypass {
                instance_id,
                bypassed,
            } => Some((instance_id, bypassed)),
            _ => None,
        })
        .collect()
}

fn slot_flags(app: &mut Resonance, track_id: u64) -> Vec<(String, bool)> {
    let view: PluginParamsView = call(
        app,
        "track.plugin_params",
        serde_json::json!({ "track_id": track_id }),
    )
    .result()
    .expect("plugin_params succeeds");
    view.plugins
        .into_iter()
        .map(|p| (p.plugin_id, p.bypassed))
        .collect()
}

#[test]
fn a_named_slot_is_bypassed_and_reported_back() {
    let (mut app, rx) = app();
    let (track_id, eq, _) = track_with_two_effects(&mut app);
    let _ = rx.try_iter().count();

    let ack: MutationAck = call(
        &mut app,
        "track.set_plugin_bypass",
        serde_json::json!({ "track_id": track_id, "plugin_id": PLUGIN, "bypassed": true }),
    )
    .result()
    .expect("set_plugin_bypass succeeds");
    let _ = ack;

    assert_eq!(
        bypass_commands(&rx),
        vec![(eq, true)],
        "exactly the addressed slot is bypassed, and only it"
    );

    // The app does not believe it yet — the engine crossfades, and its
    // echo is what moves the flag. Until then the read is still false,
    // which is the honest answer.
    assert_eq!(
        slot_flags(&mut app, track_id),
        vec![(PLUGIN.to_owned(), false), (OTHER.to_owned(), false)]
    );

    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id: eq,
        bypassed: true,
        own_bypass_param: false,
    });
    assert_eq!(
        slot_flags(&mut app, track_id),
        vec![(PLUGIN.to_owned(), true), (OTHER.to_owned(), false)],
        "plugin_params reports each slot's own flag"
    );
}

#[test]
fn setting_the_state_twice_does_not_toggle_it_back() {
    // The whole reason the wire SETS rather than toggles: a client that
    // never saw its reply retries, and a toggle would re-engage the
    // plugin it had just taken out.
    let (mut app, rx) = app();
    let (track_id, eq, _) = track_with_two_effects(&mut app);
    let _ = rx.try_iter().count();

    for _ in 0..3 {
        let _: MutationAck = call(
            &mut app,
            "track.set_plugin_bypass",
            serde_json::json!({ "track_id": track_id, "plugin_id": PLUGIN, "bypassed": true }),
        )
        .result()
        .expect("succeeds");
        app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
            instance_id: eq,
            bypassed: true,
            own_bypass_param: false,
        });
    }

    assert!(
        bypass_commands(&rx).iter().all(|(_, b)| *b),
        "every command asked for bypassed=true; none flipped it back"
    );
    assert_eq!(slot_flags(&mut app, track_id)[0].1, true);
}

#[test]
fn an_unknown_plugin_is_refused_with_the_chain_that_is_there() {
    let (mut app, _rx) = app();
    let (track_id, _, _) = track_with_two_effects(&mut app);

    let error = call(
        &mut app,
        "track.set_plugin_bypass",
        serde_json::json!({ "track_id": track_id, "plugin_id": "com.nope", "bypassed": true }),
    )
    .error
    .expect("an unknown plugin is refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    let detail = format!("{error:?}");
    assert!(
        detail.contains(PLUGIN) && detail.contains(OTHER),
        "the refusal must list what the chain actually carries, so a \
         caller can correct itself without another round trip: {detail}"
    );
}

#[test]
fn occurrence_picks_between_two_copies_of_one_plugin() {
    let (mut app, rx) = app();
    let track_id = add_track(&mut app);
    for instance_id in [20u64, 21u64] {
        app.test_apply_engine_event(AudioEvent::PluginAdded {
            track_id,
            instance_id,
            plugin_name: PLUGIN.to_owned(),
            clap_plugin_id: PLUGIN.to_owned(),
            clap_file_path: format!("/plugins/{PLUGIN}.clap"),
            params: Vec::new(),
            has_gui: false,
            has_sidechain_input: false,
            output_port_count: 1,
            output_port_names: vec!["Main".to_owned()],
        });
    }
    let _ = rx.try_iter().count();

    let _: MutationAck = call(
        &mut app,
        "track.set_plugin_bypass",
        serde_json::json!({
            "track_id": track_id, "plugin_id": PLUGIN, "occurrence": 1, "bypassed": true
        }),
    )
    .result()
    .expect("succeeds");

    assert_eq!(
        bypass_commands(&rx),
        vec![(21, true)],
        "occurrence 1 is the SECOND copy, not the first"
    );
}

#[test]
fn track_set_fx_bypass_exists_and_is_idempotent() {
    // The audit's inverse parity gap: the mixer strip has had this
    // button since forever and there was no wire method at all.
    let (mut app, _rx) = app();
    let (track_id, _, _) = track_with_two_effects(&mut app);

    let _: MutationAck = call(
        &mut app,
        "track.set_fx_bypass",
        serde_json::json!({ "track_id": track_id, "bypassed": true }),
    )
    .result()
    .expect("track.set_fx_bypass succeeds");
    let before = app.revision();

    // Setting the state it is already in is a no-op — no second undo
    // entry, no revision bump.
    let _: MutationAck = call(
        &mut app,
        "track.set_fx_bypass",
        serde_json::json!({ "track_id": track_id, "bypassed": true }),
    )
    .result()
    .expect("succeeds");
    assert_eq!(
        app.revision(),
        before,
        "re-setting the state it is already in must not count as an edit"
    );
}

#[test]
fn the_chain_bypass_and_the_slot_flags_are_independent() {
    // The property that makes re-engaging a chain predictable: muting
    // the whole chain must not forget which slots the user had
    // individually bypassed.
    let (mut app, rx) = app();
    let (track_id, eq, _) = track_with_two_effects(&mut app);
    let _ = rx.try_iter().count();

    let _: MutationAck = call(
        &mut app,
        "track.set_plugin_bypass",
        serde_json::json!({ "track_id": track_id, "plugin_id": PLUGIN, "bypassed": true }),
    )
    .result()
    .expect("succeeds");
    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id: eq,
        bypassed: true,
        own_bypass_param: false,
    });

    let _: MutationAck = call(
        &mut app,
        "track.set_fx_bypass",
        serde_json::json!({ "track_id": track_id, "bypassed": true }),
    )
    .result()
    .expect("succeeds");

    assert_eq!(
        slot_flags(&mut app, track_id),
        vec![(PLUGIN.to_owned(), true), (OTHER.to_owned(), false)],
        "a chain bypass must leave the per-slot flags exactly as they were"
    );
}
