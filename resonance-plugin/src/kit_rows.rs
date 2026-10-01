//! The drum-kit library as browser rows: one [`LibraryRows`] adapter over
//! a snapshot of `resonance_common::drumkit_library` entries and their
//! marks, the twin of [`crate::nam_rows`]. The drums' Library overlay and
//! the app's kit picker / control methods build the same rows, so a query
//! means the same thing on every surface (drums-plugin-rework.md §3.4).
//! Plus the ◀/▶ and counter rules over a view of them.
//!
//! Not feature-gated: the app (no `editor-widgets`) builds the same rows.

use std::borrow::Cow;
use std::collections::HashMap;

use resonance_common::drumkit_library::{self, format_bytes, Entry, EntryStatus, Library};
use resonance_common::library_marks::{Marks, MarksStore};

use crate::library_view::{
    BrowserModel, LibraryRows, SortKey, SortValue, SOURCE_FACET, TAGS_FACET,
};

/// The mic-count facet: how many mic setups a kit has, bucketed.
pub const MICS_FACET: &str = "mics";

/// The articulation facet: `yes` when the kit declares articulation pairs.
pub const ARTICULATIONS_FACET: &str = "articulations";

/// The facets the Installed tab offers, as `(facet, label)`.
pub const FACETS: &[(&str, &str)] = &[
    (SOURCE_FACET, "Source"),
    (MICS_FACET, "Mics"),
    (ARTICULATIONS_FACET, "Articulations"),
];

/// The display columns after the title, in order, as their headers.
pub const COLUMNS: &[&str] = &["Pieces", "Mics", "Layers", "RR", "Source", "Size"];

/// The sorts the Installed tab offers, as `(label, key)`. Favourites
/// always sort first within any of them.
pub fn sort_options() -> Vec<(&'static str, SortKey)> {
    vec![
        ("Slot", SortKey::Natural),
        ("Name", SortKey::Title),
        ("Recently used", SortKey::RecentlyUsed),
        ("Recently added", SortKey::Field("added".into())),
        ("Size", SortKey::Field("size".into())),
        ("Pieces", SortKey::Field("pieces".into())),
        ("Mics", SortKey::Field("mics".into())),
    ]
}

/// The [`MICS_FACET`] bucket of a mic-setup count: `1`, `2–4` or `5+`
/// (`0` for a kit whose manifest did not parse).
pub fn mics_bucket(setups: usize) -> &'static str {
    match setups {
        0 => "0",
        1 => "1",
        2..=4 => "2–4",
        _ => "5+",
    }
}

