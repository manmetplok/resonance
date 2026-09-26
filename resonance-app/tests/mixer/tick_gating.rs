//! Tick gating: the idle-rate subscription, the `PollPeaks` gate, the
//! VU settle floor, and the BPM text_input no-op guard.
//!
//! The 16 ms `Tick` used to run unconditionally: the app redrew at 60 Hz
//! forever, `PollPeaks` went to the engine every tick even with the
//! transport stopped and every meter at zero, decayed VU levels churned
//! through subnormals for ~10 s after silence, and a playing multi-tempo
//! song rewrote the BPM field's model string every tick (clobbering an
//! in-progress edit). These tests pin the fixes in `update::tick`.

use std::time::Duration;

use resonance_app::message::{Message, TransportMessage};
use resonance_app::state::TempoEvent;
use resonance_app::update::tick;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, EngineError, TrackType};

const TRACK: u64 = 1;

/// Build an app with a capturing engine, an active project, and one
/// audio track — stopped, meters at zero: the fully idle baseline.
fn capturing_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Audio);
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn poll_peaks_count(cmds: &[AudioCommand]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, AudioCommand::PollPeaks))
        .count()
}

/// Light the meters the way the engine does: a peak snapshot event.
fn light_meters(app: &mut Resonance, level: f32) {
    app.test_apply_engine_event(AudioEvent::PeakSnapshot {
        track_peaks: vec![(TRACK, level, level)],
        bus_peaks: Vec::new(),
        master_peak_l: level,
        master_peak_r: level,
    });
}

// -- Defect 3: VU decay settles to exactly 0.0 ----------------------------

#[test]
fn vu_levels_settle_to_exact_zero_within_bounded_ticks() {
    let (mut app, _rx) = capturing_app();
    light_meters(&mut app, 0.5);

    // The decay is gradual: after a few ticks the meters are still
    // falling, well above the floor (0.5 × 0.85³ ≈ 0.31).
    for _ in 0..3 {
        app.test_update(Message::Tick);
    }
    let track = &app.test_registry().tracks[0];
    assert!(track.level_l > 0.1, "meters must fall gradually, not snap");

    // 0.5 × 0.85ⁿ drops below the 1e-4 floor at n = 53; by 60 ticks
    // (~1 s of frames) every level must be exactly 0.0 — not a subnormal,
    // not an epsilon — so caches and the idle gate see a settled meter.
    for _ in 0..57 {
        app.test_update(Message::Tick);
    }
    let track = &app.test_registry().tracks[0];
    assert_eq!(track.level_l, 0.0);
    assert_eq!(track.level_r, 0.0);
    assert_eq!(app.test_master_levels(), (0.0, 0.0));
}

// -- Defect 2: PollPeaks only when someone consumes peaks ------------------

#[test]
fn idle_tick_emits_no_poll_peaks() {
    let (mut app, rx) = capturing_app();
    drain(&rx);

    app.test_update(Message::Tick);

    let cmds = drain(&rx);
    assert_eq!(
        poll_peaks_count(&cmds),
        0,
        "stopped + settled meters must not poll peaks, got {cmds:?}"
    );
}

#[test]
fn playing_tick_polls_peaks() {
    let (mut app, rx) = capturing_app();
    app.test_set_transport_playing(true);
    drain(&rx);

    app.test_update(Message::Tick);

    assert_eq!(poll_peaks_count(&drain(&rx)), 1);
}

#[test]
fn unsettled_meters_keep_polling_until_settled() {
    let (mut app, rx) = capturing_app();
    light_meters(&mut app, 0.5);
    drain(&rx);

    // Transport stopped, but the meters are still falling: every tick
    // until they settle must poll (the master meter is always visible).
    app.test_update(Message::Tick);
    assert_eq!(poll_peaks_count(&drain(&rx)), 1);

    for _ in 0..60 {
        app.test_update(Message::Tick);
    }
    drain(&rx);

    // Settled now — the very next tick stays silent.
    app.test_update(Message::Tick);
    assert_eq!(poll_peaks_count(&drain(&rx)), 0);
}

#[test]
fn armed_track_keeps_polling_while_stopped() {
    // A record-armed (or input-monitoring) track can move the meters
    // with the transport stopped, so the poll must not be gated off.
    let (mut app, rx) = capturing_app();
    app.test_arm_first_track(true);
    drain(&rx);

    app.test_update(Message::Tick);

    assert_eq!(poll_peaks_count(&drain(&rx)), 1);
}

// -- Defect 1: two-rate tick subscription ----------------------------------

