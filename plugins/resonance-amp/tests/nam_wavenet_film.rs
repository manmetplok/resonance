//! Tests for FiLM (feature-wise linear modulation) at the 8 per-layer
//! insertion points (A2), reference `nam::FiLM` in NAM/film.h and its use
//! in `Layer::Process` / `Layer::set_weights_` (NAM/wavenet/detail.h +
//! model.cpp).
//!
//! Covers: hand-computed modulation (scale+shift and scale-only), the
//! weight consumption order (all FiLM tensors after the layer's
//! conv/input_mixin/layer1x1/head1x1, in site order), per-site width pins
//! (incl. the gated `2 * bottleneck` widths), grouped FiLM convs, the
//! reference's blended-only application of layer1x1_post_film, inactive
//! bit-identity with the film-less path, and construction-time validation
//! errors. The wavenet_a2_max fixture (all 8 sites active, with groups) is
//! covered end to end in nam_wavenet_head1x1.rs.

use resonance_amp::nam::activations::{ActivationConfig, ActivationKind};
use resonance_amp::nam::parse::{
    load_model_from_file, parse_wavenet_config, StackConfig, WaveNetConfig, WeightReader,
};
use resonance_amp::nam::wavenet::params::{
    FilmParams, GatingMode, Head1x1Params, LayerFilms,
};
use resonance_amp::nam::wavenet::WaveNetModel;
use resonance_amp::nam::{fast_tanh, NamInference};

fn write_temp_nam(name: &str, body: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_nam_film_{}_{name}.nam",
        std::process::id()
    ));
    std::fs::write(&path, body).unwrap();
    path
}

fn load_nam(name: &str, body: &str) -> Result<Box<dyn NamInference>, String> {
    let path = write_temp_nam(name, body);
    let result = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    result.map(|loaded| loaded.model)
}

fn new_format_json(config: &str, weights: &[f32]) -> String {
    let ws: Vec<String> = weights.iter().map(|w| w.to_string()).collect();
    format!(
        r#"{{
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "config": {config},
            "weights": [{}]
        }}"#,
        ws.join(",")
    )
}

/// Deterministic small filler weights (kept small to avoid saturation).
fn counted_weights(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.01 + (i as f32) * 0.003).collect()
}

/// Minimal 1-wide single-layer JSON config with the given extra fields
/// (film blocks etc.) spliced in. channels = bottleneck = condition = 1,
/// kernel 1, ungated, layer1x1 active, no head1x1, biasless kernel-1 head.
fn one_wide_json(extra: &str) -> String {
    format!(
        r#"{{"layers": [{{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1, "bottleneck": 1,
            "dilations": [1], "kernel_size": 1,
            "activation": "Tanh", "gating_mode": "none",
            "head_bias": false,
            "layer1x1": {{"active": true, "groups": 1}}{extra} }}],
            "head": null, "head_scale": 1.0}}"#
    )
}

/// Scale/shift of a 1-wide, group-1, biased FiLM conv fed condition `a`,
/// weight layout `[w_scale, w_shift, b_scale, b_shift]`.
fn film1(w: &[f32; 4], a: f32) -> (f32, f32) {
    ((w[0] * a) + w[2], (w[1] * a) + w[3])
}

// -- Hand-computed modulation --------------------------------------------------

