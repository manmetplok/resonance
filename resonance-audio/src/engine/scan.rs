//! Plugin bundle scanning. Iterates the platform's CLAP directories
//! (`~/.clap` + `/usr/lib/clap/` on Linux, `~/Library/Audio/Plug-Ins/CLAP`
//! + `/Library/Audio/Plug-Ins/CLAP` on macOS, per entry.h), any extra
//! dirs in `$CLAP_PATH`, and `target/bundled/`, and loads every `.clap`
//! file or directory it finds. The collected descriptors are sent back
//! to the app via `AudioEvent::PluginsScanned`.
//!
//! Two entry points, and the difference between them matters (ba todo
//! #1307, finding X10):
//!
//! - [`scan_plugins`] is the **startup** scan. It drops every
//!   instantiated plugin and reloads every bundle from scratch —
//!   correct exactly once, before anything is instantiated, because
//!   unloading a shared library out from under a live instance is a
//!   use-after-free.
//! - [`rescan_plugins`] is the **live** scan behind `plugins.rescan` and
//!   the Settings button. It is purely additive: bundles already loaded
//!   stay loaded (instances, editors and audio untouched) and only files
//!   that are NOT yet loaded are opened. Installing a plugin therefore
//!   no longer costs an app restart, and rescanning mid-session costs
//!   nothing audible.
//!
//! What a live rescan cannot do is *remove* a bundle: dropping a library
//! that a running instance came from would crash the audio thread, so an
//! uninstalled plugin stays in the catalog until the next start. That is
//! reported as the truth rather than papered over.
//!
//! Dropping a [`ClapBundle`] does not unload its shared library — see
//! [`ClapBundle`]'s `Drop` impl — so a startup re-scan re-`dlopen`s a
//! binary that is already resident rather than mapping it in afresh.

use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;

use crate::clap_host::ClapBundle;
use crate::types::*;

/// The startup scan: drop everything, reload everything.
pub(crate) fn scan_plugins(
    shared: &super::SharedState,
    tracks: &TrackMap,
    bundles: &mut Vec<ClapBundle>,
    event_tx: &Sender<AudioEvent>,
) {
    // Unpublish every existing plugin instance. The engine loop's retire
    // sweep destroys them once no block pins them (ARCH-02 B-4) — after
    // `bundles.clear()` below, which is safe because a bundle's library
    // is never unloaded (`ClapBundle`'s `Drop`), so the instance's
    // `destroy` still has its code to call into.
    shared.edit_plugins(|plugins| plugins.clear());
    // `clear_plugins` publishes a new empty chain via `ArcSwap::store`
    // (shared by every copy of the track), so reading the published track
    // map is enough — no render-graph publish.
    for track in tracks.values() {
        // The old chain is dropped here on the scan thread, not by the
        // callback: a tiny `Vec`, but the rule is uniform (MIX-04).
        drop(track.clear_plugins());
    }
    // Clear previous scan results to avoid duplicates.
    bundles.clear();

    let dirs = scan_dirs();
    let (scanned, failures) = load_bundles(&dirs, bundles);
    report(&dirs, scanned, failures, event_tx);
    spawn_discovery(bundles, event_tx, false);
}

/// The live rescan: pick up newly installed plugins WITHOUT disturbing
/// anything already running (ba todo #1307).
///
/// Nothing is dropped and nothing is unloaded, so a plugin that is
/// currently processing audio, holding an open editor or sitting in an
/// undo snapshot is not touched at all — the only effect is that bundles
/// which appeared on disk since the last scan are now loadable. The
/// catalog it reports is the whole set (old bundles included), because
/// that is what the app mirrors wholesale.
pub fn rescan_plugins(bundles: &mut Vec<ClapBundle>, event_tx: &Sender<AudioEvent>) {
    rescan_plugins_in(&scan_dirs(), bundles, event_tx);
}

/// [`rescan_plugins`] over a given set of directories.
///
/// The seam `tests/clap_host/plugin_rescan.rs` drives. A test must not scan the
/// machine's real plugin directories: `dlopen`ing whatever third-party
/// `.clap` files happen to be installed pulls their static initialisers
/// and `atexit` handlers into the test process, and a broken one takes
/// the process down at exit with the tests already passed (ba doc #285;
/// see also [`ClapBundle`]'s `Drop` impl for the same problem's other
/// half).
pub fn rescan_plugins_in(
    dirs: &[PathBuf],
    bundles: &mut Vec<ClapBundle>,
    event_tx: &Sender<AudioEvent>,
) {
    let (scanned, failures) = load_bundles(dirs, bundles);
    report(dirs, scanned, failures, event_tx);
    // A rescan is the user asking to look again: re-index everything.
    spawn_discovery(bundles, event_tx, true);
}

