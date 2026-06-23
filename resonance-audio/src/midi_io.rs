//! Read and write Standard MIDI File (SMF) representations of MIDI
//! note data.
//!
//! Two write paths exist:
//!
//! - [`write_midi_file`] — a Format-0, notes-only file. Used by the
//!   project save/load path so each MIDI clip persists as an
//!   interchangeable `.mid` on disk rather than an inline JSON array.
//! - [`write_midi_project`] — a Format-1 file with a leading conductor
//!   track carrying Set Tempo / Time Signature meta events (derived
//!   from the project [`TempoMap`]) followed by one named track per
//!   source track. Used by whole-project / per-track MIDI export.
//!
//! The clip's tick domain is written through verbatim — `start_tick`
//! and `duration_ticks` stay in the same units the engine uses
//! (see `resonance_audio::types::TICKS_PER_QUARTER_NOTE`), which is
//! also the SMF header's pulses-per-quarter-note.

use std::path::Path;

use midly::num::{u15, u24, u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};

use crate::types::{MidiNote, SignaturePoint, TempoMap, TempoPoint, TICKS_PER_QUARTER_NOTE};

/// One named source track for a multi-track (Format 1) export.
pub struct MidiTrackSource<'a> {
    /// Track name, written as a Track Name (FF 03) meta event.
    pub name: &'a str,
    /// Notes for this track, in any order (sorted internally).
    pub notes: &'a [MidiNote],
}

/// Serialize a list of notes to a Format 0 SMF at `path`.
///
/// This is the notes-only mode: no tempo or time-signature meta events
/// are written. The note list is sorted by absolute tick, expanded into
/// paired note-on/note-off events, delta-encoded, and terminated by a
/// single End-of-Track meta event.
pub fn write_midi_file(path: &Path, notes: &[MidiNote]) -> Result<(), String> {
    let track = build_note_track(None, notes);
    let header = Header::new(Format::SingleTrack, metrical_timing());
    write_smf(path, header, vec![track])
}

/// Serialize a list of notes to the raw bytes of a Format 0 SMF.
///
/// Identical encoding to [`write_midi_file`] but returns the bytes
/// instead of writing them, so callers that need a crash-safe
/// (write-temp-then-rename) write can route the bytes through their
/// own atomic writer.
pub fn encode_midi(notes: &[MidiNote]) -> Result<Vec<u8>, String> {
    let track = build_note_track(None, notes);
    let header = Header::new(Format::SingleTrack, metrical_timing());
    let smf = Smf {
        header,
        tracks: vec![track],
    };
    let mut buf: Vec<u8> = Vec::new();
    smf.write(&mut buf)
        .map_err(|e| format!("serialize smf: {e}"))?;
    Ok(buf)
}

/// Serialize a whole project (or a subset of its tracks) to a Format 1
/// SMF at `path`.
///
/// Track 0 is a conductor track holding Set Tempo (FF 51) and Time
/// Signature (FF 58) meta events derived from `tempo_map`, placed at the
/// tick of the bar each event takes effect. Each entry in `tracks`
/// becomes its own track, named via a Track Name (FF 03) meta event,
/// with a shared tick origin so all tracks align. Passing a single
/// source track yields a conductor + one-track per-track export.
pub fn write_midi_project(
    path: &Path,
    tempo_map: &TempoMap,
    tracks: &[MidiTrackSource],
) -> Result<(), String> {
    let mut smf_tracks: Vec<Track> = Vec::with_capacity(tracks.len() + 1);
    smf_tracks.push(build_conductor_track(tempo_map));
    for t in tracks {
        smf_tracks.push(build_note_track(Some(t.name), t.notes));
    }
    let header = Header::new(Format::Parallel, metrical_timing());
    write_smf(path, header, smf_tracks)
}

/// Header timing: metrical at the engine's ticks-per-quarter-note.
fn metrical_timing() -> Timing {
    Timing::Metrical(u15::new(TICKS_PER_QUARTER_NOTE as u16))
}

