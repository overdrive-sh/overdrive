# Independent review — E09-v2 functional acceptance budget amendment

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Review ID | `roadmap_rev_20260908_e09v2_functional_budget_iteration_1` |
| Reviewer | Codex, `nw-software-crafter-reviewer` |
| Model | GPT-5.6 Luna, maximum thinking (inherited session model) |
| Review date | 2026-09-08 |
| Reviewed baseline | HEAD `c28a5f94da5fc2a5823aeaa2fa4cdc5173dc976b` |
| Dirty-work baseline | `.context/e09-v2-budget-baseline.AY6pgM/initial.diff`, `initial-status.txt`, and `historical-evidence.sha256` |
| Amendment | `roadmap-amendment-e09-v2-functional-budget.md` and the current E09-v2/roadmap/DISTILL delta |
| Native state | Pending; no native-metal run, lease, SSH change, or production completion is claimed |
| Final verdict | **APPROVED** for this bounded roadmap amendment only |

## Review mandate and authority

This is an independent review of the user-approved E09-v2 functional
acceptance amendment, not execution or completion of DELIVER step `02-04`.
The contract reviewed is exactly:

- twenty uniquely identified healthy/failure pairs;
- two ten-pair cohorts with effective concurrency `10`;
- one default-feature product build, one reusable-artifact preparation, and
  one persistent control-plane process;
- no pair retry, replacement, or discard, with partial outcomes retained;
- a remote owner budget of 600 seconds for setup and trials, followed by a
  bounded 60-second cleanup grace; and
- a separately bounded 720-second local transport wait so the nominal remote
  windows are not cut off by the local SSH wrapper.

The amendment is functional acceptance only. It does not claim native
reliability, capacity, throughput, a longer soak, a rolling worker pool, a new
Rust/probe/reclamation/lifecycle mechanism, or a new public CLI or environment
API. The optional longer soak remains non-mandatory.

## Baseline and preservation audit

The review compared the current tree with HEAD and the recorded dirty baseline.
The baseline already contained the E09-v2 resource-wait/terminal-observation
changes, the former 360-second local runner timeout, failed `02-04` execution
events, and the untracked timing diagnosis. Those are not treated as newly
authored amendment defects. The old E09-v2 `truthfulness-100` directory now
contains only its historical `evidence/` capture; the active runnable surface is
`truthfulness-20`.

The historical capture check was run with:

```sh
sha256sum -c .context/e09-v2-budget-baseline.AY6pgM/historical-evidence.sha256
```

All six recorded evidence files returned `OK`. The checker also reports one
improperly formatted comment/blank line in the checksum manifest; that does
not indicate a capture mutation. The legacy evidence remains untouched and is
not a second runnable E09-v2 expectation.

The current `execution-log.json` retains the baseline failed `02-04` entries.
No amendment DES event, GREEN claim, COMMIT, native status, or step-completion
rewrite was introduced by this review.

## Contract assessment

| Contract surface | Evidence and owner path | Result |
|---|---|---|
| Exact cardinality and identities | `run-example.sh:28-29,512-543` materializes exactly 20 pair identities, 40 Service/peer inputs, and four checked-in-derived specs per pair. `:1107-1125` aggregates exactly 20 ordered ledger rows. `test-scheduler.sh:67-98` checks the complete deterministic identity sequence. | Pass |
| Effective concurrency 10 | `run-example.sh:546-562` accepts the exact requested value `10`, rejects malformed or other values, and records positive `nproc`/`MemTotal` observations. `:1316-1330` derives two cohorts of ten. The old half-CPU/four-worker cap is gone; no capacity claim is substituted for the observation. | Pass |
| One build, preparation, and control plane | `run-example.sh:1283-1313` invokes the product build once, reusable preparation once, and `start_serve` once. `:1193-1207` requires one PID/start-tick identity among successful rows; `:100-113` checks that identity during the owner path. | Pass |
| Healthy/failure truthfulness | `run-example.sh:859-990` drives the public deploy, describe, peer-Job, and stop boundaries. Healthy requires Stable, the guest TCP startup witness, passing readiness, and the peer Job's success. Failure requires nonzero deploy, the typed startup failure at guest port `18999`, no Stable event, and the negative peer Job's unreachable result. The checked-in client enforces the exact `SVM-E08-GUEST-OK` body at `examples/service-kind-vm-workloads/client.rs:7-76`. | Pass |
| No retry/replacement/discard at pair scope | The cohort loop only advances after a joined cohort and breaks on failure (`run-example.sh:1037-1100,1316-1337`). Missing results become `not-run-cancelled` rows rather than favorable success (`:1110-1124`). The documented replacement-start trajectory is an existing allocation lifecycle observation, accepted only with the required typed failure evidence; it is not a replacement of the input pair. | Pass |
| Timeout ownership and partial evidence | The active runner fixes `600s` remote setup/trials, `60s` remote cleanup grace, and `720s` transport (`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh:7-35`). The example handles cancellation, emits reports before ordinary materialization cleanup, bounds cancellation worker/preparer work, and stops the control-plane process (`run-example.sh:1210-1263`). The runner extracts the ledger before validating the exit status (`runner.sh:46-56`). | Pass |
| Selector and historical boundary | `verification/harness/run-expectation.sh:28-42` explicitly resolves `E09-v2` to `truthfulness-20` and ignores the evidence-only `truthfulness-100` sibling. The active index and S-SVM-25 mapping point to `tcp-truthfulness-20`; E09-v1 remains separately mapped. | Pass |
| Scope and public surface | The amendment diff contains no Rust, Cargo, probe, lifecycle, reclamation, or public API changes. The new selector and shell constants are the requested operator/expectation surfaces; no soak mode, rolling pool, or generic cleanup protocol was added. | Pass |

