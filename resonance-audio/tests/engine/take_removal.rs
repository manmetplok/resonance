//! Removing a take, and removing a whole take lane (epic #15, doc #165,
//! ba todo #1397).
//!
//! Everything here drives the **real** `RemoveTake` / `RemoveTakeGroup` /
//! `RestoreTakeGroups` handlers through `EngineHandlerHarness`, and asks
//! the question that matters at the end: what does the engine *play*?
//!
//! That is not a stylistic choice. A take group is a model object, but a
//! take's recording is an ordinary `AudioClip` in the shared clip list —
//! `roll_audio_pass` pushes it there as the pass rolls — and it stays
//! inaudible only because the published comp table marks it *governed*, so
//! the clip phase skips it. Remove the take from its group and nothing
//! governs the clip any more: a structural test would see a tidy group and
//! a comp with no dangling segment, while the "deleted" pass played raw, at
//! full gain, on top of the comp. Every case below therefore renders the
//! block through the production `render_block` and asserts on samples.
//!
//! **The WAV is never deleted, and that is a policy, not an
//! impossibility.** These handlers *could* delete it — they hold
//! `&mut HandlerState`, and `HandlerState::project_dir` is the same field
//! `finalize_loop_record_pass` reads to write `audio/clip_N.wav` in the
//! first place. They do not, for two reasons that a future change has to
//! answer before adding one:
//!
//! - **Undo needs the file.** An undo restores the take through
//!   `RestoreTakeGroups` carrying the same `clip_ref`, so deleting the
//!   recording would make undo silently lossy — the take would come back
//!   in the lane with no audio under it. That is what
//!   `an_undo_restores_a_removed_take_audibly` pins.
//! - **Reclaiming a user's audio is the user's call.** It belongs to a
//!   project-level operation they ask for, not to a command on the audio
//!   path acting on one gesture.
//!
//! So a removed take's clip is *parked* — held out of the shared clip
//! list, kept in memory — which is what stops it sounding, while the file
//! stays where it was.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::*;
use resonance_common::{Comp, CompSegment, Take, TakeContent, TakeGroup, TakeId, TimelineRange};

const TRACK: TrackId = 7;
/// Long enough that the edge declick ramps (96 frames) and a seam
/// crossfade (±128 frames) leave wide stretches of untouched, full-gain
/// audio to assert on.
const SLOT_LEN: u64 = 2000;
const SEAM: u64 = 1000;
/// Anti-click ramp at the comp's outer edges (`CLIP_DECLICK_FRAMES`).
const DECLICK: u64 = 96;
/// Half a seam crossfade (`COMP_XFADE_FRAMES / 2`).
const SEAM_HALF: u64 = 128;

const A_CLIP: ClipId = 100;
const B_CLIP: ClipId = 101;
/// Take 0's recording is DC 1.0, take 1's is DC 0.25 — so a probe says not
/// only *whether* something plays but *which pass* it is.
const A_LEVEL: f32 = 1.0;
const B_LEVEL: f32 = 0.25;

/// A clip whose stereo PCM is the constant `value`, covering
/// `[0, SLOT_LEN)` on the timeline.
fn const_clip(id: ClipId, value: f32) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: 0,
        source: ClipSource::memory(vec![value; SLOT_LEN as usize * 2]),
        name: "take".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn range(start: u64, end: u64) -> TimelineRange {
    TimelineRange::from_bounds(start, end)
}

fn seg(start: u64, end: u64, take_id: u64) -> CompSegment {
    CompSegment {
        range: range(start, end),
        take_id,
    }
}

/// An audio take whose recording fills the whole slot, so nothing here
/// turns on todo #1396's audible extent — these cases are about *which*
/// take is selected, and whether a removed one can still be heard.
fn audio_take(id: TakeId, pass_index: u32, captured_at: i64, clip_ref: ClipId) -> Take {
    Take::new(
        id,
        pass_index,
        captured_at,
        range(0, SLOT_LEN),
        TakeContent::Audio { clip_ref },
    )
}

