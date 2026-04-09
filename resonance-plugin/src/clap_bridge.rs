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

use crate::param::Param;
use crate::plugin::{EventIterator, NoteEvent, ResonancePlugin};

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
    output_channels: u32,
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
    /// Pre-allocated temp buffers for left/right channel data.
    temp_left: Vec<f32>,
    temp_right: Vec<f32>,
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
        Ok(ClapAudioProcessor {
            plugin,
            shared,
            sample_rate: audio_config.sample_rate as f32,
            temp_left: vec![0.0; max_frames],
            temp_right: vec![0.0; max_frames],
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

        let mut event_iter = EventIterator::new(&self.note_events);

        // Prepare temp buffers
        let left = &mut self.temp_left[..frames];
        let right = &mut self.temp_right[..frames];

        if P::INPUT_CHANNELS.is_some() {
            // Read input from port pair (in-place processing)
            if let Some(mut pair) = audio.port_pair(0) {
                let mut channels = pair.channels()?.into_f32().ok_or(PluginError::Message("Expected f32 audio"))?;
                // Read left channel
                if let Some(ch) = channels.channel_pair(0) {
                    match ch {
                        ChannelPair::InPlace(buf) => left.copy_from_slice(&buf[..frames]),
                        ChannelPair::InputOutput(inp, _) => left.copy_from_slice(&inp[..frames]),
                        _ => left.fill(0.0),
                    }
                }
                // Read right channel
                if let Some(ch) = channels.channel_pair(1) {
                    match ch {
                        ChannelPair::InPlace(buf) => right.copy_from_slice(&buf[..frames]),
                        ChannelPair::InputOutput(inp, _) => right.copy_from_slice(&inp[..frames]),
                        _ => right.fill(0.0),
                    }
                }
            }
        } else {
            left.fill(0.0);
            right.fill(0.0);
        }

        // Process audio through the plugin
        self.plugin.process(left, right, frames, &mut event_iter);

        // Write output back
        if let Some(mut pair) = audio.port_pair(0) {
            let mut channels = pair.channels()?.into_f32().ok_or(PluginError::Message("Expected f32 audio"))?;
            // Write left channel
            if let Some(ch) = channels.channel_pair(0) {
                match ch {
                    ChannelPair::InPlace(buf) => buf[..frames].copy_from_slice(left),
                    ChannelPair::InputOutput(_, out) => out[..frames].copy_from_slice(left),
                    ChannelPair::OutputOnly(buf) => buf[..frames].copy_from_slice(left),
                    _ => {}
                }
            }
            // Write right channel
            if let Some(ch) = channels.channel_pair(1) {
                match ch {
                    ChannelPair::InPlace(buf) => buf[..frames].copy_from_slice(right),
                    ChannelPair::InputOutput(_, out) => out[..frames].copy_from_slice(right),
                    ChannelPair::OutputOnly(buf) => buf[..frames].copy_from_slice(right),
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
            output_channels: P::OUTPUT_CHANNELS,
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

        Ok(ClapMainThread {
            host,
            shared,
            plugin: Some(plugin),
            last_latency: 0,
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
            1
        }
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index != 0 {
            return;
        }
        if is_input {
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
                    in_place_pair: Some(ClapId::new(2)),
                });
            }
        } else {
            let ch = self.shared.output_channels;
            writer.set(&AudioPortInfo {
                id: ClapId::new(2),
                name: b"Output",
                channel_count: ch,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(if ch == 1 {
                    AudioPortType::MONO
                } else {
                    AudioPortType::STEREO
                }),
                in_place_pair: if self.shared.input_channels.is_some() {
                    Some(ClapId::new(1))
                } else {
                    None
                },
            });
        }
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
            plugin.save_state()
        } else {
            // Plugin is in audio processor — serialize from shared atomics
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
            crate::state::save_params(&refs)
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
            // Plugin is active — update shared atomics and mark dirty for audio processor
            if !crate::state::load_params_from_shared(
                &self.shared.param_metas,
                &self.shared.param_values,
                &data,
            ) {
                return Err(PluginError::Message("Failed to load state"));
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
