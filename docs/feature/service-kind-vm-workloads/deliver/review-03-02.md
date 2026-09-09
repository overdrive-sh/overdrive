# DELIVER implementation review — step 03-02 E12 liveness restart

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Roadmap step | `03-02` — E12 liveness restart |
| Reviewed commit | `d850c94dc69d3bef7d6cb962210672efb67b0686` |
| Reviewed parent | `b3a1fff9a23c44a79c16d27b388debc10d8203c4` |
| Reviewer | Fresh isolated implementation reviewer, Codex |
| Review iteration | 1 |
| Review date | 2026-09-09 |
| Final verdict | **NEEDS_REVISION** |

This review covers the complete as-landed step diff and the current production
composition. The only file written by this reviewer is this Markdown artifact.
The pre-existing dirty changes in `AGENTS.md` and
`docs/feature/service-kind-vm-workloads/deliver/execution-log.json` were
preserved; the post-commit `03-02` COMMIT event was read but not staged or
changed.

## Authority and review boundary

The reviewed contract is the approved `03-02` roadmap entry, together with:

- `docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md`;
- `docs/feature/service-kind-vm-workloads/feature-delta.md`;
- `docs/feature/service-kind-vm-workloads/design/wave-decisions.md`;
- `slices/slice-03-vm-service-readiness-liveness.md`;
- `distill/test-scenarios.md` and `distill/adr-0101-acceptance.md`; and
- the approved upstream `deliver/review-03-01.md`.

The closed step contract is precise: `ServiceLifecycle` detects the declared
liveness threshold and emits only the existing liveness
`StopAllocation { terminal: Stopped { by: LivenessProbe } }`; `WorkloadLifecycle`
alone observes that terminal row and chooses existing-budget restart or final
failure; E12 must show the liveness terminal, ordinary replacement startup,
no readiness-owned restart, no dead revival, and zero cleanup deltas. The
step adds no lifecycle policy, persisted state, or public API. E12's
catalogue status must remain `pending` until the separate independent native
evidence audit; this review does not perform or claim that audit.

## Contract and ownership audit

| Obligation | Evidence | Result |
|---|---|---|
| Liveness failures below threshold do not stop; a Pass resets the streak; a new streak reaches the existing StopAllocation threshold | `service_kind_vm_workloads.rs:339-458`; `service_lifecycle.rs:969-1082` | **PASS** |
| ServiceLifecycle emits only the existing liveness stop and does not choose restart/finalization | `service_lifecycle.rs:969-975,1020-1081`; S-SVM-21A assertions | **PASS** |
| WorkloadLifecycle alone chooses same-ID restart or typed liveness final failure under the existing budget | `service_kind_vm_workloads.rs:466-508`; `workload_lifecycle.rs:858-975,1340-1383` | **PASS** |
| Restart action uses the existing stop/start path and ordinary VM driver hooks; no second probe/lifecycle owner is introduced | `action_shim/mod.rs:2253-2296,2745-2791`; `vm_driver.rs:1894-1904`; `probe_runner/mod.rs:298-370` | **PASS in production composition** |
| E12 visibly proves the liveness terminal and declared threshold before replacement | E12 runner `run-example.sh:664-693`, `runner.sh:55-74`; native transcript `product-run.out:54-74,98-104` | **FAIL — F-02** |
| E12 proves a fresh ordinary replacement startup observation | E12 runner `run-example.sh:680-687`; native transcript `product-run.out:51,95` | **FAIL — F-01** |
| Readiness is not the restart owner | S-SVM-21A action exclusion; E12 runner `run-example.sh:780-782`; final transcript has no readiness failure | **PASS for the covered owner path** |
| Dead allocation does not become eligible again | Upstream S-SVM-17 seeded terminal invariant; E12 public capture has only `last terminated` history | **Internal invariant PASS; E12 black-box claim is not independently established (included in F-02)** |
| Teardown leaves no example-owned leaks | final native transcript `product-run.out:105-108` | **PASS mechanically** |
| No new public API, lifecycle state, persistence shape, or policy | full commit diff; no lifecycle production source changed | **PASS** |

