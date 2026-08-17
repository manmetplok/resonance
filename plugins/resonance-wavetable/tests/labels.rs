//! Guards for the editor's label tables and the facts they claim.
//!
//! Every assertion here pins a label to the parameter or DSP discriminant it
//! describes, so a control cannot silently start misreporting again (ba todo
//! #1271: dead LFO segment, "Sync" mislabel, wrong-unit/shape labels).

use resonance_wavetable::dsp::lfo::{LfoMode, LfoShape};
use resonance_wavetable::dsp::modulation::{
    routing_summary, ModDest, ModSlot, ModSource, NUM_MOD_SLOTS,
};
use resonance_wavetable::params::{WavetableParams, PARAM_COUNT};
use resonance_wavetable::viz::WavetableVizState;
use resonance_plugin::param::Param;

fn slot(source: ModSource, dest: ModDest, amount: f32) -> ModSlot {
    ModSlot {
        source,
        dest,
        amount,
    }
}

// ---------------------------------------------------------------------------
// Label tables track the discriminants
// ---------------------------------------------------------------------------

#[test]
fn mod_source_labels_cover_every_discriminant() {
    // The picker offers exactly one entry per integer the param can hold,
    // and each entry names the variant `from_int` produces for it.
    let params = WavetableParams::new();
    let src = &params.mod_slots[0].source;
    assert_eq!(src.min_plain(), 0.0);
    assert_eq!(src.max_plain(), (ModSource::LABELS.len() - 1) as f64);

    for (i, label) in ModSource::LABELS.iter().enumerate() {
        let variant = ModSource::from_int(i as i32);
        assert_eq!(variant as usize, i, "ModSource::from_int({i}) round-trip");
        assert_eq!(&variant.label(), label);
    }
}

#[test]
fn mod_dest_labels_cover_every_discriminant() {
    let params = WavetableParams::new();
    let dst = &params.mod_slots[0].destination;
    assert_eq!(dst.min_plain(), 0.0);
    assert_eq!(dst.max_plain(), (ModDest::LABELS.len() - 1) as f64);

    for (i, label) in ModDest::LABELS.iter().enumerate() {
        let variant = ModDest::from_int(i as i32);
        assert_eq!(variant as usize, i, "ModDest::from_int({i}) round-trip");
        assert_eq!(&variant.label(), label);
    }
}

#[test]
fn lfo_shape_labels_cover_every_discriminant() {
    // The shape control used to print the bare integer; it now prints these,
    // so the table must span exactly the param's range.
    let params = WavetableParams::new();
    assert_eq!(params.lfo1.shape.min_plain(), 0.0);
    assert_eq!(
        params.lfo1.shape.max_plain(),
        (LfoShape::LABELS.len() - 1) as f64
    );

    for (i, label) in LfoShape::LABELS.iter().enumerate() {
        let variant = LfoShape::from_int(i as i32);
        assert_eq!(variant as usize, i, "LfoShape::from_int({i}) round-trip");
        assert_eq!(&variant.label(), label);
    }
}

// ---------------------------------------------------------------------------
// LFO card subtitle comes from the live matrix
// ---------------------------------------------------------------------------

#[test]
fn routing_summary_lists_the_destinations_actually_wired() {
    let slots = vec![
        slot(ModSource::Lfo1, ModDest::FilterCutoff, 0.5),
        slot(ModSource::Lfo2, ModDest::Osc1Pan, 0.5),
        slot(ModSource::Lfo1, ModDest::Osc1Position, -0.25),
    ];
    assert_eq!(
        routing_summary(&slots, ModSource::Lfo1),
        "→ Filter Cutoff · Osc1 Position"
    );
    assert_eq!(routing_summary(&slots, ModSource::Lfo2), "→ Osc1 Pan");
}

#[test]
fn osc_balance_and_unison_detune_are_ordinary_targets() {
    // They were marked "(inert)" by the ba todo #1278 stopgap; ba todo #1323
    // implemented both, so the marker must be gone.
    let slots = vec![
        slot(ModSource::Lfo3, ModDest::OscBalance, 0.2),
        slot(ModSource::Lfo3, ModDest::UnisonDetune, 0.4),
    ];
    assert_eq!(
        routing_summary(&slots, ModSource::Lfo3),
        "→ Osc Balance · Unison Detune"
    );
}

