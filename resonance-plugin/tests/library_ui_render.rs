//! Headless render smoke test for the shared library list widgets
//! (`library_ui`) and the kit's `star_toggle` / `tag_pill`.
//!
//! Behaviour is covered in `tests/library_view.rs` against the
//! GUI-agnostic `BrowserModel`; this proves the egui skin lays out — id
//! collisions, the virtualised list, the confirm row and the tag row — on
//! a CPU-only frame (`egui::__run_test_ui`).
#![cfg(feature = "editor-widgets")]

use plugin_gui_core::egui;
use plugin_gui_core::widgets::{star_toggle, tag_pill};
use resonance_plugin::library_ui::{
    confirm_delete_row, facet_menu, library_list, search_field, tag_row, ColumnSpec,
    ConfirmOutcome, ListOptions,
};
use resonance_plugin::library_view::{BrowserModel, LibraryRows, Marks};

struct Rows {
    keys: Vec<String>,
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
    fn columns(&self, row: usize) -> Vec<String> {
        vec![format!("col {row}"), "48k".into()]
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

#[test]
fn the_list_widgets_lay_out_in_every_state() {
    let rows = Rows {
        keys: (0..1000).map(|i| format!("amp-model:{i:04}")).collect(),
        marks: Marks {
            favorite: true,
            ..Marks::default()
        },
    };
    let mut model = BrowserModel::new();
    model.refresh(&rows, 1);
    let columns = [ColumnSpec::left(80.0), ColumnSpec::right(40.0)];
    let mut draft = String::from("dj");
    let is_error = |row: usize| row == 1;

    for pending in [false, true] {
        if pending {
            model.begin_delete("amp-model:0001");
        }
        egui::__run_test_ui(|ui| {
            search_field(ui, &mut model, "search…", 200.0);
            facet_menu(ui, "kind_menu", "Kind", &mut model, &rows, "kind");
            let resp = library_list(
                ui,
                "list",
                &mut model,
                &rows,
                &ListOptions {
                    columns: &columns,
                    loaded: Some("amp-model:0002"),
                    is_error: Some(&is_error),
                    ..ListOptions::default()
                },
            );
            assert_eq!(resp.clicked, None);
            let outcome = confirm_delete_row(ui, &mut model, "Delete \"x\"?", Some("Used by 2"));
            assert_eq!(outcome, ConfirmOutcome::None);
            let tags = vec!["djent".to_string(), "rhythm".to_string()];
            let r = tag_row(ui, "tags", &tags, &mut draft, &["djent-lead".to_string()]);
            assert_eq!(r.added, None);
            assert!(!star_toggle(ui, true).clicked());
            assert!(!tag_pill(ui, "metal", true, false).clicked);
        });
    }
}
