//! Search, facets and sorting over presets: a thin, typed front for the
//! one search engine, [`library_view::BrowserModel`](crate::library_view),
//! over [`PresetRows`] (plugin-preset-library.md §4.6, §6.4).
//!
//! The browser in a plugin editor, the host's browser and the control
//! API's `presets.search` therefore agree on every result. The text syntax
//! is [`parse_search`](crate::library_view::parse_search)'s: plain tokens
//! are case- and accent-insensitive substrings of the name, author,
//! description, category and tags, ANDed; `genre:`, `tag:`, `by:`, `cat:`,
//! `for:`, `char:`, `is:fav`, `is:recent`, `is:user` / `is:factory` scope
//! themselves. Facets are OR within, AND across, and each facet's counts
//! apply the *other* facets (standard faceted search).

use crate::library_view::{BrowserModel, Sort as ViewSort, SortKey, SOURCE_FACET, TAGS_FACET};
use resonance_common::library_marks::{normalize_tag, Marks};

use super::format::canonical_category;
use super::rows::PresetRows;
use super::{PresetRecord, PresetSource};

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Factory presets in declared order, then user presets by name (per
    /// plugin, in the order the plugins were asked for). The default.
    #[default]
    Bank,
    Name,
    /// By category (uncategorised last), then name.
    Category,
    /// Most recently used first (marks' `last_used`); never-used last.
    RecentlyUsed,
    /// Most recently modified first (`meta.modified`).
    RecentlyModified,
}

impl Sort {
    /// The wire spelling (`bank`, `name`, `category`, `recent`, `modified`).
    pub fn parse(s: &str) -> Option<Sort> {
        Some(match s {
            "bank" => Sort::Bank,
            "name" => Sort::Name,
            "category" => Sort::Category,
            "recent" | "recently_used" => Sort::RecentlyUsed,
            "modified" | "recently_modified" => Sort::RecentlyModified,
            _ => return None,
        })
    }

    fn view(self) -> ViewSort {
        match self {
            Sort::Bank => ViewSort::by(SortKey::Natural),
            Sort::Name => ViewSort::by(SortKey::Title),
            Sort::Category => ViewSort::by(SortKey::Field("category".into())),
            Sort::RecentlyUsed => ViewSort::by(SortKey::RecentlyUsed),
            Sort::RecentlyModified => ViewSort {
                key: SortKey::Field("modified".into()),
                descending: true,
            },
        }
    }
}

/// A query over one or more plugins' presets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Free text plus scoped tokens (see the module docs).
    pub text: String,
    /// Plugin ids to search; empty searches every indexed plugin.
    pub plugins: Vec<String>,
    /// Empty means both.
    pub sources: Vec<PresetSource>,
    pub favorites_only: bool,
    pub category: Vec<String>,
    pub instrument: Vec<String>,
    pub genres: Vec<String>,
    pub character: Vec<String>,
    /// Matched against content tags ∪ personal tags.
    pub tags: Vec<String>,
    pub sort: Sort,
    /// Favourites first, on top of any sort.
    pub favorites_first: bool,
}

impl Query {
    /// Everything for one plugin, in bank order.
    pub fn plugin(plugin_id: impl Into<String>) -> Self {
        Self {
            plugins: vec![plugin_id.into()],
            ..Self::default()
        }
    }

    /// A browser model configured for this query (text, switches, facet
    /// selections, sort).
    pub fn to_model(&self) -> BrowserModel {
        let mut model = BrowserModel::new();
        model.set_query(self.text.clone());
        model.set_favorites_only(self.favorites_only);
        model.set_favorites_first(self.favorites_first);
        model.set_sort(self.sort.view());
        for s in &self.sources {
            model.toggle_facet(SOURCE_FACET, s.as_str());
        }
        for c in self.category.iter().filter_map(|c| canonical_category(c)) {
            model.toggle_facet("category", &c);
        }
        for (facet, values) in [
            ("instrument", &self.instrument),
            ("genres", &self.genres),
            ("character", &self.character),
            (TAGS_FACET, &self.tags),
        ] {
            for v in values.iter().filter_map(|v| normalize_tag(v)) {
                if !model.is_facet_selected(facet, &v) {
                    model.toggle_facet(facet, &v);
                }
            }
        }
        model
    }
}

/// One matching preset.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub plugin_id: String,
    pub record: PresetRecord,
    pub favorite: bool,
    /// Content tags ∪ personal tags, content first.
    pub tags: Vec<String>,
    /// Personal tags only.
    pub personal_tags: Vec<String>,
    pub marks: Marks,
    /// `marks.last_used` in RFC 3339.
    pub last_used: Option<String>,
}

/// `value → count` for one facet: seeded vocabulary first, then by count.
pub type FacetCounts = Vec<(String, usize)>;

/// Facet counts that come back with the hits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    pub source: FacetCounts,
    pub category: FacetCounts,
    pub instrument: FacetCounts,
    pub genres: FacetCounts,
    pub character: FacetCounts,
    pub tags: FacetCounts,
}

/// What a query returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryResult {
    pub hits: Vec<Hit>,
    pub facets: Facets,
}

/// Run `q` over `rows` (already restricted to `q.plugins`).
pub fn run(rows: &PresetRows, q: &Query) -> QueryResult {
    let mut model = q.to_model();
    model.refresh(rows, 0u64);
    let counts = |facet: &str| -> FacetCounts {
        model
            .facet_counts(rows, facet)
            .into_iter()
            .map(|c| (c.value, c.count))
            .collect()
    };
    let facets = Facets {
        source: counts(SOURCE_FACET),
        category: counts("category"),
        instrument: counts("instrument"),
        genres: counts("genres"),
        character: counts("character"),
        tags: counts(TAGS_FACET),
    };
    let hits = model
        .view()
        .iter()
        .map(|&i| {
            let row = &rows.rows[i];
            let marks = row.marks.clone().unwrap_or_default();
            Hit {
                plugin_id: row.plugin_id.clone(),
                record: row.record.clone(),
                favorite: marks.favorite,
                tags: row.all_tags(),
                personal_tags: marks.tags.clone(),
                last_used: marks.last_used_rfc3339(),
                marks,
            }
        })
        .collect();
    QueryResult { hits, facets }
}
