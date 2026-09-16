//! Plugin-delay-compensation math and delay-line behavior
//! (`crate::latency`): track-stage chain summation across tracks and
//! sub-tracks; bus-stage chain summation; the max-minus-chain delay
//! computation; and the `LatencyComp` apply paths — per-track, per-bus
//! and the shared dry line (delay, tail flush across blocks, reset on
//! playhead discontinuity, wet/dry send alignment). No live CLAP plugin
//! needed — plugin latencies are supplied by a lookup table and the
//! render-path tests drive synthetic comp tables through the real
//! `render_block` loop.

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use resonance_audio::__test_support::{
    add_external_offsets, affects_latency, bus_chain_latencies, chain_latencies,
    compensation_delays, render_aux_with_comp_for_test, AutomationSnapshot, LatencyComp,
    MAX_COMP_LATENCY,
};
use resonance_audio::types::{
    AudioCommand, AudioClip, AuxSend, Bus, BusId, ClipSource, FadeCurve, FrozenSource, SendId,
    SendSource, Track, TrackId, TrackOutput, TrackType,
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
fn chain_latencies_sum_track_and_parent_instrument_bus_stage_separate() {
    // Plugin latencies: 1 → 100, 2 → 50, 3 → 7, 4 → 30.
    let lat: HashMap<u64, u64> = [(1u64, 100u64), (2, 50), (3, 7), (4, 30)].into();

    let mut tracks: IndexMap<TrackId, Track> = IndexMap::new();
    // Track 10: two FX, routed to master → 150.
    let t10 = Track::new(10, "fx".into());
    t10.push_plugin(1);
    t10.push_plugin(2);
    tracks.insert(10, t10);
    // Track 11: no FX, routed to bus 5 (which carries plugin 3). The
    // bus chain is *not* part of the track stage — it shows up in
    // `bus_chain_latencies` and is equalized by the bus-stage delays.
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

    let resolve = |id| lat.get(&id).copied().unwrap_or(0);
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, resolve).into_iter().collect();
    assert_eq!(chains[&10], 150);
    assert_eq!(chains[&11], 0, "bus chains are equalized in the bus stage");
    assert_eq!(chains[&12], 30);
    assert_eq!(chains[&13], 80);

    let bus_chains: HashMap<BusId, u64> =
        bus_chain_latencies(&busses, resolve).into_iter().collect();
    assert_eq!(bus_chains[&5], 7);
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
        chain_latencies(&tracks, resolve).into_iter().collect();
    assert_eq!(chains[&10], 150, "live: instrument + FX (track stage)");

    // Frozen: the cache replaces instrument + FX (pre-trimmed), so the
    // track stage contributes nothing; the bus chain still runs live
    // downstream and stays counted in the bus stage.
    tracks[&10].frozen_source.store(Some(Arc::new(dummy_frozen_source())));
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, resolve).into_iter().collect();
    assert_eq!(chains[&10], 0, "frozen: idle chain adds no latency");
    let bus_chains: HashMap<BusId, u64> =
        bus_chain_latencies(&busses, resolve).into_iter().collect();
    assert_eq!(bus_chains[&5], 7, "the live bus chain still counts");
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
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, |id| lat.get(&id).copied().unwrap_or(0))
            .into_iter()
            .collect();
    assert_eq!(chains[&10], 0, "bypassed audio track contributes nothing");
    assert_eq!(chains[&11], 30, "instrument still counts; its FX don't");
}

