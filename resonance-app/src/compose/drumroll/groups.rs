//! Drum groups — grouped polymetric drum patterns.
//!
//! A drum group is a named collection of articulations (pads from the kit)
//! that share a single rhythm. The group carries its own grid (subdivision)
//! and cycle length, so different groups can run different polymeter or
//! polyrhythm against the section's base meter. Pads inside a group hold a
//! per-step pattern and an articulation weight: when the same step fires,
//! the weights decide which pad's sample triggers.

use resonance_common::drum_map::{self as gm, GM_PADS};
use serde::{Deserialize, Serialize};

/// One pad inside a drum group. Each pad maps to a MIDI note from the kit
/// and carries an independent pattern (sized to `cycle`) plus an
/// articulation weight used by the generator to pick which pad fires when
/// the group's step is on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrumGroupPad {
    /// Display name (e.g. "Closed", "Half Open").
    pub name: String,
    /// MIDI note number played when this pad fires.
    pub note: u8,
    /// Articulation weight (0..=100). Relative — divided by group total.
    pub weight: u32,
    /// Step-by-step pattern, indexed 0..cycle. Non-zero = velocity-scaled
    /// hit. Stored as u8 (0..=100) so it serialises compactly.
    pub pattern: Vec<u8>,
}

impl DrumGroupPad {
    /// Resize the pattern to match a new cycle length, preserving any
    /// existing steps that still fit and zero-padding new tail steps.
    pub fn resize_pattern(&mut self, new_cycle: usize) {
        if new_cycle == self.pattern.len() {
            return;
        }
        let mut next = vec![0u8; new_cycle];
        let copy_len = new_cycle.min(self.pattern.len());
        next[..copy_len].copy_from_slice(&self.pattern[..copy_len]);
        self.pattern = next;
    }
}

/// One drum group. Grouping is project-scoped — a group is shared across
/// every section of the song so reorganising your kit doesn't fragment
/// per-section state. Per-section variations (cycle, pattern) live on the
/// section's lane generator config instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrumGroup {
    pub id: u64,
    /// Display name shown in the lane and the manager.
    pub name: String,
    /// Group color (RGB). Tints the group's row in the compose lane and
    /// the swatch in the manager modal.
    pub color: [u8; 3],
    /// Subdivision: steps per beat (2 = 8ths, 3 = triplet 8ths, 4 = 16ths,
    /// 5 = quintuplets, 6 = sextuplets, 7 = septuplets).
    pub grid: u8,
    /// Cycle length in steps before the pattern restarts. Drives polymeter
    /// — a 7-step cycle on a 4-step grid runs 7/16 against a 4/4 bar.
    pub cycle: u32,
    /// Step offset applied when rendering this group. Lets users rotate
    /// the pattern against the bar.
    pub phase: u32,
    /// Articulations and their per-step patterns.
    pub pads: Vec<DrumGroupPad>,
    /// Generator knobs — independent per group. Surfaced in the right rail.
    pub density: f32,
    pub swing: f32,
    pub accent: f32,
    pub humanize: f32,
    pub fills: f32,
    /// Optional style tag (e.g. "Tool · syncopated"). Free-form text shown
    /// as metadata on the lane; the generator may key off it for presets.
    pub style: String,
    /// Per-group RNG seed. Bumped by Generate / Regenerate so repeated
    /// presses yield fresh variations.
    pub seed: u64,
}

impl DrumGroup {
    /// Total cycle length in pattern steps. Convenience for callers.
    pub fn pattern_len(&self) -> usize {
        self.cycle as usize
    }

    /// Returns true when this group's combined cycle/grid does not match
    /// the section's base meter (so it should display the polymeter tag).
    pub fn is_off_grid(&self, base_grid: u8, base_cycle: u32) -> bool {
        self.grid != base_grid || self.cycle != base_cycle
    }

    /// Sum of every pad's articulation weight, clamped to at least 1 so
    /// downstream divisions never produce a zero.
    pub fn total_weight(&self) -> u32 {
        self.pads.iter().map(|p| p.weight).sum::<u32>().max(1)
    }

    /// Percentage share of one pad's weight (0..=100).
    pub fn weight_share(&self, pad_index: usize) -> u32 {
        let total = self.total_weight();
        let w = self.pads.get(pad_index).map(|p| p.weight).unwrap_or(0);
        (w * 100 + total / 2) / total
    }

    /// "Realigns every N bars" — number of bars before this group's cycle
    /// realigns with the section's base cycle on the base grid. `1` means
    /// every bar.
    pub fn realign_bars(&self, base_grid: u8, base_cycle: u32) -> u32 {
        if self.grid == 0 || base_grid == 0 || self.cycle == 0 || base_cycle == 0 {
            return 1;
        }
        // The group's bar length in "reference steps" is
        //     cycle * (base_grid / grid)
        // Use a common multiplier of 1000 to keep this in integer math
        // even when grid != base_grid.
        let group_ref_steps_x1000 = (self.cycle as u64) * (base_grid as u64) * 1000 / (self.grid as u64);
        let base_x1000 = (base_cycle as u64) * 1000;
        let lcm = lcm_u64(group_ref_steps_x1000, base_x1000);
        lcm.div_ceil(base_x1000) as u32
    }
}

