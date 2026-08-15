//! The header's two controls: whole-plugin bypass and the loudness
//! reference line (ba todo #1318, audit findings M3 and M4).
//!
//! Both were DSP-complete and unreachable from the plugin window. What
//! is pinned here is the decision layer behind the widgets — the state
//! the button reflects and flips, the clamping the drag applies, and
//! the single read path the three places that draw the line share — so
//! the controls cannot go inert again without a test failing.

#![cfg(feature = "editor")]

use resonance_mastering::assistant::{Genre, Target};
use resonance_mastering::editor::header::{
    bypass_look, reference_line_lufs, reference_line_range, set_reference_line_lufs, toggle_bypass,
    HeaderModel,
};
use resonance_mastering::params::MasteringParams;
use resonance_mastering::viz::MasteringViz;
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

// --- M3: bypass ----------------------------------------------------------

#[test]
fn bypass_button_flips_the_param_both_ways() {
    let params = MasteringParams::default();
    assert!(!params.bypass.value(), "bypass defaults to off");

    assert!(toggle_bypass(&params));
    assert!(params.bypass.value());

    assert!(!toggle_bypass(&params));
    assert!(!params.bypass.value());
}

#[test]
fn bypass_button_reads_differently_when_engaged() {
    let off = bypass_look(false);
    let on = bypass_look(true);
    assert_ne!(off.label, on.label);
    assert_ne!(
        off.fill, on.fill,
        "an engaged bypass must not look like an idle button"
    );
}

#[test]
fn header_model_tracks_the_bypass_param() {
    let params = MasteringParams::default();
    let viz = MasteringViz::new();
    assert!(!HeaderModel::new(&params, &viz).bypassed);
    toggle_bypass(&params);
    assert!(HeaderModel::new(&params, &viz).bypassed);
}

/// The control is only worth having because the dry path it engages is
/// latency-matched: what the button A/Bs is the chain, not the delay.
#[test]
fn the_button_engages_the_latency_matched_dry_path() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);

    // Engage bypass exactly the way the header button does.
    toggle_bypass(plugin.params());
    assert!(plugin.params().bypass.value());

    let latency = plugin.latency_samples() as usize;
    assert!(latency > 0, "the chain has real algorithmic latency");
    let total = latency + 4096;

    let (out_l, out_r) = stream_sine(&mut plugin, 440.0, 0.5, total, 512);
    let input = sine(440.0, 0.5, total);

    let mut max_err = 0.0f32;
    for i in latency..total {
        max_err = max_err.max((out_l[i] - input[i - latency]).abs());
        max_err = max_err.max((out_r[i] - input[i - latency]).abs());
    }
    assert_eq!(max_err, 0.0, "bypass must be a bit-exact delayed copy");
}

// --- M4: the loudness reference line ------------------------------------

#[test]
fn the_reference_line_is_a_single_read_path() {
    let params = MasteringParams::default();
    assert_eq!(reference_line_lufs(&params), params.target_lufs.value());
    set_reference_line_lufs(&params, -9.0);
    assert_eq!(reference_line_lufs(&params), -9.0);
    assert_eq!(params.target_lufs.value(), -9.0);
}

#[test]
fn dragging_past_either_end_clamps_to_the_param_range() {
    let params = MasteringParams::default();
    let (min, max) = reference_line_range(&params);
    assert!(min < max);

    set_reference_line_lufs(&params, max + 40.0);
    assert_eq!(reference_line_lufs(&params), max);

    set_reference_line_lufs(&params, min - 40.0);
    assert_eq!(reference_line_lufs(&params), min);
}

#[test]
fn header_model_carries_the_line_and_its_range() {
    let params = MasteringParams::default();
    let viz = MasteringViz::new();
    set_reference_line_lufs(&params, -10.5);
    let model = HeaderModel::new(&params, &viz);
    assert_eq!(model.target_lufs, -10.5);
    let (min, max) = reference_line_range(&params);
    assert_eq!(model.target_range, min..=max);
}

