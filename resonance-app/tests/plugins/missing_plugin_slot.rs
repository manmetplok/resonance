//! A missing plugin must be **visible**, **positioned** and
//! **recoverable** (ba doc #275 P5, todo #1309).
//!
//! [`missing_plugin_state_preserved`](super::missing_plugin_state_preserved)
//! covers the other half of the same finding: that a missing plugin's
//! SETTINGS survive. This file is about the slot the settings hang off.
//!
//! The state of affairs it exists to end:
//!
//! ```text
//! machine B: the .clap isn't installed
//!            -> AddPlugin fails, the engine answers a generic Error string
//!               with no instance in it
//!            -> the chain keeps a placeholder slot that renders exactly
//!               like a working plugin
//!            -> the user clicks it, sees an empty parameter panel, and
//!               concludes the plugin has no parameters
//!            -> no badge, no list of what is missing, and no way to put
//!               anything else in that position without moving it
//! ```
//!
//! Four properties are pinned here, and every one of them is a thing the
//! old build got wrong:
//!
//! 1. the slot is MARKED — in its own state, on the mixer strip, and in
//!    the load warning that lists what is missing;
//! 2. it KEEPS ITS POSITION, through the failure and through a replace;
//! 3. a **relocate** (the same plugin, found again) restores the settings
//!    the slot has been holding, and puts the instance back at the right
//!    index in the ENGINE's chain, which is numbered differently from
//!    the app's because the engine never had the missing plugin at all;
//! 4. a **swap** (a different plugin) keeps the position and drops the
//!    old plugin's state, because it cannot mean anything to the new one.

use resonance_app::message::{Message, PluginMessage, UiMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::{Resonance, TestChain};
use resonance_audio::types::{
    AudioCommand, AudioEvent, ParamInfo, PluginInstanceId, ScannedPlugin, TrackType,
};

const TRACK: u64 = 7;
const BUS: u64 = 3;

const VERB: u64 = 100;
const EQ: u64 = 101;
const COMP: u64 = 102;

const VERB_ID: &str = "com.thirdparty.mega-verb";
const EQ_ID: &str = "com.resonance.eq";
const COMP_ID: &str = "com.resonance.compressor";

/// The blob the project saved for the missing plugin. Byte-identity is
/// asserted, so any re-encode on the way through shows up as a failure.
const BLOB: &[u8] = &[0x00, 0xff, b'M', b'V', 0xde, 0xad];

fn catalog() -> Vec<ScannedPlugin> {
    vec![
        ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: EQ_ID.to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
        ScannedPlugin {
            clap_file_path: "/plugins/compressor.clap".to_owned(),
            clap_plugin_id: COMP_ID.to_owned(),
            name: "Resonance Compressor".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
    ]
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_apply_engine_event(AudioEvent::PluginsScanned { plugins: catalog() });
    app
}

fn slot(instance_id: PluginInstanceId, plugin_id: &str, name: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance_id,
        name.to_owned(),
        plugin_id.to_owned(),
        format!("/plugins/{plugin_id}.clap"),
        Vec::new(),
        false,
    )
}

/// A chain of three, exactly as a project load leaves it before any
/// engine echo has come back: every slot optimistically present.
fn chain_of_three(app: &mut Resonance) {
    app.test_push_track_plugin(TRACK, slot(VERB, VERB_ID, "MegaVerb"));
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID, "Resonance Compressor"));
}

/// The engine's answer for a plugin it could not instantiate.
fn refuse(app: &mut Resonance, instance_id: PluginInstanceId, plugin_id: &str) {
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(instance_id),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
}

/// The engine's answer for a plugin it did instantiate.
fn accept(app: &mut Resonance, instance_id: PluginInstanceId, plugin_id: &str) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: vec![ParamInfo {
            id: 1,
            name: "Mix".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.5,
            current_value: 0.5,
            ..Default::default()
        }],
        has_gui: true,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

