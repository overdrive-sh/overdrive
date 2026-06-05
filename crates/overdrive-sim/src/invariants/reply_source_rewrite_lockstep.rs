//! `ReplySourceRewriteLockstep` — unconnected-udp-sendmsg4 Slice 02
//! (US-02; J-PLAT-004 / K3). GH #200, ADR-0053 rev 2026-06-05.
//!
//! **Always invariant**: after `register_local_backend(vip, vip_port,
//! backend, proto)`, the `SimDataplane` reply mirror carries
//! `BackendKey(backend_ip, backend_port, proto) → vip` — i.e. the reply
//! source the unconnected-UDP recvmsg4 path would present for that
//! backend identity is the **VIP**, never the backend. Every forward
//! `local_backend` entry has a matching reply-mirror entry; deregister
//! purges both in lockstep.
//!
//! This is the **structural defense BELOW Tier-3** for the reply-source
//! identity. There is NO Tier-2 `BPF_PROG_TEST_RUN` backstop for
//! `cgroup_sock_addr` (ENOTSUPP ≤ 6.8), so the kernel recvmsg4 reply
//! rewrite is a Tier-3-only gate; this Tier-1 invariant pins the SAME
//! observable contract on the Sim adapter, meeting Tier-3 at the shared
//! backend identity (the two-pronged pin). A forward-only / asymmetric
//! regression — register writes the forward entry but NOT the reply
//! mirror — turns this RED (the #163-class mutation this slice kills).
//!
//! Mirrors `ReverseNatLockstep`'s shape (the
//! `submit-a-udp-service.yaml` step-4 template), retargeted from the XDP
//! `update_service` / `reverse_nat` wire path to the cgroup
//! `register_local_backend` / `reply_mirror` same-host reply path.
//!
//! # RED scaffold (Slice 02 / S-02-01, S-02-02)
//!
//! The evaluator body is a `todo!("RED scaffold: …")` panic. Per
//! `.claude/rules/testing.md` § "RED scaffolds" + § "Downstream fallout
//! on pre-existing tests", a RED invariant evaluator MUST panic with the
//! "RED scaffold" message rather than return `InvariantResult::Fail`: an
//! `InvariantResult::Fail` reds the green bar and forces every
//! full-invariant-walk test (`run_boots_…`,
//! `default_harness_run_passes_…`) to fail, which the project convention
//! forbids (the bar stays green; lefthook passes without `--no-verify`).
//! The adjacent walk tests carry `#[should_panic(expected = "RED
//! scaffold")]` until this lands GREEN.
//!
//! The real evaluator — driving `register_local_backend` then asserting
//! `reply_source_for(BackendKey) == Some(vip)`, plus the S-02-02
//! forward-only-mutation asymmetry assertion — is the DELIVER Slice-01/02
//! GREEN target. It lands when `SimDataplane::register_local_backend`
//! gains the reply-mirror write (DDD-5d) under the same `local_state`
//! mutex acquisition. At that point the evaluator body's `todo!()` is
//! replaced by the real assertions (the scenario in the fn docstring
//! below) and the `#[should_panic]` attributes on the walk tests are
//! removed in the same commit.
//!
//! The asymmetry is the point (S-02-02): a forward-only mutation — the
//! mirror write removed while the forward `local_backend` entry stays —
//! must keep this invariant RED. That is exactly the #163-class
//! regression this invariant exists to catch: a forward entry must never
//! be observable without its paired reply-mirror entry.

use crate::harness::InvariantResult;

/// Drive the unconnected-UDP reply-path lockstep scenario and return an
/// `InvariantResult` pinned to the canonical kebab-case name.
///
/// # Scenario
///
/// 1. Build a `SimDataplane` with N same-host UDP services, each one VIP
///    + one local backend, via `register_local_backend(vip, vip_port,
///    backend, Udp)` — the unconnected-UDP shape.
/// 2. After each register, assert
///    `reply_source_for(BackendKey(backend_ip, backend_port, udp)) ==
///    Some(vip)` for every registered backend (the reply source the app
///    would read is the VIP).
/// 3. Deregister one service; assert its reply-mirror entry is purged
///    (no orphan reverse mapping leaks).
/// 4. Re-register it; assert the reply-mirror entry reappears with the
///    matching VIP.
///
/// The lockstep guarantee comes from `SimDataplane`: `local_backends`
/// and `reply_mirror` live inside one `Mutex<LocalState>`, and
/// `register_local_backend` writes both under one acquisition (DDD-5d).
/// A forward-only mutation (forward entry written, reply mirror not)
/// fails step 2 — the regression this invariant exists to catch.
///
/// RED scaffold: the body is `todo!("RED scaffold: …")`. DELIVER Slice
/// 01/02 replaces it with the real driver implementing the scenario
/// above and removes the `#[should_panic(expected = "RED scaffold")]`
/// attributes on the harness walk tests in the same commit.
#[expect(
    clippy::todo,
    reason = "RED scaffold; GREEN body awaits register_local_backend (Slice 01/02)"
)]
pub async fn evaluate_reply_source_rewrite_lockstep() -> InvariantResult {
    todo!(
        "RED scaffold: S-02-01 reply-source-rewrite lockstep — drive register_local_backend then assert reply_source_for(BackendKey) == Some(vip), plus the S-02-02 forward-only-mutation asymmetry (lands GREEN in Slice 01/02 when the SimDataplane reply-mirror write lands)"
    )
}
