//! Render-level checks for the mixer's per-channel automation controls
//! (todo #383, arch doc #162 §3) and the strips' lazy-body fingerprints.
//! The lane header (parameter picker + Read toggle) lives in the
//! inspector's AUTOMATION group since mixer-cleanup.md §2.2; the strips
//! keep only the live automated tints on fader and pan.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::AudioEvent;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

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

fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// The lane header (parameter picker + Read toggle) left the strips for
/// the inspector's AUTOMATION group (mixer-cleanup.md §2.2, §3.4): a
/// lane on the master shows no Read toggle anywhere until the master is
/// selected, and then the inspector draws it.
#[test]
fn read_toggle_lives_in_the_inspector_not_on_the_strips() {
    let mut app = build_app();

    // No automation yet → no Read toggle anywhere.
    {
        let mut ui = simulator(&app);
        assert!(
            ui.find("READ").is_err(),
            "no Read toggle should render before any lane is added"
        );
    }

    let lane = AutomationLane::new(
        1,
        AutomationTarget::MasterGain,
        vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
    );
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(None)));
    {
        let mut ui = simulator(&app);
        assert!(
            ui.find("READ").is_err(),
            "the master strip no longer draws a lane header"
        );
    }

    let _ = app.update(Message::Ui(UiMessage::SelectMaster));
    let mut ui = simulator(&app);
    ui.find("READ")
        .expect("the master inspector's AUTOMATION group draws the lane's Read toggle");
}

// ---------------------------------------------------------------------
// Mixer-strip lazy-body fingerprints.
//
// Every strip's non-live body (head, chips, buttons, plugin chain,
// automation header, pan) renders inside `iced::widget::lazy` keyed on
// the fingerprints in `view/mixer/strip_fingerprint.rs`. The cached
// subtree is reused until the hash moves, so every rendered facet MUST
// move it — a missed field means the strip keeps drawing stale state —
// while the live meter levels (rendered outside the lazy region) must
// NOT.
// ---------------------------------------------------------------------

/// Demo track 2 — "Synth Bass", hosting plugin instance `2 * 100`.
const BASS: u64 = 2;
const BASS_PLUGIN: u64 = 200;
/// Demo bus 100 — "Bus 1 · Drums", hosting plugin instance 10001.
const DRUM_BUS: u64 = 100;
const DRUM_BUS_PLUGIN: u64 = 10001;

fn track_fp(app: &Resonance) -> u64 {
    app.test_track_strip_fingerprint(BASS)
        .expect("demo track exists")
}

#[test]
fn strip_fingerprint_is_stable_and_ignores_live_state_outside_the_lazy_body() {
    let mut app = build_app();
    let baseline = track_fp(&app);
    assert_eq!(
        baseline,
        track_fp(&app),
        "identical state must produce an identical fingerprint"
    );

    // Live meter levels tick per frame and render OUTSIDE the lazy body
    // (the fader/meter block is rebuilt every frame) — they must never
    // invalidate the cached strip body.
    app.test_set_track_levels(BASS, 0.93, 0.87);
    assert_eq!(
        baseline,
        track_fp(&app),
        "meter levels are live state outside the lazy body and must not move the hash"
    );

    // Track selection tints the outer container border, also outside
    // the lazy body — it must not invalidate it either. (If selection
    // ever moves inside the body, it must be added to the fingerprint
    // and this assertion updated.)
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(3))));
    assert_eq!(
        baseline,
        track_fp(&app),
        "selection renders outside the lazy body and must not move the hash"
    );
}

#[test]
fn track_strip_fingerprint_moves_with_every_rendered_facet() {
    use resonance_app::message::{PluginMessage, TrackMessage};

    let cases: &[(&str, fn(&mut Resonance))] = &[
        ("track name", |app| {
            let _ = app.update(Message::Track(TrackMessage::SetTrackName(
                BASS,
                "Renamed".to_string(),
            )));
        }),
        ("mute", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleMute(BASS)));
        }),
        ("solo", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleSolo(BASS)));
        }),
        ("record arm", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleRecordArm(BASS)));
        }),
        ("input monitor", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleMonitor(BASS)));
        }),
        ("track colour", |app| {
            let _ = app.update(Message::Track(TrackMessage::SetTrackColor(
                BASS,
                [1, 2, 3],
            )));
        }),
        // The inline rename swaps the name for a field, and every
        // keystroke redraws it.
        ("strip rename opened", |app| {
            let _ = app.update(Message::Ui(UiMessage::BeginStripRename(BASS)));
        }),
        ("chain FX bypass", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleTrackFxBypass(BASS)));
        }),
        ("pan", |app| {
            let _ = app.update(Message::Track(TrackMessage::SetTrackPan(BASS, 0.42)));
        }),
        // The focused slot's line takes the highlight treatment.
        ("slot focus", |app| {
            let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(BASS_PLUGIN)));
        }),
        // Per-slot bypass turns the line's dot into a ring and dims it.
        ("plugin-slot bypass", |app| {
            app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
                instance_id: BASS_PLUGIN,
                bypassed: true,
                own_bypass_param: false,
            });
        }),
    ];

    for (facet, mutate) in cases {
        let mut app = build_app();
        let before = track_fp(&app);
        mutate(&mut app);
        let after = track_fp(&app);
        assert_ne!(
            before, after,
            "facet `{facet}` is rendered by the lazy strip body but did not move \
             its fingerprint — the cached tree would keep drawing stale state"
        );
    }
}

