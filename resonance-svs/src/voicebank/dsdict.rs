//! OpenUtau DiffSinger dictionaries (`dsdict*.yaml`): the word → phoneme
//! table a bank ships for one language, which OpenUtau's phonemizer
//! consults before its own G2P.
//!
//! The format is a YAML document with an `entries:` list of
//! `{grapheme, phonemes}` maps, next to `symbols:` (each phoneme's class)
//! and sometimes `replacements:`, which are ignored here. Banks write it
//! by hand, so the reader is tolerant: an entry whose grapheme YAML reads
//! as a number or boolean (`no`, `1`) is kept as text, and an entry that is
//! not a map with a grapheme and a phoneme list is skipped rather than
//! failing the whole file.

use std::path::Path;

use anyhow::{Context, Result};
use serde_yaml_ng::Value;

/// One dictionary row: a spelling and the bank phonemes to sing for it,
/// exactly as written (language prefixes included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DsDictEntry {
    pub grapheme: String,
    pub phonemes: Vec<String>,
}

/// Read a `dsdict*.yaml`'s entries. Errors only when the file cannot be
/// read or is not YAML; a file without `entries:` yields none.
pub fn load_dsdict(path: &Path) -> Result<Vec<DsDictEntry>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading dictionary {}", path.display()))?;
    parse_dsdict(&text).with_context(|| format!("parsing dictionary {}", path.display()))
}

/// [`load_dsdict`] on text already in memory.
pub fn parse_dsdict(text: &str) -> Result<Vec<DsDictEntry>> {
    let doc: Value = serde_yaml_ng::from_str(text)?;
    let Some(entries) = doc.get("entries").and_then(Value::as_sequence) else {
        return Ok(Vec::new());
    };
    Ok(entries.iter().filter_map(entry).collect())
}

fn entry(value: &Value) -> Option<DsDictEntry> {
    let grapheme = scalar_text(value.get("grapheme")?)?;
    let phonemes: Vec<String> = value
        .get("phonemes")?
        .as_sequence()?
        .iter()
        .filter_map(scalar_text)
        .collect();
    (!grapheme.is_empty() && !phonemes.is_empty()).then_some(DsDictEntry { grapheme, phonemes })
}

/// A YAML scalar as the text it was written as.
fn scalar_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Some(text.trim().to_owned())
}
