//! `drum_kits.*` — the per-user drum-kit library (drums-plugin-rework.md
//! §8, slice K9). The twin of [`super::amp_models`].
//!
//! Answered above the mutation gate: the library is a fact about the
//! user's machine, not about the open project, so it needs no project,
//! records no undo entry and never bumps `revision`. The app reads the
//! same files the drums plugin does (`resonance_common::drumkit_library`
//! and the shared `library_marks` store), so no drums instance has to be
//! running. The rows and the search are the plugin's own
//! (`resonance_plugin::kit_rows` over the shared `BrowserModel`), so a
//! query means the same here as in the drums' Library overlay.
//!
//! The app never migrates `installed.json`, imports, deletes or downloads:
//! those stay the plugin's (D6). It opens and scans the index on first
//! use, then answers from the last index and rescans off the update loop
//! (hashing only a kit whose manifest moved), so a kit added since shows
//! up in a later answer.

use resonance_common::drumkit_library::{self, Entry, EntryStatus, Library, Source};
use resonance_common::library_marks::Marks;
use resonance_control::ids::TrackId;
use resonance_control::methods::drum_kits::{
    self, DrumKitEntry, DrumKitList, DrumKitSource, DrumKitStatus, DrumKitUser,
};
use resonance_control::{Request, Response, RpcError};
use resonance_plugin::kit_rows::KitRows;
use resonance_plugin::library_view::{BrowserModel, SOURCE_FACET};

use super::reply::{failure, success};
use crate::Resonance;

/// The library handle and its roots live in `state::library_cache`
/// (`ControlEndpointState` holds the cache — ARCH2-05); this module stays
/// their import path.
pub use crate::state::library_cache::{DrumKitLibraryCache, DrumKitLibraryRoots};

/// Handle a `drum_kits.*` request, or `None` for another namespace.
pub(super) fn try_handle(app: &mut Resonance, request: &Request) -> Option<Response> {
    match request.method.as_str() {
        drum_kits::LIST => Some(list(app, request)),
        drum_kits::SET_MARKS => Some(set_marks(app, request)),
        _ => None,
    }
}

fn wire_source(source: Source) -> DrumKitSource {
    match source {
        Source::Plok => DrumKitSource::Plok,
        Source::Imported => DrumKitSource::Imported,
        Source::Local => DrumKitSource::Local,
    }
}

/// The tracks whose Resonance Drums has library slot `slot` selected.
fn users_of(app: &Resonance, slot: Option<u32>) -> Vec<DrumKitUser> {
    let Some(slot) = slot else {
        return Vec::new();
    };
    let mut tracks: Vec<_> = app
        .registry
        .tracks
        .iter()
        .filter(|t| {
            t.plugins
                .iter()
                .any(|s| crate::drums_mirror::kit_slot(s) == Some(slot))
        })
        .collect();
    tracks.sort_by_key(|t| t.order);
    tracks
        .into_iter()
        .map(|t| DrumKitUser {
            track_id: TrackId(t.id),
            track_name: t.name.clone(),
        })
        .collect()
}

/// The wire form of one entry with its marks.
fn wire_entry(app: &Resonance, e: &Entry, marks: Option<&Marks>) -> DrumKitEntry {
    let (status, error) = match &e.status {
        EntryStatus::Ok => (DrumKitStatus::Ok, None),
        EntryStatus::ManifestError(reason) => (DrumKitStatus::ManifestError, Some(reason.clone())),
        EntryStatus::MissingFiles(n) => (
            DrumKitStatus::MissingFiles,
            Some(format!("{n} sample file(s) missing")),
        ),
        EntryStatus::DuplicateOf(dir) => (
            DrumKitStatus::Duplicate,
            Some(format!("duplicate of {}", dir.display())),
        ),
    };
    DrumKitEntry {
        slot: e.slot,
        id: e.id.clone(),
        name: e.name.clone(),
        description: e.description().map(str::to_string),
        pieces: e.pieces.len(),
        piece_names: e.pieces.iter().map(|p| p.name.clone()).collect(),
        mic_setups: e.mic_setups.len(),
        layers: e.layers_max,
        rr: e.rr_max,
        size_bytes: e.size_bytes,
        source: wire_source(e.source),
        favorite: marks.is_some_and(|m| m.favorite),
        tags: marks.map(|m| m.tags.clone()).unwrap_or_default(),
        last_used: marks.and_then(|m| m.last_used_rfc3339()),
        status,
        error,
        loaded_in: users_of(app, e.slot),
    }
}

/// `drum_kits.list`.
fn list(app: &mut Resonance, request: &Request) -> Response {
    let params: drum_kits::ListParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let cache = &mut app.control.drum_kit_library;
    let snapshot = cache.marks().snapshot();
    let lib = cache.library();
    let rows = KitRows::build(
        lib,
        Some(&snapshot),
        (lib.generation(), snapshot.generation()),
    );
    let library_generation = lib.generation();
    let total = lib.len();

    let mut view = BrowserModel::new();
    view.set_query(params.query.unwrap_or_default());
    view.set_favorites_only(params.favorites_only);
    if let Some(source) = params.source {
        let label = match source {
            DrumKitSource::Plok => Source::Plok,
            DrumKitSource::Imported => Source::Imported,
            DrumKitSource::Local => Source::Local,
        }
        .label();
        view.set_facet(SOURCE_FACET, Some(label));
    }
    view.refresh(&rows, 1);

    let matched = view.view_len();
    let limit = params.limit.unwrap_or(usize::MAX);
    let kits = view
        .view()
        .iter()
        .take(limit)
        .map(|&r| wire_entry(app, &rows.rows[r].entry, rows.marks_of(r)))
        .collect();
    success(
        request,
        &DrumKitList {
            kits,
            matched,
            library_generation,
            total,
        },
    )
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
                    "id prefix {id:?} matches more than one kit; send more of the id"
                )))
            }
            _ => {}
        }
    }
    Err(RpcError::not_found(format!(
        "no installed drum kit with id {id:?}; drum_kits_list reports the ids"
    )))
}

/// `drum_kits.set_marks` — star and/or re-tag one kit in the shared marks
/// store (the same store, key and lock the drums' Library overlay uses).
/// Per-user state: no undo entry, no `revision` bump.
fn set_marks(app: &mut Resonance, request: &Request) -> Response {
    let params: drum_kits::SetMarksParams = match request.params() {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    if params.favorite.is_none() && params.tags.is_none() {
        return failure(
            request,
            RpcError::invalid_params("nothing to set: pass favorite and/or tags"),
        );
    }
    let cache = &mut app.control.drum_kit_library;
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
        Ok(marks) => success(request, &wire_entry(app, &entry, Some(&marks))),
        Err(e) => failure(
            request,
            RpcError::internal(format!("could not save the marks: {e}")),
        ),
    }
}

/// The app's roots: the user's data dir for the real app, a private
/// directory under the test process's hermetic root otherwise.
pub(crate) fn roots_for(hermetic: bool) -> DrumKitLibraryRoots {
    if !hermetic {
        return DrumKitLibraryRoots {
            kits: drumkit_library::default_root(),
            marks: resonance_common::library_marks::default_library_dir(),
        };
    }
    let base = crate::user_dirs::hermetic_subdir("drumkit-library");
    DrumKitLibraryRoots {
        kits: Some(base.join("kits")),
        marks: Some(base.join("marks")),
    }
}
