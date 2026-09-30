//! The preset system as a plugin sees it: the user-preset directory
//! (save / rename / delete through `PresetBank`), the merged
//! factory+user listing, stable ids, and the loaded-preset identity that
//! has to survive closing the window (ba todo #1332, audit findings X1
//! and X2; plugin-preset-library.md P0).
//!
//! The index, query, trash and legacy converter are covered in
//! `tests/preset_library.rs`.
//!
//! Every test points its bank at a private temporary root through
//! `PresetBank::with_root`, so nothing here reads or writes the real
//! `~/.local/share/resonance/plugin-presets`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use resonance_plugin::presets::{
    FactoryPreset, NamingKind, PresetBank, PresetEditor, PresetEvent, PresetFile, PresetRef,
    PresetSession, PresetSource,
};
use resonance_plugin::{
    BoolParam, EventIterator, ExtraStateSaver, FloatParam, FloatRange, IntParam, IntRange,
    OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// A small plugin surface to drive the preset system with
// ---------------------------------------------------------------------------

struct TestParams {
    mix: FloatParam,
    taps: IntParam,
    freeze: BoolParam,
}

impl TestParams {
    fn new() -> Self {
        Self {
            mix: FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            taps: IntParam::new("taps", "Taps", 3, IntRange::Linear { min: 1, max: 8 }),
            freeze: BoolParam::new("freeze", "Freeze", false),
        }
    }

    fn refs(&self) -> Vec<&dyn Param> {
        vec![&self.mix, &self.taps, &self.freeze]
    }
}

/// One bare state document (the pre-format-1 factory shape, still
/// accepted) and one format-1 file with metadata.
const FACTORY: &[FactoryPreset] = &[
    FactoryPreset {
        id: "init",
        name: "Init",
        json: r#"{"params":{"mix":0.5,"taps":3.0,"freeze":0.0}}"#,
    },
    FactoryPreset {
        id: "wide",
        name: "Wide",
        json: r#"{"format":"resonance.preset","format_version":1,"id":"wide",
            "plugin":{"id":"com.resonance.test"},
            "meta":{"name":"Wide","category":"Creative","character":["wide"],"tags":["spread"]},
            "state":{"encoding":"resonance-json",
                     "doc":{"params":{"mix":0.9,"taps":7.0,"freeze":1.0}}}}"#,
    },
];

fn init() -> PresetRef {
    PresetRef::factory("init", "Init")
}

fn wide() -> PresetRef {
    PresetRef::factory("wide", "Wide")
}

/// A name-only reference, resolved against the bank by name — what a
/// project saved before preset ids carries.
fn user(name: &str) -> PresetRef {
    PresetRef::unresolved(PresetSource::User, name)
}

fn names(list: &[PresetRef]) -> Vec<String> {
    list.iter().map(|p| p.name.clone()).collect()
}

/// Every `.json` file directly in `dir`.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().map(|e| e == "json").unwrap_or(false));
    files.sort();
    files
}

/// A temporary preset root unique to the calling test.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "resonance-preset-test-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn bank(&self) -> PresetBank {
        PresetBank::new("com.resonance.test", FACTORY)
            .with_root(self.0.clone())
            .with_plugin_info("Test Plugin", "9.8.7")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

#[test]
fn saving_writes_a_format_1_file_into_the_plugins_own_directory() {
    let root = TempRoot::new("dir");
    let bank = root.bank();
    let params = TestParams::new();

    let saved = bank.save("My Sound", &params.refs()).expect("save failed");
    assert_eq!(saved.name, "My Sound");
    assert_eq!(saved.source, PresetSource::User);
    assert_eq!(saved.id.len(), 36, "a hyphenated UUID: {}", saved.id);
    assert_eq!(&saved.id[14..15], "4", "UUID version 4: {}", saved.id);

    let dir = bank.user_dir().expect("a bank with a root always has a dir");
    assert!(dir.ends_with("com.resonance.test"), "{dir:?}");
    let files = json_files(&dir);
    assert_eq!(files.len(), 1);
    let stem_expected = format!("My_Sound-{}.json", &saved.id[..8]);
    assert_eq!(files[0].file_name().unwrap().to_string_lossy(), stem_expected);

    let file = PresetFile::parse(&std::fs::read_to_string(&files[0]).unwrap()).unwrap();
    assert_eq!(file.format, "resonance.preset");
    assert_eq!(file.format_version, 1);
    assert_eq!(file.id, saved.id);
    assert_eq!(file.plugin.id, "com.resonance.test");
    assert_eq!(file.plugin.name.as_deref(), Some("Test Plugin"));
    assert_eq!(file.plugin.version.as_deref(), Some("9.8.7"));
    assert_eq!(file.meta.name, "My Sound");
    assert!(file.meta.created.is_some() && file.meta.modified.is_some());
    assert_eq!(file.state.encoding, "resonance-json");
}

