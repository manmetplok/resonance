//! The key actually ARRIVES — host-side sidechain delivery (ba doc #275 P0).
//!
//! `sidechain_taps.rs` pins the routing model in isolation: given a
//! capture, which plugin reads it. It went green the whole time the
//! feature was broken, because every assertion stopped at the tap struct.
//! What was missing was the other half — whether the mixer ever *calls*
//! the keyed process path for a given chain, and whether a given signal
//! is ever *captured* as a source. Four of the five chains never did:
//!
//! | chain                        | before        | now |
//! |------------------------------|---------------|-----|
//! | audio-track inserts          | key delivered | ✓   |
//! | instrument-track inserts     | **ignored**   | ✓   |
//! | sub-track (tap) inserts      | **ignored**   | ✓   |
//! | bus inserts                  | **ignored**   | ✓   |
//! | master inserts               | **ignored**   | ✓   |
//!
//! and as SOURCES, only top-level tracks were captured — a route keyed
//! from a drum tap or from a bus resolved to a slot nobody ever wrote,
//! which `SidechainTaps::key` reports as `None`, which the plugin reads
//! as "no external key" and silently falls back to its own input. That
//! is why a gate at Range 60 keyed from a *silent* track measured the
//! same as one keyed from a kick hitting every beat.
//!
//! So every assertion here is on RENDERED AUDIO with a plugin that
//! reports the key it was handed, never on the route table.

use crate::multi_out_harness;

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;

use multi_out_harness::{at_master, peak, EngineState, PARENT, SR, TAP_A, TAP_B};
use resonance_audio::test_support::{
    __instance_from_raw_for_test, render_stem, PluginSlot, StemSource,
};
use resonance_audio::types::*;

/// What the key monitor emits when the host connected NO key port. Picked
/// well clear of every tap level so a fallback can never be mistaken for
/// a delivered key — this is exactly the confusion the field report hit,
/// where "keyed off its own input" and "keyed off the kick" measured the
/// same.
const NO_KEY: f32 = 0.5;

/// The tap the tests key off. `multi_out_harness`'s instrument writes
/// this constant on port 1 (`TAP_A`).
const KEY_LEVEL: f32 = multi_out_harness::PORT_LEVELS[1];

const MONITOR_ID: PluginInstanceId = 900;
const BUS: BusId = 7;

/// Two chunks of the offline renderer. A key is deliberately one block
/// old (see `types::sidechain`), so a single-chunk render can only ever
/// show the fallback — the second chunk is where a delivered key shows up.
const CHUNK: usize = 1024;
const TWO_CHUNKS: u64 = 2 * CHUNK as u64;

// ---------------------------------------------------------------------------
// A fake effect that reports the key it was handed
// ---------------------------------------------------------------------------

struct MonitorState {
    active: bool,
    /// `process()` calls, when the test wants to know whether this
    /// instance ran at all.
    calls: Option<Arc<AtomicUsize>>,
}

unsafe fn monitor_state<'a>(plugin: *const clap_plugin) -> &'a mut MonitorState {
    &mut *((*plugin).plugin_data as *mut MonitorState)
}

