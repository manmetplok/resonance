//! The plugin half of the skill ↔ plugin lockstep (warmth-width-depth.md
//! §8.1): every param key, choice label and factory preset name that a
//! skill in `resonance-agent-plugin/` names inside a **keys block** for a
//! plugin must exist in that plugin's real parameter table.
//!
//! A keys block is a region of skill markdown between two HTML comments,
//! each on its own line:
//!
//! ```text
//! <!-- keys: com.resonance.color -->
//! ... every `inline code` span here is checked ...
//! <!-- /keys -->
//! ```
//!
//! Inside it, every inline-code span must be one of that plugin's param
//! string keys (`drive`), one of its stepped params' value labels
//! (`Tape`), or one of its factory preset names (`Bus — Warm Glue`). A
//! span may use `{n}` for a band index (`corr_b{n}_gain`), which matches
//! when at least one real key fits with digits in its place. Fenced code
//! and nested blocks are refused, so a block reads as a key map and
//! nothing else.
//!
//! Why here and not in `resonance-mcp/tests/agent_plugin_lockstep.rs`:
//! reading a plugin's real table means linking the plugin, and the
//! layering (ARCHITECTURE.md, `tools/arch-invariants`) lets `resonance-mcp`
//! depend on `resonance-control` only. Each plugin crate therefore runs
//! this check on itself through [`skill_keys_test!`](crate::skill_keys_test),
//! while `agent_plugin_lockstep.rs` checks that every block names a plugin
//! whose crate carries that test. Parsing the params sources instead was
//! the fallback, and it cannot see keys built by `concat!` or `format!`
//! (the EQ's `band{n}_*`, mastering's `{prefix}_b{n}_*`).
//!
//! Nothing here depends on the plugin SDK: the macro collects the
//! surface at the call site, where `resonance_plugin` is in scope.

use std::path::{Path, PathBuf};

/// The opening marker's prefix; the plugin id and ` -->` follow.
pub const OPEN: &str = "<!-- keys:";
/// The closing marker.
pub const CLOSE: &str = "<!-- /keys -->";

/// What a skill may name for one plugin.
#[derive(Debug, Default)]
pub struct Surface {
    pub clap_id: String,
    pub keys: Vec<String>,
    /// The display text of every value of every stepped param (choice
    /// labels such as `Tape`, `On`/`Off` for switches).
    pub labels: Vec<String>,
    pub presets: Vec<String>,
}

/// One inline-code span inside a keys block.
#[derive(Debug, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub text: String,
}

/// One keys block: the plugin it names and the spans inside it.
#[derive(Debug, PartialEq, Eq)]
pub struct Block {
    pub line: usize,
    pub plugin_id: String,
    pub spans: Vec<Span>,
}

/// Every keys block in `text`, or a description of the first malformed
/// one (unclosed, nested, a stray close, fenced code inside, no id).
pub fn blocks(text: &str) -> Result<Vec<Block>, String> {
    let mut out = Vec::new();
    let mut open: Option<Block> = None;
    // A marker inside fenced code outside any block is an example (the
    // plugin README shows one), not a block.
    let mut fenced = false;
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim();
        if open.is_none() && line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(rest) = line.strip_prefix(OPEN) {
            if let Some(block) = &open {
                return Err(format!(
                    "line {line_no}: keys block opened inside the block from line {}",
                    block.line
                ));
            }
            let id = rest.trim().strip_suffix("-->").unwrap_or("").trim();
            if id.is_empty() || id.contains(char::is_whitespace) {
                return Err(format!(
                    "line {line_no}: write the marker as `{OPEN} <plugin id> -->`"
                ));
            }
            open = Some(Block {
                line: line_no,
                plugin_id: id.to_owned(),
                spans: Vec::new(),
            });
            continue;
        }
        if line == CLOSE {
            match open.take() {
                Some(block) => out.push(block),
                None => return Err(format!("line {line_no}: `{CLOSE}` with no open block")),
            }
            continue;
        }
        let Some(block) = &mut open else {
            continue;
        };
        if line.starts_with("```") {
            return Err(format!(
                "line {line_no}: fenced code inside the keys block from line {}",
                block.line
            ));
        }
        for (j, span) in raw.split('`').enumerate() {
            if j % 2 == 1 && !span.trim().is_empty() {
                block.spans.push(Span {
                    line: line_no,
                    text: span.trim().to_owned(),
                });
            }
        }
    }
    match open {
        Some(block) => Err(format!("line {}: keys block never closed", block.line)),
        None => Ok(out),
    }
}

