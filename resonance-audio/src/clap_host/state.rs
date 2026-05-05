//! CLAP state extension: serialize and deserialize plugin state
//! through the host-supplied `clap_istream` / `clap_ostream` callbacks.
//! Also owns the activate/deactivate cycle that wraps `load_state` so
//! plugins re-run their own `initialize()` and pick up new persisted
//! fields.

use std::ffi::c_void;

use clap_sys::stream::{clap_istream, clap_ostream};

use super::instance::ClapInstance;

impl ClapInstance {
    /// Save the plugin's full state (params + persisted fields) via CLAP state extension.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let state_ext = self.state_ext?;
        let save_fn = unsafe { (*state_ext).save }?;

        let mut buf: Vec<u8> = Vec::new();

        unsafe extern "C" fn ostream_write(
            stream: *const clap_ostream,
            buffer: *const c_void,
            size: u64,
        ) -> i64 {
            let buf = &mut *((*stream).ctx as *mut Vec<u8>);
            let slice = std::slice::from_raw_parts(buffer as *const u8, size as usize);
            buf.extend_from_slice(slice);
            size as i64
        }

        let mut stream = clap_ostream {
            ctx: &mut buf as *mut Vec<u8> as *mut c_void,
            write: Some(ostream_write),
        };

        let ok = unsafe { save_fn(self.plugin, &mut stream) };
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

        unsafe extern "C" fn istream_read(
            stream: *const clap_istream,
            buffer: *mut c_void,
            size: u64,
        ) -> i64 {
            let ctx = &mut *((*stream).ctx as *mut IstreamCtx);
            let remaining = ctx.len - ctx.pos;
            let to_read = (size as usize).min(remaining);
            if to_read == 0 {
                return 0;
            }
            std::ptr::copy_nonoverlapping(ctx.data.add(ctx.pos), buffer as *mut u8, to_read);
            ctx.pos += to_read;
            to_read as i64
        }

        let mut ctx = IstreamCtx {
            data: data.as_ptr(),
            len: data.len(),
            pos: 0,
        };

        let mut stream = clap_istream {
            ctx: &mut ctx as *mut IstreamCtx as *mut c_void,
            read: Some(istream_read),
        };

        unsafe { load_fn(self.plugin, &mut stream) }
    }

    /// Load state with full lifecycle cycle: stop → deactivate → load → activate → start.
    /// This ensures `initialize()` runs again so the plugin picks up new persist fields.
    pub fn reload_with_state(&mut self, data: &[u8]) -> bool {
        if !self.active {
            return self.load_state(data);
        }

        // Stop processing
        if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
            unsafe { stop(self.plugin) };
        }
        // Deactivate
        if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
            unsafe { deactivate(self.plugin) };
        }

        self.active = false;

        // Load state
        let ok = self.load_state(data);
        if !ok {
            return false;
        }

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