/// Two audio takes over `[0, SLOT_LEN)`: take 0 → `A_CLIP`, take 1 →
/// `B_CLIP` (the newer pass).
fn two_take_group() -> TakeGroup {
    let mut group = TakeGroup::new(1, TRACK, range(0, SLOT_LEN));
    group.add_take(audio_take(0, 0, 1_000, A_CLIP));
    group.add_take(audio_take(1, 1, 2_000, B_CLIP));
    group
}

/// A group holding one audio take, for the last-take cases.
fn one_take_group() -> TakeGroup {
    let mut group = TakeGroup::new(1, TRACK, range(0, SLOT_LEN));
    group.add_take(audio_take(0, 0, 1_000, A_CLIP));
    group
}

/// Take 0 over the first half, take 1 over the second.
fn split_comp() -> Comp {
    Comp {
        segments: vec![seg(0, SEAM, 0), seg(SEAM, SLOT_LEN, 1)],
    }
}

/// A harness holding `group` plus the recordings its takes name — the state
/// a finished cycle-record run leaves behind.
fn harness_with(group: TakeGroup) -> EngineHandlerHarness {
    let mut h = EngineHandlerHarness::new();
    for take in &group.takes {
        if let TakeContent::Audio { clip_ref } = take.content {
            let level = if clip_ref == A_CLIP { A_LEVEL } else { B_LEVEL };
            h.push_clip(const_clip(clip_ref, level));
        }
    }
    h.seed_take_group(group);
    h
}

/// The engine's rendered output over the whole slot.
fn render(h: &EngineHandlerHarness) -> Vec<f32> {
    h.render_track(TRACK, 0, SLOT_LEN as usize)
}

/// Loudest sample anywhere in the block.
fn peak(out: &[f32]) -> f32 {
    out.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

/// Assert that nothing anywhere in the block is as loud as `A_LEVEL` — the
/// removed take's recording. Deliberately a *peak* check rather than a
/// probe: an un-parked take leaks on the ordinary clip path, which is not
/// bound to any comp segment, so it can surface anywhere.
fn assert_take_a_is_inaudible(out: &[f32]) {
    let peak = peak(out);
    assert!(
        peak < A_LEVEL - 0.01,
        "the removed take is still audible: peak {peak} (its recording is DC {A_LEVEL})"
    );
}

// ---------------------------------------------------------------------------
// Removing one take
// ---------------------------------------------------------------------------

/// The take goes, and so does every comp segment naming it. The group is
/// re-covered from the survivors, so the slot has no hole where the take
/// used to be.
#[test]
fn removing_a_take_drops_it_and_leaves_no_dangling_segment() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    });

    h.remove_take(1, 0);

    let group = h.take_group(1).expect("the group survives");
    assert_eq!(group.takes.iter().map(|t| t.id).collect::<Vec<_>>(), [1]);
    assert!(
        group.comp.segments.iter().all(|s| s.take_id != 0),
        "no segment may name the removed take: {:?}",
        group.comp.segments
    );
    assert!(
        group.is_full_cover(),
        "the survivor must inherit the hole, not leave silence mid-part"
    );
}

/// **The acceptance case.** A removed take cannot be heard.
///
/// Fails if the removal only touches the take-group store: clip `A_CLIP`
/// would stop being governed and play raw on the ordinary clip path, so the
/// "deleted" pass would come back *louder* than it was, summed on top of
/// the surviving one.
#[test]
fn a_removed_take_is_not_audible() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    });

    // Before: take 0 is what plays over the first half.
    let before = render(&h);
    assert!(
        (before[(DECLICK + 100) as usize] - A_LEVEL).abs() < 1e-6,
        "fixture check: take 0 should be playing the first half"
    );

    h.remove_take(1, 0);

    let after = render(&h);
    assert_take_a_is_inaudible(&after);
    // ...and the survivor now covers the whole slot at its own level.
    for probe in [(DECLICK + 100) as usize, (SEAM + SEAM_HALF) as usize] {
        assert!(
            (after[probe] - B_LEVEL).abs() < 1e-6,
            "frame {probe} should be the surviving take at {B_LEVEL}, got {}",
            after[probe]
        );
    }
    assert_eq!(
        h.clip_ids(),
        vec![B_CLIP],
        "the removed take's recording must leave the render's input"
    );
    assert_eq!(h.parked_clip_ids(), vec![A_CLIP], "parked, not destroyed");
}

