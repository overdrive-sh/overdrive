# DELIVER review — step 01-03 Native measurements

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Roadmap step | `01-03` — Native measurements |
| Iteration | 1 |
| Review date | 2026-09-12 |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated reviewer selection) |
| Base before step | `111d404c17f778067d197a07d1acbbc044cd6eaa` |
| Primary step commit | `c353af6c6454be2df1b2417105f2337d6f7364ab` |
| Initial DES commit | `bd086b74a6cc43b26fe04ef288d9bbab20c72bde` |
| EDD policy commit | `a3ebd296f2b4a8bfac5fc85145ababc313da1f84` |
| Evidence-remediation commits | `057434d8`, `e97a61a9`, `fe6844ba`, `cdcc9622`, `eeeb71ab`, `6d3e7887` |
| Independent evidence approval | `30cfabea` and `review-01-03-evidence.md` iteration 4 |
| Verdict | **CHANGES_REQUESTED** |

The mandatory repository instructions and the reviewer definition were read in
full. The required `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and
`nw-tdd-methodology` skills were loaded and applied. Repository rules override
the reviewer's legacy two-iteration and YAML-only defaults.

## Contract and reviewed scope

The review used the approved roadmap step, S-VLL-11, S-VLL-12, shared
S-VLL-13, ADR-0102, ADR-0103, the architecture brief, and the current EDD
classification policy. The governing step contract is:

- S-VLL-11 owns exactly 1,200 scheduled warm-host-cache native trials: READY,
  finite Job, and cooperative Service, each with 200 sequential trials and 200
  trials in twenty ten-worker cohorts through one persistent in-process server.
- Every scheduled trial remains in the failure ledger. The nearest-rank
  distributions use the approved boundaries and retain failures rather than
  filtering or retrying them into success.
- READY P95/P99 remain 2.0/3.0 seconds; finite Job P95/P99 remain 0.5/1.0
  seconds; cooperative Service P95/P99 remain 1.0/1.5 seconds. Public stop to
  `vm.lifecycle.stop_enter` is a distinct distribution with no added SLO, and
  terminal observation retains its separate 30-second post-admission bound.
- Normal reaper-observed VMM exit, `/proc` absence, run-directory/cgroup/clone/
  index absence, lifecycle stage clocks, clone-before-index removal, and
  instrumentation-overhead recording must remain independently observable.
- S-VLL-12 retains exactly 20 E09-v2 pairs in two ten-worker cohorts, one
  persistent control-plane identity, independent per-worker advancement from
  its own healthy cleanup to failure submission, no pair retry/drop, and the
  restored 600-second owner and 60-second observation/cleanup windows.
- E06/E08/E10/E11 are point-in-time EDD records, not recurring tests or
  statistical benchmarks. Their status transitions are owned by the separate
  evidence reviewer. S11 remains an in-process native test/benchmark and must
  not emit expectation evidence or spawn the built Overdrive product binary.

Reviewed changes include the S11 test and nextest configuration, the E09-v2
example/runner/scheduler surfaces, the E06 and E10 runner/example fallout, the
receipt harness and host-safe tests, the independent evidence review/status
transition, the DES history, and all named commits. Raw expectation captures
were not re-audited through implementation code; the approved different-fox
artifact was used for that evidence boundary.

## Implementation and test evidence inspected

### S-VLL-11 body and nextest configuration

The activated test has the exact Rustdoc declaration
`/// CONTRACT_SHAPE: bounded-change.` and no `#[ignore]` or expected-panic
attribute. The test is selected by its exact nextest filter. Its scoped
`slow-timeout = { period = "60s", terminate-after = 180 }` supplies a finite
three-hour liveness guard without widening any other test's timeout. The
existing module-level `host-kernel-shared` assignment still includes S11, so
the timeout-only override does not remove its single-writer host-kernel group.

The body establishes one `spawn_vm_server()` before all measured work. It
pre-reads the immutable kernel and staged rootfs, retains fresh allocation IDs,
and schedules:

- READY ordinals 0 through 199 sequentially;
- finite Job ordinals 0 through 199 sequentially;
- cooperative Service ordinals 0 through 199 sequentially;
- ordinals 200 through 399 for each profile in twenty `join_all` cohorts of ten.

The resulting ledger is asserted to contain exactly 1,200 entries, 400 per
profile, with the exact ordinal set `0..400`. A separate uninstrumented READY
control is outside the scheduled population and uses the same persistent
server solely for the design-required instrumentation calibration.

