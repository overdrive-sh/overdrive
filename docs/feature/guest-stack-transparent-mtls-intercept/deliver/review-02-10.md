# DELIVER review — 02-10 Activate BTR-3 lifecycle-port invariant

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Roadmap step | `02-10` |
| Iteration | 1 |
| Implementation commit reviewed | `1b56d986fc932c1e27e7b8b2754c571774ae302c` (`feat(mtls): activate BTR-3 lifecycle port`) |
| Commit trailer | `Step-Id: 02-10` present |
| DES evidence | `RED`, `GREEN`, and `COMMIT` are recorded for `02-10`; integrity verifier reports all 13 step traces complete |
| Verdict | **NEEDS REMEDIATION** |

## Sources and contract

This review checked ADR-0089 section 7, including the exact public port,
production binding, socket-free Sim surface, and Tier-1 invariant
requirements; ADR-0076's retained lower worker boundary; the BTR-3 feature
delta; the BTR-3 wave decision; DISTILL S-GTI-BTR-03 and its RED
classification; the approved `02-10` roadmap amendment; and the independent
roadmap review. The binding evaluator requirements include exact initial and
final ownership, one replacement start for the same allocation, the
failure-cut states, and a pure checker whose negative control fails when
`TeardownPending` is treated as absence
(`adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md:561-595`,
`distill/test-scenarios.md:129-160`, `feature-delta.md:1657-1683`, and
`deliver/roadmap.json:1115-1141`).

No mutation command was run or requested. Mutation testing is expressly
outside this individual roadmap step.

## What conforms

The production contract shape matches ADR-0089 exactly. The action shim owns
only `MtlsInterceptLifecycle` with the two specified async methods, and the
implementation for `Arc<MtlsInterceptWorker>` delegates to the existing
inherent lifecycle owner (`crates/overdrive-control-plane/src/action_shim/mod.rs:64-133`).
Both public dispatcher forms accept the specified optional trait object
(`:848-888`, `:904-923`), and the production AppState wrapper explicitly
borrows the existing concrete Arc as the trait object (`:1081-1109`). The
server's concrete `mtls_worker_owner` and `shutdown_owner` composition remain
unchanged (`crates/overdrive-control-plane/src/lib.rs:1233-1234`, `:1369-1370`,
`:3133`, `:3195`). No wrapper owner, probe, retry/cancellation API, recovery
protocol, or lower-worker redesign was introduced.

The restart owner path is also correct: every prior driver stop completes
before `cleanup_restart_abort`; that helper awaits the lifecycle port before
the existing structural teardown and slot release
(`action_shim/mod.rs:2242-2256`, `:1423-1433`). Replacement provisioning,
identity, driver start, and lifecycle start remain afterward
(`:2281-2286`, `:2331-2360`, `:2364-2365`, `:2612-2629`). Existing typed
errors and fail-closed cleanup remain in that path.

`SimMtlsInterceptLifecycle` is socket-free and exposes exactly the sanctioned
Sim state, outcomes, fault insertion, and atomic snapshot surface
(`crates/overdrive-sim/src/adapters/mtls_intercept_lifecycle.rs:17-101`). Its
stop transition retains `TeardownPending` on a consumed stop fault and removes
the state only after a successful stop (`:103-131`); its repeated-start
failure records only `StartPriorTeardownFailed`, preserving the one-outcome
per public call rule (`:141-167`). The invariant drives real
`dispatch_with_network_provisioner` and establishes its initial state through
`StartAllocation`, not preload (`same_id_restart_lifecycle.rs:367-406`). The
Tier-3 real-worker listener/guard test remains separate and passed in the
`integration-tests` lane.

## Findings

### F-01 — The Tier-1 evaluator cannot enforce several required BTR-3 facts

**Severity:** High — blocking acceptance-evidence gap. This is not a claim of
a presently reachable production lifecycle failure; it is a concrete failure
of the required invariant oracle, which is itself the deliverable for this
step.

The current pure checker accepts only a bare cross-port trace. Its event
variants carry no allocation identity or lifecycle state
(`crates/overdrive-sim/src/invariants/same_id_restart_lifecycle.rs:52-60`),
and `check_order` finds the *first* occurrence of each required effect and
checks only that those six positions are increasing (`:430-455`). It therefore
cannot reject a different allocation ID, an additional driver or lifecycle
start after the first correctly ordered one, or a stale/partial lifecycle
snapshot. The current evaluator checks only one map lookup for its allocation
(`:421-423`, `:465-477`) rather than the required exact ownership snapshot.

More directly, the required `TeardownPending`-as-absence negative control is
not a possible input to `check_order`: the function receives only
`&[TraceEvent]`, while neither `TraceEvent` nor its caller supplies a lifecycle
snapshot. The only negative control deletes `LifecycleStopCompleted`
(`:457-475`). The nearby test merely executes the ordinary failure path and
asserts that the adapter remains `TeardownPending` (`:629-641`); it never
supplies a counterfactual pending-as-absence fact to a pure checker and cannot
demonstrate that required failure.