unsafe extern "C" fn m_init(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn m_destroy(_plugin: *const clap_plugin) {}
unsafe extern "C" fn m_activate(
    plugin: *const clap_plugin,
    _sr: f64,
    _min: u32,
    _max: u32,
) -> bool {
    monitor_state(plugin).active = true;
    true
}
unsafe extern "C" fn m_deactivate(plugin: *const clap_plugin) {
    monitor_state(plugin).active = false;
}
unsafe extern "C" fn m_start(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn m_stop(_plugin: *const clap_plugin) {}
unsafe extern "C" fn m_reset(_plugin: *const clap_plugin) {}
unsafe extern "C" fn m_main_thread(_plugin: *const clap_plugin) {}

/// Overwrite the main output with the first sample of the KEY input port
/// (CLAP input port 1), or with [`NO_KEY`] when the host connected no key.
///
/// Overwriting rather than mixing is what makes the assertions exact: the
/// rendered level *is* the key level, so a test reads the key the mixer
/// actually delivered rather than inferring it from a gain change.
unsafe extern "C" fn m_process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    if let Some(calls) = &monitor_state(plugin).calls {
        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    let p = &*process;
    let frames = p.frames_count as usize;

    let mut value = NO_KEY;
    if p.audio_inputs_count >= 2 && !p.audio_inputs.is_null() {
        let key: &clap_audio_buffer = &*p.audio_inputs.add(1);
        if !key.data32.is_null() {
            let chan = *key.data32;
            if !chan.is_null() && frames > 0 {
                value = *chan;
            }
        }
    }

    if p.audio_outputs_count >= 1 && !p.audio_outputs.is_null() {
        let out: &clap_audio_buffer = &*p.audio_outputs;
        if !out.data32.is_null() {
            for ch in 0..(out.channel_count as usize).min(2) {
                let chan = *out.data32.add(ch);
                if chan.is_null() {
                    continue;
                }
                for f in 0..frames {
                    *chan.add(f) = value;
                }
            }
        }
    }
    1
}

/// Main input + key input, one stereo output: the port shape
/// `resonance-plugin`'s `SIDECHAIN_INPUT` produces, which is what
/// `ClapInstance::has_sidechain_input` keys off.
unsafe extern "C" fn m_ports_count(_plugin: *const clap_plugin, is_input: bool) -> u32 {
    if is_input {
        2
    } else {
        1
    }
}

unsafe extern "C" fn m_ports_get(
    _plugin: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    let limit = if is_input { 2 } else { 1 };
    if index >= limit || info.is_null() {
        return false;
    }
    let out = &mut *info;
    out.id = index as clap_id;
    out.name = [0; 256];
    let label: &[u8] = match (is_input, index) {
        (true, 1) => b"Key\0",
        (true, _) => b"In\0",
        _ => b"Out\0",
    };
    for (slot, byte) in out.name.iter_mut().zip(label.iter()) {
        *slot = *byte as c_char;
    }
    out.flags = 0;
    out.channel_count = 2;
    out.port_type = ptr::null();
    out.in_place_pair = u32::MAX;
    true
}

static MONITOR_PORTS_EXT: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(m_ports_count),
    get: Some(m_ports_get),
};

unsafe extern "C" fn m_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if !id.is_null() && CStr::from_ptr(id) == CStr::from_ptr(CLAP_EXT_AUDIO_PORTS.as_ptr()) {
        return &MONITOR_PORTS_EXT as *const clap_plugin_audio_ports as *const c_void;
    }
    ptr::null()
}

fn key_monitor() -> PluginSlot {
    key_monitor_with_calls(None)
}

