//! The K5 editor (drums-plugin-rework.md §6): every tab fits its window,
//! every control is real, and every edit is one undoable host edit.
//!
//! Headless egui frames through `TestEditor`, read back as probed rects
//! with the clip each was laid out under (see `editor_layout.rs` for why
//! "drawn" is not "visible"). A control below the fold must be reachable:
//! the test scrolls its region with the mouse wheel until it is.
#![cfg(feature = "editor")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use plugin_gui_core::egui;
use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::kit::NUM_OUTPUT_PORTS;
use resonance_drums::kit_loader::{spawn_loader, KitStatus, PadMicChoices, DEFAULT_OVERHEAD_SETUP};
use resonance_drums::params::OUTPUT_MODE_MULTI;
use resonance_drums::{EditorFrameProbe, ProbedRect, ResonanceDrums, TestEditor};
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

/// §6.1: 960×640 default, 780×520 minimum; 1571×856 is what this
/// machine's tiling compositor maps every plugin editor at (CLAUDE.md).
const SIZES: [(f32, f32); 3] = [(960.0, 640.0), (780.0, 520.0), (1571.0, 856.0)];
const TABS: [&str; 3] = ["Pads", "Mix", "Setup"];
const TOLERANCE: f32 = 1.0;
const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
const KICK: usize = 0;
const SNARE: usize = 1;
const TOM: usize = 9;

// ---------------------------------------------------------------------------
// A fixture kit: two kick-in mics to pick between, a snare with top and
// bottom, a tom with the snare's bottom as bleed, two overhead setups and
// a room — the catalog the Setup tab and the inspector's MICS read.
// ---------------------------------------------------------------------------

type Setup = (&'static str, &'static str, &'static str, u16);

const PIECES: [(&str, &[Setup]); 3] = [
    (
        "SD Kick mit Teppich",
        &[
            ("01_KickIn_e901", "KickIn", "k_in", 1),
            ("02_KickIn_B91", "KickIn", "k_in2", 1),
            ("04_KickOut_TLM170", "KickOut", "k_out", 1),
            ("23_OHsAB_e914", "OHsAB", "k_ohab", 2),
            ("25_OHsXY_USM69i", "OHsXY", "k_ohxy", 2),
            ("30_Room", "Room", "k_room", 2),
        ],
    ),
    (
        "SD Snare Normal",
        &[
            ("06_SNTop_MD441", "SNTop", "s_top", 1),
            ("09_SNBtm_e906", "SNBtm", "s_btm", 1),
            ("23_OHsAB_e914", "OHsAB", "s_ohab", 2),
            ("30_Room", "Room", "s_room", 2),
        ],
    ),
    (
        "SD Tom01 mit Teppich",
        &[
            ("11_Tom01_e904", "Tom01", "t_close", 1),
            ("09_SNBtm_e906", "SNBtm", "t_snbtm", 1),
            ("23_OHsAB_e914", "OHsAB", "t_ohab", 2),
        ],
    ),
];

/// Which brand/mic each setup's manifest entry names, so the friendly
/// labels can be checked.
fn mic_of(file: &str) -> (&'static str, String) {
    ("Sennheiser", format!("M-{file}"))
}

fn write_wav(path: &Path, channels: u16, frames: usize) {
    let block_align = channels * 2;
    let data_len = frames * block_align as usize;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        // A decaying burst, so the waveform has a shape.
        let v = ((1.0 - i as f32 / frames as f32) * 0.5 * i16::MAX as f32) as i16;
        for _ in 0..channels {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(path, out).expect("write fixture wav");
}

struct Kit {
    dir: PathBuf,
    manifest: PathBuf,
}

impl Drop for Kit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture_kit() -> Kit {
    fixture_kit_of(&PIECES)
}

/// The fixture kit plus the kick's "ohne Teppich" alternate, so the kick
/// pad offers articulation chips.
fn fixture_kit_with_articulation() -> Kit {
    let mut pieces = PIECES.to_vec();
    pieces.push((
        "SD Kick ohne Teppich",
        &[("01_KickIn_e901", "KickIn", "ko_in", 1), ("23_OHsAB_e914", "OHsAB", "ko_ohab", 2)],
    ));
    fixture_kit_of(&pieces)
}

fn fixture_kit_of(pieces_spec: &[(&str, &[Setup])]) -> Kit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-editor-tabs-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut pieces = serde_json::Map::new();
    for &(piece, setups) in pieces_spec {
        let mut map = serde_json::Map::new();
        for &(key, position, file, channels) in setups {
            // Three velocity layers, two takes each.
            let mut rounds = serde_json::Map::new();
            for rr in 1..=2 {
                let mut vels = serde_json::Map::new();
                for vel in 1..=3 {
                    let name = format!("{file}_rr{rr}_v{vel}.wav");
                    write_wav(&dir.join(&name), channels, 1_200 * vel);
                    vels.insert(format!("Vel0{vel}"), serde_json::json!(name));
                }
                rounds.insert(format!("RR{rr}"), serde_json::Value::Object(vels));
            }
            let (brand, mic) = mic_of(file);
            map.insert(
                key.to_string(),
                serde_json::json!({
                    "brand": brand,
                    "channel": "1",
                    "mic": mic,
                    "position": position,
                    "rounds": rounds,
                }),
            );
        }
        pieces.insert(piece.to_string(), serde_json::Value::Object(map));
    }
    let manifest = dir.join("drum_samples.json");
    std::fs::write(&manifest, serde_json::to_vec(&pieces).unwrap()).unwrap();
    Kit { dir, manifest }
}

fn booted() -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    assert!(plugin.initialize(RATE, BLOCK as u32));
    plugin
}

/// A plugin playing the fixture kit.
fn with_kit(kit: &Kit) -> ResonanceDrums {
    let mut plugin = booted();
    let choices: [PadMicChoices; NUM_PADS] = std::array::from_fn(|_| PadMicChoices::default());
    spawn_loader(
        kit.manifest.clone(),
        RATE,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        choices,
        [false; NUM_PADS],
    );
    settle(&mut plugin);
    plugin
}

fn settle(plugin: &mut ResonanceDrums) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let pending = plugin.bridge.pending_kit.lock().is_some();
        match plugin.bridge.kit_status.lock().clone() {
            KitStatus::Loaded { .. } if !pending => break,
            KitStatus::Error { message } if !pending => panic!("kit failed to load: {message}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "load never settled");
        std::thread::sleep(Duration::from_millis(2));
    }
    for _ in 0..4 {
        render(plugin, &[]);
    }
}

fn render(plugin: &mut ResonanceDrums, events: &[NoteEvent]) {
    let mut buffers: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    let mut ports: Vec<OutputBuffer<'_>> = buffers
        .iter_mut()
        .map(|(l, r)| OutputBuffer {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        })
        .collect();
    let mut iter = EventIterator::new(events);
    plugin.process(&mut ports, BLOCK, &mut iter, None);
}

