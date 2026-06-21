//! Tests for the optional sidechain (external key) input port.
//!
//! Covers two surfaces:
//! 1. The pure input-port policy helpers (`input_port_count`,
//!    `sidechain_port_index`, `SIDECHAIN_PORT_ID`) that the CLAP audio-ports
//!    extension and the audio processor share to decide how many input ports
//!    to declare and where the key lives.
//! 2. The `ResonancePlugin::process_with_key` trait contract: the default
//!    forwards to `process` (so plugins that don't opt in are unaffected), and
//!    a plugin that opts in receives the key buffer.

use resonance_plugin::clap_bridge::{input_port_count, sidechain_port_index, SIDECHAIN_PORT_ID};
use resonance_plugin::{
    EventIterator, KeyBuffer, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// Pure input-port policy
// ---------------------------------------------------------------------------

#[test]
fn input_port_count_covers_every_combination() {
    // No main input, no sidechain (instrument): zero input ports.
    assert_eq!(input_port_count(None, None), 0);
    // Main input only: the legacy single-input effect.
    assert_eq!(input_port_count(Some(2), None), 1);
    // Main input + sidechain: a plugin opting in declares 2 input ports.
    assert_eq!(input_port_count(Some(2), Some(1)), 2);
    assert_eq!(input_port_count(Some(1), Some(2)), 2);
    // Sidechain only (instrument keyed externally): one input port.
    assert_eq!(input_port_count(None, Some(1)), 1);
}

#[test]
fn sidechain_index_follows_the_main_input() {
    // No sidechain declared -> no index.
    assert_eq!(sidechain_port_index(Some(2), None), None);
    assert_eq!(sidechain_port_index(None, None), None);
    // Main input present -> sidechain is the second port (index 1).
    assert_eq!(sidechain_port_index(Some(2), Some(1)), Some(1));
    // No main input -> sidechain takes index 0.
    assert_eq!(sidechain_port_index(None, Some(2)), Some(0));
}

#[test]
fn sidechain_port_id_is_distinct_from_main_and_output_ids() {
    // Main input is id 1; output ids are `2 + index` (at most 8 ports).
    assert_ne!(SIDECHAIN_PORT_ID, 1);
    for output_index in 0..8u32 {
        assert_ne!(SIDECHAIN_PORT_ID, 2 + output_index);
    }
}

// ---------------------------------------------------------------------------
// Minimal test plugins
// ---------------------------------------------------------------------------

fn no_param(_: usize) -> &'static dyn Param {
    unreachable!("test plugins declare zero params")
}

/// A plain effect that never opts into a sidechain port. It only implements
/// `process`; the default `process_with_key` must forward to it.
struct LegacyEffect {
    process_calls: usize,
}

impl ResonancePlugin for LegacyEffect {
    const CLAP_ID: &'static str = "test.legacy";
    const NAME: &'static str = "Legacy";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &[];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self { process_calls: 0 }
    }
    fn param_count(&self) -> usize {
        0
    }
    fn param(&self, index: usize) -> &dyn Param {
        no_param(index)
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        self.process_calls += 1;
    }
}

/// An effect that opts into a mono sidechain key and reads it.
struct KeyedEffect {
    /// First sample of the key's left channel from the last call, or `None`
    /// if no key was delivered.
    last_key: Option<f32>,
    /// True if the bare `process` path was ever taken (it must not be when
    /// the bridge calls `process_with_key`).
    bare_process_used: bool,
}

impl ResonancePlugin for KeyedEffect {
    const CLAP_ID: &'static str = "test.keyed";
    const NAME: &'static str = "Keyed";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &[];
    const INPUT_CHANNELS: Option<u32> = Some(2);
    const SIDECHAIN_INPUT: Option<u32> = Some(1);

    fn new() -> Self {
        Self {
            last_key: None,
            bare_process_used: false,
        }
    }
    fn param_count(&self) -> usize {
        0
    }
    fn param(&self, index: usize) -> &dyn Param {
        no_param(index)
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        self.bare_process_used = true;
    }
    fn process_with_key(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        key: Option<KeyBuffer<'_>>,
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        self.last_key = key.map(|k| k.left[0]);
    }
}

// ---------------------------------------------------------------------------
// Trait contract
// ---------------------------------------------------------------------------

#[test]
fn default_sidechain_input_is_none() {
    assert_eq!(LegacyEffect::SIDECHAIN_INPUT, None);
}

#[test]
fn default_process_with_key_forwards_to_process() {
    let mut plugin = LegacyEffect::new();
    let mut events = EventIterator::empty();
    // The default `process_with_key` must call `process`, ignoring the key.
    plugin.process_with_key(&mut [], None, 0, &mut events, None);
    plugin.process_with_key(&mut [], None, 0, &mut events, None);
    assert_eq!(plugin.process_calls, 2);
}

#[test]
fn opted_in_plugin_receives_the_key() {
    let mut plugin = KeyedEffect::new();
    let mut events = EventIterator::empty();

    let left = [0.5_f32];
    let right = [0.5_f32];
    plugin.process_with_key(
        &mut [],
        Some(KeyBuffer {
            left: &left,
            right: &right,
        }),
        1,
        &mut events,
        None,
    );
    assert_eq!(plugin.last_key, Some(0.5));
    assert!(
        !plugin.bare_process_used,
        "the keyed plugin's own process_with_key override should run, not the bare process path",
    );

    // No key routed this block -> the override sees `None`.
    plugin.process_with_key(&mut [], None, 0, &mut events, None);
    assert_eq!(plugin.last_key, None);
}