/// conv_post FiLM with scale + shift, hand-computed: the conv output
/// (after its bias) is modulated `z * scale + shift` before the mixin sum,
/// with scale/shift each an affine map of the condition (reference
/// `FiLM::Process`: one biased conv producing 2*width channels, scale on
/// top, shift on the bottom).
#[test]
fn film_conv_post_scale_shift_hand_computed() {
    let c0 = 0.5f32; // conv 1x1
    let cb = 0.05f32; // conv bias
    let m = 0.6f32; // mixin 1x1
    let l = [0.7f32, 0.01]; // layer1x1 w + bias (residual only)
    let f = [0.8f32, -0.4, 1.1, 0.07]; // conv_post film [ws, wh, bs, bh]
    let h = 0.9f32; // head_rechannel
    let s = 2.0f32; // head_scale

    let weights = [c0, cb, m, l[0], l[1], f[0], f[1], f[2], f[3], h, s];
    let config = one_wide_json(r#", "conv_post_film": {"shift": true, "groups": 1}"#);
    let mut model =
        load_nam("conv_post", &new_format_json(&config, &weights)).expect("model must load");

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = x; // no rechannel (1 -> 1); condition = layer input
        let (scale, shift) = film1(&f, a);
        let z = ((c0 * a) + cb) * scale + shift; // conv + bias, then FiLM
        let z = z + m * a; // mixin sum AFTER conv_post FiLM
        let t = fast_tanh(z);
        let expected = (h * fast_tanh(t)) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "conv_post scale+shift mismatch (got {out}, expected {expected})"
        );
    }
}

/// activation_post FiLM with scale only: `shift: false` halves the conv
/// output (width, not 2*width) and the modulation is a pure per-channel
/// scale of the activated z.
#[test]
fn film_activation_post_scale_only_hand_computed() {
    let c0 = 0.5f32;
    let cb = 0.05f32;
    let m = 0.6f32;
    let l = [0.7f32, 0.01];
    let f = [0.9f32, 0.3]; // activation_post film, scale only: [w, b]
    let h = 0.9f32;
    let s = 2.0f32;

    let weights = [c0, cb, m, l[0], l[1], f[0], f[1], h, s];
    let config = one_wide_json(r#", "activation_post_film": {"shift": false, "groups": 1}"#);
    let mut model =
        load_nam("act_post", &new_format_json(&config, &weights)).expect("model must load");

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = x;
        let z = ((c0 * a) + cb) + m * a;
        let t = fast_tanh(z) * ((f[0] * a) + f[1]); // scale-only FiLM
        let expected = (h * fast_tanh(t)) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "activation_post scale-only mismatch (got {out}, expected {expected})"
        );
    }
}

// -- Weight order across all 8 sites -------------------------------------------

