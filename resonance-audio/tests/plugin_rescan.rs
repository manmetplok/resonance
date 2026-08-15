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
//! These scan the real directories the app scans, so how much they find
//! depends on the checkout: in one where `scripts/bundle.sh` has not
//! run, `target/bundled` is empty and the load-nothing-twice assertion
//! holds over an empty set. The properties are the same either way, and
//! they bite as soon as there is a bundle to find.

use crossbeam_channel::unbounded;
use resonance_audio::__test_support::{rescan_plugins, ClapBundle};
use resonance_audio::types::AudioEvent;

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

fn drain(rx: &crossbeam_channel::Receiver<AudioEvent>) -> Vec<AudioEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[test]
fn a_second_rescan_loads_nothing_twice() {
    let (tx, rx) = unbounded();
    let mut bundles: Vec<ClapBundle> = Vec::new();

    rescan_plugins(&mut bundles, &tx);
    let first = scanned_from(&drain(&rx));
    let loaded_after_first = bundles.len();

    // Scanning again over the same directories: every bundle is already
    // held, so nothing is opened a second time and the catalog is the
    // same set.
    rescan_plugins(&mut bundles, &tx);
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

    // Also: no bundle appears twice within one report. (Two scan
    // directories can point at the same file through a symlink, which
    // is exactly how a dev checkout is usually laid out.)
    let mut paths: Vec<&str> = first.iter().map(|(_, path)| path.as_str()).collect();
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
fn a_rescan_always_reports_even_with_nothing_to_find() {
    // A catalog of zero is a real answer ("nothing installed"), and the
    // app clears its in-progress flag on it. Silence would leave the
    // Settings button reading "Scanning..." forever.
    let (tx, rx) = unbounded();
    let mut bundles: Vec<ClapBundle> = Vec::new();
    rescan_plugins(&mut bundles, &tx);
    let events = drain(&rx);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::PluginsScanned { .. })),
        "expected a PluginsScanned, got {events:?}"
    );
}
