//! The editor under every algorithm (reverb-algorithms.md §4.6): the
//! tank view is per-algorithm (an energy ring for the FDN engines, the
//! figure of eight for Plate, the echo train for Spring), and every one
//! lays out at the window's minimum size and draws without panicking,
//! idle and with a live tail in the viz.

use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::dsp::Algorithm;
use resonance_reverb::editor::headless_editor;
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// The window's minimum and preferred sizes (`editor/mod.rs`): every
/// control fits at both.
const SIZES: [(f32, f32); 2] = [(800.0, 700.0), (1320.0, 780.0)];

/// A few blocks of a plucked burst through the plugin, so the viz holds
/// live energies, lengths and a tail.
fn excite(plugin: &mut ResonanceReverb) {
    plugin.initialize(SR, BLOCK as u32);
    let mut n = 0usize;
    for _ in 0..12 {
        let mut left: Vec<f32> = (n..n + BLOCK)
            .map(|i| {
                if i < 480 {
                    (i as f32 * 0.37).sin() * 0.8
                } else {
                    0.0
                }
            })
            .collect();
        let mut right = left.clone();
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, BLOCK, &mut EventIterator::empty(), None);
        n += BLOCK;
    }
}

/// The tank view's title, and one text only its own drawing paints.
fn expected(algorithm: Algorithm) -> (&'static str, fn(&str) -> bool) {
    match algorithm {
        Algorithm::Plate => ("PLATE TANK", |t| t.starts_with("AP1 ")),
        Algorithm::Spring => ("SPRINGS", |t| t.starts_with("A ") && t.ends_with(" ms")),
        Algorithm::Classic => ("FDN TANK", |t| t == "8 lines · Householder"),
        Algorithm::Ambience => ("FDN TANK", |t| t == "8 lines"),
        _ => ("FDN TANK", |t| t == "16 lines · in pairs"),
    }
}

#[test]
fn every_algorithm_draws_its_own_tank_view_at_every_size() {
    for &algorithm in Algorithm::BUILT {
        for live in [false, true] {
            let mut plugin = ResonanceReverb::new();
            plugin.params.algorithm.set_value(algorithm as i32);
            plugin.params.mix.set_value(1.0);
            if live {
                excite(&mut plugin);
            }
            for size in SIZES {
                let mut editor = headless_editor(&plugin, size);
                let frame = editor.settled();
                let what = format!("{algorithm:?} at {size:?} (live {live})");
                let (title, own) = expected(algorithm);
                assert!(frame.shows(title), "{what}: no `{title}`");
                assert!(
                    frame.texts.iter().any(|t| own(&t.text)),
                    "{what}: the tank view is not its own"
                );
                for id in ["algorithm", "mix", "decay", "size", "er_tail_balance"] {
                    assert!(frame.widget(id).is_some(), "{what}: `{id}` is not drawn");
                }
                let hidden = frame.hidden_widgets(1.0);
                assert!(hidden.is_empty(), "{what}: {hidden:?}");
            }
        }
    }
}

/// Switching the algorithm under an open editor redraws the tank view
/// for the new one on the next frame.
#[test]
fn the_tank_view_follows_an_algorithm_switch() {
    let plugin = ResonanceReverb::new();
    let mut editor = headless_editor(&plugin, SIZES[0]);
    for &algorithm in Algorithm::BUILT {
        plugin.params.algorithm.set_value(algorithm as i32);
        let frame = editor.settled();
        let (title, _) = expected(algorithm);
        assert!(frame.shows(title), "{algorithm:?}: no `{title}`");
    }
}