fn catalog_entry(id: &str) -> ScannedPlugin {
    catalog()
        .into_iter()
        .find(|p| p.clap_plugin_id == id)
        .expect("catalog entry")
}

// ---------------------------------------------------------------------------
// 1. The slot is marked
// ---------------------------------------------------------------------------

/// The headline: after the engine refuses it, the slot says so — and the
/// two beside it are untouched. Before this, all three read identically.
#[test]
fn a_refused_plugin_marks_its_own_slot_and_only_its_own() {
    let mut app = app();
    chain_of_three(&mut app);
    refuse(&mut app, VERB, VERB_ID);

    assert_eq!(
        app.test_chain_slots(TestChain::Track(TRACK)),
        vec![
            (VERB, VERB_ID.to_owned(), true),
            (EQ, EQ_ID.to_owned(), false),
            (COMP, COMP_ID.to_owned(), false),
        ],
        "only the refused slot is missing, and the chain still has three slots in order"
    );
    assert_eq!(
        app.test_plugin_unavailable_reason(VERB).as_deref(),
        Some("Failed to load plugin: no such file"),
        "the loader's own words are kept, not replaced with a house string"
    );
}

/// A slot that comes back is no longer missing. Without this the badge
/// would be a one-way door and reinstalling the plugin would leave a
/// working plugin permanently marked broken.
#[test]
fn an_instance_that_turns_up_clears_the_mark() {
    let mut app = app();
    chain_of_three(&mut app);
    refuse(&mut app, VERB, VERB_ID);
    accept(&mut app, VERB, VERB_ID);

    assert_eq!(app.test_plugin_unavailable_reason(VERB), None);
    assert_eq!(
        app.test_chain_slots(TestChain::Track(TRACK))[0],
        (VERB, VERB_ID.to_owned(), false)
    );
}

/// The mixer strip's pill is what a user actually sees. It carries a
/// warning glyph, so the state is legible without relying on the tint.
#[test]
fn the_mixer_strip_pill_marks_a_missing_plugin() {
    let label = Resonance::test_strip_plugin_label;

    assert_eq!(label("MegaVerb", false), "MegaVerb");
    assert_eq!(label("MegaVerb", true), "\u{26a0} MegaVerb");
    assert!(
        label("A Very Long Plugin Name", true).chars().count()
            <= label("A Very Long Plugin Name", false).chars().count(),
        "the marker is paid for out of the name, not added to the pill's width budget"
    );
}

// ---------------------------------------------------------------------------
// 2. The load warning lists what is missing
// ---------------------------------------------------------------------------

/// "an audible/visible warning on project load lists what is missing
/// rather than a generic error" — the rows name the plugin AND where it
/// sits, which is exactly what the `AudioEvent::Error` string it
/// replaced could not do.
#[test]
fn the_load_warning_names_every_missing_plugin_and_where_it_sits() {
    let mut app = app();
    app.test_add_bus(BUS, "Drums");
    chain_of_three(&mut app);
    app.test_push_master_plugin(slot(200, VERB_ID, "MegaVerb"));

    refuse(&mut app, VERB, VERB_ID);
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(200),
        clap_plugin_id: VERB_ID.to_owned(),
        clap_file_path: "/plugins/mega-verb.clap".to_owned(),
        reason: "Failed to load plugin: no such file".to_owned(),
    });

    let rows = app
        .test_missing_plugin_warning()
        .expect("the warning is raised by the failure, not by anything the user did");
    assert_eq!(
        rows,
        vec![
            // `test_add_track` names by mixer order, not by id.
            "MegaVerb \u{2014} Track 1 track \u{b7} slot 1".to_owned(),
            "MegaVerb \u{2014} Master chain \u{b7} slot 1".to_owned(),
        ],
        "one row per dead slot, naming the plugin and its position"
    );
}

/// A project whose plugins are all present must not raise the warning at
/// all — a modal on every load would train the user to dismiss it.
#[test]
fn no_warning_when_nothing_is_missing() {
    let mut app = app();
    chain_of_three(&mut app);
    accept(&mut app, EQ, EQ_ID);

    assert_eq!(app.test_missing_plugin_warning(), None);
}

