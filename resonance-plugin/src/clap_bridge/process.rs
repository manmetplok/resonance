//! Audio-processor lifecycle: activate, process, deactivate, reset.

use std::sync::atomic::Ordering;

use clack_extensions::latency::HostLatency;
use clack_plugin::events::event_types::NoteExpressionType;
use clack_plugin::prelude::*;

use super::ports::sidechain_port_index;
use super::shared::{ClapAudioProcessor, ClapMainThread, ClapShared, MAX_OUTPUT_PORTS};
use crate::plugin::{
    ControlEvent, EventIterator, KeyBuffer, NoteEvent, OutputBuffer, PluginEvent, ResonancePlugin,
    TempoInfo,
};

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

        // Bring the plugin's params and the shared atomics into agreement
        // before the plugin moves into the audio processor (PLG-08).
        reconcile_params(&plugin, shared);

        plugin.initialize(
            audio_config.sample_rate as f32,
            audio_config.max_frames_count,
        );

        // Capture the plugin's latency now that it is fully initialized —
        // some plugins (e.g. resonance-mastering) only build their DSP chain
        // inside `initialize()`, so any earlier query would read 0. The
        // plugin object moves into the audio processor below, so the
        // main-thread latency extension can no longer reach it; the host's
        // single post-activation `latency.get()` is served from this cached
        // value instead (CLAP only defines the query while active).
        //
        // This is also the point where a runtime latency change becomes
        // official: a plugin that pushed a new figure through
        // `HostHandle::set_latency_samples` asked for a restart, and the
        // re-activation that follows lands here and re-reads it.
        main_thread
            .host_handle
            .store_latency(plugin.latency_samples());
        // A change the plugin reported while inactive is announced here,
        // during activation — where CLAP allows `clap_host_latency.changed()`
        // and where the host re-reads the latency anyway — and taken, so
        // the main-thread callback the report also asked for finds nothing
        // left to say. Otherwise that late callback made the host run a
        // second, redundant restart cycle (FU-M1b).
        if main_thread.host_handle.take_latency_dirty() {
            if let Some(latency) = main_thread.host.shared().get_extension::<HostLatency>() {
                latency.changed(&mut main_thread.host);
            }
        }
        // From here on the plugin is active, so a further latency change
        // needs a restart request rather than a bare notification.
        main_thread.host_handle.set_active(true);

        let max_frames = audio_config.max_frames_count as usize;
        let port_count = shared.output_ports.len();
        let output_scratch = (0..port_count)
            .map(|_| (vec![0.0_f32; max_frames], vec![0.0_f32; max_frames]))
            .collect();
        // Only allocate key scratch for plugins that declare a sidechain
        // port; others keep these empty (and never read them).
        let key_len = if P::SIDECHAIN_INPUT.is_some() {
            max_frames
        } else {
            0
        };
        Ok(ClapAudioProcessor {
            plugin,
            shared,
            input_left: vec![0.0; max_frames],
            input_right: vec![0.0; max_frames],
            key_left: vec![0.0; key_len],
            key_right: vec![0.0; key_len],
            output_scratch,
            // Sized for the worst realistic block: notes plus a dense
            // controller stream (a fader sweep is ~1 CC per ms). The buffer
            // is cleared, never shrunk, so the capacity is reached at most
            // once per instance. A `push` past it would allocate on the
            // audio thread — deliberately preferred over silently dropping
            // events, because a dropped NoteOff is a note stuck forever
            // while one reallocation is a single glitch that never repeats.
            input_events: Vec::with_capacity(1024),
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

        // Handle input events: param changes, note events, MIDI controllers.
        self.input_events.clear();
        for event in events.input {
            if let Some(core_event) = event.as_core_event() {
                use clack_plugin::events::spaces::CoreEventSpace;
                match core_event {
                    CoreEventSpace::ParamValue(e) => {
                        if let Some(clap_id) = e.param_id() {
                            let value = e.value();
                            if let Some(slot) = self.shared.find_slot(clap_id.get()) {
                                // Applied instantly, block-quantized — the
                                // bridge does not smooth automation. Plugins
                                // de-zipper by feeding their `Smoother`s from
                                // param values at the start of `process()`;
                                // see the smoothing contract on
                                // `Param::set_plain`.
                                let param = self.plugin.param(slot);
                                param.set_plain(value);
                                // The landed value, not the wire value —
                                // see the note in `clap_bridge/params.rs`.
                                self.shared.set_value(slot, param.get_plain());
                                self.shared.note_host_param_change(slot);
                            }
                        }
                    }
                    CoreEventSpace::NoteOn(e) => {
                        if let crate::Match::Specific(key) = e.key() {
                            self.input_events.push(PluginEvent::Note(NoteEvent::NoteOn {
                                note: key as u8,
                                velocity: e.velocity() as f32,
                                timing: e.header().time(),
                            }));
                        }
                    }
                    CoreEventSpace::NoteOff(e) => {
                        if let crate::Match::Specific(key) = e.key() {
                            self.input_events.push(PluginEvent::Note(NoteEvent::NoteOff {
                                note: key as u8,
                                timing: e.header().time(),
                            }));
                        }
                    }
                    CoreEventSpace::NoteChoke(e) => {
                        if let crate::Match::Specific(key) = e.key() {
                            self.input_events.push(PluginEvent::Note(NoteEvent::Choke {
                                note: key as u8,
                                timing: e.header().time(),
                            }));
                        }
                    }
                    // Raw MIDI 1.0: control change, aftertouch and pitch
                    // bend. The CLAP note dialect cannot express a control
                    // change at all, so the bridge declares the MIDI dialect
                    // alongside it (see `ports.rs`) and decodes the
                    // channel-voice messages here. Note on/off are
                    // deliberately *not* decoded from raw MIDI: the port
                    // prefers the CLAP dialect, and accepting notes from both
                    // would double-trigger against a host that sends both.
                    CoreEventSpace::Midi(e) => {
                        if let Some(control) = super::midi::decode_midi(e.data(), e.header().time())
                        {
                            self.input_events.push(PluginEvent::Control(control));
                        }
                    }
                    // Poly aftertouch the CLAP-native way. A CLAP-dialect
                    // host expresses per-note pressure as a note expression
                    // rather than as MIDI, so both routes have to land on the
                    // same plugin-facing event.
                    CoreEventSpace::NoteExpression(e) => {
                        if e.expression_type() == Some(NoteExpressionType::Pressure) {
                            if let crate::Match::Specific(key) = e.key() {
                                self.input_events.push(PluginEvent::Control(
                                    ControlEvent::PolyPressure {
                                        channel: match e.channel() {
                                            crate::Match::Specific(c) => c as u8,
                                            _ => 0,
                                        },
                                        note: key as u8,
                                        pressure: e.value() as f32,
                                        timing: e.header().time(),
                                    },
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Re-sync params from shared atomics if state was loaded while
        // active.
        //
        // Gated on an even publication generation: a load that is still
        // part-way through storing its values (see `ClapShared::params_gen`)
        // has not become visible yet, and copying out of the middle of one
        // would hand the plugin a mix of the old state and the new. There
        // is no waiting involved — an odd generation just leaves
        // `params_dirty` set for a later block, which is exactly what the
        // flag is for.
        let publish_gen = self.shared.param_publish_gen();
        if publish_gen & 1 == 0 && self.shared.params_dirty.swap(false, Ordering::Acquire) {
            for i in 0..self.plugin.param_count() {
                if i < self.shared.param_values.len() {
                    self.plugin.param(i).set_plain(self.shared.get_value(i));
                }
            }
            // A load that opened its window *after* the check above could
            // have been publishing while the loop ran, so what the plugin
            // just took may be a blend. Hand the flag back rather than
            // leave it: the next block re-runs the copy against the
            // finished load.
            //
            // The Acquire fence is what makes the re-read below *evidence*:
            // the value loads above are Relaxed, and without the fence
            // nothing stops them sinking below the generation re-read on a
            // weakly-ordered CPU (the SeqCst load is an acquire, which only
            // pins later accesses, not earlier ones) — a bracket the loads
            // escaped validates nothing. The fence keeps every load above
            // it above the re-read, and pairs with the Release fence in
            // `begin_param_publish`: a value load that caught a mid-publish
            // store forces the re-read to observe that publish's odd
            // generation (or later), so the mismatch is detected.
            std::sync::atomic::fence(Ordering::Acquire);
            if self.shared.param_publish_gen() != publish_gen {
                self.shared.params_dirty.store(true, Ordering::Release);
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
        //
        // # Racing a concurrent `state::load` (ba todos #1363, #1374)
        //
        // CLAP allows `state::load` ([main-thread]) to run concurrently
        // with `process` ([audio-thread]). Load stores every param into
        // the shared atomics (Relaxed), runs the plugin's extra-state
        // saver, and then sets `params_dirty` (Release). A load that
        // lands after this block's dirty swap above is therefore *not*
        // handled by that swap, and a naive store here would put the
        // plugin's stale value back over the freshly loaded one. Because
        // the still-set dirty flag makes the *next* block copy shared ->
        // plugin, that clobber is permanent: the load is lost, not merely
        // delayed.
        //
        // Three guards, in the order they fire:
        //
        // 1. `params_dirty` is re-read immediately before each store. A
        //    load that has already announced itself makes the audio
        //    thread abandon the rest of this block's push-back entirely
        //    (`break`, not `continue`: once a load has landed, *every*
        //    slot's shared value may be a loaded one, so no later slot
        //    is safe to write either). The loaded values stay untouched
        //    and the still-set flag re-syncs the plugin from them next
        //    block.
        // 2. `params_gen` is read once before the loop reads any slot, and
        //    again after each store (ba todo #1374). It has to come
        //    before the slot reads, not merely before the store: read
        //    after them, a whole window could close between the dirty
        //    check and the generation read, and a loaded value taken
        //    inside that window would then pass as quiescent and be
        //    exchanged away — the flag that followed copying the stale
        //    value into the plugin. Odd means a load is publishing right now:
        //    its values are already in the atomics with no flag yet
        //    announcing them, which is precisely the case guard 1 cannot
        //    see, so the push-back stands down. A generation that
        //    *changed* across the store means a whole window opened (and
        //    possibly closed) around it, so the store is undone — a
        //    compare-exchange back to the value we found, which can only
        //    land if nothing has written the slot since — and the
        //    push-back stands down as well.
        //
        //    This is the read side of a seqlock, but the reader here is
        //    itself a writer of the same slots, so it cannot re-read and
        //    retry its way to a consistent view. It does not need to:
        //    abandoning the push-back is always available and always
        //    correct. That keeps the audio thread wait-free — a fixed
        //    two loads and at most one extra exchange, never a spin.
        //    Like every seqlock read side, the bracket closes with an
        //    Acquire fence before the confirming generation re-read, so
        //    the bracketed accesses cannot sink below it (see the fence
        //    comments at both re-reads).
        // 3. The store itself is a compare-exchange against the value we
        //    just read, so a load landing in the few instructions
        //    between that read and the exchange makes the exchange fail
        //    and leaves the loaded value in place.
        //
        // What this costs: when a load lands mid-block, this block's
        // genuine editor edits are dropped — the plugin-side edit is
        // overwritten by the next block's shared -> plugin re-sync. That
        // is the right way round. A state load is a deliberate
        // whole-instrument action (preset recall, project open) whose
        // whole point is to replace every value; an editor tweak is a
        // continuous gesture the user will simply still be making on the
        // next block. The alternative — letting one knob survive a
        // preset recall — leaves the instrument in a state that matches
        // neither the preset nor anything the user asked for.
        //
        // What this does NOT close, honestly: guard 2 brackets the store
        // but cannot fuse with it, and guard 3 keys on the slot's value.
        // So one case survives — a load whose value for a slot is
        // *exactly the value that slot already held*, published in the
        // handful of instructions between the bracket and the exchange.
        // Nothing distinguishes it from no write at all, so the exchange
        // succeeds and the plugin ends up with the editor's pending edit
        // instead of the loaded value. Note what that case is not: the
        // shared atomics already agreed with the load, so nothing the
        // host reads back or re-saves is wrong, and the only difference
        // is which of two values — one of them a knob the user is
        // holding — reaches the DSP. Closing it needs the load's values
        // to live somewhere this loop cannot write at all: a staging
        // buffer the dirty flag hands over, which is a change to the
        // layout of `ClapShared` and to every reader of `param_values`.
        //
        // `gen_before` is a SeqCst (so acquire) load, which keeps every
        // Relaxed slot read below from being hoisted above it. Odd: a
        // load is publishing, and whatever the slots hold may be its
        // values with no flag announcing them yet — skip the push-back.
        let gen_before = self.shared.param_publish_gen();
        let push_back_count = if gen_before & 1 == 1 {
            0
        } else {
            self.plugin.param_count()
        };
        for i in 0..push_back_count {
            if i < self.shared.param_values.len() {
                let plugin_v = self.plugin.param(i).get_plain();
                let shared_v = self.shared.get_value(i);
                if shared_v.to_bits() != plugin_v.to_bits() {
                    // Checked here rather than once before the loop so it
                    // covers a load that lands *while* the loop is walking
                    // the params, and inside the difference test so the
                    // steady state (nothing changed) pays nothing for it.
                    // Acquire pairs with the Release store in
                    // `state::load` and keeps the exchange below from
                    // being reordered ahead of it.
                    if self.shared.params_dirty.load(Ordering::Acquire) {
                        break;
                    }
                    let stored = self.shared.compare_exchange_value(i, shared_v, plugin_v);
                    // Same rule as the state-load re-sync bracket above: a
                    // generation bracket only proves anything if nothing it
                    // brackets can sink below the confirming re-read. The
                    // Acquire fence pins the Relaxed `get_value` read (and
                    // the exchange, without leaning on the exchange's own
                    // SeqCst ordering) above the re-read, and pairs with the
                    // Release fence in `begin_param_publish` so a slot value
                    // taken from a mid-publish window forces the re-read to
                    // see that window's odd generation or later.
                    std::sync::atomic::fence(Ordering::Acquire);
                    if self.shared.param_publish_gen() != gen_before {
                        // A publication window opened around the exchange.
                        // If the exchange landed it may have landed on a
                        // loaded value, so put back what was there — the
                        // compare-exchange makes that safe, since it can
                        // only land while the slot still holds what we
                        // just wrote.
                        if stored {
                            let _ = self.shared.compare_exchange_value(i, plugin_v, shared_v);
                        }
                        break;
                    }
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

        let mut event_iter = EventIterator::mixed(&self.input_events);

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

        // Read the external sidechain (key) signal, if this plugin declares
        // one, into the key scratch. Always stereo-shaped: a mono key port is
        // mirrored into both channels so detectors read either uniformly.
        // `key` is `None` for plugins without a sidechain port, or when the
        // host did not connect it for this block.
        let key = if let Some(sc_index) =
            sidechain_port_index(P::INPUT_CHANNELS, P::SIDECHAIN_INPUT)
        {
            let key_left = &mut self.key_left[..frames];
            let key_right = &mut self.key_right[..frames];
            key_left.fill(0.0);
            key_right.fill(0.0);
            let mut connected = false;
            if let Some(port) = audio.input_port(sc_index) {
                if let Some(channels) = port.channels()?.into_f32() {
                    connected = true;
                    if let Some(l) = channels.channel(0) {
                        key_left.copy_from_slice(&l[..frames]);
                    }
                    match channels.channel(1) {
                        // Stereo key port: use the second channel.
                        Some(r) => key_right.copy_from_slice(&r[..frames]),
                        // Mono key port: mirror the single channel.
                        None => key_right.copy_from_slice(key_left),
                    }
                }
            }
            connected.then_some(KeyBuffer {
                left: &self.key_left[..frames],
                right: &self.key_right[..frames],
            })
        } else {
            None
        };

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
        // `new_shared` refuses a layout with more ports than the array
        // holds, so a plugin cannot get this far with one (PLG-09).
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
            .process_with_key(port_views, key, frames, &mut event_iter, tempo);

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
        main_thread.host_handle.set_active(false);

        // Hand the plugin back in agreement with the shared values (ba
        // todo #1376, PLG-08).
        //
        // A load that lands while active and is followed by deactivation
        // with no `process()` block in between never reaches the plugin
        // on its own: only the audio thread copies `shared` into the
        // plugin. That is what a host does when it opens a project with
        // the transport stopped, and the main-thread `state::save` path
        // asks the PLUGIN, so saving in that window used to persist every
        // parameter at its default. `reconcile_params` applies such a load.
        //
        // Without a pending load the plugin is the newer side: an editor
        // edit made after the last block's push-back lives only in the
        // plugin, and copying `shared` over it would snap the knob back.
        reconcile_params(&self.plugin, self.shared);

        let mut plugin = self.plugin;
        plugin.deactivate();
        main_thread.plugin = Some(plugin);
    }

    fn reset(&mut self) {
        self.plugin.reset();
    }
}

/// Make the plugin's params and the shared atomics agree at an activation
/// boundary (`activate` / `deactivate`, main thread, no `process()` in
/// flight and no `state::load` either — both are `[main-thread]`).
///
/// Whichever side holds the newer values wins:
///
/// * `params_dirty` set: a state load was published into `shared` and no
///   block has applied it yet, so `shared` → plugin (ba todo #1376);
/// * otherwise the plugin: every host-side writer (`flush`, an inactive
///   `state::load`, `ParamValue` events) writes both sides, but an editor
///   writes only the plugin — after the last block's push-back, or while
///   the plugin is deactivated — so plugin → `shared` (PLG-08).
fn reconcile_params<P: ResonancePlugin>(plugin: &P, shared: &ClapShared<'_>) {
    let count = plugin.param_count().min(shared.param_values.len());
    if shared.params_dirty.swap(false, Ordering::AcqRel) {
        for i in 0..count {
            plugin.param(i).set_plain(shared.get_value(i));
        }
    } else {
        for i in 0..count {
            shared.set_value(i, plugin.param(i).get_plain());
        }
    }
}
