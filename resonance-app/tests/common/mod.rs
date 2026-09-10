//! Shared test-support helpers for the `resonance-app` integration tests.
//!
//! Each file under `tests/` is compiled as its own crate, so this module is
//! shared by declaring `mod common;` at the top of every golden-image test and
//! routing pixel comparisons through [`assert_golden`].
//!
//! # Golden-image snapshot test hermetics
//!
//! Golden-image snapshot tests render the UI through `iced_test`'s
//! [`Simulator`](iced_test::simulator::Simulator) and compare the resulting
//! pixels against a committed PNG under `tests/snapshots/`. On a conformant GPU
//! rasterizer the output is deterministic, so a divergence signals a real
//! rendering change.
//!
//! Some environments — notably the ba verify gate, which runs on a
//! **non-conformant** software Vulkan implementation (radv reports "not a
//! conformant Vulkan implementation") — produce slightly different pixels for
//! the *same* UI. Under those renderers every golden diverges regardless of the
//! change under test, which repeatedly wedged the verify gate.
//!
//! ## When goldens run vs. skip
//!
//! * **Run (default, and on this machine):** when `RESONANCE_SKIP_GOLDENS` is unset
//!   (or not `"1"`), [`assert_golden`] performs the exact pixel comparison and
//!   fails the test on divergence. Behavior is identical to the old inline
//!   `assert!(snap.matches_image(...))`.
//! * **Skip (non-conformant env):** when the verify gate sets
//!   `RESONANCE_SKIP_GOLDENS=1`, [`assert_golden`] still renders the snapshot
//!   (exercising the full UI code path), but treats the pixel comparison as a
//!   pass and logs a note instead of asserting.
//!
//! `RESONANCE_SKIP_GOLDENS=1` is the reliable, explicit contract set by the
//! gate. Auto-detection from the wgpu adapter name is a possible future
//! enhancement, but the environment variable is authoritative.

use iced_test::simulator::Snapshot;

/// Returns `true` when golden-image pixel comparisons should be skipped in this
/// environment.
///
/// True iff `RESONANCE_SKIP_GOLDENS` is exactly `"1"` — not `"true"`, not
/// `"yes"`. The ba verify-gate overrides set `=1`; anything else runs the
/// pixel diff. There is no CI: this machine is canonical, and goldens are
/// blessed here (see the note in `project_snapshot_goldens_env_divergent`).
#[allow(dead_code)]
pub fn should_skip_goldens() -> bool {
    std::env::var("RESONANCE_SKIP_GOLDENS").as_deref() == Ok("1")
}

