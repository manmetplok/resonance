//! Keeps `resonance-agent-plugin` in lockstep with this crate.
//!
//! The plugin ships skills — prose procedures — that name MCP tools and
//! assume a control-protocol shape. Prose does not fail to compile, so
//! without these checks a renamed tool or a protocol bump leaves the
//! skills quietly describing a surface that no longer exists, and the
//! agent following them only finds out mid-mix, one failed tool call at
//! a time.
//!
//! What is pinned:
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
//!   a reference that has been renamed just carries on without it;
//! - every plugin param key, choice label and factory preset name a skill
//!   names inside a `<!-- keys: <plugin id> --> … <!-- /keys -->` block,
//!   against that plugin's real table (warmth-width-depth.md §8.1). This
//!   crate may not link plugins (it depends on `resonance-control` only),
//!   so the table check itself runs in each plugin crate's
//!   `tests/skill_keys.rs` (`resonance_dsp_test_support::skill_keys`).
//!   What runs here is the other half: the blocks are well formed, and
//!   every plugin a block names has that test, on its own crate and
//!   plugin type, so no block goes unchecked;
//! - outside the blocks, no inline-code span may be a param key or preset
//!   name (it would be checked against no plugin), and every other
//!   single-word span must be a tool, a control method, a schema or
//!   description word, a plugin id, a skill or a crate;
//! - every control method a skill's preflight names, against
//!   `resonance_control::methods::capabilities()`: a misspelt one would
//!   stop the skill as "app too old" on a build that has everything.
//!
//! The scans above the keys-block checks read the text *outside* the
//! blocks.

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
const WIRE_FIELDS: &[&str] = &[
    "clip_id",
    "clip_ids",
    "pool_asset_id",
    "reference_id",
    "section_id",
    "track_id",
];

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
        let text = strip_key_blocks(&std::fs::read_to_string(&file).expect("read skill markdown"));
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
    for field in WIRE_FIELDS.iter().chain(KEY_SHAPED_WIRE_WORDS) {
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

/// Every inline-code span in `text`, trimmed, outside fenced code (whose
/// lines are tool-call sketches, not names).
fn inline_code_spans(text: &str) -> Vec<String> {
    // Blank fenced code first (markers included), then pair backticks
    // across the rest: an inline span may wrap onto the next line.
    let mut prose = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            prose.push_str(line);
        }
        prose.push('\n');
    }
    prose
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|span| span.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|span| !span.is_empty())
        .collect()
}

/// The skill names (`mixing`, `song-structure`): the directories under
/// `skills/`. Skills point at each other by name.
fn skill_names() -> BTreeSet<String> {
    std::fs::read_dir(plugin_dir().join("skills"))
        .expect("read skills dir")
        .map(|e| e.expect("read dir entry").file_name().to_string_lossy().into_owned())
        .collect()
}

/// Workspace crate directory names (`resonance-mcp`, `resonance-drums`),
/// which the skills cite as sources of truth.
fn crate_names() -> BTreeSet<String> {
    let root = plugin_dir().parent().expect("workspace root").to_path_buf();
    let mut names = BTreeSet::new();
    for dir in [root.clone(), root.join("plugins")] {
        for entry in std::fs::read_dir(&dir).expect("read workspace dir") {
            let path = entry.expect("read dir entry").path();
            if path.join("Cargo.toml").is_file() {
                names.insert(path.file_name().expect("dir name").to_string_lossy().into_owned());
            }
        }
    }
    names
}

/// A span that is one word and is meant as a name: no whitespace, and not
/// a number, a path, a file name, a placeholder or a JSON fragment. Those
/// are the spans [`skills_inline_identifiers_exist`] requires to be known.
fn is_single_word_name(span: &str) -> bool {
    const FILE_EXTENSIONS: &[&str] = &[".md", ".rs", ".json", ".rproj", ".wav", ".flac", ".mid", ".sh"];
    !span.contains(char::is_whitespace)
        && !span.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '-' | '+' | '±' | '−' | '.'))
        && !span.contains(['/', '$', '{', '}', ':', '"', '=', '(', '<', '*', '[', ','])
        && !span.starts_with("mcp__")
        && !FILE_EXTENSIONS.iter().any(|ext| span.ends_with(ext))
}

