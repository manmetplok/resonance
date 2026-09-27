//! The editor's controls against the parameters they edit (ba todo #1285,
//! audit findings F4 / W4).
//!
//! Before the migration every knob in this editor mapped its arc
//! **linearly** across `min..max`, passed a hardcoded `0.0` as its
//! double-click default, and formatted its own readout from a
//! caller-supplied unit suffix. All three are properties of the *call
//! site*, so there are two kinds of test here:
//!
//! * a source guard over the five tab modules — a control call may name
//!   only its parameter and a cell caption, and no tab may reach past the
//!   bindings to the raw widget;
//! * behavioural tests that a control's 0..1 travel is exactly the
//!   parameter's own `FloatRange`, that a reset lands on the declared
//!   default verbatim, and that the readout is the parameter's own
//!   formatter.
//!
//! They need no GUI: the bindings add only egui plumbing on top of
//! `normalized_value` / `default_normalized` / `plain_at_normalized` /
//! `set_normalized` / `Param::display`, which is the whole contract.

use resonance_wavetable::params::{WavetableParams, PARAM_COUNT};
use resonance_wavetable::presets::PRESETS;
use resonance_plugin::{FloatParam, FloatRange, IntParam, Param};

// ---------------------------------------------------------------------------
// The sources under guard
// ---------------------------------------------------------------------------

/// The five tab modules, compiled in so the guard can never drift from
/// the files the editor actually builds.
const TABS: [(&str, &str); 5] = [
    ("osc.rs", include_str!("../src/editor/tabs/osc.rs")),
    (
        "env_filter.rs",
        include_str!("../src/editor/tabs/env_filter.rs"),
    ),
    ("lfo.rs", include_str!("../src/editor/tabs/lfo.rs")),
    ("fx.rs", include_str!("../src/editor/tabs/fx.rs")),
    (
        "mod_matrix.rs",
        include_str!("../src/editor/tabs/mod_matrix.rs"),
    ),
];

/// The module the param bindings themselves live in.
const BINDINGS: &str = include_str!("../src/editor/tabs/mod.rs");

/// Split a single-line call's argument list on commas. Safe here because
/// the guard also rejects any argument that is not a plain path, a string
/// literal or a bare identifier — none of which can contain a comma.
fn call_args<'a>(line: &'a str, callee: &str) -> Option<Vec<&'a str>> {
    let rest = line.trim().strip_prefix(callee)?;
    let inner = rest
        .trim_end()
        .strip_suffix(';')?
        .trim_end()
        .strip_suffix(')')?;
    Some(inner.split(',').map(str::trim).collect())
}

// ---------------------------------------------------------------------------
// Every parameter, listed once
// ---------------------------------------------------------------------------

/// Every `FloatParam` in the synth. All 52 are drawn by a knob or a
/// slider, so this list is also the set of controls under test.
fn float_params(p: &WavetableParams) -> Vec<(String, &FloatParam)> {
    let mut out: Vec<(String, &FloatParam)> = vec![
        ("master_volume".into(), &p.master_volume),
        ("glide_time".into(), &p.glide_time),
        ("osc_balance".into(), &p.osc_balance),
        ("unison.detune".into(), &p.unison.detune),
        ("unison.spread".into(), &p.unison.spread),
        ("filter.cutoff".into(), &p.filter.cutoff),
        ("filter.resonance".into(), &p.filter.resonance),
        ("filter.env_depth".into(), &p.filter.env_depth),
        ("filter.keytrack".into(), &p.filter.keytrack),
        ("filter.drive".into(), &p.filter.drive),
        ("filter.fm".into(), &p.filter.fm),
        ("chorus.rate".into(), &p.chorus.rate),
        ("chorus.depth".into(), &p.chorus.depth),
        ("chorus.mix".into(), &p.chorus.mix),
        ("delay.time_l".into(), &p.delay.time_l),
        ("delay.time_r".into(), &p.delay.time_r),
        ("delay.feedback".into(), &p.delay.feedback),
        ("delay.mix".into(), &p.delay.mix),
        ("distortion.drive".into(), &p.distortion.drive),
        ("distortion.mix".into(), &p.distortion.mix),
    ];
    for (name, osc) in [("osc1", &p.osc1), ("osc2", &p.osc2)] {
        out.push((format!("{name}.position"), &osc.position));
        out.push((format!("{name}.fine"), &osc.fine));
        out.push((format!("{name}.level"), &osc.level));
        out.push((format!("{name}.pan"), &osc.pan));
    }
    for (name, env) in [("amp_env", &p.amp_env), ("mod_env", &p.mod_env)] {
        out.push((format!("{name}.attack"), &env.attack));
        out.push((format!("{name}.decay"), &env.decay));
        out.push((format!("{name}.sustain"), &env.sustain));
        out.push((format!("{name}.release"), &env.release));
        out.push((format!("{name}.curve"), &env.curve));
    }
    for (name, lfo) in [("lfo1", &p.lfo1), ("lfo2", &p.lfo2), ("lfo3", &p.lfo3)] {
        out.push((format!("{name}.rate"), &lfo.rate));
        out.push((format!("{name}.depth"), &lfo.depth));
    }
    for (i, slot) in p.mod_slots.iter().enumerate() {
        out.push((format!("mod_slots[{i}].amount"), &slot.amount));
    }
    out
}

