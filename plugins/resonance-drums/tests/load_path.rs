//! The kit load path (drums-plugin-rework.md §7 E4, E5, E6, §5.4):
//! incremental reloads, the shared sample cache, mono takes, partial
//! kits and load progress.
//!
//! Every test writes its own kit into its own temp directory: the sample
//! cache is process-wide, and two tests on one directory would share
//! takes and break each other's decode counts.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_drums::articulation::{self, ARTICULATION_ALT};
use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::dsp::{DrumSampler, Hit, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::kit_loader::{
    spawn_loader, KitStatus, LoadPhase, LoadStats, PadMicChoices, DEFAULT_OVERHEAD_SETUP,
};
use resonance_drums::params::DrumParams;
use resonance_drums::reload::reload_kit;
use resonance_drums::{DrumsExtraState, ResonanceDrums};
use resonance_plugin::plugin::ExtraStateSaver;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// Frames in every fixture take.
const TAKE_FRAMES: usize = 2_400;

// ---------------------------------------------------------------------------
// Fixture kit
// ---------------------------------------------------------------------------

/// 16-bit PCM WAV at `RATE` with `channels` channels holding `level`
/// (the right channel at `-level`, so stereo is visibly stereo).
fn write_wav(path: &Path, channels: u16, level: f32) {
    let block_align = channels * 2;
    let data_len = TAKE_FRAMES * block_align as usize;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
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

/// Not a WAV at all.
fn write_corrupt(path: &Path) {
    std::fs::write(path, b"RIFF\x10\x00\x00\x00WAVEjunkjunkjunk").expect("write corrupt file");
}

/// Which fixture files are broken.
#[derive(Clone, Copy, PartialEq)]
enum Damage {
    None,
    /// One snare take.
    OneSnareTake,
    /// Every closed-hat file.
    WholeHat,
    /// Both KickIn takes of the soft layer (Vel01).
    KickInSoftLayer,
    /// Every kick overhead file.
    WholeKickOh,
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

/// Kick (two KickIn setups, KickOut, OH; plus its ohne-Teppich piece),
/// snare (SNTop with two takes, OH) and closed hat (Hat with two takes,
/// OH). Close mics are mono, overheads stereo. 14 files for the default
/// setup: kick 2×2 + 2 + 2, snare 2 + 1, hat 2 + 1.
fn fixture_kit(damage: Damage) -> Kit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-load-path-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut level = 0.05f32;
    let mut wav = |name: &str, channels: u16| {
        level += 0.01;
        write_wav(&dir.join(name), channels, level);
    };
    for f in [
        "kin_1_1", "kin_1_2", "kin_2_1", "kin_2_2", "kinalt_1", "kinalt_2",
    ] {
        wav(&format!("{f}.wav"), 1);
    }
    wav("kout_1.wav", 1);
    wav("kout_2.wav", 1);
    wav("koh_1.wav", 2);
    wav("koh_2.wav", 2);
    wav("ohne_in.wav", 1);
    wav("ohne_oh.wav", 2);
    wav("sn_1.wav", 1);
    wav("sn_2.wav", 1);
    wav("snoh.wav", 2);
    wav("hat_1.wav", 1);
    wav("hat_2.wav", 1);
    wav("hatoh.wav", 2);
    match damage {
        Damage::None => {}
        Damage::OneSnareTake => write_corrupt(&dir.join("sn_2.wav")),
        Damage::WholeHat => {
            write_corrupt(&dir.join("hat_1.wav"));
            // Missing outright, not just corrupt.
            std::fs::remove_file(dir.join("hat_2.wav")).unwrap();
            write_corrupt(&dir.join("hatoh.wav"));
        }
        Damage::KickInSoftLayer => {
            write_corrupt(&dir.join("kin_1_1.wav"));
            write_corrupt(&dir.join("kin_2_1.wav"));
        }
        Damage::WholeKickOh => {
            write_corrupt(&dir.join("koh_1.wav"));
            write_corrupt(&dir.join("koh_2.wav"));
        }
    }
    let setup = |pos: &str, rounds: &str| {
        format!(r#"{{"brand":"t","channel":"1","mic":"m","position":"{pos}","rounds":{rounds}}}"#)
    };
    let manifest = format!(
        r#"{{
  "_meta": {{"name": "fixture"}},
  "SD Kick mit Teppich": {{
    "01_KickIn_e901": {kin},
    "02_KickIn_alt": {kinalt},
    "03_KickOut": {kout},
    "23_OHsAB_e914": {koh}
  }},
  "SD Kick ohne Teppich": {{
    "01_KickIn_e901": {ohne_in},
    "23_OHsAB_e914": {ohne_oh}
  }},
  "SD Snare Normal": {{
    "04_SNTop": {sn},
    "23_OHsAB_e914": {snoh}
  }},
  "SD Hat Closed": {{
    "06_Hat": {hat},
    "23_OHsAB_e914": {hatoh}
  }}
}}"#,
        kin = setup(
            "KickIn",
            r#"{"RR1":{"Vel01":"kin_1_1.wav","Vel02":"kin_1_2.wav"},"RR2":{"Vel01":"kin_2_1.wav","Vel02":"kin_2_2.wav"}}"#
        ),
        kinalt = setup(
            "KickIn",
            r#"{"RR1":{"Vel01":"kinalt_1.wav","Vel02":"kinalt_2.wav"}}"#
        ),
        kout = setup(
            "KickOut",
            r#"{"RR1":{"Vel01":"kout_1.wav","Vel02":"kout_2.wav"}}"#
        ),
        koh = setup(
            "OHsAB",
            r#"{"RR1":{"Vel01":"koh_1.wav","Vel02":"koh_2.wav"}}"#
        ),
        ohne_in = setup("KickIn", r#"{"RR1":{"Vel01":"ohne_in.wav"}}"#),
        ohne_oh = setup("OHsAB", r#"{"RR1":{"Vel01":"ohne_oh.wav"}}"#),
        sn = setup(
            "SNTop",
            r#"{"RR1":{"Vel01":"sn_1.wav"},"RR2":{"Vel01":"sn_2.wav"}}"#
        ),
        snoh = setup("OHsAB", r#"{"RR1":{"Vel01":"snoh.wav"}}"#),
        hat = setup(
            "Hat",
            r#"{"RR1":{"Vel01":"hat_1.wav"},"RR2":{"Vel01":"hat_2.wav"}}"#
        ),
        hatoh = setup("OHsAB", r#"{"RR1":{"Vel01":"hatoh.wav"}}"#),
    );
    let manifest_path = dir.join("drum_samples.json");
    std::fs::write(&manifest_path, manifest).expect("write fixture manifest");
    Kit {
        dir,
        manifest: manifest_path,
    }
}

// ---------------------------------------------------------------------------
// Plugin helpers
// ---------------------------------------------------------------------------

fn booted() -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    // Multi output (E11): these tests read the Overhead port.
    plugin
        .bridge
        .params
        .output_mode
        .set_value(resonance_drums::params::OUTPUT_MODE_MULTI);
    assert!(plugin.initialize(RATE, BLOCK as u32));
    plugin
}

/// What the editor's kit picker does.
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

/// Wait for the load in flight to finish, and return what it did.
fn settle(plugin: &ResonanceDrums) -> LoadStats {
    let bridge = &plugin.bridge;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let pending = bridge.pending_kit.lock().is_some();
        let status = bridge.kit_status.lock().clone();
        match status {
            KitStatus::Loaded { .. } if !pending => return bridge.load_stats.lock().clone(),
            KitStatus::Error { message } if !pending => panic!("kit failed to load: {message}"),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "load never settled: pending {pending}, status {:?}",
            bridge.kit_status.lock()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Render one block with `events`; returns every port as (left, right).
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

fn hit(note: u8) -> [NoteEvent; 1] {
    [NoteEvent::NoteOn {
        note,
        velocity: 1.0,
        timing: 0,
    }]
}

/// Peak over every port of a block that strikes `note` (after silencing
/// whatever was still sounding).
fn strike_peak(plugin: &mut ResonanceDrums, note: u8) -> f32 {
    plugin.reset();
    render(plugin, &hit(note))
        .iter()
        .flat_map(|(l, r)| l.iter().chain(r))
        .fold(0.0f32, |m, s| m.max(s.abs()))
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

// ---------------------------------------------------------------------------
// E4: incremental reload
// ---------------------------------------------------------------------------

#[test]
fn changing_one_pads_close_mic_decodes_only_that_pads_files() {
    let kit = fixture_kit(Damage::None);
    let plugin = booted();
    pick(&plugin, &kit);
    let first = settle(&plugin);
    assert_eq!(first.files, 14, "{first:?}");
    assert_eq!(
        first.decoded, 14,
        "a fresh kit decodes every file: {first:?}"
    );
    assert_eq!(first.reused_pads, 0);
    let snare_before = built_pad(&plugin, 1);

    // The kick's KickIn moves to the other setup.
    plugin.bridge.pad_choices.lock()[0]
        .close_setups
        .insert("KickIn".to_string(), "02_KickIn_alt".to_string());
    assert!(reload_kit(&plugin.bridge));
    let second = settle(&plugin);
    assert_eq!(
        second.rebuilt_pads, 1,
        "only the kick is rebuilt: {second:?}"
    );
    assert_eq!(second.reused_pads, NUM_PADS - 1);
    // Kick: KickIn alt (2) + KickOut (2) + OH (2). Only the two alt files
    // are new; the other four are the takes the previous kit holds.
    assert_eq!(second.files, 6, "{second:?}");
    assert_eq!(second.decoded, 2, "{second:?}");
    assert_eq!(second.cached, 4, "{second:?}");
    // (The kit-wide figure also counts built-in pads other tests may
    // hold; the kick's own is what this load decided.)
    let kick_build = plugin.bridge.built_kit.lock().as_ref().unwrap().pad_builds[0].clone();
    assert_eq!(
        kick_build.shared_bytes, 0,
        "the instance's own takes are not 'shared'"
    );

    let kick = built_pad(&plugin, 0);
    assert_eq!(kick.close_mics[0].setup_key, "02_KickIn_alt");
    // The untouched snare is the very same takes.
    let snare_after = built_pad(&plugin, 1);
    assert!(Arc::ptr_eq(
        snare_before.close_mics[0].layers[0].round_robins[0].shared(),
        snare_after.close_mics[0].layers[0].round_robins[0].shared(),
    ));
}

/// A reload that leaves the snare alone still reports its unreadable
/// take — and since a partial pad is never reused, it tries the take
/// again, and picks it up once the file is readable.
#[test]
fn a_reload_keeps_reporting_and_retries_a_partial_pad() {
    let kit = fixture_kit(Damage::OneSnareTake);
    let plugin = booted();
    pick(&plugin, &kit);
    assert_eq!(settle(&plugin).unreadable, 1);

    // A kick mic change, the snare untouched and its file still broken.
    plugin.bridge.pad_choices.lock()[0]
        .close_setups
        .insert("KickIn".to_string(), "02_KickIn_alt".to_string());
    assert!(reload_kit(&plugin.bridge));
    let second = settle(&plugin);
    assert_eq!(second.unreadable, 1, "the reload forgot the snare: {second:?}");
    assert_eq!(second.unreadable_paths, vec![kit.dir.join("sn_2.wav")]);
    assert_eq!(second.rebuilt_pads, 2, "kick and the partial snare: {second:?}");
    match &*plugin.bridge.kit_status.lock() {
        KitStatus::Loaded { unreadable, .. } => assert_eq!(*unreadable, 1),
        other => panic!("status {other:?}"),
    }

    // The file comes good; the next reload of anything picks it up.
    write_wav(&kit.dir.join("sn_2.wav"), 1, 0.4);
    plugin.bridge.pad_choices.lock()[0]
        .close_setups
        .insert("KickIn".to_string(), "01_KickIn_e901".to_string());
    assert!(reload_kit(&plugin.bridge));
    let third = settle(&plugin);
    assert_eq!(third.unreadable, 0, "{third:?}");
    assert_eq!(built_pad(&plugin, 1).close_mics[0].layers[0].round_robins.len(), 2);
    let status = plugin.bridge.kit_status.lock().clone();
    match status {
        KitStatus::Loaded { unreadable, .. } => assert_eq!(unreadable, 0),
        other => panic!("status {other:?}"),
    }
}

/// The overhead pick only touches pads that have an overhead to pick:
/// the built-in pads filling the pieces the kit lacks are kept.
#[test]
fn an_overhead_change_rebuilds_only_pads_with_an_overhead() {
    let kit = fixture_kit(Damage::None);
    let plugin = booted();
    pick(&plugin, &kit);
    settle(&plugin);
    plugin
        .bridge
        .overhead_setup_key
        .lock()
        .push_str("-other");
    assert!(reload_kit(&plugin.bridge));
    let stats = settle(&plugin);
    // Kick, snare and hat have an OH setup; nothing else in the fixture does.
    assert_eq!(stats.rebuilt_pads, 3, "{stats:?}");
    assert_eq!(stats.reused_pads, NUM_PADS - 3);
}

#[test]
fn an_articulation_change_decodes_only_that_pads_files() {
    let kit = fixture_kit(Damage::None);
    let plugin = booted();
    pick(&plugin, &kit);
    settle(&plugin);

    plugin.bridge.params.pads[0]
        .articulation
        .set_value(ARTICULATION_ALT);
    // The watcher may get there first; either way one load runs.
    articulation::apply_pending(&plugin.bridge);
    let stats = settle(&plugin);
    assert_eq!(stats.rebuilt_pads, 1, "{stats:?}");
    // SD Kick ohne Teppich: one KickIn file and one OH file.
    assert_eq!(stats.files, 2, "{stats:?}");
    assert_eq!(stats.decoded, 2, "{stats:?}");
    assert_eq!(
        built_pad(&plugin, 0).close_mics.len(),
        1,
        "ohne has KickIn only"
    );
}

// ---------------------------------------------------------------------------
// E5: shared cache, mono
// ---------------------------------------------------------------------------

#[test]
fn a_second_instance_on_the_same_kit_decodes_nothing_and_shares_the_takes() {
    let kit = fixture_kit(Damage::None);
    let a = booted();
    pick(&a, &kit);
    let first = settle(&a);
    assert_eq!(first.decoded, 14);

    let b = booted();
    pick(&b, &kit);
    let second = settle(&b);
    assert_eq!(second.decoded, 0, "the second instance decoded: {second:?}");
    assert_eq!(second.cached, 14);
    // Every byte of the kit is shared: its files, and the built-in pads
    // filling the pieces the fixture lacks, which A holds too.
    assert_eq!(second.shared_bytes, second.kit_bytes, "{second:?}");
    assert_eq!(
        b.bridge.kit_shared_bytes.load(Ordering::Relaxed),
        second.kit_bytes
    );
    // The first instance shares none of the kit's files. (Its built-in
    // pads may be shared with whatever other test holds them.)
    let a_builds = a.bridge.built_kit.lock().as_ref().unwrap().pad_builds.clone();
    for (pad, build) in a_builds.iter().enumerate().take(3) {
        assert_eq!(build.shared_bytes, 0, "pad {pad} of the first instance");
    }

    for pad in [0, 1, 2] {
        let (pa, pb) = (built_pad(&a, pad), built_pad(&b, pad));
        for (ba, bb) in pa.close_mics.iter().zip(&pb.close_mics) {
            for (la, lb) in ba.layers.iter().zip(&bb.layers) {
                for (ta, tb) in la.round_robins.iter().zip(&lb.round_robins) {
                    assert!(
                        Arc::ptr_eq(ta.shared(), tb.shared()),
                        "pad {pad} not shared"
                    );
                }
            }
        }
    }

    // A reload in B rebuilds the kick only, but the figure is still the
    // whole kit's: the reused snare and hat keep theirs, and the kick's
    // KickOut / OH takes B already held stay shared. Only the new KickIn
    // files, which A does not hold, are B's alone.
    b.bridge.pad_choices.lock()[0]
        .close_setups
        .insert("KickIn".to_string(), "02_KickIn_alt".to_string());
    assert!(reload_kit(&b.bridge));
    let third = settle(&b);
    assert_eq!(third.reused_pads, NUM_PADS - 1);
    let kick_in_alt = resonance_drums::sample_info::total_sample_bytes(&[LoadedPad {
        extra_banks: Vec::new(),
        overhead: None,
        close_mics: vec![built_pad(&b, 0).close_mics[0].clone()],
        ..built_pad(&b, 0)
    }]) as u64;
    assert_eq!(
        third.shared_bytes,
        third.kit_bytes - kick_in_alt,
        "{third:?}"
    );
    assert_eq!(
        b.bridge.kit_shared_bytes.load(Ordering::Relaxed),
        third.shared_bytes
    );
}

/// The built-in kit is shared memory too: a second instance booting
/// while another holds it reports all of it shared, not 0.
#[test]
fn a_second_instance_on_the_built_in_kit_reports_it_shared() {
    let a = booted();
    let b = booted();
    let kit_bytes = b.bridge.kit_bytes.load(Ordering::Relaxed);
    assert!(kit_bytes > 0);
    assert_eq!(b.bridge.kit_shared_bytes.load(Ordering::Relaxed), kit_bytes);
    drop(a);
}

#[test]
fn a_mono_file_stays_mono_and_a_stereo_one_stereo() {
    let kit = fixture_kit(Damage::None);
    let plugin = booted();
    pick(&plugin, &kit);
    settle(&plugin);
    let kick = built_pad(&plugin, 0);
    let close = &kick.close_mics[0].layers[0].round_robins[0];
    assert_eq!(close.channels(), 1);
    assert_eq!(close.frames(), TAKE_FRAMES);
    assert_eq!(
        close.bytes(),
        TAKE_FRAMES * 4,
        "frames × 1 channel × 4 bytes"
    );
    let oh = &kick.overhead.as_ref().unwrap().layers[0].round_robins[0];
    assert_eq!(oh.channels(), 2);
    assert_eq!(oh.bytes(), TAKE_FRAMES * 2 * 4);
}

/// A pad on every port with one take: either `mono` itself, or the same
/// samples duplicated to stereo the way the old decoder stored them.
fn one_take_kit(mono: &[f32], as_mono: bool) -> Vec<LoadedPad> {
    let take = || {
        if as_mono {
            LoadedSample::mono(mono.to_vec())
        } else {
            LoadedSample::from_data(mono.iter().flat_map(|&s| [s, s]).collect())
        }
    };
    drum_map::PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "x".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![take()])],
            }],
            extra_banks: Vec::new(),
            overhead: Some(LoadedMicBank {
                position: "OH".to_string(),
                setup_key: String::new(),
                layers: vec![VelocityLayer::new(vec![take()])],
            }),
        })
        .collect()
}

fn render_bits(pads: Vec<LoadedPad>) -> Vec<u32> {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(RATE);
    sampler.pads = pads;
    let params = DrumParams::default();
    // Off-centre pans and unequal in/out trims, so left and right differ.
    params.pads[0].pan.set_value(-0.6);
    params.pads[1].pan.set_value(0.4);
    params.pads[0].trims[0].set_value(-3.0);
    params.pads[0].trims[1].set_value(-10.5);
    let mut out = Vec::new();
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    for block in 0..12 {
        let hits: Vec<Hit> = match block {
            0 => vec![
                Hit {
                    frame: 0,
                    note: drum_map::KICK,
                    velocity: 0.9,
                },
                Hit {
                    frame: 17,
                    note: drum_map::SNARE,
                    velocity: 0.5,
                },
            ],
            3 => vec![Hit {
                frame: 64,
                note: drum_map::HIHAT_CLOSED,
                velocity: 0.7,
            }],
            5 => vec![Hit {
                frame: 5,
                note: drum_map::HIHAT_OPEN,
                velocity: 1.0,
            }],
            8 => vec![Hit {
                frame: 99,
                note: drum_map::HIHAT_PEDAL,
                velocity: 0.8,
            }],
            _ => Vec::new(),
        };
        {
            let mut it = bufs.iter_mut();
            let mut ports: [PortBuffers<'_>; NUM_OUTPUT_PORTS] = std::array::from_fn(|_| {
                let (l, r) = it.next().unwrap();
                PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                }
            });
            sampler.render_block(&mut ports, BLOCK, &params, &hits);
        }
        for (l, r) in &bufs {
            out.extend(l.iter().chain(r).map(|s| s.to_bits()));
        }
    }
    out
}

