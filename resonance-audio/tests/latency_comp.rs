//! Plugin-delay-compensation math and delay-line behavior
//! (`crate::latency`): per-chain latency summation across tracks,
//! sub-tracks and busses; the max-minus-chain delay computation; and
//! the `LatencyComp` apply path (delay, tail flush across blocks,
//! reset on playhead discontinuity). No live CLAP plugin needed —
//! plugin latencies are supplied by a lookup table.

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use resonance_audio::__test_support::{
    add_external_offsets, affects_latency, chain_latencies, compensation_delays, LatencyComp,
    MAX_COMP_LATENCY,
};
use resonance_audio::types::{
    AudioCommand, Bus, BusId, FrozenSource, Track, TrackId, TrackOutput, TrackType,
};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

/// A minimal frozen source so a test track reads as frozen; the decoded
/// samples themselves are never touched by the latency math.
fn dummy_frozen_source() -> FrozenSource {
    let cache_ref = FreezeCacheRef::new("t.wav".into(), 48_000, 32, 1, FreezeCacheStatus::Frozen);
    FrozenSource::new(cache_ref, Arc::new(vec![0.0; 4]), 48_000, 2)
}

#[test]
fn compensation_delays_align_every_chain_to_max() {
    let chains = vec![(1u64, 389), (2, 0), (3, 100)];
    let (max, delays) = compensation_delays(&chains);
    assert_eq!(max, 389);
    assert_eq!(delays.len(), chains.len());
    for (&(id, chain), &(d_id, delay)) in chains.iter().zip(delays.iter()) {
        assert_eq!(id, d_id);
        assert_eq!(chain + delay, max, "track {id} must align to max");
    }
}

#[test]
fn compensation_delays_empty_and_all_zero() {
    assert_eq!(compensation_delays(&[]), (0, vec![]));
    let (max, delays) = compensation_delays(&[(1, 0), (2, 0)]);
    assert_eq!(max, 0);
    assert!(delays.iter().all(|&(_, d)| d == 0));
}

#[test]
fn compensation_delays_clamp_hostile_latency() {
    let (max, delays) = compensation_delays(&[(1, u64::MAX), (2, 0)]);
    assert_eq!(max, MAX_COMP_LATENCY);
    assert_eq!(delays, vec![(1, 0), (2, MAX_COMP_LATENCY)]);
}

#[test]
fn chain_latencies_sum_track_bus_and_parent_instrument() {
    // Plugin latencies: 1 → 100, 2 → 50, 3 → 7, 4 → 30.
    let lat: HashMap<u64, u64> = [(1u64, 100u64), (2, 50), (3, 7), (4, 30)].into();

    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    // Track 10: two FX, routed to master → 150.
    let t10 = Track::new(10, "fx".into());
    t10.push_plugin(1);
    t10.push_plugin(2);
    tracks.insert(10, t10);
    // Track 11: no FX, routed to bus 5 (which carries plugin 3) → 7.
    let t11 = Track::new(11, "to bus".into());
    t11.set_output(TrackOutput::Bus(5));
    tracks.insert(11, t11);
    // Track 12: instrument (plugin 4) → 30.
    let t12 = Track::new(12, "parent".into());
    t12.push_plugin(4);
    tracks.insert(12, t12);
    // Track 13: sub-track of 12 port 1, own FX plugin 2 → 30 + 50 = 80.
    let t13 = Track::new_sub_track(13, "sub".into(), 12, 1);
    t13.push_plugin(2);
    tracks.insert(13, t13);

    let mut busses: IndexMap<BusId, Bus> = IndexMap::new();
    let mut bus = Bus::new(5, "bus".into());
    bus.plugin_ids.push(3);
    busses.insert(5, bus);

    let chains = chain_latencies(&tracks, &busses, |id| lat.get(&id).copied().unwrap_or(0));
    let chains: HashMap<TrackId, u64> = chains.into_iter().collect();
    assert_eq!(chains[&10], 150);
    assert_eq!(chains[&11], 7);
    assert_eq!(chains[&12], 30);
    assert_eq!(chains[&13], 80);
}

