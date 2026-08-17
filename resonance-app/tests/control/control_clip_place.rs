//! `pool.*` / `clip.*` through the real update path (ba doc #265):
//! placing a sample on the timeline and editing the placement.
//!
//! The import half needs an engine worker thread, so these tests drive
//! the half that does not: an asset already in the pool. That is the same
//! code path a fresh import lands in — `clip.place` short-circuits to it
//! whenever the file is known — so placement, trimming, fades, gain,
//! moves and deletes are all exercised against the real handlers and the
//! real undo/revision machinery.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{ClipState, PoolAsset, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{FadeCurve, TrackType};
use resonance_common::AudioFormat;
use resonance_control::job::{JobState, JobStatus};
use resonance_control::methods::clip::{FadeResult, PlaceResult, TrimResult};
use resonance_control::methods::pool::PoolView;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const SR: u32 = 48_000;
const AUDIO_TRACK: u64 = 1;
const MIDI_TRACK: u64 = 2;
const ASSET: u64 = 7;
/// Four seconds at the project rate — long enough to trim both edges.
const ASSET_FRAMES: u64 = 4 * SR as u64;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-clip-place.rprj"));
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

fn roundtrip(app: &mut Resonance, id: i64, method: &str, params: serde_json::Value) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let request = Request::new(id, method, &params).expect("params serialize");
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, 1, method, params)
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

/// Place the fixture asset at `bar` and return the job's `PlaceResult`.
/// An already-pooled asset resolves its job inside the handler, so the
/// status is terminal by the time the reply is read.
fn place_at(app: &mut Resonance, bar: u32) -> PlaceResult {
    let response = call(
        app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "asset_id": ASSET,
            "start": { "bar": bar },
        }),
    );
    let started: resonance_control::job::JobStarted =
        response.result().expect("clip.place starts a job");
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

fn job_status(app: &mut Resonance, job_id: u64) -> JobStatus {
    let response = call(app, "job.status", serde_json::json!({ "job_id": job_id }));
    response.result().expect("job.status succeeds")
}

fn clip(app: &Resonance, clip_id: u64) -> ClipState {
    app.test_clips()
        .iter()
        .find(|c| c.id == clip_id)
        .expect("clip exists")
        .clone()
}

fn bar_to_sample(app: &Resonance, bar: u32) -> u64 {
    app.test_tempo_map().bar_to_sample(bar)
}

// ---------------------------------------------------------------------------
// pool.list
// ---------------------------------------------------------------------------

#[test]
fn pool_list_reports_the_project_assets() {
    let mut app = app();
    let view: PoolView = call(&mut app, "pool.list", serde_json::json!({}))
        .result()
        .expect("pool.list succeeds");

    assert_eq!(view.assets.len(), 1);
    let asset = &view.assets[0];
    assert_eq!(u64::from(asset.id), ASSET);
    assert_eq!(asset.original_path, "/samples/kick.wav");
    // The name a placed clip gets: the source file's stem.
    assert_eq!(asset.name, "kick");
    assert_eq!(asset.duration_frames, ASSET_FRAMES);
    assert!((asset.duration_seconds - 4.0).abs() < 1e-9);
    assert_eq!(asset.source_sample_rate, 44_100);
    assert_eq!(asset.format, "wav");
    assert_eq!(asset.usage_count, 0, "nothing placed yet");
    assert!(!asset.missing);
}

#[test]
fn placing_raises_the_asset_usage_count() {
    let mut app = app();
    place_at(&mut app, 1);
    place_at(&mut app, 3);

    let view: PoolView = call(&mut app, "pool.list", serde_json::json!({}))
        .result()
        .expect("pool.list succeeds");
    assert_eq!(view.assets[0].usage_count, 2, "two clips play the asset");
}

// ---------------------------------------------------------------------------
// clip.place
// ---------------------------------------------------------------------------

