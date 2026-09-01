# Platform Review — DISTILL Expectation Boundary

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Target commit | `9558af759e049564b7149bea6459961979899e96` |
| Parent commit | `c33f0396edf86c1db888a4c36b751911258c48fb` |
| Review type | Fresh isolated PLATFORM review |
| Reviewer | `nw-platform-architect-reviewer` |
| Review date | 2026-08-29 |
| Iteration | 1 |
| Verdict | **NEEDS_REVISION** |

## Verdict summary

The target commit gets the evidence-layer boundary largely right: active
verification now has one E07 built-product journey, the removed E08/E09
contracts remain Rust-owned, the example sources and workload specs are checked
in, and the roadmap is explicitly non-executable pending regeneration.

The journey is not operationally feasible or evidence-safe yet. The callee's
implicit production TCP startup probe is unreachable from the host-side probe
runner, so the sole Service deploy reaches `Failed` before the documented call
journey can complete. The example also omits the preparation required by the
canonical metal appliance: real kernel/rootfs inputs, static guest installation,
same-filesystem reflink staging, a traversable explicit data directory, and
production KEK delivery. Independently, the executable pending runner exits
zero without driving the product, and the shared harness therefore persists a
manifest claiming execution (and incorrectly labels it Lima execution).

Approval requires zero blocker, high, or medium findings. This iteration has
one blocker, two high findings, and one medium finding.

## Finding counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| High | 2 |
| Medium | 1 |
| Low | 0 |

## What is correct

- The active expectation catalogue contains exactly one feature expectation,
  E07. E08 and E09 are absent from the active index, expectation tree, DISTILL
  ownership map, roadmap runtime criteria, and final DELIVER gate. Their names
  remain only in historical review artifacts, which are audit history rather
  than live execution gates.
- E07 is scoped to one checked-in `[service]` + `[exec]` callee and one
  `[job]` + `[vm]` caller. The eventual runner is required to use the built
  default-feature binary and the public `serve`, `deploy`, `workload describe`,
  and `job stop` surfaces.
- The caller and callee sources, TOML specs, and example README are checked in.
  Neither the current runner nor the E07 contract generates replacement Rust,
  Cargo manifests, or workload specs inline.
- E07 expressly excludes netlink decoding, normalized nft identity, packet
  capture, exact counters, original-destination handling, TLS/kTLS state,
  generation stability, and private cleanup. It invokes no Rust test binary,
  imports no `overdrive-*` crate, and contains no duplicate product oracle.
- The canonical `cargo xtask metal run --` implementation now acquires
  `/run/lock/overdrive-metal-shared.lock` before remote-tree mutation, retains
  the lease through the run, and invokes the fail-closed native x86_64/KVM
  preflight. DISTILL correctly says that Lima and nested virtualization cannot
  supply runtime evidence.
- `roadmap.json` has `validation.status = pending` and
  `requires_regeneration = true`. Its final gate names only independently
  approved E07 evidence, so the superseded E08/E09 runtime gates are not live.

## Findings

### B-01 — The callee's inferred startup probe makes the sole journey fail

**Severity:** Blocker

**Evidence**

- `examples/guest-stack-transparent-mtls-intercept/callee.toml:13-15` declares
  a TCP listener but no explicit startup-probe policy.
- `crates/overdrive-core/src/aggregate/workload_spec.rs:1275-1340` infers a
  startup TCP probe to `0.0.0.0:<first-listener-port>` whenever a Service has a
  listener and does not explicitly declare startup probes.
- Production `serve` gives each allocation its own network namespace, while
  the current production probe runner does not enter that namespace. The
  repository documents the resulting behavior at
  `examples/quick-bind-service.toml:39-66`: a serving workload with the same
  inferred probe reaches `Failed { StartupProbeFailed }` after the 60-second
  startup deadline.
- `crates/overdrive-cli/src/commands/deploy.rs:613-687` consumes a Service
  deployment stream until `Stable`, `Failed`, or `Stopped`, with `Failed`
  returning exit 1.
- The example's operator sequence deploys the callee before the caller
  (`README.md:19-24`). It therefore blocks on and then fails the first deploy;
  detaching it would only race the same terminal failure.

**Platform impact**

The one stakeholder journey cannot reach its promised reply outcome through
the production composition root. No runner, evidence capture, timeout tuning,
or host qualification can make an unreachable health probe a valid pass.

**Required remediation**

Give the callee an explicit production-reachable startup policy. For this
narrow call-success journey, the minimal supported choice is the explicit
empty startup list:

```toml
[health_check]
startup = []
```

That preserves first-Running Service stability without pretending the broken
host-namespace TCP probe is a workload-health check. If an explicit exec probe
is chosen instead, demonstrate that it actually runs in the callee's workload
context. The README and eventual runner must then use the same foreground or
detached deployment lane and prove the callee remains available until the
reply-dependent caller result is observed.

### H-01 — The “operator-runnable” example omits appliance-critical preparation

**Severity:** High

**Evidence**

- `callee.toml:6` requires a host executable at
  `/var/lib/overdrive/examples/guest-stack-transparent-mtls-intercept/callee`.
  `caller.toml:5-8` requires a statically runnable guest executable plus a
  kernel and private rootfs at two more fixed paths. None of those materialized
  files exists in the checkout.
- The example README says only “After compiling” and “installing”
  (`README.md:9-17`). It gives no exact compile commands or targets, no source
  for the base kernel/rootfs, no clone/mount/install/unmount procedure, no
  executable/static-link validation, and no checked-in preparation entry point.
