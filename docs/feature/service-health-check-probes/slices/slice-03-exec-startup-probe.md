# Slice 03 — Explicit Exec startup probe (in-cgroup)

**Stories:** US-03
**Priority:** P2
**KPI:** K1
**Dependencies:** Slice 01

## Outcome the operator can verify

Ana declares `[[health_check.startup]] type = "exec", command = ["/usr/local/bin/healthcheck.sh"]`. The probe runs as a member of the workload's cgroup (`/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-payments-0.scope`). Exit 0 = Pass.

## Adds onto Slice 01

| Component | Change |
|---|---|
| TOML parser | Accept `type = "exec", command: [String], [timeout_seconds]` |
| `ProbeRunner` | New `ExecProbe` dispatcher; cgroup placement via clone3 OR cgroup.procs write |
| `ParseError::ExecProbeMissingCommand { probe_idx }` | New variant for empty command array |
| Worker test fixture | Asserts /proc/<pid>/cgroup of probe process matches alloc's scope |

## Acceptance test additions (Linux integration only)

- Exec probe `/bin/true` → Pass
- Exec probe `/bin/false` → Fail with `last_fail_reason: "exit 1"`
- Exec probe `/usr/local/bin/nonexistent` → Fail with `last_fail_reason: "exec: command not found"`
- Exec probe `/bin/sleep 10` with `timeout_seconds = 2` → Fail with `last_fail_reason: "timeout after 2s"` AND SIGKILL delivered at 2s
- Cgroup membership assertion: probe PID's /proc/<pid>/cgroup names `alloc-<id>.scope`, NOT the worker's scope

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests -E 'test(exec_probe_cgroup_membership)'` passes.

## Out of scope

Remote exec, multi-namespace exec, exec with custom env vars (operator can wrap in shell script for now).
