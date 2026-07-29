//! Newline-delimited JSON framing.
//!
//! Every protocol message is one UTF-8 JSON document terminated by `\n`,
//! written to a unix stream socket. [`write_message`] frames outgoing
//! messages; [`MessageReader`] wraps the read half of the stream and
//! yields parsed envelopes one line at a time.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{BufRead, BufReader, Read, Write};

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
    line: String,
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
            line: String::new(),
        }
    }

    /// Read the next message, parsed as `T`.
    ///
    /// Returns `Ok(None)` at EOF. Unknown JSON fields inside the message
    /// are ignored (serde default), so newer peers stay compatible. On
    /// [`FramingError::Invalid`] the connection is still positioned at
    /// the start of the next line, so a server can reply with a parse
    /// error and keep reading.
    pub fn read_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, FramingError> {
        loop {
            self.line.clear();
            let bytes = self.inner.read_line(&mut self.line)?;
            if bytes == 0 {
                return Ok(None);
            }
            let trimmed = self.line.trim();
            if trimmed.is_empty() {
                continue;
            }
            return match serde_json::from_str(trimmed) {
                Ok(message) => Ok(Some(message)),
                Err(source) => Err(FramingError::Invalid {
                    source,
                    line: trimmed.to_owned(),
                }),
            };
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