/// Dismissing has to stick. Failures arrive one engine event at a time,
/// so a dismissal that the next failure undid would make the modal look
/// un-closable on a project missing several plugins.
#[test]
fn dismissing_the_warning_survives_the_next_failure() {
    let mut app = app();
    chain_of_three(&mut app);
    refuse(&mut app, VERB, VERB_ID);
    assert!(app.test_missing_plugin_warning().is_some());

    let _ = app.update(Message::Ui(UiMessage::DismissMissingPlugins));
    assert_eq!(app.test_missing_plugin_warning(), None);

    refuse(&mut app, EQ, EQ_ID);
    assert_eq!(
        app.test_missing_plugin_warning(),
        None,
        "a second failure must not re-open a warning the user closed"
    );

    // ...but asking for it back works.
    let _ = app.update(Message::Ui(UiMessage::ShowMissingPlugins));
    assert_eq!(
        app.test_missing_plugin_warning().map(|r| r.len()),
        Some(2),
        "and it then shows BOTH, including the one that arrived after the dismissal"
    );
}

// ---------------------------------------------------------------------------
// 3. Relocate: the same plugin, found again
// ---------------------------------------------------------------------------

/// The recovery that gets the original sound back. The slot keeps its
/// instance id, so the settings parked against that id (ba todo #1308)
/// are still there to be pushed into the new instance.
#[test]
fn relocating_re_adds_under_the_same_instance_id_and_keeps_the_position() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(VERB, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(EQ, COMP_ID, "Resonance Compressor"));
    refuse(&mut app, VERB, EQ_ID);

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: VERB,
        plugin: catalog_entry(EQ_ID),
    }));

    let adds: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|c| match c {
            AudioCommand::AddPlugin {
                clap_plugin_id,
                id_hint,
                ..
            } => Some((clap_plugin_id, id_hint)),
            _ => None,
        })
        .collect();
    assert_eq!(
        adds,
        vec![(EQ_ID.to_owned(), Some(VERB))],
        "re-added under the SAME instance id — that id is what the preserved settings are keyed by"
    );
    assert_eq!(
        app.test_chain_slots(TestChain::Track(TRACK))[0].0,
        VERB,
        "and it is still the first slot in the chain"
    );
}

/// The subtle one. The engine's chain never contained the missing
/// plugin, so it is numbered differently from the app's: with slot 0
/// missing, the app's slot 1 is the engine's slot 0. When the missing
/// plugin comes back the engine APPENDS it, and the move that puts it
/// back has to use the engine's numbering — a move to the app's index
/// would run off the end of a shorter chain.
///
/// Both halves are asserted: the blob goes in, and the position is
/// restored, in that order.
#[test]
fn a_recovered_plugin_gets_its_state_back_and_is_moved_to_the_right_engine_slot() {
    // Two dead slots ahead of the one that recovers, so the app index
    // (2) and the engine index (0) are as far apart as this chain
    // allows. Counting SLOTS instead of LIVE slots would ask a
    // one-plugin engine chain to move something to index 2.
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(200, VERB_ID, "MegaVerb"));
    app.test_push_track_plugin(TRACK, slot(201, VERB_ID, "MegaVerb"));
    app.test_push_track_plugin(TRACK, slot(VERB, VERB_ID, "MegaVerb"));
    app.test_seed_plugin_state(VERB, BLOB.to_vec());
    refuse(&mut app, 200, VERB_ID);
    refuse(&mut app, 201, VERB_ID);
    refuse(&mut app, VERB, VERB_ID);

    let rx = app.test_capture_engine();
    accept(&mut app, VERB, VERB_ID);

    let mut restored_blob = None;
    let mut moved_to = None;
    for cmd in std::iter::from_fn(|| rx.try_recv().ok()) {
        match cmd {
            AudioCommand::LoadPluginState { instance_id, data } if instance_id == VERB => {
                restored_blob = Some(data)
            }
            AudioCommand::MovePlugin {
                instance_id,
                to_index,
                ..
            } if instance_id == VERB => moved_to = Some(to_index),
            _ => {}
        }
    }
    assert_eq!(
        restored_blob.as_deref(),
        Some(BLOB),
        "the settings the slot was holding are handed to the instance that finally appeared"
    );
    assert_eq!(
        moved_to,
        Some(0),
        "app slot 2, engine slot 0: the two dead slots ahead of it are not in the engine's \
         chain at all"
    );

    // And with one live slot in front of it, the engine index is 1 — the
    // count is of LIVE slots before, not a constant.
    let mut app = crate::missing_plugin_slot::app();
    app.test_push_track_plugin(TRACK, slot(200, VERB_ID, "MegaVerb"));
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(VERB, VERB_ID, "MegaVerb"));
    refuse(&mut app, 200, VERB_ID);
    accept(&mut app, EQ, EQ_ID);
    refuse(&mut app, VERB, VERB_ID);

    let rx = app.test_capture_engine();
    accept(&mut app, VERB, VERB_ID);
    let moved_to = std::iter::from_fn(|| rx.try_recv().ok()).find_map(|c| match c {
        AudioCommand::MovePlugin {
            instance_id,
            to_index,
            ..
        } if instance_id == VERB => Some(to_index),
        _ => None,
    });
    assert_eq!(
        moved_to,
        Some(1),
        "app slot 2, engine slot 1: one live slot before it"
    );
}

