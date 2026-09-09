# E11 Evidence Audit — Different-Fox Review

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Roadmap step | `03-01` — E11 readiness recovery |
| Expectation | `E11-vm-service-readiness-traffic-recovery` |
| Auditor | Independent evidence-only auditor (Codex) |
| Audit date | `2026-09-09T16:10:17Z` |
| Declared substrate | `native-metal` |
| Final capture date | `2026-09-09T11:02:58Z` (`verification.yaml`) |
| Final capture SHA | `485c7ca3fdb15ffe4e449420049de32910212b16` |
| Capture state | `working_tree_dirty: true`; runner exit `0` |
| Verdict | **NEEDS_RECAPTURE** |

The separate implementation review in
`docs/feature/service-kind-vm-workloads/deliver/review-03-01.md` records an
iteration-3 **APPROVED** verdict. That establishes the implementation gate
only; it is not used as proof of any E11 evidence claim.

## Audit-only boundary

This audit examined the approved `03-01` roadmap contract, the E11 README and
runner as the black-box assertion anchor, the verification catalogue rules,
the final receipt and product transcript, the extracted ledger, provenance
records, and every E11 attempt retained in the checkout. The implementation
review was read only to establish its final review status.

No production or test implementation, source diff, or generated crate was
read for correctness. No Rust test, Cargo test/check/build command,
expectation harness, or crate import/link was run. Mutation testing was not
run, as explicitly directed. The retained dirty patch was treated as an
identity/provenance artifact and hashed without using it as implementation
evidence. Only this Markdown artifact is written; the README, INDEX, receipt,
ledger, DES log, staging area, and commits are unchanged.

## Contract and anchors

The approved `03-01` entry in `deliver/roadmap.json` requires E11 to show one
replica moving from ready to unready and back to ready, with each transition
within two seconds, the same VM remaining `Running` and `Stable` without a
restart, no guest reply during the failed window, and zero cleanup delta. The
same entry requires the E11 harness evidence and independent audit.

The E11 README defines the public journey as `Pass -> Fail -> Pass` and names
the exact peer Job reply `SVM-E08-GUEST-OK`. Its anchors are:

- `S-SVM-27A`, `S-SVM-27B`, and `S-SVM-27C` for before, unavailable, and
  recovered peer Jobs;
- `US-SVM-3` and `K3` in `feature-delta.md`; and
- accepted lifecycle gate ownership.

The anchors are present in `evidence/verification.yaml:13-14` and predate
this audit. The roadmap implementation note names the peer VM Job as the
E11 oracle; this audit therefore uses the captured Job verdicts and the
runner's explicit `peer_result` values, without inventing a wire-capture
requirement.

## Evidence inventory

| Artifact | Role and observed disposition |
|---|---|
| `execution-substrate` | Declares `native-metal` (`:1`). |
| `evidence/verification.yaml` | Receipt: E11, seed `25717`, product/harness SHA, dirty state, harness invocation, native-metal status, runner invoked, and exit `0` (`:1-16`). It deliberately does not self-stamp `satisfied`. |
| `evidence/product-run.meta` | Final invocation, native-metal example command, remote journey/cleanup budgets, UTC start/finish, and exit `0` (`:1-9`). |
| `evidence/product-run.out` | Complete redacted native-metal transcript: metal lease and preflight, materialization, built product deployment, three peer Jobs, three Service descriptions, ledger, PASS marker, and teardown (`:1-151`). |
| `evidence/readiness-recovery.tsv` | Three-row extracted ledger with the exact header and all phase fields (`:1-4`). |
| `evidence/run.log` | Retained runner log; it contains the final ledger/output and the remote invocation (`:1-153`). |
| `evidence/dirty-status.txt` | Captures the dirty source/evidence state at the run (`:1-26`). |
| `evidence/dirty-diff.patch` | Retained dirty-state provenance only; SHA-256 is `0fd36f536f4789fa9104fa58731dfcff0bc5eed1c05206d873424fabb25afc68`. Its implementation contents were not used in this audit. |
| `README.md`, `runner.sh` | Contract and executable black-box predicate; SHA-256 values are recorded below. |
| Separate attempt directories | None exist under the E11 expectation directory. This absence is material to the attempt-history finding below. |

