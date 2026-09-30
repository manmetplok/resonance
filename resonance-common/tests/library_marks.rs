//! The shared library marks store (nam-model-library.md §4.3 / §8,
//! plugin-preset-library.md §4.5 / §11): per-item read-modify-write under a
//! lock, `generation`, kinds that never collide, orphan pruning with an
//! injected clock, vocabulary and tag completion, and the freshness poll.
//!
//! Every test uses its own temporary directory through the explicit-dir
//! API; nothing here reads `RESONANCE_LIBRARY_DIR` or the user's data dir.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use resonance_common::library_marks::{
    self, fingerprint, kind, mark_key, normalize_tag, split_key, vocab, FreshnessPoll, Marks,
    MarksStore, ORPHAN_RETENTION_SECS,
};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-marks-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const T0: i64 = 1_790_000_000;

#[test]
fn keys_are_kind_colon_opaque_id() {
    assert_eq!(mark_key(kind::AMP_MODEL, "9f2c"), "amp-model:9f2c");
    let preset = mark_key(kind::PLUGIN_PRESET, "com.resonance.reverb:tight-room");
    assert_eq!(
        split_key(&preset),
        Some(("plugin-preset", "com.resonance.reverb:tight-room")),
        "only the first ':' separates the kind"
    );
    assert_eq!(split_key("no-colon"), None);
    assert_eq!(split_key(":id"), None);
}

#[test]
fn a_fresh_store_is_empty_and_writes_nothing_until_a_mark() {
    let dir = temp_dir("fresh");
    let store = MarksStore::open(&dir).unwrap();
    assert_eq!(store.generation(), 0);
    assert_eq!(store.iter().count(), 0);
    assert!(!store.path().exists());
    assert_eq!(store.marks("amp-model:x"), Marks::default());
}

#[test]
fn marks_round_trip_through_the_file_with_rfc3339_times() {
    let dir = temp_dir("roundtrip");
    let key = mark_key(kind::AMP_MODEL, "abc");
    let mut store = MarksStore::open(&dir).unwrap();
    store.set_favorite(&key, true).unwrap();
    store.set_tags(&key, &["Djent", "rhythm", "djent"]).unwrap();
    store.record_use(&key, T0).unwrap();
    store.record_use(&key, T0 + 60).unwrap();
    assert_eq!(store.generation(), 4);

    let text = std::fs::read_to_string(store.path()).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["generation"], 4);
    let item = &doc["items"]["amp-model:abc"];
    assert_eq!(item["favorite"], true);
    assert_eq!(item["tags"], serde_json::json!(["djent", "rhythm"]));
    assert_eq!(item["use_count"], 2);
    let last = item["last_used"].as_str().unwrap();
    assert!(last.ends_with('Z'), "UTC RFC 3339, got {last}");
    assert_eq!(library_marks::parse_timestamp(last), Some(T0 + 60));

    let reopened = MarksStore::open(&dir).unwrap();
    let m = reopened.marks(&key);
    assert!(m.favorite);
    assert_eq!(m.tags, vec!["djent", "rhythm"]);
    assert_eq!(m.last_used, Some(T0 + 60));
    assert_eq!(m.use_count, 2);
    assert_eq!(m.rating, None);
}

#[test]
fn an_item_back_at_its_defaults_is_removed_not_stored() {
    let dir = temp_dir("defaults");
    let key = mark_key(kind::AMP_MODEL, "abc");
    let mut store = MarksStore::open(&dir).unwrap();
    store.toggle_favorite(&key).unwrap();
    assert!(store.get(&key).is_some());
    store.toggle_favorite(&key).unwrap();
    assert!(store.get(&key).is_none());
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(doc["items"], serde_json::json!({}));
    assert_eq!(doc["generation"], 2, "an unmark is still a write");
}

#[test]
fn two_stores_writing_different_items_both_survive() {
    let dir = temp_dir("two-writers");
    let mut a = MarksStore::open(&dir).unwrap();
    let mut b = MarksStore::open(&dir).unwrap();
    // `b` has a stale in-memory copy when `a` writes; its write re-reads
    // under the lock, so `a`'s star is not lost.
    a.set_favorite("amp-model:one", true).unwrap();
    b.add_tag("amp-model:two", "lead").unwrap();
    let fresh = MarksStore::open(&dir).unwrap();
    assert!(fresh.is_favorite("amp-model:one"));
    assert_eq!(fresh.marks("amp-model:two").tags, vec!["lead"]);
    assert_eq!(fresh.generation(), 2);
    assert!(b.is_favorite("amp-model:one"), "a write refreshes the writer's copy");
}

