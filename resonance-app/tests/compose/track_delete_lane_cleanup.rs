//! Deleting a track must take its Compose lanes with it (code review
//! VIEW-12).
//!
//! Track removal used to prune only the arrange clips: every section kept
//! the removed track's lane generator, so the next chord edit re-derived
//! the lane and `LoadMidiClipDirect` created a clip on a track that no
//! longer exists — and that clip was saved with the project.

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
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

fn progression(app: &mut Resonance, section_id: SectionDefinitionId, numerals: [&str; 4]) {
    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(numerals.into_iter().map(str::to_owned).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
}

/// A placed 4-bar section with a generated bass lane on `BASS`.
fn app_with_bass_lane() -> (Resonance, SectionDefinitionId) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_add_track(BASS, TrackType::Instrument);

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
    progression(&mut app, section_id, ["i", "iv", "v", "i"]);
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
            seed: Some(7),
            options: None,
        },
    )
    .result::<GenerateResult>()
    .expect("generate.part succeeds");
    assert_eq!(app.test_derived_clip_count(BASS), 1);
    (app, section_id)
}

#[test]
fn track_delete_purges_its_compose_lanes() {
    let (mut app, section_id) = app_with_bass_lane();
    let rx = app.test_capture_engine();
    let clip_id = app
        .test_midi_clips()
        .iter()
        .find(|c| c.track_id == BASS)
        .expect("derived clip mirrored")
        .id;

    // The engine drops the track and echoes `TrackRemoved`.
    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: BASS });

    assert_eq!(app.test_derived_clip_count(BASS), 0, "derived clip entry left behind");
    assert!(
        app.test_lane_generator_tag(u64::from(section_id), BASS).is_none(),
        "the removed track's lane generator survived"
    );
    assert!(
        app.test_midi_clips().iter().all(|c| c.id != clip_id),
        "the derived MIDI clip is still in the project"
    );
    assert!(
        rx.try_iter()
            .any(|cmd| matches!(cmd, AudioCommand::DeleteMidiClip { clip_id: id } if id == clip_id)),
        "the engine was never told to drop the derived MIDI clip"
    );
}

#[test]
fn chord_edit_after_track_delete_creates_no_ghost_clip() {
    let (mut app, section_id) = app_with_bass_lane();
    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: BASS });

    // A chord edit re-derives every chord-reading lane of the section.
    let rx = app.test_capture_engine();
    progression(&mut app, section_id, ["i", "vi", "iv", "v"]);

    let loaded_on_dead: Vec<_> = rx
        .try_iter()
        .filter_map(|cmd| match cmd {
            AudioCommand::LoadMidiClipDirect { clip_id, track_id, .. } if track_id == BASS => {
                Some(clip_id)
            }
            _ => None,
        })
        .collect();
    assert!(loaded_on_dead.is_empty(), "clip loaded on the deleted track: {loaded_on_dead:?}");
    let registry = app.test_registry();
    let orphans: Vec<_> = app
        .test_midi_clips()
        .iter()
        .filter(|c| !registry.tracks.iter().any(|t| t.id == c.track_id))
        .map(|c| (c.id, c.track_id))
        .collect();
    assert!(orphans.is_empty(), "MIDI clips on tracks missing from the registry: {orphans:?}");
}

/// The guard on the derive path itself: a generator left pointing at a
/// track that isn't in the registry (e.g. restored by an older save) must
/// not re-derive onto it.
#[test]
fn regenerate_skips_a_lane_whose_track_is_gone() {
    let (mut app, section_id) = app_with_bass_lane();
    // Drop the track from the registry without the removal hook, as a
    // project saved before the fix would load.
    app.test_registry_mut().tracks.retain(|t| t.id != BASS);

    let rx = app.test_capture_engine();
    progression(&mut app, section_id, ["i", "vi", "iv", "v"]);
    assert!(
        !rx.try_iter()
            .any(|cmd| matches!(cmd, AudioCommand::LoadMidiClipDirect { track_id, .. } if track_id == BASS)),
        "a lane was re-derived onto a track that is not in the registry"
    );
}
