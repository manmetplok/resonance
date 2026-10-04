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
    /// Whether the manifest leaves this dependency's default features on
    /// (i.e. no `default-features = false`).
    uses_default_features: bool,
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
                        uses_default_features: d["uses_default_features"].as_bool().unwrap_or(true),
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
/// about code is not tripped by a sentence about the rule. String
/// literals are kept (some rules look for one) and a `//` inside one is
/// not a comment (code review ARCH2-07: cutting at the first `//` made
/// everything after a `"https://…"` invisible to every rule).
fn code_lines(path: &Path) -> Vec<(usize, String)> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut lexer = Lexer::Code;
    text.lines()
        .enumerate()
        .map(|(i, line)| (i + 1, strip_line_comment(line, &mut lexer)))
        .collect()
}

/// Where a line starts, as far as comment stripping cares: string
/// literals may span lines.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Lexer {
    Code,
    /// Inside a `"…"` literal (escapes apply).
    Str,
    /// Inside a raw `r#…#"…"#…#` literal with this many `#`s.
    RawStr(usize),
}

/// `line` up to its `//` comment, if any, honouring string and char
/// literals; `state` carries an open string literal to the next line.
fn strip_line_comment(line: &str, state: &mut Lexer) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match *state {
            Lexer::Str => match chars[i] {
                '\\' => i += 2,
                '"' => {
                    *state = Lexer::Code;
                    i += 1;
                }
                _ => i += 1,
            },
            Lexer::RawStr(hashes) => {
                let closes = chars[i] == '"'
                    && chars[i + 1..].iter().take(hashes).filter(|c| **c == '#').count() == hashes;
                if closes {
                    *state = Lexer::Code;
                    i += 1 + hashes;
                } else {
                    i += 1;
                }
            }
            Lexer::Code => {
                let c = chars[i];
                let prev_ident = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
                if c == '/' && chars.get(i + 1) == Some(&'/') {
                    return chars[..i].iter().collect();
                } else if c == '"' {
                    *state = Lexer::Str;
                    i += 1;
                } else if !prev_ident && (c == 'r' || (c == 'b' && chars.get(i + 1) == Some(&'r'))) {
                    // Raw string: `r"`, `r#"`, `br##"` …; else an identifier.
                    let start = if c == 'b' { i + 2 } else { i + 1 };
                    let hashes = chars[start..].iter().take_while(|c| **c == '#').count();
                    if chars.get(start + hashes) == Some(&'"') {
                        *state = Lexer::RawStr(hashes);
                        i = start + hashes + 1;
                    } else {
                        i += 1;
                    }
                } else if c == '\'' {
                    // A char literal (`'"'`, `'\''`, `'/'`) — or a lifetime.
                    if chars.get(i + 1) == Some(&'\\') {
                        let close = chars[i + 2..].iter().position(|c| *c == '\'');
                        i = close.map_or(i + 1, |p| i + 2 + p + 1);
                    } else if chars.get(i + 2) == Some(&'\'') {
                        i += 3;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
    line.to_owned()
}

/// The comment stripper itself: `//` inside a string is code, and every
/// workspace source file lexes back to plain code at its end (a file that
/// ends "inside a string" means a literal was misread, which would hide
/// the rest of that file from every rule).
#[test]
fn comment_stripper_honours_string_literals() {
    let mut s = Lexer::Code;
    assert_eq!(
        strip_line_comment(r#"let u = "https://x"; f(); // note"#, &mut s),
        r#"let u = "https://x"; f(); "#
    );
    assert_eq!(strip_line_comment(r##"let r = r#"a // b"#; g() // c"##, &mut s), r##"let r = r#"a // b"#; g() "##);
    assert_eq!(strip_line_comment(r#"let q = '"'; h() // d"#, &mut s), r#"let q = '"'; h() "#);
    assert_eq!(strip_line_comment(r#"fn f<'a>(x: &'a str) // e"#, &mut s), r#"fn f<'a>(x: &'a str) "#);
    assert_eq!(strip_line_comment(r#"let m = "one"#, &mut s), r#"let m = "one"#);
    assert_eq!(s, Lexer::Str, "a string left open carries to the next line");
    assert_eq!(strip_line_comment(r#"two // still string"; k() // f"#, &mut s), r#"two // still string"; k() "#);
    assert_eq!(s, Lexer::Code);

    let root = workspace_root();
    let mut unbalanced = Vec::new();
    for p in packages() {
        let mut files = Vec::new();
        rust_files(&p.dir, &mut files);
        for file in files {
            let text = fs::read_to_string(&file).unwrap_or_default();
            let mut state = Lexer::Code;
            for line in text.lines() {
                strip_line_comment(line, &mut state);
            }
            if state != Lexer::Code {
                unbalanced.push(file.strip_prefix(&root).unwrap_or(&file).display().to_string());
            }
        }
    }
    report("arch-invariants: the comment stripper misread a string literal in", &unbalanced);
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
/// `plugins_never_name_a_platform_runtime`. `resonance-mastering-assist`
/// is the mastering plugin's ("`resonance-mastering-assist` ──►
/// resonance-mastering plugin").
///
/// Those two, and music theory, are per-plugin edges, not fleet-wide
/// ones (code review ARCH2-07): [`PLUGIN_EXTRA_DEPS`] grants each to the
/// plugin the diagram draws it to. `resonance-dsp-test-support` is a test
/// harness, so any crate may have it only as a dev-dependency
/// (`test_support_crates_are_dev_dependencies_only`).
const PLUGIN_DEPS: &[&str] = &[
    "resonance-plugin",
    "resonance-common",
    "resonance-dsp",
    "resonance-metering",
    "plugin-gui-core",
    "resonance-dsp-test-support",
];

/// Edges of the diagram that reach one plugin only.
const PLUGIN_EXTRA_DEPS: &[(&str, &[&str])] = &[
    // "resonance-mastering-assist ──► resonance-mastering plugin"
    ("resonance-mastering", &["resonance-mastering-assist"]),
    // "resonance-music-theory ──► resonance-granular-delay plugin
    // (scale-quantized grain pitch)"
    ("resonance-granular-delay", &["resonance-music-theory"]),
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
        // "a library crate: depends on `resonance-metering` and
        // `resonance-dsp` only"
        "resonance-mastering-assist" => &["resonance-metering", "resonance-dsp"],
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
/// the one crate with no row, except that it "depends on no plugin crate"
/// (checked here too, any dependency kind).
///
/// Exercised 2026-09-26: added `resonance-app = { path = "../resonance-app" }`
/// to `resonance-audio/Cargo.toml` → failed with
/// `resonance-audio -> resonance-app (normal)`; reverted.
/// Exercised 2026-09-28: put `resonance-mastering = { path =
/// "../plugins/resonance-mastering", default-features = false }` back in
/// `resonance-app/Cargo.toml` → failed with `resonance-app ->
/// resonance-mastering (normal)`; reverted.
/// Exercised 2026-10-04 (ARCH2-07: per-plugin rows): added
/// `resonance-mastering-assist = { path = "../../resonance-mastering-assist" }`
/// to `plugins/resonance-compressor/Cargo.toml` → failed with
/// `resonance-compressor -> resonance-mastering-assist (normal)`; reverted.
#[test]
fn crate_dag_matches_architecture_md() {
    let root = workspace_root();
    let pkgs = packages();
    let internal: BTreeSet<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    let plugins: BTreeSet<&str> = pkgs
        .iter()
        .filter(|p| p.is_plugin(&root))
        .map(|p| p.name.as_str())
        .collect();
    let mut violations = Vec::new();
    for p in &pkgs {
        if p.name == "resonance-app" {
            for d in &p.deps {
                if plugins.contains(d.name.as_str()) {
                    violations.push(format!(
                        "resonance-app -> {} ({}): the app depends on no plugin crate; \
                         move what it needs into a library crate both can use",
                        d.name, d.kind
                    ));
                }
            }
            continue;
        }
        let allowed: Vec<&str> = if p.is_plugin(&root) {
            let extra = PLUGIN_EXTRA_DEPS
                .iter()
                .filter(|(plugin, _)| *plugin == p.name)
                .flat_map(|(_, deps)| deps.iter());
            PLUGIN_DEPS.iter().chain(extra).copied().collect()
        } else {
            match allowed_internal_deps(&p.name) {
                Some(a) => a.to_vec(),
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

/// ARCHITECTURE.md → Crate Layering: "Adding a crate means deciding its
/// layer here". The diagram had drifted (code review ARCH2-09: no
/// `resonance-control`, `resonance-mcp` or `resonance-dsp-test-support`)
/// while `allowed_internal_deps` carried their rows, so the prose and the
/// table disagreed silently. Every non-plugin workspace package must be
/// named in that section; plugins are drawn as a class ("every plugin"),
/// so they are exempt.
///
/// Exercised 2026-10-04: deleted the `resonance-dsp-test-support` line of
/// the diagram → failed with "resonance-dsp-test-support is not named";
/// restored.
#[test]
fn every_crate_is_named_in_the_layering_section() {
    let root = workspace_root();
    let doc = fs::read_to_string(root.join("ARCHITECTURE.md")).expect("ARCHITECTURE.md");
    let start = doc.find("## Crate Layering").expect("a `## Crate Layering` section");
    let section = &doc[start..];
    let section = &section[..section[3..].find("\n## ").map_or(section.len(), |i| i + 3)];
    let named = |name: &str| {
        section.match_indices(name).any(|(i, _)| {
            let before = section[..i].chars().next_back();
            let after = section[i + name.len()..].chars().next();
            let edge =
                |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_');
            edge(before) && edge(after)
        })
    };
    let violations: Vec<String> = packages()
        .iter()
        .filter(|p| !p.is_plugin(&root) && !named(&p.name))
        .map(|p| format!("{} is not named in ARCHITECTURE.md → Crate Layering", p.name))
        .collect();
    report(
        "ARCHITECTURE.md → Crate Layering names every workspace crate",
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
/// Exercised 2026-10-04 (ARCH2-07: prefix matching): added `iced_widget =
/// "0.14"` to `resonance-audio/Cargo.toml` → failed with
/// `resonance-audio -> iced_widget`; then `winit = "0.30"` to
/// `plugins/resonance-gate/Cargo.toml` → failed; both reverted.
#[test]
fn framework_crates_stay_where_the_diagram_puts_them() {
    let pkgs = packages();
    let internal: BTreeSet<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    // (dependency name prefix, crates that may name it directly). A
    // prefix covers the crate and its family (`iced` → `iced_widget`,
    // `iced_core`, …; `egui` → `egui_extras`, `egui-wgpu`); the LONGEST
    // matching prefix decides, so `egui_glow` has its own, narrower row.
    // An empty owner list bans the stack outright.
    let table: &[(&str, &[&str])] = &[
        ("iced", &["resonance-app"]),
        ("egui", &["plugin-gui-core", "wayland-plugin-gui", "cocoa-plugin-gui"]),
        ("egui_glow", &["wayland-plugin-gui", "cocoa-plugin-gui"]),
        ("glow", &["wayland-plugin-gui", "cocoa-plugin-gui"]),
        ("eframe", &[]),
        ("winit", &[]),
        ("wgpu", &[]),
        ("wayland", &["wayland-plugin-gui"]),
        ("smithay", &["wayland-plugin-gui"]),
        ("khronos", &["wayland-plugin-gui"]),
        ("objc2", &["cocoa-plugin-gui"]),
        ("dispatch2", &["cocoa-plugin-gui"]),
    ];
    let family = |dep: &str, prefix: &str| {
        dep == prefix
            || dep
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('_') || rest.starts_with('-'))
    };
    // CLAP is the plugin ABI: the SDK speaks it, the host speaks it, the
    // plugins' in-process host tests speak it. Nothing below them does.
    let no_clap: &[&str] = &[
        "resonance-dsp",
        "resonance-metering",
        "resonance-mastering-assist",
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
            // Workspace crates (`wayland-plugin-gui`) are the DAG test's.
            let row = table
                .iter()
                .filter(|(prefix, _)| family(&d.name, prefix))
                .max_by_key(|(prefix, _)| prefix.len());
            if let (Some((_, owners)), false) = (row, internal.contains(d.name.as_str())) {
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

/// Whether `code` (one file, comments stripped, joined) imports the raw
/// classic knob from the widget kit: `widgets::knob` named directly, or
/// `knob` as an item of a `widgets::{…}` group.
fn imports_or_calls_classic_knob(code: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut rest = code;
    while let Some(at) = rest.find("widgets::") {
        let after = &rest[at + "widgets::".len()..];
        if after.starts_with("knob") && !after["knob".len()..].starts_with(is_ident) {
            return true;
        }
        if let Some(group) = after.strip_prefix('{') {
            let end = group.find('}').unwrap_or(group.len());
            if group[..end]
                .split(|c: char| !is_ident(c))
                .any(|item| item == "knob")
            {
                return true;
            }
        }
        rest = after;
    }
    false
}

/// code-review-2026-10-02 PUX-11: "Converge on one `ThemedKnob`-based
/// binding in `editor_widgets` that has gestures, typed entry and skew.
/// Add an arch-invariant that no plugin calls `widgets::knob(`
/// directly." The classic range-mapped knob takes its range, default
/// and curve as arguments, so a plugin calling it restates `params.rs`
/// (audit finding F4) and gets none of what the binding adds: the
/// declared skew, the mandatory reset, typed entry and the one-edit-
/// per-gesture host announce (PUX-01). Plugins draw knobs through
/// `resonance_plugin::editor_widgets::{float_knob, param_knob}`.
///
/// Exercised 2026-10-03: added `let _ = plugin_gui_core::widgets::knob(`
/// … `);` to `plugins/resonance-gate/src/editor/widgets.rs` → failed on
/// that file; then `use plugin_gui_core::widgets::{chip_button, knob};`
/// alone → failed; both reverted.
#[test]
fn plugins_draw_knobs_only_through_the_param_binding() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for p in packages().iter().filter(|p| p.is_plugin(&root)) {
        let mut files = Vec::new();
        rust_files(&p.dir.join("src"), &mut files);
        for file in files {
            let code: String = code_lines(&file)
                .into_iter()
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n");
            if imports_or_calls_classic_knob(&code) {
                violations.push(format!(
                    "{}: uses `plugin_gui_core::widgets::knob` — bind the param through \
                     `resonance_plugin::editor_widgets::{{float_knob, param_knob}}`",
                    file.strip_prefix(&root).unwrap_or(&file).display()
                ));
            }
        }
    }
    report(
        "PUX-11: plugins draw knobs through the one param binding, never the raw classic knob",
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
    "drum_map",            // drums: the GM pad contract shared with the app
    "decode_wav_stereo",   // drums: sample decode
    "decode_wav_native",   // drums: sample decode that keeps mono takes mono
    "decode_wav_split",    // drums: a resident head + an on-disk tail (E14 streaming)
    "WavTail",             // drums: reads a streamed take's tail from its file
    "TailScratch",         // drums: the tail reader's reusable buffers
    "decode_wav_channels", // ir: impulse-response decode
    "factory_presets",     // resonance-plugin: the factory-preset codec
    "library_marks",       // favourites/tags/recents shared by every library kind
    "nam_library",         // amp: the NAM model index, header reader and slot table
    "drumkit_library",     // drums, resonance-plugin: the kit index, sidecars, import and slot table
    "reveal",              // show a file in the platform file manager
    "atomic_file",         // resonance-plugin: atomic replace + quarantine for preset files
    "preset_session",      // resonance-plugin: the preset-identity CLAP extension ABI
    "param_flags",         // resonance-plugin: the state-excluded-params CLAP extension ABI
    "kit_info",            // drums, resonance-plugin: the kit-pads CLAP extension ABI
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
            let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
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

/// ARCH-07 A7-3: `resonance-common`'s nine DAW-model modules (`take`,
/// `midi_map`, `device_definition`, `automation`, `freeze`,
/// `device_registry`, `group_identity`, `external_instrument`,
/// `track_group`) live behind its `model` feature (on by default). A plugin
/// that leaves default features on gets the model compiled in regardless of
/// what `plugins_reach_only_common_utilities` allows it to *name* — this is
/// the type-level half of that guard. Every crate in `PLUGINS_ON_COMMON`,
/// plus `resonance-plugin` itself (the SDK; it reaches common only for
/// `scan_directory`/`factory_presets`), must set `default-features = false`
/// on its `resonance-common` dependency.
///
/// Exercised 2026-09-26: dropped `default-features = false` from
/// `plugins/resonance-drums/Cargo.toml`'s `resonance-common` dependency ->
/// failed on that line; restored.
#[test]
fn plugins_disable_default_features_on_resonance_common() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for p in packages()
        .iter()
        .filter(|p| PLUGINS_ON_COMMON.contains(&p.name.as_str()) || p.name == "resonance-plugin")
    {
        for d in p.deps.iter().filter(|d| d.name == "resonance-common") {
            if d.uses_default_features {
                violations.push(format!(
                    "{}/Cargo.toml: resonance-common dependency must set `default-features = \
                     false` — otherwise the `model` feature (Take, MidiBinding, DeviceDefinition, \
                     ...) is compiled in regardless of the source-level allow-list",
                    p.rel_dir(&root).display()
                ));
            }
        }
    }
    report(
        "ARCH-07 A7-3: resonance-common's model is feature-gated; resonance-plugin and its \
         plugins disable its default features",
        &violations,
    );
}

// Caveat on A7-3 (DEP-05, code-review-2026-10-02): the test above checks
// each plugin's *manifest* in isolation, which is correct for `cargo build
// -p <plugin>`. `scripts/bundle.sh`, though, builds the whole bundle set in
// one `cargo build --release -p a -p b -p c ...` invocation, and Cargo
// unifies features across a single build graph — so if *any* crate in that
// invocation (or a dependency shared between two of them) pulls in
// `resonance-common`'s `model` feature, every other plugin in the same
// build gets it compiled in too, this invariant's per-manifest check
// notwithstanding. In practice LTO strips the unreachable code back out of
// each cdylib (verified: gate.clap has zero symphonia symbols despite the
// bundle enabling `decode`/`drumkit-zip` workspace-wide), so the trim holds
// at the *shipped binary* level even though it doesn't hold at the
// resolved-feature-set level the comment above might suggest. Don't take a
// clean run of `plugins_disable_default_features_on_resonance_common` as
// proof the bundle build compiled less — check with `cargo tree -e
// features` over the bundle's actual `-p` set, or `nm` on the release
// artifact, if that distinction matters.

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
/// five that own process-global state and so need a process of their own —
/// a `#[global_allocator]` (`sidechain_taps`, `retire_queue`,
/// `render_pool_rt`, which counts every thread's allocations; one per
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
        "render_pool_rt",
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
/// Exercised 2026-10-04 (ARCH2-07: any `cfg(…test…)`, inner attributes
/// too): added `#[cfg(all(test, unix))] mod t {}` to
/// `resonance-dsp/src/lib.rs` → failed; then `#![cfg(test)]` at the top
/// of `resonance-dsp/src/biquad.rs` → failed; both reverted.
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
            let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
            for (n, code) in code_lines(&file) {
                if is_test_cfg(&code) && !allowed.contains(rel.as_str()) {
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

/// Whether `code` is a `#[cfg(…)]` / `#![cfg(…)]` / `#[cfg_attr(…)]`
/// attribute that names the bare `test` cfg anywhere in its predicate —
/// `cfg(test)`, `cfg(all(test, unix))`, `cfg(any(test, feature = …))` —
/// but not a feature that merely contains the word (`feature =
/// "test-internals"`).
fn is_test_cfg(code: &str) -> bool {
    let t = code.trim_start();
    let Some(attr) = ["#[cfg(", "#![cfg(", "#[cfg_attr(", "#![cfg_attr("]
        .iter()
        .find_map(|p| t.strip_prefix(p))
    else {
        return false;
    };
    // Drop quoted feature names, then look for `test` as a whole token.
    let unquoted: String = attr.split('"').step_by(2).collect::<Vec<_>>().join(" ");
    unquoted
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|tok| tok == "test")
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

/// True for a match arm that is a bare wildcard: `_ =>` or `| _ =>`.
fn is_wildcard_arm(code: &str) -> bool {
    let t = code.trim_start();
    t.starts_with("_ =>") || t.starts_with("| _ =>")
}

/// ARCH-06 A6-4 (refactor-intent A-10): every sub-message enum classifies
/// itself for undo in an exhaustive `fn undo_action`, and `undo/classify.rs`
/// only delegates. A catch-all arm — `Message::X(_) => UndoAction::Skip`
/// in the classifier, or `_ =>` inside an `undo_action` — lets a new
/// variant land silently non-undoable (or silently recorded, clearing the
/// redo stack); without one, a new variant does not compile until someone
/// decides what undo does with it.
///
/// Exercised 2026-09-26: before A-10 it failed on 36 lines of
/// `classify.rs`; after, adding `_ => UndoAction::Skip,` to
/// `TakeMessage::undo_action` failed on that line; reverted.
#[test]
fn undo_classification_has_no_catch_all_arms() {
    let root = workspace_root();
    let classify = root.join("resonance-app/src/undo/classify.rs");
    let mut violations = Vec::new();
    for (n, code) in code_lines(&classify) {
        if code.contains("(_) => UndoAction::Skip")
            || code.contains("(_) => UndoAction::Record")
            || is_wildcard_arm(&code)
        {
            violations.push(format!(
                "resonance-app/src/undo/classify.rs:{n}: catch-all undo arm — delegate to the enum's `undo_action`"
            ));
        }
    }
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src"), &mut files);
    let mut impls = 0;
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        // Brace depth of the `fn undo_action` body we are inside, if any.
        let mut body: Option<i32> = None;
        for (n, code) in code_lines(&file) {
            if body.is_none() && code.contains("fn undo_action(") {
                impls += 1;
                body = Some(0);
            }
            let Some(depth) = body.as_mut() else {
                continue;
            };
            if is_wildcard_arm(&code) {
                violations.push(format!(
                    "{rel}:{n}: `_ =>` in an `undo_action` — enumerate the variants"
                ));
            }
            let opened = code.matches('{').count() as i32;
            let closed = code.matches('}').count() as i32;
            let was_open = *depth > 0 || opened > 0;
            *depth += opened - closed;
            if was_open && *depth <= 0 {
                body = None;
            }
        }
    }
    report(
        "ARCH-06 A6-4: undo classification is exhaustive per message enum (no catch-all arms)",
        &violations,
    );
    assert!(
        impls >= 30,
        "found only {impls} `fn undo_action` impls in resonance-app/src — did the pattern move? update this test"
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
/// Allowed for now, each a later step: `resonance-dsp-test-support` (a
/// test-only crate: the bless notice), and the plugin files below —
/// plugin crates keep stderr until their own sweep; none of these sites
/// is reachable from `process()`. `resonance-app` swept in FU-H6a.
///
/// Exercised 2026-09-26: added `eprintln!("x");` to
/// `resonance-common/src/scan.rs` → failed on that line; reverted.
#[test]
fn library_crates_log_through_tracing_not_stderr() {
    let root = workspace_root();
    let allowed_crates: BTreeSet<&str> = ["resonance-dsp-test-support"].into_iter().collect();
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
            let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
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
/// The same holds for `resonance-audio/src/render_pool/` (code review
/// ARCH2-07): its workers render track and bus jobs inside the callback's
/// deadline (realtime-multithreading.md), so they are audio threads too.
///
/// Exercised 2026-09-26: added `tracing::warn!("x");` to
/// `resonance-audio/src/mixer/callback/mod.rs` → failed on that line;
/// reverted. Exercised 2026-10-04: the same line in
/// `resonance-audio/src/render_pool/mod.rs` → failed; reverted.
#[test]
fn audio_callback_never_logs() {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in ["resonance-audio/src/mixer", "resonance-audio/src/render_pool"] {
        let before = files.len();
        rust_files(&root.join(dir), &mut files);
        assert!(files.len() > before, "{dir} moved? update this test");
    }
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
        "ARCH-05: nothing under resonance-audio/src/{mixer,render_pool}/ logs or prints (audio threads)",
        &violations,
    );
}

/// Epic B's done-when (code review ARCH-02; refactor-intent.md → Epic B):
/// the audio callback reads the project through the published render
/// graph (`engine/render_graph.rs`, one wait-free `ArcSwap` load), never
/// through a lock a control-thread writer can hold. So nothing under
/// `resonance-audio/src/engine/` or `src/mixer/` names an `RwLock` (which
/// also covers `HandlerCtx` and the offline `ChunkCtx`), nothing under
/// `mixer/` `try_read`s, and nothing under `engine/` takes a `.write()`
/// guard. Doc comments are cut off by `code_lines`, so the modules may
/// still tell the history. [`ENGINE_WRITE_ALLOWED`] lists the files that
/// may keep a `.write()` — empty since B-6: the plugin instance lock is a
/// `Mutex` in `PluginSlot`, not a write guard.
///
/// Exercised 2026-09-27: added `let _l = parking_lot::RwLock::new(0);
/// let _g = _l.write(); let _r = _l.try_read();` to
/// `resonance-audio/src/mixer/callback/play.rs` (→ `RwLock`, `try_read`)
/// and the first two to `resonance-audio/src/engine/tracks.rs` (→
/// `RwLock`, `.write()`) → failed on those four; reverted.
const ENGINE_WRITE_ALLOWED: &[&str] = &[];

#[test]
fn engine_and_mixer_take_no_state_lock() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for (dir, forbid_try_read, forbid_write) in [
        ("resonance-audio/src/engine", false, true),
        ("resonance-audio/src/mixer", true, false),
    ] {
        let mut files = Vec::new();
        rust_files(&root.join(dir), &mut files);
        assert!(!files.is_empty(), "{dir} moved? update this test");
        for file in files {
            let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
            for (n, code) in code_lines(&file) {
                if code.contains("RwLock") {
                    violations.push(format!(
                        "{rel}:{n}: `RwLock` — publish through the render graph (`engine/render_graph.rs`) instead"
                    ));
                }
                if forbid_try_read && code.contains("try_read") {
                    violations.push(format!(
                        "{rel}:{n}: `try_read` on the audio path — load the render graph instead"
                    ));
                }
                if forbid_write
                    && code.contains(".write()")
                    && !ENGINE_WRITE_ALLOWED.contains(&rel.as_str())
                {
                    violations.push(format!(
                        "{rel}:{n}: `.write()` guard in the engine — edit through `SharedState::edit_*` instead"
                    ));
                }
            }
        }
    }
    report(
        "ARCH-02 / Epic B: no RwLock in engine/ or mixer/, no try_read in mixer/, no .write() in engine/",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Error taxonomy (ARCH-05 epic C, C-5): no new `Result<_, String>` in a
// `pub fn` of resonance-audio or resonance-common.
// ---------------------------------------------------------------------------

/// Whether `sig` — a function signature's source text, comments stripped
/// and lines joined with spaces, from `pub fn` through (not including)
/// its opening `{` or terminating `;` — declares a return type of
/// `Result<_, String>`: some `Result<...>` whose *last* top-level generic
/// argument (i.e. the Err type; nested `<...>`/`(...)`/`[...]` don't
/// count as top-level) is exactly `String`. Matches
/// `Result<(), String>`, `Result<(Vec<f32>, String), String>` (Ok type
/// containing `String` doesn't fool it) and multi-line signatures
/// (rustfmt puts `-> Result<...> {` on its own line, but this doesn't
/// depend on that); doesn't match `Result<String, MyError>` or a
/// `Result<_, String>` that isn't the function's own return type (e.g.
/// a closure parameter's) since it's called on the field/arg text
/// surrounding it too — deliberately over-eager on that boundary, since
/// missing a real violation is worse than an occasional false positive.
fn returns_result_string(sig: &str) -> bool {
    let mut idx = 0;
    while let Some(rel) = sig[idx..].find("Result<") {
        let start = idx + rel + "Result<".len();
        let mut depth = 1i32;
        let mut end = None;
        for (i, c) in sig[start..].char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        let inner = &sig[start..end];
        let mut depth2 = 0i32;
        let mut last_comma = None;
        for (i, c) in inner.char_indices() {
            match c {
                '<' | '(' | '[' => depth2 += 1,
                '>' | ')' | ']' => depth2 -= 1,
                ',' if depth2 == 0 => last_comma = Some(i),
                _ => {}
            }
        }
        if let Some(ci) = last_comma {
            if inner[ci + 1..].trim() == "String" {
                return true;
            }
        }
        idx = end;
    }
    false
}

/// The identifier a `pub fn ...` line declares, e.g. `"decode_file"` for
/// `pub fn decode_file(path: &str, ...`. `None` if `code`'s trimmed
/// start isn't `pub fn ` (a `pub(crate) fn` doesn't count — the C-5 rule
/// is about `pub fn` signatures only).
fn pub_fn_name(code: &str) -> Option<&str> {
    let rest = code.trim_start().strip_prefix("pub fn ")?;
    let end = rest.find(['(', '<', ' ']).unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

/// Every `pub fn` in `file` whose signature returns `Result<_, String>`,
/// as `(line, fn_name)`. Walks from each `pub fn` line to its opening
/// `{` (or a `;` for a trait-style declaration with no body), joining
/// the lines in between — a signature with a multi-line parameter list
/// still has its `-> Result<...>` recognised. Bounded to 60 lines past
/// the `pub fn` so a parse hiccup can't scan the rest of the file.
fn pub_fn_result_string_violations(file: &std::path::Path) -> Vec<(usize, String)> {
    let lines = code_lines(file);
    let mut violations = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (line_no, ref first) = lines[i];
        if let Some(name) = pub_fn_name(first) {
            let mut sig = String::new();
            let mut j = i;
            while j < lines.len() && j < i + 60 {
                let (_, ref code) = lines[j];
                sig.push_str(code);
                sig.push(' ');
                if code.contains('{') || code.trim_end().ends_with(';') {
                    break;
                }
                j += 1;
            }
            if returns_result_string(&sig) {
                violations.push((line_no, name.to_string()));
            }
            i = j;
        }
        i += 1;
    }
    violations
}

/// C-4 landed with zero `Result<_, String>` left in resonance-common
/// (`arch-migration-plan.md` → "C-4 (7/7)"), so this side of the
/// allow-list starts empty — any future `pub fn Result<_, String>` here
/// is new and should be rejected, not grown into.
const RESONANCE_COMMON_ALLOW: &[(&str, &str)] = &[];

/// C-3 (parts 1 + 2 / "C-3b") converted every `pub`/`pub(crate)`
/// `Result<_, String>` fn in resonance-audio except
/// `recording.rs::take_clip_source`, which stayed `Result<_, String>`
/// deliberately (`arch-migration-plan.md` → "C-3 part 1 landed") — but
/// that one is a private `fn`, not `pub fn`, so it's already outside
/// this check's scope and needs no entry here.
const RESONANCE_AUDIO_ALLOW: &[(&str, &str)] = &[];

/// ARCH-05 epic C, C-5: "resonance-audio/common public fns don't return
/// `Result<_, String>`" (`refactor-intent.md` → "Epic C — Engine error
/// taxonomy", "Done when"). `resonance-amp`'s NAM loader is exempt by
/// construction — this only walks the two crates named in the rule, not
/// `plugins/`.
///
/// The allow-lists exist for exactly the transition where one crate
/// converts before the other; both are empty as of C-3b/C-4 landing
/// together, and should stay empty — a new entry here is the rule
/// silently regressing, not a place to grow.
///
/// Exercised 2026-09-26: added `pub fn x() -> Result<(), String> { Ok(()) }`
/// to `resonance-audio/src/lib.rs` → failed on that line; reverted.
#[test]
fn engine_common_public_fns_dont_return_result_string() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for (crate_dir, allow) in [
        ("resonance-audio/src", RESONANCE_AUDIO_ALLOW),
        ("resonance-common/src", RESONANCE_COMMON_ALLOW),
    ] {
        let mut files = Vec::new();
        rust_files(&root.join(crate_dir), &mut files);
        for file in files {
            let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
            for (line, name) in pub_fn_result_string_violations(&file) {
                if allow.iter().any(|(f, n)| rel.ends_with(f) && *n == name) {
                    continue;
                }
                violations.push(format!(
                    "{rel}:{line}: pub fn `{name}` returns Result<_, String> — use a thiserror type per module (see C-3/C-4) and add EngineError::from"
                ));
            }
        }
    }
    report(
        "ARCH-05 epic C (C-5): resonance-audio/common pub fns don't return Result<_, String>",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// View-layer visual polish (code review 2026-10-02, batch U3)
// ---------------------------------------------------------------------------

/// ux-guidelines.md → Typography: the "never below 11px" promise was
/// aspirational — 9 and 10px are the real floor in dense chrome (mixer
/// chips, badges, meter labels), and the doc now says so. What's still
/// banned is `.size(8)`: it measured below the 4.5:1-at-small-text
/// readability line for every informational label that used it (code
/// review UX-07). The two exceptions below are glyphs, not text — a
/// filled-circle status dot and a caret icon — allow-listed by their
/// exact line so a future *text* `.size(8)` in either file still trips.
///
/// Exercised 2026-10-02: added `.size(8)` to a `text("3")` in
/// `view/transport.rs` → failed on that line; reverted.
#[test]
fn view_text_never_shrinks_below_the_9px_floor() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src/view"), &mut files);
    let allowed: &[(&str, &str)] = &[
        ("view/remote_indicator.rs", ".size(8)"),
        (
            "view/compose/drumroll/pattern_picker.rs",
            "theme::icon(theme::fa::CARET_RIGHT).size(8)",
        ),
    ];
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        for (n, code) in code_lines(&file) {
            if !code.contains(".size(8)") {
                continue;
            }
            let is_allowed = allowed
                .iter()
                .any(|(f, fragment)| rel.ends_with(f) && code.contains(fragment));
            if !is_allowed {
                violations.push(format!(
                    "{rel}:{n}: text below the 9px floor (ux-guidelines.md → Typography) — raise it, or if this is a glyph/dot rather than legible text, add it to the allow-list"
                ));
            }
        }
    }
    report(
        "ux-guidelines.md → Typography: no view/ text below the 9px floor outside the glyph allow-list",
        &violations,
    );
}

/// ux-guidelines.md → Color Rules: "Never use pure white ... Never use
/// pure black ... for backgrounds" (code review UX-19). Every prior
/// `Color::WHITE` / `Color::BLACK` in `view/` was migrated to a theme
/// token (`TEXT_1`, `ON_ACCENT_TEXT`, ...); this keeps the count at zero.
///
/// Exercised 2026-10-02: added `.color(iced::Color::WHITE)` to
/// `view/transport.rs` → failed on that line; reverted.
#[test]
fn view_layer_never_hardcodes_pure_white_or_black() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src/view"), &mut files);
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        for (n, code) in code_lines(&file) {
            if code.contains("Color::WHITE") || code.contains("Color::BLACK") {
                violations.push(format!(
                    "{rel}:{n}: hardcoded pure white/black — add or use a `theme.rs` token instead"
                ));
            }
        }
    }
    report(
        "ux-guidelines.md → Color Rules: no Color::WHITE / Color::BLACK in view/ — use a theme token",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Rules added by code review ARCH2-07
// ---------------------------------------------------------------------------

/// Test-harness crates no production build may link: they exist for
/// `tests/` and `benches/` (`resonance-dsp-test-support`: the DSP golden
/// / bless helpers).
const TEST_SUPPORT_CRATES: &[&str] = &["resonance-dsp-test-support"];

/// ARCH2-07: "`resonance-dsp-test-support` is allowed as a normal
/// dependency" — the plugin allow-list named it, so any plugin could ship
/// it. It may only ever be a dev-dependency, of any crate.
///
/// Exercised 2026-10-04: moved `resonance-dsp-test-support` from
/// `[dev-dependencies]` to `[dependencies]` in
/// `plugins/resonance-gate/Cargo.toml` → failed with `resonance-gate ->
/// resonance-dsp-test-support (normal)`; reverted.
#[test]
fn test_support_crates_are_dev_dependencies_only() {
    let mut violations = Vec::new();
    for p in packages() {
        for d in p.deps.iter().filter(|d| TEST_SUPPORT_CRATES.contains(&d.name.as_str())) {
            if d.kind != "dev" {
                violations.push(format!("{} -> {} ({})", p.name, d.name, d.kind));
            }
        }
    }
    report(
        "ARCH2-07: test-support crates are dev-dependencies only",
        &violations,
    );
}

/// ARCHITECTURE.md / `update/control/mod.rs` (`run_via_update`): "This is
/// the only way a mutating control method may touch state" — a control
/// handler synthesizes a domain `Message` and routes it through
/// `update()`, which runs the gates, the undo recorder and the
/// transaction. Sending an `AudioCommand` straight from a handler skips
/// all three (and the mirror the handler's echo would update). Zero sites
/// when the rule was written.
///
/// Exercised 2026-10-04: added `let _ = app.engine.send(
/// resonance_audio::types::AudioCommand::Stop);` to a handler in
/// `resonance-app/src/update/control/transport.rs` → failed on that line;
/// reverted.
#[test]
fn control_handlers_never_send_engine_commands() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src/update/control"), &mut files);
    assert!(!files.is_empty(), "update/control moved? update this test");
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        for (n, code) in code_lines(&file) {
            if code.contains("engine.send(") {
                violations.push(format!(
                    "{rel}:{n}: a control handler sends an engine command — synthesize the \
                     domain Message and `run_via_update` it"
                ));
            }
        }
    }
    report(
        "update/control: handlers reach the engine only through update() (run_via_update)",
        &violations,
    );
}

/// Top-level test files (one binary each) per crate outside the app and
/// the engine, which have their own exact lists above. A ratchet, not a
/// ban (code review ARCH2-07: ~410 such files when it was written; moving
/// them is not worth the churn): the count must equal the cap, so a new
/// file fails until someone either makes it a module of an existing
/// binary (preferred — every binary relinks the crate) or raises the cap
/// deliberately, and merging files must lower it.
const TEST_BINARY_CAPS: &[(&str, usize)] = &[
    ("cocoa-plugin-gui", 3),
    ("plugin-gui-core", 4),
    ("plugins/resonance-amp", 29),
    ("plugins/resonance-color", 14),
    ("plugins/resonance-compressor", 14),
    ("plugins/resonance-delay", 15),
    ("plugins/resonance-drums", 43),
    ("plugins/resonance-eq", 14),
    ("plugins/resonance-gate", 14),
    ("plugins/resonance-granular-delay", 24),
    ("plugins/resonance-ir", 11),
    ("plugins/resonance-mastering", 35),
    ("plugins/resonance-reverb", 10),
    ("plugins/resonance-stereo", 6),
    ("plugins/resonance-wavetable", 28),
    ("resonance-common", 25), // +1 ARCH2-04: the content_index conformance suite
    ("resonance-control", 6),
    ("resonance-dsp", 33),
    ("resonance-dsp-test-support", 1),
    ("resonance-mastering-assist", 2),
    ("resonance-mcp", 7),
    ("resonance-metering", 25),
    ("resonance-music-theory", 42),
    ("resonance-plugin", 33),
    ("resonance-svs", 5),
    ("tools/arch-invariants", 1),
    ("wayland-plugin-gui", 5),
];

/// The CLAUDE.md test-binary rule, extended to every crate as a ratchet
/// (see [`TEST_BINARY_CAPS`]).
///
/// Exercised 2026-10-04: created `resonance-dsp/tests/scratch.rs` →
/// failed with "34 top-level test files, cap 33"; deleted.
#[test]
fn test_binary_counts_only_grow_deliberately() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for p in packages() {
        let rel = p.rel_dir(&root).display().to_string();
        if rel == "resonance-app" || rel == "resonance-audio" {
            continue;
        }
        let count = fs::read_dir(p.dir.join("tests"))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|x| x == "rs"))
                    .count()
            })
            .unwrap_or(0);
        match TEST_BINARY_CAPS.iter().find(|(dir, _)| *dir == rel) {
            None if count > 0 => violations.push(format!(
                "{rel}: {count} top-level test files and no row in `TEST_BINARY_CAPS`"
            )),
            None => {}
            Some((_, cap)) if count > *cap => violations.push(format!(
                "{rel}: {count} top-level test files, cap {cap} — add a module to an existing \
                 test binary instead (or raise the cap deliberately)"
            )),
            Some((_, cap)) if count < *cap => violations.push(format!(
                "{rel}: {count} top-level test files, cap {cap} — lower the cap to {count}"
            )),
            Some(_) => {}
        }
    }
    report(
        "ARCH2-07: per-crate test-binary counts are a ratchet (TEST_BINARY_CAPS)",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// First-party plugin identity has one spelling (code review ARCH2-03)
// ---------------------------------------------------------------------------

/// ARCH2-03: "The app depends on no plugin crate" was enforced, but the
/// app depended on plugin internals by string literal — rename a CLAP id
/// and every template track became a missing-plugin slot. Each
/// first-party CLAP id (`com.resonance.<dir>` for every
/// `plugins/resonance-<dir>/`) is spelled once, in
/// `resonance-plugin/src/first_party.rs`; the plugins declare `CLAP_ID`
/// from it and the app imports it. A quoted literal of one anywhere else
/// in the app's, the SDK's or a plugin's sources fails. (`resonance-
/// control` spells `MASTERING_PLUGIN_ID` itself — it links nothing — and
/// the app's `builtin_templates` tests pin it to the plugin's.)
///
/// Exercised 2026-10-04: changed `const CLAP_ID` in
/// `plugins/resonance-gate/src/lib.rs` back to `"com.resonance.gate"` →
/// failed on that line; reverted.
#[test]
fn first_party_clap_ids_are_spelled_once() {
    let root = workspace_root();
    let literals: Vec<String> = packages()
        .iter()
        .filter(|p| p.is_plugin(&root))
        .filter_map(|p| p.name.strip_prefix("resonance-").map(|n| format!("\"com.resonance.{n}\"")))
        .collect();
    assert!(literals.len() >= 13, "plugin dirs moved? found {literals:?}");
    let home = "resonance-plugin/src/first_party.rs";
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src"), &mut files);
    rust_files(&root.join("resonance-plugin/src"), &mut files);
    for p in packages().iter().filter(|p| p.is_plugin(&root)) {
        rust_files(&p.dir.join("src"), &mut files);
    }
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        if rel == home {
            continue;
        }
        for (n, code) in code_lines(&file) {
            if let Some(lit) = literals.iter().find(|l| code.contains(l.as_str())) {
                violations.push(format!(
                    "{rel}:{n}: spells {lit} — use the `resonance_plugin::first_party` constant"
                ));
            }
        }
    }
    report(
        "ARCH2-03: first-party CLAP ids are spelled only in resonance_plugin::first_party",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// The wire protocol stays in the control layer (code review ARCH2-11)
// ---------------------------------------------------------------------------

/// Where `resonance-app` may name `resonance_control`: the control layer
/// itself (prefixes) and the listed files, each for a stated reason.
const CONTROL_LAYER_PREFIXES: &[&str] = &[
    "resonance-app/src/update/control/",
    "resonance-app/src/control_socket",
    "resonance-app/src/control_jobs",
    "resonance-app/src/test_support/",
    "resonance-app/src/state/control.rs",
];
const CONTROL_WIRE_ALLOWED_FILES: &[&str] = &[
    // `PendingPluginPresetSave` (only ever armed by `presets.save` over
    // the control API) carries the request's `PresetMetaInput` until the
    // engine's state echo lands.
    "resonance-app/src/lib.rs",
    // Resolves a control `clip.place` import job by its method name.
    "resonance-app/src/engine_events/pool.rs",
];

/// ARCH2-11: "Wire-protocol types are used as app domain types" — preset
/// sources in state/messages and `MAX_BARS` as the section limit made a
/// protocol-only change ripple into app state, messages and undo. App
/// state, messages, views and the domain handlers use app-owned (or
/// `resonance_plugin::presets`) types; the control layer maps them onto
/// the wire (`update/control/view_model`, `plugin_presets::wire_source`).
///
/// Exercised 2026-10-04: put `pub source:
/// resonance_control::methods::plugin_preset::PluginPresetSource,` back
/// in `state/presets.rs`'s `SlotPresetIdentity` (text only) → failed on
/// that line; reverted.
#[test]
fn wire_protocol_types_stay_in_the_control_layer() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src"), &mut files);
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        if CONTROL_LAYER_PREFIXES.iter().any(|p| rel.starts_with(p))
            || CONTROL_WIRE_ALLOWED_FILES.contains(&rel.as_str())
        {
            continue;
        }
        for (n, code) in code_lines(&file) {
            if names_path(&code, "resonance_control") {
                violations.push(format!(
                    "{rel}:{n}: names `resonance_control` outside the control layer — use an \
                     app-owned type here and map to the wire in `update/control/`"
                ));
            }
        }
    }
    report(
        "ARCH2-11: `resonance_control` (the wire protocol) is named only in the control layer",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// Engine features with an app caller (code review ARCH2-01)
// ---------------------------------------------------------------------------

/// The variant names of `pub enum <name>` in `file`: the identifiers that
/// open a line at one indent level inside the enum's braces.
fn enum_variants(file: &Path, name: &str) -> Vec<String> {
    let lines = code_lines(file);
    let open = format!("pub enum {name} {{");
    let start = lines
        .iter()
        .position(|(_, l)| l.trim() == open)
        .unwrap_or_else(|| panic!("`{open}` not found in {}", file.display()));
    let mut out = Vec::new();
    for (_, line) in &lines[start + 1..] {
        if line.starts_with('}') {
            break;
        }
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        if !rest.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        out.push(ident);
    }
    out
}

/// `AudioCommand` variants nothing in the app sends yet — engine features
/// that exist and are tested on the engine side but have no UI or control
/// surface (code review ARCH2-01). Each line is a decision someone has to
/// make (wire it, or delete it from the engine), never a place to park a
/// new command: a new variant must have an app caller when it lands.
const ORPHANED_AUDIO_COMMANDS: &[&str] = &[];

/// `AudioCommand` variants the engine sends to itself, by design never
/// from the app: worker threads post them on the retry channel, and
/// `ShutDown` is sent by `AudioEngine::shutdown` / `Drop`.
const ENGINE_INTERNAL_AUDIO_COMMANDS: &[&str] =
    &["BounceTargetCancelled", "ReferenceAnalyzed", "ShutDown"];

/// ARCHITECTURE.md → Audio Engine Public API: an engine feature is an
/// `AudioCommand` the app sends. ARCH2-01 found ~13 commands (and the
/// stem export's whole event queue) with no caller, maintained and tested
/// for nobody. Every variant must be named as `AudioCommand::<Variant>`
/// somewhere in `resonance-app/src` outside `test_support` and `demo`
/// (test-only since ARCH2-08; they arm things production never does), or sit in `ORPHANED_AUDIO_COMMANDS` —
/// and an entry that gains a caller must leave the list.
///
/// Exercised 2026-10-04: added a variant `ArchProbe,` to `AudioCommand`
/// → failed with "ArchProbe: no app caller"; removed `"QueryIoLatency"`
/// from the allow-list → failed likewise; both reverted.
#[test]
fn every_audio_command_has_an_app_caller() {
    let root = workspace_root();
    let variants = enum_variants(
        &root.join("resonance-audio/src/types/commands.rs"),
        "AudioCommand",
    );
    assert!(
        variants.len() > 100,
        "found only {} AudioCommand variants — parser broken?",
        variants.len()
    );
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src"), &mut files);
    let mut code = String::new();
    for file in &files {
        let rel = file.strip_prefix(&root).unwrap_or(file).display().to_string();
        // `test_support` and `demo` compile only with the `test-support`
        // feature (ARCH2-08): a caller there is a test, not the app.
        if rel.split('/').any(|seg| seg.starts_with("test_support"))
            || rel == "resonance-app/src/demo.rs"
        {
            continue;
        }
        for (_, line) in code_lines(file) {
            code.push_str(&line);
            code.push('\n');
        }
    }
    let named = |v: &str| {
        let needle = format!("AudioCommand::{v}");
        code.match_indices(&needle).any(|(i, _)| {
            !code[i + needle.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    };
    let mut violations = Vec::new();
    for v in &variants {
        if ENGINE_INTERNAL_AUDIO_COMMANDS.contains(&v.as_str()) {
            continue;
        }
        let orphan = ORPHANED_AUDIO_COMMANDS.contains(&v.as_str());
        match (named(v), orphan) {
            (false, false) => violations.push(format!(
                "AudioCommand::{v}: no app caller in resonance-app/src (outside test_support) — \
                 wire it, or delete it from the engine"
            )),
            (true, true) => violations.push(format!(
                "AudioCommand::{v} has an app caller now — drop it from `ORPHANED_AUDIO_COMMANDS`"
            )),
            _ => {}
        }
    }
    for o in ORPHANED_AUDIO_COMMANDS.iter().chain(ENGINE_INTERNAL_AUDIO_COMMANDS) {
        if !variants.iter().any(|v| v == o) {
            violations.push(format!(
                "{o}: in `ORPHANED_AUDIO_COMMANDS` but no longer an AudioCommand"
            ));
        }
    }
    report(
        "ARCH2-01: every AudioCommand variant has an app caller (or a listed, deliberate orphan)",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// One plugin chain API for every owner (code review ARCH2-02)
// ---------------------------------------------------------------------------

/// ARCH2-02: the track/bus/master insert chains share one command and one
/// event per edit, parameterised by `ChainOwner` — `AddPlugin { owner, .. }`,
/// not `AddPlugin` / `AddPluginToBus` / `AddPluginToMaster`. Nine
/// command/event pairs meant every chain behaviour was written and fixed
/// three times (and fixed on two surfaces, missed on the third: FU-A13c/h).
/// A variant name that spells out a bus or master twin is that triplication
/// coming back.
///
/// Exercised 2026-10-04: added `AddPluginToBus { bus_id: BusId },` to
/// `AudioCommand` (text only) → failed naming it; reverted.
#[test]
fn plugin_chain_commands_and_events_take_a_chain_owner() {
    let root = workspace_root();
    let twins = ["ToBus", "InBus", "FromBus", "ToMaster", "InMaster", "FromMaster"];
    let mut violations = Vec::new();
    for (file, name) in [
        ("resonance-audio/src/types/commands.rs", "AudioCommand"),
        ("resonance-audio/src/types/events.rs", "AudioEvent"),
    ] {
        let variants = enum_variants(&root.join(file), name);
        assert!(
            variants.len() > 50,
            "found only {} {name} variants — parser broken?",
            variants.len()
        );
        for v in variants {
            if let Some(twin) = twins.iter().find(|t| v.contains(*t)) {
                violations.push(format!(
                    "{name}::{v}: a per-owner chain variant (`{twin}`) — carry a `ChainOwner` on \
                     the shared variant instead"
                ));
            }
        }
    }
    report(
        "ARCH2-02: plugin chain commands and events are owner-parameterised, never per-owner twins",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// The state module is the bottom of the app (code review ARCH2-05)
// ---------------------------------------------------------------------------

/// ARCH2-05: inside `resonance-app` the direction is update → state ← view,
/// with the socket on top of all three. `state/` held types defined in
/// `view::` (`UiViewCaches`, `TransportLabels`, `arrange_layout`), `update::`
/// (`ShiftOutcome`, `TypingProbe`, `KeymapEditorState`, `reconcile::Origin`,
/// the library caches) and `control_socket::` (`ConnId`, `ControlServer`,
/// `ReplySender`), so it could not be built or tested without the layers
/// above it. Now state owns every type it holds (the socket handle behind
/// `state::control::ControlListener`), and no code line under `state/`
/// names those three modules. Doc comments may still link to them.
///
/// Exercised 2026-10-04: added `use crate::view::settings::view;` to
/// `state/ui_transient.rs` (text only) → failed on that line; reverted.
#[test]
fn state_module_imports_nothing_from_view_update_or_the_socket() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("resonance-app/src/state"), &mut files);
    files.push(root.join("resonance-app/src/state.rs"));
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        for (n, code) in code_lines(&file) {
            for module in ["crate::view", "crate::update", "crate::control_socket"] {
                let prefixed = format!("{module}::");
                let bare = format!("{module};");
                if names_path(&code, &prefixed) || names_path(&code, &bare) {
                    violations.push(format!(
                        "{rel}:{n}: names `{module}` — state owns the types it holds; move the \
                         type into `state/` and re-export it from its old home"
                    ));
                }
            }
        }
    }
    report(
        "ARCH2-05: nothing under resonance-app/src/state names crate::view, crate::update or \
         crate::control_socket",
        &violations,
    );
}

// ---------------------------------------------------------------------------
// `mod.rs` size ratchet (code review ARCH2-12)
// ---------------------------------------------------------------------------

/// Every `src/**/mod.rs` over ARCHITECTURE.md's ~200-line guideline, with
/// the line count it had when this list was written (2026-10-04, after the
/// ARCH2-12 splits). A cap may only come down: lower it when you shrink
/// the file, drop the entry when the file reaches 200 lines, and never add
/// one for a new file — a new `mod.rs` that outgrows 200 lines wants to be
/// a directory of real modules (ARCHITECTURE.md → Anti-Patterns).
const MOD_RS_ALLOWANCE: &[(&str, usize)] = &[
    ("resonance-app/src/view/timeline/mod.rs", 1442),
    ("plugins/resonance-drums/src/kit_loader/mod.rs", 1087),
    ("plugins/resonance-drums/src/stream/mod.rs", 932),
    ("resonance-audio/src/render_pool/mod.rs", 926),
    ("resonance-app/src/view/performance/mod.rs", 903),
    ("resonance-app/src/update/project_io/mod.rs", 891),
    ("resonance-app/src/update/compose/mod.rs", 877),
    ("resonance-music-theory/src/derive/vocal/style/mod.rs", 854),
    ("resonance-audio/src/engine/thread/mod.rs", 846),
    ("plugins/resonance-color/src/dsp/mod.rs", 811),
    ("resonance-audio/src/clap_host/mod.rs", 766),
    ("plugins/resonance-drums/src/editor/mod.rs", 691),
    ("plugins/resonance-mastering/src/stages/multiband/mod.rs", 683),
    ("resonance-app/src/update/control/mod.rs", 673),
    ("resonance-app/src/commands/mod.rs", 643),
    ("resonance-audio/src/engine/bounce/mod.rs", 602),
    ("resonance-app/src/view/track_header/mod.rs", 562),
    ("resonance-app/src/view/compose/expanded_editor/mod.rs", 537),
    ("resonance-app/src/view/mixer/inspector/mod.rs", 488),
    ("plugins/resonance-granular-delay/src/dsp/mod.rs", 479),
    ("resonance-plugin/src/presets/mod.rs", 459),
    ("plugins/resonance-granular-delay/src/editor/controls/mod.rs", 410),
    ("plugins/resonance-wavetable/src/dsp/render/mod.rs", 398),
    ("resonance-plugin/src/clap_bridge/mod.rs", 383),
    ("resonance-app/src/view/compose/vocal_lane/mod.rs", 375),
    ("resonance-app/src/view/mod.rs", 369),
    ("plugins/resonance-wavetable/src/user_wavetable/mod.rs", 358),
    ("plugins/resonance-wavetable/src/params/mod.rs", 356),
    ("resonance-dsp/src/timestretch/mod.rs", 354),
    ("resonance-app/src/view/midi_editor/mod.rs", 332),
    ("resonance-app/src/undo/mod.rs", 328),
    ("resonance-app/src/update/compose/lane_inspector/mod.rs", 322),
    ("resonance-app/src/view/compose/lane_inspector/instrument/mod.rs", 322),
    ("resonance-music-theory/src/derive/vocal/mod.rs", 310),
    ("resonance-music-theory/src/generator/markov/mod.rs", 306),
    ("resonance-app/src/view/compose/lane_inspector/mod.rs", 302),
    ("resonance-app/src/view/compose/tracks/mod.rs", 280),
    ("plugins/resonance-mastering/src/stages/linear_phase_eq/mod.rs", 267),
    ("resonance-app/src/view/compose/vocal_roll/mod.rs", 266),
    ("resonance-audio/src/engine/thread/dispatch/mod.rs", 263),
    ("plugins/resonance-mastering/src/editor/mod.rs", 256),
    ("resonance-app/src/view/compose/chord_lane/mod.rs", 255),
    ("plugins/resonance-amp/src/nam/mod.rs", 250),
    ("resonance-svs/src/voicebank/mod.rs", 250),
    ("resonance-app/src/view/compose/drum_groups_manager/mod.rs", 249),
    ("resonance-app/src/view/compose/lane_inspector/chord/mod.rs", 240),
    ("resonance-app/src/update/project_io/reconcile/mod.rs", 236),
    ("resonance-app/src/view/compose/drumroll/mod.rs", 236),
    ("resonance-app/src/view/mixer/mod.rs", 233),
    ("plugins/resonance-stereo/src/editor/mod.rs", 227),
    ("plugins/resonance-reverb/src/editor/mod.rs", 226),
    ("resonance-app/src/update/control/track/mod.rs", 223),
    ("resonance-audio/src/types/tempo/mod.rs", 219),
    ("plugins/resonance-gate/src/editor/mod.rs", 212),
    ("plugins/resonance-amp/src/nam/wavenet/model/mod.rs", 211),
    ("resonance-app/src/compose/vocal_svs/mod.rs", 209),
    ("resonance-music-theory/src/generator/mod.rs", 208),
    ("resonance-app/src/view/compose/lane_inspector/drums/mod.rs", 204),
];

/// How many lines a `mod.rs` may have without an entry above.
const MOD_RS_LINE_LIMIT: usize = 200;

/// ARCHITECTURE.md → Anti-Patterns: "`mod.rs` should re-export and
/// dispatch, not house types", with ~200 lines as the tell. 64 files broke
/// it when ARCH2-12 was filed and nothing checked. This is a ratchet: the
/// listed files may not grow past their cap, nothing unlisted may pass the
/// limit, and an entry whose file has come back under the limit must go.
///
/// Exercised 2026-10-04: lowered `resonance-app/src/view/mod.rs`'s cap by
/// one (text only) → failed naming it; reverted.
#[test]
fn mod_rs_files_only_shrink() {
    let root = workspace_root();
    let mut files = Vec::new();
    for pkg in packages() {
        rust_files(&pkg.dir.join("src"), &mut files);
    }
    files.sort();
    files.dedup();
    let mut violations = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if file.file_name().is_none_or(|n| n != "mod.rs") {
            continue;
        }
        let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        let lines = fs::read_to_string(&file)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        let cap = MOD_RS_ALLOWANCE
            .iter()
            .find(|(p, _)| *p == rel)
            .map(|(_, c)| *c);
        if cap.is_some() {
            seen.insert(rel.clone());
        }
        match cap {
            Some(cap) if lines > cap => violations.push(format!(
                "{rel}: {lines} lines, over its {cap}-line cap — move the growth into a \
                 sibling module, not into mod.rs"
            )),
            Some(_) if lines <= MOD_RS_LINE_LIMIT => violations.push(format!(
                "{rel}: {lines} lines — under the limit now; drop it from `MOD_RS_ALLOWANCE`"
            )),
            None if lines > MOD_RS_LINE_LIMIT => violations.push(format!(
                "{rel}: {lines} lines — a mod.rs re-exports and dispatches; give the directory \
                 real modules (ARCHITECTURE.md → Anti-Patterns)"
            )),
            _ => {}
        }
    }
    for (p, _) in MOD_RS_ALLOWANCE {
        if !seen.contains(*p) {
            violations.push(format!("{p}: in `MOD_RS_ALLOWANCE` but no such file — drop it"));
        }
    }
    report(
        "ARCH2-12: `mod.rs` files stay under 200 lines, or under their listed, shrinking cap",
        &violations,
    );
}
