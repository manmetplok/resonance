//! The live plugin rescan is additive (ba todo #1307, finding X10).
//!
//! The startup scan drops every instantiated plugin and reloads every
//! bundle, which is only safe before anything has been instantiated —
//! so installing a plugin used to cost an app restart. The live rescan
//! must never unload a bundle a running instance came from, and must
//! never load one twice: two `ClapBundle`s over one library would run
//! the entry point's init/deinit pair twice for the same shared object,
//! while the first one's instances are still processing audio.
//!
//! `rescan_plugins` takes only the bundle list and the event channel —
//! not the instance map — so it *cannot* reach a running plugin. That is
//! the structural half of the guarantee; this is the behavioural half.
//!
//! These drive `rescan_plugins_in` over a temp directory, never the
//! machine's real plugin folders. `dlopen`ing whatever third-party
//! `.clap` files happen to be installed pulls their static initialisers
//! and `atexit` handlers into the test process, and a broken one aborts
//! it *after* the tests have passed — which is how the recursive vendor
//! walk turned this file red the moment it started finding
//! `~/.clap/<vendor>/*.clap` (ba doc #285).
//!
//! The bundle they load is one of our own: the workspace's plugin
//! cdylibs are CLAP bundles, so symlinking one in as `<name>.clap` gives
//! a real, loadable, first-party binary with no third-party code in the
//! process. A build that has not produced one yet (a bare
//! `cargo test -p resonance-audio` rather than `./scripts/run-tests.py`)
//! leaves the two loaded-bundle assertions with nothing to bite on, and
//! [`first_party_cdylib`] says so on stderr rather than passing quietly.

use crossbeam_channel::unbounded;
use resonance_audio::test_support::{rescan_plugins_in, ClapBundle};
use resonance_audio::types::AudioEvent;
use std::path::{Path, PathBuf};

/// Every plugin a scan reported, as `(clap id, file path)` pairs.
fn scanned_from(events: &[AudioEvent]) -> Vec<(String, String)> {
    events
        .iter()
        .find_map(|e| match e {
            AudioEvent::PluginsScanned { plugins } => Some(
                plugins
                    .iter()
                    .map(|p| (p.clap_plugin_id.clone(), p.clap_file_path.clone()))
                    .collect(),
            ),
            _ => None,
        })
        .expect("a scan always reports a catalog, even an empty one")
}

/// Every path the scan reported as unloadable.
fn failures_from(events: &[AudioEvent]) -> Vec<String> {
    events
        .iter()
        .find_map(|e| match e {
            AudioEvent::PluginScanFailed { failures } => {
                Some(failures.iter().map(|f| f.path.clone()).collect())
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn drain(rx: &crossbeam_channel::Receiver<AudioEvent>) -> Vec<AudioEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// A scratch directory of this test's own, removed on drop.
struct ScanDir(PathBuf);

impl ScanDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("resonance-rescan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scan dir");
        ScanDir(dir)
    }

    fn dirs(&self) -> Vec<PathBuf> {
        vec![self.0.clone()]
    }

    /// A file that ends in `.clap` and is not a CLAP bundle.
    fn write_junk(&self, rel: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().expect("junk path has a parent")).expect("mkdir");
        std::fs::write(&path, b"not a shared object").expect("write junk bundle");
        path
    }

    /// Symlink `src` in under `rel`, as a bundle the scan will try to load.
    fn link_bundle(&self, rel: &str, src: &Path) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().expect("bundle path has a parent")).expect("mkdir");
        std::os::unix::fs::symlink(src, &path).expect("symlink bundle");
        path
    }
}

impl Drop for ScanDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One of the workspace's own plugin cdylibs, or `None` when this build
/// has not produced any.
///
/// Looks next to the test binary (`target/<profile>/deps/..`), which is
/// where cargo puts them.
fn first_party_cdylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?;
    let mut found: Vec<PathBuf> = std::fs::read_dir(profile_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libresonance_") && n.ends_with(".so"))
        })
        .collect();
    // Sorted so a run picks the same bundle every time.
    found.sort();
    if found.is_empty() {
        eprintln!(
            "plugin_rescan: no first-party plugin cdylib in {} — the \
             already-loaded and duplicate-path assertions have nothing to \
             bite on. Build the workspace (./scripts/run-tests.py, or \
             cargo build --workspace) to cover them.",
            profile_dir.display()
        );
    }
    found.into_iter().next()
}

