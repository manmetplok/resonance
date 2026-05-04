//! Hardware MIDI clock send and receive.
//!
//! MIDI clock is a 24-PPQN timing protocol: the master sends an 0xF8
//! pulse for every 1/24th of a quarter note plus 0xFA (Start), 0xFB
//! (Continue), 0xFC (Stop), and 0xF2 (Song Position Pointer)
//! messages around the transport state.
//!
//! Threading model mirrors `midi_hardware`: the engine control thread
//! owns the connections, opens/closes them, and pushes outbound clock
//! pulses; the input port spawns a midir thread that pushes parsed
//! clock messages onto a bounded channel that the engine drains.

use std::time::Instant;

use crossbeam_channel::Sender;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

/// Standard MIDI System Real-Time messages used by the clock protocol.
const STATUS_CLOCK: u8 = 0xF8;
const STATUS_START: u8 = 0xFA;
const STATUS_CONTINUE: u8 = 0xFB;
const STATUS_STOP: u8 = 0xFC;
const STATUS_SONG_POSITION: u8 = 0xF2;

/// Inbound clock messages drained on the engine control thread.
#[derive(Debug, Clone)]
pub enum MidiClockEvent {
    /// 0xFA — start playback from the beginning of the song.
    Start { arrival: Instant },
    /// 0xFB — resume playback from the current position.
    Continue { arrival: Instant },
    /// 0xFC — stop playback.
    Stop,
    /// 0xF8 — one 24-PPQN clock tick.
    Clock { arrival: Instant },
    /// 0xF2 — Song Position Pointer in MIDI beats (16th notes).
    SongPosition { sixteenths: u16 },
}

/// Output (master) state. Holds the open midir connection plus the
/// last clock tick the engine emitted, so the per-iteration poll can
/// emit only the new pulses since the previous tick.
pub struct MidiClockSender {
    enabled: bool,
    device_name: Option<String>,
    conn: Option<MidiOutputConnection>,
    /// Last emitted absolute clock tick (24 PPQN). Reset on Start /
    /// Stop / SongPosition so a fresh transport segment doesn't dump
    /// thousands of catch-up pulses. `None` means "anchor to the next
    /// observed playhead" — used after a fresh configure() so we
    /// don't replay ticks behind the current position.
    last_clock_tick: Option<u64>,
}

impl Default for MidiClockSender {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiClockSender {
    pub fn new() -> Self {
        Self {
            enabled: false,
            device_name: None,
            conn: None,
            last_clock_tick: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.enabled && self.conn.is_some()
    }

    /// Set the target device and enabled flag. Closes any prior
    /// connection. When `enabled` is false, the connection is closed
    /// regardless of `device`.
    pub fn configure(
        &mut self,
        device: Option<String>,
        enabled: bool,
    ) -> Result<(), String> {
        // If the configuration didn't actually change, leave the
        // existing connection in place.
        if self.enabled == enabled && self.device_name == device {
            return Ok(());
        }
        self.enabled = enabled;
        self.device_name = device.clone();
        self.conn = None;
        self.last_clock_tick = None;

        if !enabled {
            return Ok(());
        }
        let Some(name) = device else {
            return Ok(());
        };

        let output = MidiOutput::new("resonance-clock-out")
            .map_err(|e| format!("create midi clock output: {e}"))?;
        let port = output
            .ports()
            .into_iter()
            .find(|p| output.port_name(p).map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("midi clock output port not found: {name}"))?;
        let conn = output
            .connect(&port, "resonance-clock-out-conn")
            .map_err(|e| format!("connect midi clock output {name}: {e}"))?;
        self.conn = Some(conn);
        Ok(())
    }

    /// Emit a 0xFA Start, resetting the local clock-tick counter so
    /// `poll_send_clock` only emits pulses from this point forward.
    pub fn send_start(&mut self) {
        if !self.enabled {
            return;
        }
        self.last_clock_tick = Some(0);
        if let Some(conn) = self.conn.as_mut() {
            let _ = conn.send(&[STATUS_START]);
        }
    }

    /// Emit 0xFB Continue, leaving `last_clock_tick` aligned to the
    /// current position so the next poll emits pulses from there.
    pub fn send_continue(&mut self, abs_clock_tick: u64) {
        if !self.enabled {
            return;
        }
        self.last_clock_tick = Some(abs_clock_tick);
        if let Some(conn) = self.conn.as_mut() {
            let _ = conn.send(&[STATUS_CONTINUE]);
        }
    }

    /// Emit 0xFC Stop. The clock-tick counter stays where it is — a
    /// later Continue resumes pulses from the current playhead.
    pub fn send_stop(&mut self) {
        if !self.enabled {
            return;
        }
        if let Some(conn) = self.conn.as_mut() {
            let _ = conn.send(&[STATUS_STOP]);
        }
    }

    /// Emit a Song Position Pointer in 16th notes from the song start
    /// and reset the local clock counter so the next poll lines up.
    pub fn send_song_position(&mut self, sixteenths: u16, abs_clock_tick: u64) {
        if !self.enabled {
            return;
        }
        self.last_clock_tick = Some(abs_clock_tick);
        if let Some(conn) = self.conn.as_mut() {
            let value = sixteenths.min(0x3FFF);
            let lsb = (value & 0x7F) as u8;
            let msb = ((value >> 7) & 0x7F) as u8;
            let _ = conn.send(&[STATUS_SONG_POSITION, lsb, msb]);
        }
    }

    /// Catch up to `abs_clock_tick`, emitting one 0xF8 pulse per
    /// integer step. Bounded internally so a freak playhead jump
    /// never emits a pathological burst. When called with no anchor
    /// established (e.g. after a fresh configure() while playback is
    /// already in progress), we silently anchor to the current tick
    /// without emitting any pulses behind the playhead.
    pub fn poll_send_clock(&mut self, abs_clock_tick: u64) {
        if !self.enabled {
            return;
        }
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        let last = match self.last_clock_tick {
            Some(t) => t,
            None => {
                self.last_clock_tick = Some(abs_clock_tick);
                return;
            }
        };
        if abs_clock_tick <= last {
            return;
        }
        let mut count = abs_clock_tick - last;
        // Worst-case at 999 BPM with 60 Hz polling is ~7 pulses per
        // tick; capping at 64 avoids flooding the wire if `last`
        // somehow desyncs (e.g. a tempo-event reload).
        const MAX_BURST: u64 = 64;
        if count > MAX_BURST {
            count = MAX_BURST;
        }
        for _ in 0..count {
            let _ = conn.send(&[STATUS_CLOCK]);
        }
        self.last_clock_tick = Some(abs_clock_tick);
    }
}

/// Input (slave) state. Holds the open midir connection; parsed
/// messages flow out through the sender provided to `configure`.
pub struct MidiClockReceiver {
    enabled: bool,
    device_name: Option<String>,
    _conn: Option<MidiInputConnection<()>>,
    tx: Sender<MidiClockEvent>,
}

impl MidiClockReceiver {
    pub fn new(tx: Sender<MidiClockEvent>) -> Self {
        Self {
            enabled: false,
            device_name: None,
            _conn: None,
            tx,
        }
    }