Failure accounting is fail-closed in the live oracle. Deploy errors create a
ledger row; running/terminal timeouts, stop failures, unexpected terminal
state, operator request failure, and driver-artifact residue append to that
row; `failures.is_empty()` is asserted before quantiles are evaluated. Missing
stage events then panic rather than disappearing from a distribution. The
nearest-rank implementation sorts a copy and selects
`ceil(numerator * n / denominator) - 1`; at `n = 400`, P95 and P99 are ranks
380 and 396.

The three primary stage distributions use the approved host-clock events:

| Profile | Implemented stage | Gate |
|---|---|---|
| READY | `vm.lifecycle.create_enter` to `vm.lifecycle.ready` | P95 <= 2 s, P99 <= 3 s |
| finite Job | `vm.beacon.exec.released` to matching `vmm.process.reaped` | P95 <= 500 ms, P99 <= 1 s |
| cooperative Service | `vm.lifecycle.stop_enter` to independently observed full driver-artifact absence | P95 <= 1 s, P99 <= 1.5 s |

Every trial is correlated from `vm.lifecycle.created` to
`vmm.process.reaped`; exit code zero, no signal, and `/proc/<pid>` absence are
independently asserted. `finish_native_trial` observes absence of the run
directory, allocation cgroup, per-launch clone, and clone-index link. Existing
S-VLL-13 clone-index tests retain the temporal clone-before-index guarantee;
S11 does not falsely infer that ordering from final absence alone. The writer,
reaper-to-cleanup, public-stop return, public-stop-to-`stop_enter`, terminal,
and broker-queue distributions remain separate. The public-stop-to-admission
loop has no fixture-local SLO; after `stop_enter`, the existing terminal poll
keeps its 30-second bound.

The test uses the in-process production composition and legitimate Cloud
Hypervisor/guest fixtures. It does not spawn the built Overdrive binary, invoke
an expectation runner, add a public/test seam, or duplicate a production
algorithm. No production/API file changed in this step.

One reporting defect remains and is finding F1 below.

### S11 execution evidence and source identity

Only the uncontended fourth attempt was accepted as a passing execution
signal:

- `.context/vm-lifecycle-latency/s11-reruns/s11-no-admission-slo-20260912-attempt4.log`
  records one selected test passing in 4,849.696 seconds with 169 skipped;
- the paired `.exit` file contains `0`;
- the failed connection/transport attempt and the overlapping attempt that
  enforced the rejected 120-second admission SLO were excluded from the
  verdict and from every quantitative claim.

The post-run E06 dirty-source receipt captured the S11 source as Git blob
`f365bd9126cce52392e11530a634ab5b8017c087`. Re-inserting only the old reasoned
`#[ignore = "pending DELIVER step 01-03: ..."]` line into the current committed
S11 source hashes to that exact blob. The current committed blob is
`c182b6dca97547c403163204964ede41cc6f5256`; therefore the measurement body
that passed attempt 4 is byte-identical to the committed body, and its only
later S11-source change was reasoned activation. No commit after `c353af6c`
changes the S11 file or its measurement logic. The later EDD documentation,
raw-evidence, review, and status commits do not by themselves require another
1,200-trial execution.

The final nextest timeout is also adequate for the observed 80-minute run and
is scoped to S11. The attempt-4 log proves the test executed to completion; it
does not, however, retain the required benchmark report. That distinct
deficiency is F1.

### S-VLL-12 and example/expectation boundaries

The checked-in E09-v2 example remains an operator-runnable product journey.
`run_trial` advances from its own `healthy-cleanup-complete` marker directly to
failure deployment; it does not wait for a cohort-wide healthy cleanup gate.
`run_cohort` starts exactly ten workers, observes all healthy-active and
failure-active markers, joins every PID, and records missing/cancelled results
as `not-run-cancelled` rather than shortening the ledger. `run_suite` executes
two cohorts to cover trials 1 through 20, keeps one `serve` PID/start-tick/
executable identity, requires twenty pass rows, and contains no pair-level
retry loop. The expectation runner retains 600 seconds for the remote owner,
60 seconds for remote cleanup, a bounded transport margin, and extracts the
ledger before failing on an incomplete run.

The implementation boundary remains sound:

- repository-root examples drive public commands and checked-in specs;
- expectation runners drive the built default-feature binary and use external
  shell tools;
- no expectation runner invokes `cargo test`, `cargo nextest`, a Rust test
  binary, or imports/links an `overdrive-*` crate;
- S11 remains in-process and performs the internal event/correlation/cleanup
  assertions that an expectation must not absorb.

The E06 runner's default exports for the selected kernel and rootfs are needed
to make the native preflight and artifact selection fail closed under the
exact harness invocation. E09/E10 changes retain operator outputs, resource
observations, and derived ledgers without moving private lifecycle assertions
out of integration tests.

