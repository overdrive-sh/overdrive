# ADR-0028 — cgroup v2 delegation pre-flight: hard refusal on missing delegation; explicit `--allow-no-cgroups` dev escape hatch

## Status

Superseded by ADR-0034. 2026-05-02. The hard-refusal pre-flight
disposition recorded here remains in force; the
`--allow-no-cgroups` escape hatch is removed by ADR-0034 (escape
hatch was structurally broken — it produced
`state: Terminated`-while-process-alive in the StopAllocation path
— and rendered redundant by the canonical
`cargo xtask lima run --` dev path now documented in
`.claude/rules/testing.md`). Read this ADR alongside ADR-0034 for
the current disposition.

Original status: Accepted. 2026-04-27. Decision-makers: Morgan
(proposing), user ratification 2026-04-27. Tags: phase-1,
first-workload, application-arch.

## Context

US-04 (Control-plane cgroup isolation) ships a cgroup v2
delegation pre-flight check at `overdrive serve` startup. The
check verifies the kernel exposes cgroup v2 (unified hierarchy
mounted at `/sys/fs/cgroup/`) AND the running UID has the
`cpu` and `memory` controllers delegated via
`cgroup.subtree_control`.

When the pre-flight fails, two dispositions are possible:

- **(a)** Hard refusal — log an actionable error, exit
  non-zero, do not bind the HTTPS listener.
- **(b)** Warn-and-continue — log a warning, run with no
  cgroup isolation, bind the listener.

The DISCUSS wave Risk 4 flagged a developer-experience concern:
"Pre-flight cgroup delegation check refuses to start on
developer machines without delegated cgroup v2 (e.g.
unconfigured Linux dev box)." User decision (D8): hard refusal,
with an explicit `--allow-no-cgroups` dev escape hatch.

This ADR records the decision and specifies the escape hatch.

## Decision

### 1. Default disposition: hard refusal on missing cgroup delegation

When `overdrive serve` starts (without `--allow-no-cgroups`):

```
1. Check kernel exposes cgroup v2:
     - /sys/fs/cgroup/cgroup.controllers exists
     - /proc/filesystems lists "cgroup2"
2. Check delegation to running UID:
     - The slice the process is in has `cpu` and `memory` listed
       in its `cgroup.subtree_control`
     - Or the running UID is root (no delegation needed)
3. If any check fails, log actionable error, exit code 1,
   do NOT bind the listener.
```

The error message names the failed check, the actionable fix,
and the dev escape hatch:

```
Error: cgroup v2 delegation required.

  Overdrive serve needs the `cpu` and `memory` controllers delegated
  to UID 1000.

  Detected: cgroup v2 IS available, BUT the cpu controller is not in
  the subtree_control of /sys/fs/cgroup/user.slice/user-1000.slice/.

  Try one of:

    1. Run via the bundled systemd unit (production):
         systemctl --user start overdrive

    2. Grant delegation manually (one-time):
         sudo systemctl set-property user-1000.slice Delegate=yes
         systemctl --user daemon-reload

    3. Run as root (development only — no isolation guarantees):
         sudo overdrive serve

    4. Run without cgroup isolation (development only — workloads
       are unbounded; control plane is not protected):
         overdrive serve --allow-no-cgroups

  Documentation: https://docs.overdrive.sh/operations/cgroup-delegation
```

The error answers "what / why / how to fix" per the
`nw-ux-tui-patterns` shape (US-04 AC).

### 2. Pre-flight is part of the `overdrive serve` boot path

The check runs at the start of `run_server` — before
`run_server_with_obs_and_driver` is invoked, before the
trust-triple is written, before the listener is bound. A pre-
flight failure produces no on-disk side effects:

```
1. Run pre-flight check                    ← NEW (this ADR)
2. Mint ephemeral CA + leaf certs          (ADR-0010)
3. Open LocalIntentStore                   (existing)
4. Resolve NodeId, Region from config      (ADR-0025)
5. Write node_health row                   (ADR-0025)
6. Construct ReconcilerRuntime + Driver    (existing + ADR-0022)
7. Build AppState, Router                  (existing)
8. Bind TCP listener                       (existing)
9. Write trust triple                      (ADR-0010)
10. Spawn axum_server task                 (existing)
```

A pre-flight failure aborts the boot at step 1; nothing
downstream runs. The trust triple is not minted; the redb file
is not opened; no fork happens. Operators who rerun `serve`
after fixing delegation see clean state.

### 3. Dev escape hatch: `overdrive serve --allow-no-cgroups`

The `--allow-no-cgroups` CLI flag explicitly bypasses the
pre-flight. When set:

- The pre-flight is skipped.
- `Driver::start` operations skip the cgroup scope creation /
  PID placement / limit writes (the workload runs as a normal
  child process under the running UID, no cgroup scope of its
  own).
- The control plane runs without enrolling itself in
  `overdrive.slice/control-plane.slice/`.
- A startup banner names the disposition:

```
WARNING: --allow-no-cgroups set. Workloads run without cgroup
         isolation; the control plane is not protected from
         workload CPU bursts. Development use only — production
         deployments require cgroup v2 delegation.
```

The flag's name is deliberately verbose — `--allow-no-cgroups`
communicates the trade-off at the call site. Operators who type
the flag know what they are getting; operators who do not type
the flag get the safe default.

The flag is scoped to `overdrive serve` only; it does not
appear on `overdrive job submit` / `overdrive cluster status`
/ etc. because those commands do not interact with cgroups
directly.

### 4. Pre-flight detection details

The pre-flight checks, in order:

```
Step 1: Kernel exposes cgroup v2.
  - Read /proc/filesystems; require a line containing "cgroup2".
  - If absent: error "cgroup v2 not available on this kernel"
    naming the kernel version (uname -r) and pointing to the
    minimum-supported kernel doc.

Step 2: cgroup v2 is mounted.
  - stat /sys/fs/cgroup/cgroup.controllers; require regular file.
  - If absent: error "cgroup v2 not mounted; expected
    /sys/fs/cgroup/cgroup.controllers" + actionable fix.

Step 3: Delegation OR running as root.
  - If geteuid() == 0 (root): skip step 4; root has implicit
    access to all controllers.
  - Else: continue to step 4.

Step 4: Required controllers delegated.
  - Read /proc/self/cgroup; extract the cgroup path the process
    is in.
  - Read <that_path>/cgroup.subtree_control (or the parent's
    if the file is empty); require both "cpu" and "memory" to
    appear.
  - If absent: error "controllers cpu/memory not delegated to
    UID <uid>" naming the missing controller and the systemd
    fix.
```

The check is read-only — pre-flight does not attempt to write
any cgroup file. The step-by-step structure means the error
message can name the *specific* failed check, not just "cgroup
not available."

### 5. Failure modes covered by tests

The US-04 AC requires the following pre-flight failure modes
to produce actionable errors:

- cgroup v2 not available (cgroup v1 host).
- cgroup v2 mounted but `subtree_control` lacks `cpu`.
- cgroup v2 mounted but `subtree_control` lacks `memory`.
- cgroup v2 mounted but neither controller is delegated.
- cgroup v2 not mounted at all.

Each is verified by an `integration-tests`-gated Linux test
that constructs the failure state (test container with
manipulated cgroup mount) and asserts the error message names
the cause. The `--allow-no-cgroups` path is verified by an
analogous test that runs `overdrive serve` with the flag set,
asserts the listener binds, and asserts a workload starts (no
cgroup scope created).

## Alternatives considered

### Alternative A — Warn-and-continue by default

Always log a warning and proceed; never refuse to start. The
operator is responsible for noticing the warning.

**Rejected on user pre-decision (D8).** Phase 1 first-workload
is the moment the §4 "control plane runs in dedicated cgroups
with kernel-enforced resource reservations" claim becomes
testable. Warn-and-continue means an operator running on a
mis-configured host gets a working server with *no isolation*
— and discovers the absence only when a runaway workload
takes the control plane down. The whole point of the §4 claim
is the structural backstop; warn-and-continue defaults erode
the backstop. Hard refusal at boot is the disposition that
respects the architectural commitment.

The escape hatch (`--allow-no-cgroups`) absorbs the dev-
ergonomics objection: developers who *want* to skip the check
can do so explicitly. The flag-presence is discoverable in the
error message; no operator has to figure out the bypass on
their own.

### Alternative B — Auto-detect environment, refuse only in production

Use environment heuristics (running under systemd? interactive
TTY? `RUST_LOG=debug` set?) to choose between hard refusal
(production-shaped) and warn-and-continue (dev-shaped).

**Rejected.** Heuristic-based safety is fragile. The signals
the heuristic would key on (TTY, env vars, parent process)
are not reliable indicators of "production vs dev" — every
operator has their own combination, and the decision boundary
will reliably get them wrong some fraction of the time. An
explicit flag is unambiguous.

### Alternative C — Refuse ONLY when no cgroup v2 at all; warn on partial delegation

Hard refusal on cgroup v2 absent; warn-and-continue when v2 is
mounted but delegation is missing.

**Rejected.** Partial delegation is the most common
mis-configuration — and exactly the case where warn-and-
continue produces an unsafe runtime. The control plane runs
*in* the parent slice; without delegation, the workload scopes
created by ProcessDriver fail to create or fail their PID
write. ProcessDriver's per-call `mkdir` failures would surface
as `DriverError::SpawnFailed` for every job; the operator
would see "all my jobs fail to spawn" with no context that
the root cause is a one-time delegation gap. Hard-refusal on
delegation surfaces the root cause once at boot, not N times
per job.

### Alternative D — Pre-flight check, but skip with an environment variable

`OVERDRIVE_ALLOW_NO_CGROUPS=1 overdrive serve`. Same effect as
the flag but configured via env.

