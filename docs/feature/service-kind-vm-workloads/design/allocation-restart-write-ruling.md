# Allocation restart write ruling

## Disposition

- Date: 2026-09-07. Scope: component-level DESIGN, propose mode; Service-kind VM workloads, step `02-03`, E10 cleanup blocker.
- Verdict: **the rejected restart-write defect is reproduced through the current production convergence entry point in seeded `overdrive-sim`**. This is not merely a same-tick collision between two serial reconcilers.
- Correction to the initial hypothesis: the accepted competing row is **`Terminated`, counter 36, authored by the old Exec allocation attempt's exit observer**, not the preceding `Failed`, counter 35. The replacement Exec allocation attempt is running while the allocation's current observation remains Terminated. Its Running hook is released despite no accepted Running occurrence.
- Recommended bounded contract correction: recognize compound-write rejection at the existing successful restart publication boundary; fresh-read/rebuild and re-propose once, then allow Running-confirmed effects only on acceptance. Exhausted rejection unwinds the newly started attempt using the existing awaited unwind sequence. See Proposed ADR-0099. No new persistence subsystem, writer queue, recovery protocol, allocation identity, or public API is required.
- **ADR-0098 is not necessary to explain or correct this demonstrated defect.** Its missing-binding premise is not established by the supplied native evidence or its focused fixture. Preserve its existing dirty implementation; removal or withdrawal is a separate user disposition, not an action taken by this ruling.
- Independent DESIGN review is required before the original step's crafter resumes against ADR-0099. Nothing here self-approves the amendment or claims E10 green.

## Research performed before local implementation inspection

The first substantive investigation after mandatory instructions was actual web search, not local pattern selection. Queries included `site.kubernetes.io docs resourceVersion conflict retry status update`, `site.github.com/kubernetes/kubernetes pkg kubelet status status_manager.go syncPod updateStatus`, and `site.github.com/hashicorp/nomad client allocrunner taskrunner updateTaskState restart`. The mechanisms below were verified in official documentation and pinned upstream source, not inferred from search snippets.

| System / boundary | Verified mechanism | Applicability and important difference |
| --- | --- | --- |
| Kubernetes API optimistic update | A stale `resourceVersion` update is rejected with HTTP 409. `client-go`'s `RetryOnConflict` requires each attempt to fetch the current object again, modify it, and return the update error; retries are bounded by the supplied backoff. | A rejected write is not an acknowledgement. A retry must use current input. Kubernetes resource versions are server-controlled optimistic concurrency, not Overdrive's caller-stamped LWW register. Neither API accepts two equal local versions as distinct writes. |
| Kubernetes kubelet status publication | The status manager maintains an incrementing local cached status version separately from its API-acknowledged version. One publisher goroutine drives status syncs. Failed patches return before advancing the acknowledged version; the unsynced version is retried by later updates or periodic sync. Pod UID checks prevent an old local status being applied to a different Pod incarnation. | Runtime progress and API status acknowledgement are distinct. Kubernetes does **not** establish a universal rule that a container must be killed whenever remote status publication fails. Importing its cache/publisher ownership would be a new architecture here, not a small rejection fix. |
| Nomad client task lifecycle and server reporting | A task restart updates the local task state and restart event. Local state-store failure is logged, and the allocation state updater is still notified. Client allocation updates are batched; failed `Node.UpdateAlloc` RPCs are returned to pending updates; `AcknowledgeState` runs only after successful RPC completion. | Nomad deliberately separates local task progress from server acknowledgement. It does not silently acknowledge a failed RPC, but it also does not roll back every task restart because reporting failed. Its local allocation/task authority and asynchronous reporting are not equivalent to this allocation-current-row gate. |

Primary sources:

- [Kubernetes API concurrency control](https://kubernetes.io/docs/reference/using-api/api-concepts/#resource-versions): stale update rejection and resource-version semantics.
- [client-go v0.34.0 retry implementation](https://github.com/kubernetes/client-go/blob/v0.34.0/util/retry/util.go#L68): fetch anew on each attempt; return the original update error; bounded retry.
- [kubelet v1.34.0 status manager](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/kubelet/status/status_manager.go): publisher loop at lines 249–258; local version increment at 845–858; unsynced selection at 970–991; UID check and patch-error return at 1005–1043; API acknowledgement at 1061; pending-version comparison at 1083–1086.
- [Nomad v1.10.0 task runner](https://github.com/hashicorp/nomad/blob/v1.10.0/client/allocrunner/taskrunner/task_runner.go): restart state at 906–909; in-memory/local-store update and notification at 1341–1394; restart-event accounting at 1448–1451.
- [Nomad v1.10.0 client reporting](https://github.com/hashicorp/nomad/blob/v1.10.0/client/client.go): asynchronous update collection at 2212–2241; RPC failure restoration at 2281–2290; acknowledgement only on success at 2293–2305.
- [Nomad restart policy](https://developer.hashicorp.com/nomad/docs/job-specification/restart): task restart within an allocation is distinct from scheduling a replacement allocation.

**Research conclusion:** distinguish runtime truth, write acceptance, and reconciliation policy. Reuse fresh-read/re-propose only where Overdrive's own accepted contract requires accepted Running before its release hooks. The sources do not justify copying a durable publisher, adding a new system of record, or redefining Running/Stable.

## Revalidated contract and production ownership

Authoritative context inspected: selected `deliver/roadmap.json` step `02-03`; feature-delta DESIGN and lifecycle-gate sections; architecture brief Service-kind VM health section; ADRs 0092–0098 and ADR-0098 review; additionally ADR-0077's single-writer limitation and ADR-0078's predecessor-derived crash facts. ADR-0097's corrected `Option<UnixInstant>` is retained. GH #281 is unrelated.

The following references describe the dirty working implementation inspected and reproduced, not an assertion about HEAD alone.

| Order | Actual owner, state, and production path |
| --- | --- |
| 1 | `spawn_convergence_loop`, `crates/overdrive-control-plane/src/lib.rs:3332`, drains broker evaluations and awaits each `run_convergence_tick` serially (`:3378`). One tick number may be shared; serial predecessor-derived writes alone do not collide. |
| 2 | `run_convergence_tick`, `crates/overdrive-control-plane/src/reconciler_runtime.rs:1527`, uses registered reconcilers, production hydration, persisted view, output validation, and action dispatch. The spike enters here for both `workload-lifecycle` and `service-lifecycle`. |
| 3 | The initial `StartAllocation` produces Running(34). `ServiceLifecycle` consumes a failed startup result after its declared deadline and authors `FinalizeFailed { ServiceFailed::StartupProbeFailed }`, `crates/overdrive-reconcilers/src/service_lifecycle.rs:1272`. This is the operator Service startup failure, not a failed `Driver::start`. |
| 4 | `FinalizeFailed`, `crates/overdrive-control-plane/src/action_shim/mod.rs:1564`, distinguishes Stable from a genuine terminal. Genuine terminal cleanup awaits mTLS stop, tears down the allocation netns, releases its in-memory slot binding, calls the terminal hook, then publishes Failed(35) with no workload address (`:1764`, `:1789`). It does not call Exec `Driver::stop`. The terminal hook stops probes; Exec process ownership is still live until restart's stop-half. |
| 5 | `WorkloadLifecycle` alone selects the restart under its existing budget, `crates/overdrive-reconcilers/src/workload_lifecycle.rs:950`, `:1052`. `restart_allocation_action` (`:1429`) uses the **same allocation ID**, not a fresh allocation. No allocation-attempt identifier is added by this ruling. |
| 6 | `RestartAllocation` captures Failed(35) before awaiting driver stop, `crates/overdrive-control-plane/src/action_shim/mod.rs:2259`, `:2263`, `:2280`. It retains that snapshot across old-attempt stop, cleanup, replacement network provision, and replacement driver start. |
| 7 | Exec stop sets its intentional-stop flag before SIGTERM and awaits its watcher, `crates/overdrive-worker/src/driver.rs:680`, `:706`. The independent exit observer can consume the old attempt's real exit during that await. Joining the watcher does not serialize the observer's observation write with the action shim. |
| 8 | `handle_exit_event`, `crates/overdrive-control-plane/src/worker/exit_observer.rs:451`, fresh-reads Failed(35), classifies the intentional stop as Terminated / `Stopped { by: Operator }`, and derives counter 36 (`:474`, `:479`). The terminal-attempt fence is Job-specific (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:109`), so this Service observation is accepted. The Operator label is the current classifier's output, not evidence of an operator stop command at this point. |
| 9 | Restart derives its proposal from the earlier Failed(35), also counter 36, `crates/overdrive-control-plane/src/action_shim/mod.rs:2562`. Its Running write returns `Ok(None)` at `:2612`. The arm handles only `Err` as failed publication; it proceeds to the allocation-driver routing index, mTLS installation, exit-emission release, and Running hook (`:2660`–`:2705`). |
| 10 | Current Terminated / Operator is not restartable (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1378`, `:1468`). A subsequent operator desired-stop branch selects only observed Running allocations (`:649`). This makes the lost Running publication consequential for cleanup: the replacement can be live without being selected by that observation-driven stop path. |

The snapshot labelled “allocation recovered” in the native log is printed by `build_alloc_status_row` **before store acceptance**, `crates/overdrive-control-plane/src/action_shim/mod.rs:398`. It is not evidence that a recovery observation committed.

**VM ownership clarification, 2026-09-07:** row 4's retained process-owner
statement is specific to the **Exec** witness; it must not be generalized to VM.
VM finalization releases an **ending-authorship claim**, and the existing
registered VmReclamation subsequently owns terminal artifact disposal through
its execution-time claim, host `kill_scope`, and `discard_artifacts`. Its
30-second resync is not a second Driver::stop. Seed 257204 in the separate
[VM finalization/reclamation ruling](vm-finalize-failed-ownership-ruling.md)
proves that combined path kills the surviving VMM while preserving the authored
StartupProbeFailed row and occurrence history. The earlier inference that a
missing Driver entry alone proves an orphan is withdrawn; no direct-stop
amendment or new owner is justified by that intermediate state. This does not
alter ADR-0099's independently proven Exec write-acknowledgement correction.

### Store fidelity, retry, and shutdown

`LogicalTimestamp::dominating`, `crates/overdrive-core/src/traits/observation_store.rs:327`, computes `max(tick + 1, prior.counter + 1)`; equal counter and writer do not dominate. The compound lifecycle-write contract explicitly returns `Ok(None)` for rejection (`:2066`). Local redb enforces it inside its write transaction (`crates/overdrive-store-local/src/observation_backend.rs:460`, `:473`); Sim enforces the same comparator while holding its current/occurrence lock (`crates/overdrive-sim/src/adapters/observation_store.rs:308`, `:318`). Neither appends a rejected occurrence. The store is behaving correctly.

The Sim store's synchronous-ready operations can mask interleavings in a tight single-threaded test. The spike supplies legal delay at the **existing Driver port**, like the repository's existing `terminal_contention` invariant supplies scheduling at the ObservationStore port. It does not alter the comparator, fabricate a competing allocation row, or add a public Sim seam.

The convergence owner checks shutdown between completed ticks, not by cancelling a pending restart future (`lib.rs:3400`). The exit observer selects cancellation before receiving each event and completes `run_with_retry` for an event already consumed (`exit_observer.rs:198`, `:206`). That retry loop retries errors, not `Ok(None)` (`:342`); restart currently also has no rejection retry. The spike uses no task abort, shutdown, crash/restart, or imaginary detached owner. No change to these shutdown boundaries is proposed.

## Reproduction

Diagnostic triple, fixed before implementation exploration:

- Hypothesis: distinct observations from the current owner path use the same LWW version, and restart effects continue after its Running write is rejected.
- Prediction: seeded Sim retains a non-Running observation while the replacement driver and Running hook advance, matching native evidence.
- Falsification: the owner cannot produce the version tie, rejection prevents those effects, or the residual network belongs to another allocation/attempt.

The hypothesis is supported, with the competing-state correction recorded above.

Reproducer: `crates/overdrive-sim/tests/allocation_restart_write_spike.rs`. Every live test carries `CONTRACT_SHAPE: bounded-change.` The fixture persists a valid Service intent; real registered WorkloadLifecycle produces the initial allocation and restart; real registered ServiceLifecycle consumes one injected failed **probe result** and produces the terminal action; the real exit observer produces the competing allocation row. No allocation-status row is seeded. The one-failure, one-second startup declaration shortens E10's three-failure journey without changing its terminal action or restart owner.

The existing `SimDriver::inject_exit_after` and `SimClock` schedule the old attempt's intentional exit while the existing Driver stop future is pending. Seed `257203` determines its delay. The test models no actual netns/BPF effects: AppState's non-mTLS Exec composition is used for the ordering invariant. The native evidence is a separate host-effect layer, not a claim that Sim observed kernel resources.

Command, run twice against the current dirty implementation (second run after formatting only):

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test allocation_restart_write_spike --no-capture
```

| Scenario | Seed | Result |
| --- | ---: | --- |
| No competing exit during stop | 257203 | PASS: Running(34) → Failed(35) → Running(36); restart count 1; driver Running; two Running hooks. |
| Old attempt exit completes during stop | 257203 | **FAIL**: Running(34) → Failed(35) → Terminated(36); no replacement Running occurrence; convergence returns `Ok(())`; replacement driver Running; two Running hooks; stored restart count 0. |

The failing safety assertion is “replacement Running hook released with no accepted Running observation.” Nextest reports one passed / one failed; xtask exits 1 after the guest nextest failure (100). This intentional RED is retained as a DESIGN reproducer, not an implementation completion claim or DES event. First run ID: `fa654829-2f9c-4993-8bfc-d4d470675e2d`; second: `09c4440a-824d-49d0-90a8-4f000d8971c8`.

### Native evidence checked without taking the host lease

Read-only SSH inspected existing root-owned evidence at `ubuntu@151.115.99.251`:

```sh
ssh -o BatchMode=yes ubuntu@151.115.99.251 'sudo sed -n "1,65p" /tmp/accra-e10-spike/serve.log; sudo sed -n "1,110p" /tmp/accra-e10-spike/service-describe.log'
```

- `10:05:43.905809Z`: initial allocation `alloc-service-exec-http-302-0` Running proposal includes `Some(10.99.0.2)`; initial intercept installation follows.
- `10:05:47.240949Z`: a cleanup call sees neither held binding nor prior address. This message alone cannot establish a missing teardown.
- `10:05:47.281838Z`: builder logs recovery from Failed; `10:05:47.282069Z`: Local store rejects incoming `(36,local)` against stored `(36,local)`.
- `10:05:47.284513Z`: replacement intercept installation succeeds **after rejection**.
- The immediate describe snapshot shows Failed(35). A separate earlier matrix capture, `verification/expectations/E10-vm-service-http-cross-driver-status/evidence/product-run.out:36`, shows Terminated(36), Operator reason, and the failed cleanup ledger at `:47`–`:50` with `ovd-ns-0000` / `ovd-hv-0000` (network delta 2). Its `product-run.meta` records canonical metal invocation from `09:56:01Z` to `09:57:19Z`, exit 1. These are two runs, not one reconstructed timeline.

These are historical native captures, not a fresh run against an attested current source hash. They corroborate the independently reproduced ordering and show the host-effect consequence; they do not replace the seeded regression. No host process was stopped, no network object removed, and no shared lease holder interrupted. New E10 capture remains a post-implementation obligation.

## ADR-0098 necessity ruling

ADR-0098's accepted fixture starts from a synthetic Running row plus no allocation binding and verifies address-derived teardown. It proves the helper's response to that input; it does not establish how the production owner lost its binding while leaving its original netns present.

In the reproduced owner path, genuine terminal cleanup already releases the old binding **after teardown**. Restart's cleanup can consequently observe no binding and no address because that old attempt has already been cleaned. Restart then provisions a replacement assignment. Its rejected Running row never persists the replacement workload address. Inferring that old cleanup must recover a lost binding confuses the old allocation attempt's completed teardown with the replacement attempt's later provision.

Therefore ADR-0098 neither prevents the equal-version rejection nor gates replacement intercept installation on acceptance. It is **not a dependency of the bounded correction** and is not established as necessary by E10. This ruling does not claim that its helper can never be useful on any other path. A different production-reachable missing-binding defect would need its own reproducible evidence and explicit scope approval. Do not expand this fix into address-based recovery, boot adoption, or persisted network ownership.

The current dirty ADR-0098 code and its approved review are preserved. If the user directs removal, compare E10 with that mechanism absent after the acknowledgement correction; do not silently erase previously accepted work while acting on this ruling.

## Recommended mechanism and alternatives

Proposed ADR-0099 specifies an **existing-contract correction**, not a new architecture: the same action shim retains restart effect ownership, the same compound store returns acceptance, the same reconcilers own decisions, and the same in-memory allocation slot owns structural teardown. Public signatures, persisted row shapes, parser/CLI/wire shape, and C4 context/container boundaries remain unchanged.

1. **Bounded fresh-read/rebuild/re-propose at rejected Running publication — recommended.** Reuse the existing stop arm's finite two-proposal discipline (`action_shim/mod.rs:2772`) and the existing restart unwind. No new publisher, durable pending record, global lock, or clock. Rebuild predecessor-derived crash facts from the fresh accepted row, not the rejected candidate, to preserve ADR-0078 and avoid double-counting.
2. **Unwind immediately on first rejection — viable simpler safety alternative, not selected.** It prevents the leaked replacement but unnecessarily abandons this already-authorized restart because its own old-attempt exit won one expected contention. A single re-proposal can retain the successful restart without changing its policy owner.
3. **Only refresh the prior after stop — insufficient as a complete acknowledgement rule.** A read and compound write remain separate async calls. Acceptance is already returned; ignoring it remains wrong. No new test or requirement for unrelated writers follows from this observation.
4. **Store-assigned versions / serialized publisher / durable pending lifecycle — rejected for this task.** These would change public API, consistency/ownership, or persistence architecture. The bounded witness does not show that dependency is unavoidable; Kubernetes/Nomad adoption is not authorization.
5. **ADR-0098 address fallback / larger tick counter — rejected as remedies for this defect.** Neither distinguishes accepted from rejected Running publication. The equal proposal comes from two owners sharing an earlier predecessor, not merely an inadequately large tick.

Limits: this ruling proves the selected same-ID Exec restart schedule, not every lifecycle writer or every delayed-exit schedule. It does not authorize generalized writer serialization, new allocation-attempt fencing, broker changes, hydration policy changes, or remote-peer conflict semantics. Those are not requirements of this amendment.

## Handoff and self-review

Independent review must check the reproduction's real owner path, its explicit non-kernel scope, predecessor/counter semantics, proposed bounded rejection behavior, and ADR-0098 necessity—not merely whether a helper test passes. Before implementation, approve or reject Proposed ADR-0099 separately. The existing crafter must not invent an error variant or public retry method to implement it.

The proposed change adds at most one fresh read and one compound re-proposal on an actual rejection; the no-conflict path is unchanged. Its reliability target is the already-promised Running confirmation and E10 cleanup, not new availability/performance targets. The design reuses current ports and components, has a failing executable witness and passing control, and records limits rather than adopting a preferred external architecture. No issue, DES log, commit, production code, mutation exclusion, or existing review artifact was changed.