#[test]
fn a_mono_take_renders_bit_identical_to_its_duplicated_stereo_take() {
    let mono: Vec<f32> = (0..4 * BLOCK)
        .map(|i| ((i as f32 * 0.031).sin() * 0.7) * (1.0 - i as f32 / (4 * BLOCK) as f32))
        .collect();
    let a = render_bits(one_take_kit(&mono, true));
    let b = render_bits(one_take_kit(&mono, false));
    assert!(
        a.iter().any(|&bits| f32::from_bits(bits) != 0.0),
        "the render is silent: the comparison proves nothing"
    );
    assert_eq!(a.len(), b.len());
    let first_diff = a.iter().zip(&b).position(|(x, y)| x != y);
    assert_eq!(
        first_diff, None,
        "mono and duplicated-stereo renders differ"
    );
}

// ---------------------------------------------------------------------------
// E6: partial kits
// ---------------------------------------------------------------------------

#[test]
fn a_corrupt_take_is_counted_and_the_kit_still_loads_and_plays() {
    let kit = fixture_kit(Damage::OneSnareTake);
    let mut plugin = booted();
    pick(&plugin, &kit);
    let stats = settle(&plugin);
    assert_eq!(stats.unreadable, 1, "{stats:?}");
    assert_eq!(stats.unreadable_paths, vec![kit.dir.join("sn_2.wav")]);
    match &*plugin.bridge.kit_status.lock() {
        KitStatus::Loaded {
            unreadable,
            unreadable_paths,
            ..
        } => {
            assert_eq!(*unreadable, 1);
            assert_eq!(unreadable_paths.len(), 1);
        }
        other => panic!("status {other:?}"),
    }
    // The snare keeps its readable take; nothing else is touched.
    let snare = built_pad(&plugin, 1);
    assert_eq!(snare.close_mics[0].layers[0].round_robins.len(), 1);

    render(&mut plugin, &[]); // the audio thread takes the kit
    assert!(
        strike_peak(&mut plugin, drum_map::KICK) > 0.01,
        "kick silent"
    );
    assert!(
        strike_peak(&mut plugin, drum_map::SNARE) > 0.01,
        "snare silent"
    );
    assert!(
        strike_peak(&mut plugin, drum_map::HIHAT_CLOSED) > 0.01,
        "hat silent"
    );
}

