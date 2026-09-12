# Evolution — VM lifecycle latency (GH #283)

**Finalized:** 2026-09-12. **Feature slug:** `vm-lifecycle-latency`.
**Waves:** DESIGN → DISTILL → DELIVER → FINALIZE; DEVOPS was explicitly
skipped. **Completion record:** [roadmap](../feature/vm-lifecycle-latency/deliver/roadmap.json),
[execution log](../feature/vm-lifecycle-latency/deliver/execution-log.json), and
the three step reviews under
[`docs/feature/vm-lifecycle-latency/deliver/`](../feature/vm-lifecycle-latency/deliver/).

## Business and problem context

VM lifecycle work was serialized through one convergence owner even when two
workloads were independent, so one slow `Driver::start` or `Driver::stop`
could stall otherwise eligible work. The healthy VM Service stop path also
took about 12 seconds in the reproduced baseline: roughly two seconds waiting
on the SHUTDOWN writer followed by a separate ten-second VMM grace, ending in
forced termination because the guest PID 1 waited synchronously for the
workload and did not read control while it ran.

The feature makes that path responsive without changing the public operator
commands, `Driver`/`Vmm` traits, allocation-state vocabulary, guest wire
frames, or persistence model. Its accepted design and premise evidence are in
the [feature delta](../feature/vm-lifecycle-latency/feature-delta.md),
[research](../research/orchestration/vm-lifecycle-latency-283-comprehensive-research.md),
[RCA](../analysis/root-cause-analysis-vm-lifecycle-latency-283.md),
[approved RCA review](../analysis/review-vm-lifecycle-latency-283.md),
[ADR-0102](../product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md),
and [ADR-0103](../product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md).

## What shipped

### Bounded convergence ownership

The existing convergence owner now owns at most eight complete evaluation
futures concurrently and at most one active evaluation for each exact
`TargetResource`. The lease covers hydration, durable View write-through,
action dispatch, re-enqueue, and result consumption; it is not released merely
because a driver call was submitted. Pending work remains keyed by
`(ReconcilerName, TargetResource)`, replacement retains the first enqueue age,
and the broker admits the oldest eligible key while preserving same-target
exclusion and immediate refill after a completed result.

Shutdown closes admission and drains every already admitted evaluation before
the convergence owner exits. Its `pending_at_exit` value is the locked broker
snapshot taken after active results are consumed; submissions linearized after
that snapshot remain pending and are not reported as completed. Existing
exit-observer cancellation priority, View fsync-before-dispatch, LWW/session
authorship, and reclamation ownership remain unchanged.

### Responsive host and guest termination

`VmDriver::stop` now begins the SHUTDOWN writer cap and the single VMM grace
together. Two seconds is a cap on writer work, ten seconds is the host VMM
grace, and neither is a minimum or an additive `2 + 10` wait. Every writer is
completed or aborted and joined, VMM completion is awaited, and the existing
driver-artifact cleanup calls complete before the stop continuation.

The production guest init remains the sole PID 1 lifecycle owner, but it now
supervises the control stream and direct child concurrently. It establishes a
child-led process group, keeps reading control while the command runs, applies
SIGTERM once on SHUTDOWN, waits one five-second guest grace, escalates remaining
group members with SIGKILL, reaps direct and adopted children, preserves the
direct child's real exit/signal result, emits EXIT at most once, and powers off
without waiting for a post-EXIT SHUTDOWN frame. READY remains before EXEC and
is not redefined as Service health or effect completion.

For natural and post-EXEC exits, the accepted-session watcher and operator stop
still race through the same `LiveMap`. The winner moves the unique, non-`Clone`
`LiveVm`, installs unit `EndingInFlight`, and alone owns cleanup. The watcher
consumes the reaper-observed VMM exit, preserves the guest/OOM facts, awaits the
Running-confirmed gate, performs the existing cgroup-kill, cgroup-scope,
run-directory, and clone-then-index cleanup sequence, emits
`vm.lifecycle.cleanup_calls_finished`, and only then sends the existing
`ExitEvent`. No second cleanup authority, payload-bearing ending state, retry
store, or stronger public `Driver::stop Ok` promise was added.

### Landlock reconciliation

