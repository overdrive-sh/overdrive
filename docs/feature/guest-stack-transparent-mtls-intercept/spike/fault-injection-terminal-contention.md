# Fault-injection probe: VM Job exit versus public stop

Date: 2026-08-31
Phase: NW-SPIKE Phase 1 PROBE
Repository commit: `e40899607d30c276dc9bbaf6975bc8c0331c272b`
Probe identity: `/root/spike_terminal_contention_v2`
Elapsed wall time: approximately 39 minutes (within the one-hour budget)

Verdict: BIGGER THAN EXPECTED

## Question

With the real default-feature Overdrive binary on the configured bare-metal
host, when a real guest VM Job's exit contends with the public stop path, do
the existing durable ObservationStore terminal state and normal cleanup
converge without an additional outbox, replay, hydration, or recovery
mechanism?

## Answer

The exact terminal-write contention question remains unproven. The black-box
probe could overlap the public stop with the frozen VMM's later exit, but it
could not observe internal `TransitionSource` values, occurrence rows, or
ObservationStore write outcomes. It therefore did not establish that both
terminal writers reached competing writes, or that an LWW rejection and the
bounded fresh-read rebase path executed.

What the probe did establish is a subordinate production
reachability/model-fidelity result: three clean-baseline repetitions with
different external overlap windows all converged to the same public result:

- the live public submit stream reported that the Job was stopped by the
  operator and exited with the documented stop status `130`;
- a subsequent public `workload describe` read reported the durable allocation
  row as `Terminated` with exactly `reason: stopped`;
- both exact allocation cgroups disappeared;
- the allocation netns, host/workload veths, and TAP disappeared;
- the marker-owned example materialization, Cloud Hypervisor process, and Exec
  process were absent in an independent, later canonical-lease inspection.

The probe added no product API or feature implementation. It used the
repository's current production binary and normal public
deploy/stop/describe/stream paths. Because it did not resolve the exact
writer-level schedule, it cannot support a conclusion for or against an
outbox, replay loop, hydrator, recovery path, or any other terminal-write
mechanism.

## Contract and production path checked

The accepted design describes LWW current state and bounded lifecycle
occurrences in the same ObservationStore transaction, while direct broadcast
remains best-effort (`design/wave-decisions.md:412`). It bounds the proposed
Stop versus exit-observer handling to at most two fresh-read compound proposals
and treats a second loss as successful convergence after releasing supervision
and the process-local route (`design/wave-decisions.md:558-565` and
`feature-delta.md:1546-1565`). It also removes the proposed terminal
outbox/replay protocol (`feature-delta.md:866-876`). These are design premises,
not behaviors established by this black-box probe.

The probe's scheduling premise used these referenced production-path facts:

- VM stop has a ten-second VMM grace window and awaits VMM termination
  (`crates/overdrive-worker/src/vm_driver.rs:71,1718`).
- Cloud Hypervisor termination waits for an already-exited VMM or the grace
  window before requesting a kill (`crates/overdrive-host/src/vmm.rs:534`).
- The real exit observer writes through `write_alloc_lifecycle`
  (`crates/overdrive-control-plane/src/worker/exit_observer.rs:548`).

These references were used only to choose a plausible external overlap window.
The production run confirms that the stop/exit ordering can be exercised at
the process boundary and can converge publicly; it does not reveal the
internal write schedule needed to answer the probe's exact question.

## Probe boundary

The probe used:

- the checked-in `guest-stack-transparent-mtls-intercept` example unchanged:
  exactly one VM Job (`gti-e07-caller`) calling exactly one Exec Service
  (`gti-e07-callee`);
- `cargo build -p overdrive-cli --bin overdrive` with default features;
- the canonical `cargo xtask metal run --` runner and its shared Run lease;
- an isolated checked-in session wrapper/keyring for every repetition;
- only public `deploy`, `job stop`, `workload describe`, and TTY-backed deploy
  streaming for workload control and terminal observation;
- `SIGSTOP`/`SIGCONT` only after resolving one PID from the exact allocation
  cgroup and revalidating its Linux start time, executable, and exact cgroup
  membership immediately before each signal.

There were no Rust tests, expectation runners, mocks, test binaries, inline
replacement workload specs, or production-code instrumentation.

## Fault ordering

Each repetition performed this same bounded sequence:

1. Deploy the Exec Service and wait for public `Running`.
2. Resolve its sole exact-cgroup process, prove it is the checked-in
   `e07-callee` binary, then stop that exact PID.
3. Start the checked-in VM Job through the real public streaming command and
   wait for public `Running`.
4. Resolve the sole caller cgroup process, prove it is Cloud Hypervisor for the
   exact caller allocation, and observe exactly one established callee socket
   with `Recv-Q=38`. This proves the single guest request reached the stopped
   Exec Service.
