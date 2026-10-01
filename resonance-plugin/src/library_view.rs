//! The GUI-agnostic view-state of a library browser: search, facets,
//! favourites-first sorting, ◀/▶ stepping over the current view, the
//! audition bracket and the confirm-in-place delete.
//!
//! One model serves every library kind (plugin presets, NAM models, later
//! IRs and kits) and both toolkits: the egui skin in
//! [`crate::library_ui`] (behind `editor-widgets`) and the app's iced skin.
//! That is why this module is **not** feature-gated
//! (plugin-preset-library.md §6.1 / §10.2 item 3, nam-model-library.md §10).
//!
//! The content lives with its owner behind the [`LibraryRows`] trait; this
//! model only ever holds row indices and mark keys. A caller:
//!
//! 1. implements [`LibraryRows`] over its snapshot of the content and marks,
//! 2. calls [`BrowserModel::refresh`] once per frame with a revision number
//!    it bumps whenever the rows or their marks change (the view is cached
//!    and recomputed only when the revision or the query changed), then
//! 3. reads [`BrowserModel::view`] and turns clicks into the methods here.
//!
//! # Search syntax
//!
//! Whitespace-separated tokens, all of which must match (AND). Matching is
//! case- and accent-insensitive substring matching.
//!
//! - a plain token matches the title, the row's [`LibraryRows::search_text`]
//!   and its tags (content and personal);
//! - `is:fav` (or `is:favorite` / `is:favourite` / `is:starred`) keeps
//!   favourites, `is:recent` keeps rows with a recorded use, and any other
//!   `is:<v>` scopes the `source` facet (`is:user`, `is:factory`,
//!   `is:tone3000`);
//! - `tag:<t>` matches a tag, `by:<name>` the `author` facet, `genre:<g>`
//!   the `genres` facet, `cat:` `category`, `for:` `instrument`, `char:`
//!   `character` (the preset library's spellings), and `<facet>:<value>`
//!   any facet the rows declare in [`LibraryRows::facet_names`]. A `word:`
//!   prefix that is none of these is plain text (so `http://…` still
//!   searches). [`parse_search`] is the one tokenizer.
//!
//! Matching is substring matching, a superset of the token-prefix rule.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use resonance_common::library_marks::vocab;
pub use resonance_common::library_marks::Marks;

/// The facet name for tags. [`LibraryRows::facet_values`] with this name
/// returns the row's *content* tags; the model merges in the personal
/// tags from [`LibraryRows::marks`].
pub const TAGS_FACET: &str = "tags";

/// The facet `by:` searches.
pub const AUTHOR_FACET: &str = "author";

/// The facet `is:<value>` searches (other than `is:fav` / `is:recent`):
/// where an item comes from (`user`, `factory`, `tone3000`, `imported`).
pub const SOURCE_FACET: &str = "source";

/// What [`BrowserModel::refresh`] compares to decide the rows changed: one
/// counter, or two compared as a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Revision(pub u64, pub u64);

impl From<u64> for Revision {
    fn from(v: u64) -> Self {
        Revision(v, 0)
    }
}

impl From<(u64, u64)> for Revision {
    fn from((a, b): (u64, u64)) -> Self {
        Revision(a, b)
    }
}

/// A sortable value one row offers for one field.
#[derive(Debug, Clone, PartialEq)]
pub enum SortValue {
    /// No value; always sorts last, whatever the direction.
    None,
    Text(String),
    Number(f64),
}

/// The rows a browser shows, owned by the caller. Only `row_count`, `key`
/// and `title` are required.
pub trait LibraryRows {
    fn row_count(&self) -> usize;

    /// The row's identity: its mark key `"<kind>:<id>"`. Unique among the
    /// rows; selection, stepping and deletes are keyed by it, so they
    /// survive re-sorting and rescans.
    fn key(&self, row: usize) -> &str;

    /// Display name; always searched, and the `Title` sort key.
    fn title(&self, row: usize) -> &str;

    /// Secondary line (author · source · …). Display only.
    fn subtitle(&self, _row: usize) -> String {
        String::new()
    }

