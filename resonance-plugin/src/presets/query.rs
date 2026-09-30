//! Search, facets and sorting over the preset index
//! (plugin-preset-library.md §4.6, §6.4).
//!
//! - **Text** is case- and accent-insensitive and token-prefix based over
//!   name, author, description, tags and category; tokens are ANDed, and
//!   name hits rank first. A token like `genre:metal`, `tag:ferrous`,
//!   `is:fav`, `by:jorrit`, `cat:bass`, `for:vocal`, `char:dark`,
//!   `is:user` scopes itself, so the same string works in a browser's
//!   search field and in an MCP `query`.
//! - **Facets** are OR within a facet, AND across facets. Each facet's
//!   counts are computed on the result set with the *other* facets
//!   applied (standard faceted search), and a selected value stays listed
//!   at zero.

use std::collections::BTreeMap;

use super::marks::PresetMarks;
use super::{PresetRecord, PresetSource};

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Factory presets in declared order, then user presets by name: the
    /// order of the compact bar's list. The default.
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
}

/// One matching preset.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub plugin_id: String,
    pub record: PresetRecord,
    pub favorite: bool,
    /// Content tags ∪ personal tags, content first.
    pub tags: Vec<String>,
    pub marks: PresetMarks,
    /// Every free-text token matched the name.
    pub name_hit: bool,
}

/// `value → count` for one facet, most common first.
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

/// One indexed preset with its marks, as the query engine sees it.
pub(crate) struct Candidate<'a> {
    pub plugin_id: &'a str,
    pub plugin_order: usize,
    pub record: &'a PresetRecord,
    pub marks: PresetMarks,
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Lowercase with Latin diacritics folded (`"Café"` → `"cafe"`): the one
/// fold every library kind shares, `library_marks::vocab::fold`.
pub fn fold(s: &str) -> String {
    resonance_common::library_marks::vocab::fold(s)
}

/// Folded alphanumeric words of `s`.
fn words(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// A scoped token from the text field.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scoped {
    Genre(String),
    Tag(String),
    Category(String),
    Instrument(String),
    Character(String),
    Author(Vec<String>),
    Favorite,
    Source(PresetSource),
}

/// The text split into free prefix tokens and scoped filters.
struct ParsedText {
    free: Vec<String>,
    scoped: Vec<Scoped>,
}

fn parse_text(text: &str) -> ParsedText {
    let mut free = Vec::new();
    let mut scoped = Vec::new();
    for token in text.split_whitespace() {
        let parsed = token.split_once(':').and_then(|(key, value)| {
            let slug = || super::vocab::normalize_facet(value);
            Some(match fold(key).as_str() {
                "genre" | "genres" => Scoped::Genre(slug()?),
                "tag" | "tags" => Scoped::Tag(slug()?),
                "cat" | "category" => Scoped::Category(fold(value)),
                "for" | "instrument" => Scoped::Instrument(slug()?),
                "char" | "character" => Scoped::Character(slug()?),
                "by" | "author" => Scoped::Author(words(value)),
                "is" => match fold(value).as_str() {
                    "fav" | "favorite" | "favourite" | "starred" => Scoped::Favorite,
                    "factory" => Scoped::Source(PresetSource::Factory),
                    "user" => Scoped::Source(PresetSource::User),
                    _ => return None,
                },
                _ => return None,
            })
        });
        match parsed {
            Some(s) => scoped.push(s),
            None => free.extend(words(token)),
        }
    }
    ParsedText { free, scoped }
}

fn prefix_in(token: &str, haystack: &[String]) -> bool {
    haystack.iter().any(|w| w.starts_with(token))
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// The facets a candidate is filtered on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Facet {
    Source,
    Category,
    Instrument,
    Genres,
    Character,
    Tags,
}

const FACETS: [Facet; 6] = [
    Facet::Source,
    Facet::Category,
    Facet::Instrument,
    Facet::Genres,
    Facet::Character,
    Facet::Tags,
];

struct Prepared<'a, 'b> {
    c: &'b Candidate<'a>,
    tags: Vec<String>,
    favorite: bool,
    /// Passes text, scoped tokens and `favorites_only`.
    base: bool,
    name_hit: bool,
}

fn facet_values(p: &Prepared<'_, '_>, facet: Facet) -> Vec<String> {
    let meta = &p.c.record.meta;
    match facet {
        Facet::Source => vec![p.c.record.preset.source.as_str().to_string()],
        Facet::Category => meta.category.iter().cloned().collect(),
        Facet::Instrument => meta.instrument.clone(),
        Facet::Genres => meta.genres.clone(),
        Facet::Character => meta.character.clone(),
        Facet::Tags => p.tags.clone(),
    }
}

fn selected(q: &Query, facet: Facet) -> Vec<String> {
    let slugged = |v: &[String]| -> Vec<String> {
        v.iter()
            .filter_map(|s| super::vocab::normalize_facet(s))
            .collect()
    };
    match facet {
        Facet::Source => q.sources.iter().map(|s| s.as_str().to_string()).collect(),
        Facet::Category => q.category.iter().map(|c| fold(c.trim())).collect(),
        Facet::Instrument => slugged(&q.instrument),
        Facet::Genres => slugged(&q.genres),
        Facet::Character => slugged(&q.character),
        Facet::Tags => slugged(&q.tags),
    }
}

fn passes_facet(p: &Prepared<'_, '_>, facet: Facet, wanted: &[String]) -> bool {
    if wanted.is_empty() {
        return true;
    }
    let values = facet_values(p, facet);
    if facet == Facet::Category {
        return values.iter().any(|v| wanted.contains(&fold(v)));
    }
    values.iter().any(|v| wanted.contains(v))
}

