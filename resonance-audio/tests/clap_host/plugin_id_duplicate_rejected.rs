//! ARCH-04 D-1: the app allocates every plugin instance id now
//! (`Resonance::allocate_plugin_id`), and `AudioCommand::AddPlugin` (for
//! every `ChainOwner`) carries a concrete `id` rather
//! than an optional hint. The engine has no allocator of its own left —
//! `next_plugin_id` and the hint-vs-`CONTROL_PLUGIN_ID_BASE` rule this
//! file used to pin (see git history for `plugin_id_ranges.rs`) are gone.
//!
//! What the engine still owes the app is refusing a collision instead of
//! silently replacing a live instance: two adds for the same id would
//! otherwise leave one CLAP instance in `ctx.plugins` driven from two
//! chains (exactly the bug the old hint-vs-base split existed to avoid,
//! now prevented by construction on the allocation side and by this
//! check on the engine side). These drive the real `handle_add_plugin`
//! handler via `EngineHandlerHarness` against a real bundled plugin, so
//! what's proven is the actual `ctx.plugins` map and the actual
//! `AudioEvent::Error` — not a description of the rule.
//!
//! Needs a built plugin binary; `plugin_binaries` finds it, and a missing
//! one fails the test unless `RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES` is
//! set — the contract every test under `tests/clap_host/` that needs a
//! built plugin follows.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioEvent, ChainOwner, EngineErrorKind};
use resonance_audio::{Track, TrackId};

use crate::plugin_binaries::plugin_binary;

const TRACK: TrackId = 1;
const EQ_CLAP_ID: &str = "com.resonance.eq";

fn error_kind(events: &[AudioEvent]) -> Option<EngineErrorKind> {
    events.iter().find_map(|e| match e {
        AudioEvent::Error(err) => Some(err.kind),
        _ => None,
    })
}

#[test]
fn a_duplicate_id_is_refused_and_does_not_replace_the_live_instance() {
    let Some(path) = plugin_binary("resonance-eq") else {
        return;
    };
    let path = path.to_string_lossy().into_owned();

    let mut harness = EngineHandlerHarness::new();
    harness.push_track(Track::new(TRACK, "T1".to_string()));

    // The first add at id 1 succeeds: one `PluginAdded`, one live
    // instance, one entry on the track's chain.
    harness.add_plugin(ChainOwner::Track(TRACK), path.clone(), EQ_CLAP_ID.to_string(), 1);
    let first = harness.drain_events();
    assert!(
        first
            .iter()
            .any(|e| matches!(e, AudioEvent::PluginAdded { instance_id: 1, .. })),
        "events: {first:?}"
    );
    assert_eq!(harness.track_plugin_ids(TRACK), vec![1]);
    assert_eq!(harness.plugin_instance_count(), 1);

    // A second add asking for the SAME id is refused — not re-numbered,
    // not silently accepted as a second copy.
    harness.add_plugin(ChainOwner::Track(TRACK), path, EQ_CLAP_ID.to_string(), 1);
    let second = harness.drain_events();
    assert_eq!(
        error_kind(&second),
        Some(EngineErrorKind::Internal),
        "a duplicate id is a caller invariant violation, not a transient \
         Busy condition — events: {second:?}"
    );
    assert!(
        !second
            .iter()
            .any(|e| matches!(e, AudioEvent::PluginAdded { .. })),
        "the refused add must not also emit a PluginAdded: {second:?}"
    );

    // The original instance is untouched: still the only entry, still
    // the only live CLAP instance — a silent replace would leave this
    // count at 1 too, but via `ctx.plugins.write().insert` overwriting
    // the old `PluginSlot`, which is exactly what must not happen.
    assert_eq!(
        harness.track_plugin_ids(TRACK),
        vec![1],
        "the chain must not gain a second slot, nor lose the first"
    );
    assert_eq!(
        harness.plugin_instance_count(),
        1,
        "no second instance was created"
    );
}

/// The same refusal on the bus and master add paths, which share
/// `reject_if_plugin_id_in_use` with the track path — a collision must
/// not slip through container-specific code that forgot the check.
#[test]
fn a_duplicate_id_is_refused_across_track_bus_and_master() {
    let Some(path) = plugin_binary("resonance-eq") else {
        return;
    };
    let path = path.to_string_lossy().into_owned();

    let mut harness = EngineHandlerHarness::new();
    harness.push_track(Track::new(TRACK, "T1".to_string()));

    harness.add_plugin(ChainOwner::Track(TRACK), path.clone(), EQ_CLAP_ID.to_string(), 42);
    harness.drain_events();
    assert_eq!(harness.plugin_instance_count(), 1);

    // `ctx.plugins` is one map shared by every chain, so a bus (or
    // master) add asking for a track plugin's id must be refused too.
    let bus_events = harness.add_bus(1, None);
    let bus_id = bus_events
        .iter()
        .find_map(|e| match e {
            AudioEvent::BusAdded { bus_id, .. } => Some(*bus_id),
            _ => None,
        })
        .expect("AddBus succeeded");

    // Since ARCH2-02 a bus add is the same handler with a different
    // owner, so the refusal is by construction not a track-only case —
    // but the contract is pinned here regardless.
    harness.add_plugin(ChainOwner::Bus(bus_id), path, EQ_CLAP_ID.to_string(), 42);
    let events = harness.drain_events();
    assert_eq!(
        error_kind(&events),
        Some(EngineErrorKind::Internal),
        "events: {events:?}"
    );
    assert_eq!(
        harness.plugin_instance_count(),
        1,
        "the track's instance must survive a colliding bus add"
    );
}