- The canonical metal path preflights a selected readable kernel and rootfs
  (`infra/metal/native-preflight.sh:53-58`). The established appliance fixture
  uses `/srv/vm/overdrive-testing/{kernel,rootfs.ext4}`; the example instead
  assumes unrelated `/var/lib/overdrive/...` artifacts without establishing
  how the metal runner creates or selects them.
- The production VM driver FICLONEs each allocation rootfs into
  `<data-dir>/vm/clone-staging`; that staging directory must share the rootfs
  master's filesystem (`crates/overdrive-worker/src/vm_driver.rs:435-449`).
  Plain `overdrive serve` uses the XDG/HOME default data directory
  (`crates/overdrive-cli/src/main.rs:244-255`), while the example gives no
  guarantee that this directory and its `/var/lib` rootfs are on the same
  reflink-capable filesystem or that the confined VMM identity can traverse it.
- Production `serve` obtains the workload-identity KEK from
  `$CREDENTIALS_DIRECTORY/<kek-id>` and refuses a cold start when none is
  delivered. The example supplies neither that production credential delivery
  nor an explicit data/config directory, bind address, or readiness check.

**Platform impact**

An operator following the checked-in instructions encounters missing artifacts
or a fail-closed serve/VM start before E07 exercises networking. The unspecified
filesystem placement is especially material: moving the files until they
exist does not establish the intra-filesystem reflink and confinement posture
required by the production VM driver.

**Required remediation**

Land one checked-in, operator-invocable preparation entry point shared by the
README and E07 runner. It must:

- accept or discover documented real metal kernel/rootfs inputs and fail when
  they are absent;
- compile the checked-in callee for the host and caller for the guest's
  `x86_64-unknown-linux-musl` target, verifying executable and static linkage;
- reflink/copy a private rootfs, mount it, install the caller at the exact
  checked-in spec path, and unmount/detach it on every exit path;
- place the private rootfs and explicit `serve --data-dir` on the same
  qualified reflink-capable filesystem, with the required ancestor traversal
  posture for the confined VMM identity;
- deliver a per-run KEK through the production `CREDENTIALS_DIRECTORY`
  contract, use an isolated config directory and bind, wait boundedly for
  readiness, and remove all runner-owned credentials/materialization in traps.

Changing the stable paths to a documented appliance staging root such as a
private child of `/srv/vm/overdrive-testing` is acceptable. The entry point may
materialize binaries and a rootfs from the checked-in assets, but it must not
generate replacement source, Cargo manifests, or TOML specs.

### H-02 — The pending runner false-passes and the harness persists false substrate evidence

**Severity:** High

**Evidence**

- `verification/expectations/E07-guest-first-mesh-dial-born-captured/runner.sh`
  is mode `100755`. It checks only that four fixture files exist, prints two
  future-tense `[pending]` lines, and exits 0 (`runner.sh:7-19`). Direct
  execution at the reviewed commit returned 0 without invoking the product.
- `verification/harness/run-expectation.sh:37-39` creates the expectation's
  evidence directory before execution. Lines 70-83 treat any executable runner
  as `EXECUTED=true`, tee its narration to `evidence/run.log`, and retain its
  zero status.
- The harness then writes `executed_in_lima: true` and
  `runner_exit_code: "0"` (`run-expectation.sh:88-103`) and returns success at
  line 107. It has no pending-stub discriminator.
- E07 requires native non-virtualized metal, so the same boolean would remain
  factually wrong even after a real metal runner replaces the stub. Unlike the
  existing E06 workaround, E07 currently requires no independent
  `execution_substrate.txt` record and does not explain the manifest defect.
- DISTILL itself says the pending stub must block rather than treat commands as
  evidence (`distill/test-scenarios.md:194-198`). The executable zero-exit stub
  contradicts that requirement.

**Platform impact**

The normal evidence command can create a commit-pinned, zero-exit manifest and
run log that look executed while containing only narration. That is a direct
false-positive path at the final DELIVER gate. A future genuine run would also
carry an incorrect Lima substrate label unless the harness contract changes.

**Required remediation**

Until the real runner lands, remove the executable bit or make the stub fail
closed with a nonzero pending status; it must not return 0. Update the harness
so it does not create successful execution evidence merely because a runner is
executable. The persisted manifest should distinguish `pending`, `executed`,
and `failed`, record the actual substrate (`native-metal`, `lima`, or other),
and make a zero final command status contingent on a real completed run. A
pending or blocked attempt may retain diagnostic output, but it must never
become a successful evidence manifest.

### M-01 — The deadline and cleanup contract is not bounded enough for shared metal

**Severity:** Medium

**Evidence**

- `caller.rs:15-29` wraps attempts in a nominal 90-second loop, but
  `TcpStream::connect(&address)` has no explicit per-attempt deadline. DNS and
  TCP connection establishment can therefore outlive the loop's deadline.
- The caller discards failure from `set_read_timeout` (`caller.rs:19`) and then
  calls `read_exact`; if the timeout was not installed, that read can block
  indefinitely. No write timeout is configured. This contradicts the README's
  statement that any timeout exits nonzero (`README.md:27-28`).
- The E07 README says the eventual runner “stops the service/server” and proves
  temporary materialization removed (`E07 README.md:35-40`), but its exact
  command list stops only the caller. It names no callee stop invocation,
  outer wall-clock deadline, signal-safe teardown, before/after shared-host
  snapshots, or final residue checks.
- The native lease is intentionally held until cleanup and final probes. An
  unbounded connect/read or incomplete service/VMM cleanup can therefore hold
  the shared metal lane indefinitely or contaminate the next serialized run.

**Platform impact**

The expected-reply witness can hang beyond its narrated deadline, and failure
paths do not yet define how runner-owned processes, guest resources,
credentials, loop devices, mounts, private images, and files are removed
without touching pre-existing shared-host state.

