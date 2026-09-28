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
//! (`Tape`), or one of its factory preset names (`Bus — Warm Glue`).
//! Fenced code and nested blocks are refused, so a block reads as a key
//! map and nothing else. Three rules make the check more than a spelling
//! test:
//!
//! - **A label is bound to its key.** Each label must be a value of the
//!   nearest key named before it in the same *scope*: the same table row,
//!   list item or paragraph. In a table body a label with no key before it
//!   in its row falls back to a key in its column's header cell (`| Mode
//!   (`widen_mode`) |` over a column of modes). `Tape` after `speed` is
//!   refused, because `Tape` is a value of `mode`.
//! - **`{n}` is a band index, and ranges are counted.** A span may use
//!   `{n}` (`corr_b{n}_gain`). If the text declares a range as `n = A-B`
//!   in the same scope, or anywhere earlier in the block, every index
//!   `A..=B` must give a real key and `B + 1` must not: the text's band
//!   count is the plugin's. Without a declaration, at least one real key
//!   must fit.
//! - Nothing else is accepted: a span that is no key, label or preset
//!   fails, so a tool name or meter field inside a block fails too.
//!
//! Why here and not in `resonance-mcp/tests/agent_plugin_lockstep.rs`:
//! reading a plugin's real table means linking the plugin, and the
//! layering (ARCHITECTURE.md, `tools/arch-invariants`) lets `resonance-mcp`
//! depend on `resonance-control` only. Each plugin crate therefore runs
//! this check on itself through [`skill_keys_test!`](crate::skill_keys_test),
//! while `agent_plugin_lockstep.rs` checks that every block names a plugin
//! whose crate carries that test, and that keys and presets never appear
//! outside a block. Parsing the params sources instead was the fallback,
//! and it cannot see keys built by `concat!` or `format!` (the EQ's
//! `band{n}_*`, mastering's `{prefix}_b{n}_*`).
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
    /// Every stepped param's key with the display text of each of its
    /// values (choice labels such as `Tape`, `On`/`Off` for switches).
    pub choices: Vec<(String, Vec<String>)>,
    pub presets: Vec<String>,
}

impl Surface {
    fn is_label(&self, text: &str) -> bool {
        self.choices.iter().any(|(_, labels)| labels.iter().any(|l| l == text))
    }

    /// Whether `span` is a key, or a `{n}` pattern at least one key fits.
    fn is_key(&self, span: &str) -> bool {
        if span.contains("{n}") {
            self.keys.iter().any(|k| matches_pattern(span, k))
        } else {
            self.keys.iter().any(|k| k == span)
        }
    }

    /// The labels of every key `key` names (a `{n}` pattern names several).
    fn labels_of(&self, key: &str) -> impl Iterator<Item = &String> {
        let key = key.to_owned();
        self.choices
            .iter()
            .filter(move |(k, _)| if key.contains("{n}") { matches_pattern(&key, k) } else { *k == key })
            .flat_map(|(_, labels)| labels.iter())
    }
}

/// Where a span sits in a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Which table of the block, counting from 0.
    pub table: usize,
    /// Which column, counting from 0.
    pub column: usize,
    /// In the table's header row.
    pub header: bool,
}

/// One inline-code span inside a keys block.
#[derive(Debug, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub text: String,
    /// The table row, list item or paragraph the span sits in. Two spans
    /// share a scope exactly when they share that unit.
    pub scope: usize,
    /// Set for a span in a table row.
    pub cell: Option<Cell>,
}

/// A band range the text declares (`n = 0-3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub line: usize,
    pub scope: usize,
    pub lo: u32,
    pub hi: u32,
}

/// One keys block: the plugin it names, the spans inside it and the band
/// ranges its text declares.
#[derive(Debug, PartialEq, Eq)]
pub struct Block {
    pub line: usize,
    pub plugin_id: String,
    pub spans: Vec<Span>,
    pub ranges: Vec<Range>,
}

/// What the previous line inside a block was, for scope boundaries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prev {
    Blank,
    Row,
    Prose,
}