#[test]
fn a_pad_whose_every_file_fails_is_silent_not_the_built_in_sample() {
    let kit = fixture_kit(Damage::WholeHat);
    let mut plugin = booted();
    pick(&plugin, &kit);
    let stats = settle(&plugin);
    assert_eq!(stats.unreadable, 3, "{stats:?}");
    let hat = built_pad(&plugin, 2);
    assert!(
        hat.close_mics.is_empty() && hat.overhead.is_none(),
        "hat has banks"
    );

    render(&mut plugin, &[]);
    assert!(
        strike_peak(&mut plugin, drum_map::KICK) > 0.01,
        "kick silent"
    );
    assert_eq!(
        strike_peak(&mut plugin, drum_map::HIHAT_CLOSED),
        0.0,
        "the broken hat must be silent, not the built-in fallback"
    );
}

/// The take a kit holds for `file` of `kit`, as the shared cache has it.
fn take_of(kit: &Kit, file: &str) -> Arc<resonance_drums::kit::SampleData> {
    use resonance_drums::kit_loader::cache::{self, SampleKey};
    let key = SampleKey::for_file_preload(
        &kit.dir.join(file),
        RATE,
        resonance_drums::stream::DEFAULT_PRELOAD,
    )
    .unwrap();
    cache::global().lookup(&key).expect("a take some kit holds")
}