#[test]
fn places_a_pooled_asset_and_reports_the_clip() {
    let mut app = app();
    let target = bar_to_sample(&app, 4);

    let placed = place_at(&mut app, 5); // wire bar 5 = 0-based bar 4

    assert_eq!(u64::from(placed.track_id), AUDIO_TRACK);
    assert_eq!(u64::from(placed.asset_id), ASSET);
    assert_eq!(placed.start.sample, target);
    assert_eq!(placed.start.bar, 5);
    assert_eq!(placed.length_samples, ASSET_FRAMES);
    assert_eq!(placed.name, "kick");

    // The reported id addresses a real clip on the real track, tied back
    // to the asset it came from.
    let clip = clip(&app, u64::from(placed.clip_id));
    assert_eq!(clip.track_id, AUDIO_TRACK);
    assert_eq!(clip.start_sample, target);
    assert_eq!(clip.total_frames, ASSET_FRAMES);
    assert_eq!(
        clip.asset_ref.map(|r| r.asset_id),
        Some(ASSET),
        "the placement links the clip to its pool asset"
    );
}

#[test]
fn a_sample_position_is_not_snapped_to_the_grid() {
    // The GUI drop path snaps to the grid at the current zoom. An API
    // caller that names a sample must get that sample.
    let mut app = app();
    let off_grid = bar_to_sample(&app, 2) + 731;

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "asset_id": ASSET,
            "start": { "sample": off_grid },
        }),
    );
    let started: resonance_control::job::JobStarted = response.result().expect("job starts");
    let status = job_status(&mut app, u64::from(started.job_id));
    let placed: PlaceResult =
        serde_json::from_value(status.result.expect("result")).expect("PlaceResult");

    assert_eq!(placed.start.sample, off_grid);
    assert_eq!(clip(&app, u64::from(placed.clip_id)).start_sample, off_grid);
}

#[test]
fn place_defaults_to_bar_one() {
    let mut app = app();
    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": AUDIO_TRACK, "asset_id": ASSET }),
    );
    let started: resonance_control::job::JobStarted = response.result().expect("job starts");
    let status = job_status(&mut app, u64::from(started.job_id));
    let placed: PlaceResult =
        serde_json::from_value(status.result.expect("result")).expect("PlaceResult");
    assert_eq!(placed.start.sample, 0);
}

#[test]
fn placing_is_one_undoable_step() {
    let mut app = app();
    let before = app.revision();
    let placed = place_at(&mut app, 1);
    assert!(app.revision() > before, "the placement bumps the revision");
    assert!(app.test_clips().iter().any(|c| c.id == u64::from(placed.clip_id)));

    // One entry, whose snapshot predates the placement — the same shape
    // `import_placement.rs` pins for a dragged-in file, so undoing a
    // control placement rewinds exactly as far as undoing a GUI one.
    let history = app.test_undo_history();
    let entries = history.test_undo_entries();
    assert_eq!(entries.len(), 1, "exactly one undo entry for the placement");
    assert!(
        entries[0].project.file.clips.is_empty(),
        "the snapshot predates the placed clip"
    );
}

#[test]
fn place_rejects_a_non_audio_track() {
    let mut app = app();
    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": MIDI_TRACK, "asset_id": ASSET }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("audio track"), "{message}");
    assert!(app.test_clips().is_empty(), "nothing was placed");
}

#[test]
fn place_needs_exactly_one_source() {
    let mut app = app();

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": AUDIO_TRACK }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("asset_id or a path"), "{message}");

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "asset_id": ASSET,
            "path": "/samples/kick.wav",
        }),
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("not both"), "{message}");
}

#[test]
fn place_rejects_unknown_ids() {
    let mut app = app();

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": 999, "asset_id": ASSET }),
    );
    assert!(expect_error(response, ErrorKind::NotFound).contains("track"));

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": AUDIO_TRACK, "asset_id": 999 }),
    );
    assert!(expect_error(response, ErrorKind::NotFound).contains("pool.list"));

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": AUDIO_TRACK, "path": "/nope/missing.wav" }),
    );
    assert!(expect_error(response, ErrorKind::NotFound).contains("no file at"));
}

#[test]
fn a_path_already_in_the_pool_reuses_its_asset() {
    // The whole point of matching on original_path: placing the same file
    // by path a second time must not import a duplicate.
    let mut app = app();
    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "path": "/samples/kick.wav",
            "start": { "bar": 2 },
        }),
    );
    let started: resonance_control::job::JobStarted = response.result().expect("job starts");
    let status = job_status(&mut app, u64::from(started.job_id));
    assert_eq!(
        status.state,
        JobState::Done,
        "a known path places without importing: {:?}",
        status.error
    );
    let placed: PlaceResult =
        serde_json::from_value(status.result.expect("result")).expect("PlaceResult");
    assert_eq!(u64::from(placed.asset_id), ASSET);
    assert_eq!(app.test_pool().assets.len(), 1, "no duplicate asset");
    assert_eq!(app.test_pending_import_count(), 0, "no import was queued");
}