The same omission leaves the replacement-stage failure checks incomplete.
Provision, identity, and driver-start cuts assert only that later trace events
are absent (`:533-582`), while S-GTI-BTR-03 requires lifecycle absence at each
of those cuts (`distill/test-scenarios.md:137-150`).

**Evidence and reproducibility:** the mandated seeded run below passes on the
current implementation, but the lines cited above show that the passing
evaluator has no state/identity/cardinality input and contains no
pending-as-absence checker call. This is a bounded test-oracle nonconformance,
not an unproven hypothetical production execution. The exact accepted checks
are explicit: the checker must reject stale/partial ownership, more than one
replacement start, a different allocation ID, and convergence beyond one
redrive (`distill/test-scenarios.md:147-160`); ADR-0089 also requires the
pending-as-absence pure-checker control
(`docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md:584-592`).

**Required bounded remediation:** revise only the invariant-local pure
checker and its test inputs to consume the already sanctioned lifecycle
snapshot and observed trace facts, then make it reject the specified exact
same-ID ownership/cardinality and failure-cut conditions. Add the missing
counterfactual `TeardownPending`-as-absence negative control and prove it
fails. Preserve the existing production port, dispatcher composition,
socket-free adapter surface, lower-worker integration evidence, and all
existing error types. Do not add production APIs, a worker wrapper, a Sim
socket/worker, retry/cancellation/recovery machinery, or an additional cleanup
mechanism.

## Verification evidence

| Check | Result |
|---|---|
| `git diff --check 1b56d986fc932c1e27e7b8b2754c571774ae302c^ 1b56d986fc932c1e27e7b8b2754c571774ae302c` | Pass |
| `cargo xtask lima run -- cargo check -p overdrive-control-plane -p overdrive-sim --all-targets --features integration-tests` | Pass |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --test acceptance -E 'test(same_id_restart_removes_prior_protection_before_replacement_provision)'` | Pass — 1 test |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --lib -E 'test(same_id_restart_lifecycle)'` | Pass — 3 tests |
| `cargo xtask lima run -- cargo dst --seed 424242 --only same-id-restart-removes-prior-protection-before-replacement-provision` | Pass — invariant reported `status=pass` for seed `424242` |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests -E 'test(same_id_restart_real_worker_closes_prior_listener_and_drops_guard_before_stop_completion)'` | Pass — 1 independent real-worker test |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/guest-stack-transparent-mtls-intercept/deliver/` | Pass — all 13 steps have complete DES traces |
| Scope audit | Production and Sim port changes are bounded to the accepted ADR-0089 shape; finding F-01 is limited to missing invariant evidence. |

## Remediation disposition

F-01 is open and must return to the original `02-10` crafter. The remediation
is confined to the BTR-3 invariant's observation/checker evidence and must be
re-reviewed against the cited control cases. No production defect or
architecture change has been accepted, and no adjacent hardening is authorized.

## Final verdict

**NEEDS REMEDIATION.** The production lifecycle port, owner composition,
ordering, errors, Sim adapter, fixed-seed dispatch, and independent worker
evidence conform. The mandatory BTR-3 pure invariant, however, currently
cannot prove or negatively control several contract facts it is required to
own, including `TeardownPending` being treated as absence. The next roadmap
step must not begin until the original crafter resolves F-01 and this reviewer
approves the re-review.

---

## Iteration 2 — reproduction-gate re-review

The repository rule requires a concrete, bounded failure against the current
implementation before a remediation finding is actionable. I therefore
re-ran F-01 as a non-persistent in-module spike. No production API, test seam,
or source change remains after the reproduction; `git diff` was empty for the
temporary invariant file before this review update.

### Reproduced counterexample

The temporary test used `Fixture::new(424_242)`, drove its normal successful
`StartAllocation`, cleared the observation trace, then drove the same fixture's
normal `RestartAllocation` through `dispatch_with_network_provisioner`. It
appended one additional observed `LifecycleStartCompleted` to that completed
replacement trace and asserted that the current pure checker reject it. This
is the accepted simulated dispatcher path followed by the required
counterfactual checker input; it neither creates a test-only production state
nor alters the adapter or dispatch algorithm.

Command:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim --lib \
  -E 'test(reviewer_spike_checker_rejects_duplicate_replacement_lifecycle_start)'
```

Observed result: the one temporary test failed at
`same_id_restart_lifecycle.rs:638` with:

```text
the pure checker accepted a second replacement lifecycle start:
[DriverStop, LifecycleStopCompleted, NetworkTeardownAndSlotRelease,
 ReplacementProvision, DriverStartCompleted { identity_present: true },
 LifecycleStartCompleted, LifecycleStartCompleted]
