//! The audio buffers handed to a plugin's `process()`: one
//! `clap_audio_buffer` per port the plugin **declared**, each with the
//! channel count it declared (code review HOST-05).
//!
//! `clap/process.h` ties `audio_inputs_count` / `audio_outputs_count` to
//! `clap_plugin_audio_ports.count()` and each buffer's `channel_count` to
//! its port's, and a plugin is entitled to index every port it declared.
//! The host used to pass one input (two with a routed key), at most as
//! many outputs as the caller had buffers for, and always two channels —
//! so a sidechain compressor with no key routed, or a 16-out sampler on
//! an 8-port track, read or wrote through null.
//!
//! Every channel the caller has no buffer for is backed here instead: an
//! unconnected input reads [`PortBuffers::silence`], an unwanted output
//! writes into a scratch channel of its own. All of it is sized at
//! activation from the plugin's `audio_ports.get()`; [`PortBuffers::bind`]
//! re-points every channel every block, so no pointer from an earlier
//! call (a render-pool worker's scratch, say) survives into the next.
//! Allocation-free after construction.

use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;

use super::StereoBufMut;

/// Upper bound on the ports per direction, and on the channels per port,
/// this host backs. Far past anything real (a 32-out sampler, a 7.1.4
/// bed), and it bounds what a plugin's claim can make us allocate.
pub(crate) const MAX_PORTS: usize = 64;
pub(crate) const MAX_PORT_CHANNELS: u32 = 64;
/// Distinct scratch channels backing unconnected outputs, at most (32 KiB
/// each at the 8192-frame activation maximum).
const MAX_SCRATCH_CHANNELS: usize = 64;

/// Pre-allocated `clap_audio_buffer` arrays and their channel backing for
/// one plugin instance.
pub(crate) struct PortBuffers {
    /// Declared channel count of each input / output port.
    in_channels: Vec<u32>,
    out_channels: Vec<u32>,
    /// The arrays handed to the plugin, one entry per declared port.
    in_bufs: Vec<clap_audio_buffer>,
    out_bufs: Vec<clap_audio_buffer>,
    /// Every port's channel pointers, flat; port `p` owns
    /// `[offset[p], offset[p] + channels[p])`.
    in_ptrs: Vec<*mut f32>,
    out_ptrs: Vec<*mut f32>,
    in_offset: Vec<usize>,
    out_offset: Vec<usize>,
    /// Write targets for the output channels the caller may not back
    /// (every one but the main port's first two — the caller always passes
    /// a main pair), `max_frames` long each. One per channel up to
    /// [`MAX_SCRATCH_CHANNELS`]; past that the rest share the last, so a
    /// plugin declaring hundreds of channels costs bounded memory.
    out_scratch: Vec<Vec<f32>>,
    /// Flat output channel → its `out_scratch` entry (unused for the main
    /// pair).
    out_scratch_index: Vec<usize>,
    /// What an unconnected input channel reads: zeros, re-zeroed for the
    /// block's frames before each call it is used in.
    silence: Vec<f32>,
    /// The main input's mono downmix, for a plugin whose main input port
    /// is mono (the track is always stereo).
    mono_in: Vec<f32>,
    /// The most frames one block may bind.
    max_frames: usize,
}

impl PortBuffers {
    /// Size everything for the declared layout. `in_channels[p]` /
    /// `out_channels[p]` are the channel counts of each port, already
    /// clamped to [`MAX_PORT_CHANNELS`] and at most [`MAX_PORTS`] long.
    pub(crate) fn new(in_channels: Vec<u32>, out_channels: Vec<u32>, max_frames: usize) -> Self {
        debug_assert!(in_channels.len() <= MAX_PORTS && out_channels.len() <= MAX_PORTS);
        let offsets = |channels: &[u32]| {
            let mut at = 0usize;
            channels
                .iter()
                .map(|&n| {
                    let o = at;
                    at += n as usize;
                    o
                })
                .collect::<Vec<_>>()
        };
        let in_offset = offsets(&in_channels);
        let out_offset = offsets(&out_channels);
        let in_total: usize = in_channels.iter().map(|&n| n as usize).sum();
        let out_total: usize = out_channels.iter().map(|&n| n as usize).sum();

        let mut out_scratch: Vec<Vec<f32>> = Vec::new();
        let mut out_scratch_index = Vec::with_capacity(out_total);
        for (p, &n) in out_channels.iter().enumerate() {
            for c in 0..n {
                let caller_backed = p == 0 && c < 2;
                if !caller_backed && out_scratch.len() < MAX_SCRATCH_CHANNELS {
                    out_scratch.push(vec![0.0; max_frames]);
                }
                out_scratch_index.push(out_scratch.len().saturating_sub(1));
            }
        }
        // Any input channel other than the main pair (a key port, a
        // third channel, a port past the key) may have to read silence.
        let main_in = in_channels.first().copied().unwrap_or(0);
        let needs_silence = in_total > main_in.min(2) as usize;
        let buffer = || clap_audio_buffer {
            data32: ptr::null_mut(),
            data64: ptr::null_mut(),
            channel_count: 0,
            latency: 0,
            constant_mask: 0,
        };
        Self {
            in_bufs: in_channels.iter().map(|_| buffer()).collect(),
            out_bufs: out_channels.iter().map(|_| buffer()).collect(),
            in_ptrs: vec![ptr::null_mut(); in_total],
            out_ptrs: vec![ptr::null_mut(); out_total],
            in_offset,
            out_offset,
            out_scratch,
            out_scratch_index,
            silence: if needs_silence {
                vec![0.0; max_frames]
            } else {
                Vec::new()
            },
            mono_in: if main_in == 1 {
                vec![0.0; max_frames]
            } else {
                Vec::new()
            },
            max_frames,
            in_channels,
            out_channels,
        }
    }

