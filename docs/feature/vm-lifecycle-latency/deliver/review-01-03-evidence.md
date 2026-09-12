# Adversarial evidence review — step 01-03

## Review state

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Step | `01-03` — Native measurements |
| Latest iteration | 4 |
| Current remediation commit | `eeeb71abbe2d3d79d890777804f5e185fb1281ca` |
| Current DES-log commit | `6d3e7887fbb481174cb58fcdd635c59de194beef` |
| Current E10 recorded source SHA | `a3ebd296f2b4a8bfac5fc85145ababc313da1f84` plus dirty-state receipt |
| Reviewer | Fresh independent “different fox” evidence reviewer |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated reviewer selection) |
| Review date | 2026-09-12 |
| Verdict | **APPROVED** |
| Open findings | 0 (`F1`-`F4` closed) |

## Executive assessment

The retained native-metal captures materially support E06, E08, the third
E09-v2 attempt, and E11 at their recorded `111d404c...` source plus preserved
dirty source state. E09-v2 has exactly 20 distinct, complete, passing ledger
rows, one `(control_plane_pid, control_plane_start_ticks)` identity, one healthy
and one failure deployment transcript per trial, and no cancelled, substituted,
retried, or discarded pair. E11 records the exact `Pass -> Fail -> Pass`
readiness sequence while the same Service allocation remains `Running`,
`Stable`, and at restart count zero. E06 and E08 retain non-empty built-product
operator output and zero owned-resource cleanup deltas.

E10 cannot be certified. Its six 302/404/503 cells record `deploy_exit=0`,
directly contradicting the expectation's required nonzero failure deploy stream.
More fundamentally, the current capture retains only the derived eight-cell
ledger: it does not retain the underlying deploy, describe, stop, post-cleanup,
or accepted/stale-session output needed to independently verify the claimed
prior `Failed` snapshot, exact probe, same allocation/restart preservation, or
stale-session refusal. The ledger labels those facts, but a label emitted by the
capturing script is not independent executed evidence of them.

The very large `dirty-diff.patch` files are syntactically complete, finite text
patches and contain interpretable source-state provenance; their size alone does
not invalidate the product captures. They are nevertheless self-referential
receipts: each patch includes a diff of the same output file while that file was
being rewritten, so its self-hunk does not describe its final committed bytes.
They also recursively include earlier dirty evidence receipts. This explains the
10.8-million-line commit and proves the receipts are not exact, self-reproducing
snapshots of the entire dirty tree. The executable runner/example deltas remain
recoverable and hash-identifiable, so this defect does not by itself refute
E06/E08/E09-v2/E11; it must not be described as pristine or complete provenance.

## Governing EDD policy

This review applies the current working-tree `.claude/rules/verification.md`:

- An expectation is a historical, point-in-time feature-verification record,
  not a recurring test or CI gate.
- A later HEAD does not require a rerun and does not invalidate evidence for the
  recorded SHA/environment.
- The claim must be supported by actual commands, stdout/stderr, exit status,
  substrate, SHA, and dirty-state receipt. Narration or a derived success label
  cannot replace the underlying observation for a disputed sub-claim.
- A different reviewer, not the implementation/capture author, renders the
  evidence verdict.
- Black-box evidence drives the built product and must not invoke Cargo tests,
  Rust test binaries, or import/link an `overdrive-*` crate.

No production implementation was inspected as a substitute for evidence. S11
is a Rust native acceptance test and is outside this audit.

## Catalogue-wide boundary and integrity checks

### SHA, time, and execution status

All five current manifests record `overdrive_sha:
111d404c17f778067d197a07d1acbbc044cd6eaa`, `working_tree_dirty: true`,
`execution_substrate: native-metal`, `executed_in_lima: false`, and
`runner_exit_code: 0`. Their capture times and product metadata are ordered:

| Expectation | Manifest time (UTC) | Product interval (UTC) | Result |
|---|---:|---:|---|
| E06 | 00:06:48 | deploy 00:07:11; Running 00:07:13 | exit 0 |
| E09-v2 | 00:42:48 | 00:42:55–00:44:55 | exit 0 |
| E08 | 00:45:41 | 00:45:51–00:46:39 | exit 0 |
| E11 | 00:46:44 | 00:46:57–00:48:01 | exit 0 |
| E10 | 08:02:35 | 08:02:51–08:05:09 | exit 0 |

The metadata and transcript times do not contradict one another. E10 also
retains its earlier failed 2026-09-08 attempt under
`evidence/first-attempt-2026-09-08T234614Z/`, so the successful capture did not
erase that diagnostic history.

### Built-product and black-box boundary

The non-diff capture files contain no `cargo test`, `cargo nextest`, Rust test
binary path, `extern crate overdrive`, or `use overdrive_*::` invocation.

- E06 records the exact build command, with no feature flag, in
  `evidence/binary_under_test.txt:1-4` and `evidence/build.log:1`, then records
  the direct built-binary deploy/describe outcome.
- E08, E09-v2, E10, and E11 record `cargo xtask metal run -- ...` at the start of
  `product-run.out`, followed by direct `/home/ubuntu/overdrive/target/debug/
  overdrive` CLI commands in the retained product transcript where the runner
  emitted them. Their build output is real but does not retain a binary digest
  or a standalone `binary_under_test.txt`; therefore these captures support the
  built-product boundary but do not support a claim about a particular binary
  bit hash.

The metal transport prints `fail-closed native x86_64/KVM preflight` before the
remote journey. E06 additionally records `x86_64`, Linux
`7.0.0-29-generic`, cgroup v2, `/dev/kvm`, Cloud Hypervisor v53.0, and staged
kernel/rootfs sizes in `probe_before_capability.txt:1-7`. E08/E09-v2/E10/E11 do
not retain the kernel release or kernel/rootfs hashes; their environment claim
must remain scoped to the recorded native-metal transport, checked-in spec
digests, and successful real-guest observations, not to a named kernel/rootfs
build.

### Anchor resolution

Each README has at least one real pre-verification `Anchor:` line:

| Expectation | README anchor evidence | Assessment |
|---|---|---|
| E06 | `README.md:124-129` — S-VM-39, roadmap 03-04, K4, DWD-24, ADR-0083/0082 | Resolved and predates the 2026-09-12 capture. |
| E08 | `README.md:22-26` — S-SVM-01, US-SVM-1/2, ADR-0090/0091, K1/K2 | Resolved and predates capture. |
| E09-v2 | `README.md:60-65` — S-SVM-25 plus S-VLL-12 amendment | Resolved in the recorded base; S-VLL-12 is at `vm-lifecycle-latency/feature-delta.md:566`. |
| E10 | `README.md:39-44` — S-SVM-26 and ADR-0078/0090/0099/0100 | Resolved and predates capture. |
| E11 | `README.md:18-22` — S-SVM-27A/B/C and US-SVM-3/K3 | Resolved and predates capture. |

No `unanchored-claim` disposition is required.

## E06 — VM Job deploy reaches Running

### Claim assessment: **SUPPORTED, with canonical-record correction required**

| Dimension | Evidence assessment |
|---|---|
| Anchor | PASS — `README.md:124-129`. |
| Cardinality | PASS — one accepted `e06-vm-job`, one allocation, one observed transition from no row to `Running`. |
| Substrate | PASS — actual native-metal command plus explicit x86_64/KVM/Cloud Hypervisor/cgroup/kernel/rootfs-size capture. |
| SHA / dirty state | PASS for executed source — manifest pins `111d404c...`; the dirty receipt contains the E06 runner target blob `a82515db` at `dirty-diff.patch:2401-2402`, the same blob committed in `c353af6c`. |
| Built binary | PASS — `binary_under_test.txt:1-4` says `cargo build -p overdrive-cli --bin overdrive` with no features and records the executable path/size. |
| Operator outcome | PASS — deploy exits 0 and prints `Accepted.` (`deploy_vm_job.meta:1-4`, `deploy_vm_job.out:1-7`); the next structural poll records `Running` (`describe_poll_trail.out:8-22`, `describe_final.out:1-15`). |
| Real VM path | PASS — `serve.log:5-9` records VMM creation, READY, intercept install, and EXEC release; `resource_delta.txt:5-7` records one new hypervisor, run directory, and scope. |
| Cleanup | PASS — `leak_verdict.txt:3-6` is zero for hypervisor, scope, run directory, and XDP. |