#[test]
fn bypassed_bus_chain_contributes_zero() {
    let lat: HashMap<u64, u64> = [(3u64, 7u64)].into();
    let mut busses: IndexMap<BusId, Bus> = IndexMap::new();
    let mut bus = Bus::new(5, "bus".into());
    bus.plugin_ids.push(3);
    bus.set_fx_bypassed(true);
    busses.insert(5, bus);

    let bus_chains: HashMap<BusId, u64> =
        bus_chain_latencies(&busses, |id| lat.get(&id).copied().unwrap_or(0))
            .into_iter()
            .collect();
    assert_eq!(bus_chains[&5], 0, "bypassed bus chain is skipped by the mixer");
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

    let resolve = |id| lat.get(&id).copied().unwrap_or(0);
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, resolve).into_iter().collect();
    assert_eq!(chains[&13], 80, "live parent: sub inherits the instrument");

    // Frozen parent: the instrument fan-out never runs, so the sub-track
    // no longer inherits its latency — but its own FX still run live.
    tracks[&12].frozen_source.store(Some(Arc::new(dummy_frozen_source())));
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, resolve).into_iter().collect();
    assert_eq!(chains[&13], 50, "frozen parent: only the sub's own FX count");

    // A frozen source on the sub-track itself is ignored: the mixer's
    // sub-track pass always renders live off the parent fan-out.
    tracks[&13].frozen_source.store(Some(Arc::new(dummy_frozen_source())));
    let chains: HashMap<TrackId, u64> =
        chain_latencies(&tracks, resolve).into_iter().collect();
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
    // Master bypass changes no per-track comp, but it does change the
    // published master latency the reference A/B aligns with — so it
    // must run the refresh too (finding #19).
    assert!(affects_latency(&AudioCommand::SetMasterFxBypass { bypassed: true }));
    // Loading plugin state cycles the instance's activation and
    // re-reads its latency (finding #10) — the comp table must pick
    // the new value up.
    assert!(affects_latency(&AudioCommand::LoadPluginState {
        instance_id: 1,
        data: Vec::new(),
    }));
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
    let comp = LatencyComp::new(4, &[(1, 4), (2, 0)], 0, &[]);
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
    let comp = LatencyComp::new(max, &delays, 0, &[]);

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
    let comp = LatencyComp::new(4, &[(1, 4)], 0, &[]);
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
    let comp = LatencyComp::new(10, &[(1, 10), (2, 0), (3, 4)], 0, &[]);
    assert!(comp.delays_match(10, &[(1, 10), (2, 0), (3, 4)], 0, &[]));
    // Zero entries are irrelevant — they have no delay line.
    assert!(comp.delays_match(10, &[(3, 4), (1, 10)], 0, &[]));
    assert!(!comp.delays_match(10, &[(1, 10), (3, 5)], 0, &[]));
    assert!(!comp.delays_match(10, &[(1, 10)], 0, &[]));
    assert!(!comp.delays_match(10, &[(1, 10), (3, 4), (4, 2)], 0, &[]));
    // A track-stage max change forces a republish even with identical
    // per-track delays.
    assert!(!comp.delays_match(12, &[(1, 10), (2, 0), (3, 4)], 0, &[]));
    // Bus-stage changes force a republish too.
    assert!(!comp.delays_match(10, &[(1, 10), (2, 0), (3, 4)], 5, &[]));
    assert!(!comp.delays_match(10, &[(1, 10), (2, 0), (3, 4)], 0, &[(7, 3)]));

    let with_bus = LatencyComp::new(10, &[(1, 10)], 6, &[(7, 2)]);
    assert!(with_bus.delays_match(10, &[(1, 10)], 6, &[(7, 2)]));
    assert!(!with_bus.delays_match(10, &[(1, 10)], 4, &[(7, 2)]));
    assert!(!with_bus.delays_match(10, &[(1, 10)], 6, &[(7, 3)]));
    assert!(!with_bus.delays_match(8, &[(1, 10)], 6, &[(7, 2)]));

    let empty = LatencyComp::empty();
    assert!(empty.is_empty());
    assert!(empty.delays_match(0, &[(1, 0), (2, 0)], 0, &[]));
    assert!(!empty.delays_match(1, &[(1, 1)], 0, &[]));
    assert!(!empty.delays_match(0, &[], 3, &[]));
}

#[test]
fn delays_match_catches_track_max_change_with_identical_relative_delays() {
    // Regression (delays_match ignored track_max): in a single-track
    // project every relative delay is 0 forever — the sole track always
    // sits at the max. Adding a 2048-sample lookahead limiter therefore
    // changes no per-track delay entry, and a comparison over the
    // non-zero delay sets alone would skip the republish, leaving
    // max_latency == 0 / track_stage() == 0 stale (fader/pan/mute
    // automation then evaluates ~42.7 ms early at 48k).
    let empty = LatencyComp::empty();
    let (track_max, track_delays) = compensation_delays(&[(1, 2048)]);
    assert_eq!(track_max, 2048);
    assert!(
        track_delays.iter().all(|&(_, d)| d == 0),
        "single track: all relative delays stay 0"
    );
    assert!(
        !empty.delays_match(track_max, &track_delays, 0, &[]),
        "a track-stage max change must force a republish"
    );

    let republished = LatencyComp::new(track_max, &track_delays, 0, &[]);
    assert_eq!(republished.max_latency(), 2048);
    assert_eq!(republished.track_stage(), 2048);
    // The republished table matches its own inputs — the suppression
    // still protects delay-line state on no-op topology edits.
    assert!(republished.delays_match(track_max, &track_delays, 0, &[]));

    // Same shape: all tracks carrying equal chain latency shift
    // together, again without moving any relative delay.
    let (max2, delays2) = compensation_delays(&[(1, 512), (2, 512)]);
    assert!(delays2.iter().all(|&(_, d)| d == 0));
    assert!(!republished.delays_match(max2, &delays2, 0, &[]));

    // The max comparison clamps like the constructor does, so a
    // beyond-limit chain doesn't republish forever.
    let clamped = LatencyComp::new(MAX_COMP_LATENCY + 500, &[(1, 0)], 0, &[]);
    assert_eq!(clamped.max_latency(), MAX_COMP_LATENCY);
    assert!(clamped.delays_match(MAX_COMP_LATENCY + 500, &[(1, 0)], 0, &[]));
}

