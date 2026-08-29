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

#[cfg(target_os = "linux")]
use std::fs::OpenOptions;

use futures::stream::TryStreamExt;
use rtnetlink::packet_core::Nla;
use rtnetlink::packet_route::link::{
    InfoData, InfoKind, InfoTun, LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
};
use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute, RouteScope, RouteType};
use rtnetlink::packet_route::rule::{RuleAction, RuleAttribute, RuleMessage};
use rtnetlink::{
    Handle, IpVersion, LinkUnspec, LinkVeth, NetworkNamespace, RouteMessageBuilder, new_connection,
};

use crate::error::{NEG_ENODEV, NetlinkError};

const IFLA_TUN_TYPE: u16 = 3;
const IFLA_TUN_PERSIST: u16 = 6;
const IFLA_TUN_OWNER: u16 = 1;
const NLA_TYPE_MASK: u16 = 0x3fff;

/// Typed actual state for one desired persistent TAP name.
///
/// `Incompatible` means the name exists, but the kernel's `IFLA_LINKINFO`
/// does not identify a persistent `tun`-kind interface whose
/// `IFLA_TUN_TYPE` is `IFF_TAP`. Callers must fail closed on that collision;
/// treating it as the desired TAP would route guest traffic through an
/// unrelated device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapLinkState {
    /// No link owns the name in the observed network namespace.
    Absent,
    /// A link owns the name, but it is not the desired persistent TAP.
    Incompatible,
    /// The exact persistent TAP exists, including its administrative state.
    Persistent { up: bool, owner_uid: Option<u32> },
}

#[cfg(target_os = "linux")]
nix::ioctl_write_ptr_bad!(tun_set_iff, libc::TUNSETIFF, libc::ifreq);
#[cfg(target_os = "linux")]
nix::ioctl_write_int_bad!(tun_set_persist, libc::TUNSETPERSIST);
#[cfg(target_os = "linux")]
nix::ioctl_write_int_bad!(tun_set_owner, libc::TUNSETOWNER);

/// Create a persistent TAP interface through `/dev/net/tun` without a CLI.
///
/// `TUNSETIFF` creates the named L2 device with packet-info headers disabled;
/// `TUNSETPERSIST(1)` transfers its lifetime from this file descriptor to the
/// kernel before the descriptor is closed.
///
/// The interface is created in the caller's network namespace. Use
/// [`Client::move_link_to_netns`] immediately afterward to issue
/// `RTM_SETLINK` with `IFLA_NET_NS_FD` and place it in a workload namespace.
///
/// # Errors
///
/// [`NetlinkError::Netns`] identifies name validation, `/dev/net/tun` open,
/// `TUNSETIFF`, or `TUNSETPERSIST` failures by its `op` value.
#[cfg(target_os = "linux")]
pub fn create_persistent_tap(name: &str, owner_uid: u32) -> Result<(), NetlinkError> {
    let request = tap_request(name)?;
    let owner_uid = tap_owner_arg(owner_uid)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")
        .map_err(|source| NetlinkError::netns("tuntap-open", source))?;

    // SAFETY: `file` is an open `/dev/net/tun` descriptor and `request`
    // remains alive and correctly initialized for the complete ioctl call.
    unsafe { tun_set_iff(file.as_raw_fd(), &raw const request) }
        .map_err(|errno| NetlinkError::netns("tunsetiff", std::io::Error::from(errno)))?;
    // SAFETY: the fd owns the newly created TAP. Grant the exact numeric uid
    // that the composition root later drops Cloud Hypervisor to before the fd
    // is made persistent and closed.
    unsafe { tun_set_owner(file.as_raw_fd(), owner_uid) }
        .map_err(|errno| NetlinkError::netns("tunsetowner", std::io::Error::from(errno)))?;
    // SAFETY: `file` is still the descriptor that owns the TAP just created;
    // integer value 1 is the documented enable-persistence argument.
    unsafe { tun_set_persist(file.as_raw_fd(), 1) }
        .map_err(|errno| NetlinkError::netns("tunsetpersist", std::io::Error::from(errno)))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn tap_request(name: &str) -> Result<libc::ifreq, NetlinkError> {
    let name_bytes = name.as_bytes();
    if name_bytes.is_empty() || name_bytes.len() >= libc::IFNAMSIZ || name_bytes.contains(&b'\0') {
        return Err(NetlinkError::netns(
            "tuntap-name",
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TAP name must contain 1..IFNAMSIZ non-NUL bytes",
            ),
        ));
    }

    let mut ifr_name = [0; libc::IFNAMSIZ];
    // SAFETY: both buffers are valid for `name_bytes.len()` bytes; the input
    // length guard above leaves at least one zero terminator in `ifr_name`.
    // Copy bytes directly because libc models `c_char` as signed on x86_64
    // Linux and unsigned on aarch64 Linux.
    unsafe {
        std::ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            ifr_name.as_mut_ptr().cast::<u8>(),
            name_bytes.len(),
        );
    }
    let ifru_flags = libc::c_short::try_from(libc::IFF_TAP | libc::IFF_NO_PI).map_err(|_| {
        NetlinkError::netns(
            "tuntap-flags",
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "kernel TAP flags do not fit ifreq.ifru_flags",
            ),
        )
    })?;
    Ok(libc::ifreq { ifr_name, ifr_ifru: libc::__c_anonymous_ifr_ifru { ifru_flags } })
}