Read-only SHA-256 identities of the non-patch audit inputs are:

| File | SHA-256 |
|---|---|
| `README.md` | `795258b84a510a5310396f9921deac56499071bce421d456392e58f6724d9f44` |
| `runner.sh` | `7b0bd12efe5324bc751e66498665c7aee79357187ff17841e0e9e8895d97d5a6` |
| `execution-substrate` | `23855df3e8416fed3a0ddb2cc85c650edf13c10f12c41346efdb039825c7ef17` |
| `verification.yaml` | `8d7059a73868417c006b73a4955f5201c7f68571cc90f347e9b22dee1105172c` |
| `product-run.meta` | `a44a7386290304a02b5fe233bc33c3cf07393bb8a395d39e9ca097ea6b4e5dbe` |
| `product-run.out` | `310d477b8920c8728abd3312d46d70a71b456fbbe80dd33d07ade09e690025bc` |
| `readiness-recovery.tsv` | `22ac3255fed678372e3e998da493d8fed2f852614a2eeaf62327ff481acf7af4` |
| `run.log` | `657a15559e096d8a97365916fe37a2b295974bfc89a90afe597c64956d54c7fa` |
| `dirty-status.txt` | `7d15b4ff13ac5b316674d48f0ff919210d4e03da5dccdd59e90da2784fdc8d66` |

## Provenance and execution actuality

The receipt records `runner_invoked: true`, `execution_status: "succeeded"`,
`execution_substrate: "native-metal"`, `executed_in_lima: false`, seed
`25717`, runner exit `0`, and matching product/harness SHA
`485c7ca3fdb15ffe4e449420049de32910212b16` (`verification.yaml:1-14`). The
SHA resolves to a commit in the checkout. `working_tree_dirty: true` is not
hidden: the dirty status and dirty patch are retained, and the patch identity
is recorded above.

The invocation receipt names the checked-in
`examples/service-kind-vm-workloads/run-example.sh run readiness-recovery`
journey, a 1,200-second remote journey budget, 90-second cleanup grace, and a
1,350-second transport bound (`product-run.meta:1-9`). The transcript shows
the metal lease, fail-closed native x86_64/KVM preflight, runtime
materialization, and a direct invocation of the built
`/home/ubuntu/overdrive/target/debug/overdrive deploy ...` product binary
(`product-run.out:1-32`). It contains no Cargo test, Rust test binary,
in-process harness, or crate import. This is the built default-feature
product path declared by the E11 expectation, not a test process standing in
for the product.

The final transcript and extracted ledger agree. The observed-to-detected
latencies are `330 ms`, `197 ms`, and `167 ms`; the first-to-second and
second-to-third observation gaps are `9,023 ms` and `10,026 ms`, consistent
with the fixed readiness phases. The final output ends with the exact zero
cleanup delta for VM, probe, network, cgroup, run-directory, mount, loop, and
preparation resources (`product-run.out:148-151`).

## Claim-by-claim audit

### 1. Readiness is `Pass -> Fail -> Pass` on one Service

**Result: SATISFIED.**

The three public Service descriptions show readiness `last=pass`, then
`last=fail (HTTP 503)`, then `last=pass` (`product-run.out:50-52,86-88,122-124`).
The ledger independently preserves the same ordered states
(`readiness-recovery.tsv:2-4`), with one `before`, one `during`, and one
`after` row. The phase gaps also match the intended ten-second journey rather
than an unordered collection of snapshots.

### 2. The same allocation remains `Running` with zero restarts

**Result: SATISFIED.**