## Production entry-point and owner-path review

The implementation does not move lifecycle ownership. The current path is:

1. `ServiceLifecycle::collect_liveness_actions` calls
   `liveness_terminate_action` for a Running allocation. The counter is
   incremented only for `ProbeStatus::Fail`, removed on `Pass`, left unchanged
   on `None`, and the action is emitted only when the threshold predicate is
   true (`crates/overdrive-reconcilers/src/service_lifecycle.rs:1001-1081`).
2. The existing Stop action performs driver stop, mTLS/network cleanup, and
   writes the terminal row before releasing the VM supervision hook
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:2811-2948`).
3. The existing WorkloadLifecycle restart branch selects a terminal
   liveness-stopped row while budget remains, builds
   `Action::RestartAllocation` with the same allocation ID, and emits typed
   `ServiceFailed { LivenessProbeFailed }` only at the existing ceiling
   (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:858-975`).
4. The restart shim executes the existing stop/provision/start sequence, writes
   the Running row with `last_terminated` and incremented `restart_count`, and
   invokes `driver.on_alloc_running`; the VM driver delegates to the existing
   `ProbeRunner` (`action_shim/mod.rs:2253-2296,2545-2586,2745-2791`; `vm_driver.rs:1894-1904`).

No cancellation, forced abort, detached future, new restart cause, or new
public method was added. The S-SVM-21A/B tests exercise the pure owner
boundaries without adding a production seam. S-SVM-21B uses the generic
existing WorkloadLifecycle policy with an Exec driver; the VM-specific
driver/start path is supplied by the E12 product journey, so this is not a
cross-driver API substitution.

## Design and API-shape audit

The commit conforms to the closed design surface. It activates the two
pre-authored reconcilers tests and adds only the checked-in product example,
E12 runner/documentation/evidence, catalogue bookkeeping, and DES state. No
public method, type, enum variant, trait parameter, persisted field, CLI
command, broker target, VM lifecycle state, or alternate restart authority was
introduced. The same-ID replacement represented in the ledger matches the
feature delta and wave decisions (`feature-delta.md:661-664`,
`wave-decisions.md:106-109`).

The two transitioned tests each carry the exact required declaration
`/// CONTRACT_SHAPE: bounded-change.`. Their assertions are non-vacuous:
S-SVM-21A checks below-threshold silence, Pass reset, exact one StopAllocation,
terminal cause, counter clearing, and absence of restart/finalization; S-SVM-21B
checks same-ID restart, restart count, ServiceLifecycle enqueue, ceiling
finalization, and no RestartAllocation after exhaustion.

The Rust tests remain in-process and do not launch the built product binary or
the expectation harness. E12 remains black-box: the runner drives the
checked-in example and default-feature `overdrive` binary, imports no
`overdrive-*` crate, invokes no Rust test binary, and does not recreate the
probe/workload specification inline. The required boundary is therefore
preserved; the findings below concern what that black-box oracle claims, not a
boundary violation.

## Native E12 evidence and provenance

The retained final receipt records `expectation_id: E12`, native metal,
`executed_in_lima: false`, runner exit `0`, the parent product SHA, and
`working_tree_dirty: true` (`verification/expectations/E12-vm-service-liveness-restart-describe/evidence/verification.yaml:1-16`).
The dirty status and full dirty patch are retained. The two earlier failed
captures remain under `evidence/attempt-1` and `evidence/attempt-2`; neither
was overwritten or silently converted into the final pass. The final
`product-run.meta` records the bounded native-metal invocation and cleanup
grace. This is genuine product evidence, but the acceptance oracle is not
honest enough to approve for the two findings below.

The final transcript records these concrete facts:

| Phase | Transcript evidence | What it actually establishes |
|---|---|---|
| Before | `product-run.out:33-53` | One Stable Service, allocation Running, restart count 0, and all three probe roles passing at `last_observed_at=1788965690507`. |
| “Terminal” sample | `product-run.out:54-74` | The same allocation is still `Running` with restart count 0. It contains one liveness Fail (`HTTP 503`) at `1788965692513`, but no terminal allocation row. |
| After | `product-run.out:75-97` | Same allocation is Running with restart count 1 and a generic `last terminated: ... stopped` history. Startup remains `last=pass` at the **unchanged** timestamp `1788965690507`; readiness and liveness are later observations at `1788965705547`. |
| Ledger/cleanup | `product-run.out:98-108` | The shell-generated ledger labels the Running sample `terminal` and hard-codes `liveness-probe`; the final zero-delta teardown is present. |

The checked-in VM fixture declares `failure_threshold = 2`
(`examples/service-kind-vm-workloads/liveness-restart.toml:35-41`), so the
single failed observation in the saved “terminal” describe cannot itself show
that the threshold was reached. The final receipt appropriately leaves the
catalogue status `pending`; no independent different-fox audit is claimed.

## DES, commit, scope, and formatting audit

The live execution log contains the complete ordered step trace:

| Phase | Status | Decision | Timestamp |
|---|---|---|---|
| RED | `EXECUTED` | `PASS` | `2026-09-09T14:40:06Z` |
| GREEN | `EXECUTED` | `PASS` | `2026-09-09T14:56:45Z` |
| COMMIT | `EXECUTED` | `PASS` | `2026-09-09T14:58:54Z` |

`PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity
docs/feature/service-kind-vm-workloads/deliver` reports all nine steps have
complete DES traces. The COMMIT event is legitimately dirty because it was
recorded after `des-commit` and was not staged by this review.

