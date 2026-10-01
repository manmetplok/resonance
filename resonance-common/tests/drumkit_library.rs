//! The drum-kit library (drums-plugin-rework.md §3, §6.5, §9): scanning
//! at depth 0 and 1, manifest-hash ids, sidecars and name precedence, the
//! `installed.json` migration, import through `.staging/` (cancel, disk
//! check, zip), delete, the lazy missing-file check and the cross-process
//! lock.
//!
//! Every test uses its own temporary root through the explicit-root API;
//! nothing reads `RESONANCE_DRUMKIT_DIR` or the user's data dir.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use resonance_common::drumkit_library::{
    self, find_manifest, measure_size, read_sidecar, write_sidecar, EntryStatus, ImportJob,
    ImportOutcome, Library, LibraryError, Sidecar, Source, LIBRARY_FILE, MANIFEST_FILE,
    SIDECAR_FILE, SOURCE_IMPORTED, SOURCE_PLOK, STAGING_DIR,
};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance-kitlib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A small kit in `manifest_dir`: `pieces` pieces, two mic setups, two
/// round robins of three velocity layers, every sample `sample_bytes`
/// long. `seed` changes the manifest bytes (and so the id).
fn write_kit(
    manifest_dir: &Path,
    seed: u32,
    pieces: usize,
    meta: Option<&str>,
    sample_bytes: usize,
) {
    std::fs::create_dir_all(manifest_dir).unwrap();
    let mut obj = serde_json::Map::new();
    for p in 0..pieces {
        let piece = format!("SD Piece {p}");
        let mut setups = serde_json::Map::new();
        for (key, pos, brand, mic) in [
            ("01_KickIn_e901", "KickIn", "Sennheiser", "e901"),
            ("19_OHsAB_KM184", "OHsAB", "Neumann", "KM184"),
        ] {
            let mut rounds = serde_json::Map::new();
            for rr in 1..=2 {
                let mut vels = serde_json::Map::new();
                for v in 1..=3 {
                    let file = format!("{piece} {key} RR{rr:02} Vel{v:02}.wav");
                    std::fs::write(manifest_dir.join(&file), vec![seed as u8; sample_bytes])
                        .unwrap();
                    vels.insert(format!("Vel{v:02}"), file.into());
                }
                rounds.insert(format!("RR{rr:02}"), vels.into());
            }
            setups.insert(
                key.into(),
                serde_json::json!({
                    "brand": brand, "channel": &key[..2], "mic": mic, "position": pos,
                    "rounds": rounds,
                }),
            );
        }
        obj.insert(piece, setups.into());
    }
    if let Some(meta) = meta {
        obj.insert("_meta".into(), serde_json::from_str(meta).unwrap());
    }
    let mut text = serde_json::to_string_pretty(&obj).unwrap();
    text.push_str(&format!("\n{}", " ".repeat(seed as usize)));
    std::fs::write(manifest_dir.join(MANIFEST_FILE), text).unwrap();
}

