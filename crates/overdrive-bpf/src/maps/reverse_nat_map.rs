//! `REVERSE_NAT_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `ReverseKey { client_ip: u32, client_port: u16, backend_ip: u32,
//! backend_port: u16, proto: u8, _pad: [u8; 3] }` → `OriginalDest
//! { vip: u32, vip_port: u16, _pad: [u8; 2] }`.
//!
//! All values stored host-order; conversion at the kernel boundary
//! via the shared `reverse_key_from_packet` /
//! `original_dest_to_wire` helpers per architecture.md § 11
//! endianness lockstep.
//!
//! `max_entries = 1_048_576` (operator-tunable in future; Phase
//! 2.2 fixed) per architecture.md § 10.
//!
//! **RED scaffold** — `#[map]` declaration not yet emitted.
//! DELIVER lands it per Slice 05 (US-05; S-2.2-15..18).

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