The raw evidence therefore supports the historical E06 claim at the manifest's
recorded source plus dirty runner. It does not support treating `c353af6c` or
current HEAD as the binary source SHA.

The README's “CURRENT CAPTURE” prose is stale: `README.md:5-6` and
`README.md:274-280` call SHA `fff9fe16` the current capture, while the canonical
`evidence/verification.yaml:2-4` and September 12 outputs record `111d404c...`
plus a dirty tree. The old different-fox attribution at `README.md:73-79` also
describes the prior evidence set. This review independently certifies the new
raw capture, but the README still needs to identify the actual canonical pin.

## E08 — VM Service guest health

### Claim assessment: **SUPPORTED**

| Dimension | Evidence assessment |
|---|---|
| Anchor | PASS — `README.md:22-26`. |
| Cardinality | PASS — one Service and one checked-in peer VM Job journey. |
| Substrate | PASS with scope limitation — native-metal invocation and successful real VM Service/VM Job observations; no kernel/rootfs hash retained. |
| SHA / dirty state | PASS for executed source — manifest pins `111d404c...`; its dirty receipt records the then-executed example blob `18093fb5` at `dirty-diff.patch:304-305`. |
| Operator outcome | PASS — `product-run.out:22-32` retains accepted deploy, Stable ordering, startup probe index 0, and exit 0. `:33-54` records the VM allocation `Running`, terminal `Stable`, guest address, TCP pass, and HTTP pass. |
| Peer traffic | PASS — `product-run.out:55-69` records the peer VM Job acceptance and public `Succeeded` result, followed by the byte-exact guest-reply oracle. |
| Cleanup | PASS — `product-run.out:70-75` records explicit stops and zero deltas for VM, probe, network, cgroup, run directory, mount, loop, and preparation. |

`product-run.out` and `run.log` are byte-identical (`178f7b2b...`) and non-empty
(3,734 bytes), so there is no missing or truncated secondary transcript.

## E09-v2 — third TCP truthfulness attempt

### Claim assessment: **SUPPORTED**

The README's `pending` status is correct before this different-fox review; the
current evidence itself supports transition to `satisfied` for this historical
capture.

| Dimension | Evidence assessment |
|---|---|
| Anchor | PASS — `README.md:60-65`, including the S-VLL-12 amendment. |
| Cardinality | PASS — exactly 20 data rows, 20 unique trial numbers, 20 unique healthy IDs, and 20 unique failure IDs in `tcp-truthfulness-20.tsv:2-21`. |
| Persistent identity | PASS — all 20 rows carry the sole pair `1335078 / 245960957`; `product-run.out:23` records that process identity once. |
| No substitution | PASS — all rows are `outcome=pass`, `stage=complete`; no `failed` or `not-run-cancelled` row exists. There are exactly 20 case-begin and 20 case-end transcript boundaries and exactly one healthy, failure, healthy-client, and failure-client deploy block per case. |
| Healthy half | PASS — all 20 healthy deploy exits are 0, all healthy peers are `Succeeded`, and all healthy cleanup cells are `zero-runtime`. The raw transcript has 20 healthy deploy blocks and 20 zero command exit records. |
| Failure half | PASS — all 20 failure deploy exits are 1, all report `StartupProbeFailed` and a failed TCP target at `0.0.0.0:18999`, all negative-control peers complete `Succeeded`, and all failure cleanup cells are `zero-runtime`. The raw transcript has 20 `did not converge to stable` failures and no `service-...-f is stable` line. |
| Terminal trajectory | PASS — the ledger records post-stop `Terminated`; the raw transcript retains prior failed startup probes and, where replacement-start occurs, `Running` without a `terminal: Stable` line before explicit stop. This is the trajectory expressly allowed by `README.md:25-28`, not a substituted success. |
| One process / time bound | PASS — metadata records 00:42:55–00:44:55 and exit 0, inside the declared 600-second remote owner budget. |

The final raw marker at `product-run.out:46712` says 20/20 through one
unchanged PID, but approval does not rely on that narration: the independent TSV
cardinality/uniqueness calculation and raw block counts produce the same result.
There is no evidence of a pair-level retry, replacement ID, dropped row, or
cancellation substitution.

## E10 — VM Service HTTP cross-driver status

### Claim assessment: **REFUTED / INSUFFICIENT**

| Dimension | Evidence assessment |
|---|---|
| Anchor | PASS — `README.md:39-44`. |
| Cardinality | PASS — exactly eight unique `(driver,status)` cells and eight allocation IDs in `http-status-cross-driver.tsv:2-9`. |
| Substrate | PASS with scope limitation — actual native-metal invocation, but no kernel/rootfs or binary digest retained. |
| SHA / dirty state | PASS for executed script source — the E10 receipt records the final example blob `ae53f241` at `dirty-diff.patch:320-321`, matching `c353af6c`; the manifest still correctly identifies base plus dirty state as the executed source. |
| Recovered Running / restart | SUMMARY ONLY — six cells say `before_stop=Running`, `restart_before=1`, and `trajectory=recovered-running-operator-stop`; no underlying public describe output is retained. |
| Prior Failed / probe | SUMMARY ONLY — six cells say `failure_surface=deploy+prior-failed+probe` and `probe_result=status-{302,404,503}`; no prior snapshot or rendered probe line is retained. |
| Stop / cleanup | SUMMARY ONLY — eight cells say `after_stop=Terminated`, `settled_state=Terminated`, and `cleanup=zero-delta`; no stop output, second post-cleanup describe, or resource complement is retained. |
| Stale-session refusal | **NO EVIDENCE** — neither the ledger nor `product-run.out` records accepted/current session identity, a stale session attempt, or a refused stale observation. |
| Failure deploy result | **REFUTED** — every one of the eight cells records `deploy_exit=0`, including all six 302/404/503 cells. |
| Sentinel | PASS only as a summary count — all four sentinel columns total zero, but underlying operator streams are absent. |

The contradiction is exact. `README.md:20-23` requires the 302/404/503
nonzero deploy stream as the occurrence surface. Yet
`http-status-cross-driver.tsv:3-5,7-9` and `product-run.out:23-29` record exit 0
for all six failure cells.

The capture is also not independently auditable for the new incarnation-aware
contract. `product-run.out:20-31` consists only of the ledger and a generated
PASS sentence after the metal/build preamble. `run.log` is an incomplete ledger
tail (it starts at exec/302 and omits the header and exec/204). There are no
per-cell command lines, stdout/stderr, describe rows, session identities, stop
results, or cleanup snapshots. Byte-count columns and prose classifications
cannot substitute for the missing bytes. The current `Status: pending` in
`README.md:3-4` must remain pending.

## E11 — VM Service readiness recovery

### Claim assessment: **SUPPORTED**

