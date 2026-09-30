//! RFC 3339 timestamps for the marks file: Unix seconds in memory, UTC
//! strings like `2026-09-30T14:02:11Z` on disk.

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// The current time in Unix seconds (UTC). The store never reads the clock
/// itself: callers pass `now`, so tests inject one.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `secs` as an RFC 3339 UTC string, or `None` outside `time`'s range.
pub fn format_timestamp(secs: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(secs)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

/// Parse an RFC 3339 string (any offset) to Unix seconds.
pub fn parse_timestamp(text: &str) -> Option<i64> {
    OffsetDateTime::parse(text, &Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

/// `serde(with = …)` for `Option<i64>` stored as an RFC 3339 string. An
/// unparsable string reads as `None` rather than failing the whole file.
pub(super) mod serde_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
        match v.and_then(super::format_timestamp) {
            Some(text) => s.serialize_str(&text),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        let v = Option::<serde_json::Value>::deserialize(d)?;
        Ok(match v {
            Some(serde_json::Value::String(s)) => super::parse_timestamp(&s),
            Some(serde_json::Value::Number(n)) => n.as_i64(),
            _ => None,
        })
    }
}
