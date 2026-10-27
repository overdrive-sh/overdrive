//! `VsockBeaconIo` — the production adapter of the node beacon listener port
//! (`overdrive_core::vm::beacon::BeaconIo`, D17 / DG-4): real `AF_VSOCK`,
//! async over `AsyncFd`.
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! DG-4. Sim counterpart: `overdrive-sim::SimBeaconIo`.

use std::collections::BTreeMap;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, VsockAddr, bind, getpeername, getsockname, listen,
    shutdown, socket,
};
use overdrive_core::guest_transport::{GuestCid, VsockListenPort};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::VsockPort;
use parking_lot::Mutex;
use rustix::net::{SocketFlags, accept_with};
use tokio::io::unix::AsyncFd;

/// `VMADDR_CID_ANY` from `linux/vm_sockets.h`.
const VMADDR_CID_ANY: u32 = u32::MAX;
/// `VMADDR_PORT_ANY` from `linux/vm_sockets.h`.
const VMADDR_PORT_ANY: u32 = u32::MAX;
/// DG-4's maximum line size, excluding the terminator.
const BEACON_LINE_MAX: usize = 4_096;
static NEXT_INSTANCE: AtomicU32 = AtomicU32::new(1);

struct BeaconConnection {
    socket: AsyncFd<OwnedFd>,
    read_buffer: Mutex<Vec<u8>>,
    read_lock: tokio::sync::Mutex<()>,
    write_lock: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct State {
    listener: Option<Arc<AsyncFd<OwnedFd>>>,
    bound_port: Option<VsockPort>,
    connections: BTreeMap<u64, Arc<BeaconConnection>>,
    next_sequence: u32,
}

/// The node beacon listener on the host kernel: binds `VMADDR_CID_ANY`,
/// accepts guest connections and reports each one's kernel peer
/// (`BeaconConnKey`).
pub struct VsockBeaconIo {
    instance: u32,
    state: Mutex<State>,
}

impl VsockBeaconIo {
    /// Infallible; opens no socket and touches no kernel state. The listener
    /// is created by `BeaconIo::bind`, whose failure `BeaconIntake::bind`
    /// reports as `BeaconIntakeError::Bind`.
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "DG-4-CTOR pins no `Default`: the beacon adapter is constructed explicitly at the composition root"
    )]
    pub fn new() -> Self {
        Self {
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(State::default()),
        }
    }

    fn sequence(&self, socket: &BeaconSocket) -> u64 {
        let raw = socket.raw();
        let instance = u32::try_from(raw >> 32).unwrap_or(0);
        if instance != self.instance {
            unreachable!("a beacon handle another adapter issued was passed to VsockBeaconIo");
        }
        raw & u64::from(u32::MAX)
    }

    fn handle(&self, sequence: u32) -> BeaconSocket {
        BeaconSocket::from_raw((u64::from(self.instance) << 32) | u64::from(sequence))
    }

    fn connection(&self, socket: &BeaconSocket) -> Arc<BeaconConnection> {
        let sequence = self.sequence(socket);
        self.state
            .lock()
            .connections
            .get(&sequence)
            .cloned()
            .unwrap_or_else(|| unreachable!("a live beacon handle names an open connection"))
    }
}