#[cfg(target_os = "linux")]
fn tap_owner_arg(owner_uid: u32) -> Result<libc::c_int, NetlinkError> {
    libc::c_int::try_from(owner_uid).map_err(|_| {
        NetlinkError::netns(
            "tuntap-owner",
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TAP owner uid does not fit the kernel ioctl argument",
            ),
        )
    })
}

/// Converge the owner uid of an existing persistent TAP in the caller's
/// current network namespace.
///
/// Reopening the named TAP with `TUNSETIFF` attaches this descriptor to the
/// persistent device; `TUNSETOWNER` then updates the uid permitted to reopen
/// it. Root may call this idempotently after restart to repair an older or
/// drifted owner before the confined VMM launches.
#[cfg(target_os = "linux")]
pub fn set_persistent_tap_owner(name: &str, owner_uid: u32) -> Result<(), NetlinkError> {
    let request = tap_request(name)?;
    let owner_uid = tap_owner_arg(owner_uid)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")
        .map_err(|source| NetlinkError::netns("tuntap-open", source))?;
    // SAFETY: attach this owned tun fd to the named existing TAP.
    unsafe { tun_set_iff(file.as_raw_fd(), &raw const request) }
        .map_err(|errno| NetlinkError::netns("tunsetiff", std::io::Error::from(errno)))?;
    // SAFETY: update the owner on the TAP attached to this fd.
    unsafe { tun_set_owner(file.as_raw_fd(), owner_uid) }
        .map_err(|errno| NetlinkError::netns("tunsetowner", std::io::Error::from(errno)))?;
    Ok(())
}

