//! Tempo-synced pre-delay and decay (reverb-algorithms.md §4.5, phase
//! R1), on Classic.
//!
//! Pre-delay is measured, not computed: the wet onset of an impulse with
//! sync on, against the onset of the same plugin at a 0 ms knob. Every
//! wet path sits behind the pre-delay, so the two renders are the same
//! signal shifted by exactly the synced tap. Decay is proven by render
//! equality: a synced decay must render bit-identically to the knob set
//! to the value the sync should have produced.

use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin, TempoInfo};
use resonance_reverb::params::ReverbParams;
use resonance_reverb::sync::{decay_s, predelay_ms, DECAY_SYNC_LABELS, PREDELAY_SYNC_LABELS};
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn tempo_sig(bpm: f32, num: u16, den: u16) -> Option<TempoInfo> {
    Some(TempoInfo {
        bpm,
        time_sig_num: num,
        time_sig_den: den,
        playing: true,
        song_pos_beats: 0.0,
    })
}

fn tempo(bpm: f32) -> Option<TempoInfo> {
    tempo_sig(bpm, 4, 4)
}

fn label_index(labels: &[&str], label: &str) -> i32 {
    labels.iter().position(|l| *l == label).unwrap() as i32
}

/// A plugin, fully wet, set up by `setup` before `initialize`.
fn plugin(setup: impl Fn(&ReverbParams)) -> ResonanceReverb {
    let mut plugin = ResonanceReverb::new();
    plugin.params.mix.set_value(1.0);
    setup(&plugin.params);
    plugin.initialize(SR, BLOCK as u32);
    plugin
}

/// An impulse at sample 0, then silence; the left output.
fn impulse(plugin: &mut ResonanceReverb, blocks: usize, tempo: Option<TempoInfo>) -> Vec<f32> {
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for b in 0..blocks {
        let mut left = vec![0.0f32; BLOCK];
        let mut right = vec![0.0f32; BLOCK];
        if b == 0 {
            left[0] = 1.0;
            right[0] = 1.0;
        }
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, BLOCK, &mut EventIterator::empty(), tempo);
        out.extend_from_slice(&left);
    }
    out
}

fn onset(render: &[f32]) -> usize {
    render
        .iter()
        .position(|x| x.abs() > 1e-9)
        .expect("the render never left silence")
}

/// Onset of the wet response at a 0 ms pre-delay knob, sync off.
fn reference_onset() -> usize {
    let mut p = plugin(|p| p.predelay.set_value(0.0));
    onset(&impulse(&mut p, 40, None))
}

#[test]
fn synced_predelay_lands_on_the_note_at_60_120_and_170_bpm() {
    let base = reference_onset();
    let sixteenth = label_index(PREDELAY_SYNC_LABELS, "1/16");
    for bpm in [60.0f32, 120.0, 170.0] {
        // The knob is far off on purpose: the sync must override it.
        let mut p = plugin(|p| {
            p.predelay.set_value(200.0);
            p.predelay_sync.set_value(sixteenth);
        });
        let got = onset(&impulse(&mut p, 120, tempo(bpm))) - base;
        // A sixteenth is a quarter of a beat.
        let want = 60.0 / bpm / 4.0 * SR;
        assert!(
            (got as f32 - want).abs() <= 1.0,
            "1/16 at {bpm} BPM: pre-delay {got} samples, want {want:.1}"
        );
    }
}

#[test]
fn every_predelay_note_value_at_120_bpm() {
    let base = reference_onset();
    // Beat = 500 ms at 120 BPM; the note value in beats is 4 / denominator.
    for (label, want_ms) in [("1/128", 15.625f32), ("1/64", 31.25), ("1/32", 62.5), ("1/8", 250.0)] {
        let idx = label_index(PREDELAY_SYNC_LABELS, label);
        assert_eq!(predelay_ms(idx, tempo(120.0)), Some(want_ms), "{label}");
        let mut p = plugin(|p| p.predelay_sync.set_value(idx));
        let got = onset(&impulse(&mut p, 100, tempo(120.0))) - base;
        let want = want_ms * 0.001 * SR;
        assert!(
            (got as f32 - want).abs() <= 1.0,
            "{label} at 120 BPM: {got} samples, want {want:.1}"
        );
    }
}

#[test]
fn predelay_sync_falls_back_to_the_knob_without_tempo() {
    let base = reference_onset();
    let mut p = plugin(|p| {
        p.predelay.set_value(40.0);
        p.predelay_sync.set_value(label_index(PREDELAY_SYNC_LABELS, "1/16"));
    });
    let got = onset(&impulse(&mut p, 40, None)) - base;
    assert_eq!(got, 1_920, "no tempo: the 40 ms knob must rule");
    // A tempo of 0 (a host with no transport) is no tempo either.
    assert_eq!(predelay_ms(4, tempo(0.0)), None);
    assert_eq!(predelay_ms(0, tempo(120.0)), None, "Off is not synced");
}