/// "35 pieces", "1 setup": a count with its noun.
fn count(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One row: an entry, its unique row key, and its display text.
#[derive(Debug, Clone)]
pub struct KitRow {
    pub entry: Entry,
    /// `drumkit:<id>`, or with a `#<dir>` suffix for a duplicate kit (row
    /// keys must be unique; marks are always looked up by id).
    pub key: String,
    pub mark_key: String,
    columns: [String; 6],
    mics_bucket: &'static str,
    /// Piece names, mic brands/models/positions, description: what a plain
    /// search token matches besides the title and tags.
    search: Vec<String>,
}

/// A snapshot of the library as rows, plus the marks of its kits.
#[derive(Debug, Clone, Default)]
pub struct KitRows {
    pub rows: Vec<KitRow>,
    marks: HashMap<String, Marks>,
    /// What this was built from (library revision, marks generation), for
    /// the caller's staleness check.
    pub built_from: (u64, u64),
}

impl KitRows {
    /// Build rows from the library and (optionally) a marks snapshot.
    pub fn build(library: &Library, marks: Option<&MarksStore>, built_from: (u64, u64)) -> Self {
        let mut rows = Vec::with_capacity(library.len());
        let mut mark_map = HashMap::new();
        for e in library.entries() {
            let mark_key = drumkit_library::mark_key(&e.id);
            let key = match &e.status {
                EntryStatus::DuplicateOf(_) => format!("{mark_key}#{}", e.dir.display()),
                _ => mark_key.clone(),
            };
            if let Some(m) = marks.and_then(|s| s.get(&mark_key)) {
                mark_map.insert(mark_key.clone(), m.clone());
            }
            let size = e.size_bytes.map(format_bytes).unwrap_or_else(|| "…".into());
            let columns = match &e.status {
                // A broken manifest says why in its row, where the counts
                // would be zeros.
                EntryStatus::ManifestError(reason) => [
                    format!("manifest error: {reason}"),
                    String::new(),
                    String::new(),
                    String::new(),
                    e.source.label().to_string(),
                    size,
                ],
                _ => [
                    count(e.pieces.len() as u64, "piece", "pieces"),
                    count(e.mic_setups.len() as u64, "setup", "setups"),
                    count(e.layers_max as u64, "layer", "layers"),
                    format!("{} RR", e.rr_max),
                    e.source.label().to_string(),
                    size,
                ],
            };
            let mut search = vec![e.dir_name.clone()];
            search.extend(e.description().map(str::to_string));
            search.extend(e.pieces.iter().map(|p| p.name.clone()));
            for m in e.mic_setups.values() {
                for s in [&m.brand, &m.mic, &m.position] {
                    if !s.is_empty() && !search.contains(s) {
                        search.push(s.clone());
                    }
                }
            }
            rows.push(KitRow {
                mics_bucket: mics_bucket(e.mic_setups.len()),
                entry: e.clone(),
                key,
                mark_key,
                columns,
                search,
            });
        }
        Self {
            rows,
            marks: mark_map,
            built_from,
        }
    }

    pub fn marks_of(&self, row: usize) -> Option<&Marks> {
        self.marks.get(&self.rows[row].mark_key)
    }
}

impl LibraryRows for KitRows {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, row: usize) -> &str {
        &self.rows[row].key
    }

    fn title(&self, row: usize) -> &str {
        &self.rows[row].entry.name
    }

    fn subtitle(&self, row: usize) -> String {
        let e = &self.rows[row].entry;
        let mut parts = vec![e.source.label().to_string()];
        parts.push(count(e.sample_count, "sample", "samples"));
        match &e.status {
            EntryStatus::MissingFiles(n) => {
                parts.push(count(*n as u64, "missing file", "missing files"))
            }
            EntryStatus::DuplicateOf(p) => parts.push(format!("duplicate of {}", p.display())),
            _ => {}
        }
        parts.join(" · ")
    }

    fn column(&self, row: usize, col: usize) -> Option<Cow<'_, str>> {
        self.rows[row]
            .columns
            .get(col)
            .map(|s| Cow::Borrowed(s.as_str()))
    }

    fn marks(&self, row: usize) -> Option<&Marks> {
        self.marks_of(row)
    }

    fn search_text(&self, row: usize) -> Vec<&str> {
        self.rows[row].search.iter().map(String::as_str).collect()
    }

    fn facet_names(&self) -> Vec<&str> {
        FACETS.iter().map(|(f, _)| *f).collect()
    }

    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        let r = &self.rows[row];
        let e = &r.entry;
        match facet {
            MICS_FACET => vec![r.mics_bucket],
            ARTICULATIONS_FACET => vec![if e.articulations.is_empty() {
                "no"
            } else {
                "yes"
            }],
            TAGS_FACET => e.index_tags().iter().map(String::as_str).collect(),
            f if f == SOURCE_FACET => vec![e.source.label()],
            _ => Vec::new(),
        }
    }

    fn sort_value(&self, row: usize, field: &str) -> SortValue {
        let e = &self.rows[row].entry;
        match field {
            "added" => SortValue::Number(-(e.added_at as f64)),
            "size" => e
                .size_bytes
                .map(|b| SortValue::Number(b as f64))
                .unwrap_or(SortValue::None),
            "pieces" => SortValue::Number(e.pieces.len() as f64),
            "mics" => SortValue::Number(e.mic_setups.len() as f64),
            _ => SortValue::None,
        }
    }
}

/// ◀/▶ over the browser's current view: the next row after the loaded
/// kit's that can be loaded (holds a slot and its manifest parsed),
/// clamped at both ends. With nothing loaded, or the loaded kit outside
/// the view, a step enters the view at its first (▶) or last (◀) loadable
/// row.
pub fn step_in_view(
    model: &BrowserModel,
    rows: &KitRows,
    loaded_id: Option<&str>,
    delta: i32,
) -> Option<u32> {
    let mut current = loaded_id.map(drumkit_library::mark_key);
    loop {
        let row = model.step_from(current.as_deref(), delta)?;
        let r = &rows.rows[row];
        if let (Some(slot), true) = (r.entry.slot, r.entry.is_loadable()) {
            return Some(slot);
        }
        current = Some(r.key.clone());
    }
}

/// The header's counter: "3 / 41 in view", or "– / 41 in view" when the
/// loaded kit is not in the view.
pub fn view_counter(model: &BrowserModel, loaded_id: Option<&str>) -> String {
    let n = model.view_len();
    if n == 0 {
        return String::new();
    }
    match loaded_id.and_then(|id| model.position_in_view(&drumkit_library::mark_key(id))) {
        Some(p) => format!("{} / {n} in view", p + 1),
        None => format!("– / {n} in view"),
    }
}
