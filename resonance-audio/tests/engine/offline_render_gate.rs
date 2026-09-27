//! The one "offline render in progress" gate (code review MIX-02 /
//! ENG-05).
//!
//! Every offline renderer (WAV / FLAC export, stem export, bounce in
//! place, freeze, offline mix measurement) drives the **same** live CLAP
//! plugin instances as the audio callback, from a worker thread. The only
//! protection used to be "refuse to start while `playing`", checked once
//! at render start: the stopped-branch monitor pass kept processing armed
//! tracks' plugin chains every quantum, and Play / Record accepted while
//! the render ran, after which the callback and the worker interleaved
//! `process()` on one stateful plugin at unrelated positions.
//!
//! Now `SharedState::offline_render_count` — held up by
//! `OfflineRenderGuard` for the whole render — is honoured on both sides:
//! the callback outputs silence and holds the transport (touching no
//! plugin, a single atomic load) while it is non-zero, and the engine's
//! Play / Record handlers refuse with an `AudioEvent::Error`.

use std::sync::atomic::Ordering;
use std::time::Instant;

use resonance_audio::test_support::{
    EngineHandlerHarness, LiveMidiEvent, MixAudioHarness, OfflineRenderGuard,
    OFFLINE_RENDER_BUSY_MSG,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const CH: usize = 2;
const IN_CH: usize = 2;

fn dc_clip(id: ClipId, track_id: TrackId, frames: usize) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![0.25; frames * CH]),
        name: format!("c{id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// One track carrying a DC clip (for the playing branch) and one armed,
/// monitor-enabled track (for the stopped-monitor branch).
fn fixture() -> MixAudioHarness {
    let mut clips = Track::new(1, "clips".into());
    clips.set_output(TrackOutput::Master);

    let mut monitored = Track::new(2, "monitored".into());
    monitored.set_output(TrackOutput::Master);
    monitored.set_monitor_enabled(true);
    monitored.set_record_armed(true);
    monitored.set_input_port(0);

    let mut tempo = TempoMap::default();
    tempo.rebuild_bar_table(SR);
    let h = MixAudioHarness::new(
        vec![clips, monitored],
        Vec::new(),
        vec![dc_clip(1, 1, BLOCK * 64)],
        Vec::new(),
        Vec::new(),
        tempo,
        BLOCK,
        CH,
        SR,
        true,
    );
    h.shared()
        .input_channels
        .store(IN_CH as u16, Ordering::Relaxed);
    h
}

fn input_block() -> Vec<f32> {
    vec![0.5; BLOCK * IN_CH]
}

fn any_signal(out: &[f32]) -> bool {
    out.iter().any(|&s| s != 0.0)
}

#[test]
fn stopped_monitoring_falls_silent_while_an_offline_render_holds_the_plugins() {
    let mut h = fixture();
    h.shared().monitoring.store(true, Ordering::Relaxed);

    h.push_monitor(&input_block());
    assert!(
        any_signal(h.render()),
        "sanity: the armed track monitors its input while stopped"
    );

    let guard = OfflineRenderGuard::mark(&h.shared_arc());
    let skips_before = h.shared().render_skip_cycles.load(Ordering::Relaxed);
    h.push_monitor(&input_block());
    let out = h.render().to_vec();
    assert!(
        !any_signal(&out),
        "the monitor pass runs the track's plugin chain — it must not run \
         while an offline render owns the plugin instances"
    );
    assert_eq!(
        h.shared().render_skip_cycles.load(Ordering::Relaxed),
        skips_before,
        "the gate is not a lock contention and must not count as a dropout"
    );

    drop(guard);
    h.push_monitor(&input_block());
    assert!(
        any_signal(h.render()),
        "monitoring resumes as soon as the render releases the gate"
    );
}

#[test]
fn playing_transport_is_held_and_silent_while_an_offline_render_is_in_progress() {
    let mut h = fixture();
    let shared = h.shared_arc();
    shared.playing.store(true, Ordering::SeqCst);
    shared.playhead.store(BLOCK as u64 * 4, Ordering::SeqCst);

    assert!(any_signal(h.render()), "sanity: the clip plays");
    assert_eq!(shared.playhead.load(Ordering::SeqCst), BLOCK as u64 * 5);

    // `playing` cannot normally be set while a render holds the gate
    // (Play refuses), but the callback must be safe on its own: a
    // render that started an instant before Play landed, or a
    // MIDI-clock master, must never make it drive the plugins.
    let guard = OfflineRenderGuard::mark(&shared);
    let out = h.render().to_vec();
    assert!(!any_signal(&out), "no arrangement render while gated");
    assert_eq!(
        shared.playhead.load(Ordering::SeqCst),
        BLOCK as u64 * 5,
        "the transport is held, not advanced through silence, so no \
         timeline is lost while the render runs"
    );

    drop(guard);
    assert!(any_signal(h.render()));
    assert_eq!(shared.playhead.load(Ordering::SeqCst), BLOCK as u64 * 6);
}

#[test]
fn live_midi_is_still_forwarded_to_the_engine_thread_while_gated() {
    let mut h = fixture();
    let _guard = OfflineRenderGuard::mark(&h.shared_arc());
    for note in [60u8, 64, 67] {
        h.send_live_midi(LiveMidiEvent::InboundNoteOn {
            track_id: 2,
            note,
            velocity: 0.8,
            arrival: Instant::now(),
        });
    }
    h.render();
    assert_eq!(
        h.drain_forwarded_midi(),
        3,
        "recording / MIDI-thru bookkeeping keeps flowing; only the \
         instrument delivery is withheld"
    );
}

#[test]
fn play_is_refused_with_an_error_while_an_offline_render_is_in_progress() {
    let mut h = EngineHandlerHarness::new();
    let guard = h.hold_offline_render();

    h.play();
    assert!(!h.is_playing(), "Play must not start under an offline render");
    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::Error(m) if m.message.contains(OFFLINE_RENDER_BUSY_MSG))),
        "the refusal must be surfaced as an error event, got {events:?}"
    );
    // ...and as the event that resets the app's optimistic `playing`
    // mirror (FU-F1a).
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::TransportRefused)),
        "the refusal must tell the app the transport did not start, got {events:?}"
    );

    drop(guard);
    h.play();
    assert!(h.is_playing(), "Play works again once the render is done");
    h.stop();
}

