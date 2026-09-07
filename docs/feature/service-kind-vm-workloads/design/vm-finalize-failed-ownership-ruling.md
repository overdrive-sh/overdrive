# VM FinalizeFailed / reclamation ruling

Date: 2026-09-07. Scope: current step `02-03`; design/reproduction only.
Original crafter retains production implementation, existing restart spike,
and native E09 ownership.

## Verdict — missing direct stop is not a proven orphan defect

**Withdraw the proposed stop-before-release remedy.** The initial hypothesis
stopped at VmDriver's map and omitted the existing VmReclamation owner.
The corrected seed **257204** drives registered production WorkloadLifecycle,
ServiceLifecycle, **and VmReclamation**, the production convergence/action shim,
real VmDriver, and existing driven ports. It demonstrates this supported path:

```text
Running(34): live VMM, driver ending-authorship claim held
  -> Service startup failure
  -> FinalizeFailed publishes Failed(35) / StartupProbeFailed
  -> driver ending-authorship claim released; VMM can still be live
  -> later direct Driver::stop reports NotFound
  -> VmReclamation: terminal + unclaimed -> DiscardStrandedArtifacts
  -> execution-time claim -> host kill_scope -> discard_artifacts -> claim release
  -> VMM dead, host artifacts absent, Failed(35) and occurrences unchanged
```

That is **not** a stranded VMM when the existing reclamation path is included.
No production failure of its accepted convergence contract was reproduced.
The direct-stop control also passes. No API, terminal-state, ownership, or
cleanup-order amendment is justified by this evidence. A preliminary ADR-0100
draft written during this investigation was removed before handoff; it is
withdrawn, not an accepted implementation contract.

## Diagnostic triple and corrected interpretation

- **Initial hypothesis:** FinalizeFailed discards a live VM's driver owner
  without awaited termination, making a subsequent stop unable to reclaim it.
- **Initial prediction:** VM remains live, driver entry disappears, later
  Driver::stop returns NotFound. This was reproduced, but it describes only
  one owner boundary and does not prove an unreclaimed resource.
- **Required falsification:** the existing registered reclamation owner
  completes cleanup while preserving the already-authored terminal outcome.
  **This occurred.** The accepted end-to-end cleanup path falsifies the inference
  that losing the Driver entry necessarily loses cleanup authority.

The user's question about VmReclamation exposed the omitted owner. Brief
§105a.3 defines supervision as a claim on **authoring an ending**, not a
requirement to retain a Driver stop handle until VMM death. Release after
terminal authorship intentionally authorizes the next cleanup owner.
An invariant demanding direct Driver::stop before every such release would
silently replace that accepted architecture. It is withdrawn.

## Complete production owner path

References describe the dirty source inspected/exercised here.

| Order | Concrete evidence |
| --- | --- |
| 1 | Production registers VmReclamation at `crates/overdrive-control-plane/src/lib.rs:2224`. `spawn_convergence_loop` at `:3332` builds registered resync schedules, submits due LocalNode evaluations, drains the broker, and awaits `run_convergence_tick` at `:3377`. |
| 2 | `crates/overdrive-reconcilers/src/vm_reclamation.rs:242` defines the existing 30-second sweep interval; `resync_schedule` at `:265` declares LocalNode, resync-only. This is the host-observation trigger, not an allocation-row subscription. The loop is serial, so 30 seconds is the scheduling interval, not a hard completion deadline under slow preceding work. |
| 3 | `crates/overdrive-control-plane/src/reconciler_runtime.rs:1574` hydrates the registered reconciler, persists its view, validates actions, and awaits the real shim at `:1703–1748`. Its existing `:1773` re-enqueue runs after dispatch, including shim errors. No action/tick timeout cancels a running dispatch; shutdown is checked between completed loop dispatches (`lib.rs:3400`). |
| 4 | Initial WorkloadLifecycle convergence authors StartAllocation. The shim's VM network branch (`action_shim/mod.rs:1182–1250`) assigns the guest network even without optional mTLS. The real VmDriver accepts READY; the shim authors Running(34), installs its allocation-driver index, and releases EXEC. No allocation lifecycle row is fixture-seeded. |
| 5 | `crates/overdrive-reconcilers/src/service_lifecycle.rs:1271–1319` consumes existing startup probe/deadline/attempt facts and emits `FinalizeFailed { ServiceFailed { StartupProbeFailed } }`. This is a Service startup decision, not Driver::start failure. |
| 6 | `action_shim/mod.rs:1564` retains no-row/exact-terminal/Job guards and Stable bypass. Its genuine-terminal path awaits mTLS stop when present (`:1766`), tears down allocation network/releases its slot binding (`:1769`), stops probes via the terminal hook (`:1778`), awaits terminal publication (`:1789`), and releases supervision (`:1791`). No direct Driver::stop occurs. |
| 7 | `crates/overdrive-worker/src/vm_driver.rs:1870` removes the supervision entry; `:1878`'s terminal hook only stops probes. LiveVm (`:843`) and BeaconWriter can drop; writer Drop (`:753`) aborts its task and writer-state Drop (`:739`) closes its write half. Neither calls Vmm::terminate. This is the documented authorship-release boundary, not evidence that every process is already dead. |
| 8 | VmReclamation hydration joins VM-driven intent and current allocation rows (`vm_reclamation.rs:283–365`); actual hydration observes host state **first**, then reads supervision **last** (`:302–317`). The terminal-plus-unclaimed planner branch (`:165–172`) selects **DiscardStrandedArtifacts**, not ReclaimAllocation. A held claim suppresses disposal. |
| 9 | The action shim dispatches DiscardStrandedArtifacts at `action_shim/mod.rs:3106`. `action_shim/reclamation.rs:299` acquires the existing execution-time lease via `ReclamationLease::try_acquire` (`:63`), which calls VmDriver's atomic `try_begin_reclamation` (`vm_driver.rs:1858`). A concurrent start that already holds the claim makes disposal a no-op. The lease's Drop releases the claim after completion/error/cancellation. |
| 10 | The disposal executor calls `VmHostState::kill_scope` followed by `discard_artifacts`, and **has no observation-store argument or write path**. `crates/overdrive-host/src/vm_host_state.rs:229` writes cgroup.kill and awaits its existing settle loop; `:259` removes run-directory/clone artifacts. The existing terminal row and cause are not rewritten by disposal. |
| 11 | The separate ReclaimAllocation executor (`reclamation.rs:176`) re-reads and refuses an already-terminal row before host effects; only an authorized non-terminal allocation receives PlatformReclaimed. The existing terminal disposal and non-terminal ending-authorship paths must not be conflated. |