fn hit(plugin: &mut ResonanceDrums, pad: usize, velocity: f32) {
    render(
        plugin,
        &[NoteEvent::NoteOn {
            note: drum_map::PAD_MAPPINGS[pad].note,
            velocity,
            timing: 0,
        }],
    );
}

// ---------------------------------------------------------------------------
// Frame helpers
// ---------------------------------------------------------------------------

fn editor(plugin: &ResonanceDrums, size: (f32, f32), tab: &str) -> TestEditor {
    let mut e = TestEditor::new(plugin, resonance_drums::library::shared(), size);
    e.show_view(tab);
    e.frame(Vec::new());
    e
}

fn settled(e: &mut TestEditor) -> EditorFrameProbe {
    e.frame(Vec::new());
    e.frame(Vec::new())
}

fn fully_visible(w: &ProbedRect, screen: egui::Rect) -> bool {
    let r = w.rect;
    r.width() > 0.0
        && r.height() > 0.0
        && w.clip.expand(TOLERANCE).contains_rect(r)
        && screen.expand(TOLERANCE).contains_rect(r)
}

/// Scroll `name`'s region until it is fully visible (at most 40 wheel
/// turns). Returns the frame it is visible in, or why not.
fn reveal(e: &mut TestEditor, mut frame: EditorFrameProbe, name: &str) -> Result<EditorFrameProbe, String> {
    for _ in 0..40 {
        let Some(w) = frame.widget(name).cloned() else {
            return Err(format!("{name} is not laid out"));
        };
        if fully_visible(&w, frame.screen) {
            return Ok(frame);
        }
        // Horizontal overflow is a layout bug, not something to scroll to.
        if w.rect.right() > frame.screen.right() + TOLERANCE || w.rect.left() < -TOLERANCE {
            return Err(format!("{name} runs off the window sideways: {:?}", w.rect));
        }
        let viewport = w.clip.intersect(frame.screen);
        if !(viewport.width() > 0.0 && viewport.height() > 0.0) {
            return Err(format!("{name} has no visible viewport: clip {:?}", w.clip));
        }
        let dy = if w.rect.bottom() > viewport.bottom() { -60.0 } else { 60.0 };
        // Over the widget's own column: a vertical scroll area's clip runs
        // the full width of its parent, so the clip's centre can be over
        // a neighbouring column.
        let at = egui::pos2(
            w.rect.center().x.clamp(viewport.left() + 1.0, viewport.right() - 1.0),
            viewport.center().y,
        );
        frame = e.wheel(at, egui::vec2(0.0, dy));
    }
    let w = frame.widget(name).cloned();
    Err(format!("{name} never scrolled into view: {:?}", w.map(|w| (w.rect, w.clip))))
}

/// The cards controls sit in: a card may be taller than its scrolling
/// region (that is what the scrolling is for), so only what is in it has
/// to come into view.
const CARDS: [&str; 10] = [
    "pad_grid",
    "inspector",
    "mix.outputs",
    "mix.table",
    "mix.global",
    "setup.banks",
    "setup.streaming",
    "setup.kit",
    "setup.table",
    "missing.banner",
];

/// Every probed control of `tab` at `size` is reachable: fully visible,
/// or scrolled into view. Table rows are virtual (only the rows in view
/// exist), so they are checked separately.
fn assert_tab_reachable(plugin: &ResonanceDrums, size: (f32, f32), tab: &str, pad: usize) {
    let mut e = editor(plugin, size, tab);
    e.select_pad(pad);
    let mut frame = settled(&mut e);
    let mut widgets: Vec<(String, f32)> = frame
        .widgets
        .iter()
        .map(|w| (w.name.clone(), w.rect.top()))
        .filter(|(n, _)| !n.starts_with("mix.row.") && !n.starts_with("setup.row."))
        .filter(|(n, _)| !CARDS.contains(&n.as_str()))
        .collect();
    assert!(widgets.len() > 10, "{tab} at {size:?} laid out almost nothing: {widgets:?}");
    // Top to bottom, so each region only ever scrolls down: a control
    // checked once is never scrolled back over by a later one.
    widgets.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (name, _) in widgets {
        frame = match reveal(&mut e, frame, &name) {
            Ok(f) => f,
            Err(why) => panic!("{tab} at {size:?}: {why}"),
        };
    }
}

// ---------------------------------------------------------------------------
// Fit
// ---------------------------------------------------------------------------

#[test]
fn every_tab_is_reachable_at_both_sizes_with_the_built_in_kit() {
    let plugin = booted();
    for size in SIZES {
        for tab in TABS {
            assert_tab_reachable(&plugin, size, tab, 0);
        }
    }
}

#[test]
fn every_tab_is_reachable_at_both_sizes_with_a_kit() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    plugin.bridge.params.output_mode.set_value(OUTPUT_MODE_MULTI);
    for pad in [KICK, SNARE, TOM] {
        for size in SIZES {
            for tab in TABS {
                assert_tab_reachable(&plugin, size, tab, pad);
            }
        }
    }
}

