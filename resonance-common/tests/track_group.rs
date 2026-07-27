use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::{MACRO_LEVEL_UNITY, TrackGroup};

#[test]
fn new_group_has_neutral_defaults() {
    let g = TrackGroup::new(7, "Drums", GroupIdentityColor::Drum);
    assert_eq!(g.id, 7);
    assert_eq!(g.name, "Drums");
    assert_eq!(g.identity_color, GroupIdentityColor::Drum);
    assert!(g.ordered_members.is_empty());
    assert_eq!(g.nesting_parent, None);
    assert!(!g.is_collapsed);
    assert!(!g.macro_mute);
    assert!(!g.macro_solo);
    assert_eq!(g.macro_level, MACRO_LEVEL_UNITY);
}

#[test]
fn new_group_with_custom_values() {
    let g = TrackGroup {
        id: 10,
        name: "Vocals".to_string(),
        identity_color: GroupIdentityColor::Vocal,
        ordered_members: vec![100, 101, 102],
        nesting_parent: Some(1),
        is_collapsed: true,
        macro_mute: true,
        macro_solo: false,
        macro_level: 0.5,
    };
    assert_eq!(g.id, 10);
    assert_eq!(g.name, "Vocals");
    assert_eq!(g.identity_color, GroupIdentityColor::Vocal);
    assert_eq!(g.ordered_members, vec![100, 101, 102]);
    assert_eq!(g.nesting_parent, Some(1));
    assert!(g.is_collapsed);
    assert!(g.macro_mute);
    assert!(!g.macro_solo);
    assert_eq!(g.macro_level, 0.5);
}

#[test]
fn round_trips_through_json() {
    let mut g = TrackGroup::new(3, "Vocals", GroupIdentityColor::Vocal);
    g.ordered_members = vec![10, 11, 12];
    g.nesting_parent = Some(1);
    g.is_collapsed = true;
    g.macro_mute = true;
    g.macro_level = 0.5;

    let json = serde_json::to_string(&g).expect("serialize");
    let back: TrackGroup = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(g, back);
}

#[test]
fn macro_level_unity_constant() {
    assert_eq!(MACRO_LEVEL_UNITY, 1.0);
}

#[test]
fn group_identity_color_default() {
    assert_eq!(GroupIdentityColor::default(), GroupIdentityColor::Drum);
}

#[test]
fn next_cycles_through_full_palette_and_wraps() {
    let mut c = GroupIdentityColor::default();
    let mut seen = Vec::new();
    for _ in 0..GroupIdentityColor::all().len() {
        seen.push(c);
        c = c.next();
    }
    assert_eq!(seen, GroupIdentityColor::all().to_vec());
    // wrapped back to the start
    assert_eq!(c, GroupIdentityColor::default());
}