#[async_trait::async_trait]
impl BeaconIo for VsockBeaconIo {
    async fn bind(&self, port: VsockListenPort) -> Result<VsockPort, io::Error> {
        let mut state = self.state.lock();
        if let Some(bound_port) = state.bound_port {
            return Ok(bound_port);
        }

        let raw_port = match port {
            VsockListenPort::Fixed(port) => port.as_u32(),
            VsockListenPort::KernelAssigned => VMADDR_PORT_ANY,
        };
        let listener = socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .map_err(io::Error::from)?;
        bind(listener.as_raw_fd(), &VsockAddr::new(VMADDR_CID_ANY, raw_port))
            .map_err(io::Error::from)?;
        listen(&listener, Backlog::MAXCONN).map_err(io::Error::from)?;
        let bound_port = VsockPort::new(
            getsockname::<VsockAddr>(listener.as_raw_fd()).map_err(io::Error::from)?.port(),
        );
        let listener = Arc::new(AsyncFd::new(listener)?);
        state.bound_port = Some(bound_port);
        state.listener = Some(listener);
        drop(state);
        Ok(bound_port)
    }

    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), io::Error> {
        let listener = self
            .state
            .lock()
            .listener
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "accept before bind"))?;

        loop {
            let mut readiness = listener.readable().await?;
            let accepted = readiness.try_io(|ready| {
                accept_with(ready.get_ref(), SocketFlags::CLOEXEC | SocketFlags::NONBLOCK)
                    .map_err(io::Error::from)
            });
            match accepted {
                Err(_) => {}
                Ok(Err(error)) => return Err(error),
                Ok(Ok(socket)) => {
                    let peer =
                        getpeername::<VsockAddr>(socket.as_raw_fd()).map_err(io::Error::from)?;
                    if let Ok(peer_cid) = GuestCid::new(peer.cid()) {
                        let key =
                            BeaconConnKey { peer_cid, peer_port: VsockPort::new(peer.port()) };
                        let connection = Arc::new(BeaconConnection {
                            socket: AsyncFd::new(socket)?,
                            read_buffer: Mutex::new(Vec::new()),
                            read_lock: tokio::sync::Mutex::new(()),
                            write_lock: tokio::sync::Mutex::new(()),
                        });
                        let sequence = {
                            let mut state = self.state.lock();
                            state.next_sequence =
                                state.next_sequence.checked_add(1).ok_or_else(|| {
                                    io::Error::other("beacon connection handle space exhausted")
                                })?;
                            let sequence = state.next_sequence;
                            state.connections.insert(u64::from(sequence), connection);
                            sequence
                        };
                        return Ok((self.handle(sequence), key));
                    }
                }
            }
        }
    }

    async fn read_line(&self, socket: &BeaconSocket) -> Result<Option<String>, io::Error> {
        let connection = self.connection(socket);
        let _read = connection.read_lock.lock().await;

        loop {
            {
                let mut buffer = connection.read_buffer.lock();
                if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                    if end > BEACON_LINE_MAX {
                        buffer.drain(..=end);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "beacon line exceeds 4096 bytes",
                        ));
                    }
                    let mut line: Vec<u8> = buffer.drain(..=end).collect();
                    line.pop();
                    return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
                }
                if buffer.len() > BEACON_LINE_MAX {
                    buffer.clear();
                    drop(buffer);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "beacon line exceeds 4096 bytes",
                    ));
                }
            }

            let mut readiness = connection.socket.readable().await?;
            let mut bytes = [0_u8; 512];
            match readiness.try_io(|ready| {
                rustix::io::read(ready.get_ref(), &mut bytes).map_err(io::Error::from)
            }) {
                Err(_) => {}
                Ok(Err(error)) => return Err(error),
                Ok(Ok(0)) => return Ok(None),
                Ok(Ok(length)) => {
                    connection.read_buffer.lock().extend_from_slice(&bytes[..length]);
                }
            }
        }
    }

    async fn write_line(&self, socket: &BeaconSocket, line: &str) -> Result<(), io::Error> {
        let connection = self.connection(socket);
        let _write = connection.write_lock.lock().await;
        let bytes = [line.as_bytes(), b"\n"].concat();
        let mut written = 0;
        while written < bytes.len() {
            let mut readiness = connection.socket.writable().await?;
            match readiness.try_io(|ready| {
                rustix::io::write(ready.get_ref(), &bytes[written..]).map_err(io::Error::from)
            }) {
                Err(_) => {}
                Ok(Err(error)) => return Err(error),
                Ok(Ok(0)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "beacon socket accepted no bytes",
                    ));
                }
                Ok(Ok(length)) => written += length,
            }
        }
        Ok(())
    }

    async fn close(&self, socket: BeaconSocket) {
        let sequence = self.sequence(&socket);
        let connection = self.state.lock().connections.remove(&sequence);
        if let Some(connection) = connection {
            let _ =
                shutdown(connection.socket.get_ref().as_raw_fd(), nix::sys::socket::Shutdown::Both);
        }
    }
}