/// Every fader's readout sits right of its slider; no two header, tab
/// bar or status bar items overlap; no two visible texts overlap.
#[test]
fn nothing_overlaps_on_any_tab() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    for size in SIZES {
        for tab in TABS {
            let mut e = editor(&plugin, size, tab);
            let frame = settled(&mut e);
            for w in &frame.widgets {
                if let Some(value) = frame.widget(&format!("{}.value", w.name)) {
                    assert!(
                        w.rect.right() <= value.rect.left() + TOLERANCE,
                        "{tab} {size:?}: {}'s value overlaps its slider: {:?} / {:?}",
                        w.name,
                        w.rect,
                        value.rect
                    );
                }
            }
            let chrome = [
                "header.brand",
                "kit.prev",
                "kit.combo",
                "kit.next",
                "header.library",
                "tabs",
                "header.preset_label",
                "header.preset",
                "status.rate",
                "status.pads",
                "status.memory",
                "status.last_hit",
            ];
            let rects: Vec<(&str, egui::Rect)> = chrome
                .iter()
                .filter_map(|n| frame.widget(n).map(|w| (*n, w.rect)))
                .collect();
            assert_eq!(rects.len(), chrome.len(), "{tab} {size:?}: chrome not all laid out");
            for (i, (a, ra)) in rects.iter().enumerate() {
                for (b, rb) in &rects[i + 1..] {
                    let o = ra.intersect(*rb);
                    assert!(
                        !(o.width() > TOLERANCE && o.height() > TOLERANCE),
                        "{tab} {size:?}: {a} {ra:?} overlaps {b} {rb:?}"
                    );
                }
            }
            let visible: Vec<_> = frame
                .texts
                .iter()
                .filter(|t| !t.text.trim().is_empty())
                .map(|t| (t, t.rect.intersect(t.clip).intersect(frame.screen)))
                .filter(|(_, v)| v.width() > 0.0 && v.height() > 0.0)
                .collect();
            for (i, (a, va)) in visible.iter().enumerate() {
                for (b, vb) in &visible[i + 1..] {
                    let o = va.intersect(*vb);
                    assert!(
                        !(o.width() > 1.5 && o.height() > 1.5),
                        "{tab} {size:?}: {:?} {va:?} overlaps {:?} {vb:?}",
                        a.text,
                        b.text
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Header / tabs / status
// ---------------------------------------------------------------------------

/// The tab bar switches the body: each tab shows its own sections.
#[test]
fn the_tab_bar_switches_the_view() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    assert!(frame.widget("pad_grid").is_some() && frame.widget("inspector").is_some());
    for (tab, section) in [("Mix", "mix.outputs"), ("Setup", "setup.banks"), ("Pads", "pad_grid")] {
        let center = frame.text_center(tab).expect("tab label drawn");
        let frame = e.click(center);
        let frame2 = settled(&mut e);
        assert_eq!(e.view(), tab, "clicking {tab} did not switch to it");
        assert!(
            frame.widget(section).is_some() || frame2.widget(section).is_some(),
            "{tab} does not show {section}"
        );
    }
}

/// The kit is named once, in the header — not again in a pad-list card,
/// a KIT card (§1.3) or the Setup tab's kit facts (whose location shows
/// the folder the kit is in, the kit's own folder name on hover). The old
/// KIT/GLOBAL bottom cards are gone.
#[test]
fn the_kit_is_named_once_and_the_old_cards_are_gone() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    for tab in TABS {
        let mut e = editor(&plugin, (960.0, 640.0), tab);
        let frame = settled(&mut e);
        let name = match plugin.bridge.kit_status.lock().clone() {
            KitStatus::Loaded { name, .. } => name,
            other => panic!("{other:?}"),
        };
        let shown = frame.texts.iter().filter(|t| t.text.contains(&name)).count();
        assert_eq!(shown, 1, "{tab}: the kit name {name:?} is drawn {shown} times");
        for gone in ["card.kit", "card.global", "kit.master"] {
            assert!(frame.widget(gone).is_none(), "{gone} is back");
        }
    }
}

/// The status bar reads the last hit as the sampler played it.
#[test]
fn the_status_bar_shows_the_last_hit() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    assert!(settled(&mut e).shows("no hit yet"));
    hit(&mut plugin, SNARE, 1.0);
    let frame = settled(&mut e);
    let text = frame
        .strings()
        .into_iter()
        .find(|s| s.contains(" v127 → layer "))
        .unwrap_or_else(|| panic!("no last hit in {:?}", frame.strings()));
    assert!(text.starts_with("Snare v127 → layer 3/3 · take "), "{text}");
    assert!(frame.shows("3 of 30 pads"), "{:?}", frame.strings());
}

// ---------------------------------------------------------------------------
// Pads tab
// ---------------------------------------------------------------------------

/// A click on a cell selects the pad and plays it; how high in the cell
/// is how hard.
#[test]
fn a_cell_click_selects_and_plays_at_the_clicked_height() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    let cell = frame.widget(&format!("pad_cell.{SNARE}")).unwrap().rect;

    e.click(egui::pos2(cell.center().x, cell.top() + 2.0));
    assert_eq!(e.selected_pad(), SNARE);
    render(&mut plugin, &[]);
    let high = plugin.bridge.last_hits.pad(SNARE).expect("the click played the pad");
    assert!(high.velocity >= 120, "a click at the top played v{}", high.velocity);

    e.select_pad(KICK);
    e.click(egui::pos2(cell.center().x, cell.bottom() - 2.0));
    assert_eq!(e.selected_pad(), SNARE);
    render(&mut plugin, &[]);
    let low = plugin.bridge.last_hits.pad(SNARE).unwrap();
    assert_ne!(low.seq, high.seq, "the second click did not play");
    assert!(low.velocity <= 15, "a click at the bottom played v{}", low.velocity);
}

/// A pad the kit lacks is dimmed, selects, and does not play.
#[test]
fn an_absent_pad_is_dimmed_and_silent() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    let absent: Vec<usize> = (0..NUM_PADS)
        .filter(|p| frame.widget(&format!("pad_cell.{p}.absent")).is_some())
        .collect();
    assert_eq!(absent.len(), NUM_PADS - 3, "{absent:?}");
    for p in [KICK, SNARE, TOM] {
        assert!(!absent.contains(&p), "pad {p} is dimmed");
    }
    let crash = 12;
    let cell = frame.widget(&format!("pad_cell.{crash}")).unwrap().rect;
    e.click(cell.center());
    assert_eq!(e.selected_pad(), crash);
    render(&mut plugin, &[]);
    assert!(plugin.bridge.last_hits.pad(crash).is_none(), "an absent pad played");
    let frame = settled(&mut e);
    assert!(frame.widget("inspector.not_in_kit").is_some());
    assert!(frame.widget("trim.mic1").is_none() && frame.widget("trim.oh").is_none());
}

/// A cell lights on a hit and goes dark again.
#[test]
fn a_cell_lights_on_a_hit() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let fill_at = |frame: &EditorFrameProbe, rect: egui::Rect| {
        frame.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
            _ => None,
        })
    };
    let frame = settled(&mut e);
    let cell = frame.widget(&format!("pad_cell.{TOM}")).unwrap().rect;
    let dark = fill_at(&frame, cell).expect("the cell's fill");
    hit(&mut plugin, TOM, 1.0);
    let lit = fill_at(&e.frame(Vec::new()), cell).unwrap();
    assert_ne!(lit, dark, "the cell did not light");
    for _ in 0..40 {
        e.frame(Vec::new());
    }
    assert_eq!(fill_at(&e.frame(Vec::new()), cell).unwrap(), dark, "the cell stayed lit");
}

