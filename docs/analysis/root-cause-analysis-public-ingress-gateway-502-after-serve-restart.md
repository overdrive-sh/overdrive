# RCA: public ingress gateway answers 502 for the whole recovery window after a graceful `serve` restart

Date: 2026-11-21. Branch `marcus-sa/workload-spec-sizes` (code identical to origin/main `e40002883` on this path).
Test: `tests/conformance/tests/public_ingress_gateway_end_to_end.rs:619`
`native_metal_kvm_public_ingress_gateway_answers_503_until_the_hydrator_reprograms_after_a_serve_restart` (S-PIG-71, serve-restart half).
Probe evidence: `spike-scratch/pig-restart-502/` (`probe.patch` = the throwaway diagnostics, already reverted; two full Metal logs). Kernel 7.0.0-38 (guest), Metal runs `4366bb437c7f2e29c0bc2ea2` and `04e34ec515a9483664e39076`.
Issue search (`gh issue list --state all --search "BackendIdentityConflict" / "serve restart 502 gateway identity" / "hydrator update_service refused ..."`): no existing issue or ADR names this. ADR-0241 (identity immutability) is the contract involved, and it does not discuss replacement at a reused endpoint.

## 1. Problem statement

After `world.restart()` (graceful shutdown, new `run_server` over the same data dir) every public `GET /ready` is answered 502 for 60 s; the backend 204 never arrives. `[(false,0,false),(true,502,true) x ~120]`.

Scope: only the post-restart half. The first-boot half of S-PIG-71 passes (`wait_until_serving` returns 204 before the restart), so the gateway, the real hook, the mTLS client and the guest are fine in a first boot.

## 2. Population diff (first boot vs post-restart)

| | first boot | after restart |
|---|---|---|
| allocation serving | `alloc-api-0` | `alloc-api-0` terminated (`platform_reclaimed`) at t=0; `alloc-api-1` Running at t~2.1 s (probe `PROBE-T`, log `w.log`) |
| dataplane `update_service` | succeeds | 60 x `dataplane.update_service.failed`: `backend id 1 is bound to spiffe://overdrive.local/workload/api/alloc/alloc-api-0, refusing spiffe://overdrive.local/workload/api/alloc/alloc-api-1` (same `service_id`/`fingerprint` every time) |
| gateway receipt | `selected`, mTLS ok | `receipt_outcome=selected backend_id=1 expected_peer_spiffe_id=.../alloc-api-0`, then `gateway-client mTLS failed: peer SPIFFE identity mismatch` (116 events) => 502 |
| SERVICE_MAP | outer entry present | outer entry present from t~0.5 s (`PROBE-SERVICE_MAP key: 02 00 60 0a 90 1f 06 00`, `Found 1 element`) |

Surface check (debugging.md s11): the empty-vs-populated surface here is SERVICE_MAP, which the test reads, and it is populated. The relevant surfaces are the gateway receipt (`GATEWAY_SELECTION_RECEIPT_MAP`, read through the `gateway.upstream.receipt.validated` event) and the dataplane identity table; `live_gateway_counts` is `(0,0)` after the request storm because every dial cleans up. The "503 until hydrated" half of the test is not what fails: the failure is a populated-but-wrong entry.

## 3. Five Whys

**WHY 1 (symptom). Why 502 every time?** The upstream connect fails after the hook selected a backend: `GatewayUpstreamConnectError::Mtls(PeerSpiffeMismatch)` projects to 502 (`crates/overdrive-gateway/src/runtime/exchange.rs:244-253`; a `Selected` receipt + failed dial is `Connect` 502, `connector.rs:918`). Evidence: `PROBE-TRACE gateway.upstream.connect_failed {"error": "gateway-client mTLS failed: peer SPIFFE identity mismatch"}` x113, each preceded by `receipt.validated ... backend_id 1 ... expected_peer_spiffe_id=.../alloc-api-0` (`/tmp/w.log`, `z.log`). The first request (status 0) is before the outer entry exists (`(false,0,false)`).

