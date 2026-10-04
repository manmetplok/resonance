//! Showing or dismissing the error banner keeps the main area's widget
//! state (code review UX-05).
//!
//! The view used to build `column![transport, error_bar, main_area]` with
//! an error and `column![transport, main_area]` without, so the main area
//! moved between child indices and iced rebuilt its widget tree from
//! scratch: scroll offsets, canvas key focus and in-progress drags were
//! lost every time an error came or went. The status area is now always a
//! child. These tests drive iced's own `UserInterface` across rebuilds
//! with one shared cache — exactly what the runtime does between frames —
//! because a `Simulator` starts from a fresh cache every time.

use iced::widget::scrollable::AbsoluteOffset;
use iced::{Rectangle, Size, Vector};
use iced_test::core::renderer::Headless;
use iced_test::core::widget::operation::{self, Operation, Outcome};
use iced_test::core::widget::Id;
use iced_test::runtime::user_interface::{Cache, UserInterface};
use resonance_app::state::{ClipState, ViewMode, ARRANGE_SCROLL_ID};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, EngineError, FadeCurve, TrackType};

const SR: u32 = 48_000;
const WINDOW: Size = Size::new(1440.0, 900.0);
const SCROLLED_X: f32 = 1500.0;

/// One audio track holding a five-minute clip: far wider than the window
/// at the default zoom, so the arrange timeline scrolls horizontally.
fn long_song() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_add_track(1, TrackType::Audio);
    let len = 300 * SR as u64;
    app.test_push_clip(ClipState {
        id: 1,
        track_id: 1,
        start_sample: 0,
        duration_samples: len,
        name: "long".into(),
        total_frames: len,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });
    app
}

fn renderer() -> iced::Renderer {
    iced_test::futures::futures::executor::block_on(iced::Renderer::new(
        iced::Font::with_name("Fira Sans"),
        iced::Pixels(16.0),
        None,
    ))
    .expect("headless renderer")
}

/// Records the translation of the scrollable with `target`'s id.
struct ReadOffset {
    target: Id,
    found: Option<Vector>,
}

impl Operation for ReadOffset {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&self.target) {
            self.found = Some(translation);
        }
    }

    fn finish(&self) -> Outcome<()> {
        Outcome::None
    }
}

/// Lay the app's view out against `cache`, run `op` on it, and hand the
/// cache back for the next frame.
fn frame(
    app: &Resonance,
    renderer: &mut iced::Renderer,
    cache: Cache,
    op: &mut dyn Operation,
) -> Cache {
    let mut ui = UserInterface::build(app.view(), WINDOW, cache, renderer);
    ui.operate(renderer, op);
    ui.into_cache()
}

fn arrange_offset(app: &Resonance, renderer: &mut iced::Renderer, cache: Cache) -> (f32, Cache) {
    let mut read = ReadOffset {
        target: ARRANGE_SCROLL_ID,
        found: None,
    };
    let cache = frame(app, renderer, cache, &mut read);
    let x = read.found.expect("the arrange timeline scrollable is in the tree").x;
    (x, cache)
}

#[test]
fn the_arrange_scroll_offset_survives_an_error_banner_coming_and_going() {
    let mut app = long_song();
    let mut renderer = renderer();

    // Scroll the timeline the way a drag of its scrollbar would.
    let mut scroll = operation::scrollable::scroll_to(
        ARRANGE_SCROLL_ID,
        AbsoluteOffset {
            x: Some(SCROLLED_X),
            y: None,
        },
    );
    let cache = frame(&app, &mut renderer, Cache::default(), &mut scroll);
    let (x, cache) = arrange_offset(&app, &mut renderer, cache);
    assert_eq!(x, SCROLLED_X, "precondition: the timeline scrolled");

    // An error lands (a failed preset star, a bounce error, …).
    app.test_handle_engine_event(AudioEvent::Error(EngineError::internal(
        "Could not star the preset",
    )));
    assert!(app.test_error_message().is_some());
    let (x, cache) = arrange_offset(&app, &mut renderer, cache);
    assert_eq!(x, SCROLLED_X, "showing the banner must keep the scroll offset");

    // The user dismisses it.
    app.test_update(resonance_app::message::Message::Ui(
        resonance_app::message::UiMessage::DismissError,
    ));
    assert!(app.test_error_message().is_none());
    let (x, _cache) = arrange_offset(&app, &mut renderer, cache);
    assert_eq!(x, SCROLLED_X, "dismissing the banner must keep the scroll offset");
}

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(resonance_app::theme::ICON_FONT_BYTES.into());
    for face in resonance_app::theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: resonance_app::theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// The status area with every line up at once, worst first: the engine
/// status (UX-04), the autosave indicator (UX-13), then the dismissable
/// error banner.
#[test]
fn status_area_stacks_engine_autosave_and_error_lines() {
    use resonance_app::message::{Message, ProjectIoMessage};

    let mut app = long_song();
    // Set directly rather than polled by a Tick: the engine-death latch
    // is process-wide, and another test in this binary may have tripped
    // it, which would turn the line into the engine-death status.
    app.test_set_engine_health(resonance_app::state::EngineHealth::StreamLost);
    for _ in 0..3 {
        app.test_update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
            Err("Disk quota exceeded (os error 122)".to_owned()),
            true,
        )));
    }
    app.test_handle_engine_event(AudioEvent::Error(EngineError::internal(
        "Could not star the preset",
    )));

    let mut ui =
        iced_test::Simulator::with_size(sim_settings(), Size::new(1440.0, 260.0), app.view());
    let snap = ui
        .snapshot(&resonance_app::theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, "tests/snapshots/status_area_all_lines.png");
}