/// The inspector offers exactly the controls the pad has: one knob per
/// playing param, a picker + trim per close mic it holds (a picker only
/// with a choice to make), the overhead trim — and nothing for mics it
/// lacks, no Solo.
#[test]
fn the_inspector_shows_exactly_the_controls_the_pad_has() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");

    let controls = |e: &mut TestEditor, pad: usize| -> Vec<String> {
        e.select_pad(pad);
        let frame = settled(e);
        let mut names: Vec<String> = frame
            .widgets
            .iter()
            .map(|w| w.name.clone())
            .filter(|n| {
                n.starts_with("knob.")
                    || n.starts_with("mic.")
                    || (n.starts_with("trim.") && !n.ends_with(".value"))
                    || n.starts_with("routing.")
                    || n.starts_with("articulation.")
                    || n == "inspector.mute"
            })
            .collect();
        names.sort();
        assert!(!frame.strings().iter().any(|s| s == "Solo"), "a Solo control is drawn");
        names
    };

    let knobs = ["knob.decay", "knob.hold", "knob.level", "knob.pan", "knob.start", "knob.tune"];
    let with = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = knobs.iter().chain(extra).map(|s| s.to_string()).collect();
        v.push("inspector.mute".into());
        v.push("routing.choke".into());
        v.push("routing.output".into());
        // Stereo (the default): the port picker is dimmed, and says why.
        v.push("routing.output.dimmed".into());
        v.sort();
        v
    };
    // Kick: two KickIn setups to pick from (a combo), one KickOut (a
    // label), both trims, the overheads' trim.
    assert_eq!(
        controls(&mut e, KICK),
        with(&["mic.0", "mic.1", "trim.mic1", "trim.mic2", "trim.oh"])
    );
    // The tom: one close mic, no second trim; the snare's bottom is bleed,
    // which is off.
    assert_eq!(controls(&mut e, TOM), with(&["mic.0", "trim.mic1", "trim.oh"]));

    // The kick's picker reads `position · brand mic`, not a raw key.
    e.select_pad(KICK);
    let frame = settled(&mut e);
    assert!(frame.shows("KickIn · Sennheiser M-k_in"), "{:?}", frame.strings());
    assert!(!frame.strings().iter().any(|s| s.contains("01_KickIn_e901")), "a raw key is drawn");
}

/// Bleed on: the tom (which has the snare's bottom as bleed) gains its
/// bleed trim; nothing else does.
#[test]
fn a_bleed_trim_appears_only_with_bleed_on() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    plugin.bridge.params.bleed_on.set_value(resonance_drums::params::BANK_ON);
    resonance_drums::selection::watch(&plugin.bridge);
    settle(&mut plugin);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    e.select_pad(TOM);
    assert!(settled(&mut e).widget("trim.bleed").is_some());
    e.select_pad(SNARE);
    assert!(settled(&mut e).widget("trim.bleed").is_none());
}

/// The waveform shows the take the pad last played, and says which.
#[test]
fn the_waveform_follows_the_last_played_take() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    e.select_pad(SNARE);
    let frame = settled(&mut e);
    assert!(
        frame.strings().iter().any(|s| s.starts_with("not played yet — showing the loudest")),
        "{:?}",
        frame.strings()
    );
    hit(&mut plugin, SNARE, 0.05);
    let frame = settled(&mut e);
    let hit = plugin.bridge.last_hits.pad(SNARE).unwrap();
    assert_eq!(hit.layer, 0, "a soft hit plays the soft layer");
    let want = format!("last hit v{} · layer 1/3 · take {}/2", hit.velocity, hit.take + 1);
    assert!(frame.shows(&want), "{want:?} not in {:?}", frame.strings());
}

// ---------------------------------------------------------------------------
// Mix tab
// ---------------------------------------------------------------------------

/// Seven port strips, each with a meter that reads its port.
#[test]
fn the_mix_tab_has_a_metered_strip_per_port() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    plugin.bridge.params.output_mode.set_value(OUTPUT_MODE_MULTI);
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    let strips = (0..NUM_OUTPUT_PORTS)
        .filter(|p| frame.widget(&format!("mix.strip.{p}")).is_some())
        .count();
    assert_eq!(strips, NUM_OUTPUT_PORTS);
    assert_eq!(frame.strings().iter().filter(|s| *s == "−∞").count(), NUM_OUTPUT_PORTS);

    // A kick in Multi: the Kick port (1) meters, and the Overhead (6).
    render(
        &mut plugin,
        &[NoteEvent::NoteOn {
            note: drum_map::PAD_MAPPINGS[KICK].note,
            velocity: 1.0,
            timing: 0,
        }],
    );
    for port in [1, 6] {
        let peak = f32::from_bits(plugin.bridge.port_peak[port].load(Ordering::Relaxed));
        assert!(peak > 0.01, "port {port} peak {peak}");
    }
    let frame = e.frame(Vec::new());
    assert!(
        frame.strings().iter().filter(|s| *s == "−∞").count() < NUM_OUTPUT_PORTS,
        "no strip moved"
    );
}

/// Every pad has a row in the table — the last one too, once scrolled to.
#[test]
fn the_mix_table_reaches_every_pad() {
    let plugin = booted();
    for size in SIZES {
        let mut e = editor(&plugin, size, "Mix");
        let frame = settled(&mut e);
        assert!(frame.widget("mix.row.0").is_some());
        let table = frame.widget("mix.table").unwrap().rect;
        let mut frame = frame;
        for _ in 0..20 {
            if frame.widget("mix.row.29").is_some_and(|w| {
                fully_visible(w, frame.screen)
            }) {
                break;
            }
            frame = e.wheel(table.center(), egui::vec2(0.0, -120.0));
        }
        let last = frame.widget("mix.row.29.mute").expect("the last row's mute");
        assert!(fully_visible(last, frame.screen), "{size:?}: {:?} in {:?}", last.rect, last.clip);
    }
}

// ---------------------------------------------------------------------------
// Setup tab
// ---------------------------------------------------------------------------

/// The Setup tab lists the kit's overhead, room and bleed mics by their
/// friendly names.
#[test]
fn the_setup_tab_lists_the_kits_banks() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Setup");
    let frame = settled(&mut e);
    for name in ["setup.oh.0", "setup.oh.1", "setup.oh.2", "setup.oh.0.level", "setup.room.on", "setup.room.setup", "setup.bleed.on", "setup.preload"] {
        assert!(frame.widget(name).is_some(), "{name} is missing");
    }
    // Slot 1 plays OHsAB (the default); 2 and 3 are off.
    assert!(frame.shows("OHsAB · Sennheiser M-k_ohab"), "{:?}", frame.strings());
    assert!(frame.widget("setup.oh.1.level").is_none(), "an empty slot has a level");
    assert!(frame.strings().iter().any(|s| s.starts_with("Room · Sennheiser M-k_room")));
    assert!(frame.widget("setup.bleed.source.SNBtm").is_some());
    let bleed = frame
        .strings()
        .into_iter()
        .find(|s| s.contains("→ heard on"))
        .unwrap();
    assert!(bleed.contains("Tom"), "{bleed}");
}