### Independent point-in-time evidence review

`docs/feature/vm-lifecycle-latency/deliver/review-01-03-evidence.md` contains
four complete native-Markdown iterations and ends **APPROVED**. Commit
`30cfabea` adds that artifact and only then transitions E09-v2 and E10 to
`satisfied` in their README files and the expectation index. The evidence
review independently establishes E09-v2 20/20 with one control-plane identity,
E10 8/8 raw cells with the exact exit/state/history/probe/stop/cleanup/no-late-
overwrite receipts, and the retained E06/E08/E11 point-in-time claims. This
implementation review does not substitute code reading for that raw-evidence
audit and does not demand recurring expectation runs under the updated EDD
policy.

### Receipt exclusion

`verification/harness/run-expectation.sh` now uses a pathspec that excludes
`verification/expectations/**/evidence/**` while diffing from `HEAD`. This
prevents both the output file's self-hunk and recursive inclusion of earlier
evidence while retaining tracked dirty production, configuration, example,
runner, harness, and documentation inputs. `dirty-status.txt` remains the
complete dirty-path inventory. The remediation does not rewrite the four
historical large receipts; the independent evidence review records their
unchanged blob identities. The host-safe receipt regression verifies that a
dirty harness input remains in the receipt while a pre-existing evidence file
does not.

### Quick-bind host-kernel serialization

The first 459-test affected run reproduced a real cross-process collision:
`workload_describe_probes` failed on `alloc-quick-bind-0` while a sibling
module used the same quick-bind Service; a later run reproduced both
`workload_describe_probes` and `service_honest_stable` failing together with
`EarlyExit` on the same allocation-derived identity. Both modules drive a real
`ExecDriver` through the node-global cgroup hierarchy, and their existing
`#[serial(workload_cgroup)]` attributes cannot coordinate nextest's separate
processes. The added override puts exactly those two modules in the existing
`host-kernel-shared` single-writer group. It does not serialize unrelated CLI
tests, relax an assertion, add a retry, or suppress either module. The
post-change affected run passed 459/459 (two already-known leaky warnings).

## Quantitative TDD and Contract Shape review

| Check | Result |
|---|---|
| Distinct step contracts | 3: S-VLL-11, S-VLL-12, shared S-VLL-13 |
| Unit-test budget | 6 (`2 x 3`) |
| New unit tests | 0; the activated Rust test is native integration/benchmark scope and the shell checks are harness integration checks |
| Budget verdict | PASS |
| RED/GREEN/COMMIT | Present in order for 01-03; later evidence remediations truthfully include GREEN FAIL, then GREEN PASS/COMMIT cycles |
| S11 declaration | PASS — exact `/// CONTRACT_SHAPE: bounded-change.` |
| New/transitioned shell declarations | **FAIL — finding F2** |
| Expected panic / ignored activation | PASS — none on S11 |
| Driving boundary | PASS — direct in-process production composition for S11; built-product public CLI for expectations |
| Mocks/test theater | PASS — no mocked SUT, tautological primary oracle, zero-assertion activation, or expected-panic pass |
| Test modification integrity | PASS — the S11 delta adds the required no-SLO stop-entry distribution and replaces a non-authoritative public exit projection with the design-owned Process reason plus independent normal reaper-exit oracle; no accepted requirement is weakened |

The S11 bounded-change test declares the complete scheduled population,
profile membership, failure complement, stage-event presence, normal-exit
complement, and artifact-absence complement. The shell declaration omission
is mechanical and blocks approval even though the scripts execute correctly.

## Verification commands and results

| Verification | Result |
|---|---|
| Mandatory repository/rule/agent/skill reads | PASS |
| `git status --short` before review | Only pre-existing user-owned `AGENTS.md` modified |
| Commit parent/order and attribution audit | PASS — Marcus author preserved; every listed Codex-created commit has exactly one required Codex co-author trailer and no Claude/Anthropic attribution |
| Primary/test/config/source scoped `git diff --check` | PASS; historical raw evidence whitespace intentionally excluded from this source hygiene check |
| `bash examples/service-kind-vm-workloads-v2/test-scheduler.sh` | PASS; includes expected injected cancellation diagnostic |
| `bash verification/harness/test-e09-v2-runner.sh` | PASS |
| `bash verification/harness/test-run-expectation.sh` | PASS |
| `bash verification/harness/test-e10-cleanup-oracle.sh` | PASS; residual cgroup/network/hypervisor/mount and altered-probe controls rejected |
| S11 attempt-4 log/exit inspection | Test PASS in 4,849.696 s; exit 0; benchmark report absent (F1) |
| S11 source hash reconstruction | PASS — `f365bd91...` equals current S11 source plus only the former reasoned ignore marker |
| Reported affected Lima run excluding S11 | PASS — 459/459, two pre-existing leaky warnings |
| Reported Lima check/clippy/fmt logs | PASS — workspace all-target check, `clippy -D warnings`, and format check |
| Host-kernel quick-bind reproduction/remediation logs | Pre-change 1/2 real failures on `alloc-quick-bind-0`; post-change 459/459 PASS |
| Final mutation gate | Not run, correctly: repository policy runs mutation once only after all step reviews are approved |

