//! Latest-wins across a re-activation (drums-plugin-rework.md §7 E3, E4).
//!
//! A host deactivates and re-activates a plugin to change the sample rate
//! or the block size, and it can do so while a kit is still decoding.
//! Three ways that used to go wrong:
//!
//! - (a) The first kit pick, still decoding when the host re-activated at
//!   another rate: `initialize` only reloaded `kit_path`, which a load
//!   writes on success, so nothing was reloaded — and the old-rate decode
//!   then landed and played pitch-shifted while the status said Loaded.
//! - (b) Kit W loaded, kit X picked, re-activation mid-decode:
//!   `initialize` reloaded W, which superseded X. The pick was lost.
//! - (c) A kit handed off but not yet taken by the audio thread when the
//!   host deactivated stayed in the mailbox and was installed after the
//!   re-activation — decoded for the old rate.
//!
//! And E4: an `initialize` at an unchanged rate with the wanted kit
//! already in the sampler decodes nothing.
//!
//! Loads are held "mid-decode" with the bridge's `decode_gate` test hook:
//! each loader waits for one message on it before decoding.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::kit_loader::{spawn_loader, KitStatus, PadMicChoices, DEFAULT_OVERHEAD_SETUP};
use resonance_drums::ResonanceDrums;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

/// The rate the test kits' WAVs are written at.
const FILE_RATE: f32 = 48_000.0;
const OTHER_RATE: f32 = 44_100.0;
const BLOCK: usize = 128;
const NUM_PORTS: usize = 7;
const KICK_PORT: usize = 1;
/// Pad volume (0.8) × master (0.8) at the defaults.
const CHAIN_GAIN: f32 = 0.64;
/// Length of the test kick, in frames at `FILE_RATE`.
const KICK_FRAMES: usize = BLOCK * 32;

fn booted_plugin(rate: f32) -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    assert!(plugin.initialize(rate, BLOCK as u32));
    render(&mut plugin, &[]);
    plugin
}

fn reactivate(plugin: &mut ResonanceDrums, rate: f32) {
    plugin.deactivate();
    assert!(plugin.initialize(rate, BLOCK as u32));
}

fn render(plugin: &mut ResonanceDrums, events: &[NoteEvent]) -> Vec<f32> {
    let mut buffers: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_PORTS)
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
    buffers.swap_remove(KICK_PORT).0
}

fn kick() -> [NoteEvent; 1] {
    [NoteEvent::NoteOn {
        note: drum_map::KICK,
        velocity: 1.0,
        timing: 0,
    }]
}

/// Strike the kick and return the level it holds late in the block, net
/// of the default gain chain. Earlier hits are silenced first (CLAP
/// `reset`), so only this one is measured.
fn kick_level(plugin: &mut ResonanceDrums) -> f32 {
    plugin.reset();
    render(plugin, &kick())[BLOCK - 1] / CHAIN_GAIN
}

/// Strike the kick and count the frames it sounds at (near) its level:
/// the decoded length of the take, which is what a decode at the wrong
/// rate gets wrong (and with it the pitch).
fn kick_frames(plugin: &mut ResonanceDrums, level: f32) -> usize {
    let threshold = 0.5 * level * CHAIN_GAIN;
    plugin.reset();
    let mut count = 0;
    let mut events: &[NoteEvent] = &kick();
    for _ in 0..(4 * KICK_FRAMES / BLOCK) {
        count += render(plugin, events)
            .iter()
            .filter(|s| s.abs() > threshold)
            .count();
        events = &[];
    }
    count
}

// ---------------------------------------------------------------------------
// Test kits
// ---------------------------------------------------------------------------

struct TempKit {
    dir: PathBuf,
    manifest: PathBuf,
    level: f32,
}

impl Drop for TempKit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 16-bit PCM stereo WAV of constant amplitude at `FILE_RATE`.
fn write_flat_wav(path: &Path, amplitude: f32, frames: usize) {
    let channels: u16 = 2;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = FILE_RATE as u32 * block_align as u32;
    let data_len = frames * block_align as usize;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&(FILE_RATE as u32).to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    let value = (amplitude * i16::MAX as f32).round() as i16;
    for _ in 0..frames * channels as usize {
        out.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, out).expect("write test wav");
}

fn temp_kit(level: f32) -> TempKit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-reactivate-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp kit dir");
    write_flat_wav(&dir.join("kick.wav"), level, KICK_FRAMES);
    let manifest = r#"{
  "SD Kick mit Teppich": {
    "01_KickIn_e901": {
      "brand": "test", "channel": "1", "mic": "e901", "position": "KickIn",
      "rounds": { "RR1": { "Vel01": "kick.wav" } }
    }
  }
}"#;
    let manifest_path = dir.join("drum_samples.json");
    std::fs::write(&manifest_path, manifest).expect("write test manifest");
    TempKit {
        dir,
        manifest: manifest_path,
        level,
    }
}

