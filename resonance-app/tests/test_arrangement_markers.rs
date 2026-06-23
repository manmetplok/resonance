//! Tests for arrangement markers data model and state management.

use resonance_app::project::ProjectFile;
use resonance_app::state::{ArrangementMarker, ArrangementMarkers};

#[test]
fn test_marker_creation() {
    let point = ArrangementMarker::new_point(1, "Intro".to_string(), [255, 0, 0], 0);
    assert_eq!(point.id, 1);
    assert_eq!(point.name, "Intro");
    assert_eq!(point.color, [255, 0, 0]);
    assert_eq!(point.start_sample, 0);
    assert!(point.is_point());
    assert!(!point.is_region());
    assert_eq!(point.effective_end(), 0);

    let region = ArrangementMarker::new_region(2, "Verse".to_string(), [0, 255, 0], 1000, 5000);
    assert_eq!(region.id, 2);
    assert_eq!(region.name, "Verse");
    assert_eq!(region.color, [0, 255, 0]);
    assert_eq!(region.start_sample, 1000);
    assert_eq!(region.end_sample, Some(5000));
    assert!(!region.is_point());
    assert!(region.is_region());
    assert_eq!(region.effective_end(), 5000);
}

#[test]
fn test_markers_collection_basic() {
    let mut markers = ArrangementMarkers::new();
    assert!(markers.is_empty());
    assert_eq!(markers.len(), 0);

    let id1 = markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    assert_eq!(id1, 1);
    assert_eq!(markers.len(), 1);
    assert!(!markers.is_empty());

    let id2 = markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 50));
    assert_eq!(id2, 2);
    assert_eq!(markers.len(), 2);

    // Markers should be sorted by start_sample
    assert_eq!(markers.as_slice()[0].id, 2); // B at 50
    assert_eq!(markers.as_slice()[1].id, 1); // A at 100
}

#[test]
fn test_markers_get_and_remove() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200));

    assert!(markers.contains(1));
    assert!(markers.contains(2));
    assert!(!markers.contains(999));

    assert_eq!(markers.get(1).unwrap().name, "A");
    assert_eq!(markers.get(2).unwrap().name, "B");
    assert!(markers.get(999).is_none());

    let removed = markers.remove(1);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().id, 1);
    assert!(!markers.contains(1));
    assert_eq!(markers.len(), 1);
}

#[test]
fn test_markers_move_start() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200));

    assert!(markers.move_start(1, 300));
    // After moving, should be re-sorted
    assert_eq!(markers.as_slice()[0].id, 2); // B at 200
    assert_eq!(markers.as_slice()[1].id, 1); // A now at 300
    assert_eq!(markers.get(1).unwrap().start_sample, 300);

    assert!(!markers.move_start(999, 500)); // Non-existent marker
}

#[test]
fn test_markers_set_region_end() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));

    assert!(markers.set_region_end(1, Some(500)));
    assert!(markers.get(1).unwrap().is_region());
    assert_eq!(markers.get(1).unwrap().end_sample, Some(500));

    // Convert back to point
    assert!(markers.set_region_end(1, None));
    assert!(markers.get(1).unwrap().is_point());
    assert_eq!(markers.get(1).unwrap().end_sample, None);
}

#[test]
fn test_marker_at() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_region(2, "B".to_string(), [0, 255, 0], 200, 400));
    markers.add(ArrangementMarker::new_point(3, "C".to_string(), [0, 0, 255], 500));

    // At exact positions
    assert!(markers.marker_at(100).is_some());
    assert_eq!(markers.marker_at(100).unwrap().id, 1);

    assert!(markers.marker_at(200).is_some());
    assert_eq!(markers.marker_at(200).unwrap().id, 2);

    assert!(markers.marker_at(500).is_some());
    assert_eq!(markers.marker_at(500).unwrap().id, 3);

    // Within region
    assert!(markers.marker_at(300).is_some());
    assert_eq!(markers.marker_at(300).unwrap().id, 2);

    // Before first
    assert!(markers.marker_at(0).is_none());

    // Between markers
    assert!(markers.marker_at(150).is_none());

    // After last
    assert!(markers.marker_at(600).is_none());
}

#[test]
fn test_next_marker() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200));
    markers.add(ArrangementMarker::new_point(3, "C".to_string(), [0, 0, 255], 300));

    // Next from before first
    assert!(markers.next_marker(0).is_some());
    assert_eq!(markers.next_marker(0).unwrap().id, 1);

    // Next from at first
    assert!(markers.next_marker(100).is_some());
    assert_eq!(markers.next_marker(100).unwrap().id, 2);

    // Next from between
    assert!(markers.next_marker(150).is_some());
    assert_eq!(markers.next_marker(150).unwrap().id, 2);

    // Next from at second
    assert!(markers.next_marker(200).is_some());
    assert_eq!(markers.next_marker(200).unwrap().id, 3);

    // Next from after last - wraps to first
    assert!(markers.next_marker(400).is_some());
    assert_eq!(markers.next_marker(400).unwrap().id, 1);
}

