/// General MIDI drum map constants and pad configuration.
use crate::kit::OutputGroup;

pub const NUM_PADS: usize = 30;

// ---------------------------------------------------------------------------
// MIDI note assignments.
//
// General MIDI standard (channel 10) where applicable; unused GM notes or
// notes below/above the standard range for the rest.
// ---------------------------------------------------------------------------

// --- GM standard notes ---
pub const KICK: u8 = 36;
pub const SNARE: u8 = 38;
pub const RIMSHOT: u8 = 37; // GM: Side Stick / Rimshot
pub const HIHAT_CLOSED: u8 = 42;
pub const HIHAT_OPEN: u8 = 46;
pub const HIHAT_PEDAL: u8 = 44; // GM: Pedal Hi-Hat
pub const TOM_LOW: u8 = 45; // GM: Low Tom
pub const TOM_MID: u8 = 47; // GM: Low-Mid Tom
pub const TOM_HIGH: u8 = 50; // GM: High Tom
pub const CRASH_16_EDGE: u8 = 49; // GM: Crash Cymbal 1
pub const CRASH_18_EDGE: u8 = 57; // GM: Crash Cymbal 2
pub const RIDE_EDGE: u8 = 51; // GM: Ride Cymbal 1
pub const RIDE_BELL: u8 = 53; // GM: Ride Bell
pub const CHINA_EDGE: u8 = 52; // GM: Chinese Cymbal

// --- Extended / non-GM notes (using free slots below 35 and above 81) ---
pub const SNARE_SIDESTICK: u8 = 39; // repurposed from GM Clap
pub const SNARE_FLAM: u8 = 21;
pub const SNARE_ROLL: u8 = 22;
pub const SNARE_HANDTUCH: u8 = 23;
pub const HIHAT_HALF_OPEN: u8 = 24;
pub const HIHAT_LOOSE: u8 = 25;
pub const HIHAT_PRESSED: u8 = 26;
pub const HIHAT_TRASH_OPEN: u8 = 27;
pub const CRASH_16_BELL: u8 = 28;
pub const CRASH_16_TIP: u8 = 29;
pub const CRASH_18_BELL: u8 = 55;
pub const CRASH_18_TIP: u8 = 58;
pub const RIDE_TIP: u8 = 59;
pub const CHINA_BELL: u8 = 60;
pub const CHINA_TIP: u8 = 61;
pub const COUNT_STICK: u8 = 31; // GM: Sticks (31)

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
    /// Whether this pad supports an articulation toggle (e.g. mit/ohne
    /// Teppich). When true, the params system exposes a toggle and the
    /// kit loader consults the articulation flag to pick the alt piece.
    pub has_articulation: bool,
}

pub const PAD_MAPPINGS: [PadMapping; NUM_PADS] = [
    // 0: Kick
    PadMapping {
        note: KICK,
        name: "Kick",
        default_sample: include_bytes!("../samples/kick.wav"),
        choke_group: None,
        output_group: OutputGroup::Kick,
        close_mic_positions: &["KickIn", "KickOut"],
        has_articulation: true,
    },
    // 1: Snare
    PadMapping {
        note: SNARE,
        name: "Snare",
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: true,
    },
    // 2: Hi-Hat Closed
    PadMapping {
        note: HIHAT_CLOSED,
        name: "Hi-Hat Closed",
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 3: Hi-Hat Open
    PadMapping {
        note: HIHAT_OPEN,
        name: "Hi-Hat Open",
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 4: Hi-Hat Half Open
    PadMapping {
        note: HIHAT_HALF_OPEN,
        name: "Hi-Hat Half Open",
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 5: Hi-Hat Loose
    PadMapping {
        note: HIHAT_LOOSE,
        name: "Hi-Hat Loose",
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 6: Hi-Hat Pedal
    PadMapping {
        note: HIHAT_PEDAL,
        name: "Hi-Hat Pedal",
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 7: Hi-Hat Pressed
    PadMapping {
        note: HIHAT_PRESSED,
        name: "Hi-Hat Pressed",
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 8: Hi-Hat Trash Open
    PadMapping {
        note: HIHAT_TRASH_OPEN,
        name: "Hi-Hat Trash Open",
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 9: Tom High
    PadMapping {
        note: TOM_HIGH,
        name: "Tom High",
        default_sample: include_bytes!("../samples/tom_high.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["Tom01"],
        has_articulation: true,
    },
    // 10: Tom Mid
    PadMapping {
        note: TOM_MID,
        name: "Tom Mid",
        default_sample: include_bytes!("../samples/tom_mid.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["Tom02"],
        has_articulation: true,
    },
    // 11: Tom Low (Floor)
    PadMapping {
        note: TOM_LOW,
        name: "Tom Low",
        default_sample: include_bytes!("../samples/tom_low.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["TomFloor"],
        has_articulation: true,
    },
    // 12: Crash 16 Edge
    PadMapping {
        note: CRASH_16_EDGE,
        name: "Crash 16 Edge",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 13: Crash 16 Bell
    PadMapping {
        note: CRASH_16_BELL,
        name: "Crash 16 Bell",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 14: Crash 16 Tip
    PadMapping {
        note: CRASH_16_TIP,
        name: "Crash 16 Tip",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 15: Crash 18 Edge
    PadMapping {
        note: CRASH_18_EDGE,
        name: "Crash 18 Edge",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 16: Crash 18 Bell
    PadMapping {
        note: CRASH_18_BELL,
        name: "Crash 18 Bell",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 17: Crash 18 Tip
    PadMapping {
        note: CRASH_18_TIP,
        name: "Crash 18 Tip",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 18: Ride Edge
    PadMapping {
        note: RIDE_EDGE,
        name: "Ride Edge",
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 19: Ride Bell
    PadMapping {
        note: RIDE_BELL,
        name: "Ride Bell",
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 20: Ride Tip
    PadMapping {
        note: RIDE_TIP,
        name: "Ride Tip",
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 21: China Edge
    PadMapping {
        note: CHINA_EDGE,
        name: "China Edge",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 22: China Bell
    PadMapping {
        note: CHINA_BELL,
        name: "China Bell",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 23: China Tip
    PadMapping {
        note: CHINA_TIP,
        name: "China Tip",
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 24: Sidestick
    PadMapping {
        note: SNARE_SIDESTICK,
        name: "Sidestick",
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 25: Rimshot
    PadMapping {
        note: RIMSHOT,
        name: "Rimshot",
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 26: Snare Flam
    PadMapping {
        note: SNARE_FLAM,
        name: "Snare Flam",
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 27: Snare Roll
    PadMapping {
        note: SNARE_ROLL,
        name: "Snare Roll",
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 28: Snare Handtuch
    PadMapping {
        note: SNARE_HANDTUCH,
        name: "Snare Handtuch",
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 29: Count Stick
    PadMapping {
        note: COUNT_STICK,
        name: "Count Stick",
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Main,
        close_mic_positions: &[],
        has_articulation: false,
    },
];

/// Find the pad index for a given MIDI note, or None if unmapped.
pub fn pad_index_for_note(note: u8) -> Option<usize> {
    PAD_MAPPINGS.iter().position(|p| p.note == note)
}
