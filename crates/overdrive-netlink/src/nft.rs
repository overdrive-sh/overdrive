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

use std::net::Ipv4Addr;

use crate::error::{NEG_EEXIST, NetlinkError};

// ---- NETLINK_NETFILTER / nfnetlink framing (pinned; spike increment-e) ------
const NETLINK_NETFILTER: i32 = 12;
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFNL_MSG_BATCH_BEGIN: u16 = 16;
const NFNL_MSG_BATCH_END: u16 = 17;

// nf_tables message ops (`enum nf_tables_msg_types`).
const NFT_MSG_NEWTABLE: u16 = 0;
const NFT_MSG_NEWCHAIN: u16 = 3;
const NFT_MSG_GETCHAIN: u16 = 4;
const NFT_MSG_NEWRULE: u16 = 6;
const NFT_MSG_GETRULE: u16 = 7;
const NFT_MSG_DELRULE: u16 = 8;

// netlink message flags.
const NLM_F_REQUEST: u16 = 0x001;
const NLM_F_ACK: u16 = 0x004;
const NLM_F_DUMP: u16 = 0x300; // NLM_F_ROOT | NLM_F_MATCH
const NLM_F_CREATE: u16 = 0x400;
const NLM_F_APPEND: u16 = 0x800;
// netlink message types.
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
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
// list + expr framing.
const NFTA_LIST_ELEM: u16 = 1;
const NFTA_EXPR_NAME: u16 = 1;
const NFTA_EXPR_DATA: u16 = 2;
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

/// One rule recovered from a `GETRULE` dump reply: its kernel-assigned handle
/// and its `NFTA_RULE_USERDATA` tag (empty when the rule carries none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleInfo {
    /// The kernel-assigned `NFTA_RULE_HANDLE` (the by-handle delete key).
    pub handle: u64,
    /// The rule's `NFTA_RULE_USERDATA` bytes (the structural identity tag).
    pub userdata: Vec<u8>,
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

/// Walk the top-level attributes of one message body (`body`), invoking `visit`
/// with `(type_without_nested_flag, payload)` for each. Bounds-checked; a
/// truncated attribute stops the walk (safe-slice, never a panic).
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
        if mtype == newrule {
            // Message body = nlmsghdr(16) + nfgenmsg(4) + attributes.
            let body = &reply[off + 20..off + mlen];
            let mut handle: Option<u64> = None;
            let mut udata: Vec<u8> = Vec::new();
            for_each_attr(body, |atype, payload| {
                if atype == NFTA_RULE_HANDLE && payload.len() >= 8 {
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&payload[..8]);
                    handle = Some(u64::from_be_bytes(bytes));
                } else if atype == NFTA_RULE_USERDATA {
                    udata = payload.to_vec();
                }
            });
            if let Some(handle) = handle {
                out.push(RuleInfo { handle, userdata: udata });
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
struct NfSock {
    fd: i32,
}

impl NfSock {
    fn open() -> std::io::Result<Self> {
        // SAFETY: `socket(2)` with valid domain/type/protocol constants; the
        // returned fd is checked and owned (closed in `Drop`).
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_NETFILTER) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
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
        this.set_recv_timeout();
        Ok(this)
    }

    fn set_recv_timeout(&self) {
        let tv = libc::timeval { tv_sec: 5, tv_usec: 0 };
        // SAFETY: `tv` is a valid initialised `timeval` of the size passed;
        // `self.fd` is an open netlink socket. A failure is non-fatal (the recv
        // would simply block); the result is intentionally unused.
        unsafe {
            libc::setsockopt(
                self.fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                std::ptr::addr_of!(tv).cast::<libc::c_void>(),
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }
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

/// Dump every rule in `ip <table> <chain>` as `(handle, userdata)` pairs — the
/// `GETRULE` structural read (ADR-0085 D10). Reads until `NLMSG_DONE`.
///
/// # Errors
///
/// [`NetlinkError::Nft`] (`op = "list-rules"`) on a socket / kernel failure.
pub fn list_rules(table: &str, chain: &str) -> Result<Vec<RuleInfo>, NetlinkError> {
    let sock = NfSock::open().map_err(|e| NetlinkError::nft("list-rules", e))?;
    let payload = get_by_table_chain(table, chain, NFTA_RULE_CHAIN, NFTA_RULE_TABLE);
    let mut msg = Vec::with_capacity((16 + payload.len()).next_multiple_of(4));
    nlmsg(&mut msg, nft_msg_type(NFT_MSG_GETRULE), NLM_F_REQUEST | NLM_F_DUMP, 1, &payload);
    sock.send(&msg).map_err(|e| NetlinkError::nft("list-rules", e))?;

    // A rule dump usually fits one 64 KiB recv; hint accordingly.
    let mut accumulated = Vec::with_capacity(65536);
    loop {
        let mut buf = vec![0u8; 65536];
        let n = sock.recv(&mut buf).map_err(|e| NetlinkError::nft("list-rules", e))?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        // A dump ends with NLMSG_DONE; a NLMSG_ERROR is a genuine failure.
        if let Some(err) = dump_error(chunk) {
            return Err(NetlinkError::nft("list-rules", err));
        }
        let done = chunk_contains_done(chunk);
        accumulated.extend_from_slice(chunk);
        if done {
            break;
        }
    }
    Ok(parse_rules(&accumulated))
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
fn chunk_contains_done(chunk: &[u8]) -> bool {
    let mut off = 0usize;
    while off + 16 <= chunk.len() {
        let Some(mlen) = ne_u32(chunk, off).map(|l| l as usize) else { break };
        let Some(mtype) = ne_u16(chunk, off + 4) else { break };
        if mlen < 16 || off + mlen > chunk.len() {
            break;
        }
        if mtype == NLMSG_DONE {
            return true;
        }
        off += (mlen + 3) & !3;
    }
    false
}

/// The first `NLMSG_ERROR` NACK (non-zero code) in a dump chunk, as an
/// `io::Error`, else `None`.
fn dump_error(chunk: &[u8]) -> Option<std::io::Error> {
    let mut off = 0usize;
    while off + 16 <= chunk.len() {
        let mlen = ne_u32(chunk, off).map(|l| l as usize)?;
        let mtype = ne_u16(chunk, off + 4)?;
        if mlen < 16 || off + mlen > chunk.len() {
            break;
        }
        if mtype == NLMSG_ERROR {
            let code = ne_u32(chunk, off + 16).map_or(0, |c| c as i32);
            if code != 0 {
                return Some(std::io::Error::from_raw_os_error(code.abs()));
            }
        }
        off += (mlen + 3) & !3;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