| Dimension | Evidence assessment |
|---|---|
| Anchor | PASS — `README.md:18-22`; the S-SVM-27A/B/C scenarios resolve in the recorded base. |
| Cardinality | PASS — exactly three rows (`before`, `during`, `after`) and exactly two readiness transitions. |
| Substrate | PASS with scope limitation — native-metal built-product run; no kernel/rootfs hash retained. |
| SHA / dirty state | PASS for executed source — the receipt records example blob `18093fb5` at `dirty-diff.patch:304-305`, the exact pre-E10 source used for this capture. |
| State sequence | PASS — `product-run.out:33-53`, `:70-90`, and `:107-127` retain `Running`, the same allocation ID, terminal `Stable`, restart count 0, and readiness `pass`, `fail (HTTP 503)`, `pass`. |
| Timing | PASS — ledger transition latencies are 334 ms and 186/157 ms observations, all below the two-second evidence bound; the maximum is 334 ms. |
| Peer outcome | PASS — before/after rows record `exact-reply`; the failed-window row records `unreachable-no-exact-reply`. Public peer Jobs are retained at `product-run.out:54-68`, `:91-105`, and `:128-142`. |
| Cleanup | PASS — explicit Service stop and all-zero owned-resource deltas at `product-run.out:151-154`. |

The three readiness observation timestamps are roughly ten seconds apart, as
the claimed fixed phases require, while allocation identity/state and restart
count stay unchanged. The ledger's `334/186/157 ms` transition figures and
`2451/3566/2453 ms` peer windows are internally consistent with the product
timestamps and do not overstate sub-second application response latency.

## Large dirty-patch integrity analysis

### Measured artifacts

| Expectation | Bytes | Lines | Git blob | SHA-256 |
|---|---:|---:|---|---|
| E06 | 390,583,223 | 5,221,260 | `a57bad2b...` | `573ae4a660fb65134a6b47ad481622d864c0d22fcfe7bb6cfcf0a35472f489a6` |
| E08 | 635,953,515 | 8,283,088 | `e24b5ad9...` | `a08e0fdcdc60a1478b8c9d4c5f0ddf91a5c345a22f812f6d7a877244a4575102` |
| E09-v2 | 671,896,468 | 8,782,028 | `1bec66f5...` | `348658262b0fd3dee1b17b6a7f226086d7c4c9e03059226951eb8e7a169704fc` |
| E10 | 879,843,971 | 11,393,961 | `0bee7467...` | `f3a7a34bae32e3964ae5e37d1fc6c6230929a96dc7463a708a0c438ffee1d8c5` |
| E11 | 428,599,614 | 5,538,744 | `9f2eaf3c...` | `aff3de50b99e9234ef0c8fa100fb60014561959698ec0fdca64e8a2c5b6c8dc6` |

All five parse successfully with `git apply --numstat`: E06 describes 54 files;
the other four describe 59 files. There are no `GIT binary patch` blocks. This
rules out simple truncation or unreadable binary-garbage corruption.

### Why they are enormous

Each receipt includes the already-dirty receipts of the other expectations as
ordinary text. Later receipts therefore contain earlier receipts recursively.
For example, the E10 receipt's index headers identify the final committed E06,
E08, E09-v2, and E11 receipt blobs (`a57bad2b`, `e24b5ad9`, `1bec66f5`, and
`9f2eaf3c`). This is finite recursive inclusion of prior captured text, not an
infinite recursion and not a missing tail.

Each receipt also includes itself while it is being generated:

| Receipt | Self-hunk line | Self-hunk target hash | Final committed hash |
|---|---:|---|---|
| E06 | 421 | `272adb0e` | `a57bad2b` |
| E08 | 5,223,906 | `a8d37267` | `e24b5ad9` |
| E09-v2 | 5,756,909 | `9a9f49ec` | `1bec66f5` |
| E10 | 5,511,473 | `d5b9f8cb` | `0bee7467` |
| E11 | 5,535,008 | `9df24c66` | `9f2eaf3c` |

The self-hunks have the shape “old tracked patch -> zero lines,” while their
index target is neither the empty-blob hash nor the eventual committed blob.
That is the observable signature of redirecting `git diff` into the same tracked
file it is diffing. Consequently no receipt can reproduce its own final bytes,
and the phrase “complete dirty-tree diff” would overreach.

The relevant executable-state portions remain independently interpretable:
the E06 runner target hash is `a82515db`; E08/E09-v2/E11 record example target
`18093fb5`; E10 records target `ae53f241`. Those hashes identify the exact
command-source revisions synced for the respective captures. The raw product
logs and ledgers are separate immutable Git blobs and are not truncated by the
self-hunk defect.

Disposition: **accurate but pathologically bloated provenance for other files,
plus a self-referential completeness defect for the receipt file itself**. Do
not reject E06/E08/E09-v2/E11 merely because of size. Future capture should
exclude its own output and unrelated prior evidence blobs. E10 requires a fresh
capture for its substantive failures regardless, so its replacement receipt
should also fix this defect.

## Commands and checks performed

All checks were read-only except creation of this review artifact.

| Check | Result |
|---|---|
| Read all mandatory repository rules and current `.claude/rules/verification.md` | PASS |
| `git show`/`git ls-tree` against commit `c353af6c` and base `111d404c` | Captures, sizes, blob IDs, and source/dirty relationships enumerated |
| README anchor resolution with `git grep` at the recorded base | PASS for all five expectations |
| Manifest/product metadata timestamp and exit-code comparison | PASS; no time/status contradiction |
| TSV cardinality/uniqueness aggregation with tab-delimited `awk` | E09 20/20 and one identity; E10 8/8; E11 3 phases and max 334 ms |
| Raw E09 transcript block/pattern counts | 20 case starts/ends; 20 each healthy/failure/client deploy block; 20 failure exit-1 wrappers; zero failure-Service Stable line |
| Black-box capture scan for Cargo tests, nextest, Rust test binaries, or crate imports | No matches outside dirty provenance patches |
| `git apply --numstat` parse of all five large receipts | PASS; 54/59-file valid text patches |
| SHA-256, byte count, line count, self-hunk, and cross-receipt blob relationship checks | Values recorded above |
| Historical rerun | Not performed and not required by current EDD policy |

## Findings and dispositions

### F1 — E10's failure deploy exit codes contradict its expectation

- **Severity:** Blocker
- **Evidence:** `E10/README.md:20-23` requires a nonzero deploy stream for
  302/404/503. `E10/evidence/http-status-cross-driver.tsv:3-5,7-9` and
  `product-run.out:23-29` record `deploy_exit=0` for all six cells.
- **Failed claim:** the typed startup failure has the required public nonzero
  deploy occurrence surface.
- **Disposition:** OPEN. Keep E10 pending. A replacement capture must either
  exhibit the accepted nonzero contract or the expectation/design must be
  separately changed before capture; the evidence reviewer must not reinterpret
  exit 0 as nonzero.

### F2 — E10 does not retain evidence for prior-failure preservation or stale-session refusal

- **Severity:** Blocker
- **Evidence:** `E10/evidence/product-run.out:20-31` contains only the derived
  ledger and PASS line. `run.log:1-7` is an incomplete ledger tail. No retained
  file contains per-cell deploy output, the before/after public describe rows,
  accepted/stale session identities, stale-watcher refusal, explicit stop
  output, or post-cleanup resource snapshot.
- **Failed claim:** the same recovered allocation preserves restart count,
  depth-one prior `Failed`, and failed probe through explicit stop and cleanup,
  while a stale session is refused.
- **Disposition:** OPEN. Retain the actual operator outputs and session/resource
  observations in the next E10 capture. A string such as
  `deploy+prior-failed+probe` is a summary, not the missing observation.

### F3 — E06's README identifies the wrong capture as current

- **Severity:** High
- **Evidence:** `E06/README.md:5-6,274-280` identifies `fff9fe16` as the current
  capture and `:73-79` attributes its audit to the old evidence. The canonical
  `E06/evidence/verification.yaml:2-4` records the present capture on 2026-09-12
  at `111d404c...` with a dirty tree.
- **Failed claim:** unambiguous canonical SHA/audit attribution for the current
  evidence files.
- **Disposition:** OPEN. Correct the README metadata to the actual historical
  pin and link this different-fox review. The raw capture itself need not be
  rerun.

