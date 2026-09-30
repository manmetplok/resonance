//! `library_view::BrowserModel` over a fake `LibraryRows`: the one browser
//! model the plugin preset browser and the NAM model manager share
//! (plugin-preset-library.md §14, nam-model-library.md §11).

use std::collections::HashMap;

use resonance_plugin::library_view::{
    Audition, AuditionBracket, AuditionEvent, BrowserModel, LibraryRows, Marks, Sort, SortKey,
    SortValue,
};

struct Item {
    key: &'static str,
    title: &'static str,
    author: &'static str,
    gear_type: &'static str,
    genres: Vec<&'static str>,
    content_tags: Vec<&'static str>,
    size: f64,
}

struct Rows {
    items: Vec<Item>,
    marks: HashMap<&'static str, Marks>,
}

impl LibraryRows for Rows {
    fn row_count(&self) -> usize {
        self.items.len()
    }
    fn key(&self, row: usize) -> &str {
        self.items[row].key
    }
    fn title(&self, row: usize) -> &str {
        self.items[row].title
    }
    fn marks(&self, row: usize) -> Option<&Marks> {
        self.marks.get(self.items[row].key)
    }
    fn search_text(&self, row: usize) -> Vec<&str> {
        vec![self.items[row].author]
    }
    fn facet_names(&self) -> Vec<&str> {
        vec!["gear_type", "genres", "author"]
    }
    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        let it = &self.items[row];
        match facet {
            "gear_type" => vec![it.gear_type],
            "genres" => it.genres.clone(),
            "author" => vec![it.author],
            "tags" => it.content_tags.clone(),
            _ => vec![],
        }
    }
    fn sort_value(&self, row: usize, field: &str) -> SortValue {
        match field {
            "size" => SortValue::Number(self.items[row].size),
            "author" => SortValue::Text(self.items[row].author.to_string()),
            _ => SortValue::None,
        }
    }
}

fn item(
    key: &'static str,
    title: &'static str,
    author: &'static str,
    gear_type: &'static str,
    genres: &[&'static str],
    size: f64,
) -> Item {
    Item {
        key,
        title,
        author,
        gear_type,
        genres: genres.to_vec(),
        content_tags: vec![],
        size,
    }
}

fn fav() -> Marks {
    Marks {
        favorite: true,
        ..Marks::default()
    }
}

fn rows() -> Rows {
    let mut items = vec![
        item("amp-model:a", "Friedman BE-100", "J. Smith", "amp", &["rock"], 4.1),
        item("amp-model:b", "Darkglass MT900", "Steve", "amp_cab", &["metal"], 3.2),
        item("amp-model:c", "5150 Block Letter", "tonekid", "amp", &["metal", "post-metal"], 0.4),
        item("amp-model:d", "Café Clean", "Zoë", "pedal", &[], 2.9),
        item("amp-model:e", "Old Marshall", "tonekid", "amp", &["rock"], 1.0),
    ];
    items[3].content_tags = vec!["jazzy"];
    let mut marks = HashMap::new();
    marks.insert("amp-model:c", fav());
    marks.insert(
        "amp-model:e",
        Marks {
            tags: vec!["rhythm".into()],
            last_used: Some(200),
            ..Marks::default()
        },
    );
    marks.insert(
        "amp-model:a",
        Marks {
            last_used: Some(100),
            ..Marks::default()
        },
    );
    Rows { items, marks }
}

fn titles(model: &BrowserModel, rows: &Rows) -> Vec<&'static str> {
    model.view().iter().map(|&r| rows.items[r].title).collect()
}

#[test]
fn the_default_view_is_favourites_first_then_natural_order() {
    let rows = rows();
    let mut model = BrowserModel::new();
    assert!(model.refresh(&rows, 1));
    assert_eq!(
        titles(&model, &rows),
        vec![
            "5150 Block Letter",
            "Friedman BE-100",
            "Darkglass MT900",
            "Café Clean",
            "Old Marshall"
        ]
    );
    assert!(!model.refresh(&rows, 1), "nothing changed: the cached view is kept");
    assert!(model.refresh(&rows, 2), "a new revision recomputes");
}

#[test]
fn favourites_first_holds_within_every_sort() {
    let rows = rows();
    let mut model = BrowserModel::new();
    model.set_sort(Sort::by(SortKey::Title));
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows)[0], "5150 Block Letter");
    assert_eq!(
        &titles(&model, &rows)[1..],
        &["Café Clean", "Darkglass MT900", "Friedman BE-100", "Old Marshall"]
    );

    model.set_favorites_first(false);
    model.set_sort(Sort {
        key: SortKey::Field("size".into()),
        descending: true,
    });
    model.refresh(&rows, 1);
    assert_eq!(
        titles(&model, &rows),
        vec![
            "Friedman BE-100",
            "Darkglass MT900",
            "Café Clean",
            "Old Marshall",
            "5150 Block Letter"
        ]
    );
}