All three public Service snapshots identify
`alloc-service-vm-readiness-recovery-0` and show `Running` with `Restarts 0`
(`product-run.out:36-40,72-76,108-112`). The guest address, certificate
serial, and startup observation remain the same across those snapshots. The
ledger independently repeats `lifecycle=Running` and `restarts=0` for every
phase (`readiness-recovery.tsv:2-4`), and the runner rejects any other
combination (`runner.sh:62-70`). This is direct same-allocation evidence, not
a conclusion from the final state alone.

### 3. Stable remains unchanged after both readiness transitions

**Result: NEEDS_RECAPTURE for the full roadmap contract.**

The transcript records the initial Stable outcome (`product-run.out:30`), but
the later public descriptions do not render a post-during or post-after
Stable state; they render the allocation's `Running` state and probe rows
only (`:69-88,105-124`). The ledger and runner likewise carry `Running`, not
an independent Stable field (`readiness-recovery.tsv:1-4`; `runner.sh:57-75`).
Running and Stable are distinct lifecycle promises. The captured output proves
the requested Running/zero-restart claim, but it does not directly prove the
stricter README/roadmap statement that Stable remained unchanged throughout.

### 4. Before and after peer Jobs receive the exact reply

**Result: SATISFIED under the approved peer-Job oracle.**

The transcript contains a successful before Job and a successful after Job
(`product-run.out:53-68,125-140`), and the ledger records
`peer_result=exact-reply` for both (`readiness-recovery.tsv:2,4`). The runner
requires those exact phase-specific values (`runner.sh:68-70`) and emits the
E11 PASS line only after all peer predicates pass (`product-run.out:147`).
The approved roadmap note explicitly makes the peer VM Job the E11 oracle;
the audit therefore does not demand a second direct host request or a new
wire-capture surface. The literal guest sentinel is not repeated in the
transcript, but the executed peer Job outcome and the runner's
`exact-reply` field are the contract-defined black-box oracle.

### 5. No guest reply occurs during the failed window

**Result: SATISFIED under the approved negative-control oracle.**

The during-window Job reaches `Verdict: Succeeded` and exits normally
(`product-run.out:89-104`), while its ledger row is
`unreachable-no-exact-reply` (`readiness-recovery.tsv:3`). The runner requires
that exact negative-control result for the `during` phase and rejects a
successful exact-reply label (`runner.sh:62-70`). This is the bounded peer Job
oracle required by E11, not an assertion inferred from the readiness row.

### 6. Both transitions complete within the accepted two-second bound

**Result: SATISFIED.**

The ledger reports `330 ms` for before, `197 ms` for the withdrawal, and
`167 ms` for recovery (`readiness-recovery.tsv:2-4`). All three are below the
runner's `2000 ms` predicate (`runner.sh:65-67`). The measured
readiness-transition rows are therefore `2/2` within the accepted bound, as
also stated by the executed PASS line (`product-run.out:147`).

### 7. Teardown leaves zero cleanup delta

**Result: SATISFIED.**

The product transcript records
`vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0`
after stopping the Service (`product-run.out:148-151`). The runner requires
that complete line verbatim (`runner.sh:81-83`).

### 8. The evidence is a native built-product execution, not a test

**Result: SATISFIED.**

The receipt declares `native-metal`, `executed_in_lima: false`, and a
succeeded runner. The invocation and transcript show the example driving
`target/debug/overdrive` through `deploy`, followed by public `describe`
operations and peer Jobs (`verification.yaml:7-12`; `product-run.meta:2-9`;
`product-run.out:1-32`). No test runner or Overdrive crate boundary appears
in the captured execution.

## Attempt disposition and retention

The E11 evidence directory contains only one final set of receipt, metadata,
transcript, ledger, log, dirty-status, and dirty-patch files. There are no
`attempt-*` directories. A read-only history-name inspection likewise found
no historical E11 attempt artifact under any other path.

