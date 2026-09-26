//! Resizing a section must re-derive its generated clips to the new
//! length (code review VIEW-05).
//!
//! `handle_resize` used to only set `length_bars`: after a shrink the old
//! full-length clips kept playing over the bars a neighbour could now
//! occupy, and after a grow the drums — whose arrangement was pinned to
//! `Bars(old_len)` by `generate.drums` — stopped at the old length.

use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};

use crate::common::roundtrip;

const SYNTH: u64 = 10;
const DRUMS: u64 = 20;
const BAR_TICKS: u64 = 4 * TICKS_PER_QUARTER_NOTE;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// An 8-bar placed section with chords, a generated pad part and
/// generated drums.
fn generated_section() -> (Resonance, SectionDefinitionId) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/section-resize-rederive.rprj"));
    app.test_add_track(SYNTH, TrackType::Instrument);
    app.test_add_drum_track(DRUMS);

    let section_id = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 8,
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
    call(&mut app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");

    let part = proto::PartParams {
        section_id,
        track_id: ProtoTrackId(SYNTH),
        role: GenerateRole::Pad,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(42),
        options: None,
    };
    call(&mut app, "generate.part", &part)
        .result::<GenerateResult>()
        .expect("generate.part succeeds");

    let drums = proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(DRUMS),
        pattern: Some("four-on-floor".to_owned()),
        density: None,
        seed: Some(7),
    };
    call(&mut app, "generate.drums", &drums)
        .result::<GenerateResult>()
        .expect("generate.drums succeeds");

    assert_eq!(clip_on(&app, SYNTH).duration_ticks, 8 * BAR_TICKS);
    assert_eq!(clip_on(&app, DRUMS).duration_ticks, 8 * BAR_TICKS);
    (app, section_id)
}

fn clip_on(app: &Resonance, track: u64) -> MidiClipState {
    let clips: Vec<_> = app
        .test_midi_clips()
        .iter()
        .filter(|c| c.track_id == track)
        .collect();
    assert_eq!(clips.len(), 1, "expected one clip on track {track}");
    clips[0].clone()
}

fn resize(app: &mut Resonance, section_id: SectionDefinitionId, length_bars: u32) {
    let response = call(
        app,
        "section.resize",
        &section_proto::ResizeParams {
            section_id,
            length_bars,
        },
    );
    assert!(response.error.is_none(), "section.resize failed: {:?}", response.error);
}

#[test]
fn shrinking_a_section_shortens_its_generated_clips() {
    let (mut app, section_id) = generated_section();
    resize(&mut app, section_id, 4);

    for track in [SYNTH, DRUMS] {
        let clip = clip_on(&app, track);
        assert_eq!(clip.duration_ticks, 4 * BAR_TICKS, "track {track} clip length");
        assert!(
            clip.notes.iter().all(|n| n.start_tick < 4 * BAR_TICKS),
            "track {track} still has notes past the new end"
        );
    }
    assert_eq!(app.test_derived_clip_count(SYNTH), 1);
    assert_eq!(app.test_derived_clip_count(DRUMS), 1);
}

#[test]
fn growing_a_section_extends_its_drums_to_every_bar() {
    let (mut app, section_id) = generated_section();
    resize(&mut app, section_id, 16);

    assert_eq!(clip_on(&app, SYNTH).duration_ticks, 16 * BAR_TICKS);
    let drums = clip_on(&app, DRUMS);
    assert_eq!(drums.duration_ticks, 16 * BAR_TICKS);
    for bar in 0..16 {
        let (lo, hi) = (bar * BAR_TICKS, (bar + 1) * BAR_TICKS);
        assert!(
            drums.notes.iter().any(|n| n.start_tick >= lo && n.start_tick < hi),
            "bar {} has no drums after the grow",
            bar + 1
        );
    }
}
