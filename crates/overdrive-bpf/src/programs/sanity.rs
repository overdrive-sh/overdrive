//! Shared sanity prologue helper for `xdp_service_map_lookup` and
//! `tc_reverse_nat`. Per Slice 06 / ADR-0040 Q3=C — single
//! `#[inline(always)]` helper, two call sites, one source of truth.
//!
//! # Five Cloudflare-order checks
//!
//! Performed in sequence; first failure short-circuits per
//! `docs/feature/phase-2-xdp-service-map/slices/slice-06-sanity-prologue.md`:
//!
//! 1. **EtherType** — must be IPv4 (`0x0800`). Non-IPv4 (IPv6, ARP,
//!    VLAN-tagged, …) returns `Verdict::PassToKernel` because the LB
//!    is not a firewall: edge protocols belong to the host's other
//!    workloads.
//! 2. **IP version + IHL** — version field must be 4, IHL must be ≥ 5
//!    (i.e. ≥ 20 bytes of IP header). Anything else means the frame
//!    is malformed; return `Verdict::Drop` and increment the
//!    `MalformedHeader` slot of `DROP_COUNTER`.
//! 3. **IP `total_length`** — the IPv4 header's `total_length` field
//!    must satisfy `IHL·4 ≤ total_length ≤ packet_length`. Anything
//!    outside that window is a malformed frame; drop + counter.
//! 4. **Protocol** — must be TCP (`6`) or UDP (`17`). Anything else
//!    (ICMP, GRE, ESP, …) returns `Verdict::PassToKernel` for the
//!    same "LB is not a firewall" reason as check 1.
//! 5. **TCP flags** — for TCP frames only, reject the canonical
//!    Cloudflare-flagged pathological flag combinations:
//!    `SYN+RST`, `SYN+FIN`, all-zero, and a few other classic
//!    nmap-shape sets. Any match drops + increments the counter.
//!    UDP frames pass this check trivially.
//!
//! # Cooperation contract with the call sites
//!
//! The helper takes:
//!
//! * the byte offsets at which the IPv4 and L4 headers START (these
//!   are different in XDP and TC because both use the same Ethernet +
//!   IPv4 + L4 layout but the verifier wants the offsets explicit at
//!   the call site);
//! * the total **packet length** (`data_end - data` for XDP;
//!   `len()` of the skb for TC), used in check 3 against the IPv4
//!   `total_length` field;
//! * a generic byte-reader closure (`read_u8` / `read_u16_be`),
//!   because `XdpContext` and `TcContext` are different types but
//!   the *shape* of the bounds-checked read is identical.
//!
//! # Why a sum type return, not `Result<(), Verdict>`
//!
//! Every code path the call site cares about is one of three
//! mutually-exclusive outcomes — `Continue`, `Drop` (XDP_DROP /
//! TC_ACT_SHOT), `PassToKernel` (XDP_PASS / TC_ACT_OK) — and the
//! caller has to translate the latter two into the program-specific
//! verdict constant anyway. A `Result<(), Verdict>` would conflate
//! "drop" and "pass" under a single `Err` variant; the sum-type
//! return makes the three-way distinction structural.
//!
//! # DROP_COUNTER attribution
//!
//! The helper increments `DROP_COUNTER[MalformedHeader]` on every
//! drop arm. The `MalformedHeader` slot is the right home per
//! `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! S-2.2-19 / S-2.2-20 — both the truncated-IPv4 and the SYN+RST
//! drops attribute to the same slot. The `SanityPrologue` slot is
//! reserved for future operator-tunable rules (POLICY_MAP / #158);
//! Slice 06's static checks are all "this header is structurally
//! malformed" and bucket together.

#![allow(dead_code)]

// `DropClass::MalformedHeader` discriminant. Mirrored from
// `crates/overdrive-core/src/dataplane/drop_class.rs`. `overdrive-bpf`
// is `#![no_std]` and cannot import `overdrive-core` directly; the
// const-assert in `drop_class.rs` is the structural drift gate, and
// the kernel-side `DROP_COUNTER` map declaration uses the same
// `SLOT_COUNT` constant.
const DROP_CLASS_MALFORMED_HEADER: u32 = 0;

// ---------- Verdict sum type ----------

