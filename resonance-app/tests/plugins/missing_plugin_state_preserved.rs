//! Regression: a plugin that is missing on *this* machine must not lose
//! its settings when the project is saved here (ba doc #275, finding P5 —
//! the one finding in the plugin audit that destroys user work).
//!
//! The reproduction, in full:
//!
//! ```text
//! machine A: a project with a third-party reverb, carefully dialled in
//! machine B: the .clap isn't installed
//!            -> AddPlugin fails, the engine answers AudioEvent::Error and
//!               never sends PluginAdded
//!            -> the chain keeps a placeholder slot with an empty param
//!               mirror and no live instance behind it
//!            -> File > Save As
//!            -> SaveAllPluginStates reports blobs for live instances only,
//!               so nothing is written for that slot, and serialization
//!               diffs an empty param mirror against nothing, so `params`
//!               is written as []
//! machine A: reopen -> the reverb is back at its factory defaults
//! ```
//!
//! No error, no badge, no prompt at any point. The fix keeps both halves
//! of the plugin's saved state app-side across the load — the opaque CLAP
//! blob in `plugin_mirror.state_cache`, the parameter values in
//! `pending_plugin_param_overrides` — and every path that writes project
//! state reads them back out.
//!
//! What is asserted here is byte-identity, not merely "something was
//! written": an opaque blob is opaque, so the only correct treatment of a
//! blob we cannot interpret is to hand back exactly the bytes we were
//! given.
//!
//! The UX half of P5 (a missing/offline badge, relocate, replace) is a
//! separate todo; this file is only about not destroying data.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use resonance_app::project::{self, LoadedProject, ProjectFile};
use resonance_app::state::PluginSlotState;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, TrackType};
use crate::common::app_active_project as app;

const TRACK: u64 = 42;
const INSTANCE: u64 = 900;
const PLUGIN_NAME: &str = "MegaVerb";
const PLUGIN_ID: &str = "com.thirdparty.mega-verb";
const CLAP_PATH: &str = "/opt/clap/MegaVerb.clap";

const MIX_ID: u32 = 1;
const DECAY_ID: u32 = 2;

/// Deliberately not valid UTF-8 and not all-printable: a CLAP state blob
/// is arbitrary bytes, and any transformation on the way through — a
/// lossy string conversion, a re-encode, a truncation at a NUL — has to
/// show up as a failure here.
const BLOB: &[u8] = &[
    0x00, 0x01, 0xff, 0xfe, 0x7f, 0x80, b'M', b'V', 0x00, 0xde, 0xad, 0xbe, 0xef, 0x0a, 0xc0, 0xff,
];

/// A unique temp directory that deletes itself when dropped. Mirrors the
/// helper in `autosave_write.rs` / `project_atomic_write.rs` (no
/// `tempfile` dependency in this crate).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "resonance_missing_plugin_{tag}_{}_{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The plugin's parameters as the engine reports them at instantiation —
/// i.e. at their defaults — with the two the user moved already applied
/// to the mirror, as a live session would have them.
fn params_as_dialled_in() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: MIX_ID,
            name: "Mix".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.5,
            current_value: 0.83,
            ..Default::default()
        },
        ParamInfo {
            id: DECAY_ID,
            name: "Decay".to_owned(),
            min_value: 0.1,
            max_value: 20.0,
            default_value: 2.0,
            current_value: 11.25,
            ..Default::default()
        },
    ]
}

/// Machine A: a saved project whose track carries the reverb, with the
/// plugin live and its opaque state captured. Returns the project file as
/// it reached disk.
fn author_project_on_machine_a(dir: &Path) -> ProjectFile {
    let mut app = app();
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            PLUGIN_NAME.to_owned(),
            PLUGIN_ID.to_owned(),
            CLAP_PATH.to_owned(),
            params_as_dialled_in(),
            false,
        ),
    );

    let file = app.test_build_project_file();
    project::save_project(dir, &file, &[(INSTANCE, BLOB.to_vec())], &[]).expect("save project");
    file
}

/// Machine B: the `.clap` is not installed, so the load replays the chain
/// but no `PluginAdded` ever comes back for it. Returns the app in
/// exactly the state a user would be looking at.
fn open_on_machine_without_the_plugin(dir: &Path) -> (Resonance, LoadedProject) {
    let loaded = project::load_project(dir).expect("load project");
    assert_eq!(
        loaded.plugin_states.get(&INSTANCE).map(|b| &b[..]),
        Some(BLOB),
        "precondition: the blob is on disk and was read back"
    );

    let mut app = app();
    // No `AudioEvent::PluginAdded` is fed in afterwards — that echo is
    // exactly what a machine without the plugin never produces.
    app.test_replay_loaded_project_from(loaded.clone());
    (app, loaded)
}

fn saved_plugin<'a>(file: &'a ProjectFile) -> &'a project::ProjectPlugin {
    &file
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .expect("the track survives")
        .plugins[0]
}

// ---------------------------------------------------------------------------