    /// Display column `col` after the title (0-based, in the order the
    /// skin lays them out), or `None` past the last. Borrowed where the
    /// rows already hold the text, so a frame allocates nothing per row.
    fn column(&self, _row: usize, _col: usize) -> Option<Cow<'_, str>> {
        None
    }

    /// The row's personal marks, if any are stored.
    fn marks(&self, _row: usize) -> Option<&Marks> {
        None
    }

    /// Extra text a plain search token matches (author, gear, file name,
    /// description …). The title and tags are always searched.
    fn search_text(&self, _row: usize) -> Vec<&str> {
        Vec::new()
    }

    /// The facets these rows offer, for scoped search tokens and for the
    /// skin's filter lists (e.g. `["gear_type", "tone_type"]` for NAM,
    /// `["category", "genres", "character"]` for presets).
    fn facet_names(&self) -> Vec<&str> {
        Vec::new()
    }

    /// The row's values for `facet`. For [`TAGS_FACET`], its content tags
    /// (personal tags come from [`marks`](Self::marks)).
    fn facet_values(&self, _row: usize, _facet: &str) -> Vec<&str> {
        Vec::new()
    }

    /// The value `field` sorts by, for [`SortKey::Field`].
    fn sort_value(&self, _row: usize, _field: &str) -> SortValue {
        SortValue::None
    }
}

/// What the view is ordered by. Favourites-first applies on top of any
/// of these (see [`BrowserModel::set_favorites_first`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SortKey {
    /// The rows' own order (a factory bank's declared order, a slot
    /// table's order).
    #[default]
    Natural,
    /// Title, case- and accent-insensitive.
    Title,
    /// Most recently used first (with `descending: false`); never-used last.
    RecentlyUsed,
    /// A field the rows provide through [`LibraryRows::sort_value`].
    Field(String),
}

/// A sort key and its direction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sort {
    pub key: SortKey,
    /// Reverse the key's natural direction. `RecentlyUsed` is newest-first
    /// when this is `false`; everything else ascends.
    pub descending: bool,
}

impl Sort {
    pub fn by(key: SortKey) -> Self {
        Self {
            key,
            descending: false,
        }
    }
}

/// One value of a facet list, with how many rows in the current result
/// carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetCount {
    pub value: String,
    /// Rows matching the query and every *other* facet's selection.
    pub count: usize,
    pub selected: bool,
}

/// A one-line message under the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Info(String),
    Error(String),
}

impl Notice {
    pub fn text(&self) -> &str {
        match self {
            Notice::Info(t) | Notice::Error(t) => t,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Notice::Error(_))
    }
}

/// The browser's view-state. Editor-only runtime state: never persisted
/// (ux-guidelines.md, the collapse-state rule).
#[derive(Debug, Clone)]
pub struct BrowserModel {
    query: String,
    favorites_only: bool,
    recent_only: bool,
    favorites_first: bool,
    facets: BTreeMap<String, BTreeSet<String>>,
    sort: Sort,
    selected: Option<String>,
    pending_delete: Option<String>,
    notice: Option<Notice>,
    // Cache.
    view: Vec<usize>,
    key_index: HashMap<String, usize>,
    rows_revision: Option<Revision>,
    dirty: bool,
}

impl Default for BrowserModel {
    fn default() -> Self {
        Self {
            query: String::new(),
            favorites_only: false,
            recent_only: false,
            favorites_first: true,
            facets: BTreeMap::new(),
            sort: Sort::default(),
            selected: None,
            pending_delete: None,
            notice: None,
            view: Vec::new(),
            key_index: HashMap::new(),
            rows_revision: None,
            dirty: true,
        }
    }
}

/// Lowercased, accent-folded text for matching.
fn fold(text: &str) -> String {
    vocab::fold(text)
}

/// One parsed search token (see the module docs for the syntax). Public so
/// every library kind parses one syntax, whatever evaluates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchToken {
    /// A plain token, lowercased and accent-folded.
    Text(String),
    /// `is:fav` / `is:favorite` / `is:favourite` / `is:starred`.
    Favorite,
    /// `is:recent`.
    Recent,
    /// A scoped token: `tag:` / `by:` / `genre:` / `cat:` / `for:` /
    /// `char:` / `is:<source>` or `<facet>:` for a facet the rows declare.
    /// `value` is lowercased and accent-folded.
    Facet { facet: String, value: String },
}

/// The facet an alias scopes to: the shared spellings the preset library's
/// query uses too (`cat:` = category, `for:` = instrument, `char:` =
/// character, `by:` = author, `genre:` = genres, `tag:` = tags).
fn facet_alias(scope: &str) -> Option<&'static str> {
    Some(match scope {
        "tag" | "tags" => TAGS_FACET,
        "by" | "author" => AUTHOR_FACET,
        "genre" | "genres" => "genres",
        "cat" | "category" => "category",
        "for" | "instrument" => "instrument",
        "char" | "character" => "character",
        _ => return None,
    })
}

