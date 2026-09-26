//! Take-lane rendering: stacked takes + comp overlay (epic #15, doc #165,
//! todo #413).
//!
//! Cycle recording keeps every loop pass as a take. This suite pins the
//! surface that makes them usable:
//!
//! * **one lane per slot** — every take of a group stacks into a single
//!   lane no matter how many record runs produced it (the ruling behind
//!   todo #1392); a track recorded over two different loop regions gets two
//!   contiguous stacks that never interleave;
//! * the **effective cover** the lane draws from — `active take → comp
//!   segment → latest take`, gap-free by construction, so every point of
//!   the slot maps to exactly one take. Since todo #1395 that is
//!   `resonance_common::effective_cover`, the same resolution the mixer
//!   reads, and `the_engine_and_the_lane_resolve_the_same_cover` re-runs a
//!   shared fixture through `build_comp_table` so neither side can drift
//!   alone;
//! * the **audible extent** — a take does not necessarily fill its lane, so
//!   the waveform is anchored to the take's own recorded span (todo #1396)
//!   and the remainder of the lane draws a flat "no audio here" line;
//! * **where the waveform comes from** — the take's WAV in the project's
//!   `audio/` directory, read once when the app learns of the take (todo
//!   #1400). Nothing here fabricates a `ClipState`: a take clip never
//!   enters `Resonance::clips`, and the lookup that pretended otherwise
//!   made *every* take draw as `media missing`;
//! * the expand / collapse affordance and the arrange rows it adds;
//! * the canvas cache fingerprint reacting to captures, comp edits, active
//!   take changes and the fold toggle;
//! * eight golden snapshots, each pinning a distinct state: a folded
//!   lane's ribbon, the expanded stack under a two-take comp, an active
//!   (soloed) take, an empty comp falling back to the latest pass, a
//!   punched-in take shorter than its lane, a MIDI stack, a flagged take's
//!   `media missing` hatch, and a take whose recording could not be read —
//!   which is a quiet card, *not* the hatch.
//!
//! The comping *gestures* that sit on this surface are todo #414, covered
//! in the sibling `take_lane_input` module.

use crate::common;

use std::collections::HashMap;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::view::arrange_layout::ArrangeRowKind;
use resonance_app::view::timeline::takes::{
    effective_cover, silent_ranges, unlit_ranges, CoverSource,
};
use resonance_app::{demo, theme, Resonance};
use resonance_audio::test_support::build_comp_table;
use resonance_audio::types::AudioEvent;
use resonance_common::{CompSegment, Take, TakeContent, TakeGroup, TakeNote, TimelineRange};
use tempfile::TempDir;

/// Window size matches the app's default & minimum window per the design
/// guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The demo audio track ("Drums Bounce") the audio take groups hang off.
const AUDIO_TRACK: u64 = 5;
/// The demo instrument track ("Synth Bass") the MIDI take group hangs off.
const MIDI_TRACK: u64 = 2;
/// Engine-assigned take-group id used throughout. One id == one lane.
const GROUP: u64 = 77;

/// Base clip id for the recorded take clips. A take's `clip_ref` names a
/// *recording* — a WAV in the project's `audio/` directory — not a placed
/// arrangement clip, so nothing here ever reaches `Resonance::clips` and
/// no clip card is drawn on the lane.
const TAKE_CLIP_BASE: u64 = 5_000;

/// Project-directory name, and therefore the session title the transport
/// bar draws into every golden here. Fixed rather than the temp dir's own
/// random name, and equal to the string the bar shows for a session with
/// no path at all — so gaining a project directory changed no pixels.
const PROJECT_NAME: &str = "Untitled";

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// Demo session in the Arrange view, viewport reported so the header column
/// virtualizes exactly as it does live.
///
/// Anchored at a temporary project directory, because a take's waveform is
/// read out of `<project>/audio/clip_{clip_ref}.wav` (todo #1400). The
/// returned [`TempDir`] must be kept alive for the test's duration — drop
/// it and the recordings vanish under the app.
///
/// The project directory is a **fixed-name** child of the temp dir, not
/// the temp dir itself: the transport bar titles the session from
/// `project_path.file_stem()`, and a `tempfile` name is random, so using
/// it directly would put a different string in every golden.
/// [`PROJECT_NAME`] is the same string the bar shows for a session with no
/// path at all, which is what these goldens were blessed against — the
/// fixture gained a project directory, and deliberately not a new title.
fn build_app() -> (Resonance, TempDir) {
    let dir = tempfile::tempdir().expect("temp project dir");
    let project = dir.path().join(PROJECT_NAME);
    std::fs::create_dir_all(&project).expect("create project dir");
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_project_path(project);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(WINDOW.1)));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));
    (app, dir)
}

/// The slot every fixture records over: 2 s in, 5 s long. At the default
/// 100 px/s zoom that is x 200..700 — comfortably inside the 1160 px lane
/// area, so nothing under test is clipped out of the goldens.
fn slot(app: &Resonance) -> TimelineRange {
    let sr = app.sample_rate as u64;
    TimelineRange::new(2 * sr, 5 * sr)
}

/// Seconds from the slot's start, as an absolute sample position.
fn at(app: &Resonance, seconds: u64) -> u64 {
    slot(app).start + seconds * app.sample_rate as u64
}

/// Write the recording an audio take references, as long as `extent`.
///
/// This is the file `recording.rs` streams a pass into, at the name
/// `clip_audio_file` derives from the take's `clip_ref`, and it is the
/// **only** source of a take's waveform (todo #1400): a take clip never
/// enters `Resonance::clips`, so there is nothing to fabricate there
/// instead. It is written before the `TakeCaptured` echo for the same
/// reason the engine finalizes it before emitting one — the app reads it
/// as it folds the event.
///
/// Its length is the take's own, not the slot's, since the lane indexes
/// peaks by recorded frame rather than by card fraction.
///
/// Each take gets a visibly different waveform (a different number of
/// swells) so the stacked cards can never be confused for one another in a
/// golden.
fn write_take_recording(
    app: &Resonance,
    dir: &TempDir,
    pass_index: u32,
    extent: TimelineRange,
) {
    let peak_count =
        (extent.length as usize).div_ceil(resonance_audio::types::WAVEFORM_PEAK_FRAMES);
    common::write_take_wav(
        &dir.path().join(PROJECT_NAME),
        TAKE_CLIP_BASE + u64::from(pass_index),
        app.sample_rate,
        extent.length,
        |i| {
            let t = i as f32 / peak_count.max(1) as f32;
            // Deterministic, no trig-on-float-input drift: a triangular
            // envelope repeated `pass_index + 2` times.
            let cycles = (pass_index + 2) as f32;
            let phase = (t * cycles).fract();
            0.25 + 0.65 * (1.0 - (phase - 0.5).abs() * 2.0)
        },
    );
}

