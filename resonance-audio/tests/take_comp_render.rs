//! Take-comp playback: building the comp render table from take groups,
//! rendering the comped cover with an equal-power crossfade at each seam,
//! and doing it identically on the live and the offline path (epic #15,
//! doc #165, todo #409).
//!
//! Two levels of coverage:
//!
//! - The comp math on its own, through `build_comp_table` /
//!   `mix_track_comp`: which take is audible where, and what the seam
//!   between two of them sounds like.
//! - The whole render block, through `render_take_comp_for_test`: that the
//!   raw recorded passes really are skipped by the clip phase, and that the
//!   live and the bounce strategy produce the *same samples* — the "a comp
//!   bounces the way it plays" half of the acceptance criteria, which no
//!   amount of testing `mix_track_comp` in isolation can show.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

use resonance_audio::__test_support::{
    build_comp_table, mix_track_comp, render_take_comp_for_test, CompRenderTable,
};
use resonance_audio::types::*;
use resonance_common::{
    Comp, CompSegment, Take, TakeContent, TakeGroup, TakeGroupId, TimelineRange,
};

const TRACK: TrackId = 7;
/// Slot long enough that the two edge declick ramps (96 frames each) and
/// the seam crossfade window (±128 frames around the midpoint) leave wide
/// stretches of untouched, full-gain audio between them to assert on.
const SLOT_LEN: u64 = 2000;
/// Midpoint of the slot: where the two comp segments hand over.
const SEAM: u64 = 1000;
/// Half-width of a seam crossfade, `COMP_XFADE_FRAMES / 2`, when both
/// neighbouring spans are longer than the window.
const SEAM_HALF: u64 = 128;
/// Anti-click ramp length at the comp's outer edges (`CLIP_DECLICK_FRAMES`).
const DECLICK: u64 = 96;

/// A clip whose stereo PCM is the constant `value` on both channels and
/// covers `[0, frames)` on the timeline, so the output sample at each frame
/// equals `value × crossfade_coefficient`.
fn const_clip(id: ClipId, frames: usize, value: f32) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: 0,
        source: ClipSource::Memory(vec![value; frames * 2]),
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

/// A two-take group bound to `[0, SLOT_LEN)`: take 0 → clip `A_CLIP`,
/// take 1 → clip `B_CLIP`. The comp / active take are set by the caller.
const A_CLIP: ClipId = 100;
const B_CLIP: ClipId = 101;

fn two_take_group() -> TakeGroup {
    let slot = TimelineRange::new(0, SLOT_LEN);
    let mut g = TakeGroup::new(1, TRACK, slot);
    g.add_take(Take::new(0, 0, 0, TakeContent::Audio { clip_ref: A_CLIP }));
    g.add_take(Take::new(1, 1, 0, TakeContent::Audio { clip_ref: B_CLIP }));
    g
}

fn groups(g: TakeGroup) -> HashMap<TakeGroupId, TakeGroup> {
    let mut m = HashMap::new();
    m.insert(g.id, g);
    m
}

/// Take 0 over `[0, SEAM)`, take 1 over `[SEAM, SLOT_LEN)`.
fn split_comp() -> Comp {
    Comp {
        segments: vec![
            CompSegment {
                range: TimelineRange::from_bounds(0, SEAM),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::from_bounds(SEAM, SLOT_LEN),
                take_id: 1,
            },
        ],
    }
}

/// Render the comp for `TRACK` over `[0, SLOT_LEN)` and return the
/// left-channel output (both channels carry identical DC).
fn render(table: &CompRenderTable, clips: &[AudioClip]) -> Vec<f32> {
    let frames = SLOT_LEN as usize;
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let tc = table.track_comp(TRACK).expect("track has a comp");
    let has_audio = mix_track_comp(tc, clips, 0, frames, &mut l, &mut r);
    assert!(has_audio, "comp should contribute audio");
    assert_eq!(l, r, "left/right must match for DC input");
    l
}

