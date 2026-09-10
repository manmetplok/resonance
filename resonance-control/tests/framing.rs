//! Newline-delimited framing: write/read round-trips, blank lines, EOF,
//! recovery after an invalid line, and the frame-length cap.

use resonance_control::framing::MAX_FRAME_LEN;
use resonance_control::{write_message, FramingError, MessageReader, Request, Response};
use serde_json::{json, Value};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn write_then_read_roundtrips_multiple_messages() {
    let first = Request::new(1i64, "transport.set_tempo", &json!({"bpm": 121.5})).unwrap();
    let second = Request::without_params(2i64, "transport.play");

    let mut wire = Vec::new();
    write_message(&mut wire, &first).unwrap();
    write_message(&mut wire, &second).unwrap();
    assert_eq!(wire.iter().filter(|&&b| b == b'\n').count(), 2);
    assert_eq!(*wire.last().unwrap(), b'\n');

    let mut reader = MessageReader::from_reader(Cursor::new(wire));
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(first));
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(second));
    assert_eq!(reader.read_message::<Request>().unwrap(), None);
}

#[test]
fn each_message_is_a_single_line() {
    let response = Response::success(1i64, &json!({"revision": 3, "note": "a\nb"})).unwrap();
    let mut wire = Vec::new();
    write_message(&mut wire, &response).unwrap();
    // The embedded newline must be escaped, leaving exactly one raw '\n'.
    assert_eq!(wire.iter().filter(|&&b| b == b'\n').count(), 1);

    let mut reader = MessageReader::from_reader(Cursor::new(wire));
    assert_eq!(reader.read_message::<Response>().unwrap(), Some(response));
}

#[test]
fn blank_lines_are_skipped() {
    let request = Request::without_params(5i64, "song.summary");
    let mut wire = Vec::from(&b"\n   \n"[..]);
    write_message(&mut wire, &request).unwrap();
    wire.extend_from_slice(b"\n");

    let mut reader = MessageReader::from_reader(Cursor::new(wire));
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(request));
    assert_eq!(reader.read_message::<Request>().unwrap(), None);
}

#[test]
fn empty_stream_is_clean_eof() {
    let mut reader = MessageReader::from_reader(Cursor::new(Vec::new()));
    assert_eq!(reader.read_message::<Request>().unwrap(), None);
}

#[test]
fn invalid_line_reports_the_line_and_reading_continues() {
    let good = Request::without_params(1i64, "transport.stop");
    let mut wire = Vec::from(&b"{not json}\n"[..]);
    write_message(&mut wire, &good).unwrap();

    let mut reader = MessageReader::from_reader(Cursor::new(wire));
    match reader.read_message::<Request>() {
        Err(FramingError::Invalid { line, .. }) => assert_eq!(line, "{not json}"),
        other => panic!("expected invalid-message error, got {other:?}"),
    }
    // The reader is positioned at the next line and keeps working.
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(good));
}

/// A hostile peer: streams `x`s with no newline in sight, counting how
/// many bytes the reader actually pulls. Finite (`cap`) so a regression
/// that keeps reading fails an assertion instead of hanging the test.
struct NewlineFreeFlood {
    served: Arc<AtomicUsize>,
    cap: usize,
}

impl Read for NewlineFreeFlood {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let already = self.served.load(Ordering::Relaxed);
        let n = buf.len().min(self.cap - already);
        buf[..n].fill(b'x');
        self.served.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
}

/// One JSON line (`{"pad":"aa…a"}`) whose serialized length is exactly
/// `frame_len` bytes, newline-terminated.
fn frame_of_len(frame_len: usize) -> Vec<u8> {
    let overhead = r#"{"pad":""}"#.len();
    let mut wire = format!("{{\"pad\":\"{}\"}}", "a".repeat(frame_len - overhead)).into_bytes();
    assert_eq!(wire.len(), frame_len);
    wire.push(b'\n');
    wire
}

#[test]
fn newline_free_flood_is_rejected_at_the_cap_without_draining_the_stream() {
    let served = Arc::new(AtomicUsize::new(0));
    let mut reader = MessageReader::from_reader(NewlineFreeFlood {
        served: Arc::clone(&served),
        cap: 4 * MAX_FRAME_LEN,
    });
    match reader.read_message::<Value>() {
        Err(FramingError::Oversized { limit }) => assert_eq!(limit, MAX_FRAME_LEN),
        other => panic!("expected oversized-frame error, got {other:?}"),
    }
    // The reader stopped at the cap — it neither buffered the flood nor
    // kept consuming it. Slack covers one BufReader read-ahead chunk.
    let served = served.load(Ordering::Relaxed);
    assert!(
        served <= MAX_FRAME_LEN + 64 * 1024,
        "reader consumed {served} bytes, past the {MAX_FRAME_LEN}-byte cap"
    );
}

#[test]
fn frame_of_exactly_the_cap_passes_and_reading_continues() {
    let mut wire = frame_of_len(MAX_FRAME_LEN);
    let follow_up = Request::without_params(2i64, "transport.play");
    write_message(&mut wire, &follow_up).unwrap();

    let mut reader = MessageReader::from_reader(Cursor::new(wire));
    let value = reader.read_message::<Value>().unwrap().unwrap();
    let overhead = r#"{"pad":""}"#.len();
    assert_eq!(value["pad"].as_str().unwrap().len(), MAX_FRAME_LEN - overhead);
    // Nothing beyond the newline was consumed by the big frame.
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(follow_up));
    assert_eq!(reader.read_message::<Request>().unwrap(), None);
}

#[test]
fn frame_one_byte_over_the_cap_is_oversized() {
    let wire = frame_of_len(MAX_FRAME_LEN + 1);
    let mut reader = MessageReader::from_reader(Cursor::new(wire));
    match reader.read_message::<Value>() {
        Err(FramingError::Oversized { limit }) => assert_eq!(limit, MAX_FRAME_LEN),
        other => panic!("expected oversized-frame error, got {other:?}"),
    }
}

#[test]
fn reader_accepts_prebuffered_input() {
    let request = Request::without_params(1i64, "song.sections");
    let mut wire = Vec::new();
    write_message(&mut wire, &request).unwrap();

    let mut reader = MessageReader::new(&wire[..]);
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(request));
    assert!(reader.into_inner().is_empty());
}
