# Walking Skeleton — phase-1-first-workload

## Inheritance, not replacement

This feature **extends** the walking skeleton landed by
`phase-1-control-plane-core`. The prior WS was a five-step CLI loop:

1. `overdrive job submit ./payments.toml` — CLI binary commits a Job to IntentStore.
2. (internal) — IntentStore commits a redb transaction.
3. `overdrive cluster status` — renders the registered reconcilers and broker counters.
4. `overdrive alloc status --job payments` — renders the (then-empty) allocation set.
5. (internal) — `noop-heartbeat` reconciler ticks; the `ReconcilerIsPure` and `AtLeastOneReconcilerRegistered` invariants are visible to DST.

The prior WS was demo-able to stakeholders with one apologetic note in
step 4: "the scheduler and ProcessDriver are the *next* feature; for
now the allocation set is empty." This feature **fills that gap and
extends the journey three steps further**:

- Step 4 — now lists a real Running row when a job has been submitted.
- Step 5 — kill the workload externally; the platform converges back.
- Step 6 — burst CPU under a workload; control plane stays responsive.
- Step 7 — `overdrive job stop`; the platform drains cleanly to Terminated.

## Walking-skeleton scenarios per journey step

The journey-extension YAML (`journey-submit-a-job-extended.yaml`)
enumerates seven steps. The DESIGN-wave `wave-decisions.md` D5 / ADR-0027 confirm
the driving-port shapes. Per project rule, every CLI command identified
by DESIGN must have at least one subprocess scenario. The mapping:

| Step | Driving port | Walking-skeleton scenario | Lane |
|---|---|---|---|
| 1: submit job | `overdrive job submit` (subprocess) | INHERITED — `phase-1-control-plane-core::tests/integration/submit_round_trip.rs` carries forward verbatim. No new WS scenario. | inherited |
| 3: cluster status — both reconcilers visible | `overdrive cluster status` (subprocess) | **3.14** in test-scenarios.md — extends the prior `cluster_status` test to assert both `noop-heartbeat` AND `job-lifecycle` are listed. | `@in-memory` |
| 4: alloc status — Running row appears | `overdrive alloc status --job <id>` (subprocess) AND `Driver::start` (Driver trait) | **3.1** — submit a 1-replica job; observe a real Running row via the same CLI command the prior WS exercised. The augmentation is "the row is no longer empty." | `@real-io @adapter-integration` |
| 5: recover from crash | `kill -9` externally + repeated `overdrive alloc status` polls | **3.7** — kill the workload process; observe Terminated → fresh alloc → Running. | `@real-io @adapter-integration` |
| 6: control plane stays responsive | `overdrive cluster status` while CPU bursts | **4.2** — submit CPU-burst workload; assert `cluster status` returns within 100 ms during the burst. Plus **4.1** — the boot-time enrolment that makes 4.2 possible. | `@real-io @adapter-integration` |
| 7: stop and drain | `overdrive job stop <id>` (subprocess) — NEW | **3.9** — `overdrive job stop payments` drives Running → Draining → Terminated; the cgroup scope is removed. | `@real-io @adapter-integration` |

Six walking-skeleton scenarios in total (2.2 is also tagged `@walking_skeleton` because it is the foundation step 4 builds on — ProcessDriver starting `/bin/sleep` under cgroup supervision; the integration test asserts the cgroup path appears in `/proc/<pid>/cgroup`). The earlier walking skeleton's five steps continue to pass byte-identical.

## Litmus test (per nw-test-design-mandates §Walking Skeleton Litmus)

For each of the six walking-skeleton scenarios:

| Scenario | Title describes user goal? | Then steps describe user observations? | Non-technical stakeholder can confirm "yes, that is what users need"? |
|---|---|---|---|
| 2.2 | "ProcessDriver starts a real /bin/sleep child and reports it Running" — Yes (operator sees a Running allocation backed by a real process) | Yes — `/sys/fs/cgroup/.../cgroup.procs` contains the PID; observable from the operator's shell | Yes |
| 3.1 | "Submitting a 1-replica job results in a Running allocation visible via CLI" — Yes (this is THE journey closure for step 4) | Yes — CLI output renders Running, node_id, spec digest | Yes |
| 3.7 | "When the workload process is killed externally, the lifecycle reconciler converges back to Running" — Yes (the platform self-heals; operator types nothing) | Yes — CLI output transitions Terminated → fresh alloc → Running | Yes |
| 3.9 | "Stopping a Running job drives it through Draining to Terminated" — Yes (operator-facing affordance closes the lifecycle) | Yes — CLI exits 0, output contains "Stopped job 'payments'.", state is terminal | Yes |
| 3.14 | "cluster status renders both noop-heartbeat and job-lifecycle in the registry" — Yes (operator confirms the new reconciler is wired) | Yes — CLI output contains both names | Yes |
| 4.1 | "A successful overdrive serve boot enrols the running PID in the control-plane slice" — Yes (operator sees the kernel-enforced isolation in `/proc/self/cgroup`) | Yes — observable via host tooling | Yes |
| 4.2 | "cluster status returns within 100 ms while a workload bursts to 100% CPU" — Yes (the kernel actually enforces the slice split) | Yes — wall-clock latency under burst | Yes |