fn key_monitor_with_calls(calls: Option<Arc<AtomicUsize>>) -> PluginSlot {
    let inst = __instance_from_raw_for_test(
        move |_host| {
            let state = Box::into_raw(Box::new(MonitorState {
                active: false,
                calls,
            }));
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(m_init),
                destroy: Some(m_destroy),
                activate: Some(m_activate),
                deactivate: Some(m_deactivate),
                start_processing: Some(m_start),
                stop_processing: Some(m_stop),
                reset: Some(m_reset),
                process: Some(m_process),
                get_extension: Some(m_get_extension),
                on_main_thread: Some(m_main_thread),
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        SR,
    )
    .expect("key-monitor effect builds");
    assert!(
        inst.has_sidechain_input(),
        "the monitor must declare a key port, or it can never be handed one"
    );
    PluginSlot::new(inst)
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// The harness kit with the key monitor loaded, both taps faded out, and
/// `TAP_B` silenced at the source.
///
/// Fading the taps to zero rather than muting them is deliberate: the tap
/// is captured POST-FX, PRE-FADER, so a source at fader zero still keys
/// at full level. That both isolates the monitor's output at master and
/// pins the documented tap point.
fn fixture() -> EngineState {
    let state = EngineState::with_port_levels([0.0, KEY_LEVEL, 0.0]);
    state.shared.edit_plugins(|p| p.insert(MONITOR_ID, Arc::new(key_monitor())));
    for tap in [TAP_A, TAP_B] {
        state.shared.tracks().get(&tap).unwrap().set_volume(0.0);
    }
    state
        .shared
        .master_volume_bits
        .store(1.0f32.to_bits(), std::sync::atomic::Ordering::Relaxed);
    state
}

fn route(state: &EngineState, source: SendSource) {
    state
        .shared
        .sidechain_routes
        .store(Arc::new(vec![SidechainRoute {
            plugin: MONITOR_ID,
            source,
            enabled: true,
        }]));
}

/// Render two chunks and hand back the peak of the SECOND one — the first
/// chunk can only ever show the fallback, because a key is one block old.
fn render_second_chunk(state: &EngineState, source: StemSource) -> f32 {
    let out = render_stem(
        source,
        0,
        TWO_CHUNKS,
        &state.shared,
        &state.tempo_map,
        SR,
    )
    .expect("render succeeds");
    assert_eq!(out.len(), TWO_CHUNKS as usize * 2);
    peak(&out[CHUNK * 2..])
}

/// Assert a rendered level is the delivered key and not the fallback,
/// naming both so a failure says which side it landed on.
#[track_caller]
fn assert_keyed(got: f32, expected: f32, chain: &str) {
    assert!(
        (got - expected).abs() < 1e-6,
        "{chain}: expected the routed key ({expected}), got {got} \
         (the self-key fallback would read {})",
        at_master(NO_KEY)
    );
}

// ---------------------------------------------------------------------------
// Delivery: every chain that hosts plugins must hand over the key
// ---------------------------------------------------------------------------

/// The bass-ducker case from the field report: a compressor on an
/// INSTRUMENT track, keyed from the kick. The route was stored, the
/// plugin ran, and the mixer called the keyless `process()` — so the
/// ducker keyed off the bass itself and the mix measured identical to
/// two decimals no matter what was routed in.
#[test]
fn an_instrument_tracks_inserts_receive_the_key() {
    let state = fixture();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Track(PARENT));
    assert_keyed(got, at_master(KEY_LEVEL), "instrument-track inserts");
}

/// The gate-on-a-drum-tap case: sub-tracks host their own effect chain,
/// run from the parent's port fan-out, and it had the same keyless call.
#[test]
fn a_sub_tracks_inserts_receive_the_key() {
    let state = fixture();
    state.shared.tracks().get(&TAP_B).unwrap().push_plugin(MONITOR_ID);
    // TAP_B carries the monitor, so it must reach master to be measured;
    // TAP_A stays at fader zero and keys anyway.
    state.shared.tracks().get(&TAP_B).unwrap().set_volume(1.0);
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Track(PARENT));
    assert_keyed(got, at_master(KEY_LEVEL), "sub-track inserts");
}

/// A bus ducker — "route the kick into the group bus's compressor" — is
/// the same shape as the track case and was broken the same way.
#[test]
fn a_busses_inserts_receive_the_key() {
    let state = fixture();
    state.add_bus(BUS, "Group");
    state.shared.edit_bus(BUS, |bus| bus.plugin_ids.push(MONITOR_ID)).unwrap();
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Master);
    assert_keyed(got, at_master(KEY_LEVEL), "bus inserts");
}

/// Master-bus ducking (the classic pumping mix). The master chain runs
/// post-sum in both the live callback and the offline bounce; both now
/// carry the key.
#[test]
fn the_master_chain_receives_the_key() {
    let state = fixture();
    state.shared.edit_master(|master| master.plugin_ids.push(MONITOR_ID));
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Master);
    assert!(
        (got - KEY_LEVEL).abs() < 1e-6,
        "master inserts: expected the routed key ({KEY_LEVEL}), got {got}"
    );
}

// ---------------------------------------------------------------------------
// Sources: what can be keyed FROM
// ---------------------------------------------------------------------------

