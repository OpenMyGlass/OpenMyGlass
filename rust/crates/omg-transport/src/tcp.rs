//! Wi-Fi / TCP transport.
//!
//! This is the working path today: it needs no Bluetooth, so the companion runs on any
//! device including a PC without a BT radio. It is also the on-demand path for
//! high-bandwidth jobs (media sync, OTA) where Wi-Fi beats BT.
//!
//! Discovery (mDNS `_openmyglass._tcp.local`, since the companion's IP changes) is a
//! `// SCAFFOLD`: today the address is given explicitly. The Wi-Fi QR bootstrap carries
//! an optional direct address hint for exactly this.

use std::net::SocketAddr;

use tokio::net::{TcpListener, TcpStream};
use tracing::debug;

use crate::{BoxStream, Link, Transport, TransportError, TransportKind};

/// Dials a Glass over TCP at a known address.
#[derive(Debug, Clone)]
pub struct TcpTransport {
    addr: String,
}

impl TcpTransport {
    /// New TCP transport to `addr` (`host:port`).
    pub fn new(addr: impl Into<String>) -> Self {
        TcpTransport { addr: addr.into() }
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::WifiTcp
    }

    async fn available(&self) -> bool {
        // Best-effort: can we even resolve/connect quickly? Keep it cheap: assume yes
        // and let dial() be authoritative. (A full reachability probe would open a
        // socket; we avoid the cost here.)
        true
    }

    async fn dial(&self) -> Result<Link<BoxStream>, TransportError> {
        debug!(addr = %self.addr, "dialing TCP");
        let stream = TcpStream::connect(&self.addr).await?;
        stream.set_nodelay(true)?;
        Ok(Link::boxed(stream))
    }
}

/// Server side: accepts companion connections. Used by the simulated Glass and by a
/// companion running in `listen` mode.
pub struct TcpListenerTransport {
    listener: TcpListener,
}

impl TcpListenerTransport {
    /// Bind and listen on `addr`.
    pub async fn bind(addr: impl Into<String>) -> Result<Self, TransportError> {
        let addr = addr.into();
        let listener = TcpListener::bind(&addr).await?;
        Ok(TcpListenerTransport { listener })
    }

    /// The bound local address (useful when binding to port 0 in tests).
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.listener.local_addr()?)
    }

    /// Accept one connection and wrap it in the frame codec.
    pub async fn accept(&self) -> Result<Link<BoxStream>, TransportError> {
        let (stream, peer) = self.listener.accept().await?;
        stream.set_nodelay(true)?;
        debug!(%peer, "accepted TCP");
        Ok(Link::boxed(stream))
    }
}
