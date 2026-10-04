//! Reordering the MASTER insert chain — `MasterBus::move_plugin` behind
//! `AudioCommand::MovePlugin { owner: ChainOwner::Master, .. }`.
//!
//! The master twin of `bus_plugin_move.rs`, over the same shape: a plain
//! `Vec<PluginInstanceId>` the engine edits on a copy and publishes in a
//! new render graph (ARCH-02 A2-5). So this is a straight `Vec` edit —
//! the reorder itself, the clamp, and the "not on this chain" answer.
//!
//! Order matters more here than anywhere else: a limiter holding a
//! ceiling has to be last, because anything after it can push the sum
//! back over the ceiling it exists to hold.

use resonance_audio::test_support::affects_latency;
use resonance_audio::types::{AudioCommand, ChainOwner, MasterBus};

fn master_with(ids: &[u64]) -> MasterBus {
    let mut master = MasterBus::new();
    master.plugin_ids = ids.to_vec();
    master
}

#[test]
fn moves_the_last_plugin_to_the_front() {
    let mut master = master_with(&[10, 20, 30]);
    assert_eq!(master.move_plugin(30, 0), Some(0));
    assert_eq!(master.plugin_ids, &[30, 10, 20]);
}

/// The move that actually matters: getting the limiter to the end.
#[test]
fn moves_the_first_plugin_to_the_end() {
    let mut master = master_with(&[10, 20, 30]);
    assert_eq!(master.move_plugin(10, 2), Some(2));
    assert_eq!(master.plugin_ids, &[20, 30, 10]);
}

/// Everything between the old and new slot shifts by one; nothing else
/// moves and nothing is duplicated or dropped.
#[test]
fn shifts_only_the_plugins_between_the_two_slots() {
    let mut master = master_with(&[1, 2, 3, 4, 5]);
    assert_eq!(master.move_plugin(4, 1), Some(1));
    assert_eq!(master.plugin_ids, &[1, 4, 2, 3, 5]);

    let mut master = master_with(&[1, 2, 3, 4, 5]);
    assert_eq!(master.move_plugin(2, 3), Some(3));
    assert_eq!(master.plugin_ids, &[1, 3, 4, 2, 5]);
}

/// An out-of-range `to_index` clamps to the last slot rather than
/// panicking — `Vec::insert` past `len` would.
#[test]
fn out_of_range_index_clamps_to_the_last_slot() {
    let mut master = master_with(&[10, 20, 30]);
    assert_eq!(master.move_plugin(10, 99), Some(2));
    assert_eq!(master.plugin_ids, &[20, 30, 10]);
}

#[test]
fn a_no_op_move_reports_the_slot_and_changes_nothing() {
    let mut master = master_with(&[10, 20, 30]);
    assert_eq!(master.move_plugin(20, 1), Some(1));
    assert_eq!(master.plugin_ids, &[10, 20, 30]);
}

#[test]
fn an_instance_not_on_the_chain_is_none_and_leaves_it_untouched() {
    let mut master = master_with(&[10, 20, 30]);
    assert_eq!(master.move_plugin(99, 0), None);
    assert_eq!(master.plugin_ids, &[10, 20, 30]);
}

#[test]
fn an_empty_chain_cannot_wrap() {
    let mut master = master_with(&[]);
    assert_eq!(master.move_plugin(10, 0), None);
    assert!(master.plugin_ids.is_empty());
}

#[test]
fn a_single_plugin_chain_moves_nowhere() {
    let mut master = master_with(&[10]);
    assert_eq!(master.move_plugin(10, 5), Some(0));
    assert_eq!(master.plugin_ids, &[10]);
}

/// Master latency is published for the reference A/B monitor to align
/// against, so every master-chain edit — reorder included — republishes
/// it rather than risking a stale figure.
#[test]
fn the_reorder_command_is_latency_affecting() {
    assert!(affects_latency(&AudioCommand::MovePlugin {
        owner: ChainOwner::Master,
        instance_id: 10,
        to_index: 0,
    }));
}
