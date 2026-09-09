# E11 Evidence Audit — Independent Re-audit

## Metadata

| Field | Value |
|---|---|
| Feature | service-kind-vm-workloads |
| Roadmap step | 03-01 — E11 readiness recovery |
| Expectation | E11-vm-service-readiness-traffic-recovery |
| Auditor | Fresh independent evidence-only auditor (Codex) |
| Model | User-selected GPT 5.6 Luna, maximum thinking |
| Audit date | 2026-09-09T18:53:34Z |
| Audit target | bb52713ee429443b04b153580832fad022fc6307 |
| Audit target parent | f1b4e900cff8b3aeab4c934b89ce720911a9dd8e |
| Declared substrate | native-metal |
| Current capture | evidence/attempt-20260909T181642Z |
| Capture timestamp | 2026-09-09T18:15:25Z (verification.yaml) |
| Captured source identity | f1b4e900cff8b3aeab4c934b89ce720911a9dd8e with working_tree_dirty: true |
| Captured runner result | native-metal, executed_in_lima: false, runner exit 0 |
| Prior evidence-audit verdict | NEEDS_RECAPTURE (2026-09-09T16:10:17Z) |
| This re-audit verdict | **SATISFIED** |

The audit target is the implementation commit bb52713ee429443b04b153580832fad022fc6307.
The native capture was made from its parent source identity with an explicitly
retained dirty tree and dirty patch. This artifact does not represent that
capture as a clean-commit run.

## Audit boundary and independence

This re-audit independently examined the approved 03-01 roadmap contract, the
current E11 README and runner, the expectations INDEX, the current canonical
evidence, the complete fresh attempt at
evidence/attempt-20260909T181642Z, the archived former-success attempt at
evidence/attempt-20260909T110258Z, the earlier attempt and its recapture
blocker, the excluded unrecoverable attempt, the operator transcript, ledger,
receipt, metadata, runner log, dirty status, dirty patch, the implementation
review, and the approved Stable-observation amendment and its DESIGN review.

The implementation review and DESIGN review establish separate gates; neither
is substituted for this evidence verdict. This audit uses the public
workload-describe and VM-client Job surfaces only. It makes no independent
claim about private Backend.healthy state, persistence internals, reconciler
ownership, wake ordering, or any other unobserved internal behavior.

No production or test implementation was changed. No evidence file, runner,
README, INDEX, DES log, roadmap, staging area, or commit was changed. No Cargo
command, Rust test, expectation execution, mutation command, or crate import
was run by this auditor. The only write is this Markdown review artifact.

## Contract and authoritative anchors

The 03-01 roadmap entry at
docs/feature/service-kind-vm-workloads/deliver/roadmap.json:315-338 requires:

1. readiness Pass to Fail to Pass on one replica;
2. each readiness transition within two seconds;
3. the same VM remaining Running and Stable without a restart;
4. no guest reply during the failed window;
5. zero teardown cleanup delta; and
6. an E11 harness capture plus independent audit.

The current E11 README at lines 10-47 pins the public journey, the exact
SVM-E08-GUEST-OK peer reply, the 11-column ledger, a direct current-row
terminal: Stable line in each of the three describe responses, the exact
peer-result values, the no-restart condition, the two-second bound, and the
zero-delta requirement. Its anchors are S-SVM-27A, S-SVM-27B, S-SVM-27C,
US-SVM-3, K3, and accepted lifecycle gate ownership.

The corresponding scenarios at
docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:124-160 and
the feature contract at feature-delta.md:170-212 and 955-957 define the
before, unavailable, and recovered peer VM client Jobs. The feature contract
also explicitly makes a checked-in plaintext VM client through the Service
frontend the traffic oracle and rejects a direct workload_addr request as a
substitute.

The approved Stable-observation amendment requires the terminal value to come
from the same public describe response as that phase's readiness result. Its
fresh-capture obligations are at
docs/feature/service-kind-vm-workloads/design/amendment-e11-stable-observation.md:355-389.
The independent DESIGN review records APPROVED for that amendment and
explicitly keeps the evidence audit separate at
docs/feature/service-kind-vm-workloads/design/review-amendment-e11-stable-observation.md:292-322.

## Evidence inventory and identity

The canonical evidence files at the E11 expectation root are byte-identical
to the corresponding files in attempt-20260909T181642Z. They are therefore
one retained capture, not two independent executions.