#[test]
fn read_toggle_and_live_pan_tint_move_the_track_fingerprint() {
    let mut app = build_app();

    // A Read-enabled pan lane on the track.
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
        lane: AutomationLane::new(
            9,
            AutomationTarget::TrackPan(BASS),
            vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
        ),
    });
    let with_lane = track_fp(&app);

    // A throttled live value arrives during playback — the pan knob
    // takes the warm automated tint, which renders inside the lazy body.
    app.test_apply_engine_event(AudioEvent::AutomatedValue {
        target: AutomationTarget::TrackPan(BASS),
        value_norm: 0.8,
    });
    let with_tint = track_fp(&app);
    assert_ne!(
        with_lane, with_tint,
        "the live automated-pan tint renders in the lazy body and must move the hash"
    );

    // Read off (engine echoes the lane disabled): the tint clears.
    let mut lane = AutomationLane::new(
        9,
        AutomationTarget::TrackPan(BASS),
        vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
    );
    lane.enabled = false;
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });
    let read_off = track_fp(&app);
    assert_ne!(
        with_tint, read_off,
        "turning a lane's Read off clears the pan tint and must move the hash"
    );
}

#[test]
fn bus_and_master_strip_fingerprints_move_with_their_facets() {
    use resonance_app::message::{BusMessage, MasterMessage};

    let bus_cases: &[(&str, fn(&mut Resonance))] = &[
        ("bus mute", |app| {
            let _ = app.update(Message::Bus(BusMessage::ToggleBusMute(DRUM_BUS)));
        }),
        ("bus FX bypass", |app| {
            let _ = app.update(Message::Bus(BusMessage::ToggleBusFxBypass(DRUM_BUS)));
        }),
        ("bus pan", |app| {
            let _ = app.update(Message::Bus(BusMessage::SetBusPan(DRUM_BUS, -0.3)));
        }),
        ("bus plugin-slot bypass", |app| {
            app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
                instance_id: DRUM_BUS_PLUGIN,
                bypassed: true,
                own_bypass_param: false,
            });
        }),
        ("bus slot focus", |app| {
            let _ = app.update(Message::Plugin(
                resonance_app::message::PluginMessage::FocusSlot(DRUM_BUS_PLUGIN),
            ));
        }),
    ];
    for (facet, mutate) in bus_cases {
        let mut app = build_app();
        let before = app
            .test_bus_strip_fingerprint(DRUM_BUS)
            .expect("demo bus exists");
        mutate(&mut app);
        let after = app
            .test_bus_strip_fingerprint(DRUM_BUS)
            .expect("demo bus exists");
        assert_ne!(before, after, "bus facet `{facet}` must move the fingerprint");
    }

    let master_cases: &[(&str, fn(&mut Resonance))] = &[
        ("master FX bypass", |app| {
            let _ = app.update(Message::Master(MasterMessage::ToggleMasterFxBypass));
        }),
        ("master chain", |app| {
            app.test_push_master_plugin(resonance_app::state::PluginSlotState::new(
                9_001,
                "Limiter".to_owned(),
                "com.resonance.limiter".to_owned(),
                "/plugins/limiter.clap".to_owned(),
                Vec::new(),
                false,
            ));
        }),
    ];
    for (facet, mutate) in master_cases {
        let mut app = build_app();
        let before = app.test_master_strip_fingerprint();
        mutate(&mut app);
        let after = app.test_master_strip_fingerprint();
        assert_ne!(
            before, after,
            "master facet `{facet}` must move the fingerprint"
        );
    }
}

#[test]
fn sub_strip_fingerprint_moves_with_its_slim_facet_set() {
    use resonance_app::message::TrackMessage;
    use resonance_app::state::TrackState;

    const SUB: u64 = 42;
    let mut app = build_app();
    app.test_push_track(TrackState::new_sub_track(
        SUB,
        10,
        "Drums \u{2192} Kick".to_string(),
        1,
        0,
    ));

    let baseline = app
        .test_track_strip_fingerprint(SUB)
        .expect("sub-track exists");
    // Live levels stay outside the sub-strip's lazy body too.
    app.test_set_track_levels(SUB, 0.7, 0.6);
    assert_eq!(
        baseline,
        app.test_track_strip_fingerprint(SUB).unwrap(),
        "sub-strip meter levels must not move the hash"
    );

    let cases: &[(&str, fn(&mut Resonance))] = &[
        ("sub mute", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleMute(SUB)));
        }),
        ("sub solo", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleSolo(SUB)));
        }),
        ("sub FX bypass", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleTrackFxBypass(SUB)));
        }),
        ("sub pan", |app| {
            let _ = app.update(Message::Track(TrackMessage::SetTrackPan(SUB, 0.25)));
        }),
        ("sub name", |app| {
            let _ = app.update(Message::Track(TrackMessage::SetTrackName(
                SUB,
                "Drums \u{2192} Snare".to_string(),
            )));
        }),
    ];
    for (facet, mutate) in cases {
        let mut app = build_app();
        app.test_push_track(TrackState::new_sub_track(
            SUB,
            10,
            "Drums \u{2192} Kick".to_string(),
            1,
            0,
        ));
        let before = app.test_track_strip_fingerprint(SUB).unwrap();
        mutate(&mut app);
        let after = app.test_track_strip_fingerprint(SUB).unwrap();
        assert_ne!(before, after, "sub-strip facet `{facet}` must move the fingerprint");
    }
}