/// Picking a setup for overhead slot 2 puts it in the slot (and loads it).
#[test]
fn an_overhead_slot_pick_reaches_the_bridge() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Setup");
    let frame = settled(&mut e);
    let combo = frame.widget("setup.oh.1").unwrap().rect;
    e.click(combo.center());
    let frame = settled(&mut e);
    let item = frame.text_center("OHsXY · Sennheiser M-k_ohxy").expect("the popup lists OHsXY");
    e.click(item);
    assert_eq!(plugin.bridge.overhead_slots()[1], "25_OHsXY_USM69i");
    // The pick is state, not a param: it is announced through
    // `mic_setup_rev`, one undoable edit.
    assert_eq!(edits(&plugin), ["mic_setup_rev"]);
}

/// The mic-setup handle is what a host needs to undo a pick through the
/// state blob: state-excluded (so the host refreshes its cached blob on
/// the edit), never automated, and visible to the host — a hidden param
/// is not exposed by the CLAP bridge, so its announce would reach nothing.
#[test]
fn the_mic_setup_handle_is_a_state_excluded_host_param() {
    let plugin = booted();
    let handle = (0..plugin.param_count())
        .map(|i| plugin.param(i))
        .find(|p| p.id() == "mic_setup_rev")
        .expect("mic_setup_rev is a param");
    assert!(handle.state_excluded() && handle.preset_excluded());
    assert!(!handle.is_automatable() && !handle.is_hidden() && !handle.is_read_only());
}

/// A pad's close-mic pick and the room setup pick are one announced edit
/// each, and each moves the handle.
#[test]
fn every_mic_setup_pick_is_one_announced_edit() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    e.select_pad(KICK);
    let frame = settled(&mut e);
    let frame = reveal(&mut e, frame, "mic.0").unwrap();
    e.click(frame.widget("mic.0").unwrap().rect.center());
    let frame = settled(&mut e);
    let item = frame.text_center("KickIn · Sennheiser M-k_in2").expect("the popup lists the B91");
    e.click(item);
    assert_eq!(
        plugin.bridge.pad_choices.lock()[KICK].close_setups.get("KickIn").map(String::as_str),
        Some("02_KickIn_B91")
    );
    assert_eq!(edits(&plugin), ["mic_setup_rev"]);
    assert_eq!(plugin.bridge.params.mic_setup_rev.value(), 1);

    e.show_view("Setup");
    let frame = settled(&mut e);
    e.click(frame.widget("setup.room.setup").unwrap().rect.center());
    let frame = settled(&mut e);
    let item = frame.text_center("Room · Sennheiser M-k_room").expect("the popup lists the room");
    e.click(item);
    assert_eq!(edits(&plugin), ["mic_setup_rev", "mic_setup_rev"]);
    assert_eq!(plugin.bridge.params.mic_setup_rev.value(), 2);
}

// ---------------------------------------------------------------------------
// Undo: one announce per gesture
// ---------------------------------------------------------------------------

fn edits(plugin: &ResonanceDrums) -> Vec<String> {
    plugin.bridge.host_asks.param_edits.lock().clone()
}

/// A knob drag writes the param on every frame but tells the host once,
/// when the drag ends — one undoable edit.
#[test]
fn a_knob_drag_is_one_announced_edit() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    let knob = frame.widget("knob.level").unwrap().rect;
    let before = plugin.bridge.params.pads[KICK].volume.value();
    let start = egui::pos2(knob.center().x, knob.top() + 20.0);
    e.drag(start, start + egui::vec2(0.0, 40.0));
    assert!(plugin.bridge.params.pads[KICK].volume.value() < before, "the drag moved nothing");
    assert_eq!(edits(&plugin), ["pad_0_level"]);
}

/// A fader drag in the Mix table, a mute click and a segment click: one
/// announce each.
#[test]
fn every_edit_kind_announces_once() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    let fader = frame.widget("mix.row.1.pan").unwrap().rect;
    e.drag(fader.center(), fader.center() + egui::vec2(-20.0, 0.0));
    assert!(plugin.bridge.params.pads[SNARE].pan.value() < 0.0);
    assert_eq!(edits(&plugin), ["pad_1_pan"]);

    let frame = settled(&mut e);
    e.click(frame.widget("mix.row.1.mute").unwrap().rect.center());
    assert!(plugin.bridge.params.pads[SNARE].mute.value());
    assert_eq!(edits(&plugin), ["pad_1_pan", "pad_1_mute"]);

    let frame = settled(&mut e);
    let multi = frame.text_center("Multi").expect("the output mode's Multi");
    e.click(multi);
    assert_eq!(plugin.bridge.params.output_mode.value(), OUTPUT_MODE_MULTI);
    assert_eq!(edits(&plugin), ["pad_1_pan", "pad_1_mute", "output_mode"]);

    // A click on the selected segment changes nothing and tells nothing.
    let frame = settled(&mut e);
    e.click(frame.text_center("Multi").unwrap());
    assert_eq!(edits(&plugin).len(), 3);
}

/// A gesture that changes nothing tells the host nothing: a drag that
/// comes back to where it started, a double-click reset of a value already
/// at its default. A reset of a moved value is one edit.
#[test]
fn a_gesture_that_changes_nothing_is_no_edit() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    let knob = frame.widget("knob.tune").unwrap().rect;
    let at = egui::pos2(knob.center().x, knob.top() + 20.0);
    // Down 30 px, then back up by what moved the knob, in one drag. The
    // first 5 px step is inside egui's click slop and moves nothing, so
    // the knob is back where it was 5 px below the press.
    e.drag_without_release(at, at + egui::vec2(0.0, 30.0));
    for i in 1..=5 {
        let p = at + egui::vec2(0.0, 30.0 - 5.0 * i as f32);
        e.frame(vec![egui::Event::PointerMoved(p)]);
    }
    e.release(at + egui::vec2(0.0, 5.0));
    settled(&mut e);
    assert_eq!(plugin.bridge.params.pads[KICK].tune.value(), 0.0);
    assert!(edits(&plugin).is_empty(), "{:?}", edits(&plugin));

    // Tune is at its default: a double-click reset changes nothing.
    e.double_click(at);
    settled(&mut e);
    assert!(edits(&plugin).is_empty(), "{:?}", edits(&plugin));

    // Moved, then reset: two edits.
    e.drag(at, at + egui::vec2(0.0, -30.0));
    assert!(plugin.bridge.params.pads[KICK].tune.value() > 0.0);
    e.double_click(at);
    settled(&mut e);
    assert_eq!(plugin.bridge.params.pads[KICK].tune.value(), 0.0);
    assert_eq!(edits(&plugin), ["pad_0_tune", "pad_0_tune"]);
}

