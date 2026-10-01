//! `com.resonance.kit-info`'s two ends of the ABI: the plugin's
//! [`answer`] (copy when it fits, length either way) and the host's
//! [`KitInfo::parse`] (nothing or garbage reads as `None`).

use resonance_common::kit_info::{answer, KitInfo, KitInfoPad};

fn info() -> KitInfo {
    KitInfo {
        from_kit: true,
        pads: vec![
            KitInfoPad {
                note: 36,
                name: "Kick".into(),
                present: true,
            },
            KitInfoPad {
                note: 49,
                name: "Crash".into(),
                present: false,
            },
        ],
    }
}

#[test]
fn an_answer_that_fits_round_trips() {
    let json = info().to_json();
    let mut buf = vec![0u8; json.len() + 16];
    // SAFETY: `buf` holds `buf.len()` bytes.
    let len = unsafe { answer(&json, buf.as_mut_ptr(), buf.len()) };
    assert_eq!(len, json.len());
    assert_eq!(KitInfo::parse(&buf[..len]), Some(info()));
}

#[test]
fn an_answer_that_does_not_fit_writes_nothing_and_reports_its_length() {
    let json = info().to_json();
    let mut buf = vec![0xAAu8; json.len() - 1];
    // SAFETY: as above.
    let len = unsafe { answer(&json, buf.as_mut_ptr(), buf.len()) };
    assert_eq!(len, json.len(), "the length the host must retry with");
    assert!(buf.iter().all(|&b| b == 0xAA), "no partial write");
    // And the truncated bytes, should a host parse them anyway, are not
    // a kit.
    assert_eq!(KitInfo::parse(&json.as_bytes()[..json.len() - 1]), None);
}

#[test]
fn a_null_buffer_only_asks_for_the_length() {
    let json = info().to_json();
    // SAFETY: a null `buf` is never written, whatever `cap` claims.
    let len = unsafe { answer(&json, std::ptr::null_mut(), 0) };
    assert_eq!(len, json.len());
    let len = unsafe { answer(&json, std::ptr::null_mut(), 1 << 20) };
    assert_eq!(len, json.len());
}

#[test]
fn an_empty_answer_is_nothing_to_report() {
    let mut buf = [0u8; 8];
    // SAFETY: as above.
    let len = unsafe { answer("", buf.as_mut_ptr(), buf.len()) };
    assert_eq!(len, 0);
    assert_eq!(KitInfo::parse(&[]), None);
}

#[test]
fn garbage_parses_as_nothing() {
    for bytes in [
        &b"not json"[..],
        b"{",
        b"[]",
        b"{\"pads\": 3}",
        b"{\"pads\": [{\"note\": 999, \"name\": \"x\", \"present\": true}]}",
        &[0xFF, 0xFE, 0x00][..],
    ] {
        assert_eq!(KitInfo::parse(bytes), None, "{:?}", String::from_utf8_lossy(bytes));
    }
}

#[test]
fn a_report_without_from_kit_reads_as_the_built_in_kit() {
    let parsed = KitInfo::parse(b"{\"pads\": []}").expect("from_kit defaults");
    assert!(!parsed.from_kit);
    assert!(parsed.pads.is_empty());
}
