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
// AudioPorts extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginAudioPortsImpl for ClapMainThread<'a, P> {
    fn count(&mut self, is_input: bool) -> u32 {
        if is_input {
            if self.shared.input_channels.is_some() {
                1
            } else {
                0
            }
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