/// A fader click (no drag) moves nothing and tells nothing; a double-click
/// resets it to its default as one edit.
#[test]
fn a_fader_click_is_no_edit_and_a_double_click_resets() {
    let plugin = booted();
    plugin.bridge.params.pads[SNARE].pan.set_value(0.5);
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    let fader = frame.widget("mix.row.1.pan").unwrap().rect;
    e.click(egui::pos2(fader.left() + 4.0, fader.center().y));
    settled(&mut e);
    assert_eq!(plugin.bridge.params.pads[SNARE].pan.value(), 0.5, "a click moved the fader");
    assert!(edits(&plugin).is_empty());
    e.double_click(fader.center());
    settled(&mut e);
    assert_eq!(plugin.bridge.params.pads[SNARE].pan.value(), 0.0);
    assert_eq!(edits(&plugin), ["pad_1_pan"]);
}

/// A drag whose control stops being drawn before the release (its row
/// scrolled away, another tab shown) still ends as one edit, once the
/// pointer lets go.
#[test]
fn a_drag_whose_control_went_away_is_still_one_edit() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    let fader = frame.widget("mix.row.1.level").unwrap().rect;
    e.drag_without_release(fader.center(), fader.center() + egui::vec2(-30.0, 0.0));
    assert!(plugin.bridge.params.pads[SNARE].volume.value() < 0.0);
    // The control is no longer drawn when the button comes up.
    e.show_view("Setup");
    e.frame(Vec::new());
    assert!(edits(&plugin).is_empty(), "announced mid-drag");
    e.release(fader.center());
    settled(&mut e);
    assert_eq!(edits(&plugin), ["pad_1_level"]);
}

/// An editor closed mid-drag announces what the drag moved.
#[test]
fn closing_the_editor_mid_drag_is_still_one_edit() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    let fader = frame.widget("mix.row.1.level").unwrap().rect;
    e.drag_without_release(fader.center(), fader.center() + egui::vec2(-30.0, 0.0));
    assert!(edits(&plugin).is_empty());
    drop(e);
    assert_eq!(edits(&plugin), ["pad_1_level"]);
}

// ---------------------------------------------------------------------------
// Hits and meters (fix6)
// ---------------------------------------------------------------------------

/// The fill of the cell at `rect`.
fn cell_fill(frame: &EditorFrameProbe, rect: egui::Rect) -> Option<egui::Color32> {
    frame.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
        _ => None,
    })
}

/// Opening the editor does not flash the pads played before it opened:
/// those hits are history.
#[test]
fn opening_the_editor_does_not_flash_old_hits() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    hit(&mut plugin, TOM, 1.0);
    let mut e = TestEditor::new(&plugin, resonance_drums::library::shared(), (960.0, 640.0));
    let first = e.frame(Vec::new());
    let cell = first.widget(&format!("pad_cell.{TOM}")).unwrap().rect;
    let lit = cell_fill(&first, cell).unwrap();
    let rest = cell_fill(&e.idle(1.0), cell).unwrap();
    assert_eq!(lit, rest, "the tom flashed for a hit from before the editor opened");
    // The last hit is still read out.
    assert!(e.frame(Vec::new()).strings().iter().any(|s| s.contains(" v127 → layer ")));
}

/// A muted pad's hit plays nothing, so its cell does not light.
#[test]
fn a_muted_pad_does_not_flash() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    plugin.bridge.params.pads[TOM].mute.set_value(true);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    let cell = frame.widget(&format!("pad_cell.{TOM}")).unwrap().rect;
    let dark = cell_fill(&frame, cell).unwrap();
    hit(&mut plugin, TOM, 1.0);
    assert_eq!(cell_fill(&e.frame(Vec::new()), cell).unwrap(), dark, "a muted pad lit");
}

/// A hit from the kit before is not this kit's: once another kit plays,
/// the status bar and the inspector read no hit until a pad is played.
#[test]
fn a_kit_switch_forgets_the_old_kits_hits() {
    let kit = fixture_kit();
    let other = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    e.select_pad(SNARE);
    hit(&mut plugin, SNARE, 1.0);
    assert!(settled(&mut e).strings().iter().any(|s| s.starts_with("last hit v127")));

    let choices: [PadMicChoices; NUM_PADS] = std::array::from_fn(|_| PadMicChoices::default());
    spawn_loader(
        other.manifest.clone(),
        RATE,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        choices,
        [false; NUM_PADS],
    );
    settle(&mut plugin);
    let frame = settled(&mut e);
    assert!(frame.shows("no hit yet"), "{:?}", frame.strings());
    assert!(
        frame.strings().iter().any(|s| s.starts_with("not played yet")),
        "{:?}",
        frame.strings()
    );
    hit(&mut plugin, SNARE, 1.0);
    assert!(settled(&mut e).strings().iter().any(|s| s.starts_with("last hit v127")));
}

/// The OUT meter falls by time, not by frame: half a second after the
/// peak it reads the same at 10 Hz as at 60 Hz.
#[test]
fn the_out_meter_falls_at_the_same_speed_at_any_frame_rate() {
    let reading = |hz: f64| -> String {
        let plugin = booted();
        let mut e = editor(&plugin, (960.0, 640.0), "Pads");
        e.set_frame_rate(hz);
        for bits in &plugin.bridge.out_peak[..] {
            bits.store(1.0f32.to_bits(), Ordering::Relaxed);
        }
        e.frame(Vec::new());
        for bits in &plugin.bridge.out_peak[..] {
            bits.store(0.0f32.to_bits(), Ordering::Relaxed);
        }
        e.idle(0.5)
            .strings()
            .into_iter()
            .find(|s| s.ends_with(" dB") && s.starts_with('-'))
            .unwrap_or_else(|| panic!("no OUT reading at {hz} Hz"))
    };
    let (slow, fast) = (reading(10.0), reading(60.0));
    assert_eq!(slow, fast, "the meter fell at different speeds");
    // 0.75 (−2.5 dB) per 100 ms, over 0.5 s.
    assert_eq!(slow, "-12.5 dB");
}

/// At rest — meters at zero, no load, no job — the editor does not tick
/// at 10 Hz; a lit cell animates at ~30 Hz, not at the display's rate.
#[test]
fn the_editor_repaints_only_as_fast_as_something_moves() {
    let kit = fixture_kit();
    let mut plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let rest = e.idle(1.0).repaint_after;
    assert!(rest >= Duration::from_millis(200), "at rest it repaints every {rest:?}");
    hit(&mut plugin, TOM, 1.0);
    let lit = e.frame(Vec::new()).repaint_after;
    // egui takes its predicted frame time (1/60 s) off a requested delay:
    // ~33 ms asked, ~16 ms reported — not 0, the display's own rate.
    assert!(
        lit >= Duration::from_millis(10) && lit <= Duration::from_millis(40),
        "a lit cell repaints every {lit:?}"
    );
}

// ---------------------------------------------------------------------------
// Layout and UX (fix6)
// ---------------------------------------------------------------------------

