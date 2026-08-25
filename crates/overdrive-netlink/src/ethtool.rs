//! Hand-rolled ethtool `FEATURES_SET` encoder over `NETLINK_GENERIC`
//! (ADR-0085 D1/D8; spike increment-b, WORKS on a real kernel).
//!
//! The `ethtool` crate is **GET-only** — its `EthtoolFeatureAttr::Wanted`
//! emit is `todo!("Does not support changing ethtool feature yet")`, so it
//! **panics** if reused for a SET. The SET path is therefore hand-encoded at
//! the wire level here, over a raw `NETLINK_GENERIC` socket.
//!
//! Mechanism (matching `ethtool -K <dev> tx off` semantics the netlink way,
//! named-bit-granular):
//!
//! 1. `FEATURES_GET` (via the `ethtool` crate) enumerates the **changeable**
//!    `tx-checksum-*` bits (the `Hw` mask) and their current active state.
//! 2. `FEATURES_SET` (hand-rolled) requests each changeable `tx-checksum-*`
//!    bit OFF through the `ETHTOOL_A_FEATURES_WANTED` bitset — a bit present
//!    with **no** `ETHTOOL_A_BITSET_BIT_VALUE` ⇒ target value 0 (off).
//!
//! Only changeable bits are set, so a fixed feature (which already delivers a
//! FULL checksum) is simply never in the request — the netlink-native
//! equivalent of the old `tx_offload_benign` "feature is fixed" swallow, with
//! no string matching.
//!
//! **The constant `ETHTOOL_MSG_FEATURES_SET = 0x0c` is load-bearing** — the
//! issue brief's `0x0a` is `WOL_SET` (spike increment-b correction). Getting
//! it wrong silently corrupts every NAT'd packet (commit 62fa6be2,
//! `.claude/rules/bpf.md` Rule 2), so it is pinned and asserted below.

// Raw netlink wire encoding: attribute lengths, message lengths, and the
// kernel `NLMSG_ERROR` code are bounded byte-boundary values where the `as`
// truncation / sign reinterpretation is the intended wire semantics (an
// individual attribute never exceeds `u16`; the error code is a NACK errno).
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use std::collections::{BTreeMap, BTreeSet};

use futures::stream::TryStreamExt;

use crate::error::NetlinkError;

// ---- ethtool_netlink.h constants (pinned; spike increment-b) ----------------
const NETLINK_GENERIC: i32 = 16;
const GENL_ID_CTRL: u16 = 16;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const NLM_F_REQUEST: u16 = 0x01;
const NLM_F_ACK: u16 = 0x04;
const NLMSG_ERROR: u16 = 0x02;
const NLA_F_NESTED: u16 = 0x8000;

/// `ETHTOOL_MSG_FEATURES_SET`. **12 / `0x0c`, NOT `0x0a`** (which is
/// `WOL_SET`) — the issue-brief correction the spike pins.
const ETHTOOL_MSG_FEATURES_SET: u8 = 12;
const ETHTOOL_A_FEATURES_HEADER: u16 = 1;
const ETHTOOL_A_FEATURES_WANTED: u16 = 3;
const ETHTOOL_A_HEADER_DEV_NAME: u16 = 2;
const ETHTOOL_A_BITSET_BITS: u16 = 3;
const ETHTOOL_A_BITSET_BITS_BIT: u16 = 1;
const ETHTOOL_A_BITSET_BIT_NAME: u16 = 2;
/// `ETHTOOL_A_BITSET_BIT_VALUE` — present ⇒ target value 1 (on). We NEVER
/// emit it (the "off" invariant); named here only so the encoder test can
/// assert its **absence**, hence `#[cfg(test)]`.
#[cfg(test)]
const ETHTOOL_A_BITSET_BIT_VALUE: u16 = 3;

/// The feature-name prefix whose bits `tx off` clears. `ethtool -K <dev> tx
/// off` expands to the `tx-checksum-*` group; we drive that group
/// named-bit-granular over netlink.
const TX_CHECKSUM_PREFIX: &str = "tx-checksum";

