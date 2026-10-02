//! `kit_rows::KitRows` over a real `drumkit_library` in a temporary root,
//! through the shared `BrowserModel`: search (names, pieces, mics, tags),
//! the source / mics / articulations facets, favourites first, columns,
//! and ◀/▶ over the view (drums-plugin-rework.md §3.4, §6.5).

use std::path::{Path, PathBuf};

use resonance_common::drumkit_library::{
    self, write_sidecar, Library, Sidecar, MANIFEST_FILE, SOURCE_IMPORTED, SOURCE_PLOK,
};
use resonance_common::library_marks::MarksStore;
use resonance_plugin::kit_rows::{
    self, mics_bucket, KitRows, ARTICULATIONS_FACET, COLUMNS, FACETS, MICS_FACET,
};
use resonance_plugin::library_view::{BrowserModel, LibraryRows, Sort, SortKey, SOURCE_FACET};

/// The temporary dirs this test thread made; removed when the thread (the
/// test) ends, pass or fail.
struct TempDirs(Vec<PathBuf>);

impl Drop for TempDirs {
    fn drop(&mut self) {
        for d in &self.0 {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

thread_local! {
    static TEMP_DIRS: std::cell::RefCell<TempDirs> =
        const { std::cell::RefCell::new(TempDirs(Vec::new())) };
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance-kitrows-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TEMP_DIRS.with(|t| t.borrow_mut().0.push(dir.clone()));
    dir
}

/// A manifest with `pieces` (key, display name) and `mics` mic setups of
/// `brand`; samples are referenced, never written (rows never open them).
fn write_kit(dir: &Path, pieces: &[(&str, &str)], mics: usize, brand: &str, articulations: bool) {
    std::fs::create_dir_all(dir).unwrap();
    let mut obj = serde_json::Map::new();
    let mut meta_pieces = serde_json::Map::new();
    for (key, name) in pieces {
        let mut setups = serde_json::Map::new();
        for m in 0..mics {
            setups.insert(
                format!("{m:02}_Pos{m}_M{m}"),
                serde_json::json!({
                    "brand": brand, "channel": format!("{m:02}"), "mic": format!("M{m}"),
                    "position": format!("Pos{m}"),
                    "rounds": {"RR01": {"Vel01": format!("{key} {m}.wav")}},
                }),
            );
        }
        obj.insert(key.to_string(), setups.into());
        meta_pieces.insert(key.to_string(), serde_json::json!({ "name": name }));
    }
    let mut meta = serde_json::json!({ "pieces": meta_pieces });
    if articulations {
        meta["articulations"] = serde_json::json!([
            {"primary": pieces[0].0, "alt": pieces[1].0, "label": "punch/deep"}
        ]);
    }
    obj.insert("_meta".into(), meta);
    std::fs::write(
        dir.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&obj).unwrap(),
    )
    .unwrap();
}

struct Fixture {
    root: PathBuf,
    lib: Library,
    marks: MarksStore,
}

fn fixture(tag: &str) -> Fixture {
    let root = temp_root(tag);
    write_kit(
        &root.join("Drummica").join("drummica"),
        &[
            ("SD Kick mit Teppich", "Kick"),
            ("SD Kick ohne Teppich", "Kick (body)"),
            ("SD Snare", "Snare"),
        ],
        6,
        "Neumann",
        true,
    );
    write_sidecar(
        &root.join("Drummica"),
        &Sidecar {
            source: SOURCE_PLOK.into(),
            index_name: Some("Drummica".into()),
            index_tags: vec!["rock".into(), "acoustic".into()],
            description: Some("Acoustic studio kit".into()),
            size_bytes: Some(9_126_805_504),
            ..Sidecar::default()
        },
    )
    .unwrap();
    write_kit(
        &root.join("IT Techno").join("ittechno"),
        &[
            ("SD Count Stick", "Perc Conga"),
            ("SD Snare Handtuch", "Clap"),
        ],
        1,
        "Sennheiser",
        false,
    );
    write_kit(
        &root.join("Middle"),
        &[("A", "Tom"), ("B", "Ride")],
        3,
        "Shure",
        false,
    );
    write_sidecar(
        &root.join("Middle"),
        &Sidecar {
            source: SOURCE_IMPORTED.into(),
            ..Sidecar::default()
        },
    )
    .unwrap();
    let lib = Library::open_and_scan(&root).unwrap();
    // The marks store lives outside the kit root, as it does for real.
    let marks = MarksStore::open(temp_root(&format!("{tag}-marks")).join("marks")).unwrap();
    Fixture { root, lib, marks }
}

fn titles(model: &BrowserModel, rows: &KitRows) -> Vec<String> {
    model
        .view()
        .iter()
        .map(|&r| rows.title(r).to_string())
        .collect()
}

fn view_for(rows: &KitRows, query: &str) -> Vec<String> {
    let mut model = BrowserModel::new();
    model.set_query(query);
    model.refresh(rows, 1);
    titles(&model, rows)
}

#[test]
fn rows_show_counts_source_and_size() {
    let f = fixture("columns");
    let rows = KitRows::build(&f.lib, None, (1, 0));
    assert_eq!(rows.row_count(), 3);
    let row = |name: &str| {
        (0..rows.row_count())
            .find(|&r| rows.title(r) == name)
            .unwrap()
    };
    let cols = |r: usize| -> Vec<String> {
        (0..COLUMNS.len())
            .map(|c| rows.column(r, c).unwrap().into_owned())
            .collect()
    };
    let d = row("Drummica");
    assert_eq!(
        cols(d),
        ["3 pieces", "6 setups", "1 layer", "1 RR", "plok", "8.5 GB"]
    );
    assert!(rows.column(d, COLUMNS.len()).is_none());
    let t = row("IT Techno");
    assert_eq!(
        cols(t),
        ["2 pieces", "1 setup", "1 layer", "1 RR", "local", "…"]
    );
    assert_eq!(
        rows.key(d),
        drumkit_library::mark_key(&rows.rows[d].entry.id)
    );
    assert_eq!(rows.subtitle(t), "local · 2 samples");
}

#[test]
fn search_matches_names_pieces_mics_and_tags() {
    let f = fixture("search");
    let rows = KitRows::build(&f.lib, None, (1, 0));
    assert_eq!(
        view_for(&rows, "conga"),
        ["IT Techno"],
        "a piece display name"
    );
    assert_eq!(view_for(&rows, "neumann"), ["Drummica"], "a mic brand");
    assert_eq!(view_for(&rows, "studio"), ["Drummica"], "the description");
    assert_eq!(view_for(&rows, "tag:rock"), ["Drummica"], "an index tag");
    assert_eq!(view_for(&rows, "middle"), ["Middle"], "the directory name");
    assert_eq!(view_for(&rows, "is:plok"), ["Drummica"]);
    assert_eq!(view_for(&rows, "is:imported"), ["Middle"]);
    assert_eq!(view_for(&rows, "articulations:yes"), ["Drummica"]);
    assert_eq!(view_for(&rows, "mics:1"), ["IT Techno"]);
}

#[test]
fn facets_bucket_mics_and_flag_articulations() {
    let f = fixture("facets");
    let rows = KitRows::build(&f.lib, None, (1, 0));
    assert_eq!(
        FACETS.iter().map(|(f, _)| *f).collect::<Vec<_>>(),
        [SOURCE_FACET, MICS_FACET, ARTICULATIONS_FACET]
    );
    assert_eq!(
        rows.facet_names(),
        [SOURCE_FACET, MICS_FACET, ARTICULATIONS_FACET]
    );
    assert_eq!(
        [1, 2, 4, 5, 14].map(mics_bucket),
        ["1", "2–4", "2–4", "5+", "5+"]
    );

    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    let counts = |model: &BrowserModel, facet: &str| -> Vec<(String, usize)> {
        let mut v: Vec<_> = model
            .facet_counts(&rows, facet)
            .into_iter()
            .map(|c| (c.value, c.count))
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        counts(&model, MICS_FACET),
        [
            ("1".to_string(), 1),
            ("2–4".to_string(), 1),
            ("5+".to_string(), 1)
        ]
    );
    assert_eq!(
        counts(&model, ARTICULATIONS_FACET),
        [("no".to_string(), 2), ("yes".to_string(), 1)]
    );
    assert_eq!(counts(&model, SOURCE_FACET).len(), 3);

    model.toggle_facet(MICS_FACET, "5+");
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), ["Drummica"]);
    model.clear_facets();
    model.toggle_facet(ARTICULATIONS_FACET, "no");
    model.set_sort(Sort::by(SortKey::Title));
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), ["IT Techno", "Middle"]);
}

