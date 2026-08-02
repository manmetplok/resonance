//! Reordering a BUS's insert chain — `Bus::move_plugin` behind
//! `AudioCommand::MovePluginInBus` (ba doc #273, todo #1237).
//!
//! The bus twin of `track_plugin_move.rs`, and deliberately a different
//! shape underneath: a bus chain is a plain `Vec<PluginInstanceId>` the
//! engine owns behind its busses write lock, not the lock-free `ArcSwap`
//! snapshot a track chain publishes. So this is a straight `Vec` edit —
//! there is no tearing property to test, only the reorder itself, the
//! clamp, and the "not on this chain" answer.

use resonance_audio::__test_support::affects_latency;
use resonance_audio::types::{AudioCommand, Bus};

fn bus_with(ids: &[u64]) -> Bus {
    let mut bus = Bus::new(1, "Drum Bus".to_string());
    bus.plugin_ids = ids.to_vec();
    bus
}

#[test]
fn moves_the_last_plugin_to_the_front() {
    let mut bus = bus_with(&[10, 20, 30]);
    assert_eq!(bus.move_plugin(30, 0), Some(0));
    assert_eq!(bus.plugin_ids, &[30, 10, 20]);
}

#[test]
fn moves_the_first_plugin_to_the_end() {
    let mut bus = bus_with(&[10, 20, 30]);
    assert_eq!(bus.move_plugin(10, 2), Some(2));
    assert_eq!(bus.plugin_ids, &[20, 30, 10]);
}

/// Everything between the old and new slot shifts by one; nothing else
/// moves and nothing is duplicated or dropped.
#[test]
fn shifts_only_the_plugins_between_the_two_slots() {
    let mut bus = bus_with(&[1, 2, 3, 4, 5]);
    assert_eq!(bus.move_plugin(4, 1), Some(1));
    assert_eq!(bus.plugin_ids, &[1, 4, 2, 3, 5]);

    let mut bus = bus_with(&[1, 2, 3, 4, 5]);
    assert_eq!(bus.move_plugin(2, 3), Some(3));
    assert_eq!(bus.plugin_ids, &[1, 3, 4, 2, 5]);
}

/// An out-of-range `to_index` clamps to the last slot rather than
/// panicking — `Vec::insert` past `len` would.
#[test]
fn out_of_range_index_clamps_to_the_last_slot() {
    let mut bus = bus_with(&[10, 20, 30]);
    assert_eq!(bus.move_plugin(10, 99), Some(2));
    assert_eq!(bus.plugin_ids, &[20, 30, 10]);
}

#[test]
fn a_no_op_move_reports_the_slot_and_changes_nothing() {
    let mut bus = bus_with(&[10, 20, 30]);
    assert_eq!(bus.move_plugin(20, 1), Some(1));
    assert_eq!(bus.plugin_ids, &[10, 20, 30]);
}

#[test]
fn an_instance_not_on_the_chain_is_none_and_leaves_it_untouched() {
    let mut bus = bus_with(&[10, 20, 30]);
    assert_eq!(bus.move_plugin(99, 0), None);
    assert_eq!(bus.plugin_ids, &[10, 20, 30]);
}

#[test]
fn an_empty_chain_cannot_wrap() {
    let mut bus = bus_with(&[]);
    assert_eq!(bus.move_plugin(10, 0), None);
    assert!(bus.plugin_ids.is_empty());
}

#[test]
fn a_single_plugin_chain_moves_nowhere() {
    let mut bus = bus_with(&[10]);
    assert_eq!(bus.move_plugin(10, 5), Some(0));
    assert_eq!(bus.plugin_ids, &[10]);
}

/// The comp table is rebuilt per chain, so a bus reorder must be treated
/// as latency-affecting for the same reason the track one is.
#[test]
fn the_reorder_command_is_latency_affecting() {
    assert!(affects_latency(&AudioCommand::MovePluginInBus {
        bus_id: 1,
        instance_id: 10,
        to_index: 0,
    }));
}
