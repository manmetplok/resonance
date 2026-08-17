//! Persisted data model for Resonance projects.
//!
//! All serde structs, tag converters, format constants, and the two
//! accumulator types ([`LoadedProject`], [`SaveCollector`]) live here.
//! File-system I/O (save / load / autosave / backup) lives in [`super::io`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use resonance_audio::types::{ClipId, FadeCurve, MidiNote, PluginInstanceId, SendSource};

pub const PROJECT_FORMAT_VERSION: u32 = 2;

/// File name of the canonical project-metadata document inside a `.rproj`
/// directory. A manual save (over)writes this file.
pub const PROJECT_JSON: &str = "project.json";

/// File name of the autosave snapshot, written alongside
/// [`PROJECT_JSON`]. Kept as a *separate* side file (never overwriting
/// the canonical `project.json`) so a crash mid-autosave can only ever
/// truncate the snapshot, leaving the last manual save fully intact.
/// See epic #32 / doc #171 "Autosave triggering".
pub const AUTOSAVE_JSON: &str = "project.autosave.json";

/// On-disk project format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: u32,
    pub sample_rate: u32,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub metronome_enabled: bool,
    pub master_volume: f32,
    /// FX plugins inserted on the master bus. Empty on legacy projects.
    #[serde(default)]
    pub master_plugins: Vec<ProjectPlugin>,
    /// Whether the master FX chain is bypassed. `false` on legacy projects.
    #[serde(default)]
    pub master_fx_bypassed: bool,
    #[serde(alias = "punch_enabled")]
    pub loop_enabled: bool,
    #[serde(alias = "punch_in")]
    pub loop_in: u64,
    #[serde(alias = "punch_out")]
    pub loop_out: u64,
    pub tracks: Vec<ProjectTrack>,
    pub clips: Vec<ProjectClip>,
    #[serde(default)]
    pub midi_clips: Vec<ProjectMidiClip>,
    #[serde(default)]
    pub busses: Vec<ProjectBus>,
    /// The aux-send graph (ba doc #273): every tap from a track (or bus)
    /// into a return bus, with its level and pre/post tap point. Sends
    /// live beside the busses rather than on the source track because a
    /// send is an edge between two entities, not a property of either —
    /// the same reason [`ProjectTrack::output_bus`] is only half a route.
    /// Empty on legacy projects (everything saved before this field
    /// existed), which load with no sends exactly as they did before.
    #[serde(default)]
    pub sends: Vec<ProjectSend>,
    /// External sidechain (key) routes (ba doc #157/#159, todo #1311):
    /// which plugin's detector is keyed from which track or bus. Stored
    /// at project scope beside the sends for the same reason they are —
    /// a route is an edge, and its two ends live in different places
    /// (the target plugin may be on a track, a bus, or master).
    ///
    /// Empty on legacy projects, which is exactly how every project
    /// written before this field existed behaved: a duck dialled in over
    /// the control API was gone the next time the project opened, because
    /// the route only ever existed inside the engine thread.
    #[serde(default)]
    pub sidechain_routes: Vec<ProjectSidechainRoute>,
    #[serde(default)]
    pub section_definitions: Vec<crate::project::sections::ProjectSectionDefinition>,
    #[serde(default)]
    pub section_placements: Vec<crate::project::sections::ProjectSectionPlacement>,
    /// Tempo change events on the tempo track. Empty on legacy projects
    /// (a single event at bar 0 with the project BPM is inferred).
    #[serde(default)]
    pub tempo_events: Vec<crate::state::TempoEvent>,
    /// Time signature change events. Empty on legacy projects.
    #[serde(default)]
    pub signature_events: Vec<crate::state::SignatureEvent>,
    /// Whether the engine should emit MIDI clock to a hardware port.
    #[serde(default)]
    pub midi_clock_send_enabled: bool,
    /// Hardware MIDI output port carrying the master clock.
    #[serde(default)]
    pub midi_clock_send_device: Option<String>,
    /// Whether the engine should slave to incoming MIDI clock.
    #[serde(default)]
    pub midi_clock_recv_enabled: bool,
    /// Hardware MIDI input port carrying the master clock.
    #[serde(default)]
    pub midi_clock_recv_device: Option<String>,
    /// Project-scoped drum groups. Each group owns its grid/cycle/phase
    /// and per-pad patterns; the audio rendering reads these to materialise
    /// the drum track's MIDI clip on every section placement. Empty on
    /// legacy projects (which then get the built-in default kit/snare/hat
    /// layout the first time a new session opens).
    ///
    /// **Legacy field.** New projects persist the
    /// [`drum_patterns`](Self::drum_patterns) bank instead and leave this
    /// empty. On load we promote a non-empty legacy list into a single
    /// "Main" entry in the bank so projects round-trip cleanly.
    #[serde(default)]
    pub drum_groups: Vec<crate::compose::DrumGroup>,
    /// Project-scoped drum pattern bank. Each pattern owns its own set
    /// of [`DrumGroup`]s; section definitions reference patterns by id
    /// via `drum_pattern_id`. Empty on legacy projects (the loader then
    /// builds a single-pattern bank from `drum_groups`).
    #[serde(default)]
    pub drum_patterns: Vec<crate::compose::DrumPattern>,
    /// Track groups (folder tracks) for project organisation.
    #[serde(default)]
    pub track_groups: Vec<resonance_common::track_group::TrackGroup>,
    /// Loaded A/B reference tracks (external mastered tracks the user
    /// auditions against the mix). Empty on legacy projects. These are
    /// **monitor-only** — see [`ProjectReferenceSettings::monitor_only`] —
    /// and never participate in any render or export.
    #[serde(default)]
    pub references: Vec<ProjectReference>,
    /// Panel-level A/B settings (active selection, monitored source,
    /// loudness-match / trim, loop-to-mix). Defaults to a neutral,
    /// mix-monitoring state on legacy projects.
    #[serde(default)]
    pub reference_settings: ProjectReferenceSettings,
    /// Arrangement markers on the timeline. Empty on legacy projects.
    #[serde(default)]
    pub arrangement_markers: Vec<crate::state::ArrangementMarker>,
    /// Imported media-pool assets (doc #175). Each entry is an audio file
    /// transcoded into the project's `audio/` directory that clips
    /// reference by id via [`ProjectClip::asset_ref`]. Empty on legacy
    /// projects (which had no media pool); such projects load with an
    /// empty pool and every clip's `asset_ref` left `None`.
    #[serde(default)]
    pub pool_assets: Vec<ProjectPoolAsset>,
    /// Project groove library: user-extracted
    /// [`GrooveTemplate`](resonance_audio::quantize::GrooveTemplate)s
    /// (ba todo #395) the user saved for reuse, each with a stable
    /// per-project id and display name. Stock grooves are *not* duplicated
    /// here — they live in code and are referenced by index from
    /// [`quantize_settings`](Self::quantize_settings). Empty on legacy
    /// projects.
    #[serde(default)]
    pub groove_library: Vec<crate::state::UserGroove>,
    /// Last-used MIDI quantize / humanize settings (ba todo #395),
    /// restored as the quantize panel's defaults on load. Neutral
    /// defaults on legacy projects.
    #[serde(default)]
    pub quantize_settings: crate::state::QuantizeSettings,
    /// Parameter-automation lanes (epic #14 / epic #40), one per
    /// [`AutomationTarget`](resonance_common::AutomationTarget). Persisted so a
    /// project round-trips its automation — in particular the epic #40
    /// `DeviceParam` lanes that drive external-synth CC/NRPN, which are
    /// re-applied on load *after* each track's `SetTrackDeviceParams` so the
    /// engine knows the bindings. Empty on legacy projects (which then load
    /// with no automation, exactly as before).
    #[serde(default)]
    pub automation_lanes: Vec<resonance_common::AutomationLane>,
    /// Performance-mode footer selection (epic #11, todo #312): which
    /// instrument tuning the live fingering diagrams are drawn for and the
    /// capo offset. Defaults to Guitar 6 / no capo on legacy projects that
    /// predate Performance mode.
    #[serde(default)]
    pub performance: ProjectPerformance,
}

