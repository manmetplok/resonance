//! Bit-identity golden for the shared render core (`mixer::render_block`,
//! ba todo #1252).
//!
//! The render core is the hot path shared by the live audio callback and
//! the offline bounce, so a refactor of it is only correct if the samples
//! it produces are **bit**-identical — not "close enough". These tests
//! render two deliberately busy projects, one through the Bounce strategy
//! and one through the Live strategy, and hash the raw `f32::to_bits()` of
//! every output sample (master *and* every bus summing buffer). The
//! expected hashes were captured from the pre-refactor render core.
//!
//! Between them the fixtures exercise every phase of the block that can be
//! driven without a real CLAP plugin: automation gain/pan/mute evaluation
//! at the comp-delayed position, per-track plugin-delay compensation, the
//! dry-path and bus-stage delay lines, clip mixing with explicit fades,
//! the automatic same-track crossfade and the edge declick, clip gain,
//! frozen-source playback substitution at both matching and mismatched
//! sample rates, bus routing, pre- and post-fader aux sends from tracks
//! and from busses, the per-bus pass, live gain ramps chained across
//! blocks through the last-gain atomics, and the live mute fade-out. The
//! instrument / multi-output fan-out phases need a plugin instance and are
//! covered bit-exactly by `stem_sub_track_render.rs`,
//! `stem_bus_sub_track_render.rs` and `sub_track_parent_fader.rs`.
//!
//! If one of these hashes changes, the render core's output changed. That
//! is a real behavioural difference: re-bless the constant only together
//! with a deliberate, documented change to the mix maths.

use std::sync::Arc;

use resonance_audio::__test_support::{
    render_aux_with_comp_for_test, AutomationSnapshot, LatencyComp, RenderBenchHarness,
};
use resonance_audio::types::*;
use resonance_common::{
    real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint, CurveKind, FreezeCacheRef,
    FreezeCacheStatus,
};

const SR: u32 = 48_000;
const BLOCK: usize = 128;

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over the raw IEEE-754 bit patterns, so a one-ULP difference is a
/// hash difference.
struct BitHash(u64);

