//! A drum group's Swing and Humanize knobs shape the notes it renders
//! (BUG-51): swing delays every odd step of the beat, humanize jitters
//! timing and velocity deterministically from the group's seed. Driven
//! through the same `build_drum_notes` the drum-clip materializer runs.

use resonance_app::compose::{ArrangementSpan, DrumGroup, DrumGroupPad};
use resonance_app::update::compose::build_drum_notes;
use resonance_audio::types::MidiNote;

const TS: u8 = 4;
const TPQN: u64 = 480;

/// One pad on 8ths (grid 2) hitting every step, two bars long.
fn eighths(swing: f32, humanize: f32, seed: u64) -> DrumGroup {
    DrumGroup {
        id: 1,
        name: "Hats".into(),
        color: [0, 0, 0],
        grid: 2,
        cycle: 8,
        phase: 0,
        pads: vec![DrumGroupPad {
            name: "Closed".into(),
            note: 42,
            weight: 100,
            pattern: vec![1; 8],
        }],
        density: 0.5,
        swing,
        accent: 0.0,
        humanize,
        fills: 0.0,
        style: "Custom".into(),
        seed,
    }
}

fn render(group: DrumGroup, bar_start: u32, bar_end: u32) -> Vec<MidiNote> {
    let span = ArrangementSpan {
        bar_start,
        bar_end,
        pattern_id: 1,
        is_fill: false,
    };
    build_drum_notes(&[(span, vec![group])], TS)
}

/// Comparable view of a note list (MidiNote has no `PartialEq`).
fn key(notes: &[MidiNote]) -> Vec<(u8, u64, u64, u32)> {
    notes
        .iter()
        .map(|n| (n.note, n.start_tick, n.duration_ticks, n.velocity.to_bits()))
        .collect()
}

fn starts(notes: &[MidiNote]) -> Vec<u64> {
    notes.iter().map(|n| n.start_tick).collect()
}

#[test]
fn zero_swing_and_humanize_render_straight() {
    let notes = render(eighths(0.0, 0.0, 7), 0, 2);
    let straight: Vec<u64> = (0..16).map(|s| s * TPQN / 2).collect();
    assert_eq!(starts(&notes), straight);
}

#[test]
fn swing_delays_the_offbeat_eighths_only() {
    let notes = render(eighths(0.5, 0.0, 7), 0, 2);
    let step = TPQN / 2;
    // Half swing moves each "and" a quarter of a step late; downbeats stay.
    let expected: Vec<u64> = (0..16)
        .map(|s| s * step + if s % 2 == 1 { step / 4 } else { 0 })
        .collect();
    assert_eq!(starts(&notes), expected);
    // A swung note ends where the next step begins — no overlap.
    for pair in notes.windows(2) {
        assert!(pair[0].start_tick + pair[0].duration_ticks <= pair[1].start_tick);
    }
}

#[test]
fn humanize_jitters_timing_and_velocity_deterministically() {
    let straight = render(eighths(0.0, 0.0, 7), 0, 2);
    let a = render(eighths(0.0, 1.0, 7), 0, 2);
    let b = render(eighths(0.0, 1.0, 7), 0, 2);
    assert_eq!(key(&a), key(&b), "same seed, same notes");

    let moved = a
        .iter()
        .zip(&straight)
        .filter(|(h, s)| h.start_tick != s.start_tick)
        .count();
    let louder_or_softer = a
        .iter()
        .zip(&straight)
        .filter(|(h, s)| h.velocity != s.velocity)
        .count();
    assert!(moved >= 8, "humanize moved only {moved} of 16 hits");
    assert!(louder_or_softer >= 8, "humanize changed only {louder_or_softer} velocities");
    for (h, s) in a.iter().zip(&straight) {
        let dt = h.start_tick as i64 - s.start_tick as i64;
        assert!(dt.abs() <= 12, "timing jitter {dt} ticks is out of range");
        let dv = (h.velocity - s.velocity).abs() * 127.0;
        assert!(dv <= 16.0, "velocity jitter {dv} is out of range");
        assert!(h.velocity >= 1.0 / 127.0 && h.velocity <= 1.0);
    }

    let other_seed = render(eighths(0.0, 1.0, 8), 0, 2);
    assert_ne!(key(&a), key(&other_seed), "a new seed rolls a new feel");
}

