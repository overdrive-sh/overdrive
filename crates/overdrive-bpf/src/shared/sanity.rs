//! Shared kernel-side sanity prologue and endianness-conversion
//! helpers per architecture.md § 11 + Q3=C.
//!
//! Two concerns colocated here, both belonging to the wire ↔ host
//! byte-order conversion boundary:
//!
//! 1. **Sanity prologue** — five static Cloudflare-shape checks
//!    (research § 7.2):
//!
//!    1. EtherType is IPv4 (`0x0800`) — non-IPv4 returns `XDP_PASS`.
//!    2. IP version is 4 and IHL ≥ 5 (20 bytes) — invalid returns
//!       `XDP_DROP`.
//!    3. IP `total_length` sanity (≥ IHL·4, ≤ packet length).
//!    4. Transport protocol is TCP (6) or UDP (17) — others return
//!       `XDP_PASS`.
//!    5. For TCP: flag combination is not nonsense (no SYN+RST, no
//!       SYN+FIN, no all-zero) — invalid returns `XDP_DROP` and
//!       increments `DROP_COUNTER[MalformedHeader]`.
//!
//!    Sanity bodies land in Slice 06.
//!
//! 2. **`reverse_key_from_packet`** — the wire ↔ host endianness
//!    conversion site for the `REVERSE_NAT_MAP` lookup key. Reads
//!    the source IP / source port / proto from a `*const u8` packet
//!    cursor in **wire order** (network byte order, big-endian) and
//!    returns a `ReverseKey` POD whose numeric values are
//!    **host order**. This is the single point of conversion per
//!    architecture.md § 11 — the userspace handle stores the same
//!    host-order numerics without any flip.
//!
//! See test-scenarios.md S-2.2-17 (endianness roundtrip),
//! S-2.2-19..21 (sanity drops).

#![allow(dead_code)]

/// Wire-shape POD matching `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`'s
/// `BackendKey` byte-for-byte. 8 bytes, all fields **host order**:
/// `ip_host` (u32) + `port_host` (u16) + `proto` (u8) + `_pad` (u8).
///
/// Re-exported here under the name `ReverseKey` because in the
/// `reverse_key_from_packet` helper's vocabulary the key represents
/// the response-side 3-tuple (a backend's egress source) used to
/// reverse-NAT back to the original VIP. The name reflects the
/// helper's domain role; the underlying POD shape is identical to
/// the map key.
///
/// `#[repr(C)]` — must match the kernel-side `bpf_map_lookup_elem`
/// byte-for-byte against the userspace seed of the same shape.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ReverseKey {
    /// Backend IPv4 address. Host-order numeric.
    pub ip_host: u32,
    /// Backend port. Host-order numeric.
    pub port_host: u16,
    /// L4 protocol — IANA proto number (TCP=6, UDP=17).
    pub proto: u8,
    /// Padding for 8-byte alignment in BPF map storage. Always 0.
    pub _pad: u8,
}

/// Build a `ReverseKey` from wire-order packet bytes.
///
/// Reads four bytes at `ipv4_src_ip_ptr` and two bytes at
/// `l4_src_port_ptr` as **network byte order** (the wire), and
/// converts each via `from_be_bytes` to a host-order numeric. The
/// resulting `ReverseKey` has the same byte layout the userspace
/// handle writes into REVERSE_NAT_MAP — the `BPF_MAP_TYPE_HASH`
/// lookup keys on the raw bytes, so userspace-host-order +
/// kernel-from_be_bytes(wire) must produce the same numeric for the
/// lookup to hit. This is the architecture.md § 11 lockstep.
///
/// # Safety
///
/// `ipv4_src_ip_ptr` must point to at least 4 bytes of valid packet
/// data (the IPv4 source-IP field at offset 12 of the IPv4 header).
/// `l4_src_port_ptr` must point to at least 2 bytes of valid packet
/// data (the TCP/UDP source-port field at offset 0 of the L4
/// header). Callers MUST have already bounds-checked the cursor via
/// the `ptr_at` helper before invoking this function.
///
/// # Why a free function over `*const u8`
///
/// The XDP / TC programs' `ptr_at` helpers return `*const T`
/// already-bounds-checked. This helper consumes the bounds-checked
/// pointers and performs only the byte-order conversion — no
/// further bounds work, no `panic`, no helper calls. Verifier-clean
/// by construction. Inlined at the call site so the verifier sees
/// the conversion arithmetic directly.
#[inline(always)]
pub unsafe fn reverse_key_from_packet(
    ipv4_src_ip_ptr: *const u8,
    l4_src_port_ptr: *const u8,
    proto: u8,
) -> ReverseKey {
    // SAFETY: caller guarantees `ipv4_src_ip_ptr` points to ≥ 4
    // bytes of valid packet data (IPv4 src-IP field).
    let ip_bytes: [u8; 4] = unsafe { *(ipv4_src_ip_ptr as *const [u8; 4]) };
    // SAFETY: caller guarantees `l4_src_port_ptr` points to ≥ 2
    // bytes of valid packet data (L4 src-port field).
    let port_bytes: [u8; 2] = unsafe { *(l4_src_port_ptr as *const [u8; 2]) };

    ReverseKey {
        ip_host: u32::from_be_bytes(ip_bytes),
        port_host: u16::from_be_bytes(port_bytes),
        proto,
        _pad: 0,
    }
}
