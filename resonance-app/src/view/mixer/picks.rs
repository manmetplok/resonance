//! Pick-list wrapper types used across the mixer strips. Each wrapper
//! exists so iced's `pick_list` has a `Display + Clone + PartialEq`
//! shape to render without leaking the underlying enum representation
//! into the UI.

use resonance_audio::MidiDeviceInfo;
use resonance_audio::types::*;

/// Which container a plugin slot belongs to. Used so `view_plugin_slot_row`
/// can emit the right remove message regardless of whether it's rendering
/// a track's plugin or a bus's plugin.
#[derive(Debug, Clone, Copy)]
pub(super) enum PluginOwner {
    Track(TrackId),
    Bus(BusId),
    Master,
}

/// Wrapper type for the output-destination pick_list so iced can render
/// it via `Display` and `PartialEq`. Variants correspond 1:1 with
/// `TrackOutput` but carry a display name for the chosen bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputChoice {
    pub label: String,
    pub output: TrackOutput,
}

impl std::fmt::Display for OutputChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Wrapper so the input-port pick_list can render 1-based channel
/// numbers and stereo pair labels without reaching into track state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortChoice {
    /// 0-indexed channel number on the device.
    pub index: u16,
    /// True if the track is mono — the label shows "In N"; false shows
    /// "In N/N+1" so the user sees which pair the stereo track reads.
    pub mono: bool,
}

impl std::fmt::Display for PortChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let one_based = self.index + 1;
        if self.mono {
            write!(f, "In {}", one_based)
        } else {
            write!(f, "In {}/{}", one_based, one_based + 1)
        }
    }
}

/// Wrapper around `Option<MidiDeviceInfo>` so the MIDI pickers can
/// include a "(None)" entry that clears the assignment. iced's
/// `pick_list` requires `Display + Clone + PartialEq` on its option
/// type and a value-typed match against the current selection, so we
/// store the device name and either render it verbatim or render
/// "(None)" for the unset variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MidiPickerChoice(pub Option<String>);

impl std::fmt::Display for MidiPickerChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => f.write_str("(None)"),
            Some(name) => f.write_str(name),
        }
    }
}

/// Build the base MIDI picker option list (None + every device), without
/// the track-specific "configured-but-unplugged" fallback. Used to
/// populate cached choices on `Resonance` — the per-track override is
/// added on demand at the call site, when needed.
pub(crate) fn midi_choices_base(available: &[MidiDeviceInfo]) -> Vec<MidiPickerChoice> {
    let mut choices: Vec<MidiPickerChoice> = Vec::with_capacity(available.len() + 1);
    choices.push(MidiPickerChoice(None));
    for d in available {
        choices.push(MidiPickerChoice(Some(d.name.clone())));
    }
    choices
}

/// MIDI channel picker entry. `None` represents "Omni" for inputs
/// (accept any channel) and "default channel 1" for outputs. The inner
/// value is the 0-indexed channel (0..=15) so it matches the raw MIDI
/// status nibble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MidiChannelChoice(pub Option<u8>);

impl std::fmt::Display for MidiChannelChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("Omni"),
            Some(ch) => write!(f, "Ch {}", ch + 1),
        }
    }
}

/// Channel picker options for an input filter: "Omni" plus 1..=16.
pub(crate) fn input_channel_choices() -> Vec<MidiChannelChoice> {
    let mut v = Vec::with_capacity(17);
    v.push(MidiChannelChoice(None));
    for ch in 0u8..16 {
        v.push(MidiChannelChoice(Some(ch)));
    }
    v
}

/// Channel picker options for an output: 1..=16. Outputs always emit
/// on a specific channel, so there's no "Omni" entry — the `None`
/// selection is rendered as channel 1 for compatibility but not
/// offered as a separate pick.
pub(crate) fn output_channel_choices() -> Vec<MidiChannelChoice> {
    (0u8..16).map(|ch| MidiChannelChoice(Some(ch))).collect()
}

/// Pick-list entry for an external-instrument MIDI **bank**. `None`
/// clears the Bank Select (the synth stays on its current bank); `Some`
/// carries the combined 14-bit value sent as CC0 (MSB) + CC32 (LSB).
/// Epic #39 ships numeric banks only — named banks arrive with the
/// device-preset epic (#40), at which point the label can resolve a
/// patch name without changing this shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BankChoice(pub Option<u16>);

impl std::fmt::Display for BankChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("(no bank)"),
            Some(bank) => write!(f, "Bank {}", bank),
        }
    }
}

/// Pick-list entry for an external-instrument MIDI **program**. `None`
/// sends no Program Change; `Some` carries the `0..=127` program number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProgramChoice(pub Option<u8>);

impl std::fmt::Display for ProgramChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            None => f.write_str("(no program)"),
            Some(program) => write!(f, "Program {}", program),
        }
    }
}

/// Bank picker options: "(no bank)" plus banks `0..=127`. The combined
/// 14-bit bank is addressed through its low 7 bits (CC32/LSB) with the
/// MSB left at 0, covering the common single-byte bank range without a
/// 16k-entry list. Built once at startup; never invalidates.
pub(crate) fn bank_choices() -> Vec<BankChoice> {
    let mut v = Vec::with_capacity(129);
    v.push(BankChoice(None));
    for bank in 0u16..=127 {
        v.push(BankChoice(Some(bank)));
    }
    v
}

/// Program picker options: "(no program)" plus programs `0..=127`.
/// Built once at startup; never invalidates.
pub(crate) fn program_choices() -> Vec<ProgramChoice> {
    let mut v = Vec::with_capacity(129);
    v.push(ProgramChoice(None));
    for program in 0u8..=127 {
        v.push(ProgramChoice(Some(program)));
    }
    v
}

/// Build the output-destination picker options: Master plus every bus.
/// Cached on `Resonance` and rebuilt only when the bus list changes,
/// so the inspector clones a refcounted slice instead of allocating
/// labels and a Vec every frame.
pub(crate) fn output_choices_for(
    busses: &[crate::state::BusState],
) -> Vec<OutputChoice> {
    use crate::theme::fa;
    let mut choices: Vec<OutputChoice> = Vec::with_capacity(1 + busses.len());
    choices.push(OutputChoice {
        label: format!("{} Master", fa::ARROW_RIGHT),
        output: TrackOutput::Master,
    });
    for bus in busses {
        choices.push(OutputChoice {
            label: format!("{} {}", fa::ARROW_RIGHT, bus.name),
            output: TrackOutput::Bus(bus.id),
        });
    }
    choices
}
