//! Centralized hard-coded limits for the audio engine.
//!
//! All capacity / size limits live here so they can be reviewed in one
//! place. Each constant documents why the limit exists and what happens
//! when it is hit.

/// Maximum number of output ports per plugin. Plugins declaring more
/// ports are truncated to this value. Used to pre-allocate per-port
/// output buffers on the audio thread.
pub const MAX_PLUGIN_OUTPUT_PORTS: usize = 8;

/// Maximum number of busses. `AddBus` is rejected once this limit is
/// reached and a warning is logged.
pub const MAX_BUSSES: usize = 32;

/// Maximum number of input channels read from the recording device.
/// Used to pre-allocate the input ring buffer.
pub const MAX_INPUT_CHANNELS: usize = 32;

/// Maximum MIDI events collected per buffer per track. Prevents the
/// pre-allocated `note_event_buf` Vec from reallocating on the audio
/// thread. Events beyond this count are silently dropped for the
/// current buffer.
pub const MAX_MIDI_EVENTS_PER_BUFFER: usize = 512;

/// Maximum number of metronome click positions resolved per buffer.
/// 16 beats covers common time signatures at typical buffer sizes.
pub const MAX_METRONOME_BEATS_PER_BUFFER: usize = 16;

/// Maximum pending parameter changes queued between process() calls.
/// Beyond this, new parameter changes for distinct param_ids are
/// silently dropped. Duplicate param_ids always update in place.
pub const MAX_PENDING_PARAMS: usize = 128;

/// Maximum pending note events queued between process() calls.
/// Prevents unbounded Vec growth on the audio thread. 256 covers
/// a full all_notes_off (128) plus a generous burst of new notes.
pub const MAX_PENDING_NOTES: usize = 256;

/// Maximum number of instrument plugins with stashed MIDI events at
/// once (see `mixer::midi_stash`). One slot per instrument whose mutex
/// was contended; slots free on the next successful lock, so hitting
/// this requires that many simultaneously contended instruments. When
/// the pool is exhausted the contended block's events are dropped.
pub const MAX_STASHED_INSTRUMENTS: usize = 64;

/// Maximum MIDI events stashed per instrument across contended blocks.
/// On overflow the slot degrades to an all-notes-off on the next
/// successful lock — note-offs are never silently lost.
pub const MAX_STASHED_EVENTS: usize = 256;

/// Maximum per-plugin latency (in samples) honoured by plugin-delay
/// compensation. ~10 s at 96 kHz. Larger reported values are clamped so
/// a misbehaving plugin can't make the engine allocate gigabyte-sized
/// delay lines (the compensation is then merely incomplete, not unsafe).
pub const MAX_COMP_LATENCY: u64 = 960_000;

/// Maximum number of undo history entries retained. Not
/// user-configurable yet.
pub const DEFAULT_HISTORY_CAPACITY: usize = 200;