/// Probed names that are containers or duplicates of another probe, not
/// controls: a card, a table row, a sample stage, a marker drawn over a
/// cell or a combo.
fn is_container(name: &str) -> bool {
    let row = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
    };
    CARDS.contains(&name)
        || name == "inspector.sample"
        || name == "pad_cell.velocity"
        || name.ends_with(".dimmed")
        || name.ends_with(".absent")
        || row("mix.row.")
        || row("setup.row.")
}

/// No two body controls overlap on screen, on any tab at any size.
#[test]
fn no_two_body_controls_overlap() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    for size in SIZES {
        for tab in TABS {
            for pad in [KICK, TOM] {
                let mut e = editor(&plugin, size, tab);
                e.select_pad(pad);
                let frame = settled(&mut e);
                let shown: Vec<(&str, egui::Rect)> = frame
                    .widgets
                    .iter()
                    .filter(|w| !is_container(&w.name))
                    .map(|w| (w.name.as_str(), w.rect.intersect(w.clip).intersect(frame.screen)))
                    .filter(|(_, v)| v.width() > 0.0 && v.height() > 0.0)
                    .collect();
                assert!(shown.len() > 20, "{tab} {size:?}: almost nothing laid out");
                for (i, (a, ra)) in shown.iter().enumerate() {
                    for (b, rb) in &shown[i + 1..] {
                        let o = ra.intersect(*rb);
                        assert!(
                            !(o.width() > 1.5 && o.height() > 1.5),
                            "{tab} {size:?} pad {pad}: {a} {ra:?} overlaps {b} {rb:?}"
                        );
                    }
                }
            }
        }
    }
}

/// The six inspector knobs sit in one row at every size — at the
/// minimum the dials shrink rather than leave Start alone on a row.
#[test]
fn the_six_knobs_fit_one_row_at_every_size() {
    let plugin = booted();
    for size in SIZES {
        let mut e = editor(&plugin, size, "Pads");
        let frame = settled(&mut e);
        let tops: Vec<f32> = ["level", "pan", "tune", "hold", "decay", "start"]
            .iter()
            .map(|k| frame.widget(&format!("knob.{k}")).unwrap().rect.top())
            .collect();
        assert!(
            tops.iter().all(|t| (t - tops[0]).abs() < 0.5),
            "{size:?}: the knobs wrapped: {tops:?}"
        );
    }
}

/// The grid grows with the window, keeping its cells' shape, and the
/// inspector's controls stop growing at a readable width.
#[test]
fn the_grid_grows_and_the_inspector_controls_are_capped() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let cell = |size| {
        let mut e = editor(&plugin, size, "Pads");
        let frame = settled(&mut e);
        (frame.widget("pad_cell.0").unwrap().rect, frame)
    };
    let (small, _) = cell((960.0, 640.0));
    let (big, frame) = cell((1571.0, 856.0));
    assert!(big.width() > small.width() * 1.6, "{small:?} → {big:?}");
    let aspect = |r: egui::Rect| r.width() / r.height();
    assert!((aspect(big) - aspect(small)).abs() < 0.05, "{small:?} → {big:?}");
    let grid = frame.widget("pad_grid").unwrap().rect;
    let share = grid.width() / frame.screen.width();
    assert!((0.5..=0.62).contains(&share), "the grid takes {share} of the window");
    for name in ["trim.mic1", "trim.oh", "mic.0"] {
        let w = frame.widget(name).unwrap_or_else(|| panic!("{name}")).rect.width();
        assert!(w <= 361.0, "{name} is {w} px wide");
    }
}

/// Hovering a playable cell shows the velocity a click there plays.
#[test]
fn hovering_a_cell_shows_the_velocity_it_plays() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    let cell = frame.widget(&format!("pad_cell.{SNARE}")).unwrap().rect;
    e.frame(vec![egui::Event::PointerMoved(egui::pos2(cell.center().x, cell.top() + 2.0))]);
    let frame = e.frame(Vec::new());
    assert!(frame.widget("pad_cell.velocity").is_some(), "no velocity readout");
    let v: u32 = frame
        .strings()
        .iter()
        .find_map(|s| s.strip_prefix('v').and_then(|n| n.parse().ok()))
        .expect("a vNN readout");
    assert!(v >= 120, "the top of the cell reads v{v}");
    // Off the grid: no readout.
    e.frame(vec![egui::Event::PointerMoved(egui::pos2(5.0, 5.0))]);
    assert!(e.frame(Vec::new()).widget("pad_cell.velocity").is_none());
}

/// In Stereo a pad's Output picker is dimmed (it plays nothing) in the
/// inspector and on the Setup table; in Multi it is not. The room setup
/// picker is dimmed while the room is off.
#[test]
fn settings_that_play_nothing_now_are_dimmed() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    assert!(settled(&mut e).widget("routing.output.dimmed").is_some());
    e.show_view("Setup");
    let frame = settled(&mut e);
    assert!(frame.widget("setup.row.0.output.dimmed").is_some());
    assert!(frame.widget("setup.row.0.choke.dimmed").is_none(), "choke applies in Stereo");
    assert!(frame.widget("setup.room.setup.dimmed").is_some(), "the room is off");

    plugin.bridge.params.output_mode.set_value(OUTPUT_MODE_MULTI);
    plugin.bridge.params.room_on.set_value(resonance_drums::params::BANK_ON);
    let frame = settled(&mut e);
    assert!(frame.widget("setup.row.0.output.dimmed").is_none());
    assert!(frame.widget("setup.room.setup.dimmed").is_none());
    e.show_view("Pads");
    assert!(settled(&mut e).widget("routing.output.dimmed").is_none());
}

/// The Stereo / Multi switch heads the OUTPUTS card it decides; the Mix
/// table is level, pan and mute only (output and choke are routing, on
/// the Setup table and in the inspector).
#[test]
fn the_mix_tab_has_one_home_per_control() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    let switch = frame.widget("mix.output_mode").expect("the output mode switch").rect;
    let outputs = frame.widget("mix.outputs").unwrap().rect;
    assert!(outputs.contains_rect(switch), "{switch:?} is not in OUTPUTS {outputs:?}");
    assert!(frame.widget("global.output_mode").is_none());
    for gone in ["mix.row.0.output", "mix.row.0.choke"] {
        assert!(frame.widget(gone).is_none(), "{gone} is back in the Mix table");
    }
    for kept in ["mix.row.0.level", "mix.row.0.pan", "mix.row.0.mute"] {
        assert!(frame.widget(kept).is_some(), "{kept} is missing");
    }
}