#[test]
fn a_second_rescan_loads_nothing_twice() {
    let Some(cdylib) = first_party_cdylib() else {
        return;
    };
    let dir = ScanDir::new("twice");
    dir.link_bundle("resonance-fixture.clap", &cdylib);

    let (tx, rx) = unbounded();
    let mut bundles: Vec<ClapBundle> = Vec::new();

    rescan_plugins_in(&dir.dirs(), &mut bundles, &tx);
    let first = scanned_from(&drain(&rx));
    let loaded_after_first = bundles.len();
    assert!(
        loaded_after_first > 0,
        "the fixture bundle must load, or the rest of this test is vacuous"
    );

    // Scanning again over the same directories: every bundle is already
    // held, so nothing is opened a second time and the catalog is the
    // same set.
    rescan_plugins_in(&dir.dirs(), &mut bundles, &tx);
    let second = scanned_from(&drain(&rx));

    assert_eq!(
        bundles.len(),
        loaded_after_first,
        "a rescan must skip bundles it already holds — loading one twice \
         would re-run its entry point under its own live instances"
    );
    assert_eq!(
        first, second,
        "the catalog a rescan reports is the whole set, and it is stable"
    );
}

#[test]
fn one_bundle_reached_two_ways_is_cataloged_once() {
    let Some(cdylib) = first_party_cdylib() else {
        return;
    };
    let dir = ScanDir::new("dupes");
    // Two names for one file, which is exactly how a dev checkout is
    // usually laid out — and, since the walk recurses, how a vendor
    // subdirectory can shadow a top-level install.
    dir.link_bundle("resonance-fixture.clap", &cdylib);
    dir.link_bundle("vendor/resonance-fixture.clap", &cdylib);

    let (tx, rx) = unbounded();
    let mut bundles: Vec<ClapBundle> = Vec::new();
    rescan_plugins_in(&dir.dirs(), &mut bundles, &tx);
    let scanned = scanned_from(&drain(&rx));

    assert_eq!(
        bundles.len(),
        1,
        "both names canonicalize to one file, so it is loaded once"
    );
    let mut paths: Vec<&str> = scanned.iter().map(|(_, path)| path.as_str()).collect();
    paths.sort_unstable();
    let unique = {
        let mut p = paths.clone();
        p.dedup();
        p.len()
    };
    assert_eq!(
        paths.len(),
        unique,
        "one bundle, one entry — got duplicates in {paths:?}"
    );
}

#[test]
fn the_walk_reaches_vendor_subdirectories() {
    // CLAP's entry.h has the search paths scanned recursively, and macOS
    // installers in particular nest bundles as `CLAP/<vendor>/X.clap`. A
    // flat `read_dir` never saw those at all. Asserted through the
    // failure report, which names the paths the scan actually tried.
    let dir = ScanDir::new("nested");
    let nested = dir.write_junk("vendor/deep/nested.clap");
    let top = dir.write_junk("top.clap");

    let (tx, rx) = unbounded();
    let mut bundles: Vec<ClapBundle> = Vec::new();
    rescan_plugins_in(&dir.dirs(), &mut bundles, &tx);
    let mut tried = failures_from(&drain(&rx));
    tried.sort();

    let mut expected = vec![
        std::fs::canonicalize(&nested)
            .expect("nested fixture exists")
            .to_string_lossy()
            .to_string(),
        std::fs::canonicalize(&top)
            .expect("top-level fixture exists")
            .to_string_lossy()
            .to_string(),
    ];
    expected.sort();
    assert_eq!(
        tried, expected,
        "the scan must reach a bundle nested under a vendor directory"
    );
}

#[test]
fn a_rescan_always_reports_even_with_nothing_to_find() {
    // A catalog of zero is a real answer ("nothing installed"), and the
    // app clears its in-progress flag on it. Silence would leave the
    // Settings button reading "Scanning..." forever.
    let dir = ScanDir::new("empty");
    let (tx, rx) = unbounded();
    let mut bundles: Vec<ClapBundle> = Vec::new();
    rescan_plugins_in(&dir.dirs(), &mut bundles, &tx);
    let events = drain(&rx);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::PluginsScanned { .. })),
        "expected a PluginsScanned, got {events:?}"
    );
    assert!(
        failures_from(&events).is_empty(),
        "nothing to find is not a failure"
    );
}