/// Outcome of the sanity prologue. The call site translates the
/// `Drop` / `PassToKernel` variants into the program-specific
/// verdict constant (`XDP_DROP` / `TC_ACT_SHOT`, `XDP_PASS` /
/// `TC_ACT_OK` respectively).
///
/// `Continue` means "all five checks passed; proceed to the
/// caller's main lookup logic." This is a zero-payload variant
/// because the caller does not need any additional state from the
/// prologue — it has the headers already bounds-checked at the
/// offsets it knows.
#[derive(Clone, Copy)]
pub enum Verdict {
    /// All five checks passed. Caller proceeds to its main logic.
    Continue,
    /// Frame is structurally malformed. Caller returns its
    /// drop verdict (`XDP_DROP` or `TC_ACT_SHOT`).
    /// `DROP_COUNTER[MalformedHeader]` already incremented.
    Drop,
    /// Frame is well-formed but is not LB traffic (non-IPv4,
    /// non-TCP/UDP). Caller hands to the kernel stack
    /// (`XDP_PASS` or `TC_ACT_OK`). DROP_COUNTER untouched.
    PassToKernel,
}

// ---------- Header layout constants ----------
//
// Mirrors the constants in `xdp_service_map.rs` and
// `tc_reverse_nat.rs`. These are kernel ABI / wire ABI; never
// change.

const ETH_TYPE_OFFSET: usize = 12;
const ETH_TYPE_IPV4: u16 = 0x0800;

const IPV4_VER_IHL_OFFSET: usize = 0; // relative to IPv4 start
const IPV4_TOT_LEN_OFFSET: usize = 2; // relative to IPv4 start
const IPV4_PROTO_OFFSET: usize = 9; // relative to IPv4 start
const IPV4_PROTO_TCP: u8 = 6;
const IPV4_PROTO_UDP: u8 = 17;

const TCP_FLAGS_OFFSET: usize = 13; // relative to L4 start

// TCP flag bits (`linux/tcp.h`).
const TCP_FIN: u8 = 0x01;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;
const TCP_PSH: u8 = 0x08;
const TCP_ACK: u8 = 0x10;
const TCP_URG: u8 = 0x20;

/// Reject pathological flag sets per the Cloudflare nmap-shape list.
/// Returns `true` when the flags are structurally invalid.
///
/// Fully unrolled — no branches the verifier can complain about,
/// no per-bit loops. The complete decision boils down to a few
/// equality checks against bit masks.
#[inline(always)]
fn tcp_flags_pathological(flags: u8) -> bool {
    // Mask off ECN/CWR (bits 0x40 / 0x80) — those are legitimate
    // signalling bits, not flag-spec violations.
    let f = flags & 0x3F;

    // (a) All-zero flags: no SYN, FIN, RST, ACK — nmap NULL scan.
    if f == 0 {
        return true;
    }
    // (b) SYN + FIN — never a legitimate combination.
    if (f & (TCP_SYN | TCP_FIN)) == (TCP_SYN | TCP_FIN) {
        return true;
    }
    // (c) SYN + RST — never a legitimate combination.
    if (f & (TCP_SYN | TCP_RST)) == (TCP_SYN | TCP_RST) {
        return true;
    }
    // (d) FIN + RST — never a legitimate combination.
    if (f & (TCP_FIN | TCP_RST)) == (TCP_FIN | TCP_RST) {
        return true;
    }
    // (e) FIN + URG + PSH (nmap Xmas scan).
    if (f & (TCP_FIN | TCP_URG | TCP_PSH)) == (TCP_FIN | TCP_URG | TCP_PSH) {
        return true;
    }
    false
}

// ---------- DROP_COUNTER write ----------
//
// The DROP_COUNTER map is declared in
// `crates/overdrive-bpf/src/maps/drop_counter.rs`. Importing it via
// `crate::maps::drop_counter::DROP_COUNTER` is the canonical path —
// the helper here owns the increment but not the map declaration.

#[inline(always)]
fn record_malformed_header_drop() {
    use crate::maps::drop_counter::DROP_COUNTER;

    // SAFETY: `get_ptr_mut` returns a per-CPU pointer; the increment
    // is per-CPU-local (research § 7.1). Single-writer per CPU within
    // an XDP/TC program context — no re-entry, no tail-calls in this
    // path — so the unsynchronised `+=` is safe.
    if let Some(counter) = DROP_COUNTER.get_ptr_mut(DROP_CLASS_MALFORMED_HEADER) {
        unsafe { *counter = (*counter).wrapping_add(1) };
    }
}

