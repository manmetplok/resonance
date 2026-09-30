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

use resonance_plugin::presets::migrate::{convert_legacy_dir, LEGACY_SUFFIX};
use resonance_plugin::presets::{
    mark_key, FactoryEntry, FactoryPreset, MarksSource, PresetBank, PresetFile, PresetLibrary,
    PresetMarks, PresetMeta, PresetRef, PresetSource, Query, SaveOptions, Sort, TRASH_RETENTION,
};
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
fn text_tokens_are_prefixes_anded_and_name_hits_rank_first() {
    let root = TempRoot::new("text");
    let (library, _) = seeded(&root);
    // "Reese" is in two names and one description; name hits come first,
    // bank order within each group.
    assert_eq!(
        hit_names(&library, &q("rees")),
        vec!["Bass — Reese", "Ferrous Reese", "Lead — Acid"]
    );
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
        fn marks(&self, key: &str) -> PresetMarks {
            if key == mark_key(PLUGIN, "lead-acid") {
                PresetMarks {
                    favorite: true,
                    tags: vec!["mine".into()],
                    last_used: Some("2026-09-30T10:00:00Z".into()),
                    use_count: 3,
                }
            } else {
                PresetMarks::default()
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
    let file = PresetFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
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
    let body = r#"{"format":"resonance.preset","format_version":1,"id":"same-id",
        "meta":{"name":"Twice"},
        "state":{"encoding":"resonance-json","doc":{"params":{}}}}"#;
    root.drop_file("a.json", body);
    root.drop_file("b.json", body);
    let bank = PresetBank::new(PLUGIN, &[]).with_library(root.library());
    assert_eq!(bank.list_user().len(), 1);
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

    // Opening the library is the next start: the backups go.
    assert!(!files_in(&root.dir()).iter().any(|f| f.ends_with(LEGACY_SUFFIX)));
}

#[test]
fn the_converter_is_idempotent_and_clears_old_backups() {
    let root = TempRoot::new("idempotent");
    root.drop_file("Take.json", r#"{"params":{"mix":0.8},"name":"Take"}"#);
    let first = convert_legacy_dir(&root.dir(), PLUGIN, SystemTime::now());
    assert_eq!(first.converted.len(), 1);
    let converted = files_in(&root.dir());

    let second = convert_legacy_dir(&root.dir(), PLUGIN, SystemTime::now());
    assert!(second.converted.is_empty(), "format-1 files are skipped");
    assert_eq!(second.backups_removed, 1, "last start's .legacy goes");
    let after: Vec<String> = files_in(&root.dir());
    assert_eq!(after.len(), 1);
    assert!(converted.contains(&after[0]));
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
