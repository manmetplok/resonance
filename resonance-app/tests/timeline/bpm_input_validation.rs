//! Tempo input validation (review VIEW-06).
//!
//! `"nan".parse::<f32>()` is `Ok(NaN)` and NaN survives `clamp`, so the
//! BPM field used to accept it: the bar table went NaN, every clip was
//! re-anchored to sample 0, and the saved project (`"bpm": null`) would
//! not load again.

use resonance_app::message::{GlobalTrackMessage, Message, TransportMessage};
use resonance_app::project::ProjectFile;
use resonance_app::Resonance;
use resonance_audio::types::AudioCommand;

fn commit(app: &mut Resonance, text: &str) {
    app.test_dispatch(Message::Transport(TransportMessage::SetBpmText(text.into())));
    app.test_dispatch(Message::Transport(TransportMessage::CommitBpm));
}

#[test]
fn nan_bpm_text_is_rejected() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_sample_rate(48_000);
    commit(&mut app, "120");
    let bar1 = app.test_tempo_map().bar_to_sample(1);
    let _ = rx.try_iter().count();

    for bad in ["nan", "NaN", "-nan", "inf", "-inf"] {
        commit(&mut app, bad);
        assert_eq!(app.test_transport_bpm(), 120.0, "{bad:?} changed the tempo");
        assert!(
            app.test_tempo_events().iter().all(|e| e.bpm == 120.0),
            "{bad:?} reached the tempo events"
        );
        assert_eq!(
            app.test_tempo_map().bar_to_sample(1),
            bar1,
            "{bad:?} moved the bar grid"
        );
        let sent: Vec<f32> = rx
            .try_iter()
            .filter_map(|c| match c {
                AudioCommand::SetBpm { bpm } => Some(bpm),
                _ => None,
            })
            .collect();
        assert!(sent.is_empty(), "{bad:?} sent SetBpm {sent:?}");
    }
}

#[test]
fn out_of_range_bpm_text_is_clamped() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(48_000);
    commit(&mut app, "0");
    assert_eq!(app.test_transport_bpm(), 20.0);
    commit(&mut app, "-50");
    assert_eq!(app.test_transport_bpm(), 20.0);
    commit(&mut app, "1000");
    assert_eq!(app.test_transport_bpm(), 300.0);
}

#[test]
fn nan_tempo_lane_edit_is_ignored() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(48_000);
    commit(&mut app, "120");
    app.test_dispatch(Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent {
        index: 0,
        bar: 0,
        bpm: f32::NAN,
    }));
    assert_eq!(app.test_tempo_events()[0].bpm, 120.0);
    assert_eq!(app.test_transport_bpm(), 120.0);
}

/// A project file that already holds a non-finite tempo (serde_json
/// writes NaN as `null`) must load with a sane fallback instead of
/// failing to parse.
#[test]
fn project_with_null_bpm_loads_with_fallback() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(48_000);
    commit(&mut app, "120");
    let mut json = serde_json::to_value(app.test_build_project_file()).unwrap();
    json["bpm"] = serde_json::Value::Null;
    json["tempo_events"] = serde_json::json!([
        { "bar": 0, "bpm": null },
        { "bar": 4, "bpm": 900.0 },
    ]);
    let file: ProjectFile = serde_json::from_value(json).expect("project must still load");
    assert_eq!(file.bpm, 120.0);
    assert_eq!(file.tempo_events[0].bpm, 120.0);
    assert_eq!(file.tempo_events[1].bpm, 300.0);

    app.test_replay_loaded_project(file);
    assert_eq!(app.test_transport_bpm(), 120.0);
    assert!(app.test_tempo_map().bar_to_sample(1) > 0);
}