fn is_separator_row(line: &str) -> bool {
    line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn is_list_item(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") {
        return true;
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0 && line[digits..].starts_with(". ")
}

/// Every `n = A-B` (hyphen or en dash, spaces optional) in `text`.
pub fn declared_ranges(text: &str) -> Vec<(u32, u32)> {
    fn number(s: &str) -> Option<(u32, &str)> {
        let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let n = s[..digits].parse().ok()?;
        Some((n, &s[digits..]))
    }
    let mut out = Vec::new();
    for (at, _) in text.match_indices('n') {
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '{') {
            continue;
        }
        let rest = text[at + 1..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let Some((lo, rest)) = number(rest.trim_start()) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('-').or_else(|| rest.strip_prefix('–')) else {
            continue;
        };
        if let Some((hi, _)) = number(rest) {
            out.push((lo, hi));
        }
    }
    out
}

/// Every keys block in `text`, or a description of the first malformed
/// one (unclosed, nested, a stray close, fenced code inside, no id).
pub fn blocks(text: &str) -> Result<Vec<Block>, String> {
    let mut out = Vec::new();
    let mut open: Option<Block> = None;
    // A marker inside fenced code outside any block is an example (the
    // plugin README shows one), not a block.
    let mut fenced = false;
    let mut scope = 0;
    let (mut table, mut tables_seen, mut row) = (0, 0, 0);
    let mut prev = Prev::Blank;
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
                ranges: Vec::new(),
            });
            (tables_seen, prev) = (0, Prev::Blank);
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
        if line.is_empty() {
            prev = Prev::Blank;
            continue;
        }
        if raw.matches('`').count() % 2 == 1 {
            return Err(format!(
                "line {line_no}: an inline-code span wraps onto another line inside the keys \
                 block from line {}; keep each span on one line",
                block.line
            ));
        }
        let is_row = line.starts_with('|');
        if is_row {
            if prev == Prev::Row {
                row += 1;
            } else {
                (table, row) = (tables_seen, 0);
                tables_seen += 1;
            }
            scope += 1;
            prev = Prev::Row;
            if is_separator_row(line) {
                continue;
            }
        } else {
            if prev != Prev::Prose || is_list_item(line) {
                scope += 1;
            }
            prev = Prev::Prose;
        }
        let mut offset = 0;
        for (j, part) in raw.split('`').enumerate() {
            if j % 2 == 0 {
                for (lo, hi) in declared_ranges(part) {
                    block.ranges.push(Range { line: line_no, scope, lo, hi });
                }
            } else if !part.trim().is_empty() {
                let cell = is_row.then(|| Cell {
                    table,
                    column: raw[..offset].matches('|').count().saturating_sub(1),
                    header: row == 0,
                });
                block.spans.push(Span {
                    line: line_no,
                    text: part.trim().to_owned(),
                    scope,
                    cell,
                });
            }
            offset += part.len() + 1;
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

/// Keys that are also wire words the skills quote outside any keys
/// block, meaning the wire word: mastering's `target_lufs` is also the
/// assistant's stage id and `render_mixdown`'s `normalize` field.
/// `resonance-mcp`'s lockstep test holds the matching list
/// (`KEY_SHAPED_WIRE_WORDS`) and checks each is a real schema property.
pub const WIRE_WORDS: &[&str] = &["b", "range", "target_lufs", "scale"];

/// The inline-code spans of `text` (already stripped of its blocks) that
/// are exactly one of `surface`'s keys or preset names, as `(line, span)`.
/// Outside a block nothing ties a key to its plugin, so this plugin checks
/// its own names there too, from its exact table (the `format!`-built
/// band keys included, which a source scan cannot see). Fenced code is
/// skipped; a span may wrap across lines.
pub fn loose_names(surface: &Surface, text: &str) -> Vec<(usize, String)> {
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
    let mut out = Vec::new();
    let mut line = 1;
    for (i, part) in prose.split('`').enumerate() {
        if i % 2 == 1 {
            let span = part.split_whitespace().collect::<Vec<_>>().join(" ");
            let named = surface.keys.iter().any(|k| *k == span) || surface.presets.iter().any(|p| *p == span);
            if named && !WIRE_WORDS.contains(&span.as_str()) {
                out.push((line, span));
            }
        }
        line += part.matches('\n').count();
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

/// Whether a span names something `surface` has, ignoring where it sits.
pub fn known(surface: &Surface, span: &str) -> bool {
    surface.is_key(span) || surface.is_label(span) || surface.presets.iter().any(|p| p == span)
}

/// What is wrong with each span of `block` against `surface`: unknown
/// names, labels not bound to a key of theirs, and `{n}` patterns whose
/// declared range the plugin does not have. Each entry is
/// `(line, message)`.
pub fn check_block(surface: &Surface, block: &Block) -> Vec<(usize, String)> {
    let mut bad = Vec::new();
    for (i, span) in block.spans.iter().enumerate() {
        let text = span.text.as_str();
        if text.contains("{n}") {
            // The scope's own declaration, else the nearest earlier one.
            let range = block
                .ranges
                .iter()
                .find(|r| r.scope == span.scope)
                .or_else(|| block.ranges.iter().rev().find(|r| r.line <= span.line));
            let key_at = |n: u32| text.replace("{n}", &n.to_string());
            let has = |n: u32| surface.keys.iter().any(|k| *k == key_at(n));
            match range {
                Some(r) => {
                    if let Some(n) = (r.lo..=r.hi).find(|n| !has(*n)) {
                        bad.push((span.line, format!(
                            "`{text}`: the text says n = {}-{}, but there is no `{}`",
                            r.lo, r.hi, key_at(n)
                        )));
                    } else if has(r.hi + 1) {
                        bad.push((span.line, format!(
                            "`{text}`: the text says n = {}-{}, but `{}` exists too; fix the \
                             band count",
                            r.lo, r.hi, key_at(r.hi + 1)
                        )));
                    }
                }
                None if !surface.is_key(text) => {
                    bad.push((span.line, format!("`{text}`: no key fits this pattern")));
                }
                None => {}
            }
            continue;
        }
        if surface.is_key(text) || surface.presets.iter().any(|p| p == text) {
            continue;
        }
        if !surface.is_label(text) {
            bad.push((span.line, format!("`{text}`: not a key, a value label or a preset")));
            continue;
        }
        // A label: bind it to the nearest key before it in its scope, or
        // to a key in its column's header.
        let in_scope = block.spans[..i]
            .iter()
            .rev()
            .filter(|s| s.scope == span.scope)
            .find(|s| surface.is_key(&s.text));
        let from_header = || {
            let cell = span.cell.filter(|c| !c.header)?;
            block.spans.iter().rev().find(|s| {
                s.cell.is_some_and(|c| c.header && c.table == cell.table && c.column == cell.column)
                    && surface.is_key(&s.text)
            })
        };
        match in_scope.or_else(from_header) {
            None => bad.push((span.line, format!(
                "`{text}`: a value label with no key before it in its row, list item or \
                 paragraph; name the key it is a value of first"
            ))),
            Some(key) if !surface.labels_of(&key.text).any(|l| l == text) => {
                bad.push((span.line, format!(
                    "`{text}` follows `{}` but is not one of its values; name the key it \
                     belongs to right before it",
                    key.text
                )));
            }
            Some(_) => {}
        }
    }
    bad
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
        println!("{}\nkeys: {:?}\nchoices: {:?}\npresets: {:?}", surface.clap_id, surface.keys, surface.choices, surface.presets);
    }
    let mut files = Vec::new();
    markdown_files(&agent_plugin_dir(manifest_dir), &mut files);
    files.sort();
    let mut bad = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read skill markdown");
        let found = blocks(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        // The README quotes keys to explain the convention; only the
        // skills themselves are held to it.
        if file.starts_with(agent_plugin_dir(manifest_dir).join("skills")) {
            for (line, span) in loose_names(surface, &strip_blocks(&text)) {
                bad.push(format!(
                    "{}:{line}: `{span}` outside a keys block; wrap it in `{OPEN} {} -->`",
                    file.display(),
                    surface.clap_id
                ));
            }
        }
        for block in found.iter().filter(|b| b.plugin_id == surface.clap_id) {
            for (line, why) in check_block(surface, block) {
                bad.push(format!("{}:{line}: {why}", file.display()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "skills name things {} does not have. Inside a `{OPEN} {} -->` block every inline-code \
         span must be a param key, a stepped param's value label (after its key) or a factory \
         preset name of that plugin (use `{{n}}` for a band index, and `n = A-B` only for the \
         real band range). Fix the skill, or move prose that is not a key out of the block:\n{}",
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
                    let labels = (lo..=hi).map(|v| param.display(v as f64)).collect();
                    surface.choices.push((param.id().to_owned(), labels));
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
