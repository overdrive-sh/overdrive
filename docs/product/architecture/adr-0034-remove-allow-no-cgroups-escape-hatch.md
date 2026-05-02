# ADR-0034 — Remove `--allow-no-cgroups` escape hatch; canonical dev path is `cargo xtask lima run --`

## Status

Accepted. 2026-05-02. Decision-makers: Morgan (proposing), user
ratification 2026-05-02. Supersedes the escape-hatch portion of
ADR-0028 (the hard-refusal pre-flight itself stays). Tags: phase-1,
first-workload, application-arch, supersession.

## Context

ADR-0028 paired a hard-refusal cgroup v2 delegation pre-flight with
an explicit `--allow-no-cgroups` CLI flag as a dev escape hatch. The
hard-refusal half of that decision has held up in practice. The
escape-hatch half has not, for two independent reasons.

### Reason 1 — the escape hatch is structurally broken; it leaks workloads

Code review of the `StopAllocation` action path in
`crates/overdrive-control-plane/src/action_shim.rs:415-424` found that
when `--allow-no-cgroups` is set, the stop path becomes a silent
no-op that returns `Ok(())` while the workload keeps running:

- `StopAllocation` constructs `AllocationHandle { pid: None, ... }`
  and passes it to `driver.stop`.
- In `crates/overdrive-worker/src/driver.rs`, the process-kill
  branch (`handle.pid.is_some() → SIGTERM, then SIGKILL pgrp`) does
  nothing when `pid` is `None` — the action shim's handle never
  carries a PID, so the kill is unconditionally skipped.
- The cgroup-kill branch (`echo 1 > <scope>/cgroup.kill`) is gated
  on `!self.allow_no_cgroups`, so it is also skipped under the
  flag.
- After waiting out the 5-second grace window, `stop` returns
  `Ok(())`. The reconciler writes `state: Terminated` into the
  ObservationStore. The OS process is alive.

The next reconciler tick reads `state: Terminated` against a still-
running PID and produces a desired-vs-actual mismatch the loop
cannot recover from without operator intervention. The dev
ergonomics affordance is itself a correctness bug — exactly the
class of "structural backstop erodes when you bypass it" that
ADR-0028's Alternative A was rejected to prevent. The flag's
existence reproduces the failure mode it was designed to authorise.

### Reason 2 — the canonical dev path is now `cargo xtask lima run --`

`.claude/rules/testing.md` § "Running integration tests locally on
macOS — Lima VM" now documents `cargo xtask lima run --` as the
canonical inner-loop path. The wrapper runs the test process as
root inside the bundled Lima VM (Ubuntu 24.04, kernel 6.8, full
cgroup v2 delegation), matching the permission surface CI's LVH
harness uses. The dev-ergonomics objection ADR-0028 § Alternative A
documented — "developers running on Linux dev VMs without
delegation" — is absorbed by the wrapper, not by the flag. The
flag is now the *secondary* shape; the primary shape requires no
configuration the operator does not already have.