The exit observer is composed in the fixture. It selects cancellation before
receiving an event (`worker/exit_observer.rs:198`), then awaits its existing
retry procedure (`:206`, `:342`). VmDriver's exit watcher emits only if
`ClaimGuard::try_begin_ending` wins from Held
(`vm_driver.rs:934`, `:2129`). In the corrected terminal-disposal schedule,
the reclamation lease is EndingInFlight during kill; afterward the claim is
absent. The old watcher's disposal-induced exit therefore cannot win this
ending-authorship check. Yielding it and the observer after disposal leaves the
current row **and occurrence history unchanged** in the seeded test.

## Seeded executable evidence and adapter fidelity

File: `crates/overdrive-sim/tests/vm_finalize_failed_ownership_spike.rs`.
Both tests declare `/// CONTRACT_SHAPE: bounded-change.`.

The fixture starts a valid VM Service through production convergence with the
real VmDriver/ProbeRunner and exit observer. Existing SimVmm, SimCgroupFs,
SimCgroupAccounting, SimObservationStore, and the existing network driven port
provide substrate boundaries; a Unix protocol peer supplies READY, receives
production EXEC, and remains connected without reporting EXIT.

The stock SimVmm and SimVmHostState are **independent models**: calling the
latter's kill_scope alone cannot flip the former's process-liveness map. The
fixture explicitly couples their existing methods through test-local Vmm and
VmHostState driven-port implementations:
successful Vmm::create records the actual config's allocation/scope/returned PID
into SimVmHostState; host kill_scope terminates only SimVmm controls created in
that scope, then delegates host-state removal. This models their shared kernel
substrate; it does not add a Sim API, production seam, status seed, or owner
mechanism. Production VmReclamation—not the test—decides whether to kill.

A positive control invokes registered VmReclamation while the live claim is
held and proves that this same coupled substrate is **not** killed. The terminal
case releases authorship through real ServiceLifecycle finalization, advances
the existing 30-second interval, and invokes the registered reclamation tick.
The fixture drives that tick explicitly; it does not itself test the automatic
broker cadence loop. The real loop registration/submission is established by
the production trace above. No production binary or native process is spawned.