    /// Number of declared input ports.
    pub(crate) fn input_count(&self) -> usize {
        self.in_channels.len()
    }

    /// Number of declared output ports.
    pub(crate) fn output_count(&self) -> usize {
        self.out_channels.len()
    }

    /// The most frames [`Self::bind`] may be called with.
    pub(crate) fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Point every declared port at its backing for one block of
    /// `frames`:
    ///
    /// - output port `p < outputs.len()` writes its first two channels
    ///   into the caller's pair; every other output channel into its own
    ///   scratch;
    /// - input port 0 (main) reads the caller's main pair in place (CLAP
    ///   allows aliased in/out) — or, if mono, a downmix of it;
    /// - input port 1 reads `key` when one is given;
    /// - every other input channel reads silence, flagged in the port's
    ///   `constant_mask`.
    ///
    /// The caller has already clamped `frames` to every slice it passes
    /// and to [`Self::max_frames`]. `outputs` is non-empty.
    pub(crate) fn bind(
        &mut self,
        outputs: &mut [StereoBufMut<'_>],
        key: Option<(&[f32], &[f32])>,
        frames: usize,
    ) {
        debug_assert!(!outputs.is_empty());
        for p in 0..self.out_channels.len() {
            let n = self.out_channels[p];
            let off = self.out_offset[p];
            for c in 0..n as usize {
                self.out_ptrs[off + c] = match (outputs.get_mut(p), c) {
                    (Some(pair), 0) => pair.left.as_mut_ptr(),
                    (Some(pair), 1) => pair.right.as_mut_ptr(),
                    _ => self.out_scratch[self.out_scratch_index[off + c]].as_mut_ptr(),
                };
            }
            self.out_bufs[p] = clap_audio_buffer {
                data32: self.out_ptrs[off..].as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: n,
                latency: 0,
                constant_mask: 0,
            };
        }

        let main_l = outputs[0].left.as_mut_ptr();
        let main_r = outputs[0].right.as_mut_ptr();
        let silence = self.silence.as_mut_ptr();
        let mut silence_used = false;
        for p in 0..self.in_channels.len() {
            let n = self.in_channels[p];
            let off = self.in_offset[p];
            let mut constant: u64 = 0;
            for c in 0..n as usize {
                self.in_ptrs[off + c] = match (p, c, key) {
                    (0, 0, _) if n == 1 => {
                        let (l, r) = (&outputs[0].left[..frames], &outputs[0].right[..frames]);
                        for ((m, l), r) in self.mono_in[..frames].iter_mut().zip(l).zip(r) {
                            *m = 0.5 * (l + r);
                        }
                        self.mono_in.as_mut_ptr()
                    }
                    (0, 0, _) => main_l,
                    (0, 1, _) => main_r,
                    // Cast away const: CLAP's buffer struct is shared
                    // between inputs and outputs, so it has no const
                    // variant. An input port is read-only by contract.
                    (1, 0, Some((l, _))) => l.as_ptr() as *mut f32,
                    (1, 1, Some((_, r))) => r.as_ptr() as *mut f32,
                    _ => {
                        silence_used = true;
                        if c < 64 {
                            constant |= 1 << c;
                        }
                        silence
                    }
                };
            }
            self.in_bufs[p] = clap_audio_buffer {
                data32: self.in_ptrs[off..].as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: n,
                latency: 0,
                // Silence-backed channels are flagged constant (their
                // value is the first sample, 0). That is how a port with
                // nothing connected reads: the first-party bridge treats
                // a key port that is all constant zero as unrouted and
                // keys the plugin off its own input, as it did when the
                // host left the port out.
                constant_mask: constant,
            };
        }
        if silence_used {
            // A plugin is not meant to write its inputs; one that does
            // must not leak that into the next block's "silence".
            self.silence[..frames].fill(0.0);
        }
    }

    /// The input array for `clap_process`, or null with no input ports.
    pub(crate) fn inputs_ptr(&self) -> *const clap_audio_buffer {
        if self.in_bufs.is_empty() {
            ptr::null()
        } else {
            self.in_bufs.as_ptr()
        }
    }

    /// The output array for `clap_process`, or null with no output ports.
    pub(crate) fn outputs_ptr(&mut self) -> *mut clap_audio_buffer {
        if self.out_bufs.is_empty() {
            ptr::null_mut()
        } else {
            self.out_bufs.as_mut_ptr()
        }
    }

    /// After `process()`: a mono output port the caller backed only had
    /// its left buffer written; mirror it into the right. Returns how many
    /// of `outputs` the plugin produced (the rest it never declared).
    pub(crate) fn finish(&self, outputs: &mut [StereoBufMut<'_>], frames: usize) -> usize {
        let produced = outputs.len().min(self.out_channels.len());
        for (p, pair) in outputs.iter_mut().enumerate().take(produced) {
            if self.out_channels[p] == 1 {
                pair.right[..frames].copy_from_slice(&pair.left[..frames]);
            }
        }
        produced
    }
}