// ---------------------------------------------------------------------------
// The async half: a path that has to be imported first
// ---------------------------------------------------------------------------

/// An `AssetImported` event for `original`, as the engine's import worker
/// emits it — the same fixture `import_placement.rs` uses.
fn asset_imported(id: u64, original: &str, frames: u64) -> resonance_audio::types::AudioEvent {
    resonance_audio::types::AudioEvent::AssetImported {
        asset_id: id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: original.to_string(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 48_000,
        duration_frames: frames,
        peaks: vec![(-0.5, 0.5)],
    }
}

/// Write a real file so `clip.place`'s existence check passes, and return
/// its path. Named per test so parallel runs don't collide.
fn temp_source(name: &str) -> String {
    let path = std::env::temp_dir().join(format!("resonance-control-clip-{name}.wav"));
    std::fs::write(&path, b"RIFF").expect("write fixture");
    path.to_string_lossy().into_owned()
}

#[test]
fn an_unpooled_path_waits_for_its_import_then_reports_the_clip() {
    let mut app = app();
    let source = temp_source("place-async");
    let target = bar_to_sample(&app, 2);

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({
            "track_id": AUDIO_TRACK,
            "path": source,
            "start": { "bar": 3 },
        }),
    );
    let started: resonance_control::job::JobStarted = response.result().expect("job starts");
    let job_id = u64::from(started.job_id);

    // Still importing: the engine hasn't reported, so the job must not
    // claim a result it does not have.
    let status = job_status(&mut app, job_id);
    assert!(!matches!(status.state, JobState::Done | JobState::Error));
    assert_eq!(app.test_pending_import_count(), 1, "a placement is queued");

    // The worker finishes: the asset lands, the queued placement runs, and
    // the job resolves with the real clip.
    app.test_handle_engine_event(asset_imported(42, &source, 96_000));

    let status = job_status(&mut app, job_id);
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    let placed: PlaceResult =
        serde_json::from_value(status.result.expect("result")).expect("PlaceResult");
    assert_eq!(u64::from(placed.asset_id), 42);
    assert_eq!(u64::from(placed.track_id), AUDIO_TRACK);
    assert_eq!(placed.start.sample, target, "placed where it was asked, unsnapped");
    assert_eq!(placed.length_samples, 96_000);

    let clip = clip(&app, u64::from(placed.clip_id));
    assert_eq!(clip.start_sample, target);
    assert_eq!(clip.asset_ref.map(|r| r.asset_id), Some(42));
}

#[test]
fn a_failed_import_fails_the_place_job() {
    let mut app = app();
    let source = temp_source("place-fails");

    let response = call(
        &mut app,
        "clip.place",
        serde_json::json!({ "track_id": AUDIO_TRACK, "path": source }),
    );
    let started: resonance_control::job::JobStarted = response.result().expect("job starts");
    let job_id = u64::from(started.job_id);

    app.test_handle_engine_event(resonance_audio::types::AudioEvent::ImportFailed {
        asset_id: 43,
        path: source.clone(),
        reason: "unsupported codec".to_string(),
    });

    let status = job_status(&mut app, job_id);
    assert_eq!(status.state, JobState::Error, "a failed import fails the job");
    let error = status.error.expect("the failure names the file and reason");
    assert!(error.contains("unsupported codec"), "{error}");
    assert!(app.test_clips().is_empty(), "nothing was placed");
}