All seven walking-skeleton scenarios pass the litmus test.

## What the WS extension proves

The prior feature's WS proved: "submit goes through, the reconciler primitive is real (proof-of-life), the CLI ↔ server round-trip closes." It DID NOT prove: "a real OS process runs the workload, the platform self-heals, the cgroup story holds under real CPU pressure, the operator can stop a job."

This feature's WS extension proves all four. Specifically:

1. **The convergence loop closes against a real Linux kernel** (3.1, 3.7, 4.2). The reconciler primitive demonstrated by `noop-heartbeat` is now exercised by `JobLifecycle` against a real `ProcessDriver`, real cgroupfs writes, real `tokio::process::Command::spawn`, and a real Linux scheduler under CPU pressure.
2. **The §4 isolation claim is honest, not paper** (4.1, 4.2). The whitepaper §4 commitment that "control plane processes run in dedicated cgroups with kernel-enforced resource reservations" is asserted under a real workload burst against a real kernel. If the test fails, the claim is paper.
3. **The §18 reconciler primitive scales beyond proof-of-life** (3.1, 3.7, 3.9, 3.14). The `JobLifecycle` reconciler is the first non-trivial reconciler shipped against the trait. Its presence in the registry, its convergence behaviour, its purity under DST, and its observable end-to-end effect via `Driver::start` / `Driver::stop` all close.
4. **The operator-facing affordance is bidirectional** (3.9). `overdrive job submit` had a partner — `overdrive job stop`. Without it, the prior journey was one-way; the operator could start a workload but not cleanly end it. The WS extension closes the lifecycle.

## What the WS does NOT prove (deliberately deferred)

- **Multi-node placement**. Phase 1 is single-node; the BTreeMap input has exactly one entry. The determinism property still has to hold (covered by 1.2, 1.3 properties), but no scenario exercises real multi-node behaviour. Phase 2+.
- **Right-sizing under live load**. ADR-0026 D9 writes `cpu.weight` and `memory.max` from the spec; no scenario exercises live resize via `Driver::resize`. Phase 1 is start-time enforcement only; §14 right-sizing arrives in a later phase.
- **MicroVm / WASM driver convergence**. Only `ProcessDriver` ships in this feature. The `Driver` trait surface is the one the action shim calls; MicroVm / WASM impls land later under the same trait.
- **Workflow primitive convergence**. ADR-0023 §Alternative C explicitly defers; this feature ships only the action shim, not the workflow runtime.
- **`MigrateAllocation`**. Phase 3+ when `overdrive-fs` migration tooling lands.

These exclusions are intentional and tracked in `wave-decisions.md` § "Scope correction" (DISCUSS) + the DESIGN ADRs.

## Walking-skeleton lane discipline

Per project's two-lane model (`.claude/rules/testing.md`):

- **Default lane** (`@in-memory`): one walking-skeleton scenario (3.14) — exercises the in-process server with `SimDriver` + `LocalObservationStore`. macOS / Windows developers run this lane on every PR; no cgroup dependency.
- **Integration-tests lane** (`@real-io @adapter-integration`): six walking-skeleton scenarios (2.2, 3.1, 3.7, 3.9, 4.1, 4.2) — Linux-only, gated `--features integration-tests`. CI runs this lane on the Linux Tier 3 matrix per `.claude/rules/testing.md`.

Both lanes are first-class. The default lane exists so a developer on macOS can iterate against the same WS scenario shape (cluster status surfaces both reconcilers) without standing up a Linux VM. The integration-tests lane exists because §22 of the whitepaper is explicit that "real-kernel testing catches bugs at the boundary between Overdrive and the kernel — verifier rejections, kernel-version regressions, hook-attachment quirks, kTLS offload edge cases, LSM hook semantics" — none of which `SimDriver` can.