Commit metadata preserves the existing Git author and contains exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 03-02`
line. No Claude, Anthropic, or generated-by attribution is present. The 27
changed paths are step-related acceptance tests, example/runner, E12 evidence
and documentation, catalogue bookkeeping, and DES bookkeeping. No unrelated
production subsystem is included.

`cargo fmt --all -- --check`, workspace check, and clippy pass. `git diff
--check` reports only the repository's expected whitespace in retained PTY
transcripts and dirty patches; no Rust, shell, Markdown, or configuration
source formatting defect was found.

## Findings — iteration 1

### F-01 — E12 accepts a stale startup result as ordinary replacement startup

**Dimension:** Completeness / evidence-oracle integrity  
**Severity:** High — blocking the E12 acceptance criterion  
**Status:** Open

The E12 contract says the same-ID replacement follows the ordinary VM
Running/startup path. The runner's replacement predicate only greps for any
`startup probe[0] ... last=pass` in the current describe
(`examples/service-kind-vm-workloads/run-example.sh:680-687`). It never checks
that the replacement produced a new startup observation after the restart.

The final native product run demonstrates the false-positive condition through
the real owner path, rather than a hypothetical test state: the baseline
startup observation is `last_observed_at=1788965690507`
(`product-run.out:50-53`), and the post-restart describe still reports
`last_observed_at=1788965690507` (`product-run.out:94-97`) while the replacement
has `restart_count=1` (`:78-81`). Readiness has a later timestamp, so this is
not a transcript-wide clock freeze. The runner exits successfully despite
accepting this unchanged startup row.

The production path that makes this proof obligation reachable is concrete.
The restart implementation itself documents that RestartAllocation is a fresh
process spawn and that startup timing is measured from the new Running
transition (`crates/overdrive-control-plane/src/action_shim/mod.rs:2525-2530`):
the liveness Stop path removes the VM supervisor and writes the terminal row
(`action_shim/mod.rs:2811-2948`), WorkloadLifecycle emits same-ID Restart, and
the restart shim invokes `on_alloc_running` for the new VM attempt
(`action_shim/mod.rs:2745-2791`; `vm_driver.rs:1894-1904`). The
ServiceLifecycle view also deduplicates an already-announced same-ID Stable
entry at `service_lifecycle.rs:497-503`. Whether retaining the old startup
observation is an intentional same-ID semantic or a production lifecycle gap
is not inferred here; the captured E12 claim is simply not established.

**Reproduction:** the retained native-metal run is a bounded production-owner
reproduction: `verification.yaml` records runner exit `0`, while the before and
after public describes contain the exact equal startup timestamp above. No
test-only abort, seeded row, or hypothetical schedule is involved.

**Smallest remediation:** make the E12 evidence predicate prove a
post-restart startup observation—using an existing public timestamp/observation
surface—and retain a capture where that predicate passes. If the existing
same-ID lifecycle intentionally reuses the startup observation, surface that
as a DESIGN clarification and correct the E12 contract; do not add a new
generation field or public API merely to satisfy this review. The original
step crafter must own any example/runner correction and refreshed capture.

### F-02 — The E12 “terminal” row is sampled before threshold/terminal publication and its cause is hard-coded

**Dimension:** Completeness / testing-theater and evidence integrity  
**Severity:** High — blocking the liveness-owner and S-SVM-28 proof  
**Status:** Open

`wait_for_liveness_restart` copies the first describe containing a liveness
failure (`examples/service-kind-vm-workloads/run-example.sh:664-677`). It does
not wait for an `AllocState::Terminated|Failed` current row, an observed
`Stopped { by: LivenessProbe }` terminal, or any threshold witness. The
runner explicitly permits `Running` for the row it calls `terminal`
(`verification/expectations/E12-vm-service-liveness-restart-describe/runner.sh:60-65`),
then prints `terminal_reason liveness-probe` literally rather than deriving it
from an observed terminal surface (`run-example.sh:800-807`).

The final native transcript proves the mismatch: the saved terminal sample is
`Running` with `restarts 0` (`product-run.out:54-74` and ledger `:101`), and
contains only one failed liveness observation. The fixture's declared
threshold is two. The later `last terminated` and restart count 1 show that a
terminal predecessor eventually existed (`product-run.out:78-82`), but the
public output only calls it generic `stopped`; it does not connect that row to
the liveness threshold. The shell ledger therefore claims a liveness-caused
terminal transition that the recorded describe never observed.

This is a reachable proof defect on the actual production path. The
ServiceLifecycle threshold code requires the second failure before emitting
the Stop action (`service_lifecycle.rs:1020-1081`); only then can the action
shim publish the terminal row consumed by WorkloadLifecycle
(`action_shim/mod.rs:2811-2948`; `workload_lifecycle.rs:858-975`). The final
runner captures before that publication and still passes. It is not a claim
that the production owner chose the wrong action; it is a claim that E12 does
not prove the action it reports.

The same gap weakens the black-box “dead allocation never becomes eligible”
claim: the runner checks `last terminated` and absence of a readiness failure,
but observes no backend eligibility or peer-traffic outcome and does not
assert a terminal allocation. Upstream S-SVM-17 remains valuable independent
in-process safety evidence, but it cannot turn this hard-coded E12 ledger
attribution into the required public terminal/restart sequence.

**Reproduction:** the retained final native capture is a bounded
production-owner reproduction of the false pass: runner exit `0` with a
Running/restart-0 row in the field named `terminal`, one failure against a
threshold of two, and a literal liveness attribution. The two retained failed
attempts do not repair this final oracle defect.

**Smallest remediation:** have the example/runner capture the existing
terminal publication before the replacement (or another already-approved
black-box observable that binds the replacement to the terminal row), require
the terminal state/threshold evidence, and derive—not hard-code—the ledger's
terminal attribution. Add the existing eligibility/traffic observable only if
it is already part of the approved E12 contract; if the current public
surface cannot expose the required distinction, return the gap to DESIGN
instead of inventing a new API or lifecycle state. The original step crafter
must make this bounded evidence correction and recapture E12.

## Non-findings and dispositions

| Candidate | Disposition |
|---|---|
| S-SVM-21A counter increment/reset/threshold semantics | Not a finding. The focused test asserts the exact existing Stop action, Pass reset, and no restart/finalization; production code matches. |
| S-SVM-21B use of an Exec driver in the pure WorkloadLifecycle policy test | Not a finding. The test exercises the driver-neutral existing policy owner; E12 supplies the VM driver/start/cleanup boundary. |
| Readiness-owned restart | Not a finding. The accepted ServiceLifecycle test excludes Restart/Finalize from the liveness detector, and the E12 capture contains no readiness failure; no reachable readiness restart was reproduced. |
| VM watcher/session ownership or same-ID cleanup redesign | Not a finding in this step. No new ordering defect was reproduced; the review does not prescribe architecture outside the approved lifecycle path. |
| E12 catalogue status | Not a finding. `README.md` and `INDEX.md` correctly leave E12 `pending`; this review does not claim the separate independent evidence audit. |
| Native transcript whitespace | Not a finding. The retained PTY alignment/trailing bytes and dirty patches are verbatim evidence, consistent with repository convention. |
| Mutation testing | Not run, as explicitly prohibited during an individual roadmap step. |

## Verification performed

| Command or evidence | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(vm_liveness_threshold_emits_only_the_existing_liveness_stop) or test(workload_lifecycle_alone_decides_restart_after_liveness_stop)'` | **PASS** — 2/2 focused S-SVM-21A/B tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(service_kind_vm_workloads)' --no-fail-fast` | **PASS** — 6 tests, 1 skipped. |
| `cargo xtask lima run -- cargo fmt --all -- --check` | **PASS**. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | **PASS**. |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | **PASS**. |
| `bash -n examples/service-kind-vm-workloads/run-example.sh verification/expectations/E12-vm-service-liveness-restart-describe/runner.sh && examples/service-kind-vm-workloads/run-example.sh check-source` | **PASS**. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/service-kind-vm-workloads/deliver` | **PASS** — all nine steps have complete DES traces. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | **PASS** for roadmap format. |
| Required native `verification/harness/run-expectation.sh E12` receipt and retained final evidence | **Mechanically PASS** — native-metal, runner exit 0, zero cleanup delta; acceptance oracle is rejected by F-01/F-02. No native rerun was performed by this reviewer. |
| `git diff --check b3a1fff9a23c44a79c16d27b388debc10d8203c4..d850c94dc69d3bef7d6cb962210672efb67b0686` | **Expected evidence-only whitespace findings** in retained PTY output/dirty patches; no source formatting defect. |

No production, test, evidence, design, DES, staging, or commit file was
modified by this review other than this artifact.

## Remediation disposition and verdict

F-01 and F-02 are proven, bounded acceptance-oracle defects in the current
native production capture. They do not authorize a new public API, lifecycle
state, persistence mechanism, restart policy, or architecture. The original
03-02 crafter must correct the example/runner predicates within the approved
contract, produce fresh honest E12 evidence, and return this same step to its
step-specific reviewer. The independent native evidence audit remains a
separate gate after the implementation review is approved.

**NEEDS_REVISION.** The in-process liveness owner tests, production owner
boundaries, public API shape, DES trace, commit metadata, and cleanup delta are
otherwise compliant. Approval is blocked because the final E12 pass accepts a
stale startup observation and labels a pre-terminal first failure as a
liveness-caused terminal allocation without observing the threshold/terminal
publication.

## Review iteration 2 — remediation re-review

### Iteration metadata and scope

| Field | Value |
|---|---|
| Review iteration | 2 (same step-specific reviewer) |
| Remediation commit | `654e10e81dc46b9b31128339704b864c06def800` |
| Remediation predecessor | `d850c94dc69d3bef7d6cb962210672efb67b0686` |
| Review date | 2026-09-09 |
| Review scope | Full remediation range, current composition, live DES log, and retained fresh E12 native-metal receipt |
| Final iteration verdict | **APPROVED** |

This iteration re-opened both iteration-1 findings against the exact
remediation range. The review did not edit production code, tests, evidence,
design, DES state, staging, or either commit. The only write remains this
Markdown artifact. The live post-remediation `03-02` COMMIT DES event is
legitimately dirty after `des-commit` and was not staged.

### Authority and review boundary

The same accepted contract governs this re-review: ADR-0101, the feature delta,
wave decisions, Slice 03, the S-SVM scenarios and ADR-0101 acceptance, and the
approved 03-01 review. In particular:

- `ServiceLifecycle` only emits the existing liveness `StopAllocation` after
  the declared failure threshold; a successful observation resets its existing
  counter.
- `WorkloadLifecycle` alone chooses same-ID restart or final liveness failure
  under the existing budget.
- E12 must show the terminal liveness transition before ordinary replacement
  startup, no readiness-owned restart, no dead revival, and zero cleanup.
- No new public API, lifecycle policy, persisted state, or lifecycle owner is
  authorized.
- The E12 catalogue remains `pending`; this implementation review is not the
  separate independent/different-fox evidence audit and does not mark E12
  `satisfied`.

### Remediation verification

#### F-01 — fresh ordinary replacement startup observation — CLOSED

The remediation records both pre-restart public observations and requires the
replacement to change both of them. `first_service_alloc_started_at` extracts
the public allocation `Since` field and
`first_service_startup_observed_at` extracts the startup probe's existing
`last_observed_at` (`examples/service-kind-vm-workloads/run-example.sh:396-406`).
The baseline captures and validates both values
(`run-example.sh:803-815`); the replacement path requires a different startup
timestamp and a different `Since` value before returning success
(`run-example.sh:720-734`). The post-capture checks repeat those assertions
(`run-example.sh:843-859`). No new generation field, API, or production state
was added.

The fresh retained native-metal receipt proves the predicates against the
real built-product owner path:

- baseline startup observation: `1788968249596`; baseline `Since`:
  `(c=15,w=local)` (`verification/.../product-run.out:50-53`, ledger line 93);
- terminal row: `Terminated`, restart count `0`, `Since` `(c=213,w=local)`;
- replacement row: `Running`, restart count `1`, `Since` `(c=214,w=local)`;
- replacement startup observation: `1788968284160`
  (`product-run.out:68-90`, ledger lines 94-95).

The replacement therefore has a new public lifecycle-start stamp and a new
startup observation. Its new certificate serial and `driver started` row are
also visible in the after describe. The unchanged startup timestamp in the
terminal row is expected historical data for the allocation that has just
terminated; the contract requires the *replacement* observation to be fresh,
which is now asserted and captured.

**Disposition:** F-01 is closed. The previous real production-owner-path
false pass (restart count 1 with an unchanged startup timestamp) is no longer
accepted by the example or runner, and the remediation receipt contains the
distinct values.

#### F-02 — threshold/terminal witness and derived liveness attribution — CLOSED

The example now reads the checked-in liveness threshold and counts distinct
persisted liveness-failure observation timestamps
(`run-example.sh:676-708`). It saves the terminal describe only when the
observed count is at least that threshold, the current allocation is
`Terminated` or `Failed`, and restart count is zero
(`run-example.sh:713-719`). The replacement is only accepted after that saved
terminal snapshot and after its own `Running` row, `last terminated` history,
startup pass, readiness pass, and fresh observations
(`run-example.sh:720-734`). The ledger's `liveness-probe` attribution is no
longer unconditional: it is derived only when the observed count reaches the
threshold, the saved state is terminal, and the saved terminal describe has a
liveness failure (`run-example.sh:872-880`).

The runner independently parses the fixture threshold and rejects malformed
or non-positive values. It requires exactly one before/terminal/after row,
terminal state plus restart count zero, HTTP 503 liveness failure,
`threshold_failures >= expected_threshold`, same allocation identity, equal
terminal/after threshold witness, and fresh before/after startup and `Since`
values (`verification/expectations/E12-vm-service-liveness-restart-describe/runner.sh:17-24,58-100`).
The fixture remains `failure_threshold = 2`; the fresh receipt records 13
distinct failures, then the terminal row, then restart count 1.

The final product transcript has the required ordering: terminal public state
is `Replicas (desired/running): 1/0` and `Terminated` at `(c=213,w=local)`
with liveness HTTP 503 (`product-run.out:54-67`), followed by the replacement
`Running` row at `(c=214,w=local)` and `last terminated` history
(`product-run.out:68-90`). Readiness is `pass` in both terminal and after
describes, and the shell rejects any readiness failure across the transcript
(`runner.sh:102-105`). The public terminal snapshot has no allocation address
while `running=0`; the replacement later has its address and certificate. This
is consistent with the upstream seeded S-SVM-17 terminal/dead-backend
invariant and with the approved separation of describe evidence from the
in-process lifecycle-owner proof.

**Disposition:** F-02 is closed. The prior native false pass captured a
`Running`/restart-0 sample with one failure below threshold and a literal
reason. The remediated path captures a terminal row at or beyond threshold
before restart and derives the ledger attribution from those observed facts.
No current production owner path reproduces a dead allocation becoming
eligible, and no new black-box API or traffic assertion is authorized by the
E12 contract; the existing S-SVM-17 invariant and terminal public projection
remain the appropriate complementary evidence.

### Exact design, API, and scope audit

The remediation commit changes only the checked-in E12 fixture timing, example
orchestration/parsing, E12 README/runner, execution-log bookkeeping, and
retained evidence. `git diff d850c94d..654e10e8 --name-status` contains no
production Rust source, test source, public type/method/trait/enum, persisted
field, restart policy, lifecycle state, or new owner. Changing the guest's
failure delay from 3000 ms to 20000 ms is bounded fixture timing: it lets the
existing thresholded terminal row be observed before the already-existing
restart, without changing `failure_threshold = 2` or any product policy.

The README now states the exact terminal-before-replacement ordering and the
two existing timestamp witnesses (`README.md:8-43`). The in-process
S-SVM-21A/B tests and their exact `bounded-change` Contract Shape declarations
are unchanged from the approved first iteration. They continue to exercise
production reconcilers in process; the shell expectation continues to drive
the built default-feature binary and does not invoke tests, import/link an
`overdrive-*` crate, or recreate a probe program. This preserves the distinct
integration-test/black-box expectation boundary.

### Native evidence provenance and actuality

The final retained receipt is an independent fresh run of the remediated
working tree, dated `2026-09-09T15:37:13Z`, with:

```
execution_status: "succeeded"
execution_substrate: "native-metal"
executed_in_lima: false
runner_invoked: true
runner_exit_code: "0"
working_tree_dirty: true
overdrive_sha: d850c94dc69d3bef7d6cb962210672efb67b0686
```

The dirty working-tree receipt is correct: the remediation source was tested
before its commit and the dirty patch/status are retained. The native product
transcript has a clean four-line ledger (header plus before, terminal, and
after), the runner PASS line, and all zero cleanup deltas
(`product-run.out:91-101`). The preceding old false-pass capture remains under
`evidence/attempt-3`, and the first remediation attempt remains under
`evidence/attempt-4` with `execution_status: failed` and its runner failure.
Those retained failed/obsolete attempts are not silently rewritten as the
final pass. The separate evidence catalogue still correctly says `pending`.

I did not perform the separate different-fox native evidence audit and do not
claim that catalogue gate.

### DES, commit, and formatting audit

The live DES log now has two complete ordered execution traces for this step:

| Trace | RED | GREEN | COMMIT |
|---|---|---|---|
| Original implementation | 2026-09-09T14:40:06Z | 2026-09-09T14:56:45Z | 2026-09-09T14:58:54Z |
| F-01/F-02 remediation | 2026-09-09T15:25:29Z | 2026-09-09T15:38:47Z | 2026-09-09T15:40:01Z |

`PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity
docs/feature/service-kind-vm-workloads/deliver` reports all nine steps have
complete DES traces, and the roadmap-only validator reports `Roadmap format
OK`. The post-remediation COMMIT event is the expected uncommitted DES log
write after `des-commit`; this reviewer did not stage it.

The remediation commit preserves author
`Marcus Schack Abildskov <work@marcus-sa.dev>`, has exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 03-02`,
and has no Claude/Anthropic/generated-by attribution. `cargo fmt --all --
--check` and `git diff --check` were run. The latter reports only trailing
whitespace/blank-line bytes in retained PTY transcripts and dirty patches; no
source formatting defect is present.