/// A take recording filling its whole slot — the ordinary cycle-record
/// pass.
fn write_take_recording_for_slot(app: &Resonance, dir: &TempDir, pass_index: u32) {
    write_take_recording(app, dir, pass_index, slot(app));
}

/// Capture `passes` audio loop passes into `GROUP` through the real engine
/// dispatch, each with a real recording on disk that fills the slot.
fn capture_audio_passes(app: &mut Resonance, dir: &TempDir, passes: u32) {
    for pass_index in 0..passes {
        write_take_recording_for_slot(app, dir, pass_index);
        capture_audio_pass(app, pass_index);
    }
}

/// Drive one `TakeCaptured` for `pass_index` through the real dispatch. The
/// engine assigns take ids per group (#409); mirroring `pass_index` keeps
/// the ids readable in assertions.
fn capture_audio_pass(app: &mut Resonance, pass_index: u32) {
    capture_audio_pass_over(app, pass_index, slot(app));
}

/// Drive one `TakeCaptured` whose pass recorded only `extent` of the slot —
/// a punch-in, or a pass cut short at stop.
///
/// The event's `extent` is the app's **only** account of that (todo #1396):
/// no `RecordingFinished` follows a take clip, so nothing the app holds can
/// be consulted instead. `write_take_recording` puts the matching audio on
/// disk for the waveform, but the lane and the promote clamp both resolve
/// the extent from the take — nothing measures the file.
fn capture_audio_pass_over(app: &mut Resonance, pass_index: u32, extent: TimelineRange) {
    let slot = slot(app);
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: u64::from(pass_index),
        track_id: AUDIO_TRACK,
        slot,
        pass_index,
        extent,
        content: TakeContent::Audio {
            clip_ref: TAKE_CLIP_BASE + u64::from(pass_index),
        },
    });
}

/// The audible extent the lane resolves for a take recorded over `extent`
/// — the model's own [`Take::audible_extent`], which since todo #1396 is
/// the single definition the lane, the promote clamp and the engine share.
fn extent_within(slot: TimelineRange, extent: TimelineRange) -> Option<TimelineRange> {
    let audible =
        Take::new(0, 0, 0, extent, TakeContent::Audio { clip_ref: 1 }).audible_extent(slot);
    (!audible.is_empty()).then_some(audible)
}

/// Capture `passes` MIDI loop passes into `GROUP` on the instrument track.
/// Each pass plays the same rhythm a fourth higher so the stacked cards are
/// distinguishable.
fn capture_midi_passes(app: &mut Resonance, passes: u32) {
    let slot = slot(app);
    for pass_index in 0..passes {
        let notes: Vec<TakeNote> = (0..8)
            .map(|i| TakeNote {
                note: 52 + (pass_index * 5) as u8 + ((i * 3) % 12) as u8,
                velocity: 0.8,
                start_tick: i * 240,
                duration_ticks: 200,
            })
            .collect();
        app.test_apply_engine_event(AudioEvent::TakeCaptured {
            group_id: GROUP,
            take_id: u64::from(pass_index),
            track_id: MIDI_TRACK,
            slot,
            pass_index,
            // A MIDI take's extent is its whole slot: its notes are its
            // content and a rest is not a hole (todo #1396).
            extent: slot,
            content: TakeContent::Midi { notes },
        });
    }
}

/// Replace the group's comp with `segments`, through the engine echo that
/// carries it live (`TakeCompChanged`, routed since todo #411) rather than
/// by reaching into the projection.
fn set_comp(app: &mut Resonance, segments: Vec<CompSegment>) {
    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: GROUP,
        segments,
    });
}

fn set_active(app: &mut Resonance, take_id: Option<u64>) {
    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: GROUP,
        take_id,
    });
}

fn toggle_lane(app: &mut Resonance, track_id: u64) {
    let _ = app.update(Message::Ui(UiMessage::ToggleTakeLane(track_id)));
}

/// The `(group, take)` pairs of the take sub-rows currently in the shared
/// arrange layout, in row order.
fn take_rows(app: &Resonance) -> Vec<(u64, u64)> {
    app.test_arrange_row_layout()
        .rows()
        .iter()
        .filter_map(|row| match row.kind {
            ArrangeRowKind::TakeRow { group, take, .. } => Some((group, take)),
            _ => None,
        })
        .collect()
}

fn group_of(app: &Resonance, id: u64) -> &TakeGroup {
    app.test_take_groups()
        .iter()
        .find(|g| g.id == id)
        .expect("take group")
}

// ---------------------------------------------------------------------
// One lane per slot
// ---------------------------------------------------------------------

/// The ruling this todo was built against: takes accumulate into **one**
/// lane for a track + loop region, however many times record was pressed.
/// Three passes, then two more over the same slot, must read as one lane of
/// five stacked takes — not two lanes.
///
/// The view reaches that by keying purely on the engine's `group_id`, which
/// is correct both today and after todo #1392 makes a second record run
/// reuse the group. Nothing here assumes a group maps to one record run.
#[test]
fn passes_from_separate_record_runs_stack_into_one_lane() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    // ...transport stopped here, and the user hit record again over the
    // same loop region. Post-#1392 the engine reuses the group id.
    for pass_index in 3..5 {
        write_take_recording_for_slot(&app, &dir, pass_index);
        capture_audio_pass(&mut app, pass_index);
    }
    toggle_lane(&mut app, AUDIO_TRACK);

    assert_eq!(
        app.test_take_groups().len(),
        1,
        "one slot, one group — not one group per record run"
    );
    assert_eq!(
        take_rows(&app),
        vec![(GROUP, 0), (GROUP, 1), (GROUP, 2), (GROUP, 3), (GROUP, 4)],
        "one lane of five stacked takes, in capture order"
    );
}