| Artifact | Evidence role and audit observation |
|---|---|
| execution-substrate | Contains native-metal at line 1. SHA-256 23855df3e8416fed3a0ddb2cc85c650edf13c10f12c41346efdb039825c7ef17. |
| attempt verification.yaml | E11 receipt, capture timestamp, source and harness identity, seed 25717, dirty-state disclosure, native-metal status, runner invocation, and exit 0 at lines 1-16. |
| attempt product-run.meta | Exact example command, remote command and timeout budgets, output root, start/finish timestamps, and exit 0 at lines 1-9. |
| attempt product-run.out | Complete redacted native operator transcript at lines 1-156: metal lease and preflight, synchronization, product build, deployment, public describes, peer Jobs, ledger, PASS marker, and teardown. |
| attempt readiness-recovery.tsv | Extracted four-line ledger with the pinned header and three phase rows. |
| attempt run.log | Retained runner log at lines 1-158. It starts with two orphan during/after ledger rows, then records the complete current invocation and output from line 3 through line 158. It is supporting log evidence, not the sole authoritative transcript. |
| attempt dirty-status.txt | Source and evidence working-tree state captured at lines 1-21. |
| attempt dirty-diff.patch | Retained dirty-source provenance. SHA-256 410f8684203a3de070ba9ad914d37ec0ec67364041d90959fa5e6704d187fe4f. |
| current README | Contract and status remained pending during this audit. SHA-256 3a30d2ff1fd687afc9936e585f2a00905501d73dd1e5205a02df1bb4692f7f4. |
| current runner | Executable black-box predicates. SHA-256 dba1a804b915831f0c7c6788991535dac99a2ac1025f912bed0900d153d56f584. |
| expectations INDEX | E11 status remained pending during this audit. SHA-256 04d8bf02dae02da16f4dc99f032331476d6de98102aaf31b078a7c56babf4142. |

The fresh attempt's principal SHA-256 identities are:

| File | SHA-256 |
|---|---|
| verification.yaml | f358fd78027bd2ff9db9f2e15e237bc3b1c768053492a41d0b68b69098106117 |
| product-run.meta | 44bb9c1924fe8040c40a0206c62da4cca42892684001782ca9791bb005689b94 |
| product-run.out | c8632f79eef1e4b205bd2cc55890a3f9ba65d298364b4cbc08edcdb4fbdf98f3 |
| readiness-recovery.tsv | 4a8454b26f264259f61422542ace0a05157a2c20a28b539ddee1513b05acd349 |
| run.log | 662ff36210c3bdc852957570c550b643e28c4cbe49481640c122bbde9450b8a6 |
| dirty-status.txt | 95b21a1171244305181bc82b489b94f9106b81892c25f00f914945b7df7b7bcf |

The canonical root copies have the same identities. The excluded historical
disposition has SHA-256 c236878397a0bb39b341e74cb137614c5b71d7ffd5f085bf38f58c467aecd4bd.
The prior recapture blocker has SHA-256
350bbf07556e78d45ec6b964f0acde858b3628f6d6b9b1a2f0528ff071e08400.

## Native built-product provenance

The fresh receipt at
evidence/attempt-20260909T181642Z/verification.yaml:1-16 records:

- expectation E11 and fixed seed 25717;
- source and harness SHA f1b4e900cff8b3aeab4c934b89ce720911a9dd8e;
- working_tree_dirty: true;
- harness invocation verification/harness/run-expectation.sh E11;
- execution_substrate native-metal;
- executed_in_lima: false;
- runner_invoked: true; and
- execution_status succeeded with runner_exit_code 0.

The dirty state is truthful and complete enough to identify the source used for
the run: dirty-status.txt retains the runtime, example, runner, evidence,
documentation, and untracked amendment paths present at capture, and
dirty-diff.patch retains the corresponding source-state patch. The receipt
does not claim that a clean bb527 binary was executed. Commit bb527 is the
follow-on commit whose parent is exactly the receipt's f1b4 source identity;
the implementation review at review-03-01.md:705-718 separately approves that
commit and explicitly leaves this evidence audit as the next gate.

The metadata at
evidence/attempt-20260909T181642Z/product-run.meta:1-9 names the checked-in
readiness-recovery example, a 1,200-second remote journey, 90-second cleanup
grace, and 1,350-second transport bound. The operator transcript shows:

- the remote native-metal lease and fail-closed x86_64/KVM preflight at
  product-run.out:1-18;
- remote compilation of overdrive-control-plane and overdrive-cli at
  lines 19-21;
- runtime materialization and verification at lines 22-23; and
- direct execution of /home/ubuntu/overdrive/target/debug/overdrive deploy
  against the checked-in readiness-recovery specification at lines 24-34.

This is a native built-product boundary. The transcript contains no Rust test
binary, in-process test harness, crate import, or expectation runner standing
in for the product. The build and remote transport are part of the captured
operator journey; the product outcome is observed through public deploy,
workload describe, and VM client Job surfaces.