### Findings and dispositions — iteration 2

| Finding | Severity in iteration 1 | Iteration-2 status | Disposition |
|---|---|---|---|
| F-01 — stale startup observation accepted as replacement startup | High | **CLOSED** | Runner now requires changed replacement `Since` and startup `last_observed_at`; fresh native receipt proves both changes. |
| F-02 — pre-terminal below-threshold sample and hard-coded liveness attribution | High | **CLOSED** | Example waits for terminal state and observed threshold, derives attribution, and fresh native receipt records 13 failures before the terminal/restart sequence. |
| New findings | — | **None** | No reachable production-owner-path defect was reproduced in the bounded remediation review. |

Potential omissions that were explicitly tested as hypotheses are not promoted
to findings. The runner does not add a new peer-traffic request or a new
allocation-generation API to E12; the approved E12 surface is describe, the
final terminal describe shows no allocation address while running count is
zero, and the seeded S-SVM-17 production-composition invariant covers terminal
eligibility. The counter is intentionally checked by the existing in-process
S-SVM-21A reset/threshold test; this fixed guest journey has a persistent
post-delay failure sequence, so no Pass/reset schedule is reachable in this
expectation. The failed attempt-4 ledger duplication is retained as failed
evidence, while the final capture is clean; it is not a current defect.