#[test]
fn humanize_never_moves_a_note_before_its_span() {
    // Every seed: the first hit of a span starting at bar 1 never lands in
    // bar 0.
    for seed in 0..64 {
        let notes = render(eighths(0.0, 1.0, seed), 1, 2);
        let span_start = TS as u64 * TPQN;
        assert!(
            notes.iter().all(|n| n.start_tick >= span_start),
            "seed {seed}: {:?}",
            starts(&notes)
        );
    }
}

// ---------------------------------------------------------------------------
// The knobs re-render the drum clips (the sliders used to change nothing).
// ---------------------------------------------------------------------------

use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::Resonance;
use resonance_audio::types::AudioCommand;

const DRUMS: u64 = 60;

/// The notes of the drum clip the engine was last told to load.
fn loaded_drum_notes(rx: &crossbeam_channel::Receiver<AudioCommand>) -> Option<Vec<MidiNote>> {
    let mut last = None;
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::LoadMidiClipDirect { track_id, notes, .. } = cmd {
            if track_id == DRUMS {
                last = Some(notes);
            }
        }
    }
    last
}

fn set_all(app: &mut Resonance, group_ids: &[u64], msg: impl Fn(u64) -> DrumGroupsMessage) {
    for &group_id in group_ids {
        let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(msg(group_id))));
    }
}

#[test]
fn swing_and_humanize_sliders_re_render_the_drum_clips() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
    let response = crate::common::call(
        &mut app,
        "section.create",
        serde_json::json!({ "name": "Verse", "length_bars": 2, "place": true }),
    );
    let section: resonance_control::methods::section::CreateResult =
        response.result().expect("section.create succeeds");
    app.test_add_drum_track(DRUMS);
    let response = crate::common::call(
        &mut app,
        "generate.drums",
        serde_json::json!({
            "section_id": section.section_id,
            "track_id": DRUMS,
            "pattern": "four-on-floor",
            "seed": 11,
        }),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let straight = loaded_drum_notes(&rx).expect("generate.drums loads a clip");

    let def_id: u64 = section.section_id.into();
    let pattern_id = app
        .compose_state()
        .find_definition(def_id)
        .and_then(|d| d.primary_pattern_id())
        .expect("the section has a pattern");
    let group_ids: Vec<u64> = app
        .compose_state()
        .drum_patterns
        .iter()
        .find(|p| p.id == pattern_id)
        .expect("pattern in the bank")
        .groups
        .iter()
        .map(|g| g.id)
        .collect();

    set_all(&mut app, &group_ids, |group_id| DrumGroupsMessage::SetGroupHumanize {
        group_id,
        humanize: 1.0,
    });
    let humanized = loaded_drum_notes(&rx).expect("a humanize change re-renders the clip");
    assert_ne!(key(&humanized), key(&straight), "humanize changed the notes");

    set_all(&mut app, &group_ids, |group_id| DrumGroupsMessage::SetGroupHumanize {
        group_id,
        humanize: 0.0,
    });
    assert_eq!(
        key(&loaded_drum_notes(&rx).expect("re-rendered")),
        key(&straight),
        "humanize back at 0 renders the straight groove again"
    );

    set_all(&mut app, &group_ids, |group_id| DrumGroupsMessage::SetGroupSwing {
        group_id,
        swing: 1.0,
    });
    // four-on-floor's groups run on 16ths with every hit on an even step,
    // so full 16th swing leaves its notes where they were — what the
    // knob does is pinned by the `build_drum_notes` cases above; here it
    // only has to reach the clip.
    assert!(
        loaded_drum_notes(&rx).is_some(),
        "a swing change re-renders the clip"
    );
}