/// The recording is parked, never deleted: the clip is still held, which is
/// what makes the undo below able to restore audible material rather than a
/// silent card. The handler has `project_dir` and could unlink the WAV; the
/// module header says why it must not.
#[test]
fn a_removed_takes_recording_is_kept_not_destroyed() {
    let mut h = harness_with(two_take_group());

    h.remove_take(1, 0);

    assert_eq!(h.parked_clip_ids(), vec![A_CLIP]);
    assert!(!h.clip_ids().contains(&A_CLIP));
}

/// A solo that named the removed take is cleared, and the change is echoed
/// — a mirror that kept it would draw a solo that solos nothing.
#[test]
fn removing_the_soloed_take_clears_the_solo() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.active_take = Some(0);
        g
    });
    let _ = h.drain_events();

    h.remove_take(1, 0);

    assert_eq!(h.take_group(1).expect("group").active_take, None);
    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(
            e,
            AudioEvent::ActiveTakeChanged {
                group_id: 1,
                take_id: None
            }
        )),
        "expected the cleared solo to be echoed, got {events:?}"
    );
    assert_take_a_is_inaudible(&render(&h));
}

/// Removing some *other* take does not end a solo.
#[test]
fn removing_another_take_leaves_the_solo_alone() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g.active_take = Some(1);
        g
    });
    let _ = h.drain_events();

    h.remove_take(1, 0);

    assert_eq!(h.take_group(1).expect("group").active_take, Some(1));
    assert!(
        !h.drain_events()
            .iter()
            .any(|e| matches!(e, AudioEvent::ActiveTakeChanged { .. })),
        "a solo that did not change must not be echoed"
    );
}

/// One event per fact: the removal, then the re-covered comp. A mirror
/// applies both with handlers it already has.
#[test]
fn removing_a_take_echoes_the_removal_and_the_recovered_comp() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    });
    let _ = h.drain_events();

    h.remove_take(1, 0);

    let events = h.drain_events();
    assert!(
        matches!(
            events.first(),
            Some(AudioEvent::TakeRemoved {
                group_id: 1,
                take_id: 0
            })
        ),
        "the removal is echoed first, got {events:?}"
    );
    let comp = events
        .iter()
        .find_map(|e| match e {
            AudioEvent::TakeCompChanged {
                group_id: 1,
                segments,
            } => Some(segments.clone()),
            _ => None,
        })
        .expect("the re-covered comp is echoed");
    assert_eq!(comp, vec![seg(0, SLOT_LEN, 1)]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AudioEvent::TakeGroupRemoved { .. })),
        "the group survives, so it must not be reported as removed"
    );
}

/// A removal that leaves the comp alone echoes no comp change — an echo
/// that says nothing invites a mirror to re-apply its own input.
#[test]
fn removing_an_uncomped_take_echoes_no_comp_change() {
    // No comp at all: the group plays its latest take by fallback, and
    // removing the *older* take moves nothing.
    let mut h = harness_with(two_take_group());
    let _ = h.drain_events();

    h.remove_take(1, 0);

    let events = h.drain_events();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AudioEvent::TakeCompChanged { .. })),
        "expected no comp echo, got {events:?}"
    );
}

// ---------------------------------------------------------------------------
// The last take, and the whole lane
// ---------------------------------------------------------------------------

