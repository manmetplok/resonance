//! VIEW-11: the Compose instrument-track canvas must not be shifted by
//! the Arrange timeline's vertical scroll. The canvas sits inside
//! Compose's own scrollable and has a fixed height; it used to subtract
//! `viewport.scroll_offset_y` (Arrange's scroll) from every row, so
//! scrolling Arrange down and switching to Compose clipped the top lanes.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

const WINDOW: (f32, f32) = (1440.0, 900.0);

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

fn snapshot(app: &Resonance) -> iced_test::simulator::Snapshot {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    ui.snapshot(&theme::resonance_theme())
        .expect("snapshot should render")
}

#[test]
fn compose_tracks_render_identically_under_arrange_vertical_scroll() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    let unscrolled = snapshot(&app);

    // Scroll Arrange down through the real reducer.
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(400.0)));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0, 3000.0,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(300.0)));
    assert_eq!(app.test_arrange_scroll_y(), 300.0, "precondition: Arrange scrolled");
    let scrolled = snapshot(&app);

    // `matches_image` writes the reference when the file is absent, then
    // the second call compares against it — a pixel diff with no
    // committed golden.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("compose_unscrolled.png");
    assert!(unscrolled.matches_image(&path).expect("write reference"));
    assert!(
        scrolled.matches_image(&path).expect("compare"),
        "Arrange's vertical scroll leaked into the Compose track canvas"
    );
}
