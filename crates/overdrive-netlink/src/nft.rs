//! Hand-rolled nftables encoder over raw `NETLINK_NETFILTER` (ADR-0085
//! D1/D2/D7; spike increment-e, WORKS on a real kernel — the connection-divert
//! proof in `spike/findings-e.md`).
//!
//! This is the single auditable home for ALL nftables kernel wire encoding
//! (ADR-0085 D2). `rustables` is NOT used: it has **no typed `tproxy`
//! expression** and **no public raw-expression escape hatch** (its `nlmsg`
//! module is `pub(crate)`, `ExpressionRaw`'s field is private), so it
//! structurally cannot express the load-bearing verb — and it drags a
//! `bindgen` 0.72 + `libclang` build dependency. The whole nft path is
//! therefore hand-rolled here (§ "Alternatives Considered" in ADR-0085).
//!
//! # Wire byte-order discipline (load-bearing)
//!
//! Every nft integer **attribute value** is BIG-endian on the wire (the kernel
//! reads them via `nla_get_be32` / `nla_get_be64`). Netlink **message and
//! attribute headers** (`len`, `type`) are HOST byte order (native). Getting a
//! flip wrong silently mis-diverts packets with no compile error — the
//! golden-bytes unit test ([`tests::tproxy_expr_matches_findings_e_pin`]) and
//! the real-divert Tier-3 ATs are the guards.
//!
//! # Structural rule identity — `NFTA_RULE_USERDATA`
//!
//! Handle recovery + the §5 boot sweep are STRUCTURAL, not a `# handle N` text
//! scrape (ADR-0085 D10). Each rule this module installs carries an
//! `NFTA_RULE_USERDATA` tag ([`userdata_inbound`] / [`userdata_egress`] /
//! [`userdata_output_divert`] / [`userdata_exemption`]) with the
//! [`USERDATA_MAGIC`] prefix and a `kind` discriminator byte. The GETRULE dump
//! reply carries each rule's `(NFTA_RULE_HANDLE, NFTA_RULE_USERDATA)` back, so:
//!
//! - per-rule handle recovery is [`handle_for_userdata`] (the exact tag → its
//!   kernel handle);
//! - the port-blind §5 sweep is [`workload_rule_handles`] (every per-workload
//!   `kind`, never the shared exemption);
//! - the head-exemption idempotence guard is [`has_exemption`].
//!
//! The tag is a sibling rule attribute (`NFTA_RULE_USERDATA`), wholly separate
//! from `NFTA_RULE_EXPRESSIONS`, so it does NOT perturb the pinned `tproxy`
//! expression bytes.

// Raw netlink wire encoding: attribute lengths, message lengths, and the
// kernel `NLMSG_ERROR` code are bounded byte-boundary values where the `as`
// truncation / sign reinterpretation is the intended wire semantics (an
// individual attribute never exceeds `u16`; the error code is a NACK errno).
// Mirrors the module-level allow on the sibling `ethtool` encoder.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use crate::error::{NEG_EEXIST, NetlinkError};

// ---- NETLINK_NETFILTER / nfnetlink framing (pinned; spike increment-e) ------
const NETLINK_NETFILTER: i32 = 12;
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFNL_MSG_BATCH_BEGIN: u16 = 16;
const NFNL_MSG_BATCH_END: u16 = 17;

// nf_tables message ops (`enum nf_tables_msg_types`).
const NFT_MSG_NEWTABLE: u16 = 0;
const NFT_MSG_GETTABLE: u16 = 1;
const NFT_MSG_DELTABLE: u16 = 2;
const NFT_MSG_NEWCHAIN: u16 = 3;
const NFT_MSG_GETCHAIN: u16 = 4;
const NFT_MSG_DELCHAIN: u16 = 5;
const NFT_MSG_NEWRULE: u16 = 6;
const NFT_MSG_GETRULE: u16 = 7;
const NFT_MSG_DELRULE: u16 = 8;
const NFT_MSG_NEWGEN: u16 = 15;
const NFT_MSG_GETGEN: u16 = 16;

// netlink message flags.
const NLM_F_REQUEST: u16 = 0x001;
const NLM_F_ACK: u16 = 0x004;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_DUMP: u16 = 0x300; // NLM_F_ROOT | NLM_F_MATCH
const NLM_F_CREATE: u16 = 0x400;
const NLM_F_APPEND: u16 = 0x800;
const NLM_F_MULTI: u16 = 0x002;
const NLM_F_DUMP_INTR: u16 = 0x010;
// netlink message types.
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLMSG_OVERRUN: u16 = 4;
// nlattr nested flag.
const NLA_F_NESTED: u16 = 0x8000;

// families / versions.
/// `NFPROTO_IPV4` — the `ip` family the shared `overdrive-mtls` table lives in.
const NFPROTO_IPV4: u8 = 2;
const NFPROTO_BRIDGE: u8 = 7;

/// Closed private nft family discriminator shared by the one codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NftFamily {
    Ipv4,
    Bridge,
}

impl NftFamily {
    const fn nfproto(self) -> u8 {
        match self {
            Self::Ipv4 => NFPROTO_IPV4,
            Self::Bridge => NFPROTO_BRIDGE,
        }
    }
}
const AF_UNSPEC: u8 = 0;
const IPPROTO_TCP: u8 = 6;

// table attrs.
const NFTA_TABLE_NAME: u16 = 1;
// chain attrs.
const NFTA_CHAIN_TABLE: u16 = 1;
const NFTA_CHAIN_HANDLE: u16 = 2;
const NFTA_CHAIN_NAME: u16 = 3;
const NFTA_CHAIN_HOOK: u16 = 4;
const NFTA_CHAIN_POLICY: u16 = 5;
const NFTA_CHAIN_TYPE: u16 = 7;
// hook attrs.
const NFTA_HOOK_HOOKNUM: u16 = 1;
const NFTA_HOOK_PRIORITY: u16 = 2;
// rule attrs.
const NFTA_RULE_TABLE: u16 = 1;
const NFTA_RULE_CHAIN: u16 = 2;
const NFTA_RULE_HANDLE: u16 = 3;
const NFTA_RULE_EXPRESSIONS: u16 = 4;
const NFTA_RULE_USERDATA: u16 = 7;
const NFTA_SET_TABLE: u16 = 1;
const NFTA_SET_NAME: u16 = 2;
const NFTA_SET_KEY_TYPE: u16 = 4;
const NFTA_SET_KEY_LEN: u16 = 5;
const NFTA_SET_ID: u16 = 10;
const NFTA_SET_USERDATA: u16 = 13;
// flowtable/object attrs.
const NFTA_FLOWTABLE_TABLE: u16 = 1;
const NFTA_FLOWTABLE_NAME: u16 = 2;
const NFTA_FLOWTABLE_HANDLE: u16 = 6;
const NFTA_OBJ_TABLE: u16 = 1;
const NFTA_OBJ_NAME: u16 = 2;
const NFTA_OBJ_HANDLE: u16 = 6;
const NFTA_SET_ELEM_KEY: u16 = 1;
const NFTA_SET_ELEM_LIST_TABLE: u16 = 1;
const NFTA_SET_ELEM_LIST_SET: u16 = 2;
const NFTA_SET_ELEM_LIST_ELEMENTS: u16 = 3;
const NFTA_SET_ELEM_LIST_SET_ID: u16 = 4;
const NFTA_SET_ELEM_LIST_ELEMENTS_NESTED: u16 = NFTA_SET_ELEM_LIST_ELEMENTS | NLA_F_NESTED;
const NFT_MSG_NEWSET: u16 = 9;
const NFT_MSG_DELSET: u16 = 11;
const NFT_MSG_GETSET: u16 = 10;
const NFT_MSG_NEWSETELEM: u16 = 12;
const NFT_MSG_GETSETELEM: u16 = 13;
const NFT_MSG_DELSETELEM: u16 = 14;
// NFT_MSG_TRACE (17) sits between GETGEN and stateful-object messages.
const NFT_MSG_NEWOBJ: u16 = 18;
const NFT_MSG_GETOBJ: u16 = 19;
const NFT_MSG_NEWFLOWTABLE: u16 = 22;
const NFT_MSG_GETFLOWTABLE: u16 = 23;
// ruleset generation attrs.
const NFTA_GEN_ID: u16 = 1;
// list + expr framing.
const NFTA_LIST_ELEM: u16 = 1;
const NFTA_EXPR_NAME: u16 = 1;
const NFTA_EXPR_DATA: u16 = 2;
// anonymous counter attrs.
const NFTA_COUNTER_BYTES: u16 = 1;
const NFTA_COUNTER_PACKETS: u16 = 2;
// data.
const NFTA_DATA_VALUE: u16 = 1;
const NFT_IFNAME_KEY_TYPE: u32 = 41;
const NFTA_DATA_VERDICT: u16 = 2;
const NFTA_VERDICT_CODE: u16 = 1;
// payload.
const NFTA_PAYLOAD_DREG: u16 = 1;
const NFTA_PAYLOAD_BASE: u16 = 2;
const NFTA_PAYLOAD_OFFSET: u16 = 3;
const NFTA_PAYLOAD_LEN: u16 = 4;
const NFT_PAYLOAD_NETWORK_HEADER: u32 = 1;
const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;
// cmp.
const NFTA_CMP_SREG: u16 = 1;
const NFTA_CMP_OP: u16 = 2;
const NFTA_CMP_DATA: u16 = 3;
const NFT_CMP_EQ: u32 = 0;
const NFT_CMP_NEQ: u32 = 1;
// immediate.
const NFTA_IMMEDIATE_DREG: u16 = 1;
const NFTA_IMMEDIATE_DATA: u16 = 2;
const NFTA_LOOKUP_SET: u16 = 1;
const NFTA_LOOKUP_SREG: u16 = 2;
const NFTA_LOOKUP_FLAGS: u16 = 5;
// meta.
const NFTA_META_DREG: u16 = 1;
const NFTA_META_KEY: u16 = 2;
const NFTA_META_SREG: u16 = 3;
const NFT_META_L4PROTO: u32 = 16;
const NFT_META_MARK: u32 = 3;
const NFT_META_IIFNAME: u32 = 6;
// tproxy (`nf_tables.h`: FAMILY=1, REG_ADDR=2, REG_PORT=3).
const NFTA_TPROXY_FAMILY: u16 = 1;
const NFTA_TPROXY_REG_ADDR: u16 = 2;
const NFTA_TPROXY_REG_PORT: u16 = 3;
// registers.
const NFT_REG_VERDICT: u32 = 0;
const NFT_REG_1: u32 = 1;
const NFT_REG_2: u32 = 2;
const NFT_REG_3: u32 = 3;
const NFT_REG_9: u32 = 9;
// verdict.
const NF_ACCEPT: u32 = 1;
// `IFNAMSIZ` — the kernel `meta iifname` load copies a NUL-padded 16-byte name.
const IFNAMSIZ: usize = 16;
const NFNLGRP_NFTABLES: u32 = 7;
const OBSERVATION_DEADLINE: Duration = Duration::from_secs(5);

/// The chain-type string for a base chain (`nft add chain … { type <T> … }`).
///
/// The `prerouting` chain is [`ChainKind::Filter`]; the REV-5 `output` chain
/// MUST be [`ChainKind::Route`] so the kernel re-evaluates the route after the
/// divert's `meta mark set`, firing the fwmark → local route on the output
/// path (spike `findings-output-hook-legb.md`).
#[derive(Clone, Copy, Debug)]
pub enum ChainKind {
    /// `type filter` — the prerouting TPROXY chain.
    Filter,
    /// `type route` — the REV-5 output divert chain (route re-evaluation).
    Route,
}

impl ChainKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Filter => "filter",
            Self::Route => "route",
        }
    }
}

/// A base-chain specification (hook num, priority, type).
#[derive(Clone, Copy, Debug)]
pub struct BaseChainSpec {
    /// The netfilter hook number (`NF_INET_PRE_ROUTING` = 0 / `NF_INET_LOCAL_OUT` = 3).
    pub hooknum: u32,
    /// The chain priority (`mangle` = -150).
    pub priority: i32,
    /// The chain type.
    pub kind: ChainKind,
}

/// `NF_INET_PRE_ROUTING` — the prerouting hook the inbound/egress TPROXY chain binds.
pub const NF_INET_PRE_ROUTING: u32 = 0;
/// `NF_INET_LOCAL_OUT` — the output hook the REV-5 divert chain binds.
pub const NF_INET_LOCAL_OUT: u32 = 3;
/// The `mangle` chain priority (where TPROXY / route re-eval must live).
pub const PRIORITY_MANGLE: i32 = -150;

/// One anonymous nftables counter sampled from a `GETRULE` reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleCounterSnapshot {
    /// Packets accepted by the counter expression.
    pub packets: u64,
    /// Validated bytes accepted by the counter expression.
    pub bytes: u64,
}

/// One rule recovered from a `GETRULE` dump reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleInfo {
    /// The kernel-assigned `NFTA_RULE_HANDLE` (the by-handle delete key).
    pub handle: u64,
    /// The rule's `NFTA_RULE_USERDATA` bytes (the structural identity tag).
    pub userdata: Vec<u8>,
    /// The rule's single anonymous counter, or `None` for counter-free rules.
    pub counter: Option<RuleCounterSnapshot>,
    /// Complete ordered expression program with counter values replaced by
    /// the encoder's typed anonymous-counter placeholder.
    pub normalized_program: Vec<u8>,
}

/// One generation-bracketed, strictly decoded nftables rule snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSnapshot {
    /// Full non-zero ruleset generation shared by both brackets.
    pub generation: u32,
    /// Every rule returned by the complete multipart dump.
    pub rules: Vec<RuleInfo>,
}

/// A subscribed, read-only nftables observer.
///
/// The socket joins `NFNLGRP_NFTABLES` before its first `GETGEN`. Any queued
/// notification, sequence mismatch, overrun, interrupted dump, malformed
/// frame, or generation change fails the observation closed.
#[derive(Debug)]
pub struct NftRuleObserver {
    socket: NfSock,
    next_sequence: u32,
}

/// One ordered rule mutation in an nftables atomic transaction.
///
/// The transaction API is intentionally handle- and program-explicit: callers
/// must audit ownership before constructing deletes, and inserts carry their
/// complete expression program plus structural userdata identity.
#[derive(Clone, Copy, Debug)]
pub enum AtomicRuleMutation<'a> {
    /// Delete exactly one audited rule by its kernel handle.
    Delete {
        /// IPv4 nft table name.
        table: &'a str,
        /// Chain containing the audited handle.
        chain: &'a str,
        /// Exact `NFTA_RULE_HANDLE` to delete.
        handle: u64,
    },
    /// Insert one rule at the chain head with its complete structural identity.
    Insert {
        /// IPv4 nft table name.
        table: &'a str,
        /// Chain receiving the rule.
        chain: &'a str,
        /// Complete encoded `NFTA_RULE_EXPRESSIONS` list.
        exprs: &'a [u8],
        /// Exact `NFTA_RULE_USERDATA` ownership tag.
        userdata: &'a [u8],
    },
}

// =============================================================================
// Structural rule identity — NFTA_RULE_USERDATA tags (pure)
// =============================================================================

/// The magic prefix every `overdrive-mtls` rule userdata tag carries, so a
/// foreign rule sharing the chain is never mistaken for one of ours.
pub const USERDATA_MAGIC: &[u8] = b"ovdmtls";
const KIND_EXEMPTION: u8 = 0x00;
const KIND_INBOUND: u8 = 0x01;
const KIND_OUTPUT_DIVERT: u8 = 0x02;
const KIND_EGRESS: u8 = 0x03;

fn userdata(kind: u8, key: &[u8]) -> Vec<u8> {
    let mut tag = Vec::with_capacity(USERDATA_MAGIC.len() + 1 + key.len());
    tag.extend_from_slice(USERDATA_MAGIC);
    tag.push(kind);
    tag.extend_from_slice(key);
    tag
}

/// Userdata tag for an inbound prerouting `tproxy` rule (per-virt, port-keyed).
#[must_use]
pub fn userdata_inbound(vip: Ipv4Addr, vport: u16, agent_port: u16) -> Vec<u8> {
    let mut key = Vec::with_capacity(8);
    key.extend_from_slice(&vip.octets());
    key.extend_from_slice(&vport.to_be_bytes());
    key.extend_from_slice(&agent_port.to_be_bytes());
    userdata(KIND_INBOUND, &key)
}

/// Userdata tag for an egress prerouting `tproxy` rule (per-`(host_veth, port)`).
#[must_use]
pub fn userdata_egress(host_veth: &str, agent_port: u16) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + host_veth.len());
    key.extend_from_slice(&agent_port.to_be_bytes());
    key.extend_from_slice(host_veth.as_bytes());
    userdata(KIND_EGRESS, &key)
}

/// Userdata tag for a REV-5 output-divert rule (per-virt, port-blind).
#[must_use]
pub fn userdata_output_divert(vip: Ipv4Addr, vport: u16) -> Vec<u8> {
    let mut key = Vec::with_capacity(6);
    key.extend_from_slice(&vip.octets());
    key.extend_from_slice(&vport.to_be_bytes());
    userdata(KIND_OUTPUT_DIVERT, &key)
}

/// Userdata tag for the shared leg-S `meta mark <mark> accept` exemption.
#[must_use]
pub fn userdata_exemption() -> Vec<u8> {
    userdata(KIND_EXEMPTION, &[])
}

fn is_ours(tag: &[u8]) -> bool {
    tag.starts_with(USERDATA_MAGIC) && tag.len() > USERDATA_MAGIC.len()
}

fn kind_of(tag: &[u8]) -> Option<u8> {
    if is_ours(tag) { tag.get(USERDATA_MAGIC.len()).copied() } else { None }
}

/// The kernel handle of the rule whose userdata EXACTLY equals `userdata`, or
/// `None` when no such rule is in the reply.
///
/// The structural per-rule handle
/// recovery (ADR-0085 D10) that replaces the `# handle N` text scrape — used by
/// the install to store the just-appended rule's handle for by-handle teardown,
/// and by the egress idempotence check to find an already-present rule.
#[must_use]
pub fn handle_for_userdata(rules: &[RuleInfo], userdata: &[u8]) -> Option<u64> {
    rules.iter().find(|rule| rule.userdata == userdata).map(|rule| rule.handle)
}

/// The handles of EVERY per-workload rule in the reply — inbound, egress, and
/// output-divert — NEVER the shared leg-S exemption or a foreign rule.
///
/// Per-workload = `kind` ∈ {inbound, egress, output-divert}; the exemption
/// (`kind` = exemption) and foreign rules (no magic prefix) are excluded. The
/// port-blind §5 boot sweep classifier (ADR-0085 D10): a restart loses the dead
/// leg-C/leg-F ports, so the sweep keys on the `kind` discriminator, not a port.
#[must_use]
pub fn workload_rule_handles(rules: &[RuleInfo]) -> Vec<u64> {
    rules
        .iter()
        .filter(|rule| {
            matches!(kind_of(&rule.userdata), Some(KIND_INBOUND | KIND_EGRESS | KIND_OUTPUT_DIVERT))
        })
        .map(|rule| rule.handle)
        .collect()
}

/// True iff the reply already carries the shared leg-S exemption.
///
/// The structural replacement for the deleted `dump_has_leg_s_exemption` text
/// parse (ADR-0085 D10), so the exemption is inserted exactly once at each head.
#[must_use]
pub fn has_exemption(rules: &[RuleInfo]) -> bool {
    rules.iter().any(|rule| kind_of(&rule.userdata) == Some(KIND_EXEMPTION))
}

// =============================================================================
// Pure wire-encoding primitives (default-lane unit-testable — no I/O)
// =============================================================================

fn pad4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

/// Append one nlattr `[u16 len][u16 type][payload][pad4]` (header native-endian).
fn attr(buf: &mut Vec<u8>, typ: u16, payload: &[u8]) {
    let len = 4 + payload.len();
    buf.extend_from_slice(&(len as u16).to_ne_bytes());
    buf.extend_from_slice(&typ.to_ne_bytes());
    buf.extend_from_slice(payload);
    pad4(buf);
}

/// Append one nlattr carrying a big-endian `u32` value (`nla_get_be32`).
fn attr_be32(buf: &mut Vec<u8>, typ: u16, val: u32) {
    attr(buf, typ, &val.to_be_bytes());
}

/// Wrap `inner` as one nested attr of `typ`, returned as fresh bytes.
fn nested(typ: u16, inner: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity((4 + inner.len()).next_multiple_of(4));
    attr(&mut v, typ | NLA_F_NESTED, inner);
    v
}

/// One expression as an `NFTA_LIST_ELEM`: `{ NFTA_EXPR_NAME, NFTA_EXPR_DATA }`.
fn expr(name: &str, data: &[u8]) -> Vec<u8> {
    let namez = cstr(name);
    // The NUL-terminated NFTA_EXPR_NAME attr followed by the nested
    // NFTA_EXPR_DATA attr — each 4-byte padded, so the pre-size is exact.
    let mut inner = Vec::with_capacity(
        (4 + namez.len()).next_multiple_of(4) + (4 + data.len()).next_multiple_of(4),
    );
    attr(&mut inner, NFTA_EXPR_NAME, &namez);
    attr(&mut inner, NFTA_EXPR_DATA | NLA_F_NESTED, data);
    nested(NFTA_LIST_ELEM, &inner)
}

fn data_value(bytes: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity((4 + bytes.len()).next_multiple_of(4));
    attr(&mut d, NFTA_DATA_VALUE, bytes);
    d
}

fn e_payload(base: u32, offset: u32, len: u32, dreg: u32) -> Vec<u8> {
    // four 8-byte NFTA_PAYLOAD_* be32 attrs.
    let mut d = Vec::with_capacity(4 * 8);
    attr_be32(&mut d, NFTA_PAYLOAD_DREG, dreg);
    attr_be32(&mut d, NFTA_PAYLOAD_BASE, base);
    attr_be32(&mut d, NFTA_PAYLOAD_OFFSET, offset);
    attr_be32(&mut d, NFTA_PAYLOAD_LEN, len);
    expr("payload", &d)
}

fn e_cmp(sreg: u32, op: u32, value: &[u8]) -> Vec<u8> {
    let val = data_value(value);
    // two 8-byte be32 attrs + the nested NFTA_CMP_DATA attr wrapping `val`.
    let mut d = Vec::with_capacity(2 * 8 + (4 + val.len()).next_multiple_of(4));
    attr_be32(&mut d, NFTA_CMP_SREG, sreg);
    attr_be32(&mut d, NFTA_CMP_OP, op);
    attr(&mut d, NFTA_CMP_DATA | NLA_F_NESTED, &val);
    expr("cmp", &d)
}

fn e_cmp_eq(sreg: u32, value: &[u8]) -> Vec<u8> {
    e_cmp(sreg, NFT_CMP_EQ, value)
}

fn e_meta_load(key: u32, dreg: u32) -> Vec<u8> {
    // two 8-byte be32 attrs.
    let mut d = Vec::with_capacity(2 * 8);
    attr_be32(&mut d, NFTA_META_DREG, dreg);
    attr_be32(&mut d, NFTA_META_KEY, key);
    expr("meta", &d)
}

fn e_meta_set(key: u32, sreg: u32) -> Vec<u8> {
    // two 8-byte be32 attrs.
    let mut d = Vec::with_capacity(2 * 8);
    attr_be32(&mut d, NFTA_META_KEY, key);
    attr_be32(&mut d, NFTA_META_SREG, sreg);
    expr("meta", &d)
}

fn e_immediate_value(dreg: u32, value: &[u8]) -> Vec<u8> {
    let val = data_value(value);
    // one 8-byte be32 attr + the nested NFTA_IMMEDIATE_DATA attr wrapping `val`.
    let mut d = Vec::with_capacity(8 + (4 + val.len()).next_multiple_of(4));
    attr_be32(&mut d, NFTA_IMMEDIATE_DREG, dreg);
    attr(&mut d, NFTA_IMMEDIATE_DATA | NLA_F_NESTED, &val);
    expr("immediate", &d)
}

fn e_immediate_verdict(code: u32) -> Vec<u8> {
    let mut verdict = Vec::with_capacity(8);
    attr_be32(&mut verdict, NFTA_VERDICT_CODE, code);
    let vd = nested(NFTA_DATA_VERDICT, &verdict);
    // one 8-byte be32 attr + the nested NFTA_IMMEDIATE_DATA attr wrapping `vd`.
    let mut d = Vec::with_capacity(8 + (4 + vd.len()).next_multiple_of(4));
    attr_be32(&mut d, NFTA_IMMEDIATE_DREG, NFT_REG_VERDICT);
    attr(&mut d, NFTA_IMMEDIATE_DATA | NLA_F_NESTED, &vd);
    expr("immediate", &d)
}

/// The `tproxy` expression, wrapped as one `NFTA_LIST_ELEM`.
///
/// `NFTA_TPROXY_FAMILY = be32 NFPROTO_IPV4`, `NFTA_TPROXY_REG_ADDR = be32
/// <reg_addr>`, `NFTA_TPROXY_REG_PORT = be32 <reg_port>` — the pinned
/// kernel-accepted wire bytes (`spike/findings-e.md`), encoded, never
/// re-derived. Pure so the golden-bytes test pins the layout against the pin.
#[must_use]
pub fn expr_tproxy_ipv4(reg_addr: u32, reg_port: u32) -> Vec<u8> {
    // three 8-byte be32 tproxy attrs.
    let mut d = Vec::with_capacity(3 * 8);
    attr_be32(&mut d, NFTA_TPROXY_FAMILY, u32::from(NFPROTO_IPV4));
    attr_be32(&mut d, NFTA_TPROXY_REG_ADDR, reg_addr);
    attr_be32(&mut d, NFTA_TPROXY_REG_PORT, reg_port);
    expr("tproxy", &d)
}

/// One anonymous, non-terminal nftables `counter` expression.
fn e_anonymous_counter() -> Vec<u8> {
    expr("counter", &[])
}

/// The `iifname "<host_veth>"` match: `meta iifname → reg1` then
/// `cmp reg1 == <name NUL-padded to IFNAMSIZ>`. The kernel `meta iifname` load
/// copies a NUL-padded 16-byte name, so the exact match compares all 16 bytes.
fn e_iifname_eq(host_veth: &str) -> Vec<u8> {
    let mut name = [0u8; IFNAMSIZ];
    let bytes = host_veth.as_bytes();
    let take = bytes.len().min(IFNAMSIZ - 1); // keep a NUL terminator
    name[..take].copy_from_slice(&bytes[..take]);
    let mut ex = e_meta_load(NFT_META_IIFNAME, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &name));
    ex
}

fn e_iifname_lookup(set: &str) -> Vec<u8> {
    let mut ex = e_meta_load(NFT_META_IIFNAME, NFT_REG_1);
    let mut data = Vec::new();
    attr(&mut data, NFTA_LOOKUP_SET, &cstr(set));
    attr_be32(&mut data, NFTA_LOOKUP_SREG, NFT_REG_1);
    ex.extend(expr("lookup", &data));
    ex
}

fn e_lookup(set: &str, sreg: u32) -> Vec<u8> {
    let mut data = Vec::new();
    attr(&mut data, NFTA_LOOKUP_SET, &cstr(set));
    attr_be32(&mut data, NFTA_LOOKUP_SREG, sreg);
    expr("lookup", &data)
}

fn e_ipv4_lookup(set: &str, offset: u32) -> Vec<u8> {
    let mut ex = e_payload(NFT_PAYLOAD_NETWORK_HEADER, offset, 4, NFT_REG_1);
    ex.extend(e_lookup(set, NFT_REG_1));
    ex
}

fn e_ipv4_tcp_destination_lookup(set: &str) -> Vec<u8> {
    let mut ex = e_meta_load(NFT_META_L4PROTO, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &[IPPROTO_TCP]));
    ex.extend(e_payload(NFT_PAYLOAD_NETWORK_HEADER, 16, 4, NFT_REG_1));
    ex.extend(e_payload(NFT_PAYLOAD_TRANSPORT_HEADER, 2, 2, NFT_REG_9));
    ex.extend(e_lookup(set, NFT_REG_1));
    ex
}

fn e_mark_equals(mark: u32) -> Vec<u8> {
    let mut ex = e_meta_load(NFT_META_MARK, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &mark.to_ne_bytes()));
    ex
}

fn e_mark_not_equals(mark: u32) -> Vec<u8> {
    let mut ex = e_meta_load(NFT_META_MARK, NFT_REG_1);
    ex.extend(e_cmp(NFT_REG_1, NFT_CMP_NEQ, &mark.to_ne_bytes()));
    ex
}

fn e_tcp_protocol() -> Vec<u8> {
    let mut ex = e_meta_load(NFT_META_L4PROTO, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &[IPPROTO_TCP]));
    ex
}

fn e_drop() -> Vec<u8> {
    e_immediate_verdict(0)
}

/// The expression list for the inbound prerouting rule
/// `ip daddr <vip> tcp dport <vport> tproxy to <agent_ip>:<agent_port>
/// meta mark set <set_mark> accept` — the spike-e-proven layout.
#[must_use]
pub fn inbound_tproxy_rule_exprs(
    vip: Ipv4Addr,
    vport: u16,
    agent_ip: Ipv4Addr,
    agent_port: u16,
    set_mark: u32,
) -> Vec<u8> {
    // ip daddr <vip>: payload(network, off=16, len=4) → reg1 ; cmp reg1 == vip.
    let mut ex = e_payload(NFT_PAYLOAD_NETWORK_HEADER, 16, 4, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &vip.octets()));
    // tcp: meta l4proto → reg1 ; cmp reg1 == 6.
    ex.extend(e_meta_load(NFT_META_L4PROTO, NFT_REG_1));
    ex.extend(e_cmp_eq(NFT_REG_1, &[IPPROTO_TCP]));
    // dport <vport>: payload(transport, off=2, len=2) → reg1 ; cmp reg1 == vport(be16).
    ex.extend(e_payload(NFT_PAYLOAD_TRANSPORT_HEADER, 2, 2, NFT_REG_1));
    ex.extend(e_cmp_eq(NFT_REG_1, &vport.to_be_bytes()));
    ex.extend(tproxy_and_mark_and_accept(agent_ip, agent_port, set_mark));
    ex
}

/// The expression list for the egress prerouting rule
/// `iifname "<host_veth>" meta l4proto tcp tproxy to <agent_ip>:<agent_port>
/// meta mark set <set_mark> accept` — the active-side mirror of inbound.
#[must_use]
pub fn egress_tproxy_rule_exprs(
    host_veth: &str,
    agent_ip: Ipv4Addr,
    agent_port: u16,
    set_mark: u32,
) -> Vec<u8> {
    let mut ex = e_iifname_eq(host_veth);
    // meta l4proto tcp.
    ex.extend(e_meta_load(NFT_META_L4PROTO, NFT_REG_1));
    ex.extend(e_cmp_eq(NFT_REG_1, &[IPPROTO_TCP]));
    // D7: exactly one anonymous counter after both selection predicates and
    // before the byte-identical redirect/mark/accept tail.
    ex.extend(e_anonymous_counter());
    ex.extend(tproxy_and_mark_and_accept(agent_ip, agent_port, set_mark));
    ex
}

/// The shared `meta mark set <set_mark> tproxy to
/// <agent_ip>:<agent_port> accept` tail of both TPROXY rules. Marking precedes
/// TPROXY deliberately: when the transparent listener is absent the kernel
/// ends the rule with `NFT_BREAK`, but the existing fwmark/local route still
/// prevents the packet from resuming its original cleartext route.
fn tproxy_and_mark_and_accept(agent_ip: Ipv4Addr, agent_port: u16, set_mark: u32) -> Vec<u8> {
    // meta mark set <set_mark>: reg3 = mark (HOST order — matched by the fwmark
    // ip rule) ; meta mark set sreg=reg3.
    let mut ex = e_immediate_value(NFT_REG_3, &set_mark.to_ne_bytes());
    ex.extend(e_meta_set(NFT_META_MARK, NFT_REG_3));
    // load tproxy dst: reg1 = agent_ip (network order), reg2 = agent_port (be16).
    ex.extend(e_immediate_value(NFT_REG_1, &agent_ip.octets()));
    ex.extend(e_immediate_value(NFT_REG_2, &agent_port.to_be_bytes()));
    // tproxy verb: family ipv4, reg_addr=reg1, reg_port=reg2.
    ex.extend(expr_tproxy_ipv4(NFT_REG_1, NFT_REG_2));
    ex.extend(e_immediate_verdict(NF_ACCEPT));
    ex
}

