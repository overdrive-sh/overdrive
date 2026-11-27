//! `upstream_host` — the host adapters of the two upstream ports (ADR-0358).
//!
//! The resolver-file reader and the exchange run on real files and sockets.

use std::io::{self, Read as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use overdrive_core::guest_transport::wire::MAX_GUEST_DATAGRAM_PAYLOAD;
use socket2::SockRef;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, Interest};
use tokio::net::{TcpStream, UdpSocket};

use super::upstream::{
    DnsTransport, ExchangeError, UpstreamConfigError, UpstreamExchange, UpstreamResolvers,
};
use super::wire::reply_matches;

/// Reads nameserver candidates from a resolver file on every call.
pub struct ResolvConfUpstreams {
    path: PathBuf,
}

impl ResolvConfUpstreams {
    /// A reader over `path` (tests; production reads `/etc/resolv.conf`).
    #[must_use]
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait::async_trait]
impl UpstreamResolvers for ResolvConfUpstreams {
    async fn candidates(&self) -> Result<Vec<IpAddr>, UpstreamConfigError> {
        let bytes = tokio::fs::read(&self.path).await.map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => UpstreamConfigError::Absent,
            _ => UpstreamConfigError::Unreadable { source },
        })?;
        Ok(nameserver_addresses(&String::from_utf8_lossy(&bytes)))
    }
}

/// The plain IPv4/IPv6 addresses of the `nameserver` lines, in file order.
/// Comments, other keywords, scoped (`fe80::1%eth0`) and malformed tokens are
/// skipped.
fn nameserver_addresses(contents: &str) -> Vec<IpAddr> {
    contents
        .lines()
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            (tokens.next() == Some("nameserver")).then_some(())?;
            tokens.next()?.parse::<IpAddr>().ok()
        })
        .collect()
}

/// Tokio UDP/TCP exchange, one fresh socket per exchange.
pub struct HostUpstreamExchange;

#[async_trait::async_trait]
impl UpstreamExchange for HostUpstreamExchange {
    async fn exchange(
        &self,
        server: IpAddr,
        transport: DnsTransport,
        query: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, ExchangeError> {
        let target = SocketAddr::new(server, UPSTREAM_PORT);
        let attempt = async {
            match transport {
                DnsTransport::Udp => udp_exchange(target, query).await,
                DnsTransport::Tcp => tcp_exchange(target, query).await,
            }
        };
        tokio::time::timeout(timeout, attempt).await.unwrap_or(Err(ExchangeError::Timeout))
    }
}

/// The DNS port every upstream is queried on.
const UPSTREAM_PORT: u16 = 53;

/// Receive buffer for one UDP datagram: one byte over the largest guest
/// datagram, so a datagram that fills it is known to be too large.
const UDP_RECEIVE_BUFFER: usize = MAX_GUEST_DATAGRAM_PAYLOAD as usize + 1;

/// Classify a socket error: a refusal or an unreachable-network ICMP is
/// `Unreachable`; anything else is `Io`.
fn classify(source: io::Error) -> ExchangeError {
    match source.kind() {
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkUnreachable => ExchangeError::Unreachable { source },
        _ => ExchangeError::Io { source },
    }
}

async fn udp_exchange(target: SocketAddr, query: &[u8]) -> Result<Vec<u8>, ExchangeError> {
    let local = if target.is_ipv4() {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
    };
    let socket = UdpSocket::bind(local).await.map_err(classify)?;
    socket.connect(target).await.map_err(classify)?;
    socket.send(query).await.map_err(classify)?;
    // The receive goes through a borrowed `socket2` view of the same descriptor,
    // so tokio's own readiness gate, which would skip the call on an error-only
    // wake, does not hide the pending ICMP error. No descriptor is duplicated.
    let view = SockRef::from(&socket);
    // One buffer per exchange, reused across wakes.
    let mut buffer = vec![0u8; UDP_RECEIVE_BUFFER];
    loop {
        // `async_io` over READABLE | ERROR: a pending ICMP error wakes the wait
        // (plain readability does not) and a would-block clears both readiness
        // bits, so the loop never spins.
        let received = socket
            .async_io(Interest::READABLE | Interest::ERROR, || (&*view).read(&mut buffer))
            .await
            .map_err(classify)?;
        // A datagram that fills the buffer may have been truncated.
        let Some(datagram) = buffer.get(..received).filter(|_| received < UDP_RECEIVE_BUFFER)
        else {
            return Err(ExchangeError::ReplyTooLarge);
        };
        if reply_matches(query, datagram) {
            return Ok(datagram.to_vec());
        }
    }
}

async fn tcp_exchange(target: SocketAddr, query: &[u8]) -> Result<Vec<u8>, ExchangeError> {
    let length = u16::try_from(query.len()).map_err(|_| ExchangeError::Io {
        source: io::Error::new(io::ErrorKind::InvalidInput, "query exceeds the TCP frame limit"),
    })?;
    let mut stream = TcpStream::connect(target).await.map_err(classify)?;
    let mut frame = Vec::with_capacity(2 + query.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(query);
    stream.write_all(&frame).await.map_err(classify)?;
    let mut prefix = [0u8; 2];
    stream.read_exact(&mut prefix).await.map_err(classify)?;
    let advertised = u16::from_be_bytes(prefix);
    // The buffer grows only as bytes arrive; it is never sized from the prefix.
    let mut reply = Vec::new();
    let read =
        stream.take(u64::from(advertised)).read_to_end(&mut reply).await.map_err(classify)?;
    if read < usize::from(advertised) {
        return Err(ExchangeError::Io {
            source: io::Error::new(io::ErrorKind::UnexpectedEof, "upstream reply was truncated"),
        });
    }
    if !reply_matches(query, &reply) {
        return Err(ExchangeError::Io {
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "upstream reply does not match the query",
            ),
        });
    }
    Ok(reply)
}