/// All 8 FiLM sites active on a 1-wide layer with layer1x1 AND head1x1,
/// every tensor holding distinct values, hand-computed end to end. Pins the
/// reference weight order (`Layer::set_weights_`: conv, input_mixin,
/// layer1x1, head1x1, then conv_pre/conv_post/input_mixin_pre/
/// input_mixin_post/activation_pre/activation_post/layer1x1_post/
/// head1x1_post films) and each site's application point. Also pins the
/// reference quirk that under `gating_mode: none` the layer1x1_post film's
/// weights ARE consumed but the modulation is NOT applied (`Layer::Process`
/// only applies it in the BLENDED branch): if the engine skipped those 4
/// weights, the head1x1_post film would read them and the expectation would
/// fail; if it applied them, the residual (fed back through... nothing here,
/// single layer, but the skip math below assumes no modulation) would
/// change nothing — the applied-or-not distinction is pinned separately in
/// `film_layer1x1_post_only_applies_when_blended`.
#[test]
fn film_weight_order_all_sites_hand_computed() {
    let c0 = 0.45f32;
    let cb = 0.03f32;
    let m = 0.55f32;
    let l = [0.65f32, 0.02]; // layer1x1 w + bias
    let hx = [0.75f32, -0.04]; // head1x1 w + bias
    let f_cp = [0.30f32, -0.20, 1.05, 0.02]; // conv_pre
    let f_cpo = [-0.25f32, 0.15, 0.95, -0.03]; // conv_post
    let f_mp = [0.20f32, 0.10, 1.10, 0.04]; // input_mixin_pre
    let f_mpo = [0.35f32, -0.15, 0.90, 0.05]; // input_mixin_post
    let f_ap = [-0.30f32, 0.25, 1.15, -0.02]; // activation_pre
    let f_apo = [0.40f32, 0.20, 0.85, 0.03]; // activation_post
    let f_lp = [9.0f32, 9.0, 9.0, 9.0]; // layer1x1_post (consumed, NOT applied)
    let f_hp = [0.15f32, -0.10, 1.20, 0.06]; // head1x1_post
    let hr = 0.95f32; // head_rechannel
    let s = 1.5f32; // head_scale

    let mut weights = vec![c0, cb, m, l[0], l[1], hx[0], hx[1]];
    for f in [&f_cp, &f_cpo, &f_mp, &f_mpo, &f_ap, &f_apo, &f_lp, &f_hp] {
        weights.extend_from_slice(f);
    }
    weights.push(hr);
    weights.push(s);
    assert_eq!(weights.len(), 41);

    let film = r#"{"shift": true, "groups": 1}"#;
    let config = one_wide_json(&format!(
        r#", "head1x1": {{"active": true, "out_channels": 1, "groups": 1}},
           "conv_pre_film": {film}, "conv_post_film": {film},
           "input_mixin_pre_film": {film}, "input_mixin_post_film": {film},
           "activation_pre_film": {film}, "activation_post_film": {film},
           "layer1x1_post_film": {film}, "head1x1_post_film": {film}"#
    ));
    let mut model =
        load_nam("all_sites", &new_format_json(&config, &weights)).expect("model must load");

    let apply = |f: &[f32; 4], a: f32, v: f32| {
        let (scale, shift) = film1(f, a);
        v * scale + shift
    };
    for &x in &[0.2f32, -0.35, 0.6] {
        let a = x; // condition (no rechannel)
        let xin = apply(&f_cp, a, a); // conv_pre: modulated conv input
        let z = (c0 * xin) + cb;
        let z = apply(&f_cpo, a, z); // conv_post
        let condm = apply(&f_mp, a, a); // input_mixin_pre: modulated condition
        let mb = apply(&f_mpo, a, m * condm); // input_mixin_post
        let z = z + mb;
        let z = apply(&f_ap, a, z); // activation_pre
        let t = apply(&f_apo, a, fast_tanh(z)); // activation_post
        // Skip: head1x1 (with bias) then head1x1_post FiLM.
        let skip = apply(&f_hp, a, (hx[0] * t) + hx[1]);
        // layer1x1_post is NOT applied under gating none (residual unused
        // for the output here anyway — single layer).
        let expected = (hr * fast_tanh(skip)) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "all-sites weight order mismatch (got {out}, expected {expected})"
        );
    }
}

// -- layer1x1_post: blended-only application -----------------------------------

fn gating_film_json(gating_mode: &str) -> String {
    format!(
        r#"{{"layers": [{{
            "input_size": 1, "condition_size": 1, "head_size": 1,
            "channels": 1, "bottleneck": 1,
            "dilations": [1, 1], "kernel_size": 1,
            "activation": "Tanh", "gating_mode": "{gating_mode}",
            "secondary_activation": "Sigmoid",
            "head_bias": false,
            "layer1x1": {{"active": true, "groups": 1}},
            "layer1x1_post_film": {{"shift": true, "groups": 1}} }}],
            "head": null, "head_scale": 1.0}}"#
    )
}

/// Two-layer weights for `gating_film_json` differing only in the
/// layer1x1_post film tensor. Two layers so the layer-1 residual (the only
/// thing layer1x1_post modulates) reaches the output through layer 2.
fn gated_blended_weights(film: [f32; 4]) -> Vec<f32> {
    let per_layer_pre = [
        0.5f32, -0.3, // conv 2x1 (primary + secondary halves)
        0.05, 0.1, // conv bias
        0.6, -0.2, // mixin 2x1
        0.7, 0.01, // layer1x1 w + bias
    ];
    let mut w = Vec::new();
    for _ in 0..2 {
        w.extend_from_slice(&per_layer_pre);
        w.extend_from_slice(&film);
    }
    w.push(0.9); // head_rechannel
    w.push(2.0); // head_scale
    w
}