#[test]
fn a_saved_preset_is_a_full_snapshot_of_every_param() {
    let root = TempRoot::new("snapshot");
    let bank = root.bank();

    // Only one parameter is moved off its default; the snapshot must
    // still carry all three, or recalling it would leave whatever the
    // previous patch had in the other two (finding P7).
    let params = TestParams::new();
    params.mix.set_plain(0.2);
    let saved = bank.save("Partial?", &params.refs()).expect("save failed");

    let value: serde_json::Value =
        serde_json::from_str(&bank.json_for(&saved).expect("the state document")).unwrap();
    let map = value.get("params").and_then(|v| v.as_object()).unwrap();
    assert_eq!(map.len(), params.refs().len(), "{map:?}");
    for p in params.refs() {
        assert!(map.contains_key(p.id()), "missing param '{}'", p.id());
    }
    // …and the document is versioned like project state is.
    assert_eq!(
        value.get("version").and_then(|v| v.as_u64()),
        Some(resonance_plugin::STATE_VERSION as u64)
    );

    let target = TestParams::new();
    target.mix.set_plain(1.0);
    target.taps.set_plain(8.0);
    target.freeze.set_plain(1.0);
    assert!(bank.apply(&user("Partial?"), &target.refs()));
    assert!((target.mix.get_plain() - 0.2).abs() < 1e-6);
    assert_eq!(target.taps.get_plain(), 3.0);
    assert_eq!(target.freeze.get_plain(), 0.0);
}

#[test]
fn saving_the_same_name_twice_overwrites_and_keeps_the_id() {
    let root = TempRoot::new("overwrite");
    let bank = root.bank();
    let params = TestParams::new();

    params.mix.set_plain(0.1);
    let first = bank.save("Take", &params.refs()).unwrap();
    params.mix.set_plain(0.7);
    let second = bank.save("Take", &params.refs()).unwrap();

    assert_eq!(first.id, second.id, "a re-save is the same preset");
    assert_eq!(bank.list_user(), vec![first.clone()]);
    assert_eq!(json_files(&bank.user_dir().unwrap()).len(), 1);

    let fresh = TestParams::new();
    assert!(bank.apply(&first, &fresh.refs()));
    assert!((fresh.mix.get_plain() - 0.7).abs() < 1e-6);
}

/// D11: user preset names are unique per plugin, case-insensitively, so a
/// name-addressed load can never be ambiguous. Saving "TAKE" over "Take"
/// is the same preset, renamed to the new spelling.
#[test]
fn names_are_unique_case_insensitively() {
    let root = TempRoot::new("case");
    let bank = root.bank();
    let params = TestParams::new();

    let first = bank.save("Take", &params.refs()).unwrap();
    let again = bank.save("TAKE", &params.refs()).unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(names(&bank.list_user()), vec!["TAKE"]);

    bank.save("Other", &params.refs()).unwrap();
    let err = bank.rename(&again, "other").expect_err("case-insensitive clash");
    assert!(err.contains("already exists"), "{err}");
    // A case-only rename of itself is fine.
    let recased = bank.rename(&again, "take").unwrap();
    assert_eq!(recased.id, first.id);
}

/// Review fix 1: a case-only rename or re-save changes only the case of
/// the file name. On a case-insensitive filesystem the old and the new
/// path are one file, and "write new, delete old" deleted the preset.
/// The library now moves the file first and rewrites it in place; on any
/// filesystem the preset must survive, as exactly one file.
#[test]
fn a_case_only_rename_or_resave_keeps_the_preset() {
    let root = TempRoot::new("case-only");
    let bank = root.bank();
    let params = TestParams::new();
    params.mix.set_plain(0.42);
    let saved = bank.save("Take", &params.refs()).unwrap();

    let renamed = bank.rename(&saved, "TAKE").unwrap();
    let path = bank.record(&renamed).unwrap().path.unwrap();
    assert!(path.is_file(), "the renamed preset's file must exist: {path:?}");
    assert_eq!(json_files(&bank.user_dir().unwrap()), vec![path.clone()]);

    let resaved = bank.save("take", &params.refs()).unwrap();
    assert_eq!(resaved.id, saved.id);
    let path = bank.record(&resaved).unwrap().path.unwrap();
    assert!(path.is_file(), "the re-saved preset's file must exist: {path:?}");
    assert_eq!(json_files(&bank.user_dir().unwrap()), vec![path]);

    // …and it still loads, from a fresh index too.
    let fresh = PresetBank::new("com.resonance.test", FACTORY).with_library(Arc::new(
        resonance_plugin::presets::PresetLibrary::new().with_root(root.0.clone()),
    ));
    let target = TestParams::new();
    assert!(fresh.apply(&resaved, &target.refs()));
    assert!((target.mix.get_plain() - 0.42).abs() < 1e-6);
}

#[test]
fn a_nameless_preset_is_refused_before_anything_is_written() {
    let root = TempRoot::new("noname");
    let bank = root.bank();
    let params = TestParams::new();

    assert!(bank.save("   ", &params.refs()).is_err());
    assert!(bank.save("///", &params.refs()).is_err());
    assert!(bank.list_user().is_empty());
}

