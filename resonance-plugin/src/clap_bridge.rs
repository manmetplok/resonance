/// CLAP bridge: maps ResonancePlugin to clack-plugin's trait hierarchy.
///
/// Architecture:
/// - ClapShared: holds param metadata + atomic values, host handle (Send+Sync)
/// - ClapMainThread: holds Option<P> (plugin when not active), extension impls
/// - ClapAudioProcessor: holds P (plugin when active), processes audio

use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, PluginGui, PluginGuiImpl, Window,
};
use clack_extensions::latency::{PluginLatency, PluginLatencyImpl};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePorts,
    PluginNotePortsImpl,
};
use clack_extensions::params::{
    ParamInfo, ParamInfoFlags, ParamInfoWriter, ParamDisplayWriter,
    PluginAudioProcessorParams, PluginMainThreadParams, PluginParams,
};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};

use crate::gui::{EditorFactory, PluginEditor};
use crate::param::Param;
use crate::plugin::{EventIterator, NoteEvent, OutputBuffer, OutputPortSpec, ResonancePlugin};

// ---------------------------------------------------------------------------
// Param metadata stored in SharedState
// ---------------------------------------------------------------------------

pub(crate) struct ParamMeta {
    pub clap_id: u32,
    pub str_id: String,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub is_stepped: bool,
    pub is_hidden: bool,
}

// ---------------------------------------------------------------------------
// SharedState (Send + Sync, shared between threads)
// ---------------------------------------------------------------------------

pub struct ClapShared<'a> {
    #[allow(dead_code)]
    host: HostSharedHandle<'a>,
    pub(crate) param_metas: Vec<ParamMeta>,
    /// Atomic param values (f64 bit-punned to u64), indexed by param slot.
    pub(crate) param_values: Vec<AtomicU64>,
    /// Map from CLAP param ID to slot index.
    pub(crate) clap_id_to_slot: std::collections::HashMap<u32, usize>,
    input_channels: Option<u32>,
    /// Cached output-port layout, captured once from `ResonancePlugin::output_layout()`
    /// at plugin construction. The CLAP audio-ports extension, the host, and the
    /// audio processor all consult this instead of re-calling the plugin hook.
    output_ports: Vec<OutputPortSpec>,
    midi_input: bool,
    /// Flag: shared param values have been updated (e.g. state load while active).
    /// The audio processor should re-sync plugin params from shared atomics.
    pub(crate) params_dirty: AtomicBool,
}

impl ClapShared<'_> {
    pub fn find_slot(&self, clap_id: u32) -> Option<usize> {
        self.clap_id_to_slot.get(&clap_id).copied()
    }

    pub fn get_value(&self, slot: usize) -> f64 {
        f64::from_bits(self.param_values[slot].load(Ordering::Relaxed))
    }

    pub fn set_value(&self, slot: usize, value: f64) {
        self.param_values[slot].store(value.to_bits(), Ordering::Relaxed);
    }
}

// SAFETY: HostSharedHandle wraps CLAP host function pointers which the CLAP spec
// mandates to be thread-safe (the host must support concurrent calls from any thread).
// All other fields are atomics, HashMap (read-only after construction), or Send+Sync types.
unsafe impl Send for ClapShared<'_> {}
unsafe impl Sync for ClapShared<'_> {}

impl<'a> PluginShared<'a> for ClapShared<'a> {}

// ---------------------------------------------------------------------------
// MainThreadState
// ---------------------------------------------------------------------------

pub struct ClapMainThread<'a, P: ResonancePlugin> {
    #[allow(dead_code)]
    host: HostMainThreadHandle<'a>,
    shared: &'a ClapShared<'a>,
    plugin: Option<P>,
    last_latency: u32,
    /// Editor factory harvested from the plugin at construction time. `None`
    /// if the plugin has no GUI. Kept alive across activate/deactivate so
    /// the host can open the editor while audio is running.
    editor_factory: Option<std::sync::Arc<dyn EditorFactory>>,
    /// The currently-open editor, if any. Created by `gui_create`, dropped
    /// by `gui_destroy`.
    editor: Option<Box<dyn PluginEditor>>,
    /// Extra-state saver harvested from the plugin at construction time.
    /// `None` if the plugin has no extra state. Kept alive across
    /// activate/deactivate so the host can save/load project state while
    /// the plugin is in the audio processor.
    extra_state_saver: Option<std::sync::Arc<dyn crate::plugin::ExtraStateSaver>>,
}

