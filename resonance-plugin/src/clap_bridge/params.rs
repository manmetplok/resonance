//! Parameter handling for the CLAP bridge.
//!
//! Covers both main-thread params (info, value<->text conversion, flush) and
//! the audio-processor params flush path.

use std::fmt::Write as FmtWrite;

use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams,
};
use clack_plugin::prelude::*;

use super::shared::{ClapAudioProcessor, ClapMainThread};
use crate::plugin::ResonancePlugin;

// ---------------------------------------------------------------------------
// Params extension (main thread)
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginMainThreadParams for ClapMainThread<'a, P> {
    fn count(&mut self) -> u32 {
        self.shared.visible_indices.len() as u32
    }

    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        let Some(&meta_idx) = self.shared.visible_indices.get(param_index as usize) else {
            return;
        };
        let meta = &self.shared.param_metas[meta_idx];
        let mut flags = ParamInfoFlags::IS_AUTOMATABLE;
        if meta.is_stepped {
            flags |= ParamInfoFlags::IS_STEPPED;
        }

        info.set(&ParamInfo {
            id: ClapId::new(meta.clap_id),
            name: meta.name.as_bytes(),
            // The param's group. Empty for an ungrouped param, which is
            // what CLAP expects; a `/`-separated path otherwise, which
            // hosts nest in their automation-lane picker (ba todo #1289).
            module: meta.module.as_bytes(),
            default_value: meta.default,
            min_value: meta.min,
            max_value: meta.max,
            flags,
            cookie: Default::default(),
        });
    }

    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        let slot = self.shared.find_slot(param_id.get())?;
        Some(self.shared.get_value(slot))
    }

    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> core::fmt::Result {
        if let Some(slot) = self.shared.find_slot(param_id.get()) {
            if let Some(plugin) = &self.plugin {
                if slot < plugin.param_count() {
                    let text = plugin.param(slot).display(value);
                    return write!(writer, "{}", text);
                }
            }
            // Active: the plugin object is in the audio processor, so ask
            // the text source it handed over at construction, if any.
            if let Some(text) = self
                .param_text_source
                .as_ref()
                .and_then(|s| s.display(slot, value))
            {
                return write!(writer, "{}", text);
            }
            write!(writer, "{:.2}", value)
        } else {
            write!(writer, "{:.2}", value)
        }
    }

    fn text_to_value(&mut self, param_id: ClapId, text: &std::ffi::CStr) -> Option<f64> {
        let slot = self.shared.find_slot(param_id.get())?;
        let text = text.to_str().ok()?;
        if let Some(plugin) = &self.plugin {
            if slot < plugin.param_count() {
                return plugin.param(slot).parse(text);
            }
        }
        self.param_text_source.as_ref()?.parse(slot, text)
    }

    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        _output_parameter_changes: &mut OutputEvents,
    ) {
        for event in input_parameter_changes {
            if let Some(core_event) = event.as_core_event() {
                use clack_plugin::events::spaces::CoreEventSpace;
                if let CoreEventSpace::ParamValue(e) = core_event {
                    if let Some(clap_id) = e.param_id() {
                        if let Some(slot) = self.shared.find_slot(clap_id.get()) {
                            // Store what the param LANDED on, not the raw
                            // wire value: `set_plain` clamps, rounds and
                            // demotes through f32, so storing `e.value()`
                            // leaves the atomics holding a number the
                            // plugin never adopted — and `get_value` reads
                            // the atomics, so the host would report and
                            // re-save an out-of-range or over-precise
                            // value. Same divergence `state.rs` closes on
                            // the load path.
                            let mut landed = e.value();
                            if let Some(plugin) = &self.plugin {
                                if slot < plugin.param_count() {
                                    let param = plugin.param(slot);
                                    param.set_plain(e.value());
                                    landed = param.get_plain();
                                }
                            }
                            self.shared.set_value(slot, landed);
                            self.shared.note_host_param_change();
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Params extension (audio processor)
// ---------------------------------------------------------------------------

impl<P: ResonancePlugin> PluginAudioProcessorParams for ClapAudioProcessor<'_, P> {
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        _output_parameter_changes: &mut OutputEvents,
    ) {
        for event in input_parameter_changes {
            if let Some(core_event) = event.as_core_event() {
                use clack_plugin::events::spaces::CoreEventSpace;
                if let CoreEventSpace::ParamValue(e) = core_event {
                    if let Some(clap_id) = e.param_id() {
                        if let Some(slot) = self.shared.find_slot(clap_id.get()) {
                            if slot < self.plugin.param_count() {
                                self.plugin.param(slot).set_plain(e.value());
                            }
                            self.shared.note_host_param_change();
                            // Mirror into the shared atomics, exactly as
                            // the `ParamValue` arm of `process()` does.
                            // While the plugin is active the main-thread
                            // `params.get_value` and `state.save` read
                            // from `shared` (the owned plugin lives in
                            // this audio processor and is unreachable
                            // from there), so without this a host that
                            // delivers a change via `flush` instead of
                            // `process` would move the DSP but still
                            // report and persist the old value.
                            self.shared.set_value(slot, e.value());
                        }
                    }
                }
            }
        }
    }
}
