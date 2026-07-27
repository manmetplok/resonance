//! Editor metering state (ba todo #1079): the audio thread publishes
//! block-rate readouts into the shared `GranularViz` atomics the
//! editor consumes — effective delay, host tempo, scheduler state and
//! grain counts.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo};

const SR: f32 = 48_000.0;

fn run_blocks(
    plugin: &mut ResonanceGranularDelay,
    left: &mut [f32],
    right: &mut [f32],
    block: usize,
    tempo: Option<TempoInfo>,
) {
    let frames = left.len();
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, tempo);
        pos += n;
    }
}

fn sine(frames: usize, freq: f32, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
        .collect()
}

#[test]
fn viz_publishes_delay_grains_and_tempo() {
    let mut plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(500.0);
    plugin.params.density_hz.set_value(25.0);
    plugin.params.mix.set_value(1.0);
    plugin.initialize(SR, 4096);

    // Free-running, no host tempo: delay readout follows time_ms, BPM
    // reads 0, the voice path is disengaged.
    let frames = SR as usize;
    let input = sine(frames, 440.0, 0.5);
    let mut left = input.clone();
    let mut right = input;
    run_blocks(&mut plugin, &mut left, &mut right, 512, None);
    let viz = plugin.viz();
    assert!(
        (viz.read_delay_ms() - 500.0).abs() < 1.0,
        "delay readout {} ms, expected ~500",
        viz.read_delay_ms()
    );
    assert_eq!(viz.read_bpm(), 0.0);
    assert!(!viz.read_engaged());
    assert_eq!(viz.read_psola_voices(), 0);
    assert_eq!(
        viz.read_active_grains() as usize,
        plugin.active_grains(),
        "grain readout out of sync with the metering hook"
    );
    assert!(viz.read_active_grains() > 0, "no grains after 1 s of input");

    // Tempo-synced: 1/4 at 120 BPM = 500 ms; BPM is published.
    plugin.params.sync.set_plain(1.0);
    plugin.params.division.set_value(4); // 1/4
    let tempo = Some(TempoInfo {
        bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    });
    let input = sine(frames, 440.0, 0.5);
    let mut left = input.clone();
    let mut right = input;
    run_blocks(&mut plugin, &mut left, &mut right, 512, tempo);
    let viz = plugin.viz();
    assert_eq!(viz.read_bpm(), 120.0);
    assert!(
        (viz.read_delay_ms() - 500.0).abs() < 1.0,
        "synced delay readout {} ms, expected 500 (1/4 @ 120)",
        viz.read_delay_ms()
    );
}
