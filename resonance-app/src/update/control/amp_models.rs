//! `amp_models.*` — the per-user NAM model library (nam-model-library.md
//! §9.3).
//!
//! Answered above the mutation gate: the library is a fact about the
//! user's machine, not about the open project, so it needs no project,
//! records no undo entry and never bumps `revision`. The app reads the same
//! files the amp plugin does (`resonance_common::nam_library` and the
//! shared `library_marks` store), so no amp instance has to be running.
//! The search is the plugin's own: the shared `library_view::BrowserModel`
//! over the same rows.

use resonance_common::library_marks::{Marks, MarksStore};
use resonance_common::nam_library::{self, Entry, EntryStatus, Library, Source};
use resonance_control::methods::amp_models::{self, AmpModelEntry, AmpModelList, AmpModelSource};
use resonance_control::{Request, Response, RpcError};
use resonance_plugin::library_view::{BrowserModel, LibraryRows};

use super::reply::{failure, success};
use crate::Resonance;

/// Handle an `amp_models.*` request, or `None` for another namespace.
pub(super) fn try_handle(_app: &mut Resonance, request: &Request) -> Option<Response> {
    match request.method.as_str() {
        amp_models::LIST => Some(list(request)),
        amp_models::SET_MARKS => Some(set_marks(request)),
        _ => None,
    }
}

/// The library at its root (`RESONANCE_AMP_MODEL_DIR` or the data dir),
/// rescanned so files added since the last scan — by the amp, a file
/// manager or another process — are listed. Only new or changed files are
/// hashed.
fn open_library() -> Library {
    let Some(root) = nam_library::default_root() else {
        return Library::empty();
    };
    let mut lib = Library::open(root);
    if let Err(e) = lib.rescan() {
        tracing::warn!("amp_models: library rescan failed: {e}");
    }
    lib
}

/// The shared marks store (`RESONANCE_LIBRARY_DIR` or the data dir).
fn open_marks() -> MarksStore {
    MarksStore::open_default().unwrap_or_else(|e| {
        tracing::warn!("amp_models: marks unavailable: {e}");
        MarksStore::detached()
    })
}

/// The library as browser rows (the amp's `library_rows` shape: the same
/// keys, search text and facets, so a query means the same thing here as
/// in the plugin's Library panel).
struct Rows<'a> {
    entries: &'a [Entry],
    keys: Vec<String>,
    marks: Vec<Option<Marks>>,
}

impl<'a> Rows<'a> {
    fn new(lib: &'a Library, store: &MarksStore) -> Self {
        let entries = lib.entries();
        let keys = entries
            .iter()
            .map(|e| match &e.status {
                EntryStatus::DuplicateOf(_) => {
                    format!("{}#{}", nam_library::mark_key(&e.id), e.path.display())
                }
                _ => nam_library::mark_key(&e.id),
            })
            .collect();
        let marks = entries
            .iter()
            .map(|e| store.get(&nam_library::mark_key(&e.id)).cloned())
            .collect();
        Self {
            entries,
            keys,
            marks,
        }
    }
}

impl LibraryRows for Rows<'_> {
    fn row_count(&self) -> usize {
        self.entries.len()
    }
    fn key(&self, row: usize) -> &str {
        &self.keys[row]
    }
    fn title(&self, row: usize) -> &str {
        &self.entries[row].name
    }
    fn marks(&self, row: usize) -> Option<&Marks> {
        self.marks[row].as_ref()
    }
    fn search_text(&self, row: usize) -> Vec<&str> {
        let e = &self.entries[row];
        let mut out = vec![e.file_name.as_str()];
        for s in [&e.author, &e.gear, &e.gear_type, &e.tone_type].into_iter().flatten() {
            out.push(s.as_str());
        }
        out
    }
    fn facet_names(&self) -> Vec<&str> {
        vec!["gear_type", "tone_type", "author"]
    }
    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        let e = &self.entries[row];
        match facet {
            "gear_type" => e.gear_type.as_deref().into_iter().collect(),
            "tone_type" => e.tone_type.as_deref().into_iter().collect(),
            "author" => e.author.as_deref().into_iter().collect(),
            "source" => vec![e.source.label()],
            _ => Vec::new(),
        }
    }
}