Seed 257204 controls SimObservationStore and the bounded SimClock progress
schedule. Diagnostic absolute Unix timestamps are not replay inputs or
assertions. The fixture's beacon-connect timeout is not a product deadline.
The explicit fixture Vmm termination used for disposal occurs **after** the
reclamation assertions; it cannot cause the passing result.

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test vm_finalize_failed_ownership_spike --no-capture --no-fail-fast
```

| Run | Meaning and result |
| --- | --- |
| `f8137482-2e26-4ef7-9bdc-36db6b20f268` | Initial incomplete-owner invariant failed; fail-fast skipped control. This is not defect proof. |
| `3cd9f334-22f5-4211-99fc-eb0317e177cb`, `d55bb72d-8ecb-4ed6-b66a-e43158800c3e` | Initial direct-stop-only invariant FAIL/control PASS. Both omitted VmReclamation and required an unaccepted stop-before-release rule. Their interpretation is withdrawn. |
| `15c02319-94ad-448e-988a-b2253d60030c` | Corrected combined-owner invariant **PASS**, orderly-stop control **PASS**, xtask exit 0. After reclamation: live=false, host_artifacts_absent=true, row_unchanged=true, occurrences_unchanged=true. |
| `2daeee77-b932-428b-a39c-58536bd7eef2` | Unchanged corrected test rerun: **2 PASS**, xtask exit 0, identical seed-257204 outcome. |

The retained test asserts the accepted convergence contract, not the withdrawn
direct-stop ordering. It includes the real host-effect decision and lease path,
but remains simulation evidence: no claim about actual cgroup/netns/TAP or
redb effects is derived from its result.

## Native exit classification remains a separate question

CloudHypervisorVmm creates the process with kill_on_drop(false)
(`crates/overdrive-host/src/vmm.rs:291`); existing native test
`crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:1460`
relies on survival across server shutdown. The long-lived example joins its
server threads; overdrive-init executes the workload before reading
SHUTDOWN/EOF (`crates/overdrive-init/src/main.rs:201–206`). Those facts explain
why Driver-entry removal need not immediately terminate a VMM. They do not
prove an orphan once host reclamation is included.

Existing reclamation **can** account for later VMM death without an explicit
example stop. That is a supported mechanism, not proof of the trigger in the
historical VM-302 trace. Read-only inspection of the existing native capture at
`/tmp/svm-e10-matrix/e10-vm-302/` found the following:

| Captured fact | Evidence |
| --- | --- |
| Serve listening at `12:05:52.895907Z` | `serve.log:4` |
| VM EXEC released at `12:05:54.483769Z` | `serve.log:6` |
| Equal counter-36 write rejected at `12:05:57.924023Z` | `serve.log:7`, incoming/stored writer both local |
| Earlier describe: Failed(35), reason Started | `service-describe.log` |
| Stopped describe: Failed(36), reason crashed, exit None, signal None | `service-vm-http-302-stopped.log` |
| Operator stop command returned its normal response | `service-vm-http-302-stop.log`; no timestamp in this file |

Read-only commands used `ssh ubuntu@<metal-host> 'sudo -n ls -R
/tmp/svm-e10-matrix'`, then `wc -l`, `grep -nE`, and `sed -n '1,100p'`
on those exact files. Remote `rg` was unavailable, so grep/sed were used. No
lease was acquired, command workload run, cleanup performed, or host file changed.

**Inference from this capture and the existing cadence:** the observed
counter-36 contention is roughly five seconds after this fresh serve started,
well before its normal first 30-second resync. The ordinary first steady-state
VmReclamation sweep therefore does not explain that early counter-36 winner
under the documented/current registration and timer path. There is no
reclamation event or VMM stderr/exit detail in this eight-line serve log to
identify a different trigger. This bounds the hypothesis; it does not turn
network teardown into a proven VMM-exit cause, nor claim a precise operator-stop
timestamp absent from its capture. No fresh native run or shared-lease
interference was performed here.

`classify_vm_exit` (`vm_driver.rs:1971`) intentionally prefers a guest EXIT
report and does not use the hypervisor's own exit code; no report/no signal maps
to a crash with absent code/signal. Such a row alone does not show that network
removal killed the VMM. In the reproduced **terminal disposal** ordering,
reclamation leaves the original StartupProbeFailed row and history unchanged;
it does not itself explain a newly accepted crash observation. A native trace
showing an additional crash row must identify the actual writer and whether
its watcher observed a Held claim (including any intervening same-ID attempt).
That is still unresolved; it is not authorized evidence for changing the
classifier or a generalized late-attempt fence.

Zero cleanup deltas are resource evidence, not causal classification evidence.
This ruling neither approves nor rejects a particular native stop result merely
from the cleanup ledger. It does **not** authorize relaxing an expected
Terminated outcome to Terminated-or-Failed without establishing the actual
already-authored terminal/stop trajectory.

## Handoff

No public signature, accepted state meaning, cleanup ownership, ADR-0098
implementation, or ADR-0099 acknowledgement contract changes. Do not implement
the withdrawn direct-stop proposal from this investigation. The prior ADR-0099
Exec witness remains valid as an Exec witness; its driver-ownership statement
does not generalize to VM, whose ending-authorship handoff authorizes reclamation.

Artifacts retained: this ruling, the corrected seeded convergence test, and a
bounded clarification in the earlier restart-write ruling. No production code,
crafter-owned test, DES event, commit, native E09 state, or independent review
artifact was changed. The preliminary ADR-0100 draft was removed; its proposal
is documented as withdrawn here and has no implementation authority.
