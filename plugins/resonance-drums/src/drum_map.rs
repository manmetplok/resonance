/// General MIDI drum map and pad configuration.
///
/// The note numbers, pad order, and canonical names are the shared
/// contract in `resonance_common::drum_map`; this module layers the
/// DSP-side per-pad configuration (samples, choke groups, routing) on top.
use crate::kit::OutputGroup;
use resonance_common::drum_map::GM_PADS;

pub use resonance_common::drum_map::{
    pad_index_for_note, CHINA_BELL, CHINA_EDGE, CHINA_TIP, COUNT_STICK, CRASH_16_BELL,
    CRASH_16_EDGE, CRASH_16_TIP, CRASH_18_BELL, CRASH_18_EDGE, CRASH_18_TIP, HIHAT_CLOSED,
    HIHAT_HALF_OPEN, HIHAT_LOOSE, HIHAT_OPEN, HIHAT_PEDAL, HIHAT_PRESSED, HIHAT_TRASH_OPEN, KICK,
    NUM_PADS, RIDE_BELL, RIDE_EDGE, RIDE_TIP, RIMSHOT, SNARE, SNARE_FLAM, SNARE_HANDTUCH,
    SNARE_ROLL, SNARE_SIDESTICK, TOM_HIGH, TOM_LOW, TOM_MID,
};

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
        note: GM_PADS[0].note,
        name: GM_PADS[0].name,
        default_sample: include_bytes!("../samples/kick.wav"),
        choke_group: None,
        output_group: OutputGroup::Kick,
        close_mic_positions: &["KickIn", "KickOut"],
        has_articulation: true,
    },
    // 1: Snare
    PadMapping {
        note: GM_PADS[1].note,
        name: GM_PADS[1].name,
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: true,
    },
    // 2: Hi-Hat Closed
    PadMapping {
        note: GM_PADS[2].note,
        name: GM_PADS[2].name,
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 3: Hi-Hat Open
    PadMapping {
        note: GM_PADS[3].note,
        name: GM_PADS[3].name,
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 4: Hi-Hat Half Open
    PadMapping {
        note: GM_PADS[4].note,
        name: GM_PADS[4].name,
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 5: Hi-Hat Loose
    PadMapping {
        note: GM_PADS[5].note,
        name: GM_PADS[5].name,
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 6: Hi-Hat Pedal
    PadMapping {
        note: GM_PADS[6].note,
        name: GM_PADS[6].name,
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 7: Hi-Hat Pressed
    PadMapping {
        note: GM_PADS[7].note,
        name: GM_PADS[7].name,
        default_sample: include_bytes!("../samples/hihat_closed.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 8: Hi-Hat Trash Open
    PadMapping {
        note: GM_PADS[8].note,
        name: GM_PADS[8].name,
        default_sample: include_bytes!("../samples/hihat_open.wav"),
        choke_group: Some(CHOKE_HIHAT),
        output_group: OutputGroup::Hats,
        close_mic_positions: &["Hat"],
        has_articulation: false,
    },
    // 9: Tom High
    PadMapping {
        note: GM_PADS[9].note,
        name: GM_PADS[9].name,
        default_sample: include_bytes!("../samples/tom_high.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["Tom01"],
        has_articulation: true,
    },
    // 10: Tom Mid
    PadMapping {
        note: GM_PADS[10].note,
        name: GM_PADS[10].name,
        default_sample: include_bytes!("../samples/tom_mid.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["Tom02"],
        has_articulation: true,
    },
    // 11: Tom Low (Floor)
    PadMapping {
        note: GM_PADS[11].note,
        name: GM_PADS[11].name,
        default_sample: include_bytes!("../samples/tom_low.wav"),
        choke_group: None,
        output_group: OutputGroup::Toms,
        close_mic_positions: &["TomFloor"],
        has_articulation: true,
    },
    // 12: Crash 16 Edge
    PadMapping {
        note: GM_PADS[12].note,
        name: GM_PADS[12].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 13: Crash 16 Bell
    PadMapping {
        note: GM_PADS[13].note,
        name: GM_PADS[13].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 14: Crash 16 Tip
    PadMapping {
        note: GM_PADS[14].note,
        name: GM_PADS[14].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 15: Crash 18 Edge
    PadMapping {
        note: GM_PADS[15].note,
        name: GM_PADS[15].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 16: Crash 18 Bell
    PadMapping {
        note: GM_PADS[16].note,
        name: GM_PADS[16].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 17: Crash 18 Tip
    PadMapping {
        note: GM_PADS[17].note,
        name: GM_PADS[17].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 18: Ride Edge
    PadMapping {
        note: GM_PADS[18].note,
        name: GM_PADS[18].name,
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 19: Ride Bell
    PadMapping {
        note: GM_PADS[19].note,
        name: GM_PADS[19].name,
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 20: Ride Tip
    PadMapping {
        note: GM_PADS[20].note,
        name: GM_PADS[20].name,
        default_sample: include_bytes!("../samples/ride.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 21: China Edge
    PadMapping {
        note: GM_PADS[21].note,
        name: GM_PADS[21].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 22: China Bell
    PadMapping {
        note: GM_PADS[22].note,
        name: GM_PADS[22].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 23: China Tip
    PadMapping {
        note: GM_PADS[23].note,
        name: GM_PADS[23].name,
        default_sample: include_bytes!("../samples/crash.wav"),
        choke_group: None,
        output_group: OutputGroup::Cymbals,
        close_mic_positions: &[],
        has_articulation: false,
    },
    // 24: Sidestick
    PadMapping {
        note: GM_PADS[24].note,
        name: GM_PADS[24].name,
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 25: Rimshot
    PadMapping {
        note: GM_PADS[25].note,
        name: GM_PADS[25].name,
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 26: Snare Flam
    PadMapping {
        note: GM_PADS[26].note,
        name: GM_PADS[26].name,
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 27: Snare Roll
    PadMapping {
        note: GM_PADS[27].note,
        name: GM_PADS[27].name,
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 28: Snare Handtuch
    PadMapping {
        note: GM_PADS[28].note,
        name: GM_PADS[28].name,
        default_sample: include_bytes!("../samples/snare.wav"),
        choke_group: None,
        output_group: OutputGroup::Snare,
        close_mic_positions: &["SNTop", "SNBtm"],
        has_articulation: false,
    },
    // 29: Count Stick
    PadMapping {
        note: GM_PADS[29].note,
        name: GM_PADS[29].name,
        default_sample: include_bytes!("../samples/rimshot.wav"),
        choke_group: None,
        output_group: OutputGroup::Main,
        close_mic_positions: &[],
        has_articulation: false,
    },
];
