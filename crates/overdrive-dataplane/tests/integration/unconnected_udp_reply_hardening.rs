//! S-03-01 / S-03-02 / S-03-03 — unconnected-UDP reply-path error
//! hardening: no-op-on-miss (non-service UDP unaffected; no backend-IP
//! leak via always-hit), below-floor attach refusal, and
//! fixture-collision discipline.
//!
//! Feature: `unconnected-udp-sendmsg4` (GH #200, ADR-0053 rev 2026-06-05).
//! Story: US-03. Job: J-OPS-004 (primary), J-PLAT-004.
//!
//! Tags: `@US-03 @kpi-K5 @tier3 @real-io @adapter-integration @error`.
//! Tier: **Tier 3 (real kernel — THE GATE).** No Tier-2 backstop.
//!
//! # What these prove (the corrected app-sockaddr ACs — DDD-3 / DDD-3a / CA-3 / UI-1)
//!
//! - **S-03-01 (no-op-on-miss; non-service UDP unaffected):** recvmsg4
//!   attaches at a cgroup ANCESTOR and fires on EVERY unconnected-UDP
//!   `recvmsg`/`recvfrom` from any descendant — service replies AND all
//!   unrelated same-host UDP (DNS clients, a backend's own `recvfrom` of
//!   an inbound query). The REVERSE_LOCAL_MAP lookup is the discriminator.
//!   Three corrected assertions:
//!   (a) **non-service unconnected UDP is unaffected** — a same-host
//!       exchange whose source is NOT a registered backend reads its REAL
//!       sender address via `recvfrom`/`msg_name`; recvmsg4 leaves it
//!       byte-for-byte intact (pure no-op on a miss — the load-bearing
//!       new assertion, the regression the correction fixes);
//!   (b) **a service reply always HITS → VIP-sourced** — under the D1
//!       reverse-first dual-write a genuine service reply's source is
//!       always a registered backend identity, so it always hits and the
//!       app reads the VIP as the source — no backend-IP-leak path;
//!   (c) **the miss counter is observable but inert** —
//!       REVERSE_LOCAL_MISS_COUNTER increments on a non-service recv AND
//!       the source the app read on that same recv is untouched (counted
//!       but no source rewrite). recvmsg4 CANNOT deny (verifier `[1,1]`,
//!       research Q1) — every path returns 1; the no-leak guarantee (K5)
//!       holds via the always-hit dual-write, NOT a miss-path sentinel.
//!   App-sockaddr assertions, NOT `tcpdump`/wire (recvmsg4 never touches
//!   the wire). There is NO sentinel `192.0.2.1` rewrite on the miss
//!   path — it would corrupt every non-service datagram's sender address
//!   (Tier-3-observed, fixed in DELIVER step 01-03, commit `e71ad780`).
//!   No-op-on-miss is Cilium-aligned. Per DDD-3 / feature-delta CA-3 /
//!   research addendum "UI-1 adjudication (2026-06-05)".
//! - **S-03-02 (below-floor refusal):** a host below the recvmsg4 floor
//!   (< 4.20) fails `attach()` and the composition root refuses to start
//!   with a structured `health.startup.refused` (the `attach()` syscall
//!   IS the preflight — NO `/proc`/`uname` parse, DDD-5b/c). The failure
//!   routes through a `#[from]`-typed DataplaneBootError variant, never a
//!   flattened `Internal(String)`. On the 5.10+ Lima matrix this asserts
//!   the refusal SHAPE via the typed-error path (a real <4.20 kernel is
//!   not on the matrix).
//! - **S-03-03 (fixture collision):** the stub resolver binds OFF UDP
//!   5353 and asserts a clean `bind` — an `EADDRINUSE` fails the test
//!   loudly, never swallowed with `.ok()` / `let _`
//!   (`.claude/rules/debugging.md` § 11 + § 8).
//!
//! # RED scaffold
//!
//! `#[should_panic(expected = "RED scaffold")]` per the project RED
//! convention. DELIVER replaces each `panic!` with the real
//! `EbpfDataplane`-driven assertion (Slice 03 GREEN; depends on Slice 01
//! + Slice 02).

// RED scaffold doc comments name kernel maps / methods in prose; the
// canonical backticked form lands when DELIVER replaces these panics with
// the real hardening tests. Per the DISTILL scaffold lint convention.
#![allow(clippy::missing_panics_doc, clippy::doc_markdown)]

/// S-03-01 — recvmsg4 no-op-on-miss: non-service unconnected UDP reads its
/// real source; a service reply always hits and is VIP-sourced; the miss
/// counter increments on non-service recv but is behaviorally inert.
#[test]
#[should_panic(expected = "RED scaffold")]
fn non_service_unconnected_udp_reads_real_source_recvmsg4_noop_on_miss() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-01: recvmsg4 fires on ALL subtree \
         unconnected UDP (cgroup-ancestor attach). Three assertions: (a) a non-service \
         exchange (source NOT a registered backend -- a backend reading an inbound query, \
         any unrelated UDP) reads its REAL sender source via recvfrom/msg_name -- recvmsg4 \
         leaves it byte-for-byte intact (pure no-op on a REVERSE_LOCAL_MAP miss); (b) a \
         genuine service reply (source IS the registered backend) ALWAYS hits and the app \
         reads the VIP as the source -- no backend-IP-leak path; (c) REVERSE_LOCAL_MISS_COUNTER \
         increments on the non-service recv AND the source is untouched on that same recv \
         (counted but inert -- no source rewrite). recvmsg4 does not drop (cannot -- verifier \
         (1,1)). NO sentinel 192.0.2.1 rewrite on the miss path -- it would corrupt non-service \
         senders (fixed step 01-03). No-leak (K5) holds via the D1 reverse-first dual-write \
         always-hit, not a sentinel. Blocked on: recvmsg4 hit-rewrite + no-op-miss branch + \
         miss counter (Slice 03 GREEN)."
    );
}

/// S-03-02 — a below-floor kernel refuses observably at attach/preflight
/// via a typed DataplaneBootError, never a forward-only half-working service.
#[test]
#[should_panic(expected = "RED scaffold")]
fn below_floor_kernel_refuses_at_attach_preflight_observably() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-02: below recvmsg4 floor (<4.20) the \
         attach() syscall fails and the composition root refuses with health.startup.refused \
         via a #[from]-typed DataplaneBootError -- never Internal(String), never a forward-only \
         half-working service; NO /proc/uname parse). Blocked on: probe attaches both hooks + \
         CgroupSendRecvAttach/ReverseLocalProbe error variants (Slice 03 GREEN)."
    );
}

/// S-03-03 — the Tier-3 stub resolver binds off UDP 5353 and asserts a
/// clean bind; an EADDRINUSE fails the test loudly.
#[test]
#[should_panic(expected = "RED scaffold")]
fn stub_resolver_binds_off_5353_and_asserts_clean_bind() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-03: the stub UDP responder binds off the \
         systemd-resolved-owned UDP 5353/:53 and asserts a clean bind; an EADDRINUSE fails \
         the test loudly, never swallowed with .ok()/let _). Blocked on: the Tier-3 stub-resolver \
         fixture (Slice 03 GREEN)."
    );
}