// ---------- Sanity prologue (generic over context type) ----------

/// Run the 5-check Cloudflare-order sanity prologue against a packet.
///
/// All offsets are absolute — i.e. relative to the start of the
/// packet buffer (`ctx.data()`). The caller pre-bounds-checks each
/// header range via `ptr_at` (or equivalent) before calling, but the
/// helper itself does NOT call any bounds-check function — it reads
/// from `read_u8` / `read_u16_be` closures which the caller wires
/// against its own context type.
///
/// # Safety
///
/// `read_u8` and `read_u16_be` MUST already perform the verifier-
/// required bounds check internally. Both closures are expected to
/// return `Err(())` if the bounds-check fails; on `Err`, this
/// function returns `Verdict::PassToKernel` (a truncated frame the
/// kernel stack should handle, not an LB drop).
///
/// # Inlining
///
/// `#[inline(always)]` is mandatory: the verifier must see the
/// reads at the call site so it can prove the bounds checks at
/// each `?` are satisfied along every path. A non-inlined helper
/// would force the verifier to track pointer state across function
/// boundaries — typically rejected.
#[inline(always)]
pub fn sanity_check<R8, R16>(
    ipv4_offset: usize,
    l4_offset: usize,
    packet_len: usize,
    read_u8: R8,
    read_u16_be: R16,
) -> Verdict
where
    R8: Fn(usize) -> Result<u8, ()>,
    R16: Fn(usize) -> Result<u16, ()>,
{
    // (1) EtherType — must be IPv4. Non-IPv4 → PassToKernel.
    let eth_type = match read_u16_be(ETH_TYPE_OFFSET) {
        Ok(v) => v,
        Err(()) => return Verdict::PassToKernel,
    };
    if eth_type != ETH_TYPE_IPV4 {
        return Verdict::PassToKernel;
    }

    // (2) IP version + IHL. The first byte of the IPv4 header packs
    //     `version<<4 | ihl`. Version must be 4; IHL must be ≥ 5.
    let ver_ihl = match read_u8(ipv4_offset + IPV4_VER_IHL_OFFSET) {
        Ok(v) => v,
        Err(()) => return Verdict::PassToKernel,
    };
    let version = (ver_ihl >> 4) & 0x0F;
    let ihl = ver_ihl & 0x0F;
    if version != 4 || ihl < 5 {
        record_malformed_header_drop();
        return Verdict::Drop;
    }

    // (3) IP total_length sanity. The header advertises a
    //     total length (incl. its own header + payload). It must
    //     fit inside the actual packet (after the Ethernet
    //     header) AND be at least `IHL*4` bytes (the header
    //     itself).
    let total_len = match read_u16_be(ipv4_offset + IPV4_TOT_LEN_OFFSET) {
        Ok(v) => v,
        Err(()) => return Verdict::PassToKernel,
    };
    let header_bytes = (ihl as u16).wrapping_mul(4);
    if total_len < header_bytes {
        record_malformed_header_drop();
        return Verdict::Drop;
    }
    // The packet on the wire is `eth_hdr + total_len` bytes. We
    // received `packet_len` bytes; the IPv4 region claim must fit.
    let claimed_pkt_len = ipv4_offset.saturating_add(total_len as usize);
    if claimed_pkt_len > packet_len {
        record_malformed_header_drop();
        return Verdict::Drop;
    }

    // (4) Protocol — only TCP / UDP go through the LB.
    let proto = match read_u8(ipv4_offset + IPV4_PROTO_OFFSET) {
        Ok(v) => v,
        Err(()) => return Verdict::PassToKernel,
    };
    if proto != IPV4_PROTO_TCP && proto != IPV4_PROTO_UDP {
        return Verdict::PassToKernel;
    }

    // (5) TCP flags — only relevant for TCP. UDP gates this trivially.
    if proto == IPV4_PROTO_TCP {
        let flags = match read_u8(l4_offset + TCP_FLAGS_OFFSET) {
            Ok(v) => v,
            Err(()) => return Verdict::PassToKernel,
        };
        if tcp_flags_pathological(flags) {
            record_malformed_header_drop();
            return Verdict::Drop;
        }
    }

    Verdict::Continue
}