/// Inline-code words that are no tool, capability, schema property or
/// description word, but are still right. Keep this list short and give
/// each entry its reason: every entry is a word the check cannot vouch
/// for.
const PROSE_TOKENS: &[&str] = &[
    // `arrangement.insert_bars`, quoted by its method suffix.
    "insert_bars",
    // JSON literals the skills quote as values.
    "null",
    "true",
    "false",
];

/// Control-protocol method names (`meter.snapshot`), as the skills'
/// preflights quote them against `control_hello`'s `capabilities`.
fn capabilities() -> BTreeSet<String> {
    resonance_control::methods::capabilities()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// Whether `span` is shaped like a control method: `<namespace>.<name>`
/// with a namespace the protocol has.
fn is_method_shaped(span: &str, capabilities: &BTreeSet<String>) -> bool {
    let Some((head, tail)) = span.split_once('.') else {
        return false;
    };
    !tail.is_empty()
        && capabilities.iter().any(|m| m.split_once('.').is_some_and(|(ns, _)| ns == head))
}

/// Every first-party factory preset name: the `name: "…"` literals of
/// `plugins/*/src/presets.rs`.
fn first_party_preset_names() -> BTreeSet<String> {
    const MARK: &str = "name: \"";
    let plugins = plugin_dir()
        .parent()
        .expect("workspace root")
        .join("plugins");
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(&plugins).expect("read plugins dir") {
        let Ok(text) = std::fs::read_to_string(entry.expect("read dir entry").path().join("src/presets.rs")) else {
            continue;
        };
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix(MARK) {
                if let Some(end) = rest.find('"') {
                    names.insert(rest[..end].to_owned());
                }
            }
        }
    }
    names
}

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

/// Param keys that are also wire words the skills quote outside any keys
/// block, meaning the wire word. Each must be a property in some tool's
/// schema ([`allow_listed_wire_fields_exist_in_the_schemas`] checks it),
/// and the plugin-side check holds the same list
/// (`resonance_dsp_test_support::skill_keys::WIRE_WORDS`;
/// [`wire_word_lists_agree`] keeps the two equal).
const KEY_SHAPED_WIRE_WORDS: &[&str] = &[
    // `meter_compare {a, b}`: the second measurement.
    "b",
    // The `range` every meter_* call takes.
    "range",
    // `render_mixdown`'s `normalize: {target_lufs, …}` and the assistant's
    // `target_lufs` stage.
    "target_lufs",
    // `section_set_scale`'s `scale`.
    "scale",
    // The `character` facet filter of `*_plugin_presets` (timbre words).
    "character",
];

/// The two halves of the outside-a-block check must excuse the same
/// words, or one half flags what the other lets through.
#[test]
fn wire_word_lists_agree() {
    const MARK: &str = "pub const WIRE_WORDS: &[&str] = &[";
    let source = plugin_dir()
        .parent()
        .expect("workspace root")
        .join("resonance-dsp-test-support/src/skill_keys.rs");
    let text = std::fs::read_to_string(&source).expect("read skill_keys.rs");
    let list = text
        .split_once(MARK)
        .and_then(|(_, rest)| rest.split_once("];"))
        .map(|(list, _)| list)
        .expect("skill_keys.rs declares WIRE_WORDS");
    let theirs: BTreeSet<&str> = list.split(',').map(|w| w.trim().trim_matches('"')).filter(|w| !w.is_empty()).collect();
    let ours: BTreeSet<&str> = KEY_SHAPED_WIRE_WORDS.iter().copied().collect();
    assert_eq!(ours, theirs, "KEY_SHAPED_WIRE_WORDS and skill_keys::WIRE_WORDS diverged");
}

/// The param keys and preset names the scans outside keys blocks know,
/// with a sanity check that the source scans found anything.
fn plugin_names() -> (BTreeSet<String>, BTreeSet<String>) {
    let keys = first_party_param_keys();
    assert!(
        keys.contains("lim_on") && keys.contains("threshold"),
        "sanity check failed: the param-key scan found {} keys",
        keys.len()
    );
    let presets = first_party_preset_names();
    assert!(
        presets.contains("Bus — Warm Glue") && presets.contains("Tight Room"),
        "sanity check failed: the preset scan found {} names",
        presets.len()
    );
    (keys, presets)
}

