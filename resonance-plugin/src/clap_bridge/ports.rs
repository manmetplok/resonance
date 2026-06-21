//! Audio-port and note-port discovery for the CLAP bridge.

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPortsImpl,
};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePortsImpl,
};
use clack_plugin::prelude::*;

use super::shared::ClapMainThread;
use crate::plugin::ResonancePlugin;

// ---------------------------------------------------------------------------
// Input-port policy (pure helpers, shared by the extension impl + processor)
// ---------------------------------------------------------------------------

/// Distinct CLAP port id for the optional sidechain (key) input port.
///
/// Chosen well above the output-port id range (`2 + index`, capped at
/// `MAX_OUTPUT_PORTS` = 8 ports → ids 2..=9) and the main input id (1), so a
/// plugin's sidechain port id can never collide with any of its other audio
/// ports. The host mixer that delivers the key (engine layer) targets this id.
pub const SIDECHAIN_PORT_ID: u32 = 64;

/// Number of CLAP input audio ports a plugin declares, given its optional
/// main input and optional sidechain (key) input.
///
/// - no inputs (instrument, no sidechain) → 0
/// - main input only → 1
/// - main input + sidechain → 2
/// - sidechain only (instrument keyed externally) → 1
#[inline]
pub fn input_port_count(input_channels: Option<u32>, sidechain_channels: Option<u32>) -> u32 {
    input_channels.is_some() as u32 + sidechain_channels.is_some() as u32
}

/// Host input-port index that carries the sidechain key, or `None` when the
/// plugin declares no sidechain port.
///
/// The main input (when present) always occupies index 0, so the sidechain
/// port follows it at index 1; an instrument with only a sidechain port puts
/// it at index 0.
#[inline]
pub fn sidechain_port_index(
    input_channels: Option<u32>,
    sidechain_channels: Option<u32>,
) -> Option<usize> {
    sidechain_channels.map(|_| usize::from(input_channels.is_some()))
}

// ---------------------------------------------------------------------------
// AudioPorts extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginAudioPortsImpl for ClapMainThread<'a, P> {
    fn count(&mut self, is_input: bool) -> u32 {
        if is_input {
            input_port_count(self.shared.input_channels, self.shared.sidechain_channels)
        } else {
            self.shared.output_ports.len() as u32
        }
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if is_input {
            // Main input occupies index 0 when present.
            if index == 0 {
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
                    return;
                }
            }

            // Optional secondary sidechain (key) input port. Non-main
            // (IS_MAIN cleared), distinct port id, never in-place routable.
            if Some(index as usize)
                == sidechain_port_index(self.shared.input_channels, self.shared.sidechain_channels)
            {
                if let Some(ch) = self.shared.sidechain_channels {
                    writer.set(&AudioPortInfo {
                        id: ClapId::new(SIDECHAIN_PORT_ID),
                        name: b"Sidechain",
                        channel_count: ch,
                        flags: AudioPortFlags::empty(),
                        port_type: Some(if ch == 1 {
                            AudioPortType::MONO
                        } else {
                            AudioPortType::STEREO
                        }),
                        in_place_pair: None,
                    });
                }
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
        writer.set(&AudioPortInfo {
            id: port_id,
            // `AudioPortInfo::name` takes an *unterminated* UTF-8 byte
            // slice: clack's `AudioPortInfoWriter::set` copies it via
            // `write_to_array_buf`, which truncates to CLAP_NAME_SIZE - 1
            // and appends the NUL terminator itself (same contract as the
            // `b"Input"` literal above).
            name: port.name.as_bytes(),
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