/// The `IntParam`s the editor draws as a knob. The other 20 int params
/// are drawn by a different control kind — the wavetable name row
/// (`oscN_wavetable`), the filter-type and filter-model chips
/// (`filter_type`, `filter_model`) and the mod
/// matrix's two combo pills (16) — and are not knobs to migrate.
fn int_knob_params(p: &WavetableParams) -> Vec<(String, &IntParam)> {
    vec![
        ("max_voices".into(), &p.max_voices),
        ("unison.voices".into(), &p.unison.voices),
        ("osc1.coarse".into(), &p.osc1.coarse),
        ("osc2.coarse".into(), &p.osc2.coarse),
        ("lfo1.shape".into(), &p.lfo1.shape),
        ("lfo2.shape".into(), &p.lfo2.shape),
        ("lfo3.shape".into(), &p.lfo3.shape),
    ]
}

#[test]
fn the_control_list_covers_every_float_parameter() {
    let p = WavetableParams::default();
    let floats = float_params(&p);
    assert_eq!(
        floats.len(),
        52,
        "the synth declares 52 FloatParams; the control list must name all of them"
    );

    // Every listed parameter is one `param_at` really exposes...
    let exposed: Vec<&str> = (0..PARAM_COUNT).map(|i| p.param_at(i).id()).collect();
    for (field, param) in &floats {
        assert!(
            exposed.contains(&param.id()),
            "{field} is not exposed by param_at"
        );
    }
    for (field, param) in int_knob_params(&p) {
        assert!(
            exposed.contains(&param.id()),
            "{field} is not exposed by param_at"
        );
    }

    // ...and the counts add up, which is what proves nothing was left out:
    // 52 floats + 30 ints + 13 bools is the whole parameter list.
    //
    // Was 51/26/10 == 87 when this guard was written. ba todo #1324 (LFO tempo
    // sync) added three `lfoN_sync` bools and three `lfoN_division` ints, so the
    // whole list is 93. The float count is deliberately unchanged — #1324 added
    // no float — which is what makes this a real check rather than a tautology.
    // The filter models then added `filter_fm` (float) and `filter_model`
    // (int): 52/30/13 == 95.
    let mut ids: Vec<&str> = floats.iter().map(|(_, p)| p.id()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 52, "the control list repeats a parameter");
    assert_eq!(52 + 30 + 13, PARAM_COUNT);
}

// ---------------------------------------------------------------------------
// Source guard: a control call names its parameter and nothing else
// ---------------------------------------------------------------------------