/// The field report's first suspicion, now supported rather than silently
/// empty: keying off `1000000000`, a TAP of a multi-output drum kit. A
/// kick that only exists as one port of a kit plugin has no other
/// address, so if a tap can't key, "duck the bass from the kick" can't be
/// expressed at all for the built-in kit.
///
/// (This is the same assertion as the instrument-track test above; it is
/// spelled out separately because it pins the SOURCE half — the tap is
/// captured post-FX pre-fader while sitting at fader zero.)
#[test]
fn a_sub_track_can_be_the_key_source() {
    let state = fixture();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Track(PARENT));
    assert_keyed(got, at_master(KEY_LEVEL), "sub-track as key source");
}

/// `SendSource::Bus` has been a legal route target since the feature
/// landed (`sidechain::from_bus`), but no bus was ever captured, so it
/// resolved to an unwritten slot — accepted, then ignored.
#[test]
fn a_bus_can_be_the_key_source() {
    let state = fixture();
    state.add_bus(BUS, "Group");
    state.set_output(TAP_A, TrackOutput::Bus(BUS));
    // The bus is captured pre-fader too, so it can feed a key without
    // being audible itself.
    state.shared.graph.load().bus(BUS).unwrap().set_volume(0.0);
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Bus(BUS));

    let got = render_second_chunk(&state, StemSource::Master);
    // The tap reaches the bus through its own fader, which the fixture
    // parks at zero — so raise it to hear the bus carry a key.
    assert!(
        got < 1e-6 || (got - at_master(NO_KEY)).abs() > 1e-6,
        "bus source must not silently fall back to the self-key value"
    );

    // Now with the tap audible into the bus: the bus's pre-fader sum is
    // the tap at its own fader/pan, which is the signal the key carries.
    let state = fixture();
    state.add_bus(BUS, "Group");
    state.set_output(TAP_A, TrackOutput::Bus(BUS));
    state.shared.tracks().get(&TAP_A).unwrap().set_volume(1.0);
    state.shared.graph.load().bus(BUS).unwrap().set_volume(0.0);
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Bus(BUS));

    let got = render_second_chunk(&state, StemSource::Master);
    assert_keyed(
        got,
        at_master(at_master(KEY_LEVEL)),
        "bus as key source (tap panned into the bus, bus tapped pre-fader)",
    );
}

// ---------------------------------------------------------------------------
// The negative half
// ---------------------------------------------------------------------------

/// With no route the plugin must see NO key and fall back to its own
/// input — the property that made the bug invisible, and the reason the
/// field report's "toggle `enabled` to verify" advice doesn't work.
#[test]
fn an_unrouted_plugin_gets_no_key() {
    let state = fixture();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);

    let got = render_second_chunk(&state, StemSource::Track(PARENT));
    assert!(
        (got - at_master(NO_KEY)).abs() < 1e-6,
        "an unrouted plugin must see no key, got {got}"
    );
}

/// A disabled route is the same as no route: configuration kept, key not
/// delivered.
#[test]
fn a_disabled_route_delivers_no_key() {
    let state = fixture();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    state
        .shared
        .sidechain_routes
        .store(Arc::new(vec![SidechainRoute {
            plugin: MONITOR_ID,
            source: SendSource::Track(TAP_A),
            enabled: false,
        }]));

    let got = render_second_chunk(&state, StemSource::Track(PARENT));
    assert!(
        (got - at_master(NO_KEY)).abs() < 1e-6,
        "a disabled route must deliver no key, got {got}"
    );
}

/// The one-block delay is a property, not an accident: it is what makes
/// the result independent of track order and lets a track key off itself.
/// The first block of a render therefore shows the fallback.
#[test]
fn the_key_is_one_block_old() {
    let state = fixture();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Track(TAP_A));

    let out = render_stem(
        StemSource::Track(PARENT),
        0,
        TWO_CHUNKS,
        &state.shared,
        &state.tempo_map,
        SR,
    )
    .expect("render succeeds");

    let first = peak(&out[..CHUNK * 2]);
    let second = peak(&out[CHUNK * 2..]);
    assert!(
        (first - at_master(NO_KEY)).abs() < 1e-6,
        "the first block has no previous capture to read, got {first}"
    );
    assert_keyed(second, at_master(KEY_LEVEL), "second block");
}

