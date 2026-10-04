//! A latency change during playback is a time shift, not a dropout
//! (code review RT-04).
//!
//! Every latency-affecting edit — bypassing a latent chain, a preset that
//! changes a plugin's latency, adding one — republishes the compensation
//! table. It used to publish a table of empty delay lines: every
//! compensated track and bus went silent for its delay (43 ms behind a
//! 2048-sample plugin), including the ones whose delay had not changed.
//! `LatencyComp::following` carries the lines over instead:
//!
//! * an unchanged delay keeps its line — the output is bit-identical to
//!   a table that was never republished;
//! * a changed delay crossfades from the old tap to the new one over the
//!   transition, both reading the same history — no zero run, no step;
//! * a delay that falls to 0 keeps its line as a pass-through, so the
//!   way back replays real audio rather than silence;
//! * a delay that outgrows its line hands its history to the new one.
//!
//! The first half drives `LatencyComp` directly; the second runs the real
//! `SetTrackFxBypass` handler and the engine's `refresh_latency_comp`
//! against a latent fake plugin, rendering through the real callback.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::Ordering;

use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    __instance_from_raw_for_test, fade_frames, EngineHandlerHarness, LatencyComp, MixAudioHarness,
    PluginSlot,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;

/// A strictly positive test signal (0.4 DC + a 0.3 sine): a dropout
/// shows up as samples near zero, which the signal itself never has.
fn signal(n: usize) -> f32 {
    0.4 + 0.3 * (n as f32 * 2.0 * std::f32::consts::PI * 220.0 / SR as f32).sin()
}

