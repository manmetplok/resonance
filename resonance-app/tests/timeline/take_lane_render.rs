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
//! * the **effective cover** the lane draws from — the engine's resolution
//!   order `active take → comp segment → latest take`, which is gap-free by
//!   construction, so every point of the slot maps to exactly one take;
//! * the expand / collapse affordance and the arrange rows it adds;
//! * the canvas cache fingerprint reacting to captures, comp edits, active
//!   take changes and the fold toggle;
//! * six golden snapshots, each pinning a *distinct* state: a folded lane's
//!   ribbon, the expanded stack under a two-take comp, an active (soloed)
//!   take, an empty comp falling back to the latest pass, a MIDI stack, and
//!   the missing-`clip_ref` degradation.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage, ViewportMessage};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::view::arrange_layout::ArrangeRowKind;
use resonance_app::view::timeline::takes::{effective_cover, unlit_ranges, CoverSource};
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::{AudioEvent, FadeCurve};
use resonance_common::{
    CompSegment, TakeContent, TakeGroup, TakeNote, TimelineRange,
};

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
/// *recording*, not a placed arrangement clip, so these live on a track id
/// the arrange view doesn't render — they must feed the take waveform
/// without also drawing clip cards on the lane.
const TAKE_CLIP_BASE: u64 = 5_000;
const RECORDING_TRACK: u64 = 0;

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
fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(WINDOW.1)));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));
    app
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

/// Push the recorded clip an audio take references. Each take gets a
/// visibly different waveform (a different number of swells) so the stacked
/// cards can never be confused for one another in a golden.
fn push_take_clip(app: &mut Resonance, pass_index: u32) {
    let sr = app.sample_rate as u64;
    let peaks: Vec<(f32, f32)> = (0..256)
        .map(|i| {
            let t = i as f32 / 256.0;
            // Deterministic, no trig-on-float-input drift: a triangular
            // envelope repeated `pass_index + 2` times.
            let cycles = (pass_index + 2) as f32;
            let phase = (t * cycles).fract();
            let amp = 0.25 + 0.65 * (1.0 - (phase - 0.5).abs() * 2.0);
            (-amp, amp)
        })
        .collect();
    app.test_push_clip(ClipState {
        id: TAKE_CLIP_BASE + u64::from(pass_index),
        track_id: RECORDING_TRACK,
        start_sample: 0,
        duration_samples: 5 * sr,
        name: format!("take {pass_index}"),
        total_frames: 5 * sr,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: peaks,
        vocal_tuning: None,
        asset_ref: None,
    });
}

/// Capture `passes` audio loop passes into `GROUP` through the real engine
/// dispatch, backing each with a resolvable clip.
fn capture_audio_passes(app: &mut Resonance, passes: u32) {
    let slot = slot(app);
    for pass_index in 0..passes {
        push_take_clip(app, pass_index);
        app.test_apply_engine_event(AudioEvent::TakeCaptured {
            group_id: GROUP,
            track_id: AUDIO_TRACK,
            slot,
            pass_index,
            content: TakeContent::Audio {
                clip_ref: TAKE_CLIP_BASE + u64::from(pass_index),
            },
        });
    }
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
            track_id: MIDI_TRACK,
            slot,
            pass_index,
            content: TakeContent::Midi { notes },
        });
    }
}

/// Replace the group's comp with `segments`. The engine echo that will
/// carry this (`TakeCompChanged`) is todo #409's, so — exactly as the #410
/// mirror tests do — the projection is driven directly.
fn set_comp(app: &mut Resonance, segments: Vec<CompSegment>) {
    app.test_take_groups_mut().comp_changed(GROUP, segments);
}