/// The headline case: load on a machine without the plugin, Save As, and
/// the opaque blob must come back byte-identical.
#[test]
fn opaque_state_of_a_missing_plugin_survives_save_as_byte_for_byte() {
    let machine_a = TempDir::new("origin");
    let saved_as = TempDir::new("saveas");
    let authored = author_project_on_machine_a(machine_a.path());

    let (app, _loaded) = open_on_machine_without_the_plugin(machine_a.path());

    // The slot is there but hollow: no live instance ever reported a
    // parameter list for it. This is the state the old code serialized
    // from, and why it wrote nothing.
    let file = app.test_build_project_file();
    assert_eq!(
        app.test_track_plugin_instance_ids(TRACK),
        vec![INSTANCE],
        "the placeholder slot stays in the chain"
    );

    // Save As. The engine reports no blobs at all — it has no instance to
    // save state from.
    let states = app.test_plugin_states_for_save(Vec::new());
    project::save_project(saved_as.path(), &file, &states, &[]).expect("save as");

    // Back on machine A, where the plugin exists.
    let reopened = project::load_project(saved_as.path()).expect("reopen the saved-as copy");
    assert_eq!(
        reopened.plugin_states.get(&INSTANCE).map(|b| &b[..]),
        Some(BLOB),
        "the missing plugin's opaque state must round-trip byte-identically"
    );

    // The explicit parameter overrides are the other half of the plugin's
    // saved state and must survive the same trip.
    assert_eq!(
        saved_plugin(&reopened.file).params,
        saved_plugin(&authored).params,
        "the saved parameter values must round-trip unchanged"
    );
    assert!(
        !saved_plugin(&reopened.file).params.is_empty(),
        "sanity: the fixture really did carry non-default parameters"
    );

    // Identity is untouched too, so the reopened project still knows what
    // to instantiate.
    assert_eq!(saved_plugin(&reopened.file).clap_plugin_id, PLUGIN_ID);
    assert_eq!(saved_plugin(&reopened.file).clap_file_path, CLAP_PATH);
    assert_eq!(saved_plugin(&reopened.file).plugin_name, PLUGIN_NAME);
}

/// Save As is not a one-shot: a user can work on the plugin-less machine
/// for days. The blob must not decay across repeated open/save cycles.
#[test]
fn the_blob_survives_repeated_save_reopen_cycles_without_the_plugin() {
    let machine_a = TempDir::new("cycles");
    author_project_on_machine_a(machine_a.path());

    let mut source = machine_a.path().to_path_buf();
    let hops: Vec<TempDir> = (0..3).map(|n| TempDir::new(&format!("hop{n}"))).collect();
    for hop in &hops {
        let (app, _loaded) = open_on_machine_without_the_plugin(&source);
        let file = app.test_build_project_file();
        let states = app.test_plugin_states_for_save(Vec::new());
        project::save_project(hop.path(), &file, &states, &[]).expect("save as");
        source = hop.path().to_path_buf();
    }

    let final_load = project::load_project(&source).expect("load the last hop");
    assert_eq!(
        final_load.plugin_states.get(&INSTANCE).map(|b| &b[..]),
        Some(BLOB),
        "three save/reopen cycles on a machine without the plugin must not erode the blob"
    );
    assert_eq!(saved_plugin(&final_load.file).params.len(), 2);
}

/// The autosave/backup path shares the save collector's blob list, so the
/// recovery snapshot it writes has to carry the missing plugin's state
/// too — otherwise recovering from a crash is itself the data loss.
#[test]
fn the_autosave_snapshot_carries_the_missing_plugins_state() {
    let machine_a = TempDir::new("autosave_origin");
    author_project_on_machine_a(machine_a.path());

    let (app, _loaded) = open_on_machine_without_the_plugin(machine_a.path());
    let scratch = TempDir::new("autosave_scratch");
    let file = app.test_build_project_file();
    let states = app.test_plugin_states_for_save(Vec::new());
    project::save_autosave(scratch.path(), &file, &states, &[]).expect("autosave");

    // The autosave keeps its blobs under `autosave/` (code review STATE-11).
    let blob = std::fs::read(
        scratch
            .path()
            .join("autosave")
            .join("plugins")
            .join(format!("plugin_{INSTANCE}.bin")),
    )
    .expect("the autosave bundle wrote the plugin blob");
    assert_eq!(blob, BLOB, "autosave must preserve the blob byte-for-byte");
}

/// Template capture writes the same on-disk shape from the same app-side
/// blobs. A template made on the plugin-less machine must not be a
/// template of the plugin's defaults.
#[test]
fn template_capture_carries_the_missing_plugins_state() {
    let machine_a = TempDir::new("template_origin");
    author_project_on_machine_a(machine_a.path());

    let (app, _loaded) = open_on_machine_without_the_plugin(machine_a.path());
    let templates = TempDir::new("templates");
    let folder = app
        .test_save_as_template_in(templates.path(), "Reverb bed", "captured without the plugin")
        .expect("write template");

    let blob = std::fs::read(folder.join("plugins").join(format!("plugin_{INSTANCE}.bin")))
        .expect("the template bundle wrote the plugin blob");
    assert_eq!(blob, BLOB, "template capture must preserve the blob");

    let json = std::fs::read_to_string(folder.join("project.json")).expect("template project.json");
    let captured: ProjectFile = serde_json::from_str(&json).expect("parse template project.json");
    assert_eq!(
        saved_plugin(&captured).params.len(),
        2,
        "the template keeps the plugin's parameter values, not its defaults"
    );
}

