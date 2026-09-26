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
//!   reference file against the router's actual tool list — and so is
//!   every BARE `<namespace>_<name>` token (`section_place`, `edit_undo`),
//!   which is how the skills mostly name tools; the few wire field names
//!   of that shape are allow-listed and checked against the schemas;
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

/// Identifier-shaped tokens in `text` whose first `_`-separated segment
/// is one of `namespaces` — i.e. things that look like a bare tool name
/// (`section_place`, `generate_part`). The `mcp__resonance__` prefixed
/// form is skipped here; [`referenced_tools`] covers it.
fn bare_tool_like_tokens(text: &str, namespaces: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let c = bytes[i] as char;
        if !is_ident(c) {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && is_ident(bytes[i] as char) {
            i += 1;
        }
        // A token glued to a path, a URL or a `${VAR}` is not a tool
        // name standing on its own.
        let before = text[..start].chars().next_back();
        if matches!(before, Some('/' | '.' | '{' | '$' | '-')) {
            continue;
        }
        let token = &text[start..i];
        if token.starts_with("mcp__") || token.contains("__") {
            continue;
        }
        let Some((head, tail)) = token.split_once('_') else {
            continue;
        };
        if tail.is_empty() || !token.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            continue;
        }
        if namespaces.contains(head) {
            found.insert(token.to_owned());
        }
    }
    found
}

/// Tokens shaped like `<namespace>_<word>` that are wire FIELD names, not
/// tool names. Each must still appear as a property in some published
/// tool's input or output schema, so a renamed field fails here too.
const WIRE_FIELDS: &[&str] = &["clip_id", "clip_ids", "section_id", "track_id"];

/// Namespace-shaped keys of the plugin's own files (`lockstep.json`).
const PLUGIN_FILE_KEYS: &[&str] = &["control_protocol_version"];

/// Every published tool's input and output schema, serialized — enough
/// to ask "does any schema carry a property named X".
fn schema_text() -> String {
    ResonanceMcp::combined_router()
        .list_all()
        .into_iter()
        .map(|tool| {
            format!(
                "{}{}",
                serde_json::to_string(&tool.input_schema).expect("input schema serializes"),
                serde_json::to_string(&tool.output_schema).expect("output schema serializes"),
            )
        })
        .collect()
}

/// Skills mostly name tools bare (`section_place`, `edit_undo`), not with
/// the `mcp__resonance__` prefix. A renamed tool referenced only that way
/// used to pass the suite (CTL-12), so bare `<namespace>_<name>` tokens
/// must name published tools too.
#[test]
fn skills_bare_tool_names_exist() {
    let published = tool_names();
    let namespaces: BTreeSet<String> = published
        .iter()
        .filter_map(|t| t.split_once('_').map(|(head, _)| head.to_owned()))
        .collect();
    let mut files = Vec::new();
    markdown_files(&plugin_dir(), &mut files);

    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        for token in bare_tool_like_tokens(&text, &namespaces) {
            let known_field = WIRE_FIELDS.contains(&token.as_str())
                || PLUGIN_FILE_KEYS.contains(&token.as_str());
            if !published.contains(&token) && !known_field {
                missing.push(format!("{}: {token}", file.display()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "skills name tools this server does not publish (or, for a genuine wire field, add it \
         to WIRE_FIELDS):\n{}",
        missing.join("\n")
    );
}

/// The wire fields [`skills_bare_tool_names_exist`] lets through must
/// still exist, or the allow-list becomes a hiding place.
#[test]
fn allow_listed_wire_fields_exist_in_the_schemas() {
    let schemas = schema_text();
    for field in WIRE_FIELDS {
        assert!(
            schemas.contains(&format!("\"{field}\"")),
            "{field} is allow-listed as a wire field but no published tool schema has it — \
             the field was renamed; update the skills and WIRE_FIELDS"
        );
    }
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The string keys of every first-party plugin parameter: the first
/// string literal after each `Param::new(` in `plugins/*/src`. These are
/// what `set_plugin_param` accepts as a stable `param` (FU-M5c) and what
/// presets store, so a skill naming one must name one that exists.
fn first_party_param_keys() -> BTreeSet<String> {
    let plugins = plugin_dir()
        .parent()
        .expect("workspace root")
        .join("plugins");
    let mut files = Vec::new();
    rust_files(&plugins, &mut files);
    let mut keys = BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read plugin source");
        let mut rest = text.as_str();
        while let Some(at) = rest.find("Param::new(") {
            rest = &rest[at + "Param::new(".len()..];
            let Some(literal) = rest.trim_start().strip_prefix('"') else {
                continue;
            };
            if let Some(end) = literal.find('"') {
                keys.insert(literal[..end].to_owned());
            }
        }
    }
    keys
}

/// Snake-case identifiers written as inline code (`` `lim_on` ``).
fn backticked_snake_tokens(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (i, span) in text.split('`').enumerate() {
        let is_code = i % 2 == 1;
        let snake = span.contains('_')
            && !span.contains("__")
            && span.starts_with(|c: char| c.is_ascii_lowercase())
            && span
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if is_code && snake {
            found.insert(span.to_owned());
        }
    }
    found
}

/// Inline-code snake tokens that are neither a tool, a schema property
/// nor a plugin param key: control-protocol method suffixes the skills
/// quote.
const PROSE_TOKENS: &[&str] = &["insert_bars"];

/// Every published tool description, joined — job results (`meter.*`)
/// and generator params are untyped in the schemas and documented there.
fn description_text() -> String {
    ResonanceMcp::combined_router()
        .list_all()
        .into_iter()
        .map(|tool| format!("{}\n", tool.description.as_deref().unwrap_or_default()))
        .collect()
}

/// Whether `word` occurs in `text` as a whole identifier.
fn contains_word(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// FU-M5b: skills name plugin parameters (the mastering stage switches)
/// and wire fields as inline code. Each such token must still exist — as
/// a published tool, a property in some tool's schema or description, or
/// a first-party plugin parameter key — or a renamed parameter leaves the
/// skill setting something that is refused as not found.
#[test]
fn skills_inline_identifiers_exist() {
    let published = tool_names();
    let schemas = schema_text();
    let descriptions = description_text();
    let keys = first_party_param_keys();
    assert!(
        keys.contains("lim_on") && keys.contains("threshold"),
        "sanity check failed: the param-key scan found {} keys",
        keys.len()
    );
    let mut files = Vec::new();
    markdown_files(&plugin_dir(), &mut files);

    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        for token in backticked_snake_tokens(&text) {
            let known = published.contains(&token)
                || schemas.contains(&format!("\"{token}\""))
                || contains_word(&descriptions, &token)
                || keys.contains(&token)
                || PLUGIN_FILE_KEYS.contains(&token.as_str())
                || PROSE_TOKENS.contains(&token.as_str());
            if !known {
                missing.push(format!("{}: `{token}`", file.display()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "skills name identifiers that are no tool, schema field or first-party plugin \
         parameter key:\n{}",
        missing.join("\n")
    );
}
