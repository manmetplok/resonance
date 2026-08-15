/// The ResonancePlugin trait -- what plugin authors implement.
use std::sync::Arc;

use crate::param::Param;

/// Saver for plugin state that lives outside the parameter list — file
/// paths, loaded resource handles, anything the plugin needs to persist
/// alongside its params.
///
/// The CLAP bridge harvests this handle once at plugin construction and
/// calls it from the main thread at project save / project load time.
/// Because the bridge may call `save`/`load` **while the plugin is in the
/// audio processor**, implementations must only touch thread-safe shared
/// state (Arcs, atomics, parking_lot mutexes) — never fields that the
/// plugin struct owns exclusively.
///
/// Keys returned from `save` are merged into the top level of the state
/// JSON object alongside `"params"`, so existing on-disk state formats
/// that use top-level keys remain readable.
pub trait ExtraStateSaver: Send + Sync {
    /// Return key-value pairs to merge into the top-level state JSON.
    fn save(&self) -> serde_json::Map<String, serde_json::Value>;

    /// Apply previously-saved state from the top-level JSON object.
    /// Implementations typically `state.get("my_key")` into their own
    /// shared storage.
    fn load(&self, state: &serde_json::Value);
}

/// A note event for sample-accurate MIDI processing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteEvent {
    NoteOn {
        note: u8,
        velocity: f32,
        timing: u32,
    },
    NoteOff {
        note: u8,
        timing: u32,
    },
    Choke {
        note: u8,
        timing: u32,
    },
}

impl NoteEvent {
    pub fn timing(&self) -> u32 {
        match self {
            NoteEvent::NoteOn { timing, .. } => *timing,
            NoteEvent::NoteOff { timing, .. } => *timing,
            NoteEvent::Choke { timing, .. } => *timing,
        }
    }
}

/// A sample-accurate MIDI controller event — everything a keyboard sends
/// that is not a note (ba todo #1295).
///
/// These live in their own type rather than as new [`NoteEvent`] variants so
/// that plugins which exhaustively match the three note variants keep
/// compiling; see [`EventIterator::next_any`] for how the two are delivered
/// in one ordered stream.
///
/// All values are normalised, because that is what a modulation matrix wants:
/// `0.0..=1.0` for continuous controllers and pressure, `-1.0..=1.0` for the
/// bipolar pitch bend. The raw 7-/14-bit numbers are not preserved.
///
/// `#[non_exhaustive]`: this is exactly the enum whose closed-ness caused
/// finding F3 in the first place. Match with a `_ =>` arm.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ControlEvent {
    /// MIDI control change. `controller` is the CC number (0..=127, e.g. 1
    /// for the mod wheel, 64 for sustain), `value` is `0.0..=1.0`.
    ControlChange {
        channel: u8,
        controller: u8,
        value: f32,
        timing: u32,
    },
    /// Channel pressure (channel aftertouch): one pressure value for the
    /// whole channel. `pressure` is `0.0..=1.0`.
    ChannelPressure {
        channel: u8,
        pressure: f32,
        timing: u32,
    },
    /// Polyphonic key pressure (poly aftertouch): pressure for one held key.
    /// `pressure` is `0.0..=1.0`.
    ///
    /// Arrives either as MIDI poly key pressure or, from a CLAP-dialect host,
    /// as a `CLAP_NOTE_EXPRESSION_PRESSURE` note expression — the bridge
    /// normalises both to this.
    PolyPressure {
        channel: u8,
        note: u8,
        pressure: f32,
        timing: u32,
    },
    /// Pitch bend, `-1.0..=1.0` with `0.0` at centre. How many semitones
    /// that spans is the plugin's own bend-range setting.
    PitchBend {
        channel: u8,
        value: f32,
        timing: u32,
    },
}

impl ControlEvent {
    /// Offset of this event from the start of the process block, in samples.
    pub fn timing(&self) -> u32 {
        match self {
            ControlEvent::ControlChange { timing, .. } => *timing,
            ControlEvent::ChannelPressure { timing, .. } => *timing,
            ControlEvent::PolyPressure { timing, .. } => *timing,
            ControlEvent::PitchBend { timing, .. } => *timing,
        }
    }

    /// The MIDI channel (0..=15) this event arrived on.
    pub fn channel(&self) -> u8 {
        match self {
            ControlEvent::ControlChange { channel, .. } => *channel,
            ControlEvent::ChannelPressure { channel, .. } => *channel,
            ControlEvent::PolyPressure { channel, .. } => *channel,
            ControlEvent::PitchBend { channel, .. } => *channel,
        }
    }
}

