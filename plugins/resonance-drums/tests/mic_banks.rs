//! More mic banks (drums-plugin-rework.md §7 E15, slice K7b): overhead
//! setups layered on slot 1, bleed (another piece's close mic on this
//! piece's hits) and room.
//!
//! The fixture kit holds constant-level takes, one per setup, each at its
//! own level: a port's output is then the sum of the banks routed to it,
//! frame for frame, and what one bank adds or loses can be read off
//! exactly. Every test writes its own kit into its own temp directory
//! (the sample cache is process-wide, and the decode counts are per
//! file), at runtime rather than from `tests/fixtures`, for the same
//! reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::kit::{
    BankKind, LoadedPad, MAIN_PORT_INDEX, NUM_OUTPUT_PORTS, OVERHEAD_PORT_INDEX,
};
use resonance_drums::kit_loader::banks::{resolve_extra_banks, BankRequest, MicBankSetups};
use resonance_drums::kit_loader::{
    spawn_loader, KitStatus, LoadStats, MicSetup, PadMicChoices, DEFAULT_OVERHEAD_SETUP,
};
use resonance_drums::params::{BANK_OFF, BANK_ON, OUTPUT_MODE_MULTI, OUTPUT_MODE_STEREO};
use resonance_drums::ResonanceDrums;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// Frames in every fixture take: longer than a block, shorter than any
/// preload, so nothing streams.
const TAKE_FRAMES: usize = 2_400;

const KICK: usize = 0;
const SNARE: usize = 1;
/// Tom High: the Drummica table's `SD Tom01 mit Teppich`.
const TOM: usize = 9;
/// Crash 16 Edge: overheads (and room) only.
const CRASH: usize = 12;

const OH_XY: &str = "25_OHsXY_USM69i";
const ROOM: &str = "30_Room";
const ROOM_FAR: &str = "31_RoomFar";

// ---------------------------------------------------------------------------
// Fixture kit
// ---------------------------------------------------------------------------

