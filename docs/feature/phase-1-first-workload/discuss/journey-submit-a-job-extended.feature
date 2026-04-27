# SPECIFICATION ONLY — NOT EXECUTED BY ANY TOOLING.
#
# Per `.claude/rules/testing.md`: ".feature files are banned project-wide.
# All acceptance and integration tests are written directly in Rust using
# `#[test]` / `#[tokio::test]` functions." The crafter (DELIVER wave)
# translates the scenarios below into Rust tests in
# `crates/<crate>/tests/integration/<scenario>.rs`, gated behind the
# `integration-tests` feature where they touch real infrastructure (real
# processes, real cgroups), or in `tests/acceptance/<scenario>.rs` for
# in-memory DST-shaped acceptance.
#
# This file exists so the DISCUSS wave can carry the full Given/When/Then
# library across the journey extension in one place. DO NOT add a
# `cucumber-rs`, `pytest-bdd`, or any other consumer of this file.
#
# Scope reminder: Phase 1 is SINGLE-NODE. Control plane and worker run
# on the same machine; there is exactly one node (the local host) and
# it is implicit. No node-registration verb. No taints. No tolerations.
# No multi-node placement choice.
#
# Coverage: happy-path (process job submit → place → start → run) plus
# the error paths flagged in journey-submit-a-job-extended.yaml step
# failure_modes blocks; crash recovery; cgroup isolation; stop and drain.

