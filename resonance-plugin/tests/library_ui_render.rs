//! Headless frames of the shared library list widgets (`library_ui`) and
//! the kit's `star_toggle` / `tag_pill`: what they actually draw, read back
//! from the frame's text shapes, and how they answer keys.
//!
//! Behaviour is covered in `tests/library_view.rs` against the
//! GUI-agnostic `BrowserModel`; this proves the egui skin lays out and
//! shows what it should, on a CPU-only frame.
#![cfg(feature = "editor-widgets")]

use std::borrow::Cow;

use plugin_gui_core::egui;
use plugin_gui_core::widgets::{star_toggle, tag_pill};
use resonance_plugin::library_ui::{
    confirm_delete_row, facet_menu, library_list, search_field, tag_row, ColumnSpec,
    ConfirmOutcome, ListOptions, ListResponse,
};
use resonance_plugin::library_view::{BrowserModel, LibraryRows, Marks};

struct Rows {
    keys: Vec<String>,
    cols: Vec<[String; 2]>,
    marks: Marks,
}

impl LibraryRows for Rows {
    fn row_count(&self) -> usize {
        self.keys.len()
    }
    fn key(&self, row: usize) -> &str {
        &self.keys[row]
    }
    fn title(&self, row: usize) -> &str {
        &self.keys[row]
    }
    fn subtitle(&self, row: usize) -> String {
        format!("sub {row}")
    }
    fn column(&self, row: usize, col: usize) -> Option<Cow<'_, str>> {
        self.cols[row].get(col).map(|s| Cow::Borrowed(s.as_str()))
    }
    fn marks(&self, row: usize) -> Option<&Marks> {
        (row % 3 == 0).then_some(&self.marks)
    }
    fn facet_names(&self) -> Vec<&str> {
        vec!["kind"]
    }
    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        match facet {
            "kind" if row % 2 == 0 => vec!["even"],
            "kind" => vec!["odd"],
            _ => vec![],
        }
    }
}

fn rows(n: usize) -> Rows {
    Rows {
        keys: (0..n).map(|i| format!("amp-model:{i:04}")).collect(),
        cols: (0..n).map(|i| [format!("col {i}"), "48k".into()]).collect(),
        marks: Marks {
            favorite: true,
            ..Marks::default()
        },
    }
}

/// Every text the frame painted.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, &mut out);
    }
    out
}

/// Run one frame of `f` at `size` with `events`, returning what was drawn.
fn frame(
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    mut f: impl FnMut(&mut egui::Ui),
) -> Vec<String> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    let out = ctx.run_ui(input, |ui| f(ui));
    texts(&out.shapes)
}

fn key(k: egui::Key) -> egui::Event {
    egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

const COLUMNS: [ColumnSpec; 2] = [ColumnSpec::left(80.0), ColumnSpec::right(40.0)];

#[test]
fn the_list_draws_only_the_visible_rows_with_their_columns() {
    let rows = rows(1000);
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    let ctx = egui::Context::default();
    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        library_list(
            ui,
            "list",
            &mut model,
            &rows,
            &ListOptions {
                columns: &COLUMNS,
                ..ListOptions::default()
            },
        );
    });
    assert!(drawn.iter().any(|t| t == "amp-model:0000"), "{drawn:?}");
    assert!(drawn.iter().any(|t| t == "col 0"));
    assert!(drawn.iter().any(|t| t == "48k"));
    assert!(!drawn.iter().any(|t| t == "amp-model:0999"), "virtualised: row 999 is not laid out");
    assert!(!drawn.iter().any(|t| t.starts_with("sub ")), "no subtitle unless asked");

    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        library_list(
            ui,
            "list",
            &mut model,
            &rows,
            &ListOptions {
                row_height: 36.0,
                show_subtitle: true,
                ..ListOptions::default()
            },
        );
    });
    assert!(drawn.iter().any(|t| t == "sub 0"), "the subtitle line: {drawn:?}");
}