/// A track recorded over two *different* loop regions gets two stacks. They
/// stay contiguous and ordered by slot — take rows of one group never
/// interleave with another's.
#[test]
fn two_slots_on_one_track_give_two_contiguous_stacks() {
    let (mut app, _dir) = build_app();
    let sr = app.sample_rate as u64;
    let late = TimelineRange::new(20 * sr, 4 * sr);
    // Seed the *later* slot first, so ordering can only come from the slot
    // and not from arrival order.
    for (group_id, slot) in [(88u64, late), (GROUP, slot(&app))] {
        for pass_index in 0..2u32 {
            app.test_apply_engine_event(AudioEvent::TakeCaptured {
                group_id,
                take_id: u64::from(pass_index),
                track_id: AUDIO_TRACK,
                slot,
                pass_index,
                extent: slot,
                content: TakeContent::Audio { clip_ref: 1 },
            });
        }
    }
    toggle_lane(&mut app, AUDIO_TRACK);

    assert_eq!(
        take_rows(&app),
        vec![(GROUP, 0), (GROUP, 1), (88, 0), (88, 1)],
        "earlier slot's stack first, each group's takes contiguous"
    );
}

// ---------------------------------------------------------------------
// Effective cover — what the lane actually draws
// ---------------------------------------------------------------------

/// An empty comp is not silence: the engine falls back to the newest pass,
/// so the lane lights that take over the whole slot and marks it as a
/// fallback rather than as a promotion the user made.
#[test]
fn empty_comp_falls_back_to_the_latest_take() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let cover = effective_cover(group_of(&app, GROUP));

    assert_eq!(cover.len(), 1);
    assert_eq!(cover[0].range, slot(&app));
    assert_eq!(cover[0].take_id, 2, "the third pass is the latest");
    assert_eq!(cover[0].source, CoverSource::LatestFallback);
}

/// A comp spanning two takes yields exactly those spans, each marked as a
/// deliberate promotion. The cover stays ordered and gap-free, so every
/// point of the slot maps to exactly one take.
#[test]
fn comp_segments_cover_the_slot_gap_free() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let slot = slot(&app);
    let split = at(&app, 2);
    set_comp(
        &mut app,
        vec![
            CompSegment {
                range: TimelineRange::from_bounds(slot.start, split),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::from_bounds(split, slot.end()),
                take_id: 2,
            },
        ],
    );

    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(
        cover
            .iter()
            .map(|s| (s.range.start, s.range.end(), s.take_id, s.source))
            .collect::<Vec<_>>(),
        vec![
            (slot.start, split, 0, CoverSource::CompSegment),
            (split, slot.end(), 2, CoverSource::CompSegment),
        ]
    );
    assert_cover_is_a_gap_free_cover(&app);
}

/// A *partial* comp: the promoted span stays a promotion, and the
/// uncovered remainder falls back to the latest take — the user's ruling
/// that comping is progressive refinement, not assembly from silence.
///
/// **The engine agrees, and this test proves it rather than asserting it.**
/// Both layers resolve through `resonance_common::effective_cover` since
/// todo #1395, so `assert_engine_and_lane_agree` re-resolves the very same
/// group through the mixer's `build_comp_table` and fails if either side
/// drifts.
#[test]
fn partial_comp_fills_its_gaps_with_the_latest_take() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let (from, to) = (at(&app, 1), at(&app, 3));
    set_comp(
        &mut app,
        vec![CompSegment {
            range: TimelineRange::from_bounds(from, to),
            take_id: 0,
        }],
    );

    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(
        cover
            .iter()
            .map(|s| (s.take_id, s.source))
            .collect::<Vec<_>>(),
        vec![
            (2, CoverSource::LatestFallback),
            (0, CoverSource::CompSegment),
            (2, CoverSource::LatestFallback),
        ]
    );
    assert_cover_is_a_gap_free_cover(&app);
    assert_engine_and_lane_agree(&app);
}

// ---------------------------------------------------------------------
// Engine / lane agreement (todo #1395)
// ---------------------------------------------------------------------

/// The mixer's resolved comp spans for `group`, through the real
/// `build_comp_table` — `(start, end, clip_id)` per span.
fn engine_spans(group: &TakeGroup) -> Vec<(u64, u64, u64)> {
    let mut groups = HashMap::new();
    groups.insert(group.id, group.clone());
    let table = build_comp_table(&groups);
    table
        .track_comp(group.track_id)
        .map(|tc| {
            tc.spans
                .iter()
                .map(|s| (s.range.start, s.range.end(), s.clip_id))
                .collect()
        })
        .unwrap_or_default()
}

/// The lane's cover, projected onto the audio path the way the mixer
/// projects it: take → recorded clip, MIDI takes dropped (their notes sound
/// through the instrument, not through a clip), same-clip neighbours merged
/// into one read.
///
/// This projection is the *only* thing the two sides are allowed to differ
/// by. Both start from `resonance_common::effective_cover`, so any drift in
/// the tiering, in what "latest" means, or in the gap filling shows up here
/// as unequal span lists.
fn lane_spans_on_the_audio_path(group: &TakeGroup) -> Vec<(u64, u64, u64)> {
    let mut out: Vec<(u64, u64, u64)> = Vec::new();
    for span in effective_cover(group) {
        let Some(TakeContent::Audio { clip_ref }) = group.take(span.take_id).map(|t| &t.content)
        else {
            continue;
        };
        match out.last_mut() {
            Some(last) if last.2 == *clip_ref && last.1 == span.range.start => {
                last.1 = span.range.end();
            }
            _ => out.push((span.range.start, span.range.end(), *clip_ref)),
        }
    }
    out
}

/// Re-resolve the mirrored group through *both* implementations and require
/// the same answer over every region of the slot.
fn assert_engine_and_lane_agree(app: &Resonance) {
    let group = group_of(app, GROUP);
    assert_eq!(
        engine_spans(group),
        lane_spans_on_the_audio_path(group),
        "the mixer and the take lane must resolve the same cover"
    );
}

