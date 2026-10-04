//! Which note dialect a plugin is spoken to in (code review HOST-13).
//!
//! The host used to send `CLAP_EVENT_NOTE_ON` / `NOTE_OFF` to every
//! plugin and nothing else, without asking `clap.note-ports`: a plugin
//! that only accepts MIDI heard nothing, and no plugin ever got a mod
//! wheel, pitch bend or aftertouch. This reads the plugin's first note
//! input port at activation and decides, once, how notes travel and
//! whether raw MIDI controllers may be sent at all.

use clap_sys::ext::note_ports::{
    clap_note_port_info, clap_plugin_note_ports, CLAP_EXT_NOTE_PORTS, CLAP_NOTE_DIALECT_CLAP,
    CLAP_NOTE_DIALECT_MIDI, CLAP_NOTE_DIALECT_MIDI_MPE,
};
use clap_sys::plugin::clap_plugin;

/// How events reach the plugin's note input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NoteDialect {
    /// Note on / off go out as MIDI 1.0 (`CLAP_EVENT_MIDI`) rather than
    /// CLAP note events: the port prefers MIDI, or takes nothing else.
    pub(crate) notes_as_midi: bool,
    /// The port accepts MIDI 1.0, so controllers, pitch bend and
    /// aftertouch can be forwarded as `CLAP_EVENT_MIDI`.
    pub(crate) accepts_midi: bool,
    /// The plugin has a note input at all. One that declares
    /// `clap.note-ports` with no input ports gets no note events.
    pub(crate) has_note_input: bool,
}

impl NoteDialect {
    /// What a plugin without `clap.note-ports` gets: CLAP notes, as this
    /// host always sent, and no raw MIDI (nothing says it reads any).
    pub(crate) const LEGACY: Self = Self {
        notes_as_midi: false,
        accepts_midi: false,
        has_note_input: true,
    };

    /// Read the plugin's first note input port.
    ///
    /// # Safety
    /// `plugin` is a live, initialized plugin; main thread.
    pub(crate) unsafe fn query(plugin: *const clap_plugin) -> Self {
        let Some(get_ext) = (*plugin).get_extension else {
            return Self::LEGACY;
        };
        let ext = get_ext(plugin, CLAP_EXT_NOTE_PORTS.as_ptr()) as *const clap_plugin_note_ports;
        let Some(ext) = ext.as_ref() else {
            return Self::LEGACY;
        };
        let (Some(count), Some(get)) = (ext.count, ext.get) else {
            return Self::LEGACY;
        };
        if count(plugin, true) == 0 {
            return Self {
                notes_as_midi: false,
                accepts_midi: false,
                has_note_input: false,
            };
        }
        let mut info: clap_note_port_info = std::mem::zeroed();
        if !get(plugin, 0, true, &mut info) {
            return Self::LEGACY;
        }
        Self::from_dialects(info.supported_dialects, info.preferred_dialect)
    }

    /// The decision, from a port's `supported_dialects` /
    /// `preferred_dialect`. MPE is MIDI 1.0 on the wire, so it counts.
    pub(crate) fn from_dialects(supported: u32, preferred: u32) -> Self {
        let midi = CLAP_NOTE_DIALECT_MIDI | CLAP_NOTE_DIALECT_MIDI_MPE;
        let accepts_clap = supported & CLAP_NOTE_DIALECT_CLAP != 0;
        let accepts_midi = supported & midi != 0;
        let prefers_midi = preferred & midi != 0 && preferred & CLAP_NOTE_DIALECT_CLAP == 0;
        Self {
            notes_as_midi: accepts_midi && (prefers_midi || !accepts_clap),
            accepts_midi,
            // A port that names neither dialect we speak still gets CLAP
            // notes: better heard wrongly-tagged than not at all.
            has_note_input: true,
        }
    }
}
