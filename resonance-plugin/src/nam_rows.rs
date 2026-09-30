//! The NAM model library as browser rows: one [`LibraryRows`] adapter over
//! a snapshot of `resonance_common::nam_library` entries and their marks,
//! shared by the amp's Library panel and the app's `amp_models.*` control
//! methods, so a query means the same thing in both (nam-model-library.md
//! §6.2, §9.3). Plus the ◀/▶ and counter rules over a view of them.
//!
//! Not feature-gated: the app (no `editor-widgets`) builds the same rows.

use std::borrow::Cow;
use std::collections::HashMap;

use resonance_common::library_marks::{Marks, MarksStore};
use resonance_common::nam_library::{self, Entry, EntryStatus, Library, Source};

use crate::library_view::{BrowserModel, LibraryRows, SortKey, SortValue, SOURCE_FACET};

/// The facets the Installed tab offers, as `(facet, label)`.
pub const FACETS: &[(&str, &str)] = &[
    ("gear_type", "Gear"),
    ("tone_type", "Type"),
    ("architecture", "Arch"),
];

/// The sorts the Installed tab offers, as `(label, key)`. Favourites
/// always sort first within any of them.
pub fn sort_options() -> Vec<(&'static str, SortKey)> {
    vec![
        ("Slot", SortKey::Natural),
        ("Name", SortKey::Title),
        ("Recently used", SortKey::RecentlyUsed),
        ("Recently added", SortKey::Field("added".into())),
        ("Author", SortKey::Field("author".into())),
        ("Size", SortKey::Field("size".into())),
    ]
}

/// One row: an entry, its unique row key, and its display text.
#[derive(Debug, Clone)]
pub struct ModelRow {
    pub entry: Entry,
    /// `amp-model:<id>`, or with a `#<path>` suffix for a duplicate file
    /// (row keys must be unique; marks are always looked up by id).
    pub key: String,
    pub mark_key: String,
    columns: [String; 7],
    arch_short: String,
}

/// A snapshot of the library as rows, plus the marks of its models.
#[derive(Debug, Clone, Default)]
pub struct ModelRows {
    pub rows: Vec<ModelRow>,
    marks: HashMap<String, Marks>,
    /// What this was built from (library revision, marks generation), for
    /// the caller's staleness check.
    pub built_from: (u64, u64),
}

/// "4.1 MB", "412 KB".
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// "48k", "44.1k".
pub fn format_rate(hz: f64) -> String {
    let k = hz / 1000.0;
    if (k - k.round()).abs() < 0.05 {
        format!("{k:.0}k")
    } else {
        format!("{k:.1}k")
    }
}

/// `WaveNet A1` → `A1`, `A2 slimmable` → `A2s`, …: the Arch facet values.
pub fn arch_short(label: &str) -> String {
    match label {
        "WaveNet A1" => "A1".into(),
        "WaveNet A2" => "A2".into(),
        "A2 slimmable" => "A2s".into(),
        other => other.into(),
    }
}

