//! The rtnetlink client wrapper: link / address / route over
//! `NETLINK_ROUTE` (ADR-0085 D2; spike increment-a, WORKS on a real kernel).
//!
//! Every operation is a typed method on [`Client`] returning
//! [`NetlinkError`]. Idempotency is by **typed errno** — a `del` of an absent
//! link is a silent no-op (the ifindex resolve returns `None`), and the
//! caller swallows `-EEXIST` (address / kernel-auto on-link route already
//! present) on add via [`NetlinkError::errno`]. No stderr strings anywhere.
//!
//! A [`Client`] owns one netlink connection: `new` spawns the rtnetlink
//! connection future on the current tokio runtime and holds the driving
//! [`rtnetlink::Handle`]; the connection task is aborted when the client
//! drops (all in-flight ops have completed by then — the host-netns
//! `provision` awaits each op before returning).

use std::net::{IpAddr, Ipv4Addr};

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::link::LinkFlags;
use rtnetlink::{Handle, LinkUnspec, LinkVeth, RouteMessageBuilder, new_connection};

use crate::error::{NEG_ENODEV, NetlinkError};

/// A single-connection rtnetlink client over `NETLINK_ROUTE`.
pub struct Client {
    handle: Handle,
    connection: tokio::task::JoinHandle<()>,
}

impl Client {
    /// Open a netlink connection and spawn its driver on the current tokio
    /// runtime.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Connect`] when the netlink socket cannot be opened.
    pub fn new() -> Result<Self, NetlinkError> {
        let (connection, handle, _) = new_connection().map_err(NetlinkError::connect)?;
        let connection = tokio::spawn(connection);
        Ok(Self { handle, connection })
    }

    /// Observe a link by name: `Some(up)` when present (with its admin
    /// UP-state), `None` when absent (`-ENODEV`). The structured replacement
    /// for the `ip link show` presence + `state UP` / `,UP,` text parse.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Link`] for any non-`ENODEV` `RTM_GETLINK` failure
    /// (e.g. `EPERM`).
    pub async fn observe_link(&self, name: &str) -> Result<Option<bool>, NetlinkError> {
        let mut stream = self.handle.link().get().match_name(name.to_owned()).execute();
        match stream.try_next().await {
            Ok(Some(message)) => Ok(Some(message.header.flags.contains(LinkFlags::Up))),
            Ok(None) => Ok(None),
            Err(err) => absent_or_err("get", err),
        }
    }

    /// `ip link add <a> type veth peer name <b>` — atomic veth-pair creation.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Link`]. `-EEXIST` (a surviving end already holds one of
    /// the names) surfaces via [`NetlinkError::errno`]; the converge
    /// `RecreatePair` path dels both ends first so a clean create never
    /// collides.
    pub async fn add_veth_pair(&self, a: &str, b: &str) -> Result<(), NetlinkError> {
        self.handle
            .link()
            .add(LinkVeth::new(a, b).build())
            .execute()
            .await
            .map_err(|err| NetlinkError::link("add", err))
    }

    /// `ip link del <name>` — delete one named end (deleting a veth end reaps
    /// both). An absent link is a silent no-op (already gone — the benign
    /// `RecreatePair` case). A racing `-ENODEV` on the del itself surfaces via
    /// [`NetlinkError::errno`] for the caller to swallow.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Link`] for a non-benign del failure.
    pub async fn del_link(&self, name: &str) -> Result<(), NetlinkError> {
        match self.link_index(name).await? {
            None => Ok(()),
            Some(index) => self
                .handle
                .link()
                .del(index)
                .execute()
                .await
                .map_err(|err| NetlinkError::link("del", err)),
        }
    }

    /// `ip addr add <addr>/<prefix> dev <iface>`. `-EEXIST` (already assigned)
    /// surfaces via [`NetlinkError::errno`] for the caller to swallow as the
    /// idempotent converge success.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Address`] on failure, or [`NetlinkError::LinkAbsent`]
    /// (`-ENODEV`, swallowed idempotently) when the iface has vanished.
    pub async fn add_addr(
        &self,
        iface: &str,
        addr: Ipv4Addr,
        prefix: u8,
    ) -> Result<(), NetlinkError> {
        let index = self.require_index(iface).await?;
        self.handle
            .address()
            .add(index, IpAddr::V4(addr), prefix)
            .execute()
            .await
            .map_err(|err| NetlinkError::address("add", err))
    }

    /// `ip link set <iface> up` — idempotent at the kernel.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Link`] on failure, or [`NetlinkError::LinkAbsent`] when
    /// the iface has vanished.
    pub async fn set_link_up(&self, iface: &str) -> Result<(), NetlinkError> {
        let index = self.require_index(iface).await?;
        self.handle
            .link()
            .set(LinkUnspec::new_with_index(index).up().build())
            .execute()
            .await
            .map_err(|err| NetlinkError::link("set-up", err))
    }

    /// `ip route add <dst>/<prefix> dev <oif>` — the on-link route. The kernel
    /// auto-creates a connected route when the gateway address is assigned, so
    /// this legitimately collides with `-EEXIST`; the caller swallows it via
    /// [`NetlinkError::errno`].
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] on failure, or [`NetlinkError::LinkAbsent`] when
    /// the output iface has vanished.
    pub async fn add_onlink_route(
        &self,
        dst: Ipv4Addr,
        prefix: u8,
        oif: &str,
    ) -> Result<(), NetlinkError> {
        let index = self.require_index(oif).await?;
        let route = RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(dst, prefix)
            .output_interface(index)
            .build();
        self.handle
            .route()
            .add(route)
            .execute()
            .await
            .map_err(|err| NetlinkError::route("add", err))
    }

    /// Resolve a link's ifindex by name; `None` when absent (`-ENODEV`).
    async fn link_index(&self, name: &str) -> Result<Option<u32>, NetlinkError> {
        let mut stream = self.handle.link().get().match_name(name.to_owned()).execute();
        match stream.try_next().await {
            Ok(Some(message)) => Ok(Some(message.header.index)),
            Ok(None) => Ok(None),
            Err(err) => absent_or_err("get", err).map(|_| None),
        }
    }

    /// Resolve a link's ifindex, mapping absence to [`NetlinkError::LinkAbsent`]
    /// (`-ENODEV`, swallowed idempotently by the caller) — for ops that
    /// require the link to exist (addr / up / route).
    async fn require_index(&self, iface: &str) -> Result<u32, NetlinkError> {
        self.link_index(iface).await?.ok_or_else(|| NetlinkError::link_absent(iface))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Every op has completed before the client is dropped; abort the now
        // idle connection driver so it does not linger on the runtime.
        self.connection.abort();
    }
}

/// Map an `RTM_GETLINK` error to `Ok(None)` when it is `-ENODEV` (absent),
/// else surface it as a typed [`NetlinkError::Link`].
fn absent_or_err(op: &'static str, err: rtnetlink::Error) -> Result<Option<bool>, NetlinkError> {
    let typed = NetlinkError::link(op, err);
    if typed.errno() == Some(NEG_ENODEV) { Ok(None) } else { Err(typed) }
}