#[test]
fn pool_import_resolves_only_when_every_file_has_landed() {
    let mut app = app();
    let first = temp_source("batch-a");
    let second = temp_source("batch-b");

    let response = call(
        &mut app,
        "pool.import",
        serde_json::json!({ "paths": [first, second] }),
    );
    let started: resonance_control::job::JobStarted = response.result().expect("job starts");
    let job_id = u64::from(started.job_id);

    // One of two: the batch must not resolve on the first file, or a
    // client that waited on it would read a half-imported pool as done.
    app.test_handle_engine_event(asset_imported(50, &first, 24_000));
    let status = job_status(&mut app, job_id);
    assert!(
        !matches!(status.state, JobState::Done | JobState::Error),
        "still waiting on the second file"
    );

    app.test_handle_engine_event(asset_imported(51, &second, 48_000));
    let status = job_status(&mut app, job_id);
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);

    let result: resonance_control::methods::pool::ImportResult =
        serde_json::from_value(status.result.expect("result")).expect("ImportResult");
    let ids: Vec<u64> = result.assets.iter().map(|a| u64::from(a.id)).collect();
    assert_eq!(ids, vec![50, 51], "the batch reports exactly its own assets");
    // A pool-only import places nothing.
    assert!(app.test_clips().is_empty());
}

// ---------------------------------------------------------------------------
// clip.move
// ---------------------------------------------------------------------------

#[test]
fn moves_a_clip_in_time_and_across_tracks() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);
    let other = 3;
    app.test_add_track(other, TrackType::Audio);
    let target = bar_to_sample(&app, 8);

    let response = call(
        &mut app,
        "clip.move",
        serde_json::json!({
            "clip_id": clip_id,
            "start": { "bar": 9 },
            "track_id": other,
        }),
    );
    let _: MutationAck = response.result().expect("clip.move succeeds");

    let moved = clip(&app, clip_id);
    assert_eq!(moved.start_sample, target);
    assert_eq!(moved.track_id, other);
}

#[test]
fn move_keeps_the_track_when_none_is_given() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let _: MutationAck = call(
        &mut app,
        "clip.move",
        serde_json::json!({ "clip_id": clip_id, "start": { "bar": 3 } }),
    )
    .result()
    .expect("clip.move succeeds");

    assert_eq!(clip(&app, clip_id).track_id, AUDIO_TRACK);
}

#[test]
fn move_rejects_a_non_audio_destination() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let response = call(
        &mut app,
        "clip.move",
        serde_json::json!({
            "clip_id": clip_id,
            "start": { "bar": 2 },
            "track_id": MIDI_TRACK,
        }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("audio track"));
    assert_eq!(
        clip(&app, clip_id).track_id,
        AUDIO_TRACK,
        "a rejected move changes nothing"
    );
}

// ---------------------------------------------------------------------------
// clip.trim
// ---------------------------------------------------------------------------

#[test]
fn trims_both_edges_in_seconds() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let result: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({
            "clip_id": clip_id,
            "start_offset": { "seconds": 0.5 },
            "end_offset": { "seconds": 1.0 },
        }),
    )
    .result()
    .expect("clip.trim succeeds");

    assert_eq!(result.start_offset_samples, SR as u64 / 2);
    assert_eq!(result.end_offset_samples, SR as u64);
    assert_eq!(result.length_samples, ASSET_FRAMES - SR as u64 * 3 / 2);

    let trimmed = clip(&app, clip_id);
    assert_eq!(trimmed.trim_start_frames, SR as u64 / 2);
    assert_eq!(trimmed.trim_end_frames, SR as u64);
    assert_eq!(trimmed.duration_samples, result.length_samples);
}

#[test]
fn trimming_the_head_leaves_the_clip_where_it_was() {
    // Documented behaviour: the head trim hides source, it does not move
    // the clip. `start` is how a caller compensates.
    let mut app = app();
    let placed = place_at(&mut app, 3);
    let clip_id = u64::from(placed.clip_id);
    let original_start = clip(&app, clip_id).start_sample;

    let _: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({
            "clip_id": clip_id,
            "start_offset": { "samples": 4_800 },
        }),
    )
    .result()
    .expect("clip.trim succeeds");
    assert_eq!(clip(&app, clip_id).start_sample, original_start);

    let result: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({
            "clip_id": clip_id,
            "start_offset": { "samples": 9_600 },
            "start": { "sample": original_start + 9_600 },
        }),
    )
    .result()
    .expect("clip.trim succeeds");
    assert_eq!(result.start.sample, original_start + 9_600);
    assert_eq!(clip(&app, clip_id).start_sample, original_start + 9_600);
}