```

This is a required invalid condition that the current checker accepts. The
DISTILL contract requires it to reject more than one replacement lifecycle
start (`distill/test-scenarios.md:147-150`), while its use of first-occurrence
positions (`same_id_restart_lifecycle.rs:430-455`) makes the observed false
acceptance reachable deterministically for seed `424242`.

### F-01 disposition — upheld, narrowed to the reproduced cardinality gap

F-01 remains **open** as a High, blocking acceptance-evidence finding because
the evaluator accepts a duplicate replacement lifecycle completion. The
bounded remediation is to make the invariant's pure checker reject the
duplicate replacement lifecycle-start trace and retain a permanent negative
test that fails against the pre-remediation implementation. This change stays
within the existing invariant observation/checker boundary; it must not add
any production surface or lifecycle mechanism.

The iteration-1 observations about a pending-as-absence *pure-checker*
control, exact snapshot rejection, different IDs, and later-cut lifecycle
absence are not separately actionable findings in this review: I did not
reproduce a current simulated-dispatch counterexample for each under the
repository reproduction gate. They are therefore withdrawn as remediation
requirements rather than treated as static suspicions.

### Re-review verification

| Check | Result |
|---|---|
| Temporary cardinality spike (command above) | Failed as required, proving the current checker accepts the invalid duplicate completion. |
| Source cleanup after spike | Pass — no diff remains for `crates/overdrive-sim/src/invariants/same_id_restart_lifecycle.rs`. |
| `cargo xtask lima run -- cargo dst --seed 424242 --only same-id-restart-removes-prior-protection-before-replacement-provision` | Pass — the unmodified current invariant remains green, confirming the defect is an oracle false acceptance rather than a changed production execution. |
| `git diff --check` | Pass. |

### Final verdict

**NEEDS REMEDIATION.** The reproduced, seed-`424242` counterexample proves
that the required Tier-1 checker accepts a trace with two replacement
lifecycle-start completions. Return only this cardinality-oracle correction to
the original 02-10 crafter, then re-review it before advancing the roadmap.

---

## Iteration 3 — F-01 remediation re-review

| Field | Value |
|---|---|
| Remediation commit reviewed | `ddd6dfd13268b261b2431baf0452e7ce344d55f5` (`test(mtls): enforce BTR-3 lifecycle start cardinality`) |
| Commit trailer | `Step-Id: 02-10` present |
| Verdict | **APPROVED** |

### F-01 disposition — resolved

The remediation is necessary and directly closes the reproduced false
acceptance. Before its change, the iteration-2 seed-`424242` Fixture
StartAllocation→RestartAllocation spike appended a second observed
`LifecycleStartCompleted` and the current checker returned `Ok`; that is the
required-invalid condition recorded above.

`check_order` now counts every `LifecycleStartCompleted` in the post-start
replacement trace and returns an error unless the count is exactly one
(`crates/overdrive-sim/src/invariants/same_id_restart_lifecycle.rs:430-438`).
It preserves the existing ordered-effect oracle immediately afterward
(`:439-462`). The permanent bounded-change regression drives the normal
seed-`424242` Fixture StartAllocation and same-ID RestartAllocation through
the real `dispatch_with_network_provisioner` composition, appends the same
counterfactual completion, and requires `check_order` to return `Err`
(`:687-703`). This is the same accepted Sim/dispatch path as the spike, not a
test-only production state or alternate lifecycle implementation.

The checker now rejects the exact breach required by S-GTI-BTR-03—more than
one replacement lifecycle start (`distill/test-scenarios.md:147-150`). Its
count is evaluated only after the fixture clears the initial StartAllocation
trace (`same_id_restart_lifecycle.rs:690-696`), so it counts the replacement
completion rather than conflating it with initial ownership. Successful clean
and retry trajectories retain exactly one completion and continue to pass.

The remediation changes only the invariant-local pure checker and its
permanent regression. It adds no production API, port, worker, adapter state,
socket, recovery/cancellation mechanism, or cleanup path. The separate
iteration-1 static observations remain withdrawn: no new counterexample was
introduced or accepted for them during this re-review.

### Re-review verification

| Check | Result |
|---|---|
| `git diff --check ddd6dfd13268b261b2431baf0452e7ce344d55f5^ ddd6dfd13268b261b2431baf0452e7ce344d55f5` | Pass |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --lib -E 'test(same_id_restart_lifecycle)'` | Pass — 4 tests, including the permanent duplicate-completion regression |
| `cargo xtask lima run -- cargo dst --seed 424242 --only same-id-restart-removes-prior-protection-before-replacement-provision` | Pass — 1 invariant, seed `424242`, `status=pass` |
| `cargo xtask lima run -- cargo check -p overdrive-sim --all-targets` | Pass |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/guest-stack-transparent-mtls-intercept/deliver/` | Pass — all 13 step traces complete |
| Scope audit | Pass — remediation commit changes only `same_id_restart_lifecycle.rs` and its 02-10 DES `RED`/`GREEN` records; no production or public surface changed. |

No mutation command was run or requested.

### Final verdict

**APPROVED.** F-01 is resolved with a necessary, bounded checker cardinality
guard and a permanent real-dispatch seed-`424242` negative regression. The
implementation commit, both documented review iterations, and the remediation
remain within the accepted BTR-3 lifecycle-port scope.