// ---------------------------------------------------------------------------
// Key sources outside the rendered slice
// ---------------------------------------------------------------------------

/// Measuring ONE track while keying from another (ba doc #277).
///
/// This is how the feature is actually verified — `meter.measure` on the
/// ducked track, changing only the key source — and it is the one shape
/// the first fix missed. A stem renders a FILTERED slice of the graph:
/// the target track, its sub-tracks, and any parent needed to drive a
/// fan-out. A track that is only a KEY SOURCE is not in that set, so it
/// never rendered, nothing was ever captured, and the key resolved to
/// silence — which every keyed plugin reads as "no external key" and
/// answers by keying off its own input.
///
/// The result was a measurement that could not move: keying a compressor
/// from a kick hitting every beat and from a track silent for 86% of the
/// song both reported the same LUFS to two decimals, on a compressor
/// proven to have 28 LU of authority over its own input.
#[test]
fn a_key_source_outside_the_stem_still_keys() {
    let state = fixture();
    // The monitor sits on TAP_B and is keyed from TAP_A, its sibling.
    // Rendering TAP_B's stem holds TAP_A out of the mix — but it must
    // still be captured, or the key is silence.
    state.shared.tracks().get(&TAP_B).unwrap().push_plugin(MONITOR_ID);
    state.shared.tracks().get(&TAP_B).unwrap().set_volume(1.0);
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Track(TAP_B));
    assert_keyed(got, at_master(KEY_LEVEL), "key source outside the stem");
}

/// And the source must not leak INTO the stem it keys: it is rendered to
/// be captured, not to be heard. "Drums -> Hats" keyed off the kick is
/// still hats.
#[test]
fn a_key_source_outside_the_stem_does_not_join_it() {
    let state = fixture();
    // No monitor plugin at all: TAP_B is silent on its own. Keying
    // something from TAP_A must not put TAP_A's audio in this stem.
    state.shared.tracks().get(&TAP_B).unwrap().set_volume(1.0);
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Track(TAP_B));
    assert!(
        got < 1e-6,
        "a stem of a silent track must stay silent, got {got} — the key source leaked in"
    );
}

// ---------------------------------------------------------------------------
// Muted / solo-suppressed key sources (code review MIX-05)
// ---------------------------------------------------------------------------
//
// The "ghost kick": a muted kick used only to key the bass compressor. The
// key tap is post-FX / PRE-fader, so it must not depend on the source's
// mute or on somebody else's solo — but capture used to run after the
// mute/solo disposition, so a silenced source was never captured and the
// keyed plugin silently fell back to its own input. In the export and
// live alike.

/// Mixdown: a muted tap still keys, and stays out of the mix.
#[test]
fn a_muted_key_source_still_keys_in_the_mixdown() {
    let state = fixture();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    let tap = state.shared.tracks();
    let tap = tap.get(&TAP_A).unwrap();
    // Audible if it leaked: the mix would read KEY_LEVEL twice.
    tap.set_volume(1.0);
    tap.set_muted(true);
    route(&state, SendSource::Track(TAP_A));

    let got = render_second_chunk(&state, StemSource::Master);
    assert_keyed(got, at_master(KEY_LEVEL), "muted key source, mixdown");
}

/// Mixdown: a key source that another track's solo suppresses still keys.
#[test]
fn a_solo_suppressed_key_source_still_keys_in_the_mixdown() {
    let state = fixture();
    state.add_unfrozen_sibling(multi_out_harness::SIBLING, multi_out_harness::SIBLING_TAP);
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    state.shared.tracks().get(&PARENT).unwrap().set_soloed(true);
    // The sibling kit's tap carries PORT_LEVELS[1] == KEY_LEVEL.
    route(&state, SendSource::Track(multi_out_harness::SIBLING_TAP));

    let got = render_second_chunk(&state, StemSource::Master);
    assert_keyed(got, at_master(KEY_LEVEL), "solo-suppressed key source, mixdown");
}

