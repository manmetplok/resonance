//! Keeps `resonance-agent-plugin` in lockstep with this crate.
//!
//! The plugin ships skills — prose procedures — that name MCP tools and
//! assume a control-protocol shape. Prose does not fail to compile, so
//! without these checks a renamed tool or a protocol bump leaves the
//! skills quietly describing a surface that no longer exists, and the
//! agent following them only finds out mid-mix, one failed tool call at
//! a time.
//!
//! Two things are pinned:
//!
//! - the plugin's `lockstep.json` against
//!   [`resonance_control::PROTOCOL_VERSION`], so bumping the protocol
//!   forces a look at whether the procedures still hold. The pin lives in
//!   its own file rather than `plugin.json`, whose schema has no field for
//!   it: `claude plugin validate` warns on unknown manifest keys, and a
//!   warning nobody can fix is a warning everybody learns to ignore;
//! - every `mcp__resonance__<tool>` mentioned in any `SKILL.md` or
//!   reference file against the router's actual tool list;
//! - every `${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_SKILL_DIR}` path the
//!   skills point each other at, against the files on disk. Progressive
//!   disclosure only works if the pointer resolves: an agent told to read
//!   a reference that has been renamed just carries on without it.

use resonance_mcp::ResonanceMcp;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The plugin directory, a sibling of this crate.
fn plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a parent")
        .join("resonance-agent-plugin")
}

/// Every published tool name.
fn tool_names() -> BTreeSet<String> {
    ResonanceMcp::combined_router()
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

/// Every `.md` file under the plugin, recursively.
fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read plugin dir") {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// The `mcp__resonance__<tool>` names referenced in `text`.
fn referenced_tools(text: &str) -> BTreeSet<String> {
    const PREFIX: &str = "mcp__resonance__";
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find(PREFIX) {
        rest = &rest[at + PREFIX.len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        if end > 0 {
            found.insert(rest[..end].to_string());
        }
        rest = &rest[end..];
    }
    found
}

#[test]
fn plugin_pins_the_current_control_protocol() {
    let pin_file = plugin_dir().join("lockstep.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pin_file).expect("read lockstep.json"))
            .expect("lockstep.json is valid JSON");

    let pinned = json["control_protocol_version"]
        .as_u64()
        .expect("lockstep.json has control_protocol_version as a number");

    assert_eq!(
        pinned,
        u64::from(resonance_control::PROTOCOL_VERSION),
        "resonance-agent-plugin pins control protocol {pinned}, but the workspace is now at {}. \
         Re-read the skills against the new protocol, then bump both the pin and the plugin's \
         `version` so installed copies pick the change up.",
        resonance_control::PROTOCOL_VERSION,
    );
}

#[test]
fn skills_only_name_tools_that_exist() {
    let published = tool_names();
    let mut files = Vec::new();
    markdown_files(&plugin_dir(), &mut files);
    assert!(!files.is_empty(), "no markdown found under the plugin dir");

    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        for tool in referenced_tools(&text) {
            assert!(
                published.contains(&tool),
                "{} names mcp__resonance__{tool}, which this server does not publish. \
                 A skill that calls a tool by a name nobody serves fails silently at the \
                 worst moment — fix the skill, or restore the tool.",
                file.display(),
            );
        }
    }
}

/// The `${VAR}/relative/path` references in `text`, as `(var, path)`.
fn referenced_paths(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for var in ["${CLAUDE_PLUGIN_ROOT}", "${CLAUDE_SKILL_DIR}"] {
        let mut rest = text;
        while let Some(at) = rest.find(var) {
            rest = &rest[at + var.len()..];
            if !rest.starts_with('/') {
                continue;
            }
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '`' || c == ')')
                .unwrap_or(rest.len());
            found.push((var.to_string(), rest[1..end].to_string()));
            rest = &rest[end..];
        }
    }
    found
}

#[test]
fn cross_references_between_skills_resolve() {
    let root = plugin_dir();
    let mut files = Vec::new();
    markdown_files(&root, &mut files);

    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        // `${CLAUDE_SKILL_DIR}` is the directory holding this SKILL.md; for a
        // reference file it is that file's own skill directory, one level up.
        let skill_dir = if file.file_name().is_some_and(|n| n == "SKILL.md") {
            file.parent().expect("SKILL.md has a parent").to_path_buf()
        } else {
            file.parent()
                .and_then(Path::parent)
                .expect("reference file sits under a skill dir")
                .to_path_buf()
        };

        for (var, rel) in referenced_paths(&text) {
            let base = if var == "${CLAUDE_PLUGIN_ROOT}" {
                root.clone()
            } else {
                skill_dir.clone()
            };
            let target = base.join(&rel);
            assert!(
                target.exists(),
                "{} points at {var}/{rel}, which does not exist. Progressive disclosure is only \
                 as good as the pointer: an agent told to read a missing reference does not \
                 error, it just proceeds without the material.",
                file.display(),
            );
        }
    }
}

#[test]
fn every_skill_preflights_with_control_hello() {
    let mut files = Vec::new();
    markdown_files(&plugin_dir().join("skills"), &mut files);

    for file in files
        .into_iter()
        .filter(|f| f.file_name().is_some_and(|n| n == "SKILL.md"))
    {
        let text = std::fs::read_to_string(&file).expect("read SKILL.md");
        assert!(
            text.contains("mcp__resonance__control_hello"),
            "{} never calls control_hello. The plugin version pins the skills to this crate, \
             but not to the app binary the user actually has open — the handshake is the only \
             thing that catches that skew.",
            file.display(),
        );
    }
}
