//! The NAM model library (nam-model-library.md §4, §5.1, §8, §11): the
//! header reader, the hash cache, the stable slot table and its migration,
//! duplicates, import and delete, and the cross-process lock.
//!
//! Every test uses its own temporary root through the explicit-root API;
//! nothing reads `RESONANCE_AMP_MODEL_DIR` or the user's data dir.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;

use resonance_common::nam_library::{
    self, read_header, read_sidecar, sidecar_path, write_sidecar, EntryStatus, ImportOutcome,
    Library, Sidecar, Source, IMPORTED_DIR, SOURCE_TONE3000, TONE3000_DIR,
};

// ---------------------------------------------------------------------------
// Per-thread allocation accounting, for "reads metadata without allocating
// the weights". Thread-local so parallel tests do not pollute each other.
// ---------------------------------------------------------------------------

struct Counting;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATED.try_with(|a| a.set(a.get() + layout.size()));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocated() -> usize {
    ALLOCATED.with(|a| a.get())
}

// ---------------------------------------------------------------------------

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../plugins/resonance-amp/tests/fixtures")
        .join(rel)
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance-namlib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A tiny valid `.nam` whose bytes (and so id) depend on `seed`.
fn write_model(path: &Path, seed: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = format!(
        r#"{{"version":"0.5.4","architecture":"WaveNet","config":{{}},"weights":[{seed}.0],"sample_rate":48000,
            "metadata":{{"name":"Model {seed}","modeled_by":"tester","gear_type":"amp","tone_type":"crunch"}}}}"#
    );
    std::fs::write(path, text).unwrap();
}

fn slot_of_path(lib: &Library, path: &Path) -> Option<u32> {
    lib.by_path(path).and_then(|e| e.slot)
}

#[test]
fn read_header_covers_the_three_fixture_families() {
    let a1 = read_header(&fixture("a1/wavenet.nam")).unwrap();
    assert_eq!(a1.architecture, "WaveNet");
    assert_eq!(a1.architecture_label(), "WaveNet A1");
    assert_eq!(a1.name.as_deref(), Some("Test Model"));
    assert_eq!(a1.modeled_by.as_deref(), Some("Steve"));
    assert_eq!(a1.gear().as_deref(), Some("Darkglass Electronics Microtubes 900 v2"));
    assert_eq!(a1.gear_type.as_deref(), Some("amp"));
    assert_eq!(a1.tone_type.as_deref(), Some("clean"));
    assert_eq!(a1.sample_rate, 48_000.0);
    assert!(a1.validation_esr.is_some_and(|e| (e - 0.1334).abs() < 1e-3));
    assert!(a1.loudness.is_some());

    let bare = read_header(&fixture("a1/wavenet_a1_standard.nam")).unwrap();
    assert_eq!(bare.name, None, "no metadata block");
    assert_eq!(bare.sample_rate, nam_library::DEFAULT_SAMPLE_RATE);
    assert!(!bare.sample_rate_declared);

    let a2 = read_header(&fixture("a2/A2.nam")).unwrap();
    assert_eq!(a2.architecture_label(), "A2 slimmable");
    let a2_net = read_header(&fixture("a2/wavenet_a2_max.nam")).unwrap();
    assert_eq!(a2_net.architecture_label(), "WaveNet A2");

    let lstm = read_header(&fixture("lstm/lstm.nam")).unwrap();
    assert_eq!(lstm.architecture_label(), "LSTM");
    assert_eq!(lstm.validation_esr, None, "a null ESR is absent, not an error");
}