/// **The trap this todo exists for.** With no takes left the comp is empty,
/// and an empty comp is exactly the state in which the cover falls back to
/// the most recent pass — the take just deleted. Removing a group's last
/// take therefore removes the group, and the pass goes quiet.
#[test]
fn removing_a_groups_last_take_removes_the_group_and_silences_it() {
    let mut group = one_take_group();
    group.comp = Comp {
        segments: vec![seg(0, SLOT_LEN, 0)],
    };
    let mut h = harness_with(group);
    assert!(
        peak(&render(&h)) > A_LEVEL - 0.01,
        "fixture check: it plays"
    );

    h.remove_take(1, 0);

    assert!(
        h.take_group_ids().is_empty(),
        "the empty lane must not linger"
    );
    let out = render(&h);
    assert_take_a_is_inaudible(&out);
    assert_eq!(
        peak(&out),
        0.0,
        "nothing is left to play, got {}",
        peak(&out)
    );
}

/// The last-take case reports the lane, not the take: the app asked to
/// remove one take and the group went with it, which it cannot infer.
#[test]
fn removing_a_groups_last_take_echoes_the_group_not_the_take() {
    let mut h = harness_with(one_take_group());
    let _ = h.drain_events();

    h.remove_take(1, 0);

    let events = h.drain_events();
    assert!(
        matches!(
            events.as_slice(),
            [AudioEvent::TakeGroupRemoved { group_id: 1 }]
        ),
        "expected exactly one TakeGroupRemoved, got {events:?}"
    );
}

/// Removing a lane parks every recording it held. Without that, dropping
/// the group un-governs all of its take clips at once and every pass plays
/// simultaneously, on top of each other, at full gain.
#[test]
fn removing_a_group_parks_every_recording_it_held() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    });

    h.remove_take_group(1);

    assert!(h.take_group_ids().is_empty());
    assert!(h.clip_ids().is_empty(), "both recordings leave the render");
    assert_eq!(h.parked_clip_ids(), vec![A_CLIP, B_CLIP]);
    let out = render(&h);
    assert_eq!(
        peak(&out),
        0.0,
        "a removed lane must play nothing, got peak {}",
        peak(&out)
    );
}

/// A MIDI take names no clip and writes no file, so its removal parks
/// nothing — and must not disturb the audio takes beside it.
#[test]
fn removing_a_midi_take_parks_no_clip() {
    let mut group = two_take_group();
    group.add_take(Take::new(
        2,
        2,
        3_000,
        range(0, SLOT_LEN),
        TakeContent::Midi { notes: Vec::new() },
    ));
    let mut h = harness_with(group);

    h.remove_take(1, 2);

    assert!(h.parked_clip_ids().is_empty());
    assert_eq!(h.clip_ids(), vec![A_CLIP, B_CLIP]);
    assert_eq!(
        h.take_group(1).expect("group").takes.len(),
        2,
        "the audio takes are untouched"
    );
}

// ---------------------------------------------------------------------------
// Undo and redo, through the one restore path
// ---------------------------------------------------------------------------

/// **Undo restores the take, audibly.** The engine keeps no history of its
/// own (doc #105): an undo replays the app's mirror through
/// `RestoreTakeGroups`, and because the removal parked the recording rather
/// than deleting it, the restored take renders sample-for-sample as it did
/// before — not as a silent card in the lane.
#[test]
fn an_undo_restores_a_removed_take_audibly() {
    let authored = {
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    };
    let mut h = harness_with(authored.clone());
    let before = render(&h);

    h.remove_take(1, 0);
    assert_take_a_is_inaudible(&render(&h));

    // What an undo sends: the mirror as it was, rebuilt from scratch.
    h.restore_take_groups(vec![authored]);

    assert_eq!(
        render(&h),
        before,
        "an undone removal must render exactly as it did before"
    );
    assert!(
        h.parked_clip_ids().is_empty(),
        "the recording is back in play"
    );
    assert_eq!(h.clip_ids(), vec![A_CLIP, B_CLIP]);
}