/// A sampler playing `pads`, outside any plugin, so a test can look at
/// its voices. The sender keeps the mailbox connected.
fn sampler_on(pads: Vec<LoadedPad>) -> (DrumSampler, crossbeam_channel::Sender<Vec<LoadedPad>>) {
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(RATE);
    sampler.pads = pads;
    (sampler, tx)
}

/// Strike `note` and render one block. Returns, for each voice the hit
/// started that is still playing after the block, the take it reads.
fn strike_takes(sampler: &mut DrumSampler, note: u8, velocity: f32) -> Vec<*const ()> {
    use resonance_drums::voice::VoiceDestination;
    let params = DrumParams::default();
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    let mut ports: Vec<PortBuffers<'_>> = bufs
        .iter_mut()
        .map(|(l, r)| PortBuffers {
            left: l.as_mut_slice(),
            right: r.as_mut_slice(),
        })
        .collect();
    sampler.silence();
    sampler.render_block(
        &mut ports,
        BLOCK,
        &params,
        &[Hit {
            frame: 0,
            note,
            velocity,
        }],
    );
    sampler
        .voices
        .iter()
        .filter(|v| v.active)
        .map(|v| {
            let pad = &sampler.pads[v.pad_index];
            let bank = match v.destination {
                VoiceDestination::CloseMic { bank_index, .. } => &pad.close_mics[bank_index],
                VoiceDestination::Overhead { .. } => pad.overhead.as_ref().unwrap(),
                VoiceDestination::Extra { bank_index, .. } => {
                    &pad.extra_banks[bank_index as usize].bank
                }
            };
            Arc::as_ptr(bank.layers[v.layer_index].round_robins[v.rr_index].shared()) as *const ()
        })
        .collect()
}