#[test]
fn governs_every_recorded_take_clip() {
    // Both takes' clips must be skipped on the normal clip path so the raw
    // overlapping passes never play on top of the comp.
    let table = build_comp_table(&groups(two_take_group()));
    assert!(table.is_governed(A_CLIP));
    assert!(table.is_governed(B_CLIP));
    assert!(!table.is_governed(999));
    assert!(!table.is_empty());
}

#[test]
fn empty_table_governs_nothing() {
    let table = CompRenderTable::default();
    assert!(table.is_empty());
    assert!(!table.is_governed(A_CLIP));
    assert!(table.track_comp(TRACK).is_none());
}

#[test]
fn comp_plays_each_take_in_its_segment() {
    // Take A clip is DC 1.0, take B clip DC 0.0, so away from the seam and
    // the edge ramps the output reads which take is audible: ~1.0 under A,
    // ~0.0 under B.
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    let clips = [
        const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
        const_clip(B_CLIP, SLOT_LEN as usize, 0.0),
    ];
    let out = render(&table, &clips);

    // Well past the head declick and well before the seam: pure take A.
    let a = (DECLICK + 100) as usize;
    assert!((out[a] - 1.0).abs() < 1e-6, "take A region, got {}", out[a]);
    // Well after the seam: pure take B (silent clip).
    let b = (SEAM + SEAM_HALF + 100) as usize;
    assert!(out[b].abs() < 1e-6, "take B region, got {}", out[b]);
}

#[test]
fn seam_crossfade_is_equal_power() {
    // Build the same comp twice, swapping which take carries the DC so we
    // can read each side's crossfade gain in isolation, then assert the two
    // gains satisfy gainA² + gainB² == 1 across the whole seam window — the
    // defining property of an equal-power crossfade (no power dip → no
    // audible click).
    let mut ga = two_take_group();
    ga.comp = split_comp();
    let table_a = build_comp_table(&groups(ga));
    // A audible, B silent → output == outgoing (A) crossfade gain.
    let gain_a = render(
        &table_a,
        &[
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.0),
        ],
    );

    let mut gb = two_take_group();
    gb.comp = split_comp();
    let table_b = build_comp_table(&groups(gb));
    // A silent, B audible → output == incoming (B) crossfade gain.
    let gain_b = render(
        &table_b,
        &[
            const_clip(A_CLIP, SLOT_LEN as usize, 0.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 1.0),
        ],
    );

    // Constant power holds everywhere between the two edge declick ramps
    // (which are deliberately NOT constant-power: they ramp from and to
    // silence).
    for i in DECLICK as usize..(SLOT_LEN - DECLICK) as usize {
        let power = gain_a[i] * gain_a[i] + gain_b[i] * gain_b[i];
        assert!(
            (power - 1.0).abs() < 1e-5,
            "frame {i}: constant-power crossfade violated (power={power}, a={}, b={})",
            gain_a[i],
            gain_b[i]
        );
    }

    // At the seam centre both gains are sin(π/4) ≈ 0.707.
    let mid = SEAM as usize;
    let expected_mid = (FRAC_PI_2 * 0.5).sin();
    assert!(
        (gain_a[mid] - expected_mid).abs() < 0.01,
        "outgoing mid gain {}",
        gain_a[mid]
    );
    assert!(
        (gain_b[mid] - expected_mid).abs() < 0.01,
        "incoming mid gain {}",
        gain_b[mid]
    );

    // Outside the seam window (and the edge ramps) the gains are saturated.
    let before = (SEAM - SEAM_HALF - 50) as usize;
    let after = (SEAM + SEAM_HALF + 50) as usize;
    assert!((gain_a[before] - 1.0).abs() < 1e-6 && gain_b[before].abs() < 1e-6);
    assert!(gain_a[after].abs() < 1e-6 && (gain_b[after] - 1.0).abs() < 1e-6);
}