### F4 — dirty receipts are not exact self-reproducing dirty-tree snapshots

- **Severity:** Medium
- **Evidence:** every large patch has one self-hunk whose target hash differs
  from its final committed blob, at the exact lines listed in the integrity
  table. The patches recursively carry unrelated earlier evidence receipts.
- **Failed claim:** only any assertion that `dirty-diff.patch` is a complete,
  exact snapshot including its own final bytes. The executable source deltas and
  separate raw outputs remain interpretable.
- **Disposition:** OPEN, non-refuting for E06/E08/E09-v2/E11. Generate future
  receipts outside the diff target or exclude the receipt/evidence paths, while
  still preserving all dirty production/example/runner inputs. Do not rewrite
  historical raw whitespace or discard prior attempts.

## Per-expectation disposition

| Expectation | Evidence verdict | Catalogue disposition |
|---|---|---|
| E06 | Supports VM Job acceptance and real VM allocation `Running` at recorded source+dirty state | Claim supported; correct stale README pin/audit text |
| E08 | Supports guest TCP/HTTP health, Stable, peer exact-reply journey, and zero cleanup delta | Satisfied for recorded capture |
| E09-v2 | Supports third-attempt 20/20 truthfulness through one persistent control-plane identity with no pair substitution | Eligible for `satisfied` after this audit |
| E10 | Refuted on deploy exit; insufficient on preserved history/session/cleanup evidence | Remain `pending`; fresh corrected capture and re-audit required |
| E11 | Supports Pass/Fail/Pass readiness recovery, unchanged Running/Stable allocation, bounded transitions, traffic withdrawal/restoration, and cleanup | Satisfied for recorded capture |

## Final verdict

**CHANGES_REQUESTED.** E10's current capture does not satisfy its accepted
point-in-time expectation, and E06's canonical README pin is stale. The large
patches are not rejected for size, but their self-referential completeness
defect must not be described as exact full-tree provenance. E06, E08, E09-v2,
and E11 do not require reruns merely because HEAD later advanced; their findings
are scoped to metadata/receipt handling, not their historical operator outcomes.

After E10 retains the underlying public outputs with the required exit/state/
session behavior and E06's pin text is corrected, this same step-specific
evidence reviewer should append iteration 2 and re-review only those
remediations before step 01-03 can receive an approved evidence verdict.

---

## Iteration 2 — evidence-remediation re-review

### Metadata

| Field | Value |
|---|---|
| Iteration | 2 |
| Prior evidence commit | `c353af6c6454be2df1b2417105f2337d6f7364ab` |
| Current EDD clarification | `a3ebd296f2b4a8bfac5fc85145ababc313da1f84` |
| Remediation commit | `057434d8755fa79daa636f0177a61a341085582b` |
| DES remediation events | `e97a61a9ef7ced42ec21cce677be42f289c08fe2` |
| E10 recorded source | `a3ebd296f2b4a8bfac5fc85145ababc313da1f84` plus the replacement dirty receipt |
| E10 capture token | `e10-20260912T093028Z-98294` |
| Reviewer | Same step-specific independent evidence reviewer |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-12 |
| Verdict | **CHANGES_REQUESTED** |

### Iteration 2 executive assessment

The remediation closes the substantive raw-evidence and forward receipt defects
from F2 and F4. E10 now retains exactly eight raw cell directories under one
capture token. The raw exit files and PTY transcript trailers show exit 0 for
both 204 cells and exit 1 for all six 302/404/503 cells. The derived ledger is
byte-identical in its two retained locations and independently matches the raw
exit, allocation, state, restart, prior-failure, probe, byte-count, sentinel,
stop, and named-cleanup observations. Each failing cell reaches the same
allocation ID in recovered `Running` with restart count 1, retains a depth-one
prior `Failed` and the failed HTTP probe, then reaches operator-stopped
`Terminated` and remains there after its named resources are absent. F2 is
closed.

The replacement E10 dirty receipt is 78,634 bytes rather than hundreds of
megabytes, parses as a ten-file text patch, has no evidence-path or self-hunk,
and contains the exact dirty example/expectation/harness inputs used for the
capture. The four historical large receipts are byte-identical to iteration 1.
The forward-generation fix therefore closes F4 without rewriting historical
records.

Approval is still blocked by two direct record contradictions:

1. The current E10 contract says every failing streaming command first receives
   `Accepted`, then receives the typed failure and exits 1
   (`E10/README.md:22-25`). All six complete raw PTY transcripts instead contain
   only the typed `Error` block before `COMMAND_EXIT_CODE="1"`; `Accepted.` is
   absent from every raw file for those cells. The exit-code half of F1 is fixed,
   but the capture does not match the complete public-stream contract it now
   claims.
2. E06's primary current-capture pin and review link are corrected, but its
   lower Evidence section still says a 2026-08-19 audit read and rendered the
   current September 12 capture (`E06/README.md:303-308`). That is false audit
   attribution beside the correct 2026-09-12 link at `:75-82`. F3 remains open.

E10 correctly remains `pending`; no implementation/capture author has
self-certified it as `satisfied`.

### Current EDD policy and scope

Commit `a3ebd296` is authoritative for this iteration. These captures are
point-in-time feature-verification records, not recurring tests or benchmarks.
No rerun is demanded merely because later commits exist. The review asks only
whether the retained command/output/substrate/SHA/dirty receipts support the
current operator claim. It does not inspect production implementation as a
substitute for evidence, and it does not audit S11's native Rust acceptance
test.

The E10 eight-cell Exec/VM by HTTP-status contrast remains a small bounded
feature matrix, not a reliability cohort, performance distribution, or
benchmark under the clarified classification.

### Commit and scope audit

| Commit | Mechanical assessment |
|---|---|
| `a3ebd296` | Changes the EDD/test classification rules only. Author remains Marcus; exactly one required Codex co-author trailer; no Claude/Anthropic/generated-by attribution. |
| `057434d8` | 274 files, 103,434 insertions and 181 deletions. Scope is the E10 raw evidence/runner/example journey, E06/E10 catalogue prose, expectation index, and receipt harness/checks. No Rust production crate, architecture, design, DISTILL, or roadmap file changed. Author remains Marcus; exactly one required Codex trailer and no banned attribution. |
| `e97a61a9` | Changes only `docs/feature/vm-lifecycle-latency/deliver/execution-log.json`; records remediation GREEN/COMMIT events. Author remains Marcus; exact required Codex trailer and no banned attribution. |

The pre-existing user-owned `AGENTS.md` modification remains uncommitted and
untouched. This review artifact remains the reviewer's only write.

## F1 re-review — E10 deploy stream and exit contract

### Disposition: **PARTIALLY RESOLVED; OPEN**

The original exit-code contradiction is resolved:

| Cell class | Raw evidence | Independent result |
|---|---|---|
| Exec 204 | `raw-cells/e10-exec-204/service-deploy-exit.log:1`; PTY trailer `service-stream.log:11` | exit 0 |
| VM 204 | `raw-cells/e10-vm-204/service-deploy-exit.log:1`; PTY trailer `service-stream.log:11` | exit 0 |
| Exec 302/404/503 | each `service-deploy-exit.log:1`; each PTY `service-stream.log:8` | exit 1 in all three |
| VM 302/404/503 | each `service-deploy-exit.log:1`; each PTY `service-stream.log:8` | exit 1 in all three |

The six failure stdout files also carry the correct numeric status and typed
startup failure. For example:

- `e10-exec-302/service-deploy-stdout.log:1-3` records HTTP 302 and “redirect
  not followed”;
- `e10-exec-404/service-deploy-stdout.log:1-3` records HTTP 404;
- `e10-vm-503/service-deploy-stdout.log:1-3` records HTTP 503.