**WHY 2. Why does the Service map hand out a backend whose identity is the dead `alloc-api-0`?** The dataplane programmed `alloc-api-0` (the persisted `service_backends` row of the previous incarnation) after boot, and later refused to replace it with `alloc-api-1`. The new `EbpfDataplane` starts with an empty id memo and identity table (`lib.rs:689,697`), and the SERVICE_MAP pin is unlinked and recreated empty at construction (`lib.rs:423`), so the only way `backend id 1 -> alloc-api-0` exists in this process is an `update_service` that applied `alloc-api-0`. That is the hydrator's boot resync (`convergence.evaluation.admitted ... service-map-hydrator tick 2`, seq 744, before alloc-api-1 existed) reading the stored row (`service_map_hydrator.rs` `hydrate_desired`, `service_backends_rows`). Every later attempt carries `alloc-api-1`.

**WHY 3. Why is the update to `alloc-api-1` refused forever?** `plan_service_apply` (`crates/overdrive-dataplane/src/lib.rs:1359-1410`) looks up the endpoint in the allocator memo (`existing_id`, `allocators/backend_id.rs`), finds id 1, finds the identity bound to `alloc-api-0`, and returns `DataplaneError::BackendIdentityConflict` (`lib.rs:1402`; type at `crates/overdrive-core/src/traits/dataplane.rs:69-71`) before any mutation. Both allocations sit on the same endpoint (`100.95.0.2:8080`; the freed workload address is handed to the replacement), so the memo hit is the same id. The only operation that frees the memo entry is a withdraw/orphan sweep (`withdraw_service` `lib.rs:1743`, `settle_cleanup` -> `allocator.release`); a direct in-place A->B at one endpoint is never accepted.

**WHY 4. Why did the row go A -> B directly instead of A -> empty -> B?** `ServiceLifecycle`, the sole author of `service_backends` rows (`service_lifecycle.rs:1113-1185`: backends are only Running allocs, so alloc-api-0 terminated would write an empty row), did not evaluate while alloc-api-0 was terminated and alloc-api-1 was not yet Running. Its `workload/api` evaluation was admitted at seq 1509 with `queue_ms 2323`, after `workload-lifecycle` for the same target finished a 1978 ms evaluation that ran the VM create/ready inline (seq 741 admitted, seq 1508 completed; `vm.lifecycle.create_enter` 976 ... `ready` 1504). When ServiceLifecycle finally ran, alloc-api-1 was Running and the projection was `{alloc-api-1}`, which replaced the stored `{alloc-api-0}` row directly. Hydrator tick 7 (seq 1511-1514) then tried `{alloc-api-1}` against the programmed `{alloc-api-0}` and was refused; it retries each second (ticks 12, 14, 16, ... every ~1 s) with the same fingerprint `14778923225418625898` for the full window.

**WHY 5 (root causes).**
- **RC-1 (primary, design flaw): the dataplane contract forbids the normal replacement case.** The `Dataplane::update_service` identity rule (ADR-0241/R-3: a `BackendId` is immutable per `Backend.alloc`) is implemented as "the endpoint memo maps endpoint -> id, and a different alloc at a memoised endpoint is rejected". But the platform reuses a freed workload address for a replacement allocation (a crash restart, a liveness restart and a serve-restart reclaim all produce a new alloc id on the same endpoint). The identity rule should have been satisfied by minting a new id for the new identity (the old id stays immutable and readable for in-flight receipts); instead the memo conflates "endpoint" with "identity", so the rule turns a legitimate change into a terminal error that the hydrator can only retry. Nothing in the design handles it: `BackendIdentityConflict` is surfaced only as a log line (`action_shim/dataplane_update_service.rs:66-75`), the hydrator retries the same set with backoff (`service_map_hydrator.rs` View `RetryMemory`), and no component can ever clear the memo except an intermediate empty update.
- **RC-2 (contributing, ordering): the corrective path is sensitive to evaluation ordering.** The empty intermediate update that would have released the memo only happens if `service-lifecycle` evaluates between the old alloc's termination and the new alloc's Running. Per-target evaluations for `workload/api` queue behind `workload-lifecycle`'s inline VM start (seq 741 -> 1508), so the window is collapsed on this path. That ordering is incidental, not a guarantee anybody designed or tested; RC-1 must not depend on it.
- **RC-3 (contributing, boot): a fresh dataplane is programmed from the previous incarnation's stored row** (alloc-api-0 is already reclaimed: `vmm.process.reaped signal 9`, seq 728; `state: terminated, platform_reclaimed`) before `ServiceLifecycle` revalidates it. This is the converge-on-boot design (stale row -> hydrator resync) and is why requests during t=0.5-2 s are 502 rather than 503. It is benign by itself: the test only requires 503 when the entry is absent before and after, and the recovery requirement is a later 204.