With the canonical path established, the escape hatch is dead
weight. It is also a foot-gun (ADR-0028 § Negative explicitly
identified this risk: "a production deployment accidentally
launched with `--allow-no-cgroups` runs without isolation"). The
mitigation ADR-0028 relied on (loud banner + verbose flag name +
bundled systemd unit does not include the flag) does not address
the structural leak above — the leak fires regardless of
operator awareness.

## Decision

### 1. Remove the `--allow-no-cgroups` flag entirely

`overdrive serve` no longer accepts `--allow-no-cgroups`. The clap
subcommand definition drops the flag; `ServerConfig::allow_no_cgroups`
is removed; every `if self.allow_no_cgroups { ... skip ... }`
branch in the cgroup pre-flight, ProcessDriver/ExecDriver scope
creation, and ProcessDriver/ExecDriver stop path is removed.

The pre-flight check from ADR-0028 stays. The hard-refusal
disposition stays. The actionable error message from ADR-0028 stays
— with the flag's bullet removed from the "Try one of:" list. The
remaining remediation paths after the deletion are:

```
Try one of:

  1. Run via the bundled systemd unit (production):
       systemctl --user start overdrive

  2. Grant delegation manually (one-time):
       sudo systemctl set-property user-1000.slice Delegate=yes
       systemctl --user daemon-reload

  3. Run as root (development only — no isolation guarantees):
       sudo overdrive serve

  4. On macOS / Windows / non-delegated Linux dev box, use the
     bundled Lima VM (canonical inner-loop path):
       cargo xtask lima run -- overdrive serve

Documentation: https://docs.overdrive.sh/operations/cgroup-delegation
```

The Lima path replaces the flag in the operator-facing remediation
list. It is the same wrapper documented in `.claude/rules/testing.md`
for tests; reusing it for `serve` keeps one canonical dev affordance.

### 2. Alternative A from ADR-0028 — "warn-and-continue" — is still rejected

This ADR is a structural reversal of the *escape hatch*, not a
reopening of the disposition question. The §4 architectural
commitment ("control plane runs in dedicated cgroups with kernel-
enforced resource reservations") still requires the hard-refusal
default. Warn-and-continue still produces a working server with
no isolation on misconfigured hosts, and that disposition is still
unacceptable for the same reasons ADR-0028 documented.

### 3. Migration — single-cut, no deprecation

Per the project's greenfield single-cut convention, every flag
reference and every `allow_no_cgroups`-gated code path is removed
in one PR. No deprecation period. No feature-flagged old path. No
rename-marker stub. The file-level surface affected:

- `crates/overdrive-cli/src/cli.rs`, `crates/overdrive-cli/src/main.rs`
  — flag definition removed from the `serve` subcommand.
- `crates/overdrive-control-plane/src/lib.rs` (lines 222, 252, 260,
  283, 427, 436, 474 per pre-implementation review) —
  `ServerConfig::allow_no_cgroups` field and every read of it.
- `crates/overdrive-worker/src/driver.rs` (lines 155, 184, 205,
  235-236, 321, 451, 507 per pre-implementation review) —
  `ExecDriver::allow_no_cgroups` field, builder method, two skip
  branches in scope creation and stop.
- `crates/overdrive-control-plane/src/cgroup_preflight.rs` — the
  pre-flight stays, but the "if `allow_no_cgroups` is set, skip
  the check" wrapper goes away. Pre-flight runs unconditionally
  on Linux; macOS / Windows builds skip it via
  `#[cfg(target_os = "linux")]` per ADR-0028 (this is unchanged).
- The `--allow-no-cgroups`-gated tests under
  `crates/overdrive-worker/tests/integration/` and
  `crates/overdrive-control-plane/tests/integration/` are removed
  in the same PR per the project deletion-discipline rule (delete
  production code AND its tests in the same commit).

`xtask/src/main.rs` — the `cargo xtask lima run` wrapper — is the
replacement path. This ADR does not modify the wrapper; it
elevates it from "secondary inner-loop shape" to "the dev-time
substitute for the deleted flag."

The crafter handles the code/test deletions in the next phase per
the wave separation; this ADR's job is to record the architectural
decision.

## Alternatives considered

### Alternative A — Keep the flag but fix the leak

Patch the StopAllocation path so that `AllocationHandle { pid }`
carries the actual PID even when cgroups are disabled, and use
`pid → SIGTERM/SIGKILL` directly when the cgroup-kill branch is
unavailable. The flag stays; the leak goes.

**Rejected.** Two reasons. (a) The leak is the *visible* defect
discovered in code review; we have no reason to believe it is the
only one. The flag introduces a forked execution mode for every
cgroup-touching code path — pre-flight, scope creation, scope
deletion, limit writes, kill, status reads — and every fork is a
place a future change can drop the wrong branch. (b) Even if the
leak were fully patched, reason 2 still applies: the canonical dev
path is now Lima, the flag is redundant, and a redundant production
foot-gun is not earned by patching one of its known failure modes.

### Alternative B — Replace the flag with `OVERDRIVE_ALLOW_NO_CGROUPS=1` env var

Same effect; configured via env instead of CLI flag. ADR-0028 §
Alternative D considered (and rejected) the env-var-only shape;
this alternative would adopt the env shape *now* in place of the
flag, on the theory that env is at least less likely to be typed
accidentally on a production server.

**Rejected.** The objection is not the surface (flag vs env vs
config file) — the objection is that any escape hatch carries the
structural-leak risk above and is now redundant with Lima. Moving
from CLI flag to env var trades one foot-gun shape for another.
The clean fix is the deletion.

### Alternative C — Keep the flag, but only honour it under `OVERDRIVE_DEV=1`

Two-key requirement: the flag is recognised only when the env
var is also set; otherwise the flag is ignored and the pre-flight
runs normally. The intent: prevent accidental production use.

**Rejected.** Adds complexity to defend a code path the project
no longer needs. The dev surface is Lima; production has no
legitimate use for the flag; a two-key gate is a workaround for
"why is this flag still here at all."

## Consequences

### Positive

- **The §4 isolation commitment is structurally enforced, not
  optionally enforced.** With no escape hatch, no deployment shape
  can run without cgroup isolation. The architectural claim and
  the running surface match.
- **The leak in StopAllocation is removed by construction.** The
  forked execution mode — `if allow_no_cgroups → skip kill,
  return Ok(())` — does not exist after this ADR. There is one
  stop path; it kills the workload.
- **One canonical dev affordance.** `cargo xtask lima run --` is
  the dev path for tests AND for `serve`. Operators learn one
  wrapper; the project documents one path; CI uses the same
  permission surface.
- **Smaller cgroup module surface.** Removing the conditional
  branches simplifies `cgroup_preflight.rs` and `driver.rs`. The
  pre-flight runs or the binary refuses to start; ExecDriver
  creates scopes or returns `DriverError`. No tri-state.
- **Reduced foot-gun surface.** ADR-0028 § Negative explicitly
  acknowledged the foot-gun risk; deletion eliminates it.

### Negative

- **Linux dev boxes without cgroup v2 delegation lose the inline
  workaround.** Operators in this position must run via Lima, run
  as root, or grant delegation. The canonical Lima path is not
  meaningfully harder than typing `--allow-no-cgroups`, but it
  does require Lima to be installed once. Acceptable cost; Lima
  is already the documented inner-loop path for tests.
- **Existing-deployment migration cost.** This is a greenfield
  project; there are no existing deployments using the flag. Any
  in-flight feature branch that types the flag will fail to build
  after the deletion. The crafter PR removes every reference in
  the same commit per single-cut discipline.

### Quality-attribute impact

- **Reliability — fault tolerance**: positive. The
  `state: Terminated`-while-process-alive failure mode is
  eliminated. The reconciler can no longer enter a desired-vs-
  actual stalemate via the flag.
- **Maintainability — analyzability**: positive. One execution
  mode in the cgroup-touching paths instead of two.
- **Maintainability — modifiability**: positive. Future cgroup-
  touching code does not need to remember the `allow_no_cgroups`
  branch.
- **Security — confidentiality / integrity**: positive. The
  isolation-by-default property is now structurally guaranteed,
  not defaulted-with-bypass.
- **Usability — operability**: neutral. The flag's affordance is
  replaced by the Lima wrapper, which is documented in the same
  error message and is one command longer to type.

### Migration

Single-cut, no deprecation, all references removed in one PR per
the greenfield convention. The crafter PR:

1. Deletes the `--allow-no-cgroups` clap definition and
   `ServerConfig::allow_no_cgroups` field.
2. Deletes every `if self.allow_no_cgroups { ... }` branch in
   `cgroup_preflight.rs` and `driver.rs`.
3. Deletes `ExecDriver`'s builder method for the field.
4. Deletes the `--allow-no-cgroups`-gated tests entirely (do not
   salvage them by repurposing — the tests defended a code path
   that no longer exists).
5. Updates the pre-flight error message's "Try one of:" list to
   replace the flag bullet with the Lima bullet.

The crafter does not modify this ADR. ADR-0028's Status line is
updated to "Superseded by ADR-0034" in the same PR (ADRs are
immutable except for status transitions).

## Compliance

- **Whitepaper §4** (workload isolation on co-located nodes):
  this ADR strengthens the §4 commitment by removing the bypass.
- **ADR-0028** (the ADR this supersedes): the hard-refusal pre-
  flight stays; ADR-0028 § Alternative A rejection still binds;
  only the escape-hatch portion is reversed.
- **`development.md` § Deletion discipline**: deletion of unused
  production code AND its tests in the same commit. No gating,
  no salvaging tests by rewriting them, no deprecation comment.
- **`development.md` § Single-cut migrations in greenfield**: no
  deprecation period, no grace window, no feature-flagged old
  path; old code and new code do not coexist in the codebase.
- **`testing.md` § "Running integration tests locally on macOS —
  Lima VM"**: this ADR formalises the Lima wrapper's scope to
  also cover `overdrive serve`, not just test invocations. The
  testing.md amendment in this PR removes the `--allow-no-cgroups`
  bullet from the "Two acceptable shapes" block.

## References

- ADR-0028 (superseded by this ADR's escape-hatch portion) —
  cgroup v2 delegation pre-flight: hard refusal + explicit
  `--allow-no-cgroups` dev escape hatch.
- ADR-0026 — cgroup v2 direct writes (the runtime ops the
  pre-flight protects; unchanged).
- Whitepaper §4 — Workload isolation on co-located nodes.
- `.claude/rules/testing.md` § "Running integration tests locally
  on macOS — Lima VM" — the canonical Lima wrapper.
- `xtask/src/main.rs` — `cargo xtask lima run` implementation
  (referenced; unchanged by this ADR).
- `docs/feature/phase-1-first-workload/design/wave-decisions.md`
  — D8 ratification entry; this ADR is the reversal note attached
  to that entry.
