//! The preset system: the user-preset directory (save / rename /
//! delete), the merged factory+user listing, and the loaded-preset
//! identity that has to survive closing the window (ba todo #1332,
//! audit findings X1 and X2).
//!
//! Every test points its bank at a private temporary root through
//! `PresetBank::with_root`, so nothing here reads or writes the real
//! `~/.local/share/resonance/plugin-presets`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use resonance_plugin::presets::{
    FactoryPreset, NamingKind, PresetBank, PresetEditor, PresetEvent, PresetRef, PresetSession,
    PresetSource,
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

const FACTORY: &[FactoryPreset] = &[
    FactoryPreset {
        name: "Init",
        json: r#"{"params":{"mix":0.5,"taps":3.0,"freeze":0.0}}"#,
    },
    FactoryPreset {
        name: "Wide",
        json: r#"{"params":{"mix":0.9,"taps":7.0,"freeze":1.0}}"#,
    },
];

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
        PresetBank::new("com.resonance.test", FACTORY).with_root(self.0.clone())
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
fn saving_writes_a_file_into_the_plugins_own_directory() {
    let root = TempRoot::new("dir");
    let bank = root.bank();
    let params = TestParams::new();

    let saved = bank.save("My Sound", &params.refs()).expect("save failed");
    assert_eq!(saved, PresetRef::user("My Sound"));

    let dir = bank.user_dir().expect("a bank with a root always has a dir");
    assert!(dir.ends_with("com.resonance.test"), "{dir:?}");
    assert!(
        dir.join("My_Sound.json").is_file(),
        "expected the sanitised file name inside {dir:?}"
    );
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
    bank.save("Partial?", &params.refs()).expect("save failed");

    let text = std::fs::read_to_string(bank.user_dir().unwrap().join("Partial_.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let map = value.get("params").and_then(|v| v.as_object()).unwrap();

    assert_eq!(
        map.len(),
        params.refs().len(),
        "a saved preset must snapshot every declared param, found {map:?}"
    );
    for p in params.refs() {
        assert!(map.contains_key(p.id()), "missing param '{}'", p.id());
    }
    // …and the file is versioned like project state is.
    assert_eq!(
        value.get("version").and_then(|v| v.as_u64()),
        Some(resonance_plugin::STATE_VERSION as u64)
    );

    // Recall proves the snapshot is complete rather than merely present:
    // a target sitting at the far end of every range comes all the way
    // back, including the params the user never touched.
    let target = TestParams::new();
    target.mix.set_plain(1.0);
    target.taps.set_plain(8.0);
    target.freeze.set_plain(1.0);
    assert!(bank.apply(&PresetRef::user("Partial?"), &target.refs()));
    assert!((target.mix.get_plain() - 0.2).abs() < 1e-6);
    assert_eq!(target.taps.get_plain(), 3.0);
    assert_eq!(target.freeze.get_plain(), 0.0);
}

#[test]
fn saving_the_same_name_twice_overwrites_rather_than_duplicating() {
    let root = TempRoot::new("overwrite");
    let bank = root.bank();
    let params = TestParams::new();

    params.mix.set_plain(0.1);
    bank.save("Take", &params.refs()).unwrap();
    params.mix.set_plain(0.7);
    bank.save("Take", &params.refs()).unwrap();

    let user = bank.list_user();
    assert_eq!(user, vec![PresetRef::user("Take")]);

    let fresh = TestParams::new();
    assert!(bank.apply(&PresetRef::user("Take"), &fresh.refs()));
    assert!((fresh.mix.get_plain() - 0.7).abs() < 1e-6);
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
    assert_eq!(bank.list_user(), vec![PresetRef::user("Vocal — Doubler")]);
    assert!(bank.json_for(&saved).is_some());
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
    bank.save("Alpha", &params.refs()).unwrap();

    let all = bank.list();
    assert_eq!(
        all,
        vec![
            PresetRef::factory("Init"),
            PresetRef::factory("Wide"),
            PresetRef::user("Alpha"),
            PresetRef::user("Zed"),
        ],
        "factory bank in declared order first, then user presets by name"
    );
    assert!(all[0].source == PresetSource::Factory && all[3].source == PresetSource::User);
}

#[test]
fn a_user_preset_may_shadow_a_factory_name_without_replacing_it() {
    let root = TempRoot::new("shadow");
    let bank = root.bank();
    let params = TestParams::new();
    params.mix.set_plain(0.33);
    bank.save("Init", &params.refs()).unwrap();

    let all = bank.list();
    assert_eq!(all.len(), 3, "{all:?}");

    // The pair (name, source) is the identity, so both "Init"s resolve
    // to their own blob.
    let factory = TestParams::new();
    assert!(bank.apply(&PresetRef::factory("Init"), &factory.refs()));
    assert!((factory.mix.get_plain() - 0.5).abs() < 1e-6);

    let user = TestParams::new();
    assert!(bank.apply(&PresetRef::user("Init"), &user.refs()));
    assert!((user.mix.get_plain() - 0.33).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// Rename / delete
// ---------------------------------------------------------------------------

#[test]
fn renaming_moves_the_preset_and_follows_the_loaded_identity() {
    let root = TempRoot::new("rename");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();
    params.taps.set_plain(6.0);

    let saved = session.save_as(&bank, "Before", &params.refs()).unwrap();
    let renamed = session.rename(&bank, &saved, "After").unwrap();

    assert_eq!(renamed, PresetRef::user("After"));
    assert_eq!(bank.list_user(), vec![PresetRef::user("After")]);
    assert_eq!(session.current(), Some(PresetRef::user("After")));

    let fresh = TestParams::new();
    assert!(bank.apply(&renamed, &fresh.refs()));
    assert_eq!(fresh.taps.get_plain(), 6.0);
}

#[test]
fn renaming_onto_an_existing_name_is_refused_and_keeps_both() {
    let root = TempRoot::new("collide");
    let bank = root.bank();
    let params = TestParams::new();
    bank.save("One", &params.refs()).unwrap();
    bank.save("Two", &params.refs()).unwrap();

    let err = bank
        .rename(&PresetRef::user("One"), "Two")
        .expect_err("a colliding rename must be refused");
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(bank.list_user().len(), 2);
}

/// Sanitising is lossy: "Big Room" and "Big+Room" both reduce to the file
/// stem `Big_Room`. They are still two different presets, and saving the
/// second must not silently destroy the first.
#[test]
fn two_names_that_sanitise_alike_are_two_presets() {
    let root = TempRoot::new("sanitise-collide");
    let bank = root.bank();
    let params = TestParams::new();

    params.mix.set_plain(0.25);
    bank.save("Big Room", &params.refs()).unwrap();
    params.mix.set_plain(0.75);
    bank.save("Big+Room", &params.refs()).unwrap();

    let names: Vec<String> = bank.list_user().into_iter().map(|p| p.name).collect();
    assert_eq!(names, vec!["Big Room".to_string(), "Big+Room".to_string()]);

    // ...and each still recalls its own sound, so they are genuinely two
    // files and not one entry listed twice.
    params.mix.set_plain(0.0);
    assert!(bank.apply(&PresetRef::user("Big Room"), &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.25);
    assert!(bank.apply(&PresetRef::user("Big+Room"), &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.75);
}

/// Re-saving under a name that already exists still overwrites that one
/// preset — the collision handling above must not turn every save into a
/// new file.
#[test]
fn re_saving_a_colliding_name_overwrites_only_its_own_file() {
    let root = TempRoot::new("sanitise-overwrite");
    let bank = root.bank();
    let params = TestParams::new();

    params.mix.set_plain(0.25);
    bank.save("Big Room", &params.refs()).unwrap();
    params.mix.set_plain(0.75);
    bank.save("Big+Room", &params.refs()).unwrap();

    params.mix.set_plain(0.5);
    bank.save("Big Room", &params.refs()).unwrap();

    assert_eq!(bank.list_user().len(), 2, "no third preset should appear");
    params.mix.set_plain(0.0);
    assert!(bank.apply(&PresetRef::user("Big Room"), &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.5, "the re-save should have landed");
    assert!(bank.apply(&PresetRef::user("Big+Room"), &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.75, "the neighbour is untouched");
}

/// Deleting one of two colliding presets must remove the one named, not
/// whichever file the name happens to sanitise to.
#[test]
fn deleting_one_colliding_preset_leaves_the_other() {
    let root = TempRoot::new("sanitise-delete");
    let bank = root.bank();
    let params = TestParams::new();

    params.mix.set_plain(0.25);
    bank.save("Big Room", &params.refs()).unwrap();
    params.mix.set_plain(0.75);
    bank.save("Big+Room", &params.refs()).unwrap();

    bank.delete(&PresetRef::user("Big Room")).unwrap();

    let names: Vec<String> = bank.list_user().into_iter().map(|p| p.name).collect();
    assert_eq!(names, vec!["Big+Room".to_string()]);
    params.mix.set_plain(0.0);
    assert!(bank.apply(&PresetRef::user("Big+Room"), &params.refs()));
    assert_eq!(params.mix.get_plain(), 0.75);
}

#[test]
fn factory_presets_cannot_be_renamed_or_deleted() {
    let root = TempRoot::new("readonly");
    let bank = root.bank();
    assert!(bank.rename(&PresetRef::factory("Init"), "Mine").is_err());
    assert!(bank.delete(&PresetRef::factory("Init")).is_err());
    assert_eq!(bank.list().len(), 2, "the factory bank is untouched");
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

    assert!(session.load_preset(&bank, &PresetRef::factory("Wide"), &params.refs()));
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));
    assert!(!session.is_modified());
    assert_eq!(session.label("— preset —"), "Wide");
    assert_eq!(params.taps.get_plain(), 7.0);

    session.mark_modified();
    assert_eq!(session.label("— preset —"), "Wide *");

    // Saving the edited sound names it after the new user preset and the
    // asterisk goes away.
    session.save_as(&bank, "Wide+", &params.refs()).unwrap();
    assert_eq!(session.current(), Some(PresetRef::user("Wide+")));
    assert!(!session.is_modified());
}

#[test]
fn a_preset_that_no_longer_exists_leaves_the_sound_and_the_name_alone() {
    let root = TempRoot::new("missing");
    let bank = root.bank();
    let session = PresetSession::new();
    let params = TestParams::new();
    session
        .load_preset(&bank, &PresetRef::factory("Wide"), &params.refs())
        .then_some(())
        .expect("factory load failed");

    assert!(!session.load_preset(&bank, &PresetRef::user("Gone"), &params.refs()));
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));
    assert_eq!(params.taps.get_plain(), 7.0);
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
    assert_eq!(event, PresetEvent::Saved(PresetRef::user("Bar Sound")));
    assert_eq!(editor.naming(), None, "the field closes on success");
    assert_eq!(editor.error(), None);
    assert_eq!(bank.list_user(), vec![PresetRef::user("Bar Sound")]);
    assert_eq!(session.current(), Some(PresetRef::user("Bar Sound")));
}

#[test]
fn saving_over_a_factory_preset_proposes_a_copy_not_an_overwrite() {
    let root = TempRoot::new("bar-copy");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    editor.pick(&bank, &session, &PresetRef::factory("Wide"), &params.refs());
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

    // Cancelling clears both.
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
    assert_eq!(
        editor.submit(&bank, &session, &params.refs()),
        PresetEvent::Renamed(PresetRef::user("Second"))
    );

    let event = editor.delete(&bank, &session, &PresetRef::user("Second"));
    assert_eq!(event, PresetEvent::Deleted(PresetRef::user("Second")));
    assert!(bank.list_user().is_empty());

    // Deleting something that is already gone reports the failure in the
    // bar instead of panicking.
    let event = editor.delete(&bank, &session, &PresetRef::factory("Init"));
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

    let event = editor.pick(&bank, &session, &PresetRef::factory("Wide"), &params.refs());
    assert_eq!(event, PresetEvent::Loaded(PresetRef::factory("Wide")));
    assert_eq!(params.taps.get_plain(), 7.0);

    let event = editor.pick(&bank, &session, &PresetRef::user("Nope"), &params.refs());
    assert_eq!(event, PresetEvent::None);
    assert!(editor.error().is_some());
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));
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
    bank.save("Mine", &params.refs()).unwrap();

    // Nothing loaded: forwards enters at the top of the list.
    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(session.current(), Some(PresetRef::factory("Init")));

    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));

    // ...and on into the user half of the same list.
    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(session.current(), Some(PresetRef::user("Mine")));

    editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));
}