/// The expression list for the REV-5 output-divert rule.
///
/// `ip daddr <vip> tcp dport <vport> meta mark != <exempt_mark>
/// meta mark set <set_mark> accept` — NO `tproxy` verb (route re-eval on the
/// output hook fires the fwmark -> local route).
#[must_use]
pub fn output_divert_rule_exprs(
    vip: Ipv4Addr,
    vport: u16,
    exempt_mark: u32,
    set_mark: u32,
) -> Vec<u8> {
    let mut ex = e_payload(NFT_PAYLOAD_NETWORK_HEADER, 16, 4, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &vip.octets()));
    ex.extend(e_meta_load(NFT_META_L4PROTO, NFT_REG_1));
    ex.extend(e_cmp_eq(NFT_REG_1, &[IPPROTO_TCP]));
    ex.extend(e_payload(NFT_PAYLOAD_TRANSPORT_HEADER, 2, 2, NFT_REG_1));
    ex.extend(e_cmp_eq(NFT_REG_1, &vport.to_be_bytes()));
    // meta mark != <exempt_mark> (host order — the leg-S recursion guard).
    ex.extend(e_meta_load(NFT_META_MARK, NFT_REG_1));
    ex.extend(e_cmp(NFT_REG_1, NFT_CMP_NEQ, &exempt_mark.to_ne_bytes()));
    // meta mark set <set_mark>.
    ex.extend(e_immediate_value(NFT_REG_2, &set_mark.to_ne_bytes()));
    ex.extend(e_meta_set(NFT_META_MARK, NFT_REG_2));
    ex.extend(e_immediate_verdict(NF_ACCEPT));
    ex
}

/// The expression list for the shared leg-S `meta mark <mark> accept`
/// exemption — `meta mark → reg1 ; cmp reg1 == mark ; accept`.
#[must_use]
pub fn mark_accept_exemption_exprs(mark: u32) -> Vec<u8> {
    let mut ex = e_meta_load(NFT_META_MARK, NFT_REG_1);
    ex.extend(e_cmp_eq(NFT_REG_1, &mark.to_ne_bytes()));
    ex.extend(e_immediate_verdict(NF_ACCEPT));
    ex
}

const SHARED_IP_TABLE: &str = "overdrive-mtls";
const SHARED_IP_PREROUTING: &str = "prerouting";
const SHARED_IP_OUTPUT: &str = "output";
const SHARED_IP_MANAGED_GUESTS: &str = "managed_guest_ips";
const SHARED_IP_OUTBOUND_SOURCES: &str = "outbound_sources";
const SHARED_IP_INBOUND_DESTINATIONS: &str = "inbound_destinations";
const SHARED_IP_FWMARK: u32 = 0x1;

fn shared_ip_userdata(chain: &str, index: usize) -> Vec<u8> {
    format!("ovd295-ip-{chain}-{index}").into_bytes()
}

fn shared_ip_set_userdata(name: &str) -> Vec<u8> {
    format!("ovd295-ip-set-{name}").into_bytes()
}

fn shared_ip_prerouting_rule_exprs(port_f: u16, port_c: u16, index: usize) -> Vec<u8> {
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    match index {
        // leg-S exemption.
        0 => mark_accept_exemption_exprs(leg_s_mark),
        // TCX-intercepted outbound TCP from a registered source to leg F.
        1 => {
            let mut ex = e_mark_equals(0x295a);
            ex.extend(e_ipv4_lookup(SHARED_IP_OUTBOUND_SOURCES, 12));
            ex.extend(e_tcp_protocol());
            ex.extend(tproxy_and_mark_and_accept(Ipv4Addr::LOCALHOST, port_f, SHARED_IP_FWMARK));
            ex
        }
        // TCX-intercepted TCP without a registered source.
        2 => {
            let mut ex = e_mark_equals(0x295a);
            ex.extend(e_tcp_protocol());
            ex.extend(e_drop());
            ex
        }
        // Registered destination and declared TCP port to leg C.
        3 => {
            let mut ex = e_ipv4_tcp_destination_lookup(SHARED_IP_INBOUND_DESTINATIONS);
            ex.extend(tproxy_and_mark_and_accept(Ipv4Addr::LOCALHOST, port_c, SHARED_IP_FWMARK));
            ex
        }
        // Any other TCP packet directed at a managed guest is dropped.
        4 => {
            let mut ex = e_ipv4_lookup(SHARED_IP_MANAGED_GUESTS, 16);
            ex.extend(e_tcp_protocol());
            ex.extend(e_drop());
            ex
        }
        _ => unreachable!("shared prerouting rule index is bounded to five"),
    }
}

fn shared_ip_output_rule_exprs(leg_c_port: u16, index: usize) -> Vec<u8> {
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    match index {
        // leg-S exemption.
        0 => mark_accept_exemption_exprs(leg_s_mark),
        // Registered destination and declared TCP port through the local route.
        1 => {
            let mut ex = e_ipv4_tcp_destination_lookup(SHARED_IP_INBOUND_DESTINATIONS);
            ex.extend(e_mark_not_equals(leg_s_mark));
            ex.extend(e_immediate_value(NFT_REG_2, &SHARED_IP_FWMARK.to_ne_bytes()));
            ex.extend(e_meta_set(NFT_META_MARK, NFT_REG_2));
            ex.extend(e_immediate_verdict(NF_ACCEPT));
            let _ = leg_c_port;
            ex
        }
        // Any other TCP packet directed at a managed guest is dropped.
        2 => {
            let mut ex = e_ipv4_lookup(SHARED_IP_MANAGED_GUESTS, 16);
            ex.extend(e_tcp_protocol());
            ex.extend(e_drop());
            ex
        }
        _ => unreachable!("shared output rule index is bounded to three"),
    }
}

// =============================================================================
// Message assembly (pure)
// =============================================================================

fn nlmsg(buf: &mut Vec<u8>, typ: u16, flags: u16, seq: u32, payload: &[u8]) {
    let len = 16 + payload.len();
    buf.extend_from_slice(&(len as u32).to_ne_bytes());
    buf.extend_from_slice(&typ.to_ne_bytes());
    buf.extend_from_slice(&flags.to_ne_bytes());
    buf.extend_from_slice(&seq.to_ne_bytes());
    buf.extend_from_slice(&0u32.to_ne_bytes()); // pid 0 = kernel
    buf.extend_from_slice(payload);
    pad4(buf);
}

fn nfgenmsg(family: u8, res_id: u16) -> Vec<u8> {
    // family(1) + NFNETLINK_V0(1) + res_id __be16(2).
    let mut v = Vec::with_capacity(4);
    v.push(family);
    v.push(0); // NFNETLINK_V0
    v.extend_from_slice(&res_id.to_be_bytes()); // __be16
    v
}

const fn nft_msg_type(op: u16) -> u16 {
    (NFNL_SUBSYS_NFTABLES << 8) | op
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// A `NEWRULE` payload (`nfgenmsg` + table/chain + expressions + userdata).
fn newrule_payload_family(
    family: NftFamily,
    table: &str,
    chain: &str,
    exprs: &[u8],
    userdata: &[u8],
) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_RULE_TABLE, &cstr(table));
    attr(&mut payload, NFTA_RULE_CHAIN, &cstr(chain));
    attr(&mut payload, NFTA_RULE_EXPRESSIONS | NLA_F_NESTED, exprs);
    if !userdata.is_empty() {
        attr(&mut payload, NFTA_RULE_USERDATA, userdata);
    }
    payload
}

fn replace_rule_payload_family(
    family: NftFamily,
    table: &str,
    chain: &str,
    handle: u64,
    exprs: &[u8],
    userdata: &[u8],
) -> Vec<u8> {
    let mut payload = newrule_payload_family(family, table, chain, exprs, userdata);
    attr(&mut payload, NFTA_RULE_HANDLE, &handle.to_be_bytes());
    payload
}

fn newrule_payload(table: &str, chain: &str, exprs: &[u8], userdata: &[u8]) -> Vec<u8> {
    newrule_payload_family(NftFamily::Ipv4, table, chain, exprs, userdata)
}

/// A `NEWTABLE` payload (`nfgenmsg` + table name).
fn newtable_payload(table: &str) -> Vec<u8> {
    newtable_payload_family(NftFamily::Ipv4, table)
}

fn newtable_payload_family(family: NftFamily, table: &str) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_TABLE_NAME, &cstr(table));
    payload
}

/// A `NEWCHAIN` payload (`nfgenmsg` + table/name + hook{num,priority} + type + policy).
fn newchain_payload(table: &str, chain: &str, spec: BaseChainSpec) -> Vec<u8> {
    newchain_payload_family(NftFamily::Ipv4, table, chain, spec)
}

fn newchain_payload_family(
    family: NftFamily,
    table: &str,
    chain: &str,
    spec: BaseChainSpec,
) -> Vec<u8> {
    // two 8-byte be32 hook attrs.
    let mut hook = Vec::with_capacity(2 * 8);
    attr_be32(&mut hook, NFTA_HOOK_HOOKNUM, spec.hooknum);
    attr_be32(&mut hook, NFTA_HOOK_PRIORITY, spec.priority as u32);

    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_CHAIN_TABLE, &cstr(table));
    attr(&mut payload, NFTA_CHAIN_NAME, &cstr(chain));
    attr(&mut payload, NFTA_CHAIN_HOOK | NLA_F_NESTED, &hook);
    attr(&mut payload, NFTA_CHAIN_TYPE, &cstr(spec.kind.as_str()));
    attr_be32(&mut payload, NFTA_CHAIN_POLICY, NF_ACCEPT);
    payload
}

/// A `DELRULE`-by-handle payload (`nfgenmsg` + table/chain + handle be64).
fn delrule_payload(table: &str, chain: &str, handle: u64) -> Vec<u8> {
    delrule_payload_family(NftFamily::Ipv4, table, chain, handle)
}

fn delrule_payload_family(family: NftFamily, table: &str, chain: &str, handle: u64) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_RULE_TABLE, &cstr(table));
    attr(&mut payload, NFTA_RULE_CHAIN, &cstr(chain));
    attr(&mut payload, NFTA_RULE_HANDLE, &handle.to_be_bytes());
    payload
}

/// A `GET{RULE,CHAIN}` request payload keyed by table (+ chain).
fn get_by_table_chain(table: &str, chain: &str, chain_attr: u16, table_attr: u16) -> Vec<u8> {
    get_by_table_chain_family(NftFamily::Ipv4, table, chain, chain_attr, table_attr)
}

fn get_by_table_chain_family(
    family: NftFamily,
    table: &str,
    chain: &str,
    chain_attr: u16,
    table_attr: u16,
) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, table_attr, &cstr(table));
    attr(&mut payload, chain_attr, &cstr(chain));
    payload
}

fn newset_payload_schema_family(
    family: NftFamily,
    table: &str,
    set: &str,
    key_type: u32,
    key_len: u32,
    id: u32,
    userdata: &[u8],
) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_SET_TABLE, &cstr(table));
    attr(&mut payload, NFTA_SET_NAME, &cstr(set));
    attr_be32(&mut payload, NFTA_SET_KEY_TYPE, key_type);
    attr_be32(&mut payload, NFTA_SET_KEY_LEN, key_len);
    attr_be32(&mut payload, NFTA_SET_ID, id);
    if !userdata.is_empty() {
        attr(&mut payload, NFTA_SET_USERDATA, userdata);
    }
    payload
}

fn newset_payload_family(family: NftFamily, table: &str, set: &str) -> Vec<u8> {
    // `NFT_DATA_VALUE` is informational for an ifname key.  The kernel uses
    // the explicit key length and validates the family-specific set schema.
    newset_payload_schema_family(
        family,
        table,
        set,
        NFT_IFNAME_KEY_TYPE,
        IFNAMSIZ as u32,
        1,
        &[0x00, 0x04, 0x01, 0x00, 0x00, 0x00],
    )
}

fn delete_set_payload_family(family: NftFamily, table: &str, set: &str) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_SET_TABLE, &cstr(table));
    attr(&mut payload, NFTA_SET_NAME, &cstr(set));
    payload
}

fn set_elem_payload_family(
    family: NftFamily,
    table: &str,
    set: &str,
    member: &[u8; IFNAMSIZ],
) -> Vec<u8> {
    let mut element = Vec::new();
    let mut key = Vec::new();
    attr(&mut key, NFTA_DATA_VALUE, member);
    // `NFTA_SET_ELEM_LIST_ELEMENTS` is a list: each element receives its own
    // nested index attribute, whose payload is the nested key attribute.
    // Omitting either wrapper produces an indistinguishable-looking packet
    // but the kernel rejects it with EINVAL.
    let mut key_element = Vec::new();
    attr(&mut key_element, NFTA_SET_ELEM_KEY | NLA_F_NESTED, &key);
    attr(&mut element, NLA_F_NESTED | 1, &key_element);
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_SET_ELEM_LIST_TABLE, &cstr(table));
    attr(&mut payload, NFTA_SET_ELEM_LIST_SET, &cstr(set));
    // The kernel requires the numeric set id on NEW/DELSETELEM requests;
    // it is the id assigned by NEWSET above, not an ABI-visible public id.
    attr_be32(&mut payload, NFTA_SET_ELEM_LIST_SET_ID, 1);
    attr(&mut payload, NFTA_SET_ELEM_LIST_ELEMENTS_NESTED, &element);
    payload
}

fn get_set_elements_payload_family(family: NftFamily, table: &str, set: &str, id: u32) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, NFTA_SET_ELEM_LIST_TABLE, &cstr(table));
    attr(&mut payload, NFTA_SET_ELEM_LIST_SET, &cstr(set));
    attr_be32(&mut payload, NFTA_SET_ELEM_LIST_SET_ID, id);
    payload
}

fn list_set_elements_family(
    family: NftFamily,
    table: &str,
    set: &str,
    id: u32,
) -> Result<Vec<Vec<u8>>, NetlinkError> {
    let sock = NfSock::open().map_err(|source| NetlinkError::nft("list-set-elements", source))?;
    let payload = get_set_elements_payload_family(family, table, set, id);
    let mut message = Vec::new();
    nlmsg(&mut message, nft_msg_type(NFT_MSG_GETSETELEM), NLM_F_REQUEST | NLM_F_DUMP, 1, &payload);
    sock.send(&message).map_err(|source| NetlinkError::nft("list-set-elements", source))?;
    let mut members = Vec::new();
    loop {
        let mut datagram = vec![0_u8; 65_535];
        let received = sock
            .recv(&mut datagram)
            .map_err(|source| NetlinkError::nft("list-set-elements", source))?;
        let mut offset = 0;
        while offset < received {
            if received - offset < 16 {
                return Err(NetlinkError::nft(
                    "list-set-elements",
                    invalid_data("truncated set-element netlink header"),
                ));
            }
            let length = ne_u32(&datagram, offset).map_or(0, |value| value as usize);
            if length < 16 || offset + length > received {
                return Err(NetlinkError::nft(
                    "list-set-elements",
                    invalid_data("invalid set-element netlink length"),
                ));
            }
            let kind = ne_u16(&datagram, offset + 4).unwrap_or_default();
            let body = &datagram[offset + 16..offset + length];
            if kind == NLMSG_DONE {
                return Ok(members);
            }
            if kind == NLMSG_ERROR {
                let code = ne_u32(&datagram, offset + 16).map_or(0, |value| value as i32);
                if code != 0 {
                    return Err(NetlinkError::nft(
                        "list-set-elements",
                        std::io::Error::from_raw_os_error(code.abs()),
                    ));
                }
            } else if kind == nft_msg_type(NFT_MSG_NEWSETELEM) {
                for (attribute, _, value) in exact_attrs(body.get(4..).unwrap_or_default())
                    .map_err(|source| NetlinkError::nft("list-set-elements", source))?
                {
                    if attribute != NFTA_SET_ELEM_LIST_ELEMENTS {
                        continue;
                    }
                    for (_, _, element) in exact_attrs(value)
                        .map_err(|source| NetlinkError::nft("list-set-elements", source))?
                    {
                        for (element_attribute, _, element_value) in exact_attrs(element)
                            .map_err(|source| NetlinkError::nft("list-set-elements", source))?
                        {
                            if element_attribute != NFTA_SET_ELEM_KEY {
                                continue;
                            }
                            for (data_attribute, _, data_value) in exact_attrs(element_value)
                                .map_err(|source| NetlinkError::nft("list-set-elements", source))?
                            {
                                if data_attribute == NFTA_DATA_VALUE {
                                    members.push(data_value.to_vec());
                                }
                            }
                        }
                    }
                }
            }
            offset += (length + 3) & !3;
        }
    }
}

// =============================================================================
// GETRULE reply decode (pure)
// =============================================================================

fn ne_u16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_ne_bytes([*b.get(o)?, *b.get(o + 1)?]))
}
fn ne_u32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_ne_bytes([*b.get(o)?, *b.get(o + 1)?, *b.get(o + 2)?, *b.get(o + 3)?]))
}

fn invalid_data(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

fn exact_attrs(body: &[u8]) -> std::io::Result<Vec<(u16, u16, &[u8])>> {
    let mut attrs = Vec::new();
    let mut off = 0usize;
    while off < body.len() {
        if body.len() - off < 4 {
            return Err(invalid_data("trailing partial nlattr header"));
        }
        let len = usize::from(ne_u16(body, off).ok_or_else(|| invalid_data("nlattr length"))?);
        let raw_type = ne_u16(body, off + 2).ok_or_else(|| invalid_data("nlattr type"))?;
        if len < 4 {
            return Err(invalid_data("nlattr length smaller than header"));
        }
        let end = off.checked_add(len).ok_or_else(|| invalid_data("nlattr length overflow"))?;
        let aligned = end
            .checked_add(3)
            .map(|value| value & !3)
            .ok_or_else(|| invalid_data("nlattr alignment overflow"))?;
        if end > body.len() || aligned > body.len() {
            return Err(invalid_data("truncated nlattr payload or padding"));
        }
        if body[end..aligned].iter().any(|byte| *byte != 0) {
            return Err(invalid_data("non-zero nlattr alignment padding"));
        }
        let payload = &body[off + 4..end];
        if raw_type & NLA_F_NESTED != 0 {
            let _ = exact_attrs(payload)?;
        }
        attrs.push((raw_type & !NLA_F_NESTED, raw_type, payload));
        off = aligned;
    }
    Ok(attrs)
}

fn exact_cstr<'a>(payload: &'a [u8], field: &str) -> std::io::Result<&'a str> {
    let Some((&0, bytes)) = payload.split_last() else {
        return Err(invalid_data(format!("{field} is not exactly NUL terminated")));
    };
    if bytes.contains(&0) {
        return Err(invalid_data(format!("{field} contains an interior NUL")));
    }
    std::str::from_utf8(bytes).map_err(|_| invalid_data(format!("{field} is not UTF-8")))
}

fn exact_be_u64(payload: &[u8], field: &str) -> std::io::Result<u64> {
    let bytes: [u8; 8] = payload
        .try_into()
        .map_err(|_| invalid_data(format!("{field} must be exactly eight bytes")))?;
    Ok(u64::from_be_bytes(bytes))
}

fn exact_be_u32(payload: &[u8], field: &str) -> std::io::Result<u32> {
    let bytes: [u8; 4] = payload
        .try_into()
        .map_err(|_| invalid_data(format!("{field} must be exactly four bytes")))?;
    Ok(u32::from_be_bytes(bytes))
}

fn canonical_data_container(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut canonical = Vec::new();
    let attrs = exact_attrs(data)?;
    let mut seen = BTreeSet::new();
    let mut sorted = attrs;
    sorted.sort_by_key(|(kind, _, _)| *kind);
    for (kind, _, payload) in sorted {
        if !seen.insert(kind) {
            return Err(invalid_data("duplicate nft data-container attribute"));
        }
        match kind {
            NFTA_DATA_VALUE => attr(&mut canonical, kind, payload),
            NFTA_DATA_VERDICT => {
                let verdict = canonical_attr_set(payload, &[])?;
                attr(&mut canonical, kind | NLA_F_NESTED, &verdict);
            }
            _ => return Err(invalid_data("unknown nft data-container attribute")),
        }
    }
    Ok(canonical)
}

fn canonical_attr_set(data: &[u8], nested_data_kinds: &[u16]) -> std::io::Result<Vec<u8>> {
    let attrs = exact_attrs(data)?;
    let mut seen = BTreeSet::new();
    let mut sorted = attrs;
    sorted.sort_by_key(|(kind, _, _)| *kind);
    let mut canonical = Vec::with_capacity(data.len());
    for (kind, _, payload) in sorted {
        if !seen.insert(kind) {
            return Err(invalid_data("duplicate expression-data attribute"));
        }
        if nested_data_kinds.contains(&kind) {
            let nested = canonical_data_container(payload)?;
            attr(&mut canonical, kind | NLA_F_NESTED, &nested);
        } else {
            attr(&mut canonical, kind, payload);
        }
    }
    Ok(canonical)
}

fn canonical_expression_data(
    name: &str,
    data: &[u8],
    allow_unknown: bool,
) -> std::io::Result<Vec<u8>> {
    match name {
        "cmp" => canonical_attr_set(data, &[NFTA_CMP_DATA]),
        "immediate" => canonical_attr_set(data, &[NFTA_IMMEDIATE_DATA]),
        "meta" | "payload" | "tproxy" => canonical_attr_set(data, &[]),
        // nftables materializes the implicit zero lookup flags in GETRULE
        // replies. The producer omits that default, so the private identity
        // projection removes only this one semantic default.
        "lookup" => {
            let attrs = exact_attrs(data)?;
            let mut canonical = Vec::with_capacity(data.len());
            let mut seen = BTreeSet::new();
            let mut sorted = attrs;
            sorted.sort_by_key(|(kind, _, _)| *kind);
            for (kind, _, payload) in sorted {
                if !seen.insert(kind) {
                    return Err(invalid_data("duplicate lookup-data attribute"));
                }
                if kind == NFTA_LOOKUP_FLAGS && exact_be_u32(payload, "lookup flags")? == 0 {
                    continue;
                }
                attr(&mut canonical, kind, payload);
            }
            Ok(canonical)
        }
        // The bridge adapter must retain a well-formed expression it does not
        // understand as a semantic conflict.  Preserve its canonical
        // attribute framing here; the closed bridge projection names it as an
        // `Unknown` expression without exposing raw ABI bytes.  The existing
        // IPv4 decoder remains strict and still fails closed on unknown
        // expressions.
        _ if allow_unknown => canonical_attr_set(data, &[]),
        _ => Err(invalid_data(format!("unknown nft expression {name:?}"))),
    }
}

fn normalize_rule_program(
    expressions: &[u8],
    require_sampled_counter: bool,
) -> std::io::Result<(Vec<u8>, Option<RuleCounterSnapshot>)> {
    normalize_rule_program_with_unknown(expressions, require_sampled_counter, false)
}

fn normalize_rule_program_with_unknown(
    expressions: &[u8],
    require_sampled_counter: bool,
    allow_unknown: bool,
) -> std::io::Result<(Vec<u8>, Option<RuleCounterSnapshot>)> {
    let mut normalized = Vec::with_capacity(expressions.len());
    let mut counter = None;
    let mut counter_seen = false;
    for (kind, _raw_kind, element) in exact_attrs(expressions)? {
        if kind != NFTA_LIST_ELEM {
            return Err(invalid_data("rule expression list contains a non-LIST_ELEM attribute"));
        }
        let mut name = None;
        let mut data = None;
        for (attr_kind, attr_raw_kind, payload) in exact_attrs(element)? {
            match attr_kind {
                NFTA_EXPR_NAME if attr_raw_kind & NLA_F_NESTED == 0 && name.is_none() => {
                    name = Some(exact_cstr(payload, "NFTA_EXPR_NAME")?);
                }
                NFTA_EXPR_DATA if data.is_none() => {
                    data = Some(payload);
                }
                NFTA_EXPR_NAME | NFTA_EXPR_DATA => {
                    return Err(invalid_data("duplicate or wrongly nested expression attribute"));
                }
                _ => return Err(invalid_data("unknown expression framing attribute")),
            }
        }
        let name = name.ok_or_else(|| invalid_data("expression name is missing"))?;
        let data = data.ok_or_else(|| invalid_data("expression data is missing"))?;
        if name == "counter" {
            if counter_seen {
                return Err(invalid_data("rule contains duplicate anonymous counters"));
            }
            counter_seen = true;
            if data.is_empty() && !require_sampled_counter {
                normalized.extend(e_anonymous_counter());
                continue;
            }
            let mut packets = None;
            let mut bytes = None;
            for (attr_kind, raw_attr_kind, payload) in exact_attrs(data)? {
                if raw_attr_kind & NLA_F_NESTED != 0 {
                    return Err(invalid_data("counter value attribute must not be nested"));
                }
                match attr_kind {
                    NFTA_COUNTER_PACKETS if packets.is_none() => {
                        packets = Some(exact_be_u64(payload, "counter packets")?);
                    }
                    NFTA_COUNTER_BYTES if bytes.is_none() => {
                        bytes = Some(exact_be_u64(payload, "counter bytes")?);
                    }
                    NFTA_COUNTER_PACKETS | NFTA_COUNTER_BYTES => {
                        return Err(invalid_data("duplicate counter value attribute"));
                    }
                    _ => return Err(invalid_data("unknown anonymous-counter attribute")),
                }
            }
            counter = Some(RuleCounterSnapshot {
                packets: packets.ok_or_else(|| invalid_data("counter packets are missing"))?,
                bytes: bytes.ok_or_else(|| invalid_data("counter bytes are missing"))?,
            });
            normalized.extend(e_anonymous_counter());
        } else {
            normalized.extend(expr(name, &canonical_expression_data(name, data, allow_unknown)?));
        }
    }
    Ok((normalized, counter))
}

/// Canonical complete expression-program identity for an encoder-owned rule.
///
/// Kernel `GETRULE` replies may reorder expression operands and omit nested
/// flag bits while preserving their semantics. This projection sorts each
/// expression's uniquely-keyed operands, restores the known nested data
/// containers, and replaces an anonymous counter with the same typed empty
/// placeholder used for sampled kernel counters. Expression order and every
/// operand value remain exact.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] for partial, malformed,
/// duplicate, or unknown expression framing.
pub fn normalized_rule_program_identity(expressions: &[u8]) -> std::io::Result<Vec<u8>> {
    normalize_rule_program(expressions, false).map(|(program, _)| program)
}

fn decode_rule_message_family(
    payload: &[u8],
    table: &str,
    chain: &str,
    family: NftFamily,
) -> std::io::Result<RuleInfo> {
    if payload.len() < 4 {
        return Err(invalid_data("NEWRULE is missing nfgenmsg"));
    }
    if payload[0] != family.nfproto() || payload[1] != 0 {
        return Err(invalid_data("NEWRULE carries the wrong family/version"));
    }
    let mut observed_table = None;
    let mut observed_chain = None;
    let mut handle = None;
    let mut userdata = None;
    let mut program = None;
    for (kind, raw_kind, value) in exact_attrs(&payload[4..])? {
        match kind {
            NFTA_RULE_TABLE if raw_kind & NLA_F_NESTED == 0 && observed_table.is_none() => {
                observed_table = Some(exact_cstr(value, "NFTA_RULE_TABLE")?);
            }
            NFTA_RULE_CHAIN if raw_kind & NLA_F_NESTED == 0 && observed_chain.is_none() => {
                observed_chain = Some(exact_cstr(value, "NFTA_RULE_CHAIN")?);
            }
            NFTA_RULE_HANDLE if raw_kind & NLA_F_NESTED == 0 && handle.is_none() => {
                handle = Some(exact_be_u64(value, "NFTA_RULE_HANDLE")?);
            }
            NFTA_RULE_USERDATA if raw_kind & NLA_F_NESTED == 0 && userdata.is_none() => {
                userdata = Some(value.to_vec());
            }
            NFTA_RULE_EXPRESSIONS if program.is_none() => {
                program = Some(normalize_rule_program_with_unknown(
                    value,
                    true,
                    family == NftFamily::Bridge,
                )?);
            }
            NFTA_RULE_TABLE
            | NFTA_RULE_CHAIN
            | NFTA_RULE_HANDLE
            | NFTA_RULE_USERDATA
            | NFTA_RULE_EXPRESSIONS => {
                return Err(invalid_data(format!(
                    "duplicate or wrongly nested rule identity attribute kind={kind} raw={raw_kind:#x}"
                )));
            }
            _ => {}
        }
    }
    if observed_table != Some(table) || observed_chain != Some(chain) {
        return Err(invalid_data("NEWRULE table/chain does not match the requested dump"));
    }
    let (normalized_program, counter) =
        program.ok_or_else(|| invalid_data("NFTA_RULE_EXPRESSIONS is missing"))?;
    Ok(RuleInfo {
        handle: handle.ok_or_else(|| invalid_data("NFTA_RULE_HANDLE is missing"))?,
        userdata: userdata.unwrap_or_default(),
        counter,
        normalized_program,
    })
}

#[derive(Default)]
struct RuleDumpState {
    rules: Vec<RuleInfo>,
    done: bool,
}

#[cfg(test)]
fn decode_rule_dump_datagram(
    datagram: &[u8],
    sequence: u32,
    table: &str,
    chain: &str,
    state: &mut RuleDumpState,
) -> std::io::Result<()> {
    decode_rule_dump_datagram_family(datagram, sequence, table, chain, state, NftFamily::Ipv4)
}

fn decode_rule_dump_datagram_family(
    datagram: &[u8],
    sequence: u32,
    table: &str,
    chain: &str,
    state: &mut RuleDumpState,
    family: NftFamily,
) -> std::io::Result<()> {
    if state.done {
        return Err(invalid_data("data arrived after NLMSG_DONE"));
    }
    let mut off = 0usize;
    while off < datagram.len() {
        if datagram.len() - off < 16 {
            return Err(invalid_data("trailing partial nlmsghdr"));
        }
        let len =
            usize::try_from(ne_u32(datagram, off).ok_or_else(|| invalid_data("nlmsg length"))?)
                .map_err(|_| invalid_data("nlmsg length does not fit usize"))?;
        let message_type = ne_u16(datagram, off + 4).ok_or_else(|| invalid_data("nlmsg type"))?;
        let flags = ne_u16(datagram, off + 6).ok_or_else(|| invalid_data("nlmsg flags"))?;
        let observed_sequence =
            ne_u32(datagram, off + 8).ok_or_else(|| invalid_data("nlmsg sequence"))?;
        if len < 16 {
            return Err(invalid_data("nlmsg length smaller than header"));
        }
        let end = off.checked_add(len).ok_or_else(|| invalid_data("nlmsg length overflow"))?;
        let aligned = end
            .checked_add(3)
            .map(|value| value & !3)
            .ok_or_else(|| invalid_data("nlmsg alignment overflow"))?;
        if end > datagram.len() || aligned > datagram.len() {
            return Err(invalid_data("truncated nlmsg payload or alignment"));
        }
        if datagram[end..aligned].iter().any(|byte| *byte != 0) {
            return Err(invalid_data("non-zero nlmsg alignment padding"));
        }
        if observed_sequence != sequence {
            return Err(invalid_data("nft notification or sequence mismatch during GETRULE"));
        }
        if flags & NLM_F_DUMP_INTR != 0 {
            return Err(invalid_data("GETRULE dump was interrupted"));
        }
        let body = &datagram[off + 16..end];
        match message_type {
            kind if kind == nft_msg_type(NFT_MSG_NEWRULE) => {
                if state.done {
                    return Err(invalid_data("NEWRULE arrived after NLMSG_DONE"));
                }
                if flags & NLM_F_MULTI == 0 {
                    return Err(invalid_data("GETRULE data message is missing NLM_F_MULTI"));
                }
                state.rules.push(decode_rule_message_family(body, table, chain, family)?);
            }
            NLMSG_DONE => {
                let status: [u8; 4] = body
                    .try_into()
                    .map_err(|_| invalid_data("NLMSG_DONE status must be exactly four bytes"))?;
                if i32::from_ne_bytes(status) != 0 || state.done {
                    return Err(invalid_data("NLMSG_DONE is non-zero or duplicated"));
                }
                state.done = true;
                if aligned != datagram.len() {
                    return Err(invalid_data("extra message follows NLMSG_DONE"));
                }
            }
            NLMSG_ERROR => return Err(invalid_data("NLMSG_ERROR in GETRULE dump")),
            NLMSG_OVERRUN => return Err(invalid_data("NLMSG_OVERRUN in GETRULE dump")),
            _ => return Err(invalid_data("unexpected message type in GETRULE dump")),
        }
        off = aligned;
    }
    Ok(())
}