#[test]
fn seam_has_no_amplitude_jump() {
    // Both takes DC 1.0: the summed crossfade output must be continuous
    // (no sample-to-sample discontinuity) across the seam.
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    let out = render(
        &table,
        &[
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 1.0),
        ],
    );

    let max_step = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_step < 0.05,
        "comp output should be click-free; largest sample step was {max_step}"
    );
    // The crossfade of two identical DC signals peaks at √2 in the middle.
    assert!(
        (out[SEAM as usize] - std::f32::consts::SQRT_2).abs() < 0.02,
        "seam peak {}",
        out[SEAM as usize]
    );
}

#[test]
fn comp_outer_edges_ramp_from_silence() {
    // The comp's first and last frames are splices into silence, not seams,
    // so they carry the same anti-click ramp every clip edge gets. Without
    // it a take that starts mid-waveform would step the signal — the exact
    // click CLIP_DECLICK_FRAMES exists to remove.
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    let out = render(
        &table,
        &[
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 1.0),
        ],
    );

    assert!(out[0].abs() < 0.05, "comp must open from silence, got {}", out[0]);
    let last = (SLOT_LEN - 1) as usize;
    assert!(
        out[last].abs() < 0.05,
        "comp must close to silence, got {}",
        out[last]
    );
    // Fully open once the ramp has run out.
    let open = (DECLICK + 10) as usize;
    assert!((out[open] - 1.0).abs() < 1e-6, "past the ramp, got {}", out[open]);
}

#[test]
fn active_take_plays_whole_slot() {
    // Active take overrides the comp: take 1 (B clip, DC 1.0) plays the
    // entire slot; take A (silent here) is not heard.
    let mut g = two_take_group();
    // A non-trivial comp that must be ignored while an active take is set.
    g.comp = Comp {
        segments: vec![CompSegment {
            range: TimelineRange::from_bounds(0, SLOT_LEN),
            take_id: 0,
        }],
    };
    g.active_take = Some(1);
    let table = build_comp_table(&groups(g));
    let out = render(
        &table,
        &[
            const_clip(A_CLIP, SLOT_LEN as usize, 0.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 1.0),
        ],
    );

    // Whole slot is take B at unity between the edge ramps, no seam.
    let inner = DECLICK as usize..(SLOT_LEN - DECLICK) as usize;
    for (offset, &v) in out[inner.clone()].iter().enumerate() {
        let i = offset + inner.start;
        assert!(
            (v - 1.0).abs() < 1e-6,
            "frame {i}: active take should play unity, got {v}"
        );
    }
}

#[test]
fn active_take_of_unknown_id_is_not_resolvable() {
    // A selection naming a take the group does not hold resolves to no
    // audio spans at all rather than silently falling back to the comp —
    // the engine handler refuses such a command, and the table agrees.
    let mut g = two_take_group();
    g.comp = split_comp();
    g.active_take = Some(42);
    let table = build_comp_table(&groups(g));
    assert!(table.track_comp(TRACK).is_none());
    // The take clips stay governed: they are still under comp control.
    assert!(table.is_governed(A_CLIP));
}

#[test]
fn unset_selection_defaults_to_latest_take() {
    // No comp, no active take → the most recently captured take (take 1)
    // covers the slot so a freshly recorded group is audible immediately.
    let g = two_take_group(); // empty comp, active_take None
    let table = build_comp_table(&groups(g));
    let out = render(
        &table,
        &[
            const_clip(A_CLIP, SLOT_LEN as usize, 0.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 1.0),
        ],
    );
    let inner = DECLICK as usize..(SLOT_LEN - DECLICK) as usize;
    for (offset, &v) in out[inner.clone()].iter().enumerate() {
        let i = offset + inner.start;
        assert!(
            (v - 1.0).abs() < 1e-6,
            "default cover should be the latest take, frame {i} = {v}"
        );
    }
}

#[test]
fn midi_only_group_contributes_no_audio_spans() {
    // A group whose only take is MIDI yields no audio governance and no
    // track comp (the audio path stays silent for it).
    let slot = TimelineRange::new(0, SLOT_LEN);
    let mut g = TakeGroup::new(1, TRACK, slot);
    g.add_take(Take::new(0, 0, 0, TakeContent::Midi { notes: Vec::new() }));
    let table = build_comp_table(&groups(g));
    assert!(table.is_empty());
    assert!(table.track_comp(TRACK).is_none());
}

