use resonance_common::group_identity::{GroupColor, GroupIdentityColor};

#[test]
fn group_color_unpacks_components() {
    let c = GroupColor::new(0xFF, 0x80, 0x40);
    assert_eq!(c.r(), 0xFF);
    assert_eq!(c.g(), 0x80);
    assert_eq!(c.b(), 0x40);
    assert_eq!(c.0, 0xFF_8040);
}

#[test]
fn group_color_displays_as_hex() {
    assert_eq!(GroupColor::new(0xc9, 0x8f, 0x5f).to_string(), "#c98f5f");
    assert_eq!(GroupColor::new(0x00, 0x00, 0x00).to_string(), "#000000");
}

#[test]
fn palette_matches_prototype() {
    assert_eq!(GroupIdentityColor::Drum.color(), GroupColor::new(0xc9, 0x8f, 0x5f));
    assert_eq!(GroupIdentityColor::Vocal.color(), GroupColor::new(0xc9, 0x7b, 0x9c));
    assert_eq!(GroupIdentityColor::Keys.color(), GroupColor::new(0x6f, 0xb6, 0xb0));
    assert_eq!(GroupIdentityColor::Guitar.color(), GroupColor::new(0x7d, 0x86, 0xc9));
}

#[test]
fn palette_colors_are_distinct() {
    let colors: std::collections::HashSet<GroupColor> =
        GroupIdentityColor::all().iter().map(|c| c.color()).collect();
    assert_eq!(colors.len(), GroupIdentityColor::all().len());
}

#[test]
fn all_lists_every_variant_in_cycle_order() {
    assert_eq!(
        GroupIdentityColor::all(),
        &[
            GroupIdentityColor::Drum,
            GroupIdentityColor::Vocal,
            GroupIdentityColor::Keys,
            GroupIdentityColor::Guitar,
        ]
    );
}

#[test]
fn next_cycles_through_all_and_wraps() {
    let mut c = GroupIdentityColor::default();
    let mut seen = vec![c];
    for _ in 0..GroupIdentityColor::all().len() - 1 {
        c = c.next();
        seen.push(c);
    }
    assert_eq!(seen, GroupIdentityColor::all());
    // wraps back to the start
    assert_eq!(c.next(), GroupIdentityColor::default());
}

#[test]
fn default_is_drum() {
    assert_eq!(GroupIdentityColor::default(), GroupIdentityColor::Drum);
}

#[test]
fn display_names_match_variants() {
    assert_eq!(GroupIdentityColor::Drum.to_string(), "Drum");
    assert_eq!(GroupIdentityColor::Vocal.to_string(), "Vocal");
    assert_eq!(GroupIdentityColor::Keys.to_string(), "Keys");
    assert_eq!(GroupIdentityColor::Guitar.to_string(), "Guitar");
}

#[test]
fn into_group_color_uses_base_color() {
    let c: GroupColor = GroupIdentityColor::Keys.into();
    assert_eq!(c, GroupIdentityColor::Keys.color());
}

#[test]
fn serde_roundtrips_every_variant() {
    for &variant in GroupIdentityColor::all() {
        let json = serde_json::to_string(&variant).unwrap();
        let back: GroupIdentityColor = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, back);
    }
}

#[test]
fn group_identity_serializes_by_variant_name() {
    assert_eq!(
        serde_json::to_string(&GroupIdentityColor::Vocal).unwrap(),
        "\"Vocal\""
    );
}

#[test]
fn group_color_roundtrips() {
    let c = GroupColor::new(0x12, 0x34, 0x56);
    let json = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<GroupColor>(&json).unwrap(), c);
}