#[test]
fn no_control_call_site_restates_a_param_fact() {
    let mut knobs = 0;
    let mut int_knobs = 0;
    let mut sliders = 0;

    for (file, src) in TABS {
        for line in src.lines() {
            if let Some(args) = call_args(line, "float_knob(") {
                knobs += 1;
                assert_eq!(
                    args.len(),
                    3,
                    "{file}: float_knob takes ui, a caption and the param — nothing else: {}",
                    line.trim()
                );
                assert_eq!(args[0], "ui", "{file}: {}", line.trim());
                assert!(
                    args[1].starts_with('"') && args[1].ends_with('"'),
                    "{file}: the caption must be a literal, not a computed string: {}",
                    line.trim()
                );
                assert!(
                    args[2].starts_with('&'),
                    "{file}: the third argument must be the FloatParam itself: {}",
                    line.trim()
                );
            } else if let Some(args) = call_args(line, "int_knob(") {
                int_knobs += 1;
                assert_eq!(args.len(), 3, "{file}: {}", line.trim());
                assert!(args[2].starts_with('&'), "{file}: {}", line.trim());
            } else if let Some(args) = call_args(line, "float_slider(") {
                sliders += 1;
                assert_eq!(
                    args.len(),
                    3,
                    "{file}: float_slider takes ui, a width and the param: {}",
                    line.trim()
                );
                assert!(
                    args[2].starts_with('&'),
                    "{file}: the third argument must be the FloatParam itself: {}",
                    line.trim()
                );
            }
        }
    }

    assert_eq!(knobs, 30, "expected 30 float knob cells across the five tabs");
    assert_eq!(int_knobs, 3, "expected 3 plain int knob cells");
    assert_eq!(sliders, 2, "expected the balance and mod-amount sliders");
}

#[test]
fn only_known_seams_still_carry_their_own_label_table() {
    // `int_knob_fmt` is the remaining formatter argument. Two call sites, each
    // with a todo that removes it:
    //   - LFO Shape  -> ba todo #1292 moves `dsp::lfo`'s table onto the param;
    //   - LFO Div    -> ba todo #1356 does the same for `SyncDivision::LABELS`,
    //     which could not land with #1292 because #1324 (which introduced Div)
    //     and #1289 (`with_choices`) were on different epic branches until
    //     integration merged them.
    // Pinned to exactly two so it cannot spread in the meantime.
    let uses: usize = TABS
        .iter()
        .map(|(_, src)| src.matches("int_knob_fmt(").count())
        .sum();
    assert_eq!(
        uses, 2,
        "int_knob_fmt is the #1292/#1356 seam — exactly two call sites (LFO Shape, LFO Div)"
    );
}

#[test]
fn no_tab_reaches_past_the_bindings_to_the_raw_widget() {
    for (file, src) in TABS {
        for needle in [
            "knob_themed",
            "ThemedKnob",
            // `slider_unit` until ba todo #1335 retired the editor's
            // private slider; the raw widget is the shared kit's
            // `HSlider` / `widgets::slider` now.
            "HSlider",
            "widgets::slider",
            "knob_unipolar",
            "knob_bipolar",
        ] {
            assert!(
                !src.contains(needle),
                "{file} uses `{needle}` directly; controls must go through the param bindings"
            );
        }
    }
}

#[test]
fn no_tab_declares_a_unit_or_a_readout_of_its_own() {
    for (file, src) in TABS {
        let body: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !body.contains("Some(\""),
            "{file}: a literal unit argument means a control restates params.rs"
        );
        assert!(
            !body.contains("format!(\"{:."),
            "{file}: a hand-rolled numeric readout means a control restates the param's formatter"
        );
    }
}

#[test]
fn the_bindings_take_the_curve_and_the_default_from_the_parameter() {
    // The two things the old helpers hardcoded. `knob_unipolar` /
    // `knob_bipolar` are the shorthands that take a default as a plain
    // number; the bindings must build a `ThemedKnob` from the param's own
    // normalized view instead.
    for needle in ["default_normalized()", "normalized_value()", "set_normalized("] {
        assert!(
            BINDINGS.contains(needle),
            "the bindings must read `{needle}` off the parameter"
        );
    }
    for needle in ["knob_unipolar", "knob_bipolar"] {
        assert!(
            !BINDINGS.contains(needle),
            "`{needle}` takes a hardcoded default — that is the drift ba todo #1285 removed"
        );
    }
    // No binding recomputes a linear position from the bounds; that is
    // what threw the declared skew away (finding W4).
    let body: String = BINDINGS
        .lines()
        .filter(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !body.contains("min_plain()") && !body.contains("max_plain()"),
        "a float binding that reaches for min/max is mapping the arc itself"
    );
}

