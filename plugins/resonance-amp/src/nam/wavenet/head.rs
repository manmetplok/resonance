//! Dense (fully connected) head/output projection layer.

/// Dense (fully connected) layer for the head network.
pub(super) struct DenseLayer {
    pub(super) weight: Vec<f32>,
    pub(super) bias: Vec<f32>,
    pub(super) in_features: usize,
    pub(super) out_features: usize,
    pub(super) has_activation: bool,
}