#[test]
fn the_display_name_survives_characters_a_file_name_cannot_hold() {
    let root = TempRoot::new("unicode");
    let bank = root.bank();
    let params = TestParams::new();

    let saved = bank.save("Vocal — Doubler", &params.refs()).unwrap();
    assert_eq!(saved.name, "Vocal — Doubler");
    assert_eq!(names(&bank.list_user()), vec!["Vocal — Doubler"]);
    assert!(bank.json_for(&saved).is_some());
}

/// The host saves the plugin's own state blob, which carries the plugin's
/// `"preset"` session key. A preset must not claim to be a modified
/// version of itself, so the key is stripped.
#[test]
fn a_host_saved_blob_loses_its_session_identity() {
    let root = TempRoot::new("host-blob");
    let bank = root.bank();
    let blob = serde_json::json!({
        "version": 1,
        "params": {"mix": 0.3, "taps": 2.0, "freeze": 0.0},
        "ir_path": "/tmp/cab.wav",
        "preset": {"name": "Old", "source": "user", "modified": true},
    });
    let saved = bank.write_user_preset("From Host", &blob).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&bank.json_for(&saved).unwrap()).unwrap();
    assert!(doc.get("preset").is_none(), "{doc}");
    assert_eq!(doc["ir_path"], "/tmp/cab.wav", "extra state is kept");
}

/// Every write is temp + fsync + rename: nothing but the preset itself is
/// left behind, and there is never a moment with a half-written file.
#[test]
fn saving_leaves_no_temp_files_behind() {
    let root = TempRoot::new("atomic");
    let bank = root.bank();
    let params = TestParams::new();
    for i in 0..5 {
        params.mix.set_plain(i as f64 / 10.0);
        bank.save("Churn", &params.refs()).unwrap();
    }
    let all: Vec<String> = std::fs::read_dir(bank.user_dir().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].ends_with(".json"), "{all:?}");
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

#[test]
fn factory_and_user_presets_are_listed_together_and_stay_distinguishable() {
    let root = TempRoot::new("list");
    let bank = root.bank();
    let params = TestParams::new();
    bank.save("Zed", &params.refs()).unwrap();
    bank.save("alpha", &params.refs()).unwrap();

    let all = bank.list();
    assert_eq!(
        names(&all),
        vec!["Init", "Wide", "alpha", "Zed"],
        "factory bank in declared order first, then user presets by name"
    );
    assert_eq!(all[0], init());
    assert_eq!(all[1], wide());
    assert!(all[2].source == PresetSource::User && all[3].source == PresetSource::User);
    assert!(all[2].is_resolved());
}

/// Factory metadata is read from the preset file; a bare state document
/// (the old factory shape) still loads, with a name-only meta block.
#[test]
fn factory_records_carry_their_files_metadata() {
    let root = TempRoot::new("factory-meta");
    let bank = root.bank();
    let records = bank.records();
    let w = records.iter().find(|r| r.preset == wide()).unwrap();
    assert_eq!(w.meta.category.as_deref(), Some("Creative"));
    assert_eq!(w.meta.character, vec!["wide"]);
    assert_eq!(w.path, None);
    let i = records.iter().find(|r| r.preset == init()).unwrap();
    assert_eq!(i.meta.name, "Init");

    let params = TestParams::new();
    assert!(bank.apply(&wide(), &params.refs()));
    assert_eq!(params.taps.get_plain(), 7.0);
    assert!(bank.apply(&init(), &params.refs()));
    assert_eq!(params.taps.get_plain(), 3.0);
}

#[test]
fn a_user_preset_may_shadow_a_factory_name_without_replacing_it() {
    let root = TempRoot::new("shadow");
    let bank = root.bank();
    let params = TestParams::new();
    params.mix.set_plain(0.33);
    let mine = bank.save("Init", &params.refs()).unwrap();

    let all = bank.list();
    assert_eq!(all.len(), 3, "{all:?}");
    assert_ne!(mine, init(), "ids never collide across the two sets");

    let factory = TestParams::new();
    assert!(bank.apply(&init(), &factory.refs()));
    assert!((factory.mix.get_plain() - 0.5).abs() < 1e-6);

    let user_side = TestParams::new();
    assert!(bank.apply(&mine, &user_side.refs()));
    assert!((user_side.mix.get_plain() - 0.33).abs() < 1e-6);
}

/// A preset saved by another instance or process is picked up by an
/// explicit list, without a restart.
#[test]
fn a_preset_written_by_another_bank_is_listed() {
    let root = TempRoot::new("other-writer");
    let params = TestParams::new();
    let a = root.bank();
    assert!(a.list_user().is_empty());

    // A second library over the same root, as another process has.
    let other = PresetBank::new("com.resonance.test", FACTORY).with_library(Arc::new(
        resonance_plugin::presets::PresetLibrary::new().with_root(root.0.clone()),
    ));
    other.save("From Elsewhere", &params.refs()).unwrap();

    assert_eq!(names(&a.list_user()), vec!["From Elsewhere"]);
}

// ---------------------------------------------------------------------------
// Rename / delete
// ---------------------------------------------------------------------------