fn decode_generation_datagram(datagram: &[u8], sequence: u32) -> std::io::Result<u32> {
    if datagram.len() < 20 {
        return Err(invalid_data("partial GETGEN response"));
    }
    let len = usize::try_from(ne_u32(datagram, 0).ok_or_else(|| invalid_data("nlmsg length"))?)
        .map_err(|_| invalid_data("nlmsg length does not fit usize"))?;
    if len != datagram.len() || len < 20 || !len.is_multiple_of(4) {
        return Err(invalid_data("GETGEN response has malformed or trailing framing"));
    }
    if ne_u16(datagram, 4) != Some(nft_msg_type(NFT_MSG_NEWGEN))
        || ne_u32(datagram, 8) != Some(sequence)
    {
        return Err(invalid_data("GETGEN response type/sequence mismatch or notification"));
    }
    if ne_u16(datagram, 6).is_some_and(|flags| flags & NLM_F_DUMP_INTR != 0) {
        return Err(invalid_data("GETGEN response was interrupted"));
    }
    let payload = &datagram[16..];
    if payload[0] != AF_UNSPEC || payload[1] != 0 {
        return Err(invalid_data("NEWGEN carries the wrong family/version"));
    }
    let mut generation = None;
    for (kind, raw_kind, value) in exact_attrs(&payload[4..])? {
        if kind == NFTA_GEN_ID && raw_kind & NLA_F_NESTED == 0 && generation.is_none() {
            let bytes: [u8; 4] = value
                .try_into()
                .map_err(|_| invalid_data("NFTA_GEN_ID must be exactly four bytes"))?;
            generation = Some(u32::from_be_bytes(bytes));
        } else if kind == NFTA_GEN_ID {
            return Err(invalid_data("duplicate or wrongly nested NFTA_GEN_ID"));
        }
    }
    match generation {
        Some(0) | None => Err(invalid_data("NFTA_GEN_ID is missing or zero")),
        Some(value) => Ok(value),
    }
}

/// Walk the top-level attributes of one message body (`body`), invoking `visit`
/// with `(type_without_nested_flag, payload)` for each. Bounds-checked; a
/// truncated attribute stops the walk (safe-slice, never a panic).
#[cfg(test)]
fn for_each_attr(body: &[u8], mut visit: impl FnMut(u16, &[u8])) {
    let mut off = 0usize;
    while off + 4 <= body.len() {
        let Some(alen) = ne_u16(body, off).map(usize::from) else { break };
        let Some(atype) = ne_u16(body, off + 2).map(|t| t & !NLA_F_NESTED) else { break };
        if alen < 4 || off + alen > body.len() {
            break;
        }
        visit(atype, &body[off + 4..off + alen]);
        off += (alen + 3) & !3;
    }
}

/// Decode a `GETRULE` dump reply into `(handle, userdata)` pairs — the
/// structural read of `NFTA_RULE_HANDLE` + `NFTA_RULE_USERDATA` that replaces
/// the `# handle N` text scrape (ADR-0085 D10). Walks each `NEWRULE`-typed
/// nlmsg; `NLMSG_DONE` / `NLMSG_ERROR` / other types are skipped. Bounds-checked
/// throughout (a truncated reply yields the rules decoded so far, never a panic).
#[cfg(test)]
fn parse_rules(reply: &[u8]) -> Vec<RuleInfo> {
    let mut out = Vec::new();
    let newrule = nft_msg_type(NFT_MSG_NEWRULE);
    let mut off = 0usize;
    while off + 16 <= reply.len() {
        let Some(mlen) = ne_u32(reply, off).map(|l| l as usize) else { break };
        let Some(mtype) = ne_u16(reply, off + 4) else { break };
        if mlen < 16 || off + mlen > reply.len() {
            break;
        }
        if mtype == newrule && mlen >= 20 {
            // Message body = nlmsghdr(16) + nfgenmsg(4) + attributes.
            let body = &reply[off + 20..off + mlen];
            let mut handle: Option<u64> = None;
            let mut udata: Vec<u8> = Vec::new();
            let mut counter = None;
            let mut normalized_program = Vec::new();
            for_each_attr(body, |atype, payload| {
                if atype == NFTA_RULE_HANDLE && payload.len() >= 8 {
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&payload[..8]);
                    handle = Some(u64::from_be_bytes(bytes));
                } else if atype == NFTA_RULE_USERDATA {
                    udata = payload.to_vec();
                } else if atype == NFTA_RULE_EXPRESSIONS
                    && let Ok((program, sampled_counter)) = normalize_rule_program(payload, true)
                {
                    normalized_program = program;
                    counter = sampled_counter;
                }
            });
            if let Some(handle) = handle {
                out.push(RuleInfo { handle, userdata: udata, counter, normalized_program });
            }
        }
        off += (mlen + 3) & !3;
    }
    out
}

// =============================================================================
// Impure NETLINK_NETFILTER socket I/O — the async-free public operation surface
// =============================================================================

/// A raw `NETLINK_NETFILTER` socket (spike increment-e's proven dance).
#[derive(Debug)]
struct NfSock {
    fd: i32,
}

fn validate_netfilter_sender(
    sender_len: libc::socklen_t,
    sender: &libc::sockaddr_nl,
) -> std::io::Result<()> {
    if sender_len as usize != std::mem::size_of::<libc::sockaddr_nl>()
        || sender.nl_family != libc::AF_NETLINK as libc::sa_family_t
        || sender.nl_pid != 0
    {
        return Err(invalid_data("netfilter datagram sender is not the kernel"));
    }
    Ok(())
}

fn classify_notification_receive(result: std::io::Result<usize>) -> std::io::Result<()> {
    match result {
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(error),
        Ok(0) => Err(Error::new(ErrorKind::UnexpectedEof, "notification socket EOF")),
        Ok(_) => Err(invalid_data("nftables mutation notification observed")),
    }
}

impl NfSock {
    fn open() -> std::io::Result<Self> {
        Self::open_with_groups(0)
    }

