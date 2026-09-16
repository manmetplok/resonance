//! Newline-delimited JSON framing.
//!
//! Every protocol message is one UTF-8 JSON document terminated by `\n`,
//! written to a unix stream socket. [`write_message`] frames outgoing
//! messages; [`MessageReader`] wraps the read half of the stream and
//! yields parsed envelopes one line at a time, rejecting any line
//! longer than [`MAX_FRAME_LEN`] so a misbehaving peer cannot balloon
//! the read buffer.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{BufRead, BufReader, Read, Write};

/// Hard cap on the length of one incoming frame, in bytes (16 MiB).
///
/// The largest legitimate frames are bulk note payloads: a
/// `notes.replace_all` with 100k notes serializes to roughly 10 MB
/// (~100 bytes per `NoteSpec`), so 16 MiB leaves comfortable headroom
/// while stopping a runaway or hostile peer from streaming a
/// newline-free multi-GB "line" that would otherwise grow the read
/// buffer until the process OOM-aborts.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Errors produced while framing or unframing messages.
#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The outgoing value could not be serialized.
    #[error("failed to encode message: {0}")]
    Encode(#[source] serde_json::Error),
    /// An incoming line was not a valid message of the expected type.
    #[error("invalid message: {source} (line: {line:?})")]
    Invalid {
        #[source]
        source: serde_json::Error,
        /// The offending line, for diagnostics.
        line: String,
    },
    /// An incoming line exceeded [`MAX_FRAME_LEN`] before its
    /// terminating newline was seen. Nothing past the cap has been
    /// read, so the remainder of the oversized frame is still on the
    /// wire and the reader is stopped mid-line: the connection is
    /// poisoned and the caller must drop it, never keep reading (or
    /// retry) on it.
    #[error("frame exceeds the {limit}-byte cap; the connection must be dropped")]
    Oversized { limit: usize },
}

/// Serialize `message` as a single JSON line (`{...}\n`) and flush.
///
/// The message is encoded to a buffer first so a serialization failure
/// never leaves a half-written frame on the wire.
pub fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), FramingError>
where
    W: Write + ?Sized,
    T: Serialize + ?Sized,
{
    let mut frame = serde_json::to_vec(message).map_err(FramingError::Encode)?;
    frame.push(b'\n');
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

/// Line reader for the read half of a control connection.
///
/// Skips blank lines and returns `Ok(None)` on clean EOF. The message
/// type is chosen per call, so one reader can pull [`crate::Request`]s on
/// the server side and [`crate::Response`]s on the client side.
#[derive(Debug)]
pub struct MessageReader<R> {
    inner: R,
    line: Vec<u8>,
}

impl<R: Read> MessageReader<BufReader<R>> {
    /// Wrap a raw (unbuffered) read half, e.g. a `UnixStream` clone.
    pub fn from_reader(reader: R) -> Self {
        Self::new(BufReader::new(reader))
    }
}

impl<R: BufRead> MessageReader<R> {
    /// Wrap an already-buffered reader.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            line: Vec::new(),
        }
    }

    /// Read the next message, parsed as `T`.
    ///
    /// Returns `Ok(None)` at EOF. Unknown JSON fields inside the message
    /// are ignored (serde default), so newer peers stay compatible. On
    /// [`FramingError::Invalid`] the connection is still positioned at
    /// the start of the next line, so a server can reply with a parse
    /// error and keep reading. On [`FramingError::Oversized`] it is
    /// not — drop the connection (see the variant docs).
    pub fn read_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, FramingError> {
        loop {
            if self.read_bounded_line()? == 0 {
                return Ok(None);
            }
            let trimmed = self.line.trim_ascii();
            if trimmed.is_empty() {
                continue;
            }
            return match serde_json::from_slice(trimmed) {
                Ok(message) => Ok(Some(message)),
                Err(source) => Err(FramingError::Invalid {
                    source,
                    line: String::from_utf8_lossy(trimmed).into_owned(),
                }),
            };
        }
    }

    /// Read one `\n`-terminated line into `self.line`, like `read_line`
    /// but refusing to buffer (or consume) more than [`MAX_FRAME_LEN`]
    /// payload bytes. Returns the number of bytes read, 0 at EOF; a
    /// line still unterminated past the cap is
    /// [`FramingError::Oversized`].
    fn read_bounded_line(&mut self) -> Result<usize, FramingError> {
        self.line.clear();
        loop {
            // `+ 1` admits the newline of a frame that is exactly at
            // the cap; any payload byte in its place trips the check
            // below.
            let remaining = (MAX_FRAME_LEN + 1 - self.line.len()) as u64;
            let read = (&mut self.inner)
                .take(remaining)
                .read_until(b'\n', &mut self.line)?;
            // EOF (possibly mid-line, matching `read_line`) or a
            // complete line.
            if read == 0 || self.line.last() == Some(&b'\n') {
                return Ok(self.line.len());
            }
            if self.line.len() > MAX_FRAME_LEN {
                return Err(FramingError::Oversized {
                    limit: MAX_FRAME_LEN,
                });
            }
            // Short read without a delimiter: keep filling.
        }
    }

    /// Access the underlying reader.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Unwrap the underlying reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}
