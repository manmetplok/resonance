//! The workspace's architecture rules, as tests (ARCH-10).
//!
//! `ARCHITECTURE.md` and `CLAUDE.md` state rules that nothing used to
//! check: an illegal crate edge, a plugin naming the Wayland runtime, an
//! inline `#[cfg(test)]` module, a new top-level app test binary — all of
//! them compiled fine. With autonomous agents landing code in parallel,
//! prose rules erode one PR at a time (the MCP/skill lockstep test in
//! `resonance-mcp/tests/agent_plugin_lockstep.rs` is the one rule that
//! never did, because it is a test). Each test below quotes the sentence
//! it enforces and says how it was exercised once: rule flipped, test
//! failed, rule restored.
//!
//! This crate links nothing from the workspace on purpose: it reads
//! `cargo metadata --no-deps` and walks the tree, so it builds in seconds
//! and is never the target that is skipped because it is slow.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Workspace model
// ---------------------------------------------------------------------------

/// `tools/arch-invariants` → `tools` → the workspace root.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate lives two levels below the workspace root")
        .to_path_buf()
}

#[derive(Debug)]
struct Dep {
    name: String,
    /// `normal`, `dev` or `build`.
    kind: String,
    /// The `cfg(...)` the dependency is gated on, if any.
    target: Option<String>,
}

#[derive(Debug)]
struct Package {
    name: String,
    /// Directory holding the crate's `Cargo.toml`.
    dir: PathBuf,
    /// Every declared dependency of every kind and target — what the
    /// manifest says, not what resolves on this host, so a macOS-only
    /// edge is checked from Linux too.
    deps: Vec<Dep>,
    crate_types: BTreeSet<String>,
}

impl Package {
    fn rel_dir(&self, root: &Path) -> PathBuf {
        self.dir.strip_prefix(root).unwrap_or(&self.dir).to_path_buf()
    }

    /// A CLAP plugin: a crate directly under `plugins/`.
    fn is_plugin(&self, root: &Path) -> bool {
        let rel = self.rel_dir(root);
        rel.parent() == Some(Path::new("plugins"))
    }
}

/// The declared workspace, straight from cargo. `--no-deps` keeps this
/// to the manifests (no resolve, no network); `--offline` makes sure.
fn packages() -> Vec<Package> {
    let root = workspace_root();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("spawn cargo metadata");
    assert!(
        out.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md: Value = serde_json::from_slice(&out.stdout).expect("cargo metadata emits JSON");
    md["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .map(|p| {
            let manifest = PathBuf::from(p["manifest_path"].as_str().expect("manifest_path"));
            Package {
                name: p["name"].as_str().expect("name").to_owned(),
                dir: manifest.parent().expect("manifest has a directory").to_path_buf(),
                deps: p["dependencies"]
                    .as_array()
                    .expect("dependencies array")
                    .iter()
                    .map(|d| Dep {
                        name: d["name"].as_str().expect("dep name").to_owned(),
                        kind: d["kind"].as_str().unwrap_or("normal").to_owned(),
                        target: d["target"].as_str().map(str::to_owned),
                    })
                    .collect(),
                crate_types: p["targets"]
                    .as_array()
                    .expect("targets array")
                    .iter()
                    .flat_map(|t| t["crate_types"].as_array().into_iter().flatten())
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect(),
            }
        })
        .collect()
}

/// Every `*.rs` under `dir`, recursively, skipping build output.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(line number, line)` pairs with `//` comments cut off, so a rule
/// about code is not tripped by a sentence about the rule.
fn code_lines(path: &Path) -> Vec<(usize, String)> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let code = line.split("//").next().unwrap_or("");
            (i + 1, code.to_owned())
        })
        .collect()
}