## Direct public observations

The direct current-row Stable field is present in the same public Service
describe response as each readiness observation:

| Phase | Public Service response | Direct observations | Ledger row |
|---|---|---|---|
| before | product-run.out:35-55; terminal line at :40 | alloc-service-vm-readiness-recovery-0, Running, Restarts 0, terminal: Stable, readiness pass at observed_at_ms 1788977745795 | before, pass, Stable, latency 359 ms, Running, 0, exact-reply |
| during | product-run.out:72-92; terminal line at :77 | the same allocation, Running, Restarts 0, terminal: Stable, readiness fail (HTTP 503) at observed_at_ms 1788977754820 | during, fail, Stable, latency 222 ms, Running, 0, unreachable-no-exact-reply |
| after | product-run.out:109-129; terminal line at :114 | the same allocation, Running, Restarts 0, terminal: Stable, readiness pass at observed_at_ms 1788977764847 | after, pass, Stable, latency 199 ms, Running, 0, exact-reply |

The ledger at
evidence/attempt-20260909T181642Z/readiness-recovery.tsv:1-4 has exactly
this pinned header:

    phase  readiness  terminal  observed_at_ms  detected_at_ms  transition_latency_ms  client_started_at_ms  client_elapsed_ms  lifecycle  restarts  peer_result

The actual file uses tab separators, and its three rows are:

    before  pass  Stable  1788977745795  1788977746154  359  1788977746174  2453  Running  0  exact-reply
    during  fail  Stable  1788977754820  1788977755042  222  1788977755064  3567  Running  0  unreachable-no-exact-reply
    after   pass  Stable  1788977764847  1788977765046  199  1788977765067  2450  Running  0  exact-reply

The initial submit-stream Stable line at product-run.out:32 is not counted as
the E11 terminal observation. The three counted Stable values are the direct
current-row lines in the three later public describe responses. The checked-in
example obtains each phase's terminal value from that response through
run-example.sh:282-290, requires the expected readiness and Stable line in the
same response at :355-388, saves the per-phase values at :595-649, and emits
them at :678-689. This is the approved existing describe surface; no initial
stream carry-forward, private row read, readiness-derived boolean, or
reconstructed ledger value is being used.

## Claim-by-claim evidence disposition

### 1. Readiness is Pass to Fail to Pass

**Result: SATISFIED.**

The three public Service responses show readiness last=pass, then
last=fail (HTTP 503), then last=pass at product-run.out:54-55, :91-92,
and :128-129. The extracted ledger independently records exactly one before
pass, one during fail, and one after pass at readiness-recovery.tsv:2-4.
The observed timestamps preserve the expected ordering:
1788977745795, 1788977754820, and 1788977764847.

### 2. The same allocation remains Running with zero restarts

**Result: SATISFIED.**

All three public snapshots show the allocation ID
alloc-service-vm-readiness-recovery-0 at product-run.out:38-40, :75-77,
and :112-114. Each row is Running with Restarts 0. The public address and
certificate serial also remain unchanged in the corresponding responses. The
ledger repeats lifecycle Running and restarts 0 for all three phases, and the
runner rejects any other lifecycle or restart values at runner.sh:62-78.

### 3. Stable remains unchanged before, during, and after readiness changes

**Result: SATISFIED.**

Each of the three phase-specific public describe responses contains the direct
current-row line terminal: Stable at product-run.out:40, :77, and :114. The
ledger's terminal cell is Stable in all three rows at readiness-recovery.tsv:2-4.
The runner requires the exact value in every row at runner.sh:57-78 and also
requires exactly three direct Stable lines in the transcript at :80-82.

The initial submit-stream Stable line at :32 is supporting startup context only.
The current evidence does not infer Stable from Running, readiness, a prior
terminal field, or an internal lifecycle claim.

### 4. Before and after peer Jobs receive the exact guest reply

**Result: SATISFIED under the approved peer-Job oracle.**

The before and after VM client Jobs are accepted and have public
Verdict: Succeeded at product-run.out:56-70 and :130-144. Each has a public
Terminated state with exit 0. The ledger records exact-reply for before and
after at readiness-recovery.tsv:2 and :4, and the runner requires those exact
phase-specific values at runner.sh:62-71.

The oracle is the checked-in VM client Job, not a direct host request. Its
public fixture arguments are expect-reply in
examples/service-kind-vm-workloads/client-readiness-before.toml:1-8 and
client-readiness-after.toml:1-8. The checked-in client implementation tests
for the byte-exact SVM-E08-GUEST-OK body before returning success at
examples/service-kind-vm-workloads/client.rs:7 and :55-64. The accepted
scenario and feature contract make this Job result the traffic oracle. No
unrequested wire capture or workload_addr call is required.

