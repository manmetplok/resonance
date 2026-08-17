//! Tests for low-level helpers in `update::project_io::replay`.
//!
//! `migrate_auto_name` and `sort_plugins_by_saved_order` are pure
//! functions extracted from the replay path; they are exercised here
//! rather than in an inline `#[cfg(test)]` module (see the repo-wide
//! "no inline tests" convention).

use resonance_app::state::PluginSlotState;
use resonance_app::update::project_io::{migrate_auto_name, sort_plugins_by_saved_order};

/// Build a minimal `PluginSlotState` carrying only the `instance_id` we
/// care about for the ordering assertions.
fn slot(id: u64) -> PluginSlotState {
    PluginSlotState::new(
        id,
        format!("plugin_{id}"),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    )
}

fn ids(plugins: &[PluginSlotState]) -> Vec<u64> {
    plugins.iter().map(|p| p.instance_id).collect()
}

// ---------------------------------------------------------------------------
// migrate_auto_name
// ---------------------------------------------------------------------------

#[test]
fn migrates_engine_id_track_name() {
    assert_eq!(migrate_auto_name("Track 1000000006", false, 3), "Track 4");
    assert_eq!(
        migrate_auto_name("Instrument 1000000007", true, 5),
        "Instrument 6"
    );
}

#[test]
fn leaves_short_numbered_names_alone() {
    assert_eq!(migrate_auto_name("Track 12", false, 7), "Track 12");
    assert_eq!(migrate_auto_name("Track 99", false, 7), "Track 99");
}

#[test]
fn leaves_user_names_alone() {
    assert_eq!(migrate_auto_name("Bass", false, 1), "Bass");
    assert_eq!(migrate_auto_name("Lead synth", true, 2), "Lead synth");
    assert_eq!(
        migrate_auto_name("Track 1000000006 (vocals)", false, 1),
        "Track 1000000006 (vocals)"
    );
}

// ---------------------------------------------------------------------------
// sort_plugins_by_saved_order
// ---------------------------------------------------------------------------

#[test]
fn restores_saved_order_when_chain_is_scrambled() {
    let mut plugins = vec![slot(30), slot(10), slot(20)];
    sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
    assert_eq!(ids(&plugins), vec![10, 20, 30]);
}

#[test]
fn already_sorted_chain_is_unchanged() {
    let mut plugins = vec![slot(10), slot(20), slot(30)];
    sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
    assert_eq!(ids(&plugins), vec![10, 20, 30]);
}

#[test]
fn appended_plugins_not_in_saved_go_to_end_in_arrival_order() {
    // PluginAdded events for ids 40 and 50 raced ahead of replay
    // and were appended to the chain; saved order is [10, 20, 30].
    let mut plugins = vec![slot(40), slot(20), slot(10), slot(50), slot(30)];
    sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
    assert_eq!(ids(&plugins), vec![10, 20, 30, 40, 50]);
}

#[test]
fn saved_ids_missing_from_chain_are_tolerated() {
    // Plugin 20 failed to load — its placeholder was filtered out.
    // The remaining [30, 10] should still resort to [10, 30].
    let mut plugins = vec![slot(30), slot(10)];
    sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
    assert_eq!(ids(&plugins), vec![10, 30]);
}

#[test]
fn empty_saved_order_is_a_noop() {
    let mut plugins = vec![slot(30), slot(10), slot(20)];
    sort_plugins_by_saved_order(&mut plugins, &[]);
    assert_eq!(ids(&plugins), vec![30, 10, 20]);
}

#[test]
fn single_plugin_chain_is_a_noop() {
    let mut plugins = vec![slot(42)];
    sort_plugins_by_saved_order(&mut plugins, &[1, 2, 3]);
    assert_eq!(ids(&plugins), vec![42]);
}
