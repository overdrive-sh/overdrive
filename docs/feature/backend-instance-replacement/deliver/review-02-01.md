# Review: Backend Instance Replacement (Step 02-01)

**Reviewer:** Codex nWave review  
**Date:** 2026-07-01  
**Step:** `02-01` - Stable F across the cycle, un-ignore S-DBN-WS-STABLE driving the production verb  
**Artifact reviewed:** `02154a8c` plus current working-tree deliver metadata  
**AC gates:** Not run, per instruction

## Verdict

**APPROVED** for step `02-01`.

No blocker, high, or medium findings. The step is correctly limited to the declared oracle-unignore scope: one existing Tier-3 integration test was unignored, its blocked stop/redeploy cycle was replaced with the production restart route, no production source was introduced, and the sibling deferred oracle remains ignored for `03-01`.

## Findings

### Low: Active 02-01 Oracle Still Emits Stale `[02-02]` Diagnostics

**Location:** `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs:1739`, `1831`, `1847`

The active `S-DBN-WS-STABLE` test now belongs to backend-instance-replacement step `02-01`, but several `eprintln!` diagnostics still use `[02-02]`. This is not behaviorally load-bearing, and the stale `#[ignore = "...#249..."]` forward pointer was removed from the active test as required. The risk is only triage confusion when reading Tier-3 logs.

**Recommendation:** Update the three active-test diagnostic prefixes to `[02-01]` when convenient. This should not block the step.

## Scope And AC Coverage

- `S-DBN-WS-STABLE` is active: the prior `#[ignore = "02-02 DEFERRED ... #249 ..."]` attribute is gone from `answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend`.
- The cycle now drives `POST /v1/jobs/server/restart` through `run_server_restart`, matching the production route shipped in Phase 01.
- The test waits for a fresh Running allocation whose `alloc_id != alloc_b1`, then keeps the original oracle assertions: `f1_again == f1`, byte-exact post-cycle dial, F in `10.98.0.0/16`, F distinct from B1/B2, and inter-agent TLS `0x17` records with no cleartext markers.
- The step did not add mutable production source. The commit touched only the walking-skeleton integration test and deliver progress/log metadata, consistent with the mutation-skip criterion.
- The three-oracle split is preserved: `S-DBN-CHURN` remains ignored for `03-01`; the NXDOMAIN recovery oracle remains outside this step.

## Test Quality Review

The test is an existing Tier-3 oracle and still drives the production composition through real in-process HTTP, DNS resolution from the client netns, egress interception, backend translation, and wire capture. I did not find testing theater: the assertions check observable behavior at the system boundary, not private helper state.

The added polling for the fresh allocation is appropriate for the restart transition. It avoids re-capturing the old Running row during the drain window and preserves the load-bearing `alloc_b1 != alloc_b2` assertion.

## Execution Log

`execution-log.json` records `02-01` phases through `COMMIT` as `PASS`, and `.develop-progress.json` currently marks `02-01` completed with `03-01` and `04-01` pending. I did not independently execute the Tier-3 command or any AC gate because the request explicitly said not to run AC gates.