// =============================================================================
// Pure derivations (default-lane unit-testable — no I/O)
// =============================================================================

/// The `FEATURES_SET` targets: the **changeable** `tx-checksum-*` bit names.
///
/// The `Hw` mask ∩ the `tx-checksum-*` group. A fixed `tx-checksum-*` bit is
/// excluded — it already delivers a FULL checksum, so setting it would be
/// rejected; excluding it is the netlink-native equivalent of the old
/// "feature is fixed" swallow. Deterministic order (sorted) for a stable
/// wire encoding.
#[must_use]
pub fn changeable_tx_checksum_targets(changeable: &BTreeSet<String>) -> Vec<String> {
    changeable.iter().filter(|name| name.starts_with(TX_CHECKSUM_PREFIX)).cloned().collect()
}

/// True when ANY changeable `tx-checksum-*` bit is currently ACTIVE (on).
///
/// The observe predicate. After a successful disable every changeable bit is
/// off ⇒ `false` (converge emits no disable ⇒ idempotent); after a drift
/// (`ethtool -K … tx on`) a bit is on ⇒ `true` (converge emits the disable ⇒
/// repaired). Only changeable bits count: a fixed bit's state is immutable
/// and never needs (or admits) a disable.
#[must_use]
pub fn any_tx_checksum_active(
    active: &BTreeMap<String, bool>,
    changeable: &BTreeSet<String>,
) -> bool {
    changeable
        .iter()
        .filter(|name| name.starts_with(TX_CHECKSUM_PREFIX))
        .any(|name| active.get(name).copied().unwrap_or(false))
}

/// Encode the `FEATURES_SET` genl payload that requests each `name` OFF.
///
/// An `ETHTOOL_A_FEATURES_HEADER` (dev name) + an `ETHTOOL_A_FEATURES_WANTED`
/// bitset in which each name is a `BITS_BIT` carrying only `BIT_NAME` — **no
/// `ETHTOOL_A_BITSET_BIT_VALUE`**, which is exactly what requests the bit to
/// 0 (off). Pure (no I/O), so the wire layout is unit-testable in the default
/// lane without a kernel.
#[must_use]
pub fn encode_features_set_off_payload(dev: &str, names: &[String]) -> Vec<u8> {
    let mut header = Vec::new();
    nla(&mut header, ETHTOOL_A_HEADER_DEV_NAME, &cstr(dev));

    let mut bits = Vec::new();
    for name in names {
        let mut bit = Vec::new();
        nla(&mut bit, ETHTOOL_A_BITSET_BIT_NAME, &cstr(name));
        // Deliberately NO ETHTOOL_A_BITSET_BIT_VALUE => request bit -> 0 (off).
        nla(&mut bits, ETHTOOL_A_BITSET_BITS_BIT | NLA_F_NESTED, &bit);
    }
    let mut bitset = Vec::new();
    nla(&mut bitset, ETHTOOL_A_BITSET_BITS | NLA_F_NESTED, &bits);

    let mut payload = Vec::new();
    nla(&mut payload, ETHTOOL_A_FEATURES_HEADER | NLA_F_NESTED, &header);
    nla(&mut payload, ETHTOOL_A_FEATURES_WANTED | NLA_F_NESTED, &bitset);
    payload
}

/// A NUL-terminated byte string (netlink `NLA_NUL_STRING`).
fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