fn names(lib: &Library) -> Vec<String> {
    lib.entries().iter().map(|e| e.name.clone()).collect()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn scans_kits_with_the_manifest_at_depth_0_and_1() {
    let root = temp_root("depth");
    write_kit(&root.join("Flat"), 1, 2, None, 4);
    write_kit(&root.join("Nested").join("nested"), 2, 3, None, 4);
    // Depth 2 is not a kit; a stray file and a hidden dir are ignored.
    write_kit(&root.join("Deep").join("a").join("b"), 3, 1, None, 4);
    std::fs::write(root.join("notes.txt"), b"x").unwrap();
    write_kit(&root.join(".hidden"), 4, 1, None, 4);

    let lib = Library::open_and_scan(&root).unwrap();
    assert_eq!(names(&lib), ["Flat", "Nested"]);
    let flat = &lib.entries()[0];
    assert_eq!(flat.dir, root.join("Flat"));
    assert_eq!(flat.manifest_path, root.join("Flat").join(MANIFEST_FILE));
    assert_eq!(flat.rel_path, Path::new("Flat").join(MANIFEST_FILE));
    assert_eq!(flat.slot, Some(0));
    assert_eq!(flat.source, Source::Local, "no sidecar → local");
    assert!(flat.is_ok());
    let nested = &lib.entries()[1];
    assert_eq!(
        nested.rel_path,
        Path::new("Nested/nested").join(MANIFEST_FILE)
    );
    assert_eq!(nested.slot, Some(1));

    // What the manifest says, without opening a WAV.
    assert_eq!(nested.pieces.len(), 3);
    assert_eq!(nested.mic_setups.len(), 2);
    assert_eq!(nested.mic_setups["01_KickIn_e901"].brand, "Sennheiser");
    assert_eq!(
        nested.mic_label("01_KickIn_e901"),
        "Sennheiser e901 · KickIn"
    );
    assert_eq!(nested.mic_label("unknown"), "unknown");
    assert_eq!(nested.layers_max, 3);
    assert_eq!(nested.rr_max, 2);
    assert_eq!(nested.sample_count, 3 * 2 * 2 * 3);
    assert_eq!(nested.size_bytes, None, "a local kit is measured lazily");
    assert_eq!(lib.unsized_dirs().len(), 2);
    assert!(root.join(LIBRARY_FILE).is_file());
}

#[test]
fn ids_are_manifest_hashes_stable_across_rename_and_move() {
    let root = temp_root("rename");
    write_kit(&root.join("Kit A").join("kit"), 1, 1, None, 4);
    write_kit(&root.join("Kit B"), 2, 1, None, 4);
    let mut lib = Library::open_and_scan(&root).unwrap();
    let a = lib.by_dir(&root.join("Kit A")).unwrap().clone();
    let bytes = std::fs::read(&a.manifest_path).unwrap();
    assert_eq!(a.id, resonance_common::nam_library::hash_bytes(&bytes));
    assert_eq!(a.mark_key(), format!("drumkit:{}", a.id));
    assert_eq!(drumkit_library::mark_key(&a.id), a.mark_key());

    // Rename the top dir and flatten it: same manifest, same id, same slot.
    std::fs::rename(root.join("Kit A").join("kit"), root.join("Renamed")).unwrap();
    std::fs::remove_dir(root.join("Kit A")).unwrap();
    lib.rescan().unwrap();
    let moved = lib.entry(&a.id).unwrap();
    assert_eq!(moved.dir, root.join("Renamed"));
    assert_eq!(moved.slot, a.slot);
    assert_eq!(moved.name, "Renamed");
    assert_eq!(lib.by_id(&a.id), Some(moved));

    // The kit leaving frees its slot without reuse; coming back reclaims it.
    let away = temp_root("rename-away");
    std::fs::rename(root.join("Renamed"), away.join("Renamed")).unwrap();
    let report = lib.rescan().unwrap();
    assert_eq!(report.removed, vec![a.id.clone()]);
    write_kit(&root.join("Kit C"), 3, 1, None, 4);
    lib.rescan().unwrap();
    assert_eq!(lib.by_dir(&root.join("Kit C")).unwrap().slot, Some(2));
    std::fs::rename(away.join("Renamed"), root.join("Back")).unwrap();
    lib.rescan().unwrap();
    assert_eq!(lib.entry(&a.id).unwrap().slot, a.slot);
}

#[test]
fn sidecars_round_trip_and_set_provenance() {
    let root = temp_root("sidecar");
    let dir = root.join("Drummica");
    write_kit(&dir.join("drummica"), 1, 1, None, 4);
    let sc = Sidecar {
        source: SOURCE_PLOK.into(),
        index_name: Some("Drummica".into()),
        index_file: Some("drummica.zip".into()),
        sha256: Some("ab".repeat(32)),
        description: Some("Acoustic studio kit".into()),
        index_tags: vec!["acoustic".into(), "rock".into()],
        downloaded_at: Some("2026-04-12T00:00:00Z".into()),
        size_bytes: Some(9_126_805_504),
    };
    write_sidecar(&dir, &sc).unwrap();
    assert!(dir.join(SIDECAR_FILE).is_file());
    assert_eq!(read_sidecar(&dir), Some(sc.clone()));

    let mut lib = Library::open_and_scan(&root).unwrap();
    let e = &lib.entries()[0];
    assert_eq!(e.source, Source::Plok);
    assert_eq!(e.sidecar.as_ref(), Some(&sc));
    assert_eq!(e.size_bytes, Some(9_126_805_504));
    assert_eq!(e.description(), Some("Acoustic studio kit"));
    assert_eq!(e.index_tags(), ["acoustic", "rock"]);
    assert_eq!(
        e.added_at,
        resonance_common::library_marks::parse_timestamp("2026-04-12T00:00:00Z").unwrap()
    );
    assert_eq!(drumkit_library::format_bytes(9_126_805_504), "8.5 GB");

    // A sidecar edit alone is picked up by a rescan.
    write_sidecar(
        &dir,
        &Sidecar {
            description: Some("changed".into()),
            ..sc
        },
    )
    .unwrap();
    assert!(lib.rescan().unwrap().changed);
    assert_eq!(lib.entries()[0].description(), Some("changed"));
}

#[test]
fn names_come_from_the_sidecar_then_meta_then_the_directory() {
    let root = temp_root("names");
    let meta = r#"{"name":"Meta Name","pieces":{"SD Piece 0":{"name":"Kick"}},
        "articulations":[{"primary":"SD Piece 0","alt":"SD Piece 1","label":"punch/deep"}]}"#;
    write_kit(&root.join("dir-only"), 1, 2, None, 4);
    write_kit(&root.join("with-meta").join("inner"), 2, 2, Some(meta), 4);
    write_kit(&root.join("with-sidecar"), 3, 2, Some(meta), 4);
    write_sidecar(
        &root.join("with-sidecar"),
        &Sidecar {
            source: SOURCE_PLOK.into(),
            index_name: Some("Index Name".into()),
            ..Sidecar::default()
        },
    )
    .unwrap();
    let lib = Library::open_and_scan(&root).unwrap();
    let by = |d: &str| lib.by_dir(&root.join(d)).unwrap();
    assert_eq!(by("dir-only").name, "dir-only");
    assert_eq!(by("with-meta").name, "Meta Name");
    assert_eq!(by("with-sidecar").name, "Index Name");

    let m = by("with-meta");
    assert_eq!(m.pieces[0].name, "Kick", "_meta.pieces display name");
    assert_eq!(m.pieces[1].name, "SD Piece 1", "raw key without one");
    assert_eq!(m.piece_name("SD Piece 0"), "Kick");
    assert_eq!(m.articulations.len(), 1);
    assert_eq!(m.articulations[0].label, "punch/deep");
    assert!(by("dir-only").articulations.is_empty());

    // find(): exact, prefix, id prefix; nothing for an ambiguity.
    assert_eq!(lib.find("meta name").unwrap().dir, root.join("with-meta"));
    assert_eq!(lib.find("Index").unwrap().dir, root.join("with-sidecar"));
    let id = &by("dir-only").id;
    assert_eq!(lib.find(&id[..8]).unwrap().dir, root.join("dir-only"));
    assert!(lib.find("name").is_none(), "two names contain it");
}