/// Split a search string into tokens. `facet_names` are the extra facets
/// a `<facet>:<value>` token may scope to; a `word:` prefix that is no
/// alias and no such facet is plain text (so `http://…` still searches).
/// `is:<value>` other than fav/recent scopes the `source` facet
/// (`is:user`, `is:factory`, `is:imported`).
pub fn parse_search(query: &str, facet_names: &[&str]) -> Vec<SearchToken> {
    query
        .split_whitespace()
        .map(|raw| {
            let lower = fold(raw);
            if let Some((scope, value)) = lower.split_once(':') {
                if !value.is_empty() {
                    if scope == "is" {
                        return match value {
                            "fav" | "favorite" | "favourite" | "favorites" | "starred" => {
                                SearchToken::Favorite
                            }
                            "recent" => SearchToken::Recent,
                            other => SearchToken::Facet {
                                facet: SOURCE_FACET.into(),
                                value: other.into(),
                            },
                        };
                    }
                    if let Some(facet) = facet_alias(scope) {
                        return SearchToken::Facet {
                            facet: facet.into(),
                            value: value.into(),
                        };
                    }
                    if let Some(f) = facet_names.iter().find(|f| fold(f) == scope) {
                        return SearchToken::Facet {
                            facet: f.to_string(),
                            value: value.into(),
                        };
                    }
                }
            }
            SearchToken::Text(lower)
        })
        .collect()
}

/// Every tag of a row: its content tags plus its personal ones.
fn row_tags<'a>(rows: &'a dyn LibraryRows, row: usize) -> Vec<&'a str> {
    let mut tags = rows.facet_values(row, TAGS_FACET);
    if let Some(m) = rows.marks(row) {
        for t in &m.tags {
            if !tags.contains(&t.as_str()) {
                tags.push(t);
            }
        }
    }
    tags
}

fn facet_values_of<'a>(rows: &'a dyn LibraryRows, row: usize, facet: &str) -> Vec<&'a str> {
    if facet == TAGS_FACET {
        row_tags(rows, row)
    } else {
        rows.facet_values(row, facet)
    }
}

/// Whether a facet value answers a scoped search token: the same value
/// (case and accents folded), or one whose slug starts with the token's —
/// `cat:ba` finds Bass, `genre:rock` does not find post-rock.
fn facet_value_matches(value: &str, wanted: &str) -> bool {
    let folded = fold(value);
    if folded == wanted {
        return true;
    }
    match (
        crate::library_marks::normalize_tag(value),
        crate::library_marks::normalize_tag(wanted),
    ) {
        (Some(v), Some(w)) => v.starts_with(&w),
        _ => false,
    }
}

fn token_matches(rows: &dyn LibraryRows, row: usize, token: &SearchToken) -> bool {
    match token {
        SearchToken::Favorite => rows.marks(row).is_some_and(|m| m.favorite),
        SearchToken::Recent => rows.marks(row).is_some_and(|m| m.last_used.is_some()),
        SearchToken::Facet { facet, value } => facet_values_of(rows, row, facet)
            .iter()
            .any(|v| facet_value_matches(v, value)),
        SearchToken::Text(t) => {
            fold(rows.title(row)).contains(t.as_str())
                || rows
                    .search_text(row)
                    .iter()
                    .any(|s| fold(s).contains(t.as_str()))
                || row_tags(rows, row)
                    .iter()
                    .any(|s| fold(s).contains(t.as_str()))
        }
    }
}

