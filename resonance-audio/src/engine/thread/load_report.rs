//! The engine loop's half of the DSP-load report: print what the audio
//! thread published (it only stores atomics; formatting and stderr are
//! this thread's job — code review ARCH-02 A2-1 / ARCH-05), naming the
//! plugin slot that lost the most blocks to a busy lock.

use std::collections::HashMap;

use crate::clap_host::PluginMap;
use crate::types::PluginInstanceId;

use super::HandlerCtx;

#[derive(Default)]
pub(super) struct LoadReporter {
    /// Sequence of the last report printed (see `cycle_load`).
    seen: u64,
    /// Each slot's lifetime lock-miss count at the last report.
    slot_lock_misses: HashMap<PluginInstanceId, u64>,
}

impl LoadReporter {
    /// Print the report published since the last call, if any.
    pub(super) fn poll(&mut self, ctx: &HandlerCtx) {
        let Some(report) = ctx.shared.cycle_report.take_new(&mut self.seen) else {
            return;
        };
        let mut line = crate::cycle_load::format_cycle_load_line(&report);
        let plugins = ctx.plugins();
        let worst = worst_lock_miss_slot(&plugins, &mut self.slot_lock_misses);
        if let (true, Some((instance_id, plugin_id, misses))) =
            (report.lock_misses_window > 0, worst)
        {
            line.push_str(&format!(
                " | most lock misses: {} (instance {instance_id}) {misses}",
                plugin_id.unwrap_or("?"),
            ));
        }
        tracing::info!("{line}");
    }
}

/// The plugin slot whose lock-miss count rose most since `seen` was last
/// updated — `(instance id, plugin id, misses)` — and bring `seen` up to
/// date for every live slot. `None` when no slot missed.
fn worst_lock_miss_slot<'a>(
    plugins: &'a PluginMap,
    seen: &mut HashMap<PluginInstanceId, u64>,
) -> Option<(PluginInstanceId, Option<&'a str>, u64)> {
    seen.retain(|id, _| plugins.contains_key(id));
    let mut worst = None;
    for (&id, slot) in plugins.iter() {
        let now = slot.lock_misses();
        let delta = now.saturating_sub(seen.insert(id, now).unwrap_or(0));
        if delta > 0 && worst.is_none_or(|(_, _, w)| delta > w) {
            worst = Some((id, slot.plugin_id(), delta));
        }
    }
    worst
}