### 5. No exact guest reply occurs during the failed window

**Result: SATISFIED under the approved negative-control oracle.**

The during VM client Job is accepted, reaches public Verdict: Succeeded, and
has public Terminated exit 0 at product-run.out:93-107. Its fixture uses
expect-unreachable with a 1,500 ms bound at
examples/service-kind-vm-workloads/client-readiness-during.toml:1-8. The
checked-in client exits nonzero if it receives the exact guest body during
that window and returns success only after the window completes without that
body at client.rs:67-76. The ledger records
unreachable-no-exact-reply at readiness-recovery.tsv:3, and runner.sh:62-71
requires that exact negative-control value for the during phase.

The during Job's successful process verdict therefore means successful
negative-control completion, not a guest reply. This audit makes no stronger
wire-level claim than the approved VM Job oracle.

### 6. Readiness transitions meet the two-second bound

**Result: SATISFIED.**

The phase ledger reports 359 ms for the baseline observation, 222 ms for the
Pass-to-Fail withdrawal observation, and 199 ms for the Fail-to-Pass recovery
observation at readiness-recovery.tsv:2-4. The two actual transitions are
therefore 222 ms and 199 ms, both at most 2,000 ms. The runner checks the
numeric bound for each row at runner.sh:62-78, and the captured product output
records the resulting E11 PASS marker at product-run.out:152.

### 7. Teardown leaves zero cleanup delta

**Result: SATISFIED.**

After stopping the Service and removing the owned materialization, the product
transcript records the complete zero-delta line for VM, probe, network,
cgroup, run-directory, mount, loop, and preparation resources at
product-run.out:153-156. The runner requires that exact cleanup result and
the E11 PASS marker at runner.sh:84-90. This is direct captured teardown
evidence, not an assumption from process exit.

### 8. The evidence is a native built-product execution

**Result: SATISFIED.**

The receipt, metadata, metal lease/preflight transcript, remote build, and
direct target/debug/overdrive deployment establish the declared native-metal
built-product path. The receipt discloses dirty source state and retains its
patch and status records. The public outcomes are from the checked-in example
and product CLI. No test process or private implementation observation is
being substituted for the built product.

## Attempt history and retention

The retained attempt set is truthful and append-only. The canonical evidence
root is a byte-identical mirror of the fresh attempt and is not counted as a
separate run.

| Attempt | Retained artifacts and observed result | Disposition |
|---|---|---|
| attempt-20260909T091209Z | Only EXCLUDED.md is present. It records the 2026-09-09T09:12:09Z native-metal attempt, the readiness-withdrawal failure, the during-window exact guest reply, and missing raw receipt/transcript/ledger/log/status/patch at EXCLUDED.md:1-28. | **EXCLUDED_UNRECOVERABLE.** DES prose is retained as provenance only. No raw artifact was reconstructed and this attempt contributes no passing evidence. |
| attempt-20260909T110258Z | Full receipt, metadata, product output, old 10-column ledger, runner log, dirty status, and dirty patch are retained. The run exited 0 and showed Pass to Fail to Pass, Running/0, exact-reply / unreachable-no-exact-reply / exact-reply, and zero cleanup, but its three public describes did not contain a direct current terminal: Stable line and its ledger had no terminal column. | **RETAINED_HISTORICAL_PARTIAL.** This is the archived former mechanical success under an incomplete evidence surface, not a full-contract E11 pass. |
| attempt-20260909T164519Z | Full receipt, metadata, product output, old 10-column ledger, runner log, dirty status, dirty patch, and recapture-blocker.md are retained. The blocker records the same missing post-transition Stable observations at recapture-blocker.md:1-68. | **RETAINED_HISTORICAL_PARTIAL / BLOCKER-SUPERSEDED.** Its missing-Stable finding remains historical evidence; it is not silently overwritten or promoted. |
| attempt-20260909T181642Z | Full receipt, metadata, complete direct product transcript, exact 11-column ledger, runner log, dirty status, and dirty patch are retained. All current E11 predicates are directly present. | **CURRENT QUALIFYING ATTEMPT.** This is the sole attempt used for the satisfied evidence verdict. |

The prior independent evidence audit is preserved as the first audit iteration
in the history below. Its NEEDS_RECAPTURE verdict correctly identified both
the absent direct Stable observations and the then-unretained earlier failure.
The excluded disposition and the fresh direct observations close those two
historical findings without rewriting the old audit or DES history.

## Approved design and implementation gate disposition