**Required remediation**

Resolve addresses explicitly and use bounded `connect_timeout` attempts; treat
failure to install read/write timeouts as fatal. Add a runner-level wall-clock
deadline around every build, serve, deploy, describe, stop, and cleanup phase.
Install teardown traps before the first materialization, snapshot shared-host
state, stop the caller and callee through verified public commands, terminate
only the serve process started by this run, and remove only the before/after
delta plus the runner's private mount/loop/rootfs/credential/data/config paths.
Keep the canonical lease through final residue probes on success, error, and
signal. These checks prove runner hygiene only; they must not absorb Rust-owned
private lifecycle or nft cleanup assertions into E07.

## Platform boundary assessment

| Requirement | Result | Assessment |
|---|---|---|
| Exactly one built-product expectation | PASS | Only E07 is live for this feature |
| One VM Job calls one Exec Service | PASS structurally | Checked-in TOML/source shape is correct |
| Production composition is runnable | FAIL | Inferred callee probe terminates the Service (B-01) |
| Explicit artifact preparation | FAIL | Kernel, rootfs, binaries, filesystem posture, and KEK setup are unspecified (H-01) |
| Default-feature binary, no test harness/crates | PASS in contract | E07 requires the built default-feature binary and imports no crate |
| No inline source/TOML generation or duplicate oracle | PASS | Checked-in source/specs are the sole fixture definitions; internals stay Rust-owned |
| Native host and global lease | PASS as platform prerequisite | Canonical metal transport/preflight/lease are implemented, but E07 does not invoke them yet |
| Evidence cannot false-pass | FAIL | Executable narration-only runner produces zero-exit “executed” evidence (H-02) |
| Substrate metadata is truthful | FAIL | Harness maps “runner executed” to `executed_in_lima`, including for metal (H-02) |
| Deadlines and shared-host cleanup | FAIL | Caller I/O and runner teardown remain underspecified (M-01) |
| E08/E09 retirement | PASS | No stale active runtime or final-gate obligation remains |
| Roadmap execution authority | PASS | Validation remains explicitly pending and requires regeneration |

## Verification performed

- Inspected the complete target commit and all sixteen changed paths.
- Confirmed the reviewed example, E07, index, DISTILL, and roadmap files match
  target commit `9558af759e049564b7149bea6459961979899e96` despite unrelated
  pre-existing dirty work in the shared worktree.
- Traced the callee's omitted health-check section through startup-probe
  inference, the production namespace limitation documented by the root
  example, and Service deploy terminal behavior.
- Traced the VM rootfs from the checked-in spec to production FICLONE staging,
  default data-directory selection, metal kernel/rootfs qualification, and
  production KEK delivery.
- Audited the harness control flow from executable-runner detection through
  `run.log`, `verification.yaml`, substrate labeling, and final status.
- Searched active verification, DISTILL, roadmap criteria, and the final gate
  for E08/E09. Remaining hits are historical review records only.
- Confirmed E07 runner mode `100755`, both helper sources compile as Rust 2021
  metadata, the runner passes `bash -n`, direct stub execution returns 0, and
  `git show --check 9558af759e049564b7149bea6459961979899e96` is clean.
- Mutation testing and native-metal runtime execution were not performed, as
  required for this targeted DISTILL review.

## Final disposition

**NEEDS_REVISION.** Remediate B-01, H-01, H-02, and M-01, commit the corrections
to a fresh immutable revision, and repeat the independent platform review. The
roadmap must remain non-executable until its existing regeneration and approval
gate is satisfied.

This reviewer created only this native Markdown review artifact. All
pre-existing tracked and untracked workspace changes were preserved.

---

# Iteration 2 — Remediation Re-review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Target commit | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Parent commit | `9558af759e049564b7149bea6459961979899e96` |
| Review type | Same-reviewer PLATFORM remediation re-review |
| Reviewer | `nw-platform-architect-reviewer` |
| Review date | 2026-08-29 |
| Iteration | 2 |
| Verdict | **NEEDS_REVISION** |

## Verdict summary

The remediation closes important parts of every prior concern. The callee now
uses the supported explicit-empty startup policy; a checked-in preparation and
operator entry point materialize static helpers, a private rootfs, explicit
same-filesystem serve state, and production credential delivery; the caller's
DNS/connect/read/write path is finite; and the executable E07 stub now exits 75
under an explicit `native-metal` substrate, which the harness records as
pending and returns nonzero.

The sole operator journey still cannot complete. It submits the Service in the
detached lane and then asks the Job-only `Attempt / State` parser to wait for a
`Stable` allocation state. Service describe renders an `Alloc / State` table,
and allocation states contain `Running`, not `Stable`; consequently this path
returns no state until its 90-second timeout. The new preparation also purges a
stable production KEK description from the ambient session keyring without
establishing ownership or a fresh session, and the cleanup/harness migrations
retain two further medium-severity false-safety edges.

Approval requires zero blocker, high, or medium findings. Iteration 2 has one
blocker, one high finding, and two medium findings.

## Finding counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| High | 1 |
| Medium | 2 |
| Low | 0 |

## Prior-finding disposition

