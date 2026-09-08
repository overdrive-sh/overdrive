# Step 02-04 result — E09-v2 bounded acceptance and native verification

Date: 2026-09-08

## Verdict

**Blocked.** The native E09-v2 black-box acceptance run failed through the
current production path. Step 02-04 cannot advance to E10/E13 or COMMIT.

## Executed evidence

The following host-safe scheduler and expectation-runner checks passed before
the native capture:

```sh
bash examples/service-kind-vm-workloads-v2/test-scheduler.sh
bash verification/harness/test-e09-v2-runner.sh
bash -n examples/service-kind-vm-workloads-v2/run-example.sh \
  examples/service-kind-vm-workloads-v2/test-scheduler.sh \
  verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh \
  verification/harness/test-e09-v2-runner.sh
shellcheck examples/service-kind-vm-workloads-v2/run-example.sh \
  examples/service-kind-vm-workloads-v2/test-scheduler.sh \
  verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh \
  verification/harness/test-e09-v2-runner.sh
git diff --check
```

The black-box native capture then ran:

```sh
verification/harness/run-expectation.sh E09-v2
```

It executed on the declared `native-metal` substrate at
`b653e1ad1758d11be33be457849b284f78141333` with a dirty working tree and
seed `1`. The remote owner received its approved 600-second setup-and-trials
budget plus 60 seconds of cleanup grace. It exited `1` at
`2026-09-08T17:46:12Z`.

The harness preserved the verbatim output, partial twenty-row ledger, timing,
identity, dirty-tree record, and manifest in
`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/`.
The manifest records `execution_status: "failed"`; it must not be treated as
satisfied evidence.

## Reproduced failure

One persistent `overdrive serve` process ran ten healthy Service workers in
cohort 1. Each reached Stable, and the corresponding VM peer Job succeeded.
The Service then reached the public `Terminated` state after `overdrive job
stop`, but its allocation-scoped VM resources did not disappear within the
runner's 60-second cleanup observation window.

For example, trial 4's public describe reported
`alloc-service-e09-v2-c004-h-0 Terminated`, while the same captured case still
listed its cgroup scope, VM run directory, staged rootfs clone, and clone-index
entry. Equivalent retention occurred for the other concurrently stopped
healthy Services. The scheduler consequently reported `healthy-stop` failure,
failed cohort 1, and retained the remaining inputs as `not-run-cancelled`.

The evidence shows these resources disappear only when the persistent control
plane is shut down during final cleanup. This is an actual production-path
result, not an outcome fabricated by the host-safe scheduler fixtures.

## Scope disposition

The approved 02-04 design allows only a necessary bounded E09-v2
runner/scheduler terminal-observation and cleanup-path correction. The
reproduced result instead establishes that the production control plane does
not complete allocation-scoped VM resource cleanup while it remains live.
Changing production cleanup ownership or weakening the accepted cleanup oracle
would require a DESIGN decision; neither action is authorized by this DELIVER
step.

E10 and E13 were not rerun because the strict execution sequence stops at the
failed E09-v2 acceptance gate. No DES phase was recorded for work not actually
performed, and no 02-04 COMMIT was created.
