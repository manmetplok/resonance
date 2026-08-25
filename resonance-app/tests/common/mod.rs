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
    let audio_dir = project_dir.join("audio");
    std::fs::create_dir_all(&audio_dir).expect("create the project's audio dir");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let path = audio_dir.join(format!("clip_{clip_ref}.wav"));
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
