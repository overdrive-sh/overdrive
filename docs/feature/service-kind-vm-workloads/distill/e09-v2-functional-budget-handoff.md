# E09-v2 functional acceptance amendment — test handoff

Date: 2026-09-08. Author: Codex acceptance designer. Scope: the user-approved
20-pair, ten-worker functional sample and remote 600s setup/trials + 60s cleanup
bound. This is not a DELIVER execution, native performance result, review
approval, or completion of step 02-04.

## Contract and evidence boundaries

The amended [S-SVM-25](test-scenarios.md#s-svm-25--bound-and-unbound-guest-ports-are-truthful-across-twenty-paired-trials)
is the detailed stakeholder contract. It retains exact healthy guest reply,
negative observation window, StartupProbeFailed, no discarded/retried pairs,
one build/preparation/control-plane identity, and cleanup assertions. Only the
approved sample, concurrency, and lifetime contract change. Longer soaks are
separately optional. E09-v1 and the failed E09-v2 100-pair captures remain
historical, never relabeled as a twenty-pair success.

The source-grounded diagnosis is
[`docs/analysis/e09-v2-concurrency-timing.md`](../../../analysis/e09-v2-concurrency-timing.md).
No new control-plane defect or architectural remedy is asserted here.

- `examples/service-kind-vm-workloads-v2/test-scheduler.sh` loads the real
  example through its existing read-only `check-source` selector, then exercises
  its existing capacity, materialization, worker-result, cohort, and ledger
  boundaries. Temporary output is shell-local and not inherited by the
  preparer. Capacity probes and native process/resource observations are
  explicit surrogate adapters; the worker fixture synchronizes ten real child
  processes per cohort. Production guest/deploy behavior is not simulated.
- `verification/harness/test-e09-v2-runner.sh` invokes the actual expectation
  runner with external command spies. The requested remote shell executes in
  an independent process session, like an SSH remote owner. Only the external
  example command is replaced with transcript inputs or a private owner plus
  TERM-resistant descendant. Actual timeout execution is scaled to 0.5s +
  0.3s while its unscaled remote arguments must remain 600s + 60s.
- Neither test invokes the product binary, compiles code, performs SSH,
  acquires a lease, exercises KVM, or writes expectation evidence. Their
  temporary captures are test inputs/results, not native product captures.
  No new production testability API, mode, or public method is required.

## Fresh bounded RED capture

Captured before releasing the implementation peer, against HEAD
`c28a5f94da5fc2a5823aeaa2fa4cdc5173dc976b` plus the initial dirty example/runner.
Capacity, manifest, cohort, and runner failures reached the stated behavior;
the partial-ledger capture was later found to include a fixture portability
error and is not isolated production RED evidence. No DES event was written.

| Command | Exit | Observed failure | Classification |
|---|---:|---|---|
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh capacity` | 1 | `concurrency is bounded at 4 worker pairs` | Missing approved ten-worker behavior |
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh manifest` | 1 | `expected 40 healthy/failure inputs, observed 200` | Legacy 100-pair materialization |
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh partial-ledger` | 1 | `partial ledger lost, reordered, or fabricated input outcomes (0 results)` | Confounded: legacy aggregate cardinality and macOS `seq 1 0` fixture creating trial 1; not isolated production RED |
| `timeout --signal=TERM --kill-after=1s 15s bash examples/service-kind-vm-workloads-v2/test-scheduler.sh cohorts` | 1 | `cohort results lost deterministic order` | Both ten-worker surrogate cohorts ran, but aggregate included 80 extra untouched rows |
| `timeout --signal=TERM --kill-after=1s 8s bash verification/harness/test-e09-v2-runner.sh valid` | 1 | `expected exactly 100 ledger rows` | Runner rejects the approved twenty-row sample |
| `timeout --signal=TERM --kill-after=1s 8s bash verification/harness/test-e09-v2-runner.sh timeout` | 1 | `deadline did not signal the remote owner` | Local timeout does not bound the independent remote owner |

The timeout fixture's surviving PIDs were terminated by its private test
cleanup. No real remote process was involved.

Passing preservation control: `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh
invalid-capacity` rejects `0`, `-1`, and `ten` with the existing diagnostic and
without capacity evidence. `bash -n` and `shellcheck` pass for both test scripts;
`git diff --check` passes.

## Coverage and scoped completeness

Explicit examples/finite loops are appropriate for these filesystem/process
integration boundaries; no generative PBT is run against external processes.
Every test carries a `CONTRACT_SHAPE: bounded-change` declaration. The fixture
functions are not separately represented as product acceptance tests.

| Checklist | Coverage or bounded-scope disposition |
|---|---|
| C1a | Empty partial ledger preserves twenty not-started inputs |
| C1b | Transcript samples 19/20/21 plus historical 100 |
| C2a | Scheduled → complete/failed/cancelled results, untouched → not-started; stable input ordering documented in tests |
| C2b | N/A: this amendment adds no stateful operator command; malformed terminal sample is rejected |
| C3 | 0/1/3 actual result files, and the complete twenty-row aggregate |
| C4a | Repeated aggregation is byte-identical, without adding rows |
| C4b | Aggregation without any worker result yields no fabricated success |
| C5a | Explicit/default ten-worker selection; pass/failed/cancelled outcomes; single/split identity |
| C5b | N/A: no new mode or orthogonal output flag |
| C6a | Malformed concurrency input; wrong sample cardinality |
| C6b | Failed row, split identity, deadline, and invalid capacity error paths |
| C6c | Successful transcript inputs must reach metadata exit 0 before a validation rejection is accepted; timeout remains nonzero |
| C7a | N/A: no new resource-starvation policy; native capacity remains unverified, not inferred from the surrogate |
| C7b | Independent remote-session timeout, TERM-resistant descendant, retained partial transcript |
| C7c | Two ten-worker cohorts, both healthy/failure barriers, out-of-order completion, one unchanged surrogate identity |

All 15 scoped items have coverage or an explicit N/A disposition; this is an
authorship self-check, not the independent review verdict. Existing complete
feature acceptance and native evidence obligations remain separate.

## Implementation handoff

The implementation peer was released after the fresh capacity/manifest RED
capture. It owns production example/runner changes and shared feature-delta
edits. Exact DISTILL replacement text was sent directly for the S-SVM-25 row,
ADR-0094 inherited-commitment impact, and K1 disposition paragraph. This agent
edited only the detailed scenario, these tests, and this handoff.

## Final host-safe verification

After the implementation peer's production amendment, the full scheduler run
exposed the zero-result fixture portability error: macOS `seq 1 0` emits `1`
and `0`. The fixture now uses a Bash arithmetic loop, producing zero iterations
for zero results and exactly the requested positive count. All existing ledger
assertions are unchanged. This correction touched no production script.

Both complete suites below were rerun, not just their previously passing
selectors. Every command exited 0:

```sh
bash examples/service-kind-vm-workloads-v2/test-scheduler.sh
bash verification/harness/test-e09-v2-runner.sh
bash -n examples/service-kind-vm-workloads-v2/test-scheduler.sh verification/harness/test-e09-v2-runner.sh
shellcheck examples/service-kind-vm-workloads-v2/test-scheduler.sh verification/harness/test-e09-v2-runner.sh
git diff --check
```

The scheduler suite reports `HOST-SAFE scheduler tests PASS`, covering
capacity, invalid capacity, manifest, partial ledger (0/1/3 results), and both
ten-worker cohorts. The runner suite reports `HOST-SAFE runner tests PASS`,
covering valid twenty-row input, 19/21/100-row rejection, failed row, split
identity, and independently owned remote timeout with retained partial output.
It drives the active `E09-v2-vm-service-tcp-truthfulness-20` runner; the active
contract is twenty pairs, concurrency ten, 600s remote setup/trials + 60s
cleanup grace, with a 720s transport bound. These are synthetic orchestration
results, not native acceptance evidence.

Native twenty-pair execution, throughput, true guest health, real resource
reclamation, and deadline behavior with the actual product remain unverified
until separately authorized native execution and independent evidence review.
Initial dirty work, failed DES history, and old captures are preserved. No
commit, staging, DES phase event, or mutation run occurred.