/// Build one note track. When `name` is `Some`, a Track Name meta event
/// is emitted first (at tick 0). Notes are paired into note-on/note-off
/// events, sorted by absolute tick, delta-encoded, and terminated by
/// End-of-Track. All notes are written on channel 0.
fn build_note_track<'a>(name: Option<&'a str>, notes: &[MidiNote]) -> Track<'a> {
    let mut events: Vec<(u64, TrackEventKind<'static>)> = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        let key = u7::new(n.note.min(127));
        let vel = u7::new((n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8);
        events.push((
            n.start_tick,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn { key, vel },
            },
        ));
        events.push((
            n.start_tick + n.duration_ticks,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
    }

    // Stable sort keeps note-on before a coincident note-off on the
    // same pitch, which is what most sequencers expect.
    events.sort_by_key(|(tick, _)| *tick);

    let mut track: Track = Track::new();
    if let Some(name) = name {
        track.push(TrackEvent {
            delta: u28(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
        });
    }
    delta_encode(&mut track, events);
    track.push(end_of_track());
    track
}

/// Build the conductor track: tempo and time-signature meta events at
/// the tick of the bar each takes effect.
///
/// Bar→tick conversion mirrors [`TempoMap`]'s bar table, where a bar
/// holds `numerator * TICKS_PER_QUARTER_NOTE` ticks and the signature
/// active before the first signature point is that first point's value.
fn build_conductor_track(tempo_map: &TempoMap) -> Track<'static> {
    let mut events: Vec<(u64, TrackEventKind<'static>)> = Vec::new();

    // Time signatures. Pushed before tempo events so that on a stable
    // sort the time signature precedes the tempo at a shared tick.
    if tempo_map.signature_points.is_empty() {
        events.push((0, time_sig_event(tempo_map.numerator, tempo_map.denominator)));
    } else {
        let first = &tempo_map.signature_points[0];
        if first.bar != 0 {
            // The signature active from bar 0 is the first point's value.
            events.push((0, time_sig_event(first.numerator, first.denominator)));
        }
        for p in &tempo_map.signature_points {
            events.push((
                tick_at_bar(tempo_map, p.bar),
                time_sig_event(p.numerator, p.denominator),
            ));
        }
    }

    // Tempos.
    if tempo_map.tempo_points.is_empty() {
        events.push((0, tempo_event(tempo_map.bpm)));
    } else {
        let first = &tempo_map.tempo_points[0];
        if first.bar != 0 {
            // Tempo before the first point is constant at its value.
            events.push((0, tempo_event(first.bpm)));
        }
        for p in &tempo_map.tempo_points {
            events.push((tick_at_bar(tempo_map, p.bar), tempo_event(p.bpm)));
        }
    }

    events.sort_by_key(|(tick, _)| *tick);

    let mut track: Track = Track::new();
    delta_encode(&mut track, events);
    track.push(end_of_track());
    track
}

/// Absolute tick at the start of a 0-based bar, mirroring `TempoMap`'s
/// bar table: each bar spans `numerator_at_bar(bar) * TPQN` ticks.
fn tick_at_bar(tempo_map: &TempoMap, bar: u32) -> u64 {
    (0..bar)
        .map(|b| numerator_at_bar(tempo_map, b) as u64 * TICKS_PER_QUARTER_NOTE)
        .sum()
}

/// Time-signature numerator active at a 0-based bar, derived from the
/// signature points directly (no bar table needed). Mirrors
/// `TempoMap::rebuild_bar_table`: when signature points exist, bars
/// before the first point inherit that first point's numerator.
fn numerator_at_bar(tempo_map: &TempoMap, bar: u32) -> u8 {
    let points: &[SignaturePoint] = &tempo_map.signature_points;
    match points.first() {
        None => tempo_map.numerator,
        Some(first) => {
            let mut num = first.numerator;
            for p in points {
                if p.bar <= bar {
                    num = p.numerator;
                } else {
                    break;
                }
            }
            num
        }
    }
}

/// A Set Tempo meta event for `bpm` (microseconds per quarter note).
fn tempo_event(bpm: f32) -> TrackEventKind<'static> {
    let us_per_quarter = (60_000_000.0 / bpm.max(0.001) as f64)
        .round()
        .clamp(1.0, 0x00FF_FFFF as f64) as u32;
    TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us_per_quarter)))
}