// ---------------------------------------------------------------------------
// Behaviour: the arc is the parameter's own range
// ---------------------------------------------------------------------------

#[test]
fn knob_travel_is_exactly_the_declared_range() {
    let p = WavetableParams::default();
    for (field, param) in float_params(&p) {
        let range = param.range();
        assert_eq!(
            param.plain_at_normalized(0.0),
            range.min(),
            "{field}: 0 % travel must be the declared minimum"
        );
        assert_eq!(
            param.plain_at_normalized(1.0),
            range.max(),
            "{field}: 100 % travel must be the declared maximum"
        );

        let mut previous = f32::NEG_INFINITY;
        for step in 0..=100 {
            let t = step as f32 / 100.0;
            let plain = param.plain_at_normalized(t);
            assert_eq!(
                plain,
                range.denormalize(t),
                "{field}: the knob must sweep the param's own denormalize"
            );
            assert!(
                plain >= previous,
                "{field}: travel must move the value monotonically (t = {t})"
            );
            assert!(
                plain >= range.min() && plain <= range.max(),
                "{field}: {plain} at t = {t} escapes the declared range"
            );
            previous = plain;
        }
    }
}

#[test]
fn the_knob_and_the_parameter_agree_in_both_directions() {
    let p = WavetableParams::default();
    for (field, param) in float_params(&p) {
        let range = param.range();
        let tolerance = (range.max() - range.min()).abs() * 1e-4;
        for step in 0..=20 {
            // Where the host puts the value, the knob must point.
            let plain = range.denormalize(step as f32 / 20.0);
            param.set_value(plain);
            let travel = param.normalized_value();
            let round_tripped = param.plain_at_normalized(travel);
            assert!(
                (round_tripped - plain).abs() <= tolerance,
                "{field}: {plain} reads back as {round_tripped} through the arc"
            );

            // And where the user drags the knob, the parameter follows.
            param.set_normalized(travel);
            assert!(
                (param.value() - plain).abs() <= tolerance,
                "{field}: dragging to {travel} left the param at {}",
                param.value()
            );
        }
    }
}

#[test]
fn a_reset_lands_on_the_declared_default_verbatim() {
    let p = WavetableParams::default();
    for (field, param) in float_params(&p) {
        param.set_value(param.range().max());
        // Double-click-to-reset drops the knob on default_normalized.
        param.set_normalized(param.default_normalized());
        assert_eq!(
            param.value(),
            param.default_value(),
            "{field}: reset must write the declared default exactly"
        );
    }
}

#[test]
fn an_int_knobs_travel_round_trips_every_step() {
    let p = WavetableParams::default();
    for (field, param) in int_knob_params(&p) {
        let range = param.range();
        for value in range.min()..=range.max() {
            let travel = range.normalize(value) as f32;
            let back = range.min()
                + (travel.clamp(0.0, 1.0) * (range.max() - range.min()) as f32).round() as i32;
            assert_eq!(back, value, "{field}: step {value} does not survive the arc");
        }
        // And the reset position is the declared default, not the minimum.
        let default_travel = range.normalize(param.default_value()) as f32;
        let back = range.min()
            + (default_travel.clamp(0.0, 1.0) * (range.max() - range.min()) as f32).round() as i32;
        assert_eq!(back, param.default_value(), "{field}: reset misses the default");
    }
}

