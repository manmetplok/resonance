/// Plugin parameters: master volume + per-pad volume, pan, and mute.

use nih_plug::prelude::*;
use nih_plug::formatters::v2s_f32_rounded;
use std::sync::Arc;

use crate::drum_map::NUM_PADS;

#[derive(Params)]
pub struct DrumParams {
    #[id = "master_volume"]
    pub master_volume: FloatParam,

    #[nested(array, group = "Pad")]
    pub pads: [PadParams; NUM_PADS],
}

impl Default for DrumParams {
    fn default() -> Self {
        Self {
            master_volume: FloatParam::new(
                "Master Volume",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(v2s_f32_rounded(2)),
            pads: Default::default(),
        }
    }
}

#[derive(Params)]
pub struct PadParams {
    #[id = "volume"]
    pub volume: FloatParam,

    #[id = "pan"]
    pub pan: FloatParam,

    #[id = "mute"]
    pub mute: BoolParam,
}

impl Default for PadParams {
    fn default() -> Self {
        Self {
            volume: FloatParam::new(
                "Volume",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(v2s_f32_rounded(2)),
            pan: FloatParam::new(
                "Pan",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(v2s_f32_rounded(2)),
            mute: BoolParam::new("Mute", false),
        }
    }
}