#[test]
fn favourites_sort_first_and_steps_follow_the_view() {
    let mut f = fixture("favs");
    let techno = f.lib.by_dir(&f.root.join("IT Techno")).unwrap().clone();
    f.marks.set_favorite(&techno.mark_key(), true).unwrap();
    let rows = KitRows::build(&f.lib, Some(&f.marks), (1, f.marks.generation()));
    assert!(rows
        .marks_of(
            rows.rows
                .iter()
                .position(|r| r.entry.id == techno.id)
                .unwrap()
        )
        .is_some_and(|m| m.favorite));

    let mut model = BrowserModel::new();
    model.set_sort(Sort::by(SortKey::Title));
    model.refresh(&rows, 1);
    assert_eq!(titles(&model, &rows), ["IT Techno", "Drummica", "Middle"]);
    assert_eq!(view_for(&rows, "is:fav"), ["IT Techno"]);

    // ◀/▶ walk the view's order and clamp; the counter follows.
    let slot_of = |name: &str| f.lib.find(name).unwrap().slot;
    let drummica = f.lib.find("Drummica").unwrap().id.clone();
    assert_eq!(
        kit_rows::step_in_view(&model, &rows, Some(&techno.id), 1),
        slot_of("Drummica")
    );
    assert_eq!(
        kit_rows::step_in_view(&model, &rows, Some(&drummica), 1),
        slot_of("Middle")
    );
    assert_eq!(
        kit_rows::step_in_view(&model, &rows, Some(&techno.id), -1),
        None
    );
    assert_eq!(
        kit_rows::step_in_view(&model, &rows, None, -1),
        slot_of("Middle")
    );
    assert_eq!(
        kit_rows::view_counter(&model, Some(&drummica)),
        "2 / 3 in view"
    );
    assert_eq!(
        kit_rows::view_counter(&model, Some("nope")),
        "– / 3 in view"
    );

    // Sort options cover the columns a user would sort by.
    let labels: Vec<_> = kit_rows::sort_options()
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert!(labels.contains(&"Size") && labels.contains(&"Recently added"));
}

#[test]
fn a_broken_kit_says_why_and_is_stepped_over() {
    let f = fixture("broken");
    std::fs::create_dir_all(f.root.join("Broken")).unwrap();
    std::fs::write(f.root.join("Broken").join(MANIFEST_FILE), b"[1, 2]").unwrap();
    let lib = Library::open_and_scan(&f.root).unwrap();
    let rows = KitRows::build(&lib, None, (1, 0));
    let b = (0..rows.row_count())
        .find(|&r| rows.title(r) == "Broken")
        .unwrap();
    assert!(rows.column(b, 0).unwrap().starts_with("manifest error:"));

    let mut model = BrowserModel::new();
    model.set_sort(Sort::by(SortKey::Title));
    model.refresh(&rows, 1);
    // Broken sorts first by title; ▶ from nothing skips it.
    assert_eq!(titles(&model, &rows)[0], "Broken");
    assert_eq!(
        kit_rows::step_in_view(&model, &rows, None, 1),
        lib.find("Drummica").unwrap().slot
    );
    // A zero step from the unloadable row returns at once (it looped).
    let broken_id = rows.rows[b].entry.id.clone();
    assert_eq!(
        kit_rows::step_in_view(&model, &rows, Some(&broken_id), 0),
        None
    );
}
