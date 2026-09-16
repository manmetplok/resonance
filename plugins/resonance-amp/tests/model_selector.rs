//! The model selector must not move a model the user did not move.
//!
//! `file_select` is an index into `file_list`, and `process()` reads any
//! `file_select != last_file_index` as "the user picked a new model" and
//! loads `file_list[file_select]`. That makes the baseline the whole
//! safety property: an activation that leaves it unset makes the FIRST
//! `process()` call request index 0 — the alphabetically first profile in
//! the directory — over whatever is actually loaded.
//!
//! Which is not where the symptom shows up. The mixer skips the
//! arrangement render while the transport is stopped, so an effect on an
//! audio track does not process at all until playback starts or the track
//! is monitored / record-armed. The bogus load therefore surfaces one
//! user action later, as "arming the track swapped my amp model".
//!
//! Its own binary, and ONE test in it: the scratch downloads directory is
//! selected with `XDG_DATA_HOME`, and an env var is process-wide, so the
//! phases run in sequence rather than as parallel `#[test]`s.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use resonance_amp::ResonanceAmp;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use serde_json::Value;

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 256;

/// The `file_select` slot in host order (`AmpParams::param_at`).
const FILE_SELECT: usize = 0;

/// Generous: a load parses a real `.nam`, primes it twice over 2048
/// samples and sweeps a 256-point transfer curve, all on the loader
/// thread, under a debug build.
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Floor for the "definitely did not load" grace window, and the value
/// used before any real load on this machine has been measured. A flat
/// constant here can't track how fast (or slow) a real load actually is —
/// a machine where a load takes 4x this would pass a real defect quietly,
/// and a heavily loaded CI box could in principle need longer than this to
/// even prove a *negative*. Once a real load has been timed (see
/// `pump_until_loaded` below), later grace windows scale off of it
/// instead.
const MIN_GRACE: Duration = Duration::from_millis(250);

/// How much slower than a real, measured load a grace window waits before
/// concluding nothing is coming. 5x a genuine load's wall-clock cost is
/// generous enough that a slow-but-legitimate load never reads as "did not
/// load", while still bounding the wait (see `MAX_GRACE`) instead of
/// hanging on a broken loader.
const GRACE_MULTIPLIER: u32 = 5;

/// Absolute cap on a scaled grace window, so a pathological measurement
/// (a one-off scheduling hiccup during calibration) can't make the suite
/// hang.
const MAX_GRACE: Duration = Duration::from_secs(5);

/// Scale a "definitely did not load" wait off of how long a real load was
/// just measured to take, rather than guessing a flat constant — the
/// previous fixed 250 ms window's own comment admitted it was a guess.
/// Floored at `MIN_GRACE` and capped at `MAX_GRACE`.
fn scaled_grace(measured_load: Duration) -> Duration {
    (measured_load * GRACE_MULTIPLIER).clamp(MIN_GRACE, MAX_GRACE)
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/a1")
}

fn run_blocks(plugin: &mut ResonanceAmp, blocks: usize) {
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    for _ in 0..blocks {
        left.fill(0.0);
        right.fill(0.0);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
    }
}

