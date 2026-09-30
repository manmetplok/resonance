//! `PresetLibrary`: the in-memory index, query engine, freshness, trash
//! and the legacy converter (plugin-preset-library.md §4.6, §6.4, §13;
//! slice P0).
//!
//! Every library here has a private temporary root, and the ones that
//! care about time get an injected clock.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use resonance_plugin::presets::migrate::{convert_legacy_dir, LEGACY_RETENTION, LEGACY_SUFFIX};
use resonance_plugin::presets::{
    mark_key, FactoryEntry, FactoryPreset, MarksSource, PresetBank, PresetFile, PresetLibrary,
    PresetMeta, PresetRef, PresetSource, Query, SaveOptions, Sort, TRASH_RETENTION,
};
use resonance_plugin::library_marks::{Marks, SharedMarks};
use resonance_plugin::{FloatParam, FloatRange, Param};

const PLUGIN: &str = "com.resonance.test";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "resonance-preset-library-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn dir(&self) -> PathBuf {
        self.0.join(PLUGIN)
    }

    fn library(&self) -> Arc<PresetLibrary> {
        Arc::new(PresetLibrary::new().with_root(self.0.clone()))
    }

    /// Write a raw file into the plugin directory, as a user or an older
    /// build would have.
    fn drop_file(&self, name: &str, body: &str) -> PathBuf {
        std::fs::create_dir_all(self.dir()).unwrap();
        let path = self.dir().join(name);
        std::fs::write(&path, body).unwrap();
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// A settable clock.
fn clock_at(secs: u64) -> (Arc<AtomicU64>, resonance_plugin::presets::Clock) {
    let now = Arc::new(AtomicU64::new(secs));
    let read = now.clone();
    (
        now,
        Arc::new(move || UNIX_EPOCH + Duration::from_secs(read.load(Ordering::Relaxed))),
    )
}

fn mix() -> FloatParam {
    FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
}

// ---------------------------------------------------------------------------
// A library to query: factory + user presets with metadata
// ---------------------------------------------------------------------------

fn factory_file(id: &str, name: &str, meta: serde_json::Value) -> String {
    let mut meta = meta;
    meta["name"] = serde_json::Value::String(name.to_string());
    serde_json::json!({
        "format": "resonance.preset", "format_version": 1, "id": id,
        "plugin": {"id": PLUGIN}, "meta": meta,
        "state": {"encoding": "resonance-json", "doc": {"params": {"mix": 0.5}}},
    })
    .to_string()
}

fn seeded(root: &TempRoot) -> (Arc<PresetLibrary>, PresetBank) {
    let library = root.library();
    library.register_factory_entries(
        PLUGIN,
        vec![
            FactoryEntry {
                id: "bass-reese".into(),
                name: "Bass — Reese".into(),
                json: factory_file(
                    "bass-reese",
                    "Bass — Reese",
                    serde_json::json!({"category": "Bass", "genres": ["drum-and-bass"],
                        "character": ["dark", "wide"], "tags": ["reese"],
                        "instrument": ["synth-bass"], "description": "Detuned saws."}),
                ),
            },
            FactoryEntry {
                id: "pad-cafe".into(),
                name: "Pad — Café Glow".into(),
                json: factory_file(
                    "pad-cafe",
                    "Pad — Café Glow",
                    serde_json::json!({"category": "Pad", "genres": ["ambient"],
                        "character": ["warm", "wide"], "author": "Jorrit Smith"}),
                ),
            },
            FactoryEntry {
                id: "lead-acid".into(),
                name: "Lead — Acid".into(),
                json: factory_file(
                    "lead-acid",
                    "Lead — Acid",
                    serde_json::json!({"category": "Lead", "genres": ["techno", "house"],
                        "character": ["gritty"], "tags": ["303"],
                        "description": "A reese-ish squelch."}),
                ),
            },
        ],
    );
    let bank = PresetBank::new(PLUGIN, &[]).with_library(library.clone());
    let m = mix();
    bank.save_with(
        "Ferrous Reese",
        &[&m as &dyn Param],
        SaveOptions {
            meta: Some(PresetMeta {
                category: Some("bass".into()),
                genres: vec!["Industrial".into()],
                character: vec!["dark".into()],
                tags: vec!["ferrous".into()],
                ..PresetMeta::default()
            }),
            derived_from: None,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    (library, bank)
}

fn hit_names(library: &PresetLibrary, q: &Query) -> Vec<String> {
    library
        .query(q)
        .hits
        .into_iter()
        .map(|h| h.record.meta.name)
        .collect()
}

fn q(text: &str) -> Query {
    Query {
        text: text.to_string(),
        ..Query::plugin(PLUGIN)
    }
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[test]
fn an_empty_query_lists_everything_in_bank_order() {
    let root = TempRoot::new("bank-order");
    let (library, _) = seeded(&root);
    assert_eq!(
        hit_names(&library, &q("")),
        vec!["Bass — Reese", "Pad — Café Glow", "Lead — Acid", "Ferrous Reese"]
    );
}

#[test]
fn text_tokens_are_substrings_anded_in_bank_order() {
    let root = TempRoot::new("text");
    let (library, _) = seeded(&root);
    // One engine (library_view::BrowserModel): substring matching over
    // name, author, description, category and tags, in bank order.
    // "Reese" is in two names and one description.
    assert_eq!(
        hit_names(&library, &q("rees")),
        vec!["Bass — Reese", "Lead — Acid", "Ferrous Reese"]
    );
    assert_eq!(hit_names(&library, &q("eese")).len(), 3, "substring, not prefix");
    // Tokens AND: both must match somewhere.
    assert_eq!(hit_names(&library, &q("reese ferr")), vec!["Ferrous Reese"]);
    // Tags and category are searched too.
    assert_eq!(hit_names(&library, &q("303")), vec!["Lead — Acid"]);
    assert_eq!(hit_names(&library, &q("pad")), vec!["Pad — Café Glow"]);
    assert!(hit_names(&library, &q("zzz")).is_empty());
}

#[test]
fn text_search_ignores_case_and_accents() {
    let root = TempRoot::new("accents");
    let (library, _) = seeded(&root);
    assert_eq!(hit_names(&library, &q("CAFE")), vec!["Pad — Café Glow"]);
    assert_eq!(hit_names(&library, &q("café")), vec!["Pad — Café Glow"]);
}

#[test]
fn scoped_tokens_filter_on_their_own_field() {
    let root = TempRoot::new("scoped");
    let (library, _) = seeded(&root);
    assert_eq!(hit_names(&library, &q("genre:industrial")), vec!["Ferrous Reese"]);
    assert_eq!(hit_names(&library, &q("tag:reese")), vec!["Bass — Reese"]);
    assert_eq!(hit_names(&library, &q("by:jorrit")), vec!["Pad — Café Glow"]);
    assert_eq!(
        hit_names(&library, &q("cat:bass")),
        vec!["Bass — Reese", "Ferrous Reese"]
    );
    assert_eq!(hit_names(&library, &q("char:dark is:user")), vec!["Ferrous Reese"]);
    assert_eq!(hit_names(&library, &q("for:synth-bass")), vec!["Bass — Reese"]);
    // An unknown scope is plain text.
    assert!(hit_names(&library, &q("colour:red")).is_empty());
}

#[test]
fn facets_or_within_and_across_with_counts_from_the_other_facets() {
    let root = TempRoot::new("facets");
    let (library, _) = seeded(&root);

    let query = Query {
        character: vec!["dark".into(), "warm".into()],
        ..Query::plugin(PLUGIN)
    };
    let result = library.query(&query);
    let names: Vec<&str> = result.hits.iter().map(|h| h.record.meta.name.as_str()).collect();
    assert_eq!(names, vec!["Bass — Reese", "Pad — Café Glow", "Ferrous Reese"]);

    // The character facet counts ignore the character selection itself…
    let count = |list: &[(String, usize)], v: &str| {
        list.iter().find(|(k, _)| k == v).map(|(_, n)| *n)
    };
    assert_eq!(count(&result.facets.character, "gritty"), Some(1));
    assert_eq!(count(&result.facets.character, "wide"), Some(2));
    // …while other facets are counted on the filtered set.
    assert_eq!(count(&result.facets.category, "Bass"), Some(2));
    assert_eq!(count(&result.facets.category, "Lead"), None);
    assert_eq!(count(&result.facets.source, "user"), Some(1));

    // AND across facets.
    let query = Query {
        character: vec!["dark".into(), "warm".into()],
        category: vec!["Pad".into()],
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(hit_names(&library, &query), vec!["Pad — Café Glow"]);

    // A selected value with no matches stays listed at zero.
    let query = Query {
        genres: vec!["jazz".into()],
        ..Query::plugin(PLUGIN)
    };
    let result = library.query(&query);
    assert!(result.hits.is_empty());
    assert_eq!(count(&result.facets.genres, "jazz"), Some(0));
}

#[test]
fn sources_filter_and_sorts_order() {
    let root = TempRoot::new("sorts");
    let (library, _) = seeded(&root);
    let query = Query {
        sources: vec![PresetSource::Factory],
        sort: Sort::Name,
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(
        hit_names(&library, &query),
        vec!["Bass — Reese", "Lead — Acid", "Pad — Café Glow"]
    );
    let query = Query {
        sort: Sort::Category,
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(
        hit_names(&library, &query),
        vec!["Bass — Reese", "Ferrous Reese", "Lead — Acid", "Pad — Café Glow"]
    );
    // Only the user preset has a `modified` stamp, so it leads.
    let query = Query {
        sort: Sort::RecentlyModified,
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(hit_names(&library, &query)[0], "Ferrous Reese");
}

/// Marks come from whatever store is installed — the integration seam
/// round 2 fills with `library_marks`.
#[test]
fn favourites_and_personal_tags_come_from_the_marks_source() {
    struct Fake;
    impl MarksSource for Fake {
        fn marks(&self, key: &str) -> Marks {
            if key == mark_key(PLUGIN, "lead-acid") {
                Marks {
                    favorite: true,
                    tags: vec!["mine".into()],
                    last_used: Some(1_790_000_000),
                    use_count: 3,
                    ..Marks::default()
                }
            } else {
                Marks::default()
            }
        }
    }
    let root = TempRoot::new("marks");
    let (library, _) = seeded(&root);
    library.set_marks(Arc::new(Fake));

    let favs = Query {
        favorites_only: true,
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(hit_names(&library, &favs), vec!["Lead — Acid"]);
    assert_eq!(hit_names(&library, &q("is:fav")), vec!["Lead — Acid"]);
    // Personal tags merge with content tags, on a factory preset too.
    assert_eq!(hit_names(&library, &q("tag:mine")), vec!["Lead — Acid"]);
    let hit = library.query(&q("tag:mine")).hits.remove(0);
    assert_eq!(hit.tags, vec!["303", "mine"]);
    assert_eq!(hit.personal_tags, vec!["mine"]);
    assert_eq!(
        hit.last_used.as_deref(),
        Some("2026-09-21T14:13:20Z"),
        "last_used crosses to presets as RFC 3339"
    );

    let first = Query {
        favorites_first: true,
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(hit_names(&library, &first)[0], "Lead — Acid");
    let recent = Query {
        sort: Sort::RecentlyUsed,
        ..Query::plugin(PLUGIN)
    };
    assert_eq!(hit_names(&library, &recent)[0], "Lead — Acid");
}

/// Convergence (9): the shared `SharedMarks` store is a marks source.
/// Stars and personal tags written through the library land in
/// `marks.json` under `plugin-preset:<clap>:<id>`, a factory preset
/// included, and a write from another process is picked up by the next
/// query (the refresh hook).
#[test]
fn the_shared_marks_store_backs_the_library() {
    let root = TempRoot::new("shared-marks");
    let marks_dir = root.0.join("library");
    let (library, _) = seeded(&root);
    library.set_marks(Arc::new(SharedMarks::open(&marks_dir).unwrap()));

    library.set_favorite(PLUGIN, "bass-reese", true).unwrap();
    library
        .set_personal_tags(PLUGIN, "bass-reese", &["Mine Too".to_string()])
        .unwrap();
    let hit = library.query(&q("is:fav")).hits.remove(0);
    assert_eq!(hit.record.preset.id, "bass-reese");
    assert_eq!(hit.personal_tags, vec!["mine-too"]);
    let text = std::fs::read_to_string(marks_dir.join("marks.json")).unwrap();
    assert!(text.contains("plugin-preset:com.resonance.test:bass-reese"), "{text}");

    // Another process stars a second preset.
    let other = SharedMarks::open(&marks_dir).unwrap();
    other
        .set_favorite("plugin-preset:com.resonance.test:lead-acid", true)
        .unwrap();
    let before = library.marks().generation();
    assert_eq!(hit_names(&library, &q("is:fav")).len(), 2, "refreshed before the query");
    assert!(library.marks().generation() > before);

    library.record_use(PLUGIN, "lead-acid").unwrap();
    let recent = Query {
        sort: Sort::RecentlyUsed,
        ..Query::plugin(PLUGIN)
    };
    let first = library.query(&recent).hits.remove(0);
    assert_eq!(first.record.preset.id, "lead-acid");
    assert!(first.last_used.is_some());
}

#[test]
fn saved_metadata_is_normalised() {
    let root = TempRoot::new("normalise");
    let (_, bank) = seeded(&root);
    let record = bank
        .records()
        .iter()
        .find(|r| r.meta.name == "Ferrous Reese")
        .cloned()
        .unwrap();
    assert_eq!(record.meta.category.as_deref(), Some("Bass"), "vocabulary case");
    assert_eq!(record.meta.genres, vec!["industrial"], "slugged");
}

// ---------------------------------------------------------------------------
// Freshness
// ---------------------------------------------------------------------------

/// The bar reads the index; only a fingerprint check older than its poll
/// interval touches the directory again. So a file written by someone else
/// appears on the next explicit read, and not on a cached one.
#[test]
fn a_cached_read_does_not_rescan_until_its_interval_passes() {
    let root = TempRoot::new("cached");
    let library = root.library();
    let bank = PresetBank::new(PLUGIN, &[]).with_library(library.clone());
    assert!(bank.records_cached(Duration::from_secs(3600)).is_empty());

    // Another process saves a preset.
    let other = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    let m = mix();
    other.save("Elsewhere", &[&m as &dyn Param]).unwrap();

    assert!(
        bank.records_cached(Duration::from_secs(3600)).is_empty(),
        "a cached read within its interval must not touch the disk"
    );
    assert_eq!(bank.records().len(), 1, "an explicit read checks now");
    assert_eq!(bank.records_cached(Duration::from_secs(3600)).len(), 1);
}

/// Our own writes update the index at once, whatever the interval.
#[test]
fn own_writes_are_visible_immediately() {
    let root = TempRoot::new("own-writes");
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert!(bank.records_cached(Duration::from_secs(3600)).is_empty());
    let m = mix();
    let saved = bank.save("Mine", &[&m as &dyn Param]).unwrap();
    assert_eq!(bank.records_cached(Duration::from_secs(3600)).len(), 1);
    bank.delete(&saved).unwrap();
    assert!(bank.records_cached(Duration::from_secs(3600)).is_empty());
}

// ---------------------------------------------------------------------------
// Files on disk
// ---------------------------------------------------------------------------

/// §4.2: a hand-dropped format-1 file without an id gets one written into
/// it on the first scan, and keeps it.
#[test]
fn a_file_without_an_id_gets_one() {
    let root = TempRoot::new("mint");
    let path = root.drop_file(
        "hand.json",
        r#"{"format":"resonance.preset","format_version":1,
            "meta":{"name":"Hand Made"},
            "state":{"encoding":"resonance-json","doc":{"params":{"mix":0.1}}}}"#,
    );
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    let listed = bank.list_user();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].is_resolved());
    // The file moves to the `<name>-<id8>.json` convention with its id in it.
    assert!(!path.exists());
    let now_at = bank.record(&listed[0]).unwrap().path.unwrap();
    let stem = now_at.file_name().unwrap().to_string_lossy().into_owned();
    assert!(stem.starts_with("Hand_Made-"), "{stem}");
    let file = PresetFile::parse(&std::fs::read_to_string(&now_at).unwrap()).unwrap();
    assert_eq!(file.id, listed[0].id, "the id is persisted in the file");

    let again = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert_eq!(again.list_user()[0].id, listed[0].id, "and stable across scans");
}

#[test]
fn an_unparsable_file_is_quarantined_not_listed() {
    let root = TempRoot::new("corrupt");
    root.drop_file("torn.json", "{\"format\": \"resonance.pre");
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert!(bank.list_user().is_empty());
    assert_eq!(files_in(&root.dir()), vec!["torn.json.corrupt"]);
}

/// Two files with one id (a rename caught half-way) list once.
#[test]
fn duplicate_ids_list_once() {
    let root = TempRoot::new("dup");
    let body = r#"{"format":"resonance.preset","format_version":1,
        "id":"3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42",
        "meta":{"name":"Twice"},
        "state":{"encoding":"resonance-json","doc":{"params":{}}}}"#;
    root.drop_file("a.json", body);
    root.drop_file("b.json", body);
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert_eq!(bank.list_user().len(), 1);
}

fn user_file(id: &str, name: &str, mix: f64) -> String {
    serde_json::json!({
        "format": "resonance.preset", "format_version": 1, "id": id,
        "meta": {"name": name},
        "state": {"encoding": "resonance-json", "doc": {"params": {"mix": mix}}},
    })
    .to_string()
}

fn id_in(path: &Path) -> String {
    PresetFile::parse(&std::fs::read_to_string(path).unwrap())
        .unwrap()
        .id
}

/// Review fix 4: a hand-copied preset (same id, then edited) is a second
/// preset, not a duplicate to hide. The older file gets a fresh id.
#[test]
fn a_hand_copied_preset_with_a_different_sound_gets_its_own_id() {
    let root = TempRoot::new("hand-copy");
    let id = "3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42";
    let original = root.drop_file("orig.json", &user_file(id, "Take", 0.1));
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&original)
        .unwrap()
        .set_modified(old)
        .unwrap();
    root.drop_file("copy.json", &user_file(id, "Take copy", 0.9));

    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    let listed = bank.list_user();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert_ne!(listed[0].id, listed[1].id);
    let copy = listed.iter().find(|p| p.name == "Take copy").unwrap();
    assert_eq!(copy.id, id, "the newer file keeps the id");
    let take = listed.iter().find(|p| p.name == "Take").unwrap();
    let take_path = bank.record(take).unwrap().path.unwrap();
    assert_eq!(id_in(&take_path), take.id, "the new id is written into the file");
}

/// Review fix 5: a user preset never shares a marks key with a factory
/// preset. A factory file copied into the user directory carries the
/// factory slug; so does anything with a non-UUID id. Both are re-minted.
#[test]
fn a_user_preset_never_carries_a_factory_or_non_uuid_id() {
    const BANK: &[FactoryPreset] = &[FactoryPreset {
        id: "tight-room",
        name: "Tight Room",
        json: r#"{"params":{"mix":0.9}}"#,
    }];
    let root = TempRoot::new("slug-copy");
    root.drop_file("tight.json", &user_file("tight-room", "Tight Room", 0.9));
    root.drop_file("mine.json", &user_file("my-own-id", "Mine", 0.2));
    let bank = PresetBank::new(PLUGIN, BANK).with_library(root.library());
    let users = bank.list_user();
    assert_eq!(users.len(), 2);
    for u in &users {
        assert!(resonance_plugin::presets::format::is_uuid(&u.id), "{u:?}");
        let path = bank.record(u).unwrap().path.unwrap();
        assert_eq!(id_in(&path), u.id);
    }
    assert_eq!(bank.list().len(), 3, "the factory preset is still listed once");
}

/// Review fix 8: a file with an id but no name is listed under its stem,
/// not hidden.
#[test]
fn a_file_with_an_id_but_no_name_is_listed_by_its_stem() {
    let root = TempRoot::new("no-name");
    root.drop_file(
        "Nameless.json",
        &user_file("3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42", "", 0.3),
    );
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    let names: Vec<String> = bank.list_user().into_iter().map(|p| p.name).collect();
    assert_eq!(names, vec!["Nameless"]);
}

/// Review fix 14: a file written by a newer build is skipped and left
/// alone — never quarantined as corrupt.
#[test]
fn a_newer_format_version_is_skipped_not_quarantined() {
    let root = TempRoot::new("newer");
    root.drop_file(
        "future.json",
        r#"{"format":"resonance.preset","format_version":2,"id":"x",
            "entirely":"different"}"#,
    );
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert!(bank.list_user().is_empty());
    assert_eq!(files_in(&root.dir()), vec!["future.json"]);
}

// ---------------------------------------------------------------------------
// Trash (D10)
// ---------------------------------------------------------------------------

#[test]
fn trash_older_than_thirty_days_is_purged_when_the_library_opens() {
    let root = TempRoot::new("purge");
    let start = 1_800_000_000;
    let (now, clock) = clock_at(start);
    let library = Arc::new(
        PresetLibrary::new()
            .with_root(root.0.clone())
            .with_clock(clock.clone()),
    );
    let bank = PresetBank::new(PLUGIN, &[]).with_library(library.clone());
    let m = mix();
    let old = bank.save("Old", &[&m as &dyn Param]).unwrap();
    let old_trash = bank.trash(&old).unwrap();

    now.store(start + TRASH_RETENTION.as_secs() - 60, Ordering::Relaxed);
    let young = bank.save("Young", &[&m as &dyn Param]).unwrap();
    let young_trash = bank.trash(&young).unwrap();

    // A day past Old's retention, a new process opens the library.
    now.store(start + TRASH_RETENTION.as_secs() + 86_400, Ordering::Relaxed);
    let reopened = Arc::new(
        PresetLibrary::new()
            .with_root(root.0.clone())
            .with_clock(clock),
    );
    PresetBank::new(PLUGIN, &[]).with_library(reopened).list();
    assert!(!old_trash.exists(), "past retention: purged");
    assert!(young_trash.exists(), "within retention: kept");
}

// ---------------------------------------------------------------------------
// The legacy converter (§13)
// ---------------------------------------------------------------------------

/// The shapes found on a real machine before format 1: an editor save
/// (params + name), a host save (the full state blob, `"preset"` session
/// key included), and a hand-dropped file with no name at all.
#[test]
fn legacy_files_convert_to_format_1() {
    let root = TempRoot::new("legacy");
    root.drop_file(
        "Bass___Reese.json",
        r#"{"version":1,"params":{"mix":0.8},"name":"Bass — Reese"}"#,
    );
    root.drop_file(
        "From_Host.json",
        r#"{"version":1,"params":{"mix":0.2},"model_path":"/m.nam",
            "preset":{"name":"Other","source":"factory","modified":true},"name":"From Host"}"#,
    );
    root.drop_file("dropped.json", r#"{"params":{"mix":0.4}}"#);

    let report = convert_legacy_dir(&root.dir(), PLUGIN, SystemTime::now());
    assert_eq!(report.converted.len(), 3, "{report:?}");
    // Originals are kept beside the new files until the next start.
    let files = files_in(&root.dir());
    assert_eq!(files.iter().filter(|f| f.ends_with(LEGACY_SUFFIX)).count(), 3, "{files:?}");

    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    let records = bank.records();
    let by_name = |n: &str| records.iter().find(|r| r.meta.name == n).cloned().unwrap();

    let reese = by_name("Bass — Reese");
    assert_eq!(reese.meta.category.as_deref(), Some("Bass"), "parsed from the name");
    assert!(reese.preset.is_resolved());
    let stem = reese.path.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert!(stem.starts_with("Bass___Reese-"), "{stem}");

    let host = by_name("From Host");
    let doc: serde_json::Value = serde_json::from_str(&bank.json_for(&host.preset).unwrap()).unwrap();
    assert!(doc.get("preset").is_none(), "the session key is stripped: {doc}");
    assert!(doc.get("name").is_none(), "the name moved into meta: {doc}");
    assert_eq!(doc["model_path"], "/m.nam", "extra state survives");
    assert_eq!(host.plugin_version, None, "unknown for legacy files");

    let dropped = by_name("dropped");
    let m = mix();
    assert!(bank.apply(&dropped.preset, &[&m as &dyn Param]));
    assert!((m.get_plain() - 0.4).abs() < 1e-6);

    // Opening the library again soon after keeps them: they are purged
    // by age, not by the next start (review fix 6).
    let backups = files_in(&root.dir())
        .iter()
        .filter(|f| f.ends_with(LEGACY_SUFFIX))
        .count();
    assert_eq!(backups, 3);
}

/// Review fix 6: `.legacy` backups are purged by age (30 days from the
/// conversion), not by whichever library opens the directory next.
#[test]
fn the_converter_is_idempotent_and_purges_old_backups_by_age() {
    let root = TempRoot::new("idempotent");
    root.drop_file("Take.json", r#"{"params":{"mix":0.8},"name":"Take"}"#);
    let now = SystemTime::now();
    let first = convert_legacy_dir(&root.dir(), PLUGIN, now);
    assert_eq!(first.converted.len(), 1);
    let converted = files_in(&root.dir());
    assert_eq!(converted.len(), 2, "{converted:?}");

    let second = convert_legacy_dir(&root.dir(), PLUGIN, now + Duration::from_secs(60));
    assert!(second.converted.is_empty(), "format-1 files are skipped");
    assert_eq!(second.backups_removed, 0, "a fresh backup is kept");
    assert_eq!(files_in(&root.dir()), converted);

    let later = now + LEGACY_RETENTION + Duration::from_secs(86_400);
    let third = convert_legacy_dir(&root.dir(), PLUGIN, later);
    assert_eq!(third.backups_removed, 1, "past retention the backup goes");
    let after: Vec<String> = files_in(&root.dir());
    assert_eq!(after.len(), 1);
    assert!(converted.contains(&after[0]));
}

/// Review fix 7: the legacy id is derived from the plugin, file name and
/// bytes, so two converters racing on one file write one preset, not two.
#[test]
fn racing_converters_produce_one_preset() {
    let root = TempRoot::new("race");
    let body = r#"{"params":{"mix":0.8},"name":"Take"}"#;
    root.drop_file("Take.json", body);
    convert_legacy_dir(&root.dir(), PLUGIN, SystemTime::now());
    // The second process read the original before the first moved it.
    root.drop_file("Take.json", body);
    convert_legacy_dir(&root.dir(), PLUGIN, SystemTime::now());

    let json: Vec<String> = files_in(&root.dir())
        .into_iter()
        .filter(|f| f.ends_with(".json"))
        .collect();
    assert_eq!(json.len(), 1, "{json:?}");

    // The same file in another directory converts to the same id.
    let other = TempRoot::new("race-other");
    other.drop_file("Take.json", body);
    convert_legacy_dir(&other.dir(), PLUGIN, SystemTime::now());
    let a = id_in(&root.dir().join(&json[0]));
    let other_json: Vec<String> = files_in(&other.dir())
        .into_iter()
        .filter(|f| f.ends_with(".json"))
        .collect();
    assert_eq!(id_in(&other.dir().join(&other_json[0])), a);
    assert!(resonance_plugin::presets::format::is_uuid(&a));
}

/// The library runs the converter on its own the first time it indexes a
/// directory, so a plugin never sees a legacy file.
#[test]
fn the_library_converts_on_first_index() {
    let root = TempRoot::new("auto");
    root.drop_file("Old.json", r#"{"params":{"mix":0.8},"name":"Old One"}"#);
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    let listed = bank.list_user();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Old One");
    assert!(listed[0].is_resolved());
}

/// A stray legacy file dropped in later is converted by the next scan.
#[test]
fn a_legacy_file_dropped_in_later_is_converted_too() {
    let root = TempRoot::new("late");
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert!(bank.list_user().is_empty());
    root.drop_file("late.json", r#"{"params":{"mix":0.3},"name":"Late"}"#);
    assert_eq!(bank.list_user().len(), 1);
    assert!(files_in(&root.dir()).iter().any(|f| f.starts_with("Late-")));
}

/// Factory registration from a static bank reads ids and metadata.
#[test]
fn a_static_factory_bank_registers_with_its_ids() {
    const BANK: &[FactoryPreset] = &[FactoryPreset {
        id: "only",
        name: "Only",
        json: r#"{"params":{"mix":0.9}}"#,
    }];
    let root = TempRoot::new("static");
    let bank = PresetBank::new(PLUGIN, BANK).with_library(root.library());
    assert_eq!(bank.list(), vec![PresetRef::factory("only", "Only")]);
}