#[test]
fn routing_summary_ignores_slots_that_cannot_be_heard() {
    // Destination None, or a zero amount, is not a routing.
    let slots = vec![
        slot(ModSource::Lfo3, ModDest::None, 1.0),
        slot(ModSource::Lfo3, ModDest::AmpLevel, 0.0),
    ];
    assert_eq!(routing_summary(&slots, ModSource::Lfo3), "not routed");
}

#[test]
fn routing_summary_deduplicates_repeated_destinations() {
    let slots = vec![
        slot(ModSource::Lfo1, ModDest::FilterCutoff, 0.5),
        slot(ModSource::Lfo1, ModDest::FilterCutoff, 0.2),
    ];
    assert_eq!(routing_summary(&slots, ModSource::Lfo1), "→ Filter Cutoff");
}

#[test]
fn routing_summary_of_a_default_patch_is_unrouted() {
    let params = WavetableParams::new();
    let slots: Vec<ModSlot> = params
        .mod_slots
        .iter()
        .map(|s| ModSlot {
            source: ModSource::from_int(s.source.value()),
            dest: ModDest::from_int(s.destination.value()),
            amount: s.amount.value(),
        })
        .collect();
    assert_eq!(slots.len(), NUM_MOD_SLOTS);
    for source in [ModSource::Lfo1, ModSource::Lfo2, ModSource::Lfo3] {
        assert_eq!(routing_summary(&slots, source), "not routed");
    }
}

// ---------------------------------------------------------------------------
// Units and polarity the knobs claim
// ---------------------------------------------------------------------------

#[test]
fn glide_time_is_milliseconds() {
    // The knob was labelled "s" over a 0..2000 ms range.
    let params = WavetableParams::new();
    assert_eq!(params.glide_time.min_plain(), 0.0);
    assert_eq!(params.glide_time.max_plain(), 2000.0);
    assert!(
        params.glide_time.display(250.0).ends_with(" ms"),
        "glide display should carry a ms unit, got {:?}",
        params.glide_time.display(250.0)
    );
}

#[test]
fn filter_keytrack_is_unipolar() {
    // The knob was drawn bipolar, which put the centre detent at 50 %.
    let params = WavetableParams::new();
    assert_eq!(params.filter.keytrack.min_plain(), 0.0);
    assert_eq!(params.filter.keytrack.max_plain(), 1.0);
    assert_eq!(params.filter.keytrack.default_plain(), 0.0);
}

#[test]
fn every_lfo_mode_segment_is_backed_by_parameters() {
    // The control originally offered three segments over one bool, so the
    // third could never stick (ba todo #1271). It is three again — but each
    // now round-trips through the two params that back it (ba todo #1324).
    let params = WavetableParams::new();
    for lfo in [&params.lfo1, &params.lfo2, &params.lfo3] {
        assert!(lfo.retrigger.is_stepped());
        assert!(lfo.sync.is_stepped());
        for mode in [LfoMode::Free, LfoMode::Retrig, LfoMode::Sync] {
            let (sync, retrigger) = mode.to_params();
            lfo.sync.set_value(sync);
            lfo.retrigger.set_value(retrigger);
            assert_eq!(
                LfoMode::from_params(lfo.sync.value(), lfo.retrigger.value()),
                mode,
                "{} did not round-trip through its parameters",
                mode.label()
            );
        }
    }
}

#[test]
fn param_at_exposes_every_parameter_exactly_once() {
    // `param_at` is a hand-written index table and ba todo #1324 inserted six
    // parameters into the middle of it. A duplicate or a gap would silently
    // hide a parameter from presets, the host and MCP alike.
    let params = WavetableParams::new();
    let mut ids: Vec<&str> = (0..PARAM_COUNT).map(|i| params.param_at(i).id()).collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "param_at returned a duplicate id");
}

// ---------------------------------------------------------------------------
// Status bar reads real numbers
// ---------------------------------------------------------------------------

#[test]
fn io_config_is_absent_until_the_host_activates() {
    // The status bar printed "48000 Hz" / "256" literals; it now shows a
    // placeholder while these are zero.
    let viz = WavetableVizState::new();
    let snap = viz.read_snapshot();
    assert_eq!(snap.sample_rate, 0.0);
    assert_eq!(snap.max_block_frames, 0);
}

#[test]
fn io_config_reports_what_the_host_passed() {
    let viz = WavetableVizState::new();
    viz.store_io_config(96_000.0, 512);
    let snap = viz.read_snapshot();
    assert_eq!(snap.sample_rate, 96_000.0);
    assert_eq!(snap.max_block_frames, 512);
}