#[test]
fn installed_json_items_become_sidecars_once() {
    let root = temp_root("migrate");
    write_kit(&root.join("Drummica").join("drummica"), 1, 1, None, 4);
    write_kit(&root.join("IT Techno").join("ittechno"), 2, 1, None, 4);
    write_kit(&root.join("Hand Copied"), 3, 1, None, 4);
    let reg_dir = temp_root("migrate-reg");
    let reg = reg_dir.join("installed.json");
    // Drummica's path is from another root (the data dir moved): matched
    // by directory name. IT Techno's is exact.
    let text = format!(
        r#"{{"items":[
            {{"name":"Drummica","type":"drumkit","path":"/elsewhere/drumkits/Drummica","installed_at":"2026-04-12"}},
            {{"name":"IT Techno","type":"drumkit","path":"{}","installed_at":"2026-08-04"}},
            {{"name":"Gone","type":"drumkit","path":"/nowhere/Gone","installed_at":"2026-01-01"}},
            {{"name":"x","type":"amp-model","path":"/x","installed_at":"2026-01-01"}}
        ]}}"#,
        root.join("IT Techno").display()
    );
    std::fs::write(&reg, &text).unwrap();

    let in_index: Arc<drumkit_library::InIndexFn> = Arc::new(|name: &str| name == "Drummica");
    let mut lib = Library::open(&root).with_installed_json(&reg, Some(in_index.clone()));
    let report = lib.rescan().unwrap();
    assert_eq!(report.migrated, 2);
    let by = |l: &Library, d: &str| l.by_dir(&root.join(d)).unwrap().clone();
    let drummica = by(&lib, "Drummica");
    assert_eq!(drummica.source, Source::Plok);
    assert_eq!(drummica.name, "Drummica");
    assert_eq!(
        drummica.added_at,
        resonance_common::library_marks::parse_timestamp("2026-04-12T00:00:00Z").unwrap()
    );
    let techno = by(&lib, "IT Techno");
    assert_eq!(techno.source, Source::Local, "not in the index");
    assert_eq!(
        techno.sidecar.as_ref().unwrap().downloaded_at.as_deref(),
        Some("2026-08-04T00:00:00Z")
    );
    assert!(by(&lib, "Hand Copied").sidecar.is_none());
    assert_eq!(
        std::fs::read_to_string(&reg).unwrap(),
        text,
        "installed.json untouched"
    );

    // Once per index: a fresh library over the same root does not redo it.
    std::fs::remove_file(root.join("IT Techno").join(SIDECAR_FILE)).unwrap();
    let mut again = Library::open(&root).with_installed_json(&reg, Some(in_index));
    assert_eq!(again.rescan().unwrap().migrated, 0);
    assert!(by(&again, "IT Techno").sidecar.is_none());
}

