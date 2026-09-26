//! Deleting a section placement — or a whole section with its placements —
//! must take the placement's generated clips with it (code review
//! VIEW-04).
//!
//! Both handlers used to only drop the placement / definition, leaving
//! every derived MIDI clip and rendered vocal audio clip installed in the
//! engine and in the project: the lane kept playing a section that no
//! longer existed, and re-placing + regenerating stacked a second copy.

use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, ClipId, TrackType};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};

use crate::common::roundtrip;

const SYNTH: u64 = 10;
const VOCAL: u64 = 11;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/section-delete-cleanup.rprj"));
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// A placed 4-bar section with a chord progression, returning its id.
fn placed_section(app: &mut Resonance) -> SectionDefinitionId {
    let section_id = call(
        app,
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

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
    section_id
}

/// Generate a pad part on the synth track and return the derived clip id.
fn generate_part(app: &mut Resonance, section_id: SectionDefinitionId) -> ClipId {
    let params = proto::PartParams {
        section_id,
        track_id: ProtoTrackId(SYNTH),
        role: GenerateRole::Pad,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(42),
        options: None,
    };
    call(app, "generate.part", &params)
        .result::<GenerateResult>()
        .expect("generate.part succeeds");
    assert_eq!(app.test_derived_clip_count(SYNTH), 1);
    app.test_midi_clips()
        .iter()
        .find(|c| c.track_id == SYNTH)
        .expect("the derived clip is mirrored into midi_clips")
        .id
}

fn placement_of(app: &Resonance, section_id: SectionDefinitionId) -> u64 {
    app.compose_state()
        .placements
        .iter()
        .find(|p| p.definition_id == u64::from(section_id))
        .expect("section is placed")
        .id
}

/// Every clip id the captured engine stream was told to delete.
fn deleted(rx: &Receiver<AudioCommand>) -> (Vec<ClipId>, Vec<ClipId>) {
    let (mut midi, mut audio) = (Vec::new(), Vec::new());
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            AudioCommand::DeleteMidiClip { clip_id } => midi.push(clip_id),
            AudioCommand::DeleteClip { clip_id } => audio.push(clip_id),
            _ => {}
        }
    }
    (midi, audio)
}

/// Generate a MIDI part and fake an installed vocal render on the one
/// placement, returning `(definition, placement, midi clip, vocal clip)`.
fn generated_section(app: &mut Resonance) -> (SectionDefinitionId, u64, ClipId, ClipId) {
    app.test_add_track(SYNTH, TrackType::Instrument);
    let section_id = placed_section(app);
    let midi_clip = generate_part(app, section_id);
    let placement = placement_of(app, section_id);
    let vocal_clip: ClipId = 9_999;
    app.test_install_vocal_audio_clip(
        u64::from(section_id),
        placement,
        VOCAL,
        vocal_clip,
        std::path::PathBuf::from("/tmp/section-delete-cleanup-vocal.wav"),
    );
    (section_id, placement, midi_clip, vocal_clip)
}

fn assert_purged(app: &Resonance, rx: &Receiver<AudioCommand>, midi_clip: ClipId, vocal_clip: ClipId) {
    let (midi, audio) = deleted(rx);
    assert!(midi.contains(&midi_clip), "DeleteMidiClip({midi_clip}) not sent: {midi:?}");
    assert!(audio.contains(&vocal_clip), "DeleteClip({vocal_clip}) not sent: {audio:?}");
    assert_eq!(app.test_derived_clip_count(SYNTH), 0, "derived_clips entry left behind");
    assert!(
        app.test_midi_clips().iter().all(|c| c.id != midi_clip),
        "the derived MIDI clip is still in the project"
    );
    assert!(app.test_vocal_audio_clips(VOCAL).is_empty(), "vocal audio entry left behind");
}

#[test]
fn deleting_a_placement_removes_its_generated_clips() {
    let mut app = app_with_project();
    let (_section, placement, midi_clip, vocal_clip) = generated_section(&mut app);

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Compose(ComposeMessage::DeleteSectionPlacement {
        placement_id: placement,
    }));

    assert_purged(&app, &rx, midi_clip, vocal_clip);
}

#[test]
fn deleting_a_section_removes_every_generated_lane() {
    let mut app = app_with_project();
    let (section, _placement, midi_clip, vocal_clip) = generated_section(&mut app);

    let rx = app.test_capture_engine();
    let response = call(
        &mut app,
        "section.delete",
        &section_proto::DeleteParams {
            section_id: section,
            confirm: true,
        },
    );
    assert!(response.error.is_none(), "section.delete failed: {:?}", response.error);

    assert_purged(&app, &rx, midi_clip, vocal_clip);
}
