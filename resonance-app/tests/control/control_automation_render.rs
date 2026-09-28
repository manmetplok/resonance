//! `automation.set_lane` driving `meter.measure` / `meter.stems`
//! (automation-control-api.md §7, "Render"): a master volume fade-in
//! must be the exact lane the real engine renders louder, and the
//! bar-1 / bar-4 windows `meter.*` asks for must be the exact windows
//! that tell the fade apart.
//!
//! # Why this is a wiring test, not an audio one
//!
//! `Resonance::new_for_test()` builds its engine with
//! [`resonance_audio::AudioEngine::for_test_capture`]
//! (`resonance-app/src/lib.rs`): every `AudioCommand` the app sends is
//! captured and thrown away by a drain thread — there is no engine
//! thread, no render pool and no real `measure_mix` behind it. Every
//! `meter.*` test in `control_meter.rs` completes its job by calling
//! `test_apply_engine_event(AudioEvent::MixMeasured { results, .. })`
//! with a `MixMeasurement` the TEST wrote by hand. Doing that here with
//! numbers picked to show a 20 dB rise would prove nothing but that
//! `serde` round-trips a struct — the exact "silent goldens are
//! vacuous" failure mode, just with a fabricated rise instead of a
//! fabricated silence.
//!
//! So this file proves the two halves that ARE real, and leaves the
//! actual audio proof where it already lives:
//!
//! 1. `automation.set_lane` with `{"master": true, "control": "volume"}`
//!    and `"-inf"` / `0` dB points at bars 1 and 5 dispatches
//!    `AudioCommand::SetAutomationLane` carrying the byte-for-byte same
//!    `AutomationLane` (target `MasterGain`, breakpoints at frame 0 and
//!    `4 * BAR` with values `0.0` and `real_to_lane_value(.., 0.0)`)
//!    that `resonance-audio/tests/bounce/automation_render.rs`
//!    (`master_gain_ramp`, slice A0) hand-builds and feeds straight into
//!    the real `measure_mix` / `export_stems` engine functions — and
//!    that test is what proves late-window `sample_peak_db` reads
//!    `> early + 20.0` dB and stays `> -20.0` dB (not silent). The two
//!    files together are the end-to-end proof; splitting them is what
//!    the test-layout rules in CLAUDE.md ask for (no real audio at the
//!    app layer, no control wire at the audio layer).
//! 2. `meter.measure`'s bar-1 / bar-4 windows, and `meter.stems`'s same
//!    windows, resolve to the exact sample ranges `[0, BAR)` and
//!    `[3*BAR, 4*BAR)` — the early/late pair the paired audio test
//!    measures — captured off the dispatched `AudioCommand::MeasureMix`
//!    rather than a canned result.
//!
//! Bars are 120 BPM / 4/4 / 48 kHz, so one bar is exactly `BAR = 96_000`
//! samples — the same tempo `control_automation.rs` seeds, chosen so
//! `resolve_position` needs no fractional-beat rounding.

use crate::common::call;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, MeasureSource as EngineSource, StemSource};
use resonance_common::{real_to_lane_value, AutomationTarget};
use resonance_audio::types::AudioEvent;
use resonance_control::job::JobStarted;
use resonance_control::methods::automation::{AutomationValue, LaneEditResult};
use serde_json::json;

/// One 4/4 bar at 120 BPM, 48 kHz — matches `control_automation.rs`.
const BAR: u64 = 96_000;

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
    (app, cmd_rx)
}

/// The `SetAutomationLane` commands sent since the last drain.
fn set_lane_commands(rx: &Receiver<AudioCommand>) -> Vec<resonance_common::AutomationLane> {
    rx.try_iter()
        .filter_map(|c| match c {
            AudioCommand::SetAutomationLane { lane } => Some(lane),
            _ => None,
        })
        .collect()
}

/// Close out a `meter.*` job by failing it, so the offline-measure guard
/// (`has_live_offline_measure`) does not refuse the next call as `busy`.
/// The commands already captured off `cmd_rx` are what these tests
/// assert on; the job's own result is not.
fn finish_job(app: &mut Resonance, response: resonance_control::Response) {
    let started: JobStarted = response.result().expect("job starts");
    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: u64::from(started.job_id),
        message: "done".to_owned(),
    });
}

/// The `MeasureMix` commands sent since the last drain.
fn measure_commands(
    rx: &Receiver<AudioCommand>,
) -> Vec<(Vec<StemSource>, Option<(u64, u64)>, EngineSource)> {
    rx.try_iter()
        .filter_map(|c| match c {
            AudioCommand::MeasureMix {
                targets,
                range,
                source,
                ..
            } => Some((targets, range, source)),
            _ => None,
        })
        .collect()
}