/// A relocate that fails again leaves the slot exactly as it was, marked
/// with the NEW reason. Recovery must be re-attemptable.
#[test]
fn a_relocate_that_fails_again_keeps_the_slot_and_updates_the_reason() {
    let mut app = app();
    // The slot carries the EQ, so naming the EQ again is a RELOCATE
    // (same plugin, found elsewhere) rather than a swap.
    app.test_push_track_plugin(TRACK, slot(VERB, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(EQ, COMP_ID, "Resonance Compressor"));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID, "Resonance Compressor"));
    refuse(&mut app, VERB, EQ_ID);

    let _ = app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: VERB,
        plugin: catalog_entry(EQ_ID),
    }));
    // Same instance, different failure the second time round.
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(VERB),
        clap_plugin_id: EQ_ID.to_owned(),
        clap_file_path: "/plugins/eq.clap".to_owned(),
        reason: "Failed to create plugin instance: unsupported sample rate".to_owned(),
    });

    assert_eq!(
        app.test_plugin_unavailable_reason(VERB).as_deref(),
        Some("Failed to create plugin instance: unsupported sample rate")
    );
    assert_eq!(
        app.test_chain_slots(TestChain::Track(TRACK)).len(),
        3,
        "the chain is intact"
    );
}

// ---------------------------------------------------------------------------
// 4. Swap: a different plugin takes the position
// ---------------------------------------------------------------------------