/// Reference-exact application scope of layer1x1_post_film: `Layer::Process`
/// (NAM/wavenet/model.cpp) applies it ONLY in the BLENDED gating branch.
/// Under `gated`, the film weights are consumed but the modulation is
/// skipped — so two models differing only in that tensor must be
/// bit-identical. Under `blended`, the same two models must diverge.
#[test]
fn film_layer1x1_post_only_applies_when_blended() {
    let film_a = [0.4f32, -0.3, 1.5, 0.2];
    let film_b = [-0.8f32, 0.6, 0.5, -0.4];
    let samples = [0.3f32, -0.45, 0.7, 0.1];

    for (mode, expect_applied) in [("gated", false), ("blended", true)] {
        let config = gating_film_json(mode);
        let mut model_a = load_nam(
            &format!("{mode}_a"),
            &new_format_json(&config, &gated_blended_weights(film_a)),
        )
        .expect("model A must load");
        let mut model_b = load_nam(
            &format!("{mode}_b"),
            &new_format_json(&config, &gated_blended_weights(film_b)),
        )
        .expect("model B must load");

        let mut any_diff = false;
        for &x in &samples {
            let out_a = model_a.process_sample(x);
            let out_b = model_b.process_sample(x);
            if expect_applied {
                any_diff |= out_a.to_bits() != out_b.to_bits();
            } else {
                assert_eq!(
                    out_a.to_bits(),
                    out_b.to_bits(),
                    "gated layer1x1_post film must be consumed but NOT applied"
                );
            }
        }
        if expect_applied {
            assert!(
                any_diff,
                "blended layer1x1_post film must modulate the residual"
            );
        }
    }
}

// -- Per-site width pins -------------------------------------------------------

/// Direct engine config for the width pins: gated layer (mid = 2 *
/// bottleneck), distinct channel counts everywhere so each site's width is
/// unambiguous: channels 2, bottleneck 2 (mid 4), condition 3, head1x1 out
/// 6.
fn width_pin_config(films: LayerFilms, gating: GatingMode) -> WaveNetConfig {
    let secondary = match gating {
        GatingMode::None => None,
        _ => Some(ActivationConfig::simple(ActivationKind::Sigmoid)),
    };
    WaveNetConfig {
        input_size: 1,
        stacks: vec![StackConfig {
            input_size: 1,
            condition_size: 3,
            head_size: 1,
            head_kernel_size: 1,
            head_dilation: 1,
            head_bias: false,
            channels: 2,
            bottleneck: 2,
            dilations: vec![1],
            kernel_sizes: vec![1],
            activations: vec![ActivationConfig::simple(ActivationKind::Tanh)],
            gating_modes: vec![gating],
            secondary_activations: vec![secondary],
            groups_input: 1,
            groups_input_mixin: 1,
            layer1x1_groups: 1,
            head1x1: Head1x1Params {
                active: true,
                out_channels: 6,
                groups: 1,
            },
            films,
        }],
        head: vec![],
        head_size: 1,
        has_layer1x1: true,
        condition_dsp: None,
    }
}

/// Weight count of `width_pin_config` with no films: rechannel 1x2 = 2,
/// conv [mid x 2 x 1] = 2*mid, conv bias mid, mixin [mid x 3] = 3*mid,
/// layer1x1 2x2+2 = 6, head1x1 6x2+6 = 18, head_rechannel 1x6 = 6 (no
/// bias), head_scale 1.
fn width_pin_base(mid: usize) -> usize {
    2 + 2 * mid + mid + 3 * mid + 6 + 18 + 6 + 1
}

fn assert_film_count(name: &str, config: WaveNetConfig, total: usize) {
    let weights = counted_weights(total);
    let mut reader = WeightReader::new(&weights);
    WaveNetModel::from_config_and_weights(config, &mut reader)
        .unwrap_or_else(|e| panic!("{name}: construction failed: {e}"));
    assert_eq!(reader.remaining(), 0, "{name}: exact weight count");
}