## 4. Backward validation

- RC-1 + RC-2 => the first `update_service` after boot programs `{alloc-0}` (id 1, memo endpoint->1) and the second, `{alloc-1}` at the same endpoint, is refused identically on every retry. Observed: 60 identical failures, `fingerprint` constant, `backend id 1 is bound to ... alloc-api-0`.
- A programmed `{alloc-0}` entry + a live guest owned by `alloc-api-1` => the hook selects id 1, the connector expects the alloc-0 SPIFFE, the guest presents alloc-1's => `PeerSpiffeMismatch` => 502. Observed 113 times.
- RC-3 explains the outer entry existing at t~0.5 s with `alloc-api-1` not yet Running (`(true,502,true)` from the second poll) and the first poll `(false,0,false)`.
- No contradiction between branches. Not explained and not claimed: why 3 initial `connect_failed` events carry no mTLS detail (they precede the outer entry/receipt; consistent with the status-0 first request and the pre-entry connect refusals).

## 5. Contributing factors / history

- **When it started failing: not bisected.** No retained Metal evidence of this test passing was found (`verification/`, `docs/evolution/` carry no S-PIG-71 capture); the only record is `docs/evolution/2026-11-21-workload-resources-human-readable.md:67` ("fails on main before this feature (it stays 502 after a serve restart)"). The test was authored in `bc0ed58b4` (Nov 12) and revised in `4a1b4d2ca`/`81748a1aa`. A bisect would need a second Metal checkout with its own guest kit; judged not cheap. Candidates that change the path: hydrator rework Nov 17 (`31e5044e1`, `b229f69ad`, `4ebc41233`: diff against the programmed fingerprint, resync pass), Nov 18 (`de2e5f5bd`: update failure is only a log line), and vm-reclamation boot reclaim (alloc replaced on restart). Whether the pre-Nov-17 hydrator (View-marker dedup) hid the refusal or just failed differently is unknown.
- **The contract that causes it is pinned as correct by tests.** `update_rejected` / run `conflicting_alloc_rejected` in the QC-3 Quint model (`specs/quint/...selection_receipt`), `gateway_connect_dataplane_equivalence.rs` S-PIG-67 and the Sim equivalence (`overdrive-sim/src/adapters/dataplane.rs`) all assert rejection of a different alloc on a memoised endpoint, as a defensive case; none composes it with a replacement allocation at the same address, so everything is green while the composed lifecycle is wrong (testing.md "prove composed system behavior").
- **Failure surfaced only as a log**: `dataplane.update_service.failed` has no operator-visible state, so a permanently refused hydration looks like a healthy Service with a wrong backend.
- Test-fixture contributor: the test's own oracle (`503 only when no entry`) is silent on the stale-entry window, so the only failing assertion is the 60 s recovery.

## 6. Proposed fix (DESIGN decision required)

This is a design flaw in the `Dataplane::update_service` identity contract, not a typo-level bug; it needs a user decision before implementation (`.claude/rules/design.md` "Surface material design decisions"; CLAUDE.md "Implement to the design").

