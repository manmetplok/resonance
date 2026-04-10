/// Plugin parameters: master volume + per-pad volume, pan, and mute.

use resonance_plugin::*;

use crate::drum_map::NUM_PADS;

pub struct DrumParams {
    pub master_volume: FloatParam,
    pub pads: [PadParams; NUM_PADS],
}

impl Default for DrumParams {
    fn default() -> Self {
        Self {
            master_volume: FloatParam::new(
                "master_volume",
                "Master Volume",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            pads: std::array::from_fn(|i| PadParams::new(i)),
        }
    }
}

pub struct PadParams {
    pub volume: FloatParam,
    pub pan: FloatParam,
    pub mute: BoolParam,
    /// Blend amount (0..1) for this pad's overhead contribution when
    /// summed into the Overhead output port. 1.0 = full level, 0.0 =
    /// completely muted from the overhead bus. Defaults to 1.0 so the
    /// plugin sounds the same on first instantiation as it did before
    /// the multi-output rewrite.
    pub oh_blend: FloatParam,
    /// Balance (0..1) between the pad's two close-mic banks. 0.5 is
    /// equal — used as the default so the pre-existing single-bank
    /// sound is preserved. 0.0 favours the "left" side (kick In or
    /// snare Top), 1.0 favours the "right" side (kick Out or snare
    /// Btm). Ignored for pads with fewer than two close-mic banks.
    pub balance: FloatParam,
}

impl PadParams {
    fn new(index: usize) -> Self {
        // Use leaked strings for unique static IDs per pad
        let vol_id: &'static str = Box::leak(format!("pad_{}_volume", index).into_boxed_str());
        let vol_name: &'static str = Box::leak(format!("Pad {} Volume", index).into_boxed_str());
        let pan_id: &'static str = Box::leak(format!("pad_{}_pan", index).into_boxed_str());
        let pan_name: &'static str = Box::leak(format!("Pad {} Pan", index).into_boxed_str());
        let mute_id: &'static str = Box::leak(format!("pad_{}_mute", index).into_boxed_str());
        let mute_name: &'static str = Box::leak(format!("Pad {} Mute", index).into_boxed_str());
        let oh_id: &'static str = Box::leak(format!("pad_{}_oh_blend", index).into_boxed_str());
        let oh_name: &'static str = Box::leak(format!("Pad {} OH Blend", index).into_boxed_str());
        let bal_id: &'static str = Box::leak(format!("pad_{}_balance", index).into_boxed_str());
        let bal_name: &'static str = Box::leak(format!("Pad {} Balance", index).into_boxed_str());

        Self {
            volume: FloatParam::new(
                vol_id,
                vol_name,
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            pan: FloatParam::new(
                pan_id,
                pan_name,
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            mute: BoolParam::new(mute_id, mute_name, false),
            oh_blend: FloatParam::new(
                oh_id,
                oh_name,
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            balance: FloatParam::new(
                bal_id,
                bal_name,
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}

impl Default for PadParams {
    fn default() -> Self {
        Self::new(0)
    }
}
