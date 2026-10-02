//! Every plugin in the fleet renders on the one canonical palette (ba
//! todo #1338, audit doc #276's LOW palette finding).
//!
//! The fleet used to be split across two: seven effect editors on a blue
//! `classic` accent (`#5ac8fa`) and four instrument-ish editors on the
//! canonical lavender one, so a user with an amp and a wavetable open
//! side by side was looking at two products. The split was tracked in a
//! migration note in `wayland_plugin_gui::theme` and had been open long
//! enough to grow a third state — the shared range-mapped knob painted a
//! blue arc from a *private* constant, which meant even the editors that
//! had migrated still drew blue knobs.
//!
//! `classic` is deleted, so a `use ...theme::classic` cannot compile and
//! needs no test. What can still come back is the part no compiler sees:
//! a hex literal pasted into a plot, a meter or a trace. That is what
//! this file guards, together with the two remaining knob families not
//! being mixed inside one window.
//!
//! Sources are `include_str!`d where a fixed list will do, and walked
//! where the point is to catch a *new* file (a plot module added next
//! year is exactly where a stray colour appears). It is a source-text
//! check, which is a blunt instrument — but the alternative is a
//! dependency from this crate onto all eleven plugins that depend on it,
//! which is a cycle.
//!
//! **Adding a plugin crate means adding it to [`FLEET`].**

use std::path::{Path, PathBuf};

/// Which of the kit's two rotary-knob families an editor draws from.
///
/// They paint in the same palette now, but they are visibly different
/// controls: the range-mapped `knob` that
/// `editor_widgets::float_knob` wraps is a 64x76 cell with its caption
/// above the dial and a click-to-type readout; `knob_themed` is
/// unit-space, takes its geometry from a `KnobStyle`, and can draw
/// bipolar and over-unity arcs. One per window is the rule — the audit
/// called out an editor believed to carry both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Knobs {
    /// `editor_widgets::float_knob` / `widgets::knob`.
    RangeMapped,
    /// `knob_themed` and its `knob_unipolar` / `knob_bipolar` shorthands.
    Themed,
    /// Sliders and chips only — the EQ has no rotary control at all.
    None,
}