/// `text` with every keys block (markers included) blanked out, line
/// numbers kept. `agent_plugin_lockstep.rs` scans what is left.
pub fn strip_blocks(text: &str) -> String {
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
        } else if t.starts_with(OPEN) {
            inside = true;
        } else if t == CLOSE {
            inside = false;
        } else if !inside {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Whether `pattern`, with each `{n}` standing for a run of digits,
/// matches `key` exactly.
pub fn matches_pattern(pattern: &str, key: &str) -> bool {
    let parts: Vec<&str> = pattern.split("{n}").collect();
    let Some(rest) = key.strip_prefix(parts[0]) else {
        return false;
    };
    fn walk(rest: &str, parts: &[&str]) -> bool {
        let Some((literal, tail)) = parts.split_first() else {
            return rest.is_empty();
        };
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        // Try every non-empty digit run, so `b{n}1` still resolves.
        (1..=digits).any(|d| {
            rest[d..]
                .strip_prefix(literal)
                .is_some_and(|after| walk(after, tail))
        })
    }
    walk(rest, &parts[1..])
}

/// Whether a span names something `surface` has.
pub fn known(surface: &Surface, span: &str) -> bool {
    if span.contains("{n}") {
        return surface.keys.iter().any(|k| matches_pattern(span, k));
    }
    surface.keys.iter().any(|k| k == span)
        || surface.labels.iter().any(|l| l == span)
        || surface.presets.iter().any(|p| p == span)
}

/// The `resonance-agent-plugin` directory, found from a crate root.
pub fn agent_plugin_dir(manifest_dir: &str) -> PathBuf {
    Path::new(manifest_dir)
        .ancestors()
        .map(|dir| dir.join("resonance-agent-plugin"))
        .find(|dir| dir.is_dir())
        .expect("resonance-agent-plugin/ not found above the crate")
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read agent plugin dir") {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// Check every keys block for `surface.clap_id` across the agent
/// plugin's markdown. Panics listing each unknown span.
pub fn assert_skills_match(manifest_dir: &str, surface: &Surface) {
    assert!(
        !surface.keys.is_empty(),
        "{}: the param table came back empty",
        surface.clap_id
    );
    // `RESONANCE_SKILL_KEYS_DUMP=1 cargo test -p <plugin> --test skill_keys
    // -- --nocapture` prints what a block may name, for skill authors.
    if std::env::var("RESONANCE_SKILL_KEYS_DUMP").is_ok_and(|v| v == "1") {
        println!("{}\nkeys: {:?}\nlabels: {:?}\npresets: {:?}", surface.clap_id, surface.keys, surface.labels, surface.presets);
    }
    let mut files = Vec::new();
    markdown_files(&agent_plugin_dir(manifest_dir), &mut files);
    files.sort();
    let mut bad = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        let found = blocks(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for block in found.iter().filter(|b| b.plugin_id == surface.clap_id) {
            for span in &block.spans {
                if !known(surface, &span.text) {
                    bad.push(format!("{}:{}: `{}`", file.display(), span.line, span.text));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "skills name things {} does not have. Inside a `{OPEN} {} -->` block every inline-code \
         span must be a param key, a stepped param's value label or a factory preset name of \
         that plugin (use `{{n}}` for a band index). Fix the skill, or move prose that is not \
         a key out of the block:\n{}",
        surface.clap_id,
        surface.clap_id,
        bad.join("\n")
    );
}

/// Define the lockstep test for one plugin type. Call it from a file in
/// the plugin crate's own `tests/`:
///
/// ```ignore
/// resonance_dsp_test_support::skill_keys_test!(resonance_color::ResonanceColor);
/// ```
///
/// It reads the plugin's params through the SDK without a host: `new()`,
/// then `param(i)` for every index, and `FACTORY_PRESETS`.
#[macro_export]
macro_rules! skill_keys_test {
    ($plugin:ty) => {
        #[test]
        fn skills_name_only_real_keys_and_presets() {
            use ::resonance_plugin::{Param as _, ResonancePlugin as _};
            let plugin = <$plugin as ::resonance_plugin::ResonancePlugin>::new();
            let mut surface = $crate::skill_keys::Surface {
                clap_id: <$plugin as ::resonance_plugin::ResonancePlugin>::CLAP_ID.to_owned(),
                ..Default::default()
            };
            for i in 0..plugin.param_count() {
                let param = plugin.param(i);
                surface.keys.push(param.id().to_owned());
                if param.is_stepped() {
                    let (lo, hi) = (param.min_plain().round() as i64, param.max_plain().round() as i64);
                    for v in lo..=hi {
                        surface.labels.push(param.display(v as f64));
                    }
                }
            }
            surface.presets = <$plugin as ::resonance_plugin::ResonancePlugin>::FACTORY_PRESETS
                .iter()
                .map(|p| p.name.to_owned())
                .collect();
            $crate::skill_keys::assert_skills_match(env!("CARGO_MANIFEST_DIR"), &surface);
        }
    };
}