fn report(rule: &str, violations: &[String]) {
    assert!(
        violations.is_empty(),
        "{rule}\n\n{}\n",
        violations
            .iter()
            .map(|v| format!("  - {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------------------------------------------------------------------------
// Crate layering (ARCHITECTURE.md → "Crate Layering")
// ---------------------------------------------------------------------------

/// What every crate under `plugins/` may depend on, any dependency kind.
///
/// ARCHITECTURE.md: "`resonance-common` ──► ... amp/drums/ir plugins" (which
/// ones: `only_listed_plugins_depend_on_resonance_common`),
/// "`resonance-plugin` ──► every plugin", "`resonance-dsp` ──► (every FX
/// plugin)", the metering and music-theory arrows, and `plugin-gui-core`
/// as "the platform-neutral half of the editor stack". The platform
/// runtimes are deliberately absent: "no plugin names one" — a plugin
/// reaches `wayland-plugin-gui`/`cocoa-plugin-gui` only through
/// `resonance-plugin`'s `editor-widgets` feature (ARCH-08). The one
/// exception is a *dev* dependency gated on a target, which is what the
/// gate's Cocoa round-trip test needs for the NSApplication pump; see
/// `plugins_never_name_a_platform_runtime`.
const PLUGIN_DEPS: &[&str] = &[
    "resonance-plugin",
    "resonance-common",
    "resonance-dsp",
    "resonance-metering",
    "resonance-music-theory",
    "plugin-gui-core",
    "resonance-dsp-test-support",
];

/// The edge list of ARCHITECTURE.md's crate diagram, per crate, any
/// dependency kind (a dev-dependency on the app from the audio crate would
/// be exactly as wrong as a normal one). A crate missing from this table
/// is a new crate, and adding it here *is* the layering decision — make it
/// in ARCHITECTURE.md first.
fn allowed_internal_deps(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        // "framework-agnostic — no Iced, no CLAP, no plugin trait"
        "resonance-dsp" | "resonance-common" => &[],
        "resonance-metering" => &["resonance-dsp"],
        "resonance-dsp-test-support" => &[],
        // "pure music theory ... does not depend on audio, app, or plugin code"
        "resonance-music-theory" => &[],
        // "depends only on `resonance-music-theory`"
        "resonance-svs" => &["resonance-music-theory"],
        // "zero internal deps and ... the single wire contract" (review, Architecture)
        "resonance-control" => &[],
        "resonance-mcp" => &["resonance-control"],
        // "does not depend on `resonance-app`"; the music-theory edge is the
        // sanctioned narrow one (doc #160 / todo #358).
        "resonance-audio" => &[
            "resonance-dsp",
            "resonance-metering",
            "resonance-common",
            "resonance-music-theory",
        ],
        // "No windowing code; builds on every OS."
        "plugin-gui-core" => &[],
        "wayland-plugin-gui" | "cocoa-plugin-gui" => &["plugin-gui-core"],
        // The SDK: the only crate that names both runtimes (cfg-selected).
        "resonance-plugin" => &[
            "resonance-common",
            "plugin-gui-core",
            "wayland-plugin-gui",
            "cocoa-plugin-gui",
        ],
        // This crate reads the workspace; it never links it.
        "arch-invariants" => &[],
        _ => return None,
    })
}

/// ARCHITECTURE.md → Crate Layering: "The workspace is a deliberate DAG
/// ... the lower layers know nothing about the upper layers", followed by
/// the hard rules quoted in `allowed_internal_deps`. `resonance-app` "is
/// allowed to depend on everything; it is the integration layer", so it is
/// the one crate with no row.
///
/// Exercised 2026-09-26: added `resonance-app = { path = "../resonance-app" }`
/// to `resonance-audio/Cargo.toml` → failed with
/// `resonance-audio -> resonance-app (normal)`; reverted.
#[test]
fn crate_dag_matches_architecture_md() {
    let root = workspace_root();
    let pkgs = packages();
    let internal: BTreeSet<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    let mut violations = Vec::new();
    for p in &pkgs {
        if p.name == "resonance-app" {
            continue;
        }
        let allowed = if p.is_plugin(&root) {
            PLUGIN_DEPS
        } else {
            match allowed_internal_deps(&p.name) {
                Some(a) => a,
                None => {
                    violations.push(format!(
                        "{}: not in the layering table — decide its layer in \
                         ARCHITECTURE.md, then add a row to `allowed_internal_deps`",
                        p.name
                    ));
                    continue;
                }
            }
        };
        for d in &p.deps {
            // Self dev-deps (a crate turning its own feature on for tests)
            // and external crates are not edges of the diagram.
            if d.name == p.name || !internal.contains(d.name.as_str()) {
                continue;
            }
            // A plugin's target-gated *dev* dependency on a platform runtime
            // is a test harness (the gate's Cocoa NSApplication pump), not a
            // layering edge; `plugins_never_name_a_platform_runtime` owns
            // that carve-out and rejects anything wider.
            if p.is_plugin(&root) && d.kind == "dev" && d.target.is_some() {
                continue;
            }
            if !allowed.contains(&d.name.as_str()) {
                violations.push(format!(
                    "{} -> {} ({}{}) is not an edge of ARCHITECTURE.md's crate diagram",
                    p.name,
                    d.name,
                    d.kind,
                    d.target.as_deref().map(|t| format!(", {t}")).unwrap_or_default()
                ));
            }
        }
    }
    report(
        "ARCHITECTURE.md → Crate Layering: every internal dependency must be an edge of the diagram",
        &violations,
    );
}

/// ARCHITECTURE.md: "`resonance-dsp`, `resonance-metering`,
/// `resonance-common` are framework-agnostic — no Iced, no CLAP";
/// "`resonance-audio` ... still doesn't know about Iced"; `plugin-gui-core`
/// is "the platform-neutral half"; the windowing stacks live in their
/// runtime crate only. And since ARCH-08, `iced` is the app's alone: the
/// plugin SDK ships no host-GUI toolkit, and `egui` reaches plugins as
/// `plugin_gui_core::egui`, not as a dependency of their own.
///
/// Exercised 2026-09-26: re-added `iced = { workspace = true, optional =
/// true }` to `resonance-plugin/Cargo.toml` → failed with
/// `resonance-plugin -> iced`; reverted.
#[test]
fn framework_crates_stay_where_the_diagram_puts_them() {
    let pkgs = packages();
    // (dependency name prefix, crates that may name it directly)
    let table: &[(&str, &[&str])] = &[
        ("iced", &["resonance-app"]),
        ("iced_test", &["resonance-app"]),
        ("egui", &["plugin-gui-core", "wayland-plugin-gui", "cocoa-plugin-gui"]),
        ("egui_glow", &["wayland-plugin-gui", "cocoa-plugin-gui"]),
        ("wayland-client", &["wayland-plugin-gui"]),
        ("wayland-protocols", &["wayland-plugin-gui"]),
        ("wayland-egl", &["wayland-plugin-gui"]),
        ("smithay-client-toolkit", &["wayland-plugin-gui"]),
        ("khronos-egl", &["wayland-plugin-gui"]),
        ("objc2", &["cocoa-plugin-gui"]),
        ("objc2-foundation", &["cocoa-plugin-gui"]),
        ("objc2-app-kit", &["cocoa-plugin-gui"]),
        ("dispatch2", &["cocoa-plugin-gui"]),
    ];
    // CLAP is the plugin ABI: the SDK speaks it, the host speaks it, the
    // plugins' in-process host tests speak it. Nothing below them does.
    let no_clap: &[&str] = &[
        "resonance-dsp",
        "resonance-metering",
        "resonance-common",
        "resonance-music-theory",
        "resonance-svs",
        "resonance-control",
        "resonance-mcp",
        "plugin-gui-core",
        "wayland-plugin-gui",
        "cocoa-plugin-gui",
    ];
    let mut violations = Vec::new();
    for p in &pkgs {
        for d in &p.deps {
            if let Some((_, owners)) = table.iter().find(|(dep, _)| *dep == d.name) {
                if !owners.contains(&p.name.as_str()) {
                    violations.push(format!("{} -> {} ({})", p.name, d.name, d.kind));
                }
            }
            if no_clap.contains(&p.name.as_str())
                && (d.name.starts_with("clack-") || d.name == "clap-sys")
            {
                violations.push(format!("{} -> {} ({})", p.name, d.name, d.kind));
            }
        }
    }
    report(
        "ARCHITECTURE.md → Crate Layering: GUI toolkits, windowing stacks and CLAP belong to specific crates",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Plugins (ARCHITECTURE.md → "Plugin Pattern"; scripts/bundle.sh)
// ---------------------------------------------------------------------------

/// ARCHITECTURE.md: the platform runtimes are reached "through
/// `resonance_plugin::editor_host`", whose module doc promises that the
/// `RuntimeEditor` alias and `native_api` "are the only places in the
/// plugin stack that name a platform". So: no plugin *source* mentions a
/// runtime crate, and no plugin *manifest* depends on one except as a
/// target-gated dev-dependency (a test harness such as the Cocoa
/// NSApplication pump, which the SDK does not re-export).
///
/// Exercised 2026-09-26: (a) restored `wayland-plugin-gui = { path = ...,
/// optional = true }` in `plugins/resonance-gate/Cargo.toml` → failed on
/// the manifest; (b) added `use wayland_plugin_gui::Editor;` to
/// `plugins/resonance-gate/src/editor/mod.rs` → failed on the source
/// line; both reverted.
#[test]
fn plugins_never_name_a_platform_runtime() {
    let root = workspace_root();
    let runtimes = ["wayland-plugin-gui", "cocoa-plugin-gui"];
    let idents = ["wayland_plugin_gui", "cocoa_plugin_gui"];
    let mut violations = Vec::new();
    for p in packages().iter().filter(|p| p.is_plugin(&root)) {
        for d in p.deps.iter().filter(|d| runtimes.contains(&d.name.as_str())) {
            if !(d.kind == "dev" && d.target.is_some()) {
                violations.push(format!(
                    "{}/Cargo.toml depends on {} ({}) — reach the runtime via \
                     `resonance-plugin/editor-widgets` instead",
                    p.rel_dir(&root).display(),
                    d.name,
                    d.kind
                ));
            }
        }
        let mut files = Vec::new();
        rust_files(&p.dir.join("src"), &mut files);
        for file in files {
            for (n, code) in code_lines(&file) {
                if idents.iter().any(|id| code.contains(id)) {
                    violations.push(format!(
                        "{}:{n}: names a platform runtime; import from `resonance_plugin::editor_host`",
                        file.strip_prefix(&root).unwrap_or(&file).display()
                    ));
                }
            }
        }
    }
    report(
        "ARCHITECTURE.md / editor_host.rs: only `resonance_plugin::editor_host` names a platform runtime",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Plugin reach into resonance-common (ARCH-07)
// ---------------------------------------------------------------------------

/// The `resonance_common::` items plugin code may name: file/preset/
/// content utilities, never DAW model types. `flush_denormals` is not
/// here — it moved to `resonance-dsp` (ARCH-07 A7-2).
const PLUGIN_COMMON_ITEMS: &[&str] = &[
    "scan_directory",      // amp, ir; resonance-plugin's loader
    "registry",            // drums: downloadable kit content
    "drum_map",            // drums: the GM pad contract shared with the app
    "decode_wav_stereo",   // drums: sample decode
    "decode_wav_channels", // ir: impulse-response decode
    "factory_presets",     // resonance-plugin: the factory-preset codec
];

/// The plugins that declare a `resonance-common` dependency at all. The
/// other plugins reach only `resonance-dsp`/`resonance-plugin`; a new edge
/// is a decision, not a side effect of an auto-import.
const PLUGINS_ON_COMMON: &[&str] = &["resonance-amp", "resonance-drums", "resonance-ir"];

/// The item(s) a `resonance_common` occurrence names: `::x` → `x`,
/// `::{a, b::{c}}` → `a, b` (`self` skipped), a bare crate name → `""`.
fn common_items_named(rest: &str) -> Vec<String> {
    let head = |s: &str| {
        s.trim()
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '*'))
            .next()
            .unwrap_or("")
            .to_owned()
    };
    let Some(path) = rest.strip_prefix("::") else {
        return vec![String::new()];
    };
    let Some(group) = path.strip_prefix('{') else {
        return vec![head(path)];
    };
    let mut depth = 0usize;
    let mut items = vec![String::new()];
    for c in group.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => break,
            '}' => depth -= 1,
            ',' if depth == 0 => items.push(String::new()),
            _ if depth == 0 => items.last_mut().expect("non-empty").push(c),
            _ => {}
        }
    }
    items
        .iter()
        .map(|i| head(i))
        .filter(|i| !i.is_empty() && i != "self")
        .collect()
}

/// ARCHITECTURE.md → Crate Layering: "Plugins reach `resonance-common`
/// only for utilities … DAW model types are not plugin API". Every
/// `resonance_common::<item>` in a plugin crate or `resonance-plugin` (any
/// target: src, tests, benches, examples, build.rs) must be in
/// `PLUGIN_COMMON_ITEMS`; a bare `resonance_common` (`use resonance_common
/// as rc;`) or a glob would hide the item, so both fail too.
///
/// Exercised 2026-09-26: added `use resonance_common::Take;` to
/// `plugins/resonance-drums/src/lib.rs` → failed on that line; reverted.
#[test]
fn plugins_reach_only_common_utilities() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for p in packages()
        .iter()
        .filter(|p| p.is_plugin(&root) || p.name == "resonance-plugin")
    {
        let mut files = Vec::new();
        rust_files(&p.dir, &mut files);
        for file in files {
            let text: String = code_lines(&file).iter().map(|(_, l)| format!("{l}\n")).collect();
            let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
            for (off, _) in text.match_indices("resonance_common") {
                let before = text[..off].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let rest = &text[off + "resonance_common".len()..];
                for item in common_items_named(rest) {
                    if !PLUGIN_COMMON_ITEMS.contains(&item.as_str()) {
                        let shown = if item.is_empty() { "<bare crate name>" } else { &item };
                        violations.push(format!(
                            "{rel}:{}: resonance_common::{shown} — DAW model types are not \
                             plugin API; add the utility to `PLUGIN_COMMON_ITEMS` or move it \
                             down (e.g. into resonance-dsp)",
                            text[..off].matches('\n').count() + 1
                        ));
                    }
                }
            }
        }
    }
    report(
        "ARCHITECTURE.md: plugins reach resonance-common only for listed utilities (ARCH-07)",
        &violations,
    );
}

/// ARCH-07: a plugin that does not depend on `resonance-common` must not
/// gain the dependency silently, and the list must not go stale when one
/// drops it. Any dependency kind counts.
///
/// Exercised 2026-09-26: re-added `resonance-common = { path = ... }` to
/// `plugins/resonance-gate/Cargo.toml` → failed with "not in
/// `PLUGINS_ON_COMMON`"; reverted.
#[test]
fn only_listed_plugins_depend_on_resonance_common() {
    let root = workspace_root();
    let mut violations = Vec::new();
    let mut seen = BTreeSet::new();
    for p in packages().iter().filter(|p| p.is_plugin(&root)) {
        if !p.deps.iter().any(|d| d.name == "resonance-common") {
            continue;
        }
        seen.insert(p.name.clone());
        if !PLUGINS_ON_COMMON.contains(&p.name.as_str()) {
            violations.push(format!(
                "{}/Cargo.toml depends on resonance-common but is not in `PLUGINS_ON_COMMON` — \
                 use resonance-dsp/resonance-plugin, or add it to the list deliberately",
                p.rel_dir(&root).display()
            ));
        }
    }
    for name in PLUGINS_ON_COMMON.iter().filter(|n| !seen.contains(**n)) {
        violations.push(format!(
            "{name} is in `PLUGINS_ON_COMMON` but no longer depends on resonance-common — \
             drop it from the list"
        ));
    }
    report(
        "ARCH-07: only the plugins in `PLUGINS_ON_COMMON` depend on resonance-common",
        &violations,
    );
}

/// `scripts/bundle.sh` derives the bundle from `plugins/*/` and cross-checks
/// it against the workspace members both ways, then requires every plugin
/// to be a cdylib — but only when someone bundles. The same three checks,
/// in the suite: a plugin crate added without a `members` entry (the
/// ba todo #1073 failure mode), a member the bundle glob cannot see, and a
/// helper rlib parked under `plugins/` all fail here first.
///
/// Exercised 2026-09-26: removed `"plugins/resonance-gate"` from the
/// workspace `members` → failed with "not a workspace member"; reverted.
#[test]
fn plugin_dirs_and_workspace_members_agree() {
    let root = workspace_root();
    let pkgs = packages();
    let members: BTreeMap<String, &Package> = pkgs
        .iter()
        .filter(|p| p.is_plugin(&root))
        .map(|p| (p.rel_dir(&root).display().to_string(), p))
        .collect();
    let dirs: BTreeSet<String> = fs::read_dir(root.join("plugins"))
        .expect("plugins/ exists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").is_file())
        .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    let mut violations = Vec::new();
    for dir in &dirs {
        match members.get(dir) {
            None => violations.push(format!("{dir}: not a workspace member (add it to `members` in Cargo.toml)")),
            Some(p) if !p.crate_types.contains("cdylib") => violations.push(format!(
                "{dir}: not a cdylib, so not a CLAP plugin — move it out of plugins/"
            )),
            Some(_) => {}
        }
    }
    for dir in members.keys() {
        if !dirs.contains(dir) {
            violations.push(format!("{dir}: workspace member that `plugins/*/` cannot see"));
        }
    }
    // A plugin nested deeper than plugins/<name>/ is a member `is_plugin`
    // does not classify; catch it by path prefix.
    for p in &pkgs {
        let rel = p.rel_dir(&root);
        if rel.starts_with("plugins") && !p.is_plugin(&root) {
            violations.push(format!("{}: nested below plugins/<name>/ — bundle.sh will not find it", rel.display()));
        }
    }
    report(
        "scripts/bundle.sh: plugins/<name>/ ⇔ workspace member, and every one a cdylib",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Test layout (CLAUDE.md → Tests; ARCHITECTURE.md → "Test Layout")
// ---------------------------------------------------------------------------

/// CLAUDE.md: "Don't add a new file to `resonance-app/tests/`. Add a
/// module to one of the group binaries (`mixer`, `timeline`, `compose`,
/// `control`, …) instead. Every extra target re-monomorphizes the whole
/// app plus iced, which is why 240 of them cost 193s to relink after a
/// one-line change and 11 cost 8s." This is the eleven.
///
/// Exercised 2026-09-26: created `resonance-app/tests/scratch.rs` → failed
/// with `scratch.rs: new top-level test file`; deleted.
#[test]
fn app_test_binaries_are_the_known_groups() {
    let root = workspace_root();
    let known: BTreeSet<&str> = [
        "compose",
        "control",
        "e2e_compose_via_control",
        "hermetic_construction",
        "io",
        "midi",
        "mixer",
        "performance",
        "plugins",
        "timeline",
        "vocal",
    ]
    .into_iter()
    .collect();
    let actual: BTreeSet<String> = fs::read_dir(root.join("resonance-app/tests"))
        .expect("resonance-app/tests exists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "rs"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    let mut violations: Vec<String> = actual
        .iter()
        .filter(|s| !known.contains(s.as_str()))
        .map(|s| format!("{s}.rs: new top-level test file — add a module to a group binary instead"))
        .collect();
    violations.extend(
        known
            .iter()
            .filter(|k| !actual.contains(**k))
            .map(|k| format!("{k}.rs: group binary is gone — update this list if that was deliberate")),
    );
    report(
        "CLAUDE.md → Tests: resonance-app/tests/ holds exactly the group binaries",
        &violations,
    );
}

/// The same rule for `resonance-audio/tests/` (code review ARCH-03): its
/// 129 one-file targets were grouped into seven by source area, plus the
/// four that own process-global state and so need a process of their own —
/// a `#[global_allocator]` (`sidechain_taps`, `retire_queue`; one per
/// binary), a lowered `RLIMIT_FSIZE` (`recording_write_failure`), and the
/// one-shot engine-disconnect latch (`engine_send_disconnected`). A new
/// test is a module in a group; a new standalone needs one of those
/// reasons, and an entry here saying which.
///
/// Exercised 2026-09-26: created `resonance-audio/tests/scratch.rs` →
/// failed with `scratch.rs: new top-level test file`; deleted.
#[test]
fn audio_test_binaries_are_the_known_groups() {
    let root = workspace_root();
    let known: BTreeSet<&str> = [
        // groups
        "bounce",
        "clap_host",
        "engine",
        "io",
        "midi_hw",
        "mixer",
        "types",
        // standalone: process-global state
        "engine_send_disconnected",
        "recording_write_failure",
        "retire_queue",
        "sidechain_taps",
    ]
    .into_iter()
    .collect();
    let actual: BTreeSet<String> = fs::read_dir(root.join("resonance-audio/tests"))
        .expect("resonance-audio/tests exists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "rs"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    let mut violations: Vec<String> = actual
        .iter()
        .filter(|s| !known.contains(s.as_str()))
        .map(|s| format!("{s}.rs: new top-level test file — add a module to a group binary instead"))
        .collect();
    violations.extend(
        known
            .iter()
            .filter(|k| !actual.contains(**k))
            .map(|k| format!("{k}.rs: test binary is gone — update this list if that was deliberate")),
    );
    report(
        "CLAUDE.md → Tests: resonance-audio/tests/ holds exactly the group binaries",
        &violations,
    );
}

/// ARCHITECTURE.md → Test Layout: "Tests live in `<crate>/tests/`, not in
/// `#[cfg(test)] mod tests` blocks inside source files", with the
/// documented `resonance-app` private-helper exception ("shrink this list
/// when you can, don't grow it"). The allow-list below is that
/// exception, verbatim; `compose/mod.rs` is the `mod tests;` declaration
/// that pulls `compose/tests.rs` in.
///
/// Exercised 2026-09-26: added `#[cfg(test)] mod tests {}` to
/// `resonance-dsp/src/lib.rs` → failed on that line; reverted.
#[test]
fn no_inline_test_modules_outside_the_documented_exception() {
    let root = workspace_root();
    let allowed: BTreeSet<&str> = [
        "resonance-app/src/recent.rs",
        "resonance-app/src/compose/invariants.rs",
        "resonance-app/src/compose/mod.rs",
    ]
    .into_iter()
    .collect();
    let mut violations = Vec::new();
    for p in packages() {
        let mut files = Vec::new();
        rust_files(&p.dir.join("src"), &mut files);
        for file in files {
            let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
            for (n, code) in code_lines(&file) {
                if code.trim_start().starts_with("#[cfg(test)]") && !allowed.contains(rel.as_str()) {
                    violations.push(format!("{rel}:{n}: inline test module — write `tests/<feature>.rs` instead"));
                }
            }
        }
    }
    report(
        "ARCHITECTURE.md → Test Layout: no `#[cfg(test)]` in src/ beyond the documented exception",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// App-internal boundaries (ARCHITECTURE.md → "Audio Engine Public API")
// ---------------------------------------------------------------------------

/// ARCHITECTURE.md: "The app reconstructs its own state from
/// `AudioEvent`s. ... Do not add direct getter methods on `AudioEngine`
/// that read engine state". The view layer is where that would be
/// tempting, and today it has zero `.engine.` reads (review, Architecture
/// intro): a view draws app state, never the engine.
///
/// Exercised 2026-09-26: added `let _ = r.engine.foo();` to
/// `resonance-app/src/view/mod.rs` → failed on that line; reverted.
#[test]
fn view_layer_never_reads_the_engine() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src/view"), &mut files);
    let mut violations = Vec::new();
    for file in files {
        for (n, code) in code_lines(&file) {
            if code.contains(".engine.") {
                violations.push(format!(
                    "{}:{n}: the view reads the engine — mirror the state into the app via an AudioEvent",
                    file.strip_prefix(&root).unwrap_or(&file).display()
                ));
            }
        }
    }
    report(
        "ARCHITECTURE.md → Audio Engine Public API: `view/` never touches `.engine.`",
        &violations,
    );
}

/// The control API's view model maps app state onto the wire types with
/// exhaustive matches, so that a new variant on either side is a compile
/// error rather than a silently unmapped value (review ARCH-10: "no
/// wildcard arms today — correct, but unguarded"). A `_ =>` arm in that
/// directory is the erosion this guards against.
///
/// Exercised 2026-09-26: added `_ => unreachable!(),` to a match in
/// `resonance-app/src/update/control/view_model/mod.rs` → failed on that
/// line; reverted.
#[test]
fn control_view_model_has_no_wildcard_arms() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src/update/control/view_model"), &mut files);
    assert!(!files.is_empty(), "view_model directory moved? update this test");
    let mut violations = Vec::new();
    for file in files {
        for (n, code) in code_lines(&file) {
            let t = code.trim_start();
            if t.starts_with("_ =>") || t.starts_with("| _ =>") {
                violations.push(format!(
                    "{}:{n}: wildcard arm in a wire mapping — enumerate the variants",
                    file.strip_prefix(&root).unwrap_or(&file).display()
                ));
            }
        }
    }
    report(
        "resonance-app/src/update/control/view_model/: wire mappings match exhaustively",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Logging (code review ARCH-05: one facade, nothing on the audio thread)
// ---------------------------------------------------------------------------

/// True if `code` names the path `prefix` (e.g. `log::`) as a whole path
/// segment — `log::warn!` yes, `catalog::find` no.
fn names_path(code: &str, prefix: &str) -> bool {
    code.match_indices(prefix).any(|(i, _)| {
        !code[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// A file only a binary, a build script or tests compile: `src/main.rs`,
/// `src/bin/**`, and any `test_support` module.
fn is_binary_or_test_support(rel: &str) -> bool {
    rel.ends_with("/src/main.rs")
        || rel.contains("/src/bin/")
        || rel.split('/').any(|seg| seg.starts_with("test_support"))
}

/// ARCH-05 A5-1: library code logs through `tracing`, so `RUST_LOG`
/// filters it and a binary chooses where it goes; a bare `eprintln!` /
/// `println!` bypasses both. Binaries (`main.rs`, `src/bin/`), build
/// scripts (not under `src/`) and `test_support` modules may print.
///
/// Allowed for now, each a later step: `resonance-app` (its own sweep,
/// after M12), `resonance-dsp-test-support` (a test-only crate: the bless
/// notice), and the plugin files below — plugin crates keep stderr until
/// their own sweep; none of these sites is reachable from `process()`.
///
/// Exercised 2026-09-26: added `eprintln!("x");` to
/// `resonance-common/src/scan.rs` → failed on that line; reverted.
#[test]
fn library_crates_log_through_tracing_not_stderr() {
    let root = workspace_root();
    let allowed_crates: BTreeSet<&str> = ["resonance-app", "resonance-dsp-test-support"]
        .into_iter()
        .collect();
    let allowed_files: BTreeSet<&str> = [
        "plugins/resonance-amp/src/lib.rs",
        "plugins/resonance-amp/src/loader.rs",
        "plugins/resonance-amp/src/nam/parse/weights.rs",
        "plugins/resonance-drums/src/articulation.rs",
        "plugins/resonance-drums/src/dsp/sampler.rs",
        "plugins/resonance-ir/src/loader.rs",
    ]
    .into_iter()
    .collect();
    let mut violations = Vec::new();
    for p in packages() {
        if allowed_crates.contains(p.name.as_str()) {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&p.dir.join("src"), &mut files);
        for file in files {
            let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
            if is_binary_or_test_support(&rel) || allowed_files.contains(rel.as_str()) {
                continue;
            }
            for (n, code) in code_lines(&file) {
                if code.contains("println!") {
                    violations.push(format!(
                        "{rel}:{n}: stderr/stdout print in library code — use tracing::{{error,warn,info,debug}}!"
                    ));
                }
            }
        }
    }
    report(
        "ARCH-05: library crates log through `tracing`, not `eprintln!`/`println!`",
        &violations,
    );
}

/// ARCH-05 A5-2: "Rule for the RT thread: no logging — increment an
/// atomic and let the tick handler log." Everything under
/// `resonance-audio/src/mixer/` runs on (or is only called from) the
/// audio callback, so no logging macro, facade or print may appear
/// there; latch the value into `SharedState` and log it from the engine
/// loop instead (`cycle_load::OversizeBufferLatch`, `CycleReportSlot`).
///
/// Exercised 2026-09-26: added `tracing::warn!("x");` to
/// `resonance-audio/src/mixer/callback/mod.rs` → failed on that line;
/// reverted.
#[test]
fn audio_callback_never_logs() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-audio/src/mixer"), &mut files);
    assert!(!files.is_empty(), "resonance-audio/src/mixer moved? update this test");
    let mut violations = Vec::new();
    for file in files {
        for (n, code) in code_lines(&file) {
            let logs = names_path(&code, "tracing::")
                || names_path(&code, "log::")
                || code.contains("println!")
                || code.contains("eprint!")
                || names_path(&code, "print!")
                || names_path(&code, "dbg!");
            if logs {
                violations.push(format!(
                    "{}:{n}: logging on the audio thread — store an atomic in SharedState and log it from the engine loop",
                    file.strip_prefix(&root).unwrap_or(&file).display()
                ));
            }
        }
    }
    report(
        "ARCH-05: nothing under resonance-audio/src/mixer/ logs or prints (audio thread)",
        &violations,
    );
}