#[test]
fn recently_used_sorts_newest_first_and_never_used_last() {
    let rows = rows();
    let mut model = BrowserModel::new();
    model.set_favorites_first(false);
    model.set_sort(Sort::by(SortKey::RecentlyUsed));
    model.refresh(&rows, 1);
    let t = titles(&model, &rows);
    assert_eq!(&t[..2], &["Old Marshall", "Friedman BE-100"]);
    model.set_recent_only(true);
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Old Marshall", "Friedman BE-100"]);
}

#[test]
fn search_is_case_and_accent_insensitive_over_title_text_and_tags() {
    let rows = rows();
    let mut model = BrowserModel::new();
    model.set_query("cafe");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Café Clean"]);

    model.set_query("ZOE");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Café Clean"], "search_text (author) is searched");

    model.set_query("rhythm");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Old Marshall"], "personal tags are searched");

    model.set_query("jazzy");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Café Clean"], "content tags are searched");

    model.set_query("tonekid old");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Old Marshall"], "tokens are ANDed");
}

#[test]
fn scoped_tokens_filter_by_field() {
    let rows = rows();
    let mut model = BrowserModel::new();
    model.set_query("is:fav");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["5150 Block Letter"]);

    model.set_query("by:tonekid");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["5150 Block Letter", "Old Marshall"]);

    model.set_query("genre:post");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["5150 Block Letter"]);

    model.set_query("gear_type:pedal");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Café Clean"]);

    model.set_query("tag:rhy is:recent");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Old Marshall"]);

    model.set_query("nope:friedman");
    model.refresh(&rows, 1);
    assert!(titles(&model, &rows).is_empty(), "an unknown scope is plain text");
}

#[test]
fn the_search_syntax_is_the_preset_librarys_superset() {
    use resonance_plugin::library_view::{parse_search, SearchToken};
    let facet = |f: &str, v: &str| SearchToken::Facet {
        facet: f.into(),
        value: v.into(),
    };
    assert_eq!(
        parse_search("cat:Bass for:vocal char:dark is:user is:starred genres:metal by:Jo", &[]),
        vec![
            facet("category", "bass"),
            facet("instrument", "vocal"),
            facet("character", "dark"),
            facet("source", "user"),
            SearchToken::Favorite,
            facet("genres", "metal"),
            facet("author", "jo"),
        ]
    );
    assert_eq!(
        parse_search("http://x gear_type:amp empty:", &["gear_type"]),
        vec![
            SearchToken::Text("http://x".into()),
            facet("gear_type", "amp"),
            SearchToken::Text("empty:".into()),
        ]
    );
}

#[test]
fn one_slug_rule_for_marks_and_preset_metadata() {
    // The preset library has no slug rule of its own any more: its
    // metadata normaliser is library_marks::normalize_tag, so a tag typed
    // in one browser matches the same tag set in the other.
    use resonance_plugin::library_marks::normalize_tag;
    use resonance_plugin::presets::PresetMeta;
    for raw in [
        "R&B", "Drum & Bass", "  Djent Rhythm ", "Café_Crème", "lo-fi", "--x--", "!!!", "a/b",
        "Shoegaze", "ÅÄÖ", "0123456789012345678901234567890123456789", "Dvořák", "Ďábel",
        "Şahin",
    ] {
        let meta = PresetMeta {
            tags: vec![raw.to_string()],
            ..PresetMeta::default()
        }
        .normalized();
        assert_eq!(meta.tags.first().cloned(), normalize_tag(raw), "{raw:?}");
    }
    assert_eq!(normalize_tag("R&B").as_deref(), Some("r-b"));
    assert_eq!(normalize_tag("Dvořák").as_deref(), Some("dvorak"));
    assert_eq!(normalize_tag("Ďábel").as_deref(), Some("dabel"));
    assert_eq!(normalize_tag("Şahin").as_deref(), Some("sahin"));
}

