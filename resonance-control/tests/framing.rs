//! Newline-delimited framing: write/read round-trips, blank lines, EOF,
//! and recovery after an invalid line.

use resonance_control::{write_message, FramingError, MessageReader, Request, Response};
use serde_json::json;
use std::io::Cursor;

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

#[test]
fn reader_accepts_prebuffered_input() {
    let request = Request::without_params(1i64, "song.sections");
    let mut wire = Vec::new();
    write_message(&mut wire, &request).unwrap();

    let mut reader = MessageReader::new(&wire[..]);
    assert_eq!(reader.read_message::<Request>().unwrap(), Some(request));
    assert!(reader.into_inner().is_empty());
}
