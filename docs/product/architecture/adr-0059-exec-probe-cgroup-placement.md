# ADR-0059 — Exec-probe cgroup placement: `cgroup.procs` write of the spawned PID (Phase 1); reuse `ExecDriver` cgroup-manager machinery; clone3 / `CLONE_INTO_CGROUP` deferred

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, worker-subsystem, cgroup, exec-probe.

**Companion ADRs**: ADR-0054 (ProbeRunner — defines `ExecProber`
trait), ADR-0026 (cgroup v2 direct writes), ADR-0028 (cgroup
preflight refusal), ADR-0030 (exec driver pattern).

## Context

Exec probes (US-03) MUST run inside the workload's cgroup per
`feature-delta.md` C7 — otherwise the probe sees the worker's
network/mount namespace, not the workload's, defeating the operator's
purpose (e.g. checking the workload's DB connection from the
workload's net namespace).

Two mechanisms are available on Linux to place a spawned process
into a target cgroup:

- **(a) `clone3` with `CLONE_INTO_CGROUP` flag** (Linux 5.7+). Atomic:
  the child is born in the target cgroup; no transient
  worker-cgroup membership.
- **(b) `cgroup.procs` write after fork** (any cgroup v2 kernel).
  Two-phase: spawn the child via `fork+exec`, write its PID to
  `<alloc_cgroup>/cgroup.procs`, observe migration. Brief transient
  membership in the parent's cgroup (the worker scope) between
  `fork` and the write.

Open questions resolved here (P1-Q2 per `feature-delta.md`):

- Which mechanism does Phase 1 use?
- How does the sim adapter (`SimExecProber`) shape match?
- What is the kernel-version compatibility surface?

## Decision

### 1. Phase 1 — `cgroup.procs` write of spawned PID (mechanism b)

The production `CgroupExecProber` impl in
`crates/overdrive-worker/src/probe_runner/exec_prober.rs` (new):

```rust
async fn probe(
    &self,
    spec: ExecProbeSpec,
    timeout: Duration,
    alloc_cgroup: &CgroupPath,
) -> ProbeOutcome {
    let mut child = Command::new(&spec.command[0])
        .args(&spec.command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ProbeFailure::ExecSpawnFailed { reason: e.to_string() })?;

    let pid = child.id().ok_or(ProbeFailure::Io {
        reason: "child PID unavailable after spawn".to_string()
    })?;

    // Place into target cgroup. Reuses cgroup_manager::place_pid_in_scope
    // from ExecDriver per ADR-0026.
    cgroup_manager::place_pid_in_scope(alloc_cgroup, pid)
        .map_err(|e| ProbeFailure::Io {
            reason: format!("cgroup placement failed: {e}")
        })?;

    // Wait for exit OR timeout
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) if status.code() == Some(0) => Ok(()),
        Ok(Ok(status)) => Err(ProbeFailure::ExecNonZero {
            exit_code: status.code().unwrap_or(-1)
        }),
        Ok(Err(e)) => Err(ProbeFailure::Io { reason: e.to_string() }),
        Err(_elapsed) => {
            // Timeout — SIGKILL via cgroup.kill (mass-kill, prevents
            // reparented descendants surviving)
            let _ = cgroup_manager::cgroup_kill(alloc_cgroup);
            Err(ProbeFailure::Timeout { after: timeout })
        }
    }
}
```

The mechanism reuses three existing functions from
`crates/overdrive-worker/src/cgroup_manager.rs` (per ADR-0026 /
ADR-0030):

- `place_pid_in_scope(cgroup, pid)` — writes the PID to
  `cgroup.procs`.
- `cgroup_kill(cgroup)` — writes `1` to `cgroup.kill` for mass-kill
  of every PID in the scope.

**Why mechanism (b) over (a) for Phase 1**:

| Factor | (a) `clone3 + CLONE_INTO_CGROUP` | (b) `cgroup.procs` write (CHOSEN) |
|---|---|---|
| Atomicity | Strictly atomic; child born in target | Transient parent-cgroup membership (microseconds) |
| Kernel compatibility | Linux ≥ 5.7 | Any cgroup v2 kernel (Phase 1 floor: 5.10 per `testing.md`) |
| Rust ergonomics | Requires raw `clone3` syscall via `libc`; no stable wrapper in `nix` 0.27 | `tokio::process::Command` + cgroup write; standard ergonomics |
| Code reuse with `ExecDriver` | Different spawn path (raw syscall vs `Command`) | Same `Command` + cgroup-manager primitives |
| Testability | `clone3` cannot run in DST sim envelope | Plain `Command` works in sim envelope (per `SimExecProber`) |
| Phase 1 transient-membership risk | N/A | Probe runs as the worker user (uid=1000); the transient cgroup membership inherits the worker's resource limits for microseconds — operationally inert |

Mechanism (a) is **the structurally cleaner long-term shape** but
Phase 1 chooses (b) for code reuse and DST compatibility. Phase 2+
may switch to (a) once `nix::sched::clone3` wraps the flag (open
issue: `nix-rust/nix#2120`), or via a hand-rolled `libc::syscall(SYS_clone3, ...)`
wrapper isolated to one module.

### 2. Sim adapter shape (`SimExecProber`)

Per `.claude/rules/development.md` § "Production code is not shaped
by simulation" the sim adapter MUST honour the same trait surface
without imposing structural concessions on production:

```rust
// crates/overdrive-sim/src/adapters/probers.rs (new)

#[derive(Default)]
pub struct SimExecProber {
    /// Per-(alloc, probe_idx) outcome queue, drained per call.
    outcomes: parking_lot::Mutex<
        BTreeMap<(AllocationId, ProbeIdx), VecDeque<ProbeOutcome>>
    >,
}

#[async_trait]
impl ExecProber for SimExecProber {
    async fn probe(
        &self,
        _spec: ExecProbeSpec,
        _timeout: Duration,
        _alloc_cgroup: &CgroupPath,
    ) -> ProbeOutcome {
        // SimExecProber DOES NOT model cgroup placement; tests that
        // assert cgroup membership use the production CgroupExecProber
        // under a Lima Tier 3 integration test. The sim adapter is
        // for control-flow / reconciler-driven DST scenarios where
        // the cgroup IS abstracted away.
        let key = /* derive from invocation context */;
        self.outcomes.lock()
            .get_mut(&key)
            .and_then(|q| q.pop_front())
            .unwrap_or(Ok(()))
    }
}
```

The sim adapter **does not assert** cgroup membership because Tier
1 DST (`.claude/rules/testing.md`) does not model the kernel
filesystem; the cgroup-membership property belongs to Tier 3
(real-kernel integration). The cgroup-membership AC from US-03
(`#### Scenario: Exec probe runs as a member of the workload
cgroup`) is a Tier 3 test gated on `integration-tests` per
`.claude/rules/testing.md`.

### 3. Timeout cleanup — `cgroup.kill` for mass kill

When the probe times out, the child may have forked descendants
(e.g. a healthcheck script that runs `curl &`). SIGKILL on the
direct child does not reap the descendants; they linger in the
workload's cgroup as orphans.

The cleanup mechanism is `cgroup.kill` (Linux 5.14+; Phase 1 floor
is 5.10 per `testing.md`). For kernels 5.10–5.13, the cleanup
fallback writes PIDs from `cgroup.procs` via a `kill(-PID, SIGKILL)`
loop (slower; race-prone). Per `testing.md` § Tier 3 kernel matrix
the floor is 5.10; the fallback is per-probe overhead, ~ ms.

Phase 1 ship: **prefer `cgroup.kill`; fall back to PID-loop on
ENOENT**. The cgroup-manager already implements this fallback per
ADR-0026; the exec prober reuses it.

### 4. Resource attribution

The exec probe runs INSIDE the workload's cgroup; therefore its CPU
+ memory consumption count against the workload's resource limits
(per ADR-0026 `cpu.weight` + `memory.max`). This is the operator-
intended semantics: a runaway healthcheck script (`/bin/yes > /dev/null`)
hits the workload's CPU cap, not the worker's.

**Edge case**: if the workload's `memory.max` is hit by the probe
combined with the workload, the kernel OOM-killer may select the
workload process (PID 1 in the scope). This is operator-visible as
the workload alloc transitioning to `Failed` with OOM signal. The
probe is "the cause" but the workload is "the victim" — a
deliberate trade-off that matches K8s behaviour (probes inside the
container's cgroup).

### 5. Earned Trust — `CgroupExecProber::probe()` self-test

Per ADR-0054 §7, the `ProbeRunner::probe()` startup self-test
exercises `TcpProber` only. The `CgroupExecProber` carries its own
Tier-2 self-test:

- Test: spawn `/bin/true` into a tempdir-cgroup; assert exit 0,
  cgroup membership readable via `/proc/<pid>/cgroup` during the
  brief lifetime.
- Test: spawn `/bin/sleep 999` with timeout 100ms; assert
  `ProbeFailure::Timeout`, assert cgroup is empty after cleanup.

These run in `crates/overdrive-worker/tests/integration/exec_prober.rs`
gated on `integration-tests` per `.claude/rules/testing.md`. They
are Tier 3 (real kernel + cgroup) because the production
mechanism cannot run in DST sim envelope.

## Considered alternatives

### Alternative A — `clone3 + CLONE_INTO_CGROUP` (Phase 1)

Atomic placement; no transient parent-cgroup membership. Rejected
per §1 table: requires raw syscall wiring; `nix` 0.27 does not wrap
the flag; sim adapter shape diverges from production; code reuse
with `ExecDriver` is lost. Deferred to Phase 2+ when `nix` ships
the wrapper.

### Alternative B — Don't place exec probe in cgroup; run in worker namespace

The probe runs as the worker process child, in the worker's
network + mount namespace. Rejected: defeats the operator's
purpose (US-03 problem statement). Probes that need the workload's
network namespace (DB connectivity check) would not work.

### Alternative C — Use the workload's PID 1 namespace via `nsenter`

`nsenter -t <workload-pid> -n -m <command>`. Rejected: requires
CAP_SYS_ADMIN + finding the workload's PID 1 via `cgroup.procs`
read (race-prone if multiple PIDs in scope); cgroup-based placement
is simpler and matches the resource-attribution semantics.

### Alternative D — Run probe as a sidecar tokio task within the workload process

Inject probe logic into the workload via a sidecar lib. Rejected:
violates the operator's "domain-specific exec script in any
language" goal (US-03); requires workload cooperation.

## Consequences

### Positive

- **Probe runs in workload's namespaces** (per US-03 AC); operator's
  domain-specific health logic works.
- **Code reuse with `ExecDriver`**: same cgroup-manager primitives;
  bounded new code.
- **DST-friendly via sim adapter** that abstracts the cgroup; Tier
  3 covers the real-kernel cgroup placement.
- **Phase 2+ migration to `clone3` is non-breaking** — same trait
  surface; same operator-facing semantics; difference is internal.

### Negative

- **Microseconds-of transient worker-cgroup membership** for every
  exec probe spawn. Operationally inert (worker cgroup has higher
  limits than workload); does NOT survive into observable
  consequences.
- **Reuses `cgroup.procs` write path**: any bug in
  `place_pid_in_scope` affects both ExecDriver and ExecProber.
  Mitigation: the function is heavily tested per ADR-0026.

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Functional correctness | Probe sees workload's namespaces (per US-03 AC) |
| Reliability — fault tolerance | `cgroup.kill` for timeout cleanup; no orphaned descendants |
| Maintainability — modifiability | Phase 2+ `clone3` migration is non-breaking trait-internal swap |
| Performance — time behavior | Cgroup write is microseconds; well under probe interval |

## Cross-references

- ADR-0026 — cgroup v2 direct writes; place_pid_in_scope, cgroup_kill
- ADR-0028 — cgroup preflight refusal; same delegation requirements
- ADR-0030 — exec driver pattern; ExecProber reuses spawn-then-place
- ADR-0054 — ProbeRunner; defines ExecProber trait contract
- `feature-delta.md` C7, P1-Q2
- `.claude/rules/development.md` § "Production code is not shaped
  by simulation"
- `.claude/rules/testing.md` § Tier 3 kernel matrix (floor 5.10)

## Changelog

- 2026-05-24 — Initial accepted version. Resolves P1-Q2.