/// An empty project at the current format version with neutral
/// defaults (44.1 kHz, 120 BPM, 4/4). Purely a convenience for tests
/// and `..Default::default()` literals — deserialization is governed
/// by the per-field `#[serde(default)]` attributes above, not by this
/// impl.
impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: PROJECT_FORMAT_VERSION,
            sample_rate: 44100,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            master_volume: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            tracks: Vec::new(),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            busses: Vec::new(),
            sends: Vec::new(),
            sidechain_routes: Vec::new(),
            section_definitions: Vec::new(),
            section_placements: Vec::new(),
            tempo_events: Vec::new(),
            signature_events: Vec::new(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
            drum_groups: Vec::new(),
            drum_patterns: Vec::new(),
            track_groups: Vec::new(),
            references: Vec::new(),
            reference_settings: ProjectReferenceSettings::default(),
            arrangement_markers: Vec::new(),
            pool_assets: Vec::new(),
            groove_library: Vec::new(),
            quantize_settings: crate::state::QuantizeSettings::default(),
            automation_lanes: Vec::new(),
            performance: ProjectPerformance::default(),
        }
    }
}

/// Persisted Performance-mode footer selection (epic #11, todo #312):
/// the instrument tuning the live fingering diagrams are drawn for and the
/// capo offset. Mirrors the durable subset of
/// [`crate::state::PerformanceState`].
///
/// The tuning is stored by its stable display name
/// ([`Tuning::name`](resonance_music_theory::Tuning::name)) rather than its
/// `ALL_TUNINGS` index, so a project still resolves to the right instrument
/// if that list is ever reordered or extended. An unrecognised name (a
/// future build's tuning, or a hand-edited file) falls back to the default
/// Guitar 6 on load, mirroring the defensive clamping `PerformanceState`
/// already applies to a stale index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPerformance {
    /// Display name of the selected tuning, e.g. `"Guitar (6-string)"`.
    /// Resolved back to an `ALL_TUNINGS` index on load; an unknown name
    /// falls back to the default (Guitar 6).
    #[serde(default = "default_performance_tuning")]
    pub tuning: String,
    /// Capo position in frets (`0` = no capo).
    #[serde(default)]
    pub capo: u8,
}