#[test]
fn the_same_item_is_last_writer_wins() {
    let dir = temp_dir("lww");
    let mut a = MarksStore::open(&dir).unwrap();
    let mut b = MarksStore::open(&dir).unwrap();
    a.set_tags("amp-model:one", &["first"]).unwrap();
    b.set_tags("amp-model:one", &["second"]).unwrap();
    assert_eq!(MarksStore::open(&dir).unwrap().marks("amp-model:one").tags, vec!["second"]);
}

#[test]
fn kinds_do_not_collide() {
    let dir = temp_dir("kinds");
    let mut store = MarksStore::open(&dir).unwrap();
    store
        .set_favorite(&mark_key(kind::AMP_MODEL, "same"), true)
        .unwrap();
    store
        .add_tag(&mark_key(kind::PLUGIN_PRESET, "same"), "pad")
        .unwrap();
    assert!(store.is_favorite("amp-model:same"));
    assert!(!store.is_favorite("plugin-preset:same"));
    let amps: Vec<_> = store.iter_kind(kind::AMP_MODEL).map(|(id, _)| id).collect();
    assert_eq!(amps, vec!["same"]);
    let presets: Vec<_> = store.iter_kind(kind::PLUGIN_PRESET).collect();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].1.tags, vec!["pad"]);
}

#[test]
fn a_key_without_a_kind_is_refused() {
    let dir = temp_dir("badkey");
    let mut store = MarksStore::open(&dir).unwrap();
    assert!(store.set_favorite("nokind", true).is_err());
    assert!(!store.path().exists());
}

#[test]
fn reload_if_changed_picks_up_another_writer() {
    let dir = temp_dir("reload");
    let mut reader = MarksStore::open(&dir).unwrap();
    assert!(!reader.reload_if_changed().unwrap());
    MarksStore::open(&dir)
        .unwrap()
        .set_favorite("amp-model:x", true)
        .unwrap();
    assert!(reader.reload_if_changed().unwrap());
    assert!(reader.is_favorite("amp-model:x"));
    assert_eq!(reader.generation(), 1);
    assert!(!reader.reload_if_changed().unwrap(), "unchanged file: no reparse");
}

#[test]
fn unknown_fields_survive_a_write_by_this_build() {
    let dir = temp_dir("extra");
    std::fs::write(
        dir.join("marks.json"),
        r#"{"version":1,"generation":7,"future_top":true,
            "items":{"amp-model:x":{"favorite":true,"color":"red"}}}"#,
    )
    .unwrap();
    let mut store = MarksStore::open(&dir).unwrap();
    assert_eq!(store.generation(), 7);
    store.add_tag("amp-model:x", "keep").unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(doc["generation"], 8);
    assert_eq!(doc["future_top"], true);
    assert_eq!(doc["items"]["amp-model:x"]["color"], "red");
    assert_eq!(doc["items"]["amp-model:x"]["tags"], serde_json::json!(["keep"]));
}

#[test]
fn a_corrupt_file_is_quarantined_and_the_store_starts_empty() {
    let dir = temp_dir("corrupt");
    std::fs::write(dir.join("marks.json"), b"{ not json").unwrap();
    let mut store = MarksStore::open(&dir).unwrap();
    assert_eq!(store.iter().count(), 0);
    assert!(
        !dir.join("marks.json.corrupt").exists(),
        "a reader never quarantines: a writer may be mid-install"
    );
    store.set_favorite("amp-model:x", true).unwrap();
    assert!(dir.join("marks.json.corrupt").exists(), "the next write, under the lock, does");
    assert!(MarksStore::open(&dir).unwrap().is_favorite("amp-model:x"));
}

#[test]
fn a_mutation_that_changes_nothing_writes_nothing() {
    let dir = temp_dir("noop");
    let mut store = MarksStore::open(&dir).unwrap();
    // On a fresh dir: no directory entry, no lock file, no marks file.
    store.set_favorite("amp-model:x", false).unwrap();
    store.remove_tag("amp-model:x", "nope").unwrap();
    assert_eq!(
        store.prune_orphans(kind::AMP_MODEL, |_| true, T0).unwrap(),
        0
    );
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing was created");

    store.set_favorite("amp-model:x", true).unwrap();
    let gen = store.generation();
    let before = std::fs::metadata(store.path()).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    store.set_favorite("amp-model:x", true).unwrap();
    store.add_tag("amp-model:x", "").unwrap();
    assert_eq!(store.generation(), gen, "a no-op is not a write");
    assert_eq!(std::fs::metadata(store.path()).unwrap().modified().unwrap(), before);
}