#[test]
fn frozen_track_excludes_own_chain_but_keeps_downstream_bus() {
    // Instrument 100 + FX 50 on the track, plugin 7 on its output bus.
    let lat: HashMap<u64, u64> = [(1u64, 100u64), (2, 50), (3, 7)].into();
    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    let t = Track::with_type(10, "inst".into(), TrackType::Instrument);
    t.push_plugin(1);
    t.push_plugin(2);
    t.set_output(TrackOutput::Bus(5));
    tracks.insert(10, t);
    let mut busses: IndexMap<BusId, Bus> = IndexMap::new();
    let mut bus = Bus::new(5, "bus".into());
    bus.plugin_ids.push(3);
    busses.insert(5, bus);

    let resolve = |id| lat.get(&id).copied().unwrap_or(0);
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, resolve).into_iter().collect();
    assert_eq!(chains[&10], 157, "live: instrument + FX + bus");

    // Frozen: the cache replaces instrument + FX (pre-trimmed), but the
    // bus chain still runs live downstream and must stay compensated.
    tracks[&10].frozen_source.store(Some(Arc::new(dummy_frozen_source())));
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, resolve).into_iter().collect();
    assert_eq!(chains[&10], 7, "frozen: only the live bus chain counts");
}

#[test]
fn fx_bypass_excludes_effects_but_not_the_instrument() {
    let lat: HashMap<u64, u64> = [(1u64, 100u64), (2, 50), (4, 30)].into();
    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    // Audio track: every plugin is an effect — bypass drops them all.
    let audio = Track::new(10, "audio".into());
    audio.push_plugin(1);
    audio.push_plugin(2);
    audio.set_fx_bypassed(true);
    tracks.insert(10, audio);
    // Instrument track: the instrument (first plugin) keeps running
    // under FX bypass; only the effect after it is skipped.
    let inst = Track::with_type(11, "inst".into(), TrackType::Instrument);
    inst.push_plugin(4);
    inst.push_plugin(2);
    inst.set_fx_bypassed(true);
    tracks.insert(11, inst);
    let busses: IndexMap<BusId, Bus> = IndexMap::new();

    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, |id| lat.get(&id).copied().unwrap_or(0))
            .into_iter()
            .collect();
    assert_eq!(chains[&10], 0, "bypassed audio track contributes nothing");
    assert_eq!(chains[&11], 30, "instrument still counts; its FX don't");
}

#[test]
fn bypassed_bus_chain_contributes_zero() {
    let lat: HashMap<u64, u64> = [(3u64, 7u64)].into();
    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    let t = Track::new(10, "to bus".into());
    t.set_output(TrackOutput::Bus(5));
    tracks.insert(10, t);
    let mut busses: IndexMap<BusId, Bus> = IndexMap::new();
    let mut bus = Bus::new(5, "bus".into());
    bus.plugin_ids.push(3);
    bus.set_fx_bypassed(true);
    busses.insert(5, bus);

    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, |id| lat.get(&id).copied().unwrap_or(0))
            .into_iter()
            .collect();
    assert_eq!(chains[&10], 0, "bypassed bus chain is skipped by the mixer");
}

