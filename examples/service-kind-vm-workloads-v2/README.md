# E09 v2 — persistent-control-plane VM Service TCP truthfulness

This is the versioned E09 v2 operator journey.  It preserves the original
`examples/service-kind-vm-workloads/` bundle and its E09 runner.  The v2
journey reuses that bundle's checked-in `guest_server.rs`, `client.rs`, and
four source specifications, but materializes its own marker-owned artifacts
under `/srv/vm/overdrive-testing/svm-e08-v2`.

## Contract

`run-example.sh run tcp-truthfulness-20` performs one product build, one
guest-program compilation and rootfs preparation, and one `overdrive serve`
start.  It then schedules 20 logical trial pairs in two ten-pair cohorts at
concurrency 10 through that same live control plane.  A trial has a unique healthy
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
`workload describe` shows a positive restart count and the same allocation's
prior `Failed` snapshot together with the failed startup-probe observation.
The original deployment remains nonzero and its immutable stream retains the
typed `StartupProbeFailed` outcome; a generic error, merely `Running`, or
`Stable` result remains rejected.  The checked-in negative-control VM peer Job
must complete successfully only as the unreachable assertion.  Each Service
and Job is stopped through the existing public `job stop` operation.

The scheduler holds the healthy-active barrier until all ten healthy Services
are observable. Each worker then completes its own healthy peer and Service
stop plus allocation-scoped runtime cleanup and immediately submits its
independent failure Service. It does not wait for siblings' healthy stops.
The failure-active barrier still waits for all ten typed startup failures
before releasing negative peers. Thus starts and stops from different workers
can overlap, while each worker preserves its cleanup-before-replacement order.
Per-allocation cgroup/run-directory observations retain active-cohort evidence.
Polling and reclamation are bounded; they are not retries of a deploy or a
discarded trial.  If a failure or cancellation stops the suite, every input
pair still gets a deterministic ledger row (`failed` or `not-run-cancelled`).
The final ledger is emitted in trial order even though worker completion is
concurrent.

The remote example owner has one 600-second (10-minute) budget for setup and trials,
followed by a separate 60-second bounded cleanup grace.  The timeout belongs
to the remote example process and its descendants; the parent transport wait
allows both windows to complete.  A timeout is nonzero and its partial ledger,
timing, identity, and transcript output is retained before ordinary
materialization cleanup.  This bounded 20-pair sample is functional
acceptance, not reliability, native-capacity, or throughput proof.

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

The functional acceptance requires exactly 10 concurrent workers.  The
existing `SVM_E09_V2_CONCURRENCY` override is accepted only when it is `10`;
invalid host CPU or memory observations remain precondition errors, but no
half-CPU or four-worker cap silently reduces the requested concurrency.  Set
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
  bash -lc 'timeout --signal=TERM --kill-after=60s 600s \
    env SVM_E09_V2_CONCURRENCY=10 \
    examples/service-kind-vm-workloads-v2/run-example.sh run tcp-truthfulness-20'
```

The expectation wrapper invokes the same command and captures its verbatim
ledger, timing, concurrency, per-case transcript, and final-cleanup sections:

```bash
verification/harness/run-expectation.sh E09-v2
```

No native result or performance claim is implied by this checked-in script;
native verification remains pending.  The separate host-safe scheduler check
uses synthetic `sleep` workers solely to verify the
barriers, independent failure submission during a sibling stop, one-process
identity, out-of-order completion, and deterministic
ledger mechanics; it is not product evidence:

```bash
examples/service-kind-vm-workloads-v2/test-scheduler.sh
```
