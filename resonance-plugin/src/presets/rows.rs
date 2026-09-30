//! Preset records as browser rows: the one [`LibraryRows`] adapter the
//! plugin editor's browser, the host's iced browser and the control API's
//! `presets.search` all read, so a query means the same thing everywhere
//! (plugin-preset-library.md §6.1). Not feature-gated.

use std::borrow::Cow;

use crate::library_view::{LibraryRows, SortValue, AUTHOR_FACET, SOURCE_FACET, TAGS_FACET};
use resonance_common::library_marks::Marks;

use super::marks::{mark_key, MarksSource};
use super::{PresetRecord, PresetSource};

/// The facets preset rows offer, as `(facet, label)`, in the order a
/// browser lists them.
pub const FACETS: &[(&str, &str)] = &[
    ("category", "Category"),
    ("instrument", "For"),
    ("genres", "Genre"),
    ("character", "Character"),
    (TAGS_FACET, "Tags"),
];

/// Every facet name a preset row answers ([`FACETS`] plus source/author,
/// which the search syntax reaches as `is:` / `by:`).
pub const FACET_NAMES: &[&str] = &[
    "category",
    "instrument",
    "genres",
    "character",
    SOURCE_FACET,
    AUTHOR_FACET,
    "plugin",
];

/// One row: a record, which plugin it belongs to, and its marks.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetRow {
    pub plugin_id: String,
    pub record: PresetRecord,
    /// `plugin-preset:<clap>:<id>`: unique across plugins and sources (user
    /// ids are UUIDs, factory ids slugs, and the index re-mints a user file
    /// that would collide).
    pub key: String,
    pub marks: Option<Marks>,
}

impl PresetRow {
    pub fn is_favorite(&self) -> bool {
        self.marks.as_ref().is_some_and(|m| m.favorite)
    }

    /// Content tags ∪ personal tags, content first.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags = self.record.meta.tags.clone();
        for t in self.marks.iter().flat_map(|m| &m.tags) {
            if !tags.contains(t) {
                tags.push(t.clone());
            }
        }
        tags
    }
}

/// A snapshot of one or more plugins' presets with their marks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresetRows {
    pub rows: Vec<PresetRow>,
}

impl PresetRows {
    /// Rows for `(plugin_id, records)` sets in the order given (bank order
    /// within each), with marks read from `marks`.
    pub fn build<'a>(
        sets: impl IntoIterator<Item = (&'a str, &'a [PresetRecord])>,
        marks: &dyn MarksSource,
    ) -> Self {
        let mut rows = Vec::new();
        for (plugin_id, records) in sets {
            for record in records {
                let key = mark_key(plugin_id, &record.preset.id);
                let m = marks.marks(&key);
                rows.push(PresetRow {
                    plugin_id: plugin_id.to_string(),
                    record: record.clone(),
                    marks: (!m.is_default()).then_some(m),
                    key,
                });
            }
        }
        Self { rows }
    }

    /// The row whose key is `key`.
    pub fn find(&self, key: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.key == key)
    }
}

fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn source_label(source: PresetSource) -> &'static str {
    source.as_str()
}

impl LibraryRows for PresetRows {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, row: usize) -> &str {
        &self.rows[row].key
    }

    fn title(&self, row: usize) -> &str {
        &self.rows[row].record.meta.name
    }

    fn subtitle(&self, row: usize) -> String {
        let r = &self.rows[row].record;
        let mut parts = Vec::new();
        if let Some(a) = &r.meta.author {
            parts.push(format!("by {a}"));
        }
        parts.push(source_label(r.preset.source).to_string());
        if let Some(c) = &r.meta.category {
            parts.push(c.clone());
        }
        parts.join(" · ")
    }

    /// Columns: category, then `F`/`U` for the source.
    fn column(&self, row: usize, col: usize) -> Option<Cow<'_, str>> {
        let r = &self.rows[row].record;
        match col {
            0 => Some(Cow::Borrowed(r.meta.category.as_deref().unwrap_or(""))),
            1 => Some(Cow::Borrowed(match r.preset.source {
                PresetSource::Factory => "F",
                PresetSource::User => "U",
            })),
            _ => None,
        }
    }

    fn marks(&self, row: usize) -> Option<&Marks> {
        self.rows[row].marks.as_ref()
    }

    fn search_text(&self, row: usize) -> Vec<&str> {
        let m = &self.rows[row].record.meta;
        [&m.author, &m.description, &m.category]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }

    fn facet_names(&self) -> Vec<&str> {
        FACET_NAMES.to_vec()
    }

    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        let r = &self.rows[row];
        let m = &r.record.meta;
        match facet {
            "category" => m.category.as_deref().into_iter().collect(),
            "instrument" => strs(&m.instrument),
            "genres" => strs(&m.genres),
            "character" => strs(&m.character),
            "plugin" => vec![r.plugin_id.as_str()],
            f if f == TAGS_FACET => strs(&m.tags),
            f if f == AUTHOR_FACET => m.author.as_deref().into_iter().collect(),
            f if f == SOURCE_FACET => vec![source_label(r.record.preset.source)],
            _ => Vec::new(),
        }
    }

    fn sort_value(&self, row: usize, field: &str) -> SortValue {
        let m = &self.rows[row].record.meta;
        let text = |v: &Option<String>| v.clone().map(SortValue::Text).unwrap_or(SortValue::None);
        match field {
            "category" => text(&m.category),
            "modified" => text(&m.modified),
            "author" => text(&m.author),
            _ => SortValue::None,
        }
    }
}
