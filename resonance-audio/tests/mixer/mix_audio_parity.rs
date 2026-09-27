//! Bit-identity golden for the audio callback itself (`mixer::mix_audio`,
//! ba todo #1255).
//!
//! `mix_audio` is the entry point the realtime thread calls: it picks up
//! live MIDI, reads the monitor ring, then takes one of the reference-A/B,
//! count-in, stopped-monitor or playing branches, stitches
//! the render across the loop seam, and runs the master FX / metronome /
//! master-volume / metering passes. Decomposing it is only correct if the
//! samples it produces — and the state it publishes back — stay **bit**
//! identical.
//!
//! Each test below drives one branch through [`MixAudioHarness`] (the same
//! locks, scratch, monitor ring and channels the engine hands the
//! callback) and hashes the raw `f32::to_bits()` of every output sample
//! together with the callback's published side effects: playhead, master
//! peaks, per-track VU peaks and ramp state, the shortfall counter, the audition position and the reference cursor. The expected
//! hashes were captured from the pre-refactor callback.
//!
//! If one of these hashes changes, the callback's behaviour changed. That
//! is a real difference: re-bless a constant only together with a
//! deliberate, documented change to the mix.

use resonance_audio::test_support::{AutomationSnapshot, LatencyComp, MixAudioHarness};
use resonance_audio::types::*;
use resonance_common::{real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint, CurveKind};

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const CH: usize = 2;
const IN_CH: usize = 4;

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over raw IEEE-754 bit patterns, so a one-ULP difference is a
/// hash difference.
struct BitHash(u64);

impl BitHash {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }

    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(FNV_PRIME);
    }

    fn feed(&mut self, samples: &[f32]) -> &mut Self {
        for s in samples {
            for b in s.to_bits().to_le_bytes() {
                self.byte(b);
            }
        }
        self
    }

    fn feed_u64(&mut self, values: &[u64]) -> &mut Self {
        for v in values {
            for b in v.to_le_bytes() {
                self.byte(b);
            }
        }
        self
    }

    fn feed_pairs(&mut self, pairs: &[(f32, f32)]) -> &mut Self {
        for &(a, b) in pairs {
            self.feed(&[a, b]);
        }
        self
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Deterministic pseudo-random samples in `[-1, 1)`: a plain LCG, so every
/// frame of every block is distinct and a delay line that is off by one
/// sample shows up in the hash.
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

const FEEDER: BusId = 10;
const RETURN: BusId = 20;

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
        source: ClipSource::memory(noise(frames * 2, seed)),
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
            Breakpoint::new(frame, real_to_lane_value(&target, real), CurveKind::Linear)
        })
        .collect();
    AutomationLane::new(id, target, bps)
}

/// Master-gain, track-gain and bus-gain lanes, so the callback's
/// comp-delayed master evaluation and the render core's track/bus
/// evaluation are both baked into the hash.
fn automation() -> AutomationSnapshot {
    let mut snap = AutomationSnapshot::default();
    for l in [
        lane(
            1,
            AutomationTarget::MasterGain,
            &[(0, 0.0), (8 * BLOCK as u64, -12.0)],
        ),
        lane(
            2,
            AutomationTarget::TrackGain(1),
            &[(0, 0.0), (8 * BLOCK as u64, -18.0)],
        ),
        lane(
            3,
            AutomationTarget::BusGain(FEEDER),
            &[(0, -3.0), (8 * BLOCK as u64, 3.0)],
        ),
    ] {
        snap.mix_lanes.insert(l.target.clone(), l);
    }
    snap
}

/// Tempo map with the metronome armed, so the count-in and timeline click
/// passes both run.
fn tempo(metronome: bool) -> TempoMap {
    let mut map = TempoMap::default();
    map.bpm = 132.0;
    map.numerator = 4;
    map.denominator = 4;
    map.metronome_enabled = metronome;
    map.rebuild_bar_table(SR);
    map
}

