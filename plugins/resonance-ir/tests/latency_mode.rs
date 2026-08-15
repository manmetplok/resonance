//! The convolution block size is a parameter, and the plugin's reported
//! latency is that parameter (ba todo #1300, audit finding I1).
//!
//! The block size *is* this plugin's algorithmic latency: the
//! uniformly-partitioned convolver delays by exactly one hop, and the dry
//! path is delayed to match. resonance-ir is one of only two plugins in
//! the fleet with real latency and it reports it accurately, so the tests
//! that matter here are the ones that keep those two numbers equal:
//!
//! * what the plugin *reports* (`latency_samples`, which the host feeds
//!   into plugin delay compensation) is what the selected mode asks for;
//! * what the engine *does* — measured by pushing an impulse through it —
//!   is the same figure, at every mode.
//!
//! `tests/latency_mode_host.rs` closes the loop across the real CLAP ABI:
//! a mode change while active, the restart it asks the host for, and the
//! host's re-read afterwards.

use resonance_ir::dsp::{
    self, IrEngine, LatencyMode, LATENCY_MODE_LABELS, MAX_BLOCK_SIZE, MIN_BLOCK_SIZE,
};
use resonance_ir::latency;
use resonance_ir::params::IrParams;
use resonance_ir::viz::IrViz;
use resonance_ir::ResonanceIr;
use resonance_plugin::{Param, ResonancePlugin, Smoother, SmoothingStyle};

/// The rates the plugin's own base table distinguishes.
const RATES: [f32; 4] = [44_100.0, 48_000.0, 88_200.0, 96_000.0];

// ---------------------------------------------------------------------------
// The mode table
// ---------------------------------------------------------------------------

#[test]
fn normal_is_exactly_the_block_size_the_plugin_always_used() {
    // The default must not move anyone's latency: `Normal` is the
    // historical sample-rate table, verbatim.
    for rate in RATES {
        assert_eq!(
            dsp::block_size_for(rate, LatencyMode::Normal),
            dsp::block_size_for_sample_rate(rate),
            "the default mode changed the block size at {rate} Hz"
        );
    }
    assert_eq!(LatencyMode::default(), LatencyMode::Normal);
}

#[test]
fn the_modes_are_a_power_of_two_ladder_around_normal() {
    for rate in RATES {
        let tracking = dsp::block_size_for(rate, LatencyMode::Tracking);
        let normal = dsp::block_size_for(rate, LatencyMode::Normal);
        let efficient = dsp::block_size_for(rate, LatencyMode::Efficient);

        assert!(
            tracking < normal && normal < efficient,
            "at {rate} Hz the ladder is not ordered: {tracking}/{normal}/{efficient}"
        );
        for block in [tracking, normal, efficient] {
            assert!(
                block.is_power_of_two(),
                "block sizes must stay powers of two (the convolver's FFT size and \
                 the bypass delay line's wrap both depend on it), got {block}"
            );
            assert!(
                (MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block),
                "{block} escapes the declared bounds"
            );
        }

        // Each rung is two octaves of block size, so the trade is worth
        // making in either direction.
        assert_eq!(tracking * 4, normal, "tracking must be a quarter of normal");
        assert_eq!(normal * 4, efficient, "efficient must be four times normal");

        // Tracking is worth reaching for at every supported rate. (It is
        // not uniformly sub-millisecond only because the *base* table has
        // always jumped to 512 above 88 kHz — 88.2 kHz lands there, where
        // `Normal` itself is 5.8 ms. That predates this todo and moving it
        // would silently change everyone's latency, so it stands.)
        assert!(
            dsp::latency_ms(tracking, rate) < 1.5,
            "tracking mode is not short enough to track through at {rate} Hz, got {} ms",
            dsp::latency_ms(tracking, rate)
        );
    }
}

#[test]
fn an_out_of_range_mode_value_falls_back_to_normal() {
    // A host automation lane or an MCP client can write anything; the
    // fallback must not be "whichever end of the table we clamp to",
    // which for a latency control means silently parking at 46 ms.
    for index in [-7, 3, 99] {
        assert_eq!(LatencyMode::from_index(index), LatencyMode::Normal);
    }
    for mode in LatencyMode::ALL {
        assert_eq!(LatencyMode::from_index(mode.index()), mode);
    }
}

// ---------------------------------------------------------------------------
// The parameter
// ---------------------------------------------------------------------------

#[test]
fn the_mode_is_a_parameter_that_reads_as_its_name() {
    let params = IrParams::default();

    assert_eq!(
        params.latency_mode.value(),
        LatencyMode::Normal.index(),
        "the parameter must default to the mode the plugin has always used"
    );
    assert_eq!(
        params.latency_mode.range().min()..=params.latency_mode.range().max(),
        0..=LATENCY_MODE_LABELS.len() as i32 - 1,
        "the declared range must cover the choice table exactly"
    );

    // Display and parse both come off the choice table, which is what
    // makes a host automation lane, the editor's picker and
    // `track.set_plugin_param` read "Tracking" rather than "0".
    for (index, label) in LATENCY_MODE_LABELS.iter().enumerate() {
        let value = index as i32;
        assert_eq!(&params.latency_mode.display(value as f64), label);
        assert_eq!(
            params.latency_mode.parse(label),
            Some(value as f64),
            "the mode must be settable by name"
        );
    }
    // …and the raw index still parses, for a host that sends numbers.
    assert_eq!(params.latency_mode.parse("2"), Some(2.0));
}

