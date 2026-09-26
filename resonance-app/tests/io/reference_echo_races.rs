//! Reference A/B id bookkeeping across restores and in-flight echoes
//! (FU-A5a..c, found during ARCH-01 A-5).
//!
//! These drive the app against the engine's real `ReferencePlayer`
//! handlers: every `AudioCommand` the app sends is applied to a bare
//! player, and every event the player emits is folded back into the app.
//! The analysis worker is not run — a test plays its part by folding the
//! `ReferenceAnalysisProgress` / `ReferenceLoaded` events itself, at the
//! moment it wants them to land.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossbeam_channel::{unbounded, Receiver as EventRx, Sender as EventTx};

use resonance_app::message::Message;
use resonance_app::project::{LoadedProject, ProjectReference, ProjectReferenceMarker};
use resonance_app::reference::ReferenceMessage;
use resonance_app::update::project_io::BuiltinTemplateId;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ReferenceId};
use resonance_audio::{
    handle_add_ref_marker, handle_remove_ref_marker, handle_remove_reference_track,
    register_reference, ReferencePlayer,
};

/// The engine side: a bare `ReferencePlayer` driven by the commands the
/// app sends.
struct Engine {
    player: ReferencePlayer,
    tx: EventTx<AudioEvent>,
    rx: EventRx<AudioEvent>,
    /// `(id, path)` of every load the player registered, in order.
    loads: Vec<(ReferenceId, String)>,
}

impl Engine {
    fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            player: ReferencePlayer::new(),
            tx,
            rx,
            loads: Vec::new(),
        }
    }

    fn apply(&mut self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::ClearAll => {
                self.player.clear();
                let _ = self.tx.send(AudioEvent::AllCleared);
            }
            AudioCommand::LoadReferenceTrack { id_hint, path } => {
                let id = register_reference(&mut self.player, id_hint, path.clone());
                self.loads.push((id, path.to_string_lossy().into_owned()));
            }
            AudioCommand::RemoveReferenceTrack { id } => {
                handle_remove_reference_track(&mut self.player, &self.tx, id)
            }
            AudioCommand::AddRefMarker {
                ref_id,
                marker_id,
                position_samples,
                label,
            } => handle_add_ref_marker(
                &mut self.player,
                &self.tx,
                ref_id,
                marker_id,
                position_samples,
                label,
            ),
            AudioCommand::RemoveRefMarker { ref_id, marker_id } => {
                handle_remove_ref_marker(&mut self.player, &self.tx, ref_id, marker_id)
            }
            _ => {}
        }
    }

    /// Whether the player still holds `id`.
    fn holds(&self, id: ReferenceId) -> bool {
        self.player.entry_integrated_lufs(id).is_some()
    }

    fn last_load(&self) -> ReferenceId {
        self.loads.last().expect("a load reached the engine").0
    }
}

struct Fixture {
    app: Resonance,
    cmds: Receiver<AudioCommand>,
    engine: Engine,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "resonance-reference-echo-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture dir");
        let (mut app, _task, cmds) = Resonance::new_for_test_with_capture();
        app.test_set_active_project(true);
        app.test_set_project_path(root.join("song.rproj"));
        let mut f = Self {
            app,
            cmds,
            engine: Engine::new(),
            root,
        };
        f.pump();
        f
    }

    /// An (empty) audio file the reference loader can find.
    fn file(&self, name: &str) -> PathBuf {
        let path = self.root.join(format!("{name}.wav"));
        std::fs::write(&path, b"").expect("write reference file");
        path
    }

    /// Run the app's commands through the engine and its events back
    /// into the app until neither side has anything left to say.
    fn pump(&mut self) {
        for _ in 0..16 {
            let cmds: Vec<AudioCommand> = self.cmds.try_iter().collect();
            for cmd in cmds {
                self.engine.apply(cmd);
            }
            let events: Vec<AudioEvent> = self.engine.rx.try_iter().collect();
            if events.is_empty() {
                return;
            }
            for ev in events {
                self.app.test_apply_engine_event(ev);
            }
        }
        panic!("app and engine never went quiet");
    }

    /// A user gesture (recorded in the undo history), then the engine.
    fn user(&mut self, m: ReferenceMessage) {
        let _ = self.app.update(Message::Reference(m));
        self.pump();
    }

    fn undo(&mut self) {
        let _ = self.app.update(Message::Undo);
        self.pump();
    }

    /// Fold an event as the analysis worker would send it.
    fn worker(&mut self, ev: AudioEvent) {
        self.app.test_apply_engine_event(ev);
        self.pump();
    }

    /// Open a project holding `refs` from disk, as `Open…` does.
    fn open(&mut self, refs: Vec<ProjectReference>) {
        let mut file = BuiltinTemplateId::Empty.build().file;
        file.references = refs;
        self.app.test_replay_loaded_project_from(LoadedProject {
            file,
            project_dir: self.root.join("song.rproj"),
            midi_notes: HashMap::new(),
            plugin_states: HashMap::new(),
        });
        self.pump();
    }

    fn marker_ids(&self, path: &Path) -> Vec<u32> {
        let path = path.to_string_lossy();
        self.app
            .test_reference()
            .entries
            .iter()
            .find(|e| e.path == path)
            .expect("reference is listed")
            .markers
            .iter()
            .map(|m| m.id)
            .collect()
    }
}

fn saved(path: &Path, markers: &[(u32, &str)]) -> ProjectReference {
    ProjectReference {
        path: path.to_string_lossy().into_owned(),
        name: path.file_stem().unwrap().to_string_lossy().into_owned(),
        integrated_lufs: -10.0,
        markers: markers
            .iter()
            .map(|&(id, label)| ProjectReferenceMarker {
                id,
                position_samples: u64::from(id) * 48_000,
                label: label.into(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// FU-A5a — marker ids
// ---------------------------------------------------------------------------

/// Reopening a project brings a reference back with its saved markers,
/// but the engine re-registers it with an empty marker list. A marker
/// the user then adds must get an id of its own; before FU-A5a the
/// engine numbered it 1 again, and the app dropped the echo as a
/// duplicate of the restored marker 1 — the new marker never appeared.
#[test]
fn a_marker_added_after_reopening_does_not_collide_with_a_restored_one() {
    let mut f = Fixture::new("marker-reopen");
    let a = f.file("a");
    f.open(vec![saved(&a, &[(1, "intro"), (2, "drop")])]);
    assert_eq!(f.marker_ids(&a), vec![1, 2], "saved markers restored");
    let id = f.app.test_reference().entries[0].id;

    f.user(ReferenceMessage::AddMarker {
        ref_id: id,
        position_samples: 5 * 48_000,
        label: "chorus".into(),
    });

    let ids = f.marker_ids(&a);
    assert_eq!(ids.len(), 3, "the new marker is listed: {ids:?}");
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 3, "marker ids are distinct: {ids:?}");
    let entry = &f.app.test_reference().entries[0];
    assert!(
        entry.markers.iter().any(|m| m.label == "chorus"),
        "the new marker keeps its label"
    );

    // Removing the new one removes only it.
    let new_id = entry.markers.iter().find(|m| m.label == "chorus").unwrap().id;
    f.user(ReferenceMessage::RemoveMarker {
        ref_id: id,
        marker_id: new_id,
    });
    assert_eq!(f.marker_ids(&a), vec![1, 2]);
}
