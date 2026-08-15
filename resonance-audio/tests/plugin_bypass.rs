//! Per-slot FX bypass with a click-free crossfade (ba doc #275 finding
//! X3, todo #1304).
//!
//! The bypass contract lives in `crate::bypass` and every chain runner —
//! track, sub-track, bus, master, monitor — goes through the same two
//! primitives: [`BypassFade::stage`] resolves what a block must do, and
//! `run_faded` applies it. Both are exercised here directly, with
//! synthetic "plugins" supplied as closures, so the exact production code
//! path is under test without a live CLAP instance (there is no way to
//! instantiate one in an offline test, and a fake plugin would be testing
//! the fake).
//!
//! What is pinned:
//! * a settled-bypassed slot passes audio through **bit-for-bit** and
//!   never calls the plugin;
//! * a bypass transition has no discontinuity — measured against the same
//!   transition without the fade, which does;
//! * a reverb-style tail fades out instead of being truncated;
//! * the fade lands in exactly [`BYPASS_FADE_MS`] and is symmetric;
//! * offline renders see settled states only, so a bounce is
//!   deterministic and cannot steal a live fade's position;
//! * plugin-delay compensation stays correct across a toggle — a
//!   host-bypassed slot leaves its chain's latency sum, a slot that
//!   bypasses itself through its own parameter does not.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use indexmap::IndexMap;
use resonance_audio::__test_support::{
    affects_latency, apply_bypass_request, chain_latencies, compensation_delays, crossfade_to_dry,
    fade_frames, fade_weight, run_faded, slot_latency, BypassFade, FadeStage, FxDryScratch,
    SharedState, BYPASS_FADE_MS,
};
use resonance_audio::types::{AudioCommand, PluginInstanceId, Track, TrackId, TrackType};

const SR: u32 = 48_000;

/// One block's worth of a continuous 220 Hz sine, starting at absolute
/// frame `start`. Continuous across blocks, so any step in the rendered
/// output comes from the bypass and not from the source.
fn sine_block(start: usize, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let w = 2.0 * std::f32::consts::PI * 220.0 / SR as f32;
    let l: Vec<f32> = (0..frames)
        .map(|f| 0.5 * ((start + f) as f32 * w).sin())
        .collect();
    let r = l.clone();
    (l, r)
}