#[test]
fn import_copies_through_staging_and_dedupes() {
    let root = temp_root("import");
    let src_root = temp_root("import-src");
    let src = src_root.join("My Kit");
    write_kit(&src.join("inner"), 7, 2, None, 64 * 1024);
    let total = measure_size(&src).unwrap();

    let mut lib = Library::open_and_scan(&root).unwrap();
    let mut seen_staged = false;
    let mut last = Default::default();
    let outcome = lib
        .import(
            &src,
            ImportJob::new().progress(|p| {
                last = p;
                // While copying, the kit is only under `.staging/`.
                let staged = listing(&root.join(STAGING_DIR));
                if staged.len() == 1 && staged[0].starts_with("My Kit.") {
                    seen_staged = true;
                }
                assert!(!root.join("My Kit").exists(), "never listed half-copied");
            }),
        )
        .unwrap();
    assert!(seen_staged);
    assert_eq!(last.bytes_done, total);
    assert_eq!(last.bytes_total, total);
    assert_eq!(last.files_done, last.files_total);
    let ImportOutcome::Added(e) = outcome else {
        panic!("expected Added");
    };
    assert_eq!(e.dir, root.join("My Kit"));
    assert_eq!(e.rel_path, Path::new("My Kit/inner").join(MANIFEST_FILE));
    assert_eq!(e.source, Source::Imported);
    assert_eq!(e.size_bytes, Some(total));
    assert!(e.sidecar.as_ref().unwrap().downloaded_at.is_some());
    assert_eq!(read_sidecar(&e.dir).unwrap().source, SOURCE_IMPORTED);
    assert!(!root.join(STAGING_DIR).exists(), "staging cleaned up");
    assert!(
        src.join("inner").join(MANIFEST_FILE).is_file(),
        "the source is untouched"
    );

    // Same manifest again: nothing copied.
    let again = lib.import(&src, ImportJob::new()).unwrap();
    assert!(matches!(again, ImportOutcome::AlreadyPresent(ref x) if x.id == e.id));
    assert_eq!(listing(&root), ["My Kit", "library.json", "library.lock"]);

    // A folder with no manifest is refused.
    let empty = src_root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        lib.import(&empty, ImportJob::new()),
        Err(LibraryError::NotAKit { .. })
    ));
}

#[test]
fn a_cancelled_import_leaves_nothing_behind() {
    let root = temp_root("cancel");
    let src_root = temp_root("cancel-src");
    let src = src_root.join("Big");
    // 2 MiB per sample: several chunks per file.
    write_kit(&src, 1, 1, None, 2 * 1024 * 1024);
    let mut lib = Library::open_and_scan(&root).unwrap();
    let cancel = AtomicBool::new(false);
    let result = lib.import(
        &src,
        ImportJob::new().cancel(&cancel).progress(|p| {
            if p.bytes_done > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        }),
    );
    assert!(matches!(result, Err(LibraryError::Cancelled)), "{result:?}");
    assert!(!root.join(STAGING_DIR).exists());
    assert!(!root.join("Big").exists());
    assert!(lib.is_empty());
}