/// One entry of the input event stream a plugin receives, in host order.
///
/// Obtained from [`EventIterator::next_any`]. Plugins that only care about
/// notes keep using [`EventIterator::next_event`] and never see this type.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PluginEvent {
    Note(NoteEvent),
    Control(ControlEvent),
}

impl PluginEvent {
    /// Offset of this event from the start of the process block, in samples.
    pub fn timing(&self) -> u32 {
        match self {
            PluginEvent::Note(e) => e.timing(),
            PluginEvent::Control(e) => e.timing(),
        }
    }

    /// The note event, if this is one.
    pub fn as_note(&self) -> Option<NoteEvent> {
        match self {
            PluginEvent::Note(e) => Some(*e),
            _ => None,
        }
    }

    /// The controller event, if this is one.
    pub fn as_control(&self) -> Option<ControlEvent> {
        match self {
            PluginEvent::Control(e) => Some(*e),
            _ => None,
        }
    }
}

impl From<NoteEvent> for PluginEvent {
    fn from(event: NoteEvent) -> Self {
        PluginEvent::Note(event)
    }
}

impl From<ControlEvent> for PluginEvent {
    fn from(event: ControlEvent) -> Self {
        PluginEvent::Control(event)
    }
}

/// Describes one audio output port exposed by a plugin. Returned from
/// `ResonancePlugin::output_layout()` at activation time; used by the CLAP
/// bridge to declare audio ports to the host and by the host mixer to size
/// its per-port scratch buffers.
#[derive(Debug, Clone)]
pub struct OutputPortSpec {
    /// Human-readable name shown to the host (e.g. "Out", "Kick", "Snare").
    pub name: std::borrow::Cow<'static, str>,
    /// Number of audio channels for this port. Only 1 (mono) and 2 (stereo)
    /// are supported right now; everything else is rejected at activation.
    pub channel_count: u32,
}

/// Mutable stereo buffer pair for one output port, passed to
/// `ResonancePlugin::process()` in a slice — one entry per declared port in
/// `output_layout()` order. Plugins write their output directly into
/// `left` and `right` (already zeroed when the process call begins).
pub struct OutputBuffer<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

/// Read-only view of the external sidechain (key) signal for one process
/// block. Delivered to [`ResonancePlugin::process_with_key`] when the plugin
/// declares [`ResonancePlugin::SIDECHAIN_INPUT`] and the host has connected a
/// secondary input port.
///
/// Always presented stereo-shaped: a mono key port fills both `left` and
/// `right` with the same samples, so detectors can read either channel
/// without special-casing channel count. Both slices are exactly `frames`
/// samples long.
pub struct KeyBuffer<'a> {
    pub left: &'a [f32],
    pub right: &'a [f32],
}

/// Where an [`EventIterator`] reads from.
///
/// Two shapes rather than one so a caller holding a plain `&[NoteEvent]` —
/// every plugin test and benchmark in the workspace — needs no conversion and
/// no allocation, while the CLAP bridge can deliver notes and controllers
/// interleaved in exactly the order the host sent them.
enum EventSource<'a> {
    Notes(&'a [NoteEvent]),
    Mixed(&'a [PluginEvent]),
}

/// Iterator over the input events within a process block.
/// Borrows from a pre-allocated buffer to avoid audio-thread allocations.
///
/// Two ways to drain it, and a plugin should pick one:
/// - [`next_event`](Self::next_event) yields note events only, skipping
///   controllers. This is the original contract, unchanged.
/// - [`next_any`](Self::next_any) yields everything — notes *and* MIDI CC,
///   aftertouch and pitch bend — in host order.
pub struct EventIterator<'a> {
    source: EventSource<'a>,
    pos: usize,
}

impl<'a> EventIterator<'a> {
    /// Iterate a slice of note events.
    pub fn new(events: &'a [NoteEvent]) -> Self {
        Self {
            source: EventSource::Notes(events),
            pos: 0,
        }
    }

    /// Iterate a slice carrying both notes and controller events, in the
    /// order the host delivered them. This is what the CLAP bridge builds.
    pub fn mixed(events: &'a [PluginEvent]) -> Self {
        Self {
            source: EventSource::Mixed(events),
            pos: 0,
        }
    }