Qualified native-metal execution exposed a Cloud Hypervisor v53 dependency on
reading `/sys/class/net/<selected-tap>/tun_flags`. The approved ad hoc fix
centralizes the complete explicit rule list in `VmConfig::landlock_rules()`:
a networked VM receives exactly the selected allocation TAP sysfs leaf with
`access=r`, followed by the allocation run directory with `access=rw`; a
non-networked `VmConfig` retains only the run-directory rule. The access
discriminator and TAP-rule producer are private, the host adapter only places
the composed values on argv, and no parent `/sys/class/net` grant, TAP write
grant, public constructor, lifecycle gate, or second security authority was
introduced. The [DESIGN review](../feature/vm-lifecycle-latency/review-design-remediation-landlock.md)
and [implementation review](../feature/vm-lifecycle-latency/review-landlock-ad-hoc.md)
are both approved.

## Delivery record

Every roadmap step has a complete RED → GREEN → COMMIT trace and an approved
native-Markdown review artifact.

| Step | Delivered outcome | Current implementation/closure lineage |
|---|---|---|
| 01-01 | Eight complete evaluation owners, exact-target exclusion, FIFO age retention, admission close/drain, owner snapshot, and preservation oracles | `083ef6e6`, `78f189b7`, `1918489f`; final DES persistence `f628b95f`; review record `2ab06d9d` |
| Ad hoc | Least-privilege selected-TAP Landlock rule composed at the existing `VmConfig` authority | design `624e0274`; implementation `f60495a3`; review record `af158034` |
| 01-02 | Concurrent writer/VMM wait, responsive PID 1 supervision, exact direct-child result, and cleanup-before-natural-report ownership | `5edd751e`, `493b793f`, `869d150a`, locator closure `2f3cb4be`; final DES persistence `9d0b37d7`; review record `111d404c` |
| 01-03 | Activated S11 native benchmark, independent E09-v2 workers, retained E10 raw evidence, and current EDD/test/benchmark separation | implementation `8452d908`; evidence remediations `3eb2bbdc`, `f53aee27`, `a17cf0d6`; evidence approval `437e5737`; benchmark-report remediation `e6d51e95`; final review record `963e9887` |

The 01-03 entries above use the current rewritten branch commit IDs. Before
finalization, the unpushed lineage was rewritten only to replace four recursive
E06/E08/E09-v2/E11 `dirty-diff.patch` receipts with filtered source,
configuration, example, runner, and harness receipts of 14,046 to 44,444 bytes.
The current receipts exclude every expectation evidence path and self-hunk;
the E10 receipt remains the independently approved 78,634-byte filtered
receipt. The local pre-rewrite safety branch is
`backup/vm-lifecycle-latency-pre-evidence-rewrite-20260912`.

## S-VLL-11 native measurement result

The accepted fifth run used one persistent in-process production server on a
qualified non-virtualized x86_64 KVM host with Cloud Hypervisor v53.0, one VCPU
(`cpu_milli=500`), 128 MiB RAM, fresh allocation identity per trial, cold VM
boots, and a declared warm-pre-read immutable-input cache. It scheduled exactly
1,200 successful trials with zero recorded failures: 400 READY, 400 finite Job,
and 400 cooperative Service trials; every profile contains 200 sequential and
200 concurrent trials in twenty ten-worker cohorts. Nearest-rank quantiles were
recomputed independently from the retained raw ledger.

| Distribution | n | Min | Median | P95 | P99 | Max | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| READY: `vm.lifecycle.create_enter` → accepted READY | 400 | 1,122.179 ms | 1,151.723 ms | 1,526.082 ms | 1,665.804 ms | 1,714.080 ms | Pass: P95 ≤ 2,000 ms; P99 ≤ 3,000 ms |
| Finite Job: EXEC released → normal VMM reap | 400 | 34.972 ms | 44.610 ms | 117.876 ms | 182.716 ms | 252.681 ms | Pass: P95 ≤ 500 ms; P99 ≤ 1,000 ms |
| Cooperative Service: stop entered → driver artifacts absent | 400 | 54.079 ms | 198.779 ms | 355.818 ms | 414.032 ms | 481.112 ms | Pass: P95 ≤ 1,000 ms; P99 ≤ 1,500 ms |
| Admission queue | 354,305 | 0 ms | 5,959 ms | 147,739 ms | 195,862 ms | 267,539 ms | Reported separately; no added SLO |
| Stop entered → cleanup calls finished | 800 | 25.572 ms | 49.954 ms | 104.119 ms | 130.404 ms | 186.953 ms | Diagnostic distribution |
| VMM reaped → cleanup calls finished | 1,200 | 0.405 ms | 0.808 ms | 22.149 ms | 58.049 ms | 119.002 ms | Diagnostic distribution |
| Public stop return | 800 | 41.495 ms | 43.914 ms | 90.932 ms | 143.102 ms | 170.433 ms | Diagnostic distribution |
| Public stop → admission | 800 | 0 ms | 1,353.484 ms | 201,627.865 ms | 265,535.614 ms | 267,190.501 ms | Reported separately; no added SLO |
| Public stop → terminal observation | 800 | 46.085 ms | 1,596.726 ms | 201,830.480 ms | 265,647.505 ms | 267,432.689 ms | Existing post-admission bound retained; not pooled into Service latency |