| Iteration-1 finding | Disposition | Evidence |
|---|---|---|
| B-01 — inferred startup probe fails the callee | **PARTIALLY CLOSED; blocker remains** | `callee.toml:17-21` now uses `health_check.startup = []`, so the unreachable inferred probe is gone. The replacement operator observer waits for a state the Service describe surface cannot emit; see B2-01. |
| H-01 — phantom appliance preparation | **PARTIALLY CLOSED; high remains** | `prepare.sh` now owns static compilation, exact spec paths, reflink staging, rootfs installation, data/config/credential directories, and bounded mount/loop cleanup. Ambient session-keyring mutation still makes the claimed per-run KEK unsafe and potentially non-attributable; see H2-01. |
| H-02 — pending runner false-passes and substrate is false | **CLOSED for E07; global harness migration incomplete** | E07's executable stub exits 75, its checked-in substrate is `native-metal`, and an isolated harness execution recorded `execution_status: pending`, `executed_in_lima: false`, and returned nonzero. The modified generic harness still returns zero for no runner, and the active native E06 expectation was not migrated; see M2-02. |
| M-01 — deadlines and cleanup are unbounded | **PARTIALLY CLOSED; medium remains** | The caller now bounds resolution, connection, read, and write; phase commands use finite timeouts and cleanup traps. Run-directory/cgroup deletion is not before/after scoped, public stop failures are suppressed, and outer cleanup can skip a timed-out partial preparation; see M2-01. |

## What is now correct

- Active feature verification has exactly one expectation directory and index
  row: `E07-vm-job-calls-exec-service`. The old E07 slug is gone, and active
  feature/DISTILL/roadmap/example artifacts contain no E08 or E09 mapping.
- The one checked-in example still has exactly one `[job]` + `[vm]` caller and
  one `[service]` + `[exec]` callee. No new product protocol, crate, daemon,
  persistence shape, or observation field is introduced.
- `callee.toml` explicitly declares `startup = []`, which the production parser
  recognizes as the supported opt-out. The known-unreachable host-namespace TCP
  probe is no longer inferred.
- `prepare.sh` is checked in and executable. It validates the fixed spec paths,
  requires real base kernel/rootfs files, compiles both checked-in helpers for
  `x86_64-unknown-linux-musl`, rejects dynamic interpreters, reflinks a private
  rootfs, installs the exact guest command, and verifies same-filesystem and
  uid-4200 traversal constraints.
- The operator and expectation contracts generate no replacement Rust, Cargo
  manifest, or TOML spec. The product is built with default features, and the
  example drives only built `serve`, `deploy`, `workload describe`, and `job
  stop` product commands.
- `caller.rs` now bounds DNS resolution, uses `TcpStream::connect_timeout`,
  treats timeout-install failures as fatal, and applies both write and read
  timeouts under the total 90-second deadline. Its zero exit still depends on
  the byte-exact, byte-distinct reply.
- The canonical operator invocation uses one `cargo xtask metal run --`, so the
  already-implemented global lease and fail-closed native preflight span remote
  sync through command completion. Lima and nested execution remain invalid.
- E07's pending runner is honest: it invokes only `prepare.sh check-source`,
  narrates no execution claim, and exits 75. The modified harness maps that
  exact status to `pending`, records `execution_substrate: native-metal`, sets
  `executed_in_lima: false`, and returns nonzero.
- D7/kernel/wire/counter/generation, boot failure, diagnostic, C4a,
  reclamation, stop/idempotency, sibling, nft/FIB, private cleanup, and replay
  contracts remain Rust-only. The E07 success predicate uses only the public
  reply-dependent Job result.
- The roadmap still has `validation.status = pending` and
  `requires_regeneration = true`; its final DELIVER gate names only independently
  approved E07 evidence. This review does not grant roadmap execution authority.

## Iteration-2 findings

### B2-01 — The operator runner waits for an impossible Service state through a Job-only parser

**Severity:** Blocker

**Prior finding:** B-01

**Evidence**

- `run-example.sh:236-241` recognizes only the Job describe header
  `Attempt ... State` and then extracts the second cell from a numeric attempt
  row.
- Production's Service describe renderer uses the different header
  `Alloc / State / Restarts / Since`
  (`crates/overdrive-cli/src/render.rs:1094-1139`). Its allocation-state labels
  are `Pending`, `Running`, `Draining`, `Suspended`, `Terminated`, or `Failed`
  (`render.rs:666-679`); `Stable` is not an allocation-state label.
- The runner deploys the callee with `--detach` and then calls
  `wait_for_state "$CALLEE_ID" Stable 90 ...`
  (`run-example.sh:287-291`). Because detached deploy observes only acceptance,
  this impossible describe predicate is the sole convergence gate.
- Replaying the checked-in awk function against a real-shape Service render
  (`Replicas (desired/running): 1/1`, `Alloc State ...`, allocation `Running`)
  produced an empty value. The same function correctly returned `Terminated`
  for a Job `Attempt State ...` row.

**Platform impact**

Even with the startup-probe correction, the advertised operator entry point
always times out before deploying the VM caller. The feature's only checked-in
journey is therefore still not runnable, and no E07 implementation can safely
delegate to it.

**Required remediation**

Use a product surface that can actually prove Service convergence. Either:

- run the callee's bounded foreground `overdrive deploy` and require its public
  terminal `Stable` result/zero exit, then use a kind-aware describe parser to
  prove the allocation remains `Running`; or
- retain detached deploy but parse the Service `Alloc / State` table plus
  `Replicas (desired/running): 1/1` and wait for `Running`, not `Stable`.

Keep the Job parser separate and continue requiring `Terminated` plus `Verdict:
Succeeded` for the caller. Add a host-safe parser check using exact current
Service and Job render shapes so a table/aggregate-state mismatch fails before
the native-metal run.

### H2-01 — KEK preparation mutates an ambient production key without per-run ownership

**Severity:** High

**Prior finding:** H-01 and M-01

**Evidence**

- `prepare.sh cleanup` unconditionally runs
  `keyctl purge user 'overdrive:ca:kek:overdrive-ca-root'` whenever `keyctl` is
  installed (`prepare.sh:257-259`). This happens even when the marker-owned
  output tree does not exist.