/// A key or preset outside a keys block is checked by nobody against the
/// plugin it belongs to: this crate can only tell that *some* plugin has
/// it, which is how `tone_b0_gain` could be cited for the EQ. So outside
/// the blocks, every inline-code span that exactly matches any
/// first-party param key or preset name fails, and the fix is to wrap the
/// sentence in a block for its plugin (the plugin's `skill_keys` test then
/// checks it properly). Keys come from the `Param::new("…")` literals, so
/// one built by a helper or `format!` is missed here; every plugin with a
/// `skill_keys` test runs the same check from its exact table
/// (`skill_keys::loose_names`), which catches those. Scans `skills/` only:
/// the README quotes keys to explain the convention.
#[test]
fn keys_and_presets_sit_inside_key_blocks() {
    let (keys, presets) = plugin_names();
    let mut files = Vec::new();
    markdown_files(&plugin_dir().join("skills"), &mut files);

    let mut loose = Vec::new();
    for file in files {
        let text = strip_key_blocks(&std::fs::read_to_string(&file).expect("read skill markdown"));
        for span in inline_code_spans(&text) {
            let is_key = keys.contains(&span) && !KEY_SHAPED_WIRE_WORDS.contains(&span.as_str());
            if is_key || presets.contains(&span) {
                loose.push(format!("{}: `{span}`", file.display()));
            }
        }
    }
    assert!(
        loose.is_empty(),
        "skills name plugin param keys or preset names outside a `<!-- keys: <plugin id> -->` \
         block, where nothing checks them against their plugin. Wrap the sentence or table in a \
         block for that plugin (resonance-agent-plugin/README.md, \"Naming plugin keys\"); if the \
         span means a wire field of the same spelling, add it to KEY_SHAPED_WIRE_WORDS:\n{}",
        loose.join("\n")
    );
}