/// Asserts that `snapshot` matches the golden PNG at `path`, with hermetic
/// behavior on non-conformant renderers.
///
/// On a conformant environment (the default, and conformant CI) this compares
/// the rendered pixels against the committed golden and panics on divergence or
/// I/O error — exactly like the previous inline
/// `assert!(snapshot.matches_image(path).expect(...))`.
///
/// When `RESONANCE_SKIP_GOLDENS=1` is set (the verify gate on a
/// non-conformant renderer), the pixel comparison is skipped: the snapshot has already been
/// rendered by the caller, so the UI code path is still exercised, but the test
/// passes without diffing pixels. A note is printed so skips are visible in the
/// test log.
///
/// `path` is the golden location relative to the crate root, e.g.
/// `"tests/snapshots/my_test.png"`.
///
/// To re-bless goldens after an intended visual change, set `RESONANCE_BLESS=1`
/// and run the tests that own them:
///
/// ```text
/// RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer
/// ```
#[allow(dead_code)]
#[track_caller]
pub fn assert_golden(snapshot: &Snapshot, path: &str) {
    if should_skip_goldens() {
        eprintln!(
            "RESONANCE_SKIP_GOLDENS=1: skipping golden pixel comparison for {path} \
             (non-conformant render environment)"
        );
        return;
    }

    // `matches_image` CREATES the golden and returns Ok(true) when the
    // file is absent, so without this check a test whose PNG was never
    // committed (or was lost to a bad merge / `git clean`) would write a
    // fresh golden into every checkout and report ok forever — guarding
    // nothing. Fail loudly instead; blessing a new golden is a
    // deliberate act, not something a normal test run does silently.
    //
    // iced_test ALWAYS rewrites the path to a backend-suffixed file
    // (`foo.png` -> `foo-<renderer>.png`), and the renderer is chosen at
    // runtime: the default is a fallback pair, so a machine where wgpu
    // cannot initialise silently drops to tiny-skia. Probing only
    // `-wgpu` would then find the committed golden, pass the guard, and
    // let `matches_image` create a fresh `-tiny-skia.png` and return
    // Ok(true) — the exact silent pass this check exists to close.
    let stem = path.strip_suffix(".png").unwrap_or(path);
    let backend_variants = [format!("{stem}-wgpu.png"), format!("{stem}-tiny-skia.png")];

    // `RESONANCE_BLESS=1` re-renders the golden in place.
    //
    // Without it, re-blessing means fighting the two mechanisms below: the
    // absent-file guard refuses to run when the PNG is missing, and
    // `matches_image` only writes a golden when there isn't one. So the manual
    // dance was to rename `foo-wgpu.png` to `foo-tiny-skia.png` (satisfying the
    // guard with a decoy), run the test so the real file gets recreated, then
    // delete the decoy. That is not a procedure anyone should have to
    // rediscover (ba doc #285 §4-T7).
    //
    // The guard still applies to every normal run, which is the case it exists
    // for: a golden lost to a bad merge or a `git clean` must fail loudly
    // rather than regenerate itself and pass forever after.
    if std::env::var("RESONANCE_BLESS").as_deref() == Ok("1") {
        for stale in &backend_variants {
            let _ = std::fs::remove_file(stale);
        }
        snapshot
            .matches_image(path)
            .unwrap_or_else(|err| panic!("blessing {path}: {err}"));
        eprintln!("RESONANCE_BLESS=1: re-rendered golden {path}");
        return;
    }

    assert!(
        backend_variants
            .iter()
            .any(|candidate| std::path::Path::new(candidate).exists()),
        "no golden on disk for {path} — `matches_image` would silently \
         create it and pass. Bless it deliberately, or restore the \
         committed PNG."
    );

    let matched = snapshot
        .matches_image(path)
        .unwrap_or_else(|err| panic!("golden comparison i/o for {path}: {err}"));
    assert!(matched, "snapshot diverged from golden {path}");
}

/// Write the WAV a cycle-record take's `clip_ref` resolves to, inside
/// `project_dir` (epic #15, ba todo #1400).
///
/// A take's waveform is **derived from this file**: the app reads it on
/// the `TakeCaptured` echo and again on project load, and the same read
/// decides whether the lane draws a silhouette or the hatched
/// `media missing` card. So a take-lane fixture gives its takes a real
/// recording, exactly as `recording.rs` streams one to
/// `audio/clip_{id}.wav`. Before #1400 a fixture faked it with a
/// `ClipState` in `Resonance::clips`, which the running app never
/// produces for a take — and that fabrication was what stopped every take
/// drawing as missing media.
///
/// The content is authored **by peak bucket** rather than by sample, so a
/// fixture can pin an exact silhouette: bucket `b` spans
/// `WAVEFORM_PEAK_FRAMES` frames of which the first two are `-amp(b)` and
/// `+amp(b)` and the rest are silent, so
/// [`compute_waveform_peaks`](resonance_audio::types::compute_waveform_peaks)
/// reduces it to exactly `(-amp(b), amp(b))`. `frames` must leave every
/// bucket at least two frames (i.e. `frames >= 2` and
/// `frames % WAVEFORM_PEAK_FRAMES != 1`), or the last bucket's minimum and
/// maximum collapse together.
///
/// 32-bit float stereo at `sample_rate`: the engine's own recording
/// format, and the only one it will memory-map back.
#[allow(dead_code)]
pub fn write_take_wav(
    project_dir: &std::path::Path,
    clip_ref: u64,
    sample_rate: u32,
    frames: u64,
    amp: impl Fn(usize) -> f32,
) {
    let bucket = resonance_audio::types::WAVEFORM_PEAK_FRAMES as u64;
    // Through `clip_audio_file`, never by hand: it is the one definition
    // of a clip's path inside the bundle (todo #412), and the whole point
    // of having one is that a fixture cannot drift from the engine that
    // writes the file or the restore that resolves it.
    let path = project_dir.join(resonance_app::project::clip_audio_file(clip_ref));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create the project's audio dir");
    }
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec).expect("create take wav");
    for f in 0..frames {
        let sample = match f % bucket {
            0 => -amp((f / bucket) as usize),
            1 => amp((f / bucket) as usize),
            _ => 0.0,
        };
        writer.write_sample(sample).expect("write left");
        writer.write_sample(sample).expect("write right");
    }
    writer.finalize().expect("finalize take wav");
}

