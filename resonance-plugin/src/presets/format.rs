//! The preset file, format 1 (plugin-preset-library.md §4.3).
//!
//! ```json
//! { "format": "resonance.preset", "format_version": 1,
//!   "id": "3f0c9a4e-…",
//!   "plugin": { "id": "com.resonance.wavetable", "name": "…", "version": "0.4.0" },
//!   "meta": { "name": "Reese — Smooth", "category": "Bass", "tags": ["reese"], … },
//!   "state": { "encoding": "resonance-json", "doc": { "version": 1, "params": { … } } } }
//! ```
//!
//! A factory preset file is the same document, minus `plugin.version`
//! (filled in from `ResonancePlugin::VERSION` when it matters).
//!
//! `state.doc` is exactly the plugin's state document, so the state
//! loaders and the `ParamRename` migration apply to it unchanged. Anything
//! that accepts a state document also accepts a whole preset file:
//! [`unwrap_envelope`] is called from [`crate::state::migrate`] and from
//! [`super::load`].

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use resonance_common::library_marks::{normalize_tag, vocab};

/// `format` marker of a preset file.
pub const FORMAT: &str = "resonance.preset";
/// The `format_version` this build writes.
pub const FORMAT_VERSION: u32 = 1;
/// `state.encoding` of a first-party state document.
pub const ENCODING_RESONANCE_JSON: &str = "resonance-json";
/// `state.encoding` of an opaque base64 `clap.state` blob (third-party
/// plugins; plugin-preset-library.md §8, slice P7). Carried through
/// untouched by this build.
pub const ENCODING_CLAP_STATE: &str = "clap-state";

/// Descriptive metadata: the **content** half of a preset (§4.1). It
/// travels with the file; per-user marks (favourite, personal tags,
/// recents) never live here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetMeta {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instrument: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub character: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// RFC 3339 UTC, e.g. `2026-09-30T14:02:11Z`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// Id of the preset this one was saved from ("based on Tight Room").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
}

impl PresetMeta {
    /// A meta block carrying only a name.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The same metadata in its stored spelling: blank strings dropped,
    /// facet values slugged and de-duplicated (first spelling wins), and
    /// the category in its vocabulary case. Idempotent.
    pub fn normalized(mut self) -> Self {
        let blank_to_none = |s: Option<String>| {
            s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        };
        self.name = self.name.trim().to_string();
        self.author = blank_to_none(self.author);
        self.description = blank_to_none(self.description);
        self.category = self.category.as_deref().and_then(canonical_category);
        for list in [
            &mut self.instrument,
            &mut self.genres,
            &mut self.character,
            &mut self.tags,
        ] {
            let mut seen = Vec::with_capacity(list.len());
            for value in list.drain(..) {
                if let Some(v) = normalize_tag(&value) {
                    if !seen.contains(&v) {
                        seen.push(v);
                    }
                }
            }
            *list = seen;
        }
        self.created = blank_to_none(self.created);
        self.modified = blank_to_none(self.modified);
        self.derived_from = blank_to_none(self.derived_from);
        self
    }

    /// Copy the descriptive facets (category, instrument, genres,
    /// character, tags, author, description) from `other`, leaving name,
    /// timestamps and lineage alone. "Save as…" seeds a new preset this way.
    pub fn inherit_descriptive(&mut self, other: &PresetMeta) {
        self.author = other.author.clone();
        self.description = other.description.clone();
        self.category = other.category.clone();
        self.instrument = other.instrument.clone();
        self.genres = other.genres.clone();
        self.character = other.character.clone();
        self.tags = other.tags.clone();
    }
}

/// The vocabulary spelling of a category (`"bass"` → `"Bass"`), or the
/// trimmed input when it is not a seeded category. `None` for blank.
/// Categories keep their case (they are display labels, one per preset);
/// every other facet value is slugged by [`normalize_tag`].
pub fn canonical_category(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let seeded = vocab::CATEGORIES_INSTRUMENT
        .iter()
        .chain(vocab::CATEGORIES_EFFECT)
        .find(|c| c.eq_ignore_ascii_case(trimmed));
    Some(seeded.map(|c| c.to_string()).unwrap_or_else(|| trimmed.to_string()))
}

/// Which plugin a preset belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetPluginInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `ResonancePlugin::VERSION` at save, for display and warnings.
    /// `None` when unknown (legacy files, the host saving without a
    /// descriptor version).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The sound: a state document, or (third-party) an opaque blob.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetState {
    pub encoding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

impl PresetState {
    /// A third-party plugin's opaque `clap.state` (or state-context
    /// preset) bytes, base64 in the file (§8 tier T0).
    pub fn clap_blob(bytes: &[u8]) -> Self {
        use base64::Engine as _;
        Self {
            encoding: ENCODING_CLAP_STATE.to_string(),
            doc: None,
            blob: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        }
    }

    /// The blob's bytes; `None` for a document or undecodable base64.
    pub fn blob_bytes(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        if self.encoding != ENCODING_CLAP_STATE {
            return None;
        }
        base64::engine::general_purpose::STANDARD
            .decode(self.blob.as_deref()?)
            .ok()
    }