/// The largest sample-to-sample step.
fn max_step(xs: &[f32]) -> f32 {
    xs.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

/// The longest run of samples below `floor`.
fn longest_run_below(xs: &[f32], floor: f32) -> usize {
    let (mut best, mut cur) = (0, 0);
    for &x in xs {
        if x.abs() < floor {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Stream `blocks` blocks of [`signal`] through track `id` of `comp`,
/// from timeline frame `*pos`, appending the left channel to `out`.
fn stream(comp: &LatencyComp, id: TrackId, pos: &mut usize, blocks: usize, out: &mut Vec<f32>) {
    for _ in 0..blocks {
        let mut l: Vec<f32> = (*pos..*pos + BLOCK).map(signal).collect();
        let mut r = l.clone();
        comp.apply(id, &mut l, &mut r, *pos as u64);
        assert_eq!(l, r);
        out.extend_from_slice(&l);
        *pos += BLOCK;
    }
}

// ---------------------------------------------------------------------------
// LatencyComp::following
// ---------------------------------------------------------------------------

#[test]
fn an_unchanged_delay_plays_straight_through_a_republish() {
    // Track 2's delay stays 300 while track 1's moves 0 → 300 (its latent
    // chain was bypassed, say): the table is republished, and track 2
    // must not notice.
    let xfade = fade_frames(SR) as usize;
    let reference = LatencyComp::new(300, &[(1, 0), (2, 300)], 0, &[]);
    let (mut pos, mut expected) = (0, Vec::new());
    stream(&reference, 2, &mut pos, 40, &mut expected);

    let first = LatencyComp::new(300, &[(1, 0), (2, 300)], 0, &[]);
    let (mut pos, mut got) = (0, Vec::new());
    stream(&first, 2, &mut pos, 10, &mut got);
    let second = LatencyComp::following(&first, 300, &[(1, 300), (2, 300)], 0, &[], xfade);
    assert_eq!(second.delay_for(1), 300);
    stream(&second, 2, &mut pos, 30, &mut got);

    assert_eq!(got, expected, "an unchanged delay line must survive the republish bit-for-bit");
}

#[test]
fn a_changed_delay_crossfades_without_a_dropout_and_comes_back() {
    let xfade = fade_frames(SR) as usize;
    let l = 1024u64;
    let first = LatencyComp::new(l, &[(1, 0), (2, l)], 0, &[]);
    let (mut pos, mut out) = (0, Vec::new());
    stream(&first, 2, &mut pos, 20, &mut out);
    let settled = out.len();

    // Track 1's latent chain is bypassed: track 2 no longer waits for it.
    let bypassed = LatencyComp::following(&first, 0, &[(1, 0), (2, 0)], 0, &[], xfade);
    stream(&bypassed, 2, &mut pos, 20, &mut out);
    let switched_back = out.len();
    // ... and re-engaged: the 0-delay pass-through kept the history.
    let engaged = LatencyComp::following(&bypassed, l, &[(1, 0), (2, l)], 0, &[], xfade);
    stream(&engaged, 2, &mut pos, 20, &mut out);

    let steady = &out[l as usize + 1..settled];
    let steady_step = max_step(steady);
    let tail = &out[settled - 1..];
    assert_eq!(longest_run_below(tail, 0.05), 0, "a delay change must never drop out");
    let got = max_step(tail);
    assert!(
        got <= 2.0 * steady_step,
        "a delay change must not step: {got} vs the signal's own {steady_step}"
    );

    // Each change really happened: after the fade the output is the
    // input at the new delay, exactly.
    let at = |n: usize| out[n];
    for n in settled + xfade..switched_back {
        assert_eq!(at(n), signal(n), "bypassed: undelayed at {n}");
    }
    for n in switched_back + xfade..out.len() {
        assert_eq!(at(n), signal(n - l as usize), "re-engaged: delayed by {l} at {n}");
    }
}

#[test]
fn a_line_that_outgrows_its_capacity_keeps_its_history() {
    // 100 → 130 needs a bigger line (128 → 256 frames). With no
    // transition the switch is immediate, so what the new line reads is
    // exactly what it adopted.
    let first = LatencyComp::new(100, &[(1, 100)], 0, &[]);
    let (mut pos, mut out) = (0, Vec::new());
    stream(&first, 1, &mut pos, 10, &mut out);
    let switch = out.len();
    let grown = LatencyComp::following(&first, 130, &[(1, 130)], 0, &[], 0);
    stream(&grown, 1, &mut pos, 4, &mut out);

    // The old line held the last 128 frames; reading 130 back needs 131,
    // so only the first two frames after the switch are short.
    let zeros = out[switch..].iter().take_while(|&&s| s == 0.0).count();
    assert!(zeros <= 2, "{zeros} frames of silence after growing the line");
    for n in switch + zeros..out.len() {
        assert_eq!(out[n], signal(n - 130), "grown line plays history at {n}");
    }
}

#[test]
fn delays_match_ignores_retained_pass_through_lines() {
    let first = LatencyComp::new(64, &[(1, 0), (2, 64)], 0, &[]);
    let second = LatencyComp::following(&first, 0, &[(1, 0), (2, 0)], 0, &[], 240);
    assert!(second.is_empty(), "a pass-through line delays nothing");
    assert_eq!(second.delay_for(2), 0);
    assert!(second.delays_match(0, &[(1, 0), (2, 0)], 0, &[]));
    assert!(!second.delays_match(64, &[(1, 0), (2, 64)], 0, &[]));
}

// ---------------------------------------------------------------------------
// End to end: SetTrackFxBypass on a latent chain, real handler + callback
// ---------------------------------------------------------------------------

/// Latency the fake reports, in frames.
const LATENT: u32 = 1024;

unsafe extern "C" fn ok(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn noop(_p: *const clap_plugin) {}
unsafe extern "C" fn activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}
/// Silence in, silence out — the tests only need the latency it reports.
unsafe extern "C" fn process(_p: *const clap_plugin, _pr: *const clap_process) -> clap_process_status {
    CLAP_PROCESS_CONTINUE
}
unsafe extern "C" fn latency_get(_p: *const clap_plugin) -> u32 {
    LATENT
}
static LATENCY_EXT: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};
unsafe extern "C" fn get_extension(_p: *const clap_plugin, id: *const std::ffi::c_char) -> *const c_void {
    if unsafe { std::ffi::CStr::from_ptr(id) }.to_bytes() == CLAP_EXT_LATENCY.to_bytes() {
        &LATENCY_EXT as *const clap_plugin_latency as *const c_void
    } else {
        ptr::null()
    }
}

fn latent_fx() -> PluginSlot {
    let inst = __instance_from_raw_for_test(
        |_host| {
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: ptr::null_mut(),
                init: Some(ok),
                destroy: Some(noop),
                activate: Some(activate),
                deactivate: Some(noop),
                start_processing: Some(ok),
                stop_processing: Some(noop),
                reset: Some(noop),
                process: Some(process),
                get_extension: Some(get_extension),
                on_main_thread: None,
            })) as *const clap_plugin
        },
        SR,
    )
    .expect("fake latent plugin");
    assert_eq!(inst.latency_samples(), LATENT);
    PluginSlot::new(inst)
}

const FRAMES: usize = 64 * BLOCK;

