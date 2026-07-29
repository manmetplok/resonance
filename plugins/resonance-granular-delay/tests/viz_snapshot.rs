//! Grain-snapshot slot encoding and publisher (ba todo #1135): the
//! packed `u64` slots must round-trip position/pitch/size/level and the
//! reversed/voiced/generation flags within their documented precision,
//! and a processing plugin must publish a readable snapshot + coarse
//! buffer peaks through `GranularViz`.

use resonance_granular_delay::viz::{
    pack_grain, unpack_grain, GrainSnapshot, GranularViz, GRAIN_SLOTS, PEAK_BINS,
};
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

const SR: f32 = 48_000.0;

fn round_trip(g: &GrainSnapshot) -> GrainSnapshot {
    unpack_grain(pack_grain(g)).expect("packed grain must decode as active")
}

#[test]
fn pack_decode_round_trip_within_precision() {
    let cases = [
        GrainSnapshot {
            position_ms: 500.0,
            pitch_semitones: 0.0,
            size_ms: 90.0,
            level: 0.5,
            reversed: false,
            voiced: false,
            generation: 0,
        },
        GrainSnapshot {
            position_ms: 1234.56,
            pitch_semitones: -24.0,
            size_ms: 499.0,
            level: 1.0,
            reversed: true,
            voiced: false,
            generation: 2,
        },
        GrainSnapshot {
            position_ms: 12.25,
            pitch_semitones: 36.4375, // shimmer accumulation past +24
            size_ms: 10.0,
            level: 0.031,
            reversed: false,
            voiced: true,
            generation: 7,
        },
        GrainSnapshot {
            position_ms: 0.0,
            pitch_semitones: 23.94,
            size_ms: 4.0,
            level: 0.0,
            reversed: true,
            voiced: true,
            generation: 1,
        },
    ];
    for g in &cases {
        let d = round_trip(g);
        assert!(
            (d.position_ms - g.position_ms).abs() <= 0.125 + 1e-4,
            "position {} decoded as {}",
            g.position_ms,
            d.position_ms
        );
        assert!(
            (d.pitch_semitones - g.pitch_semitones).abs() <= 0.03125 + 1e-4,
            "pitch {} decoded as {}",
            g.pitch_semitones,
            d.pitch_semitones
        );
        assert!(
            (d.size_ms - g.size_ms).abs() <= 0.5 + 1e-4,
            "size {} decoded as {}",
            g.size_ms,
            d.size_ms
        );
        assert!(
            (d.level - g.level).abs() <= 0.5 / 255.0 + 1e-4,
            "level {} decoded as {}",
            g.level,
            d.level
        );
        assert_eq!(d.reversed, g.reversed);
        assert_eq!(d.voiced, g.voiced);
        assert_eq!(d.generation, g.generation);
    }
}

#[test]
fn inactive_slot_decodes_none_and_out_of_range_clamps() {
    assert_eq!(unpack_grain(0), None, "the zero word must read inactive");

    let clamped = round_trip(&GrainSnapshot {
        position_ms: 60_000.0,
        pitch_semitones: 500.0,
        size_ms: 5_000.0,
        level: 3.0,
        reversed: false,
        voiced: false,
        generation: 9,
    });
    assert_eq!(clamped.position_ms, 16_383.75);
    assert_eq!(clamped.pitch_semitones, 2047.0 / 16.0);
    assert_eq!(clamped.size_ms, 1023.0);
    assert_eq!(clamped.level, 1.0);
    assert_eq!(clamped.generation, 7);

    let negative = round_trip(&GrainSnapshot {
        position_ms: -5.0,
        pitch_semitones: -500.0,
        size_ms: -1.0,
        level: -0.5,
        reversed: false,
        voiced: false,
        generation: 0,
    });
    assert_eq!(negative.position_ms, 0.0);
    assert_eq!(negative.pitch_semitones, -128.0);
    assert_eq!(negative.size_ms, 0.0);
    assert_eq!(negative.level, 0.0);
}

#[test]
fn viz_store_and_read_grains() {
    let viz = GranularViz::new();
    let mut out = [GrainSnapshot::default(); GRAIN_SLOTS];
    assert_eq!(viz.read_grains(&mut out), 0, "fresh viz must read empty");

    let a = GrainSnapshot {
        position_ms: 250.0,
        pitch_semitones: 7.0,
        size_ms: 80.0,
        level: 0.75,
        reversed: false,
        voiced: false,
        generation: 0,
    };
    let b = GrainSnapshot {
        position_ms: 750.0,
        pitch_semitones: -12.0,
        size_ms: 120.0,
        level: 0.25,
        reversed: true,
        voiced: false,
        generation: 1,
    };
    viz.store_grain(0, &a);
    viz.store_grain(1, &b);
    viz.clear_grains_from(2);
    let n = viz.read_grains(&mut out);
    assert_eq!(n, 2);
    assert_eq!(out[0], round_trip(&a), "slot order must be preserved");
    assert_eq!(out[1], round_trip(&b));

    // Clearing from 0 empties the snapshot again.
    viz.clear_grains_from(0);
    assert_eq!(viz.read_grains(&mut out), 0);
}

fn run_seconds(plugin: &mut ResonanceGranularDelay, seconds: f32) {
    let frames = (SR * seconds) as usize;
    let block = 512;
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut left: Vec<f32> = (pos..pos + n)
            .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.5)
            .collect();
        let mut right = left.clone();
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }
}

#[test]
fn plugin_publishes_grain_snapshot_and_peaks() {
    let mut plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(500.0);
    plugin.params.density_hz.set_value(25.0);
    plugin.params.mix.set_value(1.0);
    plugin.params.feedback.set_value(0.6);
    plugin.initialize(SR, 4096);
    run_seconds(&mut plugin, 1.0);

    let viz = plugin.viz();
    let mut grains = [GrainSnapshot::default(); GRAIN_SLOTS];
    let n = viz.read_grains(&mut grains);
    assert!(n > 0, "no grains published after 1 s of input");
    assert!(
        n >= plugin.active_grains(),
        "snapshot ({n}) must cover at least the audible cloud ({})",
        plugin.active_grains()
    );
    let mut saw_ghost = false;
    for g in &grains[..n] {
        assert!(g.position_ms > 0.0, "grain position must be behind the head");
        assert!(g.size_ms >= 4.0, "grain size below the engine minimum");
        if g.generation > 0 {
            saw_ghost = true;
            assert!(
                g.position_ms > 500.0,
                "generation-{} ghost at {} ms must sit at least one delay back",
                g.generation,
                g.position_ms
            );
        } else {
            // Un-transposed defaults: first-pass grains sit near the tap
            // (500 ms ± the 20 ms default spray) at pitch 0.
            assert!(
                (g.position_ms - 500.0).abs() < 100.0,
                "first-pass grain at {} ms, expected near the 500 ms tap",
                g.position_ms
            );
            assert_eq!(g.pitch_semitones, 0.0);
        }
    }
    assert!(
        saw_ghost,
        "60 % feedback on Wet→Buffer must publish recirculation ghosts"
    );

    let mut peaks = [0.0f32; PEAK_BINS];
    let bin_ms = viz.read_peaks(&mut peaks);
    assert!(bin_ms > 0.0, "peak bin duration must be published");
    // 1 s of signal at the head end: the newest bins must carry energy.
    let written_bins = (1000.0 / bin_ms).floor() as usize;
    let newest = &peaks[PEAK_BINS - written_bins.min(8)..];
    assert!(
        newest.iter().any(|&p| p > 0.1),
        "newest peak bins are silent after 1 s of 0.5-amplitude input"
    );
}
