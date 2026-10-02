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

/// §6.1: 960×640 default, 780×520 minimum.
const SIZES: [(f32, f32); 2] = [(960.0, 640.0), (780.0, 520.0)];
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
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-editor-tabs-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut pieces = serde_json::Map::new();
    for (piece, setups) in PIECES {
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

/// The kit is named once, in the header — not again in a pad-list card
/// and a KIT card (§1.3). The old KIT/GLOBAL bottom cards are gone.
#[test]
fn the_kit_is_named_once_and_the_old_cards_are_gone() {
    let kit = fixture_kit();
    let plugin = with_kit(&kit);
    for tab in ["Pads", "Mix"] {
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
        v.push("routing.output.stereo".into());
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
        let last = frame.widget("mix.row.29.output").expect("the last row's output");
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
