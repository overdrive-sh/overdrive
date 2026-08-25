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
use std::os::fd::AsRawFd;

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::link::LinkFlags;
use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
use rtnetlink::{
    Handle, LinkUnspec, LinkVeth, NetworkNamespace, RouteMessageBuilder, new_connection,
};

use crate::error::{NEG_ENODEV, NetlinkError};

/// `ip netns add <name>` via rtnetlink's [`NetworkNamespace::add`].
///
/// A `fork` + `unshare(CLONE_NEWNET)` + mount of `/var/run/netns/<name>`. It
/// **execs no external binary** (the fork isolates the caller so it is not
/// moved into the new namespace), so it introduces no subprocess — the whole
/// point of the swap. Creates the persistent netns file the `setns` /
/// link-move path then opens by fd.
///
/// # Errors
///
/// [`NetlinkError::Netns`] (`op = "add"`) when the fork/unshare/mount fails.
/// The rtnetlink `NetworkNamespace` API reports failures as an opaque message
/// (no kernel errno), so this is a structural, fatal failure — never
/// idempotent-swallowed.
pub async fn add_netns(name: &str) -> Result<(), NetlinkError> {
    NetworkNamespace::add(name)
        .await
        .map_err(|err| NetlinkError::netns("add", std::io::Error::other(err.to_string())))
}

/// `ip netns del <name>` via [`NetworkNamespace::del`] — umount + unlink of the
/// persistent netns file. Reaps the in-netns veth end with the namespace.
///
/// # Errors
///
/// [`NetlinkError::Netns`] (`op = "del"`) on a non-benign removal failure.
pub async fn del_netns(name: &str) -> Result<(), NetlinkError> {
    NetworkNamespace::del(name)
        .await
        .map_err(|err| NetlinkError::netns("del", std::io::Error::other(err.to_string())))
}

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

    /// `ip route add default via <gateway>` — the in-netns default route
    /// (called inside [`crate::in_netns`] so it lands in the workload netns).
    /// The kernel resolves the output interface from the gateway's on-link
    /// subnet (the workload veth already carries its /30 address). `-EEXIST`
    /// (the route is already present) surfaces via [`NetlinkError::errno`] for
    /// the caller to swallow as the idempotent converge success.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] on failure.
    pub async fn add_default_route(&self, gateway: Ipv4Addr) -> Result<(), NetlinkError> {
        // No `destination_prefix` ⇒ dst prefix length 0 (`0.0.0.0/0`, the
        // default route); `gateway` sets the `via`.
        let route = RouteMessageBuilder::<Ipv4Addr>::new().gateway(gateway).build();
        self.handle
            .route()
            .add(route)
            .execute()
            .await
            .map_err(|err| NetlinkError::route("add-default", err))
    }

    /// Observe whether a `default via <gateway>` route is present — the
    /// structured replacement for the `ip route show default` text scrape.
    /// Dumps the route table and matches a route whose destination prefix
    /// length is `0` (the default route) whose gateway equals `gateway`.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] (`op = "get"`) on a dump failure.
    pub async fn observe_default_route(&self, gateway: Ipv4Addr) -> Result<bool, NetlinkError> {
        let mut stream =
            self.handle.route().get(RouteMessageBuilder::<Ipv4Addr>::new().build()).execute();
        while let Some(route) =
            stream.try_next().await.map_err(|err| NetlinkError::route("get", err))?
        {
            let is_default = route.header.destination_prefix_length == 0;
            let via_gateway = route.attributes.iter().any(|attr| {
                matches!(attr, RouteAttribute::Gateway(RouteAddress::Inet(gw)) if *gw == gateway)
            });
            if is_default && via_gateway {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ip link set <iface> netns <netns>` — move `iface` from the host netns
    /// into the target netns by fd (`setns_by_fd`). Opens
    /// `/var/run/netns/<netns_name>` (the persistent-netns file
    /// [`add_netns`] creates) and keeps the fd alive across the netlink `set`.
    /// A racing `-ENODEV` (the iface already moved) surfaces via
    /// [`NetlinkError::errno`] for the caller to swallow idempotently.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Netns`] (`op = "open-for-move"`) when the netns file
    /// cannot be opened, [`NetlinkError::LinkAbsent`] when `iface` has
    /// vanished, or [`NetlinkError::Link`] (`op = "set-netns"`) on the move.
    pub async fn move_link_to_netns(
        &self,
        iface: &str,
        netns_name: &str,
    ) -> Result<(), NetlinkError> {
        let index = self.require_index(iface).await?;
        let file = std::fs::File::open(format!("/var/run/netns/{netns_name}"))
            .map_err(|source| NetlinkError::netns("open-for-move", source))?;
        let fd = file.as_raw_fd();
        let result = self
            .handle
            .link()
            .set(LinkUnspec::new_with_index(index).setns_by_fd(fd).build())
            .execute()
            .await
            .map_err(|err| NetlinkError::link("set-netns", err));
        // Keep the netns fd open until AFTER the netlink `set` has been sent.
        drop(file);
        result
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
