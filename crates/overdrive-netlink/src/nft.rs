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

use std::collections::BTreeSet;
use std::io::{Error, ErrorKind};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use crate::error::{NEG_EEXIST, NetlinkError};

// ---- NETLINK_NETFILTER / nfnetlink framing (pinned; spike increment-e) ------
const NETLINK_NETFILTER: i32 = 12;
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFNL_MSG_BATCH_BEGIN: u16 = 16;
const NFNL_MSG_BATCH_END: u16 = 17;

// nf_tables message ops (`enum nf_tables_msg_types`).
const NFT_MSG_NEWTABLE: u16 = 0;
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
const AF_UNSPEC: u8 = 0;
const IPPROTO_TCP: u8 = 6;

// table attrs.
const NFTA_TABLE_NAME: u16 = 1;
// chain attrs.
const NFTA_CHAIN_TABLE: u16 = 1;
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

/// The shared `tproxy to <agent_ip>:<agent_port> meta mark set <set_mark>
/// accept` tail of both TPROXY rules: load the tproxy dst into reg1/reg2, the
/// `tproxy` verb, `meta mark set` from reg3, then `accept`.
fn tproxy_and_mark_and_accept(agent_ip: Ipv4Addr, agent_port: u16, set_mark: u32) -> Vec<u8> {
    // load tproxy dst: reg1 = agent_ip (network order), reg2 = agent_port (be16).
    let mut ex = e_immediate_value(NFT_REG_1, &agent_ip.octets());
    ex.extend(e_immediate_value(NFT_REG_2, &agent_port.to_be_bytes()));
    // tproxy verb: family ipv4, reg_addr=reg1, reg_port=reg2.
    ex.extend(expr_tproxy_ipv4(NFT_REG_1, NFT_REG_2));
    // meta mark set <set_mark>: reg3 = mark (HOST order — matched by the fwmark
    // ip rule) ; meta mark set sreg=reg3.
    ex.extend(e_immediate_value(NFT_REG_3, &set_mark.to_ne_bytes()));
    ex.extend(e_meta_set(NFT_META_MARK, NFT_REG_3));
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
fn newrule_payload(table: &str, chain: &str, exprs: &[u8], userdata: &[u8]) -> Vec<u8> {
    let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
    attr(&mut payload, NFTA_RULE_TABLE, &cstr(table));
    attr(&mut payload, NFTA_RULE_CHAIN, &cstr(chain));
    attr(&mut payload, NFTA_RULE_EXPRESSIONS | NLA_F_NESTED, exprs);
    if !userdata.is_empty() {
        attr(&mut payload, NFTA_RULE_USERDATA, userdata);
    }
    payload
}

/// A `NEWTABLE` payload (`nfgenmsg` + table name).
fn newtable_payload(table: &str) -> Vec<u8> {
    let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
    attr(&mut payload, NFTA_TABLE_NAME, &cstr(table));
    payload
}

/// A `NEWCHAIN` payload (`nfgenmsg` + table/name + hook{num,priority} + type + policy).
fn newchain_payload(table: &str, chain: &str, spec: BaseChainSpec) -> Vec<u8> {
    // two 8-byte be32 hook attrs.
    let mut hook = Vec::with_capacity(2 * 8);
    attr_be32(&mut hook, NFTA_HOOK_HOOKNUM, spec.hooknum);
    attr_be32(&mut hook, NFTA_HOOK_PRIORITY, spec.priority as u32);

    let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
    attr(&mut payload, NFTA_CHAIN_TABLE, &cstr(table));
    attr(&mut payload, NFTA_CHAIN_NAME, &cstr(chain));
    attr(&mut payload, NFTA_CHAIN_HOOK | NLA_F_NESTED, &hook);
    attr(&mut payload, NFTA_CHAIN_TYPE, &cstr(spec.kind.as_str()));
    attr_be32(&mut payload, NFTA_CHAIN_POLICY, NF_ACCEPT);
    payload
}

/// A `DELRULE`-by-handle payload (`nfgenmsg` + table/chain + handle be64).
fn delrule_payload(table: &str, chain: &str, handle: u64) -> Vec<u8> {
    let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
    attr(&mut payload, NFTA_RULE_TABLE, &cstr(table));
    attr(&mut payload, NFTA_RULE_CHAIN, &cstr(chain));
    attr(&mut payload, NFTA_RULE_HANDLE, &handle.to_be_bytes());
    payload
}

/// A `GET{RULE,CHAIN}` request payload keyed by table (+ chain).
fn get_by_table_chain(table: &str, chain: &str, chain_attr: u16, table_attr: u16) -> Vec<u8> {
    let mut payload = nfgenmsg(NFPROTO_IPV4, 0);
    attr(&mut payload, table_attr, &cstr(table));
    attr(&mut payload, chain_attr, &cstr(chain));
    payload
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

fn canonical_expression_data(name: &str, data: &[u8]) -> std::io::Result<Vec<u8>> {
    match name {
        "cmp" => canonical_attr_set(data, &[NFTA_CMP_DATA]),
        "immediate" => canonical_attr_set(data, &[NFTA_IMMEDIATE_DATA]),
        "meta" | "payload" | "tproxy" => canonical_attr_set(data, &[]),
        _ => Err(invalid_data(format!("unknown nft expression {name:?}"))),
    }
}

fn normalize_rule_program(
    expressions: &[u8],
    require_sampled_counter: bool,
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
            normalized.extend(expr(name, &canonical_expression_data(name, data)?));
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

fn decode_rule_message(payload: &[u8], table: &str, chain: &str) -> std::io::Result<RuleInfo> {
    if payload.len() < 4 {
        return Err(invalid_data("NEWRULE is missing nfgenmsg"));
    }
    if payload[0] != NFPROTO_IPV4 || payload[1] != 0 {
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
                program = Some(normalize_rule_program(value, true)?);
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

fn decode_rule_dump_datagram(
    datagram: &[u8],
    sequence: u32,
    table: &str,
    chain: &str,
    state: &mut RuleDumpState,
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
                state.rules.push(decode_rule_message(body, table, chain)?);
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
    let payload = get_by_table_chain(table, chain, NFTA_RULE_CHAIN, NFTA_RULE_TABLE);
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
        decode_rule_dump_datagram(&datagram[..received], sequence, table, chain, &mut state)?;
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
    let sock = NfSock::open().map_err(|e| NetlinkError::nft("list-rules", e))?;
    let sequence = 1;
    send_get_rules(&sock, table, chain, sequence)
        .map_err(|source| NetlinkError::nft("list-rules", source))?;
    receive_rule_dump(&sock, sequence, table, chain, Instant::now() + OBSERVATION_DEADLINE)
        .map_err(|source| NetlinkError::nft("list-rules", source))
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
        assert_eq!(got, INBOUND_GOLDEN, "inbound tproxy rule expr bytes drifted");
    }

    /// Egress prerouting rule expression list — covers `e_iifname_eq` (the
    /// `iifname` NUL-padded match) in addition to the shared tail.
    #[test]
    fn egress_rule_exprs_wire_golden() {
        let got = egress_tproxy_rule_exprs("veth0", Ipv4Addr::LOCALHOST, 36533, 0x1234);
        assert_eq!(got, EGRESS_GOLDEN, "egress tproxy rule expr bytes drifted");
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
            .expect("the unchanged production TPROXY expression is present");

        assert_eq!(
            counter_offsets.len(),
            1,
            "D7 requires exactly one anonymous production counter"
        );
        assert!(
            counter_offsets[0] < tproxy_offset,
            "the D7 counter is nonterminal and precedes the unchanged redirect tail"
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
