//! The algorithm switch (reverb-algorithms.md §4.2, phase R1): params,
//! crossfade, queueing, reset, legacy state.
//!
//! R1 has one algorithm, so the crossfade is driven on a `ReverbDsp`
//! whose bank holds two (or three) Classic engines: the switch logic is
//! the same code whichever engines sit in the slots, and R3 drops Plate
//! into it unchanged.

use std::f32::consts::TAU;

use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo};
use resonance_reverb::dsp::{Algorithm, ReverbDsp};
use resonance_reverb::params::{ALGORITHM_LABELS, ALGORITHM_LABELS_ALL, PARAM_COUNT};
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
/// `SWITCH_FADE_MS` at `SR`.
const FADE: usize = 2_400;

fn sine(n: usize) -> f32 {
    0.5 * (TAU * 220.0 * n as f32 / SR).sin()
}

/// A bank of `slots` Classic engines, configured the way the plugin's
/// block loop configures them, with its first sample not yet processed.
fn bank(slots: usize) -> ReverbDsp {
    let mut dsp = ReverbDsp::with_engines(SR, &vec![Algorithm::Classic; slots]);
    configure(&mut dsp);
    dsp
}

fn configure(dsp: &mut ReverbDsp) {
    dsp.set_size(0.5);
    dsp.set_decay(2.0);
    dsp.set_freeze(false);
    dsp.set_damping(8_000.0);
    dsp.set_predelay(12.0);
    dsp.set_er_level(0.5);
    dsp.set_er_time(0.5);
    dsp.set_mod_rate(1.0);
    dsp.set_mod_depth(0.3);
}

/// Feed the sine from sample `from` for `len` samples; the wet output,
/// interleaved L/R.
fn run(dsp: &mut ReverbDsp, from: usize, len: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(len * 2);
    for n in from..from + len {
        let x = sine(n);
        let (l, r) = dsp.process(x, x, 0.8, 1.0);
        out.push(l);
        out.push(r);
    }
    out
}

/// Largest sample-to-sample step on either channel.
fn max_step(interleaved: &[f32]) -> f32 {
    let mut worst = 0.0f32;
    for ch in 0..2 {
        let side: Vec<f32> = interleaved.iter().skip(ch).step_by(2).copied().collect();
        for w in side.windows(2) {
            worst = worst.max((w[1] - w[0]).abs());
        }
    }
    worst
}

#[test]
fn the_algorithm_param_offers_exactly_the_built_algorithms() {
    let plugin = ResonanceReverb::new();
    assert_eq!(PARAM_COUNT, 35);
    assert_eq!(plugin.param_count(), 35);
    let p = &plugin.params.algorithm;
    assert_eq!(p.id(), "algorithm");
    assert_eq!(p.choices(), Some(ALGORITHM_LABELS));
    assert_eq!(ALGORITHM_LABELS.len(), Algorithm::BUILT.len());
    assert_eq!(ALGORITHM_LABELS, &ALGORITHM_LABELS_ALL[..ALGORITHM_LABELS.len()]);
    assert_eq!(p.max_plain() as usize, Algorithm::BUILT.len() - 1);
    // The label round-trips through the host's text path, which is how
    // `track_set_plugin_param` resolves a choice label.
    assert_eq!(p.display(0.0), "Classic");
    assert_eq!(p.parse("Classic"), Some(0.0));
    // A newer build's index clamps onto the newest algorithm this build
    // has (the parameter's range clamps before the lookup sees it).
    p.set_plain(ALGORITHM_LABELS_ALL.len() as f64);
    assert_eq!(plugin.params.algorithm(), *Algorithm::BUILT.last().unwrap());

    // Params 22..=34, appended in the spec's order.
    let ids: Vec<&str> = (22..35).map(|i| plugin.param(i).id()).collect();
    assert_eq!(
        ids,
        [
            "algorithm",
            "low_decay_mult",
            "low_xover",
            "high_decay_mult",
            "predelay_sync",
            "decay_sync",
            "tail_build",
            "shimmer_pitch",
            "shimmer_amount",
            "nl_shape",
            "nl_length",
            "spring_tension",
            "spring_drip",
        ]
    );
    // Bass Decay sits at 1.0x in the middle of its dial.
    let mid = plugin.params.low_decay_mult.plain_at_normalized(0.5);
    assert!((mid - 1.0).abs() < 0.02, "Bass Decay mid-dial = {mid}");
}