/// The `model_path` the plugin would persist right now. Written by the
/// loader thread on every completed load, so it doubles as the public
/// answer to "which profile is loaded".
fn model_path_of(plugin: &ResonanceAmp) -> String {
    let state: Value =
        serde_json::from_slice(&plugin.save_state()).expect("save_state must emit valid JSON");
    state
        .get("model_path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Play the blocks an arm / monitor toggle would, then give the loader
/// thread `grace` to act on anything they queued. Pass `MIN_GRACE` before
/// any real load on this machine has been timed, and `scaled_grace(...)`
/// of a measured one once available — see the constants above.
fn arm_and_settle(plugin: &mut ResonanceAmp, grace: Duration) {
    run_blocks(plugin, 64);
    std::thread::sleep(grace);
    run_blocks(plugin, 64);
}

/// Pump audio until `model_path` reaches `want`, or the timeout expires.
/// Returns whatever it ended up as, so the caller asserts on the value
/// rather than on a bare bool.
fn pump_until_loaded(plugin: &mut ResonanceAmp, want: &str) -> String {
    let deadline = Instant::now() + LOAD_TIMEOUT;
    loop {
        run_blocks(plugin, 4);
        let current = model_path_of(plugin);
        if current == want || Instant::now() >= deadline {
            return current;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_selector_only_loads_what_the_user_picked() {
    // Copy the two A1 fixtures into a scratch downloads directory and
    // point `XDG_DATA_HOME` at its root, so `models::models_dir()`
    // resolves there instead of at the developer's real Tone3000
    // downloads. `scan_directory` sorts, and `.` sorts below `_`, so
    // `wavenet.nam` is index 0 — the file a spurious load lands on — and
    // `wavenet_a1_standard.nam` is index 1.
    let root = std::env::temp_dir().join(format!("resonance-amp-selector-{}", std::process::id()));
    let models = root.join("resonance/amp-models/tone3000");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&models).expect("create scratch downloads dir");
    let mut seeded: Vec<String> = ["wavenet.nam", "wavenet_a1_standard.nam"]
        .into_iter()
        .map(|name| {
            let dest = models.join(name);
            std::fs::copy(fixture_dir().join(name), &dest).expect("copy fixture");
            dest.to_string_lossy().into_owned()
        })
        .collect();
    seeded.sort();
    std::env::set_var("XDG_DATA_HOME", &root);

    // -- A freshly added amp -------------------------------------------
    // No persisted model, so nothing should be playing and nothing should
    // start playing on its own.
    let mut amp = ResonanceAmp::new();
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    assert_eq!(
        model_path_of(&amp),
        "",
        "initialize() must not load a model when none was persisted"
    );

    // No real load has been measured yet on this run, so this first
    // "definitely did not load" check uses the floor rather than a scaled
    // window.
    arm_and_settle(&mut amp, MIN_GRACE);
    assert_eq!(
        model_path_of(&amp),
        "",
        "arming the track loaded {} on its own — the selector's baseline \
         was left unset, so the first process() read its default 0 as a \
         fresh pick",
        seeded[0]
    );

    // The browser IS seeded from the downloads directory, though: that is
    // the other half of the bug — a fresh amp used to offer an empty
    // ◀/▶ browser even with profiles already downloaded. Selecting index
    // 1 only reaches the second seeded file if the list was populated.
    //
    // Timed rather than just awaited: a real load's measured wall-clock
    // cost is what later "definitely did not load" windows scale off of,
    // so a slow machine gets a proportionally longer grace instead of the
    // same flat guess as a fast one.
    let load_started = Instant::now();
    amp.param(FILE_SELECT).set_plain(1.0);
    assert_eq!(
        pump_until_loaded(&mut amp, &seeded[1]),
        seeded[1],
        "selecting index 1 did not load the second downloaded profile — \
         the model browser was not seeded from the downloads directory"
    );
    let grace = scaled_grace(load_started.elapsed());
    drop(amp);

    // -- A project reopening with a stale selector ---------------------
    // `file_select` only reaches the shared atomics a state blob is
    // written from while `process()` is running, so a model picked with
    // the transport stopped persists its path next to a stale index. The
    // path is the authority; the index must be re-derived from it.
    let target = fixture_dir()
        .join("wavenet_a1_standard.nam")
        .to_string_lossy()
        .into_owned();
    let blob = serde_json::json!({
        "model_path": target,
        "params": { "file_select": 0.0, "input_gain": 1.0, "output_gain": 0.5 },
        "version": 1,
    });

    let mut amp = ResonanceAmp::new();
    assert!(amp.load_state(&serde_json::to_vec(&blob).unwrap()));
    assert!(amp.initialize(SAMPLE_RATE, BLOCK as u32));
    assert_eq!(
        model_path_of(&amp),
        target,
        "initialize() must restore the persisted model, not the selector's \
         stale index"
    );
    assert_eq!(
        amp.param(FILE_SELECT).get_plain(),
        1.0,
        "the selector must be re-derived from the restored model path"
    );

    arm_and_settle(&mut amp, grace);
    assert_eq!(
        model_path_of(&amp),
        target,
        "the restored model was swapped out by the first blocks after \
         activation"
    );
    drop(amp);

    let _ = std::fs::remove_dir_all(&root);
}
