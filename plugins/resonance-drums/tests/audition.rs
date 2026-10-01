//! The Audition button plays the pad (ba todo #1328).
//!
//! "▶ Audition" was drawn as a control and discarded its click: the
//! sampler could trigger any pad from a note-on, but there was no way
//! for the editor thread to reach the audio thread. These tests drive
//! the channel that closes that gap — `KitBridge::audition`, drained
//! inside `process()` — and assert the hit reaches the voice allocator
//! and comes out of the right output port.

use resonance_drums::drum_map;
use resonance_drums::kit::NUM_OUTPUT_PORTS;
use resonance_drums::params::OUTPUT_MODE_MULTI;
use resonance_drums::{AuditionHit, ResonanceDrums, AUDITION_VELOCITY};
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 128;
/// Pad 0 (the kick) routes its close mic to output port 1.
const KICK_PORT: usize = 1;
/// Pad 1 (the snare) routes to output port 2.
const SNARE_PORT: usize = 2;

/// A plugin holding the embedded fallback kit — every pad makes sound,
/// with no kit on disk and no loader involved.
fn booted_plugin() -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    // Multi output (E11): these tests read a pad's own port.
    plugin.bridge.params.output_mode.set_value(OUTPUT_MODE_MULTI);
    assert!(plugin.initialize(SAMPLE_RATE, BLOCK as u32));
    plugin
}

/// Render one block with no MIDI at all — the audition must be the only
/// thing that can make a sound here. Returns the per-port left-channel
/// peak.
fn render_block(plugin: &mut ResonanceDrums) -> Vec<f32> {
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
        let mut events = EventIterator::empty();
        // `tempo: None` — no transport information at all, which is the
        // "transport stopped" case the button has to work in.
        plugin.process(&mut ports, BLOCK, &mut events, None);
    }
    buffers
        .into_iter()
        .map(|(l, _)| l.iter().fold(0.0_f32, |acc, s| acc.max(s.abs())))
        .collect()
}

/// Nothing sounds until something asks it to — the baseline the other
/// tests measure against.
#[test]
fn an_idle_plugin_renders_silence() {
    let mut plugin = booted_plugin();
    let peaks = render_block(&mut plugin);
    assert!(
        peaks.iter().all(|p| *p == 0.0),
        "an untouched plugin should be silent, got {peaks:?}"
    );
}

/// The click reaches the voice allocator: one audition, one block, and
/// the kick's own output port carries audio.
#[test]
fn an_audition_triggers_the_pad() {
    let mut plugin = booted_plugin();
    plugin.bridge.audition(drum_map::KICK);

    let peaks = render_block(&mut plugin);
    assert!(
        peaks[KICK_PORT] > 0.0,
        "the kick port should carry the auditioned hit, got {peaks:?}"
    );
    assert!(
        peaks[SNARE_PORT] == 0.0,
        "only the auditioned pad should sound, got {peaks:?}"
    );
}

/// Auditioning is per pad, not "play something".
#[test]
fn an_audition_triggers_the_pad_that_was_asked_for() {
    let mut plugin = booted_plugin();
    plugin.bridge.audition(drum_map::SNARE);

    let peaks = render_block(&mut plugin);
    assert!(peaks[SNARE_PORT] > 0.0, "snare should sound: {peaks:?}");
    assert!(peaks[KICK_PORT] == 0.0, "kick should not: {peaks:?}");
}

/// The queue only carries what was queued: the hit fires on the block
/// that drains it, plays out, and never comes back.
#[test]
fn an_audition_fires_once() {
    let mut plugin = booted_plugin();
    plugin.bridge.audition(drum_map::KICK);

    let first = render_block(&mut plugin);
    assert!(first.iter().any(|p| *p > 0.0), "the hit should sound");

    // Play the sample out (bounded, so a stuck voice fails the test
    // rather than hanging it).
    let mut blocks = 0;
    loop {
        let peaks = render_block(&mut plugin);
        if peaks.iter().all(|p| *p == 0.0) {
            break;
        }
        blocks += 1;
        assert!(blocks < 5_000, "the auditioned hit never ended");
    }

    // …and stays silent: nothing re-queued it.
    for _ in 0..16 {
        let peaks = render_block(&mut plugin);
        assert!(
            peaks.iter().all(|p| *p == 0.0),
            "an audition must not repeat, got {peaks:?}"
        );
    }
}

/// The audition velocity is what is played, not a fixed full-scale hit —
/// so a quieter audition renders quieter.
#[test]
fn the_audition_velocity_reaches_the_voice() {
    let firm = {
        let mut plugin = booted_plugin();
        plugin.bridge.audition(drum_map::KICK);
        render_block(&mut plugin)[KICK_PORT]
    };
    let gentle = {
        let mut plugin = booted_plugin();
        plugin
            .bridge
            .audition_at(drum_map::KICK, AUDITION_VELOCITY * 0.25);
        render_block(&mut plugin)[KICK_PORT]
    };

    assert!(firm > 0.0 && gentle > 0.0);
    assert!(
        gentle < firm * 0.5,
        "a quarter-velocity audition should be markedly quieter: {gentle} vs {firm}"
    );
}

/// Spamming the button cannot wedge the UI thread or stack a burst of
/// hits: the queue is bounded and full sends are dropped.
#[test]
fn spamming_audition_drops_rather_than_blocks() {
    let mut plugin = booted_plugin();
    for _ in 0..10_000 {
        plugin.bridge.audition(drum_map::KICK);
    }
    // Reaching here at all is the point — a blocking send would never
    // return with the audio thread parked.
    let peaks = render_block(&mut plugin);
    assert!(peaks[KICK_PORT] > 0.0, "queued hits should still play");
}

/// The payload is what the sampler needs and nothing more: a note and a
/// velocity, both `Copy`, so the audio side never allocates to read one.
#[test]
fn the_audition_payload_is_a_plain_note_and_velocity() {
    let hit = AuditionHit {
        note: drum_map::KICK,
        velocity: AUDITION_VELOCITY,
    };
    assert_eq!(hit, hit);
    assert!(AUDITION_VELOCITY > 0.0 && AUDITION_VELOCITY <= 1.0);
    assert_eq!(std::mem::size_of::<AuditionHit>(), 8);
}