impl<'a, P: ResonancePlugin> PluginMainThread<'a, ClapShared<'a>> for ClapMainThread<'a, P> {}

// ---------------------------------------------------------------------------
// AudioProcessor
// ---------------------------------------------------------------------------

pub struct ClapAudioProcessor<'a, P: ResonancePlugin> {
    plugin: P,
    shared: &'a ClapShared<'a>,
    #[allow(dead_code)]
    sample_rate: f32,
    /// Pre-allocated scratch buffers for the effect/instrument input
    /// (read from host into these before the plugin call).
    input_left: Vec<f32>,
    input_right: Vec<f32>,
    /// Pre-allocated output scratch, one `(left, right)` pair per declared
    /// output port. Populated by the plugin on each `process()` call and
    /// then copied back into the CLAP audio buffers.
    output_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    /// Pre-allocated buffer for note events (avoids audio-thread allocation).
    note_events: Vec<NoteEvent>,
}

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

        plugin.initialize(audio_config.sample_rate as f32, audio_config.max_frames_count);

        let max_frames = audio_config.max_frames_count as usize;
        let port_count = shared.output_ports.len();
        let output_scratch = (0..port_count)
            .map(|_| (vec![0.0_f32; max_frames], vec![0.0_f32; max_frames]))
            .collect();
        Ok(ClapAudioProcessor {
            plugin,
            shared,
            sample_rate: audio_config.sample_rate as f32,
            input_left: vec![0.0; max_frames],
            input_right: vec![0.0; max_frames],
            output_scratch,
            note_events: Vec::with_capacity(256),
        })
    }

    fn process(
        &mut self,
        _process: Process,
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
        for i in 0..self.plugin.param_count() {
            if i < self.shared.param_values.len() {
                self.shared
                    .set_value(i, self.plugin.param(i).get_plain());
            }
        }

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
                let mut channels = pair.channels()?.into_f32().ok_or(PluginError::Message("Expected f32 audio"))?;
                if let Some(ch) = channels.channel_pair(0) {
                    match ch {
                        ChannelPair::InPlace(buf) => input_left.copy_from_slice(&buf[..frames]),
                        ChannelPair::InputOutput(inp, _) => input_left.copy_from_slice(&inp[..frames]),
                        _ => input_left.fill(0.0),
                    }
                }
                if let Some(ch) = channels.channel_pair(1) {
                    match ch {
                        ChannelPair::InPlace(buf) => input_right.copy_from_slice(&buf[..frames]),
                        ChannelPair::InputOutput(inp, _) => input_right.copy_from_slice(&inp[..frames]),
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
        // The tiny allocation here (up to 8 output ports × 16 bytes each)
        // is acceptable on the audio thread — far cheaper than copying
        // audio data, and the plugin call itself does real work.
        let mut port_views: Vec<OutputBuffer<'_>> =
            Vec::with_capacity(self.output_scratch.len());
        for (l, r) in self.output_scratch.iter_mut() {
            port_views.push(OutputBuffer {
                left: &mut l[..frames],
                right: &mut r[..frames],
            });
        }

        self.plugin
            .process(&mut port_views, frames, &mut event_iter);

        // Drop the port views so the scratch is free to be re-borrowed for
        // the write-back pass below.
        drop(port_views);

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
                    ChannelPair::InPlace(buf) => buf[..frames].copy_from_slice(&scratch_l[..frames]),
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
                    ChannelPair::InPlace(buf) => buf[..frames].copy_from_slice(&scratch_r[..frames]),
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

// ---------------------------------------------------------------------------
// Plugin marker trait + DefaultPluginFactory
// ---------------------------------------------------------------------------

pub struct ClapBridge<P: ResonancePlugin>(std::marker::PhantomData<P>);

impl<P: ResonancePlugin> Plugin for ClapBridge<P> {
    type AudioProcessor<'a> = ClapAudioProcessor<'a, P>;
    type Shared<'a> = ClapShared<'a>;
    type MainThread<'a> = ClapMainThread<'a, P>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, shared: Option<&ClapShared<'_>>) {
        builder.register::<PluginAudioPorts>();
        builder.register::<PluginParams>();
        builder.register::<PluginState>();

        if let Some(shared) = shared {
            if shared.midi_input {
                builder.register::<PluginNotePorts>();
            }
        } else {
            // First call (no shared yet) — register conservatively
            builder.register::<PluginNotePorts>();
        }

        builder.register::<PluginLatency>();
        // GUI extension is registered unconditionally; plugins without an
        // editor factory return false from is_api_supported, which is the
        // CLAP-correct way to say "no editor".
        builder.register::<PluginGui>();
    }
}

impl<P: ResonancePlugin> DefaultPluginFactory for ClapBridge<P> {
    fn get_descriptor() -> PluginDescriptor {
        let mut desc = PluginDescriptor::new(P::CLAP_ID, P::NAME)
            .with_vendor(P::VENDOR)
            .with_version(P::VERSION)
            .with_description(P::DESCRIPTION);

        let features: Vec<&'static std::ffi::CStr> = P::FEATURES
            .iter()
            .filter_map(|f| match *f {
                "audio-effect" => Some(c"audio-effect"),
                "instrument" => Some(c"instrument"),
                "stereo" => Some(c"stereo"),
                "mono" => Some(c"mono"),
                "reverb" => Some(c"reverb"),
                "sampler" => Some(c"sampler"),
                "drum" | "drum-machine" => Some(c"drum-machine"),
                "synthesizer" | "synth" => Some(c"synthesizer"),
                "cabinet-simulator" => Some(c"cabinet-simulator"),
                _ => None,
            })
            .collect();
        if !features.is_empty() {
            desc = desc.with_features(features);
        }

        desc
    }

    fn new_shared<'a>(host: HostSharedHandle<'a>) -> Result<ClapShared<'a>, PluginError> {
        let temp = P::new();
        let count = temp.param_count();
        let output_ports = temp.output_layout();
        if output_ports.is_empty() {
            return Err(PluginError::Message("Plugin must declare at least one output port"));
        }
        for port in &output_ports {
            if port.channel_count != 1 && port.channel_count != 2 {
                return Err(PluginError::Message(
                    "Only mono and stereo output ports are supported",
                ));
            }
        }

        let mut param_metas: Vec<ParamMeta> = Vec::with_capacity(count);
        let mut param_values: Vec<AtomicU64> = Vec::with_capacity(count);
        let mut clap_id_to_slot: std::collections::HashMap<u32, usize> = std::collections::HashMap::with_capacity(count);

        for i in 0..count {
            let p = temp.param(i);
            let clap_id = p.clap_id();

            // Check for hash collisions
            if let Some(&existing_slot) = clap_id_to_slot.get(&clap_id) {
                panic!(
                    "CLAP param ID collision: params '{}' (slot {}) and '{}' (slot {}) both hash to {}",
                    param_metas[existing_slot].str_id, existing_slot, p.id(), i, clap_id
                );
            }

            param_metas.push(ParamMeta {
                clap_id,
                str_id: p.id().to_string(),
                name: p.name().to_string(),
                min: p.min_plain(),
                max: p.max_plain(),
                default: p.default_plain(),
                is_stepped: p.is_stepped(),
                is_hidden: p.is_hidden(),
            });
            param_values.push(AtomicU64::new(p.default_plain().to_bits()));
            clap_id_to_slot.insert(clap_id, i);
        }

        Ok(ClapShared {
            host,
            param_metas,
            param_values,
            clap_id_to_slot,
            input_channels: P::INPUT_CHANNELS,
            output_ports,
            midi_input: P::MIDI_INPUT,
            params_dirty: AtomicBool::new(false),
        })
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a ClapShared<'a>,
    ) -> Result<ClapMainThread<'a, P>, PluginError> {
        let plugin = P::new();
        for i in 0..plugin.param_count() {
            if i < shared.param_values.len() {
                shared.set_value(i, plugin.param(i).get_plain());
            }
        }

        // Harvest the editor factory and any extra-state saver before the
        // plugin may be moved to the audio processor. Both are None for
        // plugins that don't opt in.
        let editor_factory = plugin.editor_factory();
        let extra_state_saver = plugin.extra_state_saver();

        Ok(ClapMainThread {
            host,
            shared,
            plugin: Some(plugin),
            last_latency: 0,
            editor_factory,
            editor: None,
            extra_state_saver,
        })
    }
}