#[test]
fn decay_sync_sets_the_t60_from_the_tempo_and_meter() {
    let idx = |l| label_index(DECAY_SYNC_LABELS, l);
    // 120 BPM: a beat is 0.5 s.
    let t = tempo(120.0);
    assert_eq!(decay_s(idx("Off"), t), None);
    assert_eq!(decay_s(idx("1/4"), t), Some(0.5));
    assert_eq!(decay_s(idx("1/2"), t), Some(1.0));
    assert_eq!(decay_s(idx("1 bar"), t), Some(2.0));
    assert_eq!(decay_s(idx("2 bars"), t), Some(4.0));
    assert_eq!(decay_s(idx("4 bars"), t), Some(8.0));
    // The bar follows the host's meter: 3/4 at 60 BPM is 3 s, 6/8 at
    // 120 BPM is three quarter notes, 1.5 s.
    assert_eq!(decay_s(idx("1 bar"), tempo_sig(60.0, 3, 4)), Some(3.0));
    assert_eq!(decay_s(idx("1 bar"), tempo_sig(120.0, 6, 8)), Some(1.5));
    // A host that reports no meter gets four beats.
    assert_eq!(decay_s(idx("1 bar"), tempo_sig(120.0, 0, 0)), Some(2.0));
    // Held inside the `decay` param's range.
    assert_eq!(decay_s(idx("4 bars"), tempo(20.0)), Some(30.0));
    assert_eq!(decay_s(idx("1/4"), tempo(900.0)), Some(0.1));
}

#[test]
fn a_synced_decay_renders_exactly_like_the_knob_at_that_t60() {
    // 1 bar of 4/4 at 120 BPM is 2.0 s. The synced plugin's knob is far
    // away (9 s) so a fallback would show.
    let mut synced = plugin(|p| {
        p.decay.set_value(9.0);
        p.decay_sync.set_value(label_index(DECAY_SYNC_LABELS, "1 bar"));
    });
    let mut knob = plugin(|p| p.decay.set_value(2.0));
    let a = impulse(&mut synced, 200, tempo(120.0));
    let b = impulse(&mut knob, 200, tempo(120.0));
    let tail = a[24_000..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(tail > 1e-5, "the render has no tail to compare (peak {tail})");
    let diff = a.iter().zip(&b).position(|(x, y)| x.to_bits() != y.to_bits());
    assert_eq!(diff, None, "synced decay differs from a 2.0 s knob at {diff:?}");
}

#[test]
fn decay_sync_falls_back_to_the_knob_without_tempo() {
    let mut synced = plugin(|p| {
        p.decay.set_value(3.0);
        p.decay_sync.set_value(label_index(DECAY_SYNC_LABELS, "2 bars"));
    });
    let mut knob = plugin(|p| p.decay.set_value(3.0));
    let a = impulse(&mut synced, 100, None);
    let b = impulse(&mut knob, 100, None);
    let diff = a.iter().zip(&b).position(|(x, y)| x.to_bits() != y.to_bits());
    assert_eq!(diff, None, "no tempo: decay sync must leave the knob in charge");
}

#[test]
fn a_tempo_change_under_predelay_sync_does_not_click() {
    // A sustained sine through a synced pre-delay while the tempo jumps:
    // the move goes through the pre-delay's tap crossfade, so the largest
    // step stays near the steady-tempo render's.
    let sine = |n: usize| 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / SR).sin();
    let render = |jump: bool| {
        let mut p = plugin(|p| {
            p.predelay_sync.set_value(label_index(PREDELAY_SYNC_LABELS, "1/32"));
            p.mod_depth.set_value(0.0);
        });
        let mut worst = 0.0f32;
        let mut prev = None;
        for b in 0..400usize {
            let bpm = if jump && b >= 200 { 97.0 } else { 120.0 };
            let mut left: Vec<f32> = (b * BLOCK..(b + 1) * BLOCK).map(sine).collect();
            let mut right = left.clone();
            let mut outs = [OutputBuffer {
                left: &mut left,
                right: &mut right,
            }];
            p.process(&mut outs, BLOCK, &mut EventIterator::empty(), tempo(bpm));
            if b >= 150 {
                for &x in &left {
                    if let Some(prev) = prev {
                        let step: f32 = x - prev;
                        worst = worst.max(step.abs());
                    }
                    prev = Some(x);
                }
            }
        }
        worst
    };
    let (steady, jumped) = (render(false), render(true));
    assert!(
        jumped <= steady * 1.5 + 1e-3,
        "a tempo change clicked: max step {jumped:.5} against {steady:.5} steady"
    );
}
