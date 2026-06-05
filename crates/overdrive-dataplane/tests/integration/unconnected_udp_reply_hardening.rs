//! S-03-01 / S-03-02 / S-03-03 — unconnected-UDP reply-path error
//! hardening: sentinel-on-miss (no backend-IP leak to the app),
//! below-floor attach refusal, and fixture-collision discipline.
//!
//! Feature: `unconnected-udp-sendmsg4` (GH #200, ADR-0053 rev 2026-06-05).
//! Story: US-03. Job: J-OPS-004 (primary), J-PLAT-004.
//!
//! Tags: `@US-03 @kpi-K5 @tier3 @real-io @adapter-integration @error`.
//! Tier: **Tier 3 (real kernel — THE GATE).** No Tier-2 backstop.
//!
//! # What these prove (the reframed app-sockaddr ACs — DDD-3 / DDD-3a)
//!
//! - **S-03-01 (sentinel-on-miss):** with the forward LOCAL_BACKEND_MAP
//!   entry present but the REVERSE_LOCAL_MAP entry forced absent
//!   (eviction/corruption), the source the client app reads via
//!   `recvfrom` is the sentinel `192.0.2.1` (RFC 5737) — NEVER the
//!   backend IP — and REVERSE_LOCAL_MISS_COUNTER increments.
//!   recvmsg4 CANNOT deny (verifier `[1,1]`, research Q1) — the fail-safe
//!   is a source rewrite, not a drop. This is the **app-sockaddr** K5,
//!   NOT a `tcpdump`/wire assertion (recvmsg4 never touches the wire).
//!   Open question (DELIVER/Tier-3, NOT a tracking issue per
//!   `feedback_no_unilateral_gh_issues`): confirm `dig`/glibc/musl
//!   cleanly REJECT a `192.0.2.1`-sourced reply; swap the sentinel value
//!   (no design change) if a resolver surprisingly accepts it (DESIGN
//!   open-Q 1 / F-4).
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

/// S-03-01 — a forced REVERSE_LOCAL_MAP miss never leaks the backend IP
/// to the app; the source is the sentinel 192.0.2.1 and the miss counts.
#[test]
#[should_panic(expected = "RED scaffold")]
fn reverse_miss_rewrites_source_to_sentinel_not_backend_ip() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-01: forward entry present, reverse forced \
         absent; the recvfrom source the app reads is the sentinel 192.0.2.1 (RFC 5737), \
         NEVER the backend IP; REVERSE_LOCAL_MISS_COUNTER increments; recvmsg4 does not drop \
         (cannot -- verifier [1,1])). Blocked on: recvmsg4 sentinel-miss branch + miss counter \
         (Slice 03 GREEN)."
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