/// The wire form of one entry with its marks.
pub(super) fn wire_entry(e: &Entry, marks: Option<&Marks>) -> AmpModelEntry {
    let (status, error) = match &e.status {
        EntryStatus::Ok => ("ok", None),
        EntryStatus::Unreadable(reason) => ("unreadable", Some(reason.clone())),
        EntryStatus::DuplicateOf(_) => ("duplicate", None),
    };
    AmpModelEntry {
        slot: e.slot,
        id: e.id.clone(),
        name: e.name.clone(),
        author: e.author.clone(),
        gear: e.gear.clone(),
        gear_type: e.gear_type.clone(),
        tone_type: e.tone_type.clone(),
        architecture: e.architecture.clone(),
        sample_rate: e.sample_rate,
        size_bytes: e.size_bytes,
        source: match e.source {
            Source::Tone3000 { tone_id, model_id } => AmpModelSource::Tone3000 { tone_id, model_id },
            Source::Imported => AmpModelSource::Imported,
            Source::External => AmpModelSource::External,
        },
        favorite: marks.is_some_and(|m| m.favorite),
        tags: marks.map(|m| m.tags.clone()).unwrap_or_default(),
        last_used: marks.and_then(Marks::last_used_rfc3339),
        status: status.to_string(),
        error,
    }
}

/// `amp_models.list`.
fn list(request: &Request) -> Response {
    let params: amp_models::ListParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let lib = open_library();
    let store = open_marks();
    let rows = Rows::new(&lib, &store);

    let mut view = BrowserModel::new();
    view.set_query(params.query.unwrap_or_default());
    view.set_favorites_only(params.favorites_only);
    if let Some(g) = params.gear_type.as_deref() {
        view.set_facet("gear_type", Some(g));
    }
    if let Some(t) = params.tone_type.as_deref() {
        view.set_facet("tone_type", Some(t));
    }
    view.refresh(&rows, 1);

    let models = view
        .view()
        .iter()
        .map(|&r| wire_entry(&rows.entries[r], rows.marks[r].as_ref()))
        .collect();
    let result = AmpModelList {
        models,
        library_generation: lib.generation(),
        total: lib.len(),
    };
    success(request, &result)
}

/// The canonical entry of `id`: an exact content id, or a unique prefix of
/// at least 8 hex digits.
fn find_entry<'a>(lib: &'a Library, id: &str) -> Result<&'a Entry, RpcError> {
    let id = id.trim().to_ascii_lowercase();
    if let Some(e) = lib.entry(&id) {
        return Ok(e);
    }
    if id.len() >= 8 {
        let mut hits = lib
            .entries()
            .iter()
            .filter(|e| !matches!(e.status, EntryStatus::DuplicateOf(_)) && e.id.starts_with(&id));
        match (hits.next(), hits.next()) {
            (Some(e), None) => return Ok(e),
            (Some(_), Some(_)) => {
                return Err(RpcError::invalid_params(format!(
                    "id prefix {id:?} matches more than one model; send more of the id"
                )))
            }
            _ => {}
        }
    }
    Err(RpcError::not_found(format!(
        "no installed amp model with id {id:?}; amp_models_list reports the ids"
    )))
}

/// `amp_models.set_marks` — star and/or re-tag one model in the shared
/// marks store (the same store, key and lock the amp's Library panel
/// uses). Per-user state: no undo entry, no `revision` bump.
fn set_marks(request: &Request) -> Response {
    let params: amp_models::SetMarksParams = match request.params() {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    if params.favorite.is_none() && params.tags.is_none() {
        return failure(
            request,
            RpcError::invalid_params("nothing to set: pass favorite and/or tags"),
        );
    }
    let lib = open_library();
    let entry = match find_entry(&lib, &params.id) {
        Ok(e) => e.clone(),
        Err(e) => return failure(request, e),
    };
    let mut store = open_marks();
    let tags = params
        .tags
        .as_ref()
        .map(|t| resonance_common::library_marks::normalize_tags(t));
    let updated = store.update(&entry.mark_key(), |m| {
        if let Some(f) = params.favorite {
            m.favorite = f;
        }
        if let Some(t) = tags {
            m.tags = t;
        }
    });
    match updated {
        Ok(marks) => success(request, &wire_entry(&entry, Some(&marks))),
        Err(e) => failure(
            request,
            RpcError::internal(format!("could not save the marks: {e}")),
        ),
    }
}