// ---------------------------------------------------------------------------
// Through the real render block: governance + playback/bounce equivalence
// ---------------------------------------------------------------------------

/// Render `[0, SLOT_LEN)` through the production `render_block` on both
/// strategies, assert the two agree sample-for-sample, and return the
/// left-channel output. The live and bounce paths differ in plugin
/// locking, gain ramping and metering — none of which the comp path
/// touches — so equality here is the "a comp bounces the way it plays"
/// guarantee, measured rather than asserted structurally.
fn render_block_both_ways(
    table: &CompRenderTable,
    clips: impl Fn() -> Vec<AudioClip>,
) -> Vec<f32> {
    let frames = SLOT_LEN as usize;
    // `AudioClip` is deliberately not `Clone` (a mapped source is shared
    // through an `Arc`, an in-RAM one would be copied), so the caller hands
    // over a builder and each strategy gets its own set.
    let live = render_take_comp_for_test(
        vec![Track::new(TRACK, "comped".into())],
        clips(),
        table,
        0,
        frames,
        48_000,
        true,
    );
    let bounce = render_take_comp_for_test(
        vec![Track::new(TRACK, "comped".into())],
        clips(),
        table,
        0,
        frames,
        48_000,
        false,
    );
    assert_eq!(
        live, bounce,
        "comp playback and comp bounce must render identical samples"
    );
    live.chunks(2).map(|f| f[0]).collect()
}

#[test]
fn comp_renders_identically_live_and_bounced() {
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    // Take A is DC 1.0, take B DC 0.25, so the comped result is audibly
    // "A then B" and any strategy divergence would show up as a difference.
    let out = render_block_both_ways(&table, || {
        vec![
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.25),
        ]
    });

    let a = (DECLICK + 100) as usize;
    let b = (SEAM + SEAM_HALF + 100) as usize;
    assert!((out[a] - 1.0).abs() < 1e-6, "take A segment, got {}", out[a]);
    assert!((out[b] - 0.25).abs() < 1e-6, "take B segment, got {}", out[b]);
}

#[test]
fn raw_take_passes_never_double_play_through_the_block() {
    // Both take clips sit on the track in `clips` — that is what a recorded
    // cycle looks like: N fully overlapping passes over one slot. Only the
    // comp may be heard. Without governance the clip phase would sum both
    // clips (1.0 + 0.25 = 1.25, plus their automatic same-track crossfade)
    // on top of the comp.
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    let out = render_block_both_ways(&table, || {
        vec![
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.25),
        ]
    });

    // Exactly the comp's own levels, nothing summed on top.
    let a = (DECLICK + 100) as usize;
    let b = (SEAM + SEAM_HALF + 100) as usize;
    assert!(
        (out[a] - 1.0).abs() < 1e-6,
        "raw passes leaked into take A's segment: {}",
        out[a]
    );
    assert!(
        (out[b] - 0.25).abs() < 1e-6,
        "raw passes leaked into take B's segment: {}",
        out[b]
    );
    // Nothing anywhere exceeds what one crossfade of the two takes can
    // produce. An equal-power crossfade of *correlated* material sums above
    // unity (√2 at most — see `seam_has_no_amplitude_jump`), so the ceiling
    // is √2 × the loudest take, not the loudest take. Double-playing the raw
    // passes would put 1.0 + 0.25 under the comp everywhere and blow past it.
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(
        peak <= std::f32::consts::SQRT_2 + 1e-4,
        "comp peak {peak} exceeds the crossfade ceiling — a raw pass is leaking"
    );
}