### Verification performed — iteration 2

| Command or evidence | Result |
|---|---|
| `bash -n examples/service-kind-vm-workloads/run-example.sh verification/expectations/E12-vm-service-liveness-restart-describe/runner.sh && examples/service-kind-vm-workloads/run-example.sh check-source` | **PASS** |
| Focused reconcilers nextest command for S-SVM-21A/B | **PASS** — 2 passed, 5 skipped |
| Changed-scope reconcilers acceptance nextest command (`test(service_kind_vm_workloads)`) | **PASS** — 6 passed, 1 skipped |
| `cargo xtask lima run -- cargo fmt --all -- --check` | **PASS** |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | **PASS** |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | **PASS** |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/service-kind-vm-workloads/deliver` | **PASS** — all 9 steps have complete DES traces |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | **PASS** |
| Retained final `verification/harness/run-expectation.sh E12` receipt | **PASS mechanically** — native-metal, runner exit 0, terminal/threshold/fresh-startup/zero-cleanup assertions; no separate different-fox audit claimed |
| Feature-scoped mutation testing | **Not run** — prohibited during individual roadmap-step review |

### Review iteration history and remediation status

Iteration 1 (this artifact above) returned **NEEDS_REVISION** with exactly
F-01 and F-02 open. The original step crafter then remediated those findings
in `654e10e81dc46b9b31128339704b864c06def800`, reran the required RED/GREEN/
COMMIT DES phases, retained both failed/obsolete attempts and a fresh native
receipt, and returned the same step to this reviewer. This iteration found no
remaining proven defect and no need for DESIGN expansion.

**APPROVED.** The implementation and its step-scoped evidence now satisfy the
approved 03-02 contract: the liveness terminal is observed at/above threshold
before same-ID restart, replacement startup is fresh and ordinary, readiness
does not own the restart, no dead allocation is revived in the captured
terminal projection, cleanup is zero, and the owner/API/design boundaries are
preserved. The independent E12 evidence audit remains pending and separate;
this approval is the implementation-review verdict only.
