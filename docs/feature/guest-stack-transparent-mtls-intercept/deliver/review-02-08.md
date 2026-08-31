# DELIVER review — 02-08 Unwind post-assignment provision failure

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Roadmap step | `02-08` |
| Iteration | 1 |
| Commit reviewed | `ab855ee8b5377aa944e408dc8d36a52e0c99313d` (`fix(action-shim): unwind failed network provisioning`) |
| Review sources | `feature-delta.md` BTR-02/post-assignment contract; architecture brief BTR extension; DISTILL `S-GTI-BTR-02`; `deliver/roadmap.json` step `02-08` |
| Verdict | **NEEDS REMEDIATION** |

## Accepted contract checked

The binding design requires the existing allocation-keyed
`teardown_and_release_netns_raw` to run after an assigned provision failure
and before the existing Failed write. Successful teardown alone releases the
slot; a Failed write error wins over a captured cleanup error; otherwise that
existing cleanup error is returned; no aggregate error, new persistence, new
public/test-only production seam, `RestartNetworkDisposition`, or second
restart-cleanup protocol is permitted. This is stated in the feature delta
(`feature-delta.md:1587-1595`), architecture brief
(`brief.md:10342-10352`), and S-GTI-BTR-02
(`distill/test-scenarios.md:99-125`).

## Findings

### F-01 — restart path still implements the prohibited restart-cleanup protocol

**Severity:** High — binding design/API-shape divergence.

The fresh-start branch correctly calls the existing raw allocation-keyed
teardown, captures its error, writes the Failed row afterward, and applies
the specified precedence (`action_shim/mod.rs:1762-1809`). The changed
same-ID restart branch does not use that same bounded shape alone. Before
calling the raw teardown, it invokes `cleanup_restart_abort` with
`RestartNetworkDisposition::RetainForRetry`, then selects a result from both
that second cleanup result and the raw-teardown result
(`action_shim/mod.rs:2219-2276`). The helper and private enum are precisely a
restart-cleanup protocol (`action_shim/mod.rs:1350-1370`), expressly excluded
by the accepted design. The three-way return match also makes a successful
Failed write return the prior mTLS cleanup error in preference to the captured
structural cleanup error (`:2274-2275`), so restart does not have the single
cleanup-result/precedence model contracted for BTR-02.

**Production reachability:** `run_server` starts the convergence loop
(`lib.rs:3058-3078`), whose action execution reaches the real
`dispatch_with_workflow_intent` composition (`reconciler_runtime.rs:1718-1746`)
and then `dispatch`/`dispatch_single` (`action_shim/mod.rs:777-921`). A
production `RestartAllocation` with a non-terminal prior row passes the
driver-stop gate (`:2150-2185`), calls the C3 provision seam (`:2210-2215`),
and reaches this branch when `HostNetworkProvisioner::provision` returns a
typed netns error. With mTLS composed, `cleanup_restart_abort` awaits
`MtlsInterceptWorker::stop_alloc` (`:1363-1366`); that stop is fallible after
the worker has removed the intercept from its active map and awaits its task
and enforcement teardown (`mtls_intercept_worker.rs:992-1045`,
`:1524-1544`). The new code then continues to the raw structural teardown and
the Failed write. Graceful shutdown does not make this theoretical: it waits
for an active action dispatch to finish (`lib.rs:1410-1417`), and no retry or
detached-tail owner is involved.

**Required bounded remediation:** in the restart provision-failure branch,
remove the `cleanup_restart_abort(...RetainForRetry)` leg and its
three-result precedence logic. Match the fresh-start BTR-02 sequence exactly:
capture only `teardown_and_release_netns_raw`, then write the existing Failed
disposition, returning store error first, otherwise the captured structural
cleanup error, otherwise `Ok(())`. Do not add a replacement disposition,
aggregate error, persistence, public seam, or retry mechanism. The separate
prior-driver/mTLS/structural ordering is 02-09's explicitly scoped work and
must not be redesigned here.

### No additional findings

The changed test is a legitimate driven-port composition test, not fixture
theater. It enters the production action-shim dispatch form with only the C3
`WorkloadNetworkProvisioner` port substituted (`dispatch_with_network_provisioner`),
and observes the port contract at the provisioner, allocator, observation
write, result, and current-row boundaries. Its `ProvisionFailureNetwork`
double fails only after the production allocator has assigned the slot
(`action_shim_crash_observability.rs:205-236`); the scenario exercises both
Start and Restart success/cleanup-failure cases and verifies
`provision -> teardown -> failed-write`, slot retention/release, the typed
Failed cause, and no driver start (`:2130-2222`). The required declaration is
exactly `/// CONTRACT_SHAPE: bounded-change.` (`:2143`).

The test does not cover the prohibited mTLS/disposition path in F-01, so its
green result cannot validate that design constraint.

## Verification evidence

| Check | Result |
|---|---|
| `git show --check ab855ee8b5377aa944e408dc8d36a52e0c99313d` | Pass |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance post_assignment_provision_failure_tears_down_before_slot_release` | Pass — 1 test passed |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --test acceptance -- -D warnings` | Pass |
| Commit scope | Only the roadmap-listed action-shim production file and acceptance test changed. |

## Remediation disposition

