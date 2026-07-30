//! Built-in named drum patterns — a groove vocabulary available in every
//! project, not just freshly-templated ones.
//!
//! A project's pattern *bank* holds whatever the user authored; it starts
//! life with "Main" and an empty "B section" and nothing else. That left
//! `generate.drums` with a two-word vocabulary, both of them busy
//! straight-4/4 16th grooves, so building a song with any sectional
//! contrast meant hand-authoring drums note by note (ba doc #270 §5).
//!
//! The patterns here are **templates**, not bank entries: they are
//! materialised into a project's bank on demand by
//! [`instantiate`](super::super::super::update::compose::drum_groups), so
//! an already-saved project gains the whole vocabulary without a
//! migration and the GUI picker stays as short as the user made it.
//!
//! # Why these are authored, not generated
//!
//! `generate_group_pattern` fills a group by Euclidean placement from its
//! `density` + `seed`, overwriting every step. That is a fine way to roll
//! a *variation*, but it cannot express a groove: "four on the floor" is
//! a kick on each beat, "halftime" is a backbeat on 3, and a Euclidean
//! scatter at some density is neither. So each pattern below carries
//! explicit steps, and the control endpoint installs them without
//! re-rolling. Scaling is done by [`apply_density`], which thins the
//! authored steps by metrical weight rather than replacing them.

use resonance_common::drum_map as gm;

use super::groups::{DrumGroup, DrumGroupPad};
use super::pattern::DrumPattern;

/// A step grid of four steps per beat — 16ths — which every built-in
/// pattern uses. `cycle` is then `16` for one 4/4 bar.
const GRID_16: u8 = 4;
/// Steps in one 4/4 bar at [`GRID_16`].
const BAR: u32 = 16;

/// One built-in groove: a name, a one-line description for the tool
/// surface, and the groups it installs.
struct Template {
    name: &'static str,
    description: &'static str,
    length_bars: u32,
    groups: &'static [GroupSpec],
}

struct GroupSpec {
    name: &'static str,
    color: u32,
    grid: u8,
    cycle: u32,
    pads: &'static [PadSpec],
}

struct PadSpec {
    name: &'static str,
    note: u8,
    weight: u32,
    /// One entry per step; non-zero is a hit whose value is a velocity
    /// scale (the same 0..=100 convention `DrumGroupPad::pattern` uses).
    steps: &'static [u8],
}

const K: u8 = 100; // a full-velocity hit
const A: u8 = 80; // accent-adjacent / secondary
const G: u8 = 45; // ghost note

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