/// FU-M5b: skills name tools, wire fields and methods as inline code.
/// Outside keys blocks, every single-word span under `skills/` must still
/// exist — as a published tool, a control method, a property in some
/// tool's schema, a word of some tool's description (a dotted path when
/// each segment is), a first-party plugin id, a skill or a workspace crate
/// — or a rename leaves the skill naming something that is refused as not
/// found. A span shaped like a control method (`meter.snapshot`) must be
/// one exactly. Keys and presets are
/// [`keys_and_presets_sit_inside_key_blocks`]' business. The README is
/// documentation for people, not a procedure, and is not scanned.
#[test]
fn skills_inline_identifiers_exist() {
    let published = tool_names();
    let methods = capabilities();
    let schemas = schema_text();
    let descriptions = description_text();
    let plugin_ids: BTreeSet<String> = first_party_plugins().into_iter().map(|(id, _)| id).collect();
    let (keys, presets) = plugin_names();
    let (skills, crates) = (skill_names(), crate_names());
    let mut files = Vec::new();
    markdown_files(&plugin_dir().join("skills"), &mut files);

    let mut missing = Vec::new();
    for file in files {
        let text = strip_key_blocks(&std::fs::read_to_string(&file).expect("read skill markdown"));
        for span in inline_code_spans(&text) {
            if !is_single_word_name(&span) || keys.contains(&span) || presets.contains(&span) {
                continue;
            }
            if is_method_shaped(&span, &methods) {
                if !methods.contains(&span) {
                    missing.push(format!("{}: `{span}` (no such control method)", file.display()));
                }
                continue;
            }
            let word_known = |word: &str| {
                published.contains(word)
                    || schemas.contains(&format!("\"{word}\""))
                    || contains_word(&descriptions, word)
            };
            // A dotted field path (`song_summary.time_signature`) is known
            // when each of its segments is.
            let known = word_known(&span)
                || (span.contains('.') && span.split('.').all(|seg| !seg.is_empty() && word_known(seg)))
                || plugin_ids.contains(&span)
                || skills.contains(&span)
                || crates.contains(&span)
                || PLUGIN_FILE_KEYS.contains(&span.as_str())
                || PROSE_TOKENS.contains(&span.as_str());
            if !known {
                missing.push(format!("{}: `{span}`", file.display()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "skills name inline-code words that are no tool, control method, schema field, \
         description word or plugin id. Fix the name; if it is a plugin key, label or preset, \
         put it in a keys block; if it is right and none of those, add it to PROSE_TOKENS with \
         its reason:\n{}",
        missing.join("\n")
    );
}

/// The section of a `SKILL.md` under its `Preflight` heading, up to the
/// next `## ` heading.
fn preflight_section(text: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| !(l.starts_with("## ") && l.contains("Preflight")));
    let heading = lines.next()?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("## ")).collect();
    Some(format!("{heading}\n{}", body.join("\n")))
}

/// Each skill's preflight checks named control methods against
/// `control_hello`'s `capabilities`. A misspelt one reads as "the app is
/// too old" and stops the skill on a build that has everything, so every
/// method a preflight names must be a real one, and a preflight must name
/// at least one.
#[test]
fn preflights_name_real_capabilities() {
    let methods = capabilities();
    let mut files = Vec::new();
    markdown_files(&plugin_dir().join("skills"), &mut files);

    let mut problems = Vec::new();
    for file in files
        .into_iter()
        .filter(|f| f.file_name().is_some_and(|n| n == "SKILL.md"))
    {
        let text = std::fs::read_to_string(&file).expect("read SKILL.md");
        let Some(section) = preflight_section(&text) else {
            problems.push(format!("{}: no `## … Preflight` section", file.display()));
            continue;
        };
        let named: Vec<String> = inline_code_spans(&section)
            .into_iter()
            .filter(|s| is_method_shaped(s, &methods))
            .collect();
        if named.is_empty() {
            problems.push(format!("{}: the preflight names no control method", file.display()));
        }
        for name in named.iter().filter(|n| !methods.contains(*n)) {
            problems.push(format!("{}: preflight names `{name}`, not a control method", file.display()));
        }
    }
    assert!(problems.is_empty(), "preflights:\n{}", problems.join("\n"));
}

// ---------------------------------------------------------------------------
// Keys blocks (warmth-width-depth.md §8.1)
// ---------------------------------------------------------------------------

const KEYS_OPEN: &str = "<!-- keys:";
const KEYS_CLOSE: &str = "<!-- /keys -->";

/// `text` with every keys block, markers included, blanked out (line
/// count kept). The spans inside are checked against the plugin's own
/// table by its `tests/skill_keys.rs`, which is stricter than anything
/// this crate can do from source; here a key such as `clip_on` would read
/// as a misspelt `clip_*` tool.
fn strip_key_blocks(text: &str) -> String {
    // Markers inside fenced code outside a block are examples (the README
    // shows one), so fences are tracked the way the plugin-side parser
    // tracks them.
    let mut inside = false;
    let mut fenced = false;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim();
        if !inside && t.starts_with("```") {
            fenced = !fenced;
            out.push_str(line);
        } else if fenced {
            out.push_str(line);
        } else if t.starts_with(KEYS_OPEN) {
            inside = true;
        } else if t == KEYS_CLOSE {
            inside = false;
        } else if !inside {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// The plugin ids of the keys blocks in `text`, in order, or what is
/// wrong with the markers.
fn key_block_ids(text: &str) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut open: Option<usize> = None;
    let mut fenced = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if open.is_none() && t.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(rest) = t.strip_prefix(KEYS_OPEN) {
            if let Some(at) = open {
                return Err(format!("line {}: block opened inside the one from line {at}", i + 1));
            }
            let id = rest.trim().strip_suffix("-->").unwrap_or("").trim();
            if !id.starts_with("com.resonance.") || id.contains(char::is_whitespace) {
                return Err(format!(
                    "line {}: write the marker as `{KEYS_OPEN} com.resonance.<plugin> -->`",
                    i + 1
                ));
            }
            ids.push(id.to_owned());
            open = Some(i + 1);
        } else if t == KEYS_CLOSE {
            if open.take().is_none() {
                return Err(format!("line {}: `{KEYS_CLOSE}` with no open block", i + 1));
            }
        } else if open.is_some() && t.starts_with("```") {
            return Err(format!("line {}: fenced code inside a keys block", i + 1));
        }
    }
    match open {
        Some(at) => Err(format!("line {at}: keys block never closed")),
        None => Ok(ids),
    }
}

/// First-party plugins as `(CLAP id, crate dir)`, read from each
/// `plugins/*/src/lib.rs`.
fn first_party_plugins() -> Vec<(String, PathBuf)> {
    const MARK: &str = "const CLAP_ID: &'static str = \"";
    let plugins = plugin_dir()
        .parent()
        .expect("workspace root")
        .join("plugins");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&plugins).expect("read plugins dir") {
        let dir = entry.expect("read dir entry").path();
        let Ok(lib) = std::fs::read_to_string(dir.join("src/lib.rs")) else {
            continue;
        };
        if let Some(at) = lib.find(MARK) {
            let rest = &lib[at + MARK.len()..];
            let id = &rest[..rest.find('"').expect("CLAP_ID literal closes")];
            out.push((id.to_owned(), dir));
        }
    }
    out
}

/// Whether `test` (a plugin crate's `tests/skill_keys.rs`) really runs the
/// check for the plugin in `dir`: an uncommented `skill_keys_test!` whose
/// argument is `<this crate>::<the type lib.rs implements the plugin on>`.
/// A macro naming another crate's plugin compiles (given the dev-dep) and
/// passes, while this plugin's blocks go unchecked.
fn skill_keys_test_checks(dir: &Path, test: &Path) -> Result<(), String> {
    const MACRO: &str = "skill_keys_test!(";
    let text = std::fs::read_to_string(test)
        .map_err(|_| format!("{} does not exist; add the one-line `skill_keys_test!`", test.display()))?;
    let call = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .find_map(|l| l.split_once(MACRO).map(|(_, rest)| rest))
        .ok_or_else(|| format!("{} does not call `resonance_dsp_test_support::{MACRO}…)`", test.display()))?;
    let arg = call.split(')').next().unwrap_or("").trim();
    let (krate, ty) = arg
        .rsplit_once("::")
        .ok_or_else(|| format!("{}: write the argument as `<crate>::<Plugin>`, not `{arg}`", test.display()))?;
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).expect("read plugin Cargo.toml");
    let package = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("name = \""))
        .and_then(|rest| rest.split('"').next())
        .expect("plugin Cargo.toml has a package name")
        .replace('-', "_");
    if krate.trim_start_matches("::") != package {
        return Err(format!(
            "{} checks `{arg}`, which is not this crate (`{package}`): its blocks go unchecked",
            test.display()
        ));
    }
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).expect("read plugin lib.rs");
    if !lib.contains(&format!("ResonancePlugin for {ty} ")) && !lib.contains(&format!("ResonancePlugin for {ty}\n")) {
        return Err(format!(
            "{} checks `{arg}`, but {} does not implement the plugin on `{ty}`",
            test.display(),
            dir.join("src/lib.rs").display()
        ));
    }
    Ok(())
}