/// A Time Signature meta event. The SMF denominator is stored as the
/// power of two (4 → 2, 8 → 3); clocks-per-click and 32nds-per-quarter
/// use the conventional 24 / 8.
fn time_sig_event(numerator: u8, denominator: u8) -> TrackEventKind<'static> {
    let den_pow2 = (denominator.max(1) as u32).trailing_zeros() as u8;
    TrackEventKind::Meta(MetaMessage::TimeSignature(numerator, den_pow2, 24, 8))
}

/// Delta-encode absolute-tick events onto `track`. Events must already
/// be sorted by tick.
fn delta_encode(track: &mut Track, events: Vec<(u64, TrackEventKind<'static>)>) {
    let mut prev_tick: u64 = 0;
    for (tick, kind) in events {
        let delta_u64 = tick.saturating_sub(prev_tick);
        track.push(TrackEvent {
            delta: u28(delta_u64),
            kind,
        });
        prev_tick = tick;
    }
}

fn end_of_track() -> TrackEvent<'static> {
    TrackEvent {
        delta: u28(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    }
}

/// Build a `u28` delta, saturating at `u32::MAX` ticks.
fn u28(ticks: u64) -> midly::num::u28 {
    midly::num::u28::new(ticks.min(u32::MAX as u64) as u32)
}

/// Serialize `tracks` under `header` to `path`, creating parent dirs.
fn write_smf(path: &Path, header: Header, tracks: Vec<Track>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let smf = Smf { header, tracks };
    let mut buf: Vec<u8> = Vec::new();
    smf.write(&mut buf)
        .map_err(|e| format!("serialize smf: {e}"))?;
    std::fs::write(path, buf).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Parse a Format 0 (or Format 1 first-track-wins) SMF at `path`
/// back into a list of notes. Unmatched note-ons are silently
/// dropped — a repaired file round-trips bit-for-bit.
pub fn read_midi_file(path: &Path) -> Result<Vec<MidiNote>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let smf = Smf::parse(&bytes).map_err(|e| format!("parse smf: {e}"))?;

    let track = match smf.tracks.first() {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    Ok(notes_from_track(track))
}

/// Extract channel-0 notes from a single SMF track. Shared by
/// [`read_midi_file`] and multi-track readers/tests.
pub fn notes_from_track(track: &[TrackEvent]) -> Vec<MidiNote> {
    // Track pending note-ons keyed by (channel, note) so overlapping
    // notes of the same pitch on different channels don't collide.
    // We only emit channel-0 notes into the result; others are
    // ignored because `MidiClip` doesn't carry a channel field.
    let mut pending: std::collections::HashMap<(u8, u8), (u64, f32)> =
        std::collections::HashMap::new();
    let mut notes: Vec<MidiNote> = Vec::new();
    let mut abs_tick: u64 = 0;

    for ev in track {
        abs_tick += u32::from(ev.delta) as u64;
        if let TrackEventKind::Midi { channel, message } = ev.kind {
            let ch: u8 = channel.into();
            match message {
                MidiMessage::NoteOn { key, vel } => {
                    let v: u8 = vel.into();
                    let k: u8 = key.into();
                    if v == 0 {
                        // Running-status "note off" convention.
                        if let Some((start, velocity)) = pending.remove(&(ch, k)) {
                            if ch == 0 {
                                notes.push(MidiNote {
                                    note: k,
                                    velocity,
                                    start_tick: start,
                                    duration_ticks: abs_tick.saturating_sub(start),
                                });
                            }
                        }
                    } else {
                        pending.insert((ch, k), (abs_tick, v as f32 / 127.0));
                    }
                }
                MidiMessage::NoteOff { key, .. } => {
                    let k: u8 = key.into();
                    if let Some((start, velocity)) = pending.remove(&(ch, k)) {
                        if ch == 0 {
                            notes.push(MidiNote {
                                note: k,
                                velocity,
                                start_tick: start,
                                duration_ticks: abs_tick.saturating_sub(start),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    notes.sort_by_key(|n| (n.start_tick, n.note));
    notes
}

// ---------------------------------------------------------------------------
// Structured import (`ImportedSmf`)
// ---------------------------------------------------------------------------

/// SMF format we accept. Format 2 (`Sequential`) is rejected on import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmfFormat {
    /// Format 0 — a single multi-channel track.
    Format0,
    /// Format 1 — one conductor track plus parallel instrument tracks.
    Format1,
}

/// A tempo change at an absolute position, in engine ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoEvent {
    /// Absolute position in engine ticks ([`TICKS_PER_QUARTER_NOTE`] PPQ).
    pub tick: u64,
    pub bpm: f32,
}

/// A time-signature change at an absolute position, in engine ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureEvent {
    /// Absolute position in engine ticks ([`TICKS_PER_QUARTER_NOTE`] PPQ).
    pub tick: u64,
    pub numerator: u8,
    /// Resolved denominator (e.g. 8), not the raw power-of-two exponent.
    pub denominator: u8,
}

/// One track of an [`ImportedSmf`].
#[derive(Debug, Clone)]
pub struct ImportedTrack {
    /// Track or instrument name from meta events; `None` if neither present.
    pub name: Option<String>,
    /// MIDI channels (0-based) that carried note events on this track.
    pub channels: Vec<u8>,
    /// Paired notes across every channel, sorted by `(start_tick, note)`.
    /// Tick values are scaled to engine PPQ.
    pub notes: Vec<MidiNote>,
    pub note_count: usize,
    /// Lowest note number, `None` when the track has no notes.
    pub pitch_min: Option<u8>,
    /// Highest note number, `None` when the track has no notes.
    pub pitch_max: Option<u8>,
    /// True when the track carries tempo/meter/name meta but no notes —
    /// i.e. a Format 1 conductor track.
    pub is_conductor: bool,
}

/// A Standard MIDI File parsed into engine-domain structures.
///
/// All tick fields are scaled from the file's PPQ to the engine's
/// [`TICKS_PER_QUARTER_NOTE`]. Tempo/signature events carry absolute
/// engine-tick positions; the bar-indexed [`TempoPoint`]/[`SignaturePoint`]
/// vectors are ready to splice into a `TempoMap`.
#[derive(Debug, Clone)]
pub struct ImportedSmf {
    pub format: SmfFormat,
    /// Pulses-per-quarter read from the file header (before scaling).
    pub source_ppq: u16,
    pub track_count: usize,
    /// Longest note end across all tracks, in engine ticks.
    pub length_ticks: u64,
    /// Number of bars spanning `length_ticks` under the file's meter map.
    pub length_bars: u32,
    pub tempo_min_bpm: f32,
    pub tempo_max_bpm: f32,
    pub tracks: Vec<ImportedTrack>,
    /// All tempo events, sorted by absolute engine tick.
    pub tempo_events: Vec<TempoEvent>,
    /// All time-signature events, sorted by absolute engine tick.
    pub signature_events: Vec<SignatureEvent>,
    /// Bar-indexed tempo points (0-based bars), ready for a `TempoMap`.
    pub tempo_points: Vec<TempoPoint>,
    /// Bar-indexed signature points (0-based bars), ready for a `TempoMap`.
    pub signature_points: Vec<SignaturePoint>,
}

/// MIDI default tempo when a file declares none (120 BPM).
const DEFAULT_BPM: f32 = 120.0;

/// Scale a raw tick from `src_ppq` to engine PPQ, rounding half up.
fn scale_tick(raw: u64, src_ppq: u32) -> u64 {
    if src_ppq == 0 {
        return raw;
    }
    let num = raw as u128 * TICKS_PER_QUARTER_NOTE as u128;
    let den = src_ppq as u128;
    // Round half up: (num + den/2) / den, computed without truncating den/2.
    ((2 * num + den) / (2 * den)) as u64
}

/// Engine ticks in one bar of `numerator/denominator`, musically correct
/// (accounts for the denominator). E.g. 4/4 → 1920, 6/8 → 1440.
fn bar_ticks(numerator: u8, denominator: u8) -> u64 {
    let den = denominator.max(1) as u64;
    numerator as u64 * 4 * TICKS_PER_QUARTER_NOTE / den
}

/// Map an absolute engine tick to its 0-based bar index under the meter
/// map described by `sigs` (sorted by tick). Signature changes take effect
/// at the bar boundary at or before their tick.
fn tick_to_bar(target: u64, sigs: &[SignatureEvent]) -> u32 {
    let (mut num, mut den) = (4u8, 4u8);
    let mut si = 0usize;
    let mut bar = 0u32;
    let mut cur = 0u64;
    // Guard against pathological files: bars can't exceed the engine cap.
    while bar < u32::MAX {
        while let Some(ev) = sigs.get(si) {
            if ev.tick <= cur {
                num = ev.numerator;
                den = ev.denominator;
                si += 1;
            } else {
                break;
            }
        }
        let len = bar_ticks(num, den);
        if len == 0 || target < cur + len {
            return bar;
        }
        cur += len;
        bar += 1;
    }
    bar
}

/// Number of bars needed to cover `length_ticks` under `sigs` (ceiling).
fn length_in_bars(length_ticks: u64, sigs: &[SignatureEvent]) -> u32 {
    if length_ticks == 0 {
        return 0;
    }
    // The last content tick lives in bar `tick_to_bar(length-1)`, so the
    // span covers that many bars plus one.
    tick_to_bar(length_ticks - 1, sigs) + 1
}

/// Parse a Standard MIDI File at `path` into a structured [`ImportedSmf`].
///
/// Accepts Format 0 and Format 1. Format 2 and SMPTE-timed files are
/// rejected with a descriptive `Err`, as are corrupt files.
pub fn parse_midi_file(path: &Path) -> Result<ImportedSmf, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_smf_bytes(&bytes)
}

/// Parse SMF bytes into a structured [`ImportedSmf`]. Pure — no I/O.
pub fn parse_smf_bytes(bytes: &[u8]) -> Result<ImportedSmf, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("parse smf: {e}"))?;

    let format = match smf.header.format {
        Format::SingleTrack => SmfFormat::Format0,
        Format::Parallel => SmfFormat::Format1,
        Format::Sequential => {
            return Err("unsupported SMF Format 2 (sequential tracks)".to_string());
        }
    };

    let source_ppq: u16 = match smf.header.timing {
        Timing::Metrical(ppq) => u16::from(ppq),
        Timing::Timecode(_, _) => {
            return Err("unsupported SMPTE-timed SMF (only metrical PPQ timing)".to_string());
        }
    };
    let src = source_ppq as u32;

    let mut tracks: Vec<ImportedTrack> = Vec::with_capacity(smf.tracks.len());
    // Raw (file-PPQ) tempo/signature events gathered across all tracks.
    let mut raw_tempo: Vec<(u64, f32)> = Vec::new();
    let mut raw_sig: Vec<(u64, u8, u8)> = Vec::new();
    let mut length_ticks: u64 = 0;

    for track in &smf.tracks {
        let mut pending: std::collections::HashMap<(u8, u8), (u64, f32)> =
            std::collections::HashMap::new();
        let mut notes: Vec<MidiNote> = Vec::new();
        let mut channels: std::collections::BTreeSet<u8> = std::collections::BTreeSet::new();
        let mut name: Option<String> = None;
        let mut had_meta = false;
        let mut abs_tick: u64 = 0;

        for ev in track {
            abs_tick += u32::from(ev.delta) as u64;
            match ev.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch: u8 = channel.into();
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            let v: u8 = vel.into();
                            let k: u8 = key.into();
                            channels.insert(ch);
                            if v == 0 {
                                // Running-status "note off".
                                if let Some((start, velocity)) = pending.remove(&(ch, k)) {
                                    push_note(&mut notes, k, velocity, start, abs_tick, src);
                                }
                            } else {
                                pending.insert((ch, k), (abs_tick, v as f32 / 127.0));
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            let k: u8 = key.into();
                            channels.insert(ch);
                            if let Some((start, velocity)) = pending.remove(&(ch, k)) {
                                push_note(&mut notes, k, velocity, start, abs_tick, src);
                            }
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(meta) => match meta {
                    MetaMessage::TrackName(bytes) => {
                        had_meta = true;
                        if name.is_none() {
                            name = Some(String::from_utf8_lossy(bytes).into_owned());
                        }
                    }
                    MetaMessage::InstrumentName(bytes) => {
                        had_meta = true;
                        // Track name wins; fall back to instrument name.
                        if name.is_none() {
                            name = Some(String::from_utf8_lossy(bytes).into_owned());
                        }
                    }
                    MetaMessage::Tempo(us_per_quarter) => {
                        had_meta = true;
                        let us = u32::from(us_per_quarter).max(1);
                        let bpm = 60_000_000.0 / us as f32;
                        raw_tempo.push((abs_tick, bpm));
                    }
                    MetaMessage::TimeSignature(numer, denom_pow, _, _) => {
                        had_meta = true;
                        let denominator = 1u32.checked_shl(denom_pow as u32).unwrap_or(1) as u8;
                        raw_sig.push((abs_tick, numer, denominator.max(1)));
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        notes.sort_by_key(|n| (n.start_tick, n.note));
        let note_count = notes.len();
        let pitch_min = notes.iter().map(|n| n.note).min();
        let pitch_max = notes.iter().map(|n| n.note).max();
        for n in &notes {
            length_ticks = length_ticks.max(n.start_tick + n.duration_ticks);
        }
        let is_conductor = note_count == 0 && had_meta;

        tracks.push(ImportedTrack {
            name,
            channels: channels.into_iter().collect(),
            notes,
            note_count,
            pitch_min,
            pitch_max,
            is_conductor,
        });
    }

    // Scale tempo/signature events to engine ticks and sort by position.
    let mut tempo_events: Vec<TempoEvent> = raw_tempo
        .into_iter()
        .map(|(tick, bpm)| TempoEvent {
            tick: scale_tick(tick, src),
            bpm,
        })
        .collect();
    tempo_events.sort_by_key(|e| e.tick);

    let mut signature_events: Vec<SignatureEvent> = raw_sig
        .into_iter()
        .map(|(tick, numerator, denominator)| SignatureEvent {
            tick: scale_tick(tick, src),
            numerator,
            denominator,
        })
        .collect();
    signature_events.sort_by_key(|e| e.tick);

    let length_bars = length_in_bars(length_ticks, &signature_events);

    // Derive bar-indexed points ready to splice into a TempoMap.
    let tempo_points: Vec<TempoPoint> = tempo_events
        .iter()
        .map(|e| TempoPoint {
            bar: tick_to_bar(e.tick, &signature_events),
            bpm: e.bpm,
        })
        .collect();
    let signature_points: Vec<SignaturePoint> = signature_events
        .iter()
        .map(|e| SignaturePoint {
            bar: tick_to_bar(e.tick, &signature_events),
            numerator: e.numerator,
            denominator: e.denominator,
        })
        .collect();

    let (tempo_min_bpm, tempo_max_bpm) = if tempo_events.is_empty() {
        (DEFAULT_BPM, DEFAULT_BPM)
    } else {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for e in &tempo_events {
            lo = lo.min(e.bpm);
            hi = hi.max(e.bpm);
        }
        (lo, hi)
    };

    Ok(ImportedSmf {
        format,
        source_ppq,
        track_count: tracks.len(),
        length_ticks,
        length_bars,
        tempo_min_bpm,
        tempo_max_bpm,
        tracks,
        tempo_events,
        signature_events,
        tempo_points,
        signature_points,
    })
}

/// Pair a note-on with its note-off, scaling start and duration to engine
/// PPQ independently (each rounded half up).
fn push_note(notes: &mut Vec<MidiNote>, key: u8, velocity: f32, start: u64, end: u64, src: u32) {
    let raw_dur = end.saturating_sub(start);
    notes.push(MidiNote {
        note: key,
        velocity,
        start_tick: scale_tick(start, src),
        duration_ticks: scale_tick(raw_dur, src),
    });
}