const TEMPLATES: &[Template] = &[
    Template {
        name: "halftime",
        description: "Backbeat on 3 — the snare halves the perceived tempo.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    //   1 e & a 2 e & a 3 e & a 4 e & a
                    steps: &[K, 0, 0, 0, 0, 0, A, 0, 0, 0, 0, 0, 0, 0, A, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Snare",
                    note: gm::SNARE,
                    weight: 100,
                    steps: &[0, 0, 0, 0, 0, 0, 0, 0, K, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Hats",
                color: 0x9fd8c8,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Closed",
                    note: gm::HIHAT_CLOSED,
                    weight: 100,
                    steps: &[A, 0, G, 0, A, 0, G, 0, A, 0, G, 0, A, 0, G, 0],
                }],
            },
        ],
    },
    Template {
        name: "four-on-floor",
        description: "Kick on every beat, backbeat snare, straight 8th hats.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, K, 0, 0, 0, K, 0, 0, 0, K, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Snare",
                    note: gm::SNARE,
                    weight: 100,
                    steps: &[0, 0, 0, 0, K, 0, 0, 0, 0, 0, 0, 0, K, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Hats",
                color: 0x9fd8c8,
                grid: GRID_16,
                cycle: BAR,
                pads: &[
                    PadSpec {
                        name: "Closed",
                        note: gm::HIHAT_CLOSED,
                        weight: 80,
                        steps: &[0, 0, A, 0, 0, 0, A, 0, 0, 0, A, 0, 0, 0, A, 0],
                    },
                    PadSpec {
                        name: "Open",
                        note: gm::HIHAT_OPEN,
                        weight: 20,
                        steps: &[0; 16],
                    },
                ],
            },
        ],
    },
    Template {
        name: "industrial",
        description: "Mechanical gated pulse: rigid 16th hats, hard backbeat.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, 0, 0, A, 0, 0, 0, K, 0, 0, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[
                    PadSpec {
                        name: "Snare",
                        note: gm::SNARE,
                        weight: 85,
                        steps: &[0, 0, 0, 0, K, 0, 0, 0, 0, 0, 0, 0, K, 0, 0, 0],
                    },
                    PadSpec {
                        name: "Sidestick",
                        note: gm::SNARE_SIDESTICK,
                        weight: 15,
                        steps: &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, G, 0],
                    },
                ],
            },
            GroupSpec {
                name: "Hats",
                color: 0x9fd8c8,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Pressed",
                    note: gm::HIHAT_PRESSED,
                    weight: 100,
                    steps: &[A, G, G, G, A, G, G, G, A, G, G, G, A, G, G, G],
                }],
            },
        ],
    },
    Template {
        name: "breakbeat",
        description: "Syncopated kick, ghosted snare, broken 8th hats.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, 0, 0, A, 0, 0, 0, K, 0, 0, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Snare",
                    note: gm::SNARE,
                    weight: 100,
                    steps: &[0, 0, 0, 0, K, 0, 0, G, 0, 0, 0, 0, K, 0, G, 0],
                }],
            },
            GroupSpec {
                name: "Hats",
                color: 0x9fd8c8,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Closed",
                    note: gm::HIHAT_CLOSED,
                    weight: 100,
                    steps: &[A, 0, G, 0, A, 0, 0, 0, A, 0, G, 0, A, 0, 0, 0],
                }],
            },
        ],
    },
    Template {
        name: "blast",
        description: "Blast beat: kick and snare alternating 16ths under a ride.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, K, 0, K, 0, K, 0, K, 0, K, 0, K, 0, K, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Snare",
                    note: gm::SNARE,
                    weight: 100,
                    steps: &[0, K, 0, K, 0, K, 0, K, 0, K, 0, K, 0, K, 0, K],
                }],
            },
            GroupSpec {
                name: "Cymbals",
                color: 0xc8b4e8,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Ride",
                    note: gm::RIDE_EDGE,
                    weight: 100,
                    steps: &[A, 0, 0, 0, A, 0, 0, 0, A, 0, 0, 0, A, 0, 0, 0],
                }],
            },
        ],
    },
    Template {
        name: "sparse",
        description: "Almost nothing: kick on 1, snare on 3, no hats.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Snare",
                    note: gm::SNARE,
                    weight: 100,
                    steps: &[0, 0, 0, 0, 0, 0, 0, 0, A, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
        ],
    },
    Template {
        name: "toms",
        description: "Tom-led tribal groove, no hats, snare out of the way.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, 0, 0, 0, 0, K, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Toms",
                color: 0xe0a68a,
                grid: GRID_16,
                cycle: BAR,
                pads: &[
                    PadSpec {
                        name: "Low",
                        note: gm::TOM_LOW,
                        weight: 40,
                        steps: &[0, 0, 0, 0, A, 0, 0, 0, 0, 0, 0, 0, A, 0, 0, 0],
                    },
                    PadSpec {
                        name: "Mid",
                        note: gm::TOM_MID,
                        weight: 35,
                        steps: &[0, 0, A, 0, 0, 0, G, 0, 0, 0, A, 0, 0, 0, G, 0],
                    },
                    PadSpec {
                        name: "High",
                        note: gm::TOM_HIGH,
                        weight: 25,
                        steps: &[0, G, 0, 0, 0, G, 0, 0, 0, G, 0, 0, 0, G, 0, 0],
                    },
                ],
            },
        ],
    },
    Template {
        name: "build",
        description: "Rising intensity across the bar: snare accelerates into 16ths.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, K, 0, 0, 0, K, 0, 0, 0, K, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Snare",
                color: 0xe8c47b,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Snare",
                    note: gm::SNARE,
                    weight: 100,
                    // Quarters, then 8ths, then 16ths — a one-bar ramp.
                    steps: &[G, 0, 0, 0, A, 0, 0, 0, A, 0, A, 0, K, A, K, K],
                }],
            },
        ],
    },
    Template {
        name: "fill",
        description: "One-bar tom fill landing on a crash.",
        length_bars: 1,
        groups: &[
            GroupSpec {
                name: "Kick",
                color: 0xd0c4ff,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Kick",
                    note: gm::KICK,
                    weight: 100,
                    steps: &[K, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
            GroupSpec {
                name: "Toms",
                color: 0xe0a68a,
                grid: GRID_16,
                cycle: BAR,
                pads: &[
                    PadSpec {
                        name: "High",
                        note: gm::TOM_HIGH,
                        weight: 34,
                        steps: &[0, 0, 0, 0, K, A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    },
                    PadSpec {
                        name: "Mid",
                        note: gm::TOM_MID,
                        weight: 33,
                        steps: &[0, 0, 0, 0, 0, 0, K, A, 0, 0, 0, 0, 0, 0, 0, 0],
                    },
                    PadSpec {
                        name: "Low",
                        note: gm::TOM_LOW,
                        weight: 33,
                        steps: &[0, 0, 0, 0, 0, 0, 0, 0, K, A, K, A, K, A, K, A],
                    },
                ],
            },
            GroupSpec {
                name: "Cymbals",
                color: 0xc8b4e8,
                grid: GRID_16,
                cycle: BAR,
                pads: &[PadSpec {
                    name: "Crash",
                    note: gm::CRASH_16_EDGE,
                    weight: 100,
                    steps: &[K, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
        ],
    },
];

// ---------------------------------------------------------------------------
// Lookup + instantiation
// ---------------------------------------------------------------------------

/// Every built-in groove name, in vocabulary order. Surfaced in the
/// `generate_drums` tool description and in its not-found error so the
/// vocabulary is discoverable rather than guessable.
pub fn builtin_pattern_names() -> Vec<&'static str> {
    TEMPLATES.iter().map(|t| t.name).collect()
}

/// `(name, description)` for every built-in, for help text.
pub fn builtin_pattern_catalog() -> Vec<(&'static str, &'static str)> {
    TEMPLATES.iter().map(|t| (t.name, t.description)).collect()
}

/// Is `name` a built-in groove? Matched trimmed and case-insensitively,
/// the same way the project bank matches its own names.
pub fn is_builtin_pattern(name: &str) -> bool {
    find(name).is_some()
}

fn find(name: &str) -> Option<&'static Template> {
    let wanted = name.trim().to_ascii_lowercase();
    TEMPLATES.iter().find(|t| t.name == wanted)
}

/// Materialise a built-in groove as a bank-ready [`DrumPattern`],
/// allocating ids from `next_id`. Returns `None` for an unknown name.
///
/// `display_name` is what the pattern is called in the bank — callers
/// pass the section's name so a project ends up with "Chorus drums"
/// rather than three patterns all called "halftime".
pub fn instantiate_builtin(
    name: &str,
    display_name: &str,
    color: [u8; 3],
    next_id: &mut u64,
) -> Option<DrumPattern> {
    let template = find(name)?;
    let mut alloc = || {
        *next_id += 1;
        *next_id
    };
    let pattern_id = alloc();
    let groups = template
        .groups
        .iter()
        .map(|g| DrumGroup {
            id: alloc(),
            name: g.name.to_string(),
            color: rgb(g.color),
            grid: g.grid,
            cycle: g.cycle,
            phase: 0,
            pads: g
                .pads
                .iter()
                .map(|p| DrumGroupPad {
                    name: p.name.to_string(),
                    note: p.note,
                    weight: p.weight,
                    pattern: p.steps.to_vec(),
                })
                .collect(),
            // The authored steps are the groove; these knobs only matter
            // if the user later hits Generate on the group in the GUI,
            // at which point a Euclidean re-roll is what they asked for.
            density: 0.5,
            swing: 0.0,
            accent: 0.5,
            humanize: 0.15,
            fills: 0.0,
            style: format!("built-in \u{00b7} {}", template.name),
            seed: 0,
        })
        .collect();
    Some(DrumPattern {
        id: pattern_id,
        name: display_name.to_string(),
        color,
        groups,
        length_bars: template.length_bars,
    })
}

fn rgb(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

// ---------------------------------------------------------------------------
// Density
// ---------------------------------------------------------------------------

/// Metrical importance of a step, following the usual hierarchy: bar
/// downbeat 5, half-bar 4, other beats 3, off-8ths 2, anything finer 1.
/// `grid` is steps per beat.
///
/// The half-bar level matters more than it looks: without it beat 2 and
/// beat 3 tie, and a thinned four-on-the-floor keeps `1 +  2` instead of
/// the `1 + 3` that actually reads as a half-time pulse.
fn metric_weight(step: usize, grid: u8) -> u8 {
    let g = (grid.max(1)) as usize;
    if step % (g * 4) == 0 {
        5
    } else if step % (g * 2) == 0 {
        4
    } else if step % g == 0 {
        3
    } else if g % 2 == 0 && step % (g / 2) == 0 {
        2
    } else {
        1
    }
}

/// Thin a pattern's authored steps toward the strong beats.
///
/// `density` is `0.0..=1.0`. Each voice keeps the `density` fraction of
/// its own hits, chosen strongest-beat-first: the bar downbeat outranks
/// the other beats, which outrank off-8ths, which outrank 16ths, and
/// equal ranks resolve earliest-first. `1.0` is the groove exactly as
/// authored.
///
/// Keeping a *proportion* rather than applying a metrical cutoff is what
/// makes this usable as a build: four sections at 0.25 / 0.5 / 0.75 /
/// 1.0 take a four-on-the-floor kick from one hit to two to three to
/// four, instead of jumping between a couple of coarse tiers. It is also
/// monotonic — lowering density never adds a hit — which a fresh
/// Euclidean roll per section cannot promise.
///
/// A pad that had any hit keeps at least one, so thinning never silences
/// a voice outright and a kick stays a kick at any density.
pub fn apply_density(pattern: &mut DrumPattern, density: f32) {
    let d = if density.is_finite() {
        density.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if d >= 1.0 {
        return;
    }
    for group in &mut pattern.groups {
        let grid = group.grid;
        for pad in &mut group.pads {
            let hits: Vec<usize> = pad
                .pattern
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0)
                .map(|(i, _)| i)
                .collect();
            if hits.is_empty() {
                continue;
            }
            let keep_n = ((hits.len() as f32) * d).ceil().max(1.0) as usize;
            if keep_n >= hits.len() {
                continue;
            }
            // Strongest beat first; ties earliest-first so the surviving
            // hits stay front-loaded and the groove keeps its shape.
            let mut ranked = hits.clone();
            ranked.sort_by_key(|&step| (std::cmp::Reverse(metric_weight(step, grid)), step));
            for &step in &ranked[keep_n..] {
                pad.pattern[step] = 0;
            }
        }
    }
}