**Rejected.** The flag is more discoverable. An operator
diagnosing a startup error sees the suggested `--allow-no-cgroups`
in the error message; an env var would require them to read
documentation. The flag also composes with shell history
("what flag did I run last time?") in a way env vars do not.

The flag does not preclude future env-var support; if Phase
2+ packaging needs an env-shaped escape hatch (e.g. for
container orchestrators that prefer env over args), that is
an additive extension. Phase 1 ships the flag.

## Consequences

### Positive

- **The §4 isolation claim is honest.** No deployment runs
  with broken cgroup configuration silently — every deployment
  either has working cgroup isolation or has explicitly
  acknowledged its absence via the flag.
- **Boot-time error surfaces root cause once.** Operators
  fixing delegation see one error at startup, not N errors
  per submitted job.
- **Dev workflow is unblocked.** `--allow-no-cgroups` is
  one flag away. Developers running on macOS / Windows do
  not encounter the check (the binary uses `SimDriver` on
  non-Linux targets and the cgroup module is not compiled
  in); developers running on Linux dev VMs without
  delegation use the flag.
- **Production deployments via systemd unit get isolation
  by default.** The bundled `overdrive.service` unit (DEVOPS
  / packaging) enables `Delegate=yes`; operators following
  the documented installation path get cgroup isolation
  without any extra steps.
- **No silent fallback path.** A future change that breaks
  the pre-flight in a subtle way (e.g. a kernel
  upgrade changing the `subtree_control` semantics) fails
  loudly at boot with a parseable error rather than
  degrading silently.

### Negative

- **First-time-on-Linux experience requires one configuration
  step.** Operators on a fresh Linux dev box who neither
  know about the `Delegate=yes` systemd flag nor want to
  run as root must read the error message and either run
  the systemd command or use the `--allow-no-cgroups`
  flag. This is one extra step beyond "install and run."
  Acceptable cost for the safety floor; the error message
  carries enough context to do it without external
  documentation.
- **The escape hatch creates a foot-gun.** A production
  deployment accidentally launched with `--allow-no-cgroups`
  runs without isolation. Mitigation: the startup banner
  is loud; the flag name is verbose; the bundled systemd
  unit does not include the flag. Operators have to type
  it deliberately.
- **Pre-flight code is Linux-only.** The check itself is
  `#[cfg(target_os = "linux")]`. macOS / Windows dev
  builds skip it entirely (the `serve` command on those
  platforms uses `SimDriver` per ADR-0026 and never
  interacts with cgroups). One-platform conditional
  compilation is mechanical.

### Quality-attribute impact

- **Reliability — fault tolerance**: positive. Boot-time
  detection of mis-configuration prevents runtime
  cascading failures.
- **Maintainability — operability**: positive. Single
  error site at boot; clear remediation path.
- **Maintainability — analyzability**: positive. The
  pre-flight is a small standalone module; failure
  modes are enumerable.
- **Security — confidentiality / integrity**: positive.
  Cgroup isolation is the structural backstop for
  workload-vs-control-plane separation; refusing to
  start without it preserves the security property.
- **Performance — time behaviour**: neutral. Pre-flight
  reads at most three small kernel files; sub-millisecond.

### Migration

No existing deployments to migrate. The `--allow-no-cgroups`
flag lands fresh in the `overdrive serve` clap subcommand;
the pre-flight module is new code in
`overdrive-control-plane::cgroup_preflight` (or equivalent).

## Compliance

- **Whitepaper §4** (workload isolation on co-located nodes):
  the pre-flight check operationalises the §4 commitment.
- **ADR-0026** (cgroup v2 direct writes): the runtime
  cgroup operations assume v2; the pre-flight check enforces
  that assumption. The two ADRs are paired.
- **`development.md` § Errors**: pre-flight failure shapes
  use the existing `ControlPlaneError` envelope (specifically
  `ControlPlaneError::Internal` with a structured
  `CgroupPreflightError` source via `#[from]`).
- **`testing.md` § Tests gating**: pre-flight tests live
  under `integration-tests` since they manipulate real
  cgroup state. Default-lane tests on Linux run a unit test
  that mocks the filesystem; the gated tests verify against
  real kernel state.
- **`nw-ux-tui-patterns`**: the error message answers "what /
  why / how to fix" per the pattern.

## References

- ADR-0010 — TLS bootstrap; the boot sequence the pre-flight
  prepends.
- ADR-0022 — `AppState::driver` extension; ProcessDriver
  construction depends on the pre-flight passing.
- ADR-0025 — Single-node startup wiring; the boot sequence
  this ADR amends.
- ADR-0026 — cgroup v2 direct writes; the runtime operations
  the pre-flight protects.
- Whitepaper §4 — Workload isolation on co-located nodes.
- Linux kernel `Documentation/admin-guide/cgroup-v2.rst` §
  "Delegation" — the systemd `Delegate=yes` mechanism.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Priority Two item 8 enumerates the decision; D8 user
  ratification.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md`
  — US-04 acceptance criteria, including the actionable
  error-message shape.