/// A caption's hint is drawn beside it where it fits.
#[test]
fn caption_hints_are_drawn_where_they_fit() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Mix");
    let frame = settled(&mut e);
    assert!(frame.shows("harder ← linear → softer"), "{:?}", frame.strings());
    let label = frame.widget("caption.Velocity curve").unwrap().rect;
    let hint = frame.widget("caption.Velocity curve.hint").unwrap().rect;
    assert!(hint.left() >= label.right() && (hint.center().y - label.center().y).abs() < 2.0);
}

/// The built-in kit has no mics: its inspector says TRIM, with no picker.
/// A kit's says MICS.
#[test]
fn the_built_in_kit_has_a_trim_not_mics() {
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    let frame = settled(&mut e);
    assert!(frame.shows("TRIM") && !frame.shows("MICS"), "{:?}", frame.strings());
    assert!(frame.widget("mic.0").is_none() && frame.widget("trim.mic1").is_some());
    assert!(!frame.shows("built-in"), "the old placeholder picker is back");

    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    assert!(settled(&mut e).shows("MICS"));
}

/// Overhead slot 1 offers "Kit default" (the kit's first overhead setup),
/// so a pick there can be taken back.
#[test]
fn overhead_slot_1_offers_the_kit_default() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Setup");
    let frame = settled(&mut e);
    e.click(frame.widget("setup.oh.0").unwrap().rect.center());
    let frame = settled(&mut e);
    let item = frame.text_center("Kit default").expect("slot 1 lists the kit default");
    e.click(item);
    assert_eq!(plugin.bridge.overhead_slots()[0], "");
    assert_eq!(edits(&plugin), ["mic_setup_rev"]);
}

/// The sample stage says what is going on when it has nothing to draw:
/// "loading…" while a kit loads.
#[test]
fn the_sample_stage_says_loading_while_a_kit_loads() {
    let kit = fixture_kit();
    let plugin = booted();
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    e.select_pad(SNARE);
    let choices: [PadMicChoices; NUM_PADS] = std::array::from_fn(|_| PadMicChoices::default());
    // Swap the pads' infos out, so the stage has nothing to show, and
    // start a load: until it lands the stage says it is loading.
    *plugin.bridge.pad_samples.lock() = std::sync::Arc::new(Vec::new());
    spawn_loader(
        kit.manifest.clone(),
        RATE,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        choices,
        [false; NUM_PADS],
    );
    let frame = e.frame(Vec::new());
    let loading = frame.strings().iter().any(|s| s == "loading…");
    let landed = !plugin.bridge.pad_samples.lock().is_empty();
    assert!(loading || landed, "{:?}", frame.strings());
}

// ---------------------------------------------------------------------------
// Every write announces (fix6): one runtime check per kind of control
// ---------------------------------------------------------------------------

/// Pick `item` from the combo probed as `combo`.
fn pick(e: &mut TestEditor, combo: &str, item: &str) {
    let frame = settled(e);
    let frame = reveal(e, frame, combo).unwrap();
    e.click(frame.widget(combo).unwrap().rect.center());
    let frame = settled(e);
    let at = frame
        .text_center(item)
        .unwrap_or_else(|| panic!("{combo} does not list {item:?}: {:?}", frame.strings()));
    e.click(at);
}

/// Drag the slider probed as `name` 30 px to the left.
fn nudge(e: &mut TestEditor, name: &str) {
    let frame = settled(e);
    let frame = reveal(e, frame, name).unwrap();
    let r = frame.widget(name).unwrap().rect;
    e.drag(r.center(), r.center() + egui::vec2(-30.0, 0.0));
}

#[test]
fn every_inspector_write_announces_once() {
    let kit = fixture_kit_with_articulation();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Pads");
    e.select_pad(KICK);

    pick(&mut e, "routing.output", "Overhead");
    pick(&mut e, "routing.choke", "Group 3");
    nudge(&mut e, "trim.mic1");
    nudge(&mut e, "trim.oh");
    let frame = settled(&mut e);
    let frame = reveal(&mut e, frame, "articulation.1").expect("the kick has an alternate");
    e.click(frame.widget("articulation.1").unwrap().rect.center());
    let frame = settled(&mut e);
    e.click(frame.widget("inspector.mute").unwrap().rect.center());

    assert_eq!(
        edits(&plugin),
        [
            "pad_0_output",
            "pad_0_choke",
            "pad_0_mic1_trim",
            "pad_0_oh_trim",
            "pad_0_articulation",
            "pad_0_mute"
        ]
    );
}

#[test]
fn every_setup_and_global_write_announces_once() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    let mut e = editor(&plugin, (960.0, 640.0), "Setup");

    // The table first, each pick in a fresh editor: in a headless
    // context egui scrolls the virtual table to its end on the click
    // after a popup pick or a wheel turned over another column, and the
    // row is gone before its popup opens.
    pick(&mut e, "setup.row.1.choke", "Group 2");
    plugin.bridge.params.output_mode.set_value(OUTPUT_MODE_MULTI);
    let mut e = editor(&plugin, (960.0, 640.0), "Setup");
    pick(&mut e, "setup.row.1.output", "Toms");
    let mut e = editor(&plugin, (960.0, 640.0), "Setup");
    nudge(&mut e, "setup.oh.0.level");
    let frame = settled(&mut e);
    e.click(frame.widget("setup.room.on").unwrap().rect.right_center() - egui::vec2(8.0, 0.0));
    nudge(&mut e, "setup.room.level");
    let frame = settled(&mut e);
    e.click(frame.widget("setup.bleed.on").unwrap().rect.right_center() - egui::vec2(8.0, 0.0));
    nudge(&mut e, "setup.bleed.level");
    let frame = settled(&mut e);
    let frame = reveal(&mut e, frame, "setup.preload").unwrap();
    e.click(frame.widget("setup.preload").unwrap().rect.left_center() + egui::vec2(8.0, 0.0));

    e.show_view("Mix");
    nudge(&mut e, "global.polyphony");
    // Humanize rests at 0, the left end: a drag left changes nothing (and
    // tells nothing); one to the right is an edit.
    nudge(&mut e, "global.velocity_humanize");
    let frame = settled(&mut e);
    let r = frame.widget("global.velocity_humanize").unwrap().rect;
    e.drag(r.left_center() + egui::vec2(4.0, 0.0), r.left_center() + egui::vec2(40.0, 0.0));
    let frame = settled(&mut e);
    let cycle = frame.widget("global.round_robin").unwrap().rect;
    e.click(cycle.right_center() - egui::vec2(8.0, 0.0));
    nudge(&mut e, "mix.row.1.level");

    assert_eq!(
        edits(&plugin),
        [
            "pad_1_choke",
            "pad_1_output",
            "oh_1_level",
            "room_on",
            "room_level",
            "bleed_on",
            "bleed_level",
            "stream_preload",
            "polyphony",
            "velocity_humanize",
            "round_robin_mode",
            "pad_1_level",
        ]
    );
}
