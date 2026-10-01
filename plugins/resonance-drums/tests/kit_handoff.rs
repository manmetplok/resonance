//! Latest-wins kit hand-off (drums-plugin-rework.md §7 E3).
//!
//! The loader hands a decoded kit to the audio thread through a one-slot
//! channel. It used to `try_send` and ignore the result, so when an older
//! kit was still waiting in the slot the *newer* one was dropped — while
//! `kit_path` and the status went on to claim the newer kit had loaded.
//! Two loads in a row with no `process()` between them (a host that is
//! not rolling, or two quick clicks in the editor) played the first kit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::kit_loader::{
    hand_off_kit, spawn_loader, PadMicChoices, DEFAULT_OVERHEAD_SETUP,
};
use resonance_drums::ResonanceDrums;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
const NUM_PORTS: usize = 7;
const KICK_PORT: usize = 1;
/// Pad volume (0.8) × master (0.8) at the defaults.
const CHAIN_GAIN: f32 = 0.64;

fn booted_plugin() -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    assert!(plugin.initialize(SAMPLE_RATE, BLOCK as u32));
    // One block so the master/pad ramps start from the real values.
    render(&mut plugin, &[]);
    plugin
}

fn render(plugin: &mut ResonanceDrums, events: &[NoteEvent]) -> Vec<Vec<f32>> {
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
    buffers.into_iter().map(|(l, _)| l).collect()
}

/// Strike the kick and return the level it plays at, net of the default
/// gain chain — i.e. the level of the sample in the kit that is playing.
fn kick_level(plugin: &mut ResonanceDrums) -> f32 {
    let hit = [NoteEvent::NoteOn {
        note: drum_map::KICK,
        velocity: 1.0,
        timing: 0,
    }];
    let out = render(plugin, &hit);
    // Late in the block, clear of the swap and the onset.
    out[KICK_PORT][BLOCK - 1] / CHAIN_GAIN
}

/// A kit whose kick is flat DC at `level` and whose every other pad is
/// silent.
fn dc_kit(level: f32) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| LoadedPad {
            name: m.name.to_string(),
            choke_group: None,
            output_group: m.output_group,
            close_mics: if i == 0 {
                vec![LoadedMicBank {
                    position: "test".to_string(),
                    setup_key: String::new(),
                    layers: vec![VelocityLayer {
                        round_robins: vec![LoadedSample {
                            data: vec![level; BLOCK * 64],
                            frames: BLOCK * 32,
                        }],
                    }],
                }]
            } else {
                Vec::new()
            },
            overhead: None,
        })
        .collect()
}

#[test]
fn second_of_two_queued_kits_is_the_one_that_plays() {
    let mut plugin = booted_plugin();
    // Two kits handed over before the audio thread runs: the slot is
    // full when the second arrives.
    hand_off_kit(&plugin.bridge, dc_kit(0.25));
    hand_off_kit(&plugin.bridge, dc_kit(0.5));
    let level = kick_level(&mut plugin);
    assert!(
        (level - 0.5).abs() < 1e-3,
        "the newer kit must win the slot: kick plays at {level}, want 0.5"
    );
    // And the stale kit is gone, not merely queued behind it: had it
    // been swapped in now, the ringing kick would start fading.
    let again = render(&mut plugin, &[])[KICK_PORT][BLOCK - 1] / CHAIN_GAIN;
    assert!(
        (again - 0.5).abs() < 1e-3,
        "a stale kit swapped in later: {again}"
    );
}

// ---------------------------------------------------------------------------
// Through the real loader
// ---------------------------------------------------------------------------

struct TempKit {
    dir: PathBuf,
    manifest: PathBuf,
}

impl Drop for TempKit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 16-bit PCM stereo WAV of constant amplitude at the plugin's rate.
fn write_flat_wav(path: &Path, amplitude: f32, frames: usize) {
    let channels: u16 = 2;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = SAMPLE_RATE as u32 * block_align as u32;
    let data_len = frames * block_align as usize;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
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
        "resonance-drums-handoff-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp kit dir");
    write_flat_wav(&dir.join("kick.wav"), level, BLOCK * 32);
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
    }
}

fn load(plugin: &ResonanceDrums, kit: &TempKit) {
    let choices: [PadMicChoices; NUM_PADS] = std::array::from_fn(|_| PadMicChoices::default());
    spawn_loader(
        kit.manifest.clone(),
        SAMPLE_RATE,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        choices,
        [false; NUM_PADS],
    );
    // `kit_path` is written after the hand-off, so once it names this
    // kit the kit is in the slot.
    let deadline = Instant::now() + Duration::from_secs(30);
    while plugin.bridge.kit_path.lock().as_deref() != Some(kit.manifest.as_path()) {
        assert!(
            Instant::now() < deadline,
            "the loader never handed the kit over"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn two_back_to_back_loads_play_the_second_kit() {
    let first = temp_kit(0.25);
    let second = temp_kit(0.5);
    let mut plugin = booted_plugin();
    // No `process()` between the loads: the first kit is still sitting
    // in the slot when the second load finishes.
    load(&plugin, &first);
    load(&plugin, &second);
    let level = kick_level(&mut plugin);
    // 16-bit quantisation of 0.5 is well inside this.
    assert!(
        (level - 0.5).abs() < 1e-3,
        "status says the second kit loaded, but the kick plays at {level} (first kit = 0.25)"
    );
}
