# DELIVER Review — Step 01-02

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `01-02` — Shim registers supervision before gate release |
| Review iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Reviewed implementation commit | `64bae9bcd72443dbc798c294589c99493fd4f0a1` |
| COMMIT audit event | `cb8a4324ad143f2325bce5f6d00c4e9771a7716c` |
| Comparison base | `da9eea2bd2ad7d39a7ce7cdbaf7e13348b1187b6` |
| Authoritative contract | `feature-delta.md` D3, D4, B2; ADR-0255, ADR-0256, ADR-0266; approved roadmap step `01-02` |
| Review verdict | **APPROVED** |

## Executive summary

The `StartAllocation` and `RestartAllocation` Running paths now call `Driver::on_alloc_running` before `Driver::release_for_exit_emission`, exactly at the two sites pinned by D3. The added trait documentation is the approved ordering sentence; no public signature or other interface changed.

The `StopAllocation` arm calls `on_alloc_terminal` only after it has an exact durable stop row or its terminal-row write succeeds. A row read/write error returns before the probe hook, preserving the probe supervisor for retry. `FinalizeFailed` retains its existing hook, and `cleanup_restart_predecessor` gains none, matching D4 and B2.

The two D3 acceptance tests and seven named/randomized `hook_ownership` conformance runs were activated without weakening assertions. The test-support pacing adjustment was separately approved by the DISTILL review in `distill/review-hook-ownership-remediation.md`; it still drives the production observer and reads the production probe and observation-store outcomes. I found no reachable production defect, contract divergence, error-contract gap, or testing-theater issue in this step.

## Contract and production-path review

### D3 — registration precedes the exit-event gate

At the `StartAllocation` Running site, the shim now calls `driver.on_alloc_running(&spec)` at `action_shim/mod.rs:2673-2676`, before it calls `driver.release_for_exit_emission(handle).await` at `:2677-2684`. The `RestartAllocation` successor uses the same order at `:3205-3206`. This matches the exact two-site requirement in `feature-delta.md:184-195` and ADR-0256 § Decision.

The production owner path is intact: `VmDriver::on_alloc_running` registers the real `ProbeRunner` supervisor (`vm_driver.rs:2063-2065`); the gate release sends only after its existing exec-release outcome (`vm_driver.rs:1988-2029`); and the exit watcher awaits that gate before cleanup and emitting the `ExitEvent` (`vm_driver.rs:2301-2335`). The new ordering closes the reviewed no-supervisor window without a tombstone, extra state, or a new lifecycle path.

The acceptance coverage enters the production `dispatch_with_network_provisioner` path and the production exit observer (`terminal_hook_ownership.rs:214-271`). The first D3 scenario checks call order for both start and restart successor (`:574-612`). The seeded immediate-exit scenario makes the event available at gate release and verifies that after the observer consumes it the supervisor count and every probe role are zero and the row is `Failed` (`:614-646`). These assertions are tied to the accepted D3 outcome.

### D4 and B2 — existing ending owners and durable stop row

`FinalizeFailed` still calls `on_alloc_terminal` at `action_shim/mod.rs:2174-2178`. `cleanup_restart_predecessor` still performs driver stop and resource cleanup without calling the terminal hook (`:1777-1805`). The new D3 work does not alter either owner.

For `StopAllocation`, `terminal_row_durable` becomes true only when the already-current row is the exact requested terminal row (`action_shim/mod.rs:3302-3305`) or the write returns an accepted occurrence (`:3359-3364`). The call to `on_alloc_terminal` is gated by that value (`:3374-3376`). Read/write errors return before that call (`:3295-3300`, `:3365-3371`). The test projection reads supervisor presence from the real `ProbeRunner` and terminal-row presence from `ObservationStore::alloc_status_row` (`quint_hook_ownership.rs:869-889`); the B2 conformance run includes a failed stop-row write that leaves the probe supervisor present, followed by a successful retry and hook.

The distinction between the two driver hooks is important here: `on_alloc_terminal` calls `ProbeRunner::stop_alloc` (`vm_driver.rs:2067-2069`), while `release_supervision` releases the VM live-map/beacon-launch claim (`:2055-2061`). The stop-row error branches still release the latter as existing cleanup requires, but do not stop the probe supervisor. Thus their cleanup does not weaken B2.

### Public API and typed-error contract

The `Driver::on_alloc_running` signature is unchanged. The only trait-file change is the D3 ordering sentence at `driver.rs:758-761`; `terminal_row_durable` and the conformance driver's `pending_exits`/`call_count` are private implementation or test-support details. No type, variant, method, parameter, wire format, persisted format, or lifecycle meaning was added.