/// `automation.set_lane {"master": true, "control": "volume"}` from
/// `"-inf"` at bar 1 to `0` dB at bar 5 (i.e. across bars 1-4) dispatches
/// exactly the `AutomationLane` the paired real-audio test renders.
#[test]
fn set_lane_dispatches_the_exact_lane_the_audio_test_proves_rises() {
    let (mut app, cmd_rx) = app();

    let response = call(
        &mut app,
        "automation.set_lane",
        json!({
            "master": true,
            "control": "volume",
            "points": [
                { "position": { "bar": 1 }, "value": "-inf" },
                { "position": { "bar": 5 }, "value": 0 },
            ],
        }),
    );
    let result: LaneEditResult = response.result().expect("set_lane succeeds");
    let lane = result.lane.expect("a lane was created");
    assert_eq!(lane.points.len(), 2);
    assert_eq!(lane.points[0].value, AutomationValue::Text("-inf".to_owned()));
    assert_eq!(lane.points[1].value, AutomationValue::Number(0.0));
    // Read back at the samples the bars resolve to at 120 BPM / 4/4 / 48k.
    assert_eq!(lane.points[0].position.sample, 0);
    assert_eq!(lane.points[1].position.sample, 4 * BAR);

    // The command the engine actually receives: the same shape
    // `master_gain_ramp()` builds by hand in the paired audio test.
    let sent = set_lane_commands(&cmd_rx);
    assert_eq!(sent.len(), 1, "one set_lane call, one engine command");
    let sent_lane = &sent[0];
    assert_eq!(sent_lane.target, AutomationTarget::MasterGain);
    assert_eq!(sent_lane.points.len(), 2);
    assert_eq!(sent_lane.points[0].time_frames, 0);
    assert_eq!(sent_lane.points[1].time_frames, 4 * BAR);

    // The floor point is EXACTLY normalized 0 — the engine's "-60 dB /
    // exact silence" value automation-control-api.md §4.2 describes.
    assert_eq!(sent_lane.points[0].value, 0.0);
    // The unity point is whatever `real_to_lane_value` maps 0 dB to —
    // computed the same way the app computed it, not re-derived here.
    let unity = real_to_lane_value(&AutomationTarget::MasterGain, 0.0);
    assert_eq!(sent_lane.points[1].value, unity);

    // Guard against a vacuous wiring test the same way
    // `automation_render.rs` guards against a vacuous audio one: the
    // two points must differ by a lot, or a bug that wrote the SAME
    // value at both ends (making any later render flat, exactly the
    // regression that test's "sanity" assertion catches) would pass
    // here unnoticed.
    assert!(
        sent_lane.points[1].value - sent_lane.points[0].value > 0.5,
        "bar-4 point must be far from the bar-1 floor, not a no-op ramp: \
         {} vs {}",
        sent_lane.points[0].value,
        sent_lane.points[1].value,
    );
}

/// `meter.measure` over bar 1 and over bar 4 resolves to the exact
/// `[0, BAR)` / `[3*BAR, 4*BAR)` windows the paired audio test measures
/// as "early" and "late".
#[test]
fn measure_windows_bar_1_and_bar_4_to_the_samples_the_audio_test_uses() {
    let (mut app, cmd_rx) = app();

    let response = call(
        &mut app,
        "meter.measure",
        json!({ "range": { "start": { "bar": 1 }, "end": { "bar": 2 } } }),
    );
    let early = measure_commands(&cmd_rx);
    assert_eq!(early.len(), 1);
    assert_eq!(early[0].0, vec![StemSource::Master]);
    assert_eq!(early[0].1, Some((0, BAR)), "bar 1 must be [0, BAR)");
    assert_eq!(early[0].2, EngineSource::Render);
    finish_job(&mut app, response);

    let _response = call(
        &mut app,
        "meter.measure",
        json!({ "range": { "start": { "bar": 4 }, "end": { "bar": 5 } } }),
    );
    let late = measure_commands(&cmd_rx);
    assert_eq!(late.len(), 1);
    assert_eq!(
        late[0].1,
        Some((3 * BAR, 4 * BAR)),
        "bar 4 must be [3*BAR, 4*BAR)"
    );
}

/// `meter.stems` windows the same two bars identically — the master
/// entry in a stems pass must be measured over the same samples
/// `meter.measure` would use, since both back onto one `MeasureMix`.
#[test]
fn stems_windows_bar_1_and_bar_4_to_the_same_samples_as_measure() {
    let (mut app, cmd_rx) = app();

    let response = call(
        &mut app,
        "meter.stems",
        json!({ "range": { "start": { "bar": 1 }, "end": { "bar": 2 } } }),
    );
    let early = measure_commands(&cmd_rx);
    assert_eq!(early.len(), 1);
    assert!(early[0].0.contains(&StemSource::Master));
    assert_eq!(early[0].1, Some((0, BAR)));
    finish_job(&mut app, response);

    let _response = call(
        &mut app,
        "meter.stems",
        json!({ "range": { "start": { "bar": 4 }, "end": { "bar": 5 } } }),
    );
    let late = measure_commands(&cmd_rx);
    assert_eq!(late.len(), 1);
    assert!(late[0].0.contains(&StemSource::Master));
    assert_eq!(late[0].1, Some((3 * BAR, 4 * BAR)));
}