/// Four tracks (two carrying clips, one monitored input, one muted), two
/// busses and three aux sends covering pre-fader, post-fader and
/// bus-sourced taps.
fn project() -> (Vec<Track>, Vec<Bus>, Vec<AudioClip>, Vec<AuxSend>) {
    let mut t1 = Track::new(1, "clips".into());
    t1.set_output(TrackOutput::Master);
    t1.set_volume(0.8);
    t1.set_pan(-0.3);

    let mut t2 = Track::new(2, "to-bus".into());
    t2.set_output(TrackOutput::Bus(FEEDER));
    t2.set_volume(1.2);
    t2.set_pan(0.45);

    let mut t3 = Track::new(3, "monitored".into());
    t3.set_output(TrackOutput::Master);
    t3.set_monitor_enabled(true);
    t3.set_record_armed(true);
    t3.set_input_port(1);
    t3.set_volume(0.9);

    let mut t4 = Track::new(4, "muted".into());
    t4.set_output(TrackOutput::Master);
    t4.set_muted(true);

    let feeder = Bus::new(FEEDER, "Feeder".into());
    feeder.set_volume(0.7);
    feeder.set_pan(-0.2);
    let ret = Bus::new(RETURN, "Return".into());
    ret.set_volume(0.9);
    ret.set_is_return(true);

    let clips = vec![
        clip(1, 1, 0, BLOCK * 5, 11, -3.0, 32, 48),
        clip(2, 1, (BLOCK * 3) as u64, BLOCK * 5, 12, 0.0, 0, 0),
        clip(3, 2, 8, BLOCK * 6, 13, 2.0, 64, 0),
        clip(4, 4, 0, BLOCK * 4, 14, 0.0, 0, 0),
    ];

    let sends = vec![
        aux(1, SendSource::Track(1), RETURN, -6.0, false),
        aux(2, SendSource::Track(2), RETURN, -3.0, true),
        aux(3, SendSource::Bus(FEEDER), RETURN, -12.0, false),
    ];

    (vec![t1, t2, t3, t4], vec![feeder, ret], clips, sends)
}

fn harness(metronome: bool) -> MixAudioHarness {
    let (tracks, busses, clips, sends) = project();
    let h = MixAudioHarness::new(
        tracks,
        busses,
        clips,
        Vec::new(),
        sends,
        tempo(metronome),
        BLOCK,
        CH,
        SR,
        true,
    );
    h.set_latency_comp(LatencyComp::new(64, &[(1, 16), (2, 64)], 32, &[(FEEDER, 32)]));
    h.set_automation(automation());
    h
}