/// The agreement across every state a group can be in, on one fixture.
///
/// Before #1395 the two layers diverged the moment a comp had *any* segment
/// (the mixer played silence in the gaps the lane drew lit), and again as
/// soon as `takes` was not in capture order (each picked a different
/// "latest"). This walks a single three-take group through the empty,
/// partial, full, out-of-order and soloed cases and requires them to agree
/// on all of them.
#[test]
fn the_engine_and_the_lane_resolve_the_same_cover() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let slot = slot(&app);

    // 1. Never comped: the latest pass covers the slot.
    assert_engine_and_lane_agree(&app);
    assert_eq!(
        engine_spans(group_of(&app, GROUP)),
        vec![(slot.start, slot.end(), TAKE_CLIP_BASE + 2)]
    );

    // 2. One promotion in the middle — the case that used to diverge.
    let (from, to) = (at(&app, 1), at(&app, 3));
    set_comp(
        &mut app,
        vec![CompSegment {
            range: TimelineRange::from_bounds(from, to),
            take_id: 0,
        }],
    );
    assert_engine_and_lane_agree(&app);
    assert_eq!(
        engine_spans(group_of(&app, GROUP)),
        vec![
            (slot.start, from, TAKE_CLIP_BASE + 2),
            (from, to, TAKE_CLIP_BASE),
            (to, slot.end(), TAKE_CLIP_BASE + 2),
        ],
        "the mixer plays the latest take either side of the promotion"
    );

    // 3. A full, gap-free cover: nothing for tier 3 to do.
    set_comp(
        &mut app,
        vec![
            CompSegment {
                range: TimelineRange::from_bounds(slot.start, to),
                take_id: 1,
            },
            CompSegment {
                range: TimelineRange::from_bounds(to, slot.end()),
                take_id: 0,
            },
        ],
    );
    assert_engine_and_lane_agree(&app);

    // 4. A leading gap only, with the comp echoed back out of order — the
    //    mirror adopts what it is handed, and both sides sort defensively.
    set_comp(
        &mut app,
        vec![
            CompSegment {
                range: TimelineRange::from_bounds(to, slot.end()),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::from_bounds(from, to),
                take_id: 1,
            },
        ],
    );
    assert_engine_and_lane_agree(&app);

    // 5. Soloed: tier 1 overrides the comp on both sides.
    set_active(&mut app, Some(1));
    assert_engine_and_lane_agree(&app);
    assert_eq!(
        engine_spans(group_of(&app, GROUP)),
        vec![(slot.start, slot.end(), TAKE_CLIP_BASE + 1)]
    );
    set_active(&mut app, None);
    assert_engine_and_lane_agree(&app);
}

/// **The "latest take" ruling for a group holding both kinds.** "Latest" is
/// content-agnostic: the newest pass wins whether it is audio or MIDI. The
/// engine used to read it as "the last *audio* take in vector order" and
/// the lane as "the newest take of any kind", which pick different takes as
/// soon as a group is mixed.
///
/// Where the winner is a MIDI take the audio path contributes no spans —
/// its notes sound through the instrument instead, the same by-design
/// silence as soloing a MIDI take — and the lane says so by lighting that
/// take rather than an audio one.
#[test]
fn latest_take_is_content_agnostic_in_a_mixed_group() {
    let (mut app, dir) = build_app();
    write_take_recording_for_slot(&app, &dir, 0);
    capture_audio_pass(&mut app, 0);
    // A later MIDI pass into the same group.
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: 1,
        track_id: AUDIO_TRACK,
        slot: slot(&app),
        pass_index: 1,
        extent: slot(&app),
        content: TakeContent::Midi {
            notes: vec![TakeNote {
                note: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 240,
            }],
        },
    });

    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(cover.len(), 1);
    assert_eq!(cover[0].take_id, 1, "the MIDI pass is the latest one");
    assert_eq!(cover[0].source, CoverSource::LatestFallback);

    assert_engine_and_lane_agree(&app);
    assert!(
        engine_spans(group_of(&app, GROUP)).is_empty(),
        "a MIDI take carries no clip, so the audio path renders nothing"
    );

    // Promoting the audio take over part of the slot puts it back on the
    // audio path there, and only there.
    let (from, to) = (at(&app, 1), at(&app, 3));
    set_comp(
        &mut app,
        vec![CompSegment {
            range: TimelineRange::from_bounds(from, to),
            take_id: 0,
        }],
    );
    assert_engine_and_lane_agree(&app);
    assert_eq!(
        engine_spans(group_of(&app, GROUP)),
        vec![(from, to, TAKE_CLIP_BASE)]
    );
}

// ---------------------------------------------------------------------
// Audible extent — a take does not necessarily fill its lane
// ---------------------------------------------------------------------

/// A pass that punched in late, or was cut short at stop, carries audio
/// over only part of its slot. The lane must draw the intersection, so the
/// waveform stays anchored where the audio really is instead of being
/// stretched across the whole lane.
///
/// This is the same intersection todo #409 made `mix_track_comp` take
/// before reading a take clip; drawing a different one would claim audio
/// where the engine renders silence.
#[test]
fn a_takes_audible_extent_is_its_clip_intersected_with_the_slot() {
    let (app, _dir) = build_app();
    let slot = slot(&app);
    let sr = app.sample_rate as u64;

    // Punched in one second after the loop start, running to the slot end.
    assert_eq!(
        extent_within(slot, TimelineRange::new(slot.start + sr, 4 * sr)),
        Some(TimelineRange::from_bounds(slot.start + sr, slot.end())),
        "a punch-in take starts where its recording does, not where the lane does"
    );
    // Cut short at stop, one second before the slot ends.
    assert_eq!(
        extent_within(slot, TimelineRange::new(slot.start, 4 * sr)),
        Some(TimelineRange::from_bounds(slot.start, slot.end() - sr)),
    );
    // A recording overhanging both ends is clamped back to the lane.
    assert_eq!(
        extent_within(slot, TimelineRange::new(0, 20 * sr)),
        Some(slot),
        "the lane bounds what can be drawn"
    );
    // No overlap at all: nothing to draw.
    assert_eq!(
        extent_within(slot, TimelineRange::new(slot.end() + sr, sr)),
        None
    );
    assert_eq!(extent_within(slot, TimelineRange::new(0, sr)), None);
}