fn set_active(app: &mut Resonance, take_id: Option<u64>) {
    app.test_take_groups_mut()
        .active_take_changed(GROUP, take_id);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
    // ...transport stopped here, and the user hit record again over the
    // same loop region. Post-#1392 the engine reuses the group id.
    let slot = slot(&app);
    for pass_index in 3..5 {
        push_take_clip(&mut app, pass_index);
        app.test_apply_engine_event(AudioEvent::TakeCaptured {
            group_id: GROUP,
            track_id: AUDIO_TRACK,
            slot,
            pass_index,
            content: TakeContent::Audio {
                clip_ref: TAKE_CLIP_BASE + u64::from(pass_index),
            },
        });
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
    let mut app = build_app();
    let sr = app.sample_rate as u64;
    let late = TimelineRange::new(20 * sr, 4 * sr);
    // Seed the *later* slot first, so ordering can only come from the slot
    // and not from arrival order.
    for (group_id, slot) in [(88u64, late), (GROUP, slot(&app))] {
        for pass_index in 0..2u32 {
            app.test_apply_engine_event(AudioEvent::TakeCaptured {
                group_id,
                track_id: AUDIO_TRACK,
                slot,
                pass_index,
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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

/// A *partial* comp is the interesting case: the promoted span stays a
/// promotion, and the uncovered remainder falls back to the latest take —
/// the lane must not draw a hole where the engine will happily play
/// something.
#[test]
fn partial_comp_fills_its_gaps_with_the_latest_take() {
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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
}

/// The active take overrides the comp entirely — one span, whole slot,
/// flagged as a solo so the lane can draw it in the warm solo language
/// instead of the lavender comp one.
#[test]
fn active_take_overrides_the_comp() {
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 1);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);

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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
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
    let mut app = build_app();
    let empty = app.test_timeline_fingerprint();

    capture_audio_passes(&mut app, 3);
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
fn seed_two_take_comp(app: &mut Resonance) {
    capture_audio_passes(app, 3);
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
    let mut app = build_app();
    seed_two_take_comp(&mut app);
    assert!(!app.test_take_lane_expanded(AUDIO_TRACK));
    snapshot_to(&app, "tests/snapshots/take_lane_folded_ribbon.png");
}

/// **Expanded stack.** The same comp, unfolded: three 38 px take rows with
/// their own waveforms; T1 lit over the first two seconds, T3 lit over the
/// remaining three, T2 scrimmed end to end because the comp never uses it.
#[test]
fn expanded_stack_snapshot() {
    let mut app = build_app();
    seed_two_take_comp(&mut app);
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
    let mut app = build_app();
    seed_two_take_comp(&mut app);
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
    let mut app = build_app();
    capture_audio_passes(&mut app, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    let cover = effective_cover(group_of(&app, GROUP));
    assert_eq!(cover[0].source, CoverSource::LatestFallback);
    snapshot_to(&app, "tests/snapshots/take_lane_empty_comp.png");
}

/// **MIDI takes.** The instrument track's stack draws note blocks instead
/// of waveforms, in the lavender MIDI language. The comp promotes T2 over
/// the first half and T1 over the second, so the two lit regions sit on
/// different rows and read as a comp path through the stack.
#[test]
fn midi_takes_snapshot() {
    let mut app = build_app();
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

/// **Missing media.** Two audio takes; the second one's `clip_ref` names a
/// clip the app no longer holds. Its card degrades to the hatched
/// unsupported surface with a `media missing` label rather than rendering
/// as an empty (and therefore silently wrong) take — and, because the comp
/// is empty, it is also the take the engine would fall back to, so the
/// broken one is the *lit* one. That is the whole point of drawing the
/// degradation instead of hiding it.
#[test]
fn missing_media_snapshot() {
    let mut app = build_app();
    let slot = slot(&app);
    push_take_clip(&mut app, 0);
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        track_id: AUDIO_TRACK,
        slot,
        pass_index: 0,
        content: TakeContent::Audio {
            clip_ref: TAKE_CLIP_BASE,
        },
    });
    // No `push_take_clip` for this one: the recording is gone.
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        track_id: AUDIO_TRACK,
        slot,
        pass_index: 1,
        content: TakeContent::Audio { clip_ref: 999_999 },
    });
    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(take_rows(&app).len(), 2);
    snapshot_to(&app, "tests/snapshots/take_lane_missing_media.png");
}
