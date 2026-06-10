//! Shared General MIDI drum-note contract.
//!
//! Single source of truth for the pad ↔ MIDI note ↔ canonical name
//! mapping used by both the `resonance-drums` plugin (DSP pad indices)
//! and the Compose drumroll view in `resonance-app` (UI pad rows).

/// Number of pads in the `resonance-drums` kit.
pub const NUM_PADS: usize = 30;

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
pub const COWBELL: u8 = 56; // GM: Cowbell — drumroll-only; not a resonance-drums pad

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

/// One pad of the shared contract: MIDI note plus canonical name.
pub struct GmPad {
    pub note: u8,
    pub name: &'static str,
}

/// Canonical pad list; the array index is the plugin pad index.
pub const GM_PADS: [GmPad; NUM_PADS] = [
    GmPad { note: KICK, name: "Kick" },
    GmPad { note: SNARE, name: "Snare" },
    GmPad { note: HIHAT_CLOSED, name: "Hi-Hat Closed" },
    GmPad { note: HIHAT_OPEN, name: "Hi-Hat Open" },
    GmPad { note: HIHAT_HALF_OPEN, name: "Hi-Hat Half Open" },
    GmPad { note: HIHAT_LOOSE, name: "Hi-Hat Loose" },
    GmPad { note: HIHAT_PEDAL, name: "Hi-Hat Pedal" },
    GmPad { note: HIHAT_PRESSED, name: "Hi-Hat Pressed" },
    GmPad { note: HIHAT_TRASH_OPEN, name: "Hi-Hat Trash Open" },
    GmPad { note: TOM_HIGH, name: "Tom High" },
    GmPad { note: TOM_MID, name: "Tom Mid" },
    GmPad { note: TOM_LOW, name: "Tom Low" },
    GmPad { note: CRASH_16_EDGE, name: "Crash 16 Edge" },
    GmPad { note: CRASH_16_BELL, name: "Crash 16 Bell" },
    GmPad { note: CRASH_16_TIP, name: "Crash 16 Tip" },
    GmPad { note: CRASH_18_EDGE, name: "Crash 18 Edge" },
    GmPad { note: CRASH_18_BELL, name: "Crash 18 Bell" },
    GmPad { note: CRASH_18_TIP, name: "Crash 18 Tip" },
    GmPad { note: RIDE_EDGE, name: "Ride Edge" },
    GmPad { note: RIDE_BELL, name: "Ride Bell" },
    GmPad { note: RIDE_TIP, name: "Ride Tip" },
    GmPad { note: CHINA_EDGE, name: "China Edge" },
    GmPad { note: CHINA_BELL, name: "China Bell" },
    GmPad { note: CHINA_TIP, name: "China Tip" },
    GmPad { note: SNARE_SIDESTICK, name: "Sidestick" },
    GmPad { note: RIMSHOT, name: "Rimshot" },
    GmPad { note: SNARE_FLAM, name: "Snare Flam" },
    GmPad { note: SNARE_ROLL, name: "Snare Roll" },
    GmPad { note: SNARE_HANDTUCH, name: "Snare Handtuch" },
    GmPad { note: COUNT_STICK, name: "Count Stick" },
];

/// Find the pad index for a given MIDI note, or None if unmapped.
pub fn pad_index_for_note(note: u8) -> Option<usize> {
    GM_PADS.iter().position(|p| p.note == note)
}