#[test]
fn arrow_keys_move_the_selection_and_follow_it() {
    let rows = rows(200);
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    let ctx = egui::Context::default();
    let run = |events: Vec<egui::Event>, model: &mut BrowserModel| {
        let mut last = ListResponse::default();
        let drawn = frame(&ctx, egui::vec2(760.0, 300.0), events, |ui| {
            last = library_list(ui, "list", model, &rows, &ListOptions::default());
        });
        (drawn, last)
    };
    run(vec![], &mut model);
    // Favourites (every third row) sort first, so the view order is what
    // the keys walk, not the row order.
    let view = model.view().to_vec();
    let key_at = |pos: usize| format!("amp-model:{:04}", view[pos]);
    run(vec![key(egui::Key::ArrowDown)], &mut model);
    assert_eq!(model.selected(), Some(key_at(0).as_str()), "enters at the top");
    for _ in 0..119 {
        let (_, r) = run(vec![key(egui::Key::ArrowDown)], &mut model);
        assert!(r.moved.is_some());
    }
    assert_eq!(model.selected(), Some(key_at(119).as_str()));
    // The list scrolled: that row is on screen now.
    let (drawn, _) = run(vec![], &mut model);
    assert!(drawn.iter().any(|t| *t == key_at(119)), "{drawn:?}");
    let (_, last) = run(vec![key(egui::Key::ArrowUp)], &mut model);
    assert_eq!(last.moved, Some(view[118]));
    let (_, last) = run(vec![key(egui::Key::Enter)], &mut model);
    assert_eq!(last.double_clicked, Some(view[118]), "Enter activates the selection");
    let (_, last) = run(vec![key(egui::Key::Escape)], &mut model);
    assert!(last.escaped);
}

#[test]
fn the_confirm_row_shows_only_on_the_row_that_was_armed() {
    let rows = rows(3);
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    model.select("amp-model:0000");
    model.begin_delete("amp-model:0000");
    let ctx = egui::Context::default();
    let mut outcome = ConfirmOutcome::None;
    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        outcome = confirm_delete_row(ui, &mut model, "amp-model:0000", "Delete \"a\"?", Some("Used by 2"));
    });
    assert!(drawn.iter().any(|t| t == "Delete \"a\"?"));
    assert!(drawn.iter().any(|t| t == "Used by 2"));
    assert_eq!(outcome, ConfirmOutcome::None);

    // The detail pane now shows another row: no confirm there.
    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        confirm_delete_row(ui, &mut model, "amp-model:0001", "Delete \"b\"?", None);
    });
    assert!(!drawn.iter().any(|t| t.starts_with("Delete")), "{drawn:?}");
    // And selecting it disarms the delete altogether.
    model.select("amp-model:0001");
    assert_eq!(model.pending_delete(), None);
}

#[test]
fn tag_suggestions_stay_while_the_draft_is_non_empty() {
    let ctx = egui::Context::default();
    let mut draft = String::from("dj");
    let tags = vec!["rhythm".to_string()];
    let suggestions = vec!["djent".to_string()];
    // No focus on the field (a click on a suggestion takes it away).
    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        tag_row(ui, "tags", &tags, &mut draft, &suggestions);
    });
    assert!(drawn.iter().any(|t| t == "djent"), "{drawn:?}");
    assert!(drawn.iter().any(|t| t == "rhythm"));
    draft.clear();
    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        tag_row(ui, "tags", &tags, &mut draft, &suggestions);
    });
    assert!(!drawn.iter().any(|t| t == "djent"));
}

#[test]
fn the_facet_menu_and_kit_widgets_lay_out() {
    let rows = rows(10);
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    model.toggle_facet("kind", "even");
    model.refresh(&rows, 1);
    let ctx = egui::Context::default();
    let drawn = frame(&ctx, egui::vec2(760.0, 520.0), vec![], |ui| {
        search_field(ui, &mut model, "search…", 200.0);
        facet_menu(ui, "kind_menu", "Kind", &mut model, &rows, "kind");
        assert!(!star_toggle(ui, true).clicked());
        assert!(!tag_pill(ui, "metal", true, false).clicked);
    });
    assert!(drawn.iter().any(|t| t == "Kind: even"), "one ▾ (the combo's own): {drawn:?}");
    assert!(drawn.iter().any(|t| t == "metal"));
}
