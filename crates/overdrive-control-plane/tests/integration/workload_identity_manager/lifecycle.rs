//! Integration — workload-identity-manager walking skeleton (GH #35).
//!
//! Layer 3 (gated `integration-tests`, runs via Lima — exercises a REAL
//! `RcgenCa` doing real P-256 crypto, a REAL `LocalObservationStore` over
//! redb, and a real `openssl verify` subprocess). S-WIM-WS
//! (`walking_skeleton_running_alloc_issues_holds_audits_and_verifies_svid`,
//! 01-07) and S-WIM-12
//! (`restart_reissues_each_still_running_alloc_with_audit_row`, 03-02 —
//! restart-recovery re-issue with a fresh audit row + `openssl verify`) are both
//! ACTIVATED here.
//!
//! #35 is a FOUNDATION feature with NO operator CLI verb — `openssl verify`
//! is the honest external entry point (the `rcgen_ca_chain_verify` /
//! `ca_boot_and_audit` shape: assert on the tool EXIT CODE, not internal
//! reachability — `.claude/rules/testing.md` Tier 3).
//!
//! Cgroup-free: the WS exercises the control-plane convergence loop + the CA
//! chain, NOT the cgroup workload path (`SimDriver`, no real workload spawn).
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; DELIVER replaces the body
//! with real end-to-end assertions.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::SpiffeId;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_host::OsEntropy;
use overdrive_host::ca::RcgenCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use tempfile::TempDir;

const WORKLOAD_NAME: &str = "ws-payments";
const NODE_NAME: &str = "host-0";
const ALLOC_NAME: &str = "alloc-ws-0";

/// Trust-domain subject the WS root is minted for. Mirrors the
/// `rcgen_ca_chain_verify` / `ca_boot_and_audit` precedents.
fn trust_domain_subject() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/overdrive/ca").expect("trust-domain SpiffeId parses")
}