/// Append one netlink attribute (native-endian len/type header, 4-byte
/// padded) to `buf`.
fn nla(buf: &mut Vec<u8>, ty: u16, val: &[u8]) {
    let len = (4 + val.len()) as u16;
    buf.extend_from_slice(&len.to_ne_bytes());
    buf.extend_from_slice(&ty.to_ne_bytes());
    buf.extend_from_slice(val);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

// =============================================================================
// Impure netlink I/O — the async public surface
// =============================================================================

/// Disable TX-checksum offload on `iface` the netlink way.
///
/// Enumerate the changeable `tx-checksum-*` bits (`FEATURES_GET`) then request
/// each OFF (`FEATURES_SET`). A veth with NO changeable `tx-checksum-*` bit
/// already delivers a FULL checksum, so the SET is skipped as a no-op success.
///
/// # Errors
///
/// [`NetlinkError::Ethtool`] on a socket failure, a genl-family resolution
/// failure, or a kernel `NLMSG_ERROR` on the SET (e.g. `EPERM`). The caller
/// (`veth_provisioner`) treats this as FATAL and refuses the boot: booting
/// with offload still ON corrupts every NAT'd packet (commit 62fa6be2).
pub async fn disable_tx_offload(iface: &str) -> Result<(), NetlinkError> {
    let (_active, changeable) = feature_snapshot(iface).await?;
    let targets = changeable_tx_checksum_targets(&changeable);
    if targets.is_empty() {
        // No changeable tx-checksum-* feature on this iface — it already
        // delivers a full checksum; nothing to disable.
        return Ok(());
    }
    let mut sock = GenlSock::open().map_err(|e| NetlinkError::ethtool("features-set-socket", e))?;
    let family =
        sock.resolve_family("ethtool").map_err(|e| NetlinkError::ethtool("resolve-family", e))?;
    sock.features_set_off(family, iface, &targets)
        .map_err(|e| NetlinkError::ethtool("features-set", e))
}

/// Read whether TX-checksum offload is currently ON for `iface`.
///
/// True iff any changeable `tx-checksum-*` bit is active (the structured
/// replacement for the old `ethtool -k` `tx-checksumming:` text parse).
/// Drives the converge observer: after a disable it reads `false`
/// (idempotent); after a drift-back-on it reads `true` (repair).
///
/// # Errors
///
/// [`NetlinkError::Ethtool`] on a `FEATURES_GET` failure.
pub async fn tx_offload_on(iface: &str) -> Result<bool, NetlinkError> {
    let (active, changeable) = feature_snapshot(iface).await?;
    Ok(any_tx_checksum_active(&active, &changeable))
}

/// `FEATURES_GET` via the `ethtool` crate: `(active-state map, changeable
/// name set)`. The changeable set is the `Hw` mask (a bit whose `value` is
/// `true` is user-changeable).
async fn feature_snapshot(
    iface: &str,
) -> Result<(BTreeMap<String, bool>, BTreeSet<String>), NetlinkError> {
    use ethtool::{EthtoolAttr, EthtoolFeatureAttr};

    let (conn, mut handle, _) =
        ethtool::new_connection().map_err(|e| NetlinkError::ethtool("features-get-connect", e))?;
    let conn_task = tokio::spawn(conn);

    let mut active: BTreeMap<String, bool> = BTreeMap::new();
    let mut changeable: BTreeSet<String> = BTreeSet::new();
    let result = async {
        let mut stream = Box::pin(handle.feature().get(Some(iface)).execute().await);
        while let Some(msg) = stream.try_next().await.map_err(|e| {
            NetlinkError::ethtool("features-get", std::io::Error::other(e.to_string()))
        })? {
            for attr in &msg.payload.nlas {
                if let EthtoolAttr::Feature(feature) = attr {
                    match feature {
                        EthtoolFeatureAttr::Active(bits) => {
                            for bit in bits {
                                active.insert(bit.name.clone(), bit.value);
                            }
                        }
                        // `Hw` is the user-changeable mask: value=true ⇒ changeable.
                        EthtoolFeatureAttr::Hw(bits) => {
                            for bit in bits {
                                if bit.value {
                                    changeable.insert(bit.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok::<(), NetlinkError>(())
    }
    .await;

    drop(handle);
    conn_task.abort();
    result.map(|()| (active, changeable))
}

/// A raw `NETLINK_GENERIC` socket for the hand-rolled `FEATURES_SET` request
/// (spike increment-b's proven `GenlSock`).
struct GenlSock {
    fd: i32,
    seq: u32,
}

impl GenlSock {
    fn open() -> std::io::Result<Self> {
        // SAFETY: `socket(2)` with valid domain/type/protocol constants; the
        // returned fd is checked and owned (closed in `Drop`).
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_GENERIC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        // SAFETY: `addr` is a zeroed, correctly-typed `sockaddr_nl` of the
        // size passed; `fd` is a valid netlink socket.
        let rc = unsafe {
            libc::bind(
                fd,
                std::ptr::addr_of!(addr).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            // SAFETY: `fd` is a valid open descriptor.
            unsafe { libc::close(fd) };
            return Err(err);
        }
        Ok(Self { fd, seq: 1 })
    }

    /// Send a genl request (`nlmsghdr` + `genlmsghdr` + payload) and return
    /// the raw reply bytes.
    fn request(
        &mut self,
        family: u16,
        cmd: u8,
        version: u8,
        flags: u16,
        payload: &[u8],
    ) -> std::io::Result<Vec<u8>> {
        self.seq += 1;
        let mut genl = Vec::new();
        genl.push(cmd);
        genl.push(version);
        genl.extend_from_slice(&0u16.to_ne_bytes()); // reserved
        genl.extend_from_slice(payload);

        let total = 16 + genl.len();
        let mut msg = Vec::with_capacity(total);
        msg.extend_from_slice(&(total as u32).to_ne_bytes());
        msg.extend_from_slice(&family.to_ne_bytes());
        msg.extend_from_slice(&flags.to_ne_bytes());
        msg.extend_from_slice(&self.seq.to_ne_bytes());
        msg.extend_from_slice(&0u32.to_ne_bytes()); // pid=0 => kernel assigns
        msg.extend_from_slice(&genl);

        let mut dst: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        dst.nl_family = libc::AF_NETLINK as libc::sa_family_t; // nl_pid=0 => kernel
        // SAFETY: `msg` is a valid initialised buffer of the passed length;
        // `dst` is a zeroed, correctly-sized `sockaddr_nl`; `self.fd` is open.
        let sent = unsafe {
            libc::sendto(
                self.fd,
                msg.as_ptr().cast::<libc::c_void>(),
                msg.len(),
                0,
                std::ptr::addr_of!(dst).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if sent < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buf = vec![0u8; 16384];
        // SAFETY: `buf` is a valid initialised buffer of `buf.len()`; `self.fd`
        // is open.
        let n =
            unsafe { libc::recv(self.fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        buf.truncate(n as usize);
        Ok(buf)
    }

    /// Resolve a genl family id by name via `CTRL_CMD_GETFAMILY`.
    fn resolve_family(&mut self, name: &str) -> std::io::Result<u16> {
        let mut payload = Vec::new();
        nla(&mut payload, CTRL_ATTR_FAMILY_NAME, &cstr(name));
        let reply = self.request(GENL_ID_CTRL, CTRL_CMD_GETFAMILY, 1, NLM_F_REQUEST, &payload)?;
        if reply.len() < 20 {
            return Err(std::io::Error::other("short GETFAMILY reply"));
        }
        let msg_len = ne_u32(&reply, 0) as usize;
        let mut off = 20; // nlmsghdr(16) + genlmsghdr(4)
        while off + 4 <= msg_len.min(reply.len()) {
            let alen = ne_u16(&reply, off) as usize;
            let atype = ne_u16(&reply, off + 2) & 0x7fff;
            if alen < 4 {
                break;
            }
            if atype == CTRL_ATTR_FAMILY_ID && off + 6 <= reply.len() {
                return Ok(ne_u16(&reply, off + 4));
            }
            off += (alen + 3) & !3;
        }
        Err(std::io::Error::other("CTRL_ATTR_FAMILY_ID not found in GETFAMILY reply"))
    }

    /// `FEATURES_SET` each named bit OFF. `Ok(())` on a netlink ACK
    /// (`error == 0`) or a non-error reply; `Err(io::Error)` carrying the
    /// kernel errno on a NACK.
    fn features_set_off(
        &mut self,
        family: u16,
        dev: &str,
        names: &[String],
    ) -> std::io::Result<()> {
        let payload = encode_features_set_off_payload(dev, names);
        let reply =
            self.request(family, ETHTOOL_MSG_FEATURES_SET, 1, NLM_F_REQUEST | NLM_F_ACK, &payload)?;
        if reply.len() < 16 {
            return Err(std::io::Error::other("short FEATURES_SET reply"));
        }
        let ty = ne_u16(&reply, 4);
        if ty == NLMSG_ERROR {
            // nlmsghdr(16) then the error `i32` (negative errno, or 0 for ACK).
            let err = ne_u32(&reply, 16) as i32;
            if err == 0 {
                return Ok(());
            }
            return Err(std::io::Error::from_raw_os_error(err.abs()));
        }
        // A non-error reply (the kernel echoed a FEATURES_SET reply) is also
        // an accept.
        Ok(())
    }
}

impl Drop for GenlSock {
    fn drop(&mut self) {
        // SAFETY: `self.fd` is a valid descriptor opened in `open` and not
        // otherwise closed.
        unsafe { libc::close(self.fd) };
    }
}

fn ne_u16(b: &[u8], o: usize) -> u16 {
    u16::from_ne_bytes([b[o], b[o + 1]])
}
fn ne_u32(b: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// `ETHTOOL_MSG_FEATURES_SET` is `0x0c` (12), NOT `0x0a` (`WOL_SET`). The
    /// packet-corruption-critical pin (spike increment-b correction).
    #[test]
    fn features_set_command_is_0x0c_not_0x0a() {
        assert_eq!(ETHTOOL_MSG_FEATURES_SET, 0x0c);
        assert_ne!(ETHTOOL_MSG_FEATURES_SET, 0x0a);
    }

    /// Walk a `BITS` nested attribute, returning `(bit_name, has_bit_value)`
    /// for each `BITS_BIT`. Used to assert the "off" invariant on the encoded
    /// WANTED bitset.
    fn parse_wanted_bits(payload: &[u8]) -> Vec<(String, bool)> {
        // FEATURES_WANTED -> BITSET_BITS -> [BITS_BIT -> {BIT_NAME, BIT_VALUE?}]
        let wanted = find_nested(payload, ETHTOOL_A_FEATURES_WANTED)
            .expect("payload must carry ETHTOOL_A_FEATURES_WANTED");
        let bits = find_nested(&wanted, ETHTOOL_A_BITSET_BITS)
            .expect("WANTED bitset must carry ETHTOOL_A_BITSET_BITS");
        let mut out = Vec::new();
        for bit in each_attr(&bits, ETHTOOL_A_BITSET_BITS_BIT) {
            let name_bytes = find_attr(&bit, ETHTOOL_A_BITSET_BIT_NAME)
                .expect("each BITS_BIT carries a BIT_NAME");
            let name = std::ffi::CStr::from_bytes_until_nul(&name_bytes)
                .expect("NUL-terminated bit name")
                .to_string_lossy()
                .into_owned();
            let has_value = find_attr(&bit, ETHTOOL_A_BITSET_BIT_VALUE).is_some();
            out.push((name, has_value));
        }
        out
    }

    /// Return the (nested) payload of the first attr of `ty`, mask-stripping
    /// `NLA_F_NESTED`.
    fn find_nested(buf: &[u8], ty: u16) -> Option<Vec<u8>> {
        find_attr(buf, ty)
    }

    fn find_attr(buf: &[u8], ty: u16) -> Option<Vec<u8>> {
        each_attr(buf, ty).into_iter().next()
    }

    /// All attr payloads of type `ty` (matching after masking `NLA_F_NESTED`).
    fn each_attr(buf: &[u8], ty: u16) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut off = 0;
        while off + 4 <= buf.len() {
            let alen = u16::from_ne_bytes([buf[off], buf[off + 1]]) as usize;
            let atype = u16::from_ne_bytes([buf[off + 2], buf[off + 3]]) & 0x7fff;
            if alen < 4 || off + alen > buf.len() {
                break;
            }
            if atype == (ty & 0x7fff) {
                out.push(buf[off + 4..off + alen].to_vec());
            }
            off += (alen + 3) & !3;
        }
        out
    }

    prop_compose! {
        /// A set of distinct `tx-checksum-*`-shaped bit names.
        fn tx_checksum_names()(
            suffixes in prop::collection::btree_set("[a-z]{1,8}", 0..6)
        ) -> Vec<String> {
            suffixes.into_iter().map(|s| format!("tx-checksum-{s}")).collect()
        }
    }

    proptest! {
        // The encoded WANTED bitset lists EXACTLY the requested names, each
        // as a `BIT_NAME` with NO `BIT_VALUE` — the "off" invariant
        // (`BIT_VALUE` absent ⇒ target 0). This is the pure derivation
        // ADR-0085 / the roadmap name; a mutation that emits a `BIT_VALUE`
        // (turning the request into "on") is killed here.
        #[test]
        fn encoded_wanted_bits_request_off_for_each_name(names in tx_checksum_names()) {
            let payload = encode_features_set_off_payload("ovd-veth-cli", &names);
            let parsed = parse_wanted_bits(&payload);
            let parsed_names: Vec<String> = parsed.iter().map(|(n, _)| n.clone()).collect();
            prop_assert_eq!(&parsed_names, &names, "every requested name must appear once, in order");
            prop_assert!(
                parsed.iter().all(|(_, has_value)| !has_value),
                "NO bit may carry BIT_VALUE — its absence is what requests the bit OFF",
            );
        }
    }

    /// The SET targets are exactly the changeable `tx-checksum-*` names — a
    /// non-`tx-checksum-*` changeable bit (`rx-checksum`, `tso`) is excluded,
    /// and a fixed `tx-checksum-*` bit (absent from the changeable set) is
    /// excluded.
    #[test]
    fn targets_are_changeable_tx_checksum_bits_only() {
        let changeable: BTreeSet<String> = [
            "tx-checksum-ip-generic",
            "tx-checksum-sctp",
            "rx-checksum",       // changeable but not tx-checksum-*
            "tx-scatter-gather", // changeable but not tx-checksum-*
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let targets = changeable_tx_checksum_targets(&changeable);
        assert_eq!(
            targets,
            vec!["tx-checksum-ip-generic".to_string(), "tx-checksum-sctp".to_string()],
        );
    }

    /// `any_tx_checksum_active` is true iff a CHANGEABLE `tx-checksum-*` bit
    /// is active — a fixed (non-changeable) tx-checksum bit that is on does
    /// NOT count (it cannot be, and need not be, disabled).
    #[test]
    fn observe_ignores_fixed_bits_and_reads_changeable_active_state() {
        let changeable: BTreeSet<String> =
            std::iter::once("tx-checksum-ip-generic").map(String::from).collect();
        let mut active: BTreeMap<String, bool> = BTreeMap::new();
        active.insert("tx-checksum-ipv4".to_string(), true); // fixed & on: ignored
        assert!(!any_tx_checksum_active(&active, &changeable), "fixed on-bit is ignored");
        active.insert("tx-checksum-ip-generic".to_string(), true); // changeable & on
        assert!(any_tx_checksum_active(&active, &changeable), "changeable on-bit counts");
        active.insert("tx-checksum-ip-generic".to_string(), false); // disabled
        assert!(!any_tx_checksum_active(&active, &changeable), "changeable off-bit ⇒ not on");
    }
}
