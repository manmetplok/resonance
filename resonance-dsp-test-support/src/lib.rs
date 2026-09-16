//! Mechanical plumbing shared by the plugins' `dsp_golden.rs` (and
//! equivalent) tests.
//!
//! Nine plugin crates independently grew a golden-test harness that
//! renders a table of scenarios, compares the result against a binary
//! fixture on disk, and re-writes that fixture when `RESONANCE_BLESS=1`
//! (CLAUDE.md's workspace-wide convention). The IO, the bless-mode check
//! and the diff loop were copy-pasted crate to crate; this crate is that
//! plumbing, extracted so a change to the *mechanism* (say, a clearer
//! diagnostic) lands once instead of nine times.
//!
//! What stays out on purpose, per crate: the scenario tables, every
//! threshold, the choice between bit-exact and an epsilon budget, and
//! any extra guard (residual-energy checks, tail-energy checks, and so
//! on). Those are DSP policy, not plumbing, and they diverge between
//! crates deliberately.
//!
//! Two on-disk shapes exist and both are supported: a stream of `f32`
//! samples (most plugins) and a stream of raw `u32` words (drums and
//! granular-delay, which fold per-block digests in alongside the
//! sample bits). An `f32` is just its bits reinterpreted, so the `_f32`
//! functions are thin wrappers over the `_words` ones.

use std::path::{Path, PathBuf};

/// True if any of `vars` is set to exactly `"1"`. `RESONANCE_BLESS` is
/// the workspace-wide convention (CLAUDE.md); most callers layer a
/// narrower, test-specific name on top of it so one golden can be
/// reblessed inside a wider run without reblessing everything else.
pub fn blessed(vars: &[&str]) -> bool {
    vars.iter().any(|v| std::env::var(v).as_deref() == Ok("1"))
}

/// The path to a golden fixture: `<manifest_dir>/tests/golden/<file_name>`.
/// Callers pass `env!("CARGO_MANIFEST_DIR")` from their own crate — that
/// macro expands at the call site, not in here, so it always names the
/// plugin crate under test rather than this support crate.
pub fn golden_path(manifest_dir: &str, file_name: &str) -> PathBuf {
    PathBuf::from(manifest_dir).join("tests/golden").join(file_name)
}

fn bless_bytes(path: &Path, bytes: &[u8], count: usize, unit: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    eprintln!("blessed golden: {count} {unit} -> {}", path.display());
}

/// Writes `samples` as a little-endian `f32` blob, creating the parent
/// directory if needed, and prints the standard "blessed golden"
/// diagnostic.
pub fn bless_f32(path: &Path, samples: &[f32]) {
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    bless_bytes(path, &bytes, samples.len(), "samples");
}

/// Writes `words` as a little-endian `u32` blob. See [`bless_f32`].
pub fn bless_words(path: &Path, words: &[u32]) {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    bless_bytes(path, &bytes, words.len(), "words");
}

fn read_or_panic(path: &Path, bless_hint: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| {
        panic!("missing golden {}: {e}\nregenerate with {bless_hint}", path.display())
    })
}

/// Reads a golden `f32` blob, panicking with a re-bless hint if it is
/// missing, and asserting its length matches `expected_samples` (a
/// mismatch means the scenario table changed shape, not that a sample
/// moved).
pub fn load_golden_f32(path: &Path, expected_samples: usize, bless_hint: &str) -> Vec<f32> {
    let bytes = read_or_panic(path, bless_hint);
    assert_eq!(
        bytes.len(),
        expected_samples * 4,
        "golden length mismatch — the scenario set changed"
    );
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Reads a golden `u32` blob. See [`load_golden_f32`].
pub fn load_golden_words(path: &Path, expected_words: usize, bless_hint: &str) -> Vec<u32> {
    let bytes = read_or_panic(path, bless_hint);
    assert_eq!(
        bytes.len(),
        expected_words * 4,
        "golden length mismatch — the scenario set changed"
    );
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

/// The result of comparing a rendered `f32` stream against its golden:
/// an exact bitwise diff count, the first differing sample, and the peak
/// / RMS deviation. Bit-exact callers assert `first_diff.is_none()`;
/// callers that tolerate an epsilon (FFT-based DSP, where the same
/// transform legitimately rounds differently per CPU) assert `max_abs`
/// and `rms_err` stay under their own budget instead.
pub struct F32Diff {
    pub diff_count: usize,
    pub max_abs: f32,
    pub first_diff: Option<(usize, f32, f32)>,
    pub rms_err: f64,
}

/// Diffs two equal-length `f32` streams. Panics (via the `zip`'s implicit
/// truncation, silently — callers must check lengths themselves, which
/// `load_golden_f32`'s length assertion already guarantees) if they are
/// mismatched, since a length mismatch is a scenario-table change and is
/// reported earlier and more clearly by `load_golden_f32`.
pub fn compare_f32(rendered: &[f32], golden: &[f32]) -> F32Diff {
    let mut diff_count = 0usize;
    let mut max_abs = 0.0f32;
    let mut first_diff = None;
    let mut sq_err = 0.0f64;
    for (i, (a, b)) in rendered.iter().zip(golden.iter()).enumerate() {
        let d = a - b;
        sq_err += (d as f64) * (d as f64);
        if a.to_bits() != b.to_bits() {
            diff_count += 1;
            if d.abs() > max_abs {
                max_abs = d.abs();
            }
            if first_diff.is_none() {
                first_diff = Some((i, *a, *b));
            }
        }
    }
    let rms_err = (sq_err / rendered.len().max(1) as f64).sqrt();
    F32Diff { diff_count, max_abs, first_diff, rms_err }
}

/// The result of comparing a rendered `u32` word stream against its
/// golden — exact equality only, no notion of "how far off": the words
/// are as often digest hashes as they are sample bit-patterns, and a
/// hash has no meaningful magnitude to report.
pub struct WordDiff {
    pub diff_count: usize,
    pub first_diff: Option<(usize, u32, u32)>,
}

/// Diffs two equal-length `u32` streams. See [`compare_f32`] for the
/// length-mismatch note.
pub fn compare_words(rendered: &[u32], golden: &[u32]) -> WordDiff {
    let mut diff_count = 0usize;
    let mut first_diff = None;
    for (i, (a, b)) in rendered.iter().copied().zip(golden.iter().copied()).enumerate() {
        if a != b {
            diff_count += 1;
            if first_diff.is_none() {
                first_diff = Some((i, a, b));
            }
        }
    }
    WordDiff { diff_count, first_diff }
}

/// The fraction of `out`'s energy that is *not* explained by the
/// best-fit scalar multiple of `dry` — how far `out` departs from being
/// a plain gain change on its input. Used by the "this scenario isn't
/// secretly a bypass" guards: a delay's repeats, an EQ's filtering and a
/// convolver's IR are all, by construction, not a scaled copy of the dry
/// signal, so a residual near zero means the effect did nothing.
pub fn residual_fraction(out: &[f32], dry: &[f32]) -> f64 {
    let dot: f64 = out
        .iter()
        .zip(dry)
        .map(|(o, d)| (*o as f64) * (*d as f64))
        .sum();
    let den: f64 = dry.iter().map(|d| (*d as f64) * (*d as f64)).sum();
    let k = dot / den.max(1e-30);
    let resid: f64 = out
        .iter()
        .zip(dry)
        .map(|(o, d)| {
            let e = *o as f64 - k * *d as f64;
            e * e
        })
        .sum();
    let energy: f64 = out.iter().map(|o| (*o as f64) * (*o as f64)).sum();
    resid / energy
}