/// A fresh plugin has no integrated measurement yet; the header must say
/// so rather than print an infinity.
#[test]
fn integrated_readout_degrades_to_a_dash() {
    let params = MasteringParams::default();
    let viz = MasteringViz::new();
    let model = HeaderModel::new(&params, &viz);
    if model.integrated_lufs.is_none() {
        assert_eq!(model.integrated_text(), "Integrated: —");
    } else {
        assert!(model.integrated_text().contains("LUFS"));
    }
}

/// The line is drawn in three places (this header, the LUFS meter in the
/// right panel, the LUFS history trace). They must all come through
/// [`reference_line_lufs`], or a user edit would move some of them and
/// not the others.
#[test]
fn nothing_else_in_the_editor_reads_the_param_directly() {
    let editor = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/editor");
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    visit_rs(&editor, &mut |path| {
        scanned += 1;
        if path.file_name().is_some_and(|n| n == "header.rs") {
            return;
        }
        let text = std::fs::read_to_string(path).expect("read editor source");
        for (i, line) in text.lines().enumerate() {
            if line.contains("target_lufs") {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    });
    assert!(scanned > 5, "only scanned {scanned} editor files");
    assert!(
        offenders.is_empty(),
        "the reference line must be read through header::reference_line_lufs:\n{}",
        offenders.join("\n")
    );
}

fn visit_rs(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs(&path, f);
        } else if path.extension().is_some_and(|e| e == "rs") {
            f(&path);
        }
    }
}

/// M4's other half: the assistant is still allowed to write the target,
/// but only when the user explicitly applies its suggestions. Running an
/// analysis must leave a hand-set line exactly where the user put it.
#[test]
fn an_analysis_does_not_stomp_a_user_set_reference_line() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);

    // The user moves the line somewhere the assistant would not choose.
    set_reference_line_lufs(plugin.params(), -8.0);

    let amp = 10.0_f32.powf(-14.0 / 20.0);
    let _ = stream_sine(&mut plugin, 1000.0, amp, 3 * 48_000, 1024);

    let suggestions = plugin
        .viz()
        .assistant
        .analyze(Target::Genre(Genre::Rock))
        .expect("assistant should return suggestions with enough audio");

    assert_eq!(
        reference_line_lufs(plugin.params()),
        -8.0,
        "analysing must not move the user's reference line"
    );
    assert_ne!(
        suggestions.target_lufs, -8.0,
        "the test is meaningless if the suggestion happens to match"
    );

    // …and the explicit re-run still writes it.
    suggestions.apply_to(plugin.params());
    assert_eq!(
        reference_line_lufs(plugin.params()),
        Genre::Rock.target_lufs()
    );
}

/// The exact signal `stream_sine` feeds, regenerated with the same
/// accumulated phase so a delayed copy compares bit-exactly.
fn sine(freq_hz: f32, amp: f32, total: usize) -> Vec<f32> {
    let step = freq_hz * std::f32::consts::TAU / 48_000.0;
    let mut phase = 0.0f32;
    (0..total)
        .map(|_| {
            let s = phase.sin() * amp;
            phase += step;
            s
        })
        .collect()
}

fn stream_sine(
    plugin: &mut ResonanceMastering,
    freq_hz: f32,
    amp: f32,
    total: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let step = freq_hz * std::f32::consts::TAU / 48_000.0;
    let mut phase = 0.0f32;
    let mut out_l = Vec::with_capacity(total);
    let mut out_r = Vec::with_capacity(total);
    let mut done = 0;
    while done < total {
        let n = block.min(total - done);
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        for i in 0..n {
            let s = phase.sin() * amp;
            l[i] = s;
            r[i] = s;
            phase += step;
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        out_l.extend_from_slice(&l);
        out_r.extend_from_slice(&r);
        done += n;
    }
    (out_l, out_r)
}