/// Per-site modulated widths, reference `Layer` ctor (NAM/wavenet/detail.h):
/// conv_pre = channels; conv_post / input_mixin_post / activation_pre =
/// the conv output width (2*bottleneck when gated, else bottleneck);
/// input_mixin_pre = condition_size; activation_post = bottleneck;
/// layer1x1_post = channels; head1x1_post = head1x1.out_channels. A FiLM of
/// width W with condition C costs `C * (shift ? 2W : W) / groups + (shift ?
/// 2W : W)` weights.
#[test]
fn film_per_site_widths_match_reference() {
    let on = FilmParams {
        active: true,
        shift: true,
        groups: 1,
    };
    // (site name, films, gated width, ungated width)
    type Site = (&'static str, fn(FilmParams) -> LayerFilms, usize, usize);
    let sites: [Site; 8] = [
        ("conv_pre", |f| LayerFilms { conv_pre: f, ..Default::default() }, 2, 2),
        ("conv_post", |f| LayerFilms { conv_post: f, ..Default::default() }, 4, 2),
        ("input_mixin_pre", |f| LayerFilms { input_mixin_pre: f, ..Default::default() }, 3, 3),
        ("input_mixin_post", |f| LayerFilms { input_mixin_post: f, ..Default::default() }, 4, 2),
        ("activation_pre", |f| LayerFilms { activation_pre: f, ..Default::default() }, 4, 2),
        ("activation_post", |f| LayerFilms { activation_post: f, ..Default::default() }, 2, 2),
        ("layer1x1_post", |f| LayerFilms { layer1x1_post: f, ..Default::default() }, 2, 2),
        ("head1x1_post", |f| LayerFilms { head1x1_post: f, ..Default::default() }, 6, 6),
    ];
    for (name, films_of, gated_w, ungated_w) in sites {
        for (gating, mid, w) in [
            (GatingMode::Gated, 4, gated_w),
            (GatingMode::None, 2, ungated_w),
        ] {
            // shift: true -> the FiLM conv outputs 2*w channels.
            let film_count = 3 * (2 * w) + 2 * w;
            assert_film_count(
                &format!("{name} ({gating:?})"),
                width_pin_config(films_of(on), gating),
                width_pin_base(mid) + film_count,
            );
        }
    }
    // Scale-only halves the FiLM conv output: conv_post ungated, width 2,
    // shift false -> out 2 -> 3*2 + 2 = 8 weights.
    let scale_only = FilmParams {
        active: true,
        shift: false,
        groups: 1,
    };
    assert_film_count(
        "conv_post scale-only",
        width_pin_config(
            LayerFilms {
                conv_post: scale_only,
                ..Default::default()
            },
            GatingMode::None,
        ),
        width_pin_base(2) + 8,
    );
}

// -- Grouped FiLM --------------------------------------------------------------

/// Grouped conv_post FiLM hand-computed on a 2-channel layer: with groups 2
/// and shift, the 4 scale/shift rows split into two blocks — rows 0-1
/// (scales) read condition channel 0, rows 2-3 (shifts) read condition
/// channel 1 (reference `Conv1x1::set_weights_` block-diagonal layout).
#[test]
fn film_grouped_conv_hand_computed() {
    let r = [0.3f32, -0.4]; // rechannel 2x1
    let c = [0.5f32, 0.25, -0.3, 0.4]; // conv 2x2 (kernel 1)
    let cb = [0.05f32, -0.02]; // conv bias
    let m = [0.6f32, -0.5, 0.2, 0.45]; // mixin 2x2
    let l = [0.7f32, -0.2, 0.35, 0.45, 0.01, -0.01]; // layer1x1 2x2 + bias
    let fw = [0.8f32, -0.6, 0.5, 0.9]; // film conv, groups 2: g0 rows 0-1, g1 rows 2-3
    let fb = [0.1f32, -0.05, 0.02, 0.15]; // film bias [scale0, scale1, shift0, shift1]
    let h = [0.9f32, -0.7]; // head_rechannel 1x2
    let s = 2.0f32;

    let mut weights = Vec::new();
    for part in [&r[..], &c, &cb, &m, &l, &fw, &fb, &h] {
        weights.extend_from_slice(part);
    }
    weights.push(s);
    assert_eq!(weights.len(), 29);

    let config = r#"{"layers": [{
        "input_size": 1, "condition_size": 2, "head_size": 1,
        "channels": 2, "bottleneck": 2,
        "dilations": [1], "kernel_size": 1,
        "activation": "Tanh", "gating_mode": "none",
        "head_bias": false,
        "layer1x1": {"active": true, "groups": 1},
        "conv_post_film": {"shift": true, "groups": 2} }],
        "head": null, "head_scale": 1.0}"#;
    let mut model =
        load_nam("grouped", &new_format_json(config, &weights)).expect("model must load");

    for &x in &[0.25f32, -0.5, 0.75] {
        let a = [r[0] * x, r[1] * x]; // rechannel; also the condition
        let z = [
            (c[0] * a[0] + c[1] * a[1]) + cb[0],
            (c[2] * a[0] + c[3] * a[1]) + cb[1],
        ];
        // Grouped film conv: block-diagonal over the 4 output rows.
        let scale = [fw[0] * a[0] + fb[0], fw[1] * a[0] + fb[1]];
        let shift = [fw[2] * a[1] + fb[2], fw[3] * a[1] + fb[3]];
        let z = [z[0] * scale[0] + shift[0], z[1] * scale[1] + shift[1]];
        let z = [
            z[0] + (m[0] * a[0] + m[1] * a[1]),
            z[1] + (m[2] * a[0] + m[3] * a[1]),
        ];
        let t = [fast_tanh(z[0]), fast_tanh(z[1])];
        let sk = [fast_tanh(t[0]), fast_tanh(t[1])];
        let expected = (h[0] * sk[0] + h[1] * sk[1]) * s;
        let out = model.process_sample(x);
        assert_eq!(
            out.to_bits(),
            expected.to_bits(),
            "grouped conv_post film mismatch (got {out}, expected {expected})"
        );
    }
}

