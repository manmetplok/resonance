//! Force the kernel to commit physical pages for buffers that the audio
//! thread will be the first to touch.
//!
//! `vec![0.0f32; N]` (and `ringbuf::HeapRb::new(N)`) ask the allocator
//! for memory that's specialised onto Linux's `calloc` / anonymous
//! `mmap` path — the kernel hands back virtual zero pages and only
//! commits a physical page on first *write*. Doing those first writes
//! from the cpal output callback fires minor page faults inside the
//! real-time deadline; on a busy desktop that's enough to trip cpal
//! 0.17's `StreamError::BufferUnderrun` over and over for the first
//! second or two after `stream.play()`.
//!
//! `resonance-dsp::DelayLine::new` already pre-faults its own buffer
//! (commit f0de785). The mixer scratch the engine sends into the cpal
//! callback — per-track / per-bus / per-plugin-port / monitor — is
//! several hundred KB of cold pages that escaped that fix because it
//! lives in `engine/mod.rs`. This helper covers them with the same
//! one-store-per-4 KB-page trick.

/// Touch one element per 4 KB page in `slice` so the kernel commits the
/// backing physical pages before the audio thread runs. `write_volatile`
/// keeps the compiler from eliding the store on a buffer it knows was
/// just zero-allocated.
pub(crate) fn prefault_f32(slice: &mut [f32]) {
    const STRIDE: usize = 4096 / std::mem::size_of::<f32>();
    let len = slice.len();
    let ptr = slice.as_mut_ptr();
    let mut i = 0;
    while i < len {
        // SAFETY: `i < len` and `ptr` came from a `&mut [f32]` of length
        // `len`, so `ptr.add(i)` is in-bounds and aligned.
        unsafe { std::ptr::write_volatile(ptr.add(i), 0.0) };
        i += STRIDE;
    }
}