/// Default [`ProjectPerformance::tuning`] for projects saved before
/// Performance mode existed: Guitar 6, the first entry in `ALL_TUNINGS` and
/// the footer's default selection.
fn default_performance_tuning() -> String {
    resonance_music_theory::GUITAR_6.name.to_string()
}

impl Default for ProjectPerformance {
    fn default() -> Self {
        Self {
            tuning: default_performance_tuning(),
            capo: 0,
        }
    }
}

/// One persisted A/B reference track. Holds only the durable, on-disk
/// facts: the source path, its display name, the cached integrated
/// loudness (so the loudness readout doesn't blank until the re-decode
/// finishes), and the user's comparison markers. The decoded PCM and
/// waveform overview are intentionally **not** persisted — they are
/// rebuilt by re-issuing `LoadReferenceTrack` on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReference {
    /// Absolute path to the source audio file.
    pub path: String,
    /// Display name (file stem unless the engine supplied one).
    pub name: String,
    /// Cached integrated loudness (LUFS) measured during analysis, so the
    /// readout shows a value before the re-decode completes. May be
    /// `-inf` if the original analysis never finished.
    pub integrated_lufs: f32,
    /// User-placed comparison markers, in the order they were saved.
    #[serde(default)]
    pub markers: Vec<ProjectReferenceMarker>,
}

/// A persisted comparison marker on a reference track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReferenceMarker {
    /// Per-reference marker id.
    pub id: u32,
    /// Position within the reference track, in sample frames.
    pub position_samples: u64,
    /// User-facing label.
    pub label: String,
}