#[test]
fn renaming_keeps_the_id_moves_the_file_and_follows_the_loaded_identity() {
    let root = TempRoot::new("rename");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();
    params.taps.set_plain(6.0);

    let saved = session.save_as(&bank, "Before", &params.refs()).unwrap();
    let renamed = session.rename(&bank, &saved, "After").unwrap();

    assert_eq!(renamed.id, saved.id, "rename never changes the id");
    assert_eq!(renamed.name, "After");
    assert_eq!(bank.list_user(), vec![renamed.clone()]);
    assert_eq!(session.current().map(|c| c.name), Some("After".to_string()));

    let files = json_files(&bank.user_dir().unwrap());
    assert_eq!(files.len(), 1, "the old file is gone");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("After-"), "{name}");

    let fresh = TestParams::new();
    assert!(bank.apply(&renamed, &fresh.refs()));
    assert_eq!(fresh.taps.get_plain(), 6.0);
}

#[test]
fn renaming_onto_an_existing_name_is_refused_and_keeps_both() {
    let root = TempRoot::new("collide");
    let bank = root.bank();
    let params = TestParams::new();
    let one = bank.save("One", &params.refs()).unwrap();
    bank.save("Two", &params.refs()).unwrap();

    let err = bank
        .rename(&one, "Two")
        .expect_err("a colliding rename must be refused");
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(bank.list_user().len(), 2);
}