/// Start a load of `kit` at `rate` — what the editor's kit picker does.
fn pick(plugin: &ResonanceDrums, kit: &TempKit, rate: f32) {
    let choices: [PadMicChoices; NUM_PADS] = std::array::from_fn(|_| PadMicChoices::default());
    spawn_loader(
        kit.manifest.clone(),
        rate,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        choices,
        [false; NUM_PADS],
    );
}

/// Hold every loader started from now on before its decode; each message
/// sent on the returned channel lets one through.
fn gate(plugin: &ResonanceDrums) -> Sender<()> {
    let (tx, rx) = crossbeam_channel::unbounded();
    *plugin.bridge.decode_gate.lock() = Some(rx);
    tx
}

fn ungate(plugin: &ResonanceDrums) {
    *plugin.bridge.decode_gate.lock() = None;
}

/// Wait until `kit` has loaded and nothing is pending: its kit is in the
/// mailbox (or already taken).
fn wait_settled(plugin: &ResonanceDrums, kit: &TempKit) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let loaded = plugin.bridge.kit_path.lock().as_deref() == Some(kit.manifest.as_path());
        let pending = plugin.bridge.pending_kit.lock().is_some();
        if loaded && !pending {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "kit never settled: kit_path {:?}, pending {:?}, status {:?}",
            plugin.bridge.kit_path.lock(),
            plugin.bridge.pending_kit.lock(),
            plugin.bridge.kit_status.lock()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn load_and_wait(plugin: &ResonanceDrums, kit: &TempKit, rate: f32) {
    pick(plugin, kit, rate);
    wait_settled(plugin, kit);
}

fn expected_frames(rate: f32) -> f32 {
    KICK_FRAMES as f32 * rate / FILE_RATE
}

fn assert_decoded_at(frames: usize, rate: f32, what: &str) {
    let want = expected_frames(rate);
    assert!(
        (frames as f32 - want).abs() < 32.0,
        "{what}: the kick sounds for {frames} frames, want ≈ {want} (decoded at {rate} Hz); \
         at {FILE_RATE} Hz it would be {KICK_FRAMES}"
    );
}

// ---------------------------------------------------------------------------
// (a) (b) (c)
// ---------------------------------------------------------------------------

#[test]
fn a_first_pick_decoding_across_a_rate_change_is_reloaded_at_the_new_rate() {
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    let go = gate(&plugin);
    pick(&plugin, &x, FILE_RATE);
    reactivate(&mut plugin, OTHER_RATE);
    // Let both the stale 48 kHz decode and the reload through.
    go.send(()).unwrap();
    go.send(()).unwrap();
    wait_settled(&plugin, &x);
    assert!(matches!(
        *plugin.bridge.kit_status.lock(),
        KitStatus::Loaded { .. }
    ));
    let frames = kick_frames(&mut plugin, x.level);
    assert_decoded_at(frames, OTHER_RATE, "first pick across a 48k -> 44.1k re-activation");
}

#[test]
fn b_a_pick_decoding_across_a_reactivation_is_not_reverted_to_the_old_kit() {
    let w = temp_kit(0.25);
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    load_and_wait(&plugin, &w, FILE_RATE);
    assert!((kick_level(&mut plugin) - w.level).abs() < 1e-3);

    let go = gate(&plugin);
    pick(&plugin, &x, FILE_RATE);
    reactivate(&mut plugin, FILE_RATE);
    go.send(()).unwrap();
    go.send(()).unwrap();
    wait_settled(&plugin, &x);
    let level = kick_level(&mut plugin);
    assert!(
        (level - x.level).abs() < 1e-3,
        "the re-activation reverted the pick: kick plays at {level} (W = {}, X = {})",
        w.level,
        x.level
    );
}

#[test]
fn c_a_kit_handed_off_but_not_taken_is_not_installed_at_the_old_rate() {
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    // Decoded at 48 kHz and handed off; no `process()` takes it.
    load_and_wait(&plugin, &x, FILE_RATE);

    let go = gate(&plugin);
    reactivate(&mut plugin, OTHER_RATE);
    // The reload is held: the sampler must be on the built-in kit now,
    // not on the 48 kHz decode left in the mailbox.
    let level = kick_level(&mut plugin);
    assert!(
        (level - x.level).abs() > 0.05,
        "the 48 kHz kit left in the mailbox was installed after a 44.1 kHz re-activation"
    );
    go.send(()).unwrap();
    wait_settled(&plugin, &x);
    let frames = kick_frames(&mut plugin, x.level);
    assert_decoded_at(frames, OTHER_RATE, "reload after the mailbox was drained");
}

// ---------------------------------------------------------------------------
// E4: an unchanged rate reuses the kit
// ---------------------------------------------------------------------------

#[test]
fn reactivating_at_the_same_rate_reuses_the_installed_kit() {
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    load_and_wait(&plugin, &x, FILE_RATE);
    // Taken by the audio thread.
    assert!((kick_level(&mut plugin) - x.level).abs() < 1e-3);

    let generation = plugin.bridge.load_generation.load(Ordering::Acquire);
    // Any decode would hang here: nothing lets it through.
    let _go = gate(&plugin);
    reactivate(&mut plugin, FILE_RATE);
    assert_eq!(
        plugin.bridge.load_generation.load(Ordering::Acquire),
        generation,
        "an initialize at the same rate started a reload"
    );
    assert!(matches!(
        *plugin.bridge.kit_status.lock(),
        KitStatus::Loaded { .. }
    ));
    let level = kick_level(&mut plugin);
    assert!(
        (level - x.level).abs() < 1e-3,
        "the kit was not kept: kick plays at {level}"
    );
    ungate(&plugin);
}

#[test]
fn reactivating_at_the_same_rate_installs_a_kit_left_in_the_mailbox() {
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    // Handed off, never taken.
    load_and_wait(&plugin, &x, FILE_RATE);
    let generation = plugin.bridge.load_generation.load(Ordering::Acquire);
    let _go = gate(&plugin);
    reactivate(&mut plugin, FILE_RATE);
    assert_eq!(
        plugin.bridge.load_generation.load(Ordering::Acquire),
        generation,
        "a kit decoded at this very rate was decoded again"
    );
    let level = kick_level(&mut plugin);
    assert!((level - x.level).abs() < 1e-3, "kick plays at {level}");
    ungate(&plugin);
}

#[test]
fn reactivating_after_a_mic_change_decodes_again() {
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    load_and_wait(&plugin, &x, FILE_RATE);
    render(&mut plugin, &[]);
    plugin.bridge.overhead_setup_key.lock().push_str("-other");
    let generation = plugin.bridge.load_generation.load(Ordering::Acquire);
    reactivate(&mut plugin, FILE_RATE);
    assert_ne!(
        plugin.bridge.load_generation.load(Ordering::Acquire),
        generation,
        "the kit in memory was built with other mics; it must be reloaded"
    );
    wait_settled(&plugin, &x);
}

/// A mic or articulation change while a pick is decoding reloads the
/// pick, not the kit before it.
#[test]
fn a_reload_while_a_pick_decodes_reloads_the_pick() {
    let w = temp_kit(0.25);
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    load_and_wait(&plugin, &w, FILE_RATE);
    let go = gate(&plugin);
    pick(&plugin, &x, FILE_RATE);
    assert!(resonance_drums::reload::reload_kit(&plugin.bridge));
    go.send(()).unwrap();
    go.send(()).unwrap();
    wait_settled(&plugin, &x);
    let level = kick_level(&mut plugin);
    assert!((level - x.level).abs() < 1e-3, "kick plays at {level}");
}

/// A state load naming another kit supersedes a pick still decoding: the
/// pick must not land afterwards and overwrite the restored kit.
#[test]
fn a_state_load_supersedes_a_pick_still_decoding() {
    use resonance_plugin::plugin::ExtraStateSaver;

    let w = temp_kit(0.25);
    let x = temp_kit(0.5);
    let mut plugin = booted_plugin(FILE_RATE);
    let go = gate(&plugin);
    pick(&plugin, &x, FILE_RATE);

    let saver = resonance_drums::DrumsExtraState {
        kit_path: plugin.bridge.kit_path.clone(),
        overhead_setup_key: plugin.bridge.overhead_setup_key.clone(),
        pad_choices: plugin.bridge.pad_choices.clone(),
        params: plugin.bridge.params.clone(),
        reload: Some(plugin.bridge.clone()),
    };
    saver.load(&serde_json::json!({ "kit_path": w.manifest.to_string_lossy() }));
    go.send(()).unwrap();
    go.send(()).unwrap();
    wait_settled(&plugin, &w);
    // Give the superseded X loader every chance to land late.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        plugin.bridge.kit_path.lock().as_deref(),
        Some(w.manifest.as_path())
    );
    let level = kick_level(&mut plugin);
    assert!((level - w.level).abs() < 1e-3, "kick plays at {level}");
}
