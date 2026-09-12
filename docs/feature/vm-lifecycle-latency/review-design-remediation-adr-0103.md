# Independent DESIGN Remediation Review — ADR-0103 Natural/Post-EXEC Cleanup

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Review role | `nw-solution-architect-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 1 — fresh independent review |
| Base | `4ea5902b51db96ffa2047e7907491a40efe6dea5` |
| Reviewed files | `docs/product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md`; `docs/product/architecture/brief.md`; `docs/feature/vm-lifecycle-latency/feature-delta.md` |
| Authorized write | This review artifact only |
| Verdict | **CHANGES_REQUESTED** |

## Authorized scope

The authorized outcome is narrowly bounded: after a natural or post-EXEC
guest/VMM exit, the allocation run directory, workload cgroup scope, per-launch
rootfs clone, and clone-index link must not remain. The proposed remedy may
reuse and await the existing private `VmDriver` cleanup sequence while the
unique `LiveVm` is still owned. It must not add a public API, state variant,
cleanup payload, persistence/recovery subsystem, second cleanup authority, or
adjacent lifecycle hardening. Pre-EXEC no-child failures remain outside the
four-artifact promise.

This review covers only the three proposed documentation changes listed in the
metadata. Other dirty source, test, `AGENTS.md`, and DELIVER-log changes were
read only where necessary to establish the production path and reproduction;
they were neither reviewed as implementation nor modified.

## Production and reproducer evidence

### Current production owner path

The claimed failure is reachable through the current production composition.

1. The real in-process `serve` composition creates `VmDriver`, and
   `VmDriver::start` accepts READY, installs the accepted-session
   `BeaconWriter`, stores the Running-confirmed gate, and spawns the exit
   watcher (`crates/overdrive-worker/src/vm_driver.rs:1522-1575`).
2. `spawn_exit_watcher_task` currently passes the live map, exit sender,
   cgroup-accounting reader, scope, gate, and session witness to the detached
   `run_exit_watcher` task (`vm_driver.rs:1381-1410`).
3. After guest/VMM completion, `run_exit_watcher` records the exit facts, reads
   OOM accounting before teardown, awaits the Running-confirmed gate, changes
   the matching accepted session from `Live` to `EndingInFlight`, and sends
   `ExitEvent` without invoking driver cleanup (`vm_driver.rs:2140-2210`).
4. The exit observer consumes that event, writes the accepted allocation
   lifecycle occurrence, and releases supervision after every retry outcome
   (`crates/overdrive-control-plane/src/worker/exit_observer.rs:199-305`). The
   accepted row wakes `WorkloadLifecycle`, whose natural-Job path emits
   `Action::FinalizeFailed` (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:785-824`).