fn ptr(take: &Arc<resonance_drums::kit::SampleData>) -> *const () {
    Arc::as_ptr(take) as *const ()
}

/// KickIn's soft layer is unreadable. Dropping it from KickIn alone
/// would leave KickIn one layer and the KickOut / OH banks two: a soft
/// hit would play KickIn's loud take over KickOut's soft one. The cells
/// go from every bank, so every bank keeps the loud layer only, and a
/// hit at any velocity plays the loud strike on every mic.
#[test]
fn a_dropped_layer_keeps_every_bank_on_the_same_velocity_layer() {
    let kit = fixture_kit(Damage::KickInSoftLayer);
    let plugin = booted();
    pick(&plugin, &kit);
    let stats = settle(&plugin);
    assert_eq!(stats.unreadable, 2, "{stats:?}");
    let kick = built_pad(&plugin, 0);
    assert_eq!(kick.close_mics.len(), 2, "KickIn and KickOut both load");
    for bank in kick.close_mics.iter().chain(kick.overhead.iter()) {
        assert_eq!(bank.layers.len(), 1, "{} keeps one layer", bank.position);
    }

    let (mut sampler, _tx) = sampler_on(plugin.bridge.built_kit.lock().as_ref().unwrap().pads.clone());
    let loud = [
        ptr(&take_of(&kit, "kin_1_2.wav")),
        ptr(&take_of(&kit, "kin_2_2.wav")),
        ptr(&take_of(&kit, "kout_2.wav")),
        ptr(&take_of(&kit, "koh_2.wav")),
    ];
    for velocity in [0.05, 0.4, 1.0] {
        for _ in 0..2 {
            let takes = strike_takes(&mut sampler, drum_map::KICK, velocity);
            assert_eq!(takes.len(), 3, "KickIn, KickOut and OH all sound");
            for take in takes {
                assert!(
                    loud.contains(&take),
                    "a bank played a take of the dropped soft layer"
                );
            }
        }
    }
}

/// Banks of different shapes — the fixture's KickIn has two round robins,
/// KickOut and the overhead one — each play the take at the hit's
/// relative position, the same velocity layer on every mic, and no voice
/// is dropped on the hits whose round robin the smaller banks lack.
#[test]
fn banks_of_different_shapes_all_sound_on_every_hit() {
    let kit = fixture_kit(Damage::None);
    let plugin = booted();
    pick(&plugin, &kit);
    settle(&plugin);
    let (mut sampler, _tx) = sampler_on(plugin.bridge.built_kit.lock().as_ref().unwrap().pads.clone());
    let soft = [
        ptr(&take_of(&kit, "kin_1_1.wav")),
        ptr(&take_of(&kit, "kin_2_1.wav")),
        ptr(&take_of(&kit, "kout_1.wav")),
        ptr(&take_of(&kit, "koh_1.wav")),
    ];
    let mut kick_in = Vec::new();
    for _ in 0..4 {
        let takes = strike_takes(&mut sampler, drum_map::KICK, 0.1);
        assert_eq!(takes.len(), 3, "a voice was dropped: {takes:?}");
        assert!(takes.iter().all(|t| soft.contains(t)), "not all on the soft layer");
        assert!(takes.contains(&soft[2]) && takes.contains(&soft[3]));
        kick_in.extend(takes.iter().copied().filter(|t| *t == soft[0] || *t == soft[1]));
    }
    assert!(
        kick_in.contains(&soft[0]) && kick_in.contains(&soft[1]),
        "KickIn walks both its round robins"
    );
}

/// A snare whose top mic and overhead each recorded two round robins.
fn two_take_snare_kit(broken_oh_rr2: bool) -> Kit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-load-path-snare-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    write_wav(&dir.join("top_1.wav"), 1, 0.1);
    write_wav(&dir.join("top_2.wav"), 1, 0.2);
    write_wav(&dir.join("oh_1.wav"), 2, 0.3);
    if broken_oh_rr2 {
        write_corrupt(&dir.join("oh_2.wav"));
    } else {
        write_wav(&dir.join("oh_2.wav"), 2, 0.4);
    }
    let manifest = r#"{
  "SD Snare Normal": {
    "04_SNTop": {"brand":"t","channel":"1","mic":"m","position":"SNTop",
      "rounds":{"RR1":{"Vel01":"top_1.wav"},"RR2":{"Vel01":"top_2.wav"}}},
    "23_OHsAB_e914": {"brand":"t","channel":"1","mic":"m","position":"OHsAB",
      "rounds":{"RR1":{"Vel01":"oh_1.wav"},"RR2":{"Vel01":"oh_2.wav"}}}
  }
}"#;
    let manifest_path = dir.join("drum_samples.json");
    std::fs::write(&manifest_path, manifest).expect("write fixture manifest");
    Kit {
        dir,
        manifest: manifest_path,
    }
}