#[test]
fn an_algorithm_switch_does_not_click() {
    // Warm both renders into a steady sustained sine, then measure the
    // same window with and without a switch at its start.
    let warm = 48_000;
    let window = FADE * 3;

    let mut steady = bank(2);
    run(&mut steady, 0, warm);
    let held = run(&mut steady, warm, window);

    let mut switched = bank(2);
    run(&mut switched, 0, warm);
    switched.set_engine_slot(1);
    assert!(switched.switching(), "the switch did not start a fade");
    let through = run(&mut switched, warm, window);
    assert!(!switched.switching(), "the fade outlived SWITCH_FADE_MS");
    assert_eq!(switched.engine_slot(), 1);

    let peak = through.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.05, "the switched render is near silent (peak {peak})");
    let (held_step, switch_step) = (max_step(&held), max_step(&through));
    assert!(
        switch_step <= held_step + 1e-3,
        "the switch clicks: max step {switch_step:.5} against {held_step:.5} without it"
    );
}

#[test]
fn a_switch_requested_mid_fade_is_queued_and_the_newest_wins() {
    let mut dsp = bank(3);
    run(&mut dsp, 0, 4_800);
    dsp.set_engine_slot(1);
    run(&mut dsp, 4_800, 100);
    // Two requests while 0 -> 1 fades: only the newest survives.
    dsp.set_engine_slot(2);
    dsp.set_engine_slot(0);
    assert_eq!(dsp.engine_slot(), 1, "a request mid-fade cut the fade short");
    assert!(dsp.switching());
    run(&mut dsp, 4_900, FADE - 100);
    // The first fade is done, and the queued one has started.
    assert_eq!(dsp.engine_slot(), 0, "the newest request (slot 0) did not follow");
    assert!(dsp.switching(), "the queued switch did not start");
    run(&mut dsp, 4_900 + FADE, FADE);
    assert!(!dsp.switching());
    assert_eq!(dsp.engine_slot(), 0);

    // A request for the slot already fading in is not queued again.
    dsp.set_engine_slot(1);
    run(&mut dsp, 4_900 + 2 * FADE, 10);
    dsp.set_engine_slot(2);
    dsp.set_engine_slot(1);
    run(&mut dsp, 4_910 + 2 * FADE, FADE);
    assert_eq!(dsp.engine_slot(), 1);
    assert!(!dsp.switching(), "a request back to the incoming slot queued a switch");
}

/// RMS of an interleaved stretch.
fn rms(interleaved: &[f32]) -> f32 {
    (interleaved.iter().map(|x| x * x).sum::<f32>() / interleaved.len() as f32).sqrt()
}

#[test]
fn a_switch_while_frozen_waits_for_the_release_and_keeps_the_tail() {
    let mut dsp = bank(2);
    run(&mut dsp, 0, 24_000);
    dsp.set_freeze(true);
    let held = rms(&run(&mut dsp, 24_000, 4_800));
    assert!(held > 1e-3, "nothing was frozen ({held})");

    dsp.set_engine_slot(1);
    assert_eq!(dsp.engine_slot(), 0, "a switch started while frozen");
    assert!(!dsp.switching());
    let after = rms(&run(&mut dsp, 28_800, 4 * FADE));
    assert!(
        after > 0.5 * held,
        "the frozen tail did not survive a switch request: {held} -> {after}"
    );

    dsp.set_freeze(false);
    assert!(dsp.switching(), "releasing Freeze did not start the deferred switch");
    run(&mut dsp, 28_800 + 4 * FADE, FADE);
    assert_eq!(dsp.engine_slot(), 1);
    assert!(!dsp.switching());
}