The ledger no longer substitutes a hard-coded zero. Its six failure rows say
`deploy_exit=1`, and the retained validation path reads each
`service-deploy-exit.log` before accepting the row. Independent comparison of
the raw files to both ledgers produced no mismatch.

The complete current contract nevertheless fails. `E10/README.md:22-25` says
the failing streaming command first receives the asynchronous `Accepted`
acknowledgement and then the typed `Failed` terminal event. The complete raw
transcripts for all six failure cells are only eight lines each:

- Exec: `e10-exec-{302,404,503}/service-stream.log:1-8`;
- VM: `e10-vm-{302,404,503}/service-stream.log:1-8`.

Each goes directly from the command header to `Error: workload ... did not
converge to stable`, the numeric reason, reproducer/hint, and
`COMMAND_EXIT_CODE="1"`. A full-tree search across each cell directory and its
top-level transcript found zero `Accepted.` lines. This is not an output split:
the corresponding `service-deploy-stdout.log` files contain the same failure
block and `service-deploy-stderr.log` is present but empty.

The evidence therefore proves **typed failure + exit 1**, but refutes or at
least fails to evidence **Accepted then typed failure + exit 1**. A generated
ledger label cannot supply the missing acknowledgement. F1 remains open until
the retained public stream matches the current contract, or an independently
approved contract correction removes that sub-claim. This review does not
authorize either product behavior or contract prose to be invented here.

## F2 re-review — raw incarnation, cleanup, and no-late-overwrite evidence

### Disposition: **CLOSED**

Exactly eight raw cell directories exist: `e10-{exec,vm}-{204,302,404,503}`.
Every directory's `capture-token.log:1` is
`e10-20260912T093028Z-98294`, matching `product-run.meta:4`; there is no ninth,
missing, or stale-token directory. Each directory contains 28 files for a 204
case or 32 files for a failing case. Every top-level
`e10-<driver>-<status>.transcript` is byte-identical to its directory's
`case-transcript.log`.

### Raw-to-ledger cardinality and integrity

| Check | Result |
|---|---|
| Raw directories | 8 exactly |
| Capture tokens | 8/8 identical to product metadata |
| Raw and top-level ledger | Byte-identical; SHA-256 `0b62f857a11776f9234c3037fb14e355b0be68617fb2bc3fda47f9862acb03b3` |
| Ledger cardinality | 8 unique `(driver,status)` cells and 8 unique allocation IDs |
| Deploy exits | 2 cells at 0; 6 cells at 1, exactly by status class |
| Stop exits/output | 8/8 exit 0 with the exact workload named in `Stopped workload ...` |
| Raw stdout/stderr/describe byte counts | 8/8 equal ledger columns 13-15 |
| Sentinel scan | Zero occurrences in all retained operator surfaces |
| Black-box boundary | No Cargo test, nextest, Rust test binary, or `overdrive-*` crate import in product/raw capture |

`product-run.meta:2-7` records the exact native-metal invocation, capture root,
token, 09:30:29–09:33:32 UTC interval, and exit 0. `product-run.out:1-19`
records the metal transport, exclusive lease, source sync, fail-closed native
preflight, and real build completion. `product-run.out:20-31` emits the same
eight rows only after those raw cases complete.

### State and probe preservation

Both 204 raw cells show:

- `before-stop-describe.log:5-6` — the named allocation is `Running`, restart 0,
  terminal `Stable`;
- `before-stop-describe.log:20` — startup HTTP probe index 0 is `pass`;
- `stop-command.log:1`, `stop-exit.log:1`, and `stop-stdout.log:1-2` — exact
  explicit operator stop, exit 0;
- `after-stop-describe.log:5-6` and `post-cleanup-describe.log:5-6` — the same
  allocation is `Terminated`, restart 0, with a stopped reason.

All six failing raw cells show recovered state before stop:

- `before-stop-describe.log:5` — the same status-derived allocation ID is
  `Running` at restart 1;
- `:7` — a depth-one `last terminated: Failed` snapshot;
- the final probe line — startup probe index 0, HTTP GET, exact target, `fail`,
  and numeric 302/404/503 reason.

Their `recovery-observations.log` files retain the observed terminal/recovery
trajectory rather than only the settled row. Exec 302, for example, records
`Terminated` restart 0 and then `Running` restart 1 at
`e10-exec-302/recovery-observations.log:1-35`; VM 302 records its repeated
`Failed` observations and final same-allocation `Running` restart 1 at
`e10-vm-302/recovery-observations.log:1-274`.

After explicit stop, each failure cell preserves the allocation ID, restart 1,
exact prior-failure line, and failed-probe semantics in both
`after-stop-describe.log` and `post-cleanup-describe.log`. The probe observation
time may advance across stop but never regresses, and the after-stop and
post-cleanup probe lines are equal. Representative VM evidence is
`e10-vm-302/{before-stop,after-stop,post-cleanup}-describe.log`; the same
independent comparisons passed for all six cells.

### Named cleanup and external no-late-overwrite witness

The raw named-resource check is absolute, not inferred from the whole-host
delta:

- every Exec cell names three observed resources (allocation cgroup and the two
  allocation network names);
- every VM cell names five (cgroup, VM run directory, allocation-owned
  hypervisor PID, and two network names);
- every name appears in `current-session-resources.log` and is absent from both
  `post-runtime-cleanup-resources.log` and `cleanup-resources-after.log`;
- every final snapshot records `[materialization]` as `absent`;
- `cleanup-resource-complement.log:1` reports exactly 3 or 5 observed names,
  `post-stop=absent`, `serve=stopped`, and `preparation=absent`;
- the ledger truthfully records `named-absent` in all eight rows.

The whole-host `cleanup-resource-delta.log` is retained only as a diagnostic and
is not the named-resource oracle.

For failing cells, `stale-session-refusal.log:1-9` is derived from retained
public rows and agrees with them: allocation ID and restart count remain equal,
the depth-one prior `Failed` line is byte-equal, the probe semantics are equal,
the observation timestamp is non-regressive, and the post-cleanup state remains
operator-stopped `Terminated` after the current session's named resources are
gone. For 204 cells, the four-line receipt similarly preserves allocation ID and
restart 0, while the raw after/post-cleanup describes preserve Terminated. This
is an external no-late-overwrite observation; it does not pretend to expose or
prove the private `Arc<BeaconWriter>` identity.

F2 is closed. The raw evidence now supports the ledger rather than the ledger
standing in for the evidence.

## F3 re-review — E06 canonical pin and audit attribution

### Disposition: **PARTIALLY RESOLVED; OPEN**

The primary corrections are accurate:

- `E06/README.md:5-6` names the actual 2026-09-12 capture at
  `111d404c17f778067d197a07d1acbbc044cd6eaa`, dirty, seed 1;
- `:30-31` identifies that SHA as capture 3;
- `:48-50` identifies dirty runner blob `a82515db` rather than pretending the
  runner was committed at the base;
- `:69-73` describes the dirty-state receipt honestly;
- `:75-82` links this step review and attributes the 2026-09-12 different-fox
  result;
- `:277-289` repeats the correct source/dirty pin in the Evidence section.

One contradictory old paragraph remains at `E06/README.md:303-308`. It still
says “a different-fox adversarial audit (2026-08-19)” read the capture and that
“this capture is the evidence it read.” The current files were captured on
September 12 and are being reviewed here; the August 19 reviewer could not have
read them. This directly conflicts with the correct September 12 attribution at
`:75-82` and leaves two incompatible audit histories in the canonical README.

F3 therefore remains open. Remove or correct only that stale audit-attribution
paragraph; no E06 rerun is required and its operator outcome remains supported.

## F4 re-review — dirty receipt generation

### Disposition: **CLOSED**

The replacement E10 receipt has these measured properties:

| Property | Result |
|---|---|
| Size | 78,634 bytes / 1,390 lines |
| SHA-256 | `ee32f003d6a41cab3fcf3300d79d0e3fb833c9af16d4d8c54577c7f9d6a50692` |
| Parse | `git apply --numstat` succeeds |
| Scope | 10 tracked dirty files, 841 additions and 129 deletions |
| Self-hunk | None |
| Evidence-path diff header | None |
| Relevant captured inputs | example README/script; E06/E10 README; E10 runner; expectation index; harness and harness check; AGENTS/DES state |

The patch's target hashes identify the exact captured inputs: example script
`9647dd54`, E10 README `dc4203a1`, E10 runner `49fbab19`, harness
`731dae47`, and harness check `38eb2f1a`. Those match the corresponding blobs
retained in remediation commit `057434d8`; the uncommitted `AGENTS.md` state is
also preserved separately in the patch rather than silently committed.

`verification/harness/run-expectation.sh:91-101` now records status normally but
builds the patch with an explicit
`:(exclude)verification/expectations/**/evidence/**` pathspec. The current E10
receipt at `dirty-diff.patch:1344-1360` itself records that forward-generation
change without including any evidence-tree hunk.

The historical receipts are unchanged across `c353af6c` and `057434d8`:

| Expectation | Preserved blob |
|---|---|
| E06 | `a57bad2b1865c05c5318bdf38cbada14c098bb60` |
| E08 | `e24b5ad9c35621b25c156b3883c2d5bae68b590a` |
| E09-v2 | `1bec66f5ce0692687d7891718b5773afa629547c` |
| E11 | `9f2eaf3c717242b492af78747b3a79e5c2b2bf6a` |

This closes the forward-generation defect without rewriting historical raw
evidence or whitespace. F4 is closed.

## Iteration 2 verification checks

| Check | Result |
|---|---|
| Current mandatory repository rules and committed EDD clarification | Read and applied |
| Remediation commit scope and attribution | PASS mechanically |
| Exact raw E10 cell-directory cardinality | PASS — 8 |
| Exact capture-token cardinality | PASS — 8/8 one token |
| Raw deploy command/output/exit inspection | Exit classes and numeric failure vocabulary pass; Accepted-before-failure sub-claim fails |
| Raw state/restart/prior/probe comparisons | PASS — 8/8 |
| Raw stop command/output/exit comparisons | PASS — 8/8 |
| Named-resource before/post/final comparisons | PASS — 3 names per Exec, 5 per VM, absent after cleanup |
| Final materialization absence | PASS — 8/8 |
| External no-late-overwrite comparison | PASS — 8/8 at the retained public boundary |
| Raw-derived ledger equality | PASS — both copies byte-identical; all raw fields match |
| Sentinel absence | PASS — all retained operator surfaces |
| Capture boundary scan | PASS — no Cargo tests, Rust test binaries, or crate imports |
| Replacement receipt parse/scope/self-reference | PASS |
| Historical receipt blob preservation | PASS — E06/E08/E09-v2/E11 unchanged |
| Historical rerun | Not performed; current EDD policy does not require one |

Raw evidence whitespace is retained verbatim and was not treated as a defect.

## Iteration 2 finding dispositions

| Finding | Status | Evidence and required disposition |
|---|---|---|
| F1 — E10 failure deploy contract | **OPEN / PARTIALLY RESOLVED** | Exit 1 is now real for all six failure cells, but all six complete public PTY streams omit the contract's preceding `Accepted` acknowledgement. Preserve the raw failure/exit evidence; resolve the contract/output mismatch without a hard-coded summary substitution. |
| F2 — E10 raw history/session/cleanup evidence | **CLOSED** | Exactly eight token-matched raw cells independently support recovery, preserved prior failure/probe/restart, stop, named absence, and external no-late-overwrite observations. |
| F3 — E06 canonical pin | **OPEN / PARTIALLY RESOLVED** | The actual SHA/dirty pin and review link are fixed, but `README.md:303-308` still falsely attributes the current capture to the August 19 audit. Correct that paragraph only; no rerun. |
| F4 — dirty receipt completeness | **CLOSED** | E10's replacement receipt excludes evidence/self-recursion, parses, preserves relevant dirty inputs, and leaves historical receipts unchanged. |

## Iteration 2 per-expectation assessment

| Expectation | Current evidence assessment |
|---|---|
| E06 | Operator claim remains supported at `111d404c...` plus dirty runner; canonical audit prose still has one stale contradictory paragraph (F3). |
| E08 | No remediation change; iteration 1 support stands for its historical capture. |
| E09-v2 | No remediation change; iteration 1's exact 20/20, one-identity, no-substitution assessment stands. |
| E10 | Raw state/probe/stop/cleanup evidence is now sufficient and its exit classes are correct, but the complete Accepted-then-failure public-stream claim is not observed. Remain `pending`. |
| E11 | No remediation change; iteration 1 support stands for its historical capture. |

## Iteration 2 final verdict

**CHANGES_REQUESTED.** F2 and F4 are closed. F1 and F3 remain open on direct,
bounded record contradictions: six E10 failure streams omit the currently
claimed preceding `Accepted` acknowledgement, and one E06 paragraph still
attributes the September 12 capture to an August 19 audit. Neither requires an
architectural mechanism, production-code speculation, a rewrite of historical
evidence, or a recurring expectation run. E10 must remain `pending` until its
complete public-stream claim is supported or separately corrected and this
reviewer approves the evidence; E06 requires only the stale attribution text to
be corrected.

---

## Iteration 3 — final contract-correction re-review

### Metadata

| Field | Value |
|---|---|
| Iteration | 3 |
| Contract/audit remediation | `fe6844bad5d3f73ca2e9dbb77c99ef0efcd6b16b` |
| DES persistence | `cdcc9622786c30089a955b0fab2dda92552be641` |
| Retained raw-evidence remediation | `057434d8755fa79daa636f0177a61a341085582b` |
| Original step evidence | `c353af6c6454be2df1b2417105f2337d6f7364ab` |
| E10 point-in-time source | `a3ebd296f2b4a8bfac5fc85145ababc313da1f84` plus its retained dirty-state receipt |
| Reviewer | Same step-specific independent evidence reviewer |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-12 |
| Verdict | **CHANGES_REQUESTED** |

### Iteration 3 executive assessment

F1 is closed. The corrected E10 CLI-failure contract matches the accepted
architecture and the unchanged raw evidence: ADR-0093's renderer amendment is
expressly success-only, while ADR-0032 maps every streaming `Failed` terminal to
exit 1 and renders its typed `Error:` block. Both healthy 204 PTYs retain one
`Accepted.` block, Stable, and exit 0. All six failure PTYs correctly retain no
`Accepted.` prefix, do retain the exact typed convergence error and numeric HTTP
status (including `redirect not followed` for 302), and exit 1. Independent
validation of the unchanged eight-cell capture passes, and an injected
`Accepted.` line in a temporary copy of a failure PTY is rejected by the current
raw-evidence validator.

F2 and F4 remain closed. No raw evidence file changed after `057434d8`; all
eight token-matched state/history/probe/stop/cleanup/no-late-overwrite cells and
the 78,634-byte non-self-referential dirty receipt remain byte-preserved.

F3 is not fully closed. The lower E06 Evidence paragraph is now correctly
attributed to the September 12 different-fox review, but the opening status
rationale still says the **current September 12 capture** was audited on
2026-08-19 (`E06/README.md:5-8`). That direct contradiction leaves one false
audit date in the canonical current-capture block. The E06 raw claim remains
supported and no rerun is required; only the stale date must be corrected.

E10 still correctly says `pending` before this adjudication. Its retained
point-in-time evidence now supports its corrected claim and is eligible to move
to `satisfied` after this review. The overall step evidence verdict remains
`CHANGES_REQUESTED` solely because F3 remains open.

### Current EDD scope