#[test]
fn apply_bus_and_dry_delay_lines() {
    // Track stage max 3 (track 1 delayed 3); bus stage max 4 with bus 7
    // padded by 1 (its own chain would be 3). Total pipeline = 3 + 4.
    let comp = LatencyComp::new(3, &[(1, 3)], 4, &[(7, 1)]);
    assert_eq!(comp.max_latency(), 7);
    assert_eq!(comp.bus_stage(), 4);
    assert!(!comp.is_empty());

    // Bus 7's summing buffer is delayed by exactly 1 frame.
    let mut l = [0.0f32; 8];
    let mut r = [0.0f32; 8];
    l[2] = 1.0;
    r[2] = 1.0;
    assert!(comp.apply_bus(7, &mut l, &mut r, 0));
    assert_eq!(l.iter().position(|&s| s != 0.0), Some(3));
    assert!(!comp.apply_bus(99, &mut l, &mut r, 8), "unknown bus has no entry");

    // The dry (master-direct) interleaved sum is delayed by the full
    // bus stage (4 frames).
    let mut data = [0.0f32; 16]; // 8 frames stereo
    data[2 * 2] = 1.0;
    data[2 * 2 + 1] = -1.0;
    assert!(comp.apply_dry(&mut data, 2, 8, 0));
    let pos = data
        .chunks(2)
        .position(|f| f.iter().any(|&s| s != 0.0));
    assert_eq!(pos, Some(6), "impulse at frame 2 emerges at 2 + bus_stage");
    assert_eq!(data[6 * 2], 1.0);
    assert_eq!(data[6 * 2 + 1], -1.0);
}

#[test]
fn apply_dry_is_noop_without_bus_stage_latency() {
    let comp = LatencyComp::new(3, &[(1, 3)], 0, &[]);
    let mut data = [0.0f32; 8];
    data[0] = 1.0;
    assert!(!comp.apply_dry(&mut data, 2, 4, 0));
    assert_eq!(data[0], 1.0, "buffer untouched — common path stays identical");
}

/// An impulse clip: interleaved-stereo silence with a single `1.0`
/// frame at `at`, so post-render positions are exactly assertable.
fn impulse_track(id: TrackId, output: TrackOutput, at: usize, frames: usize) -> (Track, AudioClip) {
    let track = Track::new(id, format!("t{id}"));
    track.set_output(output);
    let mut samples = vec![0.0f32; frames * 2];
    samples[at * 2] = 1.0;
    samples[at * 2 + 1] = 1.0;
    let clip = AudioClip {
        id,
        track_id: id,
        start_sample: 0,
        source: ClipSource::Memory(samples),
        name: "impulse".into(),
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
    };
    (track, clip)
}

/// Frames where the interleaved-stereo output carries any signal.
fn nonzero_frames(data: &[f32]) -> Vec<usize> {
    data.chunks(2)
        .enumerate()
        .filter(|(_, f)| f.iter().any(|&s| s != 0.0))
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn render_block_delays_master_direct_signal_by_bus_stage() {
    // A master-routed impulse at frame 5 must emerge `bus_stage` (4)
    // frames later through the shared dry line in the real render loop.
    const FRAMES: usize = 48;
    let (track, clip) = impulse_track(1, TrackOutput::Master, 5, FRAMES);
    let comp = LatencyComp::new(0, &[], 4, &[]);
    let (data, _busses) = render_aux_with_comp_for_test(
        vec![track],
        vec![],
        vec![clip],
        vec![],
        FRAMES,
        48_000,
        comp,
        AutomationSnapshot::default(),
    );
    assert_eq!(nonzero_frames(&data), vec![5 + 4]);
}

#[test]
fn render_block_aligns_wet_send_with_dry_master_path() {
    // Track 1 routes to master AND pre-fader-sends into return bus 10.
    // The comp table models a bus stage of 4 where bus 10's own chain
    // is latency-free (padded by the full 4): the wet return and the
    // dry master path must land on the same output frame — the exact
    // wet/dry alignment finding #7 is about.
    // The impulse sits past the clip's automatic edge declick so its
    // amplitude — not just its position — can be asserted exactly.
    const FRAMES: usize = 512;
    const AT: usize = 200;
    let (track, clip) = impulse_track(1, TrackOutput::Master, AT, FRAMES);
    let ret = Bus::new(10, "return".into());
    let send = AuxSend {
        id: 1 as SendId,
        source: SendSource::Track(1),
        dest: 10,
        level_db: 0.0,
        pre_fader: true,
        enabled: true,
    };
    let comp = LatencyComp::new(0, &[], 4, &[(10, 4)]);
    let (data, _busses) = render_aux_with_comp_for_test(
        vec![track],
        vec![ret],
        vec![clip],
        vec![send],
        FRAMES,
        48_000,
        comp,
        AutomationSnapshot::default(),
    );
    // One single frame carries all the energy: dry + wet, together.
    assert_eq!(
        nonzero_frames(&data),
        vec![AT + 4],
        "wet return must not smear against the dry path"
    );
    // Dry and wet (send at 0 dB through the return's own fader) sum —
    // i.e. the frame holds MORE than the dry path alone, proving the wet
    // contribution landed on the same frame. Centre pan is unity on a
    // stereo-balance track (ba doc #276 BUG 3), so each path is 1.0.
    let g = 1.0f32;
    assert!((data[(AT + 4) * 2] - 2.0 * g).abs() < 1e-6);
}
