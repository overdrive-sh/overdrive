# Slice 4 — Control-plane cgroup isolation (slice creation + bootstrap enrolment)

**Story**: US-04
**Walking skeleton row**: 4 (cross-cutting — control plane structurally protected)
**Effort**: ~1 day (Linux developer; integration test depends on real kernel)
**Depends on**: Slice 2 (ProcessDriver) for the workload-side cgroup infrastructure; Slice 3 for an actual workload to assert against.

## Outcome

At server startup, `overdrive serve` creates `overdrive.slice/control-plane.slice/` (if not present) and enrols the running process into it. A cgroup v2 delegation pre-flight check refuses to start with an actionable error if delegation is missing or cgroup v2 is unavailable. ProcessDriver from Slice 2 already places workloads in `overdrive.slice/workloads.slice/<alloc_id>.scope`; this slice adds the symmetric control-plane side.

A new integration test (gated `integration-tests`, in `crates/overdrive-control-plane/tests/integration/cgroup_isolation.rs`) starts a real server, submits a job whose binary bursts to 100% CPU (e.g. `stress --cpu N`), and asserts that `overdrive cluster status` continues to respond within 100 ms during the burst.

## Value hypothesis

*If* the kernel isn't actually enforcing the slice split, *then* the §4 "control plane runs in dedicated cgroups with kernel-enforced resource reservations" claim is paper. The integration test is the disproof attempt: if the workload starves the control plane in this test, the test fails and we know the slice isn't doing what we claimed. *Conversely*, if the test passes against a real Linux kernel, the structural defence-in-depth claim is real, not aspirational.

## Disproves (what's the named pre-commitment we're falsifying)

- **"Slice creation can wait for systemd unit packaging in DEVOPS."** No — the in-process bootstrap is the SSOT for the slice topology. A future systemd unit can pre-create the parent slice, but the server still owns its per-instance hierarchy. Deferring this leaves the control plane unprotected during dev usage.
- **"On a single-node co-located host, cgroup isolation is a Phase 2+ luxury."** No — a runaway process on the same host as the control plane is exactly the case §4 calls out; the kernel-level split is the only structural answer. Phase 1 has cgroup-isolation in scope precisely because the topology is co-located.

## Scope (in)

- cgroup v2 delegation pre-flight check at `overdrive serve` startup. If unavailable: log actionable error naming the cause (no cgroup v2, or no delegation to UID), exit non-zero, no /v1 endpoint binds.
- Server-bootstrap CgroupManager: creates `overdrive.slice/control-plane.slice/` (idempotent — existing slice is fine), writes own PID into `cgroup.procs`. May also create the parent `overdrive.slice/workloads.slice/` so ProcessDriver doesn't have to.
- Integration test (`integration-tests` feature, Linux-only): submits a CPU-burst job, asserts cluster status responds within 100 ms while burst is active.
- Smoke test: at server boot, read `/proc/self/cgroup`, assert it shows the control-plane slice path.
- (Optional, DESIGN-decided) `overdrive serve --cgroup-root <path>` flag for non-default mount points.

## Scope (out)

- §14 right-sizing reconciler reading memory pressure (Phase 2+).
- eBPF-based pressure detection (Phase 2+).
- Per-workload `cpu.weight` / `memory.max` enforcement on the workload scope — this overlaps with Slice 2's "wire `Resources` from `AllocationSpec` into cgroup limits" question; if Slice 2 doesn't do it, this slice adds it as a contained extension.
- systemd unit file packaging (DEVOPS / packaging-time concern).
- **Scheduler taint/toleration support** — the other half of GH #20 — is explicitly DEFERRED. Phase 1 is single-node; with exactly one node there is no placement choice for a taint to gate against, so taint/toleration logic delivers no Phase 1 value. Lands in a later phase alongside multi-node + Raft. The user is expected to split GH #20 into two issues to track this independently.

## Target KPI

- Linux integration test passes: under 100% CPU workload burst, `overdrive cluster status` returns within 100 ms on localhost.
- Server-boot smoke test asserts the running process's cgroup path matches `overdrive.slice/control-plane.slice/`.
- Pre-flight check: a deliberate test that runs `overdrive serve` with cgroup v2 unavailable (e.g. on a cgroup v1 host or via a permission-stripped UID) exits non-zero with a structured error naming the cause.

## Acceptance flavour

See US-04 scenarios. Focus: kernel-enforced isolation under real CPU pressure (real Linux integration test, not a simulation), pre-flight check actionable error, server-boot enrolment.

## Failure modes to defend

- Server is run as a non-root user without cgroup v2 delegation: pre-flight check refuses to start. Error message names cgroup v2 delegation as the missing piece and points to a fix (run via systemd unit, or grant delegation to UID).
- cgroup v2 not available (cgroup v1 host or no `cgroup` filesystem): pre-flight check refuses to start with a different actionable error.
- The control-plane slice already has resource limits set by an outer systemd config: server enrolment must NOT override them (the slice is owned by the host's systemd, the server only enrols itself).
- Hosts where cgroup v2 is the default but delegation is configured for a different UID: pre-flight refuses; error names the UID mismatch.

## Slice taste-test

| Test | Status |
|---|---|
| ≤4 new components | PASS — pre-flight check, CgroupManager, integration test harness (3) |
| No hypothetical abstractions landing later | PASS — extends Slice 2's cgroup wiring symmetrically |
| Disproves a named pre-commitment | PASS — see above; the integration test IS the disproof attempt |
| Production-data-shaped AC | PASS — real Linux kernel, real CPU pressure, real CLI responsiveness measurement |
| Demonstrable in single session | PASS — `cargo nextest run -p overdrive-control-plane --features integration-tests` runs the burst test |
| Same-day dogfood moment | PASS — Linux developer can run the integration test on their workstation |