/// Stepping clamps rather than wrapping: running off the end and silently
/// reappearing at the other one is disorienting when you are listening
/// rather than looking.
#[test]
fn stepping_stops_at_both_ends() {
    let root = TempRoot::new("bar-step-ends");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    // Backwards from nothing enters at the bottom.
    editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));

    let event = editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(event, PresetEvent::None, "no wrap past the last preset");
    assert_eq!(session.current(), Some(PresetRef::factory("Wide")));

    editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(session.current(), Some(PresetRef::factory("Init")));
    let event = editor.step(&bank, &session, -1, &params.refs());
    assert_eq!(event, PresetEvent::None, "no wrap before the first preset");
    assert_eq!(session.current(), Some(PresetRef::factory("Init")));
}

/// Stepping actually loads the sound, not just the label.
#[test]
fn stepping_recalls_the_preset_it_lands_on() {
    let root = TempRoot::new("bar-step-sound");
    let bank = root.bank();
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let params = TestParams::new();

    editor.pick(&bank, &session, &PresetRef::factory("Init"), &params.refs());
    assert_eq!(params.taps.get_plain(), 3.0);
    editor.step(&bank, &session, 1, &params.refs());
    assert_eq!(params.taps.get_plain(), 7.0, "Wide's value should be live");
}

// ---------------------------------------------------------------------------
// Identity through save_state / load_state — the half that regressed
// every time the window closed
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

    // The identity is in the blob, next to the params.
    let value: serde_json::Value = serde_json::from_slice(&blob).unwrap();
    assert_eq!(value["preset"]["name"], "Session Sound");
    assert_eq!(value["preset"]["source"], "user");
    assert_eq!(value["preset"]["modified"], true);

    // Reopening the project: a brand-new instance restores both the
    // sound and the name the picker shows.
    let mut reopened = PresetPlugin::new();
    assert_eq!(reopened.presets.current(), None);
    assert!(reopened.load_state(&blob));
    assert_eq!(reopened.presets.current(), Some(saved_ref));
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
    session.set_current(Some(PresetRef::factory("Init")));

    let saved = session.save();
    assert_eq!(saved["ir_path"], "/tmp/cab.wav");
    assert_eq!(saved["preset"]["name"], "Init");

    // The chained saver still sees the whole state object on the way back.
    session.load(&serde_json::json!({
        "ir_path": "/tmp/cab.wav",
        "preset": {"name": "Init", "source": "factory", "modified": false},
    }));
    assert_eq!(session.current(), Some(PresetRef::factory("Init")));
}