/// The overhead's second round robin is unreadable. Dropped from the
/// overhead alone, every second hit would pick round robin 2 on the top
/// mic and find no overhead take there: the overhead silent on
/// alternate hits. The cell goes from both banks, so every hit plays the
/// one strike both mics have.
#[test]
fn a_dropped_overhead_take_does_not_silence_the_overhead_on_alternate_hits() {
    let kit = two_take_snare_kit(true);
    let mut plugin = booted();
    pick(&plugin, &kit);
    let stats = settle(&plugin);
    assert_eq!(stats.unreadable, 1, "{stats:?}");
    render(&mut plugin, &[]); // the audio thread takes the kit
    for n in 0..4 {
        plugin.reset();
        let ports = render(&mut plugin, &hit(drum_map::SNARE));
        let (l, r) = &ports[resonance_drums::kit::OVERHEAD_PORT_INDEX];
        let peak = l.iter().chain(r).fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.01, "the overhead is silent on hit {n}");
    }

    let (mut sampler, _tx) = sampler_on(plugin.bridge.built_kit.lock().as_ref().unwrap().pads.clone());
    let strike = [ptr(&take_of(&kit, "top_1.wav")), ptr(&take_of(&kit, "oh_1.wav"))];
    for _ in 0..4 {
        let mut takes = strike_takes(&mut sampler, drum_map::SNARE, 1.0);
        takes.sort();
        let mut want = strike.to_vec();
        want.sort();
        assert_eq!(takes, want, "the top mic and overhead play different strikes");
    }
}

