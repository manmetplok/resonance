//! Persistence coverage for the `track_groups` field on `ProjectFile`
//! (ba todo #679, epic #36).
//!
//! Verifies the track group registry survives a save/load round-trip
//! through the project file and that projects authored by older builds
//! (no `track_groups` key) still load via the `#[serde(default)]`
//! fallback.

use resonance_app::project::ProjectFile;
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

fn sample_groups() -> Vec<TrackGroup> {
    let mut drums = TrackGroup::new(1, "Drums", GroupIdentityColor::Drum);
    drums.ordered_members = vec![10, 11, 12];
    drums.is_collapsed = true;
    drums.macro_mute = true;
    drums.macro_level = 0.5;

    let mut vocals = TrackGroup::new(2, "Vocals", GroupIdentityColor::Vocal);
    vocals.ordered_members = vec![20];
    vocals.nesting_parent = Some(1);

    vec![drums, vocals]
}

#[test]
fn track_groups_survive_serde_round_trip() {
    let file = ProjectFile {
        track_groups: sample_groups(),
        ..ProjectFile::default()
    };

    let json = serde_json::to_string(&file).expect("serialize");
    let restored: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(restored.track_groups, file.track_groups);
}

#[test]
fn legacy_project_without_track_groups_loads_empty() {
    // A project document authored before the field existed: every
    // required key is present but `track_groups` is omitted.
    // `#[serde(default)]` must supply an empty Vec rather than failing
    // the load.
    let legacy = serde_json::json!({
        "version": ProjectFile::default().version,
        "sample_rate": 44100,
        "bpm": 120.0,
        "time_sig_num": 4,
        "time_sig_den": 4,
        "metronome_enabled": false,
        "master_volume": 0.0,
        "loop_enabled": false,
        "loop_in": 0,
        "loop_out": 0,
        "tracks": [],
        "clips": [],
    });

    let restored: ProjectFile =
        serde_json::from_value(legacy).expect("legacy project loads");
    assert!(restored.track_groups.is_empty());
}
