//! Articulation is a parameter, and the parameter is what the sampler
//! plays (ba todo #1325).
//!
//! The acceptance test here writes `pad_0_articulation` exactly the way
//! `track.set_plugin_param` does — by string id, through
//! `Param::set_plain`, with no editor and no direct reload call — and
//! then measures what the sampler renders. Before #1325 that write moved
//! a number nothing read: the loader took its articulations from a
//! separate `KitBridge` array that only the editor chips wrote.
//!
//! The kit is synthesised in a temp dir so the test is hermetic: two
//! pieces for the kick, one recorded at half the level of the other, so
//! "which piece is loaded" is directly audible in the rendered block.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use resonance_drums::articulation::{
    ARTICULATION_ALT, ARTICULATION_LABELS, ARTICULATION_PRIMARY,
};
use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::kit_loader::KitStatus;
use resonance_drums::params::{DrumParams, OUTPUT_MODE_MULTI};
use resonance_drums::reload::reload_kit;
use resonance_drums::ResonanceDrums;
use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// Sample length in frames — shorter than the silence rendered between
/// strikes, so no two hits ever overlap.
const SAMPLE_FRAMES: usize = 256;
const KICK_PORT: usize = 1;
const NUM_PORTS: usize = 7;
/// Level the loaded piece is rendered at: pad volume (0 dB) × master (0 dB).
const CHAIN_GAIN: f32 = 1.0;

// ---------------------------------------------------------------------------
// Synthetic kit
// ---------------------------------------------------------------------------

/// A temp kit directory holding two kick pieces at different levels.
struct TempKit {
    dir: PathBuf,
    manifest: PathBuf,
}

impl Drop for TempKit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Write a 16-bit PCM stereo WAV of constant amplitude at the plugin's
/// sample rate, so nothing resamples and the decoded level is exact.
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
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    let value = (amplitude * i16::MAX as f32).round() as i16;
    for _ in 0..frames {
        for _ in 0..channels {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(path, out).expect("write test wav");
}

/// Build a kit whose kick has both articulations, the alternate one
/// recorded at `alt` and the primary at `primary`.
fn build_temp_kit(tag: &str, primary: f32, alt: f32) -> TempKit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-articulation-{}-{tag}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp kit dir");

    write_flat_wav(&dir.join("kick_primary.wav"), primary, SAMPLE_FRAMES);
    write_flat_wav(&dir.join("kick_alt.wav"), alt, SAMPLE_FRAMES);

    // Only the kick is described; every other pad is silent, which is
    // what a partial kit does in production too (D7). No `_meta`: the
    // pair comes from the Drummica table.
    let manifest = format!(
        r#"{{
  "SD Kick mit Teppich": {{
    "01_KickIn_e901": {{
      "brand": "test", "channel": "1", "mic": "e901", "position": "KickIn",
      "rounds": {{ "RR1": {{ "Vel01": "kick_primary.wav" }} }}
    }}
  }},
  "SD Kick ohne Teppich": {{
    "01_KickIn_e901": {{
      "brand": "test", "channel": "1", "mic": "e901", "position": "KickIn",
      "rounds": {{ "RR1": {{ "Vel01": "kick_alt.wav" }} }}
    }}
  }}
}}"#
    );
    let manifest_path = dir.join("drum_samples.json");
    std::fs::write(&manifest_path, manifest).expect("write test manifest");

    TempKit {
        dir,
        manifest: manifest_path,
    }
}

// ---------------------------------------------------------------------------
// Driving the plugin
// ---------------------------------------------------------------------------

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

/// Render silence for long enough that any previous hit has finished and
/// the per-block parameter ramps have settled. Also the point at which a
/// freshly loaded kit is swapped in, since that happens in `process`.
fn settle(plugin: &mut ResonanceDrums) {
    for _ in 0..8 {
        render(plugin, &[]);
    }
}

/// Hit the kick at full velocity and return the block's peak on the Kick
/// output port.
fn strike_kick(plugin: &mut ResonanceDrums) -> f32 {
    settle(plugin);
    let events = [NoteEvent::NoteOn {
        note: drum_map::KICK,
        velocity: 1.0,
        timing: 0,
    }];
    render(plugin, &events)[KICK_PORT]
        .iter()
        .fold(0.0_f32, |acc, s| acc.max(s.abs()))
}