#[test]
fn a_controls_polarity_comes_from_its_range() {
    // Bipolar iff the range spans zero. The call sites no longer claim a
    // polarity, which is what let `filter_keytrack` — a 0..1 parameter —
    // be drawn centre-out with its detent at 50 % tracking.
    let p = WavetableParams::default();
    let mut bipolar: Vec<String> = float_params(&p)
        .into_iter()
        .filter(|(_, param)| param.range().min() < 0.0 && param.range().max() > 0.0)
        .map(|(field, _)| field)
        .collect();
    bipolar.sort();

    let mut expected: Vec<String> = vec![
        "osc_balance".into(),
        "osc1.fine".into(),
        "osc1.pan".into(),
        "osc2.fine".into(),
        "osc2.pan".into(),
        "amp_env.curve".into(),
        "mod_env.curve".into(),
        "filter.env_depth".into(),
    ];
    expected.extend((0..8).map(|i| format!("mod_slots[{i}].amount")));
    expected.sort();
    assert_eq!(bipolar, expected);

    assert!(
        p.filter.keytrack.range().min() >= 0.0,
        "filter_keytrack is unipolar; a bipolar arc would put its detent at 50 % tracking"
    );
}

// ---------------------------------------------------------------------------
// The drift itself, pinned
// ---------------------------------------------------------------------------

/// Where the old editor's hardcoded `0.0` reset default landed, for a
/// unipolar knob: travel 0 is the range minimum.
fn old_reset_value(param: &FloatParam) -> f32 {
    param.range().min()
}

#[test]
fn every_knob_used_to_reset_to_its_range_minimum() {
    // The old helpers passed a literal `0.0` as the knob's double-click
    // default. For the four knobs drawn bipolar that was the centre of a
    // symmetric range, which happens to be the declared default; for every
    // *unipolar* knob it was the range minimum, so a reset gave silence
    // (master), a 20 Hz cutoff, or a 0.01 Hz LFO.
    let p = WavetableParams::default();
    let mut drifted: Vec<String> = float_params(&p)
        .into_iter()
        .filter(|(_, param)| !(param.range().min() < 0.0 && param.range().max() > 0.0))
        .filter(|(_, param)| old_reset_value(param) != param.default_value())
        .map(|(field, _)| field)
        .collect();
    drifted.sort();

    let mut expected: Vec<String> = vec![
        "amp_env.attack".into(),
        "amp_env.decay".into(),
        "amp_env.release".into(),
        "amp_env.sustain".into(),
        "chorus.depth".into(),
        "chorus.mix".into(),
        "chorus.rate".into(),
        "delay.feedback".into(),
        "delay.mix".into(),
        "delay.time_l".into(),
        "delay.time_r".into(),
        "distortion.mix".into(),
        "filter.cutoff".into(),
        // The three `lfoN_depth` are absent on purpose: ba todo #1354 set
        // their declared default to 0.0, which *is* their range minimum, so
        // the old reset gesture happened to agree with the declaration
        // there. Their rates still drift.
        "lfo1.rate".into(),
        "lfo2.rate".into(),
        "lfo3.rate".into(),
        "master_volume".into(),
        "mod_env.attack".into(),
        "mod_env.decay".into(),
        "mod_env.release".into(),
        "osc1.level".into(),
        "osc2.level".into(),
        "unison.detune".into(),
        "unison.spread".into(),
    ];
    expected.sort();
    assert_eq!(
        drifted, expected,
        "the set of knobs whose reset gesture disagreed with params.rs changed"
    );

    // The three worst, spelled out.
    assert_eq!(p.master_volume.default_value(), 0.8); // reset used to mute
    assert_eq!(p.filter.cutoff.default_value(), 20000.0); // reset used to give 20 Hz
    assert_eq!(p.lfo2.rate.default_value(), 2.0); // reset used to give 0.01 Hz

    // Two int knobs had the same problem.
    assert_eq!(p.max_voices.default_value(), 16);
    assert_eq!(p.max_voices.range().min(), 1);
    assert_eq!(p.osc1.coarse.default_value(), 0);
    assert_eq!(p.osc1.coarse.range().min(), -24);
    // And two did not: their default *is* the minimum.
    assert_eq!(p.unison.voices.default_value(), p.unison.voices.range().min());
    assert_eq!(p.lfo1.shape.default_value(), p.lfo1.shape.range().min());
}

