//! Where the workspace's own plugin binaries are, for the tests that load
//! a real one — and a loud failure when one is missing.
//!
//! These tests used to return early with a `[skip]` line when the binary
//! was absent, so a checkout that never built the plugins passed them
//! without running a single assertion (and `./scripts/run-tests.py` never
//! built them). The suite script now builds the cdylibs these tests need
//! (`PLUGIN_CDYLIBS` in `scripts/run-tests.py`), so a missing binary is a
//! broken setup and fails the test. Set
//! `RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES=1` to skip instead, e.g. for a
//! plain `cargo test -p resonance-audio` that did not build them.

use std::path::PathBuf;

/// The env opt-out that turns a missing binary back into a skip.
pub const ALLOW_MISSING: &str = "RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES";

/// The binary of plugin crate `crate_name` (e.g. `"resonance-eq"`):
/// the debug cdylib cargo builds next to this test binary
/// (`target/<profile>/libresonance_eq.so`), or else the bundle
/// `scripts/bundle.sh` writes (`target/bundled/resonance-eq.clap`).
///
/// Missing, it panics with how to build it — or, with [`ALLOW_MISSING`]
/// set, prints a skip line and returns `None`.
pub fn plugin_binary(crate_name: &str) -> Option<PathBuf> {
    let cdylib = format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        crate_name.replace('-', "_"),
        std::env::consts::DLL_SUFFIX
    );
    let mut candidates = Vec::new();
    // `target/<profile>/deps/<test binary>` -> `target/<profile>/`: the
    // right profile and target dir, whatever `CARGO_TARGET_DIR` says.
    if let Some(profile_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent().map(PathBuf::from))
    {
        candidates.push(profile_dir.join(&cdylib));
        if let Some(target) = profile_dir.parent() {
            candidates.push(target.join("bundled").join(format!("{crate_name}.clap")));
        }
    }
    if let Some(found) = candidates.iter().find(|p| p.exists()) {
        return Some(found.clone());
    }
    let how = format!(
        "no {crate_name} binary (looked for {}). ./scripts/run-tests.py builds it; by hand, \
         `cargo build -p {crate_name}` (or scripts/bundle.sh). Set {ALLOW_MISSING}=1 to skip \
         the tests that need it instead",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if std::env::var_os(ALLOW_MISSING).is_some_and(|v| v != "0" && !v.is_empty()) {
        eprintln!("[skip] {how}");
        return None;
    }
    panic!("{how}");
}