// -- Inactive = identity -------------------------------------------------------

/// A config listing every film block as explicitly inactive (object with
/// `active: false`, literal `false`, and `null` forms) must consume the
/// same weights and produce bit-identical output to the same config with
/// no film fields at all (A1 path untouched).
#[test]
fn film_inactive_is_bit_identical_to_no_film() {
    let base = r#""input_size": 1, "condition_size": 1, "head_size": 1,
        "channels": 2, "bottleneck": 2,
        "dilations": [1, 2], "kernel_size": 2,
        "activation": "Tanh", "gating_mode": "none",
        "head_bias": false,
        "layer1x1": {"active": true, "groups": 1}"#;
    let plain = format!(r#"{{"layers": [{{ {base} }}], "head": null, "head_scale": 1.0}}"#);
    let inactive = format!(
        r#"{{"layers": [{{ {base},
            "conv_pre_film": {{"active": false}}, "conv_post_film": false,
            "input_mixin_pre_film": null, "input_mixin_post_film": {{"active": false, "shift": true, "groups": 2}},
            "activation_pre_film": false, "activation_post_film": {{"active": false}},
            "layer1x1_post_film": null, "head1x1_post_film": false }}],
            "head": null, "head_scale": 1.0}}"#
    );
    // rechannel 2 + 2 layers x (conv 2*2*2=8 + bias 2 + mixin 2 + l1x1 6)
    // + head_rechannel 2 + head_scale 1 = 41.
    let weights = counted_weights(41);
    let mut plain_model =
        load_nam("plain", &new_format_json(&plain, &weights)).expect("plain model must load");
    let mut inactive_model = load_nam("inactive", &new_format_json(&inactive, &weights))
        .expect("inactive-films model must load");

    for i in 0..64 {
        let x = ((i as f32) * 0.37).sin() * 0.8;
        let a = plain_model.process_sample(x);
        let b = inactive_model.process_sample(x);
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "inactive FiLM must leave the signal bit-identical (sample {i})"
        );
    }
}