/// Spring and Nonlinear ignore Freeze (§4.2): with Freeze on, a switch
/// between them has no held tail to keep and starts at once, while a
/// switch to an engine that honours Freeze still waits for the release,
/// and a newer request back to the active engine cancels it.
#[test]
fn freeze_defers_only_switches_that_involve_a_freezing_engine() {
    let algorithms = [Algorithm::Spring, Algorithm::Nonlinear, Algorithm::Hall];
    let mut dsp = ReverbDsp::with_engines(SR, &algorithms);
    configure(&mut dsp);
    run(&mut dsp, 0, 4_800);
    dsp.set_freeze(true);
    run(&mut dsp, 4_800, 480);

    dsp.set_engine_slot(1);
    assert!(dsp.switching(), "Spring -> Nonlinear waited for a Freeze neither honours");
    run(&mut dsp, 5_280, FADE);
    assert_eq!(dsp.engine_slot(), 1);

    dsp.set_engine_slot(2);
    assert_eq!(dsp.engine_slot(), 1, "a switch into a freezing engine started while frozen");
    assert!(!dsp.switching());
    dsp.set_engine_slot(1);
    dsp.set_freeze(false);
    assert!(!dsp.switching(), "a superseded deferred request started on release");
    assert_eq!(dsp.engine_slot(), 1);
}

#[test]
fn before_the_first_sample_a_switch_snaps() {
    let mut dsp = bank(2);
    dsp.set_engine_slot(1);
    assert_eq!(dsp.engine_slot(), 1);
    assert!(!dsp.switching(), "a fresh processor faded from nothing");
}

#[test]
fn a_reset_mid_switch_lands_on_the_newest_request_like_a_fresh_processor() {
    let mut reused = bank(2);
    run(&mut reused, 0, 9_000);
    reused.set_engine_slot(1);
    run(&mut reused, 9_000, 700);
    assert!(reused.switching());
    reused.clear();
    assert!(!reused.switching());
    assert_eq!(reused.engine_slot(), 1);
    let after = run(&mut reused, 0, 24_000);

    let mut fresh = bank(2);
    let want = run(&mut fresh, 0, 24_000);
    let diff = after.iter().zip(&want).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(diff, None, "reset mid-switch differs from fresh at {diff:?}");
}

// ---------------------------------------------------------------------------
// Through the plugin
// ---------------------------------------------------------------------------

const BLOCK: usize = 256;

fn tempo(bpm: f32) -> Option<TempoInfo> {
    Some(TempoInfo {
        bpm,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    })
}

/// Every new parameter off its default (the switch-relevant ones, the
/// inert decay shape, both syncs).
fn new_params_moved(plugin: &ResonanceReverb) {
    let p = &plugin.params;
    p.low_decay_mult.set_value(1.7);
    p.low_xover.set_value(400.0);
    p.high_decay_mult.set_value(0.3);
    p.predelay_sync.set_value(2);
    p.decay_sync.set_value(3);
    p.build.set_value(0.8);
    p.mod_depth.set_value(0.8);
    p.mix.set_value(1.0);
}

fn render(plugin: &mut ResonanceReverb, blocks: usize) -> Vec<f32> {
    let mut out = Vec::new();
    let mut n = 0usize;
    for _ in 0..blocks {
        let mut left: Vec<f32> = (n..n + BLOCK)
            .map(|i| if i < 720 { sine(i) } else { 0.0 })
            .collect();
        let mut right = left.clone();
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, BLOCK, &mut EventIterator::empty(), tempo(132.0));
        n += BLOCK;
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

#[test]
fn a_reset_plugin_with_the_new_params_renders_like_a_fresh_one() {
    let fresh = || {
        let mut plugin = ResonanceReverb::new();
        new_params_moved(&plugin);
        plugin.initialize(SR, BLOCK as u32);
        plugin
    };
    let mut reused = fresh();
    let _ = render(&mut reused, 7);
    reused.reset();
    let after = render(&mut reused, 64);

    let want = render(&mut fresh(), 64);
    let peak = want.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 1e-3, "the reference render is silent");
    let diff = after.iter().zip(&want).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(diff, None, "reset differs from fresh at {diff:?}");
}

