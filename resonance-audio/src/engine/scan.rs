//! Plugin bundle scanning. Iterates a fixed set of directories
//! (`~/.clap`, `/usr/lib/clap/`, `target/bundled/`) and loads every
//! `.clap` file or directory it finds. Each scan first drops every
//! currently instantiated plugin (to avoid use-after-free when unloading
//! shared libraries) and clears `track.plugin_ids`, then rebuilds the
//! `bundles` list from scratch. The collected descriptors are sent back
//! to the app via `AudioEvent::PluginsScanned`.

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

    // Drop all existing plugin instances before clearing bundles, to
    // prevent use-after-free from accessing unloaded shared libraries.
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

    // ~/.clap/
    if let Some(home) = std::env::var_os("HOME") {
        let clap_dir = std::path::PathBuf::from(home).join(".clap");
        if clap_dir.is_dir() {
            scan_dirs.push(clap_dir);
        }
    }

    // /usr/lib/clap/
    let sys_dir = std::path::PathBuf::from("/usr/lib/clap");
    if sys_dir.is_dir() {
        scan_dirs.push(sys_dir);
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

    for dir in &scan_dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Handle both .clap files and .clap directories (bundles).
            let is_clap = path.extension().map(|e| e == "clap").unwrap_or(false);
            // Also follow symlinks to .so files named *.clap.
            let is_clap = is_clap || path.to_str().map(|s| s.ends_with(".clap")).unwrap_or(false);

            if !is_clap {
                continue;
            }

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