/// `silent_ranges` is the complement of the audible extent within the slot
/// — where the row draws its flat "no audio here" line.
#[test]
fn silent_ranges_are_the_remainder_of_the_lane() {
    let (app, _dir) = build_app();
    let slot = slot(&app);
    let sr = app.sample_rate as u64;

    let punched_in = extent_within(slot, TimelineRange::new(slot.start + sr, 4 * sr));
    assert_eq!(
        silent_ranges(slot, punched_in),
        vec![TimelineRange::from_bounds(slot.start, slot.start + sr)],
        "the lane leads in silent up to the punch-in"
    );

    let cut_short = extent_within(slot, TimelineRange::new(slot.start, 4 * sr));
    assert_eq!(
        silent_ranges(slot, cut_short),
        vec![TimelineRange::from_bounds(slot.end() - sr, slot.end())],
    );

    assert!(
        silent_ranges(slot, Some(slot)).is_empty(),
        "a take filling its lane has no remainder"
    );
    assert_eq!(
        silent_ranges(slot, None),
        vec![slot],
        "a take with no audio anywhere is silent across the whole lane"
    );
}

/// The audible extent is a *drawing* fact, not a comp fact: promoting a
/// take over a stretch it never recorded still puts that take in the cover
/// (the engine renders silence there, and the lane shows a lit lane with
/// the flat line under it). The two must not be conflated.
#[test]
fn a_short_take_can_still_be_comped_across_the_whole_slot() {
    let (mut app, dir) = build_app();
    let sr = app.sample_rate as u64;
    let slot = slot(&app);
    let punched = TimelineRange::from_bounds(slot.start + sr, slot.end());
    write_take_recording(&app, &dir, 0, punched);
    capture_audio_pass_over(&mut app, 0, punched);
    set_comp(
        &mut app,
        vec![CompSegment {
            range: slot,
            take_id: 0,
        }],
    );

    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(cover.len(), 1);
    assert_eq!(cover[0].range, slot, "the comp still addresses the whole slot");
    assert!(
        unlit_ranges(&cover, slot, 0).is_empty(),
        "nothing is scrimmed: the comp selects this take throughout"
    );
    // ...while only part of the lane can actually sound — read off the
    // mirrored take, which is where the extent lives since todo #1396.
    assert_eq!(
        group_of(&app, GROUP)
            .take(0)
            .expect("take")
            .audible_extent(slot),
        punched,
    );
}

/// The active take overrides the comp entirely — one span, whole slot,
/// flagged as a solo so the lane can draw it in the warm solo language
/// instead of the lavender comp one.
#[test]
fn active_take_overrides_the_comp() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let whole = slot(&app);
    set_comp(
        &mut app,
        vec![CompSegment {
            range: whole,
            take_id: 0,
        }],
    );
    set_active(&mut app, Some(1));

    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(cover.len(), 1);
    assert_eq!(cover[0].take_id, 1);
    assert_eq!(cover[0].range, whole);
    assert_eq!(cover[0].source, CoverSource::ActiveTake);

    // Clearing it hands the slot back to the comp.
    set_active(&mut app, None);
    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(cover[0].take_id, 0);
    assert_eq!(cover[0].source, CoverSource::CompSegment);
}

/// Degenerate: a lone take covers its whole slot, so its row draws fully
/// lit rather than scrimmed away.
#[test]
fn a_single_take_covers_its_whole_slot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 1);
    let group = group_of(&app, GROUP);
    let cover = effective_cover(group);
    assert_eq!(cover.len(), 1);
    assert_eq!(cover[0].take_id, 0);
    assert!(
        unlit_ranges(&cover, group.slot, 0).is_empty(),
        "nothing to scrim on a one-take lane"
    );
}

/// `unlit_ranges` is the exact complement of a take's spans within the
/// slot — the scrim the card draws over everything the comp doesn't use.
#[test]
fn unlit_ranges_complement_a_takes_spans() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let slot = slot(&app);
    let split = at(&app, 2);
    set_comp(
        &mut app,
        vec![
            CompSegment {
                range: TimelineRange::from_bounds(slot.start, split),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::from_bounds(split, slot.end()),
                take_id: 2,
            },
        ],
    );
    let group = group_of(&app, GROUP);
    let cover = effective_cover(group);

    assert_eq!(
        unlit_ranges(&cover, slot, 0),
        vec![TimelineRange::from_bounds(split, slot.end())],
        "take 0 is dark over the second half"
    );
    assert_eq!(
        unlit_ranges(&cover, slot, 2),
        vec![TimelineRange::from_bounds(slot.start, split)],
        "take 2 is dark over the first half"
    );
    assert_eq!(
        unlit_ranges(&cover, slot, 1),
        vec![slot],
        "take 1 contributes nothing and is dark throughout"
    );
}

/// Every cover the lane draws must be ordered, contiguous and exactly
/// coextensive with the slot.
fn assert_cover_is_a_gap_free_cover(app: &Resonance) {
    let group = group_of(app, GROUP);
    let cover = effective_cover(group);
    assert!(!cover.is_empty());
    let mut cursor = group.slot.start;
    for span in &cover {
        assert_eq!(span.range.start, cursor, "span starts where the last ended");
        assert!(!span.range.is_empty(), "no zero-length spans");
        cursor = span.range.end();
    }
    assert_eq!(cursor, group.slot.end(), "the cover reaches the slot's end");
}

// ---------------------------------------------------------------------
// Expand / collapse
// ---------------------------------------------------------------------

/// The lane folds and unfolds from the track header's caret, and only the
/// sub-rows come and go — the comp ribbon lives on the track's own lane and
/// is unaffected, which is what makes a folded take folder still readable.
#[test]
fn toggling_the_lane_adds_and_removes_the_take_rows() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);

    assert!(!app.test_take_lane_expanded(AUDIO_TRACK));
    assert!(take_rows(&app).is_empty(), "folded by default");

    toggle_lane(&mut app, AUDIO_TRACK);
    assert!(app.test_take_lane_expanded(AUDIO_TRACK));
    assert_eq!(take_rows(&app), vec![(GROUP, 0), (GROUP, 1), (GROUP, 2)]);
    // The rows are contiguous, directly under the track lane, at the
    // take-row pitch.
    let layout = app.test_arrange_row_layout();
    let (track_top, track_h) = layout.track_row_rect(AUDIO_TRACK).expect("track row");
    let (first_top, first_h) = layout
        .take_row_rect(AUDIO_TRACK, GROUP, 0)
        .expect("first take row");
    assert_eq!(first_top, track_top + track_h);
    assert_eq!(first_h, theme::TAKE_ROW_HEIGHT);
    let (second_top, _) = layout
        .take_row_rect(AUDIO_TRACK, GROUP, 1)
        .expect("second take row");
    assert_eq!(second_top, first_top + theme::TAKE_ROW_HEIGHT);

    toggle_lane(&mut app, AUDIO_TRACK);
    assert!(!app.test_take_lane_expanded(AUDIO_TRACK));
    assert!(take_rows(&app).is_empty(), "folding removes the rows again");
}

