//! `sk_skb_stream_verdict_mtls` — the forward sockmap EGRESS-redirect for the
//! transparent-mTLS OUTBOUND steady state (ADR-0069, GH #26; D-MTLS-4).
//!
//! Proven in `findings-egress-ktls-splice.md` (15/15, agent-idle): an `sk_skb/
//! stream_verdict` on a sockmap holding leg F (workload-facing plaintext source)
//! and leg B (the agent's kTLS-armed peer-facing leg). For a skb arriving on leg
//! F's RX it calls `bpf_sk_redirect_map(skb, &MTLS_SOCKMAP, B_IDX, 0)` —
//! **`flags=0` = EGRESS** — which drives leg B's `tcp_sendmsg_locked` → kTLS
//! encrypt → TLS 1.3 records on the peer wire, with the AGENT issuing ZERO
//! per-byte syscalls (agent-idle forward). For leg B's own RX it `SK_PASS`es so
//! leg B's kTLS-RX is undisturbed.
//!
//! Pre-arm fail-closed: while `MTLS_ARMED[0] != 1` the verdict `SK_DROP`s leg F's
//! bytes — no plaintext can reach leg B's TX before kTLS is armed (confidentiality
//! invariant). The userspace adapter sets `MTLS_FPORT[0]` = leg F's host-order
//! `local_port` and flips `MTLS_ARMED[0] = 1` only after the handshake + kTLS arm.
//!
//! aya-ebpf 0.1.1 ships NO `#[sk_skb]` proc macro (the macro source exists in
//! `aya-ebpf-macros` but is unwired in `lib.rs`); the section is hand-rolled via
//! `#[link_section = "sk_skb/stream_verdict"]`, the name aya's loader recognises.
//! The redirect helper is the TYPED `SockMap::redirect_skb(&SkBuffContext, index,
//! flags)` → `bpf_sk_redirect_map`, not a raw syscall.

#![allow(dead_code)]

use aya_ebpf::{
    bindings::{
        __sk_buff,
        sk_action::{SK_DROP, SK_PASS},
    },
    programs::SkBuffContext,
};

use crate::maps::mtls_forward::{MTLS_ARMED, MTLS_FPORT, MTLS_SOCKMAP};

/// Leg B (the kTLS leg) slot in `MTLS_SOCKMAP` — the EGRESS-redirect target.
const B_IDX: u32 = 1;

/// Hand-rolled `sk_skb/stream_verdict` on `MTLS_SOCKMAP`. The kernel calls this
/// for each skb arriving on a member socket's RX. For a skb on leg F (matched by
/// host-order `local_port` against `MTLS_FPORT[0]`) it EGRESS-redirects into leg
/// B's kTLS TX (`flags=0`); for leg B's own RX it `SK_PASS`es. Pre-arm
/// (`MTLS_ARMED[0] != 1`) it `SK_DROP`s leg F's bytes (fail-closed).
#[unsafe(no_mangle)]
#[unsafe(link_section = "sk_skb/stream_verdict")]
pub extern "C" fn sk_skb_stream_verdict_mtls(skb: *mut __sk_buff) -> u32 {
    let ctx = SkBuffContext::new(skb);

    // SAFETY: `__sk_buff::local_port` is a fixed scalar field in the in-tree UAPI;
    // the kernel guarantees the ctx pointer is valid for the program invocation.
    let local_port = unsafe { (*ctx.skb.skb).local_port };

    // Only redirect skbs that arrived on leg F; leg B's own RX is PASSed.
    let fport = match MTLS_FPORT.get(0) {
        Some(&p) => p,
        None => 0,
    };
    if fport == 0 || local_port != fport {
        return SK_PASS;
    }

    // Pre-arm fail-closed: never let leg F's plaintext reach leg B's TX until
    // kTLS is armed (it would otherwise egress as plaintext).
    let armed = matches!(MTLS_ARMED.get(0), Some(&1));
    if !armed {
        return SK_DROP;
    }

    // ARMED: EGRESS-redirect leg F's bytes into leg B's TX (`flags=0`). On the
    // target (leg B, slot `B_IDX`) `flags=0` drives `tcp_bpf_push_locked` →
    // `tcp_sendmsg_locked` → kTLS encrypt → TLS 1.3 records on the peer wire.
    //
    // SAFETY: `SockMap::redirect_skb` is the typed `bpf_sk_redirect_map` wrapper;
    // the verifier validates the bounded redirect.
    let rc = unsafe { MTLS_SOCKMAP.redirect_skb(&ctx, B_IDX, 0) };
    if rc == i64::from(SK_PASS) {
        SK_PASS
    } else {
        // Redirect refused — fail closed (no plaintext leak).
        SK_DROP
    }
}