#[test]
fn prune_on_a_missing_library_dir_creates_nothing() {
    let dir = temp_dir("prune-missing").join("not-yet");
    let mut store = MarksStore::open(&dir).unwrap();
    assert_eq!(store.prune_orphans(kind::AMP_MODEL, |_| false, T0).unwrap(), 0);
    assert!(!dir.exists());
}

#[test]
fn a_newer_files_version_is_kept_on_write() {
    let dir = temp_dir("version");
    std::fs::write(dir.join("marks.json"), r#"{"version":7,"generation":1,"items":{}}"#).unwrap();
    let mut store = MarksStore::open(&dir).unwrap();
    store.set_favorite("amp-model:x", true).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_eq!(doc["version"], 7, "an older build must not downgrade a newer file");
}

#[test]
fn a_filesystem_without_locks_is_recognised() {
    use resonance_common::library_marks::lock_error_is_unsupported;
    assert!(lock_error_is_unsupported(&std::io::Error::from(
        std::io::ErrorKind::Unsupported
    )));
    #[cfg(target_os = "linux")]
    assert!(lock_error_is_unsupported(&std::io::Error::from_raw_os_error(37)));
    assert!(!lock_error_is_unsupported(&std::io::Error::from(
        std::io::ErrorKind::PermissionDenied
    )));
}

#[test]
fn two_stores_in_two_threads_lose_no_write() {
    // The lock is what makes this pass: without it about half of the 400
    // writes are lost to interleaved read-modify-writes.
    let dir = temp_dir("threads");
    let writers: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|tag| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut store = MarksStore::open(&dir).unwrap();
                for i in 0..200 {
                    store.set_favorite(&format!("amp-model:{tag}-{i}"), true).unwrap();
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    let store = MarksStore::open(&dir).unwrap();
    assert_eq!(store.iter_kind(kind::AMP_MODEL).count(), 400);
    assert_eq!(store.generation(), 400);
}

#[test]
fn shared_marks_readers_see_writes_and_never_wait_on_them() {
    use resonance_common::library_marks::SharedMarks;
    let dir = temp_dir("shared-threads");
    let shared = std::sync::Arc::new(SharedMarks::open(&dir).unwrap());
    let writer = {
        let shared = shared.clone();
        std::thread::spawn(move || {
            for i in 0..100 {
                shared.set_favorite(&format!("amp-model:{i}"), true).unwrap();
            }
        })
    };
    let mut last = 0;
    while !writer.is_finished() {
        let g = shared.generation();
        assert!(g >= last, "the generation never goes backwards");
        last = g;
        let _ = shared.snapshot().iter().count();
    }
    writer.join().unwrap();
    assert_eq!(shared.generation(), 100);
    assert_eq!(shared.snapshot().iter().count(), 100);
    assert!(shared.is_favorite("amp-model:42"));
}

#[test]
fn orphans_are_kept_for_ninety_days_then_pruned() {
    let dir = temp_dir("orphans");
    let mut store = MarksStore::open(&dir).unwrap();
    store.set_favorite("amp-model:gone", true).unwrap();
    store.set_favorite("amp-model:live", true).unwrap();
    store.set_favorite("plugin-preset:gone", true).unwrap();
    let live = |id: &str| id == "live";

    assert_eq!(store.prune_orphans(kind::AMP_MODEL, live, T0).unwrap(), 0);
    assert_eq!(store.marks("amp-model:gone").orphaned_at, Some(T0));
    assert_eq!(store.marks("amp-model:live").orphaned_at, None);
    assert_eq!(
        store.marks("plugin-preset:gone").orphaned_at,
        None,
        "another kind's items are not this pass's business"
    );

    // A second pass inside the window changes nothing and writes nothing.
    let gen = store.generation();
    let day = 24 * 60 * 60;
    assert_eq!(store.prune_orphans(kind::AMP_MODEL, live, T0 + 89 * day).unwrap(), 0);
    assert_eq!(store.generation(), gen);

    // Re-imported inside the window: the star survives and the stamp clears.
    assert_eq!(
        store
            .prune_orphans(kind::AMP_MODEL, |id| id == "live" || id == "gone", T0 + 89 * day)
            .unwrap(),
        0
    );
    assert_eq!(store.marks("amp-model:gone").orphaned_at, None);
    assert!(store.is_favorite("amp-model:gone"));

    // Gone again, and past the window this time.
    store.prune_orphans(kind::AMP_MODEL, live, T0 + 100 * day).unwrap();
    let removed = store
        .prune_orphans(kind::AMP_MODEL, live, T0 + 100 * day + ORPHAN_RETENTION_SECS)
        .unwrap();
    assert_eq!(removed, 1);
    assert!(store.get("amp-model:gone").is_none());
    assert!(store.is_favorite("amp-model:live"));
    assert!(store.is_favorite("plugin-preset:gone"));
}

#[test]
fn recents_come_only_from_recorded_picks_newest_first() {
    let dir = temp_dir("recent");
    let mut store = MarksStore::open(&dir).unwrap();
    store.record_use("amp-model:a", T0).unwrap();
    store.record_use("amp-model:b", T0 + 10).unwrap();
    store.set_favorite("amp-model:c", true).unwrap();
    store.record_use("plugin-preset:p", T0 + 20).unwrap();
    let recent = store.recent(kind::AMP_MODEL, 10);
    assert_eq!(
        recent,
        vec![("b".to_string(), T0 + 10), ("a".to_string(), T0)],
        "a favourite that was never picked is not recent"
    );
    assert_eq!(store.recent(kind::AMP_MODEL, 1).len(), 1);
}

#[test]
fn tags_normalise_to_lowercase_slugs() {
    assert_eq!(normalize_tag("  Djent Rhythm "), Some("djent-rhythm".into()));
    assert_eq!(normalize_tag("Drum & Bass"), Some("drum-bass".into()));
    assert_eq!(normalize_tag("Café_Crème"), Some("cafe-creme".into()));
    assert_eq!(normalize_tag("!!!"), None);
    let long = "x".repeat(40);
    assert_eq!(normalize_tag(&long).unwrap().len(), library_marks::MAX_TAG_LEN);
    assert_eq!(vocab::slug("Shoegaze"), Some("shoegaze".into()));
}

#[test]
fn shared_marks_serve_readers_and_writers_through_one_lock() {
    use resonance_common::library_marks::SharedMarks;
    let dir = temp_dir("shared");
    let shared = std::sync::Arc::new(SharedMarks::open(&dir).unwrap());
    shared
        .update("plugin-preset:com.x:y", |m| {
            m.favorite = true;
            m.last_used = Some(T0);
        })
        .unwrap();
    let m = shared.marks("plugin-preset:com.x:y");
    assert!(m.favorite);
    assert_eq!(
        m.last_used_rfc3339().as_deref(),
        library_marks::format_timestamp(T0).as_deref(),
        "RFC 3339 for consumers that carry the time as text"
    );
    assert_eq!(shared.generation(), 1);
    assert!(!shared.refresh(), "nothing changed on disk");
    MarksStore::open(&dir)
        .unwrap()
        .add_tag("plugin-preset:com.x:y", "pad")
        .unwrap();
    assert!(shared.refresh(), "the refresh hook sees another writer");
    assert_eq!(shared.marks("plugin-preset:com.x:y").tags, vec!["pad"]);
}

#[test]
fn tag_completion_reads_every_kind_then_the_seeded_vocabulary() {
    let dir = temp_dir("complete");
    let mut store = MarksStore::open(&dir).unwrap();
    store.set_tags("amp-model:a", &["metalcore", "rhythm"]).unwrap();
    store.set_tags("amp-model:b", &["metalcore"]).unwrap();
    store.set_tags("plugin-preset:x", &["mellow"]).unwrap();

    assert_eq!(
        store.tag_counts(),
        vec![
            ("metalcore".to_string(), 2),
            ("mellow".to_string(), 1),
            ("rhythm".to_string(), 1)
        ]
    );
    // Used tags first (most used first), then seeded values; no repeats.
    let got = store.complete_tag("me", &[], 10);
    assert_eq!(&got[..2], &["metalcore".to_string(), "mellow".to_string()]);
    assert!(got.contains(&"metal".to_string()), "seeded genre offered: {got:?}");
    assert!(got.contains(&"metallic".to_string()), "seeded character offered: {got:?}");
    assert_eq!(got.iter().filter(|t| *t == "metalcore").count(), 1);
    // The item's own tags are not offered again.
    let got = store.complete_tag("me", &["metalcore".to_string()], 10);
    assert!(!got.contains(&"metalcore".to_string()));
    // An empty prefix lists used tags only.
    assert_eq!(store.complete_tag("", &[], 10).len(), 3);
    assert_eq!(store.complete_tag("m", &[], 1).len(), 1);
}

#[test]
fn the_seeded_vocabulary_is_slug_clean_and_ranked() {
    for facet in vocab::Facet::ALL {
        assert_eq!(vocab::Facet::from_name(facet.name()), Some(facet));
        for v in facet.seeded() {
            assert_eq!(vocab::slug(v).as_deref(), Some(*v), "{} value {v:?}", facet.name());
        }
    }
    assert_eq!(vocab::Facet::Instrument.seeded_rank("electric-guitar"), Some(4));
    assert_eq!(vocab::Facet::Genres.seeded_rank("shoegaze"), None);
    assert!(vocab::all_seeded().any(|v| v == "post-metal"));
}

#[test]
fn the_freshness_poll_is_rate_limited_and_sees_changes() {
    let dir = temp_dir("fresh-poll");
    let file = dir.join("marks.json");
    let mut poll = FreshnessPoll::new(vec![dir.clone(), file.clone()], Duration::from_millis(500));
    let t = Instant::now();
    assert!(poll.check(t), "the first check reports");
    assert!(!poll.check(t + Duration::from_millis(600)), "nothing moved");
    std::fs::write(&file, b"{}").unwrap();
    assert!(
        !poll.check(t + Duration::from_millis(700)),
        "inside the interval nothing is stat'ed"
    );
    assert!(poll.check(t + Duration::from_millis(1200)));
    std::fs::write(dir.join("other.nam"), b"x").unwrap();
    assert!(poll.force(t + Duration::from_millis(1300)), "a new entry moves the dir");
    std::fs::write(&file, b"{\"a\":1}").unwrap();
    poll.mark_seen(t + Duration::from_millis(1400));
    assert!(!poll.force(t + Duration::from_millis(1500)), "own write marked as seen");
    assert_ne!(fingerprint(&[&file]), fingerprint(&[dir.join("missing")]));
}

// ---------------------------------------------------------------------------
// Two processes
// ---------------------------------------------------------------------------

/// Environment variables that turn `marks_child_writer` into a worker.
const CHILD_DIR: &str = "RESONANCE_MARKS_CHILD_DIR";
const CHILD_TAG: &str = "RESONANCE_MARKS_CHILD_TAG";
const WRITES_PER_CHILD: usize = 150;

/// Wait for the parent's go file, so both children start writing at once.
fn wait_for_go(dir: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.join("go").exists() {
        assert!(Instant::now() < deadline, "the parent never said go");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Not a test on its own: the parent below re-runs this binary with
/// `--ignored --exact marks_child_writer`, and the env vars make it write
/// `WRITES_PER_CHILD` distinct items through its own store.
#[test]
#[ignore = "worker half of two_processes_writing_different_items_lose_nothing"]
fn marks_child_writer() {
    let (Some(dir), Some(tag)) = (std::env::var_os(CHILD_DIR), std::env::var(CHILD_TAG).ok())
    else {
        return;
    };
    let dir = PathBuf::from(dir);
    let marks = dir.join("marks");
    let mut store = MarksStore::open(&marks).unwrap();
    wait_for_go(&dir);
    for i in 0..WRITES_PER_CHILD {
        store
            .set_favorite(&format!("amp-model:{tag}-{i}"), true)
            .unwrap();
    }
}

#[test]
fn two_processes_writing_different_items_lose_nothing() {
    let dir = temp_dir("processes");
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = ["left", "right"]
        .into_iter()
        .map(|tag| {
            Command::new(&exe)
                .args(["--ignored", "--exact", "marks_child_writer", "--test-threads=1"])
                .env(CHILD_DIR, &dir)
                .env(CHILD_TAG, tag)
                .spawn()
                .expect("spawn child test process")
        })
        .collect();
    // Both children are up and parked on the barrier before either writes.
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(dir.join("go"), b"").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success(), "child writer failed");
    }
    let store = MarksStore::open(dir.join("marks")).unwrap();
    let count = store.iter_kind(kind::AMP_MODEL).count();
    assert_eq!(
        count,
        2 * WRITES_PER_CHILD,
        "every item from both processes must survive the interleaved writes"
    );
    assert_eq!(store.generation(), (2 * WRITES_PER_CHILD) as u64);
}

#[test]
fn a_reader_never_sees_a_generation_newer_than_the_snapshot() {
    // A cache keyed on `generation()` reads the number, then the snapshot.
    // If the number were published before the snapshot swap, the reader
    // could pair generation N with the N-1 snapshot and never re-read.
    use resonance_common::library_marks::SharedMarks;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let dir = temp_dir("install-order");
    let shared = Arc::new(SharedMarks::open(&dir).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut torn = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let seen = shared.generation();
                    if shared.snapshot().generation() < seen {
                        torn += 1;
                    }
                }
                torn
            })
        })
        .collect();
    for i in 0..400 {
        shared.set_favorite(&format!("amp-model:{i}"), true).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    let torn: u64 = readers.into_iter().map(|r| r.join().unwrap()).sum();
    assert_eq!(torn, 0, "a reader paired a new generation with an older snapshot");
}