#[test]
fn import_refuses_when_the_disk_is_short() {
    let root = temp_root("space");
    let src_root = temp_root("space-src");
    let src = src_root.join("Kit");
    write_kit(&src, 1, 1, None, 1000);
    let total = measure_size(&src).unwrap();
    let mut lib = Library::open_and_scan(&root).unwrap();
    // Exactly the kit's size is not enough: the margin is 10 %.
    let err = lib
        .import(&src, ImportJob::new().free_space(|_| Some(total)))
        .unwrap_err();
    let LibraryError::InsufficientSpace { needed, available } = err else {
        panic!("expected InsufficientSpace, got {err}");
    };
    assert_eq!(available, total);
    assert!(needed >= total + total / 10);
    assert!(err.to_string().contains("short"), "{err}");
    assert!(!root.join("Kit").exists());
    assert!(!root.join(STAGING_DIR).exists());
    // With room it goes through; an unknown free space skips the check.
    lib.import(&src, ImportJob::new().free_space(|_| None))
        .unwrap();
    assert!(root.join("Kit").is_dir());
}

fn zip_dir(src: &Path, prefix: &str, zip_path: &Path) {
    let file = std::fs::File::create(zip_path).unwrap();
    let mut zw = zip::ZipWriter::new(file);
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                walk(base, &e.path(), out);
            } else {
                out.push(e.path().strip_prefix(base).unwrap().to_path_buf());
            }
        }
    }
    let mut files = Vec::new();
    walk(src, src, &mut files);
    for rel in files {
        let name = format!("{prefix}{}", rel.to_string_lossy());
        zw.start_file(name, opts).unwrap();
        zw.write_all(&std::fs::read(src.join(&rel)).unwrap())
            .unwrap();
    }
    zw.finish().unwrap();
}

#[test]
fn import_zip_extracts_through_staging_and_hoists_a_wrapper_dir() {
    let root = temp_root("zip");
    let src_root = temp_root("zip-src");
    write_kit(&src_root.join("a"), 1, 1, None, 16);
    write_kit(&src_root.join("b"), 2, 1, None, 16);
    // a.zip: kit/drum_samples.json (depth 1). b.zip: Wrapper/kit/… (depth 2).
    zip_dir(&src_root.join("a"), "kit/", &src_root.join("Zipped A.zip"));
    zip_dir(
        &src_root.join("b"),
        "Wrapper/kit/",
        &src_root.join("Zipped B.zip"),
    );

    let mut lib = Library::open_and_scan(&root).unwrap();
    let a = lib
        .import(&src_root.join("Zipped A.zip"), ImportJob::new())
        .unwrap();
    let ImportOutcome::Added(a) = a else { panic!() };
    assert_eq!(a.dir, root.join("Zipped A"));
    assert_eq!(
        a.manifest_path,
        root.join("Zipped A/kit").join(MANIFEST_FILE)
    );
    assert_eq!(a.source, Source::Imported);
    assert!(a.size_bytes.is_some());
    let b = lib
        .import_zip(&src_root.join("Zipped B.zip"), ImportJob::new())
        .unwrap();
    assert_eq!(
        b.entry().manifest_path,
        root.join("Zipped B/kit").join(MANIFEST_FILE)
    );
    // Same kit again from a zip: discarded.
    let again = lib
        .import(&src_root.join("Zipped A.zip"), ImportJob::new())
        .unwrap();
    assert!(matches!(again, ImportOutcome::AlreadyPresent(_)));
    assert_eq!(
        listing(&root),
        ["Zipped A", "Zipped B", "library.json", "library.lock"]
    );

    // install_zip carries a download's sidecar.
    write_kit(&src_root.join("c"), 3, 1, None, 16);
    zip_dir(&src_root.join("c"), "", &src_root.join("c.zip"));
    let sc = Sidecar {
        source: SOURCE_PLOK.into(),
        index_name: Some("Plok Kit".into()),
        index_file: Some("c.zip".into()),
        ..Sidecar::default()
    };
    let c = lib
        .install_zip(&src_root.join("c.zip"), "Plok Kit", sc, ImportJob::new())
        .unwrap();
    assert_eq!(c.entry().name, "Plok Kit");
    assert_eq!(c.entry().source, Source::Plok);
    assert_eq!(
        c.entry().manifest_path,
        root.join("Plok Kit").join(MANIFEST_FILE)
    );
}