#[test]
fn frozen_parent_drops_parent_instrument_for_sub_tracks() {
    let lat: HashMap<u64, u64> = [(2u64, 50u64), (4, 30)].into();
    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    let parent = Track::with_type(12, "parent".into(), TrackType::Instrument);
    parent.push_plugin(4);
    tracks.insert(12, parent);
    let sub = Track::new_sub_track(13, "sub".into(), 12, 1);
    sub.push_plugin(2);
    tracks.insert(13, sub);
    let busses: IndexMap<BusId, Bus> = IndexMap::new();

    let resolve = |id| lat.get(&id).copied().unwrap_or(0);
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, resolve).into_iter().collect();
    assert_eq!(chains[&13], 80, "live parent: sub inherits the instrument");

    // Frozen parent: the instrument fan-out never runs, so the sub-track
    // no longer inherits its latency — but its own FX still run live.
    tracks[&12].frozen_source.store(Some(Arc::new(dummy_frozen_source())));
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, resolve).into_iter().collect();
    assert_eq!(chains[&13], 50, "frozen parent: only the sub's own FX count");

    // A frozen source on the sub-track itself is ignored: the mixer's
    // sub-track pass always renders live off the parent fan-out.
    tracks[&13].frozen_source.store(Some(Arc::new(dummy_frozen_source())));
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, &busses, resolve).into_iter().collect();
    assert_eq!(chains[&13], 50, "sub-track frozen flag doesn't change rendering");
}

#[test]
fn freeze_and_bypass_commands_refresh_the_comp_table() {
    // Freeze / bypass toggles change which plugins actually run, so the
    // engine loop must rebuild delay lines after they execute.
    assert!(affects_latency(&AudioCommand::SetTrackFrozenSource {
        track_id: 1,
        source: None,
    }));
    assert!(affects_latency(&AudioCommand::UnfreezeTrack { track_id: 1 }));
    assert!(affects_latency(&AudioCommand::SetTrackFxBypass {
        track_id: 1,
        bypassed: true,
    }));
    assert!(affects_latency(&AudioCommand::SetBusFxBypass {
        bus_id: 5,
        bypassed: true,
    }));
    // Master bypass delays every path equally — no per-track comp change.
    assert!(!affects_latency(&AudioCommand::SetMasterFxBypass { bypassed: true }));
}

#[test]
fn add_external_offsets_folds_positive_only() {
    let mut chains = vec![(1u64, 100u64), (2, 0), (3, 50)];
    let offsets: HashMap<TrackId, i64> = [(1, 512i64), (2, -10), (3, 0)].into();
    add_external_offsets(&mut chains, |id| offsets.get(&id).copied().unwrap_or(0));
    // Positive offset stacks on top of the existing plugin-chain latency.
    assert_eq!(chains[0], (1, 612));
    // Negative offset is ignored — a live return can't be advanced.
    assert_eq!(chains[1], (2, 0));
    // Zero offset (and untracked tracks) are no-ops.
    assert_eq!(chains[2], (3, 50));
}

#[test]
fn external_offset_delays_rest_of_mix_to_meet_return() {
    // Track 1 is an external instrument whose hardware return is 480
    // samples round-trip late; track 2 is a plain track with no latency.
    // After folding the offset, PDC must hold the return at 0 and delay
    // the rest of the mix by 480 so everything lands together.
    let mut chains = vec![(1u64, 0u64), (2, 0)];
    add_external_offsets(&mut chains, |id| if id == 1 { 480 } else { 0 });
    let (max, delays) = compensation_delays(&chains);
    assert_eq!(max, 480);
    let delays: HashMap<TrackId, u64> = delays.into_iter().collect();
    assert_eq!(delays[&1], 0, "the late return is never delayed further");
    assert_eq!(delays[&2], 480, "the rest of the mix waits for the return");
}

