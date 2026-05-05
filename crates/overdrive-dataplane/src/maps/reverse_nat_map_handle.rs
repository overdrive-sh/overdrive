//! `ReverseNatMapHandle` — typed userspace wrapper around the
//! `REVERSE_NAT_MAP` `BPF_MAP_TYPE_HASH` per architecture.md § 10.
//!
//! Key = `ReverseKey { client_ip, client_port, backend_ip,
//! backend_port, proto, _pad }` (host-order in the map);
//! value = `OriginalDest { vip, vip_port, _pad }` (host-order).
//!
//! Endianness-conversion responsibility: this handle takes
//! host-order inputs and writes host-order values. The kernel-side
//! `reverse_key_from_packet` / `original_dest_to_wire` helpers in
//! `overdrive-bpf::shared::sanity` convert at the wire boundary.
//! Roundtrip proptest at the foot of this module asserts no
//! userspace-side endian flip sneaks in (S-2.2-17 sibling).
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 05.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