#[test]
fn tick_interval_is_slow_when_idle_and_fast_when_active() {
    let (mut app, _rx) = capturing_app();
    let fast = Duration::from_millis(resonance_app::update::TICK_INTERVAL_MS);
    let slow = Duration::from_millis(tick::TICK_INTERVAL_IDLE_MS);
    assert!(slow > fast);

    // Fully idle: stopped, meters settled, nothing armed or in flight.
    assert_eq!(tick::tick_interval(&app), slow);

    // Transport running.
    app.test_set_transport_playing(true);
    assert_eq!(tick::tick_interval(&app), fast);
    app.test_set_transport_playing(false);

    // Recording.
    app.test_set_transport_recording(true);
    assert_eq!(tick::tick_interval(&app), fast);
    app.test_set_transport_recording(false);

    // Meters still falling after a stop: fast until they settle to 0.0,
    // then back to slow — the engine event that lit them is exactly the
    // "activity appears" edge that re-arms the fast tick.
    light_meters(&mut app, 0.5);
    assert_eq!(tick::tick_interval(&app), fast);
    for _ in 0..60 {
        app.test_update(Message::Tick);
    }
    assert_eq!(tick::tick_interval(&app), slow);

    // A record-armed track (input monitoring can move meters).
    app.test_arm_first_track(true);
    assert_eq!(tick::tick_interval(&app), fast);
    app.test_arm_first_track(false);

    // Browser audition preview sounding.
    app.test_set_audition_playing(Some(std::path::PathBuf::from("/tmp/preview.wav")));
    assert_eq!(tick::tick_interval(&app), fast);
    app.test_set_audition_playing(None);

    assert_eq!(tick::tick_interval(&app), slow);
}

// -- Defect 4: BPM text_input keyed no-op guard ----------------------------

/// A playing app whose tempo map steps 120 → 140 at bar 8 (44.1 kHz,
/// 4/4: the step lands at sample 705 600).
fn multi_tempo_playing_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, rx) = capturing_app();
    app.test_set_flat_tempo(120.0);
    app.test_push_tempo_event(TempoEvent {
        bar: 8,
        bpm: 140.0,
    });
    app.test_rebuild_tempo_map();
    app.test_set_transport_playing(true);
    (app, rx)
}

#[test]
fn bpm_input_is_not_rewritten_while_displayed_value_is_unchanged() {
    let (mut app, _rx) = multi_tempo_playing_app();

    // The user is mid-edit in the BPM field (typed, not yet committed).
    app.test_update(Message::Transport(TransportMessage::SetBpmText(
        "155.5".to_owned(),
    )));

    // Ticks through a constant-tempo stretch must leave the edit alone:
    // the playhead sits in the 120 region and transport.bpm is already
    // 120, so the keyed guard sees no display change and skips the write.
    for _ in 0..30 {
        app.test_update(Message::Tick);
    }
    assert_eq!(app.test_bpm_input(), "155.5");
    assert_eq!(app.test_transport_bpm(), 120.0);
}

#[test]
fn bpm_input_is_rewritten_when_the_tempo_actually_changes() {
    let (mut app, _rx) = multi_tempo_playing_app();
    app.test_update(Message::Transport(TransportMessage::SetBpmText(
        "155.5".to_owned(),
    )));

    // Cross the bar-8 tempo step: the displayed value really changes, so
    // the field is rewritten with the fresh tempo (a real change may
    // clobber an uncommitted edit — that is showing true data).
    app.test_update(Message::Transport(TransportMessage::SeekToSample(
        1_000_000,
    )));
    app.test_update(Message::Tick);

    assert_eq!(app.test_bpm_input(), "140.0");
    assert_eq!(app.test_transport_bpm(), 140.0);
}

// -- Engine-death banner ----------------------------------------------------

/// `AudioEngine::is_disconnected` reads a process-wide latch, so the
/// test that trips it races every other banner test in this binary
/// (libtest runs them on parallel threads): a trip landing mid-test
/// would raise the engine-death banner inside an unrelated app.
/// Every test that trips or asserts around that latch takes this lock
/// and resets the latch under it.
static ENGINE_LATCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn engine_latch_guard() -> std::sync::MutexGuard<'static, ()> {
    let guard = ENGINE_LATCH_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    resonance_audio::test_support::__reset_engine_disconnect_latch_for_test();
    guard
}