The DES log records a materially different earlier native E11 attempt at
`2026-09-09T09:12:09Z`: the readiness withdrawal did not remove traffic, the
during Job received the exact guest reply, and the commit was correctly
withheld (`deliver/execution-log.json:279-290`). The later DES record reports a
successful run at `11:05:56Z` (`:300-310`), but the earlier receipt, complete
product output, ledger, and runner log are not retained in the E11 evidence
catalogue. A DES prose record is provenance of an attempted outcome; it is not
the executable receipt/output required to independently audit that outcome.

| Attempt | Retained material | Disposition |
|---|---|---|
| Earlier native failure (`09:12:09Z`) | DES narrative only; no E11 attempt receipt/output/ledger directory | **Not independently auditable; retention gap.** |
| Final native capture (`11:02:58Z` start, `11:03:51Z` receipt finish) | Complete receipt, metadata, transcript, ledger, log, dirty status, and patch identity | Mechanically qualifying for the claims above, but the full contract still lacks post-transition Stable proof. |

This is not a claim that the final run is false. It is a finding that the
failed attempt was not retained as executed evidence and that the final
capture does not expose the separate Stable state after the transitions.

## Non-findings and boundary decisions

- The private `Backend.healthy` field, observation wake path, and terminal
  invariant are not re-proven from E11's black-box evidence. They belong to the
  approved in-process/implementation evidence lane and were not used here.
- The implementation review's APPROVED verdict is not substituted for the
  evidence audit.
- The peer Job is used as the approved E11 traffic oracle. No direct
  `workload_addr` request, Rust test, or second traffic harness is required.
- `executed_in_lima: false` is correct for the declared native-metal
  substrate; this is not a missing Lima run.
- Dirty capture state is not itself a failure: the product/harness SHA, dirty
  status, and dirty-patch hash are retained. The limitation is that the
  capture is explicitly historical and dirty, not that it was silently
  presented as a clean commit.
- No mutation score or mutation exclusion was requested or evaluated.
- E11's README and `verification/expectations/INDEX.md` remain `pending`, as
  required until this audit reaches an unambiguous satisfied verdict. They
  were not changed by this audit.

## Read-only commands used

The audit used read-only inspection only: targeted `rg --files`/`rg` lookups;
`sed`/`nl` reads of the roadmap, E11 README and runner, verification rules,
receipt, metadata, transcript, ledger, logs, status, DES attempt chronology,
and implementation-review verdict; `find`/`wc` for the E11 evidence inventory;
`shasum -a 256` for evidence and dirty-patch identities; and `git status`,
`git cat-file`, `git show -s`, and `git log --name-only` for checkout/commit
provenance. No Cargo command, expectation harness, test command, source
implementation read, mutation command, staging operation, or commit was
performed.

## Verdict and exact documentation action

**NEEDS_RECAPTURE.** The final native-metal receipt is real, anchored, SHA- and
dirty-state-pinned, and directly proves `Pass -> Fail -> Pass`, same
allocation `Running/0`, the two transition bounds, the peer Job oracle
outcomes, and zero cleanup delta. It is not sufficient to close the full
approved E11 evidence contract because (1) the earlier failed native attempt
is represented only by DES prose rather than retained executable artifacts,
and (2) Stable is shown only before the transitions, not after each
transition.

Keep E11's README and INDEX entries at `pending`. Before another evidence
audit, retain the earlier failed attempt (receipt, complete product output,
ledger, runner log, dirty status, and provenance) under a timestamped
`evidence/attempt-<timestamp>/` directory, or explicitly record that the
original raw capture is unrecoverable and preserve the failure as an excluded
attempt rather than silently dropping it. Capture a fresh native-metal final
run whose public transcript or ledger includes an explicit post-withdrawal and
post-recovery Stable observation while preserving the existing peer Job,
transition-bound, same-allocation, and zero-delta receipts. Then dispatch a
new independent evidence audit. Only after that audit returns **SATISFIED**
should documentation change the E11 README status to `satisfied` and update
the matching `verification/expectations/INDEX.md` row with a link to the
approved audit; do not alter the existing receipt, final transcript, or DES
history as part of that status update.
