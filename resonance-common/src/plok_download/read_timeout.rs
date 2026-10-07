//! A per-read timeout for ureq 3, which only offers a total budget for a
//! response body (`ConfigBuilder::timeout_recv_body`) — unusable for a
//! multi-GiB download. This connector wraps ureq's default chain (TCP,
//! proxies, TLS) and caps every wait on the socket at the configured read
//! timeout ([`super::READ_TIMEOUT`] unless a test sets another),
//! so a connection that goes silent fails instead of hanging the worker
//! forever.
//!
//! Built on `ureq::unversioned`, which is outside ureq's semver promise:
//! a ureq update that breaks this fails the build here, not at runtime.

use std::fmt;
use std::time::Duration;

use ureq::unversioned::transport::time::Duration as UreqDuration;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};
use ureq::Error;

pub struct ReadTimeout {
    inner: DefaultConnector,
    limit: Duration,
}

impl ReadTimeout {
    pub fn new(limit: Duration) -> Self {
        Self {
            inner: DefaultConnector::new(),
            limit,
        }
    }
}

impl fmt::Debug for ReadTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadTimeout")
            .field("limit", &self.limit)
            .finish()
    }
}

impl Connector<()> for ReadTimeout {
    type Out = Capped;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, Error> {
        Ok(self.inner.connect(details, chained)?.map(|inner| Capped {
            inner,
            limit: self.limit,
        }))
    }
}

pub struct Capped {
    inner: Box<dyn Transport>,
    limit: Duration,
}

impl fmt::Debug for Capped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Capped")
            .field("inner", &self.inner)
            .finish()
    }
}

impl Capped {
    /// The earlier of ureq's own deadline and our per-read limit.
    fn cap(&self, timeout: NextTimeout) -> NextTimeout {
        if timeout.after.is_not_happening() || *timeout.after > self.limit {
            NextTimeout {
                after: UreqDuration::Exact(self.limit),
                reason: timeout.reason,
            }
        } else {
            timeout
        }
    }
}

impl Transport for Capped {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        let timeout = self.cap(timeout);
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let timeout = self.cap(timeout);
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}