fn cmp_sort_values(a: &SortValue, b: &SortValue, descending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ord = match (a, b) {
        (SortValue::None, SortValue::None) => return Ordering::Equal,
        // Missing values last in either direction.
        (SortValue::None, _) => return Ordering::Greater,
        (_, SortValue::None) => return Ordering::Less,
        (SortValue::Number(x), SortValue::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (SortValue::Text(x), SortValue::Text(y)) => fold(x).cmp(&fold(y)),
        (SortValue::Number(_), SortValue::Text(_)) => Ordering::Less,
        (SortValue::Text(_), SortValue::Number(_)) => Ordering::Greater,
    };
    if descending {
        ord.reverse()
    } else {
        ord
    }
}

impl BrowserModel {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Query state -------------------------------------------------------

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        let query = query.into();
        if query != self.query {
            self.query = query;
            self.dirty = true;
        }
    }

    /// The query buffer for a text field to edit in place. Marks the view
    /// stale; the next [`refresh`](Self::refresh) re-filters.
    pub fn query_mut(&mut self) -> &mut String {
        self.dirty = true;
        &mut self.query
    }

    pub fn favorites_only(&self) -> bool {
        self.favorites_only
    }

    pub fn set_favorites_only(&mut self, on: bool) {
        self.dirty |= self.favorites_only != on;
        self.favorites_only = on;
    }

    /// Only rows with a recorded use (the NAM panel's `Recent` chip).
    pub fn recent_only(&self) -> bool {
        self.recent_only
    }

    pub fn set_recent_only(&mut self, on: bool) {
        self.dirty |= self.recent_only != on;
        self.recent_only = on;
    }

    pub fn favorites_first(&self) -> bool {
        self.favorites_first
    }

    /// Favourites sort ahead of everything else, within any sort (on by
    /// default).
    pub fn set_favorites_first(&mut self, on: bool) {
        self.dirty |= self.favorites_first != on;
        self.favorites_first = on;
    }

    pub fn sort(&self) -> &Sort {
        &self.sort
    }

    pub fn set_sort(&mut self, sort: Sort) {
        if sort != self.sort {
            self.sort = sort;
            self.dirty = true;
        }
    }

    /// The selected values of `facet` (empty = no filter on it).
    pub fn facet_selection(&self, facet: &str) -> Vec<&str> {
        self.facets
            .get(facet)
            .map(|s| s.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn is_facet_selected(&self, facet: &str, value: &str) -> bool {
        self.facets.get(facet).is_some_and(|s| s.contains(&fold(value)))
    }

    /// Toggle one facet value. Within a facet the selected values are ORed;
    /// across facets they are ANDed.
    pub fn toggle_facet(&mut self, facet: &str, value: &str) {
        let value = fold(value);
        let set = self.facets.entry(facet.to_string()).or_default();
        if !set.remove(&value) {
            set.insert(value);
        }
        if set.is_empty() {
            self.facets.remove(facet);
        }
        self.dirty = true;
    }

    /// Replace a facet's selection (e.g. a one-of combo box). An empty
    /// `value` clears it.
    pub fn set_facet(&mut self, facet: &str, value: Option<&str>) {
        match value {
            Some(v) => {
                self.facets.insert(facet.to_string(), BTreeSet::from([fold(v)]));
            }
            None => {
                self.facets.remove(facet);
            }
        }
        self.dirty = true;
    }

    pub fn clear_facets(&mut self) {
        self.dirty |= !self.facets.is_empty();
        self.facets.clear();
    }

    /// How many filters (facet values, favourites-only, recent-only) are on,
    /// for a "(+ filters 2)" label.
    pub fn active_filter_count(&self) -> usize {
        self.facets.values().map(BTreeSet::len).sum::<usize>()
            + usize::from(self.favorites_only)
            + usize::from(self.recent_only)
    }

    // -- The view ----------------------------------------------------------

    /// Recompute the view if the query changed or `rows_revision` differs
    /// from the last call. Pass anything that changes whenever the rows or
    /// any row's marks change: a `u64` counter, or a `(content, marks)`
    /// pair of counters (compared as a pair, so two counters can never
    /// collide into one). Returns whether the view was recomputed. A
    /// selection or pending delete whose row is gone is dropped.
    pub fn refresh(&mut self, rows: &dyn LibraryRows, rows_revision: impl Into<Revision>) -> bool {
        let rows_revision = rows_revision.into();
        let rows_changed = self.rows_revision != Some(rows_revision);
        if !self.dirty && !rows_changed {
            return false;
        }
        if rows_changed {
            self.key_index = (0..rows.row_count())
                .map(|r| (rows.key(r).to_string(), r))
                .collect();
        }
        self.rows_revision = Some(rows_revision);
        self.dirty = false;
        self.view = self.compute(rows, None);
        if rows_changed {
            if let Some(k) = &self.pending_delete {
                if !self.key_index.contains_key(k) {
                    self.pending_delete = None;
                }
            }
        }
        true
    }

    /// Mark the view stale without a revision change (e.g. a mark was
    /// toggled on a row set whose revision the caller does not track).
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// The current view: row indices, filtered and sorted.
    pub fn view(&self) -> &[usize] {
        &self.view
    }

    pub fn view_len(&self) -> usize {
        self.view.len()
    }

    /// The row index of `key`, as of the last refresh.
    pub fn row_of(&self, key: &str) -> Option<usize> {
        self.key_index.get(key).copied()
    }

    /// Where `key` sits in the view (0-based), for a "3 / 41 in view"
    /// counter.
    pub fn position_in_view(&self, key: &str) -> Option<usize> {
        let row = self.row_of(key)?;
        self.view.iter().position(|&r| r == row)
    }

    /// Whether a row passes the query, the favourite/recent switches and
    /// every facet except `skip_facet`.
    fn passes(
        &self,
        rows: &dyn LibraryRows,
        row: usize,
        tokens: &[SearchToken],
        skip_facet: Option<&str>,
    ) -> bool {
        if self.favorites_only && !rows.marks(row).is_some_and(|m| m.favorite) {
            return false;
        }
        if self.recent_only && !rows.marks(row).is_some_and(|m| m.last_used.is_some()) {
            return false;
        }
        for (facet, wanted) in &self.facets {
            if Some(facet.as_str()) == skip_facet {
                continue;
            }
            // Selections are stored folded, so `Bass` and `bass` (a
            // category outside the seeded vocabulary, say) are one value.
            let values = facet_values_of(rows, row, facet);
            if !values.iter().any(|v| wanted.contains(&fold(v))) {
                return false;
            }
        }
        tokens.iter().all(|t| token_matches(rows, row, t))
    }

    fn compute(&self, rows: &dyn LibraryRows, skip_facet: Option<&str>) -> Vec<usize> {
        let names = rows.facet_names();
        let tokens = parse_search(&self.query, &names);
        let mut view: Vec<usize> = (0..rows.row_count())
            .filter(|&r| self.passes(rows, r, &tokens, skip_facet))
            .collect();
        if skip_facet.is_none() {
            self.sort_rows(rows, &mut view);
        }
        view
    }

    fn sort_rows(&self, rows: &dyn LibraryRows, view: &mut [usize]) {
        use std::cmp::Ordering;
        let fav = |r: usize| rows.marks(r).is_some_and(|m| m.favorite);
        let title_cmp = |a: usize, b: usize| fold(rows.title(a)).cmp(&fold(rows.title(b)));
        view.sort_by(|&a, &b| {
            let fav_ord = if self.favorites_first {
                fav(b).cmp(&fav(a))
            } else {
                Ordering::Equal
            };
            let key_ord = match &self.sort.key {
                SortKey::Natural => {
                    let o = a.cmp(&b);
                    if self.sort.descending {
                        o.reverse()
                    } else {
                        o
                    }
                }
                SortKey::Title => {
                    let o = title_cmp(a, b);
                    if self.sort.descending {
                        o.reverse()
                    } else {
                        o
                    }
                }
                SortKey::RecentlyUsed => {
                    let t = |r: usize| rows.marks(r).and_then(|m| m.last_used);
                    let as_value = |r: usize| match t(r) {
                        Some(v) => SortValue::Number(v as f64),
                        None => SortValue::None,
                    };
                    // Newest first unless reversed.
                    cmp_sort_values(&as_value(a), &as_value(b), !self.sort.descending)
                }
                SortKey::Field(field) => cmp_sort_values(
                    &rows.sort_value(a, field),
                    &rows.sort_value(b, field),
                    self.sort.descending,
                ),
            };
            fav_ord
                .then(key_ord)
                .then_with(|| title_cmp(a, b))
                .then_with(|| a.cmp(&b))
        });
    }

    /// The values of `facet` across the rows that match the query and every
    /// other facet's selection, with counts (standard faceted search).
    /// Selected values stay listed at zero. Seeded vocabulary values
    /// ([`vocab::Facet`]) come first in their declared order, then the rest
    /// by count and name.
    pub fn facet_counts(&self, rows: &dyn LibraryRows, facet: &str) -> Vec<FacetCount> {
        let candidates = self.compute(rows, Some(facet));
        // Grouped by the folded value (one entry for `Bass` and `bass`),
        // shown as first seen.
        let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for r in candidates {
            let mut seen: Vec<String> = Vec::new();
            for v in facet_values_of(rows, r, facet) {
                let key = fold(v);
                if !seen.contains(&key) {
                    counts.entry(key.clone()).or_insert_with(|| (v.to_string(), 0)).1 += 1;
                    seen.push(key);
                }
            }
        }
        for v in self.facet_selection(facet) {
            counts.entry(v.to_string()).or_insert_with(|| (v.to_string(), 0));
        }
        let seeded = vocab::Facet::from_name(facet);
        let rank = |v: &str| seeded.and_then(|f| f.seeded_rank(v));
        let mut out: Vec<FacetCount> = counts
            .into_values()
            .map(|(value, count)| FacetCount {
                selected: self.is_facet_selected(facet, &value),
                value,
                count,
            })
            .collect();
        out.sort_by(|a, b| match (rank(&a.value), rank(&b.value)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b
                .count
                .cmp(&a.count)
                .then_with(|| fold(&a.value).cmp(&fold(&b.value))),
        });
        out
    }

    // -- Selection and stepping --------------------------------------------

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The selected row's index, if it is still in the rows.
    pub fn selected_row(&self) -> Option<usize> {
        self.selected.as_deref().and_then(|k| self.row_of(k))
    }

    /// Select `key`. Selecting another row disarms a pending delete, so a
    /// second click can never delete a row other than the one that was
    /// armed.
    pub fn select(&mut self, key: impl Into<String>) {
        let key = key.into();
        if self.pending_delete.as_deref().is_some_and(|p| p != key) {
            self.pending_delete = None;
        }
        self.selected = Some(key);
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.pending_delete = None;
    }

    /// The row `delta` places from `current` in the view, clamped rather
    /// than wrapping (running off one end and reappearing at the other is
    /// disorienting while listening). With no `current`, or one not in the
    /// view, a forward step enters at the first row and a backward step at
    /// the last. `None` at either end or on an empty view.
    pub fn step_from(&self, current: Option<&str>, delta: i32) -> Option<usize> {
        if self.view.is_empty() {
            return None;
        }
        let pos = current.and_then(|k| self.position_in_view(k));
        let target = match pos {
            Some(p) => {
                let next = p as i64 + delta as i64;
                if next < 0 || next >= self.view.len() as i64 {
                    return None;
                }
                next as usize
            }
            None if delta >= 0 => 0,
            None => self.view.len() - 1,
        };
        Some(self.view[target])
    }

    /// Move the selection `delta` places through the view (↑/↓) and return
    /// the newly selected row.
    pub fn move_selection(&mut self, rows: &dyn LibraryRows, delta: i32) -> Option<usize> {
        let row = self.step_from(self.selected.as_deref(), delta)?;
        self.select(rows.key(row).to_string());
        Some(row)
    }

    // -- Confirm-in-place delete -------------------------------------------

    /// The first click of a two-click delete: the skin turns the row (or
    /// the detail pane's button) into "Delete …? [Delete] [Cancel]".
    pub fn begin_delete(&mut self, key: impl Into<String>) {
        self.pending_delete = Some(key.into());
    }

    /// The key awaiting a second click, if any.
    pub fn pending_delete(&self) -> Option<&str> {
        self.pending_delete.as_deref()
    }

    pub fn cancel_delete(&mut self) {
        self.pending_delete = None;
    }

    /// The second click: hands back the key to delete (the caller owns the
    /// delete and its policy) and clears the pending state and, if it was
    /// the deleted row, the selection.
    pub fn confirm_delete(&mut self) -> Option<String> {
        let key = self.pending_delete.take()?;
        if self.selected.as_deref() == Some(key.as_str()) {
            self.selected = None;
        }
        Some(key)
    }

    // -- Notices -----------------------------------------------------------

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub fn set_info(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice::Info(text.into()));
    }

    pub fn set_error(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice::Error(text.into()));
    }

    pub fn clear_notice(&mut self) {
        self.notice = None;
    }
}

