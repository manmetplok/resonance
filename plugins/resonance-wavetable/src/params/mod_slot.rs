use resonance_plugin::*;

use crate::dsp::modulation::ModDest;

pub struct ModSlotParams {
    pub source: IntParam,
    pub destination: IntParam,
    pub amount: FloatParam,
}

impl ModSlotParams {
    pub(super) fn new(index: usize) -> Self {
        let src_id: &'static str = Box::leak(format!("mod_{}_src", index + 1).into_boxed_str());
        let src_name: &'static str =
            Box::leak(format!("Mod {} Source", index + 1).into_boxed_str());
        let dst_id: &'static str = Box::leak(format!("mod_{}_dst", index + 1).into_boxed_str());
        let dst_name: &'static str = Box::leak(format!("Mod {} Dest", index + 1).into_boxed_str());
        let amt_id: &'static str = Box::leak(format!("mod_{}_amt", index + 1).into_boxed_str());
        let amt_name: &'static str =
            Box::leak(format!("Mod {} Amount", index + 1).into_boxed_str());

        Self {
            source: IntParam::new(src_id, src_name, 0, IntRange::Linear { min: 0, max: 8 }),
            // Spans the label table rather than a literal, so appending a
            // destination cannot leave it unreachable from the picker.
            destination: IntParam::new(
                dst_id,
                dst_name,
                0,
                IntRange::Linear {
                    min: 0,
                    max: (ModDest::LABELS.len() - 1) as i32,
                },
            ),
            amount: FloatParam::new(
                amt_id,
                amt_name,
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}
