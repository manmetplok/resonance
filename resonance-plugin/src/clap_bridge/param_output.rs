//! Reporting a parameter the plugin changed itself to the host, as CLAP
//! output parameter events (`HostHandle::announce_param_change`).
//!
//! A plugin's editor or browser writes straight into the param atomics it
//! shares with the DSP; the host never sees that write. An announced
//! param goes out as `GESTURE_BEGIN`, `PARAM_VALUE` and `GESTURE_END` —
//! one complete edit, which a host records as one undo step — from
//! `process()` (after the plugin returns) or from either `params.flush`.
//! The shared mirror the main thread reads is brought in step at the same
//! time, so the host's `get_value` agrees with the event.

use clack_plugin::events::event_types::{
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
};
use clack_plugin::prelude::*;
use clack_plugin::utils::Cookie;

use super::shared::ClapShared;
use crate::host::HostHandle;
use crate::param::Param;

/// Report every announced param as a complete edit into `out`.
///
/// `param` reads the plugin's own value for a slot. Realtime-safe: atomics
/// and `try_push` into the host's list, nothing allocates or locks. An
/// announcement that cannot go out now — the host's list is full, or a
/// state load is publishing into the mirror (it would be reported over
/// the loaded value) — is put back for the next block or flush.
pub(crate) fn report_announced<'p>(
    host: &HostHandle,
    shared: &ClapShared<'_>,
    param: impl Fn(usize) -> Option<&'p dyn Param>,
    out: &mut OutputEvents,
) {
    // Bounded: each slot is taken at most once per call (a re-armed one
    // stops the walk).
    for _ in 0..shared.param_metas.len() {
        let Some(slot) = host.take_announced() else {
            return;
        };
        let Some(p) = param(slot) else {
            continue;
        };
        let publishing = shared.param_publish_gen() & 1 == 1
            || shared.params_dirty.load(std::sync::atomic::Ordering::Acquire);
        if publishing {
            host.rearm_announced(slot);
            return;
        }
        let id = ClapId::new(shared.param_metas[slot].clap_id);
        let value = p.get_plain();
        let pushed = out.try_push(ParamGestureBeginEvent::new(0, id)).is_ok()
            && out
                .try_push(ParamValueEvent::new(
                    0,
                    id,
                    Pckn::match_all(),
                    value,
                    Cookie::empty(),
                ))
                .is_ok()
            && out.try_push(ParamGestureEndEvent::new(0, id)).is_ok();
        if !pushed {
            host.rearm_announced(slot);
            return;
        }
        shared.set_value(slot, value);
    }
}