/// Sanitising is lossy: "Big Room" and "Big+Room" both reduce to the file
/// stem `Big_Room`. They are still two different presets, and the id in
/// the file name keeps their files apart.
#[test]
fn two_names_that_sanitise_alike_are_two_presets() {
    let root = TempRoot::new("sanitise-collide");
    let bank = root.bank();
    let params = TestParams::new();

    params.mix.set_plain(0.25);
    let room = bank.save("Big Room", &params.refs()).unwrap();
    params.mix.set_plain(0.75);
    let plus = bank.save("Big+Room", &params.refs()).unwrap();

    assert_eq!(names(&bank.list_user()), vec!["Big Room", "Big+Room"]);
    assert_eq!(json_files(&bank.user_dir().unwrap()).len(), 2);

    params.mix.set_plain(0.5);
    bank.save("Big Room", &params.refs()).unwrap();
    assert_eq!(bank.list_user().len(), 2, "no third preset should appear");

    params.mix.set_plain(0.0);
    assert!(bank.apply(&room, &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.5, "the re-save should have landed");
    assert!(bank.apply(&plus, &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.75, "the neighbour is untouched");

    bank.delete(&room).unwrap();
    assert_eq!(names(&bank.list_user()), vec!["Big+Room"]);
}

#[test]
fn factory_presets_cannot_be_renamed_or_deleted() {
    let root = TempRoot::new("readonly");
    let bank = root.bank();
    assert!(bank.rename(&init(), "Mine").is_err());
    assert!(bank.delete(&init()).is_err());
    assert_eq!(bank.list().len(), 2, "the factory bank is untouched");
}

/// D10: a delete moves the file to `.trash/`, so an accidental delete —
/// from either surface, one of them an agent — is recoverable.
#[test]
fn deleting_moves_the_file_to_the_trash() {
    let root = TempRoot::new("trash");
    let bank = root.bank();
    let params = TestParams::new();
    let saved = bank.save("Doomed", &params.refs()).unwrap();

    let trashed = bank.trash(&saved).unwrap();
    assert!(trashed.is_file(), "{trashed:?}");
    assert!(trashed.starts_with(root.0.join(".trash").join("com.resonance.test")));
    assert!(bank.list_user().is_empty());
    assert!(json_files(&bank.user_dir().unwrap()).is_empty());
    // The trashed file is still a whole preset.
    let file = PresetFile::parse(&std::fs::read_to_string(&trashed).unwrap()).unwrap();
    assert_eq!(file.id, saved.id);
}

#[test]
fn deleting_the_loaded_preset_clears_the_name_but_not_the_sound() {
    let root = TempRoot::new("delete");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();
    params.mix.set_plain(0.42);

    let saved = session.save_as(&bank, "Doomed", &params.refs()).unwrap();
    session.delete(&bank, &saved).unwrap();

    assert!(bank.list_user().is_empty());
    assert_eq!(session.current(), None);
    assert!(!session.is_modified());
    assert!(
        (params.mix.get_plain() - 0.42).abs() < 1e-6,
        "deleting a preset file must not move a single parameter"
    );
}

// ---------------------------------------------------------------------------
// Session identity
// ---------------------------------------------------------------------------

#[test]
fn loading_names_the_preset_and_clears_the_modified_flag() {
    let root = TempRoot::new("session");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();

    assert_eq!(session.label("— preset —"), "— preset —");

    assert!(session.load_preset(&bank, &wide(), &params.refs()));
    assert_eq!(session.current(), Some(wide()));
    assert!(!session.is_modified());
    assert_eq!(session.label("— preset —"), "Wide");
    assert_eq!(params.taps.get_plain(), 7.0);

    session.mark_modified();
    assert_eq!(session.label("— preset —"), "Wide *");

    let saved = session.save_as(&bank, "Wide+", &params.refs()).unwrap();
    assert_eq!(session.current(), Some(saved));
    assert!(!session.is_modified());
}

/// "Save as…" from a loaded preset seeds the new one's descriptive
/// metadata and records where it came from (§6.4).
#[test]
fn save_as_inherits_meta_and_records_lineage() {
    let root = TempRoot::new("lineage");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();

    session.load_preset(&bank, &wide(), &params.refs());
    let copy = session.save_as(&bank, "Wider", &params.refs()).unwrap();
    let record = bank.record(&copy).unwrap();
    assert_eq!(record.meta.derived_from.as_deref(), Some("wide"));
    assert_eq!(record.meta.category.as_deref(), Some("Creative"));
    assert_eq!(record.meta.tags, vec!["spread"]);

    // Re-saving it in place keeps its own lineage.
    let again = session.save_as(&bank, "Wider", &params.refs()).unwrap();
    assert_eq!(again.id, copy.id);
    assert_eq!(
        bank.record(&again).unwrap().meta.derived_from.as_deref(),
        Some("wide")
    );
}

/// Review fix 2: "Save as…" onto the name of an existing user preset
/// overwrites that preset's *sound* but keeps *its* metadata and lineage;
/// the loaded preset's meta only seeds a new preset.
#[test]
fn save_as_over_an_existing_preset_keeps_its_meta() {
    let root = TempRoot::new("save-over-meta");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();

    let mine = bank
        .save_with(
            "My Bass",
            &params.refs(),
            resonance_plugin::presets::SaveOptions {
                meta: Some(resonance_plugin::presets::PresetMeta {
                    category: Some("Track".into()),
                    genres: vec!["metal".into()],
                    tags: vec!["mine".into()],
                    ..Default::default()
                }),
                derived_from: None,
            },
        )
        .unwrap();

    session.load_preset(&bank, &wide(), &params.refs());
    let over = session.save_as(&bank, "My Bass", &params.refs()).unwrap();
    assert_eq!(over.id, mine.id);
    let meta = bank.record(&over).unwrap().meta;
    assert_eq!(meta.tags, vec!["mine"]);
    assert_eq!(meta.genres, vec!["metal"]);
    assert_eq!(meta.category.as_deref(), Some("Track"));
    assert_eq!(meta.derived_from, None, "no lineage invented on overwrite");
}

#[test]
fn a_preset_that_no_longer_exists_leaves_the_sound_and_the_name_alone() {
    let root = TempRoot::new("missing");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();
    assert!(session.load_preset(&bank, &wide(), &params.refs()));

    assert!(!session.load_preset(&bank, &user("Gone"), &params.refs()));
    assert_eq!(session.current(), Some(wide()));
    assert_eq!(params.taps.get_plain(), 7.0);
}

/// A project saved before preset ids carries `{name, source}` only. The
/// bar resolves it to an id by name the first time it has a bank.
#[test]
fn a_name_only_identity_resolves_to_an_id() {
    let root = TempRoot::new("resolve");
    let bank = root.bank();
    let params = TestParams::new();
    let saved = bank.save("Legacy Sound", &params.refs()).unwrap();

    let session = PresetSession::new();
    session.load(&serde_json::json!({
        "params": {},
        "preset": {"name": "Legacy Sound", "source": "user", "modified": true},
    }));
    let before = session.current().unwrap();
    assert!(!before.is_resolved());
    assert!(before.matches(&saved), "an unresolved ref matches by name");
    assert_ne!(before, saved, "but is not equal to the resolved one");

    session.resolve(&bank);
    let after = session.current().unwrap();
    assert_eq!(after.id, saved.id);
    assert!(session.is_modified(), "resolving is not loading");
    assert_eq!(session.save()["preset"]["id"], saved.id.as_str());
}

/// Review fix 12: a session built for a plugin resolves a name-only
/// identity when state loads (§13), so the next save writes the id.
#[test]
fn a_name_only_identity_resolves_at_state_load() {
    let root = TempRoot::new("resolve-at-load");
    let bank = root.bank();
    let params = TestParams::new();
    let saved = bank.save("Old Project Sound", &params.refs()).unwrap();

    let dir = root.0.clone();
    let session = PresetSession::resolving(
        Some(Box::new(move || {
            PresetBank::new("com.resonance.test", FACTORY).with_root(dir.clone())
        })),
        None,
    );
    session.load(&serde_json::json!({
        "params": {},
        "preset": {"name": "old project sound", "source": "user", "modified": false},
    }));
    let current = session.current().unwrap();
    assert_eq!(current.id, saved.id, "resolved by name, case-insensitively");
    assert_eq!(session.save()["preset"]["id"], saved.id.as_str());

    // A name that is gone stays unresolved rather than vanishing.
    session.load(&serde_json::json!({
        "params": {},
        "preset": {"name": "Deleted Long Ago", "source": "user"},
    }));
    assert_eq!(session.current().map(|c| c.is_resolved()), Some(false));
}

/// Resolving at state load is read-only and never opens a directory the
/// process has not indexed yet: a plugin's `load_state` under plain
/// `cargo test` (or a project open) must not read, convert or write the
/// preset root. A factory identity still resolves, from memory.
#[test]
fn a_state_load_with_a_name_only_identity_writes_nothing() {
    let root = TempRoot::new("load-hermetic");
    let dir = root.0.join("com.resonance.test");
    std::fs::create_dir_all(&dir).unwrap();
    let legacy = dir.join("Legacy.json");
    std::fs::write(&legacy, r#"{"params":{"mix":0.8},"name":"Legacy"}"#).unwrap();
    let before = json_files(&dir);

    // A private library, so no other test can have opened this root.
    let library =
        Arc::new(resonance_plugin::presets::PresetLibrary::new().with_root(root.0.clone()));
    let lib = library.clone();
    let session = PresetSession::resolving(
        Some(Box::new(move || {
            PresetBank::new("com.resonance.test", FACTORY).with_library(lib.clone())
        })),
        None,
    );
    session.load(&serde_json::json!({
        "params": {},
        "preset": {"name": "Legacy", "source": "user", "modified": false},
    }));
    assert!(!session.current().unwrap().is_resolved(), "left for the bar");
    let after: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(after, vec!["Legacy.json".to_string()], "nothing converted or written");
    assert_eq!(json_files(&dir), before);
    assert!(!root.0.join(".trash").exists());

    session.load(&serde_json::json!({
        "params": {},
        "preset": {"name": "Wide", "source": "factory"},
    }));
    assert_eq!(session.current().unwrap().id, "wide", "factory resolves from memory");
    assert_eq!(json_files(&dir), before);
}

/// Review fix 3: the bar calls `resolve` every frame, so it reads the
/// cached index and never the directory within `BAR_REFRESH`.
#[test]
fn resolving_in_the_bar_reads_the_cached_index() {
    let root = TempRoot::new("resolve-cached");
    let bank = root.bank();
    bank.list(); // the library has indexed the (empty) directory

    // Another process saves the preset the project names.
    let params = TestParams::new();
    let other = PresetBank::new("com.resonance.test", FACTORY).with_library(Arc::new(
        resonance_plugin::presets::PresetLibrary::new().with_root(root.0.clone()),
    ));
    let saved = other.save("Elsewhere", &params.refs()).unwrap();

    let session = PresetSession::new();
    session.set_current(Some(user("Elsewhere")));
    session.resolve(&bank);
    assert!(
        !session.current().unwrap().is_resolved(),
        "within BAR_REFRESH the per-frame resolve must not touch the disk"
    );
    bank.list(); // an explicit read picks the new file up
    session.resolve(&bank);
    assert_eq!(session.current().unwrap().id, saved.id);
}

/// Review fix 13: `==` is an equivalence relation; `matches` is the
/// lenient comparison for name-only refs.
#[test]
fn preset_ref_equality_is_strict_and_matches_is_lenient() {
    let resolved = PresetRef::user("3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42", "Take");
    let renamed = PresetRef::user("3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42", "Take 2");
    let by_name = user("Take");
    let by_other_case = user("TAKE");

    assert_eq!(resolved, renamed, "same (source, id)");
    assert_ne!(resolved, by_name, "a resolved ref never equals a name-only one");
    assert_ne!(by_name, by_other_case, "name-only refs are equal only by exact name");
    assert!(by_name.matches(&resolved) && resolved.matches(&by_name));
    assert!(by_other_case.matches(&resolved), "matches ignores case");
    assert!(!by_name.matches(&renamed));
    assert!(!init().matches(&user("Init")), "source always counts");
}

// ---------------------------------------------------------------------------
// The preset bar's state machine — what the editor's buttons drive
// ---------------------------------------------------------------------------

#[test]
fn the_bar_saves_the_typed_name_and_reports_it() {
    let root = TempRoot::new("bar-save");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();
    params.mix.set_plain(0.31);

    editor.begin_save(&session);
    assert_eq!(editor.naming(), Some(NamingKind::SaveAs));
    assert_eq!(
        editor.name_buffer().map(|s| s.as_str()),
        Some("My Preset"),
        "with nothing loaded the bar proposes a name rather than an empty field"
    );
    editor.name_buffer().unwrap().clear();
    editor.name_buffer().unwrap().push_str("Bar Sound");

    let event = editor.submit(&bank, &session, &params.refs());
    let PresetEvent::Saved(saved) = event else {
        panic!("expected Saved, got {event:?}");
    };
    assert_eq!(saved.name, "Bar Sound");
    assert_eq!(editor.naming(), None, "the field closes on success");
    assert_eq!(editor.error(), None);
    assert_eq!(bank.list_user(), vec![saved.clone()]);
    assert_eq!(session.current(), Some(saved));
}

#[test]
fn saving_over_a_factory_preset_proposes_a_copy_not_an_overwrite() {
    let root = TempRoot::new("bar-copy");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    editor.pick(&bank, &session, &wide(), &params.refs());
    editor.begin_save(&session);
    assert_eq!(editor.name_buffer().map(|s| s.as_str()), Some("Wide (edit)"));

    // …while saving over a *user* preset offers to overwrite itself.
    editor.submit(&bank, &session, &params.refs());
    editor.begin_save(&session);
    assert_eq!(editor.name_buffer().map(|s| s.as_str()), Some("Wide (edit)"));
}

#[test]
fn a_rejected_name_keeps_the_field_open_with_what_was_typed() {
    let root = TempRoot::new("bar-error");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    editor.begin_save(&session);
    editor.name_buffer().unwrap().clear();
    editor.name_buffer().unwrap().push_str("   ");

    let event = editor.submit(&bank, &session, &params.refs());
    assert_eq!(event, PresetEvent::None);
    assert_eq!(editor.naming(), Some(NamingKind::SaveAs));
    assert_eq!(editor.name_buffer().map(|s| s.as_str()), Some("   "));
    assert!(editor.error().is_some());
    assert!(bank.list_user().is_empty());

    editor.cancel();
    assert_eq!(editor.naming(), None);
    assert_eq!(editor.error(), None);
}

#[test]
fn the_bar_renames_and_deletes_through_the_session() {
    let root = TempRoot::new("bar-rename");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    let saved = session.save_as(&bank, "First", &params.refs()).unwrap();
    editor.begin_rename(&saved);
    assert_eq!(editor.naming(), Some(NamingKind::Rename));
    editor.name_buffer().unwrap().clear();
    editor.name_buffer().unwrap().push_str("Second");
    let renamed = PresetRef::user(saved.id.clone(), "Second");
    assert_eq!(
        editor.submit(&bank, &session, &params.refs()),
        PresetEvent::Renamed(renamed.clone())
    );

    let event = editor.delete(&bank, &session, &renamed);
    assert_eq!(event, PresetEvent::Deleted(renamed));
    assert!(bank.list_user().is_empty());

    // Deleting something that cannot be deleted reports the failure in
    // the bar instead of panicking.
    let event = editor.delete(&bank, &session, &init());
    assert_eq!(event, PresetEvent::None);
    assert!(editor.error().is_some());
}

#[test]
fn picking_a_preset_loads_it_and_reports_that_every_param_may_have_moved() {
    let root = TempRoot::new("bar-pick");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    let event = editor.pick(&bank, &session, &wide(), &params.refs());
    assert_eq!(event, PresetEvent::Loaded(wide()));
    assert_eq!(params.taps.get_plain(), 7.0);

    let event = editor.pick(&bank, &session, &user("Nope"), &params.refs());
    assert_eq!(event, PresetEvent::None);
    assert!(editor.error().is_some());
    assert_eq!(session.current(), Some(wide()));
}

/// Stepping walks the merged list, so a user preset is reachable from the
/// factory bank without opening the combo (ba todo #1280).
#[test]
fn stepping_walks_factory_then_user_presets() {
    let root = TempRoot::new("bar-step");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();
    let mine = bank.save("Mine", &params.refs()).unwrap();

    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(session.current(), Some(init()));
    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(session.current(), Some(wide()));
    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(session.current(), Some(mine));
    editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(session.current(), Some(wide()));
}

/// Stepping clamps rather than wrapping.
#[test]
fn stepping_stops_at_both_ends() {
    let root = TempRoot::new("bar-step-ends");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(session.current(), Some(wide()));

    let event = editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(event, PresetEvent::None, "no wrap past the last preset");
    assert_eq!(session.current(), Some(wide()));

    editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(session.current(), Some(init()));
    let event = editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(event, PresetEvent::None, "no wrap before the first preset");
    assert_eq!(session.current(), Some(init()));
}

#[test]
fn stepping_recalls_the_preset_it_lands_on() {
    let root = TempRoot::new("bar-step-sound");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    editor.pick(&bank, &session, &init(), &params.refs());
    assert_eq!(params.taps.get_plain(), 3.0);
    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(params.taps.get_plain(), 7.0, "Wide's value should be live");
}

// ---------------------------------------------------------------------------
// Identity through save_state / load_state
// ---------------------------------------------------------------------------

struct PresetPlugin {
    params: TestParams,
    presets: Arc<PresetSession>,
}

impl ResonancePlugin for PresetPlugin {
    const CLAP_ID: &'static str = "com.resonance.test";
    const NAME: &'static str = "Test";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = "0.1.0";
    const DESCRIPTION: &'static str = "preset state fixture";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);
    const FACTORY_PRESETS: &'static [FactoryPreset] = FACTORY;

    fn new() -> Self {
        Self {
            params: TestParams::new(),
            presets: PresetSession::new(),
        }
    }
    fn param_count(&self) -> usize {
        3
    }
    fn param(&self, index: usize) -> &dyn Param {
        self.params.refs()[index]
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(self.presets.clone())
    }
}

#[test]
fn the_loaded_preset_survives_save_state_and_load_state() {
    let root = TempRoot::new("state");
    let bank = root.bank();

    let plugin = PresetPlugin::new();
    let saved_ref = plugin
        .presets
        .save_as(&bank, "Session Sound", &plugin.params.refs())
        .unwrap();
    plugin.params.mix.set_plain(0.77);
    plugin.presets.mark_modified();
    let blob = plugin.save_state();

    let value: serde_json::Value = serde_json::from_slice(&blob).unwrap();
    assert_eq!(value["preset"]["name"], "Session Sound");
    assert_eq!(value["preset"]["source"], "user");
    assert_eq!(value["preset"]["id"], saved_ref.id.as_str());
    assert_eq!(value["preset"]["modified"], true);

    let mut reopened = PresetPlugin::new();
    assert_eq!(reopened.presets.current(), None);
    assert!(reopened.load_state(&blob));
    assert_eq!(reopened.presets.current(), Some(saved_ref.clone()));
    assert_eq!(reopened.presets.current().unwrap().id, saved_ref.id);
    assert!(reopened.presets.is_modified());
    assert_eq!(reopened.presets.label("— preset —"), "Session Sound *");
    assert!((reopened.params.mix.get_plain() - 0.77).abs() < 1e-6);
}

#[test]
fn state_written_before_preset_identity_existed_still_loads() {
    let mut plugin = PresetPlugin::new();
    let legacy = br#"{"params":{"mix":0.25,"taps":5.0,"freeze":1.0}}"#;
    assert!(plugin.load_state(legacy));
    assert_eq!(plugin.presets.current(), None);
    assert!(!plugin.presets.is_modified());
    assert!((plugin.params.mix.get_plain() - 0.25).abs() < 1e-6);
}

/// A preset file is accepted wherever a state document is: `load_state`
/// of a whole format-1 file loads the document it carries.
#[test]
fn load_state_accepts_a_whole_preset_file() {
    let mut plugin = PresetPlugin::new();
    assert!(plugin.load_state(FACTORY[1].json.as_bytes()));
    assert_eq!(plugin.params.taps.get_plain(), 7.0);
    assert!(resonance_plugin::presets::load(FACTORY[1].json, 3, |i| plugin.param(i)));
}

#[test]
fn a_session_chains_a_plugins_own_extra_state_saver() {
    struct FileSaver;
    impl ExtraStateSaver for FileSaver {
        fn save(&self) -> serde_json::Map<String, serde_json::Value> {
            let mut m = serde_json::Map::new();
            m.insert("ir_path".into(), serde_json::json!("/tmp/cab.wav"));
            m
        }
        fn load(&self, state: &serde_json::Value) {
            assert_eq!(state["ir_path"], "/tmp/cab.wav");
        }
    }

    let session = PresetSession::with_extra(Arc::new(FileSaver));
    session.set_current(Some(init()));

    let saved = session.save();
    assert_eq!(saved["ir_path"], "/tmp/cab.wav");
    assert_eq!(saved["preset"]["name"], "Init");
    assert_eq!(saved["preset"]["id"], "init");

    session.load(&serde_json::json!({
        "ir_path": "/tmp/cab.wav",
        "preset": {"id": "init", "name": "Init", "source": "factory", "modified": false},
    }));
    assert_eq!(session.current(), Some(init()));
}

// ---------------------------------------------------------------------------
// The factory bank as the host reads it (ba todo #1333)
// ---------------------------------------------------------------------------

/// Encode and decode are a pair and have to stay one. The symbol carries
/// the state document (so today's host loads it unchanged) plus the id
/// and metadata.
#[test]
fn a_factory_bank_survives_the_trip_to_the_host() {
    let encoded = resonance_plugin::presets::encode_factory_bank(FACTORY)
        .expect("a well-formed bank encodes");
    let text = encoded.to_str().expect("valid utf-8");
    let decoded = resonance_plugin::presets::decode_factory_bank(text);

    let names: Vec<&str> = decoded.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["Init", "Wide"]);

    // The bodies arrive as bare state documents, not preset files.
    let params = TestParams::new();
    let body: serde_json::Value = serde_json::from_str(&decoded[1].1).unwrap();
    assert!(body.get("params").is_some(), "{body}");
    assert!(resonance_plugin::presets::apply(&decoded[1].1, &params.refs(), &[]));
    assert_eq!(params.taps.get_plain(), 7.0);

    // …and the id and metadata ride alongside.
    let entries = resonance_plugin::presets::decode_factory_entries(text);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["init", "wide"]);
    let wide_file = PresetFile::parse(&entries[1].json).unwrap();
    assert_eq!(wide_file.meta.category.as_deref(), Some("Creative"));
}

#[test]
fn an_empty_factory_bank_decodes_to_nothing() {
    let encoded =
        resonance_plugin::presets::encode_factory_bank(&[]).expect("an empty bank encodes");
    assert!(
        resonance_plugin::presets::decode_factory_bank(encoded.to_str().unwrap()).is_empty()
    );
}

/// Junk from a newer or broken build is skipped entry by entry.
#[test]
fn a_malformed_entry_does_not_take_the_bank_with_it() {
    let text = r#"[{"name":"Good","json":{"params":{}}},{"unexpected":true},{"name":"Also good","json":{}}]"#;
    let decoded = resonance_plugin::presets::decode_factory_bank(text);
    let names: Vec<&str> = decoded.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["Good", "Also good"]);

    // An entry from a build before ids gets one slugged from its name.
    let entries = resonance_plugin::presets::decode_factory_entries(text);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["good", "also-good"]);
}