#[test]
fn facets_or_within_and_across_with_counts_on_the_other_facets() {
    let rows = rows();
    let mut model = BrowserModel::new();
    model.toggle_facet("genres", "metal");
    model.toggle_facet("genres", "rock");
    model.refresh(&rows, 1);
    assert_eq!(model.view_len(), 4, "OR within a facet");

    model.toggle_facet("gear_type", "amp_cab");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), vec!["Darkglass MT900"], "AND across facets");
    assert_eq!(model.active_filter_count(), 3);

    // gear_type counts are computed with the genres selection applied but
    // not gear_type's own.
    let gear = model.facet_counts(&rows, "gear_type");
    let get = |v: &str| gear.iter().find(|c| c.value == v).map(|c| (c.count, c.selected));
    assert_eq!(get("amp"), Some((3, false)));
    assert_eq!(get("amp_cab"), Some((1, true)));
    assert_eq!(get("pedal"), None, "the pedal has no genre, so it is filtered out");

    // A selected value stays listed at zero.
    model.set_query("friedman");
    model.refresh(&rows, 1);
    let gear = model.facet_counts(&rows, "gear_type");
    assert!(gear.iter().any(|c| c.value == "amp_cab" && c.count == 0 && c.selected));

    // Seeded vocabulary values list first, in their declared order.
    model.set_query("");
    model.clear_facets();
    let genres: Vec<String> = model
        .facet_counts(&rows, "genres")
        .into_iter()
        .map(|c| c.value)
        .collect();
    assert_eq!(genres, vec!["metal", "post-metal", "rock"]);
}

#[test]
fn stepping_walks_the_view_and_clamps() {
    let rows = rows();
    let mut model = BrowserModel::new();
    model.toggle_facet("genres", "metal");
    model.refresh(&rows, 1);
    // View: 5150 (fav) then Darkglass.
    let key = |r: usize| rows.items[r].key;
    let first = model.step_from(None, 1).unwrap();
    assert_eq!(key(first), "amp-model:c");
    assert_eq!(key(model.step_from(None, -1).unwrap()), "amp-model:b");
    assert_eq!(key(model.step_from(Some("amp-model:c"), 1).unwrap()), "amp-model:b");
    assert_eq!(model.step_from(Some("amp-model:b"), 1), None, "clamped at the end");
    assert_eq!(model.step_from(Some("amp-model:c"), -1), None, "clamped at the start");
    assert_eq!(
        key(model.step_from(Some("amp-model:a"), 1).unwrap()),
        "amp-model:c",
        "a current item outside the view is an entry point"
    );
    assert_eq!(model.position_in_view("amp-model:b"), Some(1));
    assert_eq!(model.position_in_view("amp-model:a"), None);

    assert_eq!(model.move_selection(&rows, 1).map(key), Some("amp-model:c"));
    assert_eq!(model.move_selection(&rows, 1).map(key), Some("amp-model:b"));
    assert_eq!(model.selected(), Some("amp-model:b"));
    assert_eq!(model.move_selection(&rows, 1), None);
    assert_eq!(model.selected(), Some("amp-model:b"));

    let mut empty = BrowserModel::new();
    empty.set_query("zzz");
    empty.refresh(&rows, 1);
    assert_eq!(empty.step_from(None, 1), None);
}

#[test]
fn selection_is_keyed_so_it_survives_a_rescan() {
    let mut rows = rows();
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    model.select("amp-model:d");
    assert_eq!(model.selected_row(), Some(3));
    rows.items.remove(0);
    model.refresh(&rows, 2);
    assert_eq!(model.selected_row(), Some(2), "same item at its new index");
    rows.items.retain(|i| i.key != "amp-model:d");
    model.refresh(&rows, 3);
    assert_eq!(model.selected_row(), None);
}

#[test]
fn delete_is_confirmed_in_place() {
    let mut model = BrowserModel::new();
    assert_eq!(model.confirm_delete(), None, "no pending delete, nothing to confirm");
    model.select("amp-model:a");
    model.begin_delete("amp-model:a");
    assert_eq!(model.pending_delete(), Some("amp-model:a"));
    model.cancel_delete();
    assert_eq!(model.pending_delete(), None);
    assert_eq!(model.selected(), Some("amp-model:a"), "cancel keeps the selection");

    model.begin_delete("amp-model:a");
    assert_eq!(model.confirm_delete().as_deref(), Some("amp-model:a"));
    assert_eq!(model.pending_delete(), None);
    assert_eq!(model.selected(), None, "the deleted row is no longer selected");
}

#[test]
fn a_pending_delete_never_survives_a_change_of_selection_or_its_row() {
    let mut rows = rows();
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    model.select("amp-model:a");
    model.begin_delete("amp-model:a");
    model.select("amp-model:a");
    assert_eq!(model.pending_delete(), Some("amp-model:a"), "re-selecting the same row keeps it");
    model.select("amp-model:b");
    assert_eq!(model.pending_delete(), None, "another row disarms it");

    model.begin_delete("amp-model:b");
    let _ = model.move_selection(&rows, 1);
    assert_eq!(model.pending_delete(), None, "moving with the keys disarms it too");

    model.select("amp-model:c");
    model.begin_delete("amp-model:c");
    rows.items.retain(|i| i.key != "amp-model:c");
    model.refresh(&rows, 2);
    assert_eq!(model.pending_delete(), None, "its row went away");
    assert_eq!(model.confirm_delete(), None);
}