/// Human-readable subdivision label.
pub fn grid_label(grid: u8) -> &'static str {
    match grid {
        2 => "8ths",
        3 => "triplets",
        4 => "16ths",
        5 => "quintuplets",
        6 => "sextuplets",
        7 => "septuplets",
        _ => "?",
    }
}

fn gcd_u64(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd_u64(b, a % b)
    }
}

pub fn lcm_u64(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd_u64(a, b) * b
    }
}

/// Default project drum groups. Mirrors the design's `DRUM_GROUPS` sample
/// so a fresh project starts with a recognisable kick/snare/hat/toms/perc
/// layout.
pub fn default_drum_groups(next_id: &mut u64) -> Vec<DrumGroup> {
    fn rgb(hex: u32) -> [u8; 3] {
        [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
    }

    let mut alloc = || {
        *next_id += 1;
        *next_id
    };

    vec![
        DrumGroup {
            id: alloc(),
            name: "Kick".to_string(),
            color: rgb(0xd0c4ff),
            grid: 4,
            cycle: 16,
            phase: 0,
            pads: vec![DrumGroupPad {
                name: "Kick".to_string(),
                note: gm::KICK,
                weight: 100,
                pattern: vec![
                    1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 1,
                ],
            }],
            density: 0.45,
            swing: 0.0,
            accent: 0.55,
            humanize: 0.20,
            fills: 0.15,
            style: "Tool \u{00b7} syncopated".to_string(),
            seed: 0xA50115C9A22B6F2F,
        },
        DrumGroup {
            id: alloc(),
            name: "Snare".to_string(),
            color: rgb(0xe8c47b),
            grid: 4,
            cycle: 16,
            phase: 0,
            pads: vec![
                DrumGroupPad {
                    name: "Snare".to_string(),
                    note: gm::SNARE,
                    weight: 70,
                    pattern: vec![
                        0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0,
                    ],
                },
                DrumGroupPad {
                    name: "Rim Click".to_string(),
                    note: gm::RIMSHOT,
                    weight: 15,
                    pattern: vec![
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0,
                    ],
                },
                DrumGroupPad {
                    // Note 39 is the plugin's Sidestick pad (repurposed
                    // from GM Clap); external GM synths play a clap here.
                    name: "Clap".to_string(),
                    note: gm::SNARE_SIDESTICK,
                    weight: 15,
                    pattern: vec![
                        0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
                    ],
                },
            ],
            density: 0.30,
            swing: 0.0,
            accent: 0.55,
            humanize: 0.20,
            fills: 0.10,
            style: "Backbeat \u{00b7} 2 & 4".to_string(),
            seed: 0xC1D5E2F300112233,
        },
        DrumGroup {
            id: alloc(),
            name: "Hi-Hat".to_string(),
            color: rgb(0xa892ff),
            grid: 4,
            cycle: 7,
            phase: 0,
            pads: vec![
                DrumGroupPad {
                    name: "Closed".to_string(),
                    note: gm::HIHAT_CLOSED,
                    weight: 60,
                    pattern: vec![1, 1, 0, 1, 1, 1, 0],
                },
                DrumGroupPad {
                    name: "Half Open".to_string(),
                    note: gm::HIHAT_HALF_OPEN,
                    weight: 10,
                    pattern: vec![0, 0, 0, 0, 0, 0, 1],
                },
                DrumGroupPad {
                    name: "Open".to_string(),
                    note: gm::HIHAT_OPEN,
                    weight: 25,
                    pattern: vec![0, 0, 1, 0, 0, 0, 0],
                },
                DrumGroupPad {
                    name: "Pedal".to_string(),
                    note: gm::HIHAT_PEDAL,
                    weight: 5,
                    pattern: vec![0, 0, 0, 0, 0, 0, 0],
                },
            ],
            density: 0.85,
            swing: 0.0,
            accent: 0.45,
            humanize: 0.30,
            fills: 0.0,
            style: "Polymeter \u{00b7} 7/16".to_string(),
            seed: 0x7F2C91D03E55A4B8,
        },
        DrumGroup {
            id: alloc(),
            name: "Toms".to_string(),
            color: rgb(0x7fb5e8),
            grid: 3,
            cycle: 12,
            phase: 0,
            pads: vec![
                DrumGroupPad {
                    name: "Tom Hi".to_string(),
                    note: gm::TOM_HIGH,
                    weight: 34,
                    pattern: vec![0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0],
                },
                DrumGroupPad {
                    name: "Tom Mid".to_string(),
                    note: gm::TOM_MID,
                    weight: 33,
                    pattern: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                },
                DrumGroupPad {
                    name: "Tom Lo".to_string(),
                    note: gm::TOM_LOW,
                    weight: 33,
                    pattern: vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
                },
            ],
            density: 0.20,
            swing: 0.0,
            accent: 0.40,
            humanize: 0.35,
            fills: 0.30,
            style: "Triplets \u{00b7} 3:4 polyrhythm".to_string(),
            seed: 0x1357BDF02468ACE0,
        },
        DrumGroup {
            id: alloc(),
            name: "Perc".to_string(),
            color: rgb(0x6dd6a3),
            grid: 4,
            cycle: 16,
            phase: 0,
            pads: vec![
                DrumGroupPad {
                    name: "Shaker".to_string(),
                    note: GM_SHAKER,
                    weight: 65,
                    pattern: vec![
                        0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 1,
                    ],
                },
                DrumGroupPad {
                    name: "Conga".to_string(),
                    note: GM_CONGA,
                    weight: 20,
                    pattern: vec![
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    ],
                },
                DrumGroupPad {
                    name: "Tamb".to_string(),
                    note: GM_TAMBOURINE,
                    weight: 15,
                    pattern: vec![
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    ],
                },
            ],
            density: 0.42,
            swing: 0.15,
            accent: 0.45,
            humanize: 0.25,
            fills: 0.0,
            style: "Shaker \u{00b7} 16ths".to_string(),
            seed: 0x95FC2D81B0A4E673,
        },
    ]
}

/// Curated palette used by the manager for new groups. Matches the design
/// palette so re-rolling colors keeps the look coherent.
pub const GROUP_PALETTE: &[[u8; 3]] = &[
    [0xa8, 0x92, 0xff],
    [0xe8, 0xc4, 0x7b],
    [0x7f, 0xb5, 0xe8],
    [0x6d, 0xd6, 0xa3],
    [0xd0, 0xc4, 0xff],
    [0xe8, 0x9a, 0xa3],
    [0x9a, 0xa0, 0xac],
    [0xf0, 0xa8, 0x72],
];

/// A pad available in the underlying kit. Used by the manager modal as
/// the picker source. The first time the project loads we materialise
/// this from the project's drumroll pad map so the user can move pads
/// between groups.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KitPadInfo {
    pub note: u8,
    pub name: String,
    /// Category — "Kick", "Snare", "Hi-Hat", "Toms", "Cymbals", "Perc".
    pub category: String,
}

