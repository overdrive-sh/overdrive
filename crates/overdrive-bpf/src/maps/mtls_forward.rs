//! Maps for the transparent-mTLS OUTBOUND forward sockmap EGRESS-redirect
//! (ADR-0069, GH #26; D-MTLS-4). The `sk_skb_stream_verdict_mtls` program reads
//! all three; the userspace `HostMtlsEnforcement` adapter writes them.
//!
//! Proven in `findings-egress-ktls-splice.md` (15/15, agent-idle):
//! - `MTLS_SOCKMAP` holds leg F (slot 0, workload-facing plaintext source) and
//!   leg B (slot 1, the agent's kTLS-armed peer-facing leg — the redirect TARGET).
//!   A redirect-target socket MUST belong to a verdict-governed sockmap (a
//!   verdict-less map rejects inserts with EOPNOTSUPP on 7.0), so leg B lives here
//!   too even though its own RX is only `SK_PASS`ed.
//! - `MTLS_FPORT[0]` = leg F's host-order `local_port`, so the verdict only
//!   redirects skbs arriving on leg F (not leg B's own RX).
//! - `MTLS_ARMED[0]` = 1 once kTLS is armed on leg B. While 0 the verdict
//!   `SK_DROP`s leg F's bytes (fail-closed: no plaintext leak before kTLS arms).
//!
//! One `MTLS_SOCKMAP` + `MTLS_FPORT` + `MTLS_ARMED` triple per agent today (the
//! composed walking-skeleton gate drives a single outbound flow); per-connection
//! scaling of the forward path is a later-slice concern, out of 01-01 scope.

#![allow(dead_code)]

use aya_ebpf::{
    macros::map,
    maps::{Array, SockMap},
};

/// Leg F target slot — the plaintext source the verdict redirects FROM.
pub const F_IDX: u32 = 0;
/// Leg B slot — the kTLS-armed redirect TARGET (`flags=0` drives its TX).
pub const B_IDX: u32 = 1;

/// `MTLS_SOCKMAP` — `BPF_MAP_TYPE_SOCKMAP` holding leg F (slot 0) and leg B
/// (slot 1). The `sk_skb_stream_verdict_mtls` program is attached to it and fires
/// on the RX of either member; leg F's RX is EGRESS-redirected into leg B's TX.
#[map]
pub static MTLS_SOCKMAP: SockMap = SockMap::with_max_entries(2, 0);

/// `MTLS_FPORT[0]` — leg F's host-order `local_port`, set by the userspace
/// adapter so the verdict only redirects skbs arriving on leg F.
#[map]
pub static MTLS_FPORT: Array<u32> = Array::with_max_entries(1, 0);

/// `MTLS_ARMED[0]` — 1 once kTLS is armed on leg B; while 0 the verdict
/// fail-closed `SK_DROP`s leg F's bytes (no plaintext leak before the arm).
#[map]
pub static MTLS_ARMED: Array<u32> = Array::with_max_entries(1, 0);
