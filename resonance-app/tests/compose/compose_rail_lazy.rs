//! The Compose right rail is a `lazy` region (code review UX-10). Its key
//! is a revision over an owned snapshot of what the rail reads
//! (`view::ui_caches::RevisionMemo`), so it must rebuild after *any*
//! change to that state — a UI edit, an undo, a lane switch — and must not
//! keep showing the pre-edit tree. These tests drive iced's own
//! `UserInterface` across frames with one shared cache (a `Simulator`
//! starts from a fresh cache each time, which would hide a stale lazy).

use iced::{Rectangle, Size};
use iced_test::core::renderer::Headless;
use iced_test::core::widget::operation::{Operation, Outcome};
use iced_test::core::widget::Id;
use iced_test::runtime::user_interface::{Cache, UserInterface};
use resonance_app::compose::{ComposeMessage, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance};

const WINDOW: Size = Size::new(1440.0, 900.0);

fn renderer() -> iced::Renderer {
    iced_test::futures::futures::executor::block_on(iced::Renderer::new(
        iced::Font::with_name("Fira Sans"),
        iced::Pixels(16.0),
        None,
    ))
    .expect("headless renderer")
}

/// Collects every text fragment in the tree.
#[derive(Default)]
struct Texts(Vec<String>);

impl Operation for Texts {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn text(&mut self, _id: Option<&Id>, _bounds: Rectangle, text: &str) {
        self.0.push(text.to_string());
    }

    fn finish(&self) -> Outcome<()> {
        Outcome::None
    }
}

/// One frame: build the view against `cache`, collect its texts, and hand
/// the cache back for the next frame.
fn frame(app: &Resonance, renderer: &mut iced::Renderer, cache: Cache) -> (Vec<String>, Cache) {
    let mut ui = UserInterface::build(app.view(), WINDOW, cache, renderer);
    let mut texts = Texts::default();
    ui.operate(renderer, &mut texts);
    (texts.0, ui.into_cache())
}

fn shows(texts: &[String], needle: &str) -> bool {
    texts.iter().any(|t| t == needle)
}

fn build_app() -> (Resonance, u64) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-rail-lazy-test"));
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(SelectedLane::Chords)));
    let def = app
        .compose_state()
        .selected_placement()
        .expect("demo seeds a selected placement")
        .definition_id;
    (app, def)
}

fn section_name(app: &Resonance, def: u64) -> String {
    app.compose_state()
        .find_definition(def)
        .expect("definition exists")
        .name
        .clone()
}

#[test]
fn an_edit_and_its_undo_both_reach_the_lazy_rail() {
    let (mut app, def) = build_app();
    let mut renderer = renderer();
    let original = section_name(&app, def);

    // Two frames on the same state: the second reuses the lazy rail.
    let (texts, cache) = frame(&app, &mut renderer, Cache::default());
    assert!(shows(&texts, &original), "rail shows EDITING SECTION · {original}");
    let (texts, cache) = frame(&app, &mut renderer, cache);
    assert!(shows(&texts, &original));

    // Rename the section: the rail's header must follow.
    let _ = app.update(Message::Compose(ComposeMessage::RenameSection {
        definition_id: def,
        name: "Bridge Zeta".to_string(),
    }));
    let (texts, cache) = frame(&app, &mut renderer, cache);
    assert!(shows(&texts, "Bridge Zeta"), "an edit must rebuild the rail");
    assert!(!shows(&texts, &original), "the pre-edit rail must not linger");

    // Undo restores the definition from a snapshot (no ComposeMessage):
    // the rail must still rebuild.
    let _ = app.update(Message::Undo);
    assert_eq!(section_name(&app, def), original, "precondition: undo reverted the rename");
    let (texts, _cache) = frame(&app, &mut renderer, cache);
    assert!(shows(&texts, &original), "undo must rebuild the rail");
    assert!(!shows(&texts, "Bridge Zeta"), "the undone name must not linger");
}

#[test]
fn switching_lanes_rebuilds_the_lazy_rail() {
    let (mut app, _def) = build_app();
    let mut renderer = renderer();

    let (texts, cache) = frame(&app, &mut renderer, Cache::default());
    assert!(shows(&texts, "EDITING SECTION"));

    let drums = app
        .test_tracks()
        .iter()
        .find(|t| t.name == "Drums")
        .expect("demo has a Drums track")
        .id;
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(SelectedLane::Drums(drums))));
    let (texts, cache) = frame(&app, &mut renderer, cache);
    assert!(shows(&texts, "EDITING TRACK"), "lane switch must rebuild the rail");

    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(SelectedLane::Chords)));
    let (texts, _cache) = frame(&app, &mut renderer, cache);
    assert!(shows(&texts, "EDITING SECTION"));
}
