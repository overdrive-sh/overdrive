# E09 v2 — persistent-control-plane VM Service TCP truthfulness

This is the versioned E09 v2 operator journey.  It preserves the original
`examples/service-kind-vm-workloads/` bundle and its E09 runner.  The v2
journey reuses that bundle's checked-in `guest_server.rs`, `client.rs`, and
four source specifications, but materializes its own marker-owned artifacts
under `/srv/vm/overdrive-testing/svm-e08-v2`.

## Contract

`run-example.sh run tcp-truthfulness-100` performs one product build, one
guest-program compilation and rootfs preparation, and one `overdrive serve`
start.  It then schedules 100 logical trial pairs in conservative bounded
cohorts through that same live control plane.  A trial has a unique healthy
and failed Service identity and a corresponding unique VM peer Job identity;
the IDs are never reused by simultaneous workers.

For the healthy half, the public Service stream must render Stable with the
guest TCP startup probe as its witness, `workload describe` must report the
guest TCP and HTTP readiness observations as passing, and the checked-in VM
peer Job must report `Verdict: Succeeded` after receiving the exact
`SVM-E08-GUEST-OK` reply through the Service frontend.  For the failure half,
the public deployment must exit nonzero, publish the typed
`StartupProbeFailed` outcome for the deliberately unbound guest TCP port
`18999`, and never render Stable.  A public `Failed` row is accepted directly;
the existing replacement-start trajectory is also accepted only when
`workload describe` shows a nonzero restart count and the typed prior
`bind beacon listener: Address already in use` termination together with the
failed startup-probe observation.  The checked-in negative-control VM peer Job
must complete successfully only as the unreachable assertion.  Each Service
and Job is stopped through the existing public `job stop` operation.

The scheduler holds a barrier after each cohort's healthy Service reaches
Running and another after each failure Service reaches its typed startup
failure.  Cohort markers and per-allocation cgroup/run-directory evidence
prove that the configured workers overlap before release.  Polling and
reclamation are bounded; they are not retries of a deploy or a discarded
trial.  If a failure or cancellation stops the suite, every input pair still
gets a deterministic ledger row (`failed` or `not-run-cancelled`).  The final
ledger is emitted in trial order even though worker completion is concurrent.

The suite records the control-plane PID, Linux process start ticks, and
executable path before the first deployment.  Every worker barrier, public
describe poll, and stop/reclamation wait checks that identity, and successful
ledger rows are required to contain one unchanged PID/start pair.  The
control plane is stopped once, after the final measurements.

## Preparation and resource evidence

`prepare.sh prepare` compiles and validates the two immutable guest programs,
copies the expected kernel, clones the base rootfs once, installs both guest
programs once, and validates the mounted image before returning.  Every VM
allocation then uses the product's existing allocation-scoped rootfs clone;
the v2 script does not cache mutable case state between trials.  The v2
runner does not invoke `prepare.sh check`, which would remount the image a
second time; `prepare` itself performs the guest-binary and filesystem
validation.  `prepare.sh check` remains available for an operator's separate
post-preparation audit.

Before the first cohort, after every cohort's workers have stopped, and both
before and after the one control-plane shutdown, the runner snapshots the
shared runtime surfaces: Cloud Hypervisor processes, workload cgroup scopes,
VM run directories, network namespace/link names, attached BPF programs,
nftables rule identities, rootfs clone staging and clone-index entries, loop
devices, mounts, and serve-store file/byte totals.  Runtime diffs are
compared only after all owned workers in that cohort are stopped.  Durable
Service/Job records intentionally remain in the control-plane store and are
reported as store-growth measurements rather than misclassified as runtime
leaks.  Per-case resource files identify only allocation IDs observed through
public `workload describe`; no global snapshot subtraction is used while
cases are live.

The default concurrency is `2` only when the host has at least four CPUs and
one GiB of memory; otherwise it is `1`.  `SVM_E09_V2_CONCURRENCY` may lower or
raise it up to four, but never above half the detected CPU count.  Set
`SVM_E09_V2_OUTPUT_ROOT` only to a new path below the fixed staging root; the
marker and bounded cleanup refuse unowned materialization.

## Invocation

Run source validation on any host:

```bash
examples/service-kind-vm-workloads-v2/run-example.sh check-source
```

Run the full journey through the authorized native-metal wrapper:

```bash
cargo xtask metal run -- \
  bash -lc 'examples/service-kind-vm-workloads-v2/run-example.sh run tcp-truthfulness-100'
```

The expectation wrapper invokes the same command and captures its verbatim
ledger, timing, concurrency, per-case transcript, and final-cleanup sections:

```bash
verification/harness/run-expectation.sh E09-v2
```

No native 100-pair result is implied by this checked-in script.  The separate
host-safe scheduler check uses synthetic `sleep` workers solely to verify the
barrier, one-process identity, out-of-order completion, and deterministic
ledger mechanics; it is not product evidence:

```bash
examples/service-kind-vm-workloads-v2/test-scheduler.sh
```
