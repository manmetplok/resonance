//! Golden-image snapshot for the mixer inspector's **SENDS** block
//! (ba todo #1310, design doc #172).
//!
//! ROUTING used to end in two hardcoded read-only rows —
//! `Send A -> (none)` and `Send B -> (none)` — that named a feature the
//! GUI could not reach. This locks in what replaced them: one slot per
//! live send (destination picker, level slider + dB readout, PRE/POST,
//! ON, remove) and the "+ Add send" picker underneath.
//!
//! Two sends are seeded so both toggle states are on screen at once: an
//! enabled post-fader send at -6 dB and a disabled pre-fader one. Both
//! arrive the way the live app gets them — through the engine's
//! `BusAdded` / `BusRoleChanged` / `AuxSendChanged` echoes, which are
//! what the mirror is built from.
//!
//! Per-send metering is deliberately absent (cancelled todo #481).

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::{AudioEvent, SendSource};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The demo seed's "Synth Bass" instrument track.
const BASS: u64 = 2;

const REVERB_BUS: u64 = 900;
const DELAY_BUS: u64 = 901;

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

fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(BASS))));
    app
}

fn add_return_bus(app: &mut Resonance, bus_id: u64, name: &str) {
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id,
        name: name.to_string(),
    });
    app.test_apply_engine_event(AudioEvent::BusRoleChanged {
        bus_id,
        is_return: true,
    });
}

fn add_send(
    app: &mut Resonance,
    send_id: u64,
    dest: u64,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id,
        source: SendSource::Track(BASS),
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Two live sends: `-6.0 dB` post-fader and enabled into "FX Return 1",
/// and a muted pre-fader tap into "Delay".
#[test]
fn mixer_inspector_sends_populated() {
    let mut app = build_app();
    add_return_bus(&mut app, REVERB_BUS, "FX Return 1");
    add_return_bus(&mut app, DELAY_BUS, "Delay");
    add_send(&mut app, 1, REVERB_BUS, -6.0, false, true);
    add_send(&mut app, 2, DELAY_BUS, -12.0, true, false);
    snapshot_to(&app, "tests/snapshots/mixer_inspector_sends_populated.png");
}

/// A route the engine refused: no slot appears, and the reason is shown
/// inline under the picker that raised it rather than the gesture
/// silently doing nothing.
#[test]
fn mixer_inspector_send_rejected() {
    let mut app = build_app();
    add_return_bus(&mut app, REVERB_BUS, "FX Return 1");
    app.test_apply_engine_event(AudioEvent::AuxSendRejected {
        source: SendSource::Track(BASS),
        dest: REVERB_BUS,
        reason: "that send would feed back into itself".to_string(),
    });
    snapshot_to(&app, "tests/snapshots/mixer_inspector_send_rejected.png");
}