Every measured row independently retained normal reaper exit, `/proc` absence,
run-directory/cgroup/rootfs-clone/clone-index absence, and the required stage
ordering. Instrumented READY control was 2,000,433,747 ns; the uninstrumented
control was 1,996,174,007 ns; the recorded instrumentation difference was
4,259,740 ns.

The canonical retained report is schema
`overdrive.vm-lifecycle-latency.s11.native.v1`, 9,715,410 bytes,
newline-terminated, SHA-256
`f0868cd60cc3529e2798ef234d181df0f56f0c1f9e99e68afa8bffcecc16da4b`.
The qualified-metal source path was
`/srv/vm/overdrive-testing/s11-native-20260912-attempt5.json`; the pulled local
copy is
`.context/vm-lifecycle-latency/s11-reruns/attempt5/s11-native-20260912-attempt5.json`,
with matching `.log` and `.exit` receipts. The complete independent audit is
in the [step 01-03 review](../feature/vm-lifecycle-latency/deliver/review-01-03.md).

## EDD evidence disposition

The canonical point-in-time records remain under
[`verification/expectations/`](../../verification/expectations/) and were not
copied into this document.

| Expectation | Current status | Approved result |
|---|---|---|
| [E06](../../verification/expectations/E06-vm-job-deploy-reaches-running/) | `satisfied` | VM Job deploy accepted, allocation reached `Running` through the real VM path, and owned-resource cleanup deltas were zero. |
| [E08](../../verification/expectations/E08-vm-service-guest-health/) | `satisfied` | VM Service reached `Stable`, guest TCP/HTTP checks and the peer exact-reply journey passed, and teardown deltas were zero. |
| [E09-v2](../../verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/) | `satisfied` | Exactly 20/20 healthy/failure pairs completed through one persistent control-plane PID/start identity, with concurrency ten, independent per-worker failure submission, truthful healthy/failure and peer results, no pair retry/replacement/discard/cancellation substitution, and zero runtime cleanup cells. |
| [E10](../../verification/expectations/E10-vm-service-http-cross-driver-status/) | `satisfied` | Exactly 8/8 token-matched Exec/VM × HTTP 204/302/404/503 raw cells preserved correct success/failure PTY rendering and exit codes, same-allocation failure/restart/probe history through explicit stop, named cleanup absence, an external no-late-overwrite witness, and zero failure-body sentinel exposure. |
| [E11](../../verification/expectations/E11-vm-service-readiness-traffic-recovery/) | `satisfied` | The same Running/Stable allocation with restart count zero followed Pass → Fail → Pass readiness and withdrew/restored peer traffic within the recorded bound. |

The separate [different-fox evidence review](../feature/vm-lifecycle-latency/deliver/review-01-03-evidence.md)
is approved. These are historical SHA/environment-scoped EDD records, not
recurring tests and not substitutes for S11's benchmark ledger or the Rust
integration/simulation guarantees.

## L1–L6 refactor and gate dispositions

The final behavior-preserving refactor is commit `8a0adf67`. It changed six of
the twelve measured feature production files by 91 additions and 98 deletions,
with no test edit or public/accepted-private API change.

| Metric | Before | After |
|---|---:|---:|
| Total Rust lines | 19,017 | 19,010 |
| Rust code lines | 13,942 | 13,935 |
| Rust comment lines | 3,756 | 3,754 |
| PID 1 `begin_group_termination(` occurrences | 6 | 4 |
| Nested convergence admission guards | 2 | 1 |
| Duplicated originating-session `Weak::ptr_eq` predicates | 2 | 1 |

