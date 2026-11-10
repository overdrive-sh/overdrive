# Independent DISTILL review — D9 VM hook chronology oracle

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | One worker acceptance oracle in `service_kind_vm_workloads.rs::vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner` |
| Authority | D9/ADR-0261: retain liveness at Running, start it after committed Stable; readiness remains live |
| Protected work | Original step 03-01 crafter's dirty production changes, activated tests, DES phases and commit |
| Verdict | **APPROVED** for this bounded DISTILL oracle update; the step's production review remains separate |

## Reproduced mismatch and production boundary

The existing worker acceptance body failed at its Running-phase assertion that `ProbeRole::Liveness` was already live (`liveness-stable-hook-fixture-remediation.md`). That assertion expressed the pre-D9 behavior. The test constructs a real `VmDriver` with the same `Arc<ProbeRunner>` returned to the assertions (`service_kind_vm_workloads.rs:123–169`) and invokes the existing `Driver::on_alloc_running`, `on_alloc_stable` and `on_alloc_terminal` hooks. It is testing the production worker hook implementation with simulated driven ports, not a replacement hook or fixture-side role map.

The approved D9 sequence is exact: `ProbeRunner::start_alloc` registers one supervisor, starts Startup and Readiness, and retains the Liveness descriptor without spawning its task (`probe_runner/mod.rs:313–356`). After the Stable row commits, the action shim calls `Driver::on_alloc_stable` (`action_shim/mod.rs:2144–2153`); `VmDriver` then retires Startup and invokes `ProbeRunner::start_liveness` (`vm_driver.rs:2063–2077`). `on_alloc_terminal` invokes `stop_alloc`, removing that supervisor. The test calls these existing hooks directly because its bounded subject is VM-to-runner delegation. Composed D9 acceptance tests separately prove the shim's row-write-to-hook ordering; this revision supplies no missing production wiring.

## Oracle substitution and no-weakening check

| Hook boundary | Retained or revised assertions |
|---|---|
| `on_alloc_running` | Exactly one active supervisor; Startup and Readiness live as before; Liveness explicitly **not live** (descriptor retained for Stable). |
| `on_alloc_stable` | Still one supervisor; Startup no longer live; Readiness still live; Liveness now live. |
| `on_alloc_terminal` | Zero active supervisors and no live role among Startup, Readiness or Liveness. |

The diff changes only the old Running-phase Liveness assertion and adds supervisor/role complements at Stable and terminal (`service_kind_vm_workloads.rs:487–513`). It does not remove a Startup or Readiness assertion, alter the constructed descriptor set, replace the production `VmDriver`/`ProbeRunner`, or add a public/test seam. The test retains `/// CONTRACT_SHAPE: bounded-change.`. The adjacent restart re-registration and TCP probe observation controls are unchanged. A future regression that starts Liveness at Running, fails to start it at Stable, retires Readiness early, duplicates the supervisor, or leaves a role live after terminal will fail this oracle.

## Verification and disposition

| Evidence | Result |
|---|---|
| Before | Author's focused Metal run reproduced the old Running-phase Liveness assertion failure; no seed or race is involved (`liveness-stable-hook-fixture-remediation.md`). |
| After | My independent focused Metal run of the hook body plus restart re-registration and TCP probe observation controls passed **3/3**, exit 0 (run ID `5b69c9bbbf4ddcf0fa839811`). |
| Static | The author reports changed-file rustfmt and `git diff --check` passing. My read-only `git diff --check` passed. I did not rerun broad suites, mutation or Clippy; the crafter's broader 16 activated-test result is separate evidence. |

**Verdict: APPROVED for this bounded DISTILL oracle correction.** It replaces the obsolete Running-phase Liveness expectation with D9's Stable-gated chronology while retaining the Startup, Readiness and terminal complements. No production code, model, DES event or commit was changed by this reviewer; the original step 03-01 crafter may resume its own GREEN checks and commit before the independent DELIVER review.
