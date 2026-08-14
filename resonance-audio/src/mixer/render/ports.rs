//! The multi-output instrument call: hand a CLAP instance one
//! `StereoBufMut` per declared output port, from the per-block scratch
//! pool, without allocating.

use crate::clap_host::{StereoBufMut, SyncClapInstance};
use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;

/// Multi-output instrument fan-out: zero the first `port_count` port
/// scratch pairs, build a contiguous `StereoBufMut` slice over them,
/// and run `process_multi`.
pub(crate) fn process_multi_port(
    inst: &mut SyncClapInstance,
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    port_count: usize,
    frames: usize,
) {
    let mut views: [Option<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] = Default::default();
    for (i, (pl, pr)) in port_scratch.iter_mut().take(port_count).enumerate() {
        pl[..frames].fill(0.0);
        pr[..frames].fill(0.0);
        views[i] = Some(StereoBufMut {
            left: &mut pl[..frames],
            right: &mut pr[..frames],
        });
    }
    // Build a contiguous slice of StereoBufMut for the CLAP call. We
    // know ports 0..port_count are Some.
    let mut slots: [std::mem::MaybeUninit<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] =
        [const { std::mem::MaybeUninit::uninit() }; MAX_PLUGIN_OUTPUT_PORTS];
    for i in 0..port_count {
        slots[i].write(views[i].take().unwrap());
    }
    // SAFETY: the first `port_count` slots are initialized above; the
    // slice only refers to those.
    let slice: &mut [StereoBufMut<'_>] = unsafe {
        std::slice::from_raw_parts_mut(slots.as_mut_ptr() as *mut StereoBufMut<'_>, port_count)
    };
    inst.0.process_multi(slice, frames);
    // Drop the initialized entries before the MaybeUninit array goes
    // out of scope.
    for slot in slots.iter_mut().take(port_count) {
        unsafe { slot.assume_init_drop() };
    }
}