#[test]
fn trim_offsets_are_clamped_to_leave_audio() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let result: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({
            "clip_id": clip_id,
            "start_offset": { "samples": ASSET_FRAMES * 2 },
            "end_offset": { "samples": ASSET_FRAMES * 2 },
        }),
    )
    .result()
    .expect("clip.trim succeeds");

    assert!(result.length_samples >= 1, "at least one frame survives");
    assert_eq!(
        result.start_offset_samples + result.end_offset_samples + result.length_samples,
        ASSET_FRAMES
    );
}

#[test]
fn trim_restores_an_edge_with_zero() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let _: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id, "end_offset": { "seconds": 2.0 } }),
    )
    .result()
    .expect("trim succeeds");
    let result: TrimResult = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id, "end_offset": { "samples": 0 } }),
    )
    .result()
    .expect("trim succeeds");

    assert_eq!(result.end_offset_samples, 0);
    assert_eq!(result.length_samples, ASSET_FRAMES);
}

#[test]
fn trim_rejects_an_empty_or_ambiguous_amount() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let response = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("changed nothing"));

    let response = call(
        &mut app,
        "clip.trim",
        serde_json::json!({
            "clip_id": clip_id,
            "start_offset": { "beats": 1.0, "seconds": 1.0 },
        }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("exactly one"));

    let response = call(
        &mut app,
        "clip.trim",
        serde_json::json!({ "clip_id": clip_id, "start_offset": {} }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("beats, seconds or samples"));
}

// ---------------------------------------------------------------------------
// clip.set_gain / clip.set_fade
// ---------------------------------------------------------------------------

#[test]
fn sets_clip_gain() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let _: MutationAck = call(
        &mut app,
        "clip.set_gain",
        serde_json::json!({ "clip_id": clip_id, "gain_db": -6.0 }),
    )
    .result()
    .expect("clip.set_gain succeeds");
    assert!((clip(&app, clip_id).gain_db - (-6.0)).abs() < 1e-6);
}

#[test]
fn clip_gain_is_clamped_and_rejects_nonsense() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let _: MutationAck = call(
        &mut app,
        "clip.set_gain",
        serde_json::json!({ "clip_id": clip_id, "gain_db": 500.0 }),
    )
    .result()
    .expect("clip.set_gain succeeds");
    assert!(
        clip(&app, clip_id).gain_db <= 24.0,
        "gain is clamped, got {}",
        clip(&app, clip_id).gain_db
    );

    let response = call(
        &mut app,
        "clip.set_gain",
        serde_json::json!({ "clip_id": clip_id, "gain_db": f64::INFINITY }),
    );
    // serde rejects a non-finite f32 before the handler sees it; either
    // way the call must fail rather than poison the clip.
    assert!(response.error.is_some(), "a non-finite gain is refused");
}

#[test]
fn sets_fade_lengths_and_shapes() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let result: FadeResult = call(
        &mut app,
        "clip.set_fade",
        serde_json::json!({
            "clip_id": clip_id,
            "fade_in": { "seconds": 0.01 },
            "fade_out": { "seconds": 0.5 },
            "fade_in_shape": "linear",
            "fade_out_shape": "exp",
        }),
    )
    .result()
    .expect("clip.set_fade succeeds");

    // ~10 ms and 500 ms, allowing for the ms round-trip the app's setters
    // take (the inspector's unit).
    assert!(
        result.fade_in_samples.abs_diff(480) <= 48,
        "fade in {} frames",
        result.fade_in_samples
    );
    assert!(
        result.fade_out_samples.abs_diff(24_000) <= 48,
        "fade out {} frames",
        result.fade_out_samples
    );

    let faded = clip(&app, clip_id);
    assert_eq!(faded.fade_in_curve, FadeCurve::Linear);
    assert_eq!(faded.fade_out_curve, FadeCurve::Exp);
}

#[test]
fn a_shape_can_be_changed_without_touching_lengths() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let before: FadeResult = call(
        &mut app,
        "clip.set_fade",
        serde_json::json!({ "clip_id": clip_id, "fade_in": { "seconds": 0.25 } }),
    )
    .result()
    .expect("set_fade succeeds");

    let after: FadeResult = call(
        &mut app,
        "clip.set_fade",
        serde_json::json!({ "clip_id": clip_id, "fade_in_shape": "linear" }),
    )
    .result()
    .expect("set_fade succeeds");

    assert_eq!(after.fade_in_samples, before.fade_in_samples);
    assert_eq!(clip(&app, clip_id).fade_in_curve, FadeCurve::Linear);
}

