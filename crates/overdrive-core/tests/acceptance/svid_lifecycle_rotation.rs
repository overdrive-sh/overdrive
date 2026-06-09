//! Acceptance tests for the `SvidLifecycle::reconcile` NEAR-EXPIRY ROTATION
//! branch per `built-in-ca-operator-composition` Slice ① (folds GH #40).
//!
//! Layer 1: pure reconciler scenarios (Tier-1 DST shape — pure fn over typed
//! `State`, no adapter, deterministic, default lane). These are DISTILL RED
//! scaffolds: the bodies `panic!` under `#[should_panic(expected = "RED
//! scaffold")]`; DELIVER replaces them with real assertions once the
//! rotation-as-action flip lands (gate consts deleted, `near_expiry` un-skipped,
//! the branch emits an unconditional `Action::IssueSvid` with a `"rotate-svid"`
//! correlation; threshold = ½ × `WORKLOAD_SVID_TTL` = 1800s).
//!
//! Settled design (feature-delta.md D-OC-1/2/3/8; `.claude/rules/workflows.md`):
//! internal SVID near-expiry reissue is a reconciler ACTION, NOT a workflow.
//! `running ∧ held(near-expiry) → Action::IssueSvid("rotate-svid")`
//! UNCONDITIONALLY. The `ROTATION_ENABLED` gate, `CERT_ROTATION_WORKFLOW` name,
//! and `StartWorkflow`/`WorkflowName` imports are deleted (single-cut). The
//! existing GREEN gated-seam test
//! `near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered`
//! (in `svid_lifecycle_reconcile.rs`) is DELETED in the same commit.
//!
//! RED scaffold convention (`.claude/rules/testing.md`): self-contained
//! `panic!` under `#[should_panic(expected = "RED scaffold")]`; no dependence
//! on a rotate API that does not yet exist. The pinned panic message names the
//! S-OC-NN scenario so the scaffold is greppable and the GREEN transition is
//! one visible commit.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

// S-OC-01 `@dst @property @driving_port @slice-1` — for an arbitrary `now` and
// arbitrary slack STRICTLY INSIDE the near-expiry window (`held.not_after <=
// now + WORKLOAD_SVID_TTL/2`), a `running ∧ held` allocation yields EXACTLY ONE
// `Action::IssueSvid` carrying the held `spiffe_id`, the running `node_id`, and
// a `"rotate-svid"` correlation — and ZERO `StartWorkflow` (the cert_rotation
// workflow no longer exists). Universe: the emitted action list + next View.
#[test]
#[should_panic(expected = "RED scaffold")]
fn near_expiry_held_alloc_emits_one_rotate_issue_svid() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-01 / near-expiry held alloc emits one \
         rotate-correlation IssueSvid, no StartWorkflow)"
    );
}

// S-OC-02 `@dst @property @error @driving_port @slice-1` — a `running ∧ held`
// allocation whose `not_after` is far-future (STRICTLY beyond `now +
// WORKLOAD_SVID_TTL/2`) emits NO `Action::IssueSvid`; the converged action
// vector for a single held-running alloc is exactly `[Noop]`. Universe: the
// emitted action list. Property over arbitrary `now` + arbitrary far-future
// `not_after`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn held_alloc_outside_near_expiry_window_emits_no_issue_svid() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-02 / held alloc outside the near-expiry \
         window emits no IssueSvid; converged tick is [Noop])"
    );
}

// S-OC-03 `@dst @error @driving_port @slice-1` — the near-expiry `<=` boundary
// is INCLUSIVE at half-TTL: `not_after == now + 1800s` rotates (emits one
// IssueSvid); `not_after == now + 1801s` does not. This is the LIVE mutation
// target (D-OC-8 — the `#[mutants::skip]` and the `.cargo/mutants.toml`
// exclude_re entry are removed in Slice ①): this scenario must KILL `<=`→`<`
// and `<=`→`==`. Two pinned boundary examples, NOT PBT. Universe: the emitted
// IssueSvid count across the two fixtures.
#[test]
#[should_panic(expected = "RED scaffold")]
fn near_expiry_boundary_is_inclusive_at_half_ttl() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-03 / near-expiry <= boundary inclusive at \
         now + 1800s, exclusive at now + 1801s -- the live mutation kill-test)"
    );
}

// S-OC-04 `@dst @driving_port @slice-1` — the rotation threshold TRACKS ½ ×
// `WORKLOAD_SVID_TTL` (with WORKLOAD_SVID_TTL = 3600s sourced from validity.rs),
// proven through the PORT-OBSERVABLE emitted action — NOT by inspecting a
// private threshold constant. Two TTL-derived boundary fixtures: a held alloc
// expiring at `now + WORKLOAD_SVID_TTL/2` emits exactly one rotate IssueSvid; a
// held alloc expiring at `now + WORKLOAD_SVID_TTL/2 + 1s` emits none. The
// fixtures are computed FROM the TTL const (not a bare `1800`), so a regression
// that hardcodes the threshold (ignoring a TTL policy change) flips the emit
// decision and reds this test. Distinct from S-OC-03 (literal `<=` boundary
// mutation kill-test): S-OC-04 proves the boundary tracks the TTL. Universe: the
// emitted IssueSvid count across the two TTL-derived fixtures (action list only).
#[test]
#[should_panic(expected = "RED scaffold")]
fn rotation_threshold_tracks_half_of_workload_svid_ttl_via_emitted_action() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-04 / rotation threshold tracks \
         WORKLOAD_SVID_TTL/2 via the emitted action: now + TTL/2 rotates, now + TTL/2 + 1s does \
         not -- no private-threshold inspection)"
    );
}

// S-OC-05 `@dst @property @error @driving_port @slice-1` — rotate is DISTINCT
// from restart-recovery re-issue. A `running ∧ held(near-expiry)` alloc emits a
// `"rotate-svid"`-correlation IssueSvid; a separate `running ∧ ¬held ∧
// ever_issued` alloc emits an `"issue-svid"`-correlation IssueSvid; neither
// emits any `StartWorkflow`, and the two correlations are distinct. Proves the
// rotate branch is NOT routed through the (deleted) gated seam and is
// independent of the restart-recovery branch (ADR-0067 rev 6 D10). Universe:
// the emitted action list partitioned by correlation purpose. Property over
// arbitrary near-expiry slack + arbitrary ever_issued membership.
#[test]
#[should_panic(expected = "RED scaffold")]
fn rotate_is_distinct_from_restart_recovery_reissue() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-05 / rotate (held near-expiry, rotate-svid \
         correlation) is distinct from restart-recovery re-issue (unheld ever_issued, \
         issue-svid correlation); no StartWorkflow from either)"
    );
}