## Findings

### F1 — The passing S11 run retained no raw ledger, primary quantiles, or instrumentation-overhead record

- **Severity:** Blocker
- **Dimension:** Acceptance completeness / benchmark integrity / external
  validity of reported measurement
- **Locations:**
  `crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:3712`,
  `:3826`, `:4002-4053`; approved contract
  `docs/feature/vm-lifecycle-latency/feature-delta.md:335-383`; attempt-4 log
  `.context/vm-lifecycle-latency/s11-reruns/s11-no-admission-slo-20260912-attempt4.log`
- **Reachable entry path:** the exact roadmap command selects
  `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials` via
  nextest on native metal. The test retains `Vec<NativeTrial>` and captured
  events only in memory, asserts them, prints only secondary stage summaries,
  and returns success. Under the default nextest capture behavior, successful
  stdout/stderr is not present in the retained command log.
- **Bounded reproduction:** the actual successful attempt-4 log is 434 bytes
  and seven lines. It contains only compile/run metadata and the one-test PASS
  summary. Searches for `S-VLL-11 fixture`, `NativeTrial`, `READY`, `finite
  Job`, `cooperative Service`, `p95`, `p99`, `bounded-stage-event`, and
  `S-VLL-11 separate stages` return no match. Source inspection shows no
  serialization/write of the 1,200 ledger. READY/Job/Service distributions and
  the `bounded_event_overhead` value occur only in assertion failure messages;
  the success-path `eprintln!` omits the three primary distributions, the full
  ledger, and the overhead value.
- **Observed failure:** exit 0 proves the in-memory assertions passed for the
  source body, but there is no retained raw measurement from which a reviewer
  can audit exact trial rows, n/min/median/P95/P99/max, stage distributions, or
  instrumentation overhead. This directly misses the accepted benchmark
  requirement to report those values and the full ledger. A benchmark without
  retained raw measurements is not made sufficient by treating its exit code
  as an EDD receipt.
- **Required disposition:** retain a non-EDD benchmark artifact for the full
  scheduled ledger, fixture/source identity, all primary and secondary
  distribution summaries, and the instrumented/uninstrumented overhead
  comparison on the success path. Keep the Rust test in-process and do not
  write into `verification/expectations/`. Because the missing values were not
  retained by attempt 4, the corrected reporting path must be exercised once
  on the qualified native substrate; this rerun is required by the missing
  benchmark data, not by later documentation/evidence commits.

### F2 — New and transitioned shell tests lack their mandatory Contract Shape declarations

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance (mechanical)
- **Locations:** newly added
  `verification/harness/test-e10-cleanup-oracle.sh:1-136` and the transitioned
  receipt-exclusion scenario in
  `verification/harness/test-run-expectation.sh:80-95`
- **Reachable entry path:** the evidence-remediation workflow executes these
  host-safe shell tests directly; this review reproduced both successful
  entries. They are live executable tests added/transitioned in step 01-03,
  not documentation or raw point-in-time evidence.
- **Bounded reproduction:**
  `rg -n 'CONTRACT_SHAPE' verification/harness/test-e10-cleanup-oracle.sh verification/harness/test-run-expectation.sh`
  returns no matches. By contrast, the step's E09-v2 shell tests carry explicit
  `# CONTRACT_SHAPE: bounded-change.` declarations for each live contract.
- **Observed failure:** the tests pass functionally but fail the repository's
  mandatory per-test identity declaration gate. Their bounded change/complement
  oracles therefore have no declared Contract Shape for review.
- **Required disposition:** have the authorized test owner add the exact
  per-test Contract Shape declarations appropriate to the receipt-exclusion,
  cleanup-resource, and probe-preservation contracts without changing or
  weakening their executable assertions. Re-run only these host-safe shell
  tests for this finding.

## Relevant non-findings

- No public method, type, enum variant, trait, parameter, configuration flag,
  or test-only production seam was added. The accepted API shape is unchanged.
- No S11 expected-panic, ignored activation, skipped cohort, fail-fast join,
  retry-to-success, or failure filtering remains.