The approved Stable-observation amendment selected the existing
workload-describe snapshot surface and required the exact public current-row
Stable observation in each phase. Its independent DESIGN review is APPROVED,
but explicitly did not approve implementation or evidence.

The implementation review at
docs/feature/service-kind-vm-workloads/deliver/review-03-01.md:705-948 is
APPROVED for commit bb52713ee429443b04b153580832fad022fc6307 and records the
exact API/rendering shape, test gate, dirty native recapture, retained attempt
inventory, DES results, and commit trailer. That review also explicitly
leaves this independent evidence audit as a separate gate. This re-audit
accepts those artifacts as authority for their stated gates only; it does not
turn implementation assertions into additional black-box observations.

## Findings and remediation dispositions

| Finding or observation | Severity | Status | Exact disposition |
|---|---:|---|---|
| Prior captures lacked direct current Stable observations after readiness changes. | Blocking at prior audit | **CLOSED** | The approved amendment and implementation added the approved existing public observation. The fresh attempt captures terminal: Stable in each corresponding describe response, extracts those values into the pinned ledger, and the runner requires all three. No further recapture is required. |
| The 09:12 failed attempt had no recoverable raw evidence. | Evidence-retention gap at prior audit | **CLOSED** | The attempt is explicitly retained as attempt-20260909T091209Z/EXCLUDED.md. Its failure is not reconstructed or counted as passing evidence. No raw artifact is invented. |
| run.log begins with two orphan during/after ledger rows before the current invocation. | Non-blocking evidence hygiene observation | **Recorded; no remediation** | Preserve the captured log unchanged. The complete authoritative product-run.out and extracted readiness-recovery.tsv begin with the current run, agree on all values, and are byte-identical between the canonical root and fresh attempt. Rewriting the retained log would damage attempt-history fidelity and is not required by the E11 contract. |

There is no open blocking finding. No production, test, persistence, broker,
recovery, lifecycle, or private-state remediation is proposed by this audit.

## Read-only verification performed

The audit performed targeted read-only file inspection with nl, sed, rg, find,
wc, cmp, and shasum; inspected the roadmap, README, runner, INDEX, all current
and retained attempt artifacts, the recapture blocker, the DES/implementation
and DESIGN review artifacts; and checked commit identity and parentage with
git show, git rev-parse, git status, and git diff metadata. The current
canonical evidence files were compared byte-for-byte with the fresh attempt.

No Cargo command, test runner, mutation run, staging operation, commit, or
external message was performed by this auditor. The native execution being
audited is the captured run identified in verification.yaml; this audit did
not rerun it or manufacture a second result.

## Iteration history

| Iteration | Audit input | Verdict | Findings | Remediation disposition |
|---:|---|---|---|---|
| 1 | Prior audit dated 2026-09-09T16:10:17Z against the earlier final native capture | **NEEDS_RECAPTURE** | No direct post-withdrawal/post-recovery Stable observation; the 09:12 failure existed only as DES prose and had no retained raw attempt artifact | Correctly kept README and INDEX pending; required explicit excluded retention or raw recovery and a fresh native capture with direct Stable observations |
| 2 | This independent re-audit of attempt-20260909T181642Z after approved amendment and commit bb52713ee429443b04b153580832fad022fc6307 | **SATISFIED** | No blocking finding; one non-blocking retained run.log hygiene observation | Historical gap closed by explicit EXCLUDED.md disposition and fresh direct public Stable evidence; authorize a separate bounded README/INDEX status action |

## Final verdict and bounded documentation authorization

**SATISFIED.** The fresh retained attempt is a truthful native-metal
built-product execution with explicit dirty-source provenance. It directly
shows, through the approved public surfaces:

- readiness Pass to Fail to Pass;
- direct current-row Stable observations before, during, and after;
- the same allocation ID remaining Running with zero restarts;
- transition observations of 222 ms and 199 ms, within the two-second bound;
- peer outcomes exact-reply, unreachable-no-exact-reply, exact-reply using the
  approved checked-in VM Job oracle; and
- the complete zero cleanup delta.

The earlier successful partial attempt and the earlier failed attempt remain
truthfully retained with their limitations and exclusion. No evidence is
fabricated, reconstructed, silently overwritten, or self-stamped as
satisfied. This audit does not claim unproven private or internal behavior.

This **SATISFIED** verdict explicitly authorizes one separate, bounded
documentation-only action: change the E11 README status and the matching E11
verification INDEX status to satisfied, with a link to this approved audit.
That action must preserve all receipts, transcripts, ledgers, attempt
directories, the excluded disposition, and DES history. This audit does not
make that README or INDEX change.