#[test]
fn read_header_does_not_allocate_the_weights() {
    let root = temp_root("alloc");
    let path = root.join("big.nam");
    // ~2 million weights, ~16 MB of JSON: parsing them would allocate 8 MB
    // as Vec<f32> alone.
    let mut text = String::from(r#"{"version":"0.5.4","architecture":"WaveNet","config":{"layers":[1,2,3]},"weights":["#);
    for i in 0..2_000_000 {
        if i > 0 {
            text.push(',');
        }
        text.push_str("0.123456");
    }
    text.push_str(r#"],"sample_rate":44100,"metadata":{"name":"Big"}}"#);
    std::fs::write(&path, &text).unwrap();
    drop(text);

    let before = allocated();
    let h = read_header(&path).unwrap();
    let used = allocated() - before;
    assert_eq!(h.name.as_deref(), Some("Big"));
    assert_eq!(h.sample_rate, 44_100.0);
    assert!(used < 512 * 1024, "read_header allocated {used} bytes for a 16 MB file");
}

#[test]
fn a_file_that_is_not_a_model_is_refused_with_a_reason() {
    let root = temp_root("notmodel");
    let bad = root.join("bad.nam");
    std::fs::write(&bad, br#"{"hello": 1}"#).unwrap();
    let err = read_header(&bad).unwrap_err();
    assert!(err.to_string().contains("not a NAM model"), "{err}");
    assert!(read_header(&root.join("missing.nam")).is_err());
}

#[test]
fn migration_assigns_slots_in_todays_sorted_download_order() {
    let root = temp_root("migrate");
    let dl = root.join(TONE3000_DIR);
    // Written out of order on purpose.
    for (name, seed) in [("c_3.nam", 3), ("a_1.nam", 1), ("b_2.nam", 2)] {
        write_model(&dl.join(name), seed);
    }
    write_model(&root.join(IMPORTED_DIR).join("z_imported.nam"), 9);
    let old_order = resonance_common::scan_directory(&dl, "nam");

    let lib = Library::open_and_scan(&root).unwrap();
    for (index, path) in old_order.iter().enumerate() {
        assert_eq!(
            slot_of_path(&lib, Path::new(path)),
            Some(index as u32),
            "file_select {index} must keep pointing at {path}"
        );
    }
    assert_eq!(slot_of_path(&lib, &root.join("imported/z_imported.nam")), Some(3));
    assert_eq!(lib.len(), 4);
    assert!(root.join("library.json").exists());
}

#[test]
fn slots_append_are_freed_and_not_reused_below_the_high_water_mark() {
    let root = temp_root("slots");
    let dl = root.join(TONE3000_DIR);
    let a = dl.join("a.nam");
    let b = dl.join("b.nam");
    let c = dl.join("c.nam");
    write_model(&a, 1);
    write_model(&b, 2);
    write_model(&c, 3);
    let mut lib = Library::open_and_scan(&root).unwrap();
    let b_id = lib.by_path(&b).unwrap().id.clone();
    assert_eq!(slot_of_path(&lib, &b), Some(1));

    // Deleting b frees slot 1; the next model does not take it.
    let removed = lib.delete(&b).unwrap();
    assert_eq!(removed.id, b_id);
    assert!(lib.by_slot(1).is_none());
    let d = dl.join("d.nam");
    write_model(&d, 4);
    lib.rescan().unwrap();
    assert_eq!(slot_of_path(&lib, &d), Some(3), "no reuse below the high-water mark");
    assert_eq!(slot_of_path(&lib, &a), Some(0));
    assert_eq!(slot_of_path(&lib, &c), Some(2), "other slots do not move");

    // The same bytes coming back (re-download) reclaim their old slot.
    write_model(&b, 2);
    lib.rescan().unwrap();
    assert_eq!(slot_of_path(&lib, &b), Some(1));
}

#[test]
fn freed_slots_are_reused_only_past_the_last_slot() {
    let root = temp_root("full");
    // An index whose high-water mark is at the end, with every slot free.
    std::fs::write(
        root.join("library.json"),
        r#"{"version":1,"generation":5,"next_slot":1000,"slots":{},"retired":{},"files":[]}"#,
    )
    .unwrap();
    let dl = root.join(TONE3000_DIR);
    write_model(&dl.join("a.nam"), 1);
    write_model(&dl.join("b.nam"), 2);
    let lib = Library::open_and_scan(&root).unwrap();
    assert_eq!(slot_of_path(&lib, &dl.join("a.nam")), Some(0));
    assert_eq!(slot_of_path(&lib, &dl.join("b.nam")), Some(1));
    assert_eq!(lib.generation(), 6);
}

#[test]
fn rescans_hash_only_new_or_changed_files() {
    let root = temp_root("rehash");
    let dl = root.join(TONE3000_DIR);
    write_model(&dl.join("a.nam"), 1);
    write_model(&dl.join("b.nam"), 2);
    let mut lib = Library::open_and_scan(&root).unwrap();
    let gen = lib.generation();

    let report = lib.rescan().unwrap();
    assert_eq!(report.hashed, 0);
    assert!(!report.changed);
    assert_eq!(lib.generation(), gen, "nothing changed, nothing written");

    // A fresh process (a new Library over the same root) also re-hashes nothing.
    let mut other = Library::open(&root);
    assert_eq!(other.len(), 2, "the cache alone lists the entries");
    assert_eq!(other.rescan().unwrap().hashed, 0);

    std::thread::sleep(std::time::Duration::from_millis(20));
    write_model(&dl.join("b.nam"), 22);
    write_model(&dl.join("c.nam"), 3);
    let report = lib.rescan().unwrap();
    assert_eq!(report.hashed, 2);
    assert_eq!(report.added.len(), 2, "b's new bytes are a new id, and c");
    assert_eq!(report.removed.len(), 1, "b's old id is gone");
}

#[test]
fn duplicates_share_one_slot_and_point_at_the_canonical_file() {
    let root = temp_root("dupes");
    let first = root.join(TONE3000_DIR).join("a.nam");
    let copy = root.join(IMPORTED_DIR).join("a copy.nam");
    write_model(&first, 1);
    write_model(&copy, 1);
    let lib = Library::open_and_scan(&root).unwrap();
    let canon = lib.by_path(&first).unwrap();
    let dup = lib.by_path(&copy).unwrap();
    assert_eq!(canon.id, dup.id);
    assert_eq!(canon.slot, Some(0));
    assert_eq!(dup.slot, None);
    assert_eq!(dup.status, EntryStatus::DuplicateOf(first.clone()));
    assert_eq!(lib.entry(&canon.id).unwrap().path, first, "the id resolves to the canonical file");
}

#[test]
fn unreadable_files_are_listed_with_their_reason_and_keep_a_slot() {
    let root = temp_root("unreadable");
    let dl = root.join(TONE3000_DIR);
    std::fs::create_dir_all(&dl).unwrap();
    std::fs::write(dl.join("a_broken.nam"), b"{ nope").unwrap();
    write_model(&dl.join("b_ok.nam"), 1);
    let lib = Library::open_and_scan(&root).unwrap();
    let broken = lib.by_path(&dl.join("a_broken.nam")).unwrap();
    assert!(matches!(&broken.status, EntryStatus::Unreadable(r) if r.contains("not a NAM model")));
    // It held index 0 in yesterday's listing too, so it keeps slot 0 and
    // the good file keeps 1.
    assert_eq!(broken.slot, Some(0));
    assert_eq!(slot_of_path(&lib, &dl.join("b_ok.nam")), Some(1));
}

#[test]
fn a_missing_root_is_an_empty_library_and_creates_nothing() {
    let root = temp_root("missing").join("not-yet");
    let lib = Library::open_and_scan(&root).unwrap();
    assert!(lib.is_empty());
    assert!(!root.exists(), "nothing is created before the first download or import");
    assert!(Library::empty().rescan().is_err(), "no root: writes are refused");
}

#[test]
fn a_corrupt_index_is_quarantined_and_rebuilt() {
    let root = temp_root("corrupt");
    write_model(&root.join(TONE3000_DIR).join("a.nam"), 1);
    std::fs::write(root.join("library.json"), b"garbage").unwrap();
    let lib = Library::open_and_scan(&root).unwrap();
    assert!(root.join("library.json.corrupt").exists());
    assert_eq!(lib.len(), 1);
    assert_eq!(lib.entries()[0].slot, Some(0));
}

#[test]
fn entries_take_their_metadata_from_the_sidecar_then_the_file() {
    let root = temp_root("sidecar");
    let dl = root.join(TONE3000_DIR);
    let path = dl.join("Friedman_BE100_standard_48121.nam");
    write_model(&path, 1);
    let sc = Sidecar {
        source: SOURCE_TONE3000.into(),
        tone_id: Some(1934),
        model_id: Some(48121),
        tone_title: Some("Friedman BE-100".into()),
        author: Some("J. Smith".into()),
        gear: Some("Friedman BE-100".into()),
        model_name: Some("BE100".into()),
        size: Some("standard".into()),
        downloaded_at: Some("2026-09-12T10:00:00Z".into()),
    };
    write_sidecar(&path, &sc).unwrap();
    assert_eq!(read_sidecar(&path), Some(sc));
    assert!(sidecar_path(&path).to_string_lossy().ends_with(".nam.meta.json"));

    let lib = Library::open_and_scan(&root).unwrap();
    assert_eq!(lib.len(), 1, "the sidecar is not a model");
    let e = lib.by_path(&path).unwrap();
    assert_eq!(e.name, "Friedman BE-100 · standard");
    assert_eq!(e.author.as_deref(), Some("J. Smith"));
    assert_eq!(e.source, Source::Tone3000 { tone_id: 1934, model_id: 48121 });
    assert_eq!(e.gear_type.as_deref(), Some("amp"));
    assert_eq!(e.tone_type.as_deref(), Some("crunch"));
    assert_eq!(e.architecture, "WaveNet A1");
    assert_eq!(
        e.added_at,
        resonance_common::library_marks::parse_timestamp("2026-09-12T10:00:00Z").unwrap()
    );
    assert_eq!(lib.tone3000_model(48121).map(|e| e.path.clone()), Some(path.clone()));
    assert!(lib.tone3000_model(1).is_none());

    // No sidecar: metadata name, then the file stem.
    write_model(&root.join(IMPORTED_DIR).join("mine.nam"), 2);
    let mut lib = Library::open_and_scan(&root).unwrap();
    let mine = lib.by_path(&root.join("imported/mine.nam")).unwrap();
    assert_eq!(mine.name, "Model 2");
    assert_eq!(mine.source, Source::Imported);
    std::fs::copy(fixture("a1/wavenet_a1_standard.nam"), root.join("loose.nam")).unwrap();
    lib.rescan().unwrap();
    let loose = lib.by_path(&root.join("loose.nam")).unwrap();
    assert_eq!(loose.name, "loose");
    assert_eq!(loose.source, Source::External);
}

#[test]
fn import_copies_checks_and_dedupes() {
    let root = temp_root("import");
    let outside = temp_root("import-src");
    let src = outside.join("my rig.nam");
    write_model(&src, 7);
    let mut lib = Library::open(&root);

    let added = lib.import(&src).unwrap();
    let ImportOutcome::Added(entry) = &added else {
        panic!("expected a copy, got {added:?}");
    };
    assert_eq!(entry.path, root.join("imported/my rig.nam"));
    assert!(src.exists(), "the user's original is untouched");
    assert_eq!(entry.slot, Some(0));

    // Same bytes again, under another name: not copied.
    let again = outside.join("renamed.nam");
    std::fs::copy(&src, &again).unwrap();
    assert!(matches!(lib.import(&again).unwrap(), ImportOutcome::AlreadyPresent(e) if e.id == entry.id));
    assert_eq!(lib.len(), 1);

    // Different bytes under a taken name get a fresh name.
    write_model(&src, 8);
    let second = lib.import(&src).unwrap();
    assert_eq!(second.entry().path, root.join("imported/my rig-2.nam"));

    let bad = outside.join("bad.nam");
    std::fs::write(&bad, b"[]").unwrap();
    let err = lib.import(&bad).unwrap_err();
    assert!(err.to_string().contains("not a NAM model"), "{err}");
}

#[test]
fn delete_removes_the_file_and_sidecar_and_only_inside_the_root() {
    let root = temp_root("delete");
    let path = root.join(TONE3000_DIR).join("a.nam");
    write_model(&path, 1);
    write_sidecar(
        &path,
        &Sidecar {
            source: SOURCE_TONE3000.into(),
            ..Sidecar::default()
        },
    )
    .unwrap();
    let mut lib = Library::open_and_scan(&root).unwrap();
    lib.delete(&path).unwrap();
    assert!(!path.exists());
    assert!(!sidecar_path(&path).exists());
    assert!(lib.is_empty());

    let outside = temp_root("delete-outside").join("x.nam");
    write_model(&outside, 1);
    assert!(lib.delete(&outside).is_err());
    assert!(outside.exists());
}

#[test]
fn find_resolves_names_and_id_prefixes() {
    let root = temp_root("find");
    let dl = root.join(TONE3000_DIR);
    write_model(&dl.join("a.nam"), 1);
    write_model(&dl.join("b.nam"), 12);
    write_model(&dl.join("c.nam"), 3);
    let lib = Library::open_and_scan(&root).unwrap();
    assert_eq!(lib.find("model 3").unwrap().slot, Some(2), "exact name, any case");
    assert!(lib.find("Model 1").is_some_and(|e| e.slot == Some(0)), "exact beats prefix");
    assert!(lib.find("Model").is_none(), "ambiguous prefix");
    let id = lib.by_slot(1).unwrap().id.clone();
    assert_eq!(lib.find(&id[..8]).unwrap().slot, Some(1));
    assert!(lib.find("nothing").is_none());
}

#[test]
fn reload_if_changed_sees_another_writer() {
    let root = temp_root("reload");
    write_model(&root.join(TONE3000_DIR).join("a.nam"), 1);
    let mut reader = Library::open_and_scan(&root).unwrap();
    assert!(!reader.reload_if_changed());
    write_model(&root.join(TONE3000_DIR).join("b.nam"), 2);
    Library::open_and_scan(&root).unwrap();
    assert!(reader.reload_if_changed());
    assert_eq!(reader.len(), 2);
}

// ---------------------------------------------------------------------------
// Two processes allocating slots at once
// ---------------------------------------------------------------------------

const CHILD_ROOT: &str = "RESONANCE_NAMLIB_CHILD_ROOT";
const CHILD_SEED: &str = "RESONANCE_NAMLIB_CHILD_SEED";
const MODELS_PER_CHILD: u32 = 12;

#[test]
#[ignore = "worker half of two_processes_never_share_a_slot"]
fn namlib_child_scanner() {
    let (Some(root), Some(seed)) = (
        std::env::var_os(CHILD_ROOT),
        std::env::var(CHILD_SEED).ok().and_then(|s| s.parse::<u32>().ok()),
    ) else {
        return;
    };
    let root = PathBuf::from(root);
    let mut lib = Library::open(&root);
    for i in 0..MODELS_PER_CHILD {
        write_model(&root.join(TONE3000_DIR).join(format!("{seed}-{i}.nam")), seed * 1000 + i);
        lib.rescan().unwrap();
    }
}

#[test]
fn two_processes_never_share_a_slot() {
    let root = temp_root("processes");
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = [1u32, 2]
        .into_iter()
        .map(|seed| {
            Command::new(&exe)
                .args(["--ignored", "--exact", "namlib_child_scanner", "--test-threads=1"])
                .env(CHILD_ROOT, &root)
                .env(CHILD_SEED, seed.to_string())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut c in children {
        assert!(c.wait().unwrap().success());
    }
    let lib = Library::open_and_scan(&root).unwrap();
    let mut slots: Vec<u32> = lib.entries().iter().filter_map(|e| e.slot).collect();
    assert_eq!(slots.len(), (2 * MODELS_PER_CHILD) as usize, "every model got a slot");
    slots.sort();
    slots.dedup();
    assert_eq!(slots.len(), (2 * MODELS_PER_CHILD) as usize, "no slot holds two models");
    assert_eq!(*slots.last().unwrap(), 2 * MODELS_PER_CHILD - 1, "slots stay dense");
}