#[test]
fn apply_delays_signal_and_flushes_tail_across_blocks() {
    // Track 1 gets a 4-frame delay; track 2 (delay 0) gets no entry.
    let comp = LatencyComp::new(4, &[(1, 4), (2, 0)]);
    assert_eq!(comp.max_latency(), 4);
    assert_eq!(comp.delay_for(1), 4);
    assert_eq!(comp.delay_for(2), 0);

    let mut l = [0.0f32; 8];
    let mut r = [0.0f32; 8];
    assert!(!comp.apply(2, &mut l, &mut r, 0), "zero-delay track has no entry");
    assert!(!comp.apply(99, &mut l, &mut r, 0), "unknown track has no entry");

    // Impulse at timeline frame 6 must emerge at frame 10 — i.e. in the
    // *next* block, even though the track itself contributes nothing then.
    l[6] = 1.0;
    r[6] = -1.0;
    assert!(comp.apply(1, &mut l, &mut r, 0));
    assert!(l.iter().chain(r.iter()).all(|&s| s == 0.0), "block 0 is all pre-delay silence");

    let mut l2 = [0.0f32; 8];
    let mut r2 = [0.0f32; 8];
    assert!(comp.apply(1, &mut l2, &mut r2, 8));
    assert_eq!(l2[2], 1.0);
    assert_eq!(r2[2], -1.0);
    let rest: f32 = l2.iter().chain(r2.iter()).map(|s| s.abs()).sum::<f32>() - 2.0;
    assert_eq!(rest, 0.0, "only the delayed impulse may appear");
}

#[test]
fn apply_aligns_tracks_with_different_chain_latencies() {
    // Two tracks "play" the same timeline event. Track 1's chain is 3
    // frames late (simulated by writing the event 3 frames later);
    // track 2's chain has no latency. With delays (0, 3) both events
    // must land on the same output frame.
    let chains = vec![(1u64, 3u64), (2, 0)];
    let (max, delays) = compensation_delays(&chains);
    let comp = LatencyComp::new(max, &delays);

    let mut t1_l = [0.0f32; 16];
    let mut t1_r = [0.0f32; 16];
    t1_l[5 + 3] = 1.0; // event at frame 5, chain pushed it 3 frames late
    t1_r[5 + 3] = 1.0;
    comp.apply(1, &mut t1_l, &mut t1_r, 0);

    let mut t2_l = [0.0f32; 16];
    let mut t2_r = [0.0f32; 16];
    t2_l[5] = 1.0;
    t2_r[5] = 1.0;
    comp.apply(2, &mut t2_l, &mut t2_r, 0);

    let pos1 = t1_l.iter().position(|&s| s != 0.0);
    let pos2 = t2_l.iter().position(|&s| s != 0.0);
    assert_eq!(pos1, pos2, "both tracks must align after compensation");
    assert_eq!(pos1, Some(5 + max as usize));
}

#[test]
fn apply_resets_on_playhead_discontinuity() {
    let comp = LatencyComp::new(4, &[(1, 4)]);
    let mut l = [0.0f32; 8];
    let mut r = [0.0f32; 8];
    l[7] = 1.0;
    r[7] = 1.0;
    comp.apply(1, &mut l, &mut r, 0); // tail now owes an impulse at frame 11

    // Seek: the next block starts at 100, not 8 — the stale tail must
    // not replay at the new position.
    let mut l2 = [0.0f32; 8];
    let mut r2 = [0.0f32; 8];
    comp.apply(1, &mut l2, &mut r2, 100);
    assert!(l2.iter().chain(r2.iter()).all(|&s| s == 0.0));
}

#[test]
fn delays_match_detects_unchanged_tables() {
    let comp = LatencyComp::new(10, &[(1, 10), (2, 0), (3, 4)]);
    assert!(comp.delays_match(&[(1, 10), (2, 0), (3, 4)]));
    // Zero entries are irrelevant — they have no delay line.
    assert!(comp.delays_match(&[(3, 4), (1, 10)]));
    assert!(!comp.delays_match(&[(1, 10), (3, 5)]));
    assert!(!comp.delays_match(&[(1, 10)]));
    assert!(!comp.delays_match(&[(1, 10), (3, 4), (4, 2)]));

    let empty = LatencyComp::empty();
    assert!(empty.is_empty());
    assert!(empty.delays_match(&[(1, 0), (2, 0)]));
    assert!(!empty.delays_match(&[(1, 1)]));
}