/// The DONE-WHEN clause in full: load with a missing plugin, replace it,
/// chain order preserved.
#[test]
fn swapping_a_missing_plugin_keeps_its_place_in_the_chain() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(VERB, VERB_ID, "MegaVerb"));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID, "Resonance Compressor"));
    refuse(&mut app, VERB, VERB_ID);

    let rx = app.test_capture_engine();
    // A second EQ, not a second compressor: the compressor already sits
    // AFTER this slot, so a chain of [eq, comp, comp] would look the
    // same whether the replacement landed in the middle or was appended.
    let _ = app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: VERB,
        plugin: catalog_entry(EQ_ID),
    }));

    let slots = app.test_chain_slots(TestChain::Track(TRACK));
    assert_eq!(
        slots.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(),
        vec![EQ_ID, EQ_ID, COMP_ID],
        "the replacement sits in the MIDDLE, where the missing plugin was — not appended"
    );
    assert_ne!(
        slots[1].0, VERB,
        "a different plugin gets a fresh instance id; inheriting one that automation lanes \
         and key routes are keyed by would silently re-target them"
    );
    assert!(!slots[1].2, "and it is not marked missing");

    // The engine is told the same shape: drop the old, add the new,
    // put it where the slot is. The move index is the ENGINE's, which
    // is 1 here because only the EQ is live before it.
    let cmds: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(
        cmds.iter().any(
            |c| matches!(c, AudioCommand::RemovePlugin { instance_id, .. } if *instance_id == VERB)
        ),
        "the outgoing instance is dropped"
    );
    let added = cmds.iter().find_map(|c| match c {
        AudioCommand::AddPlugin {
            clap_plugin_id,
            id_hint,
            ..
        } if clap_plugin_id == EQ_ID => Some(id_hint.expect("app-allocated id")),
        _ => None,
    });
    assert_eq!(added, Some(slots[1].0));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::MovePlugin { instance_id, to_index, .. }
                if Some(*instance_id) == added && *to_index == 1
        )),
        "and moved to the engine index the slot corresponds to; got {cmds:?}"
    );
}

/// A swap discards the outgoing plugin's preserved settings, because
/// they cannot mean anything to the plugin arriving. Doing it eagerly
/// (rather than waiting for the `PluginRemoved` echo) is what stops a
/// save taken in the same frame from writing a blob for a slot that no
/// longer exists.
#[test]
fn swapping_drops_the_outgoing_plugins_preserved_state() {
    let mut app = app();
    chain_of_three(&mut app);
    app.test_seed_plugin_state(VERB, BLOB.to_vec());
    refuse(&mut app, VERB, VERB_ID);
    assert_eq!(app.test_plugin_states_for_save(Vec::new()).len(), 1);

    let _ = app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: VERB,
        plugin: catalog_entry(COMP_ID),
    }));

    assert!(
        app.test_plugin_states_for_save(Vec::new()).is_empty(),
        "MegaVerb's blob went with MegaVerb"
    );
}

/// Replacing works on a plugin that is perfectly healthy too — that is
/// what makes it a chain-editing verb and not only a repair tool.
#[test]
fn a_loaded_plugin_can_be_swapped_without_leaving_its_slot() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID, "Resonance Compressor"));
    accept(&mut app, EQ, EQ_ID);
    accept(&mut app, COMP, COMP_ID);

    let _ = app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: EQ,
        plugin: catalog_entry(COMP_ID),
    }));

    assert_eq!(
        app.test_chain_slots(TestChain::Track(TRACK))
            .iter()
            .map(|s| s.1.as_str())
            .collect::<Vec<_>>(),
        vec![COMP_ID, COMP_ID],
    );
}

/// Bus and master chains are not special-cased: the same message reaches
/// them, because plugin instance ids are unique across all three.
#[test]
fn a_missing_plugin_on_a_bus_is_replaced_the_same_way() {
    let mut app = app();
    app.test_add_bus(BUS, "Drums");
    app.test_push_bus_plugin(BUS, slot(VERB, VERB_ID, "MegaVerb"));
    app.test_push_bus_plugin(BUS, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(VERB),
        clap_plugin_id: VERB_ID.to_owned(),
        clap_file_path: "/plugins/mega-verb.clap".to_owned(),
        reason: "Failed to load plugin: no such file".to_owned(),
    });

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: VERB,
        plugin: catalog_entry(COMP_ID),
    }));

    assert_eq!(
        app.test_chain_slots(TestChain::Bus(BUS))
            .iter()
            .map(|s| s.1.as_str())
            .collect::<Vec<_>>(),
        vec![COMP_ID, EQ_ID],
        "position kept on a bus chain too"
    );
    let cmds: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(
        cmds.iter()
            .any(|c| matches!(c, AudioCommand::AddPluginToBus { bus_id, .. } if *bus_id == BUS)),
        "and it is the BUS add command that is sent, not the track one: {cmds:?}"
    );
}