/// ◀/▶ over a browser's view (nam-model-library.md §5.1,
/// drums-plugin-rework.md §6.5), shared by every library kind: from the
/// row keyed `current_key`, step `delta` places, then on in the same
/// direction one row at a time past rows `loadable` refuses (it returns
/// the slot to load, or `None` for a row that cannot be loaded). Clamped
/// at both ends; with no current row, or one outside the view, a step
/// enters at the first (▶) or last (◀) loadable row. `None` for
/// `delta == 0`: there is nowhere to go, and a zero step from an
/// unloadable row would never move.
pub fn step_loadable(
    model: &BrowserModel,
    rows: &dyn LibraryRows,
    current_key: Option<&str>,
    delta: i32,
    loadable: impl Fn(usize) -> Option<u32>,
) -> Option<u32> {
    if delta == 0 {
        return None;
    }
    let mut row = model.step_from(current_key, delta)?;
    // Every further step moves one row, so the view bounds the walk.
    for _ in 0..=model.view_len() {
        if let Some(slot) = loadable(row) {
            return Some(slot);
        }
        row = model.step_from(Some(rows.key(row)), delta.signum())?;
    }
    None
}

/// The ◀/▶ header's counter: "3 / 41 in view", "– / 41 in view" when the
/// row keyed `current_key` is not in the view, or nothing for an empty
/// view.
pub fn view_counter(model: &BrowserModel, current_key: Option<&str>) -> String {
    let n = model.view_len();
    if n == 0 {
        return String::new();
    }
    match current_key.and_then(|k| model.position_in_view(k)) {
        Some(p) => format!("{} / {n} in view", p + 1),
        None => format!("– / {n} in view"),
    }
}

