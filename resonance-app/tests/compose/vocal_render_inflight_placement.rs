//! A vocal render that completes after its placement moved or was deleted
//! installs its audio where the placement is *now* (code review VIEW-19).
//!
//! The completion carried the `(placement_id, start_sample)` list captured
//! when the render was queued, and the only guard was the lane epoch —
//! which moving a placement (or inserting bars before it) does not bump.
//! So the audio landed at the section's old position.

use resonance_app::compose::messages::VocalAudioReadyData;
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, TrackType};
use resonance_control::methods::arrangement::InsertBarsParams;
use resonance_control::methods::section as section_proto;
use resonance_control::{Request, Response};

use crate::common::roundtrip;

const VOCAL: u64 = 11;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// A placed 4-bar section with a vocal lane whose render (epoch 1) is in
/// flight. Returns `(app, definition, placement, queued start sample)`.
fn app_with_inflight_render() -> (Resonance, u64, u64, u64) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_add_track(VOCAL, TrackType::Vocal);
    let section_id = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let definition = u64::from(section_id);
    app.test_install_vocal_lane(definition, VOCAL);
    app.test_set_vocal_render_epoch(definition, VOCAL, 1);
    let (placement, start_bar) = {
        let p = &app.compose_state().placements[0];
        (p.id, p.start_bar)
    };
    let queued_start = app.test_tempo_map().bar_to_sample(start_bar);
    (app, definition, placement, queued_start)
}

fn ready(definition: u64, placement: u64, queued_start: u64) -> Message {
    Message::Compose(ComposeMessage::VocalAudioReady(Box::new(VocalAudioReadyData {
        definition_id: definition,
        track_id: VOCAL,
        wav_path: std::path::PathBuf::from("/tmp/nonexistent-vocal-inflight.wav"),
        placements: vec![(placement, queued_start)],
        clip_name: "Verse · Vocal".to_owned(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        lead_ticks: 0,
        render_epoch: 1,
    })))
}

fn loaded_starts(rx: &resonance_audio::__test_support::Receiver<AudioCommand>) -> Vec<u64> {
    rx.try_iter()
        .filter_map(|cmd| match cmd {
            AudioCommand::LoadClipFromWav { start_sample, .. } => Some(start_sample),
            _ => None,
        })
        .collect()
}

#[test]
fn audio_lands_at_the_placement_moved_while_rendering() {
    let (mut app, definition, placement, queued_start) = app_with_inflight_render();

    // Insert two bars before the section while the render runs.
    let response = call(
        &mut app,
        "arrangement.insert_bars",
        &InsertBarsParams { at_bar: 1, count: 2 },
    );
    assert!(response.error.is_none(), "insert_bars failed: {:?}", response.error);
    let moved = app.compose_state().find_placement(placement).expect("placement kept");
    let now_start = app.test_tempo_map().bar_to_sample(moved.start_bar);
    assert_ne!(now_start, queued_start, "precondition: the placement moved");

    let rx = app.test_capture_engine();
    let _ = app.update(ready(definition, placement, queued_start));
    assert_eq!(loaded_starts(&rx), vec![now_start], "audio installed at the stale position");
}

#[test]
fn audio_is_not_installed_for_a_placement_deleted_while_rendering() {
    let (mut app, definition, placement, queued_start) = app_with_inflight_render();
    let _ = app.update(Message::Compose(ComposeMessage::DeleteSectionPlacement {
        placement_id: placement,
    }));

    let rx = app.test_capture_engine();
    let _ = app.update(ready(definition, placement, queued_start));
    assert!(loaded_starts(&rx).is_empty(), "audio installed for a deleted placement");
    assert!(app.test_vocal_audio_clips(VOCAL).is_empty());
}
