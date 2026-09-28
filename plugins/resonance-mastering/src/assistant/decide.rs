//! The decision engine (`resonance_mastering_assist::decide`, re-exported
//! whole), and the plugin's half of applying it: the param lookup by key
//! that [`Suggestions::apply_to`] writes through.

pub use resonance_mastering_assist::decide::*;

use crate::params::MasteringParams;

/// The plugin param whose string id is `key`.
pub fn param_by_key<'a>(
    params: &'a MasteringParams,
    key: &str,
) -> Option<&'a dyn resonance_plugin::Param> {
    (0..crate::PARAM_COUNT)
        .map(|i| params.param_at(i))
        .find(|p| p.id() == key)
}

impl ParamSink for MasteringParams {
    fn set_param(&self, key: &str, value: f32) {
        if let Some(param) = param_by_key(self, key) {
            param.set_plain(f64::from(value));
        }
    }
}