5. `FinalizeFailed` carries only `alloc_id` and `terminal`. Its action-shim arm
   tears down mTLS/netns/probe ownership and writes the terminal row, but does
   not call `VmDriver::stop` and has no `RootfsPlan`, `VmRunDir`, or
   `CgroupPath` with which to execute the four driver-owned cleanup operations
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1558-1789`).
6. The current watcher claim replaced the `LiveVm` with unit
   `EndingInFlight`; `VmDriver::status` and a later `VmDriver::stop` therefore
   return `DriverError::NotFound` (`vm_driver.rs:1660-1712`, `:1799-1805`). No
   live-session owner remains with the discarded cleanup inputs.

The existing stop owner confirms the intended reuse boundary. After its
writer/VMM composition, `VmDriver::stop` awaits `cgroup_kill`,
`remove_workload_scope`, run-directory removal, and clone-then-index removal,
then emits `vm.lifecycle.cleanup_calls_finished` (`vm_driver.rs:1721-1796`).
The proposal extracts exactly this private sequence rather than creating a
second cleanup subsystem.

### Qualified native-metal reproduction

The reproducer drives the real production `serve` composition, `deploy`, real
Cloud Hypervisor, the production guest init, the real exit observer,
interest-router wake, `WorkloadLifecycle`, and `FinalizeFailed`. The
`GuestControlVmm` is a decorator at the already-existing `Vmm` port used only
to deliver the malformed/EOF/control-frame populations; it delegates process
creation and termination to `CloudHypervisorVmm`
(`crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:219-274`,
`:2424-2604`). The artifact oracle resolves the allocation's actual run
directory, cgroup scope, clone destination, and index link, polling their
filesystem presence directly (`:3072-3105`).

The fail-closed native preflight passed with the selected guest kernel and
rootfs. Nextest run `fb2a672f-55ee-48e2-8d8c-bc557bc213fe` executed the two
bounded S10 tests on the canonical metal runner and reproduced both failures:

- S10b post-EXEC EOF reached a finalized terminal claim, then retained all four
  artifacts for the full observation bound: run directory, cgroup scope,
  rootfs clone, and clone-index link. The three pre-EXEC no-child cases ran
  first and passed their typed-error/no-child/no-EXIT oracles before the first
  post-EXEC failure.
- S10a natural direct-child completion reached its finalized terminal claim
  with exit code 23 and then retained the same four artifacts for the full
  observation bound.

This is failure reproduction through the real production entry/caller/owner
path, not a fabricated `EndingInFlight` state. It proves the necessity of a
bounded remedy for the authorized natural/post-EXEC population. It does not
authorize extending the promise to the pre-EXEC no-child population.

## Design, API, and ownership analysis

### Exact private shape and feasibility

The proposed implementation shape is sufficiently exact and implementable:

- `async fn cleanup_driver_artifacts(cgroup_manager: &CgroupManager,
  live_vm: &LiveVm)` is private, returns unit, preserves the current operation
  order and best-effort result handling, and has exactly the stop path and
  natural watcher as callers.
- `ClaimGuard::try_begin_ending(&mut self) -> Option<LiveVm>` changes only the
  private race verdict. `Some(LiveVm)` is the unique cleanup capability;
  `None` preserves the lost-session-claim result.
- The complete private `run_exit_watcher` signature is pinned, including the
  one added owned `CgroupManager` argument and its exact position.
- `VmDriver::stop` is required to move the whole `LiveVm` and install unit
  `EndingInFlight` in the same locked critical section. The writer/VMM
  composition may borrow/take fields from that locally owned value and then
  pass `&LiveVm` to the shared cleanup helper without any new public surface.
- No `LiveMap` lock crosses an await. `CgroupManager` already implements
  `Clone`, so watcher routing requires no new capability or constructor.

No public `Driver`, `Vmm`, `BeaconWriter`, error, wire, allocation-state, or
persistence shape changes. The design also correctly retains the existing
limited `Driver::stop Ok` meaning: all cleanup calls returned, while actual
absence remains a separate native observation because several results are
best effort.

### Atomic winner and exactly-once effects

The same `LiveMap` mutex serializes the non-session-qualified stop claim and
the accepted-session-qualified watcher claim. Both replace `Live(LiveVm)` with
the existing unit `EndingInFlight` while holding that mutex. Because `LiveVm`
is not `Clone`, only the lock winner can own the value passed to
`cleanup_driver_artifacts`; the loser sees non-`Live` and cannot invoke that
helper. This preserves one driver-cleanup invocation without a second registry
or cleanup record.

The stop-wins ordering retains one `Vmm::terminate` call, the existing writer
join/abort behavior, operator-stop terminal authorship, and no watcher event.
The watcher-wins ordering occurs only after the VMM exit has already been
reaped; it does not call `Vmm::terminate` or send SHUTDOWN, and a late stop
returns the existing `NotFound`. In both orderings `EndingInFlight` remains
visible to `live_allocations`, so `VmReclamation` cannot acquire a competing
claim. The existing exit-observer and stop-shim release points remain the only
ways to retire supervision while `serve` is live.

The accepted-session `Weak<BeaconWriter>` identity check is unchanged. Guest
direct-child status, post-EXEC stream error, OOM fact, and `ExitKind` are all
captured before cleanup; the watcher sends the existing event only after
cleanup-call completion. The exit observer's retry behavior, LWW arbitration,
`WorkloadLifecycle` terminal claim, `FinalizeFailed` terminal write, and late
natural-exit fencing therefore remain downstream and unchanged.

### Scope and reuse analysis

The proposal stays on the reproduced path. It reuses `VmDriver`'s live value,
claim mutex, cgroup owner, run-directory owner, and clone-order helper. It does
not move cleanup into `FinalizeFailed`, extend `EndingInFlight`, add durable
cleanup intent/retries, rely on boot reclamation as session completion, or
strengthen the public stop result. READY, allocation Running, Service Stable,
writer/VMM overlap, guest EXEC/EXIT/poweroff, and pre-EXEC no-child contracts
remain outside the moved gate.

The Reuse Analysis declares bounded-change universes for the `LiveMap` claim,
the four driver artifacts, and the downstream row/shim-owned effects. No
unbounded-preservation procedure, new dependency edge, new technology, or new
system of record is introduced.

### Roadmap executability

Roadmap step `01-02` already owns ADR-0103 fidelity, cleanup completion,
S-VLL-07 through S-VLL-10, and the shared S-VLL-13 preservation gate. Its
existing S10 native bodies reproduce the new required outcome, and the
existing worker stop/session/clone tests provide the bounded in-process
complements. `des-roadmap validate` still accepts the unchanged roadmap as one
phase and three steps. No new roadmap step, public test seam, or acceptance
surface is required.

The roadmap itself therefore does not need invention. It remains blocked only
because the authoritative design prose has the consistency finding below.

## Findings and dispositions

### F-01 — High, blocking: stale authoritative clauses contradict the new watcher ordering

**Severity:** High; blocks independent DESIGN approval.

**Locations:**

- `docs/product/architecture/brief.md:9109` — §105a.3 transition 3 says the
  watcher changes `Held -> EndingInFlight` **immediately before** emitting its
  `ExitEvent`.
- `docs/product/architecture/brief.md:1767-1772` — the focused amendment says
  all other accepted brief sections remain authoritative.
- `docs/product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md:17`
  — the ADR still says it amends only ADR-0082 request/grace ordering and PID 1
  duties.
- Proposed ADR-0103 `:178-183` and feature delta G-4 `:288-293` instead require
  the atomic claim, then four awaited cleanup operations, then the cleanup event,
  and only then `ExitEvent`.

**Reachability and consequence:** This is not a theoretical alternative. The
current production watcher executes transition 3 immediately before its send
and therefore reproduces the four retained artifacts on qualified native
metal. The proposed remedy deliberately inserts an awaited, ordering-sensitive
driver-cleanup phase between those two operations. Leaving the existing
authoritative transition row unchanged gives the `01-02` crafter two mutually
exclusive contracts: preserve “immediately before,” or implement the new
cleanup-before-report edge. The ADR's old scope sentence likewise says the ADR
does not cover the very natural-exit change it now proposes. Under the
repository's exact-design rule, the crafter may not choose which prose wins.

**Required disposition:** Reconcile the stale scope sentence and §105a.3
transition-3 row inside the reviewed ADR/brief so they explicitly reflect the
already-selected sequence: accepted-session claim and `LiveVm` move, awaited
existing driver cleanup, then `ExitEvent`. Preserve transition 3's atomic
verdict, transitions 4-7, unit `EndingInFlight`, stop path, LWW authorship, and
all other scope exactly as proposed. This is documentation consistency only;
it does not require a new mechanism, public API, roadmap edit, test seam, or
scope expansion.

**Disposition:** Open. Return the same bounded design to independent re-review
after the authoritative clauses agree.

No other critical, high, medium, or low finding was established. In
particular, the proposed private signatures are exact; the stop/watcher race
has one atomic winner; driver cleanup is exactly once; process termination is
not duplicated; downstream LWW/session/terminal authorship is preserved; and
the remedy reuses the existing cleanup owner without broadening the authorized
outcome.

## Validation commands and results

| Validation | Result |
|---|---|
| `git diff --check -- <three reviewed files>` | Exit 0; no whitespace errors. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-roadmap validate docs/feature/vm-lifecycle-latency/deliver/roadmap.json` | Exit 0; `VALID: 1 phases, 3 steps`. |
| Qualified metal S10a/S10b command using `/srv/vm/overdrive-testing/kernel` and `/srv/vm/overdrive-testing/rootfs.ext4` through `cargo xtask metal run -- cargo nextest run ...` | Fail-closed native preflight passed; run `fb2a672f-55ee-48e2-8d8c-bc557bc213fe`; 2 tests executed, 2 failed for the expected four-artifact retention. |
| Unqualified metal command without selected guest artifacts | Refused before test mutation with `FATAL: native metal preflight: selected guest kernel is required`; correctly excluded from evidence. |
| Read-only contract scan for exact private signatures and stale ordering clauses | Exact helper/method/watcher shapes present; contradictory brief transition and ADR scope clause located as F-01. |
| Production tests after implementing the proposed remedy, mutation testing, commits, staging | Not run or performed; implementation remains paused and this reviewer wrote only this artifact. |