    pub fn empty() -> Self {
        Self {
            source: EventSource::Notes(&[]),
            pos: 0,
        }
    }

    /// Consume and return the next **note** event, skipping any controller
    /// events in between.
    ///
    /// A plugin that wants the controllers too must use
    /// [`next_any`](Self::next_any) instead — mixing the two drains one
    /// shared cursor and would drop events.
    pub fn next_event(&mut self) -> Option<NoteEvent> {
        loop {
            match self.next_any()? {
                PluginEvent::Note(event) => return Some(event),
                _ => continue,
            }
        }
    }

    /// Consume and return the next event of any kind, in host order.
    pub fn next_any(&mut self) -> Option<PluginEvent> {
        let event = match self.source {
            EventSource::Notes(events) => PluginEvent::Note(*events.get(self.pos)?),
            EventSource::Mixed(events) => *events.get(self.pos)?,
        };
        self.pos += 1;
        Some(event)
    }
}

/// Transport/tempo snapshot delivered once per process block.
///
/// `None` when the host (or offline renderer) doesn't supply transport.
#[derive(Debug, Clone, Copy)]
pub struct TempoInfo {
    pub bpm: f32,
    pub time_sig_num: u16,
    pub time_sig_den: u16,
    pub playing: bool,
    pub song_pos_beats: f64,
}