// ---------------------------------------------------------------------------
// Control-request round trip
// ---------------------------------------------------------------------------
//
// ~90 `control_*`/mixer/compose test modules each hand-rolled their own
// `roundtrip`/`call` pair to drive a `ControlRequest` through `update()`.
// They had already drifted in spelling (parameter names, panic messages,
// the fully- vs. un-qualified `Response` type) without ever differing in
// behavior, which is exactly the kind of copy that silently diverges over
// time. These two are the one canonical shape; every module whose local
// pair was behaviorally identical now imports these instead
// (`use crate::common::{call, roundtrip};`).
//
// A few modules keep a local version because it is NOT an exact
// equivalent: `control_endpoint.rs` parameterizes `conn` (multi-connection
// tests), `control_clip_place.rs` parameterizes the request `id`,
// `control_track_dispatch.rs` and `e2e_compose_via_control.rs` build the
// `Request` differently (no-params shorthand / explicit `id`). Those stay
// put rather than being forced onto this shape.

/// Drive one control request through the full `update()` path on
/// connection 1 and return the single reply the handler produced.
#[allow(dead_code)]
pub fn roundtrip(
    app: &mut resonance_app::Resonance,
    request: resonance_control::Request,
) -> resonance_control::Response {
    let (reply, rx) = resonance_app::control_socket::ReplySender::test_pair();
    let _ = app.update(resonance_app::message::Message::Control(
        resonance_app::control_socket::ControlMessage::Request(
            resonance_app::control_socket::ControlRequest {
                conn: 1,
                request,
                reply,
            },
        ),
    ));
    rx.try_recv().expect("one reply per request")
}

/// Build a `Request` with id 1 for `method`/`params` and drive it through
/// [`roundtrip`].
#[allow(dead_code)]
pub fn call(
    app: &mut resonance_app::Resonance,
    method: &str,
    params: serde_json::Value,
) -> resonance_control::Response {
    roundtrip(
        app,
        resonance_control::Request::new(1, method, &params).expect("params serialize"),
    )
}

// ---------------------------------------------------------------------------
// `app()` constructors
// ---------------------------------------------------------------------------
//
// Deliberately NOT one shape: modules differ on purpose in whether the
// project has an active flag, a saved path (which gates undo recording —
// `can_record_undo`) and a seeded sample rate/tempo map, and that drift
// changes real, observable behavior (e.g. Busy-error semantics). Forcing
// every module onto one `app()` would silently change what some tests
// test. These three constructors only capture setups that were already
// byte-identical (module-independent) across two or more modules; modules
// whose `app()` differs by even a track id or an extra event stay local
// (see e.g. `control_replace_effect.rs` / `plugins/missing_plugin_slot.rs`,
// which share this exact shape but seed a different `TRACK` constant and
// so were left alone).

/// A freshly constructed app on the Arrange tab with nothing else set up:
/// no active project, no saved path. Every module that had exactly this
/// (mostly `control_project.rs`/`control_endpoint.rs`, wrapping the tuple
/// differently, but identical in effect) imports this under the local
/// name `app` via `use crate::common::app_bare as app;`.
#[allow(dead_code)]
pub fn app_bare() -> resonance_app::Resonance {
    resonance_app::Resonance::new_for_test_on(resonance_app::state::ViewMode::Arrange).0
}

/// A bare Arrange-tab app with an active project (but no saved path, so
/// undo recording stays off).
#[allow(dead_code)]
pub fn app_active_project() -> resonance_app::Resonance {
    let (mut app, _task) = resonance_app::Resonance::new_for_test_on(resonance_app::state::ViewMode::Arrange);
    app.test_set_active_project(true);
    app
}

/// An app with an active project, sample rate pinned to 48k and a
/// committed 120 BPM tempo map — the fixture `seed_markers_from_sections`
/// and `markers_overview_ui` both built by hand before seeding markers.
#[allow(dead_code)]
pub fn app_with_tempo_120() -> resonance_app::Resonance {
    let (mut app, _task) = resonance_app::Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_sample_rate(48_000);
    let _ = app.update(resonance_app::message::Message::Transport(
        resonance_app::message::TransportMessage::SetBpmText("120".into()),
    ));
    let _ = app.update(resonance_app::message::Message::Transport(
        resonance_app::message::TransportMessage::CommitBpm,
    ));
    app
}