    pub fn is_active(&self) -> bool {
        self.enabled && self._conn.is_some()
    }

    pub fn configure(
        &mut self,
        device: Option<String>,
        enabled: bool,
    ) -> Result<(), String> {
        if self.enabled == enabled && self.device_name == device {
            return Ok(());
        }
        self.enabled = enabled;
        self.device_name = device.clone();
        self._conn = None;

        if !enabled {
            return Ok(());
        }
        let Some(name) = device else {
            return Ok(());
        };

        let input = MidiInput::new("resonance-clock-in")
            .map_err(|e| format!("create midi clock input: {e}"))?;
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("midi clock input port not found: {name}"))?;

        let tx = self.tx.clone();
        let conn = input
            .connect(
                &port,
                "resonance-clock-in-conn",
                move |_timestamp, raw, _| {
                    let arrival = std::time::Instant::now();
                    if let Some(event) = parse_clock_message(raw, arrival) {
                        let _ = tx.try_send(event);
                    }
                },
                (),
            )
            .map_err(|e| format!("connect midi clock input {name}: {e}"))?;
        self._conn = Some(conn);
        Ok(())
    }
}

/// Parse a raw MIDI message into a [`MidiClockEvent`], or return
/// `None` for any non-clock byte sequence.
pub fn parse_clock_message(raw: &[u8], arrival: Instant) -> Option<MidiClockEvent> {
    let status = *raw.first()?;
    match status {
        STATUS_CLOCK => Some(MidiClockEvent::Clock { arrival }),
        STATUS_START => Some(MidiClockEvent::Start { arrival }),
        STATUS_CONTINUE => Some(MidiClockEvent::Continue { arrival }),
        STATUS_STOP => Some(MidiClockEvent::Stop),
        STATUS_SONG_POSITION if raw.len() >= 3 => {
            let lsb = (raw[1] & 0x7F) as u16;
            let msb = (raw[2] & 0x7F) as u16;
            Some(MidiClockEvent::SongPosition {
                sixteenths: lsb | (msb << 7),
            })
        }
        _ => None,
    }
}

/// Smoothing filter that estimates BPM from inter-pulse intervals.
/// Holds the last few clock arrival times and computes BPM from the
/// average gap. 24 pulses/quarter, so `bpm = 60 / (24 * avg_secs)`.
pub struct ClockTempoTracker {
    arrivals: std::collections::VecDeque<Instant>,
    /// How many gaps to average over. 24 (= one quarter) gives a
    /// stable read under reasonable jitter without lagging too far
    /// behind real tempo changes.
    window: usize,
}

impl Default for ClockTempoTracker {
    fn default() -> Self {
        Self::new(24)
    }
}

impl ClockTempoTracker {
    pub fn new(window: usize) -> Self {
        Self {
            arrivals: std::collections::VecDeque::with_capacity(window + 1),
            window: window.max(2),
        }
    }

    pub fn reset(&mut self) {
        self.arrivals.clear();
    }

    /// Record one clock pulse arrival and return the smoothed BPM
    /// estimate, or `None` until the window has filled.
    pub fn observe(&mut self, arrival: Instant) -> Option<f32> {
        self.arrivals.push_back(arrival);
        while self.arrivals.len() > self.window + 1 {
            self.arrivals.pop_front();
        }
        if self.arrivals.len() < self.window + 1 {
            return None;
        }
        let first = *self.arrivals.front().unwrap();
        let last = *self.arrivals.back().unwrap();
        let elapsed_secs = last.duration_since(first).as_secs_f64();
        if elapsed_secs <= 0.0 {
            return None;
        }
        let avg_pulse_secs = elapsed_secs / self.window as f64;
        let bpm = 60.0 / (24.0 * avg_pulse_secs);
        Some(bpm.clamp(20.0, 999.0) as f32)
    }
}