5. Stop that exact Cloud Hypervisor PID, resume the exact callee PID, and
   observe the connection move to `FIN-WAIT-2`. The response-side completion
   is therefore released while the guest/VMM remains frozen.
6. Issue the public Job stop while the VMM is still in Linux state `T`, retain
   the overlap for 2.25, 2.75, or 3.25 seconds, revalidate exact ownership, and
   resume that VMM PID. This externally overlaps the already-issued stop with
   guest completion/VMM exit; it does not show whether two terminal proposals
   subsequently reached competing ObservationStore writes.
7. Observe the public stream and a fresh public describe, then stop the
   Service and await structural cleanup.

## Clean-baseline repetitions

The final run began with no allocation netns, no allocation veth/TAP, and both
exact cgroups absent. Its canonical lease token was
`9bfbc6db77e5709b3d2bc153`; the lease metadata named the requested workspace
commit and scenario `terminal-contention-clean-baseline`.

| Repetition | Exact callee PID/start | Exact VMM PID/start | Stop-to-resume overlap | Queue proof | Stream | Stored current row | Cleanup |
|---|---:|---:|---:|---|---|---|---|
| 1 | `3257942/149867842` | `3258116/149867872` | 2.25 s | one `ESTAB`, `Recv-Q=38` | operator-stopped, rc 130 | `Terminated`, `reason: stopped`, clock 11 | scopes and allocation network absent |
| 2 | `3258511/149868394` | `3258685/149868422` | 2.75 s | one `ESTAB`, `Recv-Q=38` | operator-stopped, rc 130 | `Terminated`, `reason: stopped`, clock 11 | scopes and allocation network absent |
| 3 | `3259084/149868985` | `3259255/149869015` | 3.25 s | one `ESTAB`, `Recv-Q=38` | operator-stopped, rc 130 | `Terminated`, `reason: stopped`, clock 10 | scopes and allocation network absent |

The distinction between the two public projections is intentional: the Job
stream renders the operator outcome as “stopped,” while allocation current
state uses the terminal `Terminated` state with `reason: stopped`. Treating the
allocation state itself as a hypothetical `Stopped` variant would be a false
failure.

## Cleanup evidence

After every repetition, the exact caller and callee cgroup paths were absent,
and a bounded poll found no `ovd-ns-*` namespace, allocation `ovd-hv-*` /
`ovd-wl-*` veth, or TAP. The remaining `ovd-veth-bk` / `ovd-veth-cli` pair and
the two mark-exemption nft rules are shared node infrastructure, not
allocation residue.

After the final run exited successfully, its marker-owned
`/srv/vm/overdrive-testing/gti-e07` tree was removed. A separate canonical
lease then reported:

```text
FINAL_POST_CLEANUP output=absent caller_scope=absent callee_scope=absent netns=absent alloc_links=absent exact_product_processes=absent
FINAL_POST_CLEANUP verdict=clean
```

## Interpretation and limit

The result is conclusive only at the external boundary. It demonstrates that
three production stop/exit overlap schedules were reachable and that each
converged to an operator-stopped durable current row plus complete allocation
cleanup. The timings, public outcomes, exact process ownership, and cleanup
evidence remain useful for checking that a simulation models a production-
reachable schedule and the same public postconditions.

It does not answer the exact terminal-write contention question. The public
API and captured serve logs expose neither bounded occurrence-history rows nor
their internal `TransitionSource`, and the probe captured no ObservationStore
accept/reject or fresh-read rebase evidence. The observed outcome is compatible
with several internal schedules, including one terminal writer completing
before the other proposes a write. It therefore does not prove the bounded LWW
mechanism executed, and it does not establish that additional machinery is
unnecessary.

The exact question must be answered by a seeded `overdrive-sim` safety,
liveness, or convergence invariant that forces both terminal writers to reach
the competing-write boundary, observes the LWW loss and bounded rebase path,
and prints the reproducing seed. This real-metal evidence is subordinate
reachability/model-fidelity evidence for that invariant, not a replacement for
it.

## Transcript and reproducibility

The complete command lines, timestamps, canonical lease records, default
product build, exact PID ownership proofs, fault ordering, public product
outputs, cleanup snapshots, exit codes, and the preliminary fail-closed/harness
corrections are in:

- `spike-scratch/guest-stack-transparent-mtls-intercept/terminal-contention/transcript.log`
- `spike-scratch/guest-stack-transparent-mtls-intercept/terminal-contention/probe.sh`

The transcript intentionally excludes copied repository source/doc contents;
it retains concise source references and actual probe evidence only. It also
contains no internal `TransitionSource`, occurrence-row, LWW-rejection, or
rebase observation; that absence is the limitation behind the inconclusive
verdict.
