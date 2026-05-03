//! Plugin-facing params for the dither stage.

use resonance_plugin::*;

use crate::stages::dither::DitherConfig;

pub const PARAM_COUNT: usize = 3;

pub struct DitherParams {
    pub on: BoolParam,
    pub target_bits: IntParam,
    pub noise_shape: BoolParam,
}

impl DitherParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.target_bits,
            2 => &self.noise_shape,
            _ => &self.on,
        }
    }

    pub fn snapshot(&self) -> DitherConfig {
        DitherConfig {
            enabled: self.on.value(),
            target_bits: self.target_bits.value(),
            noise_shape: self.noise_shape.value(),
        }
    }
}

impl Default for DitherParams {
    fn default() -> Self {
        Self {
            on: BoolParam::new("dith_on", "Dither On", false),
            target_bits: IntParam::new(
                "dith_bits",
                "Dither Bits",
                16,
                IntRange::Linear { min: 16, max: 24 },
            ),
            noise_shape: BoolParam::new("dith_ns", "Dither Shape", false),
        }
    }
}