/// Persisted panel-level A/B settings. Mirrors the durable subset of
/// `reference::ReferenceState`, addressing the active reference by its
/// index into [`ProjectFile::references`] rather than by engine id (ids
/// are reallocated on load).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReferenceSettings {
    /// Always `true`: a sentinel asserting (per design doc #198) that the
    /// persisted reference block is monitor-only and must never reach a
    /// render or export. Kept on disk as self-documenting provenance.
    #[serde(default = "default_true")]
    pub monitor_only: bool,
    /// Index into [`ProjectFile::references`] of the active reference, or
    /// `None` when nothing is selected.
    #[serde(default)]
    pub active: Option<usize>,
    /// Whether the monitored source was the reference (else the mix).
    /// Stored as a bool because the engine `ABSource` type deliberately
    /// carries no serde derive.
    #[serde(default)]
    pub ab_source_is_reference: bool,
    /// Whether the active reference is loudness-matched to the mix.
    #[serde(default)]
    pub loudness_match: bool,
    /// Manual level trim (dB) on top of any loudness match.
    #[serde(default)]
    pub trim_db: f32,
    /// Whether the reference cursor follows the mix transport.
    #[serde(default)]
    pub loop_to_mix: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProjectReferenceSettings {
    fn default() -> Self {
        Self {
            monitor_only: true,
            active: None,
            ab_source_is_reference: false,
            loudness_match: false,
            trim_db: 0.0,
            loop_to_mix: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTrack {
    pub id: u64,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    /// Whether the track's FX chain is bypassed. Default `false` keeps
    /// legacy projects loadable.
    #[serde(default)]
    pub fx_bypassed: bool,
    pub record_armed: bool,
    pub monitor_enabled: bool,
    /// External-instrument playback source (doc #257): whether the track
    /// plays recorded takes (`Recorded`) or re-drives the hardware live
    /// (`Live`). Persisted beside monitor/record-arm since it shares
    /// their engine-owned lifecycle. Default `Live` keeps legacy
    /// projects loadable and is exactly the pre-mode behaviour.
    #[serde(default)]
    pub playback_source: resonance_common::PlaybackSource,
    pub mono: bool,
    pub input_device_name: Option<String>,
    /// 0-indexed starting input channel on the track's input device.
    /// None on legacy projects (loads as 0, i.e. first channel pair).
    #[serde(default)]
    pub input_port_index: Option<u16>,
    pub plugins: Vec<ProjectPlugin>,
    #[serde(default = "default_track_type")]
    pub track_type: String,
    /// If Some, the track routes to this bus id. None (default on old
    /// projects) means the track routes directly to master.
    #[serde(default)]
    pub output_bus: Option<u64>,
    /// Instrument sub-type (synth/drum) for display in Compose. Default for
    /// legacy projects is `Synth`.
    #[serde(default)]
    pub instrument_type: crate::state::InstrumentType,
    /// Display icon for the instrument. Default for legacy projects is the
    /// icon matching `instrument_type`.
    #[serde(default)]
    pub instrument_icon: crate::state::InstrumentIcon,
    /// Arrangement role for derive-from-chords flows. Legacy projects load
    /// with `None`, meaning the track is not auto-picked by any derive.
    #[serde(default)]
    pub role: Option<crate::state::TrackRole>,
    /// When set, this track is a sub-track driven by a non-main output
    /// port of `parent_track_id`'s instrument plugin. Legacy projects
    /// load with `None` (no sub-tracks existed before this feature).
    #[serde(default)]
    pub sub_track: Option<crate::state::SubTrackLink>,
    /// Hardware MIDI input device name. `None` on legacy projects and
    /// on tracks the user hasn't assigned.
    #[serde(default)]
    pub midi_input_device: Option<String>,
    /// Hardware MIDI input channel filter (0..=15) or omni.
    #[serde(default)]
    pub midi_input_channel: Option<u8>,
    /// Hardware MIDI output device name.
    #[serde(default)]
    pub midi_output_device: Option<String>,
    /// Hardware MIDI output channel (0..=15), or `None` = channel 1.
    #[serde(default)]
    pub midi_output_channel: Option<u8>,
    /// External-instrument config (doc #169). `Some` marks the track as an
    /// external instrument. `None` on plain tracks and legacy projects. The
    /// MIDI output / audio-return devices live in the existing track fields
    /// (`midi_output_device`/`_channel`, `input_device_name`,
    /// `input_port_index`) and the monitor / record-arm flags on the track
    /// record; this carries only the bank/program + latency-offset extras.
    /// Runtime device-offline flags are not persisted (they reflect live
    /// hardware, re-checked after load).
    #[serde(default)]
    pub external_instrument: Option<ProjectExternalInstrument>,
    /// Persisted track-freeze state (ba todo #577). Defaults to
    /// [`TrackFreezeState::default`](resonance_common::TrackFreezeState)
    /// (live / unfrozen) so projects authored before freeze existed load
    /// unchanged. A frozen track records its
    /// [`FreezeCacheRef`](resonance_common::FreezeCacheRef) here; on load
    /// the cache WAV is re-decoded and re-attached via
    /// `AudioCommand::SetTrackFrozenSource` so reopening replays the cache
    /// without re-rendering. A missing / corrupt cache loads the track as
    /// stale (offer refreeze) rather than failing the project.
    #[serde(default)]
    pub freeze: resonance_common::TrackFreezeState,
}

/// On-disk external-instrument config (doc #169). Mirrors the persistable
/// fields of [`resonance_common::ExternalInstrument`]; the track id is implied
/// by the owning [`ProjectTrack`] and the runtime offline flags are not saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectExternalInstrument {
    /// Selected device-preset id (epic #40, doc #201 §5), e.g. `"moog-muse"`,
    /// or `None` when no preset is chosen. On load this resolves — via the
    /// [`DeviceDefinitionRegistry`](resonance_common::DeviceDefinitionRegistry)
    /// or the embedded [`device_definition`](Self::device_definition) — to the
    /// automatable params re-sent to the engine as `SetTrackDeviceParams`.
    /// `None` (the default) on legacy projects and tracks with no device
    /// selected, which load and behave exactly as before.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Embedded copy of a **user-authored** device definition so a project
    /// referencing a non-bundled device reopens on another machine even when
    /// the user's `device_definitions` folder isn't present there (doc #201
    /// §5 — chosen over a project-relative path so the project is
    /// self-contained). `None` for **bundled** devices — those ship inside the
    /// app and are re-resolved from the registry on load, so we never bloat
    /// the file with a copy — and whenever no device is selected.
    #[serde(default)]
    pub device_definition: Option<resonance_common::DeviceDefinition>,
    /// Selected MIDI bank (combined 14-bit MSB << 7 | LSB), or `None`.
    #[serde(default)]
    pub bank: Option<u16>,
    /// Selected MIDI program (`0..=127`), or `None`.
    #[serde(default)]
    pub program: Option<u8>,
    /// Manual latency offset in samples aligning the audio return.
    #[serde(default)]
    pub latency_offset_samples: i64,
}

/// On-disk bus state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectBus {
    pub id: u64,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    #[serde(default)]
    pub fx_bypassed: bool,
    pub plugins: Vec<ProjectPlugin>,
    /// Whether this bus is an FX *return* — the destination half of the
    /// aux-send graph in [`ProjectFile::sends`]. Set as part of the
    /// add-send gesture, so it is saved with the sends rather than left
    /// to be re-derived; without it a reloaded project's return bus
    /// reads back as an ordinary sub-mix bus. `false` on legacy projects
    /// and on every plain bus.
    #[serde(default)]
    pub is_return: bool,
}

/// One persisted aux send (ba doc #273): a tap from a track (or bus)
/// into a return bus, independent of where the source's main output
/// goes. Mirrors the durable fields of
/// [`AuxSend`](resonance_audio::types::AuxSend).
///
/// The source is stored as a `(kind, id)` pair rather than a tagged
/// enum because [`SendSource`] carries no serde derive — the same
/// reason [`ProjectClip::fade_in_curve`] and [`ProjectPoolAsset::format`]
/// round-trip through short lowercase tags. See [`send_source_tag`] /
/// [`send_source_from_tag`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSend {
    /// Engine-allocated send id. Handed back to the engine on load as a
    /// `SetAuxSend` id hint, so send ids survive a reload and any
    /// automation / client reference to one stays valid.
    pub id: u64,
    /// What feeds the send: `"track"` or `"bus"` (see
    /// [`send_source_tag`]). An unknown tag loads as a track source.
    #[serde(default = "default_send_source_kind")]
    pub source_kind: String,
    /// Id of the source track or bus, per [`Self::source_kind`].
    pub source_id: u64,
    /// Destination return bus.
    pub dest_bus: u64,
    /// Send gain in decibels applied to the tapped signal.
    #[serde(default)]
    pub level_db: f32,
    /// Whether the tap is taken before the source's volume fader.
    #[serde(default)]
    pub pre_fader: bool,
    /// A disabled send keeps its routing and level but passes no signal.
    /// Defaults to `true` so a hand-written entry that omits the field
    /// is a live send, which is what writing one down means.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Default [`ProjectSend::source_kind`]: a track tap, by far the common
/// case and the only one the mixer UI can create today.
fn default_send_source_kind() -> String {
    "track".to_string()
}

/// One persisted external sidechain (key) route (ba doc #157/#159, todo
/// #1311): the plugin whose detector is driven from somewhere else, and
/// the track or bus that drives it.
///
/// A route is stored **once, at project scope**, not on the plugin slot
/// that hosts the target — for the same reason [`ProjectSend`] lives
/// beside the busses rather than on the source track. A key route is an
/// edge between two entities and belongs to neither: the target plugin
/// can sit on a track, a bus, or the master chain, and the source can be
/// a track or a bus. Hanging it off one end would need the same struct in
/// three places and still could not say what the other end was.
///
/// `plugin_instance_id` survives a reload because every plugin is
/// re-instantiated with its saved instance id as an `id_hint` (see
/// `replay_plugins`) — the same guarantee automation lanes and plugin
/// state blobs already rely on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSidechainRoute {
    /// The keyed plugin instance. At most one entry per instance: a
    /// detector has exactly one input.
    pub plugin_instance_id: u64,
    /// What feeds the key: `"track"` or `"bus"`, per [`send_source_tag`].
    /// An unknown tag drops the route rather than guessing — see
    /// [`send_source_from_tag`] for why re-pointing it would be worse
    /// than losing it.
    #[serde(default = "default_send_source_kind")]
    pub source_kind: String,
    /// Id of the source track or bus, per [`Self::source_kind`].
    pub source_id: u64,
    /// A disabled route keeps its source but delivers no key, so the
    /// plugin falls back to keying off its own input. Defaults to `true`
    /// so a hand-written entry that omits the field is a live route,
    /// which is what writing one down means.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Split a [`SendSource`] into the `(kind, id)` pair stored on
/// [`ProjectSend`]. The engine type has no serde derive, so the project
/// layer owns this round-trip. Kept in sync with
/// [`send_source_from_tag`].
pub fn send_source_tag(source: SendSource) -> (&'static str, u64) {
    match source {
        SendSource::Track(id) => ("track", id),
        SendSource::Bus(id) => ("bus", id),
    }
}

/// Resolve a persisted `(kind, id)` tag pair back to a [`SendSource`].
///
/// `None` for a tag this build does not know, and the caller drops the
/// send. Deliberately NOT defaulting to `Track(id)` the way the
/// fade-curve tag defaults to a curve shape: track ids and bus ids are
/// independent namespaces that both start at 1, so an unrecognised tag
/// — a file from a newer build with a third source kind, or a
/// hand-edited `"Bus"` — would not pick a different flavour of the same
/// route, it would point the send at a DIFFERENT ENTITY that probably
/// exists, silently summing the wrong signal into the destination bus
/// with no error anywhere.
pub fn send_source_from_tag(kind: &str, id: u64) -> Option<SendSource> {
    match kind {
        "track" => Some(SendSource::Track(id)),
        "bus" => Some(SendSource::Bus(id)),
        _ => None,
    }
}

fn default_track_type() -> String {
    "audio".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPlugin {
    pub instance_id: u64,
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub clap_file_path: String,
    pub state_file: String,
    /// Parameter values that differ from the plugin's own defaults, as
    /// last seen by the app (doc-free bug report: "plugin parameters are
    /// not persisted in the project file").
    ///
    /// **Why this exists when `state_file` already holds a CLAP state
    /// blob.** The blob is the plugin's opaque self-serialization and it
    /// is the *only* thing that reached disk before. Two things made it
    /// insufficient:
    ///
    /// 1. The host queries a plugin's `ParamInfo` list exactly once, at
    ///    instantiation, and ships it in `AudioEvent::PluginAdded`. On
    ///    load that event lands *before* the `LoadPluginState` blob is
    ///    applied, and nothing re-reads the params afterwards — so the
    ///    app-side mirror (`PluginSlotState::params`) that
    ///    `track.plugin_params` / the mixer panel / automation all read
    ///    kept reporting instantiation-time **defaults** no matter what
    ///    the blob restored.
    /// 2. A param set while the transport is stopped is queued in the
    ///    CLAP host and only flushed inside `process()`, so the blob
    ///    written by a save-while-stopped could itself be stale.
    ///
    /// Persisting the values explicitly fixes both: they round-trip in
    /// plain, inspectable JSON, are re-applied to the mirror *and*
    /// re-sent to the engine after the blob on load, and are independent
    /// of whether the plugin implements CLAP state at all.
    ///
    /// Only non-default values are stored, so the file stays small and a
    /// plugin that adds parameters in a later version picks up its own
    /// new defaults. Absent (`#[serde(default)]`) in projects saved
    /// before this field existed, which load exactly as they did before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ProjectPluginParam>,
}

/// One persisted plugin parameter override. Addressed by the stable CLAP
/// param id; `name` is carried alongside purely so a human reading
/// `project.json` can tell what a numeric id means (it is never used to
/// match on load — a renamed param must still restore).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPluginParam {
    pub id: u32,
    #[serde(default)]
    pub name: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectClip {
    pub id: u64,
    pub track_id: u64,
    pub start_sample: u64,
    pub name: String,
    pub total_frames: u64,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
    /// Project-relative path to the clip's WAV file, e.g.
    /// `"audio/clip_42.wav"`. Absolute paths are resolved against
    /// the project directory at load time.
    pub audio_file: String,
    /// Id of the media-pool asset this clip was placed from (doc #175),
    /// or `None` for clips that aren't pool imports (recorded takes,
    /// bounces) and for legacy projects that predate the media pool.
    /// Rebuilt onto [`crate::state::ClipState::asset_ref`] on load.
    #[serde(default)]
    pub asset_ref: Option<u64>,
    /// Fade-in length in frames; `0` = no fade (epic #18, doc #156).
    /// Defaults to `0` so projects saved before fades existed load with
    /// no fade-in.
    #[serde(default)]
    pub fade_in_frames: u64,
    /// Curve shaping the fade-in ramp, stored as a short lowercase tag
    /// (see [`fade_curve_tag`]). [`FadeCurve`] carries no serde derive,
    /// so the project layer owns this round-trip. Defaults to the
    /// `EqualPower` tag for older projects.
    #[serde(default = "default_fade_curve_tag")]
    pub fade_in_curve: String,
    /// Fade-out length in frames; `0` = no fade (epic #18, doc #156).
    #[serde(default)]
    pub fade_out_frames: u64,
    /// Curve shaping the fade-out ramp, stored as a tag (see
    /// [`fade_curve_tag`]). Defaults to the `EqualPower` tag.
    #[serde(default = "default_fade_curve_tag")]
    pub fade_out_curve: String,
    /// Per-clip gain in decibels; `0.0` dB = unity (epic #18, doc #156).
    /// Defaults to `0.0` so older projects load at unity gain.
    #[serde(default)]
    pub gain_db: f32,
}

/// Default [`ProjectClip::fade_in_curve`] / [`ProjectClip::fade_out_curve`]
/// tag for projects saved before fade curves existed: the engine default,
/// `EqualPower`. Kept in sync with [`fade_curve_tag`].
fn default_fade_curve_tag() -> String {
    fade_curve_tag(FadeCurve::default()).to_string()
}

/// Serialize a [`FadeCurve`] to the short lowercase tag stored in
/// [`ProjectClip::fade_in_curve`] / [`ProjectClip::fade_out_curve`]. The
/// engine type has no serde derive, so the project layer owns this
/// round-trip. Kept in sync with [`fade_curve_from_tag`].
pub fn fade_curve_tag(curve: FadeCurve) -> &'static str {
    match curve {
        FadeCurve::Linear => "linear",
        FadeCurve::EqualPower => "equal_power",
        FadeCurve::Exp => "exp",
    }
}

/// Parse a fade-curve tag back into a [`FadeCurve`]. Unknown / unexpected
/// tags (a future build's new variant, or a hand-edited file) fall back to
/// [`FadeCurve::default`] so loading never fails on the curve label.
pub fn fade_curve_from_tag(tag: &str) -> FadeCurve {
    match tag {
        "linear" => FadeCurve::Linear,
        "equal_power" => FadeCurve::EqualPower,
        "exp" => FadeCurve::Exp,
        _ => FadeCurve::default(),
    }
}

/// One persisted media-pool asset (doc #175): an imported audio file
/// that clips reference by id. The engine transcodes every import to a
/// project-rate stereo f32 WAV under the project's `audio/` directory;
/// `project_relative_path` points at it (relocatable with the project),
/// while the remaining fields describe the original source for display.
/// The waveform thumbnail and live usage counts are runtime-derived and
/// deliberately *not* persisted — they're rebuilt on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPoolAsset {
    /// Stable per-project asset id, matched by [`ProjectClip::asset_ref`].
    pub id: u64,
    /// Project-relative path of the engine-format WAV, e.g.
    /// `"audio/asset_7.wav"`. Resolved against the project directory to
    /// detect whether the backing file is still present.
    pub project_relative_path: String,
    /// Absolute path of the source file the user originally imported
    /// (provenance / relink hint).
    pub original_path: String,
    /// Container/codec family of the original source, stored as a short
    /// lowercase tag (`"wav"`, `"flac"`, `"mp3"`, `"ogg"`, `"aac"`,
    /// `"mp4"`, or `"other"`). `resonance_common::AudioFormat` carries no
    /// serde derive, so it round-trips through this string.
    pub format: String,
    /// Channel count of the original source file.
    pub channels: u16,
    /// Sample rate of the original source file, in Hz.
    pub source_sample_rate: u32,
    /// Per-channel frame count of the imported (project-rate) WAV.
    pub duration_frames: u64,
}