    /// A first-party state document.
    pub fn json(doc: serde_json::Value) -> Self {
        Self {
            encoding: ENCODING_RESONANCE_JSON.to_string(),
            doc: Some(doc),
            blob: None,
        }
    }
}

/// One preset file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    pub format: String,
    pub format_version: u32,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub plugin: PresetPluginInfo,
    #[serde(default)]
    pub meta: PresetMeta,
    pub state: PresetState,
}

impl PresetFile {
    pub fn new(
        id: impl Into<String>,
        plugin: PresetPluginInfo,
        meta: PresetMeta,
        doc: serde_json::Value,
    ) -> Self {
        Self {
            format: FORMAT.to_string(),
            format_version: FORMAT_VERSION,
            id: id.into(),
            plugin,
            meta,
            state: PresetState::json(doc),
        }
    }

    /// Parse a preset file. Refuses anything that is not a format-1
    /// envelope (a bare state document is a *legacy* file; see
    /// [`super::migrate`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        Self::from_value(value)
    }

    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        if !is_envelope(&value) {
            return Err("not a resonance.preset file".to_string());
        }
        if is_newer_format(&value) {
            return Err("written by a newer build".to_string());
        }
        let mut file: PresetFile =
            serde_json::from_value(value).map_err(|e| format!("malformed preset file: {e}"))?;
        file.meta = file.meta.normalized();
        Ok(file)
    }

    /// Pretty-printed, newline-terminated.
    pub fn to_text(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map(|mut s| {
                s.push('\n');
                s
            })
            .map_err(|e| format!("serialize preset: {e}"))
    }

    /// The state document as JSON text, for the loaders. `None` for a
    /// blob-encoded (third-party) preset.
    pub fn state_json(&self) -> Option<String> {
        self.state
            .doc
            .as_ref()
            .and_then(|doc| serde_json::to_string(doc).ok())
    }
}

/// Whether `value` is a preset file rather than a bare state document.
pub fn is_envelope(value: &serde_json::Value) -> bool {
    value.get("format").and_then(|f| f.as_str()) == Some(FORMAT)
}

/// Whether `value` is a preset file of a format version newer than this
/// build writes. Such a file is left alone rather than quarantined.
pub fn is_newer_format(value: &serde_json::Value) -> bool {
    value
        .get("format_version")
        .and_then(|v| v.as_u64())
        .is_some_and(|v| v > FORMAT_VERSION as u64)
}

/// Whether `id` has the hyphenated lowercase 8-4-4-4-12 hex shape of a
/// UUID.
pub fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

/// A UUID-shaped id derived deterministically from `parts` (version 8,
/// "custom"), so two processes converting the same legacy file mint the
/// same id and the same file name instead of two copies. FNV-1a over the
/// parts, twice with different offsets for 128 bits: stable across
/// builds and platforms, which std's hasher is not.
pub fn derived_uuid(parts: &[&[u8]]) -> String {
    let fnv = |offset: u64| {
        let mut h = offset;
        for part in parts {
            for b in part.iter().chain(&[0xff]) {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    };
    let mut bits =
        ((fnv(0xcbf2_9ce4_8422_2325) as u128) << 64) | fnv(0x6c62_272e_07bb_0142) as u128;
    bits = (bits & !(0xf << 76)) | (0x8 << 76);
    bits = (bits & !(0x3 << 62)) | (0x2 << 62);
    uuid_text(bits)
}

fn uuid_text(bits: u128) -> String {
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Replace a preset file with the state document it carries, in place.
/// A bare state document (or a blob-encoded preset, which has no
/// document) is left as it is.
pub fn unwrap_envelope(value: &mut serde_json::Value) {
    if !is_envelope(value) {
        return;
    }
    if let Some(doc) = value
        .get_mut("state")
        .and_then(|s| s.get_mut("doc"))
        .map(serde_json::Value::take)
    {
        *value = doc;
    }
}

/// The state document inside `text`, whether `text` is a preset file or
/// already a bare state document. `None` when it is not JSON.
pub fn state_document(text: &str) -> Option<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    unwrap_envelope(&mut value);
    Some(value)
}

// ---------------------------------------------------------------------------
// Ids and timestamps
// ---------------------------------------------------------------------------

/// A fresh UUIDv4 in its hyphenated lowercase spelling.
///
/// Built from std's randomly keyed `RandomState` hasher (OS entropy per
/// process, a fresh key per call) over the clock, the pid and a counter,
/// so the crate needs no RNG dependency. Ids only have to be unique in
/// one user's library; they are not secrets.
pub fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut bits: u128 = 0;
    for half in 0u8..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u64(count);
        h.write_u32(std::process::id());
        h.write_u8(half);
        bits = (bits << 64) | h.finish() as u128;
    }
    // Version 4, RFC 4122 variant.
    bits = (bits & !(0xf << 76)) | (0x4 << 76);
    bits = (bits & !(0x3 << 62)) | (0x2 << 62);
    uuid_text(bits)
}

/// `t` as RFC 3339 UTC with second precision (`2026-09-30T14:02:11Z`).
pub fn rfc3339(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