/// Keep striking until the rendered level moves away from `previous`,
/// which is how the kit reload becomes observable. Fails the test rather
/// than hanging forever.
fn strike_until_level_changes(plugin: &mut ResonanceDrums, previous: f32) -> f32 {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let peak = strike_kick(plugin);
        if (peak - previous).abs() > 1e-3 {
            return peak;
        }
        assert!(
            Instant::now() < deadline,
            "the articulation parameter never reached the sampler: still rendering {previous}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Block until the loader has published a kit, so the first
/// measurement is of the kit under test rather than of the plugin's
/// embedded fallback.
fn wait_for_kit_loaded(plugin: &ResonanceDrums) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match &*plugin.bridge.kit_status.lock() {
            KitStatus::Loaded { .. } => return,
            KitStatus::Error { message } => panic!("test kit failed to load: {message}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "test kit never finished loading");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Write a parameter the way the control API's `set_plugin_param` and a
/// host automation lane do: look it up by its string id and go through
/// the `Param` trait. Nothing here knows it is talking to the drums.
fn set_param_by_id(plugin: &ResonanceDrums, id: &str, value: f64) {
    for index in 0..plugin.param_count() {
        let param = plugin.param(index);
        if param.id() == id {
            param.set_plain(value);
            return;
        }
    }
    panic!("no parameter with id {id}");
}

// ---------------------------------------------------------------------------
// The acceptance test
// ---------------------------------------------------------------------------

/// Writing `pad_0_articulation` through the `Param` interface — no
/// editor, no explicit reload — changes what the kick renders.
#[test]
fn a_param_write_changes_what_the_sampler_plays() {
    let kit = build_temp_kit("param-write", 0.5, 0.25);

    let mut plugin = ResonanceDrums::new();
    // Multi output (E11): this test reads the kick's own port.
    plugin.bridge.params.output_mode.set_value(OUTPUT_MODE_MULTI);
    assert!(plugin.initialize(SAMPLE_RATE, BLOCK as u32));

    // Load the kit at its default articulation, the way the editor's
    // kit picker does. This part is not what the test is about.
    *plugin.bridge.kit_path.lock() = Some(kit.manifest.clone());
    assert!(reload_kit(&plugin.bridge), "the test kit should load");
    wait_for_kit_loaded(&plugin);

    let primary = strike_kick(&mut plugin);
    assert!(
        (primary - 0.5 * CHAIN_GAIN).abs() < 0.01,
        "expected the primary kick piece at {}, got {primary}",
        0.5 * CHAIN_GAIN
    );

    // The whole point: a plain parameter write, and nothing else.
    set_param_by_id(&plugin, "pad_0_articulation", ARTICULATION_ALT as f64);

    let alternate = strike_until_level_changes(&mut plugin, primary);
    assert!(
        (alternate - 0.25 * CHAIN_GAIN).abs() < 0.01,
        "expected the alternate kick piece at {}, got {alternate}",
        0.25 * CHAIN_GAIN
    );

    // …and it goes back, so this is the parameter tracking rather than a
    // one-way switch.
    set_param_by_id(&plugin, "pad_0_articulation", ARTICULATION_PRIMARY as f64);
    let back = strike_until_level_changes(&mut plugin, alternate);
    assert!(
        (back - primary).abs() < 0.01,
        "returning the parameter should return the sound: {back} vs {primary}"
    );
}

// ---------------------------------------------------------------------------
// One source of truth
// ---------------------------------------------------------------------------

/// The articulation set the loader is built from is derived from the
/// params — there is no second array to keep in step.
#[test]
fn the_bridge_derives_articulations_from_the_params() {
    let plugin = ResonanceDrums::new();
    assert_eq!(plugin.bridge.articulations(), [false; NUM_PADS]);

    set_param_by_id(&plugin, "pad_1_articulation", ARTICULATION_ALT as f64);

    let mut expected = [false; NUM_PADS];
    expected[1] = true;
    assert_eq!(
        plugin.bridge.articulations(),
        expected,
        "the bridge must read the parameter, not a mirror of it"
    );
}

/// The value the host and the control API see is the value the plugin
/// acts on, in both directions.
#[test]
fn articulation_reads_back_through_the_param_interface() {
    let plugin = ResonanceDrums::new();
    set_param_by_id(&plugin, "pad_0_articulation", ARTICULATION_ALT as f64);

    let param = (0..plugin.param_count())
        .map(|i| plugin.param(i))
        .find(|p| p.id() == "pad_0_articulation")
        .expect("articulation param");
    assert_eq!(param.get_plain(), ARTICULATION_ALT as f64);
    assert!(plugin.bridge.articulations()[0]);
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/// The choice reads as words everywhere a parameter is rendered — the
/// host's automation lane, a typed-in value, the control API — not as a
/// bare 0 or 1.
#[test]
fn articulation_params_display_and_parse_their_labels() {
    let params = DrumParams::default();
    let param = &params.pads[0].articulation;

    // Without a kit the values read generically (the kit's own labels are
    // `pad_map.rs`'s, tested in `tests/kit_pads.rs`).
    assert_eq!(param.display(ARTICULATION_PRIMARY as f64), "Primary");
    assert_eq!(param.display(ARTICULATION_ALT as f64), "Alternate");
    assert_eq!(param.labels(), ARTICULATION_LABELS);

    // Parsing takes the label (case-insensitively) or the raw index, so
    // automation written against the number keeps working.
    assert_eq!(param.parse("Alternate"), Some(1.0));
    assert_eq!(param.parse("ALTERNATE"), Some(1.0));
    assert_eq!(param.parse("primary"), Some(0.0));
    assert_eq!(param.parse("1"), Some(1.0));
    assert_eq!(param.parse("nonsense"), None);

    // Round trip: what the host shows is what the host can type back.
    for value in 0..=1 {
        let text = param.display(value as f64);
        assert_eq!(param.parse(&text), Some(value as f64));
    }
}

/// Every pad offers the control, whatever the kit: which pads a kit pairs
/// is the kit's (an IT Techno kit pairs its kick and snare, a Drummica kit
/// its kick, snare and toms), and a hidden parameter does not exist for
/// the host at all — no lane, no `set_plugin_param` — so hiding it by a
/// static table would take the control away from kits that have it.
#[test]
fn every_pad_exposes_the_choice() {
    let params = DrumParams::default();
    for (index, pad) in params.pads.iter().enumerate() {
        assert!(!pad.articulation.is_hidden(), "pad {index} is hidden");
        assert!(pad.articulation.is_automatable(), "pad {index}");
        assert_eq!(pad.articulation.id(), format!("pad_{index}_articulation"));
    }
}

/// Defaults are unchanged: every pad boots on its primary piece, which
/// is the same kit the plugin loaded before articulation became a real
/// parameter.
#[test]
fn every_pad_defaults_to_its_primary_articulation() {
    let params = DrumParams::default();
    assert_eq!(params.articulations(), [false; NUM_PADS]);
    for pad in params.pads.iter() {
        assert_eq!(pad.articulation.default_plain(), ARTICULATION_PRIMARY as f64);
        assert_eq!(pad.articulation.get_plain(), ARTICULATION_PRIMARY as f64);
    }
}