/// One block of interleaved capture audio for `IN_CH` channels.
fn input_block(seed: u32) -> Vec<f32> {
    noise(BLOCK * IN_CH, seed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The playing branch: eight blocks over a loop whose `loop_out` falls
/// mid-block, so the seam split renders head + tail sub-blocks with the
/// instrument panic in between; monitoring, aux sends, automation, the
/// master pass, the metronome and the mix meter all run.
#[test]
fn playing_blocks_with_loop_seam_are_bit_identical() {
    // Re-blessed for FU-B6a: the feeder bus's first live block now renders
    // flat at its target gain instead of ramping in from the
    // construction-time 0 (0x17d0_3079_dca4_3989 → this).
    const EXPECTED: u64 = 0x2398_b5cf_f7a9_daa0;

    use std::sync::atomic::Ordering;
    let mut h = harness(true);
    h.shared().playing.store(true, Ordering::Relaxed);
    h.shared().monitoring.store(true, Ordering::Relaxed);
    h.shared().input_channels.store(IN_CH as u16, Ordering::Relaxed);
    h.shared().master_volume_bits.store(0.8f32.to_bits(), Ordering::Relaxed);
    // A loop that ends 40 frames into the fifth block.
    h.shared().loop_enabled.store(true, Ordering::Relaxed);
    h.shared().loop_in.store(64, Ordering::Relaxed);
    h.shared()
        .loop_out
        .store((BLOCK * 4 + 40) as u64, Ordering::Relaxed);

    let mut hash = BitHash::new();
    let mut heard = false;
    for block in 0..8u32 {
        h.push_monitor(&input_block(100 + block));
        let out = h.render().to_vec();
        heard |= out.iter().any(|&s| s != 0.0);
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
        hash.feed_pairs(&h.track_last_gains());
    }
    assert!(heard, "fixture must produce audio");

    // Block-aligned seam: `loop_out` falls exactly on a buffer boundary,
    // the case the end-of-block check's `>=` exists for (a strict `>`
    // would miss the seam on every cycle at pro-audio quanta).
    h.shared().playhead.store(0, Ordering::Relaxed);
    h.shared().loop_in.store(0, Ordering::Relaxed);
    h.shared()
        .loop_out
        .store((BLOCK * 3) as u64, Ordering::Relaxed);
    for block in 0..4u32 {
        h.push_monitor(&input_block(600 + block));
        let out = h.render().to_vec();
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
        hash.feed_pairs(&h.track_last_gains());
    }

    let got = hash.finish();
    assert_eq!(
        got, EXPECTED,
        "playing-branch output changed (got {got:#018x}) — mix_audio is no \
         longer bit-identical"
    );
}

/// The count-in branch: the playhead is pinned, the count-in-local
/// metronome fires, monitored input passes through and the master pass
/// still runs. Followed by the hand-off blocks where `count_in_remaining`
/// has reached zero but `count_in_active` is still set.
#[test]
fn count_in_blocks_are_bit_identical() {
    const EXPECTED: u64 = 0xcebe_edbe_ae66_c9ef;

    use std::sync::atomic::Ordering;
    let mut h = harness(true);
    h.shared().monitoring.store(true, Ordering::Relaxed);
    h.shared().input_channels.store(IN_CH as u16, Ordering::Relaxed);
    h.shared().count_in_active.store(true, Ordering::Relaxed);
    // Two bars of 4/4 at 132 bpm.
    let total = (SR as f64 * 60.0 / 132.0 * 8.0) as u64;
    h.shared().count_in_total.store(total, Ordering::Relaxed);
    h.shared().count_in_remaining.store(total, Ordering::Relaxed);
    h.shared().playhead.store(4 * BLOCK as u64, Ordering::Relaxed);

    let mut hash = BitHash::new();
    let mut heard = false;
    // Enough blocks to cross a beat boundary (and so fire a click), then a
    // jump to the tail of the count-in for the last-click / hand-off edge.
    for block in 0..6u32 {
        h.push_monitor(&input_block(200 + block));
        let out = h.render().to_vec();
        heard |= out.iter().any(|&s| s != 0.0);
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
        hash.feed_pairs(&h.track_last_gains());
    }
    h.shared()
        .count_in_remaining
        .store(BLOCK as u64 / 2, Ordering::Relaxed);
    for block in 0..3u32 {
        h.push_monitor(&input_block(300 + block));
        let out = h.render().to_vec();
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
        hash.feed_pairs(&h.track_last_gains());
    }
    assert!(heard, "count-in must produce clicks / monitoring");

    let got = hash.finish();
    assert_eq!(
        got, EXPECTED,
        "count-in output changed (got {got:#018x}) — mix_audio is no longer \
         bit-identical"
    );
}

/// The stopped branch: no transport, monitored input still reaches the
/// master through the pass-through + master-volume passes. Includes a
/// block with the ring starved (nothing pushed) and one with monitoring
/// switched off entirely.
#[test]
fn stopped_monitor_blocks_are_bit_identical() {
    const EXPECTED: u64 = 0xe5d8_6846_d53f_ee61;

    use std::sync::atomic::Ordering;
    let mut h = harness(false);
    h.shared().monitoring.store(true, Ordering::Relaxed);
    h.shared().input_channels.store(IN_CH as u16, Ordering::Relaxed);
    h.shared().playhead.store(9_000, Ordering::Relaxed);

    let mut hash = BitHash::new();
    let mut heard = false;
    for block in 0..4u32 {
        h.push_monitor(&input_block(400 + block));
        let out = h.render().to_vec();
        heard |= out.iter().any(|&s| s != 0.0);
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
        hash.feed_pairs(&h.track_last_gains());
    }
    // Ring pacing: pushing two blocks per callback grows the backlog, so
    // the whole-frame catch-up skip runs every block and — after
    // MONITOR_DRAIN_STREAK high cycles on the native backend — the
    // adaptive drain fires too.
    for block in 0..20u32 {
        h.push_monitor(&input_block(600 + block));
        h.push_monitor(&input_block(700 + block));
        let out = h.render().to_vec();
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
    }

    // Starved ring: a monitoring shortfall, counted and fed to the drain.
    let out = h.render().to_vec();
    hash.feed(&out);
    hash.feed_u64(&effects(&h));
    // Monitoring off: the branch exits without touching the output.
    h.shared().monitoring.store(false, Ordering::Relaxed);
    h.push_monitor(&input_block(500));
    let out = h.render().to_vec();
    hash.feed(&out);
    hash.feed_u64(&effects(&h));
    assert!(heard, "monitoring must produce audio");

    let got = hash.finish();
    assert_eq!(
        got, EXPECTED,
        "stopped-branch output changed (got {got:#018x}) — mix_audio is no \
         longer bit-identical"
    );
}

/// The reference A/B branch: the monitored source is a loaded reference,
/// so the whole output is replaced by its PCM (latency-matched against the
/// mix), the reference meter is fed and the transport keeps rolling.
/// Followed by the suppressed case (recording armed) which falls through
/// to the normal mix.
#[test]
fn reference_monitor_blocks_are_bit_identical() {
    // Re-blessed for FU-B6a: the feeder bus's first live block now renders
    // flat at its target gain instead of ramping in from the
    // construction-time 0 (0x062f_20fa_7cba_3b99 → this).
    const EXPECTED: u64 = 0x95f7_6315_fdcf_4bac;

    use std::sync::atomic::Ordering;
    let mut h = harness(false);
    h.enable_reference(noise(BLOCK * 9, 61));
    h.shared().playing.store(true, Ordering::Relaxed);
    h.shared().master_latency_samples.store(96, Ordering::Relaxed);

    let mut hash = BitHash::new();
    let mut heard = false;
    for _ in 0..4u32 {
        let out = h.render().to_vec();
        heard |= out.iter().any(|&s| s != 0.0);
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
    }
    // Recording suppresses the reference monitor: back to the real mix.
    h.shared().recording.store(true, Ordering::Relaxed);
    for _ in 0..2u32 {
        let out = h.render().to_vec();
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
    }
    assert!(heard, "reference monitor must produce audio");

    let got = hash.finish();
    assert_eq!(
        got, EXPECTED,
        "reference-branch output changed (got {got:#018x}) — mix_audio is no \
         longer bit-identical"
    );
}

/// The audition overlay: the audition preview is summed after the
/// arrangement, independent of transport, so it stays audible over a
/// playing arrangement that crosses a loop seam mid-block. Also drives the
/// live-MIDI pickup pass.
///
/// Until ARCH-02 B-6 the first five blocks were lock-contended (silent
/// arrangement, playhead advanced by the skip path); no block can be
/// skipped any more, so every block renders.
#[test]
fn audition_blocks_are_bit_identical() {
    // Captured on the pre-B-6 callback (skip path still present) with
    // this exact scenario, so deleting the skip path is pinned as a
    // no-op for rendered blocks. Replaces the contended scenario's
    // 0x40ea_d698_e423_5ccb.
    // Re-blessed for FU-B6a: the feeder bus's first live block now renders
    // flat at its target gain instead of ramping in from the
    // construction-time 0 (0xf1d5_aaa8_7cc5_cb25 → this).
    const EXPECTED: u64 = 0x01ec_d73b_9f26_15b6;

    use std::sync::atomic::Ordering;
    let mut h = harness(false);
    h.shared().playing.store(true, Ordering::Relaxed);
    h.shared().loop_enabled.store(true, Ordering::Relaxed);
    h.shared().loop_in.store(0, Ordering::Relaxed);
    h.shared()
        .loop_out
        .store((BLOCK * 3 + 17) as u64, Ordering::Relaxed);
    h.start_audition(noise(BLOCK * 5, 71), true);
    // The same overlay with the transport stopped: the audition alone.
    let mut alone = harness(false);
    alone.start_audition(noise(BLOCK * 5, 71), true);

    let mut hash = BitHash::new();
    let mut arrangement_heard = false;
    for _ in 0..8u32 {
        let out = h.render().to_vec();
        let overlay = alone.render();
        assert!(overlay.iter().any(|&s| s != 0.0), "the audition must be audible");
        assert!(out.iter().any(|&s| s != 0.0), "no block may be silent");
        arrangement_heard |= out != overlay;
        hash.feed(&out);
        hash.feed_u64(&effects(&h));
        hash.feed_pairs(&h.take_track_peaks());
        hash.feed_pairs(&h.track_last_gains());
    }
    assert!(arrangement_heard, "the arrangement must render under the overlay");
    assert!(
        h.shared().audition_pos_bits.load(Ordering::Relaxed) != 0,
        "the audition must have advanced"
    );

    let got = hash.finish();
    assert_eq!(
        got, EXPECTED,
        "audition output changed (got {got:#018x}) — mix_audio is no longer \
         bit-identical"
    );
}

/// The callback's published side effects as the goldens above were
/// captured: seven words, the fifth of which was the render-skip counter
/// (`render_skip_cycles`, deleted with the skip path in ARCH-02 B-6). It
/// was 0 on every branch these constants pin, so a literal 0 in its slot
/// keeps them valid unchanged.
fn effects(h: &MixAudioHarness) -> [u64; 7] {
    let [playhead, peak_l, peak_r, shortfalls, audition, reference] = h.side_effects();
    [playhead, peak_l, peak_r, shortfalls, 0, audition, reference]
}