- The 30-second terminal observation bound was not deleted or converted into a
  Service SLO. Public stop to `stop_enter` is correctly reported separately
  and has no design-invented threshold.
- Normal VMM exit, forced-exit rejection, `/proc` absence, and the four driver
  artifact complements are real independent assertions. Clone-before-index
  ordering remains owned by the existing clone-index regression, not inferred
  from final absence.
- The extra uninstrumented READY control is calibration work outside the 1,200
  scheduled profile population. It is not included in any primary quantile or
  used to replace a failed scheduled trial.
- The quick-bind serialization change is supported by a reproduced cross-
  process real-cgroup collision and is limited to the two modules that share
  the exact allocation/cgroup identity. It is not blanket suppression.
- E09-v2 contains no trial retry. Polling and cleanup attempts are observation/
  cleanup mechanics and do not replace a logical healthy/failure pair.
- The receipt exclusion prevents future evidence recursion while preserving
  tracked dirty source inputs and historical receipts. It does not rewrite old
  evidence.
- The independently approved evidence artifact and `30cfabea` status changes
  follow the different-fox ownership boundary. No implementation author
  self-stamped E09-v2 or E10.
- The two invalid/overlapping S11 attempts are diagnostic history only and were
  not used to establish passing quantiles or cardinality.
- `AGENTS.md` remains modified only in the user's working tree and is absent
  from every reviewed commit.

## Quality-gate summary

| Gate | Verdict |
|---|---|
| G1 — one activated S11 acceptance owner | PASS |
| G2 — semantic RED | PASS — the DES RED record precedes GREEN, and the retained native execution history contains a real assertion failure before the accepted passing attempt |
| G3 — unit assertion RED | Not applicable; no new unit test |
| G4 — no mocks inside the hexagon | PASS |
| G5 — business/domain language | PASS |
| G6 — all required implementation tests green | FAIL for handoff completeness because F1/F2 remain, despite reported runner success |
| G7 — 100% pass before commit | Mechanical run records pass; insufficient to replace F1/F2 |
| G8 — test budget | PASS |
| G9 — no test weakening to accommodate implementation | PASS |
| External validity | PASS for execution boundaries; FAIL for retained S11 benchmark report |
| Test theater | No runtime theater found; F1 is missing retention/reporting, not a vacuous oracle |
| Mutation | Correctly deferred to the single final DELIVER-wave gate |

## Final verdict

**CHANGES_REQUESTED.** The execution logic for S-VLL-11, S-VLL-12, and the
different-fox expectation review is otherwise consistent with the approved
design, and no later documentation/evidence commit changed S11's measurement
body. Approval is blocked by two concrete defects: the sole passing S11 run did
not retain the accepted benchmark ledger/distributions/overhead record, and
the new/transitioned host-safe shell tests lack mandatory per-test Contract
Shape declarations. Both findings must be remediated and re-reviewed before
step 01-03 is approved.

---

## Iteration 2 — benchmark-report and Contract Shape remediation re-review

### Metadata

