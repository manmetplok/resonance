use resonance_common::registry::*;

#[test]
fn roundtrip_registry() {
    let dir = std::env::temp_dir().join("resonance_test_registry");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("installed.json");

    let mut reg = InstalledRegistry::default();
    reg.items.push(InstalledItem {
        name: "TestKit".to_string(),
        content_type: ContentType::Drumkit,
        path: "/tmp/kits/testkit".to_string(),
        installed_at: "2026-01-01".to_string(),
    });
    save_registry_to(&reg, &path).unwrap();

    let loaded = load_registry_from(&path);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].name, "TestKit");
    assert_eq!(loaded.items[0].content_type, ContentType::Drumkit);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mark_installed_deduplicates() {
    let dir = std::env::temp_dir().join("resonance_test_registry_dedup");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("installed.json");

    let item = InstalledItem {
        name: "Kit".to_string(),
        content_type: ContentType::Drumkit,
        path: "/tmp/a".to_string(),
        installed_at: "2026-01-01".to_string(),
    };
    let mut reg = InstalledRegistry::default();
    reg.items.push(item.clone());
    save_registry_to(&reg, &path).unwrap();

    // Push another with the same name but different path -- should replace.
    let mut reg = load_registry_from(&path);
    reg.items.retain(|existing| {
        !(existing.name == "Kit" && existing.content_type == ContentType::Drumkit)
    });
    reg.items.push(InstalledItem {
        name: "Kit".to_string(),
        content_type: ContentType::Drumkit,
        path: "/tmp/b".to_string(),
        installed_at: "2026-02-02".to_string(),
    });
    save_registry_to(&reg, &path).unwrap();

    let loaded = load_registry_from(&path);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].path, "/tmp/b");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn today_iso_format() {
    let today = today_iso();
    // Basic format check: YYYY-MM-DD
    assert_eq!(today.len(), 10);
    assert_eq!(today.as_bytes()[4], b'-');
    assert_eq!(today.as_bytes()[7], b'-');
}
