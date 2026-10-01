//! The process-wide kit library (`resonance_drums::library`,
//! drums-plugin-rework.md §3.4, §3.5): what the plok.org tab offers for a
//! kit, rescans skipped behind a busy writer, the `installed.json`
//! migration's index fetch, and the "used by N instances" count.
//!
//! Every test builds its own library at a temp root; nothing here touches
//! the real data dir or the network (an index URL is a loopback listener
//! that never answers).
#![cfg(feature = "editor")]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_common::drumkit_library::{read_sidecar, write_sidecar, Sidecar, SOURCE_PLOK};
use resonance_drums::download::{ServerIndex, ServerKit, WorkerConfig};
use resonance_drums::library::{self, PlokRowState, Roots, SharedKitLibrary};

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-drums-shared-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("drumkits")
    }

    fn library_with(
        &self,
        installed_json: Option<PathBuf>,
        worker: WorkerConfig,
    ) -> Arc<SharedKitLibrary> {
        SharedKitLibrary::open(Roots {
            root: Some(self.root()),
            marks_dir: Some(self.0.join("library")),
            installed_json,
            worker,
        })
    }

    fn library(&self) -> Arc<SharedKitLibrary> {
        self.library_with(None, quiet_config())
    }

    /// A kit at `<root>/<dir>/drum_samples.json` named `name`.
    fn kit(&self, dir: &str, name: &str) -> PathBuf {
        let kit = self.root().join(dir);
        std::fs::create_dir_all(&kit).unwrap();
        let manifest = serde_json::json!({
            format!("Kick of {name}"): {
                "01_KickIn_e901": {
                    "brand": "Sennheiser", "channel": "01", "mic": "e901",
                    "position": "KickIn",
                    "rounds": { "RR01": { "Vel01": "k.wav" } }
                }
            },
            "_meta": { "name": name }
        });
        std::fs::write(kit.join("k.wav"), [0u8; 64]).unwrap();
        std::fs::write(
            kit.join("drum_samples.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        kit
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A worker config whose index URL is a closed loopback port.
fn quiet_config() -> WorkerConfig {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    WorkerConfig {
        index_url: format!("http://127.0.0.1:{port}/index.json"),
        https_only: false,
        ..WorkerConfig::default()
    }
}

/// A server that accepts connections and never answers. The listener
/// lives as long as the returned value.
fn silent_server() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/index.json", listener.local_addr().unwrap());
    let accept = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in accept.incoming().flatten() {
            held.push(s);
        }
    });
    (listener, url)
}

fn plok_sidecar(index_name: &str) -> Sidecar {
    Sidecar {
        source: SOURCE_PLOK.into(),
        index_name: Some(index_name.into()),
        index_file: Some("x.zip".into()),
        ..Sidecar::default()
    }
}

// ---------------------------------------------------------------------------
// The plok.org tab's row state
// ---------------------------------------------------------------------------

/// A kit the user made or imported is never offered as an Update of an
/// index entry that happens to share its name — that would replace it with
/// the download. A kit downloaded from plok.org under that name is.
#[test]
fn update_is_offered_only_for_kits_from_plok() {
    let home = Home::new("rowstate");
    home.kit("Beta Kit", "Beta Kit");
    let plok = home.kit("Gamma Kit", "Gamma Kit");
    write_sidecar(&plok, &plok_sidecar("Gamma Kit")).unwrap();
    let lib = home.library();
    lib.rescan().unwrap().unwrap();

    let newer = |name: &str| ServerKit {
        manifest_sha256: Some("f".repeat(64)),
        ..ServerKit::new(name, "x.zip")
    };
    assert_eq!(
        lib.plok_row_state(&newer("Beta Kit")),
        PlokRowState::Download,
        "a local kit was offered as an Update"
    );
    assert!(matches!(
        lib.plok_row_state(&newer("Gamma Kit")),
        PlokRowState::Update { dir, .. } if dir == plok
    ));
    // Without a manifest hash, a plok.org kit of the name is installed;
    // a local one is not.
    assert!(matches!(
        lib.plok_row_state(&ServerKit::new("Gamma Kit", "x.zip")),
        PlokRowState::Installed { .. }
    ));
    assert_eq!(
        lib.plok_row_state(&ServerKit::new("Beta Kit", "x.zip")),
        PlokRowState::Download
    );
    // The manifest hash itself always counts.
    let beta_id = lib.read().find("Beta Kit").unwrap().id.clone();
    assert!(matches!(
        lib.plok_row_state(&ServerKit {
            manifest_sha256: Some(beta_id),
            ..ServerKit::new("Beta Kit", "x.zip")
        }),
        PlokRowState::Installed { .. }
    ));
}

// ---------------------------------------------------------------------------
// Rescans
// ---------------------------------------------------------------------------

