/// General MIDI drum map constants and pad configuration.

use crate::kit::OutputGroup;

pub const NUM_PADS: usize = 12;

/// General MIDI drum note numbers (channel 10).
pub const KICK: u8 = 36;
pub const SNARE: u8 = 38;
pub const RIMSHOT: u8 = 37;
pub const CLAP: u8 = 39;
pub const HIHAT_CLOSED: u8 = 42;
pub const HIHAT_OPEN: u8 = 46;
pub const TOM_LOW: u8 = 45;
pub const TOM_MID: u8 = 47;
pub const TOM_HIGH: u8 = 50;
pub const CRASH: u8 = 49;
pub const RIDE: u8 = 51;
pub const COWBELL: u8 = 56;

/// Choke group IDs. Pads in the same choke group silence each other.
pub const CHOKE_HIHAT: u8 = 1;

pub struct PadMapping {
    pub note: u8,
    pub name: &'static str,
    pub default_sample: &'static [u8],
    pub choke_group: Option<u8>,
    /// Which plugin output port this pad's close signal routes to. The
    /// overhead contribution always goes to the dedicated Overhead port
    /// regardless of this field.
    pub output_group: OutputGroup,
    /// Mic positions the loader tries to load for this pad's close-mic
    /// bank. Order matters: the first position listed becomes bank 0
    /// (the "left" side of a kick/snare balance slider), the second
    /// becomes bank 1 (the "right" side).
    pub close_mic_positions: &'static [&'static str],
}

pub const PAD_MAPPINGS: [PadMapping; NUM_PADS] = [
    PadMapping {
        note: KICK,
        name: "Kick",
        default_sample: include_bytes!("../samples/kick.wav"),
        choke_group: None,
        output_group: OutputGroup::Kick,
        close_mic_positions: &["KickIn", "KickOut"],
    },
    PadMapping {
        note: SNARE,
        name: "Snare",
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
    },
    PadMapping {
        note: HIHAT_CLOSED,
        name: "Hi-Hat Closed",
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
    },
    PadMapping {
        note: HIHAT_OPEN,
        name: "Hi-Hat Open",
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
    },
    PadMapping {
        note: TOM_HIGH,
        name: "Tom High",
        default_sample: include_bytes!("../samples/tom_high.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["Tom01"],
    },
    PadMapping {
        note: TOM_MID,
        name: "Tom Mid",
        default_sample: include_bytes!("../samples/tom_mid.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["Tom02"],
    },
    PadMapping {
        note: TOM_LOW,
        name: "Tom Low",
        default_sample: include_bytes!("../samples/tom_low.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["TomFloor"],
    },
    PadMapping {
        note: CRASH,
        name: "Crash",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        // Drummica doesn't ship cymbal close mics — only overheads.
        close_mic_positions: &[],
    },
    PadMapping {
        note: RIDE,
        name: "Ride",
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
    },
    PadMapping {
        note: RIMSHOT,
        name: "Rimshot",
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
    },
    PadMapping {
        note: CLAP,
        name: "Clap",
        default_sample: include_bytes!("../samples/clap.wav"),
        choke_group: None,
        output_group: OutputGroup::Main,
        close_mic_positions: &[],
    },
    PadMapping {
        note: COWBELL,
        name: "Cowbell",
        default_sample: include_bytes!("../samples/cowbell.wav"),
        choke_group: None,
        output_group: OutputGroup::Main,
        close_mic_positions: &[],
    },
];

/// Find the pad index for a given MIDI note, or None if unmapped.
pub fn pad_index_for_note(note: u8) -> Option<usize> {
    PAD_MAPPINGS.iter().position(|p| p.note == note)
}