#[test]
fn legacy_state_loads_as_classic_with_sync_off() {
    // A project saved before the algorithm work: no `algorithm`, no
    // syncs, no decay shape.
    const LEGACY: &str = r#"{"version":1,"params":{
        "predelay":22.0,"er_level":0.55,"er_time":0.35,"size":0.42,"decay":3.1,
        "damping":6500.0,"diffusion":0.7,"mod_rate":1.6,"mod_depth":0.25,
        "width":0.8,"mix":0.45,"freeze":0.0,"er_tail_balance":0.0}}"#;
    // A fresh instance, as a project reload builds one (the shared loader
    // writes only the ids a state names; the defaults supply the rest).
    let mut plugin = ResonanceReverb::new();
    assert!(plugin.load_state(LEGACY.as_bytes()));
    let p = &plugin.params;
    assert_eq!(p.algorithm(), Algorithm::Classic);
    assert_eq!(p.algorithm.value(), 0);
    assert_eq!(p.predelay_sync.value(), 0, "a legacy state left pre-delay synced");
    assert_eq!(p.decay_sync.value(), 0, "a legacy state left decay synced");
    assert_eq!(p.low_decay_mult.value(), 1.0);
    assert_eq!(p.low_xover.value(), 250.0);
    assert_eq!(p.high_decay_mult.value(), 0.5);
    assert_eq!(p.build.value(), 0.5);
    assert_eq!(p.predelay.value(), 22.0);

    // A state naming an algorithm this build does not have yet loads
    // (clamped onto the newest algorithm this build has) rather than
    // failing.
    const FUTURE: &str = r#"{"version":1,"params":{"algorithm":99.0}}"#;
    let mut plugin = ResonanceReverb::new();
    assert!(plugin.load_state(FUTURE.as_bytes()));
    assert_eq!(plugin.params.algorithm(), *Algorithm::BUILT.last().unwrap());
}

#[cfg(feature = "editor")]
#[test]
fn the_editor_offers_the_algorithm_and_greys_what_classic_ignores() {
    use plugin_gui_core::egui;
    use resonance_reverb::editor::headless_editor;

    let plugin = ResonanceReverb::new();
    // The window's minimum size: nothing may fall off it.
    let mut editor = headless_editor(&plugin, (720.0, 680.0));
    let frame = editor.settled();
    for id in [
        "algorithm",
        "predelay_sync",
        "decay_sync",
        "low_decay_mult",
        "low_xover",
        "high_decay_mult",
        "tail_build",
        "shimmer_pitch",
        "shimmer_amount",
        "nl_shape",
        "nl_length",
        "spring_tension",
        "spring_drip",
    ] {
        assert!(frame.widget(id).is_some(), "`{id}` is not drawn");
    }

    // Greyed, not hidden: a drag on Bass Decay under Classic does nothing.
    let mut editor = headless_editor(&plugin, (1320.0, 780.0));
    let frame = editor.settled();
    let r = frame.widget("low_decay_mult").unwrap().rect;
    let from = egui::pos2(r.center().x, r.top() + 20.0);
    editor.drag(from, from - egui::vec2(0.0, 40.0), 20);
    assert_eq!(plugin.params.low_decay_mult.value(), 1.0);
    assert!(editor.announced().is_empty(), "a greyed knob announced an edit");
    // The live knob next to it still works, so the drag itself was real.
    let r = frame.widget("mix").unwrap().rect;
    let from = egui::pos2(r.center().x, r.top() + 20.0);
    editor.drag(from, from - egui::vec2(0.0, 40.0), 20);
    assert_eq!(editor.announced(), ["mix"]);
}