/// The main trait that plugin authors implement.
///
/// The CLAP bridge wraps this trait to produce a valid CLAP plugin.
pub trait ResonancePlugin: Send + 'static {
    /// CLAP plugin identifier (e.g. "com.resonance.reverb").
    const CLAP_ID: &'static str;
    /// Human-readable plugin name.
    const NAME: &'static str;
    /// Plugin vendor name.
    const VENDOR: &'static str;
    /// Plugin version string.
    const VERSION: &'static str;
    /// Short description.
    const DESCRIPTION: &'static str;
    /// CLAP feature strings, declared from [`crate::features`] — e.g.
    /// `&[features::AUDIO_EFFECT, features::REVERB, features::STEREO]`.
    ///
    /// They reach the host exactly as written; there is no translation
    /// step to fall out of. At least one main category is required, and
    /// the bridge checks that at compile time (ba todo #1298).
    const FEATURES: &'static [&'static std::ffi::CStr];

    /// Number of input channels. None = instrument (no audio input).
    const INPUT_CHANNELS: Option<u32>;
    /// Whether this plugin accepts MIDI note input.
    const MIDI_INPUT: bool = false;

    /// Number of channels on an optional external sidechain (key) input
    /// port. `None` (default) means the plugin declares **no** sidechain
    /// port and behaves exactly as before: at most one (main) input port.
    /// `Some(1)` / `Some(2)` opts into a secondary, **non-main** CLAP input
    /// port (CLAP `IS_MAIN` cleared, distinct port id) carrying an external
    /// key signal; the per-block key buffer is delivered to
    /// [`process_with_key`](ResonancePlugin::process_with_key). Only mono (1)
    /// and stereo (2) are supported; other values are rejected at plugin
    /// construction.
    const SIDECHAIN_INPUT: Option<u32> = None;

    /// Describe the plugin's audio output layout. Called once at activation
    /// and cached for the plugin's lifetime — **do not** change the port
    /// count across activations, the host caches it and sizes buffers
    /// accordingly. Default: a single stereo output named "Out".
    ///
    /// Port 0 is always the "main" output; plugins that declare multiple
    /// ports conventionally put their primary / mix-down output at index 0.
    fn output_layout(&self) -> Vec<OutputPortSpec> {
        vec![OutputPortSpec {
            name: std::borrow::Cow::Borrowed("Out"),
            channel_count: 2,
        }]
    }

    /// Create a new instance of the plugin.
    fn new() -> Self;

    /// Return the number of parameters.
    fn param_count(&self) -> usize;

    /// Return a reference to the parameter at the given index.
    fn param(&self, index: usize) -> &dyn Param;

    /// Return all parameters as a Vec (convenience, allocates).
    /// Default implementation builds from param_count/param.
    fn params(&self) -> Vec<&dyn Param> {
        (0..self.param_count()).map(|i| self.param(i)).collect()
    }

    /// Called once before processing begins. Return false on failure.
    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool;

    /// Reset all internal state (e.g. delay lines, filters).
    fn reset(&mut self);

    /// Process a buffer of audio.
    ///
    /// `outputs` is a slice of stereo buffer pairs, one per declared output
    /// port in `output_layout()` order. Each buffer has `frames` samples
    /// and is already zeroed when the plugin is called (instrument path)
    /// or pre-filled with the incoming audio (effect path on port 0).
    ///
    /// Single-output plugins simply write into `outputs[0].left` /
    /// `outputs[0].right`. Multi-output plugins (e.g. resonance-drums with
    /// its 7 group/overhead ports) fan out to the full slice.
    ///
    /// `events` provides sample-accurate note events.
    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    );

    /// Process a buffer of audio with an optional external sidechain (key)
    /// signal alongside the main input.
    ///
    /// The CLAP bridge always calls this method; the default implementation
    /// discards the key and forwards to [`process`](ResonancePlugin::process),
    /// so plugins that don't declare [`SIDECHAIN_INPUT`](ResonancePlugin::SIDECHAIN_INPUT)
    /// — and existing plugins that never override it — are completely
    /// unaffected.
    ///
    /// Plugins that opt into a sidechain port override **this** method and
    /// read `key` (the external key for this block, or `None` when the host
    /// has not connected the sidechain port). `outputs`, `frames`, `events`
    /// and `tempo` carry the same contract as [`process`](ResonancePlugin::process).
    fn process_with_key(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        key: Option<KeyBuffer<'_>>,
        frames: usize,
        events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        let _ = key;
        self.process(outputs, frames, events, tempo);
    }

    /// Save plugin state to bytes. Default: JSON serialization of params
    /// composed with whatever `extra_state_saver()` returns (so plugins that
    /// just need a couple of extra file-path fields can skip overriding
    /// this entirely and provide a saver instead).
    fn save_state(&self) -> Vec<u8> {
        let mut json = crate::state::params_to_json(&self.params());
        if let Some(saver) = self.extra_state_saver() {
            if let Some(obj) = json.as_object_mut() {
                for (k, v) in saver.save() {
                    obj.insert(k, v);
                }
            }
        }
        serde_json::to_vec(&json).unwrap_or_default()
    }

    /// Load plugin state from bytes. Default: JSON deserialization of
    /// params plus any `extra_state_saver()` contribution.
    fn load_state(&mut self, data: &[u8]) -> bool {
        let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) else {
            return false;
        };
        let ok = crate::state::load_params_from_json(&self.params(), &state);
        if let Some(saver) = self.extra_state_saver() {
            saver.load(&state);
        }
        ok
    }

    /// Optional handle that persists state outside the param list (file
    /// paths, resource pointers, etc.). Harvested once at plugin creation
    /// by the CLAP bridge and cached for the plugin's lifetime, so the
    /// bridge can save/load extra state even while the plugin is in the
    /// audio processor. Default: `None`.
    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        None
    }

    /// Report latency in samples. Default: 0.
    ///
    /// The bridge reads this at every activation, and on any host query made
    /// while the plugin is inactive. A plugin whose latency can change while
    /// it is active must *also* push the new figure through
    /// [`HostHandle::set_latency_samples`](crate::host::HostHandle::set_latency_samples)
    /// — the host cannot poll for it (CLAP only defines the query while
    /// active, and by then this object lives in the audio processor).
    fn latency_samples(&self) -> u32 {
        0
    }

    /// Receive the handle to the host that owns this instance.
    ///
    /// Called once by the CLAP bridge, on the main thread, right after
    /// `new()` and before the plugin can be activated. Plugins that need to
    /// talk back to the host — report a latency change, ask for a restart —
    /// store the handle; the default implementation drops it, which is what
    /// every plugin that only reads its inputs wants.
    ///
    /// The handle is `Send + Sync` and safe to call from the audio thread or
    /// an editor thread, so the usual shape is to keep it in an
    /// `Arc<Mutex<..>>`-free field or hand a clone to the editor.
    fn set_host(&mut self, host: Arc<crate::host::HostHandle>) {
        let _ = host;
    }

    /// Return an editor factory if this plugin has a GUI.
    ///
    /// Called once at plugin creation time (before the plugin is moved into
    /// the audio processor). Returning `Some(factory)` causes the clap_bridge
    /// to expose `CLAP_EXT_GUI`; the host can then open, resize, hide, show,
    /// and destroy the editor through the factory. Default: `None`.
    fn editor_factory(&self) -> Option<std::sync::Arc<dyn crate::gui::EditorFactory>> {
        None
    }
}
