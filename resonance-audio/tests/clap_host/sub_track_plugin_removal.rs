//! FU-A13g: a live `RemoveTrack` of a parent multi-output instrument
//! drops its sub-tracks (`docs/design/A-13-reconcile.md` §14, "Found, not
//! fixed" — sub-track plugin chains leak on a live parent delete), but
//! before this fix it left every plugin instance living on those
//! sub-tracks in `ctx.plugins`: never removed from the map, never
//! dropped, never deactivated. CPU and memory for a deleted instrument's
//! sub-track FX stayed live for the rest of the session.
//!
//! The restore path already got this right (`removals::entity_removals`
//! takes sub-tracks before parents, each by its own `RemoveTrack`); this
//! guards the single-command live-delete path the app actually uses
//! (`ConfirmRemoveTrack` sends one `RemoveTrack` for the parent and lets
//! the engine drop every sub-track with it).
//!
//! Echo contract (confirmed against the app side, `resonance-app/src/
//! update/track.rs::ConfirmRemoveTrack` and `resonance-app/src/
//! update/project_io/reconcile/removals.rs`): a track's plugin chain,
//! parent or sub-track, is taken with it silently — no `PluginRemoved`
//! echo, only `TrackRemoved`. Both call sites only ever call
//! `RestoreEchoes::expect_track_removed` / `expect_plugin_removed` for a
//! plugin removed on its OWN command outside a track removal; a track
//! delete never emits `AudioCommand::RemovePlugin` for its own chain
//! (`removals.rs`: "a plugin on a removed track sends nothing — the
//! track takes it"). So the fix must drop the sub-tracks' instances
//! without also emitting a `PluginRemoved` for them, or the app would
//! wait on an echo `RestoreEchoes` never learns to expect.
//!
//! Needs a built plugin binary; `plugin_binaries` finds it, and a missing
//! one fails the test unless `RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES` is
//! set — the contract every test under `tests/clap_host/` that needs a
//! built plugin follows.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::AudioEvent;
use resonance_audio::{Track, TrackId};

use crate::plugin_binaries::plugin_binary;

const PARENT: TrackId = 1;
const SUB: TrackId = 2;
const EQ_CLAP_ID: &str = "com.resonance.eq";

#[test]
fn removing_a_parent_track_drops_every_sub_track_plugin_instance() {
    let Some(path) = plugin_binary("resonance-eq") else {
        return;
    };
    let path = path.to_string_lossy().into_owned();

    let mut harness = EngineHandlerHarness::new();
    harness.push_track(Track::new(PARENT, "Parent".to_string()));
    harness.push_track(Track::new_sub_track(SUB, "Parent (2)".to_string(), PARENT, 1));

    // One instance on the parent, one on its sub-track.
    harness.add_plugin(PARENT, path.clone(), EQ_CLAP_ID.to_string(), 10);
    harness.add_plugin(SUB, path, EQ_CLAP_ID.to_string(), 20);
    harness.drain_events();
    assert_eq!(
        harness.plugin_instance_count(),
        2,
        "both instances should be live before the delete"
    );

    // A single `RemoveTrack` on the parent, the shape the live GUI/
    // control delete sends (`ConfirmRemoveTrack`) — the engine is
    // responsible for taking the sub-track and its chain with it.
    harness.remove_track(PARENT);
    let events = harness.drain_events();

    // Every plugin instance on the parent AND its sub-track must be gone
    // from the map — dropped via the same "extract under the write lock,
    // drop outside it" path the parent's own plugins already used, not
    // leaked, not deactivated on the audio thread.
    assert_eq!(
        harness.plugin_instance_count(),
        0,
        "the sub-track's plugin instance leaked past RemoveTrack: {events:?}"
    );

    // Echo contract: `TrackRemoved` for the parent and for the sub-track,
    // and nothing else naming either plugin instance — a track removal
    // takes its chain silently (no per-plugin `PluginRemoved`), matching
    // what `RestoreEchoes` / `ConfirmRemoveTrack` already expect for the
    // parent's own plugins.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::TrackRemoved { track_id } if *track_id == PARENT)),
        "events: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::TrackRemoved { track_id } if *track_id == SUB)),
        "events: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, AudioEvent::PluginRemoved { .. })),
        "a track removal must not also emit a per-plugin PluginRemoved — the app never \
         arms an echo expectation for one: {events:?}"
    );
}
