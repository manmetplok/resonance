//! Which model an amp instance refers to, what it shows, and how a saved
//! reference resolves against the library (nam-model-library.md §5.2).
//!
//! The reference is plugin state v2:
//!
//! ```json
//! { "model_path": "/…/tone3000/Friedman_BE100_(standard)_48121.nam",
//!   "model_id": "9f2c…",
//!   "model_name": "Friedman BE-100 · standard",
//!   "model_source": { "tone3000": { "tone_id": 1934, "model_id": 48121 } } }
//! ```
//!
//! `model_path` is the v1 key and is always written, so old projects load
//! unchanged; the other keys are optional on load and written only when
//! known. A reference that cannot be resolved is kept **verbatim** (the
//! source is held as the raw JSON it was read as), so re-saving a project
//! with a missing model writes back exactly what it read.

use std::path::{Path, PathBuf};

use resonance_common::nam_library::{self, Entry, Library, Source};
use serde_json::{Map, Value};

/// The saved model reference.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelRef {
    /// Absolute path; empty for "no model".
    pub path: String,
    /// Content id (sha256), when known.
    pub id: Option<String>,
    /// Display name, when known.
    pub name: Option<String>,
    /// `model_source` exactly as read or built.
    pub source: Option<Value>,
}

impl ModelRef {
    /// The reference to a library entry.
    pub fn from_entry(entry: &Entry) -> Self {
        Self {
            path: entry.path.to_string_lossy().into_owned(),
            id: Some(entry.id.clone()),
            name: Some(entry.name.clone()),
            source: serde_json::to_value(&entry.source).ok(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty() && self.id.is_none()
    }

    /// `model_source` parsed, if it is one this build knows.
    pub fn parsed_source(&self) -> Option<Source> {
        self.source
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// The best name to show: the saved name, else the file stem.
    pub fn display_name(&self) -> String {
        if let Some(n) = self.name.as_deref().filter(|n| !n.is_empty()) {
            return n.to_string();
        }
        Path::new(&self.path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Write the reference into a state map.
    pub fn save_into(&self, map: &mut Map<String, Value>) {
        map.insert("model_path".into(), Value::String(self.path.clone()));
        if let Some(id) = &self.id {
            map.insert("model_id".into(), Value::String(id.clone()));
        }
        if let Some(name) = &self.name {
            map.insert("model_name".into(), Value::String(name.clone()));
        }
        if let Some(source) = &self.source {
            map.insert("model_source".into(), source.clone());
        }
    }

    /// Read a reference from a state document. `None` when the document
    /// has no `model_path` at all (the caller then keeps what it has, as
    /// before v2).
    pub fn load_from(state: &Value) -> Option<Self> {
        let path = state.get("model_path")?.as_str()?.to_string();
        let text = |k: &str| {
            state
                .get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        Some(Self {
            path,
            id: text("model_id"),
            name: text("model_name"),
            source: state.get("model_source").cloned(),
        })
    }
}

/// How a saved reference resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// Nothing saved.
    Nothing,
    /// Load `path`. `entry` is its library entry (by path, else by content
    /// id), `None` for a file the library does not know (external).
    Load { path: PathBuf, entry: Option<Entry> },
    /// The file had moved (or its bytes changed) and the library has the
    /// saved id elsewhere: load that and rewrite the reference.
    Relinked { entry: Entry },
    /// Nothing to load. The reference stays as it is.
    Missing {
        name: String,
        path: String,
        source: Option<Source>,
        /// The path exists but holds different bytes than the saved id.
        file_changed: bool,
    },
}

/// Resolve a saved reference against the library (a pure function over
/// the file system and `library`; `hash` is how a file's content id is
/// computed, injectable for tests).
///
/// 1. `path` exists and its id matches `id` (or there is no `id`) → load it.
/// 2. The path is missing (or its bytes differ) and the library has an
///    entry with `id` → load that entry and rewrite the reference.
/// 3. Otherwise → missing, the reference kept verbatim.
pub fn resolve_model(
    reference: &ModelRef,
    library: &Library,
    hash: impl Fn(&Path) -> Option<String>,
) -> Resolved {
    if reference.is_empty() {
        return Resolved::Nothing;
    }
    let path = PathBuf::from(&reference.path);
    let exists = !reference.path.is_empty() && path.is_file();
    let mut file_changed = false;
    if exists {
        // The index's cached id is current when the file has not changed
        // since the last scan; only a file it does not know is hashed.
        let file_id = library
            .by_path(&path)
            .filter(|e| file_unchanged(e, &path))
            .map(|e| e.id.clone())
            .or_else(|| hash(&path));
        match (&reference.id, &file_id) {
            (Some(want), Some(have)) if want != have => file_changed = true,
            _ => {
                let entry = library
                    .by_path(&path)
                    .filter(|e| !matches!(e.status, nam_library::EntryStatus::DuplicateOf(_)))
                    .cloned()
                    .or_else(|| file_id.as_deref().and_then(|id| library.entry(id)).cloned());
                return Resolved::Load { path, entry };
            }
        }
    }
    if let Some(entry) = reference
        .id
        .as_deref()
        .and_then(|id| library.entry(id))
        .filter(|e| e.path.is_file())
    {
        return Resolved::Relinked {
            entry: entry.clone(),
        };
    }
    Resolved::Missing {
        name: reference.display_name(),
        path: reference.path.clone(),
        source: reference.parsed_source(),
        file_changed,
    }
}

fn file_unchanged(entry: &Entry, path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| {
        m.len() == entry.size_bytes
            && m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|d| d.as_secs() as i64 == entry.mtime)
    })
}

/// What the header and the control API show about the model.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ModelState {
    /// No model loaded and none referenced.
    #[default]
    None,
    /// A model is loaded and playing.
    Loaded,
    /// The saved model could not be found (§6.4). Nothing plays.
    Missing {
        name: String,
        path: String,
        source: Option<Source>,
        file_changed: bool,
        /// The `file_select` value when it went missing, so the parameter's
        /// text can say "Missing: …" for it.
        at_slot: i32,
    },
    /// `file_select` points at a slot with no model; whatever was playing
    /// keeps playing.
    EmptySlot(u32),
    /// The last load failed; whatever was playing keeps playing.
    Error(String),
}

/// The instance's model display state, shared between the loader thread,
/// the editor and the parameter's text conversion.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelStatus {
    /// What is playing, "" for nothing.
    pub name: String,
    /// The content id of what is playing.
    pub id: Option<String>,
    pub state: ModelState,
    /// The playing model is not in the library.
    pub external: bool,
    /// The playing model's file was deleted from the library.
    pub deleted: bool,
    /// A one-line notice ("Relinked: … (file had moved)").
    pub notice: Option<String>,
}

impl ModelStatus {
    pub fn is_missing(&self) -> bool {
        matches!(self.state, ModelState::Missing { .. })
    }

    /// The header's text for the model name.
    pub fn header_text(&self) -> String {
        match &self.state {
            ModelState::Missing { name, .. } => format!("Missing: {name}"),
            _ if self.name.is_empty() => "(no model loaded)".to_string(),
            _ if self.deleted => format!("{} (deleted)", self.name),
            _ => self.name.clone(),
        }
    }
}