Feature: Submit a process job and watch it run on the walking-skeleton control plane

  Background:
    Given a single-node Overdrive control plane is running locally (control plane and worker co-located on one machine)
    And the local node row is implicitly present in `node_health` (server bootstrap detail; no operator action required)
    And no jobs are committed

  # ============================================================
  # Happy path — end-to-end: submit, place, run
  # ============================================================

  Scenario: A process job reaches Running on the local node
    Given Ana has a Job spec at `./payments.toml` requesting 2000 mCPU / 4 GiB, 1 replica
    When Ana runs `overdrive job submit ./payments.toml`
    And the lifecycle reconciler converges
    Then `overdrive alloc status --job payments` lists exactly one allocation
    And the allocation's `state` is `Running`
    And the allocation's `spec_digest` equals the digest Ana computes locally from `payments.toml`

  Scenario: A workload process is actually running on the host inside its cgroup scope
    Given a job `payments` is `Running` with `alloc_id = a1b2c3...`
    When the operator inspects the host's cgroup hierarchy
    Then a cgroup scope exists at `/sys/fs/cgroup/overdrive.slice/workloads.slice/a1b2c3....scope`
    And the scope's `cgroup.procs` file contains exactly the workload's PID

  # ============================================================
  # Scheduler — first-fit, capacity-aware (single-node)
  # ============================================================

  Scenario: Scheduler is deterministic for the same input
    Given a fixed input `(nodes, job, allocs)` (single-node BTreeMap with the local host)
    When the lifecycle reconciler converges twice from the same DST seed
    Then the placement decision returns the same `Result<NodeId, PlacementError>` both times

  Scenario: Scheduler rejects placement when the local node lacks capacity
    Given the local node has capacity 4000 mCPU / 4 GiB
    And a job `huge` requesting 10 GiB
    When Ana submits the job
    And the lifecycle reconciler converges
    Then `overdrive alloc status --job huge` shows zero `Running` rows
    And the output contains a `Pending` row whose reason names insufficient capacity
    And the message names both the requested resources and the local node's free memory

  Scenario: Scheduler accounts for already-placed allocations
    Given the local node has capacity 4000 mCPU / 8 GiB
    And a job `a` with 1 replica requesting 3000 mCPU / 6 GiB is already `Running`
    When Ana submits a second job `b` requesting 2000 mCPU
    Then `b` cannot place on the local node (only 1000 mCPU free) and shows Pending with the capacity reason

  # ============================================================
  # Process driver — start, status, stop, cgroup placement
  # ============================================================

  Scenario: ProcessDriver starts a child process and reports it Running
    Given a scheduled allocation with a valid binary path and arguments
    When the action shim calls `ProcessDriver::start`
    Then the driver returns an `AllocationHandle` carrying the child PID
    And subsequent calls to `Driver::status` return `AllocationState::Running`

  Scenario: ProcessDriver places the child process in the workload cgroup scope
    Given a scheduled allocation with `alloc_id = a1b2c3...`
    When `ProcessDriver::start` returns successfully
    Then the host cgroup `/sys/fs/cgroup/overdrive.slice/workloads.slice/a1b2c3....scope` exists
    And reading `/proc/<child-pid>/cgroup` shows the same scope path

  Scenario: ProcessDriver fails to start when the binary does not exist
    Given a Job spec with binary path `/nonexistent/payments`
    When the action shim calls `ProcessDriver::start`
    Then the driver returns a `DriverError` naming the binary path
    And the action shim writes `AllocStatusRow { state: Failed, reason: <message> }`
    And `overdrive alloc status --job payments` shows the Failed state with the actionable reason

  Scenario: ProcessDriver::stop sends SIGTERM, waits for grace, then SIGKILL if needed
    Given a `Running` allocation
    When the action shim calls `ProcessDriver::stop` with a 5-second grace window
    And the workload exits within the grace window
    Then the driver returns `Ok(())`
    And the workload's cgroup scope is removed
    And `Driver::status` returns `AllocationState::Terminated`

  Scenario: ProcessDriver::stop escalates to SIGKILL when the process ignores SIGTERM
    Given a `Running` allocation whose binary ignores SIGTERM
    When the action shim calls `ProcessDriver::stop` with a 1-second grace window
    Then SIGKILL is sent after the grace expires
    And the workload's cgroup scope is removed
    And the resulting allocation state is `Terminated`

  # ============================================================
  # Job-lifecycle reconciler — convergence and self-healing
  # ============================================================

  Scenario: Job-lifecycle reconciler converges to declared replica count
    Given a job `payments` with `replicas = 1`
    When Ana submits the job
    And the lifecycle reconciler converges
    Then `overdrive alloc status --job payments` shows one Running allocation
    And the allocation's `node_id` matches the local node

  Scenario: Process exits non-zero and the reconciler restarts it
    Given a job `payments` is `Running` with one replica
    When the workload process exits with a non-zero status code
    Then within N reconciler ticks the prior allocation's `state` is `Terminated`
    And shortly after, a new allocation with a fresh `alloc_id` appears in `Running` state on the local node

  Scenario: Job-lifecycle reconciler restarts a crashed allocation (SIGKILL)
    Given a job `payments` is `Running` with one replica
    When the workload process is killed externally with SIGKILL
    Then within N reconciler ticks the prior allocation's `state` is `Terminated`
    And shortly after, a new allocation with a fresh `alloc_id` appears in `Running` state

  Scenario: Job-lifecycle reconciler bounds restart attempts via libSQL backoff
    Given a job whose binary exits with status 1 immediately on start
    When the lifecycle reconciler attempts to start it M times in succession
    Then the reconciler's private libSQL `view` records the restart count
    And after the configured ceiling, the allocation enters `Failed (backoff exhausted)` state
    And the reconciler emits no further `Action::StartAllocation` for that alloc_id until the desired state changes

  Scenario: Job-lifecycle reconciler is pure
    Given a fixed (desired, actual, view, tick) input
    When the lifecycle reconciler is invoked twice with the same inputs
    Then both invocations produce equal `(Vec<Action>, NextView)` outputs

  Scenario: Job-lifecycle reconciler does not call wall-clock directly
    Given the lifecycle reconciler is part of a `core` class crate (or its `reconcile` body is)
    When `cargo xtask dst-lint` scans the source
    Then no banned API (`Instant::now`, `SystemTime::now`, `tokio::time::sleep`) appears in `reconcile`

  # ============================================================
  # Control-plane cgroup isolation — slice + bandwidth
  # ============================================================

  Scenario: Control plane stays responsive while a workload bursts CPU
    Given a job `noisy` is `Running` on the same host as the control plane
    And the control plane is enrolled in `/sys/fs/cgroup/overdrive.slice/control-plane.slice/`
    And the workload is enrolled in `/sys/fs/cgroup/overdrive.slice/workloads.slice/<alloc_id>.scope`
    When the workload bursts to 100% CPU on every available core
    Then `overdrive cluster status` returns within 100 ms on localhost
    And the control plane's responsiveness is not degraded by the workload

  Scenario: Server refuses to start without cgroup v2 delegation
    Given a host where cgroup v2 is not delegated to the running UID
    When the operator runs `overdrive serve`
    Then the server logs an actionable error explaining cgroup delegation
    And exits with a non-zero code
    And no /v1 endpoint is bound

  # ============================================================
  # Stop-and-drain
  # ============================================================

  Scenario: Stopping a Running job drives it to Terminated cleanly
    Given a job `payments` is `Running` with one replica
    When Ana runs `overdrive job stop payments`
    Then within N reconciler ticks the allocation transitions Running → Draining → Terminated
    And the workload's cgroup scope is removed from the host
    And the allocation appears in `Terminated` state in `alloc status` output

  Scenario: Stopping a job that does not exist returns 404
    When Ana runs `overdrive job stop unknown`
    Then the CLI exits with code 1
    And the output names the unknown job ID
    And no allocation state changes

  # ============================================================
  # Empty-state honesty (cross-cutting)
  # ============================================================

  Scenario: alloc_status of a Pending job names the cause
    Given a job whose resource request exceeds the local node's capacity
    When Ana runs `overdrive alloc status --job <id>`
    Then the output is not a blank table
    And the output contains a Pending row with a reason in plain English
    And the reason names the failed constraint (capacity)
