//! [`PluginSlot`]'s lock-miss accounting (code review RT-07): the
//! render's counted `try_lock`, and what the engine's load report needs
//! to name the slot that missed most.

use std::sync::atomic::Ordering;

use super::{PluginSlot, SyncClapInstance};

impl PluginSlot {
    /// The render's `try_lock` on this slot, counting a miss (code review
    /// RT-07): a live block that cannot take the lock skips the plugin, and
    /// the miss is what makes that visible. Audio-thread safe: one
    /// `try_lock`, and two relaxed increments on a miss.
    #[inline]
    pub fn try_lock_counted(&self) -> Option<parking_lot::MutexGuard<'_, SyncClapInstance>> {
        let guard = self.instance.try_lock();
        if guard.is_none() {
            self.lock_misses.fetch_add(1, Ordering::Relaxed);
            crate::cycle_load::record_plugin_lock_miss();
        }
        guard
    }

    /// Live blocks that skipped this slot on a busy lock, ever.
    pub fn lock_misses(&self) -> u64 {
        self.lock_misses.load(Ordering::Relaxed)
    }

    /// The plugin's descriptor id, read once at construction so the
    /// engine can name a slot without taking its lock (the load report
    /// names the slot with the most lock misses).
    pub fn plugin_id(&self) -> Option<&str> {
        self.plugin_id.as_deref()
    }
}