/// The point of preserving the bytes: put the plugin back and the
/// settings come back. Both halves have to reach the engine — the opaque
/// blob and the explicit parameter values.
#[test]
fn reinstalling_the_plugin_restores_the_original_settings() {
    let machine_a = TempDir::new("restore_origin");
    let saved_as = TempDir::new("restore_saveas");
    author_project_on_machine_a(machine_a.path());

    // A round trip through the machine that doesn't have the plugin.
    {
        let (app, _loaded) = open_on_machine_without_the_plugin(machine_a.path());
        let file = app.test_build_project_file();
        let states = app.test_plugin_states_for_save(Vec::new());
        project::save_project(saved_as.path(), &file, &states, &[]).expect("save as");
    }

    // Back home, with the plugin installed again.
    let reopened = project::load_project(saved_as.path()).expect("reopen");
    let mut app = app();
    let commands = app.test_capture_engine();
    app.test_replay_loaded_project_from(reopened);
    // This time the engine can instantiate it, and reports the parameter
    // list as instantiated — i.e. at the plugin's own defaults.
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: INSTANCE,
        plugin_name: PLUGIN_NAME.to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        clap_file_path: CLAP_PATH.to_owned(),
        params: vec![
            ParamInfo {
                current_value: 0.5,
                ..params_as_dialled_in()[0].clone()
            },
            ParamInfo {
                current_value: 2.0,
                ..params_as_dialled_in()[1].clone()
            },
        ],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });

    let mut loaded_blob: Option<Vec<u8>> = None;
    let mut params_pushed: Vec<(u32, f64)> = Vec::new();
    while let Ok(cmd) = commands.try_recv() {
        match cmd {
            AudioCommand::LoadPluginState { instance_id, data } if instance_id == INSTANCE => {
                loaded_blob = Some(data);
            }
            AudioCommand::SetPluginParam {
                instance_id,
                param_id,
                value,
            } if instance_id == INSTANCE => params_pushed.push((param_id, value)),
            _ => {}
        }
    }

    assert_eq!(
        loaded_blob.as_deref(),
        Some(BLOB),
        "the reinstated plugin is handed the original bytes"
    );
    params_pushed.sort_by_key(|(id, _)| *id);
    assert_eq!(
        params_pushed,
        vec![(MIX_ID, 0.83), (DECAY_ID, 11.25)],
        "the dialled-in values are re-applied over the plugin's defaults"
    );
    assert_eq!(app.test_plugin_param(INSTANCE, MIX_ID), Some(0.83));
    assert_eq!(app.test_plugin_param(INSTANCE, DECAY_ID), Some(11.25));
}

/// A live plugin's own freshly-saved blob must still win over the copy
/// kept from the file — the preserved bytes are a fallback, not an
/// override, or every save would write the state the project was opened
/// with instead of the state the user just edited.
#[test]
fn a_live_plugins_fresh_blob_wins_over_the_preserved_copy() {
    let machine_a = TempDir::new("live_origin");
    author_project_on_machine_a(machine_a.path());

    let (app, _loaded) = open_on_machine_without_the_plugin(machine_a.path());
    let fresh: Vec<u8> = vec![0x11, 0x22, 0x33];
    let states = app.test_plugin_states_for_save(vec![(INSTANCE, fresh.clone())]);

    assert_eq!(
        states,
        vec![(INSTANCE, fresh)],
        "an instance the engine reported is written once, from the engine's bytes"
    );
}

/// A blob must not outlive the slot that referenced it: writing
/// `plugin_*.bin` for a plugin no longer in any chain would leave the
/// bundle carrying files nothing points at.
#[test]
fn no_blob_is_written_for_a_slot_that_is_gone() {
    let machine_a = TempDir::new("removed_origin");
    author_project_on_machine_a(machine_a.path());

    let (mut app, _loaded) = open_on_machine_without_the_plugin(machine_a.path());
    assert_eq!(app.test_plugin_states_for_save(Vec::new()).len(), 1);

    // The engine confirms the removal the way it would for any plugin.
    app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: INSTANCE,
    });

    assert!(
        app.test_plugin_states_for_save(Vec::new()).is_empty(),
        "a removed slot leaves no blob behind"
    );
    let file = app.test_build_project_file();
    assert!(
        file.tracks
            .iter()
            .find(|t| t.id == TRACK)
            .expect("track")
            .plugins
            .is_empty(),
        "and no plugin entry either"
    );
}