**Decision to put to the user:** when a Service update names a different `Backend.alloc` at an endpoint the allocator already memoised, `update_service` mints a fresh `BackendId` for the new identity and replaces the endpoint memo, instead of returning `BackendIdentityConflict`. The old id's applied identity stays immutable and readable (receipts already issued for it remain correct, preserving ADR-0241's actual invariant), its `BACKEND_MAP` entry becomes an orphan swept post-commit (`settle_cleanup`, `lib.rs:~1770`; `allocator.release(old_id)` removes only memo entries that point at the old id, `backend_id.rs:84-86`, so it does not disturb the new mapping). The reverse-NAT key is the same endpoint under the same VIP, so R-59 is not triggered. Rollback restores the previous memo entry via `ServiceApplyUndo`.

Alternatives considered:
1. *Make the hydrator withdraw-then-apply on a conflict* (empty update, then the set): adds a window with no backend (503) for every replacement and encodes dataplane internals in a reconciler; a fallback around the rule. Rejected.
2. *Release the memo when an alloc terminates* (ServiceLifecycle emits an empty row first): depends on evaluation ordering (RC-2), which is exactly what failed here. Rejected as the fix; useful only as a latency improvement.
3. *Never reuse the freed workload address across allocations*: not a fix for the contract and changes address-lifecycle semantics for all workloads.

Specific code changes (for the crafter once the contract is approved):
- `crates/overdrive-dataplane/src/lib.rs` `plan_service_apply` (~1390-1410): replace the `BackendIdentityConflict` return with a plan entry `PlannedBackend::Rebound` that evicts `(ip,port,proto)` from the allocator memo and mints a new id (new allocator method e.g. `rebind`, `backend_id.rs`), reserving the new identity as `Reserved`; record the evicted memo entry in `ServiceApplyUndo` for rollback.
- `crates/overdrive-core/src/traits/dataplane.rs`: the `update_service` trait docstring (preconditions/edge cases per development.md "Trait definitions specify behavior") and `BackendIdentityConflict` (delete if no producer remains, single-cut, per deletion discipline).
- `crates/overdrive-sim/src/adapters/dataplane.rs`: Sim follows the same rule (equivalence).
- `specs/quint/.../selection_receipt.qnt` + `checks.toml`: the `update_rejected` / `conflicting_alloc_rejected` actions become a rebinding action; re-run `cargo xtask quint check`; update the QC-3 driver and the equivalence tests that asserted rejection (S-PIG-67), ADR-0241 text (via the architect, not inline).

Immediate mitigation (not the fix, restores nothing durable): none that is safe without changing production; a test-only workaround (stop the Service before restart) would hide the defect.

Optional, separate and needing approval (per design.md "agents propose"): (a) emit an operator-visible state when `dataplane.update_service.failed` repeats for the same fingerprint; (b) have `ServiceLifecycle` not queue behind a VM start. Neither is needed once RC-1 is fixed.

## 7. Files affected

`crates/overdrive-dataplane/src/lib.rs`, `crates/overdrive-dataplane/src/allocators/backend_id.rs`, `crates/overdrive-core/src/traits/dataplane.rs`, `crates/overdrive-sim/src/adapters/dataplane.rs`, `crates/overdrive-dataplane/tests/integration/gateway_connect_dataplane_equivalence.rs`, `crates/overdrive-dataplane/src/quint_selection_receipt.rs`, `specs/quint/*selection_receipt*`, ADR-0241 (+ feature-delta contract text for `Dataplane::update_service`). No change to gateway, connector, hook, or test fixtures.

## 8. Risk

Medium. Touches the commit-guarded Service-map transaction (ADR-0233/0241) and the model-checked receipt protocol; the identity-immutability invariant is preserved (old id never rebound), but the QC-3 Quint model and its hazard variants must be updated and re-checked, and the Tier-3 `IdentityImmutablePerBackendId` evidence re-run. Memory/GC: one extra `BACKEND_MAP` id per replacement until the post-commit sweep (same path as any removed backend). Rollback correctness (memo eviction undo) needs a unit test. It is a design change: escalate to DESIGN with the numbered decision in section 6.