// ---------------------------------------------------------------------------
// Audition bracket
// ---------------------------------------------------------------------------

/// Something a browser can load provisionally and put back: a plugin's
/// whole sound for presets. Implemented by the caller; the bracket only
/// sequences the calls.
pub trait Audition {
    /// Everything needed to put the target back exactly (params and any
    /// sound-bearing extra state).
    type Snapshot;

    /// Capture the current state, before the first provisional load.
    fn capture(&mut self) -> Self::Snapshot;

    /// Load the item `key` provisionally.
    fn apply(&mut self, key: &str) -> Result<(), String>;

    /// Put back a captured state.
    fn restore(&mut self, snapshot: Self::Snapshot);
}

/// What an audition call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditionEvent {
    None,
    /// `key` is now loaded provisionally.
    Auditioned(String),
    /// The bracket closed keeping `key` (the last provisional load). The
    /// host records one undo entry for the whole bracket here.
    Committed(String),
    /// The bracket closed and the pre-audition state was restored. No undo
    /// entry.
    Reverted,
    /// `apply` failed; the bracket stays open on the previous state.
    Failed(String),
}

/// The audition bracket (plugin-preset-library.md §6.4): moving the
/// selection loads provisionally; before the first provisional load the
/// current state is captured; Enter/double-click commits; Esc/close
/// reverts. Kept separate from [`BrowserModel`] so a browser without
/// audition (the NAM manager) carries no snapshot type.
#[derive(Debug)]
pub struct AuditionBracket<S> {
    snapshot: Option<S>,
    provisional: Option<String>,
}

