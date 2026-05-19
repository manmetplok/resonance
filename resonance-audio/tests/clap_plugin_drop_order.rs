//! Regression test for the on-exit segfault caused by dropping a
//! `ClapBundle` (and thus `dlclose`-ing its `.clap` shared library)
//! before the `ClapInstance` created from it had a chance to run its
//! own `Drop` (which calls `(*plugin).destroy` — a function pointer
//! into the just-unloaded library).
//!
//! Reported 2026-05-19: closing the main window of `cargo run -p
//! resonance-app --release` produced a segfault. The kernel log
//! pinned two crashing threads:
//!   1. `cpal_alsa_out` (audio callback) calling `(*plugin).process`
//!      after the engine thread had already returned and dropped
//!      `HandlerState::bundles`.
//!   2. The main thread, calling `ClapInstance::drop` against the
//!      same unloaded libraries when `Resonance` finally dropped
//!      the engine and `engine.plugins` Arc hit refcount 0.
//!
//! Both shared one root cause: the engine control thread held the
//! `Vec<ClapBundle>` inside `HandlerState`, and let `state` fall out
//! of scope at thread exit. That dropped the bundles (`dlclose`)
//! while `Arc<RwLock<IndexMap<_, Mutex<SyncClapInstance>>>>` was
//! still pinned by other clones (the audio callback closure inside
//! `_stream`, and the main thread's `engine.plugins`). The instances
//! lived on with dangling function pointers into freed memory.
//!
//! Fix: explicitly swap the plugins IndexMap out *before* the engine
//! thread's `state` drops. Running each `ClapInstance::drop` while
//! the libraries are still mapped in lets `close_gui` /
//! `stop_processing` / `deactivate` / `destroy` reach valid pointers.
//!
//! This test reproduces the bug at the lowest possible level — load
//! a real `.clap` bundle, create an instance, then exercise the same
//! "drop instance via IndexMap-clear, *then* drop the bundle" pattern
//! the engine thread now uses. Skips (rather than failing) when the
//! workspace hasn't been built yet — the locally-bundled CLAPs live
//! under `target/bundled/` and aren't a precondition for the rest of
//! the test suite to run.

use std::path::PathBuf;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

use resonance_audio::__test_support::{ClapBundle, SyncClapInstance};

/// Find a locally-built CLAP bundle to load. Returns `None` if none
/// of the candidate paths exist — the test then becomes a no-op so it
/// doesn't false-fail when run from a fresh checkout.
fn find_bundled_clap() -> Option<PathBuf> {
    // `cargo test -p resonance-audio` runs with CWD =
    // `resonance-audio/`. The bundled plugins live at workspace
    // root's `target/bundled/`. Probe both layouts so the test
    // works whether `cargo test --workspace` or
    // `cargo test -p resonance-audio` is the entrypoint.
    let candidates = [
        // From workspace root.
        PathBuf::from("target/bundled/resonance-amp.clap"),
        // From `resonance-audio/` working directory.
        PathBuf::from("../target/bundled/resonance-amp.clap"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Mirror of the engine thread's plugin map type.
type PluginsArc = Arc<RwLock<IndexMap<u32, Mutex<SyncClapInstance>>>>;

#[test]
fn dropping_plugins_before_bundles_does_not_segfault() {
    let Some(bundle_path) = find_bundled_clap() else {
        eprintln!(
            "[skip] no bundled .clap found at target/bundled/resonance-amp.clap — \
             run `cargo build --workspace` first to enable this regression test"
        );
        return;
    };

    // Load the bundle and capture its first instrument descriptor.
    let bundle = ClapBundle::load(&bundle_path).expect("bundle load");
    let descriptor_id = bundle
        .descriptors()
        .first()
        .map(|d| d.id.clone())
        .expect("bundle exposes at least one plugin");

    // Wrap the bundle in the same `Vec<ClapBundle>` shape the engine
    // thread holds in `HandlerState`, and the plugins map in the same
    // `Arc<RwLock<IndexMap<_, Mutex<...>>>>` shape it shares with
    // `Resonance` and the audio callback.
    let bundles: Vec<ClapBundle> = vec![bundle];

    let plugins: PluginsArc = Arc::new(RwLock::new(IndexMap::new()));

    // Create one instance off the loaded bundle and insert it under
    // the write lock — same shape as `handle_add_plugin`.
    let instance = bundles[0]
        .create_instance(&descriptor_id, 48_000)
        .expect("create_instance");
    plugins
        .write()
        .insert(1, Mutex::new(SyncClapInstance(instance)));

    // Clone the Arc so we still have a reference outside the engine
    // thread when "engine_thread" returns — this mirrors what the
    // main thread (`Resonance.engine.plugins`) and the cpal callback
    // closure both keep alive.
    let plugins_outer = Arc::clone(&plugins);

    // -- Engine-thread teardown order (post-fix) --
    //
    // 1. Swap the plugins map out under the write lock. Dropping
    //    the swapped-out map runs `ClapInstance::drop` while the
    //    parent `.clap` is still mapped in.
    let drained: IndexMap<u32, Mutex<SyncClapInstance>> = std::mem::take(&mut *plugins.write());
    drop(drained);
    // 2. Drop the bundles last (`dlclose`). At this point the map
    //    is empty so no instance still holds a function pointer
    //    into the libraries we're about to unmap.
    drop(bundles);

    // The "outer" Arc reaches refcount 0 only when both the engine-
    // thread clone (above) and this clone are gone. Dropping it
    // here matches what happens when `Resonance` drops the engine
    // at app exit — and because the inner IndexMap is empty there's
    // nothing left to destroy. Without the fix, this same drop
    // would walk dangling `(*plugin).destroy` pointers and crash.
    drop(plugins);
    drop(plugins_outer);

    // Reaching this line means every `ClapInstance::drop` ran
    // against live function pointers and every `ClapBundle::drop`
    // (`dlclose`) ran with no live instances behind it.
}

#[test]
fn second_instance_after_first_is_destroyed_safely() {
    // Same shape as the primary test, but with two distinct
    // instances so the drop iterates the IndexMap and isn't masked
    // by a single-element edge case. Several CLAP plugins keep
    // refcounted host-side state across instances; this guards
    // against any "destroying instance A invalidates instance B"
    // ordering bug in either our code or the plugin.
    let Some(bundle_path) = find_bundled_clap() else {
        eprintln!("[skip] no bundled .clap found — see neighbour test for context");
        return;
    };

    let bundle = ClapBundle::load(&bundle_path).expect("bundle load");
    let descriptor_id = bundle
        .descriptors()
        .first()
        .map(|d| d.id.clone())
        .expect("bundle exposes at least one plugin");
    let bundles = vec![bundle];

    let plugins: PluginsArc = Arc::new(RwLock::new(IndexMap::new()));
    for id in 1u32..=2 {
        let inst = bundles[0]
            .create_instance(&descriptor_id, 48_000)
            .expect("create_instance");
        plugins
            .write()
            .insert(id, Mutex::new(SyncClapInstance(inst)));
    }

    assert_eq!(plugins.read().len(), 2);

    let drained: IndexMap<u32, Mutex<SyncClapInstance>> = std::mem::take(&mut *plugins.write());
    assert_eq!(plugins.read().len(), 0);
    drop(drained);
    drop(bundles);
    drop(plugins);
}