#[test]
fn ungoverned_clip_on_the_same_track_still_plays() {
    // Governance is per clip, not per track: a clip that is not part of any
    // take group must keep playing normally alongside the comp. Pinning
    // this stops the governance check from being widened to "skip the whole
    // track", which would silently drop ordinary edits made around a comp.
    const PLAIN: ClipId = 500;
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    assert!(!table.is_governed(PLAIN));

    let out = render_block_both_ways(&table, || {
        vec![
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.0),
            const_clip(PLAIN, SLOT_LEN as usize, 0.5),
        ]
    });

    // Comp (take A, 1.0) + the ungoverned clip (0.5) = 1.5.
    let a = (DECLICK + 200) as usize;
    assert!(
        (out[a] - 1.5).abs() < 1e-6,
        "ungoverned clip should still play under the comp, got {}",
        out[a]
    );
}

#[test]
fn empty_table_renders_the_raw_clips_unchanged() {
    // The no-take-groups case: an empty table must leave the block byte-for-
    // byte as it was before take lanes existed, which is what keeps every
    // project without a comp free of cost and free of risk.
    let out = render_block_both_ways(&CompRenderTable::default(), || {
        vec![const_clip(A_CLIP, SLOT_LEN as usize, 0.5)]
    });
    // The lone clip plays through the ordinary path at its own level.
    let mid = (SLOT_LEN / 2) as usize;
    assert!((out[mid] - 0.5).abs() < 1e-6, "raw clip level {}", out[mid]);
}

// ---------------------------------------------------------------------------
// Spans whose clip does not cover them (ba doc #292 review)
// ---------------------------------------------------------------------------

/// A clip of constant DC placed at `start` on the timeline, covering
/// `frames` frames from there — so `[start, start + frames)` is audible and
/// everything outside it is not.
fn placed_clip(id: ClipId, start: u64, frames: usize, value: f32) -> AudioClip {
    let mut c = const_clip(id, frames, value);
    c.start_sample = start;
    c
}

#[test]
fn punch_in_after_loop_start() {
    // Cycle-recording after punching in later than the loop start gives
    // pass 0 a clip that starts inside the slot, while the default cover
    // resolves to `group.slot` — so the span is wider than its clip.
    // Reading the clip at the slot start underflowed the clip-relative
    // frame index: a debug panic on the audio thread, a silent wrap in
    // release.
    let g = two_take_group();
    let table = build_comp_table(&groups(g));
    let frames = SLOT_LEN as usize;
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let tc = table.track_comp(TRACK).expect("track has a comp");

    // Take B (the default cover) punched in 500 frames into the slot.
    let clips = [
        placed_clip(A_CLIP, 0, SLOT_LEN as usize, 0.0),
        placed_clip(B_CLIP, 500, (SLOT_LEN - 500) as usize, 1.0),
    ];
    mix_track_comp(tc, &clips, 0, frames, &mut l, &mut r);

    // Nothing before the punch-in point.
    for (i, &v) in l[..500].iter().enumerate() {
        assert!(v.abs() < 1e-6, "frame {i} precedes the take, got {v}");
    }
    // Full level once past the onset ramp.
    let open = (500 + DECLICK + 10) as usize;
    assert!((l[open] - 1.0).abs() < 1e-6, "past the onset ramp, got {}", l[open]);
}

#[test]
fn take_shorter_than_its_span_stops_cleanly() {
    // A take that runs out before its span ends (a pass cut short at stop)
    // must simply stop. The take's extent is its *visible*, post-trim
    // duration, so a trimmed tail is silent too — bounding on the raw PCM
    // buffer instead would play audio the user trimmed away.
    let g = two_take_group();
    let table = build_comp_table(&groups(g));
    let frames = SLOT_LEN as usize;
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let tc = table.track_comp(TRACK).expect("track has a comp");

    // 2000 frames of PCM, 800 trimmed off the end → audible over [0, 1200).
    let mut short = placed_clip(B_CLIP, 0, SLOT_LEN as usize, 1.0);
    short.trim_end_frames = 800;
    assert_eq!(short.duration_frames(), 1200, "precondition: post-trim length");
    let clips = [placed_clip(A_CLIP, 0, SLOT_LEN as usize, 0.0), short];
    mix_track_comp(tc, &clips, 0, frames, &mut l, &mut r);

    for (i, &v) in l[1200..].iter().enumerate() {
        assert!(
            v.abs() < 1e-6,
            "frame {} is past the take's trimmed end, got {v}",
            i + 1200
        );
    }
    let open = (DECLICK + 10) as usize;
    assert!((l[open] - 1.0).abs() < 1e-6, "take body, got {}", l[open]);
    // The trimmed end is a real edge, so it ramps rather than cutting.
    assert!(
        l[1199].abs() < 0.05,
        "trimmed end must ramp to silence, got {}",
        l[1199]
    );
}