The point-in-time EDD policy from `a3ebd296` remains controlling. The raw E10
capture is judged at its recorded source/environment; later contract prose and
validator commits do not require a native recapture when they correct the
catalogue's interpretation to an already accepted public boundary and leave the
raw bytes unchanged. This review reopens neither production implementation nor
the S11 Rust acceptance test.

## F1 iteration 3 — corrected CLI failure boundary

### Disposition: **CLOSED**

The current expectation is precise at `E10/README.md:20-31`:

- the control-plane NDJSON lane may already have emitted its internal
  `Accepted` event;
- the CLI renders the acknowledgement prefix only for successful
  `Accepted -> Stable` summaries;
- a `Failed` terminal renders the terminal-only typed `Error:` block, exact
  numeric cause, and exit 1, with no separate `Accepted.` block.

This is not an invented remediation contract:

| Authority | Accepted boundary |
|---|---|
| ADR-0093 `:39-54` | Composes Accepted before Stable for the successful Service stream only. |
| ADR-0093 `:64-73` | Explicitly says the amendment is success-only and preserves `Failed` rendering and exit semantics; it does not flush Accepted separately. |
| Approved ADR-0093 review `:22-26,35` | Confines composition to `Accepted -> Stable`; every other terminal path is unchanged. |
| ADR-0032 `:573-587` | Streaming `ConvergedFailed` exits 1 and the CLI renders its typed `Error:` reason. |
| ADR-0059 `:89-100` | Keeps `Stopped` and `Failed` as distinct exit/render classes. |

The E10 anchor list now includes ADR-0093 and its approved DESIGN review at
`E10/README.md:61-69`, so the corrected claim remains externally anchored and
predates this evidence adjudication.

### Raw evidence assessment

The raw evidence under token `e10-20260912T093028Z-98294` is unchanged from
iteration 2:

| Cells | Public PTY result |
|---|---|
| `e10-exec-204`, `e10-vm-204` | One `Accepted.` block followed by Stable; `service-deploy-exit.log:1` and PTY trailer are 0. |
| `e10-exec-{302,404,503}` | No `Accepted.` line; exact `Error: workload ... did not converge to stable`, numeric HTTP reason, PTY exit 1. |
| `e10-vm-{302,404,503}` | No `Accepted.` line; exact `Error: workload ... did not converge to stable`, numeric HTTP reason, PTY exit 1. |

Every failure PTY is complete in eight lines at its
`raw-cells/e10-<driver>-<status>/service-stream.log:1-8`. The 302 files name
`HTTP 302 (redirect not followed)` at line 3; 404 and 503 name their exact
status at line 3. `service-deploy-stdout.log` carries the same typed block,
`service-deploy-stderr.log` is present and empty, and the independent exit file
is 1. A search across each failure directory and its top-level transcript finds
zero `Accepted.` lines. The two healthy PTYs retain `Accepted.` and exit 0.

The current runner's raw validator checks the exact workload-specific error,
numeric status, 302 classification, absence of `Accepted.` across PTY/stdout/
stderr, and the raw exit file before accepting the derived ledger
(`E10/runner.sh:217-245,265-278`). The checked-in example also rejects a failure
transcript containing `Accepted.` (`run-example.sh:1541-1551`). These are
validation of retained bytes, not shell synthesis of product output.

### Independent negative check

The reviewer ran the current validation-only path against the unmodified eight
raw cells and ledger; it exited 0. The reviewer then copied the raw capture to a
temporary directory outside the workspace, inserted one literal `Accepted.`
line into `e10-exec-302/service-stream.log`, and reran the same validation. It
failed with exit 1 and:

```text
E10 runner: failure PTY summary rendered the success-only Accepted prefix: exec/302
```

The temporary copy was deleted afterward. The negative check proves the current
validator does not merely accept the pre-existing ledger or hard-code the
failure classification; it reads and rejects the altered raw PTY.

F1 is closed. The point-in-time E10 capture supports the corrected CLI failure
claim without recapture.

## F2 iteration 3 — raw state/history/cleanup preservation

### Disposition: **CLOSED (confirmed)**

There is no diff under any E06/E08/E09-v2/E10/E11 `evidence/` path between
`057434d8` and current HEAD. The iteration 2 raw-to-ledger assessment therefore
remains applicable byte-for-byte.

The reviewer reran E10's validation-only path against all eight retained cells;
it exited 0. The two ledger copies remain byte-identical, all eight capture
tokens match, both 204 cells retain `Running/0 -> Terminated/0`, and all six
failure cells retain same-allocation `Running/1` plus prior `Failed` and exact
failed probe through `Terminated/1` after named resource absence. Stop exits,
sentinel counts, final materialization absence, and external no-late-overwrite
comparisons remain unchanged.

The focused cleanup/probe adversarial check also exits 0 after verifying both
positive forms and rejecting residual cgroup, network, hypervisor, mount,
changed-probe-semantics, and regressive-timestamp fixtures. F2 remains closed.

## F3 iteration 3 — E06 audit attribution

### Disposition: **PARTIALLY RESOLVED; OPEN**

The iteration 2 stale lower paragraph is fixed. `E06/README.md:303-309` now
names the September 12, 2026 different-fox review, links this artifact, and says
the current capture is the evidence it read. The correct `111d404c...` dirty
source pin remains at `:5-6`, `:30-31`, and `:277-289`.

However, the first current-capture paragraph still contains an incompatible
date:

```text
5  <!-- Status rationale — CURRENT CAPTURE (2026-09-12, SHA 111d404c..., ...)
7  `satisfied` was rendered by a DIFFERENT-FOX adversarial audit of the captured
8  evidence (2026-08-19), not self-stamped ...
```

This is the same current evidence block, not historical capture-2 prose. It
directly contradicts the correct September 12 attribution at
`E06/README.md:75-82` and `:303-309`. A reviewer should not make the reader
choose which audit date is true.

F3 remains open only for the date at line 8. Change `2026-08-19` to the actual
September 12 audit attribution (or remove that duplicate date) without touching
the raw E06 evidence. No rerun is required under current EDD policy.

## F4 iteration 3 — receipt integrity preservation

### Disposition: **CLOSED (confirmed)**

No evidence receipt changed after `057434d8`. E10's receipt remains 78,634
bytes / 1,390 lines, SHA-256
`ee32f003d6a41cab3fcf3300d79d0e3fb833c9af16d4d8c54577c7f9d6a50692`,
parses with `git apply --numstat`, and has no self-hunk or evidence-path diff
header. E06/E08/E09-v2/E11 retain the exact historical blobs recorded in
iteration 2. F4 remains closed.

## Iteration 3 commit and DES audit

| Check | Result |
|---|---|
| `fe6844ba` scope | Six files only: example README/script, E06/E10 README, E10 runner, expectation index. No evidence bytes, production crate, design, roadmap, or test-tier code changed. |
| `fe6844ba` attribution | Marcus author preserved; exactly `Co-Authored-By: Codex <codex@openai.com>`; no Claude/Anthropic/generated-by attribution. |
| `cdcc9622` scope | Only `execution-log.json`; appends step 01-03 GREEN and COMMIT PASS at 09:57:58Z/09:58:34Z. |
| `cdcc9622` attribution | Marcus author preserved; exact required Codex trailer; no banned attribution. |
| Commit diffs | `git diff --check` passes for both remediation and DES commits. |
| E10 self-certification | None: README and INDEX remain `pending` through this review. |
| Harness regression checks | Raw validator PASS; injected-Accepted negative REJECT; cleanup/probe oracle PASS; harness branch test PASS. |

## Iteration 3 finding dispositions

