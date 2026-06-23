//! Aux sends: an extra tap from a track or bus into a return bus,
//! independent of the source's main output routing. A single track or
//! bus may own several sends (e.g. parallel reverb + delay returns).
//!
//! Aux sends are plain configuration data owned by the engine control
//! thread — unlike [`Bus`](super::Bus), they are never read from the
//! audio callback, so they need no atomic fields.

use super::{BusId, SendId, TrackId};

/// What feeds an aux send: a track's signal or a bus's signal. Tracks
/// are pure sources (they can never be a send destination); busses can
/// be both a source and a destination, which is the only way a send
/// graph can form a feedback cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SendSource {
    Track(TrackId),
    Bus(BusId),
}

/// A single aux send: a tap from `source` into the return bus `dest`.
///
/// * `level_db` — send gain in decibels applied to the tapped signal.
/// * `pre_fader` — when true the tap is taken before the source's
///   volume fader (so the send level is independent of the channel
///   fader); when false it is post-fader.
/// * `enabled` — a disabled send keeps its routing/level configured but
///   contributes no signal. Disabled sends still participate in
///   cyclic-route validation, since re-enabling them must not be able
///   to close a feedback loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuxSend {
    pub id: SendId,
    pub source: SendSource,
    pub dest: BusId,
    pub level_db: f32,
    pub pre_fader: bool,
    pub enabled: bool,
}

/// Would registering a send from `source` into `dest` create a feedback
/// loop, given the sends already in `existing`? `ignore` skips one
/// existing send id — the one being updated in place — so an upsert
/// doesn't count its own prior edge against itself.
///
/// Only bus→bus sends can form a cycle: a track is a pure source and can
/// never be a destination, so a track-sourced send is always acyclic. A
/// bus's main output always lands on the master bus (a terminal, never
/// another bus), so the only routing edges among busses are the aux
/// sends themselves. We walk the existing send graph forward from
/// `dest`; if it can already reach the source bus, the new
/// `source -> dest` edge would close a loop. Disabled sends are included
/// — re-enabling one must not be able to retroactively form a cycle.
///
/// Shared by the engine's `SetAuxSend` handler and the unit tests, the
/// same way [`any_top_level_solo`](super::any_top_level_solo) is the one
/// solo predicate used by both the mixer and its tests.
pub fn aux_send_would_cycle<'a, I>(
    existing: I,
    source: SendSource,
    dest: BusId,
    ignore: Option<SendId>,
) -> bool
where
    I: IntoIterator<Item = &'a AuxSend>,
{
    let src_bus = match source {
        SendSource::Bus(b) => b,
        SendSource::Track(_) => return false,
    };
    // A bus routed straight back to itself is the degenerate one-node loop.
    if src_bus == dest {
        return true;
    }
    // Materialise the edge list once so we can re-scan it during the walk.
    let edges: Vec<&AuxSend> = existing
        .into_iter()
        .filter(|s| ignore != Some(s.id))
        .collect();
    let mut stack = vec![dest];
    let mut visited = std::collections::HashSet::new();
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur) {
            continue;
        }
        for send in &edges {
            if let SendSource::Bus(from) = send.source {
                if from == cur {
                    if send.dest == src_bus {
                        return true;
                    }
                    stack.push(send.dest);
                }
            }
        }
    }
    false
}
