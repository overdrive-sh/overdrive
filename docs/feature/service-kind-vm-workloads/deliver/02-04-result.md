# Step 02-04 result — E09-v2 bounded acceptance and native verification

Date: 2026-09-09

## Verdict

**Step 02-04 is complete and APPROVED.** Native execution succeeded for
E09-v2, E10, and E13. The [independent evidence audit](../../../analysis/review-02-04-final-evidence.md)
marks all three expectations SATISFIED, and the
[final step review](review-02-04.md) approves closure. Their expectation
READMEs and catalogue entries record `satisfied`.

The first E10 attempt stopped at native setup because the fixed materialization
path was already present. Its exact evidence is preserved under
`verification/expectations/E10-vm-service-http-cross-driver-status/evidence/first-attempt-2026-09-08T234614Z/`.
The existing marker-owned preparation cleanup was used only after canonical
lease inspection established that the tree was this checked-in example's
retained failure materialization and was not in active use. E10 was then rerun
once, followed by E13 in the required order.

## Executed host-safe checks

The bounded host-safe checks passed before native execution:

```sh
bash examples/service-kind-vm-workloads-v2/test-scheduler.sh all
bash verification/harness/test-e09-v2-runner.sh all
bash docs/analysis/test-e09-v2-failure-lifecycle.sh
bash docs/analysis/test-e09-v2-failure-submit-order.sh
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

The scheduler checks covered the all-ten healthy-cleanup barrier and bounded
missing-worker cancellation. The predicate checks covered the accepted
same-allocation recovery and all required rejection controls.

## Fresh E09-v2 native capture

The command was:

```sh
verification/harness/run-expectation.sh E09-v2
```

It ran on `native-metal` with seed `1`, product SHA
`9f6702e431c1a68a3374445368ea8fc525f23bc4`, one build/preparation, one
persistent control-plane process, two ten-worker cohorts, the approved
1200-second setup/trials budget, 60-second cleanup grace, and 1320-second
transport bound. The native lease and x86_64/KVM preflight both completed.

The captured run completed at `2026-09-08T23:46:05Z` with exit `0`. Its
ledger contains exactly 20 `pass`/`complete` rows sharing one control-plane
PID/start-tick identity. Every healthy peer succeeded with deploy result `0`;
every failure Service returned deploy result `1`, retained typed
`StartupProbeFailed` on guest TCP port `18999`, and its negative peer Job
succeeded. The capture records zero-runtime cleanup for every pair, no retry,
replacement, or discarded pair, and the success line:

```text
E09 v2 PASS: 20/20 truthful functional pairs at concurrency 10 through one unchanged control-plane PID; no retries, replacements, or discarded pairs
```

Evidence is retained under
`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/`,
including `product-run.out`, `product-run.meta`,
`tcp-truthfulness-20.tsv`, `run.log`, and `verification.yaml` with
`execution_status: "succeeded"`.
The transcript contains one non-fatal `/proc/<pid>/stat` race while sampling a
short-lived process; the runner completed all assertions and returned zero.

## E10 first attempt and bounded materialization cleanup

After E09 passed, the required next command was:

```sh
verification/harness/run-expectation.sh E10
```

Native lease acquisition, source synchronization, and the fail-closed KVM
preflight completed. The product runner then exited before building or running
the E10 matrix:

```text
svm-e08 run: refusing to overwrite pre-existing materialization: /srv/vm/overdrive-testing/svm-e08
```

The first attempt's exit-1 manifest and unmodified runner output are retained
in the `first-attempt-2026-09-08T234614Z` evidence appendix named above.

Before cleanup, a canonical exclusive-lease inspection established:

- `/srv/vm/overdrive-testing/svm-e08` was mode `0711`, owned by `root:root`,
  and its marker was exactly `svm-e08-owned-v1:e09-074-failure`;
- `examples/service-kind-vm-workloads/prepare.sh check` returned zero;
- the private mount was unmounted and the rootfs had no loop attachment;
- no product process was using the tree; and
- its retained logs were from the checked-in example's TCP startup-failure
  journey (including `service-vm-tcp-failure`) on 2026-09-07.

The existing owner-guarded cleanup was then run through the canonical metal
lease:

```sh
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
cargo xtask metal run -- bash -lc \
  '/home/ubuntu/overdrive/examples/service-kind-vm-workloads/prepare.sh cleanup'
```

It returned zero with `removed SVM E08-owned materialization`; no foreign or
active tree was removed.

## Fresh E10 native capture

The E10 rerun used the unchanged expectation command:

```sh
verification/harness/run-expectation.sh E10
```

It ran from `2026-09-08T23:53:38Z` through `2026-09-08T23:55:50Z` and
returned zero on the same product SHA. The ledger contains exactly eight
rows: Exec/VM crossed with HTTP statuses 204, 302, 404, and 503. All rows
record the checked-in helper's `deploy_exit` result as `0`; for
302/404/503, that helper result means the expected startup-failure transcript
was retained and the bounded case helper completed, not that the Service
deployment succeeded. Every row has the expected terminal trajectory, zero
failure-body sentinel bytes in deploy stdout, deploy stderr, describe output,
and probe output, and `zero-delta` cleanup. The runner recorded:

```text
E10 PASS: 8/8 Exec/VM HTTP status cells with zero failure-body sentinel exposure
```

The current E10 manifest is
`verification/expectations/E10-vm-service-http-cross-driver-status/evidence/verification.yaml`
with `execution_status: "succeeded"`.

## Fresh E13 native capture

After E10 succeeded, the required next command was:

```sh
verification/harness/run-expectation.sh E13
```

It ran from `2026-09-08T23:55:55Z` through `2026-09-08T23:58:15Z` and
returned zero on the same product SHA. Its two-row ledger records healthy
inferred TCP success with an exact guest reply and failure inferred from
`StartupProbeFailed` with verified unreachable negative-peer traffic; both
rows have zero-delta cleanup. The runner recorded:

```text
E13 PASS: inferred TCP success/failure and complementary VM peer traffic are truthful
```

The current E13 manifest is
`verification/expectations/E13-vm-service-inferred-tcp-startup/evidence/verification.yaml`
with `execution_status: "succeeded"`.

## Historical reference and scope

The replaced result's c008-c010 artifact-leak interpretation was based on
pre-stop inventories and is disproven by the premise validation in
`docs/analysis/root-cause-analysis-e09-v2-vm-stop-cleanup-premise.md`; those
observations do not establish post-terminal residue or a cleanup-ordering
defect. The later real failed-Service disposal timeout and its bounded
60-to-180-second observation correction are reviewed in
`docs/analysis/review-e09-v2-disposal-timeout.md`. The fresh E09 result above
supersedes the old capture for current E09 evidence without reviving the
invalid leak claim.

No production/API/design change was made for this closure. No E10/E13
assertion, budget, fixture, or product behavior was relaxed or extended.
The independent reviewer approved roadmap-step completion in iteration 3 of
[review-02-04.md](review-02-04.md).
