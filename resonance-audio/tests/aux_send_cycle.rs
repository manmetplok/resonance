//! Cyclic-route validation for aux sends. `aux_send_would_cycle` is the
//! shared predicate the engine's `SetAuxSend` handler consults before
//! registering a send; these tests pin its behaviour the way
//! `solo_predicate.rs` pins `any_top_level_solo`.

use resonance_audio::{aux_send_would_cycle, AuxSend, SendSource};

/// Build an enabled, post-fader bus→bus send at unity gain.
fn bus_send(id: u64, from: u64, dest: u64) -> AuxSend {
    AuxSend {
        id,
        source: SendSource::Bus(from),
        dest,
        level_db: 0.0,
        pre_fader: false,
        enabled: true,
    }
}

#[test]
fn track_source_never_cycles() {
    // Tracks are pure sources and can never be a destination, so a
    // track-sourced send is always acyclic regardless of the graph.
    let existing = vec![bus_send(1, 10, 20)];
    assert!(!aux_send_would_cycle(
        &existing,
        SendSource::Track(99),
        20,
        None
    ));
}

#[test]
fn bus_self_route_is_a_cycle() {
    let existing: Vec<AuxSend> = vec![];
    assert!(aux_send_would_cycle(
        &existing,
        SendSource::Bus(5),
        5,
        None
    ));
}

#[test]
fn empty_graph_simple_send_is_acyclic() {
    let existing: Vec<AuxSend> = vec![];
    assert!(!aux_send_would_cycle(
        &existing,
        SendSource::Bus(1),
        2,
        None
    ));
}

#[test]
fn direct_back_edge_is_a_cycle() {
    // Bus 2 already sends to bus 1; adding 1 -> 2 closes the loop.
    let existing = vec![bus_send(1, 2, 1)];
    assert!(aux_send_would_cycle(
        &existing,
        SendSource::Bus(1),
        2,
        None
    ));
}

#[test]
fn transitive_back_edge_is_a_cycle() {
    // 2 -> 3 -> 4 already exists; adding 4 -> 2 closes a 3-node loop.
    let existing = vec![bus_send(1, 2, 3), bus_send(2, 3, 4)];
    assert!(aux_send_would_cycle(
        &existing,
        SendSource::Bus(4),
        2,
        None
    ));
}

#[test]
fn parallel_branches_do_not_falsely_trip() {
    // Bus 1 fans out to 2 and 3; adding 2 -> 3 joins two sinks but
    // forms no loop back to 2.
    let existing = vec![bus_send(1, 1, 2), bus_send(2, 1, 3)];
    assert!(!aux_send_would_cycle(
        &existing,
        SendSource::Bus(2),
        3,
        None
    ));
}

#[test]
fn disabled_send_still_counts_for_cycle_detection() {
    // A disabled 2 -> 1 send is still a configured edge: adding 1 -> 2
    // must be rejected so re-enabling can't retroactively form a loop.
    let mut disabled = bus_send(1, 2, 1);
    disabled.enabled = false;
    let existing = vec![disabled];
    assert!(aux_send_would_cycle(
        &existing,
        SendSource::Bus(1),
        2,
        None
    ));
}

#[test]
fn updating_a_send_ignores_its_own_prior_edge() {
    // Send #1 currently routes bus 1 -> 2. Re-routing it to 1 -> 3 must
    // not see its own (now-replaced) 1 -> 2 edge as part of the graph.
    let existing = vec![bus_send(1, 1, 2)];
    // Without ignoring self, re-pointing dest is fine here anyway, so
    // construct a case where the stale edge would matter: send #1 is
    // 2 -> 3, and we re-route it to 3 -> 2. Ignoring self, the only
    // remaining edge set is empty, so no cycle.
    let existing2 = vec![bus_send(1, 2, 3)];
    assert!(!aux_send_would_cycle(
        &existing2,
        SendSource::Bus(3),
        2,
        Some(1)
    ));
    // And the same re-route WITHOUT ignoring self is a cycle (3 -> 2,
    // plus the stale 2 -> 3, loops).
    assert!(aux_send_would_cycle(
        &existing2,
        SendSource::Bus(3),
        2,
        None
    ));
    // Sanity: the original graph with an unrelated ignore is unaffected.
    assert!(!aux_send_would_cycle(
        &existing,
        SendSource::Bus(1),
        3,
        None
    ));
}

#[test]
fn no_panic_on_pre_existing_cycle_in_graph() {
    // Defensive: even if the stored graph somehow already contains a
    // loop (2 -> 3 -> 2), the walk terminates via the visited set.
    let existing = vec![bus_send(1, 2, 3), bus_send(2, 3, 2)];
    // Adding 1 -> 2 reaches the existing loop but never returns to 1.
    assert!(!aux_send_would_cycle(
        &existing,
        SendSource::Bus(1),
        2,
        None
    ));
}