#[test]
fn onset_and_offset_ramps_land_on_the_real_audio_edges() {
    // The anti-click ramps must sit on the take's actual first and last
    // audible frames. Anchoring them on the span instead ran the head ramp
    // through a region with no samples at all and left the real onset a raw
    // splice — the click doc #165's DoD forbids.
    let g = two_take_group();
    let table = build_comp_table(&groups(g));
    let frames = SLOT_LEN as usize;
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let tc = table.track_comp(TRACK).expect("track has a comp");

    let onset = 500u64;
    let end = 1500u64;
    let clips = [
        placed_clip(A_CLIP, 0, SLOT_LEN as usize, 0.0),
        placed_clip(B_CLIP, onset, (end - onset) as usize, 1.0),
    ];
    mix_track_comp(tc, &clips, 0, frames, &mut l, &mut r);

    // Ramps start from (near) silence at both real edges...
    assert!(
        l[onset as usize].abs() < 0.05,
        "onset must ramp from silence, got {}",
        l[onset as usize]
    );
    assert!(
        l[(end - 1) as usize].abs() < 0.05,
        "offset must ramp to silence, got {}",
        l[(end - 1) as usize]
    );
    // ...and no step anywhere: the whole rendered span is click-free.
    let max_step = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(
        max_step < 0.05,
        "comp must be click-free across the take's edges; largest step was {max_step}"
    );
}

#[test]
fn partial_take_at_a_seam_ramps_out_instead_of_cutting() {
    // The interaction the two rules have to agree on: a comped segment
    // whose take stops before the seam. The outgoing crossfade never
    // reaches it, so the ramp has to come from the declick at the take's
    // real end — otherwise the take is cut mid-waveform right where the
    // comp hands over.
    let mut g = two_take_group();
    g.comp = split_comp();
    let table = build_comp_table(&groups(g));
    let frames = SLOT_LEN as usize;
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let tc = table.track_comp(TRACK).expect("track has a comp");

    // Take A covers only [0, 800) of its [0, 1000) segment; take B covers
    // its whole segment.
    let clips = [
        placed_clip(A_CLIP, 0, 800, 1.0),
        placed_clip(B_CLIP, 0, SLOT_LEN as usize, 1.0),
    ];
    mix_track_comp(tc, &clips, 0, frames, &mut l, &mut r);

    // A is at full level in its body and has ramped away by its real end.
    assert!((l[400] - 1.0).abs() < 1e-6, "take A body, got {}", l[400]);
    assert!(l[799].abs() < 0.05, "take A must ramp out at 800, got {}", l[799]);
    // B still arrives through its own seam crossfade and reaches full level.
    let b_open = (SEAM + SEAM_HALF + 100) as usize;
    assert!((l[b_open] - 1.0).abs() < 1e-6, "take B body, got {}", l[b_open]);
    // No step anywhere, including across the silent gap the short take
    // leaves between its end and B's fade-in.
    let max_step = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.05, "seam must stay click-free; largest step {max_step}");
}

// ---------------------------------------------------------------------------
// A comp restored from a saved project (todo #1394)
// ---------------------------------------------------------------------------
//
// `store_take` was the only writer of the engine's take-group store, so
// after a reload the store was empty, `build_comp_table` produced an empty
// table and the comp the user could see rendered nothing — on playback and
// on bounce alike. These go the whole way: serde round-trip the group the
// way `project.json` does, rehydrate the store through the real restore,
// and render the result through the production block on both strategies.

