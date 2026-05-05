//! Shared kernel-side sanity prologue and endianness-conversion
//! helpers per architecture.md § 11 + Q3=C.
//!
//! The five static Cloudflare-shape checks (research § 7.2):
//!
//! 1. EtherType is IPv4 (`0x0800`) — non-IPv4 returns `XDP_PASS`.
//! 2. IP version is 4 and IHL ≥ 5 (20 bytes) — invalid returns
//!    `XDP_DROP`.
//! 3. IP `total_length` sanity (≥ IHL·4, ≤ packet length).
//! 4. Transport protocol is TCP (6) or UDP (17) — others return
//!    `XDP_PASS`.
//! 5. For TCP: flag combination is not nonsense (no SYN+RST, no
//!    SYN+FIN, no all-zero) — invalid returns `XDP_DROP` and
//!    increments `DROP_COUNTER[MalformedHeader]`.
//!
//! `reverse_key_from_packet` / `original_dest_to_wire` close the
//! § 11 endianness lockstep — wire = network-order, map storage =
//! host-order; conversion happens here and only here.
//!
//! **RED scaffold** — every helper body panics via `todo!()`.
//! DELIVER lands the bodies per Slice 05 (endianness) / Slice 06
//! (sanity prologue).
//!
//! See test-scenarios.md S-2.2-17 (endianness roundtrip),
//! S-2.2-19..21 (sanity drops).

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
