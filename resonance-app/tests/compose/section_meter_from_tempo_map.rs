//! Compose reads a section's meter from the tempo map at the section, not
//! from the transport (code review VIEW-13).
//!
//! `transport.time_sig_num` / `transport.bpm` follow the playhead during
//! playback. Compose used them for clip lengths, chord-fit validation and
//! the vocal render tempo, so the same edit produced different results
//! depending on where the song happened to be playing.

use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};

use crate::common::roundtrip;

const BASS: u64 = 10;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// 4/4 at 100 BPM, switching to 3/4 at 140 BPM on (1-based) bar 17, with
/// a 4-bar section placed at bar 1 and the playhead parked past the
/// change while playing — so the transport reads 3/4, 140 BPM.
fn app_playing_past_meter_change(with_chords: bool) -> (Resonance, SectionDefinitionId) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_add_track(BASS, TrackType::Instrument);
    app.test_set_flat_tempo(100.0);
    call(
        &mut app,
        "global.add_signature_event",
        &serde_json::json!({ "bar": 17, "numerator": 3, "denominator": 4 }),
    )
    .result::<serde_json::Value>()
    .expect("add_signature_event succeeds");
    call(
        &mut app,
        "global.add_tempo_event",
        &serde_json::json!({ "bar": 17, "bpm": 140.0 }),
    )
    .result::<serde_json::Value>()
    .expect("add_tempo_event succeeds");

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
    if with_chords {
        let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
        params.key = Some(KeyScale {
            tonic: "A".to_owned(),
            scale: "minor".to_owned(),
        });
        params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
        call(&mut app, "harmony.apply_progression", &params)
            .result::<harmony_proto::ApplyProgressionResult>()
            .expect("progression applies");
    }

    let past_change = app.test_tempo_map().bar_to_sample(19);
    app.test_set_transport_playing(true);
    app.test_apply_engine_event(AudioEvent::PlayheadMoved(past_change));
    app.test_dispatch(Message::Tick);
    assert_eq!(
        app.test_transport_time_sig().0,
        3,
        "precondition: the transport follows the playhead into 3/4"
    );
    (app, section_id)
}

#[test]
fn regenerating_a_bar_one_section_uses_its_own_signature() {
    let (mut app, section_id) = app_playing_past_meter_change(true);
    call(
        &mut app,
        "generate.part",
        &proto::PartParams {
            section_id,
            track_id: ProtoTrackId(BASS),
            role: GenerateRole::Bass,
            chord_count: None,
            beats_per_chord: None,
            sevenths: None,
            seed: Some(3),
            options: None,
        },
    )
    .result::<GenerateResult>()
    .expect("generate.part succeeds");

    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.track_id == BASS)
        .expect("derived clip mirrored");
    assert_eq!(clip.duration_ticks, 4 * 4 * TICKS_PER_QUARTER_NOTE);
}

#[test]
fn chord_fit_does_not_depend_on_the_playhead() {
    let (mut app, section_id) = app_playing_past_meter_change(false);
    // Beats 13..16 exist in a 4-bar 4/4 section, not in 4 bars of 3/4.
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id,
            start_beat: 13.0,
            duration_beats: 3.0,
            symbol: "Am".to_owned(),
        },
    );
    assert!(response.error.is_none(), "chord at beat 13 rejected: {:?}", response.error);
}
