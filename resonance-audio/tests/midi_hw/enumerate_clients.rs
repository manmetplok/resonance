//! Port enumeration reuses one sequencer client per direction: the GUI
//! polls it every two seconds for hotplug, and a client created and
//! dropped per call made every MIDI bridge on the system (PipeWire's
//! included) handle a client appearing and leaving twice a second.

use resonance_audio::test_support::{enumerate_midi_inputs, enumerate_midi_outputs};

/// How many ALSA sequencer clients named `name` exist, from the kernel's
/// own list. `None` where there is no ALSA sequencer to ask.
fn seq_clients_named(name: &str) -> Option<usize> {
    let clients = std::fs::read_to_string("/proc/asound/seq/clients").ok()?;
    let quoted = format!("\"{name}\"");
    Some(
        clients
            .lines()
            .filter(|l| l.starts_with("Client") && l.contains(&quoted))
            .count(),
    )
}

/// The first call opens one client per direction and keeps it; later
/// calls open none. Counted against what was there before, since a
/// running app holds clients of the same names. Nothing else in this
/// binary enumerates.
#[test]
fn repeated_enumeration_keeps_one_client_per_direction() {
    let count = || {
        Some((
            seq_clients_named("resonance-enumerate-in")?,
            seq_clients_named("resonance-enumerate-out")?,
        ))
    };
    let Some((in0, out0)) = count() else {
        eprintln!("no ALSA sequencer here; nothing to check");
        return;
    };
    enumerate_midi_inputs();
    enumerate_midi_outputs();
    assert_eq!(count(), Some((in0 + 1, out0 + 1)), "one client each, still open");
    for _ in 0..5 {
        enumerate_midi_inputs();
        enumerate_midi_outputs();
    }
    assert_eq!(count(), Some((in0 + 1, out0 + 1)), "no client per call");
}