/// The current preset-discovery worker, and how to stop it.
struct DiscoveryWorker {
    handle: std::thread::JoinHandle<()>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

static DISCOVERY: std::sync::Mutex<Option<DiscoveryWorker>> = std::sync::Mutex::new(None);

/// Factories a worker is inside right now (by address). A provider can
/// hang in `get_metadata`; a cancelled worker stuck in one is abandoned,
/// never joined on the engine thread, and a newer worker skips its factory
/// rather than call into it from a second thread.
static BUSY_FACTORIES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// How long engine shutdown waits for the discovery worker before it
/// abandons it (bundles are never unloaded, so an abandoned worker only
/// ever touches live code; the process exits around it).
pub const DISCOVERY_SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Index the presets of every bundle with a `clap.preset-discovery-
/// factory` on a worker thread (slice P8): never on the engine thread,
/// which a slow provider walking a large preset folder would stall. Each
/// plugin's list arrives as `AudioEvent::PluginPresetsDiscovered`; the
/// index is cached under the cache dir (`preset-discovery/`).
fn spawn_discovery(bundles: &[ClapBundle], event_tx: &Sender<AudioEvent>, force: bool) {
    let jobs: Vec<_> = bundles
        .iter()
        .filter_map(|b| {
            let factory = b.preset_discovery_factory()?;
            let ids: Vec<String> = b.descriptors().iter().map(|d| d.id.clone()).collect();
            Some((factory, PathBuf::from(b.path()), ids))
        })
        .collect();
    if jobs.is_empty() {
        return;
    }
    spawn_discovery_jobs(
        jobs,
        event_tx,
        force,
        resonance_common::library_marks::default_cache_dir(),
    );
}

/// [`spawn_discovery`] over given factories and cache dir (the test seam).
/// A previous worker is cancelled and left to finish on its own — never
/// joined here, on the engine thread, where a hung provider would wedge
/// rescans.
pub fn spawn_discovery_jobs(
    jobs: Vec<(crate::clap_host::DiscoveryFactory, PathBuf, Vec<String>)>,
    event_tx: &Sender<AudioEvent>,
    force: bool,
    cache_dir: Option<PathBuf>,
) {
    let Ok(mut slot) = DISCOVERY.lock() else {
        return;
    };
    if let Some(previous) = slot.take() {
        previous.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let event_tx = event_tx.clone();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let spawned = std::thread::Builder::new()
        .name("preset-discovery".into())
        .spawn(move || {
            for (factory, binary, ids) in jobs {
                if worker_cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let key = factory.0 as usize;
                {
                    let Ok(mut busy) = BUSY_FACTORIES.lock() else {
                        return;
                    };
                    if busy.contains(&key) {
                        tracing::warn!(
                            "preset discovery: {} is still busy in an earlier scan; skipped",
                            binary.display()
                        );
                        continue;
                    }
                    busy.push(key);
                }
                // SAFETY: the factory pointer stays valid for the process
                // (a bundle's library is never unloaded), and `BUSY_FACTORIES`
                // makes this worker the only thread inside it (an abandoned
                // worker still in it keeps it marked, and is skipped).
                let found = unsafe {
                    crate::clap_host::discovery::discover(
                        factory.0,
                        &binary,
                        &ids,
                        cache_dir.as_deref(),
                        force,
                        &worker_cancel,
                    )
                };
                if let Ok(mut busy) = BUSY_FACTORIES.lock() {
                    busy.retain(|k| *k != key);
                }
                for (plugin_id, presets) in found {
                    let event = AudioEvent::PluginPresetsDiscovered { plugin_id, presets };
                    let _ = event_tx.send(event);
                }
            }
        });
    match spawned {
        Ok(handle) => *slot = Some(DiscoveryWorker { handle, cancel }),
        Err(e) => tracing::warn!("preset discovery: worker not started: {e}"),
    }
}

/// Stop the discovery worker (engine shutdown): cancel it, wait at most
/// [`DISCOVERY_SHUTDOWN_WAIT`], then abandon it. Returns whether it ended.
pub fn shutdown_discovery() -> bool {
    let worker = DISCOVERY.lock().ok().and_then(|mut s| s.take());
    let Some(worker) = worker else {
        return true;
    };
    worker.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let deadline = std::time::Instant::now() + DISCOVERY_SHUTDOWN_WAIT;
    while !worker.handle.is_finished() {
        if std::time::Instant::now() >= deadline {
            tracing::warn!("preset discovery: a provider did not return; abandoning the worker");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = worker.handle.join();
    true
}

/// The directories a scan looks in, in priority order.
fn scan_dirs() -> Vec<PathBuf> {
    let mut scan_dirs: Vec<PathBuf> = Vec::new();

    // The platform's per-user and system-wide CLAP dirs, as documented
    // in CLAP's entry.h.
    let (user_rel, sys_path) = if cfg!(target_os = "macos") {
        ("Library/Audio/Plug-Ins/CLAP", "/Library/Audio/Plug-Ins/CLAP")
    } else {
        (".clap", "/usr/lib/clap")
    };
    if let Some(home) = std::env::var_os("HOME") {
        let clap_dir = PathBuf::from(home).join(user_rel);
        if clap_dir.is_dir() {
            scan_dirs.push(clap_dir);
        }
    }
    let sys_dir = PathBuf::from(sys_path);
    if sys_dir.is_dir() {
        scan_dirs.push(sys_dir);
    }

    // $CLAP_PATH: extra search dirs, `:`-separated (also entry.h).
    if let Some(paths) = std::env::var_os("CLAP_PATH") {
        for dir in std::env::split_paths(&paths) {
            // Skip entries that just re-list a standard dir, so a
            // plugin isn't cataloged twice.
            if dir.is_dir() && !scan_dirs.contains(&dir) {
                scan_dirs.push(dir);
            }
        }
    }

    // Bundled plugins: find target/bundled/ relative to the executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            // cargo run: target/debug/ -> look for ../../target/bundled/
            let bundled = exe_dir
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("target").join("bundled"));
            if let Some(dir) = bundled {
                if dir.is_dir() {
                    scan_dirs.push(dir);
                }
            }
        }
    }

    // Also check workspace root target/bundled/
    let workspace_bundled = PathBuf::from("target/bundled");
    if workspace_bundled.is_dir() {
        if let Ok(canonical) = workspace_bundled.canonicalize() {
            if !scan_dirs
                .iter()
                .any(|d| d.canonicalize().ok().as_ref() == Some(&canonical))
            {
                scan_dirs.push(workspace_bundled);
            }
        } else {
            scan_dirs.push(workspace_bundled);
        }
    }

    scan_dirs
}

/// Whether a directory entry names a CLAP bundle.
///
/// Both a `.clap` file and a `.clap` *directory* count, and so does a
/// symlink whose name ends in `.clap` (how a dev checkout usually points
/// at `target/bundled`).
fn is_clap_bundle(path: &Path) -> bool {
    path.extension().map(|e| e == "clap").unwrap_or(false)
        || path.to_str().map(|s| s.ends_with(".clap")).unwrap_or(false)
}

/// Load every bundle under `dirs` — recursively, see
/// [`collect_clap_paths`] — that `bundles` does not already hold,
/// appending the new ones. Returns the catalog for ALL loaded bundles
/// (already-present ones included) and the failures this pass hit.
///
/// Skipping by path is what makes a rescan safe: a bundle stays loaded
/// exactly once, at the address its live instances were created from.
fn load_bundles(
    dirs: &[PathBuf],
    bundles: &mut Vec<ClapBundle>,
) -> (Vec<ScannedPlugin>, Vec<PluginScanFailure>) {
    let mut failures = Vec::new();

    let mut clap_paths = Vec::new();
    for dir in dirs {
        collect_clap_paths(dir, 0, &mut clap_paths);
    }

    for path in clap_paths {
        // Resolve symlinks for loading — and for identity: two
        // directories can point at one bundle, and loading it twice
        // would double every entry in the catalog.
        let real_path = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let real_path_str = real_path.to_string_lossy().to_string();
        if bundles.iter().any(|b| b.path() == real_path_str) {
            continue;
        }

        match ClapBundle::load(&real_path) {
            // Keep the bundle alive for later instantiation.
            Ok(bundle) => bundles.push(bundle),
            Err(e) => {
                // Not swallowed: a bundle that fails to load is the
                // difference between "this plugin does not exist" and
                // "this plugin is broken", and only one of those is
                // the user's to fix (ba todo #1307).
                tracing::warn!("Failed to scan {}: {}", path.display(), e);
                failures.push(PluginScanFailure {
                    path: real_path_str,
                    reason: e.to_string(),
                });
            }
        }
    }

    failures.extend(duplicate_id_warnings(bundles));
    (catalog(bundles), failures)
}

/// One scan warning per bundle that declares a plugin id an earlier bundle
/// already declares (code review HOST-10): a dev `target/bundled` build
/// beside an installed copy, typically. Both stay loaded and listed, and a
/// slot instantiates from the file it names (`plugins::ensure_bundle`
/// keys by path first), but the user should know two files claim one id —
/// a project names a plugin by id as well as by path.
fn duplicate_id_warnings(bundles: &[ClapBundle]) -> Vec<PluginScanFailure> {
    let mut first_seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    let mut warnings = Vec::new();
    for bundle in bundles {
        for desc in bundle.descriptors() {
            match first_seen.get(desc.id.as_str()) {
                Some(&other) if other != bundle.path() => {
                    tracing::warn!(
                        "plugin id {} is declared by both {} and {}",
                        desc.id,
                        other,
                        bundle.path()
                    );
                    warnings.push(PluginScanFailure {
                        path: bundle.path().to_string(),
                        reason: format!(
                            "duplicate plugin id '{}': {} declares it too. Both are listed; \
                             each instance loads from the file it was added from.",
                            desc.id, other
                        ),
                    });
                }
                Some(_) => {}
                None => {
                    first_seen.insert(desc.id.as_str(), bundle.path());
                }
            }
        }
    }
    warnings
}

/// Every plugin in every loaded bundle, as the app's catalog entries.
fn catalog(bundles: &[ClapBundle]) -> Vec<ScannedPlugin> {
    let mut scanned = Vec::new();
    for bundle in bundles {
        for desc in bundle.descriptors() {
            scanned.push(ScannedPlugin {
                clap_file_path: bundle.path().to_string(),
                clap_plugin_id: desc.id.clone(),
                name: desc.name.clone(),
                vendor: desc.vendor.clone(),
                is_instrument: desc.is_instrument,
                // Our bundles ship one plugin each, so the bundle's
                // bank is this descriptor's bank.
                factory_presets: bundle.factory_presets().to_vec(),
            });
        }
    }
    scanned
}

/// Publish a completed scan: the catalog always, the failures only when
/// there are any.
fn report(
    dirs: &[PathBuf],
    scanned: Vec<ScannedPlugin>,
    failures: Vec<PluginScanFailure>,
    event_tx: &Sender<AudioEvent>,
) {
    // A checkout whose plugins were never bundled scans clean and finds
    // nothing first-party, leaving an instrument-less DAW with no hint
    // that a build step was missed — the catalog just looks empty, which
    // reads as "this app ships no instruments" (ba doc #270 §1). Say so
    // once per scan, naming the fix and where we looked.
    if !scanned.iter().any(|p| p.is_instrument) {
        let dirs: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        tracing::warn!(
            "plugins: no instruments found ({} plugin(s) scanned in [{}]). \
             The first-party plugins under plugins/ are CLAP bundles that must be \
             built first: run scripts/bundle.sh, then rescan. Until then \
             instrument tracks have no sound source and track.add_instrument has \
             nothing to offer.",
            scanned.len(),
            dirs.join(", ")
        );
    }

    if !failures.is_empty() {
        let _ = event_tx.send(AudioEvent::PluginScanFailed { failures });
    }
    let _ = event_tx.send(AudioEvent::PluginsScanned { plugins: scanned });
}

/// Gather every `.clap` under `dir` into `out`, recursing into vendor
/// subdirectories (entry.h: the search paths are scanned recursively —
/// macOS installers in particular nest bundles as e.g.
/// `CLAP/u-he/Hive.clap`) but not into `.clap` bundle directories
/// themselves. Depth-capped so a symlink cycle can't spin the scan
/// forever.
fn collect_clap_paths(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    const MAX_DEPTH: usize = 4;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_clap_bundle(&path) {
            out.push(path);
        } else if path.is_dir() && depth < MAX_DEPTH {
            collect_clap_paths(&path, depth + 1, out);
        }
    }
}
