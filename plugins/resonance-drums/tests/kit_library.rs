//! Kit selection and the kit reference (drums-plugin-rework.md §5, §9
//! `kit_library.rs`): the `kit_select` slot parameter, `kit_load_progress`,
//! state v2's `kit_ref` and its v1 conversion, presets, and the missing-kit
//! path — plugin side and the editor's banner.
//!
//! Every test builds its own kit library at a temp root and hands it to
//! the plugin (`selection.library.set`) before anything opens the default
//! one, so the tests run in parallel without sharing a slot table, and
//! nothing reads the user's data dir.
#![cfg(feature = "editor")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_drums::download::{ServerIndex, ServerKit, WorkerConfig};
use resonance_drums::drum_map;
use resonance_drums::kit::NUM_OUTPUT_PORTS;
use resonance_drums::kit_loader::KitStatus;
use resonance_drums::library::{Roots, SharedKitLibrary};
use resonance_drums::params::GLOBAL_PARAMS;
use resonance_drums::selection::{self, KitRef, ResolvedBy, NO_KIT, PARKED_KIT};
use resonance_drums::{DrumsExtraState, ResonanceDrums, TestEditor};
use resonance_plugin::plugin::ExtraStateSaver;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// `kit_select` and `kit_load_progress` close the globals, ahead of the
/// pad block.
const KIT_SELECT: usize = GLOBAL_PARAMS - 2;
const KIT_LOAD_PROGRESS: usize = GLOBAL_PARAMS - 1;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "resonance-drums-kitlib-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drumkits")).unwrap();
        Self(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("drumkits")
    }

    fn library(&self) -> Arc<SharedKitLibrary> {
        let lib = SharedKitLibrary::open(Roots {
            root: Some(self.root()),
            marks_dir: Some(self.0.join("library")),
            installed_json: None,
            worker: WorkerConfig {
                index_url: "http://127.0.0.1:9/index.json".into(),
                ..WorkerConfig::default()
            },
        });
        lib.rescan().unwrap().unwrap();
        lib
    }

    /// A playable kit `<root>/<dir>/kit/drum_samples.json` called `name`,
    /// whose kick plays at `level`.
    fn kit(&self, dir: &str, name: &str, level: f32) -> PathBuf {
        write_kit(&self.root().join(dir).join("kit"), name, level)
    }

    /// The same, outside the library root.
    fn outside_kit(&self, dir: &str, name: &str, level: f32) -> PathBuf {
        write_kit(&self.0.join("elsewhere").join(dir), name, level)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A mono 16-bit WAV holding `level` for 2400 frames.
fn write_wav(path: &Path, level: f32) {
    let frames = 2_400usize;
    let data_len = frames * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    let v = (level * i16::MAX as f32).round() as i16;
    for _ in 0..frames {
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// A one-piece kit (the kick, one KickIn take) named `name`.
fn write_kit(dir: &Path, name: &str, level: f32) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    write_wav(&dir.join("kick.wav"), level);
    let manifest = serde_json::json!({
        "_meta": { "name": name },
        "SD Kick mit Teppich": {
            "01_KickIn_e901": {
                "brand": "Sennheiser", "channel": "01", "mic": "e901",
                "position": "KickIn",
                "rounds": { "RR1": { "Vel01": "kick.wav" } }
            }
        }
    });
    let path = dir.join("drum_samples.json");
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    path
}

/// A plugin over `library`.
fn plugin_on(library: &Arc<SharedKitLibrary>) -> ResonanceDrums {
    let plugin = ResonanceDrums::new();
    assert!(
        plugin.bridge.params.selection.library.set(library.clone()),
        "nothing may have opened the default library first"
    );
    plugin
}

fn booted_on(library: &Arc<SharedKitLibrary>) -> ResonanceDrums {
    let mut plugin = plugin_on(library);
    assert!(plugin.initialize(RATE, BLOCK as u32));
    plugin
}

fn slot_of(library: &SharedKitLibrary, name: &str) -> i32 {
    library
        .read()
        .find(name)
        .and_then(|e| e.slot)
        .expect("a slotted kit") as i32
}

fn id_of(library: &SharedKitLibrary, name: &str) -> String {
    library.read().find(name).expect("kit").id.clone()
}

fn render(plugin: &mut ResonanceDrums, events: &[NoteEvent]) -> f32 {
    let mut buffers: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    {
        let mut ports: Vec<OutputBuffer<'_>> = buffers
            .iter_mut()
            .map(|(l, r)| OutputBuffer {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        let mut iter = EventIterator::new(events);
        plugin.process(&mut ports, BLOCK, &mut iter, None);
    }
    buffers
        .iter()
        .flat_map(|(l, r)| l.iter().chain(r))
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Peak of a full-velocity kick, after silencing what still sounds.
fn kick_peak(plugin: &mut ResonanceDrums) -> f32 {
    plugin.reset();
    render(
        plugin,
        &[NoteEvent::NoteOn {
            note: drum_map::KICK,
            velocity: 1.0,
            timing: 0,
        }],
    )
}

/// Default master and pad volume, between a take and the output.
const CHAIN_GAIN: f32 = 0.8 * 0.8;

/// Whether the kick plays the fixture kit whose take holds `level`.
fn plays_at(plugin: &mut ResonanceDrums, level: f32) -> bool {
    (kick_peak(plugin) - level * CHAIN_GAIN).abs() < 0.005
}

/// Whether the kick sounds, and not as the fixture kit at `level`: the
/// built-in kit is back.
fn plays_built_in_not(plugin: &mut ResonanceDrums, level: f32) -> bool {
    let peak = kick_peak(plugin);
    peak > 0.01 && (peak - level * CHAIN_GAIN).abs() >= 0.005
}

/// Wait until the load in flight has handed `manifest` off (or failed).
fn wait_handed_off(plugin: &ResonanceDrums, manifest: &Path) {
    let bridge = &plugin.bridge;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let pending = bridge.pending_kit.lock().is_some();
        let loaded = bridge.kit_path.lock().as_deref() == Some(manifest);
        if !pending && loaded && matches!(*bridge.kit_status.lock(), KitStatus::Loaded { .. }) {
            return;
        }
        if let KitStatus::Error { message } = &*bridge.kit_status.lock() {
            if !pending {
                panic!("load failed: {message}");
            }
        }
        assert!(Instant::now() < deadline, "the kit never loaded");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn kit_select_text(plugin: &ResonanceDrums) -> String {
    let p = plugin.param(KIT_SELECT);
    p.display(p.get_plain())
}

fn saver_for(plugin: &ResonanceDrums) -> DrumsExtraState {
    DrumsExtraState {
        kit_path: plugin.bridge.kit_path.clone(),
        overhead_setup_key: plugin.bridge.overhead_setup_key.clone(),
        pad_choices: plugin.bridge.pad_choices.clone(),
        params: plugin.bridge.params.clone(),
        reload: Some(plugin.bridge.clone()),
    }
}

// ---------------------------------------------------------------------------
// kit_select
// ---------------------------------------------------------------------------

/// The two params are where the spec puts them, with the CLAP-facing
/// shape it asks for: a stepped selector from "none" over every slot, and
/// a 0..1 output.
#[test]
fn the_selection_params_are_declared_as_specified() {
    let plugin = ResonanceDrums::new();
    let select = plugin.param(KIT_SELECT);
    assert_eq!(select.id(), "kit_select");
    assert_eq!(
        select.min_plain(),
        PARKED_KIT as f64,
        "-2: parked (external or missing)"
    );
    assert_eq!(select.max_plain(), selection::MAX_KIT_SLOT as f64);
    assert!(select.is_stepped());
    assert!(!select.is_automatable(), "a kit swap is no automation lane");
    assert!(select.state_excluded(), "the kit travels as a kit_ref");
    assert!(!select.is_read_only());
    assert_eq!(select.default_plain(), NO_KIT as f64);

    let progress = plugin.param(KIT_LOAD_PROGRESS);
    assert_eq!(progress.id(), "kit_load_progress");
    assert!(progress.is_read_only());
    assert!(!progress.is_automatable());
    assert_eq!((progress.min_plain(), progress.max_plain()), (0.0, 1.0));
}

/// Slots are the library's, and never move: adding a kit leaves the
/// others where they were, deleting one leaves the rest, and a kit added
/// after a delete does not inherit the deleted kit's slot — so a
/// `kit_select` value keeps naming the same kit.
#[test]
fn kit_select_slots_are_stable_across_add_and_delete() {
    let home = Home::new("slots");
    home.kit("Alpha", "Alpha Kit", 0.1);
    home.kit("Bravo", "Bravo Kit", 0.2);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let (a, b) = (slot_of(&lib, "Alpha Kit"), slot_of(&lib, "Bravo Kit"));
    assert_ne!(a, b);
    let text = |v: i32| plugin.param(KIT_SELECT).display(v as f64);
    assert_eq!(text(a), "Alpha Kit");

    home.kit("Charlie", "Charlie Kit", 0.3);
    lib.rescan().unwrap().unwrap();
    assert_eq!(
        (slot_of(&lib, "Alpha Kit"), slot_of(&lib, "Bravo Kit")),
        (a, b)
    );
    let c = slot_of(&lib, "Charlie Kit");

    lib.delete(&home.root().join("Bravo")).unwrap().unwrap();
    assert_eq!(slot_of(&lib, "Alpha Kit"), a);
    assert_eq!(slot_of(&lib, "Charlie Kit"), c);
    assert_eq!(text(b), format!("(empty slot {b})"));

    home.kit("Delta", "Delta Kit", 0.4);
    lib.rescan().unwrap().unwrap();
    let d = slot_of(&lib, "Delta Kit");
    assert!(![a, b, c].contains(&d), "slot {d} was reused");
    assert_eq!(text(a), "Alpha Kit");
    assert_eq!(text(c), "Charlie Kit");
}

/// `kit_select`'s text is the kit's name, and a name parses back to its
/// slot — which is all `ParamValue::Label` needs to pick a kit by name.
/// Both hold on an active plugin too (the bridge's text source).
#[test]
fn kit_select_text_is_the_kit_name_and_a_name_picks_the_slot() {
    let home = Home::new("text");
    home.kit("Alpha", "Alpha Kit", 0.1);
    home.kit("Bravo", "Bravo Kit", 0.2);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let select = plugin.param(KIT_SELECT);
    let b = slot_of(&lib, "Bravo Kit");

    assert_eq!(select.display(b as f64), "Bravo Kit");
    assert_eq!(select.display(NO_KIT as f64), "None (built-in kit)");
    assert_eq!(select.parse("Bravo Kit"), Some(b as f64));
    assert_eq!(select.parse("bravo kit"), Some(b as f64), "case-folded");
    assert_eq!(select.parse("Bravo"), Some(b as f64), "a unique prefix");
    let id = id_of(&lib, "Bravo Kit");
    assert_eq!(select.parse(&id[..10]), Some(b as f64), "an id prefix");
    assert_eq!(select.parse(&format!("slot {b}")), Some(b as f64));
    assert_eq!(select.parse("none"), Some(NO_KIT as f64));
    assert_eq!(select.parse("Kit"), None, "ambiguous");
    assert_eq!(select.parse("Zulu"), None);

    let source = plugin.param_text_source().expect("a text source");
    assert_eq!(
        source.display(KIT_SELECT, b as f64).as_deref(),
        Some("Bravo Kit")
    );
    assert_eq!(source.parse(KIT_SELECT, "Bravo Kit"), Some(b as f64));
}

/// Setting `kit_select` — as a host or the control API does — loads that
/// kit through the watcher, off the audio thread; `kit_load_progress`
/// reaches 1.0 only in the block that takes the kit.
#[test]
fn setting_kit_select_loads_the_kit_and_progress_completes_in_the_block_that_takes_it() {
    let home = Home::new("select");
    let manifest = home.kit("Alpha", "Alpha Kit", 0.25);
    let lib = home.library();
    let mut plugin = booted_on(&lib);
    render(&mut plugin, &[]);
    assert_eq!(
        plugin.param(KIT_LOAD_PROGRESS).get_plain(),
        1.0,
        "the built-in kit is in place"
    );

    let a = slot_of(&lib, "Alpha Kit");
    // A host write, then left to the instance's own watcher thread.
    plugin.param(KIT_SELECT).set_plain(a as f64);
    wait_handed_off(&plugin, &manifest);

    assert!(
        !plugin.bridge.load_progress.is_complete(),
        "handed off, but no block has taken it yet"
    );
    assert!(plugin.param(KIT_LOAD_PROGRESS).get_plain() < 1.0);
    render(&mut plugin, &[]);
    assert_eq!(plugin.param(KIT_LOAD_PROGRESS).get_plain(), 1.0);
    assert!(plays_at(&mut plugin, 0.25), "the kit plays");
    assert_eq!(kit_select_text(&plugin), "Alpha Kit");

    // And "none" goes back to the built-in kit.
    plugin.param(KIT_SELECT).set_plain(NO_KIT as f64);
    assert!(selection::apply_pending(&plugin.bridge).is_ok());
    let deadline = Instant::now() + Duration::from_secs(10);
    while plugin.bridge.builtin_kit.lock().is_none() {
        assert!(
            Instant::now() < deadline,
            "the built-in kit never came back"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    render(&mut plugin, &[]);
    assert_eq!(plugin.param(KIT_LOAD_PROGRESS).get_plain(), 1.0);
    assert!(plugin.bridge.kit_path.lock().is_none());
    assert!(
        plays_built_in_not(&mut plugin, 0.25),
        "the built-in kick plays"
    );
}

/// Before the host activates the plugin a selection is recorded, and the
/// activation loads it.
#[test]
fn a_selection_made_while_inactive_loads_at_activation() {
    let home = Home::new("inactive");
    let manifest = home.kit("Alpha", "Alpha Kit", 0.3);
    let lib = home.library();
    let mut plugin = plugin_on(&lib);
    plugin
        .param(KIT_SELECT)
        .set_plain(slot_of(&lib, "Alpha Kit") as f64);
    selection::apply_pending(&plugin.bridge).unwrap();
    assert_eq!(plugin.bridge.wanted_kit_path(), Some(manifest.clone()));
    assert!(plugin.initialize(RATE, BLOCK as u32));
    wait_handed_off(&plugin, &manifest);
    render(&mut plugin, &[]);
    assert!(plugin.bridge.load_progress.is_complete());
}

// ---------------------------------------------------------------------------
// kit_ref
// ---------------------------------------------------------------------------

/// Resolution order: the id in the library beats `rel_path` beats
/// `abs_path`; and a kit whose folder was renamed is found by its id.
#[test]
fn kit_ref_resolves_by_id_then_rel_path_then_abs_path() {
    let home = Home::new("resolve");
    let x = home.kit("Xray", "Xray Kit", 0.1);
    let y = home.outside_kit("Yankee", "Yankee Kit", 0.2);
    let lib = home.library();
    let root = home.root();
    let x_id = id_of(&lib, "Xray Kit");
    let resolve = |r: &KitRef| r.resolve(Some(&lib.read()), Some(&root));

    let all = KitRef {
        id: Some(x_id.clone()),
        name: Some("Xray Kit".into()),
        rel_path: Some("Nowhere/kit/drum_samples.json".into()),
        abs_path: Some(y.clone()),
    };
    assert_eq!(resolve(&all), Some((x.clone(), ResolvedBy::Id)));

    let rel = KitRef {
        id: Some("0".repeat(64)),
        rel_path: Some("Xray/kit/drum_samples.json".into()),
        ..all.clone()
    };
    assert_eq!(resolve(&rel), Some((x.clone(), ResolvedBy::RelPath)));

    let abs = KitRef {
        rel_path: Some("Nowhere/kit/drum_samples.json".into()),
        ..rel.clone()
    };
    assert_eq!(resolve(&abs), Some((y.clone(), ResolvedBy::AbsPath)));

    let nothing = KitRef {
        abs_path: Some(home.0.join("gone/drum_samples.json")),
        ..abs.clone()
    };
    assert_eq!(resolve(&nothing), None);
    // A rel_path may not climb out of the root.
    let climbing = KitRef {
        id: None,
        rel_path: Some("../elsewhere/Yankee/drum_samples.json".into()),
        abs_path: None,
        name: None,
    };
    assert_eq!(resolve(&climbing), None);

    // Renamed: the old paths are gone, the id still finds it.
    std::fs::rename(root.join("Xray"), root.join("Xray Renamed")).unwrap();
    lib.rescan().unwrap().unwrap();
    let old = KitRef {
        id: Some(x_id),
        name: Some("Xray Kit".into()),
        rel_path: Some("Xray/kit/drum_samples.json".into()),
        abs_path: Some(x),
    };
    assert_eq!(
        resolve(&old),
        Some((
            root.join("Xray Renamed/kit/drum_samples.json"),
            ResolvedBy::Id
        ))
    );
}

/// The whole v1 → v2 migration: `kit_path` becomes a `kit_ref` (with the
/// portable `rel_path` when it lies under the root), `kit_path_fallback`
/// becomes `kit_ref_fallback`, `null` stays "no kit", a v2 key already
/// there wins, and the v1 keys are gone.
#[test]
fn a_v1_state_converts_to_v2() {
    let root = PathBuf::from("/data/resonance/drumkits");
    let mut state = serde_json::json!({
        "params": {},
        "kit_path": "/data/resonance/drumkits/Drummica/drummica/drum_samples.json",
        "kit_path_fallback": "/opt/kits/Other/drum_samples.json",
    });
    selection::upgrade_v1_state(&mut state, Some(&root));
    assert!(state.get("kit_path").is_none() && state.get("kit_path_fallback").is_none());
    assert_eq!(
        state["kit_ref"],
        serde_json::json!({
            "name": "Drummica",
            "rel_path": "Drummica/drummica/drum_samples.json",
            "abs_path": "/data/resonance/drumkits/Drummica/drummica/drum_samples.json",
        })
    );
    assert_eq!(
        state["kit_ref_fallback"],
        serde_json::json!({
            "name": "Other",
            "abs_path": "/opt/kits/Other/drum_samples.json",
        })
    );

    let mut none = serde_json::json!({ "kit_path": null });
    selection::upgrade_v1_state(&mut none, Some(&root));
    assert_eq!(none, serde_json::json!({ "kit_ref": null }));

    let mut both =
        serde_json::json!({ "kit_path": "/a/drum_samples.json", "kit_ref": { "id": "ab" } });
    selection::upgrade_v1_state(&mut both, Some(&root));
    assert_eq!(both, serde_json::json!({ "kit_ref": { "id": "ab" } }));
}

/// A v1 project opens on its kit, `kit_select` names it, and the next
/// save writes the v2 reference — with the library's id — and no
/// `kit_path`. This is what the "Drummica Kit" track preset needs to load
/// once before it is re-saved.
#[test]
fn a_v1_project_opens_and_saves_as_v2() {
    let home = Home::new("v1");
    let manifest = home.kit("Alpha", "Alpha Kit", 0.2);
    let lib = home.library();
    let mut plugin = plugin_on(&lib);
    let v1 = serde_json::json!({
        "version": 1,
        "params": {},
        "kit_path": manifest.to_string_lossy(),
    });
    assert!(plugin.load_state(&serde_json::to_vec(&v1).unwrap()));
    assert_eq!(plugin.bridge.wanted_kit_path(), Some(manifest.clone()));
    assert_eq!(kit_select_text(&plugin), "Alpha Kit");
    assert_eq!(
        plugin.param(KIT_SELECT).get_plain(),
        slot_of(&lib, "Alpha Kit") as f64
    );

    let saved: serde_json::Value = serde_json::from_slice(&plugin.save_state()).unwrap();
    assert!(saved.get("kit_path").is_none(), "{saved}");
    assert!(
        saved["params"].get("kit_select").is_none(),
        "the slot is not state"
    );
    let r = KitRef::from_json(&saved["kit_ref"]).expect("a kit_ref");
    assert_eq!(r.id.as_deref(), Some(id_of(&lib, "Alpha Kit").as_str()));
    assert_eq!(r.name.as_deref(), Some("Alpha Kit"));
    assert_eq!(
        r.rel_path,
        Some(PathBuf::from("Alpha/kit/drum_samples.json"))
    );

    assert!(plugin.initialize(RATE, BLOCK as u32));
    wait_handed_off(&plugin, &manifest);
}

/// A v1 state whose fallback key names the last good kit still reopens on
/// it when the wanted kit is gone (the converted `kit_ref_fallback`).
#[test]
fn a_v1_fallback_still_reopens_on_the_last_good_kit() {
    let home = Home::new("v1fallback");
    let good = home.kit("Alpha", "Alpha Kit", 0.2);
    let lib = home.library();
    let mut plugin = plugin_on(&lib);
    let v1 = serde_json::json!({
        "version": 1,
        "params": {},
        "kit_path": home.root().join("Gone/kit/drum_samples.json").to_string_lossy(),
        "kit_path_fallback": good.to_string_lossy(),
    });
    assert!(plugin.load_state(&serde_json::to_vec(&v1).unwrap()));
    assert_eq!(plugin.bridge.wanted_kit_path(), Some(good.clone()));
    assert!(plugin.bridge.params.selection.missing().is_none());
    assert!(plugin.initialize(RATE, BLOCK as u32));
    wait_handed_off(&plugin, &good);
}

/// A preset carries the kit by reference (`kit_ref` is one of its keys),
/// so it recalls the same kit after the kit's folder was renamed — and
/// `kit_select` follows it to the kit's slot.
#[test]
fn a_preset_recalls_its_kit_by_reference_after_a_rename() {
    let home = Home::new("preset");
    let manifest = home.kit("Alpha", "Alpha Kit", 0.2);
    home.kit("Bravo", "Bravo Kit", 0.3);
    let lib = home.library();
    let src = plugin_on(&lib);
    *src.bridge.kit_path.lock() = Some(manifest);
    let saver = saver_for(&src);
    let keys = saver.preset_keys();
    assert!(
        keys.contains(&"kit_ref") && !keys.contains(&"kit_path"),
        "{keys:?}"
    );
    // The preset form: the saver's output, kept to its preset keys.
    let mut preset = serde_json::Map::new();
    for (k, v) in saver.save() {
        if keys.contains(&k.as_str()) {
            preset.insert(k, v);
        }
    }
    let preset = serde_json::Value::Object(preset);

    std::fs::rename(home.root().join("Alpha"), home.root().join("Alpha (2019)")).unwrap();
    lib.rescan().unwrap().unwrap();
    let moved = home.root().join("Alpha (2019)/kit/drum_samples.json");

    let dst = plugin_on(&lib);
    // Playing another kit when the preset is applied.
    dst.bridge
        .params
        .kit_select
        .set_value(slot_of(&lib, "Bravo Kit"));
    saver_for(&dst).load(&preset);
    assert_eq!(dst.bridge.wanted_kit_path(), Some(moved));
    assert_eq!(
        dst.bridge.params.kit_select.value(),
        slot_of(&lib, "Alpha Kit")
    );
    assert!(dst.bridge.params.selection.missing().is_none());
}

// ---------------------------------------------------------------------------
// Missing kit
// ---------------------------------------------------------------------------

fn missing_ref(home: &Home) -> serde_json::Value {
    serde_json::json!({
        "id": "d".repeat(64),
        "name": "Gone Kit",
        "rel_path": "Gone/kit/drum_samples.json",
        "abs_path": home.0.join("old-machine/Gone/kit/drum_samples.json").to_string_lossy(),
    })
}

/// A reference that resolves to nothing: the built-in kit plays,
/// `kit_select` reads "<name> (missing)" (which is how an agent sees it),
/// and a save writes the reference back unchanged.
#[test]
fn a_missing_kit_plays_the_built_in_kit_says_so_and_is_kept() {
    let home = Home::new("missing");
    let lib = home.library();
    let mut plugin = plugin_on(&lib);
    let state = serde_json::json!({ "version": 1, "params": {}, "kit_ref": missing_ref(&home) });
    assert!(plugin.load_state(&serde_json::to_vec(&state).unwrap()));
    assert!(plugin.bridge.wanted_kit_path().is_none());
    assert_eq!(kit_select_text(&plugin), "Gone Kit (missing)");
    assert_eq!(
        plugin.param(KIT_SELECT).get_plain(),
        PARKED_KIT as f64,
        "parked, not \"built-in\""
    );
    let source = plugin.param_text_source().unwrap();
    assert_eq!(
        source.display(KIT_SELECT, PARKED_KIT as f64).as_deref(),
        Some("Gone Kit (missing)"),
        "the same while the plugin is active"
    );

    assert!(plugin.initialize(RATE, BLOCK as u32));
    render(&mut plugin, &[]);
    assert!(plugin.bridge.load_progress.is_complete());
    assert!(plugin.bridge.builtin_kit.lock().is_some());
    assert!(kick_peak(&mut plugin) > 0.01, "the built-in kit plays");

    let saved: serde_json::Value = serde_json::from_slice(&plugin.save_state()).unwrap();
    assert_eq!(saved["kit_ref"], missing_ref(&home), "kept verbatim");
}

/// Missing while running: a project load whose kit is gone swaps the
/// playing kit out for the built-in one, rather than leaving the old kit
/// under a state that names another.
#[test]
fn a_missing_kit_loaded_while_running_replaces_the_playing_kit() {
    let home = Home::new("missing-active");
    let manifest = home.kit("Alpha", "Alpha Kit", 0.25);
    let lib = home.library();
    let mut plugin = booted_on(&lib);
    plugin
        .param(KIT_SELECT)
        .set_plain(slot_of(&lib, "Alpha Kit") as f64);
    selection::apply_pending(&plugin.bridge).unwrap();
    wait_handed_off(&plugin, &manifest);
    render(&mut plugin, &[]);
    assert!(plays_at(&mut plugin, 0.25));

    let state = serde_json::json!({ "version": 1, "params": {}, "kit_ref": missing_ref(&home) });
    assert!(plugin.load_state(&serde_json::to_vec(&state).unwrap()));
    let deadline = Instant::now() + Duration::from_secs(10);
    while plugin.bridge.builtin_kit.lock().is_none() {
        assert!(Instant::now() < deadline, "the built-in kit never came");
        std::thread::sleep(Duration::from_millis(2));
    }
    render(&mut plugin, &[]);
    assert!(plugin.bridge.load_progress.is_complete());
    assert!(
        plays_built_in_not(&mut plugin, 0.25),
        "kit A no longer plays"
    );
    assert_eq!(kit_select_text(&plugin), "Gone Kit (missing)");
}

/// The editor's banner, over the pad area, with what to do: Locate and
/// Choose always; Download only when the plok.org index has the kit (by
/// its manifest hash). Every button is on screen at both window sizes.
#[test]
fn the_missing_kit_banner_shows_with_its_actions() {
    let home = Home::new("banner");
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let state = serde_json::json!({ "params": {}, "kit_ref": missing_ref(&home) });
    saver_for(&plugin).load(&state);

    for size in [(960.0, 640.0), (780.0, 520.0)] {
        let mut editor = TestEditor::new(&plugin, lib.clone(), size);
        editor.frame(Vec::new());
        let frame = editor.frame(Vec::new());
        assert!(
            frame.shows("⚠ Missing kit \"Gone Kit\" — playing the built-in kit."),
            "no banner at {size:?}: {:?}",
            frame.strings()
        );
        for name in ["missing.banner", "missing.locate", "missing.choose"] {
            let w = frame
                .widget(name)
                .unwrap_or_else(|| panic!("{name} not drawn at {size:?}"));
            let seen = w.rect.intersect(w.clip).intersect(frame.screen);
            assert!(
                seen.width() > 0.0 && seen.height() > 0.0,
                "{name} is not visible at {size:?}"
            );
        }
        for name in ["missing.locate", "missing.choose"] {
            let w = frame.widget(name).unwrap();
            assert!(
                w.clip.expand(1.0).contains_rect(w.rect) && frame.screen.contains_rect(w.rect),
                "{name} is cut off at {size:?}"
            );
        }
        assert!(
            frame.widget("missing.download").is_none(),
            "no index, no download"
        );
        assert!(frame.shows("Gone Kit (missing)"), "the KIT pill names it");
    }

    // With the index offering the kit (by manifest hash), Download shows.
    lib.download().state.lock().index = Some(ServerIndex {
        drumkits: vec![ServerKit {
            manifest_sha256: Some("D".repeat(64)),
            ..ServerKit::new("Renamed On The Server", "gone.zip")
        }],
    });
    let mut editor = TestEditor::new(&plugin, lib.clone(), (960.0, 640.0));
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.widget("missing.download").is_some());
    assert!(frame.shows("Download from plok.org"));

    // Choose another kit opens the Library.
    let at = frame
        .text_center("Choose another kit")
        .expect("Choose button");
    editor.click(at);
    assert!(editor.library_open());
}

/// No banner when nothing is missing.
#[test]
fn no_banner_without_a_missing_kit() {
    let home = Home::new("nobanner");
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let mut editor = TestEditor::new(&plugin, lib, (960.0, 640.0));
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.widget("missing.banner").is_none());
}

/// Locate relinks by hash: a folder holding the same manifest (here, a
/// copy on another drive) is imported into the library and loaded, with
/// no question asked; `kit_select` names it and the banner is gone. A
/// folder with another kit asks first, and loads nothing until told.
#[test]
fn locate_relinks_a_folder_whose_manifest_matches_and_asks_otherwise() {
    let home = Home::new("locate");
    let lib = home.library();
    // The kit, somewhere outside the library, and its id.
    let found = home.outside_kit("Found", "Gone Kit", 0.2);
    let id = resonance_common::drumkit_library::hash_manifest(&found).unwrap();
    let other = home.outside_kit("Other", "Other Kit", 0.3);

    let plugin = plugin_on(&lib);
    let mut r = missing_ref(&home);
    r["id"] = serde_json::json!(id);
    saver_for(&plugin).load(&serde_json::json!({ "params": {}, "kit_ref": r }));
    assert!(plugin.bridge.params.selection.missing().is_some());

    let mut editor = TestEditor::new(&plugin, lib.clone(), (960.0, 640.0));
    // Another kit: asks, loads nothing.
    editor.locate_missing(other.parent().unwrap().to_path_buf());
    editor.finish_jobs();
    assert_eq!(
        editor.missing_mismatch(),
        Some(other.parent().unwrap().to_path_buf())
    );
    assert!(plugin.bridge.wanted_kit_path().is_none());
    assert!(plugin.bridge.params.selection.missing().is_some());

    // The same kit: relinked silently, through an import into the root.
    editor.locate_missing(found.parent().unwrap().to_path_buf());
    editor.finish_jobs();
    assert_eq!(editor.missing_error(), None);
    let wanted = plugin.bridge.wanted_kit_path().expect("the kit is loading");
    assert!(
        wanted.starts_with(home.root()),
        "imported into the library: {wanted:?}"
    );
    assert!(plugin.bridge.params.selection.missing().is_none());
    assert_eq!(
        plugin.bridge.params.kit_select.value(),
        lib.read().entry(&id).and_then(|e| e.slot).unwrap() as i32
    );
    assert_eq!(kit_select_text(&plugin), "Gone Kit");
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(
        frame.widget("missing.banner").is_none(),
        "the banner is gone"
    );
}

// ---------------------------------------------------------------------------
// Acting on kit_select: one at a time
// ---------------------------------------------------------------------------

/// A host write the watcher has not acted on yet, then a state load that
/// names another kit, with the watcher racing the load (review finding
/// 7): the state wins every time. Its kit is the one wanted, its fallback
/// is kept (a watcher acting on the stale write in between used to clear
/// it), `kit_select` reads the state's slot, and the write is not acted
/// on afterwards either.
#[test]
fn a_state_load_racing_the_watcher_is_never_undone_by_a_stale_host_write() {
    let home = Home::new("race");
    let alpha = home.kit("Alpha", "Alpha Kit", 0.1);
    home.kit("Bravo", "Bravo Kit", 0.2);
    let charlie = home.kit("Charlie", "Charlie Kit", 0.3);
    let lib = home.library();
    let (a, b) = (slot_of(&lib, "Alpha Kit"), slot_of(&lib, "Bravo Kit"));
    let state = serde_json::json!({
        "params": {},
        "kit_ref": KitRef::from_manifest_path(&alpha, Some(&home.root())).to_json(),
        "kit_ref_fallback": KitRef::from_manifest_path(&charlie, Some(&home.root())).to_json(),
    });

    for round in 0..200 {
        // Inactive, so nothing decodes: what each side records is all
        // there is to race on.
        let plugin = plugin_on(&lib);
        let bridge = plugin.bridge.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = {
            let (bridge, stop) = (bridge.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = selection::apply_pending(&bridge);
                }
            })
        };
        plugin.param(KIT_SELECT).set_plain(b as f64);
        saver_for(&plugin).load(&state);
        stop.store(true, Ordering::Relaxed);
        watcher.join().unwrap();
        let _ = selection::apply_pending(&bridge);

        assert_eq!(
            bridge.wanted_kit_path(),
            Some(alpha.clone()),
            "round {round}"
        );
        assert_eq!(
            *bridge.kit_fallback.lock(),
            Some(charlie.clone()),
            "round {round}: the state's fallback was cleared"
        );
        assert_eq!(bridge.params.kit_select.value(), a, "round {round}");
        assert_eq!(bridge.params.selection.acted(), a, "round {round}");
    }
}

/// The editor's pick and the watcher on one value: one load, not two.
#[test]
fn an_editor_pick_racing_the_watcher_starts_one_load() {
    let home = Home::new("race-pick");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let lib = home.library();
    let a = slot_of(&lib, "Alpha Kit");
    for round in 0..200 {
        let plugin = plugin_on(&lib);
        let bridge = plugin.bridge.clone();
        let before = bridge.load_generation.load(Ordering::Acquire);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = {
            let (bridge, stop) = (bridge.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = selection::apply_pending(&bridge);
                }
            })
        };
        selection::select_now(&bridge, a).unwrap();
        stop.store(true, Ordering::Relaxed);
        watcher.join().unwrap();
        assert_eq!(
            bridge.load_generation.load(Ordering::Acquire),
            before + 1,
            "round {round}: the pick was acted on twice"
        );
    }
}

// ---------------------------------------------------------------------------
// Parked: -2 is an external or missing kit, -1 is always built-in
// ---------------------------------------------------------------------------

/// Write `value` the way a host does, and let the instance act on it.
fn host_writes(plugin: &ResonanceDrums, value: i32) -> Result<(), String> {
    plugin.param(KIT_SELECT).set_plain(value as f64);
    selection::apply_pending(&plugin.bridge).map(|_| ())
}

/// An external kit (no slot) reads -2 "<name> (external)", which parses
/// back to -2. -1 then really is the built-in kit, a slot is that kit, and
/// writing -2 again — a host's undo of the pick — brings the external kit
/// back.
#[test]
fn an_external_kit_parks_at_minus_two_and_minus_two_brings_it_back() {
    let home = Home::new("parked-external");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let outside = home.outside_kit("Yankee", "Yankee Kit", 0.2);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let state = serde_json::json!({
        "params": {},
        "kit_ref": KitRef::from_manifest_path(&outside, Some(&home.root())).to_json(),
    });
    saver_for(&plugin).load(&state);
    let select = plugin.param(KIT_SELECT);
    assert_eq!(select.get_plain(), PARKED_KIT as f64);
    assert_eq!(kit_select_text(&plugin), "Yankee (external)");
    assert_eq!(select.parse("Yankee (external)"), Some(PARKED_KIT as f64));
    assert_eq!(select.display(NO_KIT as f64), "None (built-in kit)");
    assert!(plugin.bridge.params.selection.missing().is_none());

    host_writes(&plugin, NO_KIT).unwrap();
    assert_eq!(
        plugin.bridge.wanted_kit_path(),
        None,
        "-1 is the built-in kit"
    );
    assert_eq!(
        select.display(PARKED_KIT as f64),
        "Yankee (external)",
        "still remembered"
    );

    let a = slot_of(&lib, "Alpha Kit");
    host_writes(&plugin, a).unwrap();
    assert_eq!(
        plugin.bridge.wanted_kit_path(),
        Some(home.root().join("Alpha/kit/drum_samples.json"))
    );

    // Undo of the pick: the host writes the value from before it.
    host_writes(&plugin, PARKED_KIT).unwrap();
    assert_eq!(plugin.bridge.wanted_kit_path(), Some(outside.clone()));
    assert_eq!(kit_select_text(&plugin), "Yankee (external)");
    // And a save names it.
    let saved = saver_for(&plugin).save();
    let r = KitRef::from_json(&saved["kit_ref"]).unwrap();
    assert_eq!(r.abs_path.as_deref(), Some(outside.as_path()));
}

/// A missing kit is parked too: -2 "<name> (missing)". -1 leaves it for
/// the built-in kit (no banner, a save names no kit); a pick plays the
/// pick; writing -2 goes back to "missing" — banner, saved reference and
/// all.
#[test]
fn a_missing_kit_parks_at_minus_two_and_minus_two_restores_it() {
    let home = Home::new("parked-missing");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let state = serde_json::json!({ "params": {}, "kit_ref": missing_ref(&home) });
    saver_for(&plugin).load(&state);
    let sel = &plugin.bridge.params.selection;
    assert_eq!(plugin.param(KIT_SELECT).get_plain(), PARKED_KIT as f64);
    assert!(sel.missing().is_some());
    assert_eq!(
        plugin.param(KIT_SELECT).parse("Gone Kit (missing)"),
        Some(PARKED_KIT as f64)
    );

    host_writes(&plugin, NO_KIT).unwrap();
    assert!(
        sel.missing().is_none(),
        "the built-in kit, chosen: no banner"
    );
    assert_eq!(
        saver_for(&plugin).save()["kit_ref"],
        serde_json::Value::Null
    );

    host_writes(&plugin, slot_of(&lib, "Alpha Kit")).unwrap();
    assert!(sel.missing().is_none());
    assert!(plugin.bridge.wanted_kit_path().is_some());

    host_writes(&plugin, PARKED_KIT).unwrap();
    assert!(sel.missing().is_some(), "missing again");
    assert_eq!(
        plugin.bridge.wanted_kit_path(),
        None,
        "the built-in kit plays"
    );
    assert_eq!(kit_select_text(&plugin), "Gone Kit (missing)");
    assert_eq!(saver_for(&plugin).save()["kit_ref"], missing_ref(&home));
}

/// -2 with no parked kit remembered names nothing to go back to: the
/// write is refused and what plays keeps playing.
#[test]
fn minus_two_with_nothing_parked_changes_nothing() {
    let home = Home::new("parked-none");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    host_writes(&plugin, slot_of(&lib, "Alpha Kit")).unwrap();
    let wanted = plugin.bridge.wanted_kit_path();
    assert_eq!(
        plugin.param(KIT_SELECT).display(PARKED_KIT as f64),
        "(no external kit)"
    );
    assert_eq!(
        plugin.param(KIT_SELECT).parse("(no external kit)"),
        Some(PARKED_KIT as f64)
    );
    assert!(host_writes(&plugin, PARKED_KIT).is_err());
    assert_eq!(plugin.bridge.wanted_kit_path(), wanted);
}

/// Numbers and names that look like numbers: `"slot N"` and `"(empty slot
/// N)"` are always slot N; a kit's exact name beats a bare number (a kit
/// called "12" is picked by name, as its text reads); otherwise a bare
/// number is that value.
#[test]
fn kit_select_parses_numbers_and_numeric_names_sanely() {
    let home = Home::new("numeric");
    home.kit("Twelve", "12", 0.1);
    home.kit("Alpha", "Alpha Kit", 0.2);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let select = plugin.param(KIT_SELECT);
    let twelve = slot_of(&lib, "12");
    assert_ne!(
        twelve, 12,
        "the fixture needs the name and the slot to differ"
    );
    assert_eq!(select.display(twelve as f64), "12");
    assert_eq!(
        select.parse("12"),
        Some(twelve as f64),
        "the name, as it reads"
    );
    assert_eq!(select.parse("slot 12"), Some(12.0));
    assert_eq!(select.parse("(empty slot 40)"), Some(40.0));
    assert_eq!(select.display(40.0), "(empty slot 40)");
    let alpha = slot_of(&lib, "Alpha Kit");
    assert_eq!(select.parse(&alpha.to_string()), Some(alpha as f64));
    assert_eq!(select.parse("-1"), Some(NO_KIT as f64));
    assert_eq!(select.parse("-2"), Some(PARKED_KIT as f64));
    assert_eq!(select.parse("-3"), None);
    assert_eq!(select.parse("slot -1"), None);
    // Every text a value reads as parses back to it.
    for v in [NO_KIT, PARKED_KIT, alpha, twelve, 40] {
        assert_eq!(
            select.parse(&select.display(v as f64)),
            Some(v as f64),
            "{v}"
        );
    }
}

// ---------------------------------------------------------------------------
// kit_load_progress as the host reads it
// ---------------------------------------------------------------------------

/// Right after a pick — before the watcher acts on it, while the progress
/// still describes the kit before it — `kit_load_progress` reads 0, not
/// the previous kit's 1.0 (review finding 2). The same through every
/// path a host reads it: the parameter (inactive `get_value`) and the
/// text source's `live_value` (active). And `live_value` reads
/// `kit_select` as written, not the bridge's stale mirror.
#[test]
fn kit_load_progress_reads_zero_from_the_pick_until_the_audio_thread_takes_the_kit() {
    let home = Home::new("progress-pick");
    let manifest = home.kit("Alpha", "Alpha Kit", 0.25);
    let lib = home.library();
    let mut plugin = booted_on(&lib);
    render(&mut plugin, &[]);
    let source = plugin.param_text_source().unwrap();
    let live = |i: usize| source.live_value(i).expect("a live value");
    assert_eq!(live(KIT_LOAD_PROGRESS), 1.0);
    assert_eq!(live(KIT_SELECT), NO_KIT as f64);

    let a = slot_of(&lib, "Alpha Kit");
    {
        // Hold the watcher off: the write is not acted on yet.
        let _acting = plugin.bridge.params.selection.acting();
        plugin.param(KIT_SELECT).set_plain(a as f64);
        assert_eq!(plugin.param(KIT_LOAD_PROGRESS).get_plain(), 0.0);
        assert_eq!(live(KIT_LOAD_PROGRESS), 0.0);
        assert_eq!(live(KIT_SELECT), a as f64);
        assert_eq!(plugin.param(KIT_LOAD_PROGRESS).display(0.0), "0%");
    }
    wait_handed_off(&plugin, &manifest);
    assert!(live(KIT_LOAD_PROGRESS) < 1.0, "handed off, not taken");
    assert!(plugin.param(KIT_LOAD_PROGRESS).get_plain() < 1.0);
    render(&mut plugin, &[]);
    assert_eq!(live(KIT_LOAD_PROGRESS), 1.0);
    assert_eq!(plugin.param(KIT_LOAD_PROGRESS).get_plain(), 1.0);
    assert_eq!(plugin.param(KIT_LOAD_PROGRESS).display(1.0), "100%");
}

/// An empty slot loads nothing (what played keeps playing): the progress
/// reads 0 and says why, rather than the 1.0 of the kit still playing.
/// A kit that cannot load reads "failed".
#[test]
fn kit_load_progress_says_empty_slot_and_failed() {
    let home = Home::new("progress-fail");
    home.kit("Alpha", "Alpha Kit", 0.25);
    let broken = home.root().join("Broken/kit");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("drum_samples.json"), "{ not json").unwrap();
    let lib = home.library();
    let plugin = booted_on(&lib);
    let source = plugin.param_text_source().unwrap();
    let progress = plugin.param(KIT_LOAD_PROGRESS);

    let empty = (0..=selection::MAX_KIT_SLOT)
        .find(|s| lib.read().by_slot(*s as u32).is_none())
        .unwrap();
    assert!(host_writes(&plugin, empty).is_err());
    assert_eq!(progress.get_plain(), 0.0);
    assert_eq!(progress.display(0.0), "empty slot");
    assert_eq!(
        source.display(KIT_LOAD_PROGRESS, 0.0).as_deref(),
        Some("empty slot")
    );

    let broken_slot =
        lib.read()
            .entries()
            .iter()
            .find(|e| e.manifest_path.starts_with(&broken))
            .and_then(|e| e.slot)
            .expect("a kit whose manifest does not parse still holds a slot") as i32;
    assert!(host_writes(&plugin, broken_slot).is_err());
    assert_eq!(progress.get_plain(), 0.0);
    assert_eq!(progress.display(0.0), "failed");

    // A kit that loads clears it.
    host_writes(&plugin, slot_of(&lib, "Alpha Kit")).unwrap();
    assert_eq!(progress.display(0.0), "0%");
}

/// `kit_select` as the host reads it while active (`live_value`) follows
/// a state load (the kit's slot, or -2 for a missing kit) — the bridge's
/// mirror would not until a block ran (review finding 3).
#[test]
fn live_kit_select_follows_a_state_load() {
    let home = Home::new("live-state");
    let alpha = home.kit("Alpha", "Alpha Kit", 0.25);
    let lib = home.library();
    let plugin = booted_on(&lib);
    let source = plugin.param_text_source().unwrap();
    let state = serde_json::json!({
        "params": {},
        "kit_ref": KitRef::from_manifest_path(&alpha, Some(&home.root())).to_json(),
    });
    saver_for(&plugin).load(&state);
    assert_eq!(
        source.live_value(KIT_SELECT),
        Some(slot_of(&lib, "Alpha Kit") as f64)
    );
    assert!(
        source.live_value(KIT_LOAD_PROGRESS).unwrap() < 1.0,
        "loading"
    );
    let missing = serde_json::json!({ "params": {}, "kit_ref": missing_ref(&home) });
    saver_for(&plugin).load(&missing);
    assert_eq!(source.live_value(KIT_SELECT), Some(PARKED_KIT as f64));
}

/// The host is asked to re-read the progress at the start of a load, at
/// half-way and at the end — not per file (review finding 8) — and with
/// its text when it turns to (or from) "failed".
#[test]
fn progress_is_reported_at_start_half_and_done_only() {
    use selection::{note_progress, progress_worth_reporting, KitSelection, ProgressReport};
    assert!(progress_worth_reporting(1.0, 0.0), "a load starts");
    assert!(!progress_worth_reporting(0.0, 0.3));
    assert!(progress_worth_reporting(0.3, 0.55), "half-way");
    assert!(!progress_worth_reporting(0.55, 0.999));
    assert!(progress_worth_reporting(0.999, 1.0), "done");
    assert!(!progress_worth_reporting(1.0, 1.0));

    let sel = KitSelection::new();
    let reports: Vec<_> = [0.0, 0.1, 0.2, 0.5, 0.7, 0.999, 1.0, 1.0]
        .iter()
        .filter_map(|&v| note_progress(&sel, v, false))
        .collect();
    assert_eq!(reports, vec![ProgressReport::Values; 3]);
    assert_eq!(note_progress(&sel, 0.0, true), Some(ProgressReport::Text));
    assert_eq!(note_progress(&sel, 0.0, true), None);
    assert_eq!(note_progress(&sel, 0.0, false), Some(ProgressReport::Text));
}

/// The host's render mode reaches the streaming sampler: offline (a
/// bounce) waits for the disk reader, realtime never does.
#[test]
fn the_host_render_mode_reaches_the_sampler() {
    use resonance_drums::stream::{HOST_RENDER_OFFLINE, HOST_RENDER_REALTIME, HOST_RENDER_UNKNOWN};
    let mut plugin = ResonanceDrums::new();
    let mode = plugin.bridge.host_render_mode.clone();
    assert_eq!(mode.load(Ordering::Relaxed), HOST_RENDER_UNKNOWN);
    plugin.set_render_mode(true);
    assert_eq!(mode.load(Ordering::Relaxed), HOST_RENDER_OFFLINE);
    plugin.set_render_mode(false);
    assert_eq!(mode.load(Ordering::Relaxed), HOST_RENDER_REALTIME);
}

// ---------------------------------------------------------------------------
// Host undo: a user's pick is one edit, a state load is none
// ---------------------------------------------------------------------------

fn edits(plugin: &ResonanceDrums) -> u64 {
    plugin
        .bridge
        .host_asks
        .kit_select_edits
        .load(Ordering::Relaxed)
}

/// The editor's Library Load (here: an import, which loads the kit it
/// added through the same entry point as Load) is announced to the host
/// as one undoable `kit_select` edit. A state load moving `kit_select` —
/// which is what a host's undo of that edit is, in Resonance — and a host
/// write are not: a rescan at most.
#[test]
fn a_library_load_is_one_host_edit_and_a_state_load_is_none() {
    let home = Home::new("announce");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let outside = home.outside_kit("Gamma", "Gamma Kit", 0.3);
    let lib = home.library();
    let plugin = booted_on(&lib);
    let mut editor = TestEditor::new(&plugin, lib.clone(), (960.0, 640.0));
    editor.picked_for_import(outside.parent().unwrap().to_path_buf());
    editor.finish_jobs();
    let gamma = slot_of(&lib, "Gamma Kit");
    assert_eq!(plugin.bridge.params.kit_select.value(), gamma, "loaded");
    assert_eq!(edits(&plugin), 1, "one undoable edit");

    let rescans = plugin
        .bridge
        .host_asks
        .value_rescans
        .load(Ordering::Relaxed);
    let state = serde_json::json!({
        "params": {},
        "kit_ref": KitRef::from_manifest_path(
            &home.root().join("Alpha/kit/drum_samples.json"),
            Some(&home.root())
        )
        .to_json(),
    });
    saver_for(&plugin).load(&state);
    assert_eq!(
        plugin.bridge.params.kit_select.value(),
        slot_of(&lib, "Alpha Kit")
    );
    assert_eq!(edits(&plugin), 1, "a state load is no edit");
    assert!(
        plugin
            .bridge
            .host_asks
            .value_rescans
            .load(Ordering::Relaxed)
            > rescans,
        "but the host re-reads the value"
    );

    host_writes(&plugin, gamma).unwrap();
    assert_eq!(edits(&plugin), 1, "the host's own write is no new edit");
}

/// The kit in a slot changed (removed here): `kit_select`'s value is
/// unchanged and its text is not, so the watcher asks the host to re-read
/// the text.
#[test]
fn a_library_change_asks_the_host_to_re_read_kit_select_text() {
    let home = Home::new("rename-text");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let a = slot_of(&lib, "Alpha Kit");
    // Let the watcher see the library once.
    std::thread::sleep(Duration::from_millis(250));
    let before = plugin.bridge.host_asks.text_rescans.load(Ordering::Relaxed);
    lib.delete(&home.root().join("Alpha")).unwrap().unwrap();
    assert_eq!(
        plugin.param(KIT_SELECT).display(a as f64),
        format!("(empty slot {a})")
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while plugin.bridge.host_asks.text_rescans.load(Ordering::Relaxed) == before {
        assert!(Instant::now() < deadline, "no text rescan");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The banner's Download offer from the index cached on disk is looked up
/// once, not every frame (review finding 9): with the cache gone from
/// disk the offer stays — it was not read again.
#[test]
fn the_banner_reads_the_cached_index_once() {
    let home = Home::new("banner-cache");
    let lib = home.library();
    let plugin = plugin_on(&lib);
    saver_for(&plugin).load(&serde_json::json!({ "params": {}, "kit_ref": missing_ref(&home) }));
    assert!(lib.download().state.lock().index.is_none());
    let index = serde_json::json!({ "drumkits": [{
        "name": "Gone Kit", "file": "gone.zip", "manifest_sha256": "d".repeat(64),
    }] });
    let cache = home
        .root()
        .join(resonance_drums::download::INDEX_CACHE_FILE);
    std::fs::write(&cache, serde_json::to_vec(&index).unwrap()).unwrap();

    let mut editor = TestEditor::new(&plugin, lib.clone(), (960.0, 640.0));
    editor.frame(Vec::new());
    assert!(editor
        .frame(Vec::new())
        .widget("missing.download")
        .is_some());
    std::fs::remove_file(&cache).unwrap();
    for _ in 0..3 {
        assert!(
            editor
                .frame(Vec::new())
                .widget("missing.download")
                .is_some(),
            "the index was read from disk again"
        );
    }
}

// ---------------------------------------------------------------------------
// The library behind kit_select, with no editor open (review finding 10)
// ---------------------------------------------------------------------------

/// A name this process's index does not know may be a kit another process
/// (another drums instance's editor, in another host) indexed since: the
/// lookup re-reads the index once before it gives up.
#[test]
fn a_name_lookup_miss_re_reads_an_index_another_process_rewrote() {
    let home = Home::new("reload-miss");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let lib = home.library();
    let plugin = plugin_on(&lib);
    assert_eq!(plugin.param(KIT_SELECT).parse("Bravo Kit"), None);

    // "Another process": its own library over the same root scans a kit
    // in and writes the index.
    home.kit("Bravo", "Bravo Kit", 0.2);
    let other = home.library();
    let b = slot_of(&other, "Bravo Kit");
    assert!(lib.read().find("Bravo Kit").is_none(), "not re-read yet");

    assert_eq!(plugin.param(KIT_SELECT).parse("Bravo Kit"), Some(b as f64));
    assert_eq!(plugin.param(KIT_SELECT).display(b as f64), "Bravo Kit");
}

/// The process-wide library, opened by an instance with no editor, scans
/// once in the background: a kit dropped into the root by hand (never
/// indexed) is found without an editor ever opening.
#[test]
fn the_shared_library_scans_once_when_first_opened() {
    let home = Home::new("first-open");
    home.kit("Alpha", "Alpha Kit", 0.1);
    let lib = resonance_drums::library::shared_for(Roots {
        root: Some(home.root()),
        marks_dir: Some(home.0.join("library")),
        installed_json: None,
        worker: WorkerConfig {
            index_url: "http://127.0.0.1:9/index.json".into(),
            ..WorkerConfig::default()
        },
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while lib.read().find("Alpha Kit").is_none() {
        assert!(Instant::now() < deadline, "the library was never scanned");
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---------------------------------------------------------------------------
// Status bar: disk streaming
// ---------------------------------------------------------------------------

/// The status bar shows the streams' ring memory beside the samples', and
/// the underrun count once there is one.
#[test]
fn the_status_bar_shows_stream_memory_and_underruns() {
    let home = Home::new("status-stream");
    let lib = home.library();
    let plugin = plugin_on(&lib);
    let mut editor = TestEditor::new(&plugin, lib, (960.0, 640.0));
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(!frame.shows("STREAM"), "no rings in use, no readout");
    assert!(!frame.strings().iter().any(|s| s.contains("underrun")));

    plugin
        .bridge
        .stream_ring_bytes
        .store(3 * 1024 * 1024, Ordering::Relaxed);
    plugin.bridge.stream_underruns.store(3, Ordering::Relaxed);
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("STREAM"), "{:?}", frame.strings());
    assert!(
        frame.strings().iter().any(|s| s.contains("MB")),
        "{:?}",
        frame.strings()
    );
    assert!(frame.shows("3 underruns"), "{:?}", frame.strings());
}