- The operator runner invokes that cleanup before preparation
  (`run-example.sh:274-278`) and again during teardown. No code creates a fresh
  kernel session keyring or records the exact key id created by this run.
- Production uses the same stable description in the session keyring
  (`crates/overdrive-host/src/ca/keyring.rs:95-103,172-209`). Resolution is
  keyring-first and consults `CREDENTIALS_DIRECTORY` only on a miss
  (`keyring.rs:363-382`). An ambient key can therefore replace the supposedly
  per-run credential; the unconditional purge can also delete a key that this
  run did not create.
- The repository's existing black-box KEK runner identifies this exact cache
  hazard and launches each boot under `keyctl session -` so prior session keys
  cannot mask credential delivery
  (`verification/expectations/O04-ca-refuse-to-start-actionable-error/runner.sh:55-65`).

**Platform impact**

On a shared native host, the example can destroy a pre-existing production-
description key outside its marker-owned paths. Conversely, if keyctl is absent
or isolation differs across PAM/sudo environments, `serve` can reuse an
ambient cached KEK and never consume the fresh 32-byte file, making the
“per-run production credential” claim non-attributable.

**Required remediation**

Run the E07 `serve` process inside a fresh, explicitly created session keyring
(the repository precedent is `keyctl session -`). Make keyring isolation a
required precondition rather than optional cleanup. Remove the unconditional
ambient-description purge from `prepare.sh`; teardown should destroy the
isolated session or revoke only an exact key id proven to have been created by
this run. Keep the credential file itself under the marker-owned tree and
verify the fresh session starts without the description before `serve` resolves
the delivered file.

### M2-01 — Cleanup still deletes unsnapshotted global state and suppresses public stop failures

**Severity:** Medium

**Prior finding:** M-01

**Evidence**

- `snapshot_shared_state` records links, network namespaces, and selected
  processes only (`run-example.sh:93-98`). It does not snapshot VM run
  directories or allocation cgroups.
- `cleanup_runner_owned_paths` then removes every global run directory and
  cgroup matching the fixed caller/callee identifiers
  (`run-example.sh:125-153`), regardless of whether the path existed before the
  run. The final probes use the same absolute-name test rather than a
  before/after delta (`:174-185`).
- Both exact public stop commands discard every timeout, transport, HTTP, and
  product failure with `|| true` (`run-example.sh:43-47`). A successful main
  journey can therefore exit zero after direct host cleanup even though its
  promised public cleanup commands failed.
- `PREPARED` becomes 1 only after the bounded `prepare` subprocess returns
  (`run-example.sh:276-278`). If preparation is killed after creating its marker
  but before its own trap completes, the outer trap skips the cleanup retry
  because it still sees `PREPARED=0`.

**Platform impact**

The canonical lease excludes concurrent supported writers but does not confer
ownership of state that predates lease acquisition. A stale or intentionally
retained allocation with the same fixed identifier can be deleted by this
example. Cleanup can also be reported successful after the supported stop path
failed, or leave a marker-owned partial preparation after an outer timeout.

**Required remediation**

Snapshot matching VM run directories and cgroups before deployment and remove
only the proven new delta; fail before deployment if a fixed-id precondition
would make attribution ambiguous. Capture each public stop result and mark
cleanup failed on timeout/error, while retaining direct delta cleanup as a
last-resort hygiene fallback. Arrange the outer trap to retry marker-owned
preparation cleanup whenever the output marker exists, including when the
prepare command times out before `PREPARED` is assigned. Finish with bounded
mount/loop, marker-root, run-directory, cgroup, process, netns, and link residue
probes.

### M2-02 — The harness's fail-closed/substrate migration is incomplete outside the E07 stub path

**Severity:** Medium

**Prior finding:** H-02

**Evidence**

- The modified harness correctly returns nonzero for E07's exit-75 stub, but
  its final predicate still treats `RUNNER_RC=n/a` as success
  (`verification/harness/run-expectation.sh:134-140`). In an isolated copy,
  making E07's runner non-executable produced `execution_status: pending`,
  `runner_invoked: false`, and process exit 0.
- The harness now defaults every expectation without an `execution-substrate`
  file to `lima` (`run-expectation.sh:41-54`). E07 is the only expectation with
  that file. The active, satisfied E06 runner is also native-metal and explicitly
  says so, but has no checked-in substrate declaration; a fresh E06 run under
  this harness would record `execution_substrate: lima` and
  `executed_in_lima: true` again.
- `verification/README.md:65-71` now states that native KVM expectations declare
  `native-metal` and that the manifest records the true substrate, while E06's
  active README/runner retain the old “field is literally inaccurate” caveat.
  The catalogue and harness therefore disagree about the completed migration.

**Platform impact**

The central evidence command still has a zero-exit pending path, and its new
authoritative substrate field can be false for an existing native expectation.
Neither defect changes E07's current exit-75 result, but both undermine the
shared harness whose correctness the remediation claims.

**Required remediation**

Return nonzero for every non-`succeeded` execution status, including an absent
or non-executable runner. Add the checked-in `native-metal` declaration to E06
and update its active prose to distinguish historical pinned manifests from
new harness behavior. Add host-safe harness tests for succeeded, exit-75
pending, absent-runner pending, failed, Lima, native-metal, and invalid-substrate
branches.

## Iteration-2 platform boundary assessment