#[test]
fn the_plugin_exposes_the_mode_to_the_host() {
    let plugin = ResonanceIr::new();
    let ids: Vec<String> = (0..plugin.param_count())
        .map(|i| plugin.param(i).id().to_string())
        .collect();
    assert!(
        ids.contains(&"latency_mode".to_string()),
        "the latency mode must be an enumerated parameter (that is what puts it \
         in the editor, a host automation lane and the control API at once), got {ids:?}"
    );
}

// ---------------------------------------------------------------------------
// Reported latency == selected mode
// ---------------------------------------------------------------------------

/// A plugin initialized at `rate` with `mode` selected.
fn initialized(mode: LatencyMode, rate: f32) -> ResonanceIr {
    let mut plugin = ResonanceIr::new();
    plugin.params.latency_mode.set_value(mode.index());
    plugin.initialize(rate, 512);
    plugin
}

#[test]
fn the_reported_latency_is_the_selected_mode() {
    for rate in RATES {
        for mode in LatencyMode::ALL {
            let plugin = initialized(mode, rate);
            assert_eq!(
                plugin.latency_samples() as usize,
                dsp::block_size_for(rate, mode),
                "{mode:?} at {rate} Hz reports the wrong latency"
            );
        }
    }
}

#[test]
fn selecting_a_mode_reports_the_new_latency_before_it_is_applied() {
    // The reported figure is the one the plugin will impose at its next
    // activation, so it is already correct when the host asks after a
    // change — this is what keeps `HostHandle::set_latency_samples`'
    // contract (report and `latency_samples()` must agree) and what the
    // host re-reads when it services the restart.
    let mut plugin = initialized(LatencyMode::Normal, 48_000.0);
    assert_eq!(plugin.latency_samples(), 128);

    plugin
        .params
        .latency_mode
        .set_value(LatencyMode::Tracking.index());
    assert_eq!(
        plugin.latency_samples(),
        32,
        "a mode change must be visible in the reported latency immediately"
    );

    // …and the reactivation applies it for real.
    plugin.initialize(48_000.0, 512);
    assert_eq!(plugin.latency_samples(), 32);
}

// ---------------------------------------------------------------------------
// Reported latency == the delay the DSP actually imposes
// ---------------------------------------------------------------------------

/// Push an impulse through a bare `IrEngine` (no convolver loaded, so the
/// signal takes the bypass-delay path that keeps dry aligned with the
/// convolver's latency) and return the sample index the impulse comes out
/// at.
fn measured_delay(block_size: usize) -> usize {
    let mut engine = IrEngine::new(block_size);
    let mut dry_wet = Smoother::new(SmoothingStyle::Linear(50.0));
    let mut gain = Smoother::new(SmoothingStyle::Logarithmic(50.0));
    dry_wet.set_sample_rate(48_000.0);
    gain.set_sample_rate(48_000.0);
    dry_wet.reset(1.0);
    gain.reset(1.0);

    let len = block_size * 4;
    let mut left = vec![0.0_f32; len];
    let mut right = vec![0.0_f32; len];
    left[0] = 1.0;
    right[0] = 1.0;
    engine.process_block(&mut left, &mut right, &mut dry_wet, &mut gain);

    left.iter()
        .position(|s| s.abs() > 0.5)
        .unwrap_or_else(|| panic!("the impulse never came out at block size {block_size}"))
}

#[test]
fn the_engine_delays_by_exactly_the_latency_it_reports() {
    // The whole point of reporting latency is that the host can line the
    // track back up. A mode that reported one figure and delayed by
    // another would smear the mix's timing — the finding's warning.
    for rate in RATES {
        for mode in LatencyMode::ALL {
            let plugin = initialized(mode, rate);
            let reported = plugin.latency_samples() as usize;
            assert_eq!(
                measured_delay(reported),
                reported,
                "{mode:?} at {rate} Hz: the impulse must come out exactly \
                 `latency_samples()` samples late"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The editor's readout
// ---------------------------------------------------------------------------

#[test]
fn the_readout_shows_the_imposed_latency_and_says_when_a_change_is_pending() {
    let params = IrParams::default();
    let viz = IrViz::new();

    // Before activation there is nothing to report — and no invented
    // figure either.
    let readout = latency::readout(&params, &viz);
    assert_eq!(readout.active, None);
    assert_eq!(readout.pending, None);

    // Once the engine runs, the user can finally see the milliseconds the
    // plugin is spending. 128 samples at 48 kHz is 2.67 ms.
    viz.store_engine_block(128, 48_000.0);
    let readout = latency::readout(&params, &viz);
    assert_eq!(readout.active.as_deref(), Some("2.67 ms · 128 samples"));
    assert_eq!(
        readout.pending, None,
        "nothing is pending while the engine runs the selected mode"
    );

    // Selecting a mode does not apply it — the host has to cycle the
    // plugin first — so the readout says so rather than pretending.
    params.latency_mode.set_value(LatencyMode::Tracking.index());
    let readout = latency::readout(&params, &viz);
    assert_eq!(readout.active.as_deref(), Some("2.67 ms · 128 samples"));
    assert_eq!(readout.pending.as_deref(), Some("0.67 ms · 32 samples"));

    // …and once the restart lands, the pending line goes away.
    viz.store_engine_block(32, 48_000.0);
    let readout = latency::readout(&params, &viz);
    assert_eq!(readout.active.as_deref(), Some("0.67 ms · 32 samples"));
    assert_eq!(readout.pending, None);
}
