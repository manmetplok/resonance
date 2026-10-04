//! `AudioEvent::PluginMoved` mirroring onto `TrackState.plugins`
//! (ba todo #1224, doc #273).
//!
//! The engine owns chain order; the app follows. This matters beyond the
//! mixer display: `TrackState.plugins` is the order project serialization
//! writes, so a reorder only survives save/load if the event lands here.
//!
//! Driven through `test_apply_engine_event` (the real dispatch path) and the
//! read-only `test_registry` accessor — no private fields.

use resonance_app::state::{PluginSlotState, TrackState};
use resonance_app::Resonance;
use resonance_audio::types::{ChainOwner, AudioEvent};

fn slot(instance_id: u64) -> PluginSlotState {
    PluginSlotState::new(
        instance_id,
        format!("Plugin {instance_id}"),
        format!("com.example.p{instance_id}"),
        format!("/plugins/p{instance_id}.clap"),
        Vec::new(),
        false,
    )
}

/// Seed one track carrying `ids` as its insert chain.
fn app_with_chain(ids: &[u64]) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    let mut track = TrackState::new_audio(1, 0);
    track.plugins = ids.iter().copied().map(slot).collect();
    app.test_push_track(track);
    app
}

fn chain(app: &Resonance) -> Vec<u64> {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == 1)
        .expect("seeded track")
        .plugins
        .iter()
        .map(|p| p.instance_id)
        .collect()
}

fn moved(instance_id: u64, to_index: usize) -> AudioEvent {
    AudioEvent::PluginMoved {
        owner: ChainOwner::Track(1),
        instance_id,
        to_index,
    }
}

#[test]
fn plugin_moved_reorders_the_track_chain() {
    let mut app = app_with_chain(&[10, 20, 30]);
    app.test_apply_engine_event(moved(30, 0));
    assert_eq!(chain(&app), vec![30, 10, 20]);
}

#[test]
fn plugin_moved_shifts_only_the_plugins_in_between() {
    let mut app = app_with_chain(&[1, 2, 3, 4, 5]);
    app.test_apply_engine_event(moved(4, 1));
    assert_eq!(chain(&app), vec![1, 4, 2, 3, 5]);
}

/// The engine already clamps and reports the slot it used, but the mirror
/// clamps too rather than panicking in `Vec::insert` if the two ever drift.
#[test]
fn out_of_range_index_clamps_instead_of_panicking() {
    let mut app = app_with_chain(&[10, 20, 30]);
    app.test_apply_engine_event(moved(10, 99));
    assert_eq!(chain(&app), vec![20, 30, 10]);
}

#[test]
fn no_op_move_leaves_the_chain_untouched() {
    let mut app = app_with_chain(&[10, 20, 30]);
    app.test_apply_engine_event(moved(20, 1));
    assert_eq!(chain(&app), vec![10, 20, 30]);
}

#[test]
fn unknown_instance_or_track_is_ignored() {
    let mut app = app_with_chain(&[10, 20, 30]);
    app.test_apply_engine_event(moved(99, 0));
    assert_eq!(chain(&app), vec![10, 20, 30]);

    app.test_apply_engine_event(AudioEvent::PluginMoved {
        owner: ChainOwner::Track(42),
        instance_id: 10,
        to_index: 2,
    });
    assert_eq!(chain(&app), vec![10, 20, 30]);
}
