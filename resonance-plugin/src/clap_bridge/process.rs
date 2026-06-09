//! Audio-processor lifecycle: activate, process, deactivate, reset.

use std::sync::atomic::Ordering;

use clack_plugin::prelude::*;

use super::shared::{ClapAudioProcessor, ClapMainThread, ClapShared};
use crate::plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin, TempoInfo};

impl<'a, P: ResonancePlugin> PluginAudioProcessor<'a, ClapShared<'a>, ClapMainThread<'a, P>>
    for ClapAudioProcessor<'a, P>
{
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        main_thread: &mut ClapMainThread<'a, P>,
        shared: &'a ClapShared<'a>,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let mut plugin = main_thread
            .plugin
            .take()
            .ok_or(PluginError::Message("Plugin not initialized"))?;

        // Sync param values from shared atomics to plugin's params
        for i in 0..plugin.param_count() {
            if i < shared.param_values.len() {
                plugin.param(i).set_plain(shared.get_value(i));
            }
        }

        plugin.initialize(
            audio_config.sample_rate as f32,
            audio_config.max_frames_count,
        );

        let max_frames = audio_config.max_frames_count as usize;
        let port_count = shared.output_ports.len();
        let output_scratch = (0..port_count)
            .map(|_| (vec![0.0_f32; max_frames], vec![0.0_f32; max_frames]))
            .collect();
        Ok(ClapAudioProcessor {
            plugin,
            shared,
            input_left: vec![0.0; max_frames],
            input_right: vec![0.0; max_frames],
            output_scratch,
            note_events: Vec::with_capacity(256),
        })
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        let frames = audio.frames_count() as usize;
        if frames == 0 {
            return Ok(ProcessStatus::ContinueIfNotQuiet);
        }

        // Handle input events: param changes and note events
        self.note_events.clear();
        for event in events.input {
            if let Some(core_event) = event.as_core_event() {
                use clack_plugin::events::spaces::CoreEventSpace;
                match core_event {
                    CoreEventSpace::ParamValue(e) => {
                        if let Some(clap_id) = e.param_id() {
                            let value = e.value();
                            if let Some(slot) = self.shared.find_slot(clap_id.get()) {
                                self.plugin.param(slot).set_plain(value);
                                self.shared.set_value(slot, value);
                            }
                        }
                    }
                    CoreEventSpace::NoteOn(e) => {
                        if let crate::Match::Specific(key) = e.key() {
                            self.note_events.push(NoteEvent::NoteOn {
                                note: key as u8,
                                velocity: e.velocity() as f32,
                                timing: e.header().time(),
                            });
                        }
                    }
                    CoreEventSpace::NoteOff(e) => {
                        if let crate::Match::Specific(key) = e.key() {
                            self.note_events.push(NoteEvent::NoteOff {
                                note: key as u8,
                                timing: e.header().time(),
                            });
                        }
                    }
                    CoreEventSpace::NoteChoke(e) => {
                        if let crate::Match::Specific(key) = e.key() {
                            self.note_events.push(NoteEvent::Choke {
                                note: key as u8,
                                timing: e.header().time(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        // Re-sync params from shared atomics if state was loaded while active
        if self.shared.params_dirty.swap(false, Ordering::Acquire) {
            for i in 0..self.plugin.param_count() {
                if i < self.shared.param_values.len() {
                    self.plugin.param(i).set_plain(self.shared.get_value(i));
                }
            }
        }

        // Push any editor-driven parameter writes back into the shared
        // atomics so the main-thread save path (which reads from `shared`
        // while the plugin is active) sees them. Editors update the
        // plugin's own atomic storage directly via the `Arc<Params>`
        // handle they were given, bypassing the CLAP event loop that
        // normally keeps `shared` in sync. Without this, project save
        // would persist stale default values for any param the user
        // only touched through the editor.
        //
        // This runs after `params_dirty` was handled above, so a newly
        // loaded state is not immediately clobbered.
        //
        // We compare-then-store rather than store-always so the audio
        // thread doesn't ping the cache line that the editor thread is
        // reading every block when nothing changed. For a plugin with
        // 50 params at 750 Hz block rate that's 37k spurious stores a
        // second; load-then-conditional-store is essentially free on
        // x86 when the value is unchanged.
        for i in 0..self.plugin.param_count() {
            if i < self.shared.param_values.len() {
                let plugin_v = self.plugin.param(i).get_plain();
                let shared_v = self.shared.get_value(i);
                if shared_v.to_bits() != plugin_v.to_bits() {
                    self.shared.set_value(i, plugin_v);
                }
            }
        }

        let tempo = process.transport.and_then(|t| {
            use clack_plugin::events::event_types::TransportFlags;
            if !t.flags.contains(TransportFlags::HAS_TEMPO) {
                return None;
            }
            Some(TempoInfo {
                bpm: t.tempo as f32,
                time_sig_num: t.time_signature_numerator,
                time_sig_den: t.time_signature_denominator,
                playing: t.flags.contains(TransportFlags::IS_PLAYING),
                song_pos_beats: t.song_pos_beats.to_float(),
            })
        });

        let mut event_iter = EventIterator::new(&self.note_events);

        // Effect path: read the input (port 0 of the input audio buffers)
        // into scratch. The plugin sees the input pre-loaded in its
        // `outputs[0]` buffer because the legacy effect contract is
        // "read left/right, process in place, write left/right". Non-main
        // output ports (1..N) are zeroed before the call.
        let input_left = &mut self.input_left[..frames];
        let input_right = &mut self.input_right[..frames];

        if P::INPUT_CHANNELS.is_some() {
            if let Some(mut pair) = audio.port_pair(0) {
                let mut channels = pair
                    .channels()?
                    .into_f32()
                    .ok_or(PluginError::Message("Expected f32 audio"))?;
                if let Some(ch) = channels.channel_pair(0) {
                    match ch {
                        ChannelPair::InPlace(buf) => input_left.copy_from_slice(&buf[..frames]),
                        ChannelPair::InputOutput(inp, _) => {
                            input_left.copy_from_slice(&inp[..frames])
                        }
                        _ => input_left.fill(0.0),
                    }
                }
                if let Some(ch) = channels.channel_pair(1) {
                    match ch {
                        ChannelPair::InPlace(buf) => input_right.copy_from_slice(&buf[..frames]),
                        ChannelPair::InputOutput(inp, _) => {
                            input_right.copy_from_slice(&inp[..frames])
                        }
                        _ => input_right.fill(0.0),
                    }
                }
            }
        } else {
            input_left.fill(0.0);
            input_right.fill(0.0);
        }

        // Zero every output scratch pair for this frame range, then seed
        // port 0 with the input so effect plugins see their audio in-place.
        for (idx, (l, r)) in self.output_scratch.iter_mut().enumerate() {
            l[..frames].fill(0.0);
            r[..frames].fill(0.0);
            if idx == 0 && P::INPUT_CHANNELS.is_some() {
                l[..frames].copy_from_slice(input_left);
                r[..frames].copy_from_slice(input_right);
            }
        }

        // Build a transient slice of OutputBuffer views over the scratch.
        // Uses a stack array to avoid heap allocation on the audio thread.
        // A plugin declaring more ports than the array holds is a plugin
        // bug: caught by the debug_assert in debug builds, and by the
        // array's bounds check (panic, not silent truncation) in release.
        const MAX_OUTPUT_PORTS: usize = 8;
        debug_assert!(
            self.output_scratch.len() <= MAX_OUTPUT_PORTS,
            "plugin declares {} output ports; the CLAP bridge supports at most {MAX_OUTPUT_PORTS}",
            self.output_scratch.len(),
        );
        let mut port_views_arr: [std::mem::MaybeUninit<OutputBuffer<'_>>; MAX_OUTPUT_PORTS] =
            [const { std::mem::MaybeUninit::uninit() }; MAX_OUTPUT_PORTS];
        let mut port_views_len = 0;
        for (l, r) in self.output_scratch.iter_mut() {
            port_views_arr[port_views_len].write(OutputBuffer {
                left: &mut l[..frames],
                right: &mut r[..frames],
            });
            port_views_len += 1;
        }
        // SAFETY: the loop above initialized exactly the first
        // `port_views_len` elements.
        let port_views = unsafe { port_views_arr[..port_views_len].assume_init_mut() };

        self.plugin
            .process(port_views, frames, &mut event_iter, tempo);

        // port_views borrows end here (OutputBuffer has no Drop impl).

        // Copy each declared output port back into the host's audio buffers.
        for port_index in 0..self.output_scratch.len() {
            let Some(mut pair) = audio.port_pair(port_index) else {
                continue;
            };
            let mut channels = pair
                .channels()?
                .into_f32()
                .ok_or(PluginError::Message("Expected f32 audio"))?;
            let (scratch_l, scratch_r) = &self.output_scratch[port_index];
            if let Some(ch) = channels.channel_pair(0) {
                match ch {
                    ChannelPair::InPlace(buf) => {
                        buf[..frames].copy_from_slice(&scratch_l[..frames])
                    }
                    ChannelPair::InputOutput(_, out) => {
                        out[..frames].copy_from_slice(&scratch_l[..frames])
                    }
                    ChannelPair::OutputOnly(buf) => {
                        buf[..frames].copy_from_slice(&scratch_l[..frames])
                    }
                    _ => {}
                }
            }
            if let Some(ch) = channels.channel_pair(1) {
                match ch {
                    ChannelPair::InPlace(buf) => {
                        buf[..frames].copy_from_slice(&scratch_r[..frames])
                    }
                    ChannelPair::InputOutput(_, out) => {
                        out[..frames].copy_from_slice(&scratch_r[..frames])
                    }
                    ChannelPair::OutputOnly(buf) => {
                        buf[..frames].copy_from_slice(&scratch_r[..frames])
                    }
                    _ => {}
                }
            }
        }

        Ok(ProcessStatus::ContinueIfNotQuiet)
    }

    fn deactivate(self, main_thread: &mut ClapMainThread<'a, P>) {
        main_thread.plugin = Some(self.plugin);
    }

    fn reset(&mut self) {
        self.plugin.reset();
    }
}