/// A bank none of whose files can be read is dropped as a bank: its
/// failures do not take the other banks' cells with them.
#[test]
fn a_wholly_unreadable_overhead_does_not_silence_the_close_mics() {
    let kit = fixture_kit(Damage::WholeKickOh);
    let plugin = booted();
    pick(&plugin, &kit);
    let stats = settle(&plugin);
    assert_eq!(stats.unreadable, 2, "{stats:?}");
    let kick = built_pad(&plugin, 0);
    assert!(kick.overhead.is_none());
    assert_eq!(kick.close_mics.len(), 2);
    assert_eq!(kick.close_mics[0].layers.len(), 2);
    assert_eq!(kick.close_mics[0].layers[0].round_robins.len(), 2);
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

#[test]
fn progress_completes_only_once_a_process_block_has_taken_the_kit() {
    let kit = fixture_kit(Damage::None);
    let mut plugin = booted();
    render(&mut plugin, &[]);
    // With no kit chosen, the built-in kit is what is wanted, in place.
    assert!(plugin.bridge.load_progress.is_complete());

    let (go, gate) = crossbeam_channel::unbounded();
    *plugin.bridge.decode_gate.lock() = Some(gate);
    pick(&plugin, &kit);
    let held = plugin.bridge.load_progress.snapshot();
    assert_eq!(held.phase, LoadPhase::Decoding);
    assert!(!held.complete);
    assert_eq!(held.fraction(), 0.0);

    go.send(()).unwrap();
    settle(&plugin);
    let handed = plugin.bridge.load_progress.snapshot();
    assert_eq!(handed.phase, LoadPhase::HandedOff);
    assert_eq!((handed.files_done, handed.files_total), (14, 14));
    assert!(
        !handed.complete,
        "complete before the audio thread took the kit"
    );
    assert!(handed.fraction() < 1.0);

    render(&mut plugin, &[]);
    let taken = plugin.bridge.load_progress.snapshot();
    assert!(taken.complete, "{taken:?}");
    assert_eq!(taken.fraction(), 1.0);
}

#[test]
fn a_newer_load_restarts_progress_and_the_stale_kit_never_completes_it() {
    let a = fixture_kit(Damage::None);
    let b = fixture_kit(Damage::None);
    let mut plugin = booted();
    pick(&plugin, &a);
    settle(&plugin);
    // Kit a sits in the mailbox; b replaces it before any block runs.
    pick(&plugin, &b);
    settle(&plugin);
    assert!(!plugin.bridge.load_progress.is_complete());
    render(&mut plugin, &[]);
    assert!(plugin.bridge.load_progress.is_complete());
    assert_eq!(
        plugin.bridge.kit_path.lock().as_deref(),
        Some(b.manifest.as_path())
    );
}

/// A loader checks its stamp under `kit_handoff`, but a newer pick can
/// begin between that check and the old loader's `handed_off` / `failed`.
/// Those late writes must land nowhere: the newer load is still decoding.
#[test]
fn progress_writes_of_a_superseded_load_land_nowhere() {
    use resonance_drums::kit_loader::KitLoadProgress;
    let progress = KitLoadProgress::new();
    progress.begin(1);
    progress.set_total(1, 10);
    progress.begin(2);

    progress.handed_off(1, progress.note_sent());
    progress.note_taken();
    let snap = progress.snapshot();
    assert_eq!(snap.phase, LoadPhase::Decoding, "{snap:?}");
    assert!(!snap.complete, "a superseded hand-off completed the newer load");

    progress.failed(1);
    progress.set_total(1, 10);
    progress.file_done(1);
    progress.idle(1);
    let snap = progress.snapshot();
    assert_eq!(snap.phase, LoadPhase::Decoding, "{snap:?}");
    assert_eq!((snap.files_done, snap.files_total), (0, 0), "{snap:?}");

    // An older `begin` cannot take the progress back either.
    progress.begin(1);
    assert_eq!(progress.snapshot().phase, LoadPhase::Decoding);
    progress.set_total(2, 3);
    progress.file_done(2);
    assert_eq!(progress.snapshot().files_total, 3);

    // The current load's own writes land.
    let ordinal = progress.note_sent();
    progress.handed_off(2, ordinal);
    let snap = progress.snapshot();
    assert_eq!(snap.phase, LoadPhase::HandedOff);
    assert!(!snap.complete);
    progress.note_taken();
    assert!(progress.is_complete());
    progress.failed(2);
    assert_eq!(progress.snapshot().phase, LoadPhase::Failed);
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// A save taken while a pick is still decoding names the pick: saving
/// the kit the user just moved away from would revert it on reopen.
#[test]
fn saving_mid_decode_persists_the_kit_being_loaded() {
    let a = fixture_kit(Damage::None);
    let b = fixture_kit(Damage::None);
    let plugin = booted();
    pick(&plugin, &a);
    settle(&plugin);

    let (go, gate) = crossbeam_channel::unbounded();
    *plugin.bridge.decode_gate.lock() = Some(gate);
    pick(&plugin, &b);
    let saver = DrumsExtraState {
        kit_path: plugin.bridge.kit_path.clone(),
        overhead_setup_key: plugin.bridge.overhead_setup_key.clone(),
        mic_banks: plugin.bridge.mic_banks.clone(),
        pad_choices: plugin.bridge.pad_choices.clone(),
        params: plugin.bridge.params.clone(),
        reload: Some(plugin.bridge.clone()),
    };
    let saved = saver.save();
    let abs = |key: &str| {
        saved
            .get(key)
            .and_then(|r| r.get("abs_path"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    assert_eq!(abs("kit_ref"), Some(b.manifest.to_string_lossy().into_owned()));
    // …and, while the pick may still fail, the kit that loaded last.
    assert_eq!(
        abs("kit_ref_fallback"),
        Some(a.manifest.to_string_lossy().into_owned())
    );
    go.send(()).unwrap();
    settle(&plugin);
    // Settled: nothing to fall back to.
    assert!(saver.save().get("kit_ref_fallback").is_none());
}

fn saver_for(plugin: &ResonanceDrums) -> DrumsExtraState {
    DrumsExtraState {
        kit_path: plugin.bridge.kit_path.clone(),
        overhead_setup_key: plugin.bridge.overhead_setup_key.clone(),
        mic_banks: plugin.bridge.mic_banks.clone(),
        pad_choices: plugin.bridge.pad_choices.clone(),
        params: plugin.bridge.params.clone(),
        reload: Some(plugin.bridge.clone()),
    }
}

/// A project saved mid-pick reopens on the pick — and if that kit will
/// not load, on the kit that last loaded, not on the built-in kit.
#[test]
fn a_reopen_whose_kit_fails_loads_the_last_good_kit() {
    let a = fixture_kit(Damage::None);
    let missing = a.dir.join("gone").join("drum_samples.json");
    let mut plugin = ResonanceDrums::new();
    saver_for(&plugin).load(&serde_json::json!({
        "kit_path": missing.to_string_lossy(),
        "kit_path_fallback": a.manifest.to_string_lossy(),
    }));
    assert!(plugin.initialize(RATE, BLOCK as u32));
    settle(&plugin);
    assert_eq!(
        plugin.bridge.kit_path.lock().as_deref(),
        Some(a.manifest.as_path())
    );
    assert!(plugin.bridge.kit_fallback.lock().is_none());
    render(&mut plugin, &[]);
    assert!(plugin.bridge.load_progress.is_complete());
    assert!(strike_peak(&mut plugin, drum_map::SNARE) > 0.01);
}

/// When the wanted kit loads, the fallback is not used, and is dropped.
#[test]
fn a_reopen_whose_kit_loads_ignores_the_fallback() {
    let a = fixture_kit(Damage::None);
    let b = fixture_kit(Damage::None);
    let plugin = ResonanceDrums::new();
    saver_for(&plugin).load(&serde_json::json!({
        "kit_path": b.manifest.to_string_lossy(),
        "kit_path_fallback": a.manifest.to_string_lossy(),
    }));
    let mut plugin = plugin;
    assert!(plugin.initialize(RATE, BLOCK as u32));
    settle(&plugin);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        plugin.bridge.kit_path.lock().as_deref(),
        Some(b.manifest.as_path())
    );
    assert!(plugin.bridge.kit_fallback.lock().is_none());
}

/// A state from before the fallback key (or saved with nothing pending)
/// loads as it always did.
#[test]
fn a_state_without_a_fallback_still_loads() {
    let a = fixture_kit(Damage::None);
    let plugin = ResonanceDrums::new();
    saver_for(&plugin).load(&serde_json::json!({ "kit_path": a.manifest.to_string_lossy() }));
    assert!(plugin.bridge.kit_fallback.lock().is_none());
    let mut plugin = plugin;
    assert!(plugin.initialize(RATE, BLOCK as u32));
    settle(&plugin);
    assert_eq!(
        plugin.bridge.kit_path.lock().as_deref(),
        Some(a.manifest.as_path())
    );
}

// ---------------------------------------------------------------------------
// Decode pool: cancellation, process-wide budget
// ---------------------------------------------------------------------------

/// A kit whose kick has `takes` round robins on one KickIn setup.
fn many_take_kit(takes: usize) -> Kit {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-load-path-many-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut rounds = Vec::new();
    for i in 0..takes {
        write_wav(&dir.join(format!("k{i}.wav")), 1, 0.1);
        rounds.push(format!(r#""RR{i}":{{"Vel01":"k{i}.wav"}}"#));
    }
    let manifest = format!(
        r#"{{"SD Kick mit Teppich":{{"01_KickIn_e901":{{"brand":"t","channel":"1","mic":"m","position":"KickIn","rounds":{{{}}}}}}}}}"#,
        rounds.join(",")
    );
    let manifest_path = dir.join("drum_samples.json");
    std::fs::write(&manifest_path, manifest).expect("write fixture manifest");
    Kit {
        dir,
        manifest: manifest_path,
    }
}

fn request_for(kit: &Kit) -> resonance_drums::kit_loader::KitRequest {
    resonance_drums::kit_loader::KitRequest {
        path: kit.manifest.clone(),
        overhead_setup_key: DEFAULT_OVERHEAD_SETUP.to_string(),
        pad_choices: std::array::from_fn(|_| PadMicChoices::default()),
        articulations: [false; NUM_PADS],
        preload: resonance_drums::stream::DEFAULT_PRELOAD,
        banks: Default::default(),
    }
}

/// A superseded load stops decoding: once it is cancelled no further
/// file is started, and it fails rather than handing anything off.
#[test]
fn a_cancelled_load_stops_decoding() {
    use resonance_drums::kit_loader::cache::SampleCache;
    use resonance_drums::kit_loader::{decode, load_kit, LOAD_CANCELLED};
    use std::sync::atomic::AtomicUsize;

    const FILES: usize = 64;
    let kit = many_take_kit(FILES);
    let cache = SampleCache::new();
    let done = AtomicUsize::new(0);
    let outcome = load_kit(
        &request_for(&kit),
        RATE,
        None,
        None,
        &cache,
        &|| {
            done.fetch_add(1, Ordering::SeqCst);
        },
        &|_| {},
        // Cancelled as soon as the first file is in.
        &|| done.load(Ordering::SeqCst) >= 1,
    );
    assert_eq!(outcome.err().as_deref(), Some(LOAD_CANCELLED));
    // Each decode thread may have started one file before it saw the
    // cancel; none starts another.
    let decoded = cache.decode_count() as usize;
    assert!(
        decoded <= 1 + decode::decode_workers(FILES),
        "{decoded} of {FILES} files decoded after the cancel"
    );
}

/// Every load in the process draws its decode threads from one budget
/// of cores/2: several instances loading at once never run more.
#[test]
fn concurrent_loads_share_one_decode_thread_budget() {
    use resonance_drums::kit_loader::cache::SampleCache;
    use resonance_drums::kit_loader::{decode, load_kit};

    let kits: Vec<Kit> = (0..4).map(|_| many_take_kit(48)).collect();
    std::thread::scope(|scope| {
        for kit in &kits {
            scope.spawn(move || {
                let cache = SampleCache::new();
                let kit = load_kit(
                    &request_for(kit),
                    RATE,
                    None,
                    None,
                    &cache,
                    &|| {},
                    &|_| {},
                    &|| false,
                )
                .expect("load");
                assert_eq!(kit.stats.decoded, 48);
            });
        }
    });
    let (capacity, peak) = decode::decode_slot_usage();
    assert!(peak >= 1);
    assert!(
        peak <= capacity,
        "{peak} decode threads ran at once; the budget is {capacity}"
    );
}

#[test]
fn unused_cache_entries_are_swept_once_no_kit_holds_them() {
    use resonance_drums::kit_loader::cache::{SampleCache, SampleKey};
    let kit = fixture_kit(Damage::None);
    let cache = SampleCache::new();
    let path = kit.dir.join("kin_1_1.wav");
    let (sample, _) = cache.get_or_decode(&path, RATE).unwrap();
    let key = SampleKey::for_file(&path, RATE).unwrap();
    assert!(cache.lookup(&key).is_some());
    assert_eq!(cache.stats().resident_bytes, sample.bytes() as u64);
    assert_eq!(cache.sweep(), 0, "a held take is kept");
    drop(sample);
    assert!(
        cache.lookup(&key).is_none(),
        "the cache must not keep it alive"
    );
    assert_eq!(cache.sweep(), 1);
    assert_eq!(cache.stats().entries, 0);
    // A different rate is a different take.
    let (_s48, _) = cache.get_or_decode(&path, RATE).unwrap();
    let (_s44, src) = cache.get_or_decode(&path, 44_100.0).unwrap();
    assert_eq!(src, resonance_drums::kit_loader::cache::Source::Decoded);
    assert_eq!(cache.decode_count(), 3);
}

/// A file rewritten between the stat that keys it and the read is not
/// cached under the old key: the next load would otherwise be served a
/// decode of whichever version the read happened to see.
#[test]
fn a_file_rewritten_during_its_read_is_not_cached() {
    use resonance_drums::kit_loader::cache::{SampleCache, SampleKey, Source};
    let kit = fixture_kit(Damage::None);
    let cache = SampleCache::new();
    let path = kit.dir.join("kin_1_1.wav");
    let before = SampleKey::for_file(&path, RATE).unwrap();
    let (first, src) = cache
        .get_or_decode_with_hook(&path, RATE, || write_wav(&path, 2, 0.3))
        .unwrap();
    assert_eq!(src, Source::Decoded);
    assert_eq!(first.channels(), 1, "the read saw the mono version");
    assert!(
        cache.lookup(&before).is_none(),
        "a take read from a file that changed under the read was cached"
    );
    // The next fetch reads the file as it is now.
    let (second, src) = cache.get_or_decode(&path, RATE).unwrap();
    assert_eq!(src, Source::Decoded);
    assert_eq!(second.channels(), 2);
    let (third, src) = cache.get_or_decode(&path, RATE).unwrap();
    assert_eq!(src, Source::Cached, "an unchanged file is cached as before");
    assert!(Arc::ptr_eq(&second, &third));
}

/// A snare with two round robins on its top mic, overhead and room; the
/// room's second is unreadable. The room is an E15 bank: it loses that
/// cell alone (its remaining take then plays every hit, by relative
/// position), and the top mic and overhead keep both strikes — one bad
/// ambience file must not halve the close mics' round robins.
#[test]
fn an_unreadable_room_take_costs_the_room_alone() {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-drums-load-path-room-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    write_wav(&dir.join("top_1.wav"), 1, 0.1);
    write_wav(&dir.join("top_2.wav"), 1, 0.2);
    write_wav(&dir.join("oh_1.wav"), 2, 0.3);
    write_wav(&dir.join("oh_2.wav"), 2, 0.4);
    write_wav(&dir.join("room_1.wav"), 2, 0.05);
    write_corrupt(&dir.join("room_2.wav"));
    let manifest = r#"{
  "SD Snare Normal": {
    "04_SNTop": {"brand":"t","channel":"1","mic":"m","position":"SNTop",
      "rounds":{"RR1":{"Vel01":"top_1.wav"},"RR2":{"Vel01":"top_2.wav"}}},
    "23_OHsAB_e914": {"brand":"t","channel":"1","mic":"m","position":"OHsAB",
      "rounds":{"RR1":{"Vel01":"oh_1.wav"},"RR2":{"Vel01":"oh_2.wav"}}},
    "30_Room": {"brand":"t","channel":"1","mic":"m","position":"Room",
      "rounds":{"RR1":{"Vel01":"room_1.wav"},"RR2":{"Vel01":"room_2.wav"}}}
  }
}"#;
    let manifest_path = dir.join("drum_samples.json");
    std::fs::write(&manifest_path, manifest).expect("write fixture manifest");
    let kit = Kit {
        dir,
        manifest: manifest_path,
    };

    let plugin = booted();
    plugin
        .bridge
        .params
        .room_on
        .set_value(resonance_drums::params::BANK_ON);
    pick(&plugin, &kit);
    let stats = settle(&plugin);
    assert_eq!(stats.unreadable, 1, "{stats:?}");
    let snare = built_pad(&plugin, drum_map::pad_index_for_note(drum_map::SNARE).unwrap());
    let takes = |bank: &resonance_drums::kit::LoadedMicBank| {
        bank.layers.iter().map(|l| l.round_robins.len()).collect::<Vec<_>>()
    };
    assert_eq!(takes(&snare.close_mics[0]), [2], "the top mic keeps both strikes");
    assert_eq!(takes(snare.overhead.as_ref().unwrap()), [2], "so does the overhead");
    assert_eq!(snare.extra_banks.len(), 1);
    assert_eq!(takes(&snare.extra_banks[0].bank), [1], "the room loses its own");
}