impl BitHash {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }

    fn feed(&mut self, samples: &[f32]) -> &mut Self {
        for s in samples {
            for b in s.to_bits().to_le_bytes() {
                self.0 ^= b as u64;
                self.0 = self.0.wrapping_mul(FNV_PRIME);
            }
        }
        self
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Deterministic pseudo-random sample data: a plain LCG mapped to
/// `[-1, 1)`. Every bit of it is reproducible, and unlike a DC fixture it
/// makes each frame of the block distinct, so a delay line that is off by
/// one sample shows up in the hash.
fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn clip(
    id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    frames: usize,
    seed: u32,
    gain_db: f32,
    fade_in: u64,
    fade_out: u64,
) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample,
        source: ClipSource::Memory(noise(frames * 2, seed)),
        name: format!("c{id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: fade_in,
        fade_in_curve: FadeCurve::EqualPower,
        fade_out_frames: fade_out,
        fade_out_curve: FadeCurve::Linear,
        gain_db,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn frozen(dc_seed: u32, frames: usize, rate: u32) -> FrozenSource {
    let samples = Arc::new(noise(frames * 2, dc_seed));
    let cache_ref = FreezeCacheRef::new(
        "parity.wav".into(),
        rate,
        32,
        1,
        FreezeCacheStatus::Frozen,
    );
    FrozenSource::new(cache_ref, samples, rate, frames as u64)
}

fn aux(id: SendId, source: SendSource, dest: BusId, level_db: f32, pre_fader: bool) -> AuxSend {
    AuxSend {
        id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled: true,
    }
}

fn lane(id: u64, target: AutomationTarget, points: &[(u64, f32)]) -> AutomationLane {
    let bps = points
        .iter()
        .map(|&(frame, real)| {
            Breakpoint::new(
                frame,
                real_to_lane_value(&target, real),
                CurveKind::Linear,
            )
        })
        .collect();
    AutomationLane::new(id, target, bps)
}

const FEEDER: BusId = 10;
const RETURN: BusId = 20;

/// Fader / pan / mute automation over four different targets, so the
/// comp-delayed evaluation of each is baked into the hash.
fn automation() -> AutomationSnapshot {
    let mut snap = AutomationSnapshot::default();
    for l in [
        lane(
            1,
            AutomationTarget::TrackGain(1),
            &[(0, 0.0), (4 * BLOCK as u64, -18.0)],
        ),
        lane(
            2,
            AutomationTarget::TrackPan(2),
            &[(0, -0.9), (4 * BLOCK as u64, 0.9)],
        ),
        lane(3, AutomationTarget::TrackMute(5), &[(0, 1.0)]),
        lane(
            4,
            AutomationTarget::BusGain(FEEDER),
            &[(0, -3.0), (4 * BLOCK as u64, 3.0)],
        ),
    ] {
        snap.mix_lanes.insert(l.target.clone(), l);
    }
    snap
}

/// Per-track and per-bus compensation delays, plus the shared dry line —
/// so the block runs `apply`, `apply_bus` and `apply_dry` with non-zero
/// delays and the automation evaluation is shifted on both stages.
fn comp() -> LatencyComp {
    LatencyComp::new(64, &[(1, 16), (2, 64), (3, 5)], 32, &[(FEEDER, 32)])
}

/// The busy project: five tracks (clips with fades and an overlap
/// crossfade, two frozen sources, a mute-automated track), two busses, and
/// three aux sends covering pre-fader, post-fader and bus-sourced taps.
fn project() -> (Vec<Track>, Vec<Bus>, Vec<AudioClip>, Vec<AuxSend>) {
    let t1 = Track::new(1, "clips".into());
    t1.set_output(TrackOutput::Master);
    t1.set_volume(0.8);
    t1.set_pan(-0.3);

    let t2 = Track::new(2, "to-bus".into());
    t2.set_output(TrackOutput::Bus(FEEDER));
    t2.set_volume(1.2);
    t2.set_pan(0.45);

    let t3 = Track::new(3, "frozen".into());
    t3.set_output(TrackOutput::Bus(FEEDER));
    t3.set_volume(0.5);
    t3.frozen_source
        .store(Some(Arc::new(frozen(77, BLOCK * 4, SR))));

    let t4 = Track::new(4, "frozen-resampled".into());
    t4.set_output(TrackOutput::Master);
    t4.set_pan(0.2);
    // A cache rendered at a different rate takes the linear-interpolation
    // branch of the frozen-source read.
    t4.frozen_source
        .store(Some(Arc::new(frozen(91, BLOCK * 4, 44_100))));

    let t5 = Track::new(5, "mute-automated".into());
    t5.set_output(TrackOutput::Master);

    let feeder = Bus::new(FEEDER, "Feeder".into());
    feeder.set_volume(0.7);
    feeder.set_pan(-0.2);
    let ret = Bus::new(RETURN, "Return".into());
    ret.set_volume(0.9);
    ret.set_is_return(true);

    let clips = vec![
        // Two overlapping clips on track 1: the overlap is an automatic
        // crossfade on top of the explicit fades and the edge declick.
        clip(1, 1, 0, BLOCK * 3, 11, -3.0, 32, 48),
        clip(2, 1, (BLOCK * 2) as u64, BLOCK * 3, 12, 0.0, 0, 0),
        clip(3, 2, 8, BLOCK * 3, 13, 2.0, 64, 0),
        clip(4, 5, 0, BLOCK * 3, 14, 0.0, 0, 0),
    ];

    let sends = vec![
        aux(1, SendSource::Track(1), RETURN, -6.0, false),
        aux(2, SendSource::Track(2), RETURN, -3.0, true),
        aux(3, SendSource::Bus(FEEDER), RETURN, -12.0, false),
    ];

    (
        vec![t1, t2, t3, t4, t5],
        vec![feeder, ret],
        clips,
        sends,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Offline (Bounce) strategy: one block at playhead 0 with automation and
/// a full compensation table.
#[test]
fn bounce_block_output_is_bit_identical() {
    const EXPECTED: u64 = 0x99fe_15e8_e91d_a389;

    let (tracks, busses, clips, sends) = project();
    let (data, bus_bufs) = render_aux_with_comp_for_test(
        tracks,
        busses,
        clips,
        sends,
        BLOCK,
        SR,
        comp(),
        automation(),
    );

    // Guard against hashing silence: a fixture that renders nothing would
    // pass the hash check for the wrong reason.
    assert!(
        data.iter().any(|&s| s != 0.0),
        "fixture must produce master audio"
    );
    assert!(
        bus_bufs.iter().any(|(l, _)| l.iter().any(|&s| s != 0.0)),
        "fixture must produce bus audio"
    );

    let mut h = BitHash::new();
    h.feed(&data);
    for (l, r) in &bus_bufs {
        h.feed(l).feed(r);
    }
    let got = h.finish();
    assert_eq!(
        got, EXPECTED,
        "bounce render output changed (got {got:#018x}) — the render core is \
         no longer bit-identical"
    );
}

/// Live strategy across eight consecutive blocks: gain ramps chain through
/// the last-gain atomics, a muted track fades out over its first block,
/// and the VU peak writes run.
#[test]
fn live_blocks_output_is_bit_identical() {
    const EXPECTED: u64 = 0xee24_8fd4_1d34_8c76;

    let (tracks, busses, clips, sends) = project();
    // The live path has no automation snapshot in the bench harness, so
    // pin the mute statically instead — the mute fade-out is a live-only
    // branch of the disposition.
    tracks[4].set_muted(true);
    tracks[4].set_last_gains(0.6, 0.6);

    let mut harness = RenderBenchHarness::new(
        tracks,
        busses,
        clips,
        Vec::new(),
        sends,
        TempoMap::default(),
        BLOCK,
        SR,
    );

    let mut h = BitHash::new();
    let mut heard = false;
    for block in 0..8u64 {
        let out = harness.render(block * BLOCK as u64);
        heard |= out.iter().any(|&s| s != 0.0);
        h.feed(out);
    }
    assert!(heard, "fixture must produce live audio");
    let got = h.finish();
    assert_eq!(
        got, EXPECTED,
        "live render output changed (got {got:#018x}) — the render core is \
         no longer bit-identical"
    );
}
