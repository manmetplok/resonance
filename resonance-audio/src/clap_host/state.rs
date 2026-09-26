//! CLAP state extension: serialize and deserialize plugin state
//! through the host-supplied `clap_istream` / `clap_ostream` callbacks.
//! Also owns the activate/deactivate cycle that wraps `load_state` so
//! plugins re-run their own `initialize()` and pick up new persisted
//! fields.

use std::ffi::c_void;

use clap_sys::stream::{clap_istream, clap_ostream};

use super::instance::ClapInstance;

/// Ceiling on the total state size a plugin may hand us through
/// `ostream_write`. `size` arrives from third-party code over the C ABI
/// and must never be trusted for allocation: without a ceiling one bogus
/// huge value drives unbounded `Vec` growth on the engine thread, ending
/// in an alloc-failure abort. 256 MiB is far beyond any real plugin
/// state (sampler banks included) while still failing fast on garbage.
///
/// Local to this file because the crate's `limits` module is about
/// engine capacities, not FFI hardening; this constant only exists to
/// bound the two stream callbacks below.
const MAX_STATE_BYTES: usize = 256 * 1024 * 1024;

impl ClapInstance {
    /// Save the plugin's full state (params + persisted fields) via CLAP state extension.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let state_ext = self.state_ext?;
        let save_fn = unsafe { (*state_ext).save }?;

        let mut buf: Vec<u8> = Vec::new();

        /// `clap_ostream.write`, invoked by the plugin from inside
        /// `state.save()`. stream.h contract: returns the number of
        /// bytes written, -1 on write error.
        ///
        /// Hardened against a misbehaving plugin: a null `buffer` with
        /// `size == 0` is a flush-style no-op (0 bytes written); a null
        /// `buffer` with a non-zero `size` reports -1 instead of being
        /// dereferenced (even `from_raw_parts(null, 0)` alone is UB, so
        /// every check precedes slice construction); a `size` that would
        /// push the accumulated state past [`MAX_STATE_BYTES`] reports
        /// -1 *before* the slice read or any allocation. The body runs
        /// under `catch_unwind` because a panic (Vec growth, slice ops)
        /// must not unwind across the C ABI into the plugin.
        unsafe extern "C" fn ostream_write(
            stream: *const clap_ostream,
            buffer: *const c_void,
            size: u64,
        ) -> i64 {
            std::panic::catch_unwind(|| {
                if size == 0 {
                    return 0;
                }
                if buffer.is_null() {
                    return -1;
                }
                // SAFETY: `stream` is the `clap_ostream` built by
                // `save_state` below; its ctx points at the local `buf`,
                // which outlives the `save_fn` call.
                let buf = unsafe { &mut *((*stream).ctx as *mut Vec<u8>) };
                if size > (MAX_STATE_BYTES - buf.len()) as u64 {
                    return -1;
                }
                // SAFETY: `buffer` is non-null and the plugin contracts
                // it to cover `size` bytes; `size` is now known sane
                // (bounded by MAX_STATE_BYTES), so the slice read and
                // the Vec growth it feeds are both bounded.
                let slice = unsafe { std::slice::from_raw_parts(buffer as *const u8, size as usize) };
                buf.extend_from_slice(slice);
                size as i64
            })
            .unwrap_or(-1)
        }

        let stream = clap_ostream {
            ctx: &mut buf as *mut Vec<u8> as *mut c_void,
            write: Some(ostream_write),
        };