fn prepare<'a, 'b>(c: &'b Candidate<'a>, q: &Query, text: &ParsedText) -> Prepared<'a, 'b> {
    let meta = &c.record.meta;
    let mut tags = meta.tags.clone();
    for t in &c.marks.tags {
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    let favorite = c.marks.favorite;

    let name_words = words(&meta.name);
    let mut other_words = Vec::new();
    for s in [&meta.author, &meta.description, &meta.category]
        .into_iter()
        .flatten()
    {
        other_words.extend(words(s));
    }
    for t in &tags {
        other_words.extend(words(t));
    }

    let name_hit = !text.free.is_empty() && text.free.iter().all(|t| prefix_in(t, &name_words));
    let free_ok = text
        .free
        .iter()
        .all(|t| prefix_in(t, &name_words) || prefix_in(t, &other_words));
    let scoped_ok = text.scoped.iter().all(|s| match s {
        Scoped::Genre(g) => meta.genres.contains(g),
        Scoped::Tag(t) => tags.contains(t),
        Scoped::Category(cat) => meta.category.as_deref().map(fold).as_deref() == Some(cat),
        Scoped::Instrument(i) => meta.instrument.contains(i),
        Scoped::Character(ch) => meta.character.contains(ch),
        Scoped::Author(ws) => {
            let author = meta.author.as_deref().map(words).unwrap_or_default();
            ws.iter().all(|w| prefix_in(w, &author))
        }
        Scoped::Favorite => favorite,
        Scoped::Source(src) => c.record.preset.source == *src,
    });
    let fav_ok = !q.favorites_only || favorite;

    Prepared {
        c,
        tags,
        favorite,
        base: free_ok && scoped_ok && fav_ok,
        name_hit,
    }
}

fn count_facet(prepared: &[Prepared<'_, '_>], q: &Query, facet: Facet) -> FacetCounts {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let others: Vec<(Facet, Vec<String>)> = FACETS
        .iter()
        .filter(|f| **f != facet)
        .map(|f| (*f, selected(q, *f)))
        .collect();
    for p in prepared.iter().filter(|p| p.base) {
        if others.iter().all(|(f, wanted)| passes_facet(p, *f, wanted)) {
            for v in facet_values(p, facet) {
                *counts.entry(v).or_default() += 1;
            }
        }
    }
    // A selected value stays listed at zero, so it can be unselected.
    let wanted = selected(q, facet);
    let mut out: FacetCounts = counts.into_iter().collect();
    for w in wanted {
        let listed = out.iter().any(|(v, _)| {
            if facet == Facet::Category {
                fold(v) == w
            } else {
                *v == w
            }
        });
        if !listed {
            out.push((w, 0));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Run `q` over `candidates` (already restricted to `q.plugins`).
pub(crate) fn run(candidates: &[Candidate<'_>], q: &Query) -> QueryResult {
    let text = parse_text(&q.text);
    let prepared: Vec<Prepared<'_, '_>> =
        candidates.iter().map(|c| prepare(c, q, &text)).collect();

    let facets = Facets {
        source: count_facet(&prepared, q, Facet::Source),
        category: count_facet(&prepared, q, Facet::Category),
        instrument: count_facet(&prepared, q, Facet::Instrument),
        genres: count_facet(&prepared, q, Facet::Genres),
        character: count_facet(&prepared, q, Facet::Character),
        tags: count_facet(&prepared, q, Facet::Tags),
    };

    let all_facets: Vec<(Facet, Vec<String>)> =
        FACETS.iter().map(|f| (*f, selected(q, *f))).collect();
    let mut matched: Vec<&Prepared<'_, '_>> = prepared
        .iter()
        .filter(|p| p.base && all_facets.iter().all(|(f, w)| passes_facet(p, *f, w)))
        .collect();

    let rank_names = !text.free.is_empty();
    matched.sort_by(|a, b| {
        let fav = if q.favorites_first {
            b.favorite.cmp(&a.favorite)
        } else {
            std::cmp::Ordering::Equal
        };
        let name = if rank_names {
            b.name_hit.cmp(&a.name_hit)
        } else {
            std::cmp::Ordering::Equal
        };
        fav.then(name).then_with(|| compare(a.c, b.c, q.sort))
    });

    QueryResult {
        hits: matched
            .into_iter()
            .map(|p| Hit {
                plugin_id: p.c.plugin_id.to_string(),
                record: p.c.record.clone(),
                favorite: p.favorite,
                tags: p.tags.clone(),
                marks: p.c.marks.clone(),
                name_hit: p.name_hit,
            })
            .collect(),
        facets,
    }
}

fn compare(a: &Candidate<'_>, b: &Candidate<'_>, sort: Sort) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let by_name = || fold(&a.record.meta.name).cmp(&fold(&b.record.meta.name));
    let bank = || {
        a.plugin_order
            .cmp(&b.plugin_order)
            .then_with(|| a.record.bank_order.cmp(&b.record.bank_order))
    };
    // `None` sorts after every value, whichever way the values go.
    let desc_opt = |x: &Option<String>, y: &Option<String>| match (x, y) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    match sort {
        Sort::Bank => bank(),
        Sort::Name => by_name().then_with(bank),
        Sort::Category => {
            let ca = a.record.meta.category.as_deref().map(fold);
            let cb = b.record.meta.category.as_deref().map(fold);
            match (ca, cb) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(by_name)
            .then_with(bank)
        }
        Sort::RecentlyUsed => desc_opt(&a.marks.last_used, &b.marks.last_used)
            .then_with(by_name)
            .then_with(bank),
        Sort::RecentlyModified => desc_opt(&a.record.meta.modified, &b.record.meta.modified)
            .then_with(by_name)
            .then_with(bank),
    }
}
