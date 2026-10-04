//! Per-block input/output peak capture, the shape every metering plugin
//! (amp, IR, delay) reports to its editor (code review ARCH2-04: it was
//! defined three times).

/// Input/output peak magnitudes (linear) for one processed block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockPeaks {
    pub in_l: f32,
    pub in_r: f32,
    pub out_l: f32,
    pub out_r: f32,
}

