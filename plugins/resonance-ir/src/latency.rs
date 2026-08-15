//! What the plugin tells the user about its own latency (ba todo #1300,
//! audit finding I1).
//!
//! The convolution block size *is* the reported latency, and it used to
//! be derived from the sample rate alone: the user could neither choose
//! it nor see it. [`crate::params::IrParams::latency_mode`] fixed the
//! choosing; this module is the seeing. It is deliberately outside the
//! `editor` feature and free of egui so the readout can be asserted
//! without standing up a GUI — the editor's job is only to draw what
//! [`readout`] returns.

use crate::dsp::{self, LatencyMode};
use crate::params::IrParams;
use crate::viz::IrViz;

/// The latency lines the editor shows under the mode picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatencyReadout {
    /// The latency the plugin imposes right now, e.g.
    /// `"2.90 ms · 128 samples"`. `None` before the host has activated
    /// the plugin, when there is no sample rate to convert with and no
    /// block size in force.
    pub active: Option<String>,
    /// The latency the selected mode *will* impose, present only while it
    /// differs from `active` — i.e. while a mode change is waiting for the
    /// host to cycle the plugin. This is the "say so plainly" half: a
    /// runtime change is neither applied instantly nor silently ignored.
    pub pending: Option<String>,
}

/// Format a block size as the editor shows it.
fn describe(block_size: usize, sample_rate: f32) -> String {
    format!(
        "{:.2} ms · {} samples",
        dsp::latency_ms(block_size, sample_rate),
        block_size
    )
}

/// Build the readout from the selected mode and the engine's published
/// state. Pure: same inputs, same strings.
pub fn readout(params: &IrParams, viz: &IrViz) -> LatencyReadout {
    let Some((active_block, sample_rate)) = viz.engine_block() else {
        return LatencyReadout {
            active: None,
            pending: None,
        };
    };

    let selected = LatencyMode::from_index(params.latency_mode.value());
    let target = dsp::block_size_for(sample_rate, selected);

    LatencyReadout {
        active: Some(describe(active_block, sample_rate)),
        pending: (target != active_block).then(|| describe(target, sample_rate)),
    }
}