/// Mixdown: a muted bus still keys (the bus twin of the track case).
#[test]
fn a_muted_key_bus_still_keys_in_the_mixdown() {
    let state = fixture();
    state.add_bus(BUS, "Ghost");
    state.set_output(TAP_A, TrackOutput::Bus(BUS));
    state.shared.tracks().get(&TAP_A).unwrap().set_volume(1.0);
    state.shared.graph.load().bus(BUS).unwrap().set_muted(true);
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Bus(BUS));

    let got = render_second_chunk(&state, StemSource::Master);
    assert_keyed(got, at_master(KEY_LEVEL), "muted key bus, mixdown");
}

/// A muted key source whose only consumer is bypassed is not rendered at
/// all (FU-M3c): nothing reads its key, so the key-only render is pure
/// CPU. Un-bypassing the consumer brings the key-only render back.
#[test]
fn a_muted_key_bus_whose_consumer_is_bypassed_does_not_render() {
    const PROBE_ID: PluginInstanceId = 901;
    let state = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(key_monitor_with_calls(Some(Arc::clone(&calls))));
    state.shared.edit_plugins(|p| p.insert(PROBE_ID, probe));
    state.add_bus(BUS, "Ghost");
    state.shared.graph.load().bus(BUS).unwrap().set_muted(true);
    state.shared.edit_bus(BUS, |bus| bus.plugin_ids.push(PROBE_ID)).unwrap();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Bus(BUS));
    let consumer_bypass = |v: bool| {
        state
            .shared
            .plugins()
            .get(&MONITOR_ID)
            .unwrap()
            .bypass
            .set_bypassed_settled(v)
    };

    consumer_bypass(true);
    render_second_chunk(&state, StemSource::Master);
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "a muted key bus nobody reads must not run its chain"
    );

    consumer_bypass(false);
    let got = render_second_chunk(&state, StemSource::Master);
    assert!(calls.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert_keyed(got, at_master(NO_KEY), "muted key bus keyed by its probe");
}

/// A muted key source whose consumer sits in a chain that will not run
/// is not rendered either (FU-A5c): the consumer's own bypass is not the
/// only way it stops reading its key — its track's whole FX chain being
/// bypassed, or its track being muted, does the same.
#[test]
fn a_muted_key_bus_whose_consumer_chain_is_dormant_does_not_render() {
    const PROBE_ID: PluginInstanceId = 901;
    let state = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(key_monitor_with_calls(Some(Arc::clone(&calls))));
    state.shared.edit_plugins(|p| p.insert(PROBE_ID, probe));
    state.add_bus(BUS, "Ghost");
    state.shared.graph.load().bus(BUS).unwrap().set_muted(true);
    state.shared.edit_bus(BUS, |bus| bus.plugin_ids.push(PROBE_ID)).unwrap();
    state.shared.tracks().get(&PARENT).unwrap().push_plugin(MONITOR_ID);
    route(&state, SendSource::Bus(BUS));
    let probe_calls = || calls.swap(0, std::sync::atomic::Ordering::Relaxed);
    let with_parent = |f: &dyn Fn(&Track)| f(state.shared.tracks().get(&PARENT).unwrap());

    with_parent(&|t| t.fx_bypass().set_bypassed_settled(true));
    render_second_chunk(&state, StemSource::Master);
    assert_eq!(probe_calls(), 0, "consumer's chain bypassed: nobody reads the key");

    with_parent(&|t| t.fx_bypass().set_bypassed_settled(false));
    with_parent(&|t| t.set_muted(true));
    render_second_chunk(&state, StemSource::Master);
    assert_eq!(probe_calls(), 0, "consumer's track muted: nobody reads the key");

    with_parent(&|t| t.set_muted(false));
    let got = render_second_chunk(&state, StemSource::Master);
    assert!(probe_calls() > 0, "an audible, engaged consumer brings the key render back");
    assert_keyed(got, at_master(NO_KEY), "muted key bus keyed by its probe");
}