/// A rescan that finds another writer busy is not lost: the writer runs
/// it when it lets go, so a kit added outside meanwhile shows up.
#[test]
fn a_rescan_skipped_behind_a_writer_runs_when_it_finishes() {
    let home = Home::new("skipped");
    home.kit("First", "First Kit");
    let lib = home.library();
    lib.rescan().unwrap().unwrap();
    assert_eq!(lib.read().len(), 1);

    let (in_tx, in_rx) = crossbeam_channel::bounded::<()>(1);
    let (go_tx, go_rx) = crossbeam_channel::bounded::<()>(1);
    let writer = {
        let lib = lib.clone();
        std::thread::spawn(move || {
            lib.mutate(|_| {
                in_tx.send(()).unwrap();
                go_rx.recv().unwrap();
            })
        })
    };
    in_rx.recv().unwrap();
    home.kit("Second", "Second Kit");
    assert!(lib.rescan().is_none(), "the writer was busy");
    assert!(lib.rescan_wanted());
    go_tx.send(()).unwrap();
    writer.join().unwrap();
    assert!(!lib.rescan_wanted());
    assert_eq!(lib.read().len(), 2, "the skipped rescan never ran");
}

// ---------------------------------------------------------------------------
// The installed.json migration
// ---------------------------------------------------------------------------

fn installed_json(home: &Home, kit: &Path, name: &str) -> PathBuf {
    let path = home.0.join("installed.json");
    let doc = serde_json::json!({ "items": [ {
        "name": name, "type": "drumkit",
        "path": kit.to_string_lossy(), "installed_at": "2025-01-02"
    } ] });
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    path
}

/// The migration's index fetch is bounded, and a failed one does not mark
/// the kits `local` for good: the migration waits, a later scan fetches
/// once more, and once an index is at hand the kit migrates as `plok`.
#[test]
fn the_migration_waits_for_an_index_instead_of_marking_kits_local() {
    let home = Home::new("migrate");
    let kit = home.kit("Drummica", "Drummica");
    let reg = installed_json(&home, &kit, "Drummica");
    let (_silent, url) = silent_server();
    let lib = home.library_with(
        Some(reg),
        WorkerConfig {
            index_url: url,
            https_only: false,
            migration_index_timeout: Duration::from_millis(200),
            ..WorkerConfig::default()
        },
    );

    let started = Instant::now();
    lib.rescan().unwrap().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "a 200 ms migration fetch took {:?}",
        started.elapsed()
    );
    assert_eq!(lib.download().fetches_started(), 1);
    assert!(
        read_sidecar(&kit).is_none(),
        "migrated without an index: the kit is marked local for good"
    );
    lib.rescan().unwrap().unwrap();
    assert_eq!(lib.download().fetches_started(), 2, "no retry");
    lib.rescan().unwrap().unwrap();
    assert_eq!(
        lib.download().fetches_started(),
        2,
        "fetched again and again"
    );
    assert!(read_sidecar(&kit).is_none());

    // The plok.org tab's fetch brings the index: the next scan migrates.
    lib.download().state.lock().index = Some(ServerIndex {
        drumkits: vec![ServerKit::new("Drummica", "drummica.zip")],
    });
    lib.rescan().unwrap().unwrap();
    let sc = read_sidecar(&kit).expect("migrated");
    assert_eq!(sc.source, SOURCE_PLOK);
    assert_eq!(sc.index_file.as_deref(), Some("drummica.zip"));
}

/// The migration's fetch gives up as soon as its caller is cancelled (an
/// editor closing), however long its own budget.
#[test]
fn the_migration_fetch_is_cancellable() {
    let home = Home::new("migrate-cancel");
    let kit = home.kit("Drummica", "Drummica");
    let reg = installed_json(&home, &kit, "Drummica");
    let (_silent, url) = silent_server();
    let lib = home.library_with(
        Some(reg),
        WorkerConfig {
            index_url: url,
            https_only: false,
            migration_index_timeout: Duration::from_secs(60),
            ..WorkerConfig::default()
        },
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    lib.rescan_with(&cancel).unwrap().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the cancelled fetch held the rescan for {:?}",
        started.elapsed()
    );
    assert!(read_sidecar(&kit).is_none());
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// "Used by N instances" counts an instance whose load of the kit is still
/// in flight, and a bridge registered twice counts once.
#[test]
fn usage_counts_a_kit_still_loading() {
    use resonance_drums::ResonanceDrums;
    use resonance_plugin::ResonancePlugin;

    let home = Home::new("usage");
    let kit = home.kit("Loading", "Loading Kit");
    let plugin = ResonanceDrums::new();
    library::register_bridge(&plugin.bridge);
    library::register_bridge(&plugin.bridge);
    assert_eq!(library::instances_playing(&kit), 0);
    *plugin.bridge.pending_kit.lock() = Some((1, kit.join("drum_samples.json")));
    assert_eq!(library::instances_playing(&kit), 1);
    *plugin.bridge.kit_path.lock() = Some(kit.join("drum_samples.json"));
    assert_eq!(library::instances_playing(&kit), 1, "counted twice");
    assert!(library::live_instances() >= 1);
}