The 600-second timer begins around the remote example command after transport
bootstrap; the 720-second wrapper is a nominal 600 + 60 remote-window sum plus
60 seconds of transport margin. The documents state that boundary accurately
and do not turn the margin into a native duration or performance promise.

## Findings, reachability, and dispositions

No blocking, critical, high, medium, or low correctness finding was accepted.
The reviewed implementation and its host-safe tests satisfy the bounded
amendment without requiring a new owner, persistence mechanism, scheduler
protocol, recovery path, or public surface.

One roadmap-validator observation was recorded and dispositioned explicitly:
the amended timeout criterion is 40 words, producing a
`CRITERIA_TOO_LONG` warning against the 30-word local concision threshold.
The roadmap remains below its total nine-step limit, the criterion is a single
unambiguous user-authorized timeout contract, and the validator exits zero.
This is a non-blocking presentation warning, not a correctness or scope
failure; no unrelated roadmap rewrite is required.

No theoretical cancellation trace, forced test-only abort, private synthetic
state, or unseeded control-plane ordering hypothesis was promoted to a finding.
Native guest-health, kernel-resource, cleanup, and throughput behavior remains
an explicitly pending evidence layer rather than an inferred result.

## Test integrity and Contract Shape Compliance

The amendment is marked `EXEMPT FROM PARADIGM` in the roadmap because its
acceptance boundary is an operator-runnable shell example and a black-box
expectation runner. There are no new Rust unit or property tests. Counting the
single S-SVM-25 acceptance behavior would yield a conventional unit-test budget
of `2 x 1 = 2`; actual unit tests are zero, and the three host-safe shell
regression suites are boundary checks rather than unit tests subject to that
budget. No budget violation is present.

The new shell test functions carry `CONTRACT_SHAPE: bounded-change` declarations
at `examples/service-kind-vm-workloads-v2/test-scheduler.sh:26,47,67,100,145`
and `verification/harness/test-e09-v2-runner.sh:108,158`. The S-SVM-25
acceptance scenario carries `@contract-shape:bounded-change`. No source-local
pure-function Rust property was added, so the exact Rustdoc declaration
requirement is not applicable. The tests retain substantive failure assertions;
the reported macOS `seq 1 0` issue was repaired in the private zero-result
fixture without weakening the partial-ledger assertions.

The scheduler test loads the actual example functions and substitutes only
platform/resource adapters and surrogate worker processes. The runner test
spies at the external cargo/transport boundary and uses scaled deadlines with
an independently session-owned TERM-resistant descendant. Neither test starts
the product binary, runs Cargo tests, imports an `overdrive-*` crate, acquires a
lease, or writes native expectation evidence. This is an honest orchestration
test boundary, not native product evidence or source-assertion theater.

## Independent verification performed

| Command | Result |
|---|---|
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh` | PASS — capacity, malformed config, 20-pair materialization, partial ledger, and two ten-worker cohorts |
| `bash verification/harness/test-e09-v2-runner.sh` | PASS — valid 20-row input, wrong counts, failed row, split identity, timeout, TERM, descendant cleanup, and partial output |
| `bash verification/harness/test-run-expectation.sh` | PASS — selector/evidence harness branch fixtures; no expectation/native run |
| `bash -n` on the v2 example, preparer, active runner, selector harness, and both host-safe tests | PASS |
| `shellcheck` on the v2 example, preparer, active runner, selector harness, and both host-safe tests | PASS |
| `jq -e . docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | PASS |
| `jq` structural checks for nine steps, `02-04`, pending pre-review state, and S-SVM-25's active mapping | PASS |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | PASS — `VALID: 3 phases, 9 steps`; one non-blocking 40-word criterion warning |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.verify_deliver_integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | PASS — roadmap format/validator has no errors; execution log intentionally skipped |
| `git diff --check` | PASS |
| `sha256sum -c .context/e09-v2-budget-baseline.AY6pgM/historical-evidence.sha256` | PASS — all six historical files `OK`; manifest comment warning noted above |

No native-metal expectation, Cargo build, SSH command, lease acquisition,
mutation run, or production binary was run for this review.

## Limitations and approval boundary

The host-safe suites prove source-backed shell orchestration, exact configured
counts, process-boundary timeout behavior, and partial-ledger mechanics. They
do not prove native VM guest health, ten-worker native capacity, throughput,
real netns/veth/TAP/BPF/nftables/cgroup cleanup, or the actual remote owner
under SSH. Those remain the later native expectation and independent evidence
review obligations. The old 100-pair capture is historical evidence only.

This approval authorizes the exact E09-v2 roadmap amendment for the next bounded
workflow boundary. It does not mark `02-04` complete, does not stamp native
verification, does not alter failed DES history, and does not authorize the
wave-level mutation gate.

## Verdict

**APPROVED.** The amended E09-v2 contract is internally consistent across the
roadmap, DISTILL scenario, operator example, expectation runner, selector, and
host-safe verification. Exact cardinality, effective concurrency, one-build /
one-preparation /
one-control-plane ownership, no pair discard/retry semantics, remote 600 + 60
timeout ownership, 720 transport framing, partial evidence, historical capture
preservation, and scope boundaries are all represented without a proven
reachable defect. Native verification remains pending and is not implied by
this verdict.
