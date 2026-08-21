//! Plugin bundle scanning. Iterates the platform's CLAP directories
//! (`~/.clap` + `/usr/lib/clap/` on Linux, `~/Library/Audio/Plug-Ins/CLAP`
//! + `/Library/Audio/Plug-Ins/CLAP` on macOS, per entry.h), any extra
//! dirs in `$CLAP_PATH`, and `target/bundled/`, and loads every `.clap`
//! file or directory it finds. Each scan first drops every
//! currently instantiated plugin (their factories belong to bundles this
//! scan is about to replace) and clears `track.plugin_ids`, then rebuilds
//! the `bundles` list from scratch. The collected descriptors are sent
//! back to the app via `AudioEvent::PluginsScanned`.
//!
//! Dropping a [`ClapBundle`] does not unload its shared library — see
//! [`ClapBundle`]'s `Drop` impl — so a re-scan re-`dlopen`s a binary that
//! is already resident rather than mapping it in afresh.

use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::clap_host::{ClapBundle, PluginMap};
use crate::types::*;

pub(crate) fn scan_plugins(
    plugins: &Arc<RwLock<PluginMap>>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    bundles: &mut Vec<ClapBundle>,
    event_tx: &Sender<AudioEvent>,
) {
    let mut scanned = Vec::new();
    let mut scan_dirs: Vec<std::path::PathBuf> = Vec::new();

    // Drop all existing plugin instances before clearing bundles: an
    // instance is created by, and calls back into, its bundle's factory.
    {
        let mut plugins_guard = plugins.write();
        let removed: Vec<_> = plugins_guard.drain(..).collect();
        drop(plugins_guard);
        drop(removed);
    }
    // `clear_plugins` publishes a new empty chain via `ArcSwap::store`,
    // so a read guard on the tracks map is enough — write-locking it
    // here used to silence the audio callback for whatever block
    // straddled the scan.
    for track in tracks.read().values() {
        track.clear_plugins();
    }
    // Clear previous scan results to avoid duplicates.
    bundles.clear();

    // The platform's per-user and system-wide CLAP dirs, as documented
    // in CLAP's entry.h.
    let (user_rel, sys_path) = if cfg!(target_os = "macos") {
        ("Library/Audio/Plug-Ins/CLAP", "/Library/Audio/Plug-Ins/CLAP")
    } else {
        (".clap", "/usr/lib/clap")
    };
    if let Some(home) = std::env::var_os("HOME") {
        let clap_dir = std::path::PathBuf::from(home).join(user_rel);
        if clap_dir.is_dir() {
            scan_dirs.push(clap_dir);
        }
    }
    let sys_dir = std::path::PathBuf::from(sys_path);
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
    let workspace_bundled = std::path::PathBuf::from("target/bundled");
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

    let mut clap_paths = Vec::new();
    for dir in &scan_dirs {
        collect_clap_paths(dir, 0, &mut clap_paths);
    }

    for path in clap_paths {
        // Resolve symlinks for loading.
        let real_path = match std::fs::canonicalize(&path) {
            Ok(p) => p,
            Err(_) => path.clone(),
        };

        match ClapBundle::load(&real_path) {
            Ok(bundle) => {
                for desc in bundle.descriptors() {
                    scanned.push(ScannedPlugin {
                        clap_file_path: real_path.to_string_lossy().to_string(),
                        clap_plugin_id: desc.id.clone(),
                        name: desc.name.clone(),
                        vendor: desc.vendor.clone(),
                        is_instrument: desc.is_instrument,
                        // Our bundles ship one plugin each, so the
                        // bundle's bank is this descriptor's bank.
                        factory_presets: bundle.factory_presets().to_vec(),
                    });
                }
                // Keep bundle alive for later instantiation.
                bundles.push(bundle);
            }
            Err(e) => {
                eprintln!("Failed to scan {}: {}", path.display(), e);
            }
        }
    }

    // A checkout whose plugins were never bundled scans clean and finds
    // nothing first-party, leaving an instrument-less DAW with no hint
    // that a build step was missed — the catalog just looks empty, which
    // reads as "this app ships no instruments" (ba doc #270 §1). Say so
    // once per scan, naming the fix and where we looked.
    if !scanned.iter().any(|p| p.is_instrument) {
        let dirs: Vec<String> = scan_dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        eprintln!(
            "plugins: no instruments found ({} plugin(s) scanned in [{}]). \
             The first-party plugins under plugins/ are CLAP bundles that must be \
             built first: run scripts/bundle.sh, then rescan. Until then \
             instrument tracks have no sound source and track.add_instrument has \
             nothing to offer.",
            scanned.len(),
            dirs.join(", ")
        );
    }

    let _ = event_tx.send(AudioEvent::PluginsScanned { plugins: scanned });
}

/// Gather every `.clap` under `dir` into `out`, recursing into vendor
/// subdirectories (entry.h: the search paths are scanned recursively —
/// macOS installers in particular nest bundles as e.g.
/// `CLAP/u-he/Hive.clap`) but not into `.clap` bundle directories
/// themselves. Depth-capped so a symlink cycle can't spin the scan
/// forever.
fn collect_clap_paths(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    const MAX_DEPTH: usize = 4;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Handle both .clap files and .clap directories (bundles).
        let is_clap = path.extension().map(|e| e == "clap").unwrap_or(false);
        // Also follow symlinks to .so files named *.clap.
        let is_clap = is_clap || path.to_str().map(|s| s.ends_with(".clap")).unwrap_or(false);

        if is_clap {
            out.push(path);
        } else if path.is_dir() && depth < MAX_DEPTH {
            collect_clap_paths(&path, depth + 1, out);
        }
    }
}