use resonance_audio::test_support::MixAudioHarness;

const LIVE_BLOCK: usize = 128;

/// The kit + key monitor on the live callback: PARENT hosts the
/// instrument and the monitor, both taps sub-tracks at fader zero.
fn live_fixture(extra: impl FnOnce(&mut Vec<Track>, &MixAudioHarnessPlugins)) -> MixAudioHarness {
    let mut parent = Track::with_type(PARENT, "Kit".into(), TrackType::Instrument);
    parent.set_output(TrackOutput::Master);
    parent.push_plugin(multi_out_harness::INSTRUMENT_ID);
    parent.push_plugin(MONITOR_ID);
    let mut tracks = vec![parent];
    for (id, port) in [(TAP_A, 1u32), (TAP_B, 2)] {
        let mut sub = Track::new_sub_track(id, format!("Tap {port}"), PARENT, port);
        sub.set_output(TrackOutput::Master);
        sub.set_volume(0.0);
        tracks.push(sub);
    }
    let mut plugins = MixAudioHarnessPlugins(Vec::new());
    plugins.0.push((
        multi_out_harness::INSTRUMENT_ID,
        multi_out_harness::multi_out_instrument([0.0, KEY_LEVEL, 0.0]),
    ));
    plugins.0.push((MONITOR_ID, key_monitor()));
    extra(&mut tracks, &plugins);
    let h = MixAudioHarness::new(
        tracks,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        LIVE_BLOCK,
        2,
        SR,
        true,
    );
    for (id, slot) in plugins.0 {
        h.edit_plugins(|p| p.insert(id, Arc::new(slot)));
    }
    use std::sync::atomic::Ordering;
    h.shared().playing.store(true, Ordering::Relaxed);
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    h
}

/// Plugin slots a live fixture inserts once the harness exists.
struct MixAudioHarnessPlugins(Vec<(PluginInstanceId, PluginSlot)>);

fn live_route(h: &MixAudioHarness, source: SendSource) {
    h.shared().sidechain_routes.store(Arc::new(vec![SidechainRoute {
        plugin: MONITOR_ID,
        source,
        enabled: true,
    }]));
}

/// Enough blocks for the mute fade to land (the source is then no longer
/// rendered) and the key's one-block delay; returns the last block.
fn settle(h: &mut MixAudioHarness) -> Vec<f32> {
    for _ in 0..6 {
        h.render();
    }
    h.render().to_vec()
}

/// Live: the ghost kick keeps ducking after it is muted.
#[test]
fn a_muted_key_source_still_keys_live() {
    let mut h = live_fixture(|_, _| {});
    {
        let tracks = h.tracks();
        let tap = tracks.get(&TAP_A).unwrap();
        tap.set_volume(1.0);
        tap.set_muted(true);
    }
    live_route(&h, SendSource::Track(TAP_A));

    let out = settle(&mut h);
    for &s in &out {
        assert_keyed(s, at_master(KEY_LEVEL), "muted key source, live");
    }
}

/// Live: soloing another track does not switch the keyed plugin to
/// self-keying.
#[test]
fn a_solo_suppressed_key_source_still_keys_live() {
    const OTHER: TrackId = 2;
    const OTHER_ID: PluginInstanceId = 101;
    let mut h = live_fixture(|tracks, _| {
        let mut other = Track::with_type(OTHER, "Other".into(), TrackType::Instrument);
        other.set_output(TrackOutput::Master);
        other.push_plugin(OTHER_ID);
        tracks.push(other);
    });
    let other = Arc::new(multi_out_harness::multi_out_instrument([KEY_LEVEL, 0.0, 0.0]));
    h.edit_plugins(|p| p.insert(OTHER_ID, other));
    h.tracks().get(&PARENT).unwrap().set_soloed(true);
    live_route(&h, SendSource::Track(OTHER));

    let out = settle(&mut h);
    for &s in &out {
        assert_keyed(s, at_master(KEY_LEVEL), "solo-suppressed key source, live");
    }
}