// -- Validation errors ---------------------------------------------------------

/// head1x1_post_film without an active head1x1 is a construction error
/// (reference Layer ctor: "Do not use post-head 1x1 FiLM if there is no
/// head 1x1").
#[test]
fn film_head1x1_post_requires_head1x1() {
    let config = one_wide_json(r#", "head1x1_post_film": {"shift": true, "groups": 1}"#);
    let err = load_nam("no_h1x1", &new_format_json(&config, &counted_weights(16)))
        .err()
        .expect("head1x1_post_film without head1x1 must fail");
    assert!(err.contains("head1x1_post_film"), "{err}");
}

/// layer1x1_post_film without a layer1x1 is a construction error (reference
/// Layer ctor; the typed A2 parse rejects it too — this pins the engine
/// path, reachable through old-format configs with `has_layer1x1: false`).
#[test]
fn film_layer1x1_post_requires_layer1x1() {
    let mut config = width_pin_config(
        LayerFilms {
            layer1x1_post: FilmParams {
                active: true,
                shift: true,
                groups: 1,
            },
            ..Default::default()
        },
        GatingMode::None,
    );
    config.has_layer1x1 = false;
    let weights = counted_weights(64);
    let mut reader = WeightReader::new(&weights);
    let err = WaveNetModel::from_config_and_weights(config, &mut reader)
        .err()
        .expect("layer1x1_post_film without layer1x1 must fail");
    assert!(err.contains("layer1x1_post_film"), "{err}");
}

/// FiLM conv groups must divide both the condition width and the scale/shift
/// output width (reference Conv1x1 ctor validation).
#[test]
fn film_groups_must_divide_widths() {
    // Scale/shift output width (2 * bottleneck = 4) not divisible by
    // groups 3 (condition 3 IS divisible).
    let config = width_pin_config(
        LayerFilms {
            conv_post: FilmParams {
                active: true,
                shift: true,
                groups: 3,
            },
            ..Default::default()
        },
        GatingMode::None,
    );
    let weights = counted_weights(64);
    let mut reader = WeightReader::new(&weights);
    let err = WaveNetModel::from_config_and_weights(config, &mut reader)
        .err()
        .expect("indivisible film output width must fail");
    assert!(err.contains("divisible"), "{err}");

    // condition_size 1 not divisible by groups 2 (out 4 IS divisible).
    let config = one_wide_json(r#", "conv_post_film": {"shift": true, "groups": 2}"#);
    let err = load_nam("bad_cond_groups", &new_format_json(&config, &counted_weights(32)))
        .err()
        .expect("indivisible film condition width must fail");
    assert!(err.contains("divisible"), "{err}");
}

/// The engine parse surfaces the film blocks with reference defaults: a
/// bare object means `{active: true, shift: true, groups: 1}`.
#[test]
fn film_parse_defaults_surface_in_engine_config() {
    let config: serde_json::Value = serde_json::from_str(&one_wide_json(
        r#", "conv_pre_film": {}, "activation_post_film": {"shift": false, "groups": 1}"#,
    ))
    .unwrap();
    let parsed = parse_wavenet_config(config).expect("config must parse");
    let films = &parsed.stacks[0].films;
    assert_eq!(
        films.conv_pre,
        FilmParams {
            active: true,
            shift: true,
            groups: 1
        }
    );
    assert_eq!(
        films.activation_post,
        FilmParams {
            active: true,
            shift: false,
            groups: 1
        }
    );
    assert_eq!(films.conv_post, FilmParams::default());
    assert_eq!(films.head1x1_post, FilmParams::default());
}