#[test]
fn record_is_refused_with_an_error_while_an_offline_render_is_in_progress() {
    let mut h = EngineHandlerHarness::new();
    let guard = h.hold_offline_render();

    h.record(1);
    assert!(!h.is_playing());
    assert!(!h.count_in_active(), "no count-in may be armed either");
    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::Error(m) if m.message.contains(OFFLINE_RENDER_BUSY_MSG))),
        "the refusal must be surfaced as an error event, got {events:?}"
    );

    h.record(0);
    assert!(!h.is_playing());
    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::Error(m) if m.message.contains(OFFLINE_RENDER_BUSY_MSG))),
        "a precount-less Record is refused the same way, got {events:?}"
    );

    // With the gate released the same Record reaches the ordinary path
    // (which, on this bare harness, fails for want of a project
    // directory) — proving it was the gate that refused above.
    drop(guard);
    h.record(0);
    let events = h.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::Error(m) if !m.message.contains(OFFLINE_RENDER_BUSY_MSG))),
        "expected the ordinary record path's own error, got {events:?}"
    );
}

/// A measurement takes the renderer on the engine thread, before its
/// worker exists (FU-F1b): a Play handled right after the `MeasureMix`
/// command is refused even while the worker hasn't reached its render.
/// It used to be taken on the worker, leaving a window where the Play
/// landed first and the transport started, then stalled.
#[test]
fn a_play_right_after_a_measurement_is_refused_before_its_worker_runs() {
    let mut h = EngineHandlerHarness::new();
    // Park the worker at its first clip-list read, holding the renderer.
    let clips = h.clips_lock();
    let parked = clips.write();

    h.measure_master(7);
    h.play();
    assert!(!h.is_playing(), "Play must not start under a spawned measurement");
    assert!(h.drain_events().iter().any(|e| matches!(e, AudioEvent::TransportRefused)));

    drop(parked);
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let mut events = Vec::new();
    while Instant::now() < deadline
        && !events.iter().any(|e| matches!(e, AudioEvent::MixMeasureError { measure_id: 7, .. }))
    {
        events.extend(h.drain_events());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::MixMeasureError { measure_id: 7, .. })),
        "the (empty) measurement terminates, got {events:?}"
    );
    // ...and releases the renderer.
    h.play();
    assert!(h.is_playing());
    h.stop();
}
