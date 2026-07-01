//! Golden-image snapshot for the drag-to-timeline placement visuals
//! (design doc #175, epic #35, todo #605).
//!
//! Locks in the Canvas additions painted while a media-browser row is being
//! dragged over the arrangement: the lit target lane (`ACCENT_LINE` wash +
//! border), the dashed grid-snapped ghost clip (WARM), the drag pill and
//! drop tooltip trailing the cursor (target track + bar + `→ 48 kHz`
//! conversion), and the dashed "create a new audio track" drop zone below
//! the last lane.
//!
//! The drag state is installed directly (`test_set_drag_placement`) so the
//! render is deterministic — the gesture wiring itself is covered by
//! `tests/drag_placement_handlers.rs`. Window size matches the app's
//! default 1440×900; on first run `matches_image()` writes the golden.

use iced::{Point, Size};
use iced_test::simulator::Simulator;
use resonance_app::message::DropTarget;
use resonance_app::state::{
    DragPlacement, DraggedAsset, DropResolution, TrackState, ViewMode,
};
use resonance_app::{theme, Resonance, STARTUP_TAB};

const WINDOW: (f32, f32) = (1440.0, 900.0);
const SR: u32 = 48_000;
const ZOOM: f32 = 120.0; // px per second

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

fn audio_track(id: u64, order: usize, name: &str) -> TrackState {
    let mut t = TrackState::new_audio(id, order);
    t.name = name.to_string();
    t
}

/// Arrange-pinned app with three audio lanes and an in-flight drag hovering
/// over the second lane, previewing a dashed ghost clip snapped to bar 3
/// with a 44.1 → 48 kHz conversion note.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);

    app.test_push_track(audio_track(1, 0, "Drums"));
    app.test_push_track(audio_track(2, 1, "Bass"));
    app.test_push_track(audio_track(3, 2, "Keys"));

    // Hover over lane index 1 ("Bass"). Lanes begin at the fixed header
    // (ruler + collapsed global shelf) and are TRACK_HEIGHT tall, so a
    // cursor around y=250 lands in the second lane. Snap the ghost to
    // bar 3 (bars are 2 s at 120 BPM 4/4, so bar 3 starts at 4 s →
    // 192_000 samples) so the ghost + tooltip agree with the ruler.
    let start_sample: u64 = 192_000;
    app.test_set_drag_placement(DragPlacement {
        asset: DraggedAsset {
            path: std::path::PathBuf::from("/imports/vocal take.wav"),
            name: "vocal take".to_string(),
            source_sample_rate: 44_100,
            // ~3 s of audio at the project rate for a legible ghost body.
            duration_samples: 144_000,
            conversion: Some("→ 48 kHz".to_string()),
        },
        cursor: Point::new(360.0, 250.0),
        resolved: Some(DropResolution {
            target: DropTarget::ExistingTrack {
                track_id: 2,
                start_sample,
            },
            lane_index: Some(1),
            bar_label: "Bar 3.1".to_string(),
        }),
    });

    app
}

#[test]
fn drag_placement_over_lane_render() {
    let app = build_app();
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/drag_placement_over_lane_render.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