/// The comped output of `group`, rendered live and bounced (asserted
/// identical inside `render_block_both_ways`).
fn comped_output(group: TakeGroup) -> Vec<f32> {
    let table = build_comp_table(&groups(group));
    render_block_both_ways(&table, || {
        vec![
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.25),
        ]
    })
}

/// A saved comp plays *and bounces* exactly as it did before the save.
///
/// Sample-for-sample against the pre-save render, so a restore that lost
/// the segment order, the slot binding or a take's `clip_ref` fails here
/// rather than passing on a "something came out" check.
#[test]
fn a_restored_comp_bounces_identically_to_before_the_save() {
    let mut authored = two_take_group();
    authored.comp = split_comp();
    let before = comped_output(authored.clone());

    // The real durable hop: `ProjectFile.take_groups` stores the shared
    // model verbatim, so this is the shape a reload hands back.
    let json = serde_json::to_string(&authored).expect("serialize take group");
    let saved: TakeGroup = serde_json::from_str(&json).expect("deserialize take group");

    // Rehydrate the engine's store exactly as `RestoreTakeGroups` does.
    let mut store: HashMap<TakeGroupId, TakeGroup> = HashMap::new();
    let mut next_group_id = 1u64;
    let mut next_clip_id = 1u64;
    resonance_audio::__test_support::restore_take_groups_in_place(
        &mut store,
        &mut next_group_id,
        &mut next_clip_id,
        vec![saved],
    );

    let table = build_comp_table(&store);
    assert!(
        !table.is_empty(),
        "an empty table after a reload is exactly the silent-comp bug"
    );
    assert!(
        table.is_governed(A_CLIP) && table.is_governed(B_CLIP),
        "restored take clips must still be governed, or the raw passes double-play"
    );

    let after = render_block_both_ways(&table, || {
        vec![
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.25),
        ]
    });
    assert_eq!(
        after, before,
        "a reloaded comp must render sample-for-sample as it did before the save"
    );

    // Not vacuous: the segments really do carry two different takes.
    let a = (DECLICK + 100) as usize;
    let b = (SEAM + SEAM_HALF + 100) as usize;
    assert!((after[a] - 1.0).abs() < 1e-6, "take A segment, got {}", after[a]);
    assert!((after[b] - 0.25).abs() < 1e-6, "take B segment, got {}", after[b]);
}

/// A soloed take survives the same round trip: the group's `active_take`
/// still overrides the comp after a reload, so the slot plays that one take
/// whole rather than the composite.
#[test]
fn a_restored_active_take_still_overrides_the_comp() {
    let mut authored = two_take_group();
    authored.comp = split_comp();
    authored.active_take = Some(1);

    let json = serde_json::to_string(&authored).expect("serialize take group");
    let saved: TakeGroup = serde_json::from_str(&json).expect("deserialize take group");

    let mut store: HashMap<TakeGroupId, TakeGroup> = HashMap::new();
    let mut next_group_id = 1u64;
    let mut next_clip_id = 1u64;
    resonance_audio::__test_support::restore_take_groups_in_place(
        &mut store,
        &mut next_group_id,
        &mut next_clip_id,
        vec![saved],
    );

    let out = render_block_both_ways(&build_comp_table(&store), || {
        vec![
            const_clip(A_CLIP, SLOT_LEN as usize, 1.0),
            const_clip(B_CLIP, SLOT_LEN as usize, 0.25),
        ]
    });

    // Take B (0.25) across the whole slot, including where the comp would
    // otherwise have played take A.
    for probe in [(DECLICK + 100) as usize, (SEAM + SEAM_HALF + 100) as usize] {
        assert!(
            (out[probe] - 0.25).abs() < 1e-6,
            "soloed take must play the whole slot; frame {probe} is {}",
            out[probe]
        );
    }
}