/// A take sub-row is not a clip drop target — dragging a clip over the
/// stack must keep it on its original lane, exactly like a group-header or
/// automation row.
#[test]
fn take_rows_are_not_clip_drop_targets() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);

    let layout = app.test_arrange_row_layout();
    let (row_top, row_h) = layout
        .take_row_rect(AUDIO_TRACK, GROUP, 1)
        .expect("take row");
    let y = app.test_arrange_header_offset() + row_top + row_h / 2.0;
    assert_eq!(app.test_track_id_at_arrange_y(y), None);
    // ...while the track's own lane still resolves.
    let (track_top, track_h) = layout.track_row_rect(AUDIO_TRACK).expect("track row");
    assert_eq!(
        app.test_track_id_at_arrange_y(
            app.test_arrange_header_offset() + track_top + track_h / 2.0
        ),
        Some(AUDIO_TRACK)
    );
}

/// An expanded member of a collapsed track group contributes neither its
/// track row nor its take sub-rows — the stack vanishes with its track.
#[test]
fn collapsed_track_group_hides_the_take_rows() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(take_rows(&app).len(), 3);

    let group_id: u64 = 9000;
    app.test_track_groups_mut()
        .create_group_from_selection(group_id, &[AUDIO_TRACK, 4]);
    assert_eq!(take_rows(&app).len(), 3, "grouping alone changes nothing");

    app.test_track_groups_mut()
        .set_collapse_state(group_id, true);
    assert!(
        take_rows(&app).is_empty(),
        "collapsing the group hides the member's take rows"
    );
    app.test_track_groups_mut()
        .set_collapse_state(group_id, false);
    assert_eq!(take_rows(&app).len(), 3, "unfolding restores them");
}

// ---------------------------------------------------------------------
// Cache fingerprint
// ---------------------------------------------------------------------

/// Everything the take lane draws lives in the cached geometry layer, so
/// every edit that changes what it depicts must change the fingerprint —
/// and a fold round-trip must return to the original, so no spurious
/// repaint key leaks in.
#[test]
fn fingerprint_tracks_captures_comp_edits_and_the_fold() {
    let (mut app, dir) = build_app();
    let empty = app.test_timeline_fingerprint();

    capture_audio_passes(&mut app, &dir, 3);
    let captured = app.test_timeline_fingerprint();
    assert_ne!(empty, captured, "a captured pass repaints the ribbon");

    let (start, split) = (slot(&app).start, at(&app, 2));
    set_comp(
        &mut app,
        vec![CompSegment {
            range: TimelineRange::from_bounds(start, split),
            take_id: 0,
        }],
    );
    let comped = app.test_timeline_fingerprint();
    assert_ne!(captured, comped, "a comp edit repaints the lane");

    set_active(&mut app, Some(1));
    let soloed = app.test_timeline_fingerprint();
    assert_ne!(comped, soloed, "an active-take change repaints the lane");
    set_active(&mut app, None);
    assert_eq!(
        app.test_timeline_fingerprint(),
        comped,
        "clearing the solo returns to the comped fingerprint"
    );

    toggle_lane(&mut app, AUDIO_TRACK);
    assert_ne!(
        comped,
        app.test_timeline_fingerprint(),
        "unfolding reshapes the row layout and must repaint"
    );
    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(
        app.test_timeline_fingerprint(),
        comped,
        "folding back restores the original fingerprint"
    );

    // The `missing_takes` fold, the last of the hashed inputs. A take
    // flagged at load time draws the hatched degradation instead of a
    // waveform, so relinking one has to repaint — and the flag is held
    // beside the groups rather than on the take, which is exactly why it
    // needs its own term in the hash.
    app.test_mark_take_missing(GROUP, 1);
    assert_ne!(
        app.test_timeline_fingerprint(),
        comped,
        "a take whose recording went missing repaints its card"
    );

    // Two further inputs are hashed and **not** asserted here, because no
    // sequence of app events reaches either on its own: `Take::extent`
    // (todo #1396, the card's width) is immutable after capture, and the
    // peak table (todo #1400, the card's waveform) is filed by the same
    // update that mirrors the take. Both change only *with* a take, whose
    // id / `captured_at` already move the hash. They are in the fold
    // because the rule is that the hash covers everything the draw pass
    // reads — a term whose absence is unobservable today is one that bites
    // the first time a relink or an extent edit lands, and the assertion
    // to add then is right here.
}

// ---------------------------------------------------------------------
// Missing media — and the states that are not it (todo #1400)
// ---------------------------------------------------------------------

/// The headline of todo #1400: **the ordinary case is the ordinary card.**
///
/// A pass is captured, its WAV is on disk (the engine finalizes it at the
/// loop seam before emitting the event), and the lane draws a take card
/// with the recording's own waveform in it. Until #1400 this drew hatched
/// `media missing`, because `take_is_missing` ORed in
/// `self.take_clip(clip_ref).is_none()` and a take clip never enters
/// `Resonance::clips` at all (todo #1396) — so the lookup missed for every
/// audio take ever recorded.
///
/// Deliberately headless. The regression was visible only in pixels, and
/// the verify gate may skip goldens (`RESONANCE_SKIP_GOLDENS=1`), so the
/// guard has to be an assertion: it asks the *draw pass's* predicate
/// through [`Resonance::test_take_draws_missing_media`], not the flag
/// behind it, because it was precisely the two disagreeing that was the
/// bug.
#[test]
fn a_freshly_captured_take_is_not_media_missing() {
    let (mut app, dir) = build_app();
    write_take_recording_for_slot(&app, &dir, 0);
    capture_audio_pass(&mut app, 0);

    assert!(
        !app.test_take_draws_missing_media(GROUP, 0),
        "a take recorded seconds ago is not missing media"
    );
    assert!(
        app.test_missing_takes().is_empty(),
        "and nothing flagged it"
    );
    assert!(
        !app.test_clips().iter().any(|c| c.id == TAKE_CLIP_BASE),
        "no take clip entered r.clips — the lane consults none (todo #1396)"
    );
}

