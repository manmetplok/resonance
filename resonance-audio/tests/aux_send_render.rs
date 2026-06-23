//! Aux-send tap & sum in the shared mix loop (`render_block`, todo #476).
//!
//! Each enabled send taps its source track/bus signal — pre-fader (raw,
//! send level only) or post-fader (after the source's fader/pan) — scales
//! it by the send level, and sums it into the destination return bus, on
//! top of the source's normal output. These tests render one offline
//! (Bounce) block on DC clips so every level is exactly predictable; the
//! Bounce path shares `render_block` with the live mixer, so asserting it
//! here also pins the "bounce renders identically to live" guarantee.
//!
//! `render_aux_for_test` returns the interleaved master plus the per-bus
//! summing buffers. After the render a bus buffer still holds that bus's
//! signal *before* its own fader, which for a return bus is exactly the
//! accumulated aux-send contribution — so the tap can be asserted in
//! isolation from the return bus's own fader/pan.

use resonance_audio::__test_support::render_aux_for_test;
use resonance_audio::types::*;

const FRAMES: usize = 48;
const SR: u32 = 48_000;

/// Track/bus gain at unity volume and centre pan: the constant-power pan
/// law attenuates the centre by −3 dB on each channel (`cos(pi/4)`), the
/// same value `track_stereo_gains`/`bus_stereo_gains` apply.
fn center_gain() -> f32 {
    (std::f32::consts::PI * 0.25).cos()
}