/// `@walking_skeleton` `@real-io` `@adapter-integration` `@S-WIM-WS` -- an alloc
/// reaches Running, `IssueSvid` mints via the built-in CA, the SVID is held in
/// `IdentityMgr`, an audit row is observable, `openssl verify` accepts the
/// chain, and Stop drops the held entry.
///
/// # Dual-When journey (the accepted single demo-able journey)
///
/// **When 1** — an alloc reaching Running → `IssueSvid` is dispatched through
/// the REAL action-shim executor (`ca_issuance::issue_and_audit` over a real
/// `RcgenCa` + a real `LocalObservationStore`) → `IdentityMgr` holds the minted
/// SVID for the pure-derived `SpiffeId::for_allocation` identity, an
/// `issued_certificates` row is observable via the `ObservationStore`, AND
/// `openssl verify -CAfile <root> -untrusted <intermediate> <svid.pem>` exits 0
/// (assert on the tool EXIT CODE, not internal reachability — Tier 3).
///
/// **When 2** — the alloc stops (its `alloc_status` row leaves Running) →
/// `DropSvid` is dispatched → `IdentityMgr` no longer holds that allocation's
/// SVID (O2/K2 — leak resistance on stop).
///
/// # Port-to-port
///
/// The driving port is `run_convergence_tick` for the `svid-lifecycle`
/// reconciler against the `job/<workload>` target — the SAME convergence loop
/// the production boot path runs. The observable outcomes are asserted at the
/// `IdentityMgr::held_snapshot`, `ObservationStore::issued_certificate_rows`,
/// and `openssl verify` exit-code boundaries. No executor / reconciler
/// internals are exercised directly.
///
/// # Why `openssl verify` runs the CA's own chain
///
/// `IdentityMgr::held_snapshot` returns the non-secret PROJECTION (`spiffe_id`
/// + `not_after`), never the held leaf cert PEM (the leaf key stays inside
/// `IdentityMgr`, K2; no `IdentityRead` cert accessor exists until 02-02). So
/// the verify proves the CA WIRED INTO the convergence loop (`state.ca`)
/// produces chains that `openssl verify` accepts for the held identity: root +
/// intermediate from `state.ca`, leaf minted by `state.ca.issue_svid` for the
/// SAME `SpiffeId::for_allocation` the executor held. This is the
/// `ca_boot_and_audit` shape (mint-then-verify the chain the live CA produces).
#[tokio::test]
async fn walking_skeleton_running_alloc_issues_holds_audits_and_verifies_svid() {
    // GIVEN a control-plane convergence harness with a REAL RcgenCa (the `Ca`
    // port) + a REAL LocalObservationStore (the `obs` port), svid-lifecycle
    // registered, cgroup-free (SimDriver, no real workload spawn).
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;

    let workload = WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId");
    let alloc = AllocationId::new(ALLOC_NAME).expect("valid AllocationId");
    let identity = SpiffeId::for_allocation(&workload, &alloc);

    // WHEN 1 — the alloc reaches Running (its alloc_status row goes Running).
    write_alloc_state(&h, ALLOC_NAME, AllocState::Running, 1).await;

    // AND the svid-lifecycle convergence loop ticks: hydrate desired (Running
    // set) + actual (held set, empty) → reconcile emits IssueSvid → the REAL
    // action-shim executor mints via RcgenCa, writes the issued_certificates
    // audit row, and holds the SvidMaterial in IdentityMgr.
    tick(&h, 2).await;
    // A second tick lets any spawned shim work settle before we read.
    tick(&h, 3).await;

    // THEN IdentityMgr holds the alloc with the pure-derived identity (the
    // held_snapshot projection — K1/O1). Read through the driven-port boundary.
    let held = h.state.identity.held_snapshot();
    let facts = held.get(&alloc).unwrap_or_else(|| {
        panic!(
            "IssueSvid must have held the minted SVID for the Running alloc; held set: {:?}",
            held.keys().collect::<Vec<_>>()
        )
    });
    assert_eq!(
        facts.spiffe_id, identity,
        "the held SVID identity must be the pure-derived SpiffeId::for_allocation"
    );

    // AND an issued_certificates audit row is observable through the
    // ObservationStore for that identity (audit-before-hold, ADR-0063 D6).
    let audit_rows = h.state.obs.issued_certificate_rows().await.expect("read audit rows");
    assert!(
        audit_rows.iter().any(|r| r.spiffe_id == identity),
        "an issued_certificates audit row must be observable for the held identity {identity}; \
         rows: {:?}",
        audit_rows.iter().map(|r| r.spiffe_id.to_string()).collect::<Vec<_>>()
    );

    // AND `openssl verify -CAfile <root> -untrusted <intermediate>
    // <held-svid.pem>` exits 0 for the SVID ACTUALLY HELD by IdentityMgr — read
    // through the `IdentityRead::svid_for` port (the dataplane read surface,
    // ADR-0067 D7). This verifies the real held leaf material the executor minted
    // and swapped into the holder, NOT a fresh leaf re-minted here: the held
    // cert chains root → intermediate → leaf under a relying party, and the held
    // material's structured identity matches the pure-derived
    // SpiffeId::for_allocation. Root + intermediate come from the SAME CA the
    // executor used (`state.ca`).
    let node = NodeId::new(NODE_NAME).expect("valid NodeId");
    let root = h.state.ca.root().expect("RcgenCa::root self-signs a real P-256 root");
    let intermediate =
        h.state.ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");

    let held_material = h.state.identity.svid_for(&alloc).unwrap_or_else(|| {
        panic!("IdentityRead::svid_for must return the held SVID material for the Running alloc")
    });
    assert_eq!(
        held_material.spiffe_id(),
        &identity,
        "the HELD SVID material's identity must be the pure-derived SpiffeId::for_allocation"
    );

    let pem_dir = TempDir::new().expect("pem tempdir");
    let root_pem = pem_dir.path().join("root.pem");
    let inter_pem = pem_dir.path().join("intermediate.pem");
    let svid_pem = pem_dir.path().join("held-svid.pem");
    std::fs::write(&root_pem, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem, intermediate.cert_pem().as_pem().as_bytes())
        .expect("write intermediate.pem");
    // The ACTUAL held leaf cert — not a fresh mint (Finding 3).
    std::fs::write(&svid_pem, held_material.cert_pem().as_pem().as_bytes())
        .expect("write held-svid.pem");

    let output = std::process::Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem)
        .arg("-untrusted")
        .arg(&inter_pem)
        .arg(&svid_pem)
        .output()
        .expect("invoke openssl verify");
    assert!(
        output.status.success(),
        "openssl verify -CAfile root.pem -untrusted intermediate.pem held-svid.pem must exit 0 \
         (the HELD SVID chain verifies): stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // WHEN 2 — the alloc stops (its alloc_status row leaves Running). A newer
    // Terminated write wins under LWW, so the next tick's hydrate sees an empty
    // Running set.
    write_alloc_state(&h, ALLOC_NAME, AllocState::Terminated, 4).await;

    // AND the svid-lifecycle convergence loop ticks: reconcile sees
    // `¬running ∧ held` → emits DropSvid → the executor removes the held entry.
    tick(&h, 5).await;
    tick(&h, 6).await;

    // THEN IdentityMgr no longer holds the stopped allocation's SVID — the
    // node-held leaf key is unreachable in the held set (O2/K2).
    let held_after_stop = h.state.identity.held_snapshot();
    assert!(
        !held_after_stop.contains_key(&alloc),
        "DropSvid must have removed the held SVID after the alloc stopped; held set still \
         contains it: {:?}",
        held_after_stop.keys().collect::<Vec<_>>()
    );
}