| Finding | Status | Disposition |
|---|---|---|
| F1 — E10 failure deploy contract | **CLOSED** | Corrected success-only Accepted boundary is authorized by ADR-0093/review and matches all eight unchanged raw PTYs and exit receipts; negative injection is rejected. |
| F2 — E10 raw history/session/cleanup evidence | **CLOSED** | Raw evidence unchanged; eight-cell validator and focused cleanup/probe oracle remain green. |
| F3 — E06 canonical pin/audit attribution | **OPEN / PARTIALLY RESOLVED** | Lower paragraph fixed, but current-capture line 8 still says the audit occurred on 2026-08-19. Correct that one stale date; do not rerun. |
| F4 — dirty receipt completeness | **CLOSED** | Replacement and historical receipt hashes unchanged; iteration 2 assessment stands. |

## Iteration 3 per-expectation assessment

| Expectation | Evidence assessment |
|---|---|
| E06 | Point-in-time VM Job claim remains supported at `111d404c...` plus dirty runner. One duplicate audit-date sentence remains false (F3). |
| E08 | Unchanged; iteration 1 historical support stands. |
| E09-v2 | Unchanged; iteration 1's exact 20/20, one-control-plane, no-pair-substitution support stands. |
| E10 | **SUPPORTED** for the recorded `a3ebd296` plus dirty-state capture: 8/8 raw cells, correct success/failure PTY semantics and exits, preserved state/history/probes, explicit stop, named cleanup, no-late-overwrite witness, and zero sentinel leakage. Eligible for `satisfied` after this independent adjudication. |
| E11 | Unchanged; iteration 1 historical support stands. |

## Iteration 3 final verdict

**CHANGES_REQUESTED.** E10's point-in-time evidence now supports its corrected
claim, and F1/F2/F4 are closed. Step 01-03 evidence approval is blocked only by
the remaining false `2026-08-19` date in the E06 current-capture status
rationale at `README.md:8`. Correct that one attribution without recapturing
E06 or E10, then return this same reviewer for the next iteration.

---

## Iteration 4 — E06 attribution re-review

### Metadata

| Field | Value |
|---|---|
| Iteration | 4 |
| Final F3 remediation | `eeeb71abbe2d3d79d890777804f5e185fb1281ca` |
| DES persistence | `6d3e7887fbb481174cb58fcdd635c59de194beef` |
| Prior contract remediation | `fe6844bad5d3f73ca2e9dbb77c99ef0efcd6b16b` |
| Raw-evidence remediation | `057434d8755fa79daa636f0177a61a341085582b` |
| Original step evidence | `c353af6c6454be2df1b2417105f2337d6f7364ab` |
| Reviewer | Same step-specific independent evidence reviewer |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-12 |
| Verdict | **APPROVED** |

### Executive assessment

F3 is closed. E06's current-capture status block now consistently identifies
the September 12, 2026 different-fox review, links this review artifact, and
retains the actual `111d404c17f778067d197a07d1acbbc044cd6eaa` dirty-capture
pin. The remediation changes only that attribution sentence. No E06 raw
evidence, status, operator claim, anchor, runner, or other catalogue item
changed.

F1, F2, and F4 remain closed. No expectation evidence path changed after
`057434d8`, and no E10 contract/runner/index path changed after the iteration 3
remediation. The iteration 3 conclusion therefore stands: E10's point-in-time
eight-cell evidence supports its corrected success/failure PTY, exit,
state/history/probe, stop, named-cleanup, no-late-overwrite, and sentinel claim.
E10 remains `pending` in its README and INDEX through adjudication, so no
capture author self-certified it.

No finding remains.

## F3 iteration 4 — E06 current-capture attribution

### Disposition: **CLOSED**

The corrected opening block at `E06/README.md:5-10` now reads coherently:

- current capture date: `2026-09-12`;
- recorded source: `111d404c17f778067d197a07d1acbbc044cd6eaa` with the already
  documented dirty runner;
- audit: “the September 12, 2026 DIFFERENT-FOX adversarial review”;
- review link: `docs/feature/vm-lifecycle-latency/deliver/review-01-03-evidence.md`;
- self-certification boundary: explicitly not stamped by the runner author.

That agrees with the other current-capture references at
`E06/README.md:31-32`, `:76-83`, `:278-290`, and `:304-310`. A complete search
finds no remaining `2026-08-19` reference in the E06 README. The only changed
file in `eeeb71ab` is this README, and its diff changes four lines/adds no new
claim.

The underlying E06 manifest and raw evidence are byte-unchanged. The point-in-
time VM Job acceptance, Running allocation, real VMM resource delta, and zero
owned-resource leak assessment from iteration 1 remains valid. Current EDD
policy does not require a rerun because later metadata commits exist.

F3 is closed.

## Closed-finding preservation

| Finding | Iteration 4 status | Preservation evidence |
|---|---|---|
| F1 — E10 failure deploy contract | **CLOSED** | No E10 README/runner/index or raw-evidence change after `fe6844ba`; ADR-0093 success-only Accepted and raw exit-1 failure assessment stands. |
| F2 — E10 raw history/session/cleanup evidence | **CLOSED** | No evidence-path diff after `057434d8`; exact eight-cell/token/raw-ledger state remains unchanged. |
| F3 — E06 canonical pin/audit attribution | **CLOSED** | Opening and lower current-capture blocks now both identify the September 12 review and actual dirty pin. |
| F4 — dirty receipt completeness | **CLOSED** | No receipt changed; replacement E10 receipt and historical receipt hashes remain those approved in iterations 2/3. |

## Commit, DES, and worktree audit

| Check | Result |
|---|---|
| `eeeb71ab` scope | One file only: `verification/expectations/E06-vm-job-deploy-reaches-running/README.md`; 4 insertions, 3 deletions. |
| `eeeb71ab` attribution | Marcus author preserved; exactly `Co-Authored-By: Codex <codex@openai.com>`; no Claude, Anthropic, or generated-by attribution. |
| `eeeb71ab` diff hygiene | `git diff --check` passes. |
| `6d3e7887` scope | One file only: `docs/feature/vm-lifecycle-latency/deliver/execution-log.json`. |
| DES events | Appends step 01-03 GREEN PASS at `2026-09-12T10:04:51Z` and COMMIT PASS at `2026-09-12T10:05:28Z`. |
| `6d3e7887` attribution | Marcus author preserved; exact required Codex co-author trailer; no banned attribution. |
| `6d3e7887` diff hygiene | `git diff --check` passes. |
| Raw evidence/status preservation | No expectation `evidence/` path changed; E06 status remains `satisfied`, E10 remains `pending` through review. |
| Reviewer write scope | This review artifact only; pre-existing user-owned `AGENTS.md` remains untouched. |

## Iteration 4 per-expectation assessment

| Expectation | Final evidence assessment |
|---|---|
| E06 | **SUPPORTED** at `111d404c...` plus dirty runner; current-capture pin and different-fox attribution are consistent. |
| E08 | **SUPPORTED** for its historical capture; iteration 1 assessment unchanged. |
| E09-v2 | **SUPPORTED** for the third historical attempt: exact 20/20 pairs, one persistent control-plane identity, and no pair-level retry/replacement/discard/cancellation substitution. |
| E10 | **SUPPORTED** at `a3ebd296...` plus its dirty receipt: exact 8/8 raw cells, correct healthy/failure CLI summaries and exits, preserved same-allocation history/probe/restart through stop and cleanup, external no-late-overwrite witness, and truthful named absence. Eligible for `satisfied` after this independent review. |
| E11 | **SUPPORTED** for its historical readiness-recovery capture; iteration 1 assessment unchanged. |

## Iteration 4 final verdict

**APPROVED.** F1-F4 are closed and no new evidence finding is introduced.
E06, E08, E09-v2, E10, and E11 each have sufficient point-in-time evidence for
the claims reviewed here, scoped to their recorded SHA/environment/dirty state.
The historical captures do not need reruns because later commits exist. E10 may
now be transitioned from `pending` to `satisfied` by the owning workflow using
this independent different-fox verdict; this reviewer did not self-stamp or
edit the expectation.