#[test]
fn a_pair_of_counters_is_compared_as_a_pair() {
    let rows = rows();
    let mut model = BrowserModel::new();
    assert!(model.refresh(&rows, (1, 2)));
    assert!(!model.refresh(&rows, (1, 2)));
    // (2, 1) would fold to the same number as (1, 2) under many hashes;
    // as a pair it is simply different.
    assert!(model.refresh(&rows, (2, 1)));
    assert!(model.refresh(&rows, 7u64));
}

#[test]
fn one_fold_for_search_slugs_and_preset_queries() {
    use resonance_plugin::library_marks::vocab::fold;
    use resonance_plugin::presets::{FactoryEntry, PresetLibrary, Query};
    // A preset query searches through the same engine and fold.
    let lib = PresetLibrary::new();
    lib.register_factory_entries(
        "com.test",
        [FactoryEntry {
            id: "d".into(),
            name: "Dvořák Łódź".into(),
            json: r#"{"params":{}}"#.into(),
        }],
    );
    for text in ["dvorak", "LODZ", "Dvořák"] {
        let q = Query {
            text: text.into(),
            ..Query::plugin("com.test")
        };
        assert_eq!(lib.query(&q).hits.len(), 1, "{text:?}");
    }
    for (raw, want) in [
        ("Dvořák", "dvorak"),
        ("Ďábel", "dabel"),
        ("Şahin", "sahin"),
        ("Łódź", "lodz"),
        ("Café Crème", "cafe creme"),
    ] {
        assert_eq!(fold(raw), want, "{raw:?}");
    }
}

#[test]
fn notices_carry_their_kind() {
    let mut model = BrowserModel::new();
    model.set_info("already in library");
    assert!(!model.notice().unwrap().is_error());
    model.set_error("parse error");
    assert_eq!(model.notice().unwrap().text(), "parse error");
    assert!(model.notice().unwrap().is_error());
    model.clear_notice();
    assert!(model.notice().is_none());
}

/// A plugin stand-in: "state" is a value plus an extra key.
#[derive(Default)]
struct Target {
    value: f32,
    extra: String,
    applied: Vec<String>,
}

impl Audition for Target {
    type Snapshot = (f32, String);
    fn capture(&mut self) -> (f32, String) {
        (self.value, self.extra.clone())
    }
    fn apply(&mut self, key: &str) -> Result<(), String> {
        if key == "broken" {
            return Err("unreadable".into());
        }
        self.applied.push(key.to_string());
        self.value = key.len() as f32;
        self.extra = format!("extra-{key}");
        Ok(())
    }
    fn restore(&mut self, (value, extra): (f32, String)) {
        self.value = value;
        self.extra = extra;
    }
}

#[test]
fn audition_then_revert_restores_the_exact_prior_state() {
    let mut target = Target {
        value: 0.25,
        extra: "model.nam".into(),
        ..Target::default()
    };
    let mut bracket = AuditionBracket::new();
    assert!(!bracket.is_open());
    assert_eq!(bracket.audition(&mut target, "one"), AuditionEvent::Auditioned("one".into()));
    assert_eq!(bracket.audition(&mut target, "one"), AuditionEvent::None, "same key: no reload");
    assert_eq!(bracket.audition(&mut target, "three"), AuditionEvent::Auditioned("three".into()));
    assert_eq!(bracket.provisional(), Some("three"));
    assert_eq!(bracket.revert(&mut target), AuditionEvent::Reverted);
    assert_eq!((target.value, target.extra.as_str()), (0.25, "model.nam"));
    assert!(!bracket.is_open());
    assert_eq!(bracket.revert(&mut target), AuditionEvent::None);
}

#[test]
fn audition_then_commit_keeps_the_last_provisional_load() {
    let mut target = Target::default();
    let mut bracket = AuditionBracket::new();
    bracket.audition(&mut target, "one");
    bracket.audition(&mut target, "four");
    assert_eq!(bracket.commit(), AuditionEvent::Committed("four".into()));
    assert_eq!(target.extra, "extra-four");
    assert!(!bracket.is_open());
    assert_eq!(bracket.commit(), AuditionEvent::None);

    // A failed apply leaves the bracket open and revertible.
    let mut target = Target {
        value: 1.0,
        ..Target::default()
    };
    let mut bracket = AuditionBracket::new();
    assert_eq!(
        bracket.audition(&mut target, "broken"),
        AuditionEvent::Failed("unreadable".into())
    );
    assert!(bracket.is_open());
    assert_eq!(bracket.revert(&mut target), AuditionEvent::Reverted);
    assert_eq!(target.value, 1.0);
}