/// `@real-io` `@error` `@S-WIM-12` -- after a control-plane restart the held set
/// starts empty, every still-Running allocation is re-issued once during
/// recovery, and each re-issue leaves an `issued_certificates` audit row.
///
/// # The restart, simulated honestly
///
/// A control-plane restart is simulated by constructing a FRESH, EMPTY
/// `IdentityMgr` (`IdentityMgr::new(None)`) and a fresh convergence harness over
/// the SAME redb obs/intent stores (`tmp` persists across the restart) and the
/// SAME built-in `RcgenCa`. The in-memory held set is LOST — the leaf private
/// key is non-persistable (`CaKeyPem` has no `Serialize`, ADR-0063 D9) and
/// non-reconstructable (`ca_issuance.rs:34-40`); it was never on disk. So
/// `actual = ∅` post-restart and every still-Running alloc matches
/// `running ∧ ¬held → IssueSvid` (D1 RECOVERY — distinct from the gated #40
/// rotation path; the holder was reset, this is the first-issue branch running
/// again).
///
/// # Observable universe
///
/// The post-restart held map (every still-Running alloc re-held), the
/// `issued_certificates` audit rows (a FRESH row per re-issue — the audit count
/// for the identity strictly increases after recovery), the `openssl verify`
/// exit code (the re-issued chain still verifies — exit 0), and the absence of
/// any recovered old leaf key (it was never persisted — `svid_for` on the fresh
/// `IdentityMgr` reads `None` before the re-issue tick).
#[tokio::test]
async fn restart_reissues_each_still_running_alloc_with_audit_row() {
    // GIVEN a control-plane harness with a REAL RcgenCa + REAL
    // LocalObservationStore over `tmp`, svid-lifecycle registered. Two allocs
    // reach Running and each is issued + held (the pre-restart state).
    let tmp = TempDir::new().expect("tempdir");
    let ca: Arc<dyn Ca> = Arc::new(RcgenCa::new(Arc::new(OsEntropy), trust_domain_subject()));

    let h1 = build_harness_with_ca(&tmp, Arc::clone(&ca)).await;

    let workload = WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId");
    let alloc0 = AllocationId::new("alloc-ws-r0").expect("valid AllocationId");
    let alloc1 = AllocationId::new("alloc-ws-r1").expect("valid AllocationId");
    let id0 = SpiffeId::for_allocation(&workload, &alloc0);
    let id1 = SpiffeId::for_allocation(&workload, &alloc1);

    write_alloc_state(&h1, "alloc-ws-r0", AllocState::Running, 1).await;
    write_alloc_state(&h1, "alloc-ws-r1", AllocState::Running, 1).await;
    tick(&h1, 2).await;
    tick(&h1, 3).await;

    // Both allocs are held pre-restart (the issue succeeded).
    let held_before = h1.state.identity.held_snapshot();
    assert!(
        held_before.contains_key(&alloc0) && held_before.contains_key(&alloc1),
        "both Running allocs must hold an SVID before the restart; held: {:?}",
        held_before.keys().collect::<Vec<_>>()
    );

    // The pre-restart audit-row count for each identity (the recovery re-issue
    // must STRICTLY increase this — a fresh audited row, not a re-read).
    let audit_before = h1.state.obs.issued_certificate_rows().await.expect("read audit rows");
    let count0_before = audit_before.iter().filter(|r| r.spiffe_id == id0).count();
    let count1_before = audit_before.iter().filter(|r| r.spiffe_id == id1).count();
    assert!(count0_before >= 1 && count1_before >= 1, "each identity has a pre-restart audit row");

    // WHEN the control plane restarts: the OLD process exits (releasing its
    // exclusive redb locks on intent/obs/memory) — modelled by dropping `h1` and
    // everything it owns BEFORE the new process boots. A real restart frees the
    // file locks; the in-memory held set (the leaf key) dies with the process and
    // is non-persistable (ADR-0063 D9). Only the redb files on `tmp` survive.
    drop(h1);

    // The new process boots: a FRESH empty IdentityMgr + fresh harness over the
    // SAME redb stores + SAME CA. The alloc_status rows for both allocs persist
    // in redb (still Running).
    let h2 = build_harness_with_ca(&tmp, Arc::clone(&ca)).await;

    // The fresh IdentityMgr holds NOTHING — the old leaf key is unrecoverable
    // (it was never on disk; nothing reconstructs it on boot).
    let held_post_boot = h2.state.identity.held_snapshot();
    assert!(
        held_post_boot.is_empty(),
        "post-restart the held set is empty — the leaf key was never persisted; held: {:?}",
        held_post_boot.keys().collect::<Vec<_>>()
    );
    assert!(
        h2.state.identity.svid_for(&alloc0).is_none()
            && h2.state.identity.svid_for(&alloc1).is_none(),
        "the old leaf key is NOT recoverable post-restart — svid_for reads None before re-issue",
    );

    // AND the svid-lifecycle convergence loop ticks: `actual = ∅` so every
    // still-Running alloc matches `running ∧ ¬held → IssueSvid` and is re-issued
    // (bounded recovery), each through the real executor (mint + audit + hold).
    tick(&h2, 4).await;
    tick(&h2, 5).await;

    // THEN each still-Running alloc is re-held with its pure-derived identity.
    let held_after = h2.state.identity.held_snapshot();
    let f0 = held_after.get(&alloc0).unwrap_or_else(|| {
        panic!("alloc0 re-issued; held: {:?}", held_after.keys().collect::<Vec<_>>())
    });
    let f1 = held_after.get(&alloc1).unwrap_or_else(|| {
        panic!("alloc1 re-issued; held: {:?}", held_after.keys().collect::<Vec<_>>())
    });
    assert_eq!(f0.spiffe_id, id0, "re-issued alloc0 holds its pure-derived identity");
    assert_eq!(f1.spiffe_id, id1, "re-issued alloc1 holds its pure-derived identity");

    // AND each re-issue leaves a FRESH issued_certificates audit row — the
    // per-identity audit count strictly increases after recovery.
    let audit_after = h2.state.obs.issued_certificate_rows().await.expect("read audit rows");
    let count0_after = audit_after.iter().filter(|r| r.spiffe_id == id0).count();
    let count1_after = audit_after.iter().filter(|r| r.spiffe_id == id1).count();
    assert!(
        count0_after > count0_before,
        "recovery re-issue of alloc0 writes a FRESH audit row ({count0_before} → {count0_after})"
    );
    assert!(
        count1_after > count1_before,
        "recovery re-issue of alloc1 writes a FRESH audit row ({count1_before} → {count1_after})"
    );

    // AND the ACTUAL re-issued+held chain still verifies under `openssl verify`
    // (exit 0) — the leaf material IdentityMgr now holds for the recovered
    // identity (read through `IdentityRead::svid_for`) is accepted by a relying
    // party. Root + intermediate from the SAME CA; the leaf is the real HELD
    // material the recovery executor minted, NOT a fresh re-mint (Finding 3). The
    // held material's structured identity matches the recovered identity.
    let node = NodeId::new(NODE_NAME).expect("valid NodeId");
    let root = h2.state.ca.root().expect("RcgenCa::root self-signs a real P-256 root");
    let intermediate =
        h2.state.ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");

    let held0 = h2.state.identity.svid_for(&alloc0).unwrap_or_else(|| {
        panic!("IdentityRead::svid_for must return the re-held SVID for alloc0")
    });
    assert_eq!(
        held0.spiffe_id(),
        &id0,
        "the re-HELD SVID material's identity must be the recovered SpiffeId::for_allocation"
    );

    let pem_dir = TempDir::new().expect("pem tempdir");
    let root_pem = pem_dir.path().join("root.pem");
    let inter_pem = pem_dir.path().join("intermediate.pem");
    let svid_pem = pem_dir.path().join("held-svid.pem");
    std::fs::write(&root_pem, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem, intermediate.cert_pem().as_pem().as_bytes())
        .expect("write intermediate.pem");
    // The ACTUAL re-held leaf cert — not a fresh mint (Finding 3).
    std::fs::write(&svid_pem, held0.cert_pem().as_pem().as_bytes()).expect("write held-svid.pem");

    let output = std::process::Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem)
        .arg("-untrusted")
        .arg(&inter_pem)
        .arg(&svid_pem)
        .output()
        .expect("invoke openssl verify");
    assert!(
        output.status.success(),
        "openssl verify of the re-HELD chain must exit 0 after restart recovery: \
         stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// `@real-io` `@error` `@regression` `@D10` `@FINDING-2` -- REGRESSION ANCHOR for
/// the rev-5 D10 fix (formerly the characterized
/// `restart_after_successful_issue_before_clear_stalls_reissue` defect). A
/// SUCCESSFUL issue still persists a retry entry (record-on-emit +
/// persist-`next_view`-before-dispatch, ADR-0035 §5 7→8). If the control plane
/// crashes after the issue succeeded (SVID minted, `issued_certificates` audit
/// row written — audit-before-hold, ADR-0063 D6 — held in-memory) but BEFORE the
/// next converged tick clears the retry entry, restart sees an EMPTY held set
/// (the held set is volatile, dies with the process — ADR-0063 D9) PLUS the
/// durable, stale retry entry.
///
/// # What this test now pins (the FIX, not the defect)
///
/// Under D10 the runtime projects a DURABLE `ever_issued` signal into the
/// reconciler's `actual` from the surviving `issued_certificates` audit row. So
/// `¬held ∧ ever_issued` is the unambiguous restart marker and the reconciler
/// re-issues IMMEDIATELY, BYPASSING the backoff gate, AND clears the stale retry
/// entry — the stale entry can no longer suppress recovery (D10 invariants 1+2):
///
/// 1. **Immediately post-restart, at logical-0 (NO clock advance, INSIDE what
///    would be the backoff window), the alloc IS re-held** — recovery is
///    immediate, exactly as ADR-0067 D1 promises.
/// 2. **The stale retry entry is cleared** by the recovery tick — it never
///    persists as a live failure across the restart.
///
/// The crash is modelled the way `restart_reissues_*` models it — `drop(h1)`
/// then boot `h2` over the same redb files — but it ticks h1 EXACTLY ONCE (so the
/// retry entry is NOT cleared pre-crash; the crash lands in that window). The
/// clock is SHARED across the restart so `tick.now_unix` is deterministic
/// relative to the persisted `last_failure_seen_at` — and the test ticks h2 at
/// logical-0 (still inside the would-be backoff window), so a PASS proves the
/// gate was bypassed, not merely waited out.
///
/// The `¬held ∧ ¬ever_issued` never-succeeded case (a genuine failed issue still
/// backs off — D10 invariant 3) is proven at the pure-reconciler layer by
/// `unheld_never_issued_alloc_stays_backoff_gated_inside_window`
/// (`crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs`): the
/// real-CA harness here always SUCCEEDS (so it always writes an audit row →
/// always `ever_issued`), and cannot synthesise a never-issued-but-retry-pending
/// alloc without a fault-injecting CA double — which is exactly what the pure
/// layer exercises directly.
#[tokio::test]
async fn restart_after_successful_issue_reissues_immediately_via_ever_issued_audit_row() {
    use overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle;

    let tmp = TempDir::new().expect("tempdir");
    let ca: Arc<dyn Ca> = Arc::new(RcgenCa::new(Arc::new(OsEntropy), trust_domain_subject()));
    // A SHARED clock survives the simulated restart — load-bearing: the
    // persisted `last_failure_seen_at` was sampled from this clock during h1's
    // tick, and the post-restart backoff comparison reads `tick.now_unix` off the
    // SAME clock, so the deadline math is deterministic (a fresh-clock-per-harness
    // would make `now_unix` drift by real wall-time between h1 and h2 boots). The
    // test deliberately ticks h2 at logical-0, INSIDE the would-be backoff window
    // — so re-issue here can only happen by BYPASSING the gate (the D10 fix), not
    // by waiting it out.
    let clock = Arc::new(SimClock::new());

    let h1 = build_harness_with_ca_and_clock(&tmp, Arc::clone(&ca), Arc::clone(&clock)).await;

    let workload = WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId");
    let alloc = AllocationId::new("alloc-ws-c0").expect("valid AllocationId");
    let identity = SpiffeId::for_allocation(&workload, &alloc);
    let svid_name = ReconcilerName::new(<SvidLifecycle as Reconciler>::NAME).expect("valid name");

    // The alloc reaches Running; tick EXACTLY ONCE — the issue succeeds (held),
    // an issued_certificates audit row is written (audit-before-hold), and the
    // reconcile body records a retry entry on emit (attempts == 1,
    // last_failure_seen_at == now_unix). We DO NOT tick a second time, so the
    // entry is NOT cleared (the crash lands in this window).
    write_alloc_state(&h1, "alloc-ws-c0", AllocState::Running, 1).await;
    tick(&h1, 2).await;

    // The issue succeeded — the alloc is held pre-crash.
    let held_before = h1.state.identity.held_snapshot();
    assert!(
        held_before.contains_key(&alloc),
        "pre-crash the successful issue holds the SVID; held: {:?}",
        held_before.keys().collect::<Vec<_>>()
    );
    // The durable success signal — an issued_certificates audit row exists for
    // the identity (this is what `ever_issued` will project post-restart).
    let audit_before = h1.state.obs.issued_certificate_rows().await.expect("read audit rows");
    assert!(
        audit_before.iter().any(|r| r.spiffe_id == identity),
        "a durable audit row exists for the identity pre-crash (the ever_issued signal)",
    );
    // And — the defect's precondition (now harmless under D10) — a STALE retry
    // entry was persisted for the SUCCESSFUL issue (record-on-emit), not yet
    // cleared (no second tick).
    let persisted_retry = h1
        .state
        .runtime
        .loaded_svid_lifecycle_views_for_test(&svid_name)
        .expect("svid-lifecycle view map present")
        .get(&h1.target)
        .and_then(|v| v.retry.get(&alloc).cloned());
    assert!(
        persisted_retry.is_some(),
        "a retry entry is persisted on a SUCCESSFUL issue (record-on-emit) and survives the crash",
    );

    // CRASH: drop h1 (release redb locks, lose the in-memory held set). Boot h2
    // over the SAME redb files + SAME CA + SAME clock (still at logical 0).
    drop(h1);
    let h2 = build_harness_with_ca_and_clock(&tmp, Arc::clone(&ca), Arc::clone(&clock)).await;

    // Post-boot the held set is empty (volatile), but BOTH the stale retry entry
    // AND the durable audit row survive in redb.
    assert!(
        h2.state.identity.svid_for(&alloc).is_none(),
        "post-restart the held set is empty — the leaf key was never persisted",
    );
    assert!(
        h2.state
            .runtime
            .loaded_svid_lifecycle_views_for_test(&svid_name)
            .expect("view map present")
            .get(&h2.target)
            .and_then(|v| v.retry.get(&alloc).cloned())
            .is_some(),
        "the stale retry entry from the successful issue survives the restart in redb",
    );

    // Tick h2 at logical-0 (now_unix == unix_epoch == the persisted
    // last_failure_seen_at). The backoff gate WOULD compute deadline =
    // last_failure_seen_at + backoff_for_attempt(1) > now_unix and suppress — but
    // the `¬held ∧ ever_issued` branch (D10) bypasses the gate and re-issues NOW.
    tick(&h2, 3).await;
    tick(&h2, 4).await;

    // FACT 1 (the FIX) — the alloc IS re-held immediately, with NO clock advance.
    // Recovery is immediate (ADR-0067 D1), driven by the durable audit-row
    // ever_issued signal, NOT delayed by the backoff window.
    let held_after = h2.state.identity.svid_for(&alloc).unwrap_or_else(|| {
        panic!(
            "D10 FIX: immediately post-restart at logical-0 (INSIDE the would-be backoff window) \
             the alloc is re-issued — the ¬held ∧ ever_issued branch bypasses the backoff gate \
             that the stale retry entry would otherwise impose"
        )
    });
    assert_eq!(
        held_after.spiffe_id(),
        &identity,
        "the immediately-re-held SVID carries the recovered identity",
    );

    // FACT 2 — the stale retry entry is CLEARED by the recovery tick (it never
    // persists as a live failure across the restart; D10 invariant 2).
    let retry_after = h2
        .state
        .runtime
        .loaded_svid_lifecycle_views_for_test(&svid_name)
        .expect("view map present")
        .get(&h2.target)
        .and_then(|v| v.retry.get(&alloc).cloned());
    assert!(
        retry_after.is_none(),
        "the ever_issued recovery branch CLEARS the stale retry entry; it survived as: {retry_after:?}",
    );
}

// ---------------------------------------------------------------------------
// Harness — a control-plane convergence harness with a REAL RcgenCa + a REAL
// LocalObservationStore, svid-lifecycle registered, cgroup-free.
// ---------------------------------------------------------------------------

struct Harness {
    state: AppState,
    target: TargetResource,
    reconciler_name: ReconcilerName,
    start: Instant,
    deadline: Instant,
}

async fn build_harness(tmp: &TempDir) -> Harness {
    // REAL built-in CA — RcgenCa does real P-256 crypto (the `Ca` port the
    // IssueSvid executor dispatches through).
    let ca: Arc<dyn Ca> = Arc::new(RcgenCa::new(Arc::new(OsEntropy), trust_domain_subject()));
    build_harness_with_ca(tmp, ca).await
}

/// Build a convergence harness over `tmp` with an explicitly-supplied CA and a
/// FRESH (empty) `IdentityMgr`. Calling it twice over the SAME `tmp` + SAME `ca`
/// — but a fresh `IdentityMgr`/runtime/`AppState` each time — is the honest
/// control-plane-restart simulation S-WIM-12 needs: the redb obs/intent stores
/// persist across the "restart" while the in-memory held set is lost (the leaf
/// key is non-persistable, ADR-0063 D9).
async fn build_harness_with_ca(tmp: &TempDir, ca: Arc<dyn Ca>) -> Harness {
    build_harness_with_ca_and_clock(tmp, ca, Arc::new(SimClock::new())).await
}

/// Build a convergence harness with a CALLER-SUPPLIED `SimClock` so a test can
/// drive logical time deterministically (the Finding-2 characterization needs
/// the SAME clock to survive the simulated restart, since the persisted retry
/// memory's `last_failure_seen_at` is compared against `tick.now_unix =
/// UnixInstant::from_clock(state.clock)`). The default-clock callers
/// (`build_harness` / `build_harness_with_ca`) construct a fresh clock per
/// harness, matching the production "each boot is its own clock" shape.
async fn build_harness_with_ca_and_clock(
    tmp: &TempDir,
    ca: Arc<dyn Ca>,
    sim_clock: Arc<SimClock>,
) -> Harness {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime composes");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(svid_lifecycle()).await.expect("register svid-lifecycle");

    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    // REAL observation store over redb — the issued_certificates audit row is
    // written + read back through the production LocalObservationStore (the
    // ca_boot_and_audit shape). Reopening the same path post-restart re-reads
    // the persisted alloc_status + audit rows.
    let obs: Arc<dyn ObservationStore> =
        Arc::new(LocalObservationStore::open(tmp.path().join("obs.redb")).expect("open obs store"));

    let node_id = NodeId::new(NODE_NAME).expect("valid NodeId");
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver;

    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);

    let state = AppState::new(
        store,
        tmp.path().join("intent.redb"),
        obs,
        Arc::new(runtime),
        driver,
        sim_clock,
        Arc::new(SimDataplane::new()),
        ca,
        Arc::new(IdentityMgr::new(None)),
        node_id,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    let target = TargetResource::new(&format!("job/{WORKLOAD_NAME}")).expect("valid target");
    let reconciler_name = ReconcilerName::new(
        <overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle as Reconciler>::NAME,
    )
    .expect("valid reconciler name");

    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    Harness { state, target, reconciler_name, start, deadline }
}

/// Run one svid-lifecycle convergence tick at `tick_n`.
async fn tick(h: &Harness, tick_n: u64) {
    run_convergence_tick(
        &h.state,
        &h.reconciler_name,
        &h.target,
        h.start + Duration::from_millis(tick_n.saturating_mul(100)),
        tick_n,
        h.deadline,
    )
    .await
    .unwrap_or_else(|e| panic!("convergence tick {tick_n} failed: {e:?}"));
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

/// Write an `AllocStatusRow` for `alloc_raw` through the `ObservationStore`
/// port — the churn driver. A later write at a higher `counter` wins under LWW.
async fn write_alloc_state(h: &Harness, alloc_raw: &str, state: AllocState, counter: u64) {
    let writer = NodeId::new(NODE_NAME).expect("valid writer NodeId");
    let row = AllocStatusRow {
        alloc_id: AllocationId::new(alloc_raw).expect("valid AllocationId"),
        workload_id: WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId"),
        node_id: NodeId::new(NODE_NAME).expect("valid NodeId"),
        state,
        updated_at: LogicalTimestamp { counter, writer },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
    };
    h.state
        .obs
        .write(ObservationRow::AllocStatus(Box::new(row)))
        .await
        .unwrap_or_else(|e| panic!("write alloc_status row for {alloc_raw}: {e:?}"));
}