/// Once the engine's command channel disconnects, every `let _ =
/// r.engine.send(...)` call site in the app (there are dozens) silently
/// drops its command — including MCP-driven edits that still ack success
/// back to the caller. The tick handler must surface that once, as the
/// standard persistent error banner, and never clear it again on its own.
#[test]
fn disconnected_engine_sets_banner_on_next_tick_and_it_persists() {
    // Serialize against the stream-lost tests and reset the process-wide
    // latch so this test doesn't depend on prior test ordering.
    let _guard = engine_latch_guard();

    let (mut app, _task) = Resonance::new_for_test_disconnected();
    assert!(
        app.test_error_message().is_none(),
        "no banner before any send has ever failed"
    );

    // Trip the latch the way a real dead engine would: any send fails
    // because the engine thread's receiver is gone.
    let _ = app.engine.send(AudioCommand::Play);

    app.test_update(Message::Tick);
    let banner = app
        .test_error_message()
        .expect("Tick must surface the engine-disconnected banner")
        .to_string();
    assert!(
        banner.contains("engine stopped responding"),
        "banner should name the failure, got {banner:?}"
    );

    // Persists: further idle ticks must not clear or overwrite it.
    for _ in 0..5 {
        app.test_update(Message::Tick);
    }
    assert_eq!(
        app.test_error_message(),
        Some(banner.as_str()),
        "the banner must persist across subsequent ticks"
    );
}

// -- Output-stream-lost banner ----------------------------------------------

/// When the output *stream* dies (USB interface unplugged, PipeWire
/// restarted) the engine thread stays alive, so the engine-death check
/// never fires — the transport appears to run and edits still ack while
/// nothing is audible. The tick handler must poll the backends'
/// stream-lost flag into the same persistent banner, and — unlike engine
/// death — clear it again when the backend reports the stream back.
#[test]
fn lost_output_stream_raises_banner_and_recovery_clears_it() {
    // The engine-death check runs before the stream check and reads a
    // process-wide latch; hold the lock so the test that trips it can't
    // bleed a death banner into this app mid-test.
    let _guard = engine_latch_guard();

    let (mut app, _task) = Resonance::new_for_test();
    app.test_update(Message::Tick);
    assert!(
        app.test_error_message().is_none(),
        "no banner while the stream is healthy"
    );

    // The backend callback's job, done by hand: flag the stream lost.
    app.engine.__set_output_stream_lost_for_test(true);
    app.test_update(Message::Tick);
    let banner = app
        .test_error_message()
        .expect("Tick must surface the stream-lost banner")
        .to_string();
    assert!(
        banner.contains("output stream lost"),
        "banner should name the failure, got {banner:?}"
    );
    assert!(
        !banner.contains("engine stopped responding"),
        "stream loss must be worded distinctly from engine death, got {banner:?}"
    );

    // Persists while the stream stays lost.
    for _ in 0..5 {
        app.test_update(Message::Tick);
    }
    assert_eq!(
        app.test_error_message(),
        Some(banner.as_str()),
        "the banner must persist while the stream is still lost"
    );

    // PipeWire revived the stream (Streaming after Error): banner clears.
    app.engine.__set_output_stream_lost_for_test(false);
    app.test_update(Message::Tick);
    assert!(
        app.test_error_message().is_none(),
        "recovery must clear the stream-lost banner"
    );

    // A second loss re-raises: the app-side flag is a raise tracker, not
    // a one-shot latch like the engine-death one.
    app.engine.__set_output_stream_lost_for_test(true);
    app.test_update(Message::Tick);
    assert_eq!(
        app.test_error_message(),
        Some(banner.as_str()),
        "a later loss must raise the banner again"
    );
}

/// Recovery must clear only the banner this check raised: an unrelated
/// error that landed on `error_message` while the stream was down (the
/// app has one error surface) must be left standing.
#[test]
fn stream_recovery_leaves_an_unrelated_error_banner_standing() {
    let _guard = engine_latch_guard();

    let (mut app, _task) = Resonance::new_for_test();

    app.engine.__set_output_stream_lost_for_test(true);
    app.test_update(Message::Tick);
    assert!(app.test_error_message().is_some(), "loss raises the banner");

    // An unrelated engine error overwrites the single error surface.
    app.test_handle_engine_event(AudioEvent::Error(EngineError::internal("disk full")));
    assert_eq!(app.test_error_message(), Some("disk full"));

    app.engine.__set_output_stream_lost_for_test(false);
    app.test_update(Message::Tick);
    assert_eq!(
        app.test_error_message(),
        Some("disk full"),
        "recovery must not clear an error it did not raise"
    );
}