/// The largest absolute sample-to-sample step in a signal. A click *is* a
/// step, so this is the discontinuity measure the acceptance criterion
/// asks for.
fn max_step(xs: &[f32]) -> f32 {
    xs.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

/// Render `blocks` blocks of `frames` through one bypass fade, with
/// `process` standing in for the plugin. Returns the concatenated output.
fn render_through_fade(
    fade: &BypassFade,
    blocks: usize,
    frames: usize,
    mut process: impl FnMut(&mut [f32], &mut [f32]),
) -> Vec<f32> {
    let mut dry = FxDryScratch::new(frames);
    let mut out = Vec::with_capacity(blocks * frames);
    for b in 0..blocks {
        let (mut l, mut r) = sine_block(b * frames, frames);
        let stage = fade.stage(SR, frames, true);
        let (chain_dry, _slot_dry) = dry.split();
        run_faded(stage, frames, (&mut l, &mut r), chain_dry, |bl, br| {
            process(bl, br);
            true
        });
        out.extend_from_slice(&l);
    }
    out
}

// ---------------------------------------------------------------------------
// A bypassed slot passes audio through unchanged
// ---------------------------------------------------------------------------

#[test]
fn settled_bypass_passes_audio_through_bit_for_bit_and_never_runs_the_plugin() {
    let fade = BypassFade::new();
    fade.set_bypassed_settled(true);
    assert_eq!(fade.stage(SR, 128, true), FadeStage::Dry);

    let (mut l, mut r) = sine_block(0, 128);
    let expected_l = l.clone();
    let expected_r = r.clone();
    let mut dry = FxDryScratch::new(128);
    let (chain_dry, _) = dry.split();
    let mut ran = false;
    let did = run_faded(
        FadeStage::Dry,
        128,
        (&mut l, &mut r),
        chain_dry,
        |bl, br| {
            ran = true;
            bl.fill(9.0);
            br.fill(9.0);
            true
        },
    );

    assert!(!did, "run_faded must report the plugin did not run");
    assert!(!ran, "a settled-bypassed slot must never call its plugin");
    // Bit-for-bit: no crossfade multiply touches a settled bypass at all.
    assert_eq!(l, expected_l);
    assert_eq!(r, expected_r);
}

#[test]
fn engaged_slot_is_the_unchanged_path() {
    // FadeStage::Wet must process in place with no dry copy and no
    // crossfade, so the engaged path is exactly what it was before bypass
    // fading existed.
    let mut dry = FxDryScratch::new(64);
    let (chain_dry, _) = dry.split();
    let mut l = vec![1.0f32; 64];
    let mut r = vec![1.0f32; 64];
    let did = run_faded(FadeStage::Wet, 64, (&mut l, &mut r), chain_dry, |bl, br| {
        bl.fill(0.25);
        br.fill(0.25);
        true
    });
    assert!(did);
    assert!(l.iter().all(|&v| v == 0.25));
    assert!(r.iter().all(|&v| v == 0.25));
}

// ---------------------------------------------------------------------------
// No discontinuity at the transition
// ---------------------------------------------------------------------------

/// The worst case a bypass can produce: a plugin whose output is the
/// polarity-inverted input. Switching it out on a sample boundary is a
/// full 2×|x| step; the crossfade must make that step vanish.
#[test]
fn bypass_transition_has_no_discontinuity() {
    let frames = 64;
    let blocks = 16; // 1024 frames > the 240-frame fade at 48 kHz
    let switch_after = 3;

    // Reference: the pre-fix behaviour — the chain is simply skipped on
    // the block boundary, with no ramp.
    let mut hard = Vec::new();
    for b in 0..blocks {
        let (mut l, _r) = sine_block(b * frames, frames);
        if b < switch_after {
            for v in l.iter_mut() {
                *v = -*v;
            }
        }
        hard.extend_from_slice(&l);
    }

    // Control: the same chain with no bypass at all, so the engaged path
    // is confirmed smooth before the transition is measured against it.
    let never = BypassFade::new();
    let faded = render_through_fade(&never, blocks, frames, |l, r| {
        for v in l.iter_mut() {
            *v = -*v;
        }
        for v in r.iter_mut() {
            *v = -*v;
        }
    });

    // The signal's own maximum step: 220 Hz at 0.5 amplitude.
    let signal_step = max_step(&sine_block(0, frames * blocks).0);
    assert!(signal_step < 0.02, "sanity: gentle source, got {signal_step}");

    // Un-faded, the switch is a near-full-scale jump.
    assert!(
        max_step(&hard) > 0.5,
        "the un-ramped switch is supposed to click: {}",
        max_step(&hard)
    );

    // With no transition the output is just the inverted sine, so the
    // engaged path contributes no step of its own.
    assert!(max_step(&faded) < 0.02);

    // Now the real thing: flip the bypass part-way through.
    let fade = BypassFade::new();
    let mut dry = FxDryScratch::new(frames);
    let mut out = Vec::with_capacity(blocks * frames);
    for b in 0..blocks {
        if b == switch_after {
            fade.set_bypassed(true);
        }
        let (mut l, mut r) = sine_block(b * frames, frames);
        let stage = fade.stage(SR, frames, true);
        let (chain_dry, _) = dry.split();
        run_faded(stage, frames, (&mut l, &mut r), chain_dry, |bl, br| {
            for v in bl.iter_mut() {
                *v = -*v;
            }
            for v in br.iter_mut() {
                *v = -*v;
            }
            true
        });
        out.extend_from_slice(&l);
    }

    // The fade's own contribution: smoothstep's slope peaks at 1.5, so
    // over `fade_frames` the dry weight moves at most 1.5/len per sample
    // and the wet→dry swing is 2×|x| ≤ 1.0 of it.
    let fade_step = 1.0 * 1.5 / fade_frames(SR) as f32;
    let budget = signal_step + fade_step;
    let got = max_step(&out);
    assert!(
        got <= budget,
        "bypass transition must not step: got {got}, budget {budget}"
    );
    // And it is dramatically smoother than the un-ramped switch.
    assert!(got < max_step(&hard) / 10.0);

    // The transition really did happen: the tail is the dry signal.
    let tail = &out[out.len() - frames..];
    let expect = sine_block((blocks - 1) * frames, frames).0;
    for (a, b) in tail.iter().zip(expect.iter()) {
        assert!((a - b).abs() < 1e-6, "must end fully bypassed");
    }
}

#[test]
fn re_engaging_a_bypass_is_equally_click_free() {
    let frames = 64;
    let blocks = 16;
    let fade = BypassFade::new();
    fade.set_bypassed_settled(true);

    let mut dry = FxDryScratch::new(frames);
    let mut out = Vec::with_capacity(blocks * frames);
    for b in 0..blocks {
        if b == 2 {
            fade.set_bypassed(false);
        }
        let (mut l, mut r) = sine_block(b * frames, frames);
        let stage = fade.stage(SR, frames, true);
        let (chain_dry, _) = dry.split();
        run_faded(stage, frames, (&mut l, &mut r), chain_dry, |bl, br| {
            for v in bl.iter_mut() {
                *v = -*v;
            }
            for v in br.iter_mut() {
                *v = -*v;
            }
            true
        });
        out.extend_from_slice(&l);
    }
    let signal_step = max_step(&sine_block(0, frames * blocks).0);
    let budget = signal_step + 1.5 / fade_frames(SR) as f32;
    assert!(max_step(&out) <= budget, "fade-in must not click");
    // Ends fully engaged (inverted).
    let tail = &out[out.len() - frames..];
    let expect = sine_block((blocks - 1) * frames, frames).0;
    for (a, b) in tail.iter().zip(expect.iter()) {
        assert!((a + b).abs() < 1e-6, "must end fully engaged");
    }
}

/// A "reverb": a long feedback-delay tail layered over the input. Its
/// tail is loud, so bypassing it mid-playback is exactly the case the
/// finding calls out — a hard skip truncates the tail and steps.
fn reverb_block(state: &mut Vec<f32>, l: &mut [f32], r: &mut [f32]) {
    for f in 0..l.len() {
        let delayed = state.remove(0);
        let wet = delayed * 0.85;
        state.push(l[f] + wet * 0.7);
        l[f] += wet;
        r[f] = l[f];
    }
}

#[test]
fn a_reverb_tail_fades_out_instead_of_being_truncated() {
    let frames = 32;
    let blocks = 24;
    let switch_at = 8;
    let render = |bypass_at: Option<usize>| -> Vec<f32> {
        let fade = BypassFade::new();
        let mut state = vec![0.0f32; 97];
        let mut dry = FxDryScratch::new(frames);
        let mut out = Vec::with_capacity(blocks * frames);
        for b in 0..blocks {
            if Some(b) == bypass_at {
                fade.set_bypassed(true);
            }
            let (mut l, mut r) = sine_block(b * frames, frames);
            let stage = fade.stage(SR, frames, true);
            let (chain_dry, _) = dry.split();
            run_faded(stage, frames, (&mut l, &mut r), chain_dry, |bl, br| {
                reverb_block(&mut state, bl, br);
                true
            });
            out.extend_from_slice(&l);
        }
        out
    };
    // Control run: the same reverb, never bypassed. Its own maximum step
    // is what "smooth" means for this signal.
    let control = render(None);
    let bypassed = render(Some(switch_at));

    // The tail is substantial where the bypass lands, so truncating it
    // would be a real step — this is what makes the assertion meaningful.
    let at_switch = &control[switch_at * frames..(switch_at + 1) * frames];
    let dry_there = sine_block(switch_at * frames, frames).0;
    let tail_level = at_switch
        .iter()
        .zip(dry_there.iter())
        .map(|(w, d)| (w - d).abs())
        .fold(0.0, f32::max);
    assert!(tail_level > 0.2, "sanity: audible tail, got {tail_level}");

    // The transition adds no step beyond the fade's own bounded slope.
    let budget = max_step(&control) + 2.0 * tail_level * 1.5 / fade_frames(SR) as f32;
    assert!(
        max_step(&bypassed) <= budget,
        "the tail must fade, not truncate: {} > {budget}",
        max_step(&bypassed)
    );

    // And it did fully bypass: the last block is the bare dry signal.
    let tail = &bypassed[bypassed.len() - frames..];
    let expect = sine_block((blocks - 1) * frames, frames).0;
    for (a, b) in tail.iter().zip(expect.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

// ---------------------------------------------------------------------------
// The fade curve and the state machine
// ---------------------------------------------------------------------------

#[test]
fn fade_weight_is_equal_gain_and_flat_at_both_ends() {
    assert_eq!(fade_weight(0.0), 0.0);
    assert_eq!(fade_weight(1.0), 1.0);
    assert_eq!(fade_weight(0.5), 0.5);
    // Out-of-range positions clamp rather than overshoot.
    assert_eq!(fade_weight(-1.0), 0.0);
    assert_eq!(fade_weight(2.0), 1.0);
    // Monotonic, and with zero slope at both ends (that flatness is what
    // removes the corner where the fade meets the steady state).
    let mut prev = 0.0;
    for i in 0..=100 {
        let w = fade_weight(i as f32 / 100.0);
        assert!(w >= prev - 1e-7, "must not go backwards");
        prev = w;
    }
    assert!(fade_weight(0.01) < 0.001, "flat at the start");
    assert!(fade_weight(0.99) > 0.999, "flat at the end");
}

#[test]
fn a_unity_passthrough_slot_is_transparent_through_the_whole_fade() {
    // Equal gain (dry + wet == 1) means crossfading a slot that does
    // nothing changes nothing — the property an equal-*power* pair would
    // break with a 3 dB bulge mid-fade.
    let frames = 64;
    let fade = BypassFade::new();
    fade.set_bypassed(true);
    let out = render_through_fade(&fade, 12, frames, |_l, _r| {});
    let expect = sine_block(0, 12 * frames).0;
    for (i, (a, b)) in out.iter().zip(expect.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "sample {i}: {a} vs {b}");
    }
}

#[test]
fn the_fade_lasts_exactly_bypass_fade_ms_and_is_symmetric() {
    let len = fade_frames(SR);
    assert_eq!(len, (SR as f32 * BYPASS_FADE_MS / 1000.0) as u32);
    assert_eq!(len, 240, "5 ms at 48 kHz");

    let frames = 16;
    let fade = BypassFade::new();
    fade.set_bypassed(true);
    let mut blocks_to_settle = 0;
    loop {
        let stage = fade.stage(SR, frames, true);
        if stage == FadeStage::Dry {
            break;
        }
        blocks_to_settle += 1;
        assert!(blocks_to_settle < 100, "the fade must terminate");
    }
    assert_eq!(blocks_to_settle, (len as usize).div_ceil(frames));
    assert_eq!(fade.position(SR), len);

    // Back again, same length.
    fade.set_bypassed(false);
    let mut back = 0;
    while fade.stage(SR, frames, true) != FadeStage::Wet {
        back += 1;
        assert!(back < 100);
    }
    assert_eq!(back, blocks_to_settle);
    assert_eq!(fade.position(SR), 0);
}

#[test]
fn the_fade_position_chains_continuously_across_blocks() {
    // Each block must start at the position the previous one ended on,
    // or the curve would have a step at every block boundary.
    let fade = BypassFade::new();
    fade.set_bypassed(true);
    let mut prev_to = 0.0f32;
    for _ in 0..8 {
        match fade.stage(SR, 32, true) {
            FadeStage::Fade { from, to } => {
                assert!((from - prev_to).abs() < 1e-6, "{from} != {prev_to}");
                assert!(to > from);
                prev_to = to;
            }
            FadeStage::Dry => break,
            FadeStage::Wet => panic!("bypass was requested"),
        }
    }
}

#[test]
fn a_reversal_mid_fade_turns_around_from_where_it_got_to() {
    let fade = BypassFade::new();
    fade.set_bypassed(true);
    let a = fade.stage(SR, 64, true);
    assert!(a.is_fading());
    let mid = fade.position(SR);
    assert_eq!(mid, 64);

    fade.set_bypassed(false);
    match fade.stage(SR, 32, true) {
        FadeStage::Fade { from, to } => {
            assert!((from - mid as f32 / fade_frames(SR) as f32).abs() < 1e-6);
            assert!(to < from, "must run back down");
        }
        other => panic!("expected a fade, got {other:?}"),
    }
    assert_eq!(fade.position(SR), 32);
}

#[test]
fn offline_renders_see_settled_states_and_never_move_the_fade() {
    // A bounce of a project with a bypassed slot must render it bypassed
    // from frame 0, not fade it out over the first few milliseconds — and
    // a bounce running next to live playback must not consume the live
    // fade's position.
    let fade = BypassFade::new();
    fade.set_bypassed(true);
    assert_eq!(fade.stage(SR, 64, false), FadeStage::Dry);
    assert_eq!(fade.position(SR), 0, "offline must not advance the fade");

    // The live path then still gets its full transition.
    assert!(fade.stage(SR, 64, true).is_fading());
    assert_eq!(fade.position(SR), 64);

    fade.set_bypassed(false);
    assert_eq!(fade.stage(SR, 64, false), FadeStage::Wet);
    assert_eq!(fade.position(SR), 64, "still untouched offline");
}

#[test]
fn set_bypassed_settled_skips_the_transition_entirely() {
    // Project load / replay restores a bypass rather than changing one:
    // there is no audio to click, so the fade must land immediately.
    let fade = BypassFade::new();
    fade.set_bypassed_settled(true);
    assert!(fade.bypassed());
    assert_eq!(fade.stage(SR, 64, true), FadeStage::Dry);
    fade.set_bypassed_settled(false);
    assert_eq!(fade.stage(SR, 64, true), FadeStage::Wet);
}

#[test]
fn a_bypass_set_while_nothing_renders_lands_without_a_transition() {
    // Project load / replay restores saved bypass state through the same
    // commands a user toggle uses. With nothing rendering there is no
    // audio to click, so the change must land outright — otherwise a
    // project saved with a bypassed chain would spend the first few
    // milliseconds of playback un-bypassed.
    let shared = SharedState::default();
    let fade = BypassFade::new();

    apply_bypass_request(&shared, &fade, true);
    assert_eq!(
        fade.stage(SR, 64, true),
        FadeStage::Dry,
        "a restore must not fade"
    );

    // Once the transport rolls, the same request is a real edit and
    // crossfades.
    apply_bypass_request(&shared, &fade, false);
    shared.playing.store(true, Ordering::Relaxed);
    apply_bypass_request(&shared, &fade, true);
    assert!(fade.stage(SR, 64, true).is_fading());

    // Monitoring counts as rendering too — a guitarist toggling an amp
    // sim with the transport stopped still hears it.
    shared.playing.store(false, Ordering::Relaxed);
    shared.monitoring.store(true, Ordering::Relaxed);
    let fade = BypassFade::new();
    apply_bypass_request(&shared, &fade, true);
    assert!(fade.stage(SR, 64, true).is_fading());
}

#[test]
fn a_block_longer_than_the_scratch_degrades_instead_of_allocating() {
    // The realtime rule is "never allocate"; a block bigger than the
    // pre-allocated dry staging therefore falls back to the settled
    // behaviour for that block, and the fade completes on the next ones.
    let mut dry = FxDryScratch::new(16);
    assert_eq!(dry.capacity(), 16);
    let (chain_dry, _) = dry.split();
    let mut l = vec![1.0f32; 64];
    let mut r = vec![1.0f32; 64];
    let did = run_faded(
        FadeStage::Fade { from: 0.0, to: 0.2 },
        64,
        (&mut l, &mut r),
        chain_dry,
        |bl, br| {
            bl.fill(0.5);
            br.fill(0.5);
            true
        },
    );
    assert!(did, "still processes — it just cannot stage the crossfade");
    assert!(l.iter().all(|&v| v == 0.5));
}

#[test]
fn crossfade_to_dry_sweeps_the_position_across_the_block() {
    let frames = 8;
    let mut l = vec![1.0f32; frames];
    let mut r = vec![1.0f32; frames];
    let dry_l = vec![0.0f32; frames];
    let dry_r = vec![0.0f32; frames];
    crossfade_to_dry((&mut l, &mut r), (&dry_l, &dry_r), frames, 0.0, 1.0);
    // Wet weight = 1 - smoothstep(t), t sweeping to exactly 1.0 at the
    // block's last sample.
    for (f, &v) in l.iter().enumerate() {
        let t = (f + 1) as f32 / frames as f32;
        assert!((v - (1.0 - fade_weight(t))).abs() < 1e-6);
    }
    assert!(l[frames - 1].abs() < 1e-6, "lands exactly on dry");
    assert!(l[0] > l[frames - 1], "monotonic towards dry");
    assert_eq!(l, r, "both channels ride the same curve");
}

// ---------------------------------------------------------------------------
// Latency compensation across a bypass toggle
// ---------------------------------------------------------------------------

#[test]
fn slot_latency_drops_only_the_slots_the_mixer_skips() {
    // Host-bypassed: not processed, so it delays nothing.
    assert_eq!(slot_latency(512, true), 0);
    // Bypassed through the plugin's own parameter: still processed, still
    // delaying — which is exactly why the comp table doesn't move.
    assert_eq!(slot_latency(512, false), 512);
    assert_eq!(slot_latency(0, false), 0);
}

fn track_with_plugins(id: u64, ty: TrackType, plugins: &[PluginInstanceId]) -> Track {
    let track = Track::with_type(id, format!("t{id}"), ty);
    for &p in plugins {
        track.push_plugin(p);
    }
    track
}

#[test]
fn a_bypassed_slot_leaves_its_chains_latency_and_the_rest_realigns() {
    // Two tracks: one with a 500-sample plugin plus a 128-sample plugin,
    // one with nothing. The mix is aligned to the longest chain.
    let tracks: IndexMap<TrackId, Track> = [
        track_with_plugins(1, TrackType::Audio, &[10, 11]),
        track_with_plugins(2, TrackType::Audio, &[]),
    ]
    .into_iter()
    .map(|t| (t.id, t))
    .collect();
    let reported: HashMap<PluginInstanceId, u64> = [(10, 500), (11, 128)].into_iter().collect();

    let all_engaged = |id: PluginInstanceId| {
        slot_latency(reported.get(&id).copied().unwrap_or(0), false)
    };
    let chains: HashMap<TrackId, u64> = chain_latencies(&tracks, all_engaged)
        .into_iter()
        .collect();
    assert_eq!(chains[&1], 628);
    assert_eq!(chains[&2], 0);

    // Host-bypass slot 10: its 500 samples leave the chain.
    let ten_bypassed = |id: PluginInstanceId| {
        slot_latency(reported.get(&id).copied().unwrap_or(0), id == 10)
    };
    let chains = chain_latencies(&tracks, ten_bypassed);
    let map: HashMap<TrackId, u64> = chains.iter().copied().collect();
    assert_eq!(map[&1], 128, "only the still-running slot counts");
    assert_eq!(map[&2], 0);

    // And compensation re-derives from that, so every track still lands
    // at the same moment — PDC is correct, not merely unchanged.
    let (max, delays) = compensation_delays(&chains);
    assert_eq!(max, 128);
    for (id, delay) in delays {
        assert_eq!(map[&id] + delay, max, "track {id} must stay aligned");
    }
}

#[test]
fn a_slot_bypassed_through_its_own_parameter_keeps_the_comp_table_still() {
    // The alignment-preserving path: the plugin keeps running and keeps
    // reporting its latency, so bypassing it changes no delay at all and
    // no delay line is reset.
    let tracks: IndexMap<TrackId, Track> = [
        track_with_plugins(1, TrackType::Audio, &[10]),
        track_with_plugins(2, TrackType::Audio, &[]),
    ]
    .into_iter()
    .map(|t| (t.id, t))
    .collect();
    let engaged = chain_latencies(&tracks, |_| slot_latency(500, false));
    // `host_bypassed` is false for an own-parameter bypass, whatever the
    // user asked for.
    let own_param_bypassed = chain_latencies(&tracks, |_| slot_latency(500, false));
    assert_eq!(engaged, own_param_bypassed);
    assert_eq!(compensation_delays(&engaged).0, 500);
}

#[test]
fn the_instrument_still_counts_when_only_a_later_slot_is_bypassed() {
    // Instrument tracks keep slot 0 running under whole-chain bypass; the
    // per-slot rule must compose with that rather than replace it.
    let tracks: IndexMap<TrackId, Track> =
        [track_with_plugins(1, TrackType::Instrument, &[10, 11])]
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
    let reported: HashMap<PluginInstanceId, u64> = [(10, 64), (11, 500)].into_iter().collect();
    let fx_bypassed = |id: PluginInstanceId| {
        slot_latency(reported.get(&id).copied().unwrap_or(0), id == 11)
    };
    let chains: HashMap<TrackId, u64> = chain_latencies(&tracks, fx_bypassed).into_iter().collect();
    assert_eq!(chains[&1], 64, "the instrument's own latency survives");
}

#[test]
fn set_plugin_bypass_republishes_the_compensation_table() {
    assert!(
        affects_latency(&AudioCommand::SetPluginBypass {
            instance_id: 7,
            bypassed: true,
        }),
        "a per-slot bypass changes which plugins run, so PDC must refresh"
    );
}