## Iteration history

### Iteration 1 — fresh independent review

The native reproduction validates the authorized premise and exact current
owner path. The selected private move-and-cleanup mechanism is minimal,
feasible, race-safe, and consistent across the proposed ADR amendment, focused
brief summary, and feature-delta G-4. One older authoritative brief transition
and one ADR scope sentence were not superseded and still contradict that exact
ordering. No design alternative is proposed by this review; the selected
mechanism needs only documentary reconciliation.

## Final verdict

**CHANGES_REQUESTED.** F-01 must be closed before DDD-8 is independently
approved and before the original DELIVER step `01-02` crafter resumes. The
roadmap remains unchanged and does not require a new step or invented API.

---

## Iteration 2 — F-01 Remediation Re-review

### Iteration 2 metadata

| Field | Value |
|---|---|
| Review role | `nw-solution-architect-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 2 — bounded remediation re-review |
| Base | `4ea5902b51db96ffa2047e7907491a40efe6dea5` |
| Remediation scope | F-01 only: ADR scope sentence and architecture-brief transition 3 |
| Current verdict | **APPROVED** |

### Remediation examined

The architect changed only the two stale authoritative clauses identified by
F-01:

- ADR-0103 now states that, in addition to the already approved request/grace
  and PID 1 changes, its scope includes the private accepted-session
  natural/post-EXEC exit-watcher ownership and driver-artifact cleanup ordering
  (`docs/product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md:17-19`).
- Architecture brief §105a.3 transition 3 now states the complete selected
  order: record the guest/VMM result and OOM fact; await the Running-confirmed
  gate; atomically move the originating accepted session's unique `LiveVm` to
  the watcher while installing unit `EndingInFlight`; await the existing
  driver-artifact cleanup; emit `vm.lifecycle.cleanup_calls_finished`; then
  emit the natural `ExitEvent` (`docs/product/architecture/brief.md:9105-9113`).

No alternative mechanism, new ownership model, public API, persistence or
recovery behavior, error contract, state, wire frame, roadmap step, or expanded
pre-EXEC promise was introduced by the remediation.

### F-01 disposition

| Finding | Iteration 1 state | Iteration 2 evidence | Disposition |
|---|---|---|---|
| F-01 — stale authoritative clauses contradicted cleanup-before-report ordering | Open; high/blocking | ADR-0103's scope now expressly includes the private amendment, and brief transition 3 now carries the same claim → awaited cleanup → cleanup event → natural `ExitEvent` order as ADR-0103 and feature-delta G-4. | **Closed** |

The correction is necessary and sufficient. The architecture brief no longer
requires transition 3 to occur “immediately before” `ExitEvent`, and the ADR no
longer excludes its proposed natural/post-EXEC ownership change from its own
scope. A DELIVER crafter now has one authoritative interpretation of the
ordering and does not need to choose between conflicting design clauses.

### Consistency, API, and ownership re-validation

The three reviewed design sources now agree on all load-bearing details:

- The exact shared helper remains private
  `async fn cleanup_driver_artifacts(cgroup_manager: &CgroupManager,
  live_vm: &LiveVm)` with unit return, current call order, and current
  best-effort results.
- The exact private claim method remains
  `ClaimGuard::try_begin_ending(&mut self) -> Option<LiveVm>`; the
  accepted-session identity check, atomic `Live -> EndingInFlight` replacement,
  and unique moved cleanup capability are unchanged.
- The exact private `run_exit_watcher` signature still adds only the owned
  `CgroupManager` between `exit_tx` and `cgroup_accounting`.
- Stop and watcher still serialize on the same `LiveMap`; only the winner owns
  `LiveVm` and invokes driver cleanup. Stop-win retains one writer/VMM
  composition and operator terminal authorship. Watcher-win follows an already
  reaped VMM, performs no termination or SHUTDOWN request, and sends one natural
  event after cleanup. The loser cannot terminate or clean a second time.
- Unit `EndingInFlight`, `live_allocations`, reclamation refusal, accepted
  session identity, exit-observer release, LWW arbitration, and terminal
  authorship remain unchanged. The moved gate affects only natural/post-EXEC
  report ordering.
- `Driver::stop`, `Vmm`, `BeaconWriter`, `DriverError`, `InitError`, allocation
  states, guest frames, persistence, writer/VMM overlap, READY, Running, Stable,
  EXEC/EXIT/poweroff, and the pre-EXEC no-child oracle remain unchanged.

The Reuse Analysis continues to use the existing `VmDriver` cleanup owner,
`CgroupManager`, run-directory owner, and clone-then-index helper. Its bounded
universe remains one accepted allocation, one map claim, and the four named
driver artifacts. There is still no second cleanup subsystem or system of
record.

### Reproduction and roadmap re-validation

Iteration 1's qualified native-metal run
`fb2a672f-55ee-48e2-8d8c-bc557bc213fe` remains the premise evidence because
the F-01 remediation changed design prose only, not the production or S10
reproducer paths. It established both required populations through the real
production owner chain:

- S10a natural direct-child completion reached its finalized terminal claim
  and retained the run directory, cgroup scope, rootfs clone, and clone-index
  link.
- S10b ran all three pre-EXEC no-child cases through their corrected
  typed-error/no-child/no-EXIT oracle, then the post-EXEC EOF case reached its
  finalized terminal claim and retained the same four artifacts.

The unchanged roadmap remains executable without invention. Step `01-02`
already owns exact ADR-0103 fidelity, cleanup completion, S-VLL-07 through
S-VLL-10, and shared S-VLL-13 preservation. The corrected design requires no
new step, test seam, public method, state, or parameter beyond the three exact
private shapes already recorded.

### Iteration 2 validation commands and results

| Validation | Result |
|---|---|
| `git diff --check -- <three reviewed design files>` | Exit 0; no whitespace errors. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-roadmap validate docs/feature/vm-lifecycle-latency/deliver/roadmap.json` | Exit 0; `VALID: 1 phases, 3 steps`. |
| Read-only scan of ADR scope, brief transition 3, exact helper/method/watcher signatures, and cleanup-before-event clauses | Pass; F-01's two stale clauses are corrected and the three design sources agree. |
| Qualified native reproduction | Not repeated: production and S10 paths are unchanged by the prose-only F-01 remediation; iteration 1 run `fb2a672f-55ee-48e2-8d8c-bc557bc213fe` remains current premise evidence. |
| Production/tests/roadmap/DES log, staging, commit | Not modified. This iteration appended only this review history. |

### Iteration 2 findings

F-01 is closed. No new critical, high, medium, or low finding was established.
The remediation does not reopen API ambiguity, race ownership, cleanup
duplication, process termination, LWW/session authorship, scope, reuse, or
roadmap executability.

### Current final verdict

**APPROVED.** The bounded DDD-8 design remediation now has one consistent
authoritative contract. The original DELIVER step `01-02` crafter may resume,
remaining bound to the exact private signatures and ordering in ADR-0103, the
unchanged public contracts and roadmap, the existing stop/session/clone
preservation gates, and qualified native S10 artifact-absence validation after
implementation.