## 9. Regression test

The existing test is the right regression test and needs no change: `native_metal_kvm_public_ingress_gateway_answers_503_until_the_hydrator_reprograms_after_a_serve_restart` must go green (503/entry-absent, then 204 within 60 s). Additional coverage that does not exist and is required with the fix (composed, not isolated): (1) a Tier-1 seeded sim invariant in which an allocation is replaced at the same endpoint with no empty hydrator tick between (drive through `ServiceMapHydrator` + `SimDataplane`, assert the programmed fingerprint converges to the new set); (2) a Tier-3 `Dataplane` equivalence case (host vs Sim): `update_service([A@ep])` then `update_service([B@ep])` succeeds, `selected_backend_identity(old_id)` still returns A, new id returns B; (3) rollback of a failed rebinding restores the memo.

## 10. Method notes

Hypothesis/prediction/falsification used: H1 "stale entry from the old instance survives" falsified (pin recreated at `lib.rs:423`, memo empty at `lib.rs:689`; failing message is produced in the new instance); H2 "alloc-0 programmed by new instance then replacement refused" predicted `update_service.failed` with the conflict text and constant fingerprint: confirmed (60x). Probes were temporary edits to the test and `GatewayWorld` (`probe.patch`), reverted; no production code changed, nothing committed, no issues created.

## Regression bisect

