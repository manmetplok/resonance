//! The engine's plugin instance-id allocator must leave the app's
//! control range alone (ba doc #273, todo #1234).
//!
//! The control API allocates plugin instance ids itself, from
//! `CONTROL_PLUGIN_ID_BASE` upwards, so `track.add_effect` can report the
//! new plugin's id synchronously instead of waiting for the engine's
//! `PluginAdded` echo. It passes the id to the engine as an `id_hint`.
//!
//! That split is only real if the engine does NOT advance its own
//! counter past a control-range hint. The original code bumped for every
//! hint:
//!
//! ```ignore
//! if id_hint.is_some() {
//!     state.next_plugin_id = state.next_plugin_id.max(instance_id + 1);
//! }
//! ```
//!
//! — identically in `engine/plugins.rs`, `engine/busses.rs` and
//! `engine/master.rs`, all over the same `HandlerState::next_plugin_id`.
//! So the very first control add dragged the engine's counter into the
//! control range, and the next engine-allocated add handed out an id the
//! app also believed was free. `master.add_effect` is still
//! engine-allocated, so the collision was reachable over MCP alone.
//!
//! These tests drive the allocation rule the three add paths now share.

use resonance_audio::test_support::allocate_plugin_instance_id;
use resonance_audio::types::{PluginInstanceId, CONTROL_PLUGIN_ID_BASE};

/// The engine's counter as it starts on the engine thread
/// (`HandlerState::next_plugin_id`).
const ENGINE_START: PluginInstanceId = 1;

/// The four-step sequence from the review, at the level where the bug
/// lived: the engine's shared `next_plugin_id`.
///
///   1. `track.add_effect` — app allocates `CONTROL_PLUGIN_ID_BASE` and
///      passes it as a hint.
///   2. `master.add_effect` — engine-allocated, no hint. Its
///      `MasterPluginAdded` echo is NOT applied (dlopen +
///      `create_instance` + `query_params` take tens to hundreds of ms on
///      a first load), so the app's mirror stays empty and its in-use
///      scan cannot see this id.
///   3. `track.add_effect` again — the app, having seen no echo, offers
///      its next control id.
///   4. That id must not be the one the engine just handed to master.
#[test]
fn an_engine_allocated_add_between_two_control_adds_cannot_collide() {
    let mut next = ENGINE_START;

    // 1. Control add: honoured as given.
    let first_control = CONTROL_PLUGIN_ID_BASE;
    let got = allocate_plugin_instance_id(&mut next, Some(first_control));
    assert_eq!(got, first_control, "a hint is always honoured");

    // 2. Engine-allocated master add, echo still in flight.
    let master = allocate_plugin_instance_id(&mut next, None);
    assert!(
        master < CONTROL_PLUGIN_ID_BASE,
        "the engine's own allocator must stay below the control range, \
         got {master}"
    );

    // 3. The app's next control id.
    let second_control = CONTROL_PLUGIN_ID_BASE + 1;
    let got = allocate_plugin_instance_id(&mut next, Some(second_control));
    assert_eq!(got, second_control);

    // 4. The master plugin's live CLAP instance is not replaced: the
    //    engine does `ctx.plugins.write().insert(instance_id, ..)`, so
    //    equal ids would overwrite it and leave one instance processed
    //    from two chains.
    assert_ne!(
        got, master,
        "the second control add reused the id the engine gave the master \
         plugin; inserting it would replace a live CLAP instance"
    );
}

/// The same, run long enough that a per-add bump would certainly have
/// walked the engine counter into the control range.
#[test]
fn many_control_adds_never_move_the_engine_allocator() {
    let mut next = ENGINE_START;
    for i in 0..64 {
        allocate_plugin_instance_id(&mut next, Some(CONTROL_PLUGIN_ID_BASE + i));
    }
    assert_eq!(
        next, ENGINE_START,
        "control-range hints must leave the engine's counter untouched"
    );
    assert_eq!(
        allocate_plugin_instance_id(&mut next, None),
        ENGINE_START,
        "so the next engine-allocated plugin still gets a low id"
    );
}

/// The project-load replay path must keep working: those hints are
/// engine-allocated ids from a previous session, all below the base, and
/// the counter has to move past them or a later add reuses one.
#[test]
fn replay_hints_below_the_base_still_advance_the_allocator() {
    let mut next = ENGINE_START;
    for id in [3, 1, 7, 4] {
        assert_eq!(allocate_plugin_instance_id(&mut next, Some(id)), id);
    }
    assert_eq!(
        allocate_plugin_instance_id(&mut next, None),
        8,
        "the counter advanced past the highest replayed id, not merely \
         past the last one"
    );
}

/// A hint exactly on the boundary belongs to the app; one just below it
/// does not.
#[test]
fn the_boundary_is_inclusive_at_the_base() {
    let mut next = ENGINE_START;
    allocate_plugin_instance_id(&mut next, Some(CONTROL_PLUGIN_ID_BASE));
    assert_eq!(next, ENGINE_START, "the base itself is the app's");

    let mut next = ENGINE_START;
    allocate_plugin_instance_id(&mut next, Some(CONTROL_PLUGIN_ID_BASE - 1));
    assert_eq!(
        next,
        CONTROL_PLUGIN_ID_BASE,
        "one below the base is still an engine id and must be skipped"
    );
}

/// Unhinted allocation is unchanged: monotonic from the start value.
#[test]
fn unhinted_allocation_counts_up() {
    let mut next = ENGINE_START;
    let ids: Vec<_> = (0..4)
        .map(|_| allocate_plugin_instance_id(&mut next, None))
        .collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
    assert_eq!(next, 5);
}