impl ModelRows {
    /// Build rows from the library and (optionally) a marks snapshot.
    pub fn build(library: &Library, marks: Option<&MarksStore>, built_from: (u64, u64)) -> Self {
        let mut rows = Vec::with_capacity(library.len());
        let mut mark_map = HashMap::new();
        for e in library.entries() {
            let mark_key = nam_library::mark_key(&e.id);
            let key = match &e.status {
                EntryStatus::DuplicateOf(_) => format!("{mark_key}#{}", e.path.display()),
                _ => mark_key.clone(),
            };
            if let Some(m) = marks.and_then(|s| s.get(&mark_key)) {
                mark_map.insert(mark_key.clone(), m.clone());
            }
            let arch_short = arch_short(&e.architecture);
            let dash = || "—".to_string();
            let columns = match &e.status {
                // An unreadable file says why in its row, where the other
                // columns would be dashes.
                EntryStatus::Unreadable(reason) => [
                    format!("unreadable: {reason}"),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    format_size(e.size_bytes),
                ],
                _ => [
                    e.author.clone().unwrap_or_else(dash),
                    e.gear.clone().unwrap_or_else(dash),
                    e.gear_type.clone().unwrap_or_else(dash),
                    e.tone_type.clone().unwrap_or_else(dash),
                    arch_short.clone(),
                    format_rate(e.sample_rate),
                    format_size(e.size_bytes),
                ],
            };
            rows.push(ModelRow {
                entry: e.clone(),
                key,
                mark_key,
                columns,
                arch_short,
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

impl LibraryRows for ModelRows {
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
        let mut parts = Vec::new();
        if let Some(a) = &e.author {
            parts.push(format!("by {a}"));
        }
        parts.push(match &e.source {
            Source::Tone3000 { tone_id, .. } => format!("Tone3000 tone #{tone_id}"),
            Source::Imported => "imported".into(),
            Source::External => "external".into(),
        });
        parts.push(e.architecture.clone());
        parts.join(" · ")
    }

    fn column(&self, row: usize, col: usize) -> Option<Cow<'_, str>> {
        self.rows[row].columns.get(col).map(|s| Cow::Borrowed(s.as_str()))
    }

    fn marks(&self, row: usize) -> Option<&Marks> {
        self.marks_of(row)
    }

    fn search_text(&self, row: usize) -> Vec<&str> {
        let e = &self.rows[row].entry;
        let mut out = vec![e.file_name.as_str()];
        for s in [&e.author, &e.gear, &e.gear_type, &e.tone_type].into_iter().flatten() {
            out.push(s.as_str());
        }
        out
    }

    fn facet_names(&self) -> Vec<&str> {
        FACETS.iter().map(|(f, _)| *f).chain(["author"]).collect()
    }

    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        let r = &self.rows[row];
        let e = &r.entry;
        match facet {
            "gear_type" => e.gear_type.as_deref().into_iter().collect(),
            "tone_type" => e.tone_type.as_deref().into_iter().collect(),
            "architecture" => vec![r.arch_short.as_str()],
            "author" => e.author.as_deref().into_iter().collect(),
            f if f == SOURCE_FACET => vec![e.source.label()],
            _ => Vec::new(),
        }
    }

    fn sort_value(&self, row: usize, field: &str) -> SortValue {
        let e = &self.rows[row].entry;
        match field {
            "added" => SortValue::Number(-(e.added_at as f64)),
            "size" => SortValue::Number(e.size_bytes as f64),
            "author" => e
                .author
                .clone()
                .map(SortValue::Text)
                .unwrap_or(SortValue::None),
            _ => SortValue::None,
        }
    }
}

/// ◀/▶ over the browser's current view (§5.1): the next row after the
/// loaded model's that can be loaded (has a slot), clamped at both ends.
/// With nothing loaded, or the loaded model outside the view, a step
/// enters the view at its first (▶) or last (◀) loadable row.
pub fn step_in_view(
    model: &BrowserModel,
    rows: &ModelRows,
    loaded_id: Option<&str>,
    delta: i32,
) -> Option<u32> {
    let mut current = loaded_id.map(nam_library::mark_key);
    loop {
        let row = model.step_from(current.as_deref(), delta)?;
        let r = &rows.rows[row];
        if let Some(slot) = r.entry.slot {
            return Some(slot);
        }
        // Skip a row with no slot (a duplicate) and keep going.
        current = Some(r.key.clone());
    }
}

/// The header's counter: "3 / 41 in view", or "– / 41 in view" when the
/// loaded model is not in the view.
pub fn view_counter(model: &BrowserModel, loaded_id: Option<&str>) -> String {
    let n = model.view_len();
    if n == 0 {
        return String::new();
    }
    match loaded_id.and_then(|id| model.position_in_view(&nam_library::mark_key(id))) {
        Some(p) => format!("{} / {n} in view", p + 1),
        None => format!("– / {n} in view"),
    }
}