D3 introduces no fallible hook or new error path. For B2, a terminal-row read/write failure remains the existing `ObservationStoreError`, carried by the existing `ShimError::Observation(#[from] ObservationStoreError)` (`action_shim/mod.rs:3587-3589`). The conformance fault uses `ObservationStoreError::Io(PermissionDenied)`; no diagnostic string or generic variant substitutes for a designed semantic distinction. No new typed error contract is required by this step, and no `SPECIFICATION_AMBIGUITY` exists.

## Test integrity and coverage

Only the two D3 acceptance `#[ignore]` markers were removed from `terminal_hook_ownership.rs`; their bodies and assertions are unchanged. The seven `quint_hook_ownership_*` runs were activated by removing their pending markers. All seven retain `CONTRACT_SHAPE: bounded-change` declarations at `quint_hook_ownership.rs:974-1053`. The test diff contains no deleted, weakened, or replaced assertions.

The conformance driver continues to drive the real action shim and exit observer with `Sim*` adapters at driven ports. Its `PacedDriver` wraps `SimDriver` and forwards the probe hooks to a real `ProbeRunner` (`quint_hook_ownership.rs:273-354`). `Consume` forwards one pending event to the production observer receiver and waits for that event's new release call (`:739-767`); the projection does not read `pending_exits`, call counts, or scripted faults (`:869-889`). The separate DISTILL review reproduced the two stale-event failures at seeds `0x1a5311b9` and `0x94ef88cb`, then approved this private pacing correction with both seeds passing. It changed no production API, Sim adapter seam, model, assertion, or projection.

### Verification evidence

The focused tests used the canonical Metal route:

```sh
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance --no-fail-fast -E 'test(terminal_hook_ownership::)'
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --lib --no-fail-fast -E 'test(quint_hook_ownership)'
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --lib --no-fail-fast -E 'test(action_shim::)'
cargo xtask metal run -- cargo clippy -p overdrive-control-plane --features integration-tests --all-targets -- -D warnings
cargo xtask metal run -- cargo fmt -p overdrive-core -p overdrive-control-plane -- --check
```

| Evidence | Result |
|---|---|
| Terminal-hook acceptance selection | 13 passed, 452 skipped (`.context/step-01-02-crafter-final-acceptance.log`); includes both D3 acceptance scenarios and existing hook controls |
| Hook-ownership conformance selection | 7 passed, 619 skipped (`.context/step-01-02-crafter-final-conformance.log`) |
| Action-shim controls | 46 passed, 580 skipped (`.context/step-01-02-crafter-final-action-shim-controls.log`) |
| Fixed-seed conformance reruns after the pacing correction | 1/1 passed for each of `0x1a5311b9` and `0x94ef88cb` (`.context/step-01-02-crafter-seed1a-fixed.log`, `step-01-02-crafter-seed94-fixed.log`) |
| Final warnings-denied Metal Clippy | Passed (`.context/step-01-02-crafter-final-clippy.log`); an earlier `redundant_clone` finding was removed before the final pass |
| Scoped formatting and `git diff --check` | Passed (`.context/step-01-02-crafter-final-fmt.log`; independent `git diff --check da9eea2..64bae9b` also passed) |

The DES record is sequential: RED/FAIL, GREEN/FAIL during the first conformance-driver attempt, GREEN/PASS after the separately reviewed test-support correction, then COMMIT/PASS (`deliver/execution-log.json`; the final event is recorded by `cb8a4324`). The earlier GREEN failure was an actual conformance-driver sequencing defect, reproduced at the named seeds and fixed without changing the oracle. No full-workspace pass is claimed. Mutation testing remains the single final DELIVER-wave gate and was not run per repository policy.

## Scope, attribution, and review disposition

The implementation commit changes the shim, the exact pinned trait documentation sentence, the two acceptance activations, the conformance-driver pacing/activations, and the DISTILL remediation evidence and approval. It adds no public API and does not change `cleanup_restart_predecessor`. Commit `64bae9b` retains Marcus as Git author and includes exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 01-02`, and the feature `Task-Id`; there is no Claude or Anthropic attribution. No `AGENTS.md` change was made by this review. The reviewer made no implementation change, DES event, test run, or commit.

## Findings and remediation disposition

No findings. No remediation is required. D3, D4, and B2 match the accepted contract; the public API and typed-error contract are complete for this step. No design gap or `SPECIFICATION_AMBIGUITY` is present.

## Final verdict

**APPROVED** — Step 01-02 implements the accepted registration ordering and stop-row ownership behavior, preserves the existing finalize and restart-cleanup ownership, activates the required scenarios without weakening them, and passes the focused retained verification. The only change between GREEN/FAIL and GREEN/PASS was the independently approved test-support pacing correction. Step 01-03 may proceed after the orchestrator completes its mechanical checks and confirms this on-disk review artifact.
