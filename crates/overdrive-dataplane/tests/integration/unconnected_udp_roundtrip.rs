//! S-01-01 / S-01-02 / S-01-03 / S-02-03 — unconnected-UDP same-host
//! round-trip through the real cgroup sendmsg4 + recvmsg4 hooks.
//!
//! Feature: `unconnected-udp-sendmsg4` (GH #200, ADR-0053 rev 2026-06-05).
//! Story: US-01 (WALKING SKELETON) + US-02 Tier-3 prong. Job: J-OPS-004,
//! J-PLAT-004.
//!
//! Tags: `@walking_skeleton @US-01 @US-02 @kpi-K1 @kpi-K2 @kpi-K3`
//!       `@tier3 @real-io @adapter-integration @driving_adapter`.
//! Tier: **Tier 3 (real kernel — THE GATE).** There is NO Tier-2
//! `BPF_PROG_TEST_RUN` backstop for `cgroup_sock_addr` (ENOTSUPP ≤ 6.8);
//! the Tier-1 `reply-source-rewrite-lockstep` invariant is the structural
//! defense below this gate.
//!
//! # What these prove (the reframed app-sockaddr ACs — DDD-3a)
//!
//! With a same-host DNS-shape UDP service on a VIP and one local backend
//! registered via the production dual-write:
//!
//! - **S-01-01 (WS):** a same-host client `sendto(VIP)` WITHOUT `connect()`
//!   reaches the backend AND the source it reads via `recvfrom` is the
//!   **VIP** (the recvmsg4 reply-source rewrite). Asserted at the
//!   **application sockaddr layer** (`recvfrom` return), NOT via
//!   `tcpdump -i lo` (which shows the backend source on every round-trip
//!   regardless — recvmsg4 fires post-dequeue; research Q4).
//! - **S-01-02:** `bpftool`-equivalent dumps show BOTH the forward
//!   `LOCAL_BACKEND_MAP (vip, port, udp) -> backend` and the reverse
//!   `REVERSE_LOCAL_MAP (backend, udp) -> vip` entries after ONE
//!   `register_local_backend` (ordered reverse-first; no forward-without-
//!   reverse window).
//! - **S-01-03:** a second unconnected `sendto` reuses the same entries
//!   (stateless; no conntrack).
//! - **S-02-03:** the Tier-3 reply-source identity meets the Tier-1
//!   reply mirror at the shared backend identity.
//!
//! # Fixture discipline (S-03-03)
//!
//! The stub UDP responder binds OFF the systemd-resolved-owned UDP 5353
//! (and :53) per `.claude/rules/debugging.md` § 11, and asserts a clean
//! `bind` rather than swallowing `EADDRINUSE` (§ 8 — no `let _` on
//! fallible setup). The test process runs as a descendant of the
//! configured `cgroup_attach_path` (`/sys/fs/cgroup`) so the hooks fire —
//! `cargo xtask lima run --` runs nextest as root under that ancestor
//! (the `local_backend_proto_connect.rs` harness model).
//!
//! # RED scaffold
//!
//! `#[should_panic(expected = "RED scaffold")]` per the project RED
//! convention (`.claude/rules/testing.md` § "RED scaffolds"). The bodies
//! `panic!("Not yet implemented -- RED scaffold (...)")` — they PASS
//! under nextest (green bar by construction) and ARE the executable
//! specification. DELIVER replaces each `panic!` with the real
//! `EbpfDataplane`-driven round-trip (the two new programs + the dual-
//! write + the `reverse_local_map_entries` accessor must land first).

// RED scaffold doc comments name kernel maps / methods (LOCAL_BACKEND_MAP,
// REVERSE_LOCAL_MAP, register_local_backend) in prose; the canonical
// backticked form lands when DELIVER replaces these panics with the real
// round-trip. Per the DISTILL scaffold lint convention.
#![allow(clippy::missing_panics_doc, clippy::doc_markdown)]

/// S-01-01 (WALKING SKELETON) — unconnected `sendto`/`recvfrom` round-trip;
/// the `recvfrom` source the app reads is the VIP, not the backend IP.
#[test]
#[should_panic(expected = "RED scaffold")]
fn unconnected_sendto_recvfrom_reads_vip_sourced_reply() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-01 / WS: unconnected sendto(VIP) \
         round-trip; recvfrom source == VIP at the app sockaddr layer, not the backend IP). \
         ASSERTION LAYER (read before implementing): this test asserts the APPLICATION \
         sockaddr the client reads back -- the source in recvfrom/msg_name MUST equal the \
         VIP (10.96.0.10), NOT the backend IP. It does NOT assert the wire. A tcpdump -i lo \
         capture showing the BACKEND source on every datagram is EXPECTED and CORRECT: \
         recvmsg4 fires post-dequeue and never touches the wire (research Q4). 'No wire \
         assertions' means assert the app observable, NOT 'ignore the network' -- the \
         round-trip must really deliver and the app-source rewrite must really fire. \
         Blocked on: cgroup sendmsg4+recvmsg4 programs + register_local_backend dual-write \
         (Slice 01 GREEN)."
    );
}

/// S-01-02 — both forward LOCAL_BACKEND_MAP and reverse REVERSE_LOCAL_MAP
/// entries present after ONE register_local_backend (ordered reverse-first).
#[test]
#[should_panic(expected = "RED scaffold")]
fn forward_and_reverse_map_entries_present_after_one_register() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-02: bpftool-equivalent dump shows \
         LOCAL_BACKEND_MAP (vip,53,udp)->backend AND REVERSE_LOCAL_MAP (backend,udp)->vip \
         after one register_local_backend; no forward-without-reverse window). \
         Blocked on: REVERSE_LOCAL_MAP handle + reverse_local_map_entries accessor (Slice 01 GREEN)."
    );
}

/// S-01-03 — a second unconnected query reuses the same mapping (stateless).
#[test]
#[should_panic(expected = "RED scaffold")]
fn second_unconnected_query_reuses_same_mapping_statelessly() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-03: a second unconnected sendto reuses \
         the same LOCAL_BACKEND_MAP/REVERSE_LOCAL_MAP entries; recvfrom source again == VIP; \
         no per-flow state created). Blocked on: Slice 01 GREEN."
    );
}

/// S-02-03 — the Tier-3 reply-source identity meets the Tier-1 reply
/// mirror at the shared backend identity.
#[test]
#[should_panic(expected = "RED scaffold")]
fn kernel_reply_source_meets_tier1_reply_mirror_at_backend_identity() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02-03: the real recvmsg4 reply source (VIP) \
         and REVERSE_LOCAL_MAP (backend,udp)->vip match the Tier-1 reply_source_for(...) for \
         the same BackendKey; removing the kernel reply rewrite fails this Tier-3 acceptance). \
         Blocked on: Slice 01 + Slice 02 GREEN."
    );
}
