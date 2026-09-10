//! `clip.delete` confirm gating, the `clip.place` fabricated-geometry
//! guard, and the `AmountSpec` bounds.
//!
//! `clip.delete` is destructive, so it now follows the same confirm
//! convention as `track.delete` / `section.delete`: without
//! `"confirm": true` it refuses with a summary of what would be lost and
//! deletes NOTHING.
//!
//! `clip.place`'s already-pooled path completes its job synchronously
//! from the mirror. When the dispatched placement never lands — the
//! asset vanished, or a pre-dispatch gate (bounce / freeze render)
//! swallowed the `Pool` message — the job used to complete `done` with
//! `place_result`'s fallback geometry: track 0, sample 0, length 0,
//! empty name. These pin the fix: such a job FAILS with a typed error,
//! and the pool handler's vanished-asset case is an error, not a silent
//! no-op.

use resonance_app::message::{Message, PoolMessage};
use resonance_app::state::{FreezeStatus, PoolAsset, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_common::AudioFormat;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::clip::{PlaceResult, TrimResult, MAX_SECONDS};
use resonance_control::methods::notes::MAX_BEATS;
use resonance_control::{ErrorKind, MutationAck, Response};
use crate::common::call;

const SR: u32 = 48_000;
const AUDIO_TRACK: u64 = 1;
const MIDI_TRACK: u64 = 2;
const ASSET: u64 = 7;
const ASSET_FRAMES: u64 = 4 * SR as u64;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-clip-confirm.rprj"));
    app.test_add_track(AUDIO_TRACK, TrackType::Audio);
    app.test_add_track(MIDI_TRACK, TrackType::Instrument);
    app.test_add_pool_asset(PoolAsset {
        id: ASSET,
        project_relative_path: "audio/asset_7.wav".to_owned(),
        original_path: "/samples/kick.wav".to_owned(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 44_100,
        duration_frames: ASSET_FRAMES,
        thumbnail_peaks: Vec::new(),
        missing: false,
    });
    app
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

fn job_status(app: &mut Resonance, job_id: u64) -> JobStatus {
    call(app, "job.status", serde_json::json!({ "job_id": job_id }))
        .result()
        .expect("job.status succeeds")
}

/// Place the fixture asset at `bar`, expecting the synchronous `done`.
fn place_at(app: &mut Resonance, bar: u32) -> PlaceResult {
    let started = start_place(app, bar);
    let status = job_status(app, u64::from(started.job_id));
    assert_eq!(
        status.state,
        JobState::Done,
        "placing a pooled asset completes immediately: {:?}",
        status.error
    );
    serde_json::from_value(status.result.expect("a done job carries its result"))
        .expect("PlaceResult decodes")
}

/// Issue the `clip.place` and return the started job, resolving nothing.
fn start_place(app: &mut Resonance, bar: u32) -> JobStarted {
    let response = call(
        app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "asset_id": ASSET,
            "start": { "bar": bar },
        }),
    );
    response.result().expect("clip.place starts a job")
}

// ---------------------------------------------------------------------------
// clip.delete confirm gating
// ---------------------------------------------------------------------------

#[test]
fn delete_refuses_without_confirm_and_mutates_nothing() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);
    let before = app.revision();

    let response = call(
        &mut app,
        "clip.delete",
        serde_json::json!({ "clip_id": clip_id }),
    );
    let message = expect_error(response, ErrorKind::NeedsConfirmation);
    // The summary is concrete — the clip's name, track and extent — so
    // the client can decide without a follow-up song.tracks.
    assert!(message.contains("kick"), "{message}");
    assert!(message.contains(&format!("track {AUDIO_TRACK}")), "{message}");
    assert!(message.contains(&format!("{ASSET_FRAMES} sample(s)")), "{message}");
    assert!(message.contains("\"confirm\": true"), "{message}");

    assert!(
        app.test_clips().iter().any(|c| c.id == clip_id),
        "a refusal deletes nothing"
    );
    assert_eq!(app.revision(), before, "no revision bump for a refusal");
}