/// Every plugin crate, with the editor theme module that chooses its
/// palette and the knob family its editor draws.
const FLEET: &[(&str, &str, Knobs)] = &[
    (
        "resonance-amp",
        include_str!("../../plugins/resonance-amp/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-compressor",
        include_str!("../../plugins/resonance-compressor/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-delay",
        include_str!("../../plugins/resonance-delay/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-drums",
        include_str!("../../plugins/resonance-drums/src/editor/theme.rs"),
        Knobs::Themed,
    ),
    (
        "resonance-eq",
        include_str!("../../plugins/resonance-eq/src/editor/theme.rs"),
        Knobs::None,
    ),
    (
        "resonance-gate",
        include_str!("../../plugins/resonance-gate/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-granular-delay",
        include_str!("../../plugins/resonance-granular-delay/src/editor/theme.rs"),
        Knobs::Themed,
    ),
    (
        "resonance-ir",
        include_str!("../../plugins/resonance-ir/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-mastering",
        include_str!("../../plugins/resonance-mastering/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-reverb",
        include_str!("../../plugins/resonance-reverb/src/editor/theme.rs"),
        Knobs::RangeMapped,
    ),
    (
        "resonance-wavetable",
        include_str!("../../plugins/resonance-wavetable/src/editor/theme.rs"),
        Knobs::Themed,
    ),
];

/// The colours of the retired palette, and the tints the editors mixed
/// from its accent. Written without spaces because [`code_of`] strips
/// them, so `0x5a, 0xc8, 0xfa` and `0x5a,0xc8,0xfa` both match.
///
/// Only the ones that carried the *identity* of the old theme: its
/// accent, everything derived from that accent, and its surfaces. A
/// plugin-specific green or a window-chrome dot is not a palette split.
const RETIRED: &[(&str, &str)] = &[
    ("0x5a,0xc8,0xfa", "the classic accent"),
    ("0x4a,0x9e,0xcf", "the range-mapped knob's private blue arc"),
    ("0xa8,0xe1,0xff", "the bright accent used for traces and curves"),
    ("0x50,0xb4,0xff", "the delay's blue left-channel echo"),
    ("0x16,0x37,0x45", "the accent-tinted fill under curves and waveforms"),
    ("0x0f,0x22,0x30", "the reverb tail's dim blue fill"),
    ("0xff,0xb6,0x4a", "the classic warn amber"),
    ("0x14,0x14,0x18", "the classic window background"),
    ("0x1b,0x1b,0x22", "the classic panel surface"),
    ("0x25,0x25,0x2e", "the classic raised surface"),
    ("0x33,0x33,0x3e", "the classic border"),
    ("0x80,0x80,0x88", "the classic dim text"),
];

/// The audit counted eleven plugins. If that number moves, the list above
/// has to move with it — otherwise this file silently stops covering the
/// newcomer.
#[test]
fn the_fleet_is_the_size_the_audit_counted() {
    assert_eq!(
        FLEET.len(),
        11,
        "the palette finding is about 11 plugins; add the new crate to FLEET"
    );
}

/// Every editor's theme module re-exports the one shared palette, rather
/// than defining a palette of its own or reaching for a second one.
#[test]
fn every_editor_theme_re_exports_the_canonical_palette() {
    for (crate_name, theme_rs, _) in FLEET {
        assert!(
            theme_rs.contains("pub use plugin_gui_core::theme::lavender::*;"),
            "{crate_name}: its editor theme does not re-export the canonical \
             palette (via the platform-neutral `plugin_gui_core`), so this \
             editor is on a palette of its own"
        );
        assert!(
            !code_of(theme_rs).contains("theme::classic"),
            "{crate_name}: the classic palette is retired; there is one palette"
        );
    }
}

/// No editor source anywhere — including files that did not exist when
/// this was written — carries a colour from the retired palette.
///
/// This is the assertion that makes the migration stay done. Repointing
/// the theme modules was the easy half; the blue that actually made the
/// fleet look like two products was spread across scope traces, meters,
/// spectrum fills and one knob widget's private constants.
#[test]
fn no_editor_source_carries_a_retired_palette_colour() {
    let mut checked = 0usize;
    for file in editor_sources() {
        let src = std::fs::read_to_string(&file).expect("editor source is readable");
        let code = code_of(&src);
        for (hex, what) in RETIRED {
            assert!(
                !code.contains(hex),
                "{}: `{hex}` is {what}, from the palette ba todo #1338 \
                 retired. Shape the colour from a token in \
                 `plugin_gui_core::theme::lavender` instead",
                file.display()
            );
        }
        checked += 1;
    }
    // A walk that silently found nothing would pass every assertion
    // above it. The fleet has well over a hundred editor sources.
    assert!(
        checked > 100,
        "only {checked} editor sources were walked — the tree layout moved \
         and this guard is no longer looking at the editors"
    );
}

/// Each editor draws one knob family, and it is the one [`FLEET`] says.
///
/// Stated as an expected value rather than a "not both", so it fails
/// both ways: an editor that starts mixing families, and one whose
/// call sites all move without anyone noticing. The audit believed the
/// wavetable carried 25 range-mapped call sites beside 12 themed ones;
/// they are all themed, through a pair of same-named local wrappers, and
/// nothing but a check like this one tells the two apart.
#[test]
fn each_editor_draws_exactly_the_knob_family_it_should() {
    for (crate_name, _, expected) in FLEET {
        let dir = plugins_dir().join(crate_name).join("src/editor");
        let (mut range_mapped, mut themed) = (None, None);
        for file in sources_under(&dir) {
            let src = std::fs::read_to_string(&file).expect("editor source is readable");
            let code = code_of(&src);
            // The range-mapped family is only reachable through the
            // param-bound wrapper or the raw widget. A local wrapper of
            // the same name is not it — the wavetable has one.
            if (code.contains("editor_widgets") && code.contains("float_knob"))
                || code.contains("widgets::knob(")
            {
                range_mapped.get_or_insert_with(|| file.clone());
            }
            if ["knob_themed(", "knob_themed_edit(", "knob_unipolar(", "knob_bipolar("]
                .iter()
                .any(|call| code.contains(call))
            {
                themed.get_or_insert_with(|| file.clone());
            }
        }
        let found = match (&range_mapped, &themed) {
            (Some(a), Some(b)) => panic!(
                "{crate_name} draws both knob families in one window: the \
                 range-mapped one in {} and the themed one in {}. Convert \
                 one to the other",
                a.display(),
                b.display()
            ),
            (Some(_), None) => Knobs::RangeMapped,
            (None, Some(_)) => Knobs::Themed,
            (None, None) => Knobs::None,
        };
        assert_eq!(
            found, *expected,
            "{crate_name} draws {found:?} knobs, FLEET says {expected:?}"
        );
    }
}

/// Source with its comments removed.
///
/// The prose in these files is *about* the retired palette — it names the
/// hexes it replaced and the family it did not choose, which is worth
/// keeping and is not a colour anyone renders. Whitespace goes too, so a
/// literal matches however it is formatted.
fn code_of(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| line.chars())
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn plugins_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate has a workspace root above it")
        .join("plugins")
}

/// Every editor source in the fleet, plus the shared widget kit they all
/// draw from — a colour hardcoded there reaches all eleven windows at
/// once, which is how the blue knob arc survived two migrations.
fn editor_sources() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for (crate_name, _, _) in FLEET {
        files.extend(sources_under(&plugins_dir().join(crate_name).join("src")));
    }
    files.extend(sources_under(
        &plugins_dir()
            .parent()
            .expect("the workspace root is above `plugins`")
            .join("wayland-plugin-gui/src"),
    ));
    files
}

fn sources_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(sources_under(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}