#[test]
fn a_fade_is_clamped_to_the_clip_length() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let result: FadeResult = call(
        &mut app,
        "clip.set_fade",
        serde_json::json!({ "clip_id": clip_id, "fade_in": { "seconds": 60.0 } }),
    )
    .result()
    .expect("set_fade succeeds");
    assert!(
        result.fade_in_samples <= ASSET_FRAMES,
        "fade {} exceeds the {ASSET_FRAMES}-frame clip",
        result.fade_in_samples
    );
}

#[test]
fn set_fade_rejects_a_no_op_and_an_unknown_shape() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let response = call(
        &mut app,
        "clip.set_fade",
        serde_json::json!({ "clip_id": clip_id }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("changed nothing"));

    let response = call(
        &mut app,
        "clip.set_fade",
        serde_json::json!({ "clip_id": clip_id, "fade_in_shape": "s_curve" }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("unknown fade shape"));
}

// ---------------------------------------------------------------------------
// clip.delete + wrong-kind routing
// ---------------------------------------------------------------------------

#[test]
fn deletes_a_clip_and_keeps_its_asset() {
    let mut app = app();
    let placed = place_at(&mut app, 1);
    let clip_id = u64::from(placed.clip_id);

    let _: MutationAck = call(
        &mut app,
        "clip.delete",
        serde_json::json!({ "clip_id": clip_id }),
    )
    .result()
    .expect("clip.delete succeeds");

    assert!(!app.test_clips().iter().any(|c| c.id == clip_id));
    assert_eq!(
        app.test_pool().assets.len(),
        1,
        "the pool asset outlives its clip so it can be placed again"
    );
}

#[test]
fn a_midi_clip_is_refused_with_a_pointer_to_notes() {
    let mut app = app();
    app.test_push_midi_clip(resonance_app::state::MidiClipState {
        id: 500,
        track_id: MIDI_TRACK,
        start_sample: 0,
        duration_ticks: 1920,
        name: "part".to_owned(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });

    for method in ["clip.move", "clip.trim", "clip.delete", "clip.set_gain"] {
        let response = call(
            &mut app,
            method,
            serde_json::json!({
                "clip_id": 500,
                "start": { "bar": 2 },
                "start_offset": { "samples": 10 },
                "gain_db": 0.0,
            }),
        );
        let message = expect_error(response, ErrorKind::NotFound);
        assert!(message.contains("MIDI clip"), "{method}: {message}");
        assert!(message.contains("notes"), "{method}: {message}");
    }
}

#[test]
fn an_unknown_clip_is_not_found() {
    let mut app = app();
    let response = call(
        &mut app,
        "clip.delete",
        serde_json::json!({ "clip_id": 4242 }),
    );
    assert!(expect_error(response, ErrorKind::NotFound).contains("no audio clip"));
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

#[test]
fn importing_needs_a_saved_project() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    // Deliberately no project path: imported audio would have nowhere to
    // live, and the app's own import handler refuses.
    app.test_add_track(AUDIO_TRACK, TrackType::Audio);

    let response = call(
        &mut app,
        "pool.import",
        serde_json::json!({ "paths": ["/samples/kick.wav"] }),
    );
    let message = expect_error(response, ErrorKind::Busy);
    assert!(message.contains("project.save_as"), "{message}");
}

#[test]
fn import_validates_the_batch_up_front() {
    let mut app = app();

    let response = call(&mut app, "pool.import", serde_json::json!({ "paths": [] }));
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("no paths"));

    let response = call(
        &mut app,
        "pool.import",
        serde_json::json!({ "paths": ["relative/path.wav"] }),
    );
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("absolute"));

    let many: Vec<String> = (0..100).map(|i| format!("/samples/s{i}.wav")).collect();
    let response = call(&mut app, "pool.import", serde_json::json!({ "paths": many }));
    assert!(expect_error(response, ErrorKind::InvalidParams).contains("exceeds"));

    assert_eq!(app.test_pending_import_count(), 0, "nothing was queued");
}