        let ok = unsafe { save_fn(self.plugin, &stream) };
        if ok {
            Some(buf)
        } else {
            None
        }
    }

    /// Load plugin state from a byte buffer via CLAP state extension.
    pub fn load_state(&mut self, data: &[u8]) -> bool {
        let state_ext = match self.state_ext {
            Some(ext) => ext,
            None => return false,
        };
        let load_fn = match unsafe { (*state_ext).load } {
            Some(f) => f,
            None => return false,
        };

        struct IstreamCtx {
            data: *const u8,
            len: usize,
            pos: usize,
        }

        /// `clap_istream.read`, invoked by the plugin from inside
        /// `state.load()`. stream.h contract: returns the number of
        /// bytes read, 0 for end of file, -1 for a read error.
        ///
        /// Hardened against a misbehaving plugin: `size == 0` reads
        /// zero bytes (0 is also the only representable answer); a null
        /// `buffer` with a non-zero `size` reports -1 instead of being
        /// written through. The body runs under `catch_unwind` because
        /// a panic must not unwind across the C ABI into the plugin.
        unsafe extern "C" fn istream_read(
            stream: *const clap_istream,
            buffer: *mut c_void,
            size: u64,
        ) -> i64 {
            std::panic::catch_unwind(|| {
                if size == 0 {
                    return 0;
                }
                if buffer.is_null() {
                    return -1;
                }
                // SAFETY: `stream` is the `clap_istream` built by
                // `load_state` below; its ctx points at the local
                // `IstreamCtx`, which outlives the `load_fn` call.
                let ctx = unsafe { &mut *((*stream).ctx as *mut IstreamCtx) };
                let remaining = ctx.len - ctx.pos;
                let to_read = (size as usize).min(remaining);
                if to_read == 0 {
                    return 0;
                }
                // SAFETY: `buffer` is non-null and the plugin contracts
                // it to cover `size` bytes; `to_read <= size` and the
                // source range `pos..pos + to_read` stays inside the
                // `data..data + len` slice the ctx was built from.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        ctx.data.add(ctx.pos),
                        buffer as *mut u8,
                        to_read,
                    );
                }
                ctx.pos += to_read;
                to_read as i64
            })
            .unwrap_or(-1)
        }

        let mut ctx = IstreamCtx {
            data: data.as_ptr(),
            len: data.len(),
            pos: 0,
        };

        let stream = clap_istream {
            ctx: &mut ctx as *mut IstreamCtx as *mut c_void,
            read: Some(istream_read),
        };

        unsafe { load_fn(self.plugin, &stream) }
    }

    /// True while the plugin is activated (and so processed). False only
    /// after a failed (re)activation.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Load state with full lifecycle cycle: stop → deactivate → load → activate → start.
    /// This ensures `initialize()` runs again so the plugin picks up new persist fields,
    /// and the post-reactivation latency re-query picks up any latency change the new
    /// state implies (doc #260 finding #10).
    ///
    /// Returns true only when the plugin accepted the state *and* is
    /// active again. A rejected state still reactivates the plugin, with
    /// whatever state it held before (see [`Self::is_active`] to tell the
    /// two failures apart). An instance left deactivated by an earlier
    /// failure is loaded and then brought back up (code review ENG-02).
    pub fn reload_with_state(&mut self, data: &[u8]) -> bool {
        if !self.active {
            let loaded = self.load_state(data);
            return self.activate_and_start() && loaded;
        }
        self.cycle_activation(|inst| inst.load_state(data))
    }

    /// Deactivate → reactivate cycle without touching state. This is the
    /// CLAP-sanctioned safe point at which a plugin's latency may change:
    /// the engine thread runs it when the plugin called
    /// `clap_host_latency.changed()` or `clap_host.request_restart()`
    /// (see [`ClapInstance::take_host_restart_request`]), and the fresh
    /// latency is re-read on the way back up. Returns false — leaving
    /// the plugin deactivated — if reactivation fails.
    pub fn restart(&mut self) -> bool {
        if !self.active {
            return false;
        }
        self.cycle_activation(|_| true)
    }

    /// Shared activation cycle: stop → deactivate → `while_deactivated`
    /// → activate → re-query latency → start. Reactivation runs even when
    /// `while_deactivated` fails — a rejected state load leaves the
    /// plugin's previous state valid, and skipping it would leave the
    /// plugin silent for good. Returns false if either step failed; on a
    /// failed reactivation the plugin is left deactivated
    /// (`self.active == false`) and `Drop` skips the deactivate it would
    /// otherwise run.
    fn cycle_activation(&mut self, while_deactivated: impl FnOnce(&mut Self) -> bool) -> bool {
        // Stop processing
        if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
            unsafe { stop(self.plugin) };
        }
        // Deactivate
        if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
            unsafe { deactivate(self.plugin) };
        }

        self.active = false;

        let ok = while_deactivated(self);
        self.activate_and_start() && ok
    }

    /// activate → re-query latency → start, from the deactivated state.
    /// On failure the plugin is left deactivated and `false` returned.
    fn activate_and_start(&mut self) -> bool {
        // Reactivate
        if let Some(activate) = unsafe { (*self.plugin).activate } {
            let ok = unsafe { activate(self.plugin, self.sample_rate as f64, 32, 8192) };
            if !ok {
                return false;
            }
        }
        // Mark active immediately after successful activate,
        // so Drop will properly deactivate even if start_processing fails
        self.active = true;

        // Latency may only change across a deactivate → reactivate
        // boundary (and the bridge serves an activation-time cache while
        // active — todo #1125), so this is exactly where the fresh value
        // becomes readable.
        self.requery_latency();

        // Start processing
        if let Some(start) = unsafe { (*self.plugin).start_processing } {
            let ok = unsafe { start(self.plugin) };
            if !ok {
                // Deactivate since we can't start processing
                if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
                    unsafe { deactivate(self.plugin) };
                }
                self.active = false;
                return false;
            }
        }
        true
    }

    /// Reset plugin to clean state by cycling stop/start processing.
    /// Clears reverb tails, delay lines, model state, etc.
    pub fn reset_processing(&mut self) {
        if !self.active {
            return;
        }
        if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
            unsafe { stop(self.plugin) };
        }
        if let Some(start) = unsafe { (*self.plugin).start_processing } {
            unsafe { start(self.plugin) };
        }
    }
}