/// Every keys block is well formed and names a first-party plugin whose
/// crate runs `skill_keys_test!` on that plugin. A block nobody checks is
/// worse than no block: it looks verified.
#[test]
fn every_keys_block_names_a_plugin_that_checks_it() {
    let plugins = first_party_plugins();
    assert!(
        plugins.iter().any(|(id, _)| id == "com.resonance.mastering"),
        "sanity check failed: the CLAP_ID scan found {} plugins",
        plugins.len()
    );
    let mut files = Vec::new();
    markdown_files(&plugin_dir(), &mut files);

    let mut blocks = 0;
    let mut problems = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        let ids = match key_block_ids(&text) {
            Ok(ids) => ids,
            Err(e) => {
                problems.push(format!("{}: {e}", file.display()));
                continue;
            }
        };
        blocks += ids.len();
        for id in ids {
            let Some((_, dir)) = plugins.iter().find(|(p, _)| *p == id) else {
                problems.push(format!("{}: `{id}` is not a first-party plugin", file.display()));
                continue;
            };
            let test = dir.join("tests/skill_keys.rs");
            if let Err(why) = skill_keys_test_checks(dir, &test) {
                problems.push(format!("{}: names `{id}`, but {why}", file.display()));
            }
        }
    }
    assert!(blocks > 0, "no keys blocks found: the skills name no plugin keys at all?");
    assert!(problems.is_empty(), "keys blocks:\n{}", problems.join("\n"));
}