| Requirement | Result | Assessment |
|---|---|---|
| Exactly one feature expectation | PASS | One active E07 directory/row/mapping; no E08/E09 active obligation |
| One VM Job calls one Exec Service | PASS structurally | Checked-in source and spec topology remains exact |
| Callee can become available | PASS in product policy | Explicit-empty startup policy removes the unreachable inferred probe |
| Operator journey can complete | **FAIL** | Detached Service convergence waits for impossible `Stable` through a Job-only parser (B2-01) |
| Explicit appliance preparation | PASS except keyring ownership | Static binaries, rootfs, kernel, data/config paths, and production credential file are concrete |
| KEK delivery/cleanup is per-run | **FAIL** | Ambient stable key is purged/reused without a fresh session or exact ownership (H2-01) |
| Default-feature black-box boundary | PASS | No crate/test harness or private D7 oracle drives the product result |
| No inline source/TOML generation | PASS | All source/spec definitions remain checked in |
| Native preflight and global lease | PASS | Canonical `metal run` encloses preparation through final command cleanup |
| E07 pending evidence is fail-closed | PASS | Stub exit 75 -> pending/native-metal/non-Lima -> harness nonzero |
| Shared harness globally fail-closed/truthful | **FAIL** | No-runner exits zero and E06 lacks native substrate migration (M2-02) |
| Caller deadlines | PASS | DNS, connect, read, write, retry, and total deadline are finite |
| Shared-host cleanup attribution | **FAIL** | Run-dir/cgroup state is not delta-scoped; stop errors and partial prep cleanup can be hidden (M2-01) |
| Roadmap execution authority | PASS | Validation remains pending and requires regeneration/fresh approval |

## Verification performed for iteration 2

- Read the full iteration-1 platform artifact and compared every remediation
  path in commit `f064e0566611b1b8c4da7775fc169b929bccbca3` with parent
  `9558af759e049564b7149bea6459961979899e96`.
- Inspected all nineteen changed paths, including both preparation/runtime
  scripts, caller/callee code and specs, the renamed E07 contract/runner,
  substrate metadata, catalogue/index, harness, DISTILL handoff, and pending
  roadmap.
- Confirmed the target's changed active files match the target commit despite
  unrelated pre-existing dirty work in the shared workspace.
- Ran `bash -n` on `prepare.sh`, `run-example.sh`, the E07 runner, and the shared
  harness; all passed. Shellcheck was available and reported no findings.
- Ran both host-safe `check-source` entry points; both passed.
- Compiled caller and callee as Rust 2024 metadata with warnings denied; both
  passed. This is source/tooling validation, not native-metal evidence.
- Parsed the roadmap with `jq`, confirmed `validation.status = pending`, found
  exactly one E07 contract, and verified the final gate names only E07 evidence.
- Confirmed exactly one active `E07-*` directory, no active E08/E09 references,
  no active old E07 slug, executable modes `0755` for scripts, and
  `git show --check f064e0566611b1b8c4da7775fc169b929bccbca3` clean.
- Executed the E07 harness path in an isolated temporary Git repository. The
  executable stub returned harness status 1 with `execution_status: pending`,
  `execution_substrate: native-metal`, `executed_in_lima: false`, and runner
  exit 75. The non-executable-runner control retained `pending` but returned 0.
- Replayed the checked-in state parser against current Service and Job render
  shapes; it returned no Service state and returned `Terminated` for the Job.
- Traced production KEK resolution, Service/Job describe rendering, generic
  stop routing, VM reflink staging, native preflight, and lease retention to
  their production call sites.
- Native-metal execution and mutation testing were not run. E07 and the roadmap
  remain explicitly pending, and mutation testing is forbidden for this review.

## Iteration-2 final disposition

**NEEDS_REVISION.** The remediation materially improves the boundary but the
sole operator journey remains non-executable due to B2-01. Close B2-01, H2-01,
M2-01, and M2-02 in a fresh immutable commit, then repeat this same-reviewer
platform re-review. Do not capture E07 evidence or treat the pending roadmap as
executable before those findings and the existing regeneration/approval gate
are closed.

For iteration 2, this reviewer modified only this review artifact and preserved
all pre-existing tracked and untracked workspace changes.

---

# Iteration 3 — Remediation Re-review

## Metadata

| Field | Value |
|---|---|
| Review role | Platform architect reviewer |
| Review scope | Same DISTILL expectation boundary; iteration-2 remediation |
| Target commit | `5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Parent commit | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Cumulative comparison | `9558af759e049564b7149bea6459961979899e96..5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Review date | 2026-08-29 |
| Native-metal execution | Not run; E07 and roadmap remain pending |
| Mutation testing | Not run; prohibited for this review |

## Verdict summary

**NEEDS_REVISION.** Commit `5279a561` closes all four iteration-2 findings in
their substantive boundary: the operator now observes the real Service and Job
render shapes, `serve` uses a fresh session keyring without ambient purges,
cleanup uses required public stops plus token-owned fixture removal, and the
generic harness/E06 substrate migration is fail-closed and covered by host-safe
branch tests.

One new high-severity lifecycle defect remains in the fresh-session launch
handoff. Before the child PID is published and verified as the product binary,
the exit trap has no bounded way to terminate the already-running `keyctl`
wrapper/session. A signal or the explicit PID-handshake timeout can therefore
hang cleanup indefinitely; a signal after PID publication but before `exec`
can instead make cleanup refuse the owned child and leave it running. This
violates E07's own bounded success/error/signal cleanup contract.

Approval requires zero blocker, high, or medium findings. Iteration 3 has one
high finding.

## Finding counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 1 |
| Medium | 0 |
| Low | 0 |

## Iteration-2 finding disposition