// ---------------------------------------------------------------------------
// AudioPorts extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginAudioPortsImpl for ClapMainThread<'a, P> {
    fn count(&mut self, is_input: bool) -> u32 {
        if is_input {
            if self.shared.input_channels.is_some() { 1 } else { 0 }
        } else {
            self.shared.output_ports.len() as u32
        }
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if is_input {
            if index != 0 {
                return;
            }
            if let Some(ch) = self.shared.input_channels {
                writer.set(&AudioPortInfo {
                    id: ClapId::new(1),
                    name: b"Input",
                    channel_count: ch,
                    flags: AudioPortFlags::IS_MAIN,
                    port_type: Some(if ch == 1 {
                        AudioPortType::MONO
                    } else {
                        AudioPortType::STEREO
                    }),
                    // Only the main output port (index 0) gets the in-place
                    // pair with the input port; secondary outputs are not
                    // in-place routable.
                    in_place_pair: Some(ClapId::new(2)),
                });
            }
            return;
        }

        // Output ports — one AudioPortInfo per entry in `output_ports`.
        let Some(port) = self.shared.output_ports.get(index as usize) else {
            return;
        };
        // Port IDs start at 2 (legacy: input was 1, main output was 2) and
        // increase by one per additional output.
        let port_id = ClapId::new(2 + index);
        let is_main = index == 0;
        // Use a zero-terminated buffer for the name; CLAP names are
        // limited to CLAP_NAME_SIZE bytes so truncate safely if needed.
        let mut name_buf = [0u8; 32];
        let bytes = port.name.as_bytes();
        let copy_len = bytes.len().min(name_buf.len() - 1);
        name_buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
        writer.set(&AudioPortInfo {
            id: port_id,
            name: &name_buf[..=copy_len],
            channel_count: port.channel_count,
            flags: if is_main {
                AudioPortFlags::IS_MAIN
            } else {
                AudioPortFlags::empty()
            },
            port_type: Some(if port.channel_count == 1 {
                AudioPortType::MONO
            } else {
                AudioPortType::STEREO
            }),
            // Only the main output port can be in-place paired with the
            // input port for effects.
            in_place_pair: if is_main && self.shared.input_channels.is_some() {
                Some(ClapId::new(1))
            } else {
                None
            },
        });
    }
}