The refactor reused one private `ClaimGuard` originating-session predicate,
extracted the private control-line handler, flattened the convergence admission
guard, named fixed capacities/durations, and corrected stale comments. It added
no strategy layer, state, retry, timeout, persistence mechanism, or interface.
The complete [refactoring log](../feature/vm-lifecycle-latency/deliver/refactor/refactoring-log.md)
and [quality metrics](../feature/vm-lifecycle-latency/deliver/refactor/quality-metrics.md)
retain the comparison evidence.

Mutation testing was explicitly skipped by the user and recorded by current
commit `033c9503`. No mutant ran, no kill rate was measured, and the disposition
is `APPROVED_SKIP`, not a passing mutation result. See the
[mutation report](../feature/vm-lifecycle-latency/deliver/mutation/mutation-report.md).

The refactor commit was also explicitly authorized with `--no-verify` after
the qualified-metal 16-test CLI gate passed 15 tests and timed out in
`restarted_vm_boots_from_a_clean_unmodified_rootfs_copy`. The exception does
not turn that gate green. Syscall evidence showed the old VM execution's late
cleanup removing the replacement execution's allocation-derived run directory,
causing kernel staging to fail with `ENOENT`; the inverse interleaving had
already produced beacon-path `EADDRINUSE`. The separate open
[GH #284](https://github.com/overdrive-sh/overdrive/issues/284) owns the
fresh-`AllocationId` recreation and artifact-ownership correction plus the
seeded production-owner and qualified-metal hard-gate regressions. No #284
mechanism is included in this feature.

## Lessons retained

- Concurrency belongs around the complete convergence evaluation. Parallelizing
  only the inner driver call would release ownership before durable View,
  dispatch, re-enqueue, and result-consumption obligations are complete.
- Target exclusivity and submission identity are different facts. Several
  inherited tests incorrectly treated one target-exclusive admission round as
  a complete drain; the corrected oracles prove pending identity, sequential
  lease release, eventual admission, and exact counter deltas separately.
- Completion-sensitive waits must use completion, not timeout floors. The host
  writer and VMM grace can overlap, while the guest must continue consuming
  control during workload execution.
- Natural exit reporting cannot discard the move-only driver cleanup
  capability. Cleanup must finish in the winning `LiveVm` owner before the
  existing `ExitEvent` enters downstream terminal convergence.
- Least privilege is easier to audit with one value composer. The selected TAP
  read rule and run-directory write rule now share one ordered authority rather
  than being formatted independently in core and the host adapter.
- A successful benchmark process is not an auditable benchmark result unless
  it retains its raw ledger, fixture/source identity, distributions, failures,
  and instrumentation comparison. The S11 report writer now does so before a
  failure assertion can discard the evidence.
- EDD records, in-process integration tests, simulation invariants, stress
  cohorts, and benchmarks answer different questions. Keeping their runners
  and retained artifacts separate prevented one green layer from substituting
  for another.
- Dirty-tree receipts must exclude their own evidence tree. Self-referential
  and recursively nested patches obscured provenance without adding source
  truth; the filtered receipts retain relevant dirty inputs directly.
- Stable workload lineage and physical execution identity must not be
  conflated. #284 carries the independently reproduced allocation-artifact
  ownership defect rather than expanding this latency feature after delivery.

## Finalization and workspace disposition

The accepted architecture brief and ADRs already live in
[`docs/product/architecture/`](../product/architecture/); the operator example
already lives in
[`examples/service-kind-vm-workloads-v2/`](../../examples/service-kind-vm-workloads-v2/);
the EDD catalogue already lives under
[`verification/expectations/`](../../verification/expectations/). The feature
workspace contains no temporary architecture, ADR, walking-skeleton, or UX
artifact requiring migration, so finalization created no duplicate permanent
copy.

The complete [`docs/feature/vm-lifecycle-latency/`](../feature/vm-lifecycle-latency/)
directory is preserved as historical wave evidence. Session-marker audit found
`.nwave/des/deliver-session.json`, `.nwave/des/des-task-active`, and every
feature-local `.develop-progress.json` absent, so finalization removed no path.
The user-owned dirty `AGENTS.md` was preserved unchanged and excluded from
finalization staging/commit activity.