impl<S> Default for AuditionBracket<S> {
    fn default() -> Self {
        Self {
            snapshot: None,
            provisional: None,
        }
    }
}

impl<S> AuditionBracket<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a revert snapshot is held.
    pub fn is_open(&self) -> bool {
        self.snapshot.is_some()
    }

    /// The key loaded provisionally, if any.
    pub fn provisional(&self) -> Option<&str> {
        self.provisional.as_deref()
    }

    /// Load `key` provisionally, capturing the revert snapshot first if the
    /// bracket is not open yet. Re-auditioning the current key is a no-op.
    pub fn audition<A: Audition<Snapshot = S>>(&mut self, target: &mut A, key: &str) -> AuditionEvent {
        if self.provisional.as_deref() == Some(key) {
            return AuditionEvent::None;
        }
        if self.snapshot.is_none() {
            self.snapshot = Some(target.capture());
        }
        match target.apply(key) {
            Ok(()) => {
                self.provisional = Some(key.to_string());
                AuditionEvent::Auditioned(key.to_string())
            }
            Err(e) => AuditionEvent::Failed(e),
        }
    }

    /// Keep what is loaded and close the bracket.
    pub fn commit(&mut self) -> AuditionEvent {
        self.snapshot = None;
        match self.provisional.take() {
            Some(key) => AuditionEvent::Committed(key),
            None => AuditionEvent::None,
        }
    }

    /// Restore the captured state and close the bracket.
    pub fn revert<A: Audition<Snapshot = S>>(&mut self, target: &mut A) -> AuditionEvent {
        self.provisional = None;
        match self.snapshot.take() {
            Some(s) => {
                target.restore(s);
                AuditionEvent::Reverted
            }
            None => AuditionEvent::None,
        }
    }
}