Date: 2026-11-21. Question: the test passed when the public ingress gateway (GH #54) was delivered; what changed? Method: throwaway worktree `/tmp/pig-bisect` (since removed), one remote checkout, test run alone on native Metal, kernel 7.0.0-38.

**Environment drift handled first (not the regression).** On commits before `09b2fd406` the guest kit launches the outer test guest with the box's *current* Cloud Hypervisor binary (the Nov 20 fork rev, nested virtualization off by default) and no `nested=on`, so every guest-placed test fails at staging with `kvm_amd: SVM not supported by CPU 1` (run `cfb895b8c459968cf109f510`, commit `9de87b2a3`, exit 100, before the test body). To make old commits runnable, every bisect point carries one overlay: the `guest_kit.rs` part of `09b2fd406` (`/tmp/pig-nested.patch`, the `nested=on` shim for the outer guest). It touches only outer-guest launch, not `serve`, the hydrator or the dataplane. `bpf-build` and `guest-kit-prepare` were re-run where BPF/guest sources differed.

**Last known-good evidence.** The test was authored in `bc0ed58b4` (Nov 12; last body change `81748a1aa`). The repo holds no retained Metal capture of S-PIG-71 passing (the feature's execution log and evolution record name no run), so "used to pass" is established here by the bisect, not by a recorded artifact.

| Commit | Run id | Result |
|---|---|---|
| `9de87b2a3` (before the reconciler-convergence work), + nested overlay | `f10354eb8c73710f40eb6bf4` | PASS (23 s) |
| `07e2624f2` (managed_targets enumerated, no boot pass yet), + overlay | `28b107942b30a99196f6a270`, rerun `056b10cc9cffadfe4b74b633` | PASS twice (39 s, 17 s) |
| `4ebc41233` (boot-anchored resync pass), + overlay | `4cca172faf051b99446e2db7` | FAIL (78 s): `[(false,0,false),(true,502,true) x N]`, panic at `public_ingress_gateway_end_to_end.rs:665` — the same signature as origin/main `e40002883` (run `2f71bdc8077742591a88a719`) |
| `4ebc41233` + one-line experiment: first pass at `now + 30 s` instead of `now` (no boot pass) | `3222998ff73d9463ab5c17c7` | PASS (38 s) |

`4ebc41233^` is `07e2624f2`, so these two runs bracket one commit. Pass at the parent is repeated; the failure matches the three earlier failing runs (origin/main and the prior RCA's two), so this is not flakiness.

**First bad commit: `4ebc41233` "feat(reconcilers): boot-anchored resync pass replaces per-reconciler cadence and the router relist" (Nov 17, reconciler-convergence-guarantees step 03-02).**

**Mechanism (before vs after).**
- Before: the convergence loop built a cadence table from `Reconciler::resync_schedule()`; only `vm-reclamation` opted in (`lib.rs` at `07e2624f2`, doc of `spawn_convergence_loop`: "Every Phase-1 reconciler returns the default `None` EXCEPT `vm-reclamation`"). `service-map-hydrator` declares empty `interests()` and had no schedule. The removed router doc states it plainly: "on a `serve` restart `register` submits no initial evaluation". After a restart the hydrator therefore ran only when `ServiceLifecycle` wrote a `service_backends` row and enqueued it. The stale persisted row `{alloc-api-0}` was never programmed; the first programmed set was `{alloc-api-1}` (the live replacement), so the endpoint memo was empty and there was no `BackendIdentityConflict`. The gateway answered 503 (no outer entry) and then 204.
- After: `spawn_convergence_loop` starts a resync pass at boot (`lib.rs:5102`, `next_pass_at = now`, pass body `reconciler_runtime::run_resync_pass`, period `RESYNC_PASS_PERIOD` `lib.rs:5057`) that submits every reconciler's `managed_targets` Immediate. The hydrator's `managed_targets` (`service_map_hydrator.rs:324`, from `07e2624f2`) reads `all_service_backends_rows`, so the boot pass evaluates the stored previous-incarnation row, whose alloc `alloc-api-0` is already reclaimed, and programs it into the fresh dataplane (the prior RCA's RC-3). The replacement `alloc-api-1` then lands on the same endpoint and `plan_service_apply` refuses it with `BackendIdentityConflict` on every retry (prior RCA RC-1/RC-2).
- Falsification: disabling only the boot pass (experiment row above) turns the same tree green. Prediction made before the run: PASS if the boot pass is the trigger; it passed.

**Regression or exposure?** Both. `4ebc41233` is the trigger and changes observable boot behaviour: it is the commit that makes a stale row reach the dataplane at boot (and `31e5044e1`/`b229f69ad`, hydrator diffs the programmed fingerprint, are what make the refusal a permanent retry loop rather than a View-marker dedup; they passed at `07e2624f2` only because nothing woke the hydrator at boot). The defect it exposes is latent and older: the dataplane identity memo (ADR-0241) has always refused a different alloc at a memoised endpoint, the platform has always reused the freed workload address, and no test composed the two. The boot pass is the intended feature ("boot-anchored resync pass is the backstop for every reconciler"); its test coverage never included the S-PIG-71 serve-restart composition, and the 03-02 step's quality gate did not run this Metal test (the same pattern as noted in testing.md: a compile gate does not catch composed runtime behaviour).

**Recommended fix direction.** Do not revert `4ebc41233`: it implements the approved convergence guarantee, and reverting would reintroduce the "a lost wake strands a target until the next edge" defect it closes. Keep the prior RCA's design change (RC-1): a replacement at a memoised endpoint must mint a new `BackendId` instead of returning `BackendIdentityConflict`, with the composed Tier-1 sim invariant and the host/Sim equivalence case the prior RCA lists. Without it, any path that programs a stale row and then replaces its alloc at the same address (not only the boot pass) breaks. The bisect adds one constraint to that decision: a narrower alternative (skip the hydrator at boot, or order it behind `ServiceLifecycle`) would re-open the exact gap the boot pass was written to close and still leave the dataplane contract broken for crash restarts, so it is not recommended.

**Cleanup record.** Remote checkout created on the box: `/home/ubuntu/overdrive-worktrees/2625685adf22ea5b04b2ad8b00292f1c2ecaf9ca27cf1e17ce73ee55753be6fd` (plus per-run guest runtime state under `/run/ovd-gk/<run-id>` and its `target/`, which the orchestrator removes with the checkout). Local worktree `/tmp/pig-bisect` removed. No production code in the repository was changed; the overlay and the experiment existed only in the removed worktree.
