//! Regression test for the Mixer-inspector panic on a fresh project
//! with no busses.
//!
//! Bug: after adding the first track (e.g. the preset Drums track) to
//! a brand-new project and navigating to the Mixer tab, the app
//! panicked at `view/mixer/inspector.rs:450` with
//! `index out of bounds: the len is 0 but the index is 0`. Root cause:
//! `UiViewCaches::default()` seeded `output_choices` as an empty Vec,
//! and `output_block` fell back to `choices[0].clone()` when the
//! track's `output` (Master) didn't appear in that empty list.
//! `view_caches.rebuild_output` is only called when busses change or
//! a project is loaded/replayed/demo-seeded — a freshly-created
//! project that had a track added but no busses would never trigger
//! the rebuild.
//!
//! Fix: seed the default with `[Master]` so the list is never empty,
//! and additionally make `output_block` synthesize a fallback entry
//! when the track's current output isn't in the cached list (covering
//! a track routed to a stale bus mid-replay, too).

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::UiMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Min/default window size per ux-guidelines, same as the other
/// `iced_test` integration tests in this crate.
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

/// Construct the simulator and force one render pass. With the
/// pre-fix code this `view()` call panicked inside the inspector's
/// lazy block when it built the OUTPUT picker.
#[test]
fn mixer_inspector_renders_without_busses() {
    // Pin the startup tab to Mixer so `view()` lands on `view_mixer`,
    // which builds the inspector for the selected track.
    let _ = STARTUP_TAB.set(ViewMode::Mixer);

    let (mut app, _task) = Resonance::new();
    demo::seed_minimal_drum_track_no_busses(&mut app);

    // Belt-and-braces: explicitly request Mixer in case another test
    // in this binary already set STARTUP_TAB to a different tab. (The
    // OnceLock means our `.set` above is a no-op then.)
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));

    // Building the Simulator drives `view()` which drives the lazy
    // block in `inspector::view`, which calls `output_block` — the
    // exact path that used to panic.
    let _ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
}