/// Serialize an [`AudioFormat`](resonance_common::AudioFormat) to the
/// short lowercase tag stored in [`ProjectPoolAsset::format`]. The
/// engine type has no serde derive, so the project layer owns this
/// round-trip. Kept in sync with [`audio_format_from_tag`].
pub fn audio_format_tag(format: resonance_common::AudioFormat) -> &'static str {
    use resonance_common::AudioFormat;
    match format {
        AudioFormat::Wav => "wav",
        AudioFormat::Flac => "flac",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Aac => "aac",
        AudioFormat::Mp4 => "mp4",
        AudioFormat::Other => "other",
    }
}

/// Parse a [`ProjectPoolAsset::format`] tag back into an
/// [`AudioFormat`](resonance_common::AudioFormat). Unknown / unexpected
/// tags (including a future build's new variant, or a hand-edited file)
/// fall back to `Other` so loading never fails on the format label.
pub fn audio_format_from_tag(tag: &str) -> resonance_common::AudioFormat {
    use resonance_common::AudioFormat;
    match tag {
        "wav" => AudioFormat::Wav,
        "flac" => AudioFormat::Flac,
        "mp3" => AudioFormat::Mp3,
        "ogg" => AudioFormat::Ogg,
        "aac" => AudioFormat::Aac,
        "mp4" => AudioFormat::Mp4,
        _ => AudioFormat::Other,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMidiClip {
    pub id: u64,
    pub track_id: u64,
    pub start_sample: u64,
    pub duration_ticks: u64,
    pub name: String,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
    /// Project-relative path to the clip's Standard MIDI File.
    pub midi_file: String,
    /// Per-note vocal lyric annotations, parallel to the clip's note
    /// list. Carries OpenUtau-style slur markers (`"+"`) and explicit
    /// per-note label overrides. Empty when the clip isn't on a vocal
    /// track or hasn't had any lyric edits applied. Trailing empty
    /// strings are stripped by the serializer to keep the JSON lean —
    /// the replay path pads back to `notes.len()` on load.
    #[serde(default)]
    pub vocal_lyrics: Vec<String>,
}

/// Everything needed to reconstruct a project after loading from disk.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub file: ProjectFile,
    /// Absolute path to the project directory (the `.rproj` folder).
    /// The replay step needs this to resolve `ProjectClip.audio_file`
    /// into the absolute path it hands the engine.
    pub project_dir: PathBuf,
    /// MIDI notes per clip id, read from the sibling `.mid` files.
    pub midi_notes: HashMap<ClipId, Vec<MidiNote>>,
    pub plugin_states: HashMap<PluginInstanceId, Vec<u8>>,
}

/// State accumulated during an async save operation. The engine
/// streams recorded audio straight to `audio/clip_{id}.wav`, so by
/// the time the save kicks off the clip files already exist on
/// disk; we only need to collect the confirmed path list and the
/// plugin state blobs, then write `project.json`.
pub struct SaveCollector {
    pub path: PathBuf,
    /// Map from clip id to project-relative WAV path, returned by
    /// `AudioEvent::ClipsSavedToProjectDir`.
    pub clip_files: HashMap<ClipId, String>,
    pub plugin_states: Vec<(PluginInstanceId, Vec<u8>)>,
    pub clips_done: bool,
    pub plugins_done: bool,
    /// True when this collector is servicing a periodic autosave rather
    /// than a manual save. Autosaves write their metadata to
    /// [`AUTOSAVE_JSON`], leave `dirty` set, skip the recents list and
    /// versioned backups, and update `last_autosave_at` on completion.
    pub autosave: bool,
}
