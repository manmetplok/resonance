//! `amp_models.*` — the per-user NAM model library (nam-model-library.md
//! §9.3).
//!
//! Answered above the mutation gate: the library is a fact about the
//! user's machine, not about the open project, so it needs no project,
//! records no undo entry and never bumps `revision`. The app reads the same
//! files the amp plugin does (`resonance_common::nam_library` and the
//! shared `library_marks` store), so no amp instance has to be running.
//! The rows and the search are the plugin's own
//! (`resonance_plugin::nam_rows` over the shared `BrowserModel`), so a
//! query means the same here as in the amp's Library panel.
//!
//! The roots come from the app ([`AmpLibraryRoots`]): the user's data dir
//! in the real app, a private temporary one in every `new_for_test*` app.
//! The library is opened (and scanned) once and kept; a request re-reads
//! the index only when it changed on disk (one `stat`) and answers from
//! it, while the rescan that notices a moved file runs off the update
//! loop (`state::library_cache::OffThreadRescan`) — so a model added since shows up in a later
//! answer, and a name lookup that misses rescans before refusing.

use resonance_common::nam_library::{self, Entry, EntryStatus, Library, Source};
use resonance_control::methods::amp_models::{
    self, AmpModelEntry, AmpModelList, AmpModelSource, AmpModelStatus,
};
use resonance_control::{Request, Response, RpcError};
use resonance_plugin::library_view::BrowserModel;
use resonance_plugin::nam_rows::ModelRows;

use super::reply::{failure, success};
use crate::Resonance;

/// The library handle and its roots live in `state::library_cache`
/// (`ControlEndpointState` holds the cache — ARCH2-05); this module stays
/// their import path.
pub use crate::state::library_cache::{AmpLibraryCache, AmpLibraryRoots};

/// Handle an `amp_models.*` request, or `None` for another namespace.
pub(super) fn try_handle(app: &mut Resonance, request: &Request) -> Option<Response> {
    match request.method.as_str() {
        amp_models::LIST => Some(list(app, request)),
        amp_models::SET_MARKS => Some(set_marks(app, request)),
        _ => None,
    }
}

/// The wire form of one entry with its marks.
pub(super) fn wire_entry(
    e: &Entry,
    marks: Option<&resonance_common::library_marks::Marks>,
) -> AmpModelEntry {
    let (status, error) = match &e.status {
        EntryStatus::Ok => (AmpModelStatus::Ok, None),
        EntryStatus::Unreadable(reason) => (AmpModelStatus::Unreadable, Some(reason.clone())),
        EntryStatus::DuplicateOf(_) => (AmpModelStatus::Duplicate, None),
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
        last_used: marks.and_then(|m| m.last_used_rfc3339()),
        status,
        error,
    }
}

/// `amp_models.list`.
fn list(app: &mut Resonance, request: &Request) -> Response {
    let params: amp_models::ListParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let cache = &mut app.control.amp_library;
    let snapshot = cache.marks().snapshot();
    let lib = cache.library();
    let rows = ModelRows::build(lib, Some(&snapshot), (lib.generation(), snapshot.generation()));

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

    let matched = view.view_len();
    let limit = params.limit.unwrap_or(usize::MAX);
    let models = view
        .view()
        .iter()
        .take(limit)
        .map(|&r| wire_entry(&rows.rows[r].entry, rows.marks_of(r)))
        .collect();
    let result = AmpModelList {
        models,
        matched,
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
fn set_marks(app: &mut Resonance, request: &Request) -> Response {
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
    let cache = &mut app.control.amp_library;
    let entry = match find_entry(cache.library(), &params.id) {
        Ok(e) => e.clone(),
        Err(e) => return failure(request, e),
    };
    let tags = params
        .tags
        .as_ref()
        .map(|t| resonance_common::library_marks::normalize_tags(t));
    let updated = cache.marks().update(&entry.mark_key(), |m| {
        if let Some(f) = params.favorite {
            m.favorite = f;
        }
        if let Some(t) = &tags {
            m.tags = t.clone();
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

/// Resolve a model name (or id prefix) to its `Model Select` slot, for a
/// label sent to Resonance Amp's selector: answered app-side from the
/// library, with no round trip through the plugin. `Err` names why.
pub(crate) fn slot_for_label(app: &mut Resonance, text: &str) -> Result<u32, String> {
    let cache = &mut app.control.amp_library;
    let found = cache.library().find(text).and_then(|e| e.slot);
    // A miss may be a model added since the last index: look again,
    // rescanned now, before refusing.
    let found = found.or_else(|| cache.library_rescanned().find(text).and_then(|e| e.slot));
    match found {
        Some(slot) => Ok(slot),
        None => Err(format!(
            "no single installed amp model is named {text:?} (an exact name shared by two \
             models is ambiguous) — send its slot or id from amp_models_list"
        )),
    }
}

/// The app's roots: the user's data dir for the real app, a private
/// directory under the test process's hermetic root otherwise.
pub(crate) fn roots_for(hermetic: bool) -> AmpLibraryRoots {
    if !hermetic {
        return AmpLibraryRoots {
            models: nam_library::default_root(),
            marks: resonance_common::library_marks::default_library_dir(),
        };
    }
    let base = crate::user_dirs::hermetic_subdir("amp-library");
    AmpLibraryRoots {
        models: Some(base.join("models")),
        marks: Some(base.join("marks")),
    }
}