    fn open_with_groups(groups: u32) -> std::io::Result<Self> {
        // SAFETY: `socket(2)` with valid domain/type/protocol constants; the
        // returned fd is checked and owned (closed in `Drop`).
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_NETFILTER) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        addr.nl_groups = groups;
        // SAFETY: `addr` is a zeroed, correctly-typed `sockaddr_nl` of the size
        // passed; `fd` is a valid netlink socket.
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
        let this = Self { fd };
        this.set_recv_timeout(OBSERVATION_DEADLINE)?;
        Ok(this)
    }

    fn set_recv_timeout(&self, timeout: Duration) -> std::io::Result<()> {
        let seconds = timeout.as_secs().min(i64::MAX as u64) as libc::time_t;
        let micros = i64::from(timeout.subsec_micros()) as libc::suseconds_t;
        let tv = libc::timeval { tv_sec: seconds, tv_usec: micros };
        // SAFETY: `tv` is a valid initialised `timeval` of the size passed;
        // `self.fd` is an open netlink socket. Failure is propagated because a
        // missing bound is not a valid strict observation channel.
        let result = unsafe {
            libc::setsockopt(
                self.fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                std::ptr::addr_of!(tv).cast::<libc::c_void>(),
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if result < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn send(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut dst: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        dst.nl_family = libc::AF_NETLINK as libc::sa_family_t; // nl_pid=0 => kernel
        // SAFETY: `bytes` is a valid initialised buffer of the passed length;
        // `dst` is a zeroed, correctly-sized `sockaddr_nl`; `self.fd` is open.
        let sent = unsafe {
            libc::sendto(
                self.fd,
                bytes.as_ptr().cast::<libc::c_void>(),
                bytes.len(),
                0,
                std::ptr::addr_of!(dst).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if sent < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: `buf` is a valid initialised buffer of `buf.len()`; `self.fd`
        // is open.
        let n =
            unsafe { libc::recv(self.fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    fn recv_from(&self, buf: &mut [u8], flags: i32) -> std::io::Result<(usize, libc::sockaddr_nl)> {
        let mut sender: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        let mut sender_len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        // SAFETY: `buf` and `sender` are writable live storage of the lengths
        // passed; `self.fd` is an open netlink socket.
        let received = unsafe {
            libc::recvfrom(
                self.fd,
                buf.as_mut_ptr().cast::<libc::c_void>(),
                buf.len(),
                flags,
                std::ptr::from_mut(&mut sender).cast::<libc::sockaddr>(),
                std::ptr::from_mut(&mut sender_len),
            )
        };
        if received < 0 {
            return Err(std::io::Error::last_os_error());
        }
        validate_netfilter_sender(sender_len, &sender)?;
        Ok((received as usize, sender))
    }
}

impl Drop for NfSock {
    fn drop(&mut self) {
        // SAFETY: `self.fd` is a valid descriptor opened in `open` and not
        // otherwise closed.
        unsafe { libc::close(self.fd) };
    }
}

/// Walk a recv buffer for the first `NLMSG_ERROR`; `Ok(())` on an ACK (`code ==
/// 0`), `Err(io::Error)` carrying the positive kernel errno on a NACK. Returns
/// `Ok(())` when no `NLMSG_ERROR` is present (the mutation was accepted with no
/// explicit ACK — downstream reads confirm).
fn batch_ack(buf: &[u8]) -> std::io::Result<()> {
    let mut off = 0usize;
    while off + 16 <= buf.len() {
        let Some(mlen) = ne_u32(buf, off).map(|l| l as usize) else { break };
        let Some(mtype) = ne_u16(buf, off + 4) else { break };
        if mlen < 16 || off + mlen > buf.len() {
            break;
        }
        if mtype == NLMSG_ERROR {
            // struct nlmsgerr { int error; struct nlmsghdr msg; } — error at +16.
            let code = ne_u32(buf, off + 16).map_or(0, |c| c as i32);
            if code == 0 {
                return Ok(());
            }
            return Err(std::io::Error::from_raw_os_error(code.abs()));
        }
        off += (mlen + 3) & !3;
    }
    Ok(())
}

fn atomic_rule_batch(mutations: &[AtomicRuleMutation<'_>]) -> Vec<u8> {
    let mut batch = Vec::new();
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_BEGIN,
        NLM_F_REQUEST,
        1,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    for (index, mutation) in mutations.iter().enumerate() {
        let seq = index as u32 + 2;
        match mutation {
            AtomicRuleMutation::Delete { table, chain, handle } => nlmsg(
                &mut batch,
                nft_msg_type(NFT_MSG_DELRULE),
                NLM_F_REQUEST | NLM_F_ACK,
                seq,
                &delrule_payload(table, chain, *handle),
            ),
            AtomicRuleMutation::Insert { table, chain, exprs, userdata } => nlmsg(
                &mut batch,
                nft_msg_type(NFT_MSG_NEWRULE),
                NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE,
                seq,
                &newrule_payload(table, chain, exprs, userdata),
            ),
        }
    }
    let end_seq = mutations.len() as u32 + 2;
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        end_seq,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    batch
}

fn collect_atomic_rule_acks(buf: &[u8], pending: &mut BTreeSet<u32>) -> std::io::Result<()> {
    let mut off = 0usize;
    while off + 16 <= buf.len() {
        let Some(mlen) = ne_u32(buf, off).map(|length| length as usize) else { break };
        let Some(mtype) = ne_u16(buf, off + 4) else { break };
        if mlen < 16 || off + mlen > buf.len() {
            break;
        }
        if mtype == NLMSG_ERROR {
            let code = ne_u32(buf, off + 16).map_or(0, |value| value as i32);
            if code != 0 {
                return Err(std::io::Error::from_raw_os_error(code.abs()));
            }
            if let Some(seq) = ne_u32(buf, off + 8) {
                pending.remove(&seq);
            }
        }
        off += (mlen + 3) & !3;
    }
    Ok(())
}

fn send_atomic_rule_transaction(mutations: &[AtomicRuleMutation<'_>]) -> Result<(), NetlinkError> {
    if mutations.is_empty() {
        return Ok(());
    }
    let sock =
        NfSock::open().map_err(|error| NetlinkError::nft("atomic-rule-transaction", error))?;
    let batch = atomic_rule_batch(mutations);
    sock.send(&batch).map_err(|error| NetlinkError::nft("atomic-rule-transaction", error))?;

    let end = mutations.len() as u32 + 2;
    let mut pending = (2..end).collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        let mut buf = vec![0u8; 32768];
        let received = sock
            .recv(&mut buf)
            .map_err(|error| NetlinkError::nft("atomic-rule-transaction", error))?;
        collect_atomic_rule_acks(&buf[..received], &mut pending)
            .map_err(|error| NetlinkError::nft("atomic-rule-transaction", error))?;
    }
    Ok(())
}

struct BridgeRuleMutation<'a> {
    table: &'a str,
    chain: &'a str,
    expressions: &'a [u8],
    userdata: &'a [u8],
    append: bool,
}

fn send_atomic_bridge_rule_transaction(
    mutations: &[BridgeRuleMutation<'_>],
) -> Result<(), NetlinkError> {
    if mutations.is_empty() {
        return Ok(());
    }
    let sock = NfSock::open()
        .map_err(|error| NetlinkError::nft("bridge-atomic-rule-transaction", error))?;
    let mut batch = Vec::new();
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_BEGIN,
        NLM_F_REQUEST,
        1,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    for (index, mutation) in mutations.iter().enumerate() {
        let mut flags = NLM_F_CREATE;
        if mutation.append {
            flags |= NLM_F_APPEND;
        }
        nlmsg(
            &mut batch,
            nft_msg_type(NFT_MSG_NEWRULE),
            NLM_F_REQUEST | NLM_F_ACK | flags,
            index as u32 + 2,
            &newrule_payload_family(
                NftFamily::Bridge,
                mutation.table,
                mutation.chain,
                mutation.expressions,
                mutation.userdata,
            ),
        );
    }
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        mutations.len() as u32 + 2,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    sock.send(&batch)
        .map_err(|error| NetlinkError::nft("bridge-atomic-rule-transaction", error))?;
    let end = mutations.len() as u32 + 2;
    let mut pending = (2..end).collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        let mut buffer = vec![0_u8; 32_768];
        let received = sock
            .recv(&mut buffer)
            .map_err(|error| NetlinkError::nft("bridge-atomic-rule-transaction", error))?;
        collect_atomic_rule_acks(&buffer[..received], &mut pending)
            .map_err(|error| NetlinkError::nft("bridge-atomic-rule-transaction", error))?;
    }
    Ok(())
}

/// Send one batched nftables mutation (`BATCH_BEGIN` + `msg` + `BATCH_END`) and
/// read its ACK. `op` is the `NFT_MSG_*` op; `flags` the extra flags on the
/// mutation message (`NLM_F_ACK` is always added). `-EEXIST` is NOT swallowed
/// here — callers that want idempotency inspect [`NetlinkError::errno`].
fn send_batched(
    op: u16,
    flags: u16,
    payload: &[u8],
    sock_op: &'static str,
) -> Result<(), NetlinkError> {
    let sock = NfSock::open().map_err(|e| NetlinkError::nft(sock_op, e))?;
    // BATCH_BEGIN(20) + mutation(16 + payload, padded) + BATCH_END(20).
    let mut batch = Vec::with_capacity(40 + (16 + payload.len()).next_multiple_of(4));
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_BEGIN,
        NLM_F_REQUEST,
        1,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    nlmsg(&mut batch, nft_msg_type(op), NLM_F_REQUEST | NLM_F_ACK | flags, 2, payload);
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        3,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );

    sock.send(&batch).map_err(|e| NetlinkError::nft(sock_op, e))?;
    let mut buf = vec![0u8; 32768];
    let n = sock.recv(&mut buf).map_err(|e| NetlinkError::nft(sock_op, e))?;
    batch_ack(&buf[..n]).map_err(|e| NetlinkError::nft(sock_op, e))
}

fn send_batched_family(
    family: NftFamily,
    op: u16,
    flags: u16,
    payload: &[u8],
    sock_op: &'static str,
) -> Result<(), NetlinkError> {
    let sock = NfSock::open().map_err(|e| NetlinkError::nft(sock_op, e))?;
    let mut batch = Vec::with_capacity(40 + (16 + payload.len()).next_multiple_of(4));
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_BEGIN,
        NLM_F_REQUEST,
        1,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    nlmsg(&mut batch, nft_msg_type(op), NLM_F_REQUEST | NLM_F_ACK | flags, 2, payload);
    nlmsg(
        &mut batch,
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        3,
        &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
    );
    // Keep the family argument at this private codec boundary explicit.  The
    // family is carried in the operation payload; the transaction envelope is
    // intentionally AF_UNSPEC per nf_tables ABI.
    let _ = family;
    sock.send(&batch).map_err(|e| NetlinkError::nft(sock_op, e))?;
    let mut buf = vec![0u8; 32_768];
    let n = sock.recv(&mut buf).map_err(|e| NetlinkError::nft(sock_op, e))?;
    batch_ack(&buf[..n]).map_err(|e| NetlinkError::nft(sock_op, e))
}

fn send_batched_family_idempotent(
    family: NftFamily,
    op: u16,
    payload: &[u8],
    sock_op: &'static str,
) -> Result<(), NetlinkError> {
    match send_batched_family(family, op, NLM_F_CREATE, payload, sock_op) {
        Ok(()) => Ok(()),
        Err(error) if error.errno() == Some(NEG_EEXIST) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Send a mutation, swallowing `-EEXIST` as idempotent success (the netlink
/// analogue of `nft add table` / `nft add chain` being create-if-missing).
fn send_batched_idempotent(
    op: u16,
    payload: &[u8],
    sock_op: &'static str,
) -> Result<(), NetlinkError> {
    match send_batched(op, NLM_F_CREATE, payload, sock_op) {
        Ok(()) => Ok(()),
        Err(err) if err.errno() == Some(NEG_EEXIST) => Ok(()),
        Err(err) => Err(err),
    }
}

/// `nft add table ip <table>` — idempotent create-if-missing (`-EEXIST` swallowed).
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "ensure-table"`) on a non-`EEXIST` failure.
pub fn ensure_table(table: &str) -> Result<(), NetlinkError> {
    send_batched_idempotent(NFT_MSG_NEWTABLE, &newtable_payload(table), "ensure-table")
}

/// Delete one `ip` family table by exact name.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "delete-table"`) on failure.
pub fn delete_table(table: &str) -> Result<(), NetlinkError> {
    send_batched(NFT_MSG_DELTABLE, 0, &newtable_payload(table), "delete-table")
}

/// `nft add chain ip <table> <chain> { type <T> hook <H> priority <P>; policy
/// accept; }` — idempotent create-if-missing (`-EEXIST` swallowed).
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "ensure-chain"`) on a non-`EEXIST` failure.
pub fn ensure_base_chain(
    table: &str,
    chain: &str,
    spec: BaseChainSpec,
) -> Result<(), NetlinkError> {
    send_batched_idempotent(NFT_MSG_NEWCHAIN, &newchain_payload(table, chain, spec), "ensure-chain")
}

/// Delete one empty chain by exact table/name identity.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "delete-chain"`) on failure.
pub fn delete_chain(table: &str, chain: &str) -> Result<(), NetlinkError> {
    send_batched(
        NFT_MSG_DELCHAIN,
        0,
        &get_by_table_chain(table, chain, NFTA_CHAIN_NAME, NFTA_CHAIN_TABLE),
        "delete-chain",
    )
}

/// `nft add rule ip <table> <chain> <exprs>` — append (after existing rules),
/// carrying the `userdata` identity tag for later structural handle recovery.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "append-rule"`) on failure.
pub fn append_rule(
    table: &str,
    chain: &str,
    exprs: &[u8],
    userdata: &[u8],
) -> Result<(), NetlinkError> {
    send_batched(
        NFT_MSG_NEWRULE,
        NLM_F_CREATE | NLM_F_APPEND,
        &newrule_payload(table, chain, exprs, userdata),
        "append-rule",
    )
}

/// `nft insert rule ip <table> <chain> <exprs>` — prepend (at the chain head),
/// carrying the `userdata` identity tag. Used for the leg-S exemption so it
/// precedes every per-workload rule.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "insert-rule"`) on failure.
pub fn insert_rule(
    table: &str,
    chain: &str,
    exprs: &[u8],
    userdata: &[u8],
) -> Result<(), NetlinkError> {
    send_batched(
        NFT_MSG_NEWRULE,
        NLM_F_CREATE,
        &newrule_payload(table, chain, exprs, userdata),
        "insert-rule",
    )
}

/// Apply an ordered set of audited rule deletes/inserts in one nftables batch.
///
/// `NFNL_MSG_BATCH_BEGIN`/`END` gives the kernel one atomic transaction: every
/// mutation commits, or any failed operation rolls the complete batch back.
/// Each operation requests its own ACK, and this function drains every ACK so
/// a late failure cannot be mistaken for an earlier successful mutation.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "atomic-rule-transaction"`) if any operation
/// is rejected or the netlink transaction cannot be sent/acknowledged.
pub fn apply_rule_transaction_atomically(
    mutations: &[AtomicRuleMutation<'_>],
) -> Result<(), NetlinkError> {
    send_atomic_rule_transaction(mutations)
}

fn remaining_until(deadline: Instant) -> std::io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| Error::new(ErrorKind::TimedOut, "absolute nft observation deadline expired"))
}

fn send_get_rules(sock: &NfSock, table: &str, chain: &str, sequence: u32) -> std::io::Result<()> {
    send_get_rules_family(sock, table, chain, sequence, NftFamily::Ipv4)
}

fn send_get_rules_family(
    sock: &NfSock,
    table: &str,
    chain: &str,
    sequence: u32,
    family: NftFamily,
) -> std::io::Result<()> {
    let payload = get_by_table_chain_family(family, table, chain, NFTA_RULE_CHAIN, NFTA_RULE_TABLE);
    let mut message = Vec::with_capacity((16 + payload.len()).next_multiple_of(4));
    nlmsg(
        &mut message,
        nft_msg_type(NFT_MSG_GETRULE),
        NLM_F_REQUEST | NLM_F_DUMP,
        sequence,
        &payload,
    );
    sock.send(&message)
}

fn receive_rule_dump(
    sock: &NfSock,
    sequence: u32,
    table: &str,
    chain: &str,
    deadline: Instant,
) -> std::io::Result<Vec<RuleInfo>> {
    receive_rule_dump_family(sock, sequence, table, chain, deadline, NftFamily::Ipv4)
}

fn receive_rule_dump_family(
    sock: &NfSock,
    sequence: u32,
    table: &str,
    chain: &str,
    deadline: Instant,
    family: NftFamily,
) -> std::io::Result<Vec<RuleInfo>> {
    let mut state = RuleDumpState::default();
    while !state.done {
        sock.set_recv_timeout(remaining_until(deadline)?)?;
        let mut datagram = vec![0u8; 65_535];
        let (received, _) = sock.recv_from(&mut datagram, libc::MSG_TRUNC)?;
        if received == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "EOF before NLMSG_DONE"));
        }
        if received > datagram.len() {
            return Err(invalid_data("truncated netfilter datagram"));
        }
        decode_rule_dump_datagram_family(
            &datagram[..received],
            sequence,
            table,
            chain,
            &mut state,
            family,
        )?;
    }
    Ok(state.rules)
}

fn request_generation(sock: &NfSock, sequence: u32, deadline: Instant) -> std::io::Result<u32> {
    let payload = nfgenmsg(AF_UNSPEC, 0);
    let mut message = Vec::with_capacity(20);
    nlmsg(&mut message, nft_msg_type(NFT_MSG_GETGEN), NLM_F_REQUEST, sequence, &payload);
    sock.send(&message)?;
    sock.set_recv_timeout(remaining_until(deadline)?)?;
    let mut datagram = vec![0u8; 65_535];
    let (received, _) = sock.recv_from(&mut datagram, libc::MSG_TRUNC)?;
    if received == 0 {
        return Err(Error::new(ErrorKind::UnexpectedEof, "EOF before NEWGEN"));
    }
    if received > datagram.len() {
        return Err(invalid_data("truncated GETGEN datagram"));
    }
    decode_generation_datagram(&datagram[..received], sequence)
}

impl NftRuleObserver {
    /// Subscribe to nftables notifications on a fresh dedicated read-only
    /// socket. The subscription is active before the first snapshot request.
    ///
    /// # Errors
    ///
    /// Returns [`NetlinkError::Nft`] if the socket cannot be opened, bound, or
    /// subscribed.
    pub fn subscribe() -> Result<Self, NetlinkError> {
        let groups = 1_u32.checked_shl(NFNLGRP_NFTABLES - 1).ok_or_else(|| {
            NetlinkError::nft("observe-subscribe", invalid_data("invalid nftables group"))
        })?;
        let socket = NfSock::open_with_groups(groups)
            .map_err(|source| NetlinkError::nft("observe-subscribe", source))?;
        Ok(Self { socket, next_sequence: 1 })
    }

    fn sequence(&mut self) -> u32 {
        let current = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        current
    }

    /// Take one strict `GETGEN -> GETRULE -> GETGEN` snapshot.
    ///
    /// # Errors
    ///
    /// Fails closed on notification, loss/overrun, sequence/family mismatch,
    /// malformed or partial framing, timeout/EOF, interrupted dump, missing or
    /// duplicate completion, or a changed/zero generation.
    pub fn snapshot(&mut self, table: &str, chain: &str) -> Result<RuleSnapshot, NetlinkError> {
        let deadline = Instant::now() + OBSERVATION_DEADLINE;
        let before_sequence = self.sequence();
        let before = request_generation(&self.socket, before_sequence, deadline)
            .map_err(|source| NetlinkError::nft("observe-getgen-before", source))?;
        let rules_sequence = self.sequence();
        send_get_rules(&self.socket, table, chain, rules_sequence)
            .map_err(|source| NetlinkError::nft("observe-getrule-send", source))?;
        let rules = receive_rule_dump(&self.socket, rules_sequence, table, chain, deadline)
            .map_err(|source| NetlinkError::nft("observe-getrule", source))?;
        let after_sequence = self.sequence();
        let after = request_generation(&self.socket, after_sequence, deadline)
            .map_err(|source| NetlinkError::nft("observe-getgen-after", source))?;
        if before != after {
            return Err(NetlinkError::nft(
                "observe-generation",
                invalid_data("ruleset generation changed across GETRULE"),
            ));
        }
        Ok(RuleSnapshot { generation: before, rules })
    }

    /// Verify that the subscribed socket has no queued nftables notification.
    ///
    /// # Errors
    ///
    /// Any queued datagram, reported overrun, malformed read, or unexpected
    /// socket error fails the observer closed.
    pub fn ensure_no_notifications(&self) -> Result<(), NetlinkError> {
        let mut datagram = vec![0u8; 65_535];
        let received = self
            .socket
            .recv_from(&mut datagram, libc::MSG_DONTWAIT | libc::MSG_TRUNC)
            .map(|(length, _)| length);
        classify_notification_receive(received)
            .map_err(|source| NetlinkError::nft("observe-notifications", source))
    }
}

/// Strictly dump every rule in `ip <table> <chain>` with its handle, userdata,
/// anonymous counter, and normalized full expression program.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "list-rules"`) on a socket / kernel failure.
pub fn list_rules(table: &str, chain: &str) -> Result<Vec<RuleInfo>, NetlinkError> {
    list_rules_family(NftFamily::Ipv4, table, chain)
}

fn list_rules_family(
    family: NftFamily,
    table: &str,
    chain: &str,
) -> Result<Vec<RuleInfo>, NetlinkError> {
    let sock = NfSock::open().map_err(|e| NetlinkError::nft("list-rules", e))?;
    let sequence = 1;
    send_get_rules_family(&sock, table, chain, sequence, family)
        .map_err(|source| NetlinkError::nft("list-rules", source))?;
    receive_rule_dump_family(
        &sock,
        sequence,
        table,
        chain,
        Instant::now() + OBSERVATION_DEADLINE,
        family,
    )
    .map_err(|source| NetlinkError::nft("list-rules", source))
}

/// One raw chain identity recovered from a family-specific GETCHAIN dump.
///
/// This stays private to the codec.  The bridge adapter projects it into its
/// closed semantic chain shape only after the complete dump has been read.
#[derive(Debug, Clone)]
struct RawChainInfo {
    table: String,
    name: String,
    handle: u64,
    hook: Option<(u32, i32)>,
    chain_type: Option<String>,
    policy: Option<u32>,
}

#[derive(Debug, Clone)]
struct RawSetInfo {
    name: String,
    key_type: u32,
    key_len: u32,
    id: u32,
    userdata: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawOtherChildKind {
    Flowtable,
    StatefulObject,
}

#[derive(Debug, Clone)]
struct RawOtherChildInfo {
    kind: RawOtherChildKind,
    name: String,
    handle: Option<u64>,
}

/// Complete one family-specific multipart dump, preserving every data
/// message and rejecting sequence, framing, family, and generation errors at
/// this private boundary.
#[allow(
    clippy::too_many_lines,
    reason = "strict private multipart decoder keeps all framing checks together"
)]
fn receive_nft_dump_family(
    family: NftFamily,
    operation: u16,
    payload: &[u8],
    label: &'static str,
) -> Result<Vec<Vec<u8>>, NetlinkError> {
    let sock = NfSock::open().map_err(|source| NetlinkError::nft(label, source))?;
    let sequence = 1;
    let mut message = Vec::with_capacity((16 + payload.len()).next_multiple_of(4));
    nlmsg(&mut message, nft_msg_type(operation), NLM_F_REQUEST | NLM_F_DUMP, sequence, payload);
    sock.send(&message).map_err(|source| NetlinkError::nft(label, source))?;
    let expected_type = nft_msg_type(match operation {
        NFT_MSG_GETTABLE => NFT_MSG_NEWTABLE,
        NFT_MSG_GETCHAIN => NFT_MSG_NEWCHAIN,
        NFT_MSG_GETSET => NFT_MSG_NEWSET,
        NFT_MSG_GETSETELEM => NFT_MSG_NEWSETELEM,
        NFT_MSG_GETOBJ => NFT_MSG_NEWOBJ,
        NFT_MSG_GETFLOWTABLE => NFT_MSG_NEWFLOWTABLE,
        _ => operation,
    });
    let deadline = Instant::now() + OBSERVATION_DEADLINE;
    let mut data = Vec::new();
    let mut done = false;
    while !done {
        sock.set_recv_timeout(
            remaining_until(deadline).map_err(|source| NetlinkError::nft(label, source))?,
        )
        .map_err(|source| NetlinkError::nft(label, source))?;
        let mut datagram = vec![0_u8; 65_535];
        let (received, _) = sock
            .recv_from(&mut datagram, libc::MSG_TRUNC)
            .map_err(|source| NetlinkError::nft(label, source))?;
        if received == 0 || received > datagram.len() {
            return Err(NetlinkError::nft(label, invalid_data("truncated nft dump datagram")));
        }
        let mut offset = 0usize;
        while offset < received {
            if received - offset < 16 {
                return Err(NetlinkError::nft(label, invalid_data("truncated nft dump header")));
            }
            let length = ne_u32(&datagram, offset)
                .ok_or_else(|| NetlinkError::nft(label, invalid_data("missing nft dump length")))?
                as usize;
            let kind = ne_u16(&datagram, offset + 4)
                .ok_or_else(|| NetlinkError::nft(label, invalid_data("missing nft dump type")))?;
            let flags = ne_u16(&datagram, offset + 6)
                .ok_or_else(|| NetlinkError::nft(label, invalid_data("missing nft dump flags")))?;
            let observed_sequence = ne_u32(&datagram, offset + 8).ok_or_else(|| {
                NetlinkError::nft(label, invalid_data("missing nft dump sequence"))
            })?;
            if length < 16 || offset + length > received {
                return Err(NetlinkError::nft(
                    label,
                    invalid_data("invalid nft dump message length"),
                ));
            }
            let aligned = (offset + length + 3) & !3;
            if aligned > received
                || datagram[offset + length..aligned].iter().any(|byte| *byte != 0)
            {
                return Err(NetlinkError::nft(
                    label,
                    invalid_data("invalid nft dump message padding"),
                ));
            }
            if observed_sequence != sequence {
                return Err(NetlinkError::nft(label, invalid_data("nft dump sequence mismatch")));
            }
            if flags & NLM_F_DUMP_INTR != 0 {
                return Err(NetlinkError::nft(label, invalid_data("nft dump was interrupted")));
            }
            let body = &datagram[offset + 16..offset + length];
            match kind {
                value if value == expected_type => {
                    if flags & NLM_F_MULTI == 0 || body.len() < 4 {
                        return Err(NetlinkError::nft(
                            label,
                            invalid_data("nft dump data lacks multipart/family framing"),
                        ));
                    }
                    if body[0] != family.nfproto() || body[1] != 0 {
                        return Err(NetlinkError::nft(
                            label,
                            invalid_data("nft dump data carries the wrong family"),
                        ));
                    }
                    // Validate all nested attributes now; individual decoders
                    // below then receive a structurally safe payload.
                    exact_attrs(&body[4..]).map_err(|source| NetlinkError::nft(label, source))?;
                    data.push(body.to_vec());
                }
                NLMSG_DONE => {
                    if body.len() != 4 || ne_u32(body, 0) != Some(0) || done {
                        return Err(NetlinkError::nft(
                            label,
                            invalid_data("invalid or duplicate nft dump completion"),
                        ));
                    }
                    done = true;
                    if aligned != received {
                        return Err(NetlinkError::nft(
                            label,
                            invalid_data("nft dump data follows completion"),
                        ));
                    }
                }
                NLMSG_ERROR => {
                    let code = ne_u32(body, 0).map_or(0, |value| value as i32);
                    if code != 0 {
                        return Err(NetlinkError::nft(label, Error::from_raw_os_error(code.abs())));
                    }
                    return Err(NetlinkError::nft(
                        label,
                        invalid_data("unexpected nft dump acknowledgement"),
                    ));
                }
                NLMSG_OVERRUN => {
                    return Err(NetlinkError::nft(label, invalid_data("nft dump overrun")));
                }
                _ => {
                    return Err(NetlinkError::nft(
                        label,
                        invalid_data("unexpected nft dump message type"),
                    ));
                }
            }
            offset = aligned;
        }
    }
    Ok(data)
}

fn family_only_payload(family: NftFamily) -> Vec<u8> {
    nfgenmsg(family.nfproto(), 0)
}

fn family_table_payload(family: NftFamily, table: &str, table_attr: u16) -> Vec<u8> {
    let mut payload = nfgenmsg(family.nfproto(), 0);
    attr(&mut payload, table_attr, &cstr(table));
    payload
}

fn list_table_names_family(family: NftFamily) -> Result<Vec<String>, NetlinkError> {
    let bodies = receive_nft_dump_family(
        family,
        NFT_MSG_GETTABLE,
        &family_only_payload(family),
        "list-tables",
    )?;
    bodies
        .into_iter()
        .map(|body| {
            let mut name = None;
            for (kind, raw_kind, value) in exact_attrs(&body[4..])
                .map_err(|source| NetlinkError::nft("list-tables", source))?
            {
                if kind == NFTA_TABLE_NAME
                    && raw_kind & NLA_F_NESTED == 0
                    && name
                        .replace(
                            exact_cstr(value, "NFTA_TABLE_NAME")
                                .map_err(|source| NetlinkError::nft("list-tables", source))?
                                .to_owned(),
                        )
                        .is_some()
                {
                    return Err(NetlinkError::nft(
                        "list-tables",
                        invalid_data("duplicate table name"),
                    ));
                }
            }
            name.ok_or_else(|| {
                NetlinkError::nft("list-tables", invalid_data("table name is missing"))
            })
        })
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "strict private chain decoder keeps all semantic fields together"
)]
fn list_chain_info_family(
    family: NftFamily,
    table: &str,
) -> Result<Vec<RawChainInfo>, NetlinkError> {
    let bodies = receive_nft_dump_family(
        family,
        NFT_MSG_GETCHAIN,
        &family_table_payload(family, table, NFTA_CHAIN_TABLE),
        "list-chains",
    )?;
    let entries = bodies
        .into_iter()
        .map(|body| {
            let mut observed_table = None;
            let mut name = None;
            let mut handle = None;
            let mut hook = None;
            let mut chain_type = None;
            let mut policy = None;
            for (kind, raw_kind, value) in exact_attrs(&body[4..])
                .map_err(|source| NetlinkError::nft("list-chains", source))?
            {
                match kind {
                    NFTA_CHAIN_TABLE
                        if raw_kind & NLA_F_NESTED == 0 && observed_table.is_none() =>
                    {
                        observed_table = Some(
                            exact_cstr(value, "NFTA_CHAIN_TABLE")
                                .map_err(|source| NetlinkError::nft("list-chains", source))?
                                .to_owned(),
                        );
                    }
                    NFTA_CHAIN_NAME if raw_kind & NLA_F_NESTED == 0 && name.is_none() => {
                        name = Some(
                            exact_cstr(value, "NFTA_CHAIN_NAME")
                                .map_err(|source| NetlinkError::nft("list-chains", source))?
                                .to_owned(),
                        );
                    }
                    NFTA_CHAIN_HANDLE if raw_kind & NLA_F_NESTED == 0 && handle.is_none() => {
                        handle = Some(
                            exact_be_u64(value, "NFTA_CHAIN_HANDLE")
                                .map_err(|source| NetlinkError::nft("list-chains", source))?,
                        );
                    }
                    // Kernels in the supported range omit NLA_F_NESTED on
                    // this reply even though the payload is the nested hook
                    // attribute set; decode the semantic payload, not that
                    // optional wire flag.
                    NFTA_CHAIN_HOOK if hook.is_none() => {
                        let mut hooknum = None;
                        let mut priority = None;
                        for (hook_kind, hook_raw, hook_value) in exact_attrs(value)
                            .map_err(|source| NetlinkError::nft("list-chains", source))?
                        {
                            match hook_kind {
                                NFTA_HOOK_HOOKNUM
                                    if hook_raw & NLA_F_NESTED == 0 && hooknum.is_none() =>
                                {
                                    hooknum = Some(
                                        exact_be_u32(hook_value, "NFTA_HOOK_HOOKNUM").map_err(
                                            |source| NetlinkError::nft("list-chains", source),
                                        )?,
                                    );
                                }
                                NFTA_HOOK_PRIORITY
                                    if hook_raw & NLA_F_NESTED == 0 && priority.is_none() =>
                                {
                                    priority = Some(i32::from_be_bytes(
                                        exact_be_u32(hook_value, "NFTA_HOOK_PRIORITY")
                                            .map_err(|source| {
                                                NetlinkError::nft("list-chains", source)
                                            })?
                                            .to_be_bytes(),
                                    ));
                                }
                                _ => {}
                            }
                        }
                        hook = Some((
                            hooknum.ok_or_else(|| {
                                NetlinkError::nft(
                                    "list-chains",
                                    invalid_data("chain hook number is missing"),
                                )
                            })?,
                            priority.ok_or_else(|| {
                                NetlinkError::nft(
                                    "list-chains",
                                    invalid_data("chain hook priority is missing"),
                                )
                            })?,
                        ));
                    }
                    NFTA_CHAIN_TYPE if raw_kind & NLA_F_NESTED == 0 && chain_type.is_none() => {
                        chain_type = Some(
                            exact_cstr(value, "NFTA_CHAIN_TYPE")
                                .map_err(|source| NetlinkError::nft("list-chains", source))?
                                .to_owned(),
                        );
                    }
                    NFTA_CHAIN_POLICY if raw_kind & NLA_F_NESTED == 0 && policy.is_none() => {
                        policy = Some(
                            exact_be_u32(value, "NFTA_CHAIN_POLICY")
                                .map_err(|source| NetlinkError::nft("list-chains", source))?,
                        );
                    }
                    _ => {}
                }
            }
            let observed_table = observed_table.ok_or_else(|| {
                NetlinkError::nft("list-chains", invalid_data("chain table is missing"))
            })?;
            if observed_table != table {
                // Some kernels ignore the table selector on a family dump
                // and return foreign-table children as well.  They are not
                // candidates for this table; preserve them by excluding
                // them from this table-local projection.
                return Ok(None);
            }
            Ok(Some(RawChainInfo {
                table: observed_table,
                name: name.ok_or_else(|| {
                    NetlinkError::nft("list-chains", invalid_data("chain name is missing"))
                })?,
                handle: handle.ok_or_else(|| {
                    NetlinkError::nft("list-chains", invalid_data("chain handle is missing"))
                })?,
                hook,
                chain_type,
                policy,
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entries.into_iter().flatten().collect())
}

fn list_set_info_family(family: NftFamily, table: &str) -> Result<Vec<RawSetInfo>, NetlinkError> {
    let bodies = receive_nft_dump_family(
        family,
        NFT_MSG_GETSET,
        &family_table_payload(family, table, NFTA_SET_TABLE),
        "list-sets",
    )?;
    let entries = bodies
        .into_iter()
        .map(|body| {
            let mut observed_table = None;
            let mut name = None;
            let mut key_type = None;
            let mut key_len = None;
            let mut id = None;
            let mut userdata = None;
            for (kind, raw_kind, value) in
                exact_attrs(&body[4..]).map_err(|source| NetlinkError::nft("list-sets", source))?
            {
                match kind {
                    NFTA_SET_TABLE if raw_kind & NLA_F_NESTED == 0 && observed_table.is_none() => {
                        observed_table = Some(
                            exact_cstr(value, "NFTA_SET_TABLE")
                                .map_err(|source| NetlinkError::nft("list-sets", source))?
                                .to_owned(),
                        );
                    }
                    NFTA_SET_NAME if raw_kind & NLA_F_NESTED == 0 && name.is_none() => {
                        name = Some(
                            exact_cstr(value, "NFTA_SET_NAME")
                                .map_err(|source| NetlinkError::nft("list-sets", source))?
                                .to_owned(),
                        );
                    }
                    NFTA_SET_KEY_TYPE if raw_kind & NLA_F_NESTED == 0 && key_type.is_none() => {
                        key_type = Some(
                            exact_be_u32(value, "NFTA_SET_KEY_TYPE")
                                .map_err(|source| NetlinkError::nft("list-sets", source))?,
                        );
                    }
                    NFTA_SET_KEY_LEN if raw_kind & NLA_F_NESTED == 0 && key_len.is_none() => {
                        key_len = Some(
                            exact_be_u32(value, "NFTA_SET_KEY_LEN")
                                .map_err(|source| NetlinkError::nft("list-sets", source))?,
                        );
                    }
                    NFTA_SET_ID if raw_kind & NLA_F_NESTED == 0 && id.is_none() => {
                        id = Some(
                            exact_be_u32(value, "NFTA_SET_ID")
                                .map_err(|source| NetlinkError::nft("list-sets", source))?,
                        );
                    }
                    NFTA_SET_USERDATA if raw_kind & NLA_F_NESTED == 0 && userdata.is_none() => {
                        userdata = Some(value.to_vec());
                    }
                    _ => {}
                }
            }
            let observed_table = observed_table.ok_or_else(|| {
                NetlinkError::nft("list-sets", invalid_data("set table is missing"))
            })?;
            if observed_table != table {
                return Ok(None);
            }
            Ok(Some(RawSetInfo {
                name: name.ok_or_else(|| {
                    NetlinkError::nft("list-sets", invalid_data("set name is missing"))
                })?,
                key_type: key_type.ok_or_else(|| {
                    NetlinkError::nft("list-sets", invalid_data("set key type is missing"))
                })?,
                key_len: key_len.ok_or_else(|| {
                    NetlinkError::nft("list-sets", invalid_data("set key length is missing"))
                })?,
                id: id.unwrap_or(1),
                userdata,
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entries.into_iter().flatten().collect())
}

fn list_other_children_family(
    family: NftFamily,
    table: &str,
) -> Result<Vec<RawOtherChildInfo>, NetlinkError> {
    let mut children = Vec::new();
    for (operation, kind, label, table_attr, name_attr, handle_attr) in [
        (
            NFT_MSG_GETFLOWTABLE,
            RawOtherChildKind::Flowtable,
            "list-flowtables",
            NFTA_FLOWTABLE_TABLE,
            NFTA_FLOWTABLE_NAME,
            NFTA_FLOWTABLE_HANDLE,
        ),
        (
            NFT_MSG_GETOBJ,
            RawOtherChildKind::StatefulObject,
            "list-objects",
            NFTA_OBJ_TABLE,
            NFTA_OBJ_NAME,
            NFTA_OBJ_HANDLE,
        ),
    ] {
        let bodies = receive_nft_dump_family(
            family,
            operation,
            &family_table_payload(family, table, table_attr),
            label,
        )?;
        for body in bodies {
            let mut observed_table = None;
            let mut name = None;
            let mut handle = None;
            for (attribute, raw_attribute, payload) in
                exact_attrs(&body[4..]).map_err(|source| NetlinkError::nft(label, source))?
            {
                match attribute {
                    attr_kind
                        if attr_kind == table_attr
                            && raw_attribute & NLA_F_NESTED == 0
                            && observed_table.is_none() =>
                    {
                        observed_table = Some(
                            exact_cstr(payload, "child table")
                                .map_err(|source| NetlinkError::nft(label, source))?
                                .to_owned(),
                        );
                    }
                    attr_kind
                        if attr_kind == name_attr
                            && raw_attribute & NLA_F_NESTED == 0
                            && name.is_none() =>
                    {
                        name = Some(
                            exact_cstr(payload, "child name")
                                .map_err(|source| NetlinkError::nft(label, source))?
                                .to_owned(),
                        );
                    }
                    attr_kind
                        if attr_kind == handle_attr
                            && raw_attribute & NLA_F_NESTED == 0
                            && handle.is_none() =>
                    {
                        handle = Some(
                            exact_be_u64(payload, "child handle")
                                .map_err(|source| NetlinkError::nft(label, source))?,
                        );
                    }
                    _ => {}
                }
            }
            let observed_table = observed_table
                .ok_or_else(|| NetlinkError::nft(label, invalid_data("child table is missing")))?;
            if observed_table == table {
                children.push(RawOtherChildInfo {
                    kind,
                    name: name.ok_or_else(|| {
                        NetlinkError::nft(label, invalid_data("child name is missing"))
                    })?,
                    handle,
                });
            }
        }
    }
    Ok(children)
}

fn read_nft_generation() -> Result<u32, NetlinkError> {
    let sock = NfSock::open().map_err(|source| NetlinkError::nft("observe-generation", source))?;
    request_generation(&sock, 1, Instant::now() + OBSERVATION_DEADLINE)
        .map_err(|source| NetlinkError::nft("observe-generation", source))
}

/// True iff `ip <table> <chain>` exists, via `GETCHAIN`.
///
/// The kernel `-ENOENT` maps to `Ok(false)` — the structural replacement for
/// the deleted `stderr_reports_absent_chain` text classifier (ADR-0085 D10).
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "chain-exists"`) on any non-`ENOENT` failure.
pub fn chain_exists(table: &str, chain: &str) -> Result<bool, NetlinkError> {
    let sock = NfSock::open().map_err(|e| NetlinkError::nft("chain-exists", e))?;
    let payload = get_by_table_chain(table, chain, NFTA_CHAIN_NAME, NFTA_CHAIN_TABLE);
    let mut msg = Vec::with_capacity((16 + payload.len()).next_multiple_of(4));
    nlmsg(&mut msg, nft_msg_type(NFT_MSG_GETCHAIN), NLM_F_REQUEST, 1, &payload);
    sock.send(&msg).map_err(|e| NetlinkError::nft("chain-exists", e))?;

    let mut buf = vec![0u8; 32768];
    let n = sock.recv(&mut buf).map_err(|e| NetlinkError::nft("chain-exists", e))?;
    let reply = &buf[..n];
    // A NEWCHAIN reply ⇒ present. An NLMSG_ERROR ⇒ code 0 (present) / -ENOENT
    // (absent) / else a genuine failure.
    let newchain = nft_msg_type(NFT_MSG_NEWCHAIN);
    let mut off = 0usize;
    while off + 16 <= reply.len() {
        let Some(mlen) = ne_u32(reply, off).map(|l| l as usize) else { break };
        let Some(mtype) = ne_u16(reply, off + 4) else { break };
        if mlen < 16 || off + mlen > reply.len() {
            break;
        }
        if mtype == newchain {
            return Ok(true);
        }
        if mtype == NLMSG_ERROR {
            let code = ne_u32(reply, off + 16).map_or(0, |c| c as i32);
            return match code {
                0 => Ok(true),
                c if c.abs() == libc::ENOENT => Ok(false),
                c => Err(NetlinkError::nft(
                    "chain-exists",
                    std::io::Error::from_raw_os_error(c.abs()),
                )),
            };
        }
        off += (mlen + 3) & !3;
    }
    // No decisive reply — treat as absent (nothing to operate on).
    Ok(false)
}

/// `nft delete rule ip <table> <chain> handle <handle>` — by-handle delete, the
/// structural teardown that removes ONLY the target rule (ADR-0085 D10).
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "delete-rule"`) on failure.
pub fn delete_rule(table: &str, chain: &str, handle: u64) -> Result<(), NetlinkError> {
    send_batched(NFT_MSG_DELRULE, 0, &delrule_payload(table, chain, handle), "delete-rule")
}

/// Canonical semantic identity of the node-shared IPv4 intercept program.
///
/// Kernel handles and dump-generation receipts remain private to this module.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedIpInterceptIdentity {
    table_and_chains: Vec<Vec<u8>>,
    sets: Vec<Vec<u8>>,
    prerouting: Vec<Vec<u8>>,
    output: Vec<Vec<u8>>,
}

/// Complete semantic state of the node-shared IPv4 intercept program.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedIpInterceptState {
    identity: SharedIpInterceptIdentity,
    managed_guest_ips: BTreeSet<Ipv4Addr>,
    outbound_sources: BTreeSet<Ipv4Addr>,
    inbound_destinations: BTreeSet<SocketAddrV4>,
}

#[allow(clippy::missing_const_for_fn)]
impl SharedIpInterceptState {
    #[doc(hidden)]
    pub fn identity(&self) -> &SharedIpInterceptIdentity {
        &self.identity
    }

    #[doc(hidden)]
    pub fn managed_guest_ips(&self) -> &BTreeSet<Ipv4Addr> {
        &self.managed_guest_ips
    }

    #[doc(hidden)]
    pub fn outbound_sources(&self) -> &BTreeSet<Ipv4Addr> {
        &self.outbound_sources
    }

    #[doc(hidden)]
    pub fn inbound_destinations(&self) -> &BTreeSet<SocketAddrV4> {
        &self.inbound_destinations
    }
}

impl SharedIpInterceptIdentity {
    /// Build the canonical identity for exact non-zero listener ports.
    #[doc(hidden)]
    #[allow(clippy::similar_names, clippy::type_complexity)]
    pub fn for_listener_ports(leg_f_port: u16, leg_c_port: u16) -> Result<Self, NetlinkError> {
        shared_ip::SharedProgram::expected(leg_f_port, leg_c_port).map(|program| program.identity())
    }

    /// Validate and rebuild the canonical identity from normalized parts.
    #[doc(hidden)]
    pub fn from_normalized_parts(
        table_and_chains: Vec<Vec<u8>>,
        sets: Vec<Vec<u8>>,
        prerouting: Vec<Vec<u8>>,
        output: Vec<Vec<u8>>,
    ) -> Result<Self, NetlinkError> {
        shared_ip::SharedProgram::from_components(table_and_chains, sets, prerouting, output)
            .map(|program| program.identity())
    }

    /// Project the canonical worker-owned normalized identity carrier.
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn normalized_parts(&self) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>) {
        (
            self.table_and_chains.clone(),
            self.sets.clone(),
            self.prerouting.clone(),
            self.output.clone(),
        )
    }
}

/// Observe one generation-consistent shared IPv4 intercept identity.
#[doc(hidden)]
pub fn observe_shared_ip_intercept() -> Result<Option<SharedIpInterceptIdentity>, NetlinkError> {
    shared_ip::observe()
}

/// Conditionally replace the complete shared IPv4 intercept object graph.
#[doc(hidden)]
pub fn replace_shared_ip_intercept_atomically(
    expected_current: Option<&SharedIpInterceptIdentity>,
    desired: Option<&SharedIpInterceptIdentity>,
) -> Result<(), NetlinkError> {
    shared_ip::replace_public(expected_current, desired)
}

/// Observe complete constant-program identity and all typed dynamic members.
#[doc(hidden)]
pub fn observe_shared_ip_intercept_state() -> Result<Option<SharedIpInterceptState>, NetlinkError> {
    shared_ip::observe_state()
}

#[doc(hidden)]
pub fn insert_shared_ip_intercept_outbound_elements_atomically(
    expected_program: &SharedIpInterceptIdentity,
    source_addr: Ipv4Addr,
) -> Result<SharedIpInterceptState, NetlinkError> {
    shared_ip::insert_outbound(expected_program, source_addr)
}

#[doc(hidden)]
pub fn insert_shared_ip_intercept_inbound_element_atomically(
    expected_program: &SharedIpInterceptIdentity,
    destination: SocketAddrV4,
) -> Result<SharedIpInterceptState, NetlinkError> {
    shared_ip::insert_inbound(expected_program, destination)
}

#[doc(hidden)]
pub fn delete_shared_ip_intercept_elements_atomically(
    expected_program: &SharedIpInterceptIdentity,
    source_addr: Option<Ipv4Addr>,
    inbound_destinations: &[SocketAddrV4],
) -> Result<SharedIpInterceptState, NetlinkError> {
    shared_ip::delete_elements(expected_program, source_addr, inbound_destinations)
}

#[doc(hidden)]
pub fn clear_shared_ip_intercept_elements_atomically(
    expected_program: &SharedIpInterceptIdentity,
) -> Result<SharedIpInterceptState, NetlinkError> {
    shared_ip::clear_elements(expected_program)
}

/// Private semantic IPv4 shared-intercept adapter used by the worker's
/// module-private `SharedInterceptProgramIo` seam.
mod shared_ip {
    #![allow(
        clippy::expect_used,
        clippy::similar_names,
        clippy::too_many_lines,
        clippy::type_complexity,
        clippy::wildcard_imports,
        reason = "the shared IP adapter keeps one bounded identity/transaction projection over the private nft codec"
    )]

    use super::*;

    // These are the nftables datatype identifiers, not the generic
    // NFT_DATA_* payload tags.  The concatenated key is aligned to four-byte
    // IPv4 plus four-byte inet_service storage by the kernel ABI.
    const IPV4_ADDR_KEY_TYPE: u32 = 7;
    const IPV4_ADDR_INET_SERVICE_KEY_TYPE: u32 = 0x1cd;
    const MANAGED_SET_KEY_LEN: u32 = 4;
    const DESTINATION_SET_KEY_LEN: u32 = 8;
    const PREROUTING_RULE_COUNT: usize = 5;
    const OUTPUT_RULE_COUNT: usize = 3;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum ElementSet {
        ManagedGuestIps,
        OutboundSources,
        InboundDestinations,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum ElementKey {
        ManagedGuest(Ipv4Addr),
        OutboundSource(Ipv4Addr),
        Destination(SocketAddrV4),
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct ElementMembers {
        managed_guest_ips: BTreeSet<Ipv4Addr>,
        outbound_sources: BTreeSet<Ipv4Addr>,
        inbound_destinations: BTreeSet<SocketAddrV4>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Chain {
        Prerouting,
        Output,
    }

    impl Chain {
        const fn name(self) -> &'static str {
            match self {
                Self::Prerouting => SHARED_IP_PREROUTING,
                Self::Output => SHARED_IP_OUTPUT,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Rule {
        chain: Chain,
        handle: Option<u64>,
        userdata: Vec<u8>,
        expressions: Vec<u8>,
    }

    /// Complete normalized identity of the worker-owned IP program.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct SharedProgram {
        identity: SharedIpInterceptIdentity,
        rules: Vec<Rule>,
        members: ElementMembers,
    }

    impl SharedProgram {
        /// Construct the exact owned identity for fresh listener targets.
        pub fn expected(leg_f_port: u16, leg_c_port: u16) -> Result<Self, NetlinkError> {
            if leg_f_port == 0 || leg_c_port == 0 {
                return Err(NetlinkError::nft(
                    "shared-ip-identity",
                    invalid_data("shared IP listener ports must be non-zero"),
                ));
            }
            let mut rules = Vec::with_capacity(PREROUTING_RULE_COUNT + OUTPUT_RULE_COUNT);
            for index in 0..PREROUTING_RULE_COUNT {
                rules.push(Rule {
                    chain: Chain::Prerouting,
                    handle: None,
                    userdata: shared_ip_userdata(SHARED_IP_PREROUTING, index),
                    expressions: normalized_rule_program_identity(
                        &shared_ip_prerouting_rule_exprs(leg_f_port, leg_c_port, index),
                    )
                    .map_err(|source| NetlinkError::nft("shared-ip-expected", source))?,
                });
            }
            for index in 0..OUTPUT_RULE_COUNT {
                rules.push(Rule {
                    chain: Chain::Output,
                    handle: None,
                    userdata: shared_ip_userdata(SHARED_IP_OUTPUT, index),
                    expressions: normalized_rule_program_identity(&shared_ip_output_rule_exprs(
                        leg_c_port, index,
                    ))
                    .map_err(|source| NetlinkError::nft("shared-ip-expected", source))?,
                });
            }
            let table_and_chains = expected_table_and_chains();
            let sets = expected_sets();
            let prerouting = rules
                .iter()
                .filter(|rule| rule.chain == Chain::Prerouting)
                .map(encode_rule)
                .collect();
            let output =
                rules.iter().filter(|rule| rule.chain == Chain::Output).map(encode_rule).collect();
            let program = Self::from_components(table_and_chains, sets, prerouting, output)?;
            Ok(Self { identity: program.identity, rules, members: ElementMembers::default() })
        }

        /// Rebuild an identity received from the worker postcondition carrier.
        pub fn from_components(
            table_and_chains: Vec<Vec<u8>>,
            sets: Vec<Vec<u8>>,
            prerouting: Vec<Vec<u8>>,
            output: Vec<Vec<u8>>,
        ) -> Result<Self, NetlinkError> {
            validate_components(&table_and_chains, &sets, &prerouting, &output)?;
            let mut rules = Vec::with_capacity(PREROUTING_RULE_COUNT + OUTPUT_RULE_COUNT);
            for encoded in &prerouting {
                let (userdata, expressions) = decode_rule(encoded)?;
                rules.push(Rule { chain: Chain::Prerouting, handle: None, userdata, expressions });
            }
            for encoded in &output {
                let (userdata, expressions) = decode_rule(encoded)?;
                rules.push(Rule { chain: Chain::Output, handle: None, userdata, expressions });
            }
            Ok(Self {
                identity: SharedIpInterceptIdentity { table_and_chains, sets, prerouting, output },
                rules,
                members: ElementMembers::default(),
            })
        }

        pub(super) fn identity(&self) -> SharedIpInterceptIdentity {
            self.identity.clone()
        }
    }

    fn encode_rule(rule: &Rule) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(8 + rule.userdata.len() + rule.expressions.len());
        encoded.extend_from_slice(&(rule.userdata.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&(rule.expressions.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&rule.userdata);
        encoded.extend_from_slice(&rule.expressions);
        encoded
    }

    fn decode_rule(encoded: &[u8]) -> Result<(Vec<u8>, Vec<u8>), NetlinkError> {
        let userdata_len = encoded
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .ok_or_else(|| {
                NetlinkError::nft("shared-ip-program", invalid_data("missing userdata length"))
            })? as usize;
        let expressions_len = encoded
            .get(4..8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .ok_or_else(|| {
                NetlinkError::nft("shared-ip-program", invalid_data("missing expression length"))
            })? as usize;
        let userdata_end = 8usize.checked_add(userdata_len).ok_or_else(|| {
            NetlinkError::nft("shared-ip-program", invalid_data("userdata length overflow"))
        })?;
        let expressions_end = userdata_end.checked_add(expressions_len).ok_or_else(|| {
            NetlinkError::nft("shared-ip-program", invalid_data("expression length overflow"))
        })?;
        if expressions_end != encoded.len() {
            return Err(NetlinkError::nft(
                "shared-ip-program",
                invalid_data("shared IP rule component has trailing or missing bytes"),
            ));
        }
        Ok((encoded[8..userdata_end].to_vec(), encoded[userdata_end..].to_vec()))
    }

    fn expected_table_and_chains() -> Vec<Vec<u8>> {
        vec![
            b"table:overdrive-mtls".to_vec(),
            b"chain:prerouting:filter:0:-150:accept".to_vec(),
            b"chain:output:route:3:-150:accept".to_vec(),
        ]
    }

    fn encode_set(name: &str, key_type: u32, key_len: u32) -> Vec<u8> {
        let userdata = shared_ip_set_userdata(name);
        let mut encoded = Vec::with_capacity(16 + name.len() + userdata.len());
        encoded.extend_from_slice(&(name.len() as u32).to_be_bytes());
        encoded.extend_from_slice(name.as_bytes());
        encoded.extend_from_slice(&key_type.to_be_bytes());
        encoded.extend_from_slice(&key_len.to_be_bytes());
        encoded.extend_from_slice(&(userdata.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&userdata);
        encoded
    }

    fn expected_sets() -> Vec<Vec<u8>> {
        vec![
            encode_set(SHARED_IP_MANAGED_GUESTS, IPV4_ADDR_KEY_TYPE, MANAGED_SET_KEY_LEN),
            encode_set(SHARED_IP_OUTBOUND_SOURCES, IPV4_ADDR_KEY_TYPE, MANAGED_SET_KEY_LEN),
            encode_set(
                SHARED_IP_INBOUND_DESTINATIONS,
                IPV4_ADDR_INET_SERVICE_KEY_TYPE,
                DESTINATION_SET_KEY_LEN,
            ),
        ]
    }

    fn invalid_shared_ip(message: impl Into<String>) -> NetlinkError {
        NetlinkError::nft("shared-ip-observe", invalid_data(message))
    }

    fn canonicalize_dynamic_port(data: &[u8]) -> std::io::Result<(Vec<u8>, Option<u16>)> {
        let attrs = exact_attrs(data)?;
        let mut dreg = None;
        let mut immediate_data = None;
        for (kind, raw, payload) in &attrs {
            match *kind {
                NFTA_IMMEDIATE_DREG if *raw & NLA_F_NESTED == 0 && dreg.is_none() => {
                    dreg = Some(exact_be_u32(payload, "immediate destination register")?);
                }
                NFTA_IMMEDIATE_DATA if immediate_data.is_none() => {
                    immediate_data = Some((*raw, *payload));
                }
                NFTA_IMMEDIATE_DREG | NFTA_IMMEDIATE_DATA => {
                    return Err(invalid_data("duplicate immediate attribute"));
                }
                _ => return Err(invalid_data("unknown immediate attribute")),
            }
        }
        if dreg != Some(NFT_REG_2) {
            return Ok((data.to_vec(), None));
        }
        let (data_raw, data_payload) =
            immediate_data.ok_or_else(|| invalid_data("immediate data is missing"))?;
        let data_attrs = exact_attrs(data_payload)?;
        let mut value = None;
        for (kind, raw, payload) in &data_attrs {
            if *kind == NFTA_DATA_VALUE && *raw & NLA_F_NESTED == 0 && value.is_none() {
                value = Some(*payload);
            } else if *kind == NFTA_DATA_VALUE {
                return Err(invalid_data("duplicate immediate value"));
            }
        }
        let value = value.ok_or_else(|| invalid_data("immediate value is missing"))?;
        if value.len() != 2 {
            return Ok((data.to_vec(), None));
        }
        let port = u16::from_be_bytes([value[0], value[1]]);
        let mut rewritten_value = Vec::with_capacity(data_payload.len());
        for (kind, raw, payload) in data_attrs {
            if kind == NFTA_DATA_VALUE {
                attr(&mut rewritten_value, raw, &[0, 0]);
            } else {
                attr(&mut rewritten_value, raw, payload);
            }
        }
        let mut rewritten = Vec::with_capacity(data.len());
        for (kind, raw, payload) in attrs {
            if kind == NFTA_IMMEDIATE_DATA {
                attr(&mut rewritten, data_raw, &rewritten_value);
            } else {
                attr(&mut rewritten, raw, payload);
            }
        }
        Ok((rewritten, Some(port)))
    }

    fn canonicalize_rule(
        chain: Chain,
        index: usize,
        expressions: &[u8],
    ) -> std::io::Result<(Vec<u8>, Option<u16>)> {
        let normalized = normalize_rule_program(expressions, false)?.0;
        let mut canonical = Vec::with_capacity(normalized.len());
        let mut dynamic_port = None;
        for (kind, _raw, element) in exact_attrs(&normalized)? {
            if kind != NFTA_LIST_ELEM {
                return Err(invalid_data("shared IP expression list is not canonical"));
            }
            let mut name = None;
            let mut data = None;
            for (attribute, raw, payload) in exact_attrs(element)? {
                match attribute {
                    NFTA_EXPR_NAME if raw & NLA_F_NESTED == 0 && name.is_none() => {
                        name = Some(exact_cstr(payload, "shared IP expression name")?);
                    }
                    NFTA_EXPR_DATA if data.is_none() => data = Some(payload),
                    NFTA_EXPR_NAME | NFTA_EXPR_DATA => {
                        return Err(invalid_data("shared IP expression has duplicate framing"));
                    }
                    _ => return Err(invalid_data("shared IP expression has unknown framing")),
                }
            }
            let name = name.ok_or_else(|| invalid_data("shared IP expression name is missing"))?;
            let data = data.ok_or_else(|| invalid_data("shared IP expression data is missing"))?;
            let mut data = canonical_expression_data(name, data, false)?;
            if name == "immediate" && matches!((chain, index), (Chain::Prerouting, 1 | 3)) {
                let (rewritten, port) = canonicalize_dynamic_port(&data)?;
                data = rewritten;
                if let Some(port) = port
                    && dynamic_port.replace(port).is_some()
                {
                    return Err(invalid_data("shared IP rule has duplicate listener target"));
                }
            }
            canonical.extend(expr(name, &data));
        }
        Ok((canonical, dynamic_port))
    }

    fn validate_components(
        table_and_chains: &[Vec<u8>],
        sets: &[Vec<u8>],
        prerouting: &[Vec<u8>],
        output: &[Vec<u8>],
    ) -> Result<(), NetlinkError> {
        if table_and_chains != expected_table_and_chains()
            || sets != expected_sets()
            || prerouting.len() != PREROUTING_RULE_COUNT
            || output.len() != OUTPUT_RULE_COUNT
        {
            return Err(invalid_shared_ip("shared IP identity has an incompatible shape"));
        }
        let mut ports = [None, None];
        for (chain, components, count) in [
            (Chain::Prerouting, prerouting, PREROUTING_RULE_COUNT),
            (Chain::Output, output, OUTPUT_RULE_COUNT),
        ] {
            for (index, encoded) in components.iter().enumerate() {
                let (userdata, expressions) = decode_rule(encoded)?;
                if userdata != shared_ip_userdata(chain.name(), index) {
                    return Err(invalid_shared_ip("shared IP rule userdata is not exact"));
                }
                let (canonical, dynamic_port) = canonicalize_rule(chain, index, &expressions)
                    .map_err(|source| NetlinkError::nft("shared-ip-observe", source))?;
                let expected = if chain == Chain::Prerouting {
                    shared_ip_prerouting_rule_exprs(
                        dynamic_port.filter(|_| index == 1).unwrap_or_default(),
                        dynamic_port.filter(|_| index == 3).unwrap_or_default(),
                        index,
                    )
                } else {
                    shared_ip_output_rule_exprs(0, index)
                };
                let expected = canonicalize_rule(chain, index, &expected)
                    .map_err(|source| NetlinkError::nft("shared-ip-observe", source))?
                    .0;
                if canonical != expected {
                    return Err(invalid_shared_ip("shared IP rule expression is not canonical"));
                }
                if matches!((chain, index), (Chain::Prerouting, 1 | 3)) {
                    let port = dynamic_port
                        .ok_or_else(|| invalid_shared_ip("shared IP listener target is missing"))?;
                    if port == 0 {
                        return Err(invalid_shared_ip("shared IP listener target is zero"));
                    }
                    ports[usize::from(index == 3)] = Some(port);
                } else if dynamic_port.is_some() {
                    return Err(invalid_shared_ip("unexpected shared IP listener target"));
                }
                if index >= count {
                    return Err(invalid_shared_ip("shared IP rule index is out of range"));
                }
            }
        }
        if ports[0].is_none() || ports[1].is_none() {
            return Err(invalid_shared_ip("shared IP listener targets are incomplete"));
        }
        Ok(())
    }

    fn observed_chain_identity(chain: &RawChainInfo) -> Vec<u8> {
        let kind = chain.chain_type.as_deref().unwrap_or("regular");
        let (hook, priority) = chain.hook.map_or(("none", 0), |(hook, priority)| {
            let hook = match hook {
                0 => "0",
                1 => "1",
                2 => "2",
                3 => "3",
                4 => "4",
                _ => "other",
            };
            (hook, priority)
        });
        let policy = match chain.policy {
            Some(NF_ACCEPT) => "accept",
            Some(0) => "drop",
            Some(_) => "other",
            None => "none",
        };
        format!("chain:{}:{kind}:{hook}:{priority}:{policy}", chain.name).into_bytes()
    }

    fn observed_set_identity(set: &RawSetInfo) -> Vec<u8> {
        let userdata = set.userdata.clone().unwrap_or_default();
        let mut encoded = Vec::with_capacity(16 + set.name.len() + userdata.len());
        encoded.extend_from_slice(&(set.name.len() as u32).to_be_bytes());
        encoded.extend_from_slice(set.name.as_bytes());
        encoded.extend_from_slice(&set.key_type.to_be_bytes());
        encoded.extend_from_slice(&set.key_len.to_be_bytes());
        encoded.extend_from_slice(&(userdata.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&userdata);
        encoded
    }

    const fn expected_chain(chain: Chain) -> (u32, i32, &'static str) {
        match chain {
            Chain::Prerouting => (NF_INET_PRE_ROUTING, PRIORITY_MANGLE, "filter"),
            Chain::Output => (NF_INET_LOCAL_OUT, PRIORITY_MANGLE, "route"),
        }
    }

    fn collect_once() -> Result<Option<SharedProgram>, NetlinkError> {
        let tables = list_table_names_family(NftFamily::Ipv4)?;
        // The approved #295 topology intentionally uses the same table name
        // in the independent IPv4 and bridge nft families: `ip overdrive-mtls`
        // owns the constant TPROXY rules while `bridge overdrive-mtls` owns
        // the managed-TAP proof-mark guard. Family identity is part of the
        // kernel object graph, so a bridge-family table with this name is not
        // a conflict for the IPv4 observer.
        if !tables.iter().any(|table| table == SHARED_IP_TABLE) {
            return Ok(None);
        }
        let bridge_tables = list_table_names_family(NftFamily::Bridge)?;
        if bridge_tables.iter().any(|name| name == SHARED_IP_TABLE) {
            let bridge_chains = list_chain_info_family(NftFamily::Bridge, SHARED_IP_TABLE)?;
            let accepted_companion = bridge_chains.len() == 1
                && bridge_chains.first().is_some_and(|chain| {
                    chain.name == "prerouting"
                        && chain.hook == Some((0, -300))
                        && chain.chain_type.as_deref() == Some("filter")
                        && chain.policy == Some(NF_ACCEPT)
                });
            if !accepted_companion {
                return Err(invalid_shared_ip(
                    "shared IP companion bridge table has an incomplete or foreign identity",
                ));
            }
        }

        let chains = list_chain_info_family(NftFamily::Ipv4, SHARED_IP_TABLE)?;
        let names = [Chain::Prerouting.name(), Chain::Output.name()];
        if chains.len() != names.len()
            || names
                .iter()
                .any(|name| chains.iter().filter(|chain| chain.name == *name).count() != 1)
        {
            return Err(NetlinkError::nft(
                "shared-ip-observe",
                invalid_data("shared IP table has partial, duplicate, or foreign chains"),
            ));
        }
        for kind in [Chain::Prerouting, Chain::Output] {
            let chain =
                chains.iter().find(|chain| chain.name == kind.name()).expect("validated chain");
            let (hook, priority, chain_type) = expected_chain(kind);
            if chain.hook != Some((hook, priority))
                || chain.chain_type.as_deref() != Some(chain_type)
                || chain.policy != Some(NF_ACCEPT)
            {
                return Err(NetlinkError::nft(
                    "shared-ip-observe",
                    invalid_data("shared IP chain schema conflicts with owned identity"),
                ));
            }
        }

        let sets = list_set_info_family(NftFamily::Ipv4, SHARED_IP_TABLE)?;
        let expected = [
            (SHARED_IP_MANAGED_GUESTS, IPV4_ADDR_KEY_TYPE, MANAGED_SET_KEY_LEN),
            (SHARED_IP_OUTBOUND_SOURCES, IPV4_ADDR_KEY_TYPE, MANAGED_SET_KEY_LEN),
            (
                SHARED_IP_INBOUND_DESTINATIONS,
                IPV4_ADDR_INET_SERVICE_KEY_TYPE,
                DESTINATION_SET_KEY_LEN,
            ),
        ];
        if sets.len() != expected.len()
            || expected.iter().any(|(name, key_type, key_len)| {
                sets.iter().filter(|set| set.name == *name).count() != 1
                    || sets.iter().any(|set| {
                        set.name == *name
                            && (set.key_type != *key_type
                                || set.key_len != *key_len
                                || set.id == 0
                                || set.userdata.as_deref()
                                    != Some(shared_ip_set_userdata(name).as_slice()))
                    })
            })
        {
            return Err(NetlinkError::nft(
                "shared-ip-observe",
                invalid_data(
                    "shared IP set inventory is partial, foreign, duplicate, or malformed",
                ),
            ));
        }
        let mut members = ElementMembers::default();
        for (name, _, _) in expected {
            let set = sets.iter().find(|set| set.name == name).expect("validated set");
            for member in list_set_elements_family(NftFamily::Ipv4, SHARED_IP_TABLE, name, set.id)?
            {
                let inserted = match name {
                    SHARED_IP_MANAGED_GUESTS => {
                        if member.len() != MANAGED_SET_KEY_LEN as usize {
                            return Err(invalid_shared_ip(
                                "shared IP address element length is invalid",
                            ));
                        }
                        members
                            .managed_guest_ips
                            .insert(Ipv4Addr::new(member[0], member[1], member[2], member[3]))
                    }
                    SHARED_IP_OUTBOUND_SOURCES => {
                        if member.len() != MANAGED_SET_KEY_LEN as usize {
                            return Err(invalid_shared_ip(
                                "shared IP address element length is invalid",
                            ));
                        }
                        members
                            .outbound_sources
                            .insert(Ipv4Addr::new(member[0], member[1], member[2], member[3]))
                    }
                    SHARED_IP_INBOUND_DESTINATIONS => {
                        if member.len() != DESTINATION_SET_KEY_LEN as usize
                            || member[6] != 0
                            || member[7] != 0
                        {
                            return Err(invalid_shared_ip(
                                "shared IP destination element alignment is invalid",
                            ));
                        }
                        let port = u16::from_be_bytes([member[4], member[5]]);
                        if port == 0 {
                            return Err(invalid_shared_ip(
                                "shared IP destination element port is zero",
                            ));
                        }
                        members.inbound_destinations.insert(SocketAddrV4::new(
                            Ipv4Addr::new(member[0], member[1], member[2], member[3]),
                            port,
                        ))
                    }
                    _ => unreachable!("validated shared IP set name"),
                };
                if !inserted {
                    return Err(invalid_shared_ip("shared IP dynamic set has duplicate members"));
                }
            }
        }
        if !list_other_children_family(NftFamily::Ipv4, SHARED_IP_TABLE)?.is_empty() {
            return Err(invalid_shared_ip("shared IP table contains foreign stateful children"));
        }

        let mut rules = Vec::with_capacity(PREROUTING_RULE_COUNT + OUTPUT_RULE_COUNT);
        for kind in [Chain::Prerouting, Chain::Output] {
            let count = match kind {
                Chain::Prerouting => PREROUTING_RULE_COUNT,
                Chain::Output => OUTPUT_RULE_COUNT,
            };
            let observed = list_rules_family(NftFamily::Ipv4, SHARED_IP_TABLE, kind.name())?;
            if observed.len() != count {
                return Err(NetlinkError::nft(
                    "shared-ip-observe",
                    invalid_data("shared IP rule inventory is partial or foreign"),
                ));
            }
            let mut seen = BTreeSet::new();
            for (position, rule) in observed.into_iter().enumerate() {
                let expected_userdata = shared_ip_userdata(kind.name(), position);
                if rule.userdata != expected_userdata || !seen.insert(position) {
                    return Err(invalid_shared_ip(
                        "shared IP rule inventory has duplicate ownership or wrong order",
                    ));
                }
                rules.push(Rule {
                    chain: kind,
                    handle: Some(rule.handle),
                    userdata: rule.userdata,
                    expressions: rule.normalized_program,
                });
            }
            if seen.len() != count {
                return Err(NetlinkError::nft(
                    "shared-ip-observe",
                    invalid_data("shared IP rule inventory is incomplete"),
                ));
            }
        }
        let table_and_chains = std::iter::once(b"table:overdrive-mtls".to_vec())
            .chain([Chain::Prerouting, Chain::Output].into_iter().map(|kind| {
                observed_chain_identity(
                    chains.iter().find(|chain| chain.name == kind.name()).expect("validated chain"),
                )
            }))
            .collect();
        let sets = expected
            .into_iter()
            .map(|(name, _, _)| {
                observed_set_identity(
                    sets.iter().find(|set| set.name == name).expect("validated set"),
                )
            })
            .collect();
        let prerouting =
            rules.iter().filter(|rule| rule.chain == Chain::Prerouting).map(encode_rule).collect();
        let output =
            rules.iter().filter(|rule| rule.chain == Chain::Output).map(encode_rule).collect();
        let mut semantic =
            SharedProgram::from_components(table_and_chains, sets, prerouting, output)?;
        semantic.members = members;
        if semantic.rules.iter().zip(rules.iter()).any(|(expected, observed)| {
            expected.chain != observed.chain
                || expected.userdata != observed.userdata
                || expected.expressions != observed.expressions
        }) {
            return Err(invalid_shared_ip("shared IP rule semantic identity mismatch"));
        }
        Ok(Some(SharedProgram { identity: semantic.identity, rules, members: semantic.members }))
    }

    fn collect() -> Result<Option<SharedProgram>, NetlinkError> {
        let before = read_nft_generation()?;
        let observed = collect_once()?;
        let after = read_nft_generation()?;
        if before != after {
            return Err(NetlinkError::nft(
                "shared-ip-observe-generation",
                invalid_data("shared IP observation crossed a ruleset generation"),
            ));
        }
        if observed.as_ref().is_some_and(|program| {
            !program.members.managed_guest_ips.is_empty()
                || !program.members.outbound_sources.is_empty()
                || !program.members.inbound_destinations.is_empty()
        }) {
            return Err(invalid_shared_ip("shared IP dynamic set state is non-empty"));
        }
        Ok(observed)
    }

    fn collect_state() -> Result<Option<SharedIpInterceptState>, NetlinkError> {
        let before = read_nft_generation()?;
        let observed = collect_once()?;
        let after = read_nft_generation()?;
        if before != after {
            return Err(NetlinkError::nft(
                "shared-ip-observe-generation",
                invalid_data("shared IP state observation crossed a ruleset generation"),
            ));
        }
        Ok(observed.map(|program| SharedIpInterceptState {
            identity: program.identity,
            managed_guest_ips: program.members.managed_guest_ips,
            outbound_sources: program.members.outbound_sources,
            inbound_destinations: program.members.inbound_destinations,
        }))
    }

    #[derive(Clone, Copy)]
    enum SharedMutation<'a> {
        NewTable,
        NewChain { name: &'a str, spec: BaseChainSpec },
        NewSet { name: &'a str, key_type: u32, key_len: u32 },
        NewRule { chain: &'a str, expressions: &'a [u8], userdata: &'a [u8], append: bool },
        ReplaceRule { chain: &'a str, handle: u64, expressions: &'a [u8], userdata: &'a [u8] },
        DeleteRule { chain: &'a str, handle: u64 },
        DeleteSet { name: &'a str },
        DeleteChain { name: &'a str },
        DeleteTable,
    }

    fn send_transaction(mutations: &[SharedMutation<'_>]) -> Result<(), NetlinkError> {
        if mutations.is_empty() {
            return Ok(());
        }
        let sock = NfSock::open()
            .map_err(|source| NetlinkError::nft("shared-ip-atomic-transaction", source))?;
        let mut batch = Vec::new();
        nlmsg(
            &mut batch,
            NFNL_MSG_BATCH_BEGIN,
            NLM_F_REQUEST,
            1,
            &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
        );
        for (index, mutation) in mutations.iter().enumerate() {
            let sequence = index as u32 + 2;
            let (operation, flags, payload) = match mutation {
                SharedMutation::NewTable => (
                    NFT_MSG_NEWTABLE,
                    NLM_F_CREATE,
                    newtable_payload_family(NftFamily::Ipv4, SHARED_IP_TABLE),
                ),
                SharedMutation::NewChain { name, spec } => (
                    NFT_MSG_NEWCHAIN,
                    NLM_F_CREATE,
                    newchain_payload_family(NftFamily::Ipv4, SHARED_IP_TABLE, name, *spec),
                ),
                SharedMutation::NewSet { name, key_type, key_len } => (
                    NFT_MSG_NEWSET,
                    NLM_F_CREATE,
                    newset_payload_schema_family(
                        NftFamily::Ipv4,
                        SHARED_IP_TABLE,
                        name,
                        *key_type,
                        *key_len,
                        1,
                        &shared_ip_set_userdata(name),
                    ),
                ),
                SharedMutation::NewRule { chain, expressions, userdata, append } => (
                    NFT_MSG_NEWRULE,
                    NLM_F_CREATE | if *append { NLM_F_APPEND } else { 0 },
                    newrule_payload_family(
                        NftFamily::Ipv4,
                        SHARED_IP_TABLE,
                        chain,
                        expressions,
                        userdata,
                    ),
                ),
                SharedMutation::ReplaceRule { chain, handle, expressions, userdata } => (
                    NFT_MSG_NEWRULE,
                    NLM_F_REPLACE,
                    replace_rule_payload_family(
                        NftFamily::Ipv4,
                        SHARED_IP_TABLE,
                        chain,
                        *handle,
                        expressions,
                        userdata,
                    ),
                ),
                SharedMutation::DeleteRule { chain, handle } => (
                    NFT_MSG_DELRULE,
                    0,
                    delrule_payload_family(NftFamily::Ipv4, SHARED_IP_TABLE, chain, *handle),
                ),
                SharedMutation::DeleteSet { name } => (
                    NFT_MSG_DELSET,
                    0,
                    delete_set_payload_family(NftFamily::Ipv4, SHARED_IP_TABLE, name),
                ),
                SharedMutation::DeleteChain { name } => (
                    NFT_MSG_DELCHAIN,
                    0,
                    get_by_table_chain_family(
                        NftFamily::Ipv4,
                        SHARED_IP_TABLE,
                        name,
                        NFTA_CHAIN_NAME,
                        NFTA_CHAIN_TABLE,
                    ),
                ),
                SharedMutation::DeleteTable => {
                    (NFT_MSG_DELTABLE, 0, newtable_payload_family(NftFamily::Ipv4, SHARED_IP_TABLE))
                }
            };
            nlmsg(
                &mut batch,
                nft_msg_type(operation),
                NLM_F_REQUEST | NLM_F_ACK | flags,
                sequence,
                &payload,
            );
        }
        let end_sequence = mutations.len() as u32 + 2;
        nlmsg(
            &mut batch,
            NFNL_MSG_BATCH_END,
            NLM_F_REQUEST,
            end_sequence,
            &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
        );
        sock.send(&batch)
            .map_err(|source| NetlinkError::nft("shared-ip-atomic-transaction", source))?;
        let mut pending = (2..end_sequence).collect::<BTreeSet<_>>();
        while !pending.is_empty() {
            let mut buffer = vec![0_u8; 32_768];
            let received = sock
                .recv(&mut buffer)
                .map_err(|source| NetlinkError::nft("shared-ip-atomic-transaction", source))?;
            collect_atomic_rule_acks(&buffer[..received], &mut pending)
                .map_err(|source| NetlinkError::nft("shared-ip-atomic-transaction", source))?;
        }
        Ok(())
    }

    fn create(program: &SharedProgram) -> Result<(), NetlinkError> {
        let mut mutations = vec![SharedMutation::NewTable];
        mutations.extend([
            SharedMutation::NewChain {
                name: SHARED_IP_PREROUTING,
                spec: BaseChainSpec {
                    hooknum: NF_INET_PRE_ROUTING,
                    priority: PRIORITY_MANGLE,
                    kind: ChainKind::Filter,
                },
            },
            SharedMutation::NewChain {
                name: SHARED_IP_OUTPUT,
                spec: BaseChainSpec {
                    hooknum: NF_INET_LOCAL_OUT,
                    priority: PRIORITY_MANGLE,
                    kind: ChainKind::Route,
                },
            },
        ]);
        mutations.extend([
            SharedMutation::NewSet {
                name: SHARED_IP_MANAGED_GUESTS,
                key_type: IPV4_ADDR_KEY_TYPE,
                key_len: MANAGED_SET_KEY_LEN,
            },
            SharedMutation::NewSet {
                name: SHARED_IP_OUTBOUND_SOURCES,
                key_type: IPV4_ADDR_KEY_TYPE,
                key_len: MANAGED_SET_KEY_LEN,
            },
            SharedMutation::NewSet {
                name: SHARED_IP_INBOUND_DESTINATIONS,
                key_type: IPV4_ADDR_INET_SERVICE_KEY_TYPE,
                key_len: DESTINATION_SET_KEY_LEN,
            },
        ]);
        for (index, rule) in program.rules.iter().enumerate() {
            mutations.push(SharedMutation::NewRule {
                chain: rule.chain.name(),
                expressions: &rule.expressions,
                userdata: &rule.userdata,
                append: index != 0 && index != PREROUTING_RULE_COUNT,
            });
        }
        send_transaction(&mutations)
    }

    fn delete(program: &SharedProgram) -> Result<(), NetlinkError> {
        let mut mutations = Vec::with_capacity(program.rules.len() + 7);
        for rule in &program.rules {
            mutations.push(SharedMutation::DeleteRule {
                chain: rule.chain.name(),
                handle: rule.handle.ok_or_else(|| {
                    NetlinkError::nft(
                        "shared-ip-delete",
                        invalid_data("shared IP delete requires private kernel handles"),
                    )
                })?,
            });
        }
        mutations.extend([
            SharedMutation::DeleteSet { name: SHARED_IP_INBOUND_DESTINATIONS },
            SharedMutation::DeleteSet { name: SHARED_IP_OUTBOUND_SOURCES },
            SharedMutation::DeleteSet { name: SHARED_IP_MANAGED_GUESTS },
            SharedMutation::DeleteChain { name: SHARED_IP_OUTPUT },
            SharedMutation::DeleteChain { name: SHARED_IP_PREROUTING },
            SharedMutation::DeleteTable,
        ]);
        send_transaction(&mutations)
    }

    fn replace(
        expected_current: Option<&SharedIpInterceptIdentity>,
        desired: Option<&SharedIpInterceptIdentity>,
    ) -> Result<(), NetlinkError> {
        let current = collect()?;
        if current.as_ref().map(SharedProgram::identity).as_ref() != expected_current {
            return Err(NetlinkError::nft(
                "shared-ip-replace",
                invalid_data("shared IP current semantic identity changed before replacement"),
            ));
        }
        match (current, desired) {
            (None, Some(identity)) => create(&SharedProgram::from_components(
                identity.table_and_chains.clone(),
                identity.sets.clone(),
                identity.prerouting.clone(),
                identity.output.clone(),
            )?),
            (None, None) => Ok(()),
            (Some(current), None) => delete(&current),
            (Some(current), Some(identity)) => {
                let desired = SharedProgram::from_components(
                    identity.table_and_chains.clone(),
                    identity.sets.clone(),
                    identity.prerouting.clone(),
                    identity.output.clone(),
                )?;
                let mutations = current
                    .rules
                    .iter()
                    .zip(desired.rules.iter())
                    .map(|(current, desired)| {
                        Ok(SharedMutation::ReplaceRule {
                            chain: current.chain.name(),
                            handle: current.handle.ok_or_else(|| {
                                NetlinkError::nft(
                                    "shared-ip-replace",
                                    invalid_data(
                                        "shared IP replace requires private kernel handles",
                                    ),
                                )
                            })?,
                            expressions: &desired.expressions,
                            userdata: &desired.userdata,
                        })
                    })
                    .collect::<Result<Vec<_>, NetlinkError>>()?;
                send_transaction(&mutations)
            }
        }
    }

    pub(super) fn observe() -> Result<Option<SharedIpInterceptIdentity>, NetlinkError> {
        collect().map(|program| program.map(|program| program.identity))
    }

    pub(super) fn observe_state() -> Result<Option<SharedIpInterceptState>, NetlinkError> {
        collect_state()
    }

    pub(super) fn replace_public(
        expected_current: Option<&SharedIpInterceptIdentity>,
        desired: Option<&SharedIpInterceptIdentity>,
    ) -> Result<(), NetlinkError> {
        replace(expected_current, desired)
    }

    #[derive(Debug, Clone)]
    struct ElementMutation {
        set: ElementSet,
        set_id: u32,
        key: Vec<u8>,
        add: bool,
    }

    #[derive(Debug)]
    struct ElementRestoreError {
        primary: NetlinkError,
        restore: NetlinkError,
    }

    impl std::fmt::Display for ElementRestoreError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let _ = &self.restore;
            write!(formatter, "shared IP element read-back/restoration failed")
        }
    }

    impl std::error::Error for ElementRestoreError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.primary)
        }
    }

    fn element_restore_error(primary: NetlinkError, restore: NetlinkError) -> NetlinkError {
        NetlinkError::nft(
            "shared-ip-element-restore",
            std::io::Error::other(ElementRestoreError { primary, restore }),
        )
    }

    const fn element_set_name(set: ElementSet) -> &'static str {
        match set {
            ElementSet::ManagedGuestIps => SHARED_IP_MANAGED_GUESTS,
            ElementSet::OutboundSources => SHARED_IP_OUTBOUND_SOURCES,
            ElementSet::InboundDestinations => SHARED_IP_INBOUND_DESTINATIONS,
        }
    }

    fn element_key_bytes(key: ElementKey) -> Result<(ElementSet, Vec<u8>), NetlinkError> {
        match key {
            ElementKey::ManagedGuest(address) => {
                Ok((ElementSet::ManagedGuestIps, address.octets().to_vec()))
            }
            ElementKey::OutboundSource(address) => {
                Ok((ElementSet::OutboundSources, address.octets().to_vec()))
            }
            ElementKey::Destination(destination) => {
                if destination.port() == 0 {
                    return Err(invalid_shared_ip("shared IP destination port is zero"));
                }
                let mut bytes = Vec::with_capacity(8);
                bytes.extend_from_slice(&destination.ip().octets());
                bytes.extend_from_slice(&destination.port().to_be_bytes());
                bytes.extend_from_slice(&[0, 0]);
                Ok((ElementSet::InboundDestinations, bytes))
            }
        }
    }

    fn set_ids() -> Result<BTreeMap<ElementSet, u32>, NetlinkError> {
        let observed = list_set_info_family(NftFamily::Ipv4, SHARED_IP_TABLE)?;
        let expected = [
            (
                ElementSet::ManagedGuestIps,
                SHARED_IP_MANAGED_GUESTS,
                IPV4_ADDR_KEY_TYPE,
                MANAGED_SET_KEY_LEN,
            ),
            (
                ElementSet::OutboundSources,
                SHARED_IP_OUTBOUND_SOURCES,
                IPV4_ADDR_KEY_TYPE,
                MANAGED_SET_KEY_LEN,
            ),
            (
                ElementSet::InboundDestinations,
                SHARED_IP_INBOUND_DESTINATIONS,
                IPV4_ADDR_INET_SERVICE_KEY_TYPE,
                DESTINATION_SET_KEY_LEN,
            ),
        ];
        if observed.len() != expected.len() {
            return Err(NetlinkError::nft(
                "shared-ip-element-schema",
                invalid_data("shared IP element set inventory is incomplete or foreign"),
            ));
        }
        let mut ids = BTreeMap::new();
        for (semantic, name, key_type, key_len) in expected {
            let matches = observed.iter().filter(|set| set.name == name).collect::<Vec<_>>();
            let Some(set) = matches.first().copied() else {
                return Err(NetlinkError::nft(
                    "shared-ip-element-schema",
                    invalid_data("shared IP element set is absent"),
                ));
            };
            if matches.len() != 1
                || set.key_type != key_type
                || set.key_len != key_len
                || set.id == 0
                || set.userdata.as_deref() != Some(shared_ip_set_userdata(name).as_slice())
            {
                return Err(NetlinkError::nft(
                    "shared-ip-element-schema",
                    invalid_data("shared IP element set schema conflicts with owned identity"),
                ));
            }
            ids.insert(semantic, set.id);
        }
        Ok(ids)
    }

    fn element_payload(set: &str, set_id: u32, key: &[u8]) -> Vec<u8> {
        let mut element = Vec::new();
        let mut key_value = Vec::new();
        attr(&mut key_value, NFTA_DATA_VALUE, key);
        let mut key_element = Vec::new();
        attr(&mut key_element, NFTA_SET_ELEM_KEY | NLA_F_NESTED, &key_value);
        attr(&mut element, NLA_F_NESTED | 1, &key_element);
        let mut payload = nfgenmsg(NftFamily::Ipv4.nfproto(), 0);
        attr(&mut payload, NFTA_SET_ELEM_LIST_TABLE, &cstr(SHARED_IP_TABLE));
        attr(&mut payload, NFTA_SET_ELEM_LIST_SET, &cstr(set));
        attr_be32(&mut payload, NFTA_SET_ELEM_LIST_SET_ID, set_id);
        attr(&mut payload, NFTA_SET_ELEM_LIST_ELEMENTS_NESTED, &element);
        payload
    }

    fn send_element_transaction(mutations: &[ElementMutation]) -> Result<(), NetlinkError> {
        if mutations.is_empty() {
            return Ok(());
        }
        let sock = NfSock::open()
            .map_err(|source| NetlinkError::nft("shared-ip-element-transaction", source))?;
        let mut batch = Vec::new();
        nlmsg(
            &mut batch,
            NFNL_MSG_BATCH_BEGIN,
            NLM_F_REQUEST,
            1,
            &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
        );
        for (index, mutation) in mutations.iter().enumerate() {
            let operation = if mutation.add { NFT_MSG_NEWSETELEM } else { NFT_MSG_DELSETELEM };
            let flags = if mutation.add { NLM_F_CREATE } else { 0 };
            nlmsg(
                &mut batch,
                nft_msg_type(operation),
                NLM_F_REQUEST | NLM_F_ACK | flags,
                index as u32 + 2,
                &element_payload(element_set_name(mutation.set), mutation.set_id, &mutation.key),
            );
        }
        let end = mutations.len() as u32 + 2;
        nlmsg(
            &mut batch,
            NFNL_MSG_BATCH_END,
            NLM_F_REQUEST,
            end,
            &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES),
        );
        sock.send(&batch)
            .map_err(|source| NetlinkError::nft("shared-ip-element-transaction", source))?;
        let mut pending = (2..end).collect::<BTreeSet<_>>();
        while !pending.is_empty() {
            let mut buffer = vec![0_u8; 32_768];
            let received = sock
                .recv(&mut buffer)
                .map_err(|source| NetlinkError::nft("shared-ip-element-transaction", source))?;
            collect_atomic_rule_acks(&buffer[..received], &mut pending)
                .map_err(|source| NetlinkError::nft("shared-ip-element-transaction", source))?;
        }
        Ok(())
    }

    fn state_for(
        expected: &SharedIpInterceptIdentity,
    ) -> Result<SharedIpInterceptState, NetlinkError> {
        let Some(state) = collect_state()? else {
            return Err(invalid_shared_ip("shared IP constant program is absent"));
        };
        if state.identity != *expected {
            return Err(invalid_shared_ip("shared IP constant program identity mismatch"));
        }
        Ok(state)
    }

    fn mutate_and_readback(
        expected: &SharedIpInterceptIdentity,
        before: SharedIpInterceptState,
        mutations: &[ElementMutation],
        expected_after: &SharedIpInterceptState,
    ) -> Result<SharedIpInterceptState, NetlinkError> {
        if before.identity != *expected || expected_after.identity != *expected {
            return Err(invalid_shared_ip(
                "shared IP element identity is not the expected program",
            ));
        }
        if mutations.is_empty() {
            return Ok(before);
        }
        send_element_transaction(mutations)?;
        let primary = match collect_state() {
            Ok(Some(observed)) if observed == *expected_after => return Ok(observed),
            Ok(Some(_)) => invalid_shared_ip("shared IP element read-back identity mismatch"),
            Ok(None) => invalid_shared_ip("shared IP program disappeared"),
            Err(source) => source,
        };
        // A committed batch with an unexpected or failed read-back is restored once.
        let inverse = mutations
            .iter()
            .map(|mutation| ElementMutation {
                set: mutation.set,
                set_id: mutation.set_id,
                key: mutation.key.clone(),
                add: !mutation.add,
            })
            .collect::<Vec<_>>();
        if let Err(restore_source) = send_element_transaction(&inverse) {
            return Err(element_restore_error(primary, restore_source));
        }
        match collect_state() {
            Ok(Some(restored)) if restored == before => Err(primary),
            Ok(Some(_)) => Err(element_restore_error(
                primary,
                invalid_shared_ip("shared IP element restoration read-back mismatch"),
            )),
            Ok(None) => Err(element_restore_error(
                primary,
                invalid_shared_ip("shared IP program disappeared during restoration"),
            )),
            Err(restore_source) => Err(element_restore_error(primary, restore_source)),
        }
    }

    fn member_mutations(
        ids: &BTreeMap<ElementSet, u32>,
        additions: impl IntoIterator<Item = ElementKey>,
        removals: impl IntoIterator<Item = ElementKey>,
    ) -> Result<Vec<ElementMutation>, NetlinkError> {
        let mut mutations = Vec::new();
        for (key, add) in additions
            .into_iter()
            .map(|key| (key, true))
            .chain(removals.into_iter().map(|key| (key, false)))
        {
            let (set, bytes) = element_key_bytes(key)?;
            let set_id =
                *ids.get(&set).ok_or_else(|| invalid_shared_ip("shared IP set id missing"))?;
            mutations.push(ElementMutation { set, set_id, key: bytes, add });
        }
        Ok(mutations)
    }

    pub(super) fn insert_outbound(
        expected: &SharedIpInterceptIdentity,
        source: Ipv4Addr,
    ) -> Result<SharedIpInterceptState, NetlinkError> {
        let before = state_for(expected)?;
        if before.managed_guest_ips.contains(&source) || before.outbound_sources.contains(&source) {
            return Err(invalid_shared_ip("shared IP outbound group is already partially present"));
        }
        let ids = set_ids()?;
        let mutations = member_mutations(
            &ids,
            [ElementKey::ManagedGuest(source), ElementKey::OutboundSource(source)],
            [],
        )?;
        let mut expected_after = before.clone();
        expected_after.managed_guest_ips.insert(source);
        expected_after.outbound_sources.insert(source);
        mutate_and_readback(expected, before, &mutations, &expected_after)
    }

    pub(super) fn insert_inbound(
        expected: &SharedIpInterceptIdentity,
        destination: SocketAddrV4,
    ) -> Result<SharedIpInterceptState, NetlinkError> {
        if destination.port() == 0 {
            return Err(invalid_shared_ip("shared IP destination port is zero"));
        }
        let before = state_for(expected)?;
        if before.inbound_destinations.contains(&destination) {
            return Err(invalid_shared_ip("shared IP inbound member is already present"));
        }
        let ids = set_ids()?;
        let mutations = member_mutations(&ids, [ElementKey::Destination(destination)], [])?;
        let mut expected_after = before.clone();
        expected_after.inbound_destinations.insert(destination);
        mutate_and_readback(expected, before, &mutations, &expected_after)
    }

    pub(super) fn delete_elements(
        expected: &SharedIpInterceptIdentity,
        source: Option<Ipv4Addr>,
        inbound: &[SocketAddrV4],
    ) -> Result<SharedIpInterceptState, NetlinkError> {
        if source.is_none() && inbound.is_empty() {
            return Err(invalid_shared_ip("shared IP delete request is empty"));
        }
        let mut seen = BTreeSet::new();
        for destination in inbound {
            if destination.port() == 0 || !seen.insert(*destination) {
                return Err(invalid_shared_ip(
                    "shared IP delete destination is invalid or duplicated",
                ));
            }
        }
        let before = state_for(expected)?;
        if let Some(source) = source
            && (!before.managed_guest_ips.contains(&source)
                || !before.outbound_sources.contains(&source))
        {
            return Err(invalid_shared_ip("shared IP outbound group is incomplete"));
        }
        if inbound.iter().any(|destination| !before.inbound_destinations.contains(destination)) {
            return Err(invalid_shared_ip("shared IP inbound member is absent"));
        }
        let ids = set_ids()?;
        let mut mutations = Vec::new();
        if let Some(source) = source {
            let key = source.octets().to_vec();
            mutations.push(ElementMutation {
                set: ElementSet::ManagedGuestIps,
                set_id: *ids.get(&ElementSet::ManagedGuestIps).expect("managed set id"),
                key: key.clone(),
                add: false,
            });
            mutations.push(ElementMutation {
                set: ElementSet::OutboundSources,
                set_id: *ids.get(&ElementSet::OutboundSources).expect("outbound set id"),
                key,
                add: false,
            });
        }
        for destination in inbound {
            mutations.extend(member_mutations(&ids, [], [ElementKey::Destination(*destination)])?);
        }
        let mut expected_after = before.clone();
        if let Some(source) = source {
            expected_after.managed_guest_ips.remove(&source);
            expected_after.outbound_sources.remove(&source);
        }
        for destination in inbound {
            expected_after.inbound_destinations.remove(destination);
        }
        mutate_and_readback(expected, before, &mutations, &expected_after)
    }

    pub(super) fn clear_elements(
        expected: &SharedIpInterceptIdentity,
    ) -> Result<SharedIpInterceptState, NetlinkError> {
        let before = state_for(expected)?;
        let ids = set_ids()?;
        let mut mutations = Vec::new();
        for address in &before.managed_guest_ips {
            mutations.push(ElementMutation {
                set: ElementSet::ManagedGuestIps,
                set_id: *ids.get(&ElementSet::ManagedGuestIps).expect("managed set id"),
                key: address.octets().to_vec(),
                add: false,
            });
        }
        for address in &before.outbound_sources {
            mutations.push(ElementMutation {
                set: ElementSet::OutboundSources,
                set_id: *ids.get(&ElementSet::OutboundSources).expect("outbound set id"),
                key: address.octets().to_vec(),
                add: false,
            });
        }
        for destination in before.inbound_destinations.iter().copied() {
            mutations.extend(member_mutations(&ids, [], [ElementKey::Destination(destination)])?);
        }
        let mut expected_after = before.clone();
        expected_after.managed_guest_ips.clear();
        expected_after.outbound_sources.clear();
        expected_after.inbound_destinations.clear();
        mutate_and_readback(expected, before, &mutations, &expected_after)
    }
}

/// Semantic bridge-family proof-mark guard adapter (GH #295).
///
/// Raw nft family numbers, attributes, userdata and expression encodings stay
/// private to this parent module. This public boundary carries only the exact
/// semantic identities approved by D-295-DISTILL-9.
pub mod bridge {
    #![allow(
        dead_code,
        clippy::panic,
        clippy::expect_used,
        clippy::unnecessary_wraps,
        reason = "semantic adapter keeps exact validation and source projection local"
    )]
    use std::collections::BTreeSet;

    use super::{
        BaseChainSpec, BridgeRuleMutation, ChainKind, NF_ACCEPT, NFT_META_MARK, NFT_MSG_DELCHAIN,
        NFT_MSG_DELSET, NFT_MSG_DELSETELEM, NFT_MSG_DELTABLE, NFT_MSG_NEWCHAIN, NFT_MSG_NEWSET,
        NFT_MSG_NEWSETELEM, NFT_MSG_NEWTABLE, NFT_REG_2, NetlinkError, NftFamily, RawChainInfo,
        RawOtherChildKind, RuleCounterSnapshot, e_anonymous_counter, e_cmp_eq, e_iifname_lookup,
        e_immediate_value, e_immediate_verdict, e_meta_load, e_meta_set, invalid_data,
        list_chain_info_family, list_other_children_family, list_set_elements_family,
        list_set_info_family, list_table_names_family, newchain_payload_family,
        newset_payload_family, newtable_payload_family, normalized_rule_program_identity,
        read_nft_generation, send_atomic_bridge_rule_transaction, send_batched_family,
        send_batched_family_idempotent, set_elem_payload_family,
    };

    const MAX_IDENTIFIER_BYTES: usize = 255;
    const MAX_MEMBER_BYTES: usize = 15;
    const REQUIRED_PRIORITY: i32 = -300;
    const REQUIRED_INTERCEPT_MARK: u32 = 0x295a;
    const REQUIRED_ACCEPTED_MARK: u32 = 0x295b;
    const NF_BR_PRE_ROUTING: u32 = 0;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardSpec {
        table: String,
        chain: String,
        managed_taps_set: String,
        priority: i32,
        intercept_mark: u32,
        accepted_mark: u32,
    }

    impl BridgeGuardSpec {
        /// Validate the fixed guard identity without performing I/O.
        pub fn new(
            table: String,
            chain: String,
            managed_taps_set: String,
            priority: i32,
            intercept_mark: u32,
            accepted_mark: u32,
        ) -> Result<Self, BridgeGuardValidationError> {
            validate_identifier(BridgeGuardIdentifier::Table, &table)?;
            validate_identifier(BridgeGuardIdentifier::Chain, &chain)?;
            validate_identifier(BridgeGuardIdentifier::ManagedTapsSet, &managed_taps_set)?;
            if priority != REQUIRED_PRIORITY {
                return Err(BridgeGuardValidationError::PriorityMismatch {
                    expected: REQUIRED_PRIORITY,
                    actual: priority,
                });
            }
            if intercept_mark != REQUIRED_INTERCEPT_MARK {
                return Err(BridgeGuardValidationError::InterceptMarkMismatch {
                    expected: REQUIRED_INTERCEPT_MARK,
                    actual: intercept_mark,
                });
            }
            if accepted_mark != REQUIRED_ACCEPTED_MARK {
                return Err(BridgeGuardValidationError::AcceptedMarkMismatch {
                    expected: REQUIRED_ACCEPTED_MARK,
                    actual: accepted_mark,
                });
            }
            Ok(Self { table, chain, managed_taps_set, priority, intercept_mark, accepted_mark })
        }

        /// Sole public construction of the expected ordered semantic rule program.
        #[must_use]
        pub fn expected_rule_facts(&self) -> Vec<BridgeGuardRuleFact> {
            vec![
                BridgeGuardRuleFact {
                    identity: BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::InterceptAccept),
                    program: BridgeGuardRuleProgram {
                        expressions: vec![
                            BridgeGuardRuleExpression::IngressInterfaceInSet {
                                set: self.managed_taps_set.clone(),
                            },
                            BridgeGuardRuleExpression::MarkEquals { value: self.intercept_mark },
                            BridgeGuardRuleExpression::Accept,
                        ],
                    },
                },
                BridgeGuardRuleFact {
                    identity: BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::AcceptedClear),
                    program: BridgeGuardRuleProgram {
                        expressions: vec![
                            BridgeGuardRuleExpression::IngressInterfaceInSet {
                                set: self.managed_taps_set.clone(),
                            },
                            BridgeGuardRuleExpression::MarkEquals { value: self.accepted_mark },
                            BridgeGuardRuleExpression::SetMark { value: 0 },
                            BridgeGuardRuleExpression::Accept,
                        ],
                    },
                },
                BridgeGuardRuleFact {
                    identity: BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::DefaultDrop),
                    program: BridgeGuardRuleProgram {
                        expressions: vec![
                            BridgeGuardRuleExpression::IngressInterfaceInSet {
                                set: self.managed_taps_set.clone(),
                            },
                            BridgeGuardRuleExpression::Counter,
                            BridgeGuardRuleExpression::Drop,
                        ],
                    },
                },
            ]
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum BridgeGuardRuleKind {
        InterceptAccept,
        AcceptedClear,
        DefaultDrop,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BridgeGuardObservedFamily {
        Bridge,
        Inet,
        Ipv4,
        Ipv6,
        Arp,
        Netdev,
        Other,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardTableFact {
        pub family: BridgeGuardObservedFamily,
        pub name: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BridgeGuardChainType {
        Filter,
        Route,
        Nat,
        Other,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BridgeGuardChainHook {
        Prerouting,
        Input,
        Forward,
        Output,
        Postrouting,
        Ingress,
        Egress,
        Other,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BridgeGuardChainPolicy {
        Accept,
        Drop,
        Other,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardChainDefinition {
        Base {
            chain_type: BridgeGuardChainType,
            hook: BridgeGuardChainHook,
            priority: i32,
            policy: Option<BridgeGuardChainPolicy>,
        },
        Regular,
        Unsupported,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardChainOccurrence {
        pub table: BridgeGuardTableFact,
        pub name: String,
        pub handle: Option<u64>,
        pub definition: BridgeGuardChainDefinition,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardSetFact {
        pub table: BridgeGuardTableFact,
        pub name: String,
        pub key_len: u32,
        pub ifname_key: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardRuleIdentity {
        Owned(BridgeGuardRuleKind),
        Foreign,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardRuleExpression {
        IngressInterfaceInSet { set: String },
        MarkEquals { value: u32 },
        SetMark { value: u32 },
        Counter,
        Accept,
        Drop,
        Unknown { name: String },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardRuleProgram {
        pub expressions: Vec<BridgeGuardRuleExpression>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardRuleFact {
        pub identity: BridgeGuardRuleIdentity,
        pub program: BridgeGuardRuleProgram,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardRuleOccurrence {
        pub table: BridgeGuardTableFact,
        pub chain: String,
        pub handle: u64,
        pub fact: BridgeGuardRuleFact,
        pub counter: Option<RuleCounterSnapshot>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardMemberIdentity {
        Ifname(String),
        ForeignEncoding { encoded_len: usize },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardMemberOccurrence {
        pub table: BridgeGuardTableFact,
        pub set: String,
        pub identity: BridgeGuardMemberIdentity,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BridgeGuardOtherChildKind {
        Flowtable,
        StatefulObject,
        Other,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardOtherChildOccurrence {
        pub table: BridgeGuardTableFact,
        pub kind: BridgeGuardOtherChildKind,
        pub name: Option<String>,
        pub handle: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BridgeGuardInventory {
        pub generation: u32,
        pub tables: Vec<BridgeGuardTableFact>,
        pub chains: Vec<BridgeGuardChainOccurrence>,
        pub sets: Vec<BridgeGuardSetFact>,
        pub rules: Vec<BridgeGuardRuleOccurrence>,
        pub members: Vec<BridgeGuardMemberOccurrence>,
        pub other_children: Vec<BridgeGuardOtherChildOccurrence>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardObservation {
        Absent { inventory: BridgeGuardInventory },
        Exact { inventory: BridgeGuardInventory },
        Conflict { inventory: BridgeGuardInventory },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardMutationOutcome {
        Converged { observed: BridgeGuardInventory },
        Conflict { observed: BridgeGuardInventory },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BridgeGuardDeleteOutcome {
        Absent { observed: BridgeGuardInventory },
        Deleted { observed: BridgeGuardInventory },
        Conflict { observed: BridgeGuardInventory },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BridgeGuardIdentifier {
        Table,
        Chain,
        ManagedTapsSet,
    }

    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    pub enum BridgeGuardValidationError {
        #[error("bridge guard {identifier:?} identifier is empty")]
        EmptyIdentifier { identifier: BridgeGuardIdentifier },
        #[error("bridge guard {identifier:?} identifier contains NUL at byte {index}")]
        IdentifierContainsNul { identifier: BridgeGuardIdentifier, index: usize },
        #[error("bridge guard {identifier:?} identifier is {length} bytes; maximum is {maximum}")]
        IdentifierTooLong { identifier: BridgeGuardIdentifier, length: usize, maximum: usize },
        #[error("bridge guard priority must be {expected}, got {actual}")]
        PriorityMismatch { expected: i32, actual: i32 },
        #[error("bridge guard intercept mark must be {expected:#x}, got {actual:#x}")]
        InterceptMarkMismatch { expected: u32, actual: u32 },
        #[error("bridge guard accepted mark must be {expected:#x}, got {actual:#x}")]
        AcceptedMarkMismatch { expected: u32, actual: u32 },
        #[error("bridge guard managed TAP name is empty")]
        EmptyMember,
        #[error("bridge guard managed TAP name contains NUL at byte {index}")]
        MemberContainsNul { index: usize },
        #[error("bridge guard managed TAP name is {length} bytes; maximum is {maximum}")]
        MemberTooLong { length: usize, maximum: usize },
    }

    #[derive(Debug, thiserror::Error)]
    pub enum BridgeGuardError {
        #[error(transparent)]
        Validation(#[from] BridgeGuardValidationError),
        #[error(transparent)]
        Netlink(#[from] NetlinkError),
    }

    fn validate_identifier(
        identifier: BridgeGuardIdentifier,
        value: &str,
    ) -> Result<(), BridgeGuardValidationError> {
        if value.is_empty() {
            return Err(BridgeGuardValidationError::EmptyIdentifier { identifier });
        }
        if let Some(index) = value.as_bytes().iter().position(|byte| *byte == 0) {
            return Err(BridgeGuardValidationError::IdentifierContainsNul { identifier, index });
        }
        if value.len() > MAX_IDENTIFIER_BYTES {
            return Err(BridgeGuardValidationError::IdentifierTooLong {
                identifier,
                length: value.len(),
                maximum: MAX_IDENTIFIER_BYTES,
            });
        }
        Ok(())
    }

    fn validate_member(value: &str) -> Result<(), BridgeGuardValidationError> {
        if value.is_empty() {
            return Err(BridgeGuardValidationError::EmptyMember);
        }
        if let Some(index) = value.as_bytes().iter().position(|byte| *byte == 0) {
            return Err(BridgeGuardValidationError::MemberContainsNul { index });
        }
        if value.len() > MAX_MEMBER_BYTES {
            return Err(BridgeGuardValidationError::MemberTooLong {
                length: value.len(),
                maximum: MAX_MEMBER_BYTES,
            });
        }
        Ok(())
    }

    fn expected_inventory(spec: &BridgeGuardSpec) -> BridgeGuardInventory {
        let table = BridgeGuardTableFact {
            family: BridgeGuardObservedFamily::Bridge,
            name: spec.table.clone(),
        };
        let chain = BridgeGuardChainOccurrence {
            table: table.clone(),
            name: spec.chain.clone(),
            handle: None,
            definition: BridgeGuardChainDefinition::Base {
                chain_type: BridgeGuardChainType::Filter,
                hook: BridgeGuardChainHook::Prerouting,
                priority: spec.priority,
                policy: Some(BridgeGuardChainPolicy::Accept),
            },
        };
        BridgeGuardInventory {
            generation: 0,
            tables: vec![table.clone()],
            chains: vec![chain],
            sets: vec![BridgeGuardSetFact {
                table: table.clone(),
                name: spec.managed_taps_set.clone(),
                key_len: 16,
                ifname_key: true,
            }],
            rules: spec
                .expected_rule_facts()
                .into_iter()
                .map(|fact| BridgeGuardRuleOccurrence {
                    table: table.clone(),
                    chain: spec.chain.clone(),
                    handle: 0,
                    fact,
                    counter: None,
                })
                .collect(),
            members: Vec::new(),
            other_children: Vec::new(),
        }
    }

    fn table_fact(name: &str) -> BridgeGuardTableFact {
        BridgeGuardTableFact { family: BridgeGuardObservedFamily::Bridge, name: name.to_owned() }
    }

    fn project_chain(raw: RawChainInfo) -> BridgeGuardChainOccurrence {
        let definition = match (raw.hook, raw.chain_type.as_deref(), raw.policy) {
            (Some((hooknum, priority)), Some(chain_type), policy) => {
                BridgeGuardChainDefinition::Base {
                    chain_type: match chain_type {
                        "filter" => BridgeGuardChainType::Filter,
                        "route" => BridgeGuardChainType::Route,
                        "nat" => BridgeGuardChainType::Nat,
                        _ => BridgeGuardChainType::Other,
                    },
                    hook: match hooknum {
                        0 => BridgeGuardChainHook::Prerouting,
                        1 => BridgeGuardChainHook::Input,
                        2 => BridgeGuardChainHook::Forward,
                        3 => BridgeGuardChainHook::Output,
                        4 => BridgeGuardChainHook::Postrouting,
                        _ => BridgeGuardChainHook::Other,
                    },
                    priority,
                    policy: policy.map(|policy| match policy {
                        NF_ACCEPT => BridgeGuardChainPolicy::Accept,
                        0 => BridgeGuardChainPolicy::Drop,
                        _ => BridgeGuardChainPolicy::Other,
                    }),
                }
            }
            (None, None, None) => BridgeGuardChainDefinition::Regular,
            _ => BridgeGuardChainDefinition::Unsupported,
        };
        BridgeGuardChainOccurrence {
            table: table_fact(&raw.table),
            name: raw.name,
            handle: Some(raw.handle),
            definition,
        }
    }

    fn rule_expressions(spec: &BridgeGuardSpec, index: usize) -> Vec<u8> {
        let mut expressions = e_iifname_lookup(&spec.managed_taps_set);
        match index {
            0 => {
                expressions.extend(e_meta_load(NFT_META_MARK, NFT_REG_2));
                expressions.extend(e_cmp_eq(NFT_REG_2, &spec.intercept_mark.to_ne_bytes()));
                expressions.extend(e_immediate_verdict(NF_ACCEPT));
            }
            1 => {
                expressions.extend(e_meta_load(NFT_META_MARK, NFT_REG_2));
                expressions.extend(e_cmp_eq(NFT_REG_2, &spec.accepted_mark.to_ne_bytes()));
                expressions.extend(e_immediate_value(NFT_REG_2, &0_u32.to_ne_bytes()));
                expressions.extend(e_meta_set(NFT_META_MARK, NFT_REG_2));
                expressions.extend(e_immediate_verdict(NF_ACCEPT));
            }
            _ => {
                expressions.extend(e_anonymous_counter());
                expressions.extend(e_immediate_verdict(0));
            }
        }
        expressions
    }

    fn expected_rule_programs(spec: &BridgeGuardSpec) -> Vec<Vec<u8>> {
        (0..spec.expected_rule_facts().len())
            .map(|index| {
                normalized_rule_program_identity(&rule_expressions(spec, index)).unwrap_or_default()
            })
            .collect()
    }

    fn rule_expression_names(program: &[u8]) -> Vec<String> {
        let Ok(elements) = super::exact_attrs(program) else {
            return vec!["malformed".to_owned()];
        };
        elements
            .into_iter()
            .filter_map(|(kind, _, element)| {
                if kind != super::NFTA_LIST_ELEM {
                    return Some("malformed".to_owned());
                }
                super::exact_attrs(element).ok()?.into_iter().find_map(|(attribute, _, value)| {
                    (attribute == super::NFTA_EXPR_NAME)
                        .then(|| {
                            super::exact_cstr(value, "expression name").ok().map(str::to_owned)
                        })
                        .flatten()
                })
            })
            .collect()
    }

    fn project_rule(
        spec: &BridgeGuardSpec,
        table: &BridgeGuardTableFact,
        chain: &str,
        rule: &super::RuleInfo,
        expected_programs: &[Vec<u8>],
    ) -> BridgeGuardRuleOccurrence {
        let expected_rules = spec.expected_rule_facts();
        let owned_index = rule
            .userdata
            .strip_prefix(b"ovd295-bridge-")
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| value.parse::<usize>().ok());
        let expression_names = rule_expression_names(&rule.normalized_program);
        let fact = owned_index
            .and_then(|index| expected_rules.get(index).cloned())
            .and_then(|fact| {
                let index = owned_index?;
                if expected_programs.get(index) == Some(&rule.normalized_program) {
                    Some(fact)
                } else {
                    Some(BridgeGuardRuleFact {
                        identity: fact.identity,
                        program: BridgeGuardRuleProgram {
                            expressions: expression_names
                                .clone()
                                .into_iter()
                                .map(|name| BridgeGuardRuleExpression::Unknown { name })
                                .collect(),
                        },
                    })
                }
            })
            .unwrap_or_else(|| BridgeGuardRuleFact {
                identity: BridgeGuardRuleIdentity::Foreign,
                program: BridgeGuardRuleProgram {
                    expressions: expression_names
                        .into_iter()
                        .map(|name| BridgeGuardRuleExpression::Unknown { name })
                        .collect(),
                },
            });
        BridgeGuardRuleOccurrence {
            table: table.clone(),
            chain: chain.to_owned(),
            handle: rule.handle,
            fact,
            counter: rule.counter,
        }
    }

    fn encode_member(value: &str) -> Result<[u8; 16], BridgeGuardValidationError> {
        validate_member(value)?;
        let mut encoded = [0_u8; 16];
        encoded[..value.len()].copy_from_slice(value.as_bytes());
        Ok(encoded)
    }

    #[allow(dead_code, reason = "activated by D9 generation-bracketed observe implementation")]
    fn classify_inventory(
        spec: &BridgeGuardSpec,
        expected_members: &BTreeSet<String>,
        observed_inventory: BridgeGuardInventory,
    ) -> BridgeGuardObservation {
        let expected = expected_inventory(spec);
        if observed_inventory.tables.is_empty() {
            return BridgeGuardObservation::Absent { inventory: observed_inventory };
        }
        let owned_rules_match = observed_inventory.rules.len() == expected.rules.len()
            && observed_inventory.rules.iter().zip(expected.rules.iter()).all(
                |(observed, expected)| {
                    observed.table == expected.table
                        && observed.chain == expected.chain
                        && observed.fact == expected.fact
                },
            );
        let chains_match = observed_inventory.chains.len() == expected.chains.len()
            && observed_inventory.chains.iter().zip(expected.chains.iter()).all(
                |(observed, expected)| {
                    observed.table == expected.table
                        && observed.name == expected.name
                        && observed.definition == expected.definition
                },
            );
        if observed_inventory.tables == expected.tables
            && chains_match
            && observed_inventory.sets == expected.sets
            && owned_rules_match
            && observed_inventory
                .members
                .iter()
                .filter_map(|member| match &member.identity {
                    BridgeGuardMemberIdentity::Ifname(name) => Some(name.clone()),
                    BridgeGuardMemberIdentity::ForeignEncoding { .. } => None,
                })
                .collect::<BTreeSet<_>>()
                == *expected_members
            && observed_inventory.other_children.is_empty()
        {
            BridgeGuardObservation::Exact { inventory: observed_inventory }
        } else {
            BridgeGuardObservation::Conflict { inventory: observed_inventory }
        }
    }

    pub fn converge_table(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        if list_table_names_family(NftFamily::Bridge)?.iter().any(|name| name == &spec.table) {
            return Ok(BridgeGuardMutationOutcome::Converged {
                observed: expected_inventory(spec),
            });
        }
        send_batched_family_idempotent(
            NftFamily::Bridge,
            NFT_MSG_NEWTABLE,
            &newtable_payload_family(NftFamily::Bridge, &spec.table),
            "bridge-newtable",
        )?;
        Ok(BridgeGuardMutationOutcome::Converged { observed: expected_inventory(spec) })
    }
    pub fn converge_chain(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        if let Some(existing) = list_chain_info_family(NftFamily::Bridge, &spec.table)?
            .into_iter()
            .find(|chain| chain.name == spec.chain)
        {
            let observed = project_chain(existing);
            let expected = expected_inventory(spec).chains[0].clone();
            if observed.table == expected.table
                && observed.name == expected.name
                && observed.definition == expected.definition
            {
                return Ok(BridgeGuardMutationOutcome::Converged {
                    observed: expected_inventory(spec),
                });
            }
            let observed = match observe(spec, &BTreeSet::new())? {
                BridgeGuardObservation::Absent { inventory }
                | BridgeGuardObservation::Exact { inventory }
                | BridgeGuardObservation::Conflict { inventory } => inventory,
            };
            return Ok(BridgeGuardMutationOutcome::Conflict { observed });
        }
        send_batched_family_idempotent(
            NftFamily::Bridge,
            NFT_MSG_NEWCHAIN,
            &newchain_payload_family(
                NftFamily::Bridge,
                &spec.table,
                &spec.chain,
                BaseChainSpec {
                    hooknum: NF_BR_PRE_ROUTING,
                    priority: spec.priority,
                    kind: ChainKind::Filter,
                },
            ),
            "bridge-newchain",
        )?;
        Ok(BridgeGuardMutationOutcome::Converged { observed: expected_inventory(spec) })
    }
    pub fn converge_set(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        if let Some(existing) = list_set_info_family(NftFamily::Bridge, &spec.table)?
            .into_iter()
            .find(|set| set.name == spec.managed_taps_set)
        {
            if existing.key_type == super::NFT_IFNAME_KEY_TYPE
                && existing.key_len == super::IFNAMSIZ as u32
            {
                return Ok(BridgeGuardMutationOutcome::Converged {
                    observed: expected_inventory(spec),
                });
            }
            let observed = match observe(spec, &BTreeSet::new())? {
                BridgeGuardObservation::Absent { inventory }
                | BridgeGuardObservation::Exact { inventory }
                | BridgeGuardObservation::Conflict { inventory } => inventory,
            };
            return Ok(BridgeGuardMutationOutcome::Conflict { observed });
        }
        send_batched_family_idempotent(
            NftFamily::Bridge,
            NFT_MSG_NEWSET,
            &newset_payload_family(NftFamily::Bridge, &spec.table, &spec.managed_taps_set),
            "bridge-newset",
        )?;
        Ok(BridgeGuardMutationOutcome::Converged { observed: expected_inventory(spec) })
    }
    pub fn converge_rules(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        let expected_programs = expected_rule_programs(spec);
        match super::list_rules_family(NftFamily::Bridge, &spec.table, &spec.chain) {
            Ok(existing) if !existing.is_empty() => {
                let mut owned = BTreeSet::new();
                let mut conflict = false;
                for rule in &existing {
                    let Some(index) = rule
                        .userdata
                        .strip_prefix(b"ovd295-bridge-")
                        .and_then(|value| std::str::from_utf8(value).ok())
                        .and_then(|value| value.parse::<usize>().ok())
                    else {
                        conflict = true;
                        continue;
                    };
                    if index >= spec.expected_rule_facts().len()
                        || !owned.insert(index)
                        || expected_programs.get(index) != Some(&rule.normalized_program)
                    {
                        conflict = true;
                    }
                }
                if !conflict && owned.len() == spec.expected_rule_facts().len() {
                    return Ok(BridgeGuardMutationOutcome::Converged {
                        observed: expected_inventory(spec),
                    });
                }
                let observation = observe(spec, &BTreeSet::new())?;
                let BridgeGuardObservation::Conflict { inventory } = observation else {
                    return Ok(BridgeGuardMutationOutcome::Conflict {
                        observed: expected_inventory(spec),
                    });
                };
                return Ok(BridgeGuardMutationOutcome::Conflict { observed: inventory });
            }
            Ok(_) => {}
            Err(error) if error.errno() == Some(-libc::ENOENT) => {}
            Err(error) => return Err(BridgeGuardError::Netlink(error)),
        }
        let mut mutations = Vec::with_capacity(3);
        for index in 0..3 {
            let mut expressions = e_iifname_lookup(&spec.managed_taps_set);
            match index {
                0 => {
                    expressions.extend(e_meta_load(NFT_META_MARK, NFT_REG_2));
                    expressions.extend(e_cmp_eq(NFT_REG_2, &spec.intercept_mark.to_ne_bytes()));
                    expressions.extend(e_immediate_verdict(NF_ACCEPT));
                }
                1 => {
                    expressions.extend(e_meta_load(NFT_META_MARK, NFT_REG_2));
                    expressions.extend(e_cmp_eq(NFT_REG_2, &spec.accepted_mark.to_ne_bytes()));
                    expressions.extend(e_immediate_value(NFT_REG_2, &0_u32.to_ne_bytes()));
                    expressions.extend(e_meta_set(NFT_META_MARK, NFT_REG_2));
                    expressions.extend(e_immediate_verdict(NF_ACCEPT));
                }
                _ => {
                    expressions.extend(e_anonymous_counter());
                    expressions.extend(e_immediate_verdict(0));
                }
            }
            let userdata = format!("ovd295-bridge-{index}").into_bytes();
            mutations.push((expressions, userdata, index != 0));
        }
        let bridge_mutations = mutations
            .iter()
            .map(|(expressions, userdata, append)| BridgeRuleMutation {
                table: &spec.table,
                chain: &spec.chain,
                expressions,
                userdata,
                append: *append,
            })
            .collect::<Vec<_>>();
        send_atomic_bridge_rule_transaction(&bridge_mutations)?;
        Ok(BridgeGuardMutationOutcome::Converged { observed: expected_inventory(spec) })
    }

    pub fn observe(
        spec: &BridgeGuardSpec,
        expected_members: &BTreeSet<String>,
    ) -> Result<BridgeGuardObservation, BridgeGuardError> {
        let before = read_nft_generation()?;
        let tables = list_table_names_family(NftFamily::Bridge)?;
        if !tables.iter().any(|name| name == &spec.table) {
            let after = read_nft_generation()?;
            if before != after {
                return Err(BridgeGuardError::Netlink(NetlinkError::nft(
                    "bridge-observe-generation",
                    invalid_data("bridge inventory changed while table was absent"),
                )));
            }
            return Ok(BridgeGuardObservation::Absent {
                inventory: BridgeGuardInventory {
                    generation: before,
                    tables: Vec::new(),
                    chains: Vec::new(),
                    sets: Vec::new(),
                    rules: Vec::new(),
                    members: Vec::new(),
                    other_children: Vec::new(),
                },
            });
        }

        let table = table_fact(&spec.table);
        let raw_chains = list_chain_info_family(NftFamily::Bridge, &spec.table)?;
        let chains = raw_chains.iter().cloned().map(project_chain).collect::<Vec<_>>();
        let raw_sets = list_set_info_family(NftFamily::Bridge, &spec.table)?;
        let sets = raw_sets
            .iter()
            .map(|set| BridgeGuardSetFact {
                table: table.clone(),
                name: set.name.clone(),
                key_len: set.key_len,
                ifname_key: set.key_type == super::NFT_IFNAME_KEY_TYPE,
            })
            .collect::<Vec<_>>();
        let expected_programs = expected_rule_programs(spec);
        let mut rules = Vec::new();
        for chain in &chains {
            for rule in super::list_rules_family(NftFamily::Bridge, &spec.table, &chain.name)? {
                rules.push(project_rule(spec, &table, &chain.name, &rule, &expected_programs));
            }
        }
        let mut members = Vec::new();
        if let Some(set) = raw_sets.iter().find(|set| set.name == spec.managed_taps_set) {
            for member in list_set_elements_family(
                NftFamily::Bridge,
                &spec.table,
                &spec.managed_taps_set,
                set.id,
            )? {
                let identity = if member.len() == super::IFNAMSIZ {
                    let nul = member.iter().position(|byte| *byte == 0).unwrap_or(member.len());
                    std::str::from_utf8(&member[..nul]).map_or(
                        BridgeGuardMemberIdentity::ForeignEncoding { encoded_len: member.len() },
                        |name| BridgeGuardMemberIdentity::Ifname(name.to_owned()),
                    )
                } else {
                    BridgeGuardMemberIdentity::ForeignEncoding { encoded_len: member.len() }
                };
                members.push(BridgeGuardMemberOccurrence {
                    table: table.clone(),
                    set: set.name.clone(),
                    identity,
                });
            }
        }
        let other_children = list_other_children_family(NftFamily::Bridge, &spec.table)?
            .into_iter()
            .map(|child| BridgeGuardOtherChildOccurrence {
                table: table.clone(),
                kind: match child.kind {
                    RawOtherChildKind::Flowtable => BridgeGuardOtherChildKind::Flowtable,
                    RawOtherChildKind::StatefulObject => BridgeGuardOtherChildKind::StatefulObject,
                },
                name: Some(child.name),
                handle: child.handle,
            })
            .collect::<Vec<_>>();
        let after = read_nft_generation()?;
        if before != after {
            return Err(BridgeGuardError::Netlink(NetlinkError::nft(
                "bridge-observe-generation",
                invalid_data("bridge inventory changed during generation-bracketed observation"),
            )));
        }
        let observed = BridgeGuardInventory {
            generation: before,
            tables: vec![table],
            chains,
            sets,
            rules,
            members,
            other_children,
        };
        Ok(classify_inventory(spec, expected_members, observed))
    }

    pub fn insert_member(
        spec: &BridgeGuardSpec,
        tap: &str,
    ) -> Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        let member = encode_member(tap)?;
        send_batched_family_idempotent(
            NftFamily::Bridge,
            NFT_MSG_NEWSETELEM,
            &set_elem_payload_family(
                NftFamily::Bridge,
                &spec.table,
                &spec.managed_taps_set,
                &member,
            ),
            "bridge-insert-member",
        )?;
        Ok(BridgeGuardMutationOutcome::Converged { observed: expected_inventory(spec) })
    }

    pub fn delete_member(
        spec: &BridgeGuardSpec,
        tap: &str,
    ) -> Result<BridgeGuardMutationOutcome, BridgeGuardError> {
        let member = encode_member(tap)?;
        match send_batched_family(
            NftFamily::Bridge,
            NFT_MSG_DELSETELEM,
            0,
            &set_elem_payload_family(
                NftFamily::Bridge,
                &spec.table,
                &spec.managed_taps_set,
                &member,
            ),
            "bridge-delete-member",
        ) {
            Ok(()) => {}
            Err(error) if error.errno() == Some(-libc::ENOENT) => {}
            Err(error) => return Err(BridgeGuardError::Netlink(error)),
        }
        Ok(BridgeGuardMutationOutcome::Converged { observed: expected_inventory(spec) })
    }

    pub fn delete_rules(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardDeleteOutcome, BridgeGuardError> {
        let rules = super::list_rules_family(NftFamily::Bridge, &spec.table, &spec.chain)
            .map_err(BridgeGuardError::Netlink)?;
        for rule in rules {
            if rule.userdata.starts_with(b"ovd295-bridge-") {
                send_batched_family(
                    NftFamily::Bridge,
                    super::NFT_MSG_DELRULE,
                    0,
                    &super::delrule_payload_family(
                        NftFamily::Bridge,
                        &spec.table,
                        &spec.chain,
                        rule.handle,
                    ),
                    "bridge-delete-rule",
                )?;
            }
        }
        Ok(BridgeGuardDeleteOutcome::Deleted { observed: expected_inventory(spec) })
    }
    pub fn delete_set(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardDeleteOutcome, BridgeGuardError> {
        send_batched_family(
            NftFamily::Bridge,
            NFT_MSG_DELSET,
            0,
            &newset_payload_family(NftFamily::Bridge, &spec.table, &spec.managed_taps_set),
            "bridge-delete-set",
        )?;
        Ok(BridgeGuardDeleteOutcome::Deleted { observed: expected_inventory(spec) })
    }
    pub fn delete_chain(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardDeleteOutcome, BridgeGuardError> {
        send_batched_family(
            NftFamily::Bridge,
            NFT_MSG_DELCHAIN,
            0,
            &super::get_by_table_chain_family(
                NftFamily::Bridge,
                &spec.table,
                &spec.chain,
                super::NFTA_CHAIN_NAME,
                super::NFTA_CHAIN_TABLE,
            ),
            "bridge-delete-chain",
        )?;
        Ok(BridgeGuardDeleteOutcome::Deleted { observed: expected_inventory(spec) })
    }
    pub fn delete_table(
        spec: &BridgeGuardSpec,
    ) -> Result<BridgeGuardDeleteOutcome, BridgeGuardError> {
        send_batched_family(
            NftFamily::Bridge,
            NFT_MSG_DELTABLE,
            0,
            &newtable_payload_family(NftFamily::Bridge, &spec.table),
            "bridge-delete-table",
        )?;
        Ok(BridgeGuardDeleteOutcome::Deleted { observed: expected_inventory(spec) })
    }

    pub fn delete_owned_guard(
        spec: &BridgeGuardSpec,
        expected_members: &BTreeSet<String>,
    ) -> Result<BridgeGuardDeleteOutcome, BridgeGuardError> {
        match observe(spec, expected_members)? {
            BridgeGuardObservation::Absent { inventory } => {
                Ok(BridgeGuardDeleteOutcome::Absent { observed: inventory })
            }
            BridgeGuardObservation::Conflict { inventory } => {
                Ok(BridgeGuardDeleteOutcome::Conflict { observed: inventory })
            }
            BridgeGuardObservation::Exact { inventory } => {
                let _ = delete_rules(spec)?;
                let _ = delete_set(spec)?;
                let _ = delete_chain(spec)?;
                let _ = delete_table(spec)?;
                Ok(BridgeGuardDeleteOutcome::Deleted { observed: inventory })
            }
        }
    }

    #[cfg(test)]
    #[allow(clippy::doc_markdown, clippy::expect_used)]
    mod acceptance {
        use super::*;
        use crate::nft::NftFamily;

        fn spec() -> BridgeGuardSpec {
            BridgeGuardSpec::new(
                "overdrive-mtls".to_owned(),
                "prerouting".to_owned(),
                "managed_taps".to_owned(),
                REQUIRED_PRIORITY,
                REQUIRED_INTERCEPT_MARK,
                REQUIRED_ACCEPTED_MARK,
            )
            .expect("canonical bridge guard spec")
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn private_family_projection_is_closed_and_exact() {
            assert_eq!(NftFamily::Ipv4.nfproto(), 2);
            assert_eq!(NftFamily::Bridge.nfproto(), 7);
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn error_algebra_keeps_validation_distinct_from_source_bearing_netlink_failure() {
            let validation = BridgeGuardError::from(BridgeGuardValidationError::EmptyMember);
            assert!(matches!(validation, BridgeGuardError::Validation(_)));
            let netlink = BridgeGuardError::from(NetlinkError::nft(
                "bridge-observe",
                std::io::Error::from_raw_os_error(libc::EIO),
            ));
            assert!(matches!(
                netlink,
                BridgeGuardError::Netlink(NetlinkError::Nft { op: "bridge-observe", .. })
            ));
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn expected_rule_facts_are_the_single_ordered_semantic_program() {
            let facts = spec().expected_rule_facts();
            assert_eq!(facts.len(), 3);
            assert!(matches!(
                facts[0].identity,
                BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::InterceptAccept)
            ));
            assert!(matches!(
                facts[1].identity,
                BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::AcceptedClear)
            ));
            assert!(matches!(
                facts[2].identity,
                BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::DefaultDrop)
            ));
            assert_eq!(
                facts[2].program.expressions,
                [
                    BridgeGuardRuleExpression::IngressInterfaceInSet {
                        set: "managed_taps".to_owned()
                    },
                    BridgeGuardRuleExpression::Counter,
                    BridgeGuardRuleExpression::Drop,
                ]
            );
        }

        fn exact_inventory(sut: &BridgeGuardSpec) -> BridgeGuardInventory {
            let table = BridgeGuardTableFact {
                family: BridgeGuardObservedFamily::Bridge,
                name: sut.table.clone(),
            };
            BridgeGuardInventory {
                generation: 7,
                tables: vec![table.clone()],
                chains: vec![BridgeGuardChainOccurrence {
                    table: table.clone(),
                    name: sut.chain.clone(),
                    handle: Some(11),
                    definition: BridgeGuardChainDefinition::Base {
                        chain_type: BridgeGuardChainType::Filter,
                        hook: BridgeGuardChainHook::Prerouting,
                        priority: sut.priority,
                        policy: Some(BridgeGuardChainPolicy::Accept),
                    },
                }],
                sets: vec![BridgeGuardSetFact {
                    table: table.clone(),
                    name: sut.managed_taps_set.clone(),
                    key_len: 16,
                    ifname_key: true,
                }],
                rules: sut
                    .expected_rule_facts()
                    .into_iter()
                    .enumerate()
                    .map(|(index, fact)| BridgeGuardRuleOccurrence {
                        table: table.clone(),
                        chain: sut.chain.clone(),
                        handle: u64::try_from(index + 20).expect("small handle"),
                        fact,
                        counter: None,
                    })
                    .collect(),
                members: vec![BridgeGuardMemberOccurrence {
                    table,
                    set: sut.managed_taps_set.clone(),
                    identity: BridgeGuardMemberIdentity::Ifname("ovd-tp-0002".to_owned()),
                }],
                other_children: Vec::new(),
            }
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn semantic_classification_preserves_absent_exact_and_every_conflict_partition() {
            let sut = spec();
            let expected_members = BTreeSet::from(["ovd-tp-0002".to_owned()]);
            let empty = BridgeGuardInventory {
                generation: 1,
                tables: Vec::new(),
                chains: Vec::new(),
                sets: Vec::new(),
                rules: Vec::new(),
                members: Vec::new(),
                other_children: Vec::new(),
            };
            assert!(matches!(
                classify_inventory(&sut, &expected_members, empty),
                BridgeGuardObservation::Absent { .. }
            ));
            assert!(matches!(
                classify_inventory(&sut, &expected_members, exact_inventory(&sut)),
                BridgeGuardObservation::Exact { .. }
            ));

            let mut conflicts = Vec::new();
            let mut wrong_family = exact_inventory(&sut);
            wrong_family.tables[0].family = BridgeGuardObservedFamily::Ipv4;
            conflicts.push(wrong_family);
            let mut regular_chain = exact_inventory(&sut);
            regular_chain.chains.push(BridgeGuardChainOccurrence {
                table: regular_chain.tables[0].clone(),
                name: "foreign".to_owned(),
                handle: Some(91),
                definition: BridgeGuardChainDefinition::Regular,
            });
            conflicts.push(regular_chain);
            let mut reordered = exact_inventory(&sut);
            reordered.rules.swap(0, 1);
            conflicts.push(reordered);
            let mut duplicate = exact_inventory(&sut);
            duplicate.rules.push(duplicate.rules[0].clone());
            conflicts.push(duplicate);
            let mut unknown = exact_inventory(&sut);
            unknown.rules[0]
                .fact
                .program
                .expressions
                .push(BridgeGuardRuleExpression::Unknown { name: "quota".to_owned() });
            conflicts.push(unknown);
            let mut foreign_member = exact_inventory(&sut);
            foreign_member.members[0].identity =
                BridgeGuardMemberIdentity::ForeignEncoding { encoded_len: 16 };
            conflicts.push(foreign_member);
            let mut foreign_child = exact_inventory(&sut);
            foreign_child.other_children.push(BridgeGuardOtherChildOccurrence {
                table: foreign_child.tables[0].clone(),
                kind: BridgeGuardOtherChildKind::Flowtable,
                name: Some("foreign".to_owned()),
                handle: Some(99),
            });
            conflicts.push(foreign_child);

            for inventory in conflicts {
                let expected = inventory.clone();
                assert_eq!(
                    classify_inventory(&sut, &expected_members, inventory),
                    BridgeGuardObservation::Conflict { inventory: expected }
                );
            }
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn specification_validation_covers_identifier_and_fixed_policy_boundaries() {
            let identifier_cases = [
                (
                    "",
                    BridgeGuardValidationError::EmptyIdentifier {
                        identifier: BridgeGuardIdentifier::Table,
                    },
                ),
                (
                    "bad\0table",
                    BridgeGuardValidationError::IdentifierContainsNul {
                        identifier: BridgeGuardIdentifier::Table,
                        index: 3,
                    },
                ),
            ];
            for (table, expected) in identifier_cases {
                let error = BridgeGuardSpec::new(
                    table.to_owned(),
                    "prerouting".to_owned(),
                    "managed_taps".to_owned(),
                    REQUIRED_PRIORITY,
                    REQUIRED_INTERCEPT_MARK,
                    REQUIRED_ACCEPTED_MARK,
                )
                .expect_err("invalid table identifier");
                assert_eq!(error, expected);
            }
            for length in [255, 256] {
                let table = "é".repeat(length / 2) + if length % 2 == 1 { "a" } else { "" };
                let result = BridgeGuardSpec::new(
                    table,
                    "prerouting".to_owned(),
                    "managed_taps".to_owned(),
                    REQUIRED_PRIORITY,
                    REQUIRED_INTERCEPT_MARK,
                    REQUIRED_ACCEPTED_MARK,
                );
                assert_eq!(result.is_ok(), length == 255, "UTF-8 byte length {length}");
            }
            assert!(matches!(
                BridgeGuardSpec::new(
                    "overdrive-mtls".to_owned(),
                    "prerouting".to_owned(),
                    "managed_taps".to_owned(),
                    -299,
                    REQUIRED_INTERCEPT_MARK,
                    REQUIRED_ACCEPTED_MARK,
                ),
                Err(BridgeGuardValidationError::PriorityMismatch { .. })
            ));
            assert!(matches!(
                BridgeGuardSpec::new(
                    "overdrive-mtls".to_owned(),
                    "prerouting".to_owned(),
                    "managed_taps".to_owned(),
                    REQUIRED_PRIORITY,
                    0,
                    REQUIRED_ACCEPTED_MARK,
                ),
                Err(BridgeGuardValidationError::InterceptMarkMismatch { .. })
            ));
            assert!(matches!(
                BridgeGuardSpec::new(
                    "overdrive-mtls".to_owned(),
                    "prerouting".to_owned(),
                    "managed_taps".to_owned(),
                    REQUIRED_PRIORITY,
                    REQUIRED_INTERCEPT_MARK,
                    0,
                ),
                Err(BridgeGuardValidationError::AcceptedMarkMismatch { .. })
            ));
        }

        /// CONTRACT_SHAPE: bounded-change.
        #[test]
        fn member_validation_rejects_without_entering_the_netlink_scaffold() {
            let sut = spec();
            for (tap, expected) in [
                ("", BridgeGuardValidationError::EmptyMember),
                ("bad\0tap", BridgeGuardValidationError::MemberContainsNul { index: 3 }),
                (
                    "0123456789abcdef",
                    BridgeGuardValidationError::MemberTooLong {
                        length: 16,
                        maximum: MAX_MEMBER_BYTES,
                    },
                ),
                (
                    "éééééééé",
                    BridgeGuardValidationError::MemberTooLong {
                        length: 16,
                        maximum: MAX_MEMBER_BYTES,
                    },
                ),
            ] {
                assert!(matches!(
                    insert_member(&sut, tap),
                    Err(BridgeGuardError::Validation(actual)) if actual == expected
                ));
                assert!(matches!(
                    delete_member(&sut, tap),
                    Err(BridgeGuardError::Validation(actual)) if actual == expected
                ));
            }
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn member_encoding_accepts_one_through_fifteen_utf8_bytes_and_never_truncates() {
            for tap in ["a", "ovd-tp-0002", "123456789012345", "ééééééé"] {
                let encoded = encode_member(tap).expect("valid member");
                assert_eq!(&encoded[..tap.len()], tap.as_bytes());
                assert!(encoded[tap.len()..].iter().all(|byte| *byte == 0));
            }
        }

        /// CONTRACT_SHAPE: bounded-change.
        #[test]
        fn observe_and_delete_preserve_order_duplicates_foreign_children_and_full_conflicts() {
            // This is a real-kernel adapter test; the source-local lane is
            // also runnable in isolation, so establish the exact production
            // guard through the adapter before taking its read-only receipt.
            if unsafe { libc::geteuid() } != 0 {
                eprintln!("SKIP bridge source observation: root required");
                return;
            }
            let sut = spec();
            let expected_members =
                BTreeSet::from(["ovd-tp-0002".to_owned(), "ovd-tp-0003".to_owned()]);
            let _ = converge_table(&sut).expect("converge source-local bridge table");
            let _ = converge_chain(&sut).expect("converge source-local bridge chain");
            let _ = converge_set(&sut).expect("converge source-local bridge set");
            let _ = converge_rules(&sut).expect("converge source-local bridge rules");
            for member in &expected_members {
                let _ = insert_member(&sut, member).expect("converge source-local bridge member");
            }
            let observation = observe(&sut, &expected_members).expect("read-only observation");
            let BridgeGuardObservation::Exact { inventory } = observation else {
                panic!("healthy production guard is exact");
            };
            let expected_rules = sut.expected_rule_facts();
            assert_eq!(
                inventory.rules.iter().map(|rule| &rule.fact).collect::<Vec<_>>(),
                expected_rules.iter().collect::<Vec<_>>()
            );
            assert_eq!(inventory.members.len(), expected_members.len());
            assert!(inventory.other_children.is_empty());
            assert!(matches!(
                delete_owned_guard(&sut, &expected_members).expect("typed aggregate deletion"),
                BridgeGuardDeleteOutcome::Deleted { .. }
            ));
            assert!(matches!(
                observe(&sut, &BTreeSet::new()).expect("post-delete observation"),
                BridgeGuardObservation::Absent { .. }
            ));
        }
    }
}

/// True iff a recv chunk carries an `NLMSG_DONE` terminator.
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The `tproxy` expression byte layout MUST match the kernel-accepted pin in
    /// `spike/findings-e.md` EXACTLY — the packet-corruption-critical golden
    /// bytes (a wrong flip silently mis-diverts, no compile error). Encoded, not
    /// re-derived (ADR-0085 D1; CLAUDE.md "implement to the design").
    #[test]
    fn tproxy_expr_matches_findings_e_pin() {
        // The verbatim 44-byte capture from `spike/findings-e.md`:
        //   2c 00 01 80                              NFTA_LIST_ELEM|NESTED len=44
        //     0b 00 01 00  74 70 72 6f 78 79 00 00   NFTA_EXPR_NAME "tproxy\0"
        //     1c 00 02 80                            NFTA_EXPR_DATA|NESTED len=28
        //       08 00 01 00  00 00 00 02             NFTA_TPROXY_FAMILY   = be32 2
        //       08 00 02 00  00 00 00 01             NFTA_TPROXY_REG_ADDR = be32 1
        //       08 00 03 00  00 00 00 02             NFTA_TPROXY_REG_PORT = be32 2
        #[rustfmt::skip]
        let pin: &[u8] = &[
            0x2c, 0x00, 0x01, 0x80,
            0x0b, 0x00, 0x01, 0x00, 0x74, 0x70, 0x72, 0x6f, 0x78, 0x79, 0x00, 0x00,
            0x1c, 0x00, 0x02, 0x80,
            0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02,
            0x08, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x02,
        ];
        assert_eq!(
            expr_tproxy_ipv4(NFT_REG_1, NFT_REG_2),
            pin,
            "the hand-rolled tproxy expression must match the findings-e.md kernel-accepted pin byte-for-byte",
        );
    }

    /// The structural `NFTA_RULE_HANDLE` recovery predicate: given a synthetic
    /// `GETRULE` reply (the exact wire shape the kernel emits — nlmsghdr +
    /// nfgenmsg + `NFTA_RULE_HANDLE` be64 + `NFTA_RULE_USERDATA`), the decode +
    /// predicates extract the right handle / classify per-workload vs exemption.
    #[test]
    fn getrule_reply_handle_recovery_and_sweep_classification() {
        // Build a reply carrying: the leg-S exemption (handle 2), one inbound
        // tproxy rule (handle 3), one output-divert rule (handle 8), and a
        // FOREIGN rule with no ovdmtls userdata (handle 99, must be ignored).
        let vip = Ipv4Addr::new(127, 0, 0, 5);
        let exemption = userdata_exemption();
        let inbound = userdata_inbound(vip, 18555, 36533);
        let divert = userdata_output_divert(vip, 18555);
        let reply = synth_getrule_reply(&[
            (2, &exemption),
            (3, &inbound),
            (8, &divert),
            (99, b"someone-elses-rule"),
        ]);

        let rules = parse_rules(&reply);
        assert_eq!(rules.len(), 4, "every NEWRULE message must decode to a RuleInfo");

        // Per-rule handle recovery: the exact tag → its kernel handle.
        assert_eq!(handle_for_userdata(&rules, &inbound), Some(3));
        assert_eq!(handle_for_userdata(&rules, &divert), Some(8));
        assert_eq!(
            handle_for_userdata(&rules, &userdata_inbound(vip, 18555, 40000)),
            None,
            "a DIFFERENT agent-port tag must NOT match the 36533 rule (exact-userdata recovery)",
        );

        // Port-blind sweep: every per-workload kind, NEVER the exemption or a
        // foreign rule.
        let mut swept = workload_rule_handles(&rules);
        swept.sort_unstable();
        assert_eq!(
            swept,
            vec![3, 8],
            "the sweep must collect the inbound (3) + output-divert (8) handles and NEVER the \
             exemption (2) or the foreign rule (99)",
        );

        // Exemption presence guard.
        assert!(has_exemption(&rules), "the leg-S exemption tag must be detected");
        let no_exemption = parse_rules(&synth_getrule_reply(&[(3, &inbound)]));
        assert!(
            !has_exemption(&no_exemption),
            "a chain without the exemption tag must read absent"
        );
    }

    /// The empty / infra-only chain is a sweep no-op and reports no exemption.
    #[test]
    fn empty_reply_is_a_sweep_noop() {
        let rules = parse_rules(&[]);
        assert!(rules.is_empty());
        assert!(workload_rule_handles(&rules).is_empty());
        assert!(!has_exemption(&rules));
    }

    /// Build a synthetic `GETRULE` dump reply from `(handle, userdata)` pairs —
    /// the exact wire shape the kernel emits (nlmsghdr + nfgenmsg + the two
    /// attributes) so [`parse_rules`] is exercised end-to-end on real bytes.
    fn synth_getrule_reply(rules: &[(u64, &[u8])]) -> Vec<u8> {
        // hint: N rule messages (~48 B each) + the NLMSG_DONE terminator.
        let mut reply = Vec::with_capacity(rules.len() * 64 + 20);
        for (handle, udata) in rules {
            let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
            attr(&mut payload, NFTA_RULE_HANDLE, &handle.to_be_bytes());
            attr(&mut payload, NFTA_RULE_USERDATA, udata);
            nlmsg(&mut reply, nft_msg_type(NFT_MSG_NEWRULE), 0, 0, &payload);
        }
        // A dump terminates with NLMSG_DONE.
        nlmsg(&mut reply, NLMSG_DONE, 0, 0, &0i32.to_ne_bytes());
        reply
    }

    // ---- Pure byte-builder golden characterisations -----------------------
    //
    // The composite rule-expression + message-payload builders assemble the
    // `e_*` encoders, the `attr`/`nested`/`nfgenmsg` primitives, and the
    // userdata tags. No prior test asserts their bytes, so a body-replacement
    // (`-> Vec<u8> with vec![…]`) or a nested-flag corruption (`| -> &`) in any
    // sub-builder survives. These goldens pin the EXACT wire bytes (the
    // Tier-3-real-divert-verified encoding, whose load-bearing `tproxy`
    // sub-expression is itself pinned to `spike/findings-e.md` in
    // `tproxy_expr_matches_findings_e_pin`), so any change to any sub-builder's
    // output is caught here byte-for-byte.

    /// Inbound prerouting rule expression list — full byte characterisation.
    /// Transitively covers `e_payload`, `e_cmp`/`e_cmp_eq`/`data_value`,
    /// `e_meta_load`, `tproxy_and_mark_and_accept` (→ `e_immediate_value`,
    /// `expr_tproxy_ipv4`, `e_meta_set`, `e_immediate_verdict`), `expr`,
    /// `nested`, `attr`, `attr_be32`, `cstr`, `pad4`.
    #[test]
    fn inbound_rule_exprs_wire_golden() {
        let got = inbound_tproxy_rule_exprs(
            Ipv4Addr::new(127, 0, 0, 5),
            18555,
            Ipv4Addr::LOCALHOST,
            36533,
            0x1234,
        );
        assert_eq!(
            got,
            mark_before_tproxy_golden(INBOUND_GOLDEN),
            "inbound tproxy rule expr bytes drifted"
        );
    }

    /// Egress prerouting rule expression list — covers `e_iifname_eq` (the
    /// `iifname` NUL-padded match) in addition to the shared tail.
    #[test]
    fn egress_rule_exprs_wire_golden() {
        let got = egress_tproxy_rule_exprs("veth0", Ipv4Addr::LOCALHOST, 36533, 0x1234);
        assert_eq!(
            got,
            mark_before_tproxy_golden(EGRESS_GOLDEN),
            "egress tproxy rule expr bytes drifted"
        );
    }

    /// Reorder only the six already-byte-pinned tail expressions from their
    /// historical `address, port, tproxy, mark, meta-set, accept` layout into
    /// the accepted fail-closed `mark, meta-set, address, port, tproxy, accept`
    /// layout. Every expression byte remains characterized by the captured
    /// golden below; this helper makes the intentional ordering change visible
    /// without regenerating any encoder output from the encoder under test.
    fn mark_before_tproxy_golden(previous: &[u8]) -> Vec<u8> {
        const IMMEDIATE_LEN: usize = 44;
        const TPROXY_LEN: usize = 44;
        const META_SET_LEN: usize = 36;
        const ACCEPT_LEN: usize = 48;
        const TAIL_LEN: usize = IMMEDIATE_LEN * 3 + TPROXY_LEN + META_SET_LEN + ACCEPT_LEN;

        let tail = previous.len() - TAIL_LEN;
        let address = tail..tail + IMMEDIATE_LEN;
        let port = address.end..address.end + IMMEDIATE_LEN;
        let tproxy = port.end..port.end + TPROXY_LEN;
        let mark = tproxy.end..tproxy.end + IMMEDIATE_LEN;
        let meta_set = mark.end..mark.end + META_SET_LEN;
        let accept = meta_set.end..meta_set.end + ACCEPT_LEN;

        let mut reordered = previous[..tail].to_vec();
        reordered.extend_from_slice(&previous[mark]);
        reordered.extend_from_slice(&previous[meta_set]);
        reordered.extend_from_slice(&previous[address]);
        reordered.extend_from_slice(&previous[port]);
        reordered.extend_from_slice(&previous[tproxy]);
        reordered.extend_from_slice(&previous[accept]);
        reordered
    }

    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "CONTRACT_SHAPE is an exact repository-mandated machine-read declaration"
    )]
    #[test]
    fn d7_exact_rule_hit_witness_is_loss_and_mutation_conservative() {
        let program = egress_tproxy_rule_exprs("veth0", Ipv4Addr::LOCALHOST, 36533, 0x1234);
        let counter = b"counter\0";
        let counter_offsets = program
            .windows(counter.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == counter).then_some(offset))
            .collect::<Vec<_>>();
        let tproxy_offset = program
            .windows(b"tproxy\0".len())
            .position(|window| window == b"tproxy\0")
            .expect("the production TPROXY expression is present");
        let mark_set = e_meta_set(NFT_META_MARK, NFT_REG_3);
        let mark_offset = program
            .windows(mark_set.len())
            .position(|window| window == mark_set)
            .expect("the production meta-mark expression is present");

        assert_eq!(
            counter_offsets.len(),
            1,
            "D7 requires exactly one anonymous production counter"
        );
        assert!(
            counter_offsets[0] < tproxy_offset,
            "the D7 counter is nonterminal and precedes the redirect tail"
        );
        assert!(
            mark_offset < tproxy_offset,
            "the existing mark must execute before TPROXY so a dead listener stays on the local policy route"
        );
    }

    proptest! {
        /// CONTRACT_SHAPE: pure-function.
        #[allow(
            clippy::doc_markdown,
            reason = "CONTRACT_SHAPE is an exact repository-mandated machine-read declaration"
        )]
        #[test]
        fn every_d7_decoder_and_oracle_error_fails_closed(
            packets in any::<u64>(),
            bytes in any::<u64>(),
            invalid_sender_pid in 1_u32..=u32::MAX,
        ) {
        fn dumped_counter_program(packets: u64, bytes: u64) -> Vec<u8> {
            let production = egress_tproxy_rule_exprs("veth0", Ipv4Addr::LOCALHOST, 36_533, 0x1234);
            let placeholder = e_anonymous_counter();
            let offset = production
                .windows(placeholder.len())
                .position(|window| window == placeholder)
                .expect("production counter placeholder");
            let mut data = Vec::new();
            attr(&mut data, NFTA_COUNTER_BYTES, &bytes.to_be_bytes());
            attr(&mut data, NFTA_COUNTER_PACKETS, &packets.to_be_bytes());
            let sampled = expr("counter", &data);
            let mut dump = Vec::new();
            dump.extend_from_slice(&production[..offset]);
            dump.extend(sampled);
            dump.extend_from_slice(&production[offset + placeholder.len()..]);
            dump
        }

        fn rule_dump(sequence: u32, program: &[u8]) -> Vec<u8> {
            let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
            attr(&mut payload, NFTA_RULE_TABLE, &cstr("overdrive-mtls"));
            attr(&mut payload, NFTA_RULE_CHAIN, &cstr("prerouting"));
            attr(&mut payload, NFTA_RULE_HANDLE, &17_u64.to_be_bytes());
            attr(&mut payload, NFTA_RULE_EXPRESSIONS | NLA_F_NESTED, program);
            attr(&mut payload, NFTA_RULE_USERDATA, b"owned");
            let mut dump = Vec::new();
            nlmsg(
                &mut dump,
                nft_msg_type(NFT_MSG_NEWRULE),
                NLM_F_MULTI,
                sequence,
                &payload,
            );
            nlmsg(&mut dump, NLMSG_DONE, 0, sequence, &0_i32.to_ne_bytes());
            dump
        }

        let program = dumped_counter_program(packets, bytes);
        let valid = rule_dump(7, &program);
        let mut state = RuleDumpState::default();
        decode_rule_dump_datagram(&valid, 7, "overdrive-mtls", "prerouting", &mut state)
            .expect("strict valid dump");
        assert!(state.done);
        assert_eq!(state.rules.len(), 1);
        prop_assert_eq!(state.rules[0].counter, Some(RuleCounterSnapshot { packets, bytes }));
        assert_eq!(
            state.rules[0].normalized_program,
            egress_tproxy_rule_exprs("veth0", Ipv4Addr::LOCALHOST, 36_533, 0x1234)
        );

        let mut corruptions = Vec::new();
        let mut wrong_sequence = valid.clone();
        wrong_sequence[8..12].copy_from_slice(&6_u32.to_ne_bytes());
        corruptions.push(wrong_sequence);
        let mut wrong_family = valid.clone();
        wrong_family[16] = AF_UNSPEC;
        corruptions.push(wrong_family);
        let mut interrupted = valid.clone();
        interrupted[6..8]
            .copy_from_slice(&(NLM_F_MULTI | NLM_F_DUMP_INTR).to_ne_bytes());
        corruptions.push(interrupted);
        let mut missing_multipart = valid.clone();
        missing_multipart[6..8].copy_from_slice(&0_u16.to_ne_bytes());
        corruptions.push(missing_multipart);
        let mut truncated = valid.clone();
        truncated.pop();
        corruptions.push(truncated);
        let mut extra_after_done = valid.clone();
        nlmsg(&mut extra_after_done, NLMSG_DONE, 0, 7, &0_i32.to_ne_bytes());
        corruptions.push(extra_after_done);
        let first_length = usize::try_from(ne_u32(&valid, 0).expect("first message length"))
            .expect("message length fits usize");
        corruptions.push(valid[..first_length].to_vec());
        let mut wrong_type = valid.clone();
        wrong_type[4..6].copy_from_slice(&nft_msg_type(NFT_MSG_NEWCHAIN).to_ne_bytes());
        corruptions.push(wrong_type);
        let mut malformed_attr = valid.clone();
        malformed_attr[20..22].copy_from_slice(&3_u16.to_ne_bytes());
        corruptions.push(malformed_attr);
        let mut error_done = valid.clone();
        let status = error_done.len() - 4;
        error_done[status..].copy_from_slice(&1_i32.to_ne_bytes());
        corruptions.push(error_done);

        for corrupt in corruptions {
            let mut rejected = RuleDumpState::default();
            let result = decode_rule_dump_datagram(
                &corrupt,
                7,
                "overdrive-mtls",
                "prerouting",
                &mut rejected,
            );
            prop_assert!(
                result.is_err() || !rejected.done,
                "every framing, family, sequence, interruption, and completion mutation fails"
            );
        }

        let missing_counter_values =
            rule_dump(7, &egress_tproxy_rule_exprs("veth0", Ipv4Addr::LOCALHOST, 36_533, 0x1234));
        assert!(
            decode_rule_dump_datagram(
                &missing_counter_values,
                7,
                "overdrive-mtls",
                "prerouting",
                &mut RuleDumpState::default(),
            )
            .is_err(),
            "a partial counter is never normalized into a valid witness"
        );

        let sampled_counter = dumped_counter_program(packets, bytes);
        let mut duplicate_counter = sampled_counter.clone();
        duplicate_counter.extend_from_slice(&sampled_counter);
        prop_assert!(
            decode_rule_dump_datagram(
                &rule_dump(7, &duplicate_counter),
                7,
                "overdrive-mtls",
                "prerouting",
                &mut RuleDumpState::default(),
            )
            .is_err(),
            "duplicate sampled counters fail closed",
        );

        let mut unknown_expression = sampled_counter;
        unknown_expression.extend(expr("unknown", &[]));
        prop_assert!(
            decode_rule_dump_datagram(
                &rule_dump(7, &unknown_expression),
                7,
                "overdrive-mtls",
                "prerouting",
                &mut RuleDumpState::default(),
            )
            .is_err(),
            "unknown expressions fail closed",
        );

        let mut generation_payload = nfgenmsg(AF_UNSPEC, 0);
        attr(&mut generation_payload, NFTA_GEN_ID, &9_u32.to_be_bytes());
        let mut generation = Vec::new();
        nlmsg(&mut generation, nft_msg_type(NFT_MSG_NEWGEN), 0, 11, &generation_payload);
        prop_assert_eq!(decode_generation_datagram(&generation, 11).expect("valid GETGEN"), 9);
        let mut zero_generation = generation;
        let generation_value = zero_generation.len() - 4;
        zero_generation[generation_value..].copy_from_slice(&0_u32.to_be_bytes());
        prop_assert!(decode_generation_datagram(&zero_generation, 11).is_err());

        let mut sender: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        sender.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        sender.nl_pid = invalid_sender_pid;
        prop_assert!(
            validate_netfilter_sender(
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
                &sender,
            )
            .is_err(),
            "every non-kernel sender PID fails closed",
        );
        prop_assert!(
            classify_notification_receive(Err(Error::from_raw_os_error(libc::ENOBUFS))).is_err(),
            "notification loss fails closed",
        );
        prop_assert!(
            classify_notification_receive(Ok(1)).is_err(),
            "every queued notification fails closed",
        );
        prop_assert!(
            classify_notification_receive(Err(Error::from(ErrorKind::WouldBlock))).is_ok(),
            "only an empty notification queue is accepted",
        );
        }
    }

    /// REV-5 output-divert rule expression list — covers the `meta mark != …`
    /// NEQ comparison (`e_cmp` with `NFT_CMP_NEQ`) and the no-`tproxy` shape.
    #[test]
    fn output_divert_rule_exprs_wire_golden() {
        let got = output_divert_rule_exprs(Ipv4Addr::new(127, 0, 0, 5), 18555, 0x5678, 0x1234);
        assert_eq!(got, OUTPUT_DIVERT_GOLDEN, "output-divert rule expr bytes drifted");
    }

    /// Shared leg-S `meta mark <mark> accept` exemption expression list.
    #[test]
    fn mark_accept_exemption_exprs_wire_golden() {
        let got = mark_accept_exemption_exprs(0x5678);
        assert_eq!(got, MARK_ACCEPT_GOLDEN, "mark-accept exemption expr bytes drifted");
    }

    /// `NEWTABLE` payload — covers `newtable_payload`, `nfgenmsg`, `attr`, `cstr`.
    #[test]
    fn newtable_payload_wire_golden() {
        assert_eq!(newtable_payload("ovd"), NEWTABLE_GOLDEN);
    }

    /// `NEWRULE` payload — covers `newrule_payload` (incl. the
    /// `if !userdata.is_empty()` guard: a non-empty userdata tag is appended)
    /// and the `NFTA_RULE_EXPRESSIONS | NLA_F_NESTED` flag.
    #[test]
    fn newrule_payload_wire_golden() {
        let got = newrule_payload("ovd", "c", &[0xDE, 0xAD, 0xBE, 0xEF], &[0x01, 0x02]);
        assert_eq!(got, NEWRULE_GOLDEN);
    }

    /// `NEWCHAIN` payload — covers `newchain_payload`, the
    /// `NFTA_CHAIN_HOOK | NLA_F_NESTED` flag, `ChainKind::as_str` ("filter"),
    /// and the `PRIORITY_MANGLE` (-150) big-endian priority bytes.
    #[test]
    fn newchain_payload_wire_golden() {
        let got = newchain_payload(
            "ovd",
            "c",
            BaseChainSpec {
                hooknum: NF_INET_PRE_ROUTING,
                priority: PRIORITY_MANGLE,
                kind: ChainKind::Filter,
            },
        );
        assert_eq!(got, NEWCHAIN_GOLDEN);
    }

    /// `DELRULE`-by-handle payload — covers `delrule_payload` and the be64 handle.
    #[test]
    fn delrule_payload_wire_golden() {
        assert_eq!(delrule_payload("ovd", "c", 0x1122_3344_5566_7788), DELRULE_GOLDEN);
    }

    /// CONTRACT_SHAPE: pure-function.
    #[allow(clippy::doc_markdown)]
    #[test]
    fn atomic_rule_batch_frames_every_mutation_inside_one_kernel_transaction() {
        let exprs = [0xDE, 0xAD, 0xBE, 0xEF];
        let mutations = [
            AtomicRuleMutation::Delete { table: "ovd", chain: "c", handle: 7 },
            AtomicRuleMutation::Insert {
                table: "ovd",
                chain: "c",
                exprs: &exprs,
                userdata: b"owned",
            },
        ];
        let batch = atomic_rule_batch(&mutations);
        let mut types = Vec::new();
        let mut flags = Vec::new();
        let mut sequences = Vec::new();
        let mut off = 0usize;
        while off + 16 <= batch.len() {
            let length = ne_u32(&batch, off).expect("message length") as usize;
            types.push(ne_u16(&batch, off + 4).expect("message type"));
            flags.push(ne_u16(&batch, off + 6).expect("message flags"));
            sequences.push(ne_u32(&batch, off + 8).expect("message sequence"));
            off += (length + 3) & !3;
        }
        assert_eq!(
            types,
            [
                NFNL_MSG_BATCH_BEGIN,
                nft_msg_type(NFT_MSG_DELRULE),
                nft_msg_type(NFT_MSG_NEWRULE),
                NFNL_MSG_BATCH_END,
            ]
        );
        assert_eq!(sequences, [1, 2, 3, 4]);
        assert_eq!(flags[1] & NLM_F_ACK, NLM_F_ACK);
        assert_eq!(flags[2] & (NLM_F_ACK | NLM_F_CREATE), NLM_F_ACK | NLM_F_CREATE);
    }

    /// CONTRACT_SHAPE: pure-function.
    #[allow(clippy::doc_markdown)]
    #[test]
    fn atomic_rule_ack_walk_rejects_a_late_nack_after_an_earlier_ack() {
        fn ack(reply: &mut Vec<u8>, sequence: u32, code: i32) {
            let mut payload = Vec::with_capacity(20);
            payload.extend_from_slice(&code.to_ne_bytes());
            payload.extend_from_slice(&[0u8; 16]);
            nlmsg(reply, NLMSG_ERROR, 0, sequence, &payload);
        }

        let mut reply = Vec::new();
        ack(&mut reply, 2, 0);
        ack(&mut reply, 3, -libc::ENOENT);
        let mut pending = [2, 3].into_iter().collect::<BTreeSet<_>>();
        let error = collect_atomic_rule_acks(&reply, &mut pending)
            .expect_err("a later failed mutation rejects the complete transaction");
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
        assert_eq!(pending, std::iter::once(3).collect());
    }

    /// `GET{RULE,CHAIN}` request payload — covers `get_by_table_chain`.
    #[test]
    fn get_by_table_chain_wire_golden() {
        let got = get_by_table_chain("ovd", "c", NFTA_RULE_CHAIN, NFTA_RULE_TABLE);
        assert_eq!(got, GET_BY_TABLE_CHAIN_GOLDEN);
    }

    /// `userdata_egress` tag — `ovdmtls` magic + `KIND_EGRESS` (0x03) +
    /// `agent_port` (be16) + `host_veth` bytes. Hand-computed (independent of
    /// the builder) so it is an oracle, not a snapshot.
    #[test]
    fn userdata_egress_tag_layout() {
        // "ovdmtls" + 0x03 + 0x1234 (be16) + "veth0"
        let expected = b"ovdmtls\x03\x12\x34veth0".to_vec();
        assert_eq!(userdata_egress("veth0", 0x1234), expected);
    }

    // ---- Small pure predicates / helpers ---------------------------------

    /// `ChainKind::as_str` is the SSOT for the base-chain `type` string.
    #[test]
    fn chain_kind_as_str_is_filter_or_route() {
        assert_eq!(ChainKind::Filter.as_str(), "filter");
        assert_eq!(ChainKind::Route.as_str(), "route");
    }

    /// `PRIORITY_MANGLE` is -150 (the `mangle` priority where TPROXY / route
    /// re-eval must live). Pins the sign so `-150 -> 150` is caught.
    #[test]
    fn priority_mangle_is_negative_150() {
        assert_eq!(PRIORITY_MANGLE, -150);
    }

    /// `nft_msg_type` packs the nftables subsys into the high byte OR'd with the
    /// op. Pins the composition so `<< -> >>` / `| -> &` / body-replacement die.
    #[test]
    fn nft_msg_type_packs_subsys_and_op() {
        assert_eq!(nft_msg_type(NFT_MSG_NEWRULE), (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_NEWRULE);
        assert_eq!(nft_msg_type(NFT_MSG_NEWRULE), 0x0A06);
        assert_eq!(nft_msg_type(NFT_MSG_GETRULE), 0x0A07);
    }

    /// `is_ours` requires BOTH the `ovdmtls` magic prefix AND at least a kind
    /// byte after it — a foreign tag long enough to have a byte at the kind
    /// offset must still read `false` (the `&&` conjunction is load-bearing).
    #[test]
    fn is_ours_requires_magic_prefix_and_a_kind_byte() {
        assert!(is_ours(b"ovdmtls\x01"), "magic + a kind byte is ours");
        assert!(!is_ours(b"ovdmtls"), "magic alone (no kind byte) is NOT ours (len == magic)");
        assert!(
            !is_ours(b"deadbeef"),
            "a long foreign tag with no magic is NOT ours (kills && -> ||)"
        );
        assert!(!is_ours(b"ovd"), "too short to carry the magic");
    }

    /// A 16-char `iifname` must be truncated to 15 chars + a NUL terminator
    /// (`IFNAMSIZ - 1`), matching the kernel's NUL-padded 16-byte `meta iifname`
    /// load. Kills the `IFNAMSIZ - 1 -> + 1 / / 1` off-by-one.
    #[test]
    fn iifname_16char_name_truncates_to_keep_a_nul_terminator() {
        // A 16-char name and its 15-char prefix must encode identically, because
        // the 16th char is dropped to keep byte [15] NUL.
        let full16 = e_iifname_eq("0123456789abcdef");
        let trunc15 = e_iifname_eq("0123456789abcde");
        assert_eq!(full16, trunc15, "a 16-char iifname must truncate to 15 chars + NUL");
    }

    // ---- GETRULE decode bounds / walk safety ------------------------------

    /// `parse_rules` skips a leading non-`NEWRULE` message, decodes multiple
    /// rules, and honours the `off + mlen > reply.len()` bound when the last
    /// rule ends EXACTLY at the buffer end (no trailing `NLMSG_DONE`). Kills the
    /// `mlen < 16` and `off + mlen > len` comparison-operator mutants.
    #[test]
    fn parse_rules_walks_multiple_messages_and_honours_length_bounds() {
        let vip = Ipv4Addr::new(127, 0, 0, 5);
        let inbound = userdata_inbound(vip, 18555, 36533);
        let divert = userdata_output_divert(vip, 18555);
        let mut reply = Vec::new();
        // A leading 16-byte non-NEWRULE message (NLMSG_DONE, empty payload) that
        // must be skipped — if the `mlen < 16` guard flips to `<=`/`==`, the walk
        // breaks here and both rules are lost.
        nlmsg(&mut reply, NLMSG_DONE, 0, 0, &[]);
        // Two rules, the LAST ending exactly at reply.len() (no DONE after it):
        // if `off + mlen > len` flips to `>=`/`==`, the last rule is skipped.
        for (h, u) in [(3u64, &inbound), (8u64, &divert)] {
            let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
            attr(&mut payload, NFTA_RULE_HANDLE, &h.to_be_bytes());
            attr(&mut payload, NFTA_RULE_USERDATA, u);
            nlmsg(&mut reply, nft_msg_type(NFT_MSG_NEWRULE), 0, 0, &payload);
        }
        let handles: Vec<u64> = parse_rules(&reply).into_iter().map(|r| r.handle).collect();
        assert_eq!(handles, vec![3, 8], "both rules must decode, leading non-rule skipped");
    }

    /// `parse_rules` must break (never slice past the buffer) on a message whose
    /// declared length exceeds the bytes present. Kills the `mlen < 16 || off +
    /// mlen > reply.len()` `|| -> &&` mutant (which would proceed to a panicking
    /// out-of-bounds body slice).
    #[test]
    fn parse_rules_breaks_on_a_truncated_trailing_message_without_panic() {
        let inbound = userdata_inbound(Ipv4Addr::new(127, 0, 0, 5), 18555, 36533);
        let mut reply = synth_getrule_reply(&[(3, &inbound)]);
        // Append a NEWRULE header lying that it is 200 bytes; only 16 are present.
        reply.extend_from_slice(&200u32.to_ne_bytes());
        reply.extend_from_slice(&nft_msg_type(NFT_MSG_NEWRULE).to_ne_bytes());
        reply.extend_from_slice(&0u16.to_ne_bytes());
        reply.extend_from_slice(&[0u8; 8]);
        let rules = parse_rules(&reply);
        assert_eq!(
            rules.len(),
            1,
            "the one valid rule is recovered; the truncated tail is skipped"
        );
        assert_eq!(rules[0].handle, 3);
    }

    /// `for_each_attr` must (a) skip a valid empty (`alen == 4`) attribute and
    /// still reach a later one, and (b) break on an attribute whose length runs
    /// past the body. Kills the `alen < 4` `< -> <=/==` and `off + alen >
    /// body.len()` `|| -> &&` mutants (the latter would panic on an OOB slice).
    #[test]
    fn for_each_attr_handles_empty_and_truncated_attributes() {
        let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
        // (a) a valid empty attr (alen = 4, no payload) BEFORE the handle: if
        // `alen < 4` flips to `<=`/`==`, the walk breaks here and the handle is
        // never seen.
        payload.extend_from_slice(&4u16.to_ne_bytes()); // alen = 4
        payload.extend_from_slice(&99u16.to_ne_bytes()); // arbitrary type, no payload
        attr(&mut payload, NFTA_RULE_HANDLE, &7u64.to_be_bytes());
        // (b) a truncated attr claiming 200 bytes with none present: the bounds
        // guard must break rather than slice past the body.
        payload.extend_from_slice(&200u16.to_ne_bytes());
        payload.extend_from_slice(&NFTA_RULE_USERDATA.to_ne_bytes());
        let mut reply = Vec::new();
        nlmsg(&mut reply, nft_msg_type(NFT_MSG_NEWRULE), 0, 0, &payload);
        let rules = parse_rules(&reply);
        assert_eq!(rules.len(), 1, "the handle must be recovered across the empty attr");
        assert_eq!(rules[0].handle, 7);
        assert!(rules[0].userdata.is_empty(), "the truncated userdata attr must be skipped");
    }

    // ---- Golden byte vectors (captured from the Tier-3-verified encoding) --

    const INBOUND_GOLDEN: &[u8] = &[
        52, 0, 1, 128, 12, 0, 1, 0, 112, 97, 121, 108, 111, 97, 100, 0, 36, 0, 2, 128, 8, 0, 1, 0,
        0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 1, 8, 0, 3, 0, 0, 0, 0, 16, 8, 0, 4, 0, 0, 0, 0, 4, 44, 0,
        1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0,
        0, 0, 0, 12, 0, 3, 128, 8, 0, 1, 0, 127, 0, 0, 5, 36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116,
        97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 16, 44, 0, 1,
        128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0,
        0, 0, 12, 0, 3, 128, 5, 0, 1, 0, 6, 0, 0, 0, 52, 0, 1, 128, 12, 0, 1, 0, 112, 97, 121, 108,
        111, 97, 100, 0, 36, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 2, 8, 0, 3, 0,
        0, 0, 0, 2, 8, 0, 4, 0, 0, 0, 0, 2, 44, 0, 1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2,
        128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 0, 12, 0, 3, 128, 6, 0, 1, 0, 72, 123, 0,
        0, 44, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0, 24, 0,
        2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 12, 0, 2, 128, 8, 0, 1, 0, 127, 0, 0, 1, 44, 0, 1, 128, 14,
        0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0, 24, 0, 2, 128, 8, 0, 1, 0, 0,
        0, 0, 2, 12, 0, 2, 128, 6, 0, 1, 0, 142, 181, 0, 0, 44, 0, 1, 128, 11, 0, 1, 0, 116, 112,
        114, 111, 120, 121, 0, 0, 28, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 2, 8, 0, 2, 0, 0, 0, 0, 1, 8,
        0, 3, 0, 0, 0, 0, 2, 44, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116,
        101, 0, 0, 0, 24, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 3, 12, 0, 2, 128, 8, 0, 1, 0, 52, 18, 0,
        0, 36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116, 97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 2, 0, 0,
        0, 0, 3, 8, 0, 3, 0, 0, 0, 0, 3, 48, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105,
        97, 116, 101, 0, 0, 0, 28, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 0, 16, 0, 2, 128, 12, 0, 2, 128,
        8, 0, 1, 0, 0, 0, 0, 1,
    ];

    const EGRESS_GOLDEN: &[u8] = &[
        36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116, 97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 1, 0, 0, 0,
        0, 1, 8, 0, 2, 0, 0, 0, 0, 6, 56, 0, 1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 44, 0, 2, 128, 8,
        0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 0, 24, 0, 3, 128, 20, 0, 1, 0, 118, 101, 116,
        104, 48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116, 97, 0,
        0, 0, 0, 20, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 16, 44, 0, 1, 128, 8,
        0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 0,
        12, 0, 3, 128, 5, 0, 1, 0, 6, 0, 0, 0, 20, 0, 1, 128, 12, 0, 1, 0, 99, 111, 117, 110, 116,
        101, 114, 0, 4, 0, 2, 128, 44, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97,
        116, 101, 0, 0, 0, 24, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 12, 0, 2, 128, 8, 0, 1, 0, 127,
        0, 0, 1, 44, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0,
        24, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 2, 12, 0, 2, 128, 6, 0, 1, 0, 142, 181, 0, 0, 44, 0, 1,
        128, 11, 0, 1, 0, 116, 112, 114, 111, 120, 121, 0, 0, 28, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0,
        2, 8, 0, 2, 0, 0, 0, 0, 1, 8, 0, 3, 0, 0, 0, 0, 2, 44, 0, 1, 128, 14, 0, 1, 0, 105, 109,
        109, 101, 100, 105, 97, 116, 101, 0, 0, 0, 24, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 3, 12, 0, 2,
        128, 8, 0, 1, 0, 52, 18, 0, 0, 36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116, 97, 0, 0, 0, 0,
        20, 0, 2, 128, 8, 0, 2, 0, 0, 0, 0, 3, 8, 0, 3, 0, 0, 0, 0, 3, 48, 0, 1, 128, 14, 0, 1, 0,
        105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0, 28, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 0,
        16, 0, 2, 128, 12, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1,
    ];

    const OUTPUT_DIVERT_GOLDEN: &[u8] = &[
        52, 0, 1, 128, 12, 0, 1, 0, 112, 97, 121, 108, 111, 97, 100, 0, 36, 0, 2, 128, 8, 0, 1, 0,
        0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 1, 8, 0, 3, 0, 0, 0, 0, 16, 8, 0, 4, 0, 0, 0, 0, 4, 44, 0,
        1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0,
        0, 0, 0, 12, 0, 3, 128, 8, 0, 1, 0, 127, 0, 0, 5, 36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116,
        97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 16, 44, 0, 1,
        128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0,
        0, 0, 12, 0, 3, 128, 5, 0, 1, 0, 6, 0, 0, 0, 52, 0, 1, 128, 12, 0, 1, 0, 112, 97, 121, 108,
        111, 97, 100, 0, 36, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 2, 8, 0, 3, 0,
        0, 0, 0, 2, 8, 0, 4, 0, 0, 0, 0, 2, 44, 0, 1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2,
        128, 8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 0, 12, 0, 3, 128, 6, 0, 1, 0, 72, 123, 0,
        0, 36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116, 97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 1, 0, 0,
        0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 3, 44, 0, 1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128,
        8, 0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 1, 12, 0, 3, 128, 8, 0, 1, 0, 120, 86, 0, 0,
        44, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0, 24, 0, 2,
        128, 8, 0, 1, 0, 0, 0, 0, 2, 12, 0, 2, 128, 8, 0, 1, 0, 52, 18, 0, 0, 36, 0, 1, 128, 9, 0,
        1, 0, 109, 101, 116, 97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 2, 0, 0, 0, 0, 3, 8, 0, 3, 0, 0,
        0, 0, 2, 48, 0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0,
        28, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 0, 16, 0, 2, 128, 12, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0,
        1,
    ];

    const MARK_ACCEPT_GOLDEN: &[u8] = &[
        36, 0, 1, 128, 9, 0, 1, 0, 109, 101, 116, 97, 0, 0, 0, 0, 20, 0, 2, 128, 8, 0, 1, 0, 0, 0,
        0, 1, 8, 0, 2, 0, 0, 0, 0, 3, 44, 0, 1, 128, 8, 0, 1, 0, 99, 109, 112, 0, 32, 0, 2, 128, 8,
        0, 1, 0, 0, 0, 0, 1, 8, 0, 2, 0, 0, 0, 0, 0, 12, 0, 3, 128, 8, 0, 1, 0, 120, 86, 0, 0, 48,
        0, 1, 128, 14, 0, 1, 0, 105, 109, 109, 101, 100, 105, 97, 116, 101, 0, 0, 0, 28, 0, 2, 128,
        8, 0, 1, 0, 0, 0, 0, 0, 16, 0, 2, 128, 12, 0, 2, 128, 8, 0, 1, 0, 0, 0, 0, 1,
    ];

    const NEWTABLE_GOLDEN: &[u8] = &[2, 0, 0, 0, 8, 0, 1, 0, 111, 118, 100, 0];

    const NEWRULE_GOLDEN: &[u8] = &[
        2, 0, 0, 0, 8, 0, 1, 0, 111, 118, 100, 0, 6, 0, 2, 0, 99, 0, 0, 0, 8, 0, 4, 128, 222, 173,
        190, 239, 6, 0, 7, 0, 1, 2, 0, 0,
    ];

    const NEWCHAIN_GOLDEN: &[u8] = &[
        2, 0, 0, 0, 8, 0, 1, 0, 111, 118, 100, 0, 6, 0, 3, 0, 99, 0, 0, 0, 20, 0, 4, 128, 8, 0, 1,
        0, 0, 0, 0, 0, 8, 0, 2, 0, 255, 255, 255, 106, 11, 0, 7, 0, 102, 105, 108, 116, 101, 114,
        0, 0, 8, 0, 5, 0, 0, 0, 0, 1,
    ];

    const DELRULE_GOLDEN: &[u8] = &[
        2, 0, 0, 0, 8, 0, 1, 0, 111, 118, 100, 0, 6, 0, 2, 0, 99, 0, 0, 0, 12, 0, 3, 0, 17, 34, 51,
        68, 85, 102, 119, 136,
    ];

    const GET_BY_TABLE_CHAIN_GOLDEN: &[u8] =
        &[2, 0, 0, 0, 8, 0, 1, 0, 111, 118, 100, 0, 6, 0, 2, 0, 99, 0, 0, 0];
}