fn db_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// A track that mixes a constant `1.0` DC clip across the whole block, so
/// its post-plugin (pre-fader) buffer is exactly `1.0` per sample.
fn dc_track(id: TrackId, output: TrackOutput) -> (Track, AudioClip) {
    let track = Track::new(id, format!("t{id}"));
    track.set_output(output);
    let clip = AudioClip {
        id,
        track_id: id,
        start_sample: 0,
        source: ClipSource::Memory(vec![1.0; FRAMES * 2]),
        name: "dc".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
    };
    (track, clip)
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

const RETURN_BUS: BusId = 10;

/// Two tracks post-fader-send into one return bus at different levels:
/// the return buffer holds the level-scaled sum of both, and the dry
/// master is unchanged (the sends are *parallel*, added on top).
#[test]
fn two_tracks_post_fader_sum_into_return_with_dry_intact() {
    let g = center_gain();
    let lvl1 = -6.0f32;
    let lvl2 = -12.0f32;

    // Baseline render with no sends: pure dry mix. (`AudioClip` isn't
    // `Clone`, so each render builds its own tracks + clips.)
    let (t1a, c1a) = dc_track(1, TrackOutput::Master);
    let (t2a, c2a) = dc_track(2, TrackOutput::Master);
    let (dry_master, dry_bufs) = render_aux_for_test(
        vec![t1a, t2a],
        vec![Bus::new(RETURN_BUS, "Reverb".into())],
        vec![c1a, c2a],
        vec![],
        FRAMES,
        SR,
    );
    // No sends → the return bus stays silent.
    assert!(dry_bufs[0].0.iter().all(|&s| s == 0.0));

    // Render with both sends active.
    let (t1, c1) = dc_track(1, TrackOutput::Master);
    let (t2, c2) = dc_track(2, TrackOutput::Master);
    let (master, bufs) = render_aux_for_test(
        vec![t1, t2],
        vec![Bus::new(RETURN_BUS, "Reverb".into())],
        vec![c1, c2],
        vec![
            aux(1, SendSource::Track(1), RETURN_BUS, lvl1, false),
            aux(2, SendSource::Track(2), RETURN_BUS, lvl2, false),
        ],
        FRAMES,
        SR,
    );

    // The return buffer is the post-fader sum: each track's DC (1.0) times
    // its fader/pan gain `g` times the send's linear level.
    let expected_return = g * db_lin(lvl1) + g * db_lin(lvl2);
    for f in 0..FRAMES {
        assert!(
            (bufs[0].0[f] - expected_return).abs() < 1e-5,
            "return L frame {f}: {} != {expected_return}",
            bufs[0].0[f]
        );
        assert!((bufs[0].1[f] - expected_return).abs() < 1e-5);
    }

    // Dry signal intact: the only change at the master is the return bus's
    // own contribution (its accumulated sends through its fader `g`). The
    // direct track paths are untouched.
    for f in 0..FRAMES {
        let delta = master[f * 2] - dry_master[f * 2];
        assert!(
            (delta - expected_return * g).abs() < 1e-5,
            "master delta frame {f}: {delta} != {}",
            expected_return * g
        );
    }
}

/// A pre-fader send ignores the source fader/pan (send level only); a
/// post-fader send follows them. With the fader pulled to 0.5 the two
/// taps differ by exactly the fader/pan gain.
#[test]
fn pre_fader_send_ignores_fader_post_fader_follows_it() {
    let lvl = 0.0f32; // unity send
    let g = center_gain();

    // Post-fader: track at 0.5 → tap is DC * (g * 0.5).
    let (tp, cp) = dc_track(1, TrackOutput::Master);
    tp.set_volume(0.5);
    let (_m, post_bufs) = render_aux_for_test(
        vec![tp],
        vec![Bus::new(RETURN_BUS, "R".into())],
        vec![cp],
        vec![aux(1, SendSource::Track(1), RETURN_BUS, lvl, false)],
        FRAMES,
        SR,
    );
    let expected_post = g * 0.5;
    assert!((post_bufs[0].0[0] - expected_post).abs() < 1e-5);

    // Pre-fader: the same 0.5 fader is bypassed → tap is the raw DC (1.0).
    let (tpre, cpre) = dc_track(1, TrackOutput::Master);
    tpre.set_volume(0.5);
    let (_m, pre_bufs) = render_aux_for_test(
        vec![tpre],
        vec![Bus::new(RETURN_BUS, "R".into())],
        vec![cpre],
        vec![aux(1, SendSource::Track(1), RETURN_BUS, lvl, true)],
        FRAMES,
        SR,
    );
    assert!((pre_bufs[0].0[0] - 1.0).abs() < 1e-5, "pre-fader tap should be the raw signal");
    assert!(
        pre_bufs[0].0[0] > post_bufs[0].0[0],
        "pre-fader send must exceed the fader-attenuated post-fader send"
    );
}

/// A disabled send contributes nothing, even with a valid route.
#[test]
fn disabled_send_is_silent() {
    let (t, c) = dc_track(1, TrackOutput::Master);
    let mut send = aux(1, SendSource::Track(1), RETURN_BUS, 0.0, false);
    send.enabled = false;
    let (_m, bufs) = render_aux_for_test(
        vec![t],
        vec![Bus::new(RETURN_BUS, "R".into())],
        vec![c],
        vec![send],
        FRAMES,
        SR,
    );
    assert!(bufs[0].0.iter().all(|&s| s == 0.0), "disabled send must not feed the return");
}

/// A bus can feed another (later-ordered) return bus: a track routed into
/// bus A, which post-fader-sends into return bus B, lands the
/// twice-gained signal in B's buffer.
#[test]
fn bus_to_bus_send_taps_post_fader() {
    const BUS_A: BusId = 5;
    const BUS_B: BusId = 10;
    let g = center_gain();

    // Track 1 → bus A; bus A → bus B (unity send, post-fader).
    let (t, c) = dc_track(1, TrackOutput::Bus(BUS_A));
    let (_m, bufs) = render_aux_for_test(
        vec![t],
        vec![Bus::new(BUS_A, "A".into()), Bus::new(BUS_B, "B".into())],
        vec![c],
        vec![aux(1, SendSource::Bus(BUS_A), BUS_B, 0.0, false)],
        FRAMES,
        SR,
    );

    // Bus A buffer = track DC (1.0) routed post-fader = g.
    assert!((bufs[0].0[0] - g).abs() < 1e-5, "bus A should hold the routed track signal");
    // Bus B buffer = bus A's post-fader signal (g) times A's fader (g).
    let expected_b = g * g;
    assert!(
        (bufs[1].0[0] - expected_b).abs() < 1e-5,
        "bus B should hold A's post-fader send: {} != {expected_b}",
        bufs[1].0[0]
    );
}