/// Where the waveform comes from now: the recording itself.
///
/// One peak bucket per `WAVEFORM_PEAK_FRAMES` frames of the WAV, read on
/// the capture echo. Pinning the *values* — not just "non-empty" — is what
/// stops a future change quietly substituting a placeholder silhouette and
/// still passing: the amplitudes here are the ones `write_take_recording`
/// authored into the file.
#[test]
fn a_takes_waveform_is_read_from_its_recording() {
    let (mut app, dir) = build_app();
    let extent = slot(&app);
    write_take_recording_for_slot(&app, &dir, 0);
    capture_audio_pass(&mut app, 0);

    let peaks = app.test_take_peaks(GROUP, 0);
    let expected_buckets =
        (extent.length as usize).div_ceil(resonance_audio::types::WAVEFORM_PEAK_FRAMES);
    assert_eq!(
        peaks.len(),
        expected_buckets,
        "one bucket per WAVEFORM_PEAK_FRAMES frames of the recording"
    );
    // Bucket 0 of pass 0: t = 0, so the triangular envelope is at its
    // trough, 0.25.
    let (lo, hi) = peaks[0];
    assert!(
        (hi - 0.25).abs() < 1e-6 && (lo + 0.25).abs() < 1e-6,
        "bucket 0 is the envelope the WAV carries, got ({lo}, {hi})"
    );
    // A MIDI take carries its notes inline and has no recording to read.
    // Re-delivering take 0 as a MIDI pass therefore *drops* the audio
    // table rather than leaving it to draw under the note blocks — the
    // stale-peak eviction in `TakeGroupState::take_captured`.
    capture_midi_passes(&mut app, 1);
    assert!(
        app.test_take_peaks(GROUP, 0).is_empty(),
        "a take that is no longer audio keeps no waveform"
    );
}

/// **A recording the app cannot read is not, by itself, missing media.**
///
/// The capture path reads the WAV to get the waveform; when that read
/// fails it files no peaks — and deliberately does *not* raise todo #412's
/// flag. That flag means "the recording this project references is not on
/// this machine", a verdict a project *load* is entitled to reach; a pass
/// the engine has just recorded and will happily play from its own mapped
/// copy is not that. Reporting it as `media missing` is the exact bug
/// #1400 removed, so the degradation is a card with no silhouette, drawn
/// in [`an_unreadable_recording_snapshot`].
///
/// Fold the two back together and this fails while every other take-lane
/// case still passes.
#[test]
fn a_capture_whose_recording_cannot_be_read_is_not_flagged() {
    let (mut app, _dir) = build_app();
    // No `write_take_recording_for_slot`: nothing was ever written at the
    // name this take's `clip_ref` resolves to.
    capture_audio_pass(&mut app, 0);

    assert!(
        app.test_take_peaks(GROUP, 0).is_empty(),
        "there was nothing to read"
    );
    assert!(
        app.test_missing_takes().is_empty(),
        "a failed read on the capture path flags nothing"
    );
    assert!(
        !app.test_take_draws_missing_media(GROUP, 0),
        "so the card is quiet, not hatched"
    );
}

/// A **flagged** take does hatch, whatever else is true of it.
///
/// This is the load-path verdict (todo #412): the project references a
/// recording this machine cannot produce, the take stays in the comp
/// rather than being dropped — dropping would leave a `CompSegment`
/// pointing at a take id that no longer exists — and the lane must say so.
/// Since #1400 it is the *only* route into the hatch, which is why the
/// assertion is here and not implied by a golden.
#[test]
fn a_flagged_take_draws_as_missing_media() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    assert!(!app.test_take_draws_missing_media(GROUP, 1));

    app.test_mark_take_missing(GROUP, 1);
    assert_eq!(app.test_missing_takes(), vec![(GROUP, 1)]);
    assert!(
        app.test_take_draws_missing_media(GROUP, 1),
        "the flag alone reaches the degradation"
    );
    assert!(
        !app.test_take_draws_missing_media(GROUP, 0),
        "and only the flagged take"
    );
    assert!(
        !app.test_take_peaks(GROUP, 1).is_empty(),
        "its peaks are not dropped — a relink must be able to restore the card"
    );
}

// ---------------------------------------------------------------------
// Golden snapshots
// ---------------------------------------------------------------------

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// The two-take comp used by the collapsed / expanded pair, so the two
/// goldens differ *only* in the fold state.
fn seed_two_take_comp(app: &mut Resonance, dir: &TempDir) {
    capture_audio_passes(app, dir, 3);
    let slot = slot(app);
    let split = at(app, 2);
    set_comp(
        app,
        vec![
            CompSegment {
                range: TimelineRange::from_bounds(slot.start, split),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::from_bounds(split, slot.end()),
                take_id: 2,
            },
        ],
    );
}

/// **Folded lane.** Track 5 holds a 3-take group whose comp runs T1 then
/// T3. No sub-rows: the whole state is carried by the comp ribbon on the
/// track's own lane (two lavender blocks, seam at 2 s into the slot,
/// labelled T1 / T3) plus the `▸ 3 takes` caret in the header.
#[test]
fn folded_lane_ribbon_snapshot() {
    let (mut app, dir) = build_app();
    seed_two_take_comp(&mut app, &dir);
    assert!(!app.test_take_lane_expanded(AUDIO_TRACK));
    snapshot_to(&app, "tests/snapshots/take_lane_folded_ribbon.png");
}

/// **Expanded stack.** The same comp, unfolded: three 38 px take rows with
/// their own waveforms; T1 lit over the first two seconds, T3 lit over the
/// remaining three, T2 scrimmed end to end because the comp never uses it.
#[test]
fn expanded_stack_snapshot() {
    let (mut app, dir) = build_app();
    seed_two_take_comp(&mut app, &dir);
    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(take_rows(&app).len(), 3);
    snapshot_to(&app, "tests/snapshots/take_lane_expanded_stack.png");
}

/// **Active take.** T2 is soloed, so the whole slot switches to the warm
/// solo language: the ribbon is one amber `T2 solo` block, T2's card is
/// ringed amber and fully lit, and both comp segments are overridden — T1
/// and T3 go dark despite still being in the comp.
#[test]
fn active_take_snapshot() {
    let (mut app, dir) = build_app();
    seed_two_take_comp(&mut app, &dir);
    set_active(&mut app, Some(1));
    toggle_lane(&mut app, AUDIO_TRACK);
    snapshot_to(&app, "tests/snapshots/take_lane_active_take.png");
}