/// 16-bit PCM WAV at `RATE` holding `level` (the right channel of a
/// stereo file at `-level`).
fn write_wav(path: &Path, channels: u16, level: f32) {
    let block_align = channels * 2;
    let data_len = TAKE_FRAMES * block_align as usize;
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
    let v = (level * i16::MAX as f32).round() as i16;
    for _ in 0..TAKE_FRAMES {
        out.extend_from_slice(&v.to_le_bytes());
        if channels == 2 {
            out.extend_from_slice(&(-v).to_le_bytes());
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

/// (setup key, position, file, channels, level) per piece.
type Setup = (&'static str, &'static str, &'static str, u16, f32);

const PIECES: [(&str, &[Setup]); 4] = [
    (
        "SD Kick mit Teppich",
        &[
            ("01_KickIn_e901", "KickIn", "k_in", 1, 0.10),
            ("04_KickOut_TLM170", "KickOut", "k_out", 1, 0.11),
            // Bleed: the snare's bottom mic, heard on the kick.
            ("09_SNBtm_e906", "SNBtm", "k_snbtm", 1, 0.03),
            ("23_OHsAB_e914", "OHsAB", "k_ohab", 2, 0.05),
            (OH_XY, "OHsXY", "k_ohxy", 2, 0.04),
            (ROOM, "Room", "k_room", 2, 0.02),
            (ROOM_FAR, "RoomFar", "k_roomfar", 2, 0.015),
        ],
    ),
    (
        "SD Snare Normal",
        &[
            ("06_SNTop_MD441", "SNTop", "s_top", 1, 0.12),
            // The snare's own bottom mic: a close mic here, not bleed.
            ("09_SNBtm_e906", "SNBtm", "s_btm", 1, 0.07),
            ("23_OHsAB_e914", "OHsAB", "s_ohab", 2, 0.06),
            (OH_XY, "OHsXY", "s_ohxy", 2, 0.045),
            (ROOM, "Room", "s_room", 2, 0.025),
        ],
    ),
    (
        "SD Tom01 mit Teppich",
        &[
            ("11_Tom01_e904", "Tom01", "t_close", 1, 0.09),
            ("09_SNBtm_e906", "SNBtm", "t_snbtm", 1, 0.035),
            ("23_OHsAB_e914", "OHsAB", "t_ohab", 2, 0.055),
        ],
    ),
    (
        "SD Crash 16 Edge",
        &[
            ("23_OHsAB_e914", "OHsAB", "c_ohab", 2, 0.08),
            (OH_XY, "OHsXY", "c_ohxy", 2, 0.065),
            (ROOM, "Room", "c_room", 2, 0.03),
        ],
    ),
];

fn fixture_kit() -> Kit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-mic-banks-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut pieces = serde_json::Map::new();
    for (piece, setups) in PIECES {
        let mut map = serde_json::Map::new();
        for &(key, position, file, channels, level) in setups {
            let name = format!("{file}.wav");
            write_wav(&dir.join(&name), channels, level);
            map.insert(
                key.to_string(),
                serde_json::json!({
                    "brand": "Brand",
                    "channel": "1",
                    "mic": file,
                    "position": position,
                    "rounds": {"RR1": {"Vel01": name}},
                }),
            );
        }
        pieces.insert(piece.to_string(), serde_json::Value::Object(map));
    }
    let manifest = dir.join("drum_samples.json");
    std::fs::write(&manifest, serde_json::to_vec(&pieces).unwrap()).unwrap();
    Kit { dir, manifest }
}

// ---------------------------------------------------------------------------
// Plugin helpers
// ---------------------------------------------------------------------------

fn booted(mode: i32) -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    plugin.bridge.params.output_mode.set_value(mode);
    assert!(plugin.initialize(RATE, BLOCK as u32));
    plugin
}

fn pick(plugin: &ResonanceDrums, kit: &Kit) {
    let choices: [PadMicChoices; NUM_PADS] = std::array::from_fn(|_| PadMicChoices::default());
    spawn_loader(
        kit.manifest.clone(),
        RATE,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        choices,
        [false; NUM_PADS],
    );
}

/// Wait for the load in flight to finish, then let the audio thread take
/// the kit; returns what the load did.
fn settle(plugin: &mut ResonanceDrums) -> LoadStats {
    let deadline = Instant::now() + Duration::from_secs(30);
    let stats = loop {
        let pending = plugin.bridge.pending_kit.lock().is_some();
        let status = plugin.bridge.kit_status.lock().clone();
        match status {
            KitStatus::Loaded { .. } if !pending => break plugin.bridge.load_stats.lock().clone(),
            KitStatus::Error { message } if !pending => panic!("kit failed to load: {message}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "load never settled");
        std::thread::sleep(Duration::from_millis(2));
    };
    // The swap, and its fade, out of the way.
    for _ in 0..4 {
        render(plugin, &[]);
    }
    stats
}

/// What the watcher does when a param it watches moves (here, at once).
fn watch(plugin: &ResonanceDrums) {
    resonance_drums::selection::watch(&plugin.bridge);
}

fn render(plugin: &mut ResonanceDrums, events: &[NoteEvent]) -> Vec<(Vec<f32>, Vec<f32>)> {
    let mut buffers: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    {
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
    buffers
}

/// Strike pad `pad` at full velocity on a clean slate (params settled:
/// one block renders first, so every level ramp has arrived) and return
/// each port's (left, right) level. The takes are constant, so every
/// frame of a port holds the same value — checked.
fn strike(plugin: &mut ResonanceDrums, pad: usize) -> [(f32, f32); NUM_OUTPUT_PORTS] {
    render(plugin, &[]);
    plugin.reset();
    let out = render(
        plugin,
        &[NoteEvent::NoteOn {
            note: drum_map::PAD_MAPPINGS[pad].note,
            velocity: 1.0,
            timing: 0,
        }],
    );
    std::array::from_fn(|port| {
        let (l, r) = &out[port];
        for i in 0..BLOCK {
            assert!(
                (l[i] - l[0]).abs() < 1e-6 && (r[i] - r[0]).abs() < 1e-6,
                "port {port} is not constant at frame {i}"
            );
        }
        (l[0], r[0])
    })
}

/// The (left, right) level a take of `bank` plays at (a mono take on
/// both sides).
fn take_level(bank: &resonance_drums::kit::LoadedMicBank) -> (f32, f32) {
    bank.layers[0].round_robins[0].frame(0)
}

fn built_pad(plugin: &ResonanceDrums, pad: usize) -> LoadedPad {
    plugin
        .bridge
        .built_kit
        .lock()
        .as_ref()
        .expect("a built kit")
        .pads[pad]
        .clone()
}

fn extra_keys(pad: &LoadedPad) -> Vec<(BankKind, String)> {
    pad.extra_banks
        .iter()
        .map(|e| (e.kind, e.bank.setup_key.clone()))
        .collect()
}

fn assert_close(got: (f32, f32), want: (f32, f32), what: &str) {
    assert!(
        (got.0 - want.0).abs() < 1e-5 && (got.1 - want.1).abs() < 1e-5,
        "{what}: got {got:?}, want {want:?}"
    );
}

fn add(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 + b.0, a.1 + b.1)
}

fn scale(a: (f32, f32), g: f32) -> (f32, f32) {
    (a.0 * g, a.1 * g)
}

// ---------------------------------------------------------------------------
// Off by default
// ---------------------------------------------------------------------------

/// With nothing turned on, a kit loads exactly the banks it always did —
/// close mics and overhead slot 1 — and the catalog says what the Setup
/// tab could turn on.
#[test]
fn every_extra_bank_is_off_by_default() {
    let kit = fixture_kit();
    let mut plugin = booted(OUTPUT_MODE_MULTI);
    pick(&plugin, &kit);
    let stats = settle(&mut plugin);
    // Kick in/out/OH, snare top/btm/OH, tom close/OH, crash OH.
    assert_eq!(stats.files, 9, "{stats:?}");
    for pad in [KICK, SNARE, TOM, CRASH] {
        assert!(built_pad(&plugin, pad).extra_banks.is_empty(), "pad {pad}");
    }
    let params = &plugin.bridge.params;
    assert!(!params.bleed_enabled() && !params.room_enabled());
    assert_eq!(
        plugin.bridge.overhead_slots(),
        [
            DEFAULT_OVERHEAD_SETUP.to_string(),
            String::new(),
            String::new()
        ]
    );

    let catalog = plugin.bridge.catalog.lock().clone();
    assert_eq!(catalog.overhead_setups(), ["23_OHsAB_e914", OH_XY]);
    assert_eq!(catalog.room_setups(), [ROOM, ROOM_FAR]);
    assert_eq!(catalog.bleed.len(), 1, "{:?}", catalog.bleed);
    assert_eq!(catalog.bleed[0].position, "SNBtm");
    assert_eq!(catalog.bleed[0].setups, ["09_SNBtm_e906"]);
    assert_eq!(
        catalog.bleed[0].pads,
        [KICK, TOM],
        "the snare's own SN Btm is no bleed"
    );
    assert!(catalog.has_bleed());
    // (The fixture names each file's mic after the file; a setup is
    // labelled from the first piece that has it.)
    assert_eq!(catalog.label(OH_XY), "OHsXY · Brand c_ohxy");
    assert_eq!(catalog.label("nope"), "nope");
}

// ---------------------------------------------------------------------------
// Bleed
// ---------------------------------------------------------------------------

/// `bleed_on` loads the bleed banks and nothing else (E4: only the pads
/// that gain one are rebuilt, only their files decoded); off removes
/// exactly that bank's signal.
#[test]
fn bleed_loads_only_its_banks_and_off_removes_exactly_its_energy() {
    let kit = fixture_kit();
    let mut plugin = booted(OUTPUT_MODE_STEREO);
    pick(&plugin, &kit);
    settle(&mut plugin);
    let without = strike(&mut plugin, KICK);

    plugin.bridge.params.bleed_on.set_value(BANK_ON);
    watch(&plugin);
    let on = settle(&mut plugin);
    assert_eq!(on.rebuilt_pads, 2, "kick and tom gain a bleed bank: {on:?}");
    assert_eq!(on.decoded, 2, "only the two bleed files decode: {on:?}");
    assert_eq!(
        on.cached, 5,
        "the kick's and tom's other banks are cached: {on:?}"
    );
    let kick = built_pad(&plugin, KICK);
    assert_eq!(
        extra_keys(&kick),
        [(BankKind::Bleed, "09_SNBtm_e906".to_string())]
    );
    assert_eq!(
        extra_keys(&built_pad(&plugin, TOM)),
        [(BankKind::Bleed, "09_SNBtm_e906".to_string())]
    );
    assert!(built_pad(&plugin, SNARE).extra_banks.is_empty());
    let bleed = take_level(&kick.extra_banks[0].bank);
    assert!(bleed.0 > 0.02, "the bleed take is not silent");

    let with = strike(&mut plugin, KICK);
    // Stereo: everything on Main, the bleed take on top of the rest.
    assert_close(
        with[MAIN_PORT_INDEX],
        add(without[MAIN_PORT_INDEX], bleed),
        "bleed on",
    );
    for (port, level) in with.iter().enumerate().skip(1) {
        assert_eq!(*level, (0.0, 0.0), "port {port} in Stereo");
    }

    // The level and the pad's trim scale it.
    plugin.bridge.params.bleed_level.set_value(-6.0206);
    let half = strike(&mut plugin, KICK);
    assert_close(
        half[MAIN_PORT_INDEX],
        add(without[MAIN_PORT_INDEX], scale(bleed, 0.5)),
        "bleed at -6 dB",
    );
    plugin.bridge.params.bleed_level.set_value(0.0);
    plugin.bridge.params.pads[KICK].trims[3].set_value(-60.0);
    assert_close(
        strike(&mut plugin, KICK)[MAIN_PORT_INDEX],
        without[MAIN_PORT_INDEX],
        "bleed trim -inf",
    );
    plugin.bridge.params.pads[KICK].trims[3].set_value(0.0);

    // Off: silent at once (the banks are muted before the reload lands)…
    plugin.bridge.params.bleed_on.set_value(BANK_OFF);
    assert_close(
        strike(&mut plugin, KICK)[MAIN_PORT_INDEX],
        without[MAIN_PORT_INDEX],
        "muted",
    );
    // …and the reload drops the banks, decoding nothing.
    watch(&plugin);
    let off = settle(&mut plugin);
    assert_eq!(off.rebuilt_pads, 2, "{off:?}");
    assert_eq!(off.decoded, 0, "{off:?}");
    assert!(built_pad(&plugin, KICK).extra_banks.is_empty());
    let after = strike(&mut plugin, KICK);
    assert_eq!(after, without, "off restores the kit bit for bit");
}

/// A bleed bank plays the mic the position's owner picked — the snare's
/// SN Btm pick on the kick — else the piece's first setup there.
#[test]
fn bleed_follows_the_owning_pads_mic_pick() {
    let setup = |position: &str| MicSetup {
        brand: String::new(),
        channel: String::new(),
        mic: String::new(),
        position: position.to_string(),
        rounds: BTreeMap::new(),
    };
    let piece: BTreeMap<String, MicSetup> = [
        ("01_KickIn", setup("KickIn")),
        ("09_SNBtm_e906", setup("SNBtm")),
        ("10_SNBtm_KM184", setup("SNBtm")),
        ("23_OHsAB_e914", setup("OHsAB")),
        ("25_OHsXY", setup("OHsXY")),
        ("26_OHsXY_b", setup("OHsXY")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let mut choices: Vec<PadMicChoices> = (0..NUM_PADS).map(|_| PadMicChoices::default()).collect();
    let banks = BankRequest {
        setups: MicBankSetups {
            extra_overheads: ["25_OHsXY".to_string(), "23_OHsAB_e914".to_string()],
            room: String::new(),
        },
        bleed: true,
        room: true,
    };
    let resolve = |choices: &[PadMicChoices]| {
        resolve_extra_banks(KICK, &piece, DEFAULT_OVERHEAD_SETUP, choices, &banks)
    };
    // Slot 3 names slot 1's setup: not played twice. No room setup in
    // the piece: no room bank.
    assert_eq!(
        resolve(&choices),
        [
            (BankKind::Overhead { slot: 1 }, "25_OHsXY".to_string()),
            (BankKind::Bleed, "09_SNBtm_e906".to_string()),
        ]
    );
    choices[SNARE]
        .close_setups
        .insert("SNBtm".to_string(), "10_SNBtm_KM184".to_string());
    assert_eq!(
        resolve(&choices)[1],
        (BankKind::Bleed, "10_SNBtm_KM184".to_string())
    );
    // Everything off: nothing, whatever the setups say.
    let off = BankRequest {
        setups: banks.setups.clone(),
        ..BankRequest::default()
    };
    let only_oh = resolve_extra_banks(KICK, &piece, DEFAULT_OVERHEAD_SETUP, &choices, &off);
    assert_eq!(only_oh.len(), 1, "the overhead slots are on by their setup");
    let none = BankRequest::default();
    assert!(resolve_extra_banks(KICK, &piece, DEFAULT_OVERHEAD_SETUP, &choices, &none).is_empty());
}

// ---------------------------------------------------------------------------
// Overheads
// ---------------------------------------------------------------------------

/// A second overhead setup layers on the first; each slot's level scales
/// its bank and nothing else.
#[test]
fn each_overhead_slot_level_scales_only_its_bank() {
    let kit = fixture_kit();
    let mut plugin = booted(OUTPUT_MODE_MULTI);
    pick(&plugin, &kit);
    settle(&mut plugin);
    let single = strike(&mut plugin, KICK);
    let ab = take_level(built_pad(&plugin, KICK).overhead.as_ref().unwrap());

    assert!(plugin.bridge.set_overhead_slot(1, OH_XY));
    let stats = settle(&mut plugin);
    assert_eq!(
        stats.rebuilt_pads, 3,
        "kick, snare and crash have OH XY: {stats:?}"
    );
    assert_eq!(stats.decoded, 3, "only the XY files decode: {stats:?}");
    let kick = built_pad(&plugin, KICK);
    assert_eq!(
        extra_keys(&kick),
        [(BankKind::Overhead { slot: 1 }, OH_XY.to_string())]
    );
    assert!(
        built_pad(&plugin, TOM).extra_banks.is_empty(),
        "the tom has no XY"
    );
    let xy = take_level(&kick.extra_banks[0].bank);

    let both = strike(&mut plugin, KICK);
    assert_close(
        both[OVERHEAD_PORT_INDEX],
        add(ab, xy),
        "AB + XY on Overhead",
    );
    assert_eq!(both[1], single[1], "the Kick port is untouched");

    let params = &plugin.bridge.params;
    params.oh_levels[1].set_value(-6.0206);
    let lowered = strike(&mut plugin, KICK);
    assert_close(
        lowered[OVERHEAD_PORT_INDEX],
        add(ab, scale(xy, 0.5)),
        "XY at -6 dB",
    );
    assert_eq!(lowered[1], single[1]);

    plugin.bridge.params.oh_levels[0].set_value(-60.0);
    let xy_only = strike(&mut plugin, KICK);
    assert_close(xy_only[OVERHEAD_PORT_INDEX], scale(xy, 0.5), "slot 1 off");
    assert_eq!(xy_only[1], single[1]);

    // The pad's OH trim covers every overhead slot.
    plugin.bridge.params.oh_levels[0].set_value(0.0);
    plugin.bridge.params.oh_levels[1].set_value(0.0);
    plugin.bridge.params.pads[KICK].trims[2].set_value(-6.0206);
    let trimmed = strike(&mut plugin, KICK);
    assert_close(
        trimmed[OVERHEAD_PORT_INDEX],
        scale(add(ab, xy), 0.5),
        "OH trim",
    );
    plugin.bridge.params.pads[KICK].trims[2].set_value(0.0);

    // A cymbal's overheads are its sound: every slot on its own port.
    let crash = strike(&mut plugin, CRASH);
    let crash_pad = built_pad(&plugin, CRASH);
    let cymbals = 5;
    assert_close(
        crash[cymbals],
        add(
            take_level(crash_pad.overhead.as_ref().unwrap()),
            take_level(&crash_pad.extra_banks[0].bank),
        ),
        "crash AB + XY on Cymbals",
    );
    assert_eq!(crash[OVERHEAD_PORT_INDEX], (0.0, 0.0));

    // Slot 3 on slot 1's setup adds nothing, rebuilds nothing, and hands
    // nothing off: a swap would fade every voice and restart the round
    // robins for the same kit. Nor does a room setup while room is off.
    let taken = plugin.bridge.load_progress.kits_taken();
    assert!(plugin.bridge.set_overhead_slot(2, DEFAULT_OVERHEAD_SETUP));
    let dup = settle(&mut plugin);
    assert_eq!(dup.rebuilt_pads, 0, "{dup:?}");
    assert!(plugin.bridge.set_room_setup(ROOM_FAR));
    assert_eq!(settle(&mut plugin).rebuilt_pads, 0);
    assert_eq!(
        plugin.bridge.load_progress.kits_taken(),
        taken,
        "a bank change that built nothing swapped a kit in"
    );
    plugin.bridge.set_room_setup("");
    settle(&mut plugin);
    // Emptying slot 2 drops the XY banks again, decoding nothing.
    assert!(plugin.bridge.set_overhead_slot(1, ""));
    let emptied = settle(&mut plugin);
    assert_eq!(
        (emptied.rebuilt_pads, emptied.decoded),
        (3, 0),
        "{emptied:?}"
    );
    assert_eq!(strike(&mut plugin, KICK), single);
}

// ---------------------------------------------------------------------------
// Room, and routing
// ---------------------------------------------------------------------------

/// Bleed and room are ambience: the Overhead port in Multi (a cymbal's
/// room too), Main in Stereo.
#[test]
fn room_and_bleed_route_to_overhead_in_multi_and_main_in_stereo() {
    let kit = fixture_kit();
    let mut plugin = booted(OUTPUT_MODE_MULTI);
    pick(&plugin, &kit);
    settle(&mut plugin);
    let plain_kick = strike(&mut plugin, KICK);
    let plain_crash = strike(&mut plugin, CRASH);

    plugin.bridge.params.room_on.set_value(BANK_ON);
    plugin.bridge.params.bleed_on.set_value(BANK_ON);
    watch(&plugin);
    let stats = settle(&mut plugin);
    // Room on kick, snare, crash; bleed on kick and tom.
    assert_eq!(stats.decoded, 5, "{stats:?}");
    assert_eq!(stats.rebuilt_pads, 4, "{stats:?}");
    let kick = built_pad(&plugin, KICK);
    assert_eq!(
        extra_keys(&kick),
        [
            (BankKind::Bleed, "09_SNBtm_e906".to_string()),
            (BankKind::Room, ROOM.to_string()),
        ]
    );
    let bleed = take_level(&kick.extra_banks[0].bank);
    let room = take_level(&kick.extra_banks[1].bank);

    let multi = strike(&mut plugin, KICK);
    assert_eq!(
        multi[1], plain_kick[1],
        "the Kick port carries the close mics only"
    );
    assert_close(
        multi[OVERHEAD_PORT_INDEX],
        add(plain_kick[OVERHEAD_PORT_INDEX], add(bleed, room)),
        "OH + bleed + room on Overhead",
    );
    assert_eq!(multi[MAIN_PORT_INDEX], (0.0, 0.0));

    let crash = strike(&mut plugin, CRASH);
    let crash_room = take_level(&built_pad(&plugin, CRASH).extra_banks[0].bank);
    assert_eq!(
        crash[5], plain_crash[5],
        "the crash's own sound stays on Cymbals"
    );
    assert_close(
        crash[OVERHEAD_PORT_INDEX],
        crash_room,
        "the crash's room on Overhead",
    );

    // Room level scales the room bank only.
    plugin.bridge.params.room_level.set_value(-6.0206);
    let lowered = strike(&mut plugin, KICK);
    assert_close(
        lowered[OVERHEAD_PORT_INDEX],
        add(
            plain_kick[OVERHEAD_PORT_INDEX],
            add(bleed, scale(room, 0.5)),
        ),
        "room at -6 dB",
    );
    plugin.bridge.params.room_level.set_value(0.0);

    // Stereo: the whole hit on Main.
    plugin
        .bridge
        .params
        .output_mode
        .set_value(OUTPUT_MODE_STEREO);
    let stereo = strike(&mut plugin, KICK);
    let total = multi.iter().fold((0.0, 0.0), |acc, port| add(acc, *port));
    assert_close(stereo[MAIN_PORT_INDEX], total, "everything on Main");
    for (port, level) in stereo.iter().enumerate().skip(1) {
        assert_eq!(*level, (0.0, 0.0), "port {port} in Stereo");
    }

    // The room setup is the kit's choice; a piece without it plays its
    // first room setup.
    assert!(plugin.bridge.set_room_setup(ROOM_FAR));
    let moved = settle(&mut plugin);
    assert_eq!(moved.decoded, 1, "only the kick has RoomFar: {moved:?}");
    assert_eq!(
        extra_keys(&built_pad(&plugin, KICK))[1],
        (BankKind::Room, ROOM_FAR.to_string())
    );
    assert_eq!(
        extra_keys(&built_pad(&plugin, SNARE)),
        [(BankKind::Room, ROOM.to_string())]
    );
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The bank setups travel in the plugin state (`mic_banks`), the on/off
/// and levels as params; a state from before E15 loads with every extra
/// bank off.
#[test]
fn bank_choices_round_trip_and_older_states_load_without_them() {
    let src = ResonanceDrums::new();
    // No sample rate yet: the choices are recorded, nothing loads.
    assert!(!src.bridge.set_overhead_slot(1, OH_XY));
    assert!(!src.bridge.set_overhead_slot(2, "24_OHsAB_KM184"));
    assert!(!src.bridge.set_room_setup(ROOM_FAR));
    let p = &src.bridge.params;
    p.bleed_on.set_value(BANK_ON);
    p.room_on.set_value(BANK_ON);
    p.oh_levels[2].set_value(-3.0);
    p.bleed_level.set_value(-9.0);
    p.room_level.set_value(2.0);
    p.pads[KICK].trims[4].set_value(-4.5);

    let bytes = src.save_state();
    let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        state["mic_banks"],
        serde_json::json!({"overheads": [OH_XY, "24_OHsAB_KM184"], "room": ROOM_FAR})
    );

    let mut dst = ResonanceDrums::new();
    assert!(dst.load_state(&bytes));
    assert_eq!(
        dst.bridge.overhead_slots(),
        [
            DEFAULT_OVERHEAD_SETUP.to_string(),
            OH_XY.to_string(),
            "24_OHsAB_KM184".to_string()
        ]
    );
    assert_eq!(dst.bridge.mic_banks.lock().room, ROOM_FAR);
    let q = &dst.bridge.params;
    assert!(q.bleed_enabled() && q.room_enabled());
    assert_eq!(q.oh_levels[2].value(), -3.0);
    assert_eq!(q.bleed_level.value(), -9.0);
    assert_eq!(q.room_level.value(), 2.0);
    assert_eq!(q.pads[KICK].trims[4].value(), -4.5);
    assert_eq!(dst.bridge.bank_request(), src.bridge.bank_request());

    // A state from before E15: no `mic_banks`, none of the new params.
    let mut old = state.clone();
    old.as_object_mut().unwrap().remove("mic_banks");
    let params = old["params"].as_object_mut().unwrap();
    params.retain(|id, _| {
        !(id.starts_with("oh_")
            || id.starts_with("bleed")
            || id.starts_with("room")
            || id.ends_with("_bleed_trim")
            || id.ends_with("_room_trim"))
    });
    params.insert("pad_0_level".to_string(), serde_json::json!(-2.5));
    let mut fresh = ResonanceDrums::new();
    assert!(fresh.load_state(&serde_json::to_vec(&old).unwrap()));
    assert_eq!(*fresh.bridge.mic_banks.lock(), MicBankSetups::default());
    assert_eq!(fresh.bridge.bank_request(), BankRequest::default());
    assert_eq!(
        fresh.bridge.params.pads[0].volume.value(),
        -2.5,
        "the rest loads as before"
    );
    assert_eq!(fresh.bridge.params.oh_levels[2].value(), 0.0);
}

/// The new params sit where the plan says: globals after
/// `stream_preload`, the two new trims at the end of each pad's block.
#[test]
fn the_bank_params_are_where_hosts_find_them() {
    use resonance_drums::params::{GLOBAL_PARAMS, PARAMS_PER_PAD};
    let plugin = ResonanceDrums::new();
    let ids: Vec<&str> = (9..GLOBAL_PARAMS).map(|i| plugin.param(i).id()).collect();
    assert_eq!(
        ids,
        [
            "oh_1_level",
            "oh_2_level",
            "oh_3_level",
            "bleed_on",
            "bleed_level",
            "room_on",
            "room_level"
        ]
    );
    assert_eq!(plugin.param(4).id(), "kit_select");
    assert_eq!(plugin.param(5).id(), "kit_load_progress");
    for pad in [0, 29] {
        let base = GLOBAL_PARAMS + pad * PARAMS_PER_PAD;
        assert_eq!(plugin.param(base + 6).id(), format!("pad_{pad}_oh_trim"));
        assert_eq!(plugin.param(base + 12).id(), format!("pad_{pad}_start"));
        assert_eq!(
            plugin.param(base + 13).id(),
            format!("pad_{pad}_bleed_trim")
        );
        assert_eq!(plugin.param(base + 14).id(), format!("pad_{pad}_room_trim"));
    }
    let bleed_on = plugin.param(12);
    assert_eq!(bleed_on.default_plain(), 0.0);
    assert!(!bleed_on.is_automatable(), "a change is a load");
    assert_eq!(bleed_on.display(1.0), "On");
    let oh2 = plugin.param(10);
    assert_eq!((oh2.default_plain(), oh2.max_plain()), (0.0, 6.0));
    assert!(oh2.is_automatable());
}

/// The Setup tab polls `overhead_slots()` on the UI thread while a host
/// loads state on another. Both read `overhead_setup_key` and
/// `mic_banks`; `overhead_slots` used to hold `mic_banks` while taking
/// `overhead_setup_key`, the state load the other way round — an ABBA
/// deadlock. Hammered from two threads, both finish.
#[test]
fn polling_the_overhead_slots_never_deadlocks_a_state_load() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let src = ResonanceDrums::new();
    src.bridge.set_overhead_slot(1, OH_XY);
    src.bridge.set_room_setup(ROOM_FAR);
    let bytes = src.save_state();

    let mut loader = ResonanceDrums::new();
    let bridge = loader.bridge.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = crossbeam_channel::bounded::<&str>(2);

    let poll_stop = stop.clone();
    let poll_done = done_tx.clone();
    std::thread::spawn(move || {
        let mut polls = 0u64;
        while !poll_stop.load(Ordering::Relaxed) || polls < 1_000 {
            std::hint::black_box(bridge.overhead_slots());
            std::hint::black_box(bridge.wanted_request());
            polls += 1;
        }
        let _ = poll_done.send("poller");
    });
    let load_stop = stop.clone();
    std::thread::spawn(move || {
        for _ in 0..2_000 {
            assert!(loader.load_state(&bytes));
        }
        load_stop.store(true, Ordering::Relaxed);
        let _ = done_tx.send("loader");
    });

    let deadline = Instant::now() + Duration::from_secs(60);
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(Instant::now());
        done_rx
            .recv_timeout(left)
            .expect("overhead_slots() and a state load deadlocked");
    }
}
