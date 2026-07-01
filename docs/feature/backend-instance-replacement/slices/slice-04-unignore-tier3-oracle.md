# Slice 04 — Un-`#[ignore]` the Tier-3 oracle (S-DBN-WS-STABLE + S-DBN-CHURN + S-DBN-NXDOMAIN-02 RECOVERY) — TERMINAL

> DISCUSS brief (2026-06-29). Feature: `backend-instance-replacement` (#249).
> **Terminal verification gate for US-BIR-1 + US-BIR-2 — NOT a user story** (a green test
> run is the oracle/DoD, not a user-invocable operator outcome).
> Job: J-OPS-003 (extended). **TERMINAL SLICE — lands the feature against its oracle.** Effort: ≈0.5d.
> Drives the production **`overdrive workload restart <id>`** verb (per ADR-0073) — built by slices 01–03.

## Goal (one line)

With the replace action landed, un-`#[ignore]` the **three** #249-deferred acceptance tests
(across **two** files) and confirm all GREEN on the pinned-6.18 Tier-3 matrix, driving the
production replace action — no test-only wiring standing in for the verb.

## Learning hypothesis

The three ATs were `#[ignore]`'d *solely* because "cycling/recovering the backend to a NEW
AllocationId/`workload_addr` while the job stays declared needs a replace/restart verb that
does not exist." With the verb landed (slices 01–03), replacing their
`stop_and_converge` + same-spec-redeploy cycle/recovery with the production replace action
makes all three pass. The three ATs:

- `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`:
  - `answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend`
    (**S-DBN-WS-STABLE**) — `alloc_b1 ≠ alloc_b2`, `f1_again == f1`, post-cycle dial lands `B2`,
    the `lo:SERVICE_PORT` 0x17 oracle confirms TLS 1.3.
  - `in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend`
    (**S-DBN-CHURN**) — in-flight fails fast within `CHURN_BOUND`; subsequent fresh connect lands `B2`.
- `crates/overdrive-control-plane/tests/integration/dns_responder_nxdomain.rs`:
  - `recovered_job_after_stop_resolves_to_the_same_stable_frontend`
    (**S-DBN-NXDOMAIN-02 RECOVERY leg**) — after the SAME `<job>` recovers to Running-AND-HEALTHY
    post-stop, `getent` re-resolves the **same stable `F`** (the withhold-not-release Tier-3
    `getent` recovery observable; NXDOMAIN-while-stopped → resolving-the-same-`F`-on-recovery).
    The withhold-not-release F-retention invariant is *already* Tier-1 mutation-gated at 01-04
    (S-DBN-FRONTEND-03 / S-DBN-IDX-02); only this Tier-3 `getent` recovery observable is #249-blocked.

**Predicted:** all three GREEN — the cycle / churn / recovery each route through the production
replace action and the existing assertions hold (`alloc_b1 ≠ alloc_b2`, `f1_again == f1`,
post-cycle/post-churn dial lands `B2`, the recovered `<job>` re-resolves the SAME `F`).

## Thinnest change

In each of the three ATs: replace the `stop_and_converge(&skeleton, <id>)` +
`deploy_and_wait_running(… <kind>_service_spec …)` cycle/recovery with the production
**`overdrive workload restart <id>`** action (`POST /v1/jobs/:id/restart`, per ADR-0073);
remove the `#[ignore = "…#249…"]` attribute. The rest of each AT (the
assertions, the wire-capture / `getent` oracle) is unchanged.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — all three ATs drive `run_server_with_obs_and_driver` + `POST /v1/jobs` + the replace action + `getaddrinfo`/`getent` + connect; the existing assertions are the oracle.
- **Thinnest?** Yes — un-ignore + swap the cycle/recovery mechanism; no new assertions invented.
- **No `#[test]`-only composition?** The replace action is the production path all three ATs drive — none hand-clears an intent key nor supplies the replacement production omits (CLAUDE.md vertical-slice rule).

## Acceptance (= terminal verification-gate criteria; proves US-BIR-1 + US-BIR-2)

- [ ] All **three** `#[ignore = "…#249…"]` attributes removed: `answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend` + `in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend` (in `dns_responder_walking_skeleton.rs`) AND `recovered_job_after_stop_resolves_to_the_same_stable_frontend` (in `dns_responder_nxdomain.rs`). The strings are *removed*, not rewritten — no stale #249 forward-pointer remains.
- [ ] All three ATs cycle/recover the instance via the **production `overdrive workload restart <id>` action** (`POST /v1/jobs/:id/restart`, per ADR-0073), NOT a test-only intent-key clear or a `stop_and_converge`-then-same-spec-redeploy that dead-ends.
- [ ] All three ATs GREEN on the pinned-6.18 appliance-kernel Tier-3 matrix (the merge gate; dev-Lima is necessary-but-not-sufficient).
- [ ] No AT installs/clears a rule/key, binds a socket, or supplies an address production does not itself install/clear/bind/supply.

## Dependencies

- **slices 01–03** (the full replace action + stable `F` + in-flight churn fail-fast).
- The three ATs (the oracle) — already present, `#[ignore]`'d to #249, across the two files above.