Iteration 1 requires the original 02-08 crafter to resolve F-01 with the
bounded change above. Re-review must verify the restart branch has the same
single structural-cleanup capture and return precedence as fresh start, while
leaving 02-09's prior-protection ordering to its dedicated step.

---

## Iteration 2 — remediation re-review

| Field | Value |
|---|---|
| Remediation commit reviewed | `201ef67ada19b2e54c3e68384c8c1b9adca92d93` (`fix(action-shim): simplify restart provision unwind`) |
| Commit trailer | `Step-Id: 02-08` |
| Verdict | **NEEDS REMEDIATION** |

### F-01 disposition — resolved

The remediation removes `RestartNetworkDisposition` completely and reduces
`cleanup_restart_abort` to its one valid behavior: await mTLS teardown, then
perform structural teardown (`action_shim/mod.rs:1345-1360`). Its two
remaining callers are the existing later restart-abort paths; both use that
single structural-release behavior (`:2261-2277`, `:2318-2330`). No public
surface, persistence, retry owner, aggregate error, or test-only production
seam was added.

Most importantly, the post-assignment provision-failure path no longer calls
that helper. After the real restart route has passed its prior-row and driver
stop gates (`:2145-2174`) and `provision_and_inject_netns` fails
(`:2199-2207`), it captures only the existing
`teardown_and_release_netns_raw` (`:2208-2218`), then writes the existing
Failed disposition (`:2219-2231`). Its two-result match now gives the store
error precedence, otherwise returns the captured existing structural cleanup
error, otherwise succeeds (`:2232-2246`) — exactly the fresh-start sequence
at `:1751-1799` and the BTR-02 contract. The raw helper still invokes
provisioner teardown before releasing the allocation slot and leaves the slot
held on teardown failure (`:1317-1333`).

This removal is necessary and bounded: the prohibited retained-network mode
and its parameter were the sole mechanism that created the second
restart-cleanup protocol. `rg` confirms no `RestartNetworkDisposition` use
remains. The separate prior-protection ordering before replacement provision
is still 02-09's explicit scope; this remediation neither implements nor
redesigns it.

### F-02 — the dedicated integration lane remains red on a superseded restart contract

**Severity:** High — active regression failure and incomplete bounded test
fallout.

The remediation makes the intended BTR-02 production change, but leaves the
active integration test
`restart_provision_failure_cleans_prior_intercept_without_releasing_the_slot`
asserting the rejected retain-for-retry behavior
(`mtls_install_fail_closed.rs:1206-1223`). Its network adapter also asserts
that structural teardown may run only after the prior intercept has converged
(`:1068-1081`). At `201ef67a`, the BTR-02 restart branch correctly invokes raw
structural teardown without the prohibited mTLS cleanup leg, so the test
panics at that adapter assertion before it can observe the new release
outcome.

This is not an unrelated future test. DISTILL names this exact committed test
as superseded, says its retain-for-retry behavior is rejected, and requires it
to be transitioned with BTR-2/BTR-3 implementation
(`distill/red-classification.md:43-52`). The dedicated integration lane is a
CI surface (`.claude/rules/testing.md:133-137`), and the repository's tests are
permanent executable evidence rather than disposable scaffolding. Leaving it
red means the step has not completed its explicitly identified compiler/test
fallout even though the default-feature package suite is green.

**Reproduction through the real production entry:**

```text
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \
  --features integration-tests \
  -E 'test(restart_provision_failure_cleans_prior_intercept_without_releasing_the_slot)'
```

At `201ef67a` (with only the later review-document commit on top), nextest ran
the real `dispatch_with_network_provisioner` integration composition and
failed 1/1 at `mtls_install_fail_closed.rs:1078`:
`replacement network teardown must run only after prior interception teardown
converges`. The trace is production-reachable for the same reason as F-01: a
real same-ID `RestartAllocation` passes the prior-driver stop gate, receives a
typed failure from the replacement provisioner, and enters the BTR-02 raw
teardown branch.

**Required bounded remediation:** transition this one superseded provision-
failure case so its BTR-02 oracle no longer requires retain-for-retry and the
dedicated integration lane is green. Keep the edit limited to test fallout
required by BTR-02; do not implement 02-09's prior-protection-first production
ordering or broaden the production change. The later BTR-03 step remains
responsible for its final ordering trace.

### Re-review verification

| Check | Result |
|---|---|
| `git show --check 201ef67ada19b2e54c3e68384c8c1b9adca92d93` | Pass |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance post_assignment_provision_failure_tears_down_before_slot_release` | Pass — 1 test passed |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --test acceptance -- -D warnings` | Pass |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests -E 'test(restart_provision_failure_cleans_prior_intercept_without_releasing_the_slot)'` | **Fail** — 0 passed, 1 failed at `mtls_install_fail_closed.rs:1078` |
| Contract-shape/test-boundary audit | Pass — the existing C3 driven-port test still drives both production Start and Restart dispatch arms and carries the exact `/// CONTRACT_SHAPE: bounded-change.` declaration. |
| Scope audit | Production remediation is bounded; the explicitly identified superseded integration-test fallout is missing. |

### Final verdict

**NEEDS REMEDIATION.** F-01 is resolved and the production BTR-02 sequence is
correct on both action arms, but F-02 leaves an active, design-identified
integration regression red. Re-review is required after that bounded test
transition; 02-09 must not be used to absorb 02-08's incomplete regression
fallout.