/// **Comp not started yet.** Three takes, no segments: the newest pass (T3)
/// carries the slot on the fallback tier, drawn in the quieter hairline
/// treatment that distinguishes "this is the default" from "you chose
/// this", with T1 and T2 scrimmed.
#[test]
fn empty_comp_snapshot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(cover[0].source, CoverSource::LatestFallback);
    snapshot_to(&app, "tests/snapshots/take_lane_empty_comp.png");
}

/// **A take shorter than its lane.** Pass 0 punched in a second after the
/// loop start; pass 1 ran the whole slot. The comp promotes pass 0 across
/// the *entire* slot, which is a real state the engine renders as silence
/// over that first second — so the golden must show T1's waveform anchored
/// where its audio starts, a flat "no audio" line before it under a fully
/// lit lane, and the untouched pass 1 scrimmed below. Rendering this as a
/// full-width stretched waveform (the pre-review behaviour) would claim
/// audio the engine never plays.
#[test]
fn punched_in_take_snapshot() {
    let (mut app, dir) = build_app();
    let sr = app.sample_rate as u64;
    let slot = slot(&app);
    let punched = TimelineRange::from_bounds(slot.start + sr, slot.end());
    write_take_recording(&app, &dir, 0, punched);
    capture_audio_pass_over(&mut app, 0, punched);
    write_take_recording_for_slot(&app, &dir, 1);
    capture_audio_pass(&mut app, 1);
    set_comp(
        &mut app,
        vec![CompSegment {
            range: slot,
            take_id: 0,
        }],
    );
    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(take_rows(&app).len(), 2);
    // The state under test: lit throughout, audible only after the punch-in.
    let cover = effective_cover(group_of(&app, GROUP));
    assert!(unlit_ranges(&cover, slot, 0).is_empty());
    assert_eq!(
        silent_ranges(slot, extent_within(slot, TimelineRange::new(slot.start + sr, 4 * sr))),
        vec![TimelineRange::from_bounds(slot.start, slot.start + sr)],
    );
    snapshot_to(&app, "tests/snapshots/take_lane_punched_in_take.png");
}

/// **MIDI takes.** The instrument track's stack draws note blocks instead
/// of waveforms, in the lavender MIDI language. The comp promotes T2 over
/// the first half and T1 over the second, so the two lit regions sit on
/// different rows and read as a comp path through the stack.
#[test]
fn midi_takes_snapshot() {
    let (mut app, _dir) = build_app();
    capture_midi_passes(&mut app, 3);
    let slot = slot(&app);
    let split = at(&app, 2);
    set_comp(
        &mut app,
        vec![
            CompSegment {
                range: TimelineRange::from_bounds(slot.start, split),
                take_id: 1,
            },
            CompSegment {
                range: TimelineRange::from_bounds(split, slot.end()),
                take_id: 0,
            },
        ],
    );
    toggle_lane(&mut app, MIDI_TRACK);
    assert_eq!(take_rows(&app).len(), 3);
    snapshot_to(&app, "tests/snapshots/take_lane_midi_takes.png");
}

/// **Missing media.** Two audio takes; the second is one a project load
/// flagged, because the recording it references is not on this machine.
/// Its card degrades to the hatched unsupported surface with a
/// `media missing` label rather than rendering as an empty (and therefore
/// silently wrong) take — and, because the comp is empty, it is also the
/// take the engine would fall back to, so the broken one is the *lit* one.
/// That is the whole point of drawing the degradation instead of hiding
/// it.
///
/// The flag is the route because since todo #1400 it is the **only** one.
/// The lane used to reach the same pixels a second way — a `clip_ref` that
/// resolved to no mirrored `ClipState` — and that arm was a standing false
/// positive rather than a route: it fired for every audio take, including
/// the healthy one in this very fixture. See
/// [`a_capture_whose_recording_cannot_be_read_is_not_flagged`] for the
/// state that used to be conflated with this one.
#[test]
fn missing_media_snapshot() {
    let (mut app, dir) = build_app();
    let slot = slot(&app);
    write_take_recording_for_slot(&app, &dir, 0);
    capture_audio_pass(&mut app, 0);
    // The second pass's recording never made it to this machine. It keeps
    // its extent — the pass filled its slot; what is gone is the WAV.
    write_take_recording_for_slot(&app, &dir, 1);
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: 1,
        track_id: AUDIO_TRACK,
        slot,
        pass_index: 1,
        extent: slot,
        content: TakeContent::Audio {
            clip_ref: TAKE_CLIP_BASE + 1,
        },
    });
    app.test_mark_take_missing(GROUP, 1);
    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(take_rows(&app).len(), 2);
    snapshot_to(&app, "tests/snapshots/take_lane_missing_media.png");
}

/// **A capture whose recording could not be read** — the state that used
/// to be indistinguishable from missing media, and is now visibly not it.
///
/// T1 recorded normally; T2's WAV was never written, so the app has no
/// waveform for it and did not flag it either
/// ([`a_capture_whose_recording_cannot_be_read_is_not_flagged`]). Its card
/// keeps its body, its edge, its `T2` tag and its comp lighting — a pass
/// *was* recorded here and the engine will play it — and simply carries no
/// silhouette. Set against `take_lane_missing_media.png`, which hatches
/// the same row: this is a lane that cannot show you something, not one
/// reporting that your audio is gone.
///
/// Replaces `take_lane_flagged_missing.png`, whose whole claim was that
/// two routes into the hatch degrade identically. There is one route now,
/// so a second image of it would pin nothing.
#[test]
fn an_unreadable_recording_snapshot() {
    let (mut app, dir) = build_app();
    let slot = slot(&app);
    write_take_recording_for_slot(&app, &dir, 0);
    capture_audio_pass(&mut app, 0);
    // No recording written for pass 1.
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: 1,
        track_id: AUDIO_TRACK,
        slot,
        pass_index: 1,
        extent: slot,
        content: TakeContent::Audio {
            clip_ref: TAKE_CLIP_BASE + 1,
        },
    });
    toggle_lane(&mut app, AUDIO_TRACK);
    assert!(app.test_take_peaks(GROUP, 1).is_empty());
    assert!(!app.test_take_draws_missing_media(GROUP, 1));
    snapshot_to(&app, "tests/snapshots/take_lane_unreadable_recording.png");
}