| Field | Value |
|---|---|
| Iteration | 2 |
| Remediation commit | `fda7043df64e8b80f3b32ec2237eb3cfbe44239b` |
| DES remediation commit | `62563e2de648eb7bb6b0d25e468bb9ccdbc791d0` |
| Prior verdict | **CHANGES_REQUESTED** |
| Current verdict | **APPROVED** |
| Reviewer | Same step-specific fresh isolated `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated reviewer selection) |
| Review date | 2026-09-12 |

Iteration 2 re-reviewed only F1/F2 and checked that the previously accepted
S-VLL-12, different-fox evidence, receipt, quick-bind serialization, API/design,
commit, and DES conclusions remain intact. No implementation, test,
configuration, expectation evidence, design, roadmap, DES-log, `AGENTS.md`, or
commit was edited by the reviewer.

### Remediation scope and attribution

`fda7043d` changes exactly four files:

- `.config/nextest.toml` adds S11-only successful-output retention;
- `vm_stop_restart_and_vmm_death.rs` adds the test-private benchmark report,
  its atomic writer, a synthetic report test, and the live report write before
  failure assertions;
- `test-e10-cleanup-oracle.sh` adds declarations only;
- `test-run-expectation.sh` adds a declaration only.

It adds no production file, public API, expectation record, raw evidence,
design, roadmap, or EDD status change. `62563e2d` changes only
`docs/feature/vm-lifecycle-latency/deliver/execution-log.json`, appending GREEN
PASS at `2026-09-12T12:34:09Z` and COMMIT PASS at
`2026-09-12T12:34:44Z`; all earlier events, including the truthful evidence-
remediation GREEN FAIL followed by PASS, remain present.

Both commits preserve Marcus Schack Abildskov as author, carry exactly
`Co-Authored-By: Codex <codex@openai.com>`, contain no Claude/Anthropic or
generated-by attribution, and exclude the pre-existing dirty `AGENTS.md`.
Scoped `git diff --check` passes.

## F1 re-review — retained S11 native benchmark report

### Disposition: **CLOSED**

The sole accepted remediation execution is uncontended attempt 5. Its retained
files are:

- `.context/vm-lifecycle-latency/s11-reruns/attempt5/s11-native-20260912-attempt5.json`;
- the paired `.log` with success output;
- the paired `.exit` containing `0`.

The JSON is 9,715,410 bytes, newline-terminated, parses successfully, declares
schema `overdrive.vm-lifecycle-latency.s11.native.v1`, and independently hashes
to
`f0868cd60cc3529e2798ef234d181df0f56f0c1f9e99e68afa8bffcecc16da4b`.
The successful nextest log reports the identical path, hash, size, 1,200 trial
count, zero failure count, overhead values, and all nine distribution summaries
before the one-test PASS summary at 5,890.894 seconds. No attempt-5 `.tmp` file
remains in the retained local artifact directory.

### Fixture, source, and execution identity

The report contains all required identity fields:

| Identity | Retained value |
|---|---|
| Source file | `/home/ubuntu/overdrive/crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs` |
| Source SHA-256 | `d417b62778dda073cd49b64db500c926e856171105191e4d5f28842a37419213` |
| Current/remediation source SHA-256 | Exact match to the retained source hash |
| In-process binary | `/home/ubuntu/overdrive/target/debug/deps/integration-3c1c2270a562a6ab` |
| In-process binary SHA-256 | `de0bfd544bebb514a431c603e9e9739c40e589d30cec6d184e6f290feb8bf600` |
| Host/kernel | Native x86_64 AMD EPYC 8024P; Linux `7.0.0-29-generic`; 65,401,772 KiB RAM |
| Guest kernel SHA-256 | `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682` |
| Staged rootfs | 67,108,864 bytes; SHA-256 `3be2a043f65e00598540c6fc68824566e98d62d98acc0b7fb2b285142464d6ab` |
| Guest init SHA-256 | `796c71d5d2b555e7c61c46ff69dc854bdcb8bb006775ee670c9dd874700e8679` |
| Cooperative server SHA-256 | `a4d628031b696ef9ecf9d422c36c6f2e45bdd04a894af07814ac0a0249c43e9f` |
| Cloud Hypervisor | v53.0; SHA-256 `448af3d4e59b22c2987f7df94c213ad40fb53a10d437e42b5ee6c4fce7c29ecc` |
| Resource/cache profile | `cpu_milli=500`, 128 MiB, `warm-pre-read` |

The source hash is computed from the exact S11 test source on the leased remote
checkout and matches the committed remediation file byte-for-byte. The binary,
guest init, immutable guest kernel/rootfs, cooperative server, and Cloud
Hypervisor hashes independently bind the remaining executable/fixture inputs.
No later source commit changes the S11 body after `fda7043d`; `62563e2d` is
DES-log-only.

### Independent ledger and schedule validation

Independent `jq` aggregation over the retained raw rows produced:

| Check | Result |
|---|---|
| Total trial rows | 1,200 |
| Unique workload IDs | 1,200 |
| Unique allocation IDs | 1,200 |
| Successful rows | 1,200 |
| Rows with a nonempty failure list | 0 |
| Top-level failure records | 0 |
| Rows per profile | READY 400; finite Job 400; cooperative Service 400 |
| Rows per profile/mode | 200 sequential and 200 concurrent for every profile |
| Ordinals | Exactly 0 through 399 once per profile |
| Concurrent cohorts | 60 profile/cohort groups; cohorts 0 through 19; exactly ten unique workers in each |
| Mode/cohort/worker mapping defects | 0 |
| Workload/allocation identity-format defects | 0 |

The report's schedule declares 1,200 scheduled trials, 400 per profile, 200
sequential, 200 concurrent, twenty concurrent cohorts, ten workers per cohort,
and one persistent in-process server. The raw rows independently agree with
every declared value. The uninstrumented ordinal 9,999 calibration control is
retained separately under `overhead_comparison` and does not appear in or
replace any scheduled profile row.

### Independent distribution recomputation

The reviewer recomputed nearest-rank median/P95/P99 and min/max directly from
the raw per-trial duration vectors. All nine recomputed summaries are exactly
equal to the reported summaries:

| Distribution | n | min ns | median ns | P95 ns | P99 ns | max ns |
|---|---:|---:|---:|---:|---:|---:|
| READY create-to-ready | 400 | 1,122,178,910 | 1,151,722,591 | 1,526,082,261 | 1,665,804,228 | 1,714,079,658 |
| finite Job EXEC-release-to-reap | 400 | 34,971,536 | 44,610,444 | 117,875,595 | 182,715,577 | 252,681,273 |
| cooperative Service stop-entry-to-artifact-absence | 400 | 54,079,125 | 198,779,255 | 355,818,231 | 414,032,217 | 481,112,253 |
| admission queue | 354,305 | 0 | 5,959,000,000 | 147,739,000,000 | 195,862,000,000 | 267,539,000,000 |
| stop-entry-to-cleanup-calls | 800 | 25,572,389 | 49,954,076 | 104,118,980 | 130,403,722 | 186,952,805 |
| VMM-reap-to-cleanup-calls | 1,200 | 404,719 | 808,046 | 22,148,620 | 58,048,930 | 119,002,382 |
| public stop return | 800 | 41,495,395 | 43,914,316 | 90,931,942 | 143,102,033 | 170,433,151 |
| public-stop-to-admission | 800 | 0 | 1,353,483,992 | 201,627,865,208 | 265,535,613,907 | 267,190,500,566 |
| public-stop-to-terminal observation | 800 | 46,085,163 | 1,596,726,428 | 201,830,480,454 | 265,647,504,587 | 267,432,689,255 |

The three primary P95/P99 values are below their approved 2/3-second,
0.5/1-second, and 1/1.5-second thresholds. Public-stop-to-admission is retained
as a distinct non-SLO distribution and is not pooled into cooperative-Service
latency. The long queue/admission tails are reported rather than filtered or
treated as a new product failure.

### Exit, cleanup, stage, and overhead complements

Independent scans find zero trial rows violating any of these complements:

- at least one convergence completion;
- matching VMM reaper exit `Some(0)`, signal `None`, and `/proc` absence;
- run-directory, allocation-cgroup, rootfs-clone, and clone-index absence;
- non-null primary duration;
- `completed` writer disposition and all public-stop distributions for every
  READY and cooperative-Service trial;
- the finite-Job EXEC-release boundary and no substituted stop duration;
- monotonic create/created/reap/cleanup/artifact timestamps;
- profile-specific READY, EXEC-release, stop-entry, writer, and VMM ordering;
- exact equality between each primary duration and its raw stage-timestamp
  subtraction, plus equality between reaper-to-cleanup duration and its raw
  timestamps.

All 400 READY and 400 cooperative-Service terminal rows are operator-stopped;
all 400 finite Jobs are process-stopped. Eighty-five finite-Job rows carry the
explicit `Completed { exit_code: 0 }` projection and 315 carry the accepted
`None/None` public projection; every one independently carries a normal VMM
reaper exit, so no missing public projection is substituted for process
completion.

Instrumentation is explicitly retained:

- instrumented control: 2,000,433,747 ns;
- uninstrumented control: 1,996,174,007 ns;
- saturating difference: 4,259,740 ns.

Independent arithmetic confirms the reported difference exactly.

### Report-path integrity and boundary

The report writer serializes pretty JSON, creates a same-directory
PID-qualified temporary file with `create_new`, writes and newline-terminates
it, `sync_all`s the file, atomically renames over the requested final path,
syncs the containing directory, removes a temporary file on every error path,
and hashes/stats the completed path. The default location is
`target/benchmark-reports/vm-lifecycle-latency/s11-native.json`; the native run
uses the explicit non-repository-evidence path
`/srv/vm/overdrive-testing/s11-native-20260912-attempt5.json`.

The test builds and atomically writes the complete report before asserting the
collected trial failures and report-builder failure complement. Thus a
completed trial/stage/VMM/cleanup failure is durable before the failing
assertion. `success-output = "immediate"` is scoped to the exact S11 nextest
selector and the attempt-5 log demonstrates that fixture identity, receipt,
overhead, and all summaries survive a successful test.

The new synthetic test has exact
`/// CONTRACT_SHAPE: bounded-change.`. It writes and replaces the same report
twice, proves stable SHA-256 and size, parses the completed JSON, verifies the
row/stage/terminal/VMM/cleanup/distribution/overhead schema, requires a trailing
newline, and proves the report directory contains no temporary file. Its final
retained Lima run passes 1/1 in 0.009 seconds.