/// A whole lane comes back the same way, which is the case
/// `RestoreTakeGroups`' "wholesale, never additive" rule was written for:
/// the engine's store no longer holds the group at all, so only a
/// replacement can recreate it.
#[test]
fn an_undo_restores_a_removed_lane_audibly() {
    let authored = {
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    };
    let mut h = harness_with(authored.clone());
    let before = render(&h);

    h.remove_take_group(1);
    assert_eq!(peak(&render(&h)), 0.0);

    h.restore_take_groups(vec![authored]);

    assert_eq!(render(&h), before);
    assert!(h.parked_clip_ids().is_empty());
}

/// **Redo is the other direction of the same path**, and it is the one that
/// bites. A redo restores the store to a state *without* the take — so the
/// restore has to park the recording the incoming groups stopped claiming,
/// or the redone removal would leave the clip registered, un-governed, and
/// playing raw.
#[test]
fn a_redone_removal_leaves_the_take_silent() {
    let authored = {
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    };
    let mut h = harness_with(authored.clone());

    // remove → undo → redo, all through the real handlers.
    h.remove_take(1, 0);
    h.restore_take_groups(vec![authored.clone()]);

    let mut redone = authored;
    redone.remove_take(0).expect("take 0");
    h.restore_take_groups(vec![redone]);

    assert_take_a_is_inaudible(&render(&h));
    assert_eq!(h.parked_clip_ids(), vec![A_CLIP]);
    assert!(!h.clip_ids().contains(&A_CLIP));
}

/// A restore leaves the recordings of takes it *keeps* exactly where they
/// were — it must not park and re-push a clip that never left the lane.
#[test]
fn a_restore_that_changes_nothing_disturbs_no_recording() {
    let authored = {
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    };
    let mut h = harness_with(authored.clone());
    let before = render(&h);

    h.restore_take_groups(vec![authored]);

    assert_eq!(render(&h), before);
    assert!(h.parked_clip_ids().is_empty());
    assert_eq!(h.clip_ids(), vec![A_CLIP, B_CLIP]);
}

/// Loading a project drops the park with everything else. The clip list has
/// just been drained, so nothing could un-park into the new project anyway
/// — keeping it would only hold the previous project's mappings open.
#[test]
fn clear_all_drops_the_parked_recordings() {
    let mut h = harness_with(two_take_group());
    h.remove_take(1, 0);
    assert_eq!(h.parked_clip_ids(), vec![A_CLIP]);

    h.clear_all();

    assert!(h.parked_clip_ids().is_empty());
    assert!(h.take_group_ids().is_empty());
    assert!(h.clip_ids().is_empty());
}

// ---------------------------------------------------------------------------
// Missing lookups
// ---------------------------------------------------------------------------

/// An unknown group, or a take the group does not hold, is ignored — the
/// handlers' standing missing-lookup convention, and what makes a stale
/// command idempotent rather than destructive.
#[test]
fn removing_an_unknown_group_or_take_changes_nothing() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    });
    let before = render(&h);
    let _ = h.drain_events();

    h.remove_take(99, 0);
    h.remove_take(1, 99);
    h.remove_take_group(99);

    assert_eq!(h.take_group_ids(), vec![1]);
    assert_eq!(h.take_group(1).expect("group").takes.len(), 2);
    assert_eq!(render(&h), before);
    assert!(h.parked_clip_ids().is_empty());
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "nothing happened, so nothing is echoed: {events:?}"
    );
}

/// Removing the same take twice is safe: the second call finds nothing and
/// does nothing, rather than taking the lane with it as a "last take".
#[test]
fn removing_a_take_twice_is_a_no_op_the_second_time() {
    let mut h = harness_with({
        let mut g = two_take_group();
        g.comp = split_comp();
        g
    });

    h.remove_take(1, 0);
    let after_first = render(&h);
    let _ = h.drain_events();

    h.remove_take(1, 0);

    assert_eq!(h.take_group_ids(), vec![1], "the lane must survive");
    assert_eq!(render(&h), after_first);
    assert!(h.drain_events().is_empty());
}