/// GM percussion notes for instruments the `resonance-drums` plugin has
/// no pad for. Kept in the kit library so drum groups can still drive an
/// external General-MIDI synth; the plugin simply ignores these notes
/// (`pad_index_for_note` returns `None`).
pub const GM_SHAKER: u8 = 70; // GM: Maracas
pub const GM_CONGA: u8 = 64; // GM: Low Conga
pub const GM_TAMBOURINE: u8 = 54; // GM: Tambourine

/// The full drum kit pad library shown by the manager.
///
/// Contract: every plugin-backed pad is derived 1:1 from
/// [`resonance_common::drum_map::GM_PADS`] — the same note table the
/// `resonance-drums` sampler triggers from — so assigning a pad in the
/// manager always reaches the matching plugin pad. The remaining "Perc"
/// pads (cowbell, shaker, conga, tambourine) have no plugin pad and exist
/// for external GM synths; their notes are GM-standard and never collide
/// with `GM_PADS`.
pub fn default_kit_pads() -> Vec<KitPadInfo> {
    fn category(note: u8) -> &'static str {
        match note {
            gm::KICK => "Kick",
            gm::SNARE
            | gm::RIMSHOT
            | gm::SNARE_SIDESTICK
            | gm::SNARE_FLAM
            | gm::SNARE_ROLL
            | gm::SNARE_HANDTUCH => "Snare",
            gm::HIHAT_CLOSED
            | gm::HIHAT_OPEN
            | gm::HIHAT_HALF_OPEN
            | gm::HIHAT_LOOSE
            | gm::HIHAT_PEDAL
            | gm::HIHAT_PRESSED
            | gm::HIHAT_TRASH_OPEN => "Hi-Hat",
            gm::TOM_HIGH | gm::TOM_MID | gm::TOM_LOW => "Toms",
            gm::COUNT_STICK | gm::COWBELL | GM_SHAKER | GM_CONGA | GM_TAMBOURINE => "Perc",
            _ => "Cymbals",
        }
    }

    GM_PADS
        .iter()
        .map(|p| (p.name, p.note))
        .chain([
            ("Cowbell", gm::COWBELL),
            ("Shaker", GM_SHAKER),
            ("Conga", GM_CONGA),
            ("Tambourine", GM_TAMBOURINE),
        ])
        .map(|(name, note)| KitPadInfo {
            note,
            name: name.to_string(),
            category: category(note).to_string(),
        })
        .collect()
}