#[test]
fn test_prev_marker() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200));
    markers.add(ArrangementMarker::new_point(3, "C".to_string(), [0, 0, 255], 300));

    // Prev from after last
    assert!(markers.prev_marker(400).is_some());
    assert_eq!(markers.prev_marker(400).unwrap().id, 3);

    // Prev from at last
    assert!(markers.prev_marker(300).is_some());
    assert_eq!(markers.prev_marker(300).unwrap().id, 2);

    // Prev from between
    assert!(markers.prev_marker(250).is_some());
    assert_eq!(markers.prev_marker(250).unwrap().id, 2);

    // Prev from at first
    assert!(markers.prev_marker(100).is_some());
    assert_eq!(markers.prev_marker(100).unwrap().id, 3); // Wraps to last

    // Prev from before first - wraps to last
    assert!(markers.prev_marker(0).is_some());
    assert_eq!(markers.prev_marker(0).unwrap().id, 3);
}

#[test]
fn test_from_vec() {
    let vec = vec![
        ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200),
        ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100),
        ArrangementMarker::new_point(3, "C".to_string(), [0, 0, 255], 300),
    ];

    let markers: ArrangementMarkers = vec.into();
    assert_eq!(markers.len(), 3);
    // Should be sorted by start_sample
    assert_eq!(markers.as_slice()[0].id, 1);
    assert_eq!(markers.as_slice()[1].id, 2);
    assert_eq!(markers.as_slice()[2].id, 3);
}

#[test]
fn test_into_vec() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200));

    let vec: Vec<ArrangementMarker> = markers.into();
    assert_eq!(vec.len(), 2);
}

#[test]
fn test_marker_at_region_end_inclusive() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_region(
        1,
        "Verse".to_string(),
        [0, 255, 0],
        200,
        400,
    ));

    // The region covers its start, interior, and end inclusively.
    assert_eq!(markers.marker_at(200).unwrap().id, 1);
    assert_eq!(markers.marker_at(300).unwrap().id, 1);
    assert_eq!(markers.marker_at(400).unwrap().id, 1);
    // Just outside the span on either side: no cover.
    assert!(markers.marker_at(199).is_none());
    assert!(markers.marker_at(401).is_none());
}

/// Markers survive a serde round-trip through `ProjectFile` — the shape
/// the snapshot/replay undo machinery and project save/load both use.
#[test]
fn test_project_file_round_trip() {
    let mut file = ProjectFile::default();
    file.arrangement_markers = vec![
        ArrangementMarker::new_point(1, "Intro".to_string(), [255, 0, 0], 0),
        ArrangementMarker::new_region(2, "Verse".to_string(), [0, 255, 0], 1000, 5000),
    ];

    let json = serde_json::to_string(&file).unwrap();
    let parsed: ProjectFile = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.arrangement_markers.len(), 2);
    assert_eq!(parsed.arrangement_markers[0].id, 1);
    assert!(parsed.arrangement_markers[0].is_point());
    assert_eq!(parsed.arrangement_markers[1].id, 2);
    assert_eq!(parsed.arrangement_markers[1].name, "Verse");
    assert_eq!(parsed.arrangement_markers[1].end_sample, Some(5000));
}

/// Legacy projects authored before markers existed have no
/// `arrangement_markers` key; `#[serde(default)]` must load them as an
/// empty list rather than failing to deserialize.
#[test]
fn test_legacy_project_loads_without_markers() {
    let json = serde_json::to_string(&ProjectFile::default()).unwrap();
    let stripped = json.replace(",\"arrangement_markers\":[]", "");
    // Sanity: the field really was removed from the serialized form.
    assert!(!stripped.contains("arrangement_markers"));

    let parsed: ProjectFile = serde_json::from_str(&stripped).unwrap();
    assert!(parsed.arrangement_markers.is_empty());
}

#[test]
fn test_allocate_id_unique_and_skips_existing() {
    let mut markers = ArrangementMarkers::new();
    let a = markers.allocate_id();
    let b = markers.allocate_id();
    assert_ne!(a, b);
    assert!(b > a);

    // An id already present in the collection is skipped, never reissued.
    markers.add(ArrangementMarker::new_point(
        b + 1,
        "X".to_string(),
        [0, 0, 0],
        0,
    ));
    let c = markers.allocate_id();
    assert_ne!(c, b + 1, "allocate must skip the existing id");
    assert!(markers.get(c).is_none(), "allocated id is fresh/unused");
}

#[test]
fn test_allocate_id_resumes_after_rebuild_from_vec() {
    // Rebuilding from a persisted Vec restores the counter to max+1, so the
    // first allocation after load can't collide with a loaded marker —
    // mirroring how the track registry restores next_sub_track_id on load.
    let vec = vec![
        ArrangementMarker::new_point(5, "A".to_string(), [0, 0, 0], 0),
        ArrangementMarker::new_point(9, "B".to_string(), [0, 0, 0], 100),
    ];
    let mut markers: ArrangementMarkers = vec.into();
    assert_eq!(markers.allocate_id(), 10, "counter resumes at max id + 1");
}

#[test]
fn test_clear() {
    let mut markers = ArrangementMarkers::new();
    markers.add(ArrangementMarker::new_point(1, "A".to_string(), [255, 0, 0], 100));
    markers.add(ArrangementMarker::new_point(2, "B".to_string(), [0, 255, 0], 200));

    assert_eq!(markers.len(), 2);
    markers.clear();
    assert!(markers.is_empty());
}