fn signal_clip(id: ClipId, track_id: TrackId) -> AudioClip {
    let data: Vec<f32> = (0..FRAMES + 4096).flat_map(|i| [signal(i), signal(i)]).collect();
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(data),
        name: "signal".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// Tracks `latent` (each with its own latent fake, no audio) plus track
/// 2 playing [`signal`], all to master. Returns the engine and a callback
/// rendering through what the engine publishes.
fn project(latent: &[(TrackId, PluginInstanceId)]) -> (EngineHandlerHarness, MixAudioHarness) {
    let mut h = EngineHandlerHarness::new();
    for &(track_id, plugin_id) in latent {
        let track = Track::new(track_id, "latent".into());
        track.push_plugin(plugin_id);
        h.push_track(track);
        h.insert_plugin_slot(plugin_id, latent_fx());
    }
    h.push_track(Track::new(2, "signal".into()));
    h.push_clip(signal_clip(1, 2));
    let shared = h.shared_arc();
    shared.master_volume_bits.store(1.0f32.to_bits(), Ordering::Relaxed);
    shared.playing.store(true, Ordering::Relaxed);
    h.refresh_latency_comp();
    let cb = MixAudioHarness::on_shared(shared, BLOCK, 2, SR);
    cb.adopt_latency_comp(h.published_latency_comp());
    (h, cb)
}

/// Bypass (or re-engage) `track_id`'s chain the way the engine loop does:
/// the handler, then the comp republish.
fn set_chain_bypass(h: &mut EngineHandlerHarness, cb: &MixAudioHarness, track_id: TrackId, bypassed: bool) {
    h.dispatch(AudioCommand::SetTrackFxBypass { track_id, bypassed });
    h.refresh_latency_comp();
    cb.adopt_latency_comp(h.published_latency_comp());
}

fn left(cb: &mut MixAudioHarness) -> Vec<f32> {
    cb.render().chunks(2).map(|f| f[0]).collect()
}

#[test]
fn bypassing_a_latent_chain_mid_play_never_silences_the_other_tracks() {
    let (mut h, mut cb) = project(&[(1, 501)]);
    assert_eq!(h.published_latency_comp().delay_for(2), LATENT as u64);

    let mut out = Vec::new();
    for block in 0..48 {
        match block {
            16 => set_chain_bypass(&mut h, &cb, 1, true),
            32 => set_chain_bypass(&mut h, &cb, 1, false),
            _ => {}
        }
        out.extend(left(&mut cb));
    }
    assert_eq!(h.published_latency_comp().delay_for(2), LATENT as u64, "re-engaged");

    // Past the warm-up from Play (the line fills over LATENT frames).
    let settled = &out[LATENT as usize + BLOCK..16 * BLOCK];
    let floor = settled.iter().copied().fold(f32::MAX, f32::min);
    assert!(floor > 0.05, "sanity: the steady signal is well clear of zero ({floor})");
    let steady_step = max_step(settled);

    let after = &out[16 * BLOCK - 1..];
    let run = longest_run_below(after, floor * 0.5);
    let fade = fade_frames(SR) as usize;
    assert!(
        run == 0,
        "track 2 dropped out for {run} frames across the bypass toggles (fade is {fade})"
    );
    let got = max_step(after);
    assert!(
        got <= 2.0 * steady_step,
        "the realignment must not click: step {got} vs the signal's own {steady_step}"
    );
}

#[test]
fn a_track_whose_delay_is_unchanged_is_untouched_by_the_republish() {
    // Tracks 1 and 3 both carry LATENT; bypassing 1 leaves the maximum —
    // and so track 2's delay — where it was, while track 1 itself now
    // needs a delay line. Track 2 must render exactly as if nothing
    // happened.
    let render = |toggle: bool| {
        let (mut h, mut cb) = project(&[(1, 501), (3, 503)]);
        let mut out = Vec::new();
        for block in 0..40 {
            if toggle && block == 16 {
                set_chain_bypass(&mut h, &cb, 1, true);
                let comp = h.published_latency_comp();
                assert_eq!(comp.delay_for(1), LATENT as u64, "the table did change");
                assert_eq!(comp.delay_for(2), LATENT as u64);
            }
            out.extend(left(&mut cb));
        }
        out
    };
    let untouched = render(false);
    assert!(untouched[2 * LATENT as usize..].iter().all(|&s| s > 0.05), "not vacuous");
    assert_eq!(render(true), untouched);
}