| Iteration-2 finding | Disposition | Evidence |
|---|---|---|
| B2-01 — impossible Service state through a Job-only parser | **CLOSED** | `run-example.sh:161-234` separates the Service `Alloc / State` parser from the Job `Attempt / State` parser and contains a host-safe cross-shape rejection check. The Service gate requires `Running` plus exact replicas `1/1` (`:198-215`); the Job gate requires `Terminated`, followed by exact `Verdict: Succeeded` (`:217-234,317-320`). These shapes match production `render.rs:1094-1139` and `:1037-1068,1146-1150`. |
| H2-01 — ambient KEK mutation and non-attributable credential | **CLOSED, with a distinct launch-lifecycle defect below** | `prepare.sh` no longer purges any key. `run-example.sh:263-294` requires `keyctl session -`, proves the fresh session is accessible, rejects a pre-existing production description in `@s`, and starts the built `serve` with the marker-owned credentials directory. Nominal termination releases that anonymous session. H3-01 concerns interruption during the wrapper-to-product handoff, not ambient key reuse. |
| M2-01 — unsnapshotted global deletion, hidden stop errors, partial-prep gap | **CLOSED** | The expectation boundary was corrected rather than retaining a private-state sweeper: E07 no longer deletes or asserts product-private run directories, cgroups, processes, namespaces, or links, which remain Rust-test responsibilities. `run-example.sh:90-123` requires both exact public stop calls, records their output, marks every error, terminates only its verified serve lifecycle, and removes only the exact token-marked preparation tree. Deployment/preparation flags are set before the possibly partial operations (`:258-260,308-316`), so failure paths attempt public/marker cleanup and cannot turn cleanup failure into a successful run. |
| M2-02 — incomplete harness/substrate migration | **CLOSED** | `run-expectation.sh:134-140` returns zero only for `execution_status: succeeded`; absent, exit-75, and failed runners all return nonzero. E06 and E07 both declare `native-metal`; E06's active README/runner distinguish the historical pinned manifest from fresh truthful fields. `test-run-expectation.sh` covers success, pending-75, absent, failed, default-Lima, native-metal, other, and invalid-substrate branches, and passed locally. |

## What is correct in iteration 3

- Active feature verification still has exactly one expectation directory and
  catalogue row, `E07-vm-job-calls-exec-service`. There are no active E08/E09
  mappings or remnants of the superseded E07 slug in the feature's live
  DISTILL, roadmap, example, catalogue, or expectation artifacts.
- The checked-in bundle still expresses exactly one `[service]` + `[exec]`
  callee and one `[job]` + `[vm]` caller. The Job can report the public
  `Succeeded` verdict only after its checked-in caller receives the byte-exact,
  byte-distinct reply and exits zero.
- The Service convergence gate uses the real public allocation state
  `Running` and exact replicas `1/1`; the Job terminal gate independently uses
  `Attempt / State = Terminated` plus `Verdict: Succeeded`. The host-safe parser
  self-check rejects applying either parser to the other workload kind.
- `callee.toml` retains the supported explicit-empty startup policy, avoiding
  the production-unreachable inferred host-namespace TCP probe without
  inventing a new product state.
- `prepare.sh` arms its exit/signal traps and records process-local ownership
  before its first write to the fixed output tree (`:210-221`). The operator
  adds a per-invocation UUID token, refuses any pre-existing output tree, and
  will not remove a marker belonging to a different invocation.
- KEK delivery now follows the repository's isolation precedent: `keyctl` is a
  required precondition, `serve` runs inside `keyctl session -`, the new
  session must not already contain `overdrive:ca:kek:overdrive-ca-root`, and no
  ambient key is purged. The credential file remains inside the token-owned,
  mode-restricted fixture tree.
- Cleanup is at the correct black-box boundary. It requires the public generic
  workload stop endpoint for both the naturally terminated Job and the
  Service, records failures instead of suppressing them, and does not duplicate
  private product cleanup assertions that belong in Rust integration/native
  tests. Its direct filesystem action is restricted to the invocation-token
  fixture delta.
- The generic evidence harness is now fail-closed for every non-succeeded
  status. Both existing native expectations carry explicit substrate metadata,
  and E06's historical evidence remains immutable while active documentation
  accurately describes fresh captures.
- E07 remains an executable exit-75 stub with no fabricated product evidence.
  In an isolated repository, the harness returned 1 and recorded `pending`,
  `native-metal`, `executed_in_lima: false`, and runner exit 75.
- D7 framing, nft identity, packet/counter equality, TLS/kTLS, generation/loss,
  boot diagnostics, C4a, restart/reclamation, stop/idempotency, sibling
  preservation, private cleanup, and replay remain Rust-only. E07 observes only
  the stakeholder-visible named-peer reply-dependent outcome.
- Roadmap validation remains `pending` with `requires_regeneration = true` and
  the final evidence gate names only E07. This review grants no roadmap
  execution or evidence-capture authority.

## Iteration-3 finding

### H3-01 — Early fresh-session launch paths cannot terminate the owned wrapper within a bound

**Severity:** High

**Evidence**

- `run-example.sh:263-295` backgrounds `keyctl session - ...`, but the child
  shell writes its PID before `exec` at `:289-291`, while the parent records the
  keyctl wrapper PID only after the background launch at `:295` and reads the
  child PID later at `:296-305`. Signals are already armed, so cleanup can run
  in either handoff window.
- `terminate_serve` enters when either PID is present, but signals a process
  only when `SERVE_PID` is nonempty, live, and already resolves through
  `/proc/<pid>/exe` to the built `overdrive` binary (`:52-81`). If only
  `SESSION_WRAPPER_PID` is known, it sends no signal and performs an unbounded
  `wait` on that wrapper (`:83-85`).
- The five-second PID-file loop does not resolve this. If the wrapper remains
  alive without publishing the file, `:303` calls `die`; the EXIT trap then
  takes the unbounded wrapper-only wait described above. A signal in the same
  window has the same result.