#[test]
fn the_declared_skew_reaches_the_arc() {
    let p = WavetableParams::default();
    let mut skewed = 0;
    for (field, param) in float_params(&p) {
        let range = param.range();
        let linear_mid = (range.min() + range.max()) / 2.0;
        let arc_mid = param.plain_at_normalized(0.5);
        match range {
            FloatRange::Linear { .. } => assert!(
                (arc_mid - linear_mid).abs() < 1e-5,
                "{field} is Linear, so half travel must be the arithmetic middle"
            ),
            // Every skewed range here declares a negative factor, which
            // gives the low end of the range more of the dial: half travel
            // must land well below the arithmetic middle. Without this the
            // skew is declared and thrown away, which is finding W4.
            FloatRange::Skewed { .. } => {
                skewed += 1;
                assert!(
                    arc_mid < linear_mid * 0.8,
                    "{field}: half travel is {arc_mid}, barely below the linear middle \
                     {linear_mid} — the declared skew is not reaching the control"
                );
            }
        }
    }
    assert_eq!(
        skewed, 15,
        "15 float params declare a skew the old editor ignored"
    );
}

#[test]
fn the_skew_is_what_makes_the_low_end_reachable() {
    // Finding W4 in numbers. The old arc was linear, so at half travel a
    // 20-20000 Hz cutoff sat at 10 kHz and everything under 2 kHz was
    // crammed into the first tenth of the dial. The declared skew puts
    // half travel at a few hundred Hz instead.
    let p = WavetableParams::default();
    let cutoff = &p.filter.cutoff;
    let linear_mid = (cutoff.range().min() + cutoff.range().max()) / 2.0;
    assert!((linear_mid - 10_010.0).abs() < 1.0);
    let arc_mid = cutoff.plain_at_normalized(0.5);
    assert!(
        (300.0..600.0).contains(&arc_mid),
        "half travel on the cutoff should be a few hundred Hz, got {arc_mid}"
    );
    // A tenth of the dial used to reach 2 kHz; it now reaches ~20 Hz, so
    // the bottom of the range is where the fine control is.
    assert!(cutoff.plain_at_normalized(0.1) < 30.0);
    // And the whole 20 Hz - 2 kHz span now occupies most of the arc.
    assert!(cutoff.range().normalize(2000.0) > 0.6);

    // Sub-100 ms attacks: with a linear arc, 5 ms sat at 0.08 % of the
    // travel — under a pixel of a 52 px dial. The declared skew gives it
    // a sixth of the arc.
    let attack = &p.amp_env.attack;
    let linear_travel = (0.005 - attack.range().min()) / (attack.range().max() - attack.range().min());
    assert!(linear_travel < 0.001, "{linear_travel}");
    assert!(
        attack.range().normalize(0.005) > 0.1,
        "5 ms should be a settable distance up the arc"
    );
}

#[test]
fn the_readout_is_the_params_own_formatter() {
    let p = WavetableParams::default();
    // (what the old call site printed, what the parameter prints)
    let cases: [(&str, String, &str); 12] = [
        ("master", p.master_volume.display(0.8), "-1.9 dB"),
        ("master muted", p.master_volume.display(0.0), "-inf dB"),
        ("osc1 position", p.osc1.position.display(0.0), "0%"),
        ("osc1 level", p.osc1.level.display(1.0), "100%"),
        ("osc1 fine", p.osc1.fine.display(0.0), "0.0 ct"),
        ("unison detune", p.unison.detune.display(15.0), "15.0 ct"),
        ("amp attack", p.amp_env.attack.display(0.005), "0.005 s"),
        ("amp sustain", p.amp_env.sustain.display(0.8), "80%"),
        ("filter cutoff", p.filter.cutoff.display(8000.0), "8000 Hz"),
        ("lfo1 rate", p.lfo1.rate.display(1.0), "1.00 Hz"),
        ("delay time L", p.delay.time_l.display(375.0), "375 ms"),
        ("delay feedback", p.delay.feedback.display(0.4), "40%"),
    ];
    for (what, got, want) in cases {
        assert_eq!(got, want, "{what} reads back as `{got}`, expected `{want}`");
    }

    // The old strings, for the record: `{:.2}` plus a caller-supplied
    // suffix. None of them is what the parameter says.
    assert_eq!(format!("{:.2}", 0.8), "0.80"); // master, no dB at all
    assert_eq!(format!("{:.2}{}", 0.005, "s"), "0.01s"); // 5 ms rounded away
    assert_eq!(format!("{:.2}{}", 8000.0, "Hz"), "8000.00Hz");
    assert_eq!(format!("{:.2}", 0.4), "0.40"); // a percentage as a fraction
}