#[test]
fn delete_with_confirm_deletes_the_clip() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let _: MutationAck = call(
        &mut app,
        "clip.delete",
        serde_json::json!({ "clip_id": clip_id, "confirm": true }),
    )
    .result()
    .expect("confirmed clip.delete succeeds");
    assert!(!app.test_clips().iter().any(|c| c.id == clip_id));
}

// ---------------------------------------------------------------------------
// clip.place: a swallowed placement fails the job
// ---------------------------------------------------------------------------

#[test]
fn a_freeze_in_flight_refuses_up_front_and_never_starts_a_job() {
    // The update()-level gates swallow `Pool` messages while a freeze (or
    // bounce) renders — the scenario in which the synchronously-completed
    // job used to report `done` with track 0 / sample 0 / length 0 / empty
    // name. The mutation gate refuses those requests with `busy` before a
    // job can even start; the handler's mirror check (see
    // `a_vanished_asset_placement_is_an_error_not_a_silent_noop` for the
    // pool half) backstops any future divergence between the two by
    // failing the job instead of fabricating geometry.
    let mut app = app();
    app.test_set_freeze_status(MIDI_TRACK, FreezeStatus::Freezing { fraction: 0.5 });

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "asset_id": ASSET,
            "start": { "bar": 1 },
        }),
    );
    let message = expect_error(response, ErrorKind::Busy);
    assert!(message.contains("freeze"), "{message}");
    assert!(app.test_clips().is_empty(), "nothing was placed");

    // Once the freeze has drained, the same call goes through with the
    // real geometry.
    app.test_set_freeze_status(MIDI_TRACK, FreezeStatus::Idle);
    let placed = place_at(&mut app, 1);
    assert_eq!(u64::from(placed.track_id), AUDIO_TRACK);
    assert_eq!(placed.length_samples, ASSET_FRAMES);
    assert_eq!(placed.name, "kick");
}

#[test]
fn a_vanished_asset_placement_is_an_error_not_a_silent_noop() {
    // The pool handler itself: `PlacePooledAsset` naming an asset that is
    // no longer pooled used to return without a trace. Dispatch it
    // directly (only the control endpoint sends it) and pin the refusal.
    let mut app = app();
    app.test_remove_pool_asset(ASSET);

    let _ = app.update(Message::Pool(PoolMessage::PlacePooledAsset {
        clip_id: 9_999,
        asset_id: ASSET,
        track_id: AUDIO_TRACK,
        start_sample: 0,
    }));

    assert!(app.test_clips().is_empty(), "nothing was placed");
    let message = app
        .test_error_message()
        .expect("the dropped placement is surfaced, not silent");
    assert!(message.contains("no longer in the pool"), "{message}");
}

// ---------------------------------------------------------------------------
// AmountSpec bounds
// ---------------------------------------------------------------------------

#[test]
fn absurd_trim_amounts_are_rejected_with_the_bound_stated() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    // seconds: 1e12 * 48_000 would leave u64 territory downstream.
    let response = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id, "end_offset": { "seconds": 1e12 } }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(
        message.contains(&format!("{} limit", MAX_SECONDS as u64)),
        "the error states the bound: {message}"
    );

    // beats: shares the notes.* bound.
    let response = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id, "end_offset": { "beats": 1e12 } }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(
        message.contains(&format!("{} limit", MAX_BEATS as u64)),
        "the error states the bound: {message}"
    );

    let clip = app
        .test_clips()
        .iter()
        .find(|c| c.id == clip_id)
        .expect("clip exists")
        .clone();
    assert_eq!(clip.trim_end_frames, 0, "a rejected trim changes nothing");
}

#[test]
fn amounts_at_the_bounds_are_accepted_and_clamped_as_before() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    // The bound itself passes validation; the existing leave-audio clamp
    // then applies, exactly as for any oversized-but-sane amount.
    let result: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id, "end_offset": { "seconds": MAX_SECONDS } }),
    )
    .result()
    .expect("the bound is inclusive");
    assert!(result.length_samples >= 1, "the clamp still leaves audio");
}