- If the child has published `$$` but has not yet completed `exec`, its exact
  executable is still `bash`. `terminate_serve` then refuses to signal the
  process and returns early at `:66-70`, so the owned wrapper/session can
  continue while the outer cleanup removes its credential/data tree.
- E07's checked-in contract explicitly requires traps to work on success,
  error, and signal, termination of the started serve PID with a bounded
  TERM/KILL wait, and death of the fresh session keyring with that lifecycle
  (`verification/expectations/E07-vm-job-calls-exec-service/README.md:59-65`).

**Platform impact**

An interruption or stalled launch during this ordinary asynchronous handoff can
hold the canonical native-metal lease indefinitely, or leave the fresh keyring
wrapper/product descendant alive after fixture removal. That contaminates the
shared host and makes the operator's promised finite cleanup untrue. The
nominal no-signal journey remains structurally runnable, so this is high rather
than blocker severity.

**Required remediation**

Make the entire fresh-session launch unit addressable before it can outlive the
parent—for example, an owned process group/session wrapper with a recorded
wrapper identity—and bound every teardown wait. Cleanup must terminate that
owned unit when the product PID is absent, not yet exec'd, or already exec'd;
the exact-product check may protect unrelated reused PIDs, but must not make the
owned pre-exec handoff unkillable. Add host-safe fault-injection coverage for
at least: signal before PID publication, PID-handshake timeout while the wrapper
is live, signal after publication but before exec, normal TERM completion, and
TERM-to-KILL escalation. Each branch must finish within its deadline and leave
no wrapper/descendant alive.

## Iteration-3 platform boundary assessment

| Requirement | Result | Assessment |
|---|---|---|
| Exactly one feature expectation | PASS | One active E07 directory/row/mapping; E08/E09 remain absent and Rust-only obligations remain unmapped |
| One VM Job calls one Exec Service | PASS | Exact checked-in topology and reply-dependent zero-exit result |
| Service convergence semantics | PASS | Public `Alloc / State = Running` plus replicas `1/1` |
| Job success semantics | PASS | Public `Attempt / State = Terminated` plus `Verdict: Succeeded` |
| Explicit appliance preparation | PASS | Static helpers, private rootfs/kernel, exact guest path, same-filesystem data path, and credential are concrete |
| Fresh KEK ownership | PASS nominally | Fresh anonymous session begins without the production description; ambient keys are neither reused nor purged |
| Trap ordering | PASS | Outer cleanup is armed before runtime mutation and preparation cleanup is armed before the fixed-tree write |
| Bounded serve/session cleanup | **FAIL** | Wrapper-only and published-pre-exec interruption paths are not safely bounded (H3-01) |
| Public/owned-delta cleanup boundary | PASS | Required public stops are honest; only the invocation-token fixture tree is directly removed; private cleanup stays in Rust |
| Default-feature black-box boundary | PASS | No crate, Rust test binary, or private feature oracle drives E07 |
| Shared harness fail-closed/substrate truth | PASS | Only succeeded returns zero; E06/E07 declare native-metal; branch tests pass |
| E07 pending evidence honesty | PASS | Exit 75 records pending/native-metal/non-Lima and returns nonzero |
| Rust-only internal guarantees | PASS | D7, E08/E09-class internal obligations, and private cleanup are not duplicated into E07 |
| Roadmap execution authority | PASS | Validation remains pending and requires regeneration/fresh approval |

## Verification performed for iteration 3

- Read the complete iteration-2 review and inspected all fifteen paths changed
  by `5279a561fcff74f61e8329f07fb6a72af0abe051`, plus the cumulative active
  expectation/example/DISTILL/roadmap boundary from `9558af75`.
- Confirmed every reviewed active target file matches the target commit despite
  unrelated pre-existing tracked and untracked workspace changes.
- Ran `bash -n` on both example scripts, the E06/E07 runners, and both harness
  scripts; all passed. Shellcheck passed with only its known dynamic-source
  informational (`SC1091`) excluded for E06's checked-in helper source.
- Ran `prepare.sh check-source` and `run-example.sh check-source`; both passed,
  including the exact Service/Job parser fixtures and cross-kind rejection.
- Compiled both checked-in Rust helpers as Rust 2024 metadata with warnings
  denied; both passed. This is source/tooling validation, not native evidence.
- Ran `verification/harness/test-run-expectation.sh`; all declared status and
  substrate branches passed.
- Ran the actual E07 stub through the harness in an isolated temporary Git
  repository. The harness returned 1 and its manifest recorded `pending`,
  `native-metal`, `executed_in_lima: false`, and exit 75.
- Parsed the roadmap with `jq`, confirmed `validation.status = pending` and
  `requires_regeneration = true`, found exactly one E07 directory, and found no
  active E08/E09 or superseded-E07 references in the reviewed boundary.
- Confirmed all six executable scripts are mode `0755` and
  `git show --check 5279a561fcff74f61e8329f07fb6a72af0abe051` is clean.
- Traced production Service/Job rendering, Job verdict derivation, generic stop
  routing/idempotency, and the checked-in public/private evidence split to
  their production and contract call sites.
- Native-metal execution and mutation testing were not run. E07 and roadmap
  execution remain expressly pending, and mutation testing is prohibited here.

## Iteration-3 final disposition

**NEEDS_REVISION.** Close H3-01 in a fresh immutable commit, including bounded
host-safe lifecycle fault tests, then repeat this same-reviewer platform
re-review. Do not capture E07 evidence or treat the pending roadmap as
executable before the finding and the existing regeneration/approval gate are
closed.

For iteration 3, this reviewer modified only this review artifact and preserved
all pre-existing tracked and untracked workspace changes.