#[test]
fn no_readout_doubles_up_the_declared_unit() {
    let p = WavetableParams::default();
    for (field, param) in float_params(&p) {
        let unit = param.unit();
        if unit.is_empty() {
            continue;
        }
        for step in 0..=10 {
            let value = param.plain_at_normalized(step as f32 / 10.0);
            let text = param.display(value as f64);
            assert_eq!(
                text.matches(unit.trim()).count(),
                1,
                "{field}: `{text}` repeats the unit `{unit}`"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The 23 factory presets are untouched by the migration
// ---------------------------------------------------------------------------

#[test]
fn every_factory_preset_loads_bit_identically() {
    // The migration is editor-only, but the knobs now write through
    // `set_normalized` rather than `set_plain`, so this pins that nothing
    // a preset stores is rounded, clamped or otherwise moved on the way
    // in. Every value must survive the f64 -> f32 store exactly.
    let params = WavetableParams::default();
    for entry in PRESETS {
        let value: serde_json::Value =
            serde_json::from_str(entry.json).unwrap_or_else(|e| panic!("{}: {e}", entry.name));
        let map = value
            .get("params")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("{}: no \"params\" object", entry.name));

        resonance_plugin::presets::load(entry.json, PARAM_COUNT, |i| params.param_at(i));

        for i in 0..PARAM_COUNT {
            let param = params.param_at(i);
            let Some(want) = map.get(param.id()).and_then(|v| v.as_f64()) else {
                panic!("{}: no value for {}", entry.name, param.id());
            };
            assert_eq!(
                param.get_plain(),
                want as f32 as f64,
                "{}: {} was moved on load (declared range {}..{})",
                entry.name,
                param.id(),
                param.min_plain(),
                param.max_plain()
            );
        }
    }
}

/// A fresh instance, the host's "reset to default" and the "Init" preset
/// are all the same sound.
///
/// They used to differ on four parameters — `filter_cutoff` (8 kHz
/// declared against 20 kHz in the preset) and the three `lfoN_depth`
/// (0.5/0.3/0.3 declared against 0.0). Reported with ba todo #1285 and
/// decided in ba todo #1354: init.json won, and the declarations moved to
/// match it. "Init" means inert, and of the two candidate sounds a fresh
/// instance that arrives pre-filtered at 8 kHz with three LFOs already
/// modulating is the surprising one.
///
/// The rule the rest of this file works to — the parameter declaration is
/// the truth — is not repealed by that; this was the one place where the
/// declaration was the mistake.
#[test]
fn the_init_preset_matches_every_declared_default() {
    let params = WavetableParams::default();
    let init = PRESETS
        .iter()
        .find(|e| e.name == "Init")
        .expect("an Init preset");
    let value: serde_json::Value = serde_json::from_str(init.json).unwrap();
    let map = value.get("params").and_then(|v| v.as_object()).unwrap();

    let mut disagree: Vec<(String, f32, f32)> = Vec::new();
    let mut checked = 0usize;
    for i in 0..PARAM_COUNT {
        let param = params.param_at(i);
        let Some(preset) = map.get(param.id()).and_then(|v| v.as_f64()) else {
            continue;
        };
        checked += 1;
        let declared = param.default_plain() as f32;
        if preset as f32 != declared {
            disagree.push((param.id().to_string(), preset as f32, declared));
        }
    }
    disagree.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        disagree,
        Vec::new(),
        "(id, init.json, declared) — picking Init must be a no-op on a \
         fresh instance"
    );
    // And the comparison was not vacuous: Init carries every parameter.
    assert_eq!(
        checked, PARAM_COUNT,
        "Init must name every parameter for this to mean anything"
    );
}