#[test]
fn delete_removes_the_kit_frees_its_slot_and_refuses_anything_else() {
    let root = temp_root("delete");
    write_kit(&root.join("A").join("a"), 1, 1, None, 4);
    write_kit(&root.join("B"), 2, 1, None, 4);
    let mut lib = Library::open_and_scan(&root).unwrap();
    let a = lib.by_dir(&root.join("A")).unwrap().clone();

    for bad in [
        root.clone(),
        root.join("A").join("a"),
        root.join("A").join("..").join("B"),
        root.join(LIBRARY_FILE),
        PathBuf::from("/tmp"),
    ] {
        assert!(
            lib.delete(&bad).is_err(),
            "{} must be refused",
            bad.display()
        );
    }
    assert!(root.join("A").is_dir() && root.join("B").is_dir());

    let removed = lib.delete(&root.join("A")).unwrap();
    assert_eq!(removed.id, a.id);
    assert!(!root.join("A").exists());
    assert!(lib.entry(&a.id).is_none());
    assert_eq!(names(&lib), ["B"]);
    assert_eq!(lib.by_dir(&root.join("B")).unwrap().slot, Some(1));
}

#[test]
fn missing_files_are_found_lazily_and_bad_manifests_say_why() {
    let root = temp_root("missing");
    write_kit(&root.join("Kit"), 1, 2, None, 4);
    std::fs::create_dir_all(root.join("Broken")).unwrap();
    std::fs::write(root.join("Broken").join(MANIFEST_FILE), b"{\"SD Kick\": 3}").unwrap();
    let mut lib = Library::open_and_scan(&root).unwrap();
    let broken = lib.by_dir(&root.join("Broken")).unwrap();
    assert!(matches!(&broken.status, EntryStatus::ManifestError(r) if r.contains("bad piece")));
    assert!(!broken.is_loadable());
    assert!(broken.slot.is_some(), "a broken kit still holds its slot");

    let kit = lib.by_dir(&root.join("Kit")).unwrap().clone();
    let victims: Vec<_> = std::fs::read_dir(root.join("Kit"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .take(2)
        .collect();
    for v in &victims {
        std::fs::remove_file(v).unwrap();
    }
    // A scan never stats samples.
    lib.rescan().unwrap();
    assert!(lib.entry(&kit.id).unwrap().is_ok());
    assert_eq!(lib.check_missing_files(&kit.id).unwrap(), 2);
    let e = lib.entry(&kit.id).unwrap();
    assert_eq!(e.status, EntryStatus::MissingFiles(2));
    assert!(e.is_loadable());
    let mut missing = drumkit_library::missing_files(&kit.manifest_path).unwrap();
    missing.sort();
    let mut victims = victims;
    victims.sort();
    assert_eq!(missing, victims);
}

#[test]
fn sizes_are_recorded_reloaded_and_shared_with_other_readers() {
    let root = temp_root("size");
    write_kit(&root.join("Kit"), 1, 1, None, 100);
    let mut writer = Library::open_and_scan(&root).unwrap();
    let mut reader = Library::open(&root);
    assert!(!reader.reload_if_changed());
    let dirs = writer.unsized_dirs();
    assert_eq!(dirs, vec![root.join("Kit")]);
    let bytes = measure_size(&dirs[0]).unwrap();
    writer.record_size(&dirs[0], bytes).unwrap();
    assert_eq!(writer.entries()[0].size_bytes, Some(bytes));
    assert!(writer.unsized_dirs().is_empty());
    assert!(reader.reload_if_changed());
    assert_eq!(reader.entries()[0].size_bytes, Some(bytes));
    assert_eq!(reader.total_bytes(), bytes);
    // An unchanged library rescans without writing.
    let gen = writer.generation();
    assert!(!writer.rescan().unwrap().changed);
    assert_eq!(writer.generation(), gen);
}

#[test]
fn a_missing_root_is_empty_and_find_manifest_prefers_depth_0() {
    let root = temp_root("noroot").join("absent");
    let lib = Library::open_and_scan(&root).unwrap();
    assert!(lib.is_empty());
    assert!(!root.exists(), "nothing is created until an import");
    assert!(matches!(
        Library::empty().rescan(),
        Err(LibraryError::NoRoot)
    ));

    let kit = temp_root("prefer");
    write_kit(&kit, 1, 1, None, 4);
    write_kit(&kit.join("sub"), 2, 1, None, 4);
    assert_eq!(find_manifest(&kit), Some(kit.join(MANIFEST_FILE)));
}

// ---------------------------------------------------------------------------
// Two processes writing the index at once
// ---------------------------------------------------------------------------

const CHILD_ROOT: &str = "RESONANCE_KITLIB_CHILD_ROOT";
const CHILD_SEED: &str = "RESONANCE_KITLIB_CHILD_SEED";
const KITS_PER_CHILD: u32 = 25;

/// Not a test on its own: the parent re-runs this binary with `--ignored
/// --exact kitlib_child_scanner`. Adds kits one at a time, rescanning after
/// each, and logs every (id, slot) it sees.
#[test]
#[ignore = "worker half of two_processes_never_share_or_move_a_slot"]
fn kitlib_child_scanner() {
    let (Some(root), Some(seed)) = (
        std::env::var_os(CHILD_ROOT),
        std::env::var(CHILD_SEED)
            .ok()
            .and_then(|s| s.parse::<u32>().ok()),
    ) else {
        return;
    };
    let root = PathBuf::from(root);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !root.join("go").exists() {
        assert!(std::time::Instant::now() < deadline, "no go");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut lib = Library::open(&root);
    let mut log = String::new();
    for i in 0..KITS_PER_CHILD {
        write_kit(
            &root.join(format!("kit-{seed}-{i}")),
            seed * 100 + i,
            1,
            None,
            1,
        );
        lib.rescan().unwrap();
        for e in lib.entries() {
            if let Some(slot) = e.slot {
                log.push_str(&format!("{} {slot}\n", e.id));
            }
        }
    }
    std::fs::write(root.join(format!(".slots-{seed}.log")), log).unwrap();
}

#[test]
fn two_processes_never_share_or_move_a_slot() {
    let root = temp_root("processes");
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = [1u32, 2]
        .into_iter()
        .map(|seed| {
            Command::new(&exe)
                .args([
                    "--ignored",
                    "--exact",
                    "kitlib_child_scanner",
                    "--test-threads=1",
                ])
                .env(CHILD_ROOT, &root)
                .env(CHILD_SEED, seed.to_string())
                .spawn()
                .unwrap()
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::fs::write(root.join("go"), b"").unwrap();
    for mut c in children {
        assert!(c.wait().unwrap().success());
    }
    let mut seen: std::collections::HashMap<String, u32> = Default::default();
    for seed in [1, 2] {
        let log = std::fs::read_to_string(root.join(format!(".slots-{seed}.log"))).unwrap();
        for line in log.lines() {
            let (id, slot) = line.split_once(' ').unwrap();
            let slot: u32 = slot.parse().unwrap();
            let first = *seen.entry(id.to_string()).or_insert(slot);
            assert_eq!(first, slot, "kit {id} moved from slot {first} to {slot}");
        }
    }
    let lib = Library::open(&root);
    let mut slots: Vec<u32> = lib.entries().iter().filter_map(|e| e.slot).collect();
    assert_eq!(
        slots.len(),
        (2 * KITS_PER_CHILD) as usize,
        "every kit got a slot"
    );
    slots.sort();
    slots.dedup();
    assert_eq!(
        slots.len(),
        (2 * KITS_PER_CHILD) as usize,
        "no slot holds two kits"
    );
    for e in lib.entries() {
        assert_eq!(seen.get(&e.id), e.slot.as_ref());
    }
}