// ---------------------------------------------------------------------------
// Params extension (main thread)
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginMainThreadParams for ClapMainThread<'a, P> {
    fn count(&mut self) -> u32 {
        self.shared
            .param_metas
            .iter()
            .filter(|m| !m.is_hidden)
            .count() as u32
    }

    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        let visible: Vec<_> = self
            .shared
            .param_metas
            .iter()
            .filter(|m| !m.is_hidden)
            .collect();

        if let Some(meta) = visible.get(param_index as usize) {
            let mut flags = ParamInfoFlags::IS_AUTOMATABLE;
            if meta.is_stepped {
                flags |= ParamInfoFlags::IS_STEPPED;
            }

            info.set(&ParamInfo {
                id: ClapId::new(meta.clap_id),
                name: meta.name.as_bytes(),
                module: b"",
                default_value: meta.default,
                min_value: meta.min,
                max_value: meta.max,
                flags,
                cookie: Default::default(),
            });
        }
    }

    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        let slot = self.shared.find_slot(param_id.get())?;
        Some(self.shared.get_value(slot))
    }

    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> core::fmt::Result {
        if let Some(slot) = self.shared.find_slot(param_id.get()) {
            if let Some(plugin) = &self.plugin {
                if slot < plugin.param_count() {
                    let text = plugin.param(slot).display(value);
                    return write!(writer, "{}", text);
                }
            }
            write!(writer, "{:.2}", value)
        } else {
            write!(writer, "{:.2}", value)
        }
    }

    fn text_to_value(&mut self, param_id: ClapId, text: &std::ffi::CStr) -> Option<f64> {
        let slot = self.shared.find_slot(param_id.get())?;
        if let Some(plugin) = &self.plugin {
            if slot < plugin.param_count() {
                return plugin.param(slot).parse(text.to_str().ok()?);
            }
        }
        None
    }

    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        _output_parameter_changes: &mut OutputEvents,
    ) {
        for event in input_parameter_changes {
            if let Some(core_event) = event.as_core_event() {
                use clack_plugin::events::spaces::CoreEventSpace;
                if let CoreEventSpace::ParamValue(e) = core_event {
                    if let Some(clap_id) = e.param_id() {
                        if let Some(slot) = self.shared.find_slot(clap_id.get()) {
                            self.shared.set_value(slot, e.value());
                            if let Some(plugin) = &self.plugin {
                                if slot < plugin.param_count() {
                                    plugin.param(slot).set_plain(e.value());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Params extension (audio processor)
// ---------------------------------------------------------------------------

impl<P: ResonancePlugin> PluginAudioProcessorParams for ClapAudioProcessor<'_, P> {
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        _output_parameter_changes: &mut OutputEvents,
    ) {
        for event in input_parameter_changes {
            if let Some(core_event) = event.as_core_event() {
                use clack_plugin::events::spaces::CoreEventSpace;
                if let CoreEventSpace::ParamValue(e) = core_event {
                    if let Some(clap_id) = e.param_id() {
                        if let Some(slot) = self.shared.find_slot(clap_id.get()) {
                            if slot < self.plugin.param_count() {
                                self.plugin.param(slot).set_plain(e.value());
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// State extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginStateImpl for ClapMainThread<'a, P> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let data = if let Some(plugin) = &self.plugin {
            // Main-thread path: the plugin's own `save_state` composes
            // params with any extra-state saver via the trait default.
            plugin.save_state()
        } else {
            // Audio-processor path: the owned plugin is currently inside
            // `ClapAudioProcessor`, so we can't call `save_state` directly.
            // Serialize params from the shared atomics and merge any
            // extra-state saver's output using the same `"extra" ->
            // top-level` shape the plugin would produce.
            let temp_params: Vec<crate::param::TempParamOwned> = self
                .shared
                .param_metas
                .iter()
                .enumerate()
                .map(|(i, meta)| crate::param::TempParamOwned {
                    id: meta.str_id.clone(),
                    value: self.shared.get_value(i),
                })
                .collect();
            let refs: Vec<&dyn Param> = temp_params.iter().map(|p| p as &dyn Param).collect();
            let mut json = crate::state::params_to_json(&refs);
            if let Some(saver) = &self.extra_state_saver {
                if let Some(obj) = json.as_object_mut() {
                    for (k, v) in saver.save() {
                        obj.insert(k, v);
                    }
                }
            }
            serde_json::to_vec(&json).unwrap_or_default()
        };
        output
            .write_all(&data)
            .map_err(|_| PluginError::Message("Failed to write state"))?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut data = Vec::new();
        input
            .read_to_end(&mut data)
            .map_err(|_| PluginError::Message("Failed to read state"))?;

        if let Some(plugin) = &mut self.plugin {
            if !plugin.load_state(&data) {
                return Err(PluginError::Message("Failed to load state"));
            }
            // Sync loaded values back to shared atomics
            for i in 0..plugin.param_count() {
                if i < self.shared.param_values.len() {
                    self.shared.set_value(i, plugin.param(i).get_plain());
                }
            }
        } else {
            // Audio-processor path: parse once, load params into shared
            // atomics, and hand the parsed value to the extra-state saver
            // so file paths etc. land in their shared storage.
            let state: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|_| PluginError::Message("Failed to load state"))?;
            if !crate::state::load_params_from_shared_json(
                &self.shared.param_metas,
                &self.shared.param_values,
                &state,
            ) {
                return Err(PluginError::Message("Failed to load state"));
            }
            if let Some(saver) = &self.extra_state_saver {
                saver.load(&state);
            }
            self.shared.params_dirty.store(true, Ordering::Release);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NotePorts extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginNotePortsImpl for ClapMainThread<'a, P> {
    fn count(&mut self, is_input: bool) -> u32 {
        if is_input && P::MIDI_INPUT {
            1
        } else {
            0
        }
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut NotePortInfoWriter) {
        if index == 0 && is_input && P::MIDI_INPUT {
            writer.set(&NotePortInfo {
                id: ClapId::new(1),
                name: b"Note Input",
                supported_dialects: NoteDialects::CLAP,
                preferred_dialect: Some(NoteDialect::Clap),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Latency extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginLatencyImpl for ClapMainThread<'a, P> {
    fn get(&mut self) -> u32 {
        if let Some(plugin) = &self.plugin {
            let lat = plugin.latency_samples();
            self.last_latency = lat;
            lat
        } else {
            self.last_latency
        }
    }
}

// ---------------------------------------------------------------------------
// GUI extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginGuiImpl for ClapMainThread<'a, P> {
    fn is_api_supported(&mut self, configuration: GuiConfiguration) -> bool {
        let Some(factory) = &self.editor_factory else {
            return false;
        };
        let Ok(api) = configuration.api_type.0.to_str() else {
            return false;
        };
        factory.supports(api, configuration.is_floating)
    }

    fn get_preferred_api(&mut self) -> Option<GuiConfiguration<'_>> {
        let factory = self.editor_factory.as_ref()?;
        let (api, is_floating) = factory.preferred()?;
        let api_type = match api {
            "wayland" => GuiApiType::WAYLAND,
            "x11" => GuiApiType::X11,
            "win32" => GuiApiType::WIN32,
            "cocoa" => GuiApiType::COCOA,
            _ => return None,
        };
        Some(GuiConfiguration {
            api_type,
            is_floating,
        })
    }

    fn create(&mut self, configuration: GuiConfiguration) -> Result<(), PluginError> {
        let factory = self
            .editor_factory
            .as_ref()
            .ok_or(PluginError::Message("plugin has no editor factory"))?;
        let api = configuration
            .api_type
            .0
            .to_str()
            .map_err(|_| PluginError::Message("invalid GUI api string"))?;
        let editor = factory
            .create(api, configuration.is_floating)
            .ok_or(PluginError::Message("editor creation failed"))?;
        self.editor = Some(editor);
        Ok(())
    }

    fn destroy(&mut self) {
        self.editor = None;
    }

    fn set_scale(&mut self, _scale: f64) -> Result<(), PluginError> {
        // Wayland handles scale via the compositor — the runtime reads it
        // from wl_output events. For other APIs this would matter.
        Ok(())
    }

    fn get_size(&mut self) -> Option<GuiSize> {
        if let Some(editor) = &self.editor {
            let (w, h) = editor.size();
            Some(GuiSize { width: w, height: h })
        } else if let Some(factory) = &self.editor_factory {
            let (w, h) = factory.preferred_size();
            Some(GuiSize { width: w, height: h })
        } else {
            None
        }
    }

    fn can_resize(&mut self) -> bool {
        self.editor.as_ref().map(|e| e.can_resize()).unwrap_or(false)
    }

    fn set_size(&mut self, size: GuiSize) -> Result<(), PluginError> {
        let editor = self
            .editor
            .as_mut()
            .ok_or(PluginError::Message("no editor to resize"))?;
        if editor.set_size(size.width, size.height) {
            Ok(())
        } else {
            Err(PluginError::Message("set_size refused"))
        }
    }

    fn set_parent(&mut self, _window: Window) -> Result<(), PluginError> {
        // We are Wayland-only and floating-only in v1. Pretend to succeed so
        // hosts that call set_parent unconditionally (even with is_floating=true)
        // don't fail the handshake.
        Ok(())
    }

    fn set_transient(&mut self, _window: Window) -> Result<(), PluginError> {
        // v1: no-op. Could later map to xdg-foreign-unstable-v2 on Wayland
        // to mark the plugin window as transient for the host window.
        Ok(())
    }

    fn suggest_title(&mut self, title: &str) {
        if let Some(editor) = &mut self.editor {
            editor.set_title(title);
        }
    }

    fn show(&mut self) -> Result<(), PluginError> {
        if let Some(editor) = &mut self.editor {
            editor.show();
            Ok(())
        } else {
            Err(PluginError::Message("no editor to show"))
        }
    }

    fn hide(&mut self) -> Result<(), PluginError> {
        if let Some(editor) = &mut self.editor {
            editor.hide();
            Ok(())
        } else {
            Err(PluginError::Message("no editor to hide"))
        }
    }
}