/// Persistent TAP creation is a Linux-only kernel facility.
///
/// # Errors
///
/// Always returns [`std::io::ErrorKind::Unsupported`] off Linux.
#[cfg(not(target_os = "linux"))]
pub fn create_persistent_tap(_name: &str, _owner_uid: u32) -> Result<(), NetlinkError> {
    Err(NetlinkError::netns(
        "tuntap-unsupported",
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "persistent TAP creation is supported only on Linux",
        ),
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn set_persistent_tap_owner(_name: &str, _owner_uid: u32) -> Result<(), NetlinkError> {
    Err(NetlinkError::netns(
        "tuntap-unsupported",
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "persistent TAP ownership is supported only on Linux",
        ),
    ))
}

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

    /// Observe whether `name` is the exact persistent TAP resource.
    ///
    /// This inspects typed `RTM_GETLINK` attributes: `IFLA_INFO_KIND=tun`,
    /// `IFLA_TUN_TYPE=IFF_TAP`, and `IFLA_TUN_PERSIST=1`. A dummy, veth, TUN,
    /// or non-persistent TAP with the same name is [`TapLinkState::Incompatible`],
    /// never accepted as desired state.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Link`] for any non-`ENODEV` `RTM_GETLINK` failure.
    pub async fn observe_persistent_tap(&self, name: &str) -> Result<TapLinkState, NetlinkError> {
        let mut stream = self.handle.link().get().match_name(name.to_owned()).execute();
        match stream.try_next().await {
            Ok(Some(message)) => Ok(persistent_tap_state(&message)),
            Ok(None) => Ok(TapLinkState::Absent),
            Err(err) => absent_or_err("get-tap", err).map(|_| TapLinkState::Absent),
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
    /// (`-ENODEV`, surfaced fail-closed by convergence callers) when the iface
    /// has vanished.
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

    /// Observe an exact IPv4 address and prefix on `iface`.
    ///
    /// Address identity includes `AddressMessage.header.prefix_len`; the same
    /// address under `/32` is therefore not accepted for a desired `/30`.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Address`] on a dump failure, or
    /// [`NetlinkError::LinkAbsent`] when `iface` has vanished.
    pub async fn observe_addr(
        &self,
        iface: &str,
        addr: Ipv4Addr,
        prefix: u8,
    ) -> Result<bool, NetlinkError> {
        let index = self.require_index(iface).await?;
        let mut stream = self
            .handle
            .address()
            .get()
            .set_link_index_filter(index)
            .set_address_filter(IpAddr::V4(addr))
            .execute();
        while let Some(message) =
            stream.try_next().await.map_err(|err| NetlinkError::address("get", err))?
        {
            if message.header.prefix_len == prefix {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Converge `addr` on `iface` to exactly `prefix`.
    ///
    /// Any same-address entry with a different prefix is deleted before the
    /// exact entry is added. This repairs `/32`↔`/30` drift instead of treating
    /// a kernel `EEXIST` from a mismatched address as convergence success.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Address`] on dump/delete/add failure, or
    /// [`NetlinkError::LinkAbsent`] when `iface` has vanished. In particular,
    /// `ENODEV` is surfaced so the caller fails closed and retries from a new
    /// observation; it is never reported as successful convergence.
    pub async fn converge_addr(
        &self,
        iface: &str,
        addr: Ipv4Addr,
        prefix: u8,
    ) -> Result<(), NetlinkError> {
        let index = self.require_index(iface).await?;
        let mut stream = self
            .handle
            .address()
            .get()
            .set_link_index_filter(index)
            .set_address_filter(IpAddr::V4(addr))
            .execute();
        let mut exact_present = false;
        let mut mismatched = Vec::new();
        while let Some(message) =
            stream.try_next().await.map_err(|err| NetlinkError::address("get", err))?
        {
            if message.header.prefix_len == prefix {
                exact_present = true;
            } else {
                mismatched.push(message);
            }
        }
        drop(stream);

        for message in mismatched {
            self.handle
                .address()
                .del(message)
                .execute()
                .await
                .map_err(|err| NetlinkError::address("del-mismatched", err))?;
        }
        if !exact_present {
            self.handle
                .address()
                .add(index, IpAddr::V4(addr), prefix)
                .execute()
                .await
                .map_err(|err| NetlinkError::address("add-exact", err))?;
        }
        Ok(())
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

    /// Observe whether an exact IPv4 `<dst>/<prefix> dev <oif>` on-link route
    /// exists in the main table.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] on a route dump failure, or
    /// [`NetlinkError::LinkAbsent`] when `oif` has vanished.
    pub async fn observe_onlink_route(
        &self,
        dst: Ipv4Addr,
        prefix: u8,
        oif: &str,
    ) -> Result<bool, NetlinkError> {
        let index = self.require_index(oif).await?;
        let mut stream =
            self.handle.route().get(RouteMessageBuilder::<Ipv4Addr>::new().build()).execute();
        while let Some(route) =
            stream.try_next().await.map_err(|err| NetlinkError::route("get-onlink", err))?
        {
            let main_table = route.header.table == libc::RT_TABLE_MAIN
                || route.attributes.iter().any(|attr| {
                    matches!(attr, RouteAttribute::Table(value) if *value == u32::from(libc::RT_TABLE_MAIN))
                });
            let exact_prefix = route.header.destination_prefix_length == prefix;
            let exact_destination = route.attributes.iter().any(|attr| {
                matches!(attr, RouteAttribute::Destination(RouteAddress::Inet(value)) if *value == dst)
            });
            let exact_oif = route
                .attributes
                .iter()
                .any(|attr| matches!(attr, RouteAttribute::Oif(value) if *value == index));
            let has_gateway =
                route.attributes.iter().any(|attr| matches!(attr, RouteAttribute::Gateway(_)));
            if main_table && exact_prefix && exact_destination && exact_oif && !has_gateway {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ip route add <dst>/<prefix> via <gateway> dev <oif>` — a host return
    /// route whose next hop is reached over the allocation transit veth.
    /// `-EEXIST` surfaces through [`NetlinkError::errno`] for the convergence
    /// caller to treat as an idempotent success.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] on failure, or [`NetlinkError::LinkAbsent`]
    /// when the output interface has vanished.
    pub async fn add_route_via(
        &self,
        dst: Ipv4Addr,
        prefix: u8,
        gateway: Ipv4Addr,
        oif: &str,
    ) -> Result<(), NetlinkError> {
        let index = self.require_index(oif).await?;
        let route = RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(dst, prefix)
            .gateway(gateway)
            .output_interface(index)
            .build();
        self.handle
            .route()
            .add(route)
            .execute()
            .await
            .map_err(|err| NetlinkError::route("add-via", err))
    }

    /// Observe whether an exact IPv4 `<dst>/<prefix> via <gateway> dev
    /// <oif>` route exists.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] on a route dump failure, or
    /// [`NetlinkError::LinkAbsent`] when `oif` has vanished.
    pub async fn observe_route_via(
        &self,
        dst: Ipv4Addr,
        prefix: u8,
        gateway: Ipv4Addr,
        oif: &str,
    ) -> Result<bool, NetlinkError> {
        let index = self.require_index(oif).await?;
        let mut stream =
            self.handle.route().get(RouteMessageBuilder::<Ipv4Addr>::new().build()).execute();
        while let Some(route) =
            stream.try_next().await.map_err(|err| NetlinkError::route("get-via", err))?
        {
            let main_table = route.header.table == libc::RT_TABLE_MAIN
                || route.attributes.iter().any(|attr| {
                    matches!(attr, RouteAttribute::Table(value) if *value == u32::from(libc::RT_TABLE_MAIN))
                });
            let exact_prefix = route.header.destination_prefix_length == prefix;
            let exact_destination = route.attributes.iter().any(|attr| {
                matches!(attr, RouteAttribute::Destination(RouteAddress::Inet(value)) if *value == dst)
            });
            let exact_gateway = route.attributes.iter().any(|attr| {
                matches!(attr, RouteAttribute::Gateway(RouteAddress::Inet(value)) if *value == gateway)
            });
            let exact_oif = route
                .attributes
                .iter()
                .any(|attr| matches!(attr, RouteAttribute::Oif(value) if *value == index));
            if main_table && exact_prefix && exact_destination && exact_gateway && exact_oif {
                return Ok(true);
            }
        }
        Ok(false)
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
        let file = tokio::fs::File::open(format!("/var/run/netns/{netns_name}"))
            .await
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

    /// `ip rule add fwmark <fwmark> lookup <table>` — `RTM_NEWRULE`, the FIB
    /// policy rule routing fwmark-stamped packets via `table`.
    ///
    /// Netlink does **not** dedup FIB rules — a naked re-add
    /// (`NLM_F_EXCL | NLM_F_CREATE`) STACKS a duplicate, identical to iproute2
    /// `ip rule add` (spike increment-D). Callers MUST therefore guard with
    /// [`Client::fib_rule_fwmark_present`] first (the ported dump-then-add
    /// guard, ADR-0085 D6); this method does no presence check of its own.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] (`op = "rule-add"`) on failure.
    pub async fn add_fib_rule_fwmark(&self, fwmark: u32, table: u32) -> Result<(), NetlinkError> {
        self.handle
            .rule()
            .add()
            .v4()
            .action(RuleAction::ToTable)
            .fw_mark(fwmark)
            .table_id(table)
            .execute()
            .await
            .map_err(|err| NetlinkError::route("rule-add", err))
    }

    /// True iff a FIB rule matching BOTH `fwmark == fwmark` AND `lookup <table>`
    /// already exists — the structured replacement for the deleted
    /// `ip rule show` + `ip_rule_dump_has_fwmark` text scrape (ADR-0085 D6/D10).
    /// Dumps the v4 FIB rules (`RTM_GETRULE`) and applies the pure conjunction
    /// [`fib_rule_matches_fwmark_lookup`] to each, so the dump-then-add guard
    /// fires the add only when the rule is genuinely absent.
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] (`op = "rule-get"`) on a dump failure.
    pub async fn fib_rule_fwmark_present(
        &self,
        fwmark: u32,
        table: u32,
    ) -> Result<bool, NetlinkError> {
        let mut stream = self.handle.rule().get(IpVersion::V4).execute();
        while let Some(rule) =
            stream.try_next().await.map_err(|err| NetlinkError::route("rule-get", err))?
        {
            if fib_rule_matches_fwmark_lookup(&rule, fwmark, table) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ip route add local 0.0.0.0/0 dev <oif> table <table>` — `RTM_NEWROUTE`
    /// (kind `Local`, scope `Host`), the loopback catch-all route that delivers
    /// fwmark-redirected packets to a local socket instead of forwarding them.
    /// `-EEXIST` (already converged) surfaces via [`NetlinkError::errno`] for
    /// the caller to swallow as the idempotent success — the typed replacement
    /// for the deleted `stderr.contains("File exists")` check (ADR-0085 D6).
    ///
    /// # Errors
    ///
    /// [`NetlinkError::Route`] (`op = "local-add"`) on failure, or
    /// [`NetlinkError::LinkAbsent`] when `oif` has vanished.
    pub async fn add_local_route(&self, table: u32, oif: &str) -> Result<(), NetlinkError> {
        let index = self.require_index(oif).await?;
        let route = RouteMessageBuilder::<Ipv4Addr>::new()
            .kind(RouteType::Local)
            .scope(RouteScope::Host)
            .table_id(table)
            .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
            .output_interface(index)
            .build();
        self.handle
            .route()
            .add(route)
            .execute()
            .await
            .map_err(|err| NetlinkError::route("local-add", err))
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
    /// (`-ENODEV`, surfaced by create/add/up callers and accepted only by
    /// delete) — for ops that require the link to exist (addr / up / route).
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

/// Classify one typed `RTM_GETLINK` reply as the exact persistent TAP or an
/// incompatible same-name link.
fn persistent_tap_state(message: &LinkMessage) -> TapLinkState {
    let mut kind_is_tun = false;
    let mut tun_type = None;
    let mut persistent = None;
    let mut owner_uid = None;

    for attribute in &message.attributes {
        let LinkAttribute::LinkInfo(infos) = attribute else {
            continue;
        };
        for info in infos {
            match info {
                LinkInfo::Kind(InfoKind::Tun) => kind_is_tun = true,
                LinkInfo::Data(InfoData::Tun(tun_infos)) => {
                    for tun_info in tun_infos {
                        let InfoTun::Other(nla) = tun_info else {
                            continue;
                        };
                        match nla.kind() & NLA_TYPE_MASK {
                            IFLA_TUN_TYPE => tun_type = nla_u8(nla),
                            IFLA_TUN_PERSIST => persistent = nla_u8(nla),
                            IFLA_TUN_OWNER => owner_uid = nla_u32(nla),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let is_tap = tun_type == u8::try_from(libc::IFF_TAP).ok();
    if kind_is_tun && is_tap && persistent == Some(1) {
        TapLinkState::Persistent { up: message.header.flags.contains(LinkFlags::Up), owner_uid }
    } else {
        TapLinkState::Incompatible
    }
}

/// Read the one-byte payload used by the kernel's `IFLA_TUN_*` attributes.
fn nla_u8(nla: &impl Nla) -> Option<u8> {
    if nla.value_len() != 1 {
        return None;
    }
    let mut value = [0_u8; 1];
    nla.emit_value(&mut value);
    Some(value[0])
}

fn nla_u32(nla: &impl Nla) -> Option<u32> {
    if nla.value_len() != 4 {
        return None;
    }
    let mut value = [0_u8; 4];
    nla.emit_value(&mut value);
    Some(u32::from_ne_bytes(value))
}

/// Map an `RTM_GETLINK` error to `Ok(None)` when it is `-ENODEV` (absent),
/// else surface it as a typed [`NetlinkError::Link`].
fn absent_or_err(op: &'static str, err: rtnetlink::Error) -> Result<Option<bool>, NetlinkError> {
    let typed = NetlinkError::link(op, err);
    if typed.errno() == Some(NEG_ENODEV) { Ok(None) } else { Err(typed) }
}

/// Pure: does this dumped FIB rule (from an `RTM_GETRULE` reply) match BOTH
/// `fwmark == mark` AND lookup `table`? The netlink analogue of the deleted
/// `ip_rule_dump_has_fwmark` text conjunction (ADR-0085 D6/D10): both conjuncts
/// must hold on the SAME rule, else a rule that fwmark-matches but routes
/// elsewhere — or one that looks up our table for a different mark — would be
/// mistaken for the rule we ensure and [`Client::add_fib_rule_fwmark`] would be
/// wrongly skipped, leaving the fwmark unrouted (and, on a stacked re-add, the
/// spike-D duplicate). The table lands in `header.table` when it is ≤ 255
/// (table 100 is) and in the `FRA_TABLE` attribute when it is larger (or when
/// the kernel emits it there anyway), so both are checked (spike-D
/// `count_fwmark_table_rules`).
fn fib_rule_matches_fwmark_lookup(rule: &RuleMessage, mark: u32, table: u32) -> bool {
    let mut has_mark = false;
    let mut has_table = u32::from(rule.header.table) == table;
    for attr in &rule.attributes {
        match attr {
            RuleAttribute::FwMark(m) if *m == mark => has_mark = true,
            RuleAttribute::Table(t) if *t == table => has_table = true,
            _ => {}
        }
    }
    has_mark && has_table
}

#[cfg(test)]
mod tests {
    use rtnetlink::packet_core::DefaultNla;
    use rtnetlink::packet_route::link::{
        InfoData, InfoKind, InfoTun, LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
    };
    use rtnetlink::packet_route::rule::{RuleAttribute, RuleMessage};

    use super::{
        IFLA_TUN_OWNER, IFLA_TUN_PERSIST, IFLA_TUN_TYPE, TapLinkState,
        fib_rule_matches_fwmark_lookup, persistent_tap_state,
    };

    fn tun_link(tun_type: u8, persistent: u8, owner_uid: u32) -> LinkMessage {
        let mut message = LinkMessage::default();
        message.header.flags = LinkFlags::Up;
        message.attributes.push(LinkAttribute::LinkInfo(vec![
            LinkInfo::Kind(InfoKind::Tun),
            LinkInfo::Data(InfoData::Tun(vec![
                InfoTun::Other(DefaultNla::new(IFLA_TUN_TYPE, vec![tun_type])),
                InfoTun::Other(DefaultNla::new(IFLA_TUN_PERSIST, vec![persistent])),
                InfoTun::Other(DefaultNla::new(IFLA_TUN_OWNER, owner_uid.to_ne_bytes().to_vec())),
            ])),
        ]));
        message
    }

    /// Typed TAP classification requires all three kernel facts: tun-kind,
    /// `IFF_TAP`, and persistence. Same-kind TUN or non-persistent TAP states
    /// remain incompatible.
    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    fn persistent_tap_classifier_requires_tap_type_and_persistence() {
        let iff_tap = u8::try_from(libc::IFF_TAP).expect("IFF_TAP fits the kernel u8 NLA");
        let iff_tun = u8::try_from(libc::IFF_TUN).expect("IFF_TUN fits the kernel u8 NLA");
        assert_eq!(
            persistent_tap_state(&tun_link(iff_tap, 1, 4_200)),
            TapLinkState::Persistent { up: true, owner_uid: Some(4_200) },
        );
        assert_eq!(persistent_tap_state(&tun_link(iff_tap, 0, 4_200)), TapLinkState::Incompatible,);
        assert_eq!(persistent_tap_state(&tun_link(iff_tun, 1, 4_200)), TapLinkState::Incompatible,);
    }

    /// Build a dumped FIB rule with an optional `fwmark` (the `FRA_FWMARK`
    /// attribute), a `header.table` byte (where table ≤ 255 lands), and an
    /// optional `FRA_TABLE` attribute (where table > 255 lands). Mirrors the
    /// two places the kernel reports the rule's table in an `RTM_GETRULE`
    /// reply (spike-D `count_fwmark_table_rules`).
    fn rule(fwmark: Option<u32>, header_table: u8, attr_table: Option<u32>) -> RuleMessage {
        let mut msg = RuleMessage::default();
        msg.header.table = header_table;
        if let Some(mark) = fwmark {
            msg.attributes.push(RuleAttribute::FwMark(mark));
        }
        if let Some(table) = attr_table {
            msg.attributes.push(RuleAttribute::Table(table));
        }
        msg
    }

    #[test]
    fn fwmark_rule_matched_with_table_in_header() {
        // table 100 ≤ 255 lands in `header.table`; fwmark in `FRA_FWMARK`.
        let r = rule(Some(1), 100, None);
        assert!(
            fib_rule_matches_fwmark_lookup(&r, 1, 100),
            "a rule marking on fwmark 0x1 with the header carrying table 100 must match"
        );
    }

    #[test]
    fn fwmark_rule_matched_with_table_in_attribute() {
        // A kernel that emits `FRA_TABLE` (or a table > 255) carries it in the
        // attribute, not the header byte.
        let r = rule(Some(1), 0, Some(100));
        assert!(
            fib_rule_matches_fwmark_lookup(&r, 1, 100),
            "a rule marking on fwmark 0x1 with table 100 in the FRA_TABLE attr must match"
        );
    }

    #[test]
    fn requires_both_fwmark_and_table_on_the_same_rule() {
        // The netlink analogue of the deleted
        // `ip_rule_fwmark_requires_both_conjuncts_on_the_same_line`, and the
        // load-bearing guard whose loss reintroduces the spike-D duplicate.
        // Rule A marks on OUR fwmark (0x1) but looks up a DIFFERENT table
        // (200); rule B looks up OUR table (100) but marks a DIFFERENT fwmark
        // (0x2). NEITHER single rule both marks on 0x1 AND looks up 100, so the
        // per-rule predicate must read false for each — the dump-then-add guard
        // then correctly reports "absent" and the `add_fib_rule_fwmark` fires
        // exactly once. Under an `&& -> ||` mutant, rule A would satisfy the
        // fwmark conjunct and rule B the table conjunct, wrongly reporting the
        // rule present and skipping the add (leaving the fwmark unrouted).
        let rule_a_wrong_table = rule(Some(1), 200, None);
        let rule_b_wrong_mark = rule(Some(2), 100, None);
        assert!(
            !fib_rule_matches_fwmark_lookup(&rule_a_wrong_table, 1, 100),
            "our fwmark but the wrong lookup table must NOT match (both conjuncts required)"
        );
        assert!(
            !fib_rule_matches_fwmark_lookup(&rule_b_wrong_mark, 1, 100),
            "our lookup table but the wrong fwmark must NOT match (both conjuncts required)"
        );
    }

    #[test]
    fn vanilla_rule_with_neither_conjunct_reads_absent() {
        // A vanilla `main`-table policy rule (no fwmark, table 254) must read
        // absent so the guard lets the `add_fib_rule_fwmark` fire.
        let r = rule(None, 254, None);
        assert!(
            !fib_rule_matches_fwmark_lookup(&r, 1, 100),
            "a rule carrying neither our fwmark nor our lookup table must read absent"
        );
    }

    #[test]
    fn table_attribute_value_is_compared_not_merely_present() {
        // Our fwmark, but the FRA_TABLE attribute carries the WRONG table (200,
        // not the 100 we look up) and the header byte is neutral. The
        // `RuleAttribute::Table(t) if *t == table` value guard is load-bearing:
        // without it, ANY Table attribute would satisfy the lookup conjunct and
        // a rule that marks on our fwmark but routes to a different table would
        // be mistaken for ours (skipping the add, leaving the fwmark unrouted).
        let wrong_table = rule(Some(1), 0, Some(200));
        assert!(
            !fib_rule_matches_fwmark_lookup(&wrong_table, 1, 100),
            "a Table attr with a DIFFERENT value must not satisfy the lookup conjunct"
        );
        // The matching value still matches (guards against an over-strict flip).
        let right_table = rule(Some(1), 0, Some(100));
        assert!(fib_rule_matches_fwmark_lookup(&right_table, 1, 100));
    }
}