This is test-private benchmark machinery in the existing native integration
module. It adds no public production/test seam, does not spawn the built
Overdrive binary, does not invoke an expectation runner, and writes neither
raw nor derived content into `verification/expectations/`. The benchmark
artifact therefore remains distinct from EDD point-in-time records and from
black-box expectation execution.

F1 is closed. Attempt 5 is valid and no further 1,200-trial rerun is required.

## F2 re-review — shell Contract Shape declarations

### Disposition: **CLOSED**

The remediation adds exactly these declarations:

- `test-e10-cleanup-oracle.sh:74` —
  `# CONTRACT_SHAPE: bounded-change.` for already-absent/observed-then-absent
  cleanup and the complete retained-resource rejection complement;
- `test-e10-cleanup-oracle.sh:115` —
  `# CONTRACT_SHAPE: bounded-change.` for monotone observation time with exact
  probe semantics preserved;
- `test-run-expectation.sh:80` —
  `# CONTRACT_SHAPE: bounded-change.` for retaining dirty source input while
  excluding the receipt itself and every expectation-evidence path.

The commit changes no shell assertion, fixture, command, expected result, or
test control flow beside those comments. The declarations precisely name the
bounded delta and its complement rather than applying one vague file-level
label to unrelated contracts.

The reviewer reran only the two relevant host-safe scripts:

| Command | Result |
|---|---|
| `bash verification/harness/test-e10-cleanup-oracle.sh` | PASS; clean controls accepted, residual cgroup/network/hypervisor/mount and changed/regressive probe controls rejected |
| `bash verification/harness/test-run-expectation.sh` | PASS; dirty source retained and evidence/self-recursion excluded |

F2 is closed.

## Preservation of iteration-1 accepted conclusions

- S-VLL-12 remains the same 20-pair/two-cohort, concurrency-ten example and
  expectation runner. Neither remediation commit touches it or its approved
  point-in-time evidence.
- `review-01-03-evidence.md` remains APPROVED after four iterations; E09-v2 and
  E10 status transitions remain owned by `30cfabea` and are untouched.
- The receipt pathspec and its historical-receipt non-rewrite remain unchanged.
- The quick-bind nextest assignment remains the same minimal two-module
  serialization backed by the reproduced real-cgroup cross-process race.
- The S11 module retains its existing `host-kernel-shared` membership. The new
  success-output setting changes capture visibility only.
- No public API, production behavior, architecture owner, expectation/test
  boundary, mutation exclusion, or roadmap scope changed.
- The final mutation gate remains correctly deferred until this implementation
  review is approved.

## Iteration-2 verification summary

| Verification | Result |
|---|---|
| Attempt-5 JSON SHA-256 | PASS — exact expected `f0868cd6...16da4b` |
| Attempt-5 JSON size/schema/newline | PASS — 9,715,410 bytes; v1; newline-terminated |
| Log/exit/receipt agreement | PASS — exit 0; hash, size, trials, failures, overhead, summaries identical |
| Source hash against committed remediation file | PASS — exact `d417b627...419213` |
| Raw ledger cardinality/identity/schedule | PASS — 1,200 unique rows; exact 400/200/200/cohort/worker sets |
| Raw failure complement | PASS — 1,200 successes; zero row failures and zero failure records |
| Independent distribution recomputation | PASS — all nine summaries byte-value equal to report |
| Primary targets | PASS — READY, Job, and Service P95/P99 all within approved limits |
| VMM `/proc`/cleanup/stage complements | PASS — zero bad rows |
| Instrumentation comparison | PASS — exact 4,259,740 ns difference |
| Atomic report writer and synthetic test | PASS |
| Successful output retention | PASS — complete receipt and summaries in log |
| Temporary residue | PASS — none retained; synthetic test checks replacement cleanup |
| EDD/test boundary | PASS |
| Shell declarations and unchanged assertions | PASS |
| Host-safe remediation scripts | PASS |
| Commit scope/attribution/diff hygiene | PASS |
| DES remediation events | PASS |
| Dirty work preservation | PASS — only user-owned `AGENTS.md` plus this untracked review artifact |

## Iteration-2 findings

No open or new findings.

| Prior finding | Status | Disposition |
|---|---|---|
| F1 — missing retained S11 benchmark report | **CLOSED** | Attempt 5 retains and independently validates the complete v1 raw report and successful receipt; no further native rerun required. |
| F2 — missing shell Contract Shape declarations | **CLOSED** | Three exact per-contract declarations added without assertion changes; both host-safe scripts pass. |

## Iteration-2 final verdict

**APPROVED.** F1 and F2 are closed, no new defect is found, and all iteration-1
accepted conclusions remain valid. Step 01-03 now satisfies S-VLL-11,
S-VLL-12, shared S-VLL-13, the approved API/design shape, the repository's
example/expectation/integration-test boundaries, and the DELIVER review gate.
The final wave-level mutation gate may proceed under the repository's strict
sequence.
