# DELIVER roadmap review — Service-kind VM workloads

- **Review ID:** `roadmap_rev_20260906_iteration_1`
- **Reviewer:** `nw-solution-architect-reviewer` (fresh isolated reviewer)
- **Artifact:** `docs/feature/service-kind-vm-workloads/deliver/roadmap.json`
- **Iteration:** 1
- **Final verdict:** **NEEDS_REVISION**

## Executive summary

The roadmap preserves the accepted GH #257 architecture: it names the exact
`ServiceSpecV3`, `ProbeRunner::start_alloc(&AllocationSpec)`, and mandatory
`VmDriver::new` surfaces; invents no public API; keeps VM Exec probes under GH
#280; preserves Running, Stable, readiness, liveness-termination, and restart
ownership; and uses peer VM Jobs for every Service-traffic journey that the
accepted DISTILL handoff assigns one. Its linear dependency graph is acyclic,
all 32 DISTILL IDs have one coarse step owner, all 24 Rust RED scaffolds exist,
and the roadmap passes both DES roadmap validators.

It is not executable as written. The first step changes the parser-side Service
payload from `exec` to `driver` while leaving two compiling CLI consumers of
`.exec` to step 01-03, so 01-01 cannot reach GREEN without crossing a later
step's production boundary or inventing a forbidden compatibility surface.
The six expectation commands bypass the evidence harness and therefore cannot
produce the pinned evidence, manifest, status transition, or independent audit
required to complete DELIVER. The roadmap also supplies only eight executable
locators for 32 scenario IDs, omits an executable final mutation command and
gate, and exceeds the mandatory eight-step concision ceiling. Its one-hour
estimate for three substantial native-metal matrices has no measured basis.

## Review basis

- Accepted DESIGN: `design/wave-decisions.md`, ADR-0090, ADR-0091, and
  `design/review-design.md` iteration 2 (**APPROVED**).
- Accepted DISTILL: `feature-delta.md`, `distill/test-scenarios.md`,
  `distill/red-classification.md`, all three slice briefs, the 24 checked-in
  Rust RED scaffolds, and E08-E13 expectation contracts/runners.
- Current production parser, CLI projection, ProbeRunner, ExecDriver,
  VmDriver, control-plane composition, ServiceLifecycle, and WorkloadLifecycle
  paths.
- Repository roadmap, testing, verification, design, and DELIVER rules.

## Mandatory roadmap checks

### 1. External validity — FAIL

The intended journeys correctly use the built default-feature `serve`,
`deploy`, `workload describe`, and peer VM Job surfaces. However, every roadmap
command invokes an expectation `runner.sh` directly. That bypasses the
catalogue harness and cannot complete the required evidence artifact or review
workflow. See D2.

### 2. Acceptance-criterion implementation coupling — PASS under the repository-specific design contract

The criteria name exact public signatures and types only where ADR-0090/0091
already make those shapes contractual. Removing them would weaken the
repository's “implement to the design; never invent API surface” rule. No
criterion prescribes a new private decomposition or unsanctioned public shape.

### 3. Step decomposition ratio — PASS

Eight steps name sixteen unique production/example entries, a ratio of
`8 / 16 = 0.5`. No three-step substitution pattern exists. The problem is the
placement of the V3 compiler cut, not over-decomposition; see D1.

### 4. Implementation code in roadmap — PASS

The roadmap repeats accepted public contracts and observable invariants but
contains no method body, loop, algorithm, or pseudocode. The exact API text is
authoritative DESIGN fidelity rather than roadmap-authored implementation.

### 5. Concision and precision — FAIL

JSON string values contain 1,865 words. The mandatory limit for a medium
roadmap of four to eight steps is 1,500. Individual descriptions, criteria,
criterion counts, and implementation notes are within their local limits, but
the total is over the blocking ceiling. See D5.

### 6. Unit/acceptance boundary — FAIL

Rust tests remain in-process and expectations remain built-product black-box
journeys, which is the correct architectural split. The executable locator map
is nevertheless incomplete, and the expectation verification path bypasses
the evidence layer. See D2 and D3.

## Blocking findings

### D1 — The `ServiceSpecV3` compiler cut is split across non-atomic steps

**Severity:** BLOCKER

Step 01-01 requires `ServiceSpec`/`ServiceSpecLatest` to alias the exact
`ServiceSpecV3` whose driver field replaces V2's `exec`
(`roadmap.json:33-63`; ADR-0091 Decision). Step 01-03 defers both CLI Service
projection lanes to a later step (`roadmap.json:99-131`). In the current
production tree, those lanes read `service_spec.exec.command` and
`service_spec.exec.args` at `crates/overdrive-cli/src/commands/deploy.rs:321-322`
and `:653-654`. Re-aliasing `ServiceSpec` in 01-01 therefore makes the CLI fail
to compile.

The crafter cannot close 01-01 honestly by retaining an `exec` field, adding a
compatibility accessor, or introducing another public payload: ADR-0091 pins
the exact V3 field set and forbids parallel surface. Updating the CLI union
projection early would instead implement S-SVM-23/24 production behavior
before their owned RED activation in 01-03, contrary to the roadmap's atomic
scaffold and step-ownership rule.

**Required remediation:** put the V3 alias cut, every compiler-required direct
consumer, both CLI union projections, and the S-SVM-23/24 RED activations in
one GREEN/COMMIT boundary. Keep the independently testable VmDriver runner
composition work in its own correctly ordered step. Do not solve the ordering
defect with new API.

### D2 — E08-E13 bypass the mandatory evidence-capture and independent-review path

**Severity:** BLOCKER

Steps 02-02 through 03-02 invoke
`cargo xtask metal run -- verification/expectations/<ID>/runner.sh`
(`roadmap.json:189-191`, `:222-225`, `:263-265`, `:292-295`). The operational
SSOT requires capture through `verification/harness/run-expectation.sh <ID>`.
That harness sets `REPO_ROOT` (which every E08-E13 runner consumes), pins SHA,
dirty state, seed, harness SHA, and substrate, captures `run.log`, validates
anchors, and writes `evidence/verification.yaml`. Direct runner execution does
none of this.

No roadmap criterion requires the generated evidence, an honest README/INDEX
status transition, or the different-fox evidence audit before the owning step
is approved. A passing shell journey could therefore leave every expectation
formally `pending`, which violates `.claude/rules/verification.md`'s DELIVER
contract and does not complete the accepted DISTILL handoff.

**Required remediation:** make the native-metal invocation of the catalogue
harness—not the runner—the executable gate for each of E08-E13. Require its
successful pinned evidence, honest README and INDEX status, and independent
evidence review before the owning step can be approved. Keep each runner a
thin black-box wrapper over the checked-in example and keep all Rust/private
invariants in their existing test lanes.

### D5 — The roadmap exceeds the mandatory eight-step concision ceiling

**Severity:** BLOCKER

The roadmap contains 1,865 words across JSON string values, above the 1,500-word
limit for four to eight steps in `nw-roadmap-design` and
`nw-roadmap-review-checks`. This is not a validator cosmetic-warning finding;
it is the review skill's total-roadmap gate. Repeated paradigm, scaffold,
Lima/native-metal, and expectation-boundary wording accounts for much of the
duplication even though each local field remains below its own cap.

**Required remediation:** move repeated cross-step rules into one compact
roadmap-level contract and shorten duplicated notes while retaining the exact
ADR APIs, scenario oracles, lifecycle owners, and executable locators. Do not
trim load-bearing behavior merely to satisfy the count.

## High-severity findings

### D3 — Thirty-two scenario IDs have only eight executable locators

**Severity:** HIGH

The `scenario_ids` arrays contain all 32 DISTILL IDs exactly once, and the tree
contains all 24 expected Rust scaffolds. But each grouped step has only one
`test_file`/`scenario_name` pair. Several pairs identify only a minority of the
owned work:

| Step | Owned scenarios | Singular locator omits |
|---|---:|---|
| 01-01 | 8 Rust scenarios | Seven named core functions |
| 01-03 | 5 Rust scenarios | Worker S-SVM-16, control-plane S-SVM-22, and both CLI functions |
| 02-03 | E09, E10, E13 | E10 and E13 runner identities |
| 03-01 | 2 Rust + 3 E11 claims | Reconciler S-SVM-20 and all three E11 mode claims |
| 03-02 | 2 Rust + E12 | S-SVM-21B and E12 |

The DISTILL technical map also assigns four distinct Contract Shapes that are
not reproduced per executable. Broad file comments such as “all eight
scaffolds” are not exact RED activation locators. A crafter cannot mechanically
prove that it activated only its complete owned set, and a reviewer cannot
audit 24/24 scaffold transition from the roadmap alone.

**Required remediation:** add one exact executable mapping per S-SVM ID with
owner step, artifact/test file, function or expectation-mode identity, and
Contract Shape. Preserve the efficient grouped steps; do not manufacture 32
micro-steps.

### D4 — The final mutation gate is not executable or complete

**Severity:** HIGH

The only mutation text is a prose item inside step 03-02 saying to run
feature-scoped mutation testing after reviews (`roadmap.json:295`). It names no
sanctioned command, omits `--features integration-tests`, states neither the
absolute `>=80%` nor baseline-regression gate, and does not require the
structured `target/xtask/mutants-summary.json` result. It is also nested in a
step verification list even though repository DELIVER rules make mutation a
single wave-level gate after every step review and expectation evidence review.

**Required remediation:** declare one executable final DELIVER gate using the
`cargo xtask mutants` wrapper with the repository-required integration-test
feature and kill-rate/baseline criteria. Place it explicitly after all eight
step reviews and E08-E13 evidence approvals; keep mutation forbidden during
individual steps and do not edit exclusions as part of a step.

## Medium-severity findings

### D6 — The native-metal matrix estimate has no credible measurement basis

**Severity:** MEDIUM

Step 02-03 estimates one hour to activate, execute, ledger, clean, and review
three modes containing 100 isolated healthy/failure pairs, an eight-cell
cross-driver HTTP matrix, and two inferred-startup journeys
(`roadmap.json:196-227`). The current `run-example.sh` is a 40-line pending
dispatcher with all six modes exiting 75. The nearest delivered E07 product
journey required a 386-line example runner plus dedicated lifecycle helpers;
the roadmap supplies no measured single-trial duration or prior-run estimate
showing that the 02-03 execution alone fits one hour.

**Required remediation:** re-estimate from a measured representative native
trial and the delivered E07 orchestration reference, then update step and total
hours without changing accepted scope. If the accepted slice cap cannot hold,
surface that estimate mismatch rather than weakening the 100/100, 8/8, cleanup,
or review obligations.

### D7 — The ADR exclusion patterns point at paths that do not exist

**Severity:** MEDIUM

`implementation_scope.excluded_patterns` lists
`docs/architecture/decisions/0090-*.md` and `0091-*.md`
(`roadmap.json:325-331`), while the accepted ADRs live under
`docs/product/architecture/adr-0090-*.md` and `adr-0091-*.md`. File lists are
guidance rather than restrictive allowlists, so this does not block required
compiler fallout, but the entries do not protect the accepted contracts they
claim to exclude.

**Required remediation:** correct the paths or replace them with one explicit
statement that accepted DESIGN/ADRs are read-only inputs to DELIVER. Preserve
the repository rule that tightly related compiler, test, manifest, harness,
and configuration fallout may extend the listed scope.

## Correctness checks that passed

- DES schema: `VALID: 3 phases, 8 steps`; roadmap-only DELIVER integrity reports
  no validator errors.
- Scenario cardinality: DISTILL 32, roadmap 32, unique roadmap IDs 32; no
  missing or duplicate ID.
- RED inventory: 24 exact `#[should_panic(expected = "RED scaffold")]`
  functions across core 8, worker 7, sim 1, reconcilers 5, control-plane 1,
  and CLI 2.
- Dependency graph: strict acyclic sequence
  `01-01 -> 01-02 -> 01-03 -> 02-01 -> 02-02 -> 02-03 -> 03-01 -> 03-02`.
- Accepted API fidelity: exact V3 field family, replacement `start_alloc`
  signature, mandatory fifth ProbeRunner constructor dependency, existing
  unions/errors/hooks, and no `Vm + None` behavior are retained.
- HTTP/TCP-only scope: VM Exec probes are rejected at both ingress boundaries
  with GH #280 guidance; no guest Exec protocol or lifecycle surface appears.
- Lifecycle ownership: driver/Beacon owns Running, startup owns Stable,
  readiness owns `Backend.healthy`, ServiceLifecycle emits only liveness stop,
  and WorkloadLifecycle alone restarts/finalizes.
- VM peer-client journey: E08, E09, E11, and E13 require checked-in plaintext
  `[job] + [vm]` callers through the Service name/frontend; direct
  host-to-`workload_addr` traffic is explicitly insufficient.
- Layering intent: Rust tests stay in-process; expectations use checked-in
  examples and the built default-feature product. D2 concerns evidence capture,
  not a collapse of Rust and black-box boundaries.
- Validation state: `validation.status = pending` is honest and must remain so
  until roadmap remediation receives a fresh independent approval.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 3 |
| Critical | 0 |
| High | 2 |
| Medium | 2 |
| Low | 0 |

## Disposition

**NEEDS_REVISION.** Return D1-D7 to the roadmap author. Do not initialize DELIVER
execution or dispatch step 01-01 while validation remains pending. After
remediation, a fresh isolated roadmap reviewer must re-run the complete review
against the effective roadmap; no finding authorizes public API invention,
lifecycle redesign, VM Exec expansion, or weaker expectation oracles.

---

## Iteration 2 — remediation re-review

- **Review ID:** `roadmap_rev_20260906_iteration_2`
- **Reviewer:** `nw-solution-architect-reviewer` (fresh independent re-review)
- **Artifact:** `docs/feature/service-kind-vm-workloads/deliver/roadmap.json`
- **Final verdict:** **NEEDS_REVISION**

### Verdict summary

The remediation correctly closes D1, D3, D5, D6, and D7. The V3 alias and both
CLI projections now share one step; all 32 scenario IDs have exact, resolving,
shape-correct executable mappings; total roadmap text is below the mandatory
limit; the estimate is grounded in the recorded E07 run; and the ADR exclusion
paths are real.

D2 is not closed. The new commands put the evidence harness *inside* the remote
metal execution boundary. The metal synchronizer deliberately excludes `.git`,
while the harness requires local Git metadata before it can start, and metal
run performs no evidence pullback. The established E07 direction is the
opposite: run the harness in the local repository, then let the expectation
runner invoke the metal command and capture its output into local evidence.

D4 is only partially closed. The command is now exact, Lima-wrapped, and uses
the sanctioned wrapper with `integration-tests`, but diff mode enforces only
the 80% floor. Its summary contains no baseline or drift values, so the roadmap's
claimed hard two-percentage-point regression check is not executed. The
remediation also replaced all exact Rust verification commands with prose
labels, creating D8: the mapped RED functions are identifiable, but the
RED-to-GREEN commands are no longer executable from the roadmap.

### Prior-finding dispositions

| Finding | Disposition | Fresh verification |
|---|---|---|
| D1 — split V3 compiler cut | **RESOLVED** | S-SVM-23/24 and `crates/overdrive-cli/src/commands/deploy.rs` moved into 01-01 (`roadmap.json:74-95`). The exact V3 alias, all direct consumers, both CLI lanes, and workspace all-target compilation now share one commit boundary. Step 01-03 owns only VmDriver/production runner composition. |
| D2 — expectation evidence bypass | **UNRESOLVED** | The roadmap now names the harness and audits (`:21`, `:181`, `:204`, `:239`, `:262`), but all six commands incorrectly wrap the harness in `cargo xtask metal run --` (`:193`, `:217-219`, `:251`, `:274`). See current finding D2-R2. |
| D3 — incomplete executable map | **RESOLVED** | `roadmap.scenario_mappings` has 32 rows, 32 unique IDs, and 32 unique artifact/executable identities (`:24-57`). Every Rust artifact/function exists, every expectation mode resolves, every owner matches its step's `scenario_ids`, and all Contract Shapes byte-match the DISTILL technical map. |
| D4 — incomplete final mutation gate | **PARTIALLY RESOLVED** | The final gate is wave-level, after all step/evidence reviews, exact, Lima-wrapped, and includes `integration-tests` plus the 80% floor (`:58-64`). The claimed baseline-regression hard gate is not implemented by that command. See D4-R2. |
| D5 — roadmap over 1,500 words | **RESOLVED** | Current JSON string values total 1,458 words. Every description is at most 16 words, every step has at most five criteria, the longest criterion is 24 words, and every note is at most 22 words. |
| D6 — unsupported estimate | **RESOLVED** | E07 evidence records 00:25:24Z to 00:25:50Z, the stated 26 seconds. The roadmap shows its 200-journey arithmetic and increases 02-03 to eight hours and the total to 32 hours (`:6`, `:14`, `:210`). Step and phase estimates both sum to 32. |
| D7 — nonexistent ADR exclusions | **RESOLVED** | The exclusions now name the two actual `docs/product/architecture/adr-0090-*` and `adr-0091-*` files, while the roadmap explicitly treats scope lists as guidance and allows bounded fallout (`:22`, `:280-283`). |

### Mandatory roadmap checks

#### 1. External validity — FAIL

All required production entry points and peer VM Job journeys remain present,
but the declared E08-E13 commands cannot produce repository-local evidence.
D2-R2 remains blocking.

#### 2. Acceptance-criterion implementation coupling — PASS

Exact public names/signatures repeat already-accepted ADR contracts. No new
private decomposition or unsanctioned API appears.

#### 3. Step decomposition ratio — PASS

Eight steps target eleven unique production/example files, a ratio of
`8 / 11 = 0.73`. The grouped matrix step avoids three identical orchestration
steps, and no no-op validation-only step exists.

#### 4. Implementation code in roadmap — PASS

The roadmap contains accepted signatures and behavioral constraints, not
method bodies, algorithms, loops, or pseudocode.

#### 5. Concision and precision — PASS

The 1,458-word total and every local field satisfy the quantitative limits.
The compression did, however, remove executable Rust commands; that is a
precision/executability defect rather than a word-count failure. See D8.

#### 6. Unit/acceptance boundary — PASS

The 24 Rust scenarios remain in-process at core, worker, sim, reconciler,
control-plane, and CLI boundaries. E08-E13 remain checked-in, default-feature,
built-product black-box journeys. No expectation imports a crate or invokes a
Rust test, and peer VM Jobs remain the Service-traffic oracle. D2-R2 concerns
where evidence is captured, not a collapse of these layers.

### Blocking finding

#### D2-R2 — The harness is on the wrong side of the metal boundary

**Severity:** BLOCKER

Every native verification command has the shape
`cargo xtask metal run -- verification/harness/run-expectation.sh ENN`
(`roadmap.json:193`, `:217-219`, `:251`, `:274`). `metal run` synchronizes the
tree to the remote host while unconditionally excluding `.git`
(`infra/metal/bootstrap.sh:227-244`) and states that the remote has no Git
metadata by default (`:247-266`). The harness immediately executes
`git rev-parse --show-toplevel` (`verification/harness/run-expectation.sh:22-25`)
and later reads HEAD/status/diff (`:58-67`). On the declared default metal path,
it therefore cannot initialize its evidence capture.

Even if stale or explicitly materialized remote Git metadata made that command
run, the harness would write `run.log`, dirty evidence, and
`verification.yaml` into the remote synchronized tree. `metal run` is a one-way
local-to-remote rsync followed by SSH execution and has no reverse evidence
sync (`bootstrap.sh:227-244`, `:322-343`), so those files would not reach the
reviewed repository.

The delivered E07 precedent runs `verification/harness/run-expectation.sh E07`
locally; its runner invokes `cargo xtask metal run --` for only the product
example and captures that remote stdout into local `$EVIDENCE_DIR`
(`verification/expectations/E07-vm-job-calls-exec-service/runner.sh:27-44`).
E08-E13 currently call their example directly, so wrapping the outer harness
in metal also prevents them from adopting that established shape coherently.

**Required remediation:** keep the harness in the local Git worktree and make
each activated native-metal runner own the bounded `cargo xtask metal run --`
dispatch/capture, following E07's evidence direction. The roadmap gate must
leave pinned evidence in the local expectation directory for the independent
auditor. Do not add crate imports, test invocation, or a second product path.

### High-severity findings

#### D4-R2 — The stated mutation drift gate is not enforced by diff mode

**Severity:** HIGH

The final command is
`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`
(`roadmap.json:58-63`). That is the correct per-PR wrapper and it enforces the
80% absolute floor. The same roadmap block also requires regression versus
`mutants-baseline/main/kill_rate.txt` to be at most two percentage points.

The current wrapper does not compute that result in diff mode:

- `Mode::Diff` and `Mode::Workspace` are separate (`xtask/src/mutants.rs:48-58`);
- diff dispatch calls `evaluate_diff_gate`, while only workspace dispatch
  receives a baseline path (`:166-172`);
- the diff gate returns `baseline_pct: None` and `drift_pp: None`
  (`:587-646`);
- workspace drift is a soft warning, not a hard failure (`:7-11`, `:719-730`).

Consequently wrapper exit 0 and `mutants-summary.json` cannot prove the
roadmap's asserted hard drift criterion. This is also a current mismatch
between `.claude/rules/testing.md`'s prose and the executable wrapper; the
roadmap may not paper over it with a non-executed condition.

**Required remediation:** reconcile the required per-PR baseline policy with
an existing executable gate or surface the tooling/rule gap for explicit
resolution before claiming it in the roadmap. Do not treat a workspace
soft-warning run as the diff-scoped hard gate and do not add a second mutation
run casually; repository DELIVER rules require one final wave gate.

#### D8 — Rust RED-to-GREEN verification entries are prose, not commands

**Severity:** HIGH

The remediation replaced the original Lima-wrapped Rust commands with labels
such as `Lima: core schema/acceptance plus CLI acceptance and workspace
all-target compile` (`roadmap.json:94`), `Lima: overdrive-worker ProbeRunner and
S-SVM-12/13 acceptance` (`:117`), and analogous summaries at `:140`, `:170`,
`:193`, `:251`, and `:274`. None is executable shell syntax; none pins the
package/test filters the crafter must run; and the all-target compiler gate
does not state whether the required `integration-tests` surface participates.

The exact 32-row locator map fixes ownership but does not execute RED or GREEN.
An implementer can choose materially different command scopes and still claim
to have followed these labels, undermining the strict per-step
RED-GREEN-COMMIT-review boundary.

**Required remediation:** restore concrete, Lima-wrapped `cargo nextest run`
commands for each step's exact mapped Rust functions and a concrete
Lima-wrapped workspace all-target `cargo check` command for the V3 compiler
cut. Keep test execution separate from the built-product expectations and stay
within the 1,500-word roadmap ceiling by removing redundant prose, not commands.

### Fresh correctness checks that passed

- DES validation: `VALID: 3 phases, 8 steps`; roadmap-only DELIVER integrity
  reports no errors.
- Scenario map: 32 rows, 32 unique IDs, 32 unique artifact/executable
  identities; exact equality with every step's `scenario_ids` ownership.
- Rust RED inventory: all 24 mapped files/functions exist and still carry the
  checked-in RED scaffold form.
- Contract Shape fidelity: every mapping exactly matches the DISTILL technical
  map, including S-SVM-11/12/17/18 unbounded-preservation.
- Atomic ordering: strict acyclic sequence remains
  `01-01 -> 01-02 -> 01-03 -> 02-01 -> 02-02 -> 02-03 -> 03-01 -> 03-02`;
  01-01 no longer has a known compile-invalid intermediate.
- API fidelity: exact V3, replacement ProbeRunner registration signature,
  mandatory fifth VmDriver runner, existing driver unions/errors/hooks, and no
  `Vm + None` behavior are unchanged.
- Scope: HTTP/TCP-only with VM Exec rejected under GH #280; no Exec transport,
  runner registry, new lifecycle state, or persistence surface appears.
- Lifecycle: Running/Stable/readiness/liveness-stop/restart owners remain
  distinct and match ADR-0090/0091 plus ADR-0087.
- Peer-client journey: E08, E09, E11, and E13 still require checked-in
  plaintext VM Job callers through the Service frontend, never direct
  host-to-guest traffic.
- Estimate arithmetic: phase estimates and step estimates both total 32 hours;
  the E07 26-second source measurement is real.
- Validation status remains honestly `pending`.

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 2 |
| Medium | 0 |
| Low | 0 |

### Final disposition

**NEEDS_REVISION.** D1, D3, D5, D6, and D7 are closed. Remediate D2-R2,
D4-R2, and D8 without changing the accepted public API, lifecycle ownership,
HTTP/TCP-only scope, scenario oracles, or single final mutation discipline.
Do not approve the roadmap or dispatch step 01-01. Re-review the resulting
effective roadmap before DELIVER begins.

---

## Iteration 3 — remediation re-review

**Reviewer:** `nw-solution-architect-reviewer` stance

**Date:** 2026-09-06

**Artifact:** `docs/feature/service-kind-vm-workloads/deliver/roadmap.json`

**Verdict:** **APPROVED — D2-R2, D4-R2, and D8 are resolved; no blocker,
critical, or high finding remains**

### Iteration-2 finding dispositions

| Iteration-2 finding | Disposition | Verification |
|---|---|---|
| D2-R2 — harness on wrong side of metal boundary | **RESOLVED** | The authoritative evidence gate now requires the harness in the local Git checkout and assigns `cargo xtask metal run` to each runner (`roadmap.json:15`, `:22`). Every E08-E13 verification entry invokes `verification/harness/run-expectation.sh ENN` locally (`:200`, `:225-227`, `:261`, `:287`). No roadmap command sends the harness, its Git queries, or its evidence directory through `metal run`. Runner/README/evidence work remains inside each expectation step's scope (`:196`, `:222`, `:257`, `:283`). |
| D4-R2 — mutation drift gate not enforced | **RESOLVED** | The single final command runs the sanctioned diff wrapper once, inside Lima, with `integration-tests`, then gates the generated summary with `jq` against the checked-in baseline (`:59-64`). Shell parsing succeeds. With the current `100.0` baseline, the exact predicate accepts `status=pass, kill_rate_pct=98`, rejects `97.9`, rejects `status=fail`, and independently retains the wrapper's 80% floor. `jq` is provisioned in the Lima image. The post-run assertion is not a second mutation invocation. |
| D8 — Rust verification was prose rather than commands | **RESOLVED** | All Rust-owning steps now carry concrete Lima-wrapped commands with package, target, and exact test filters (`:95-98`, `:122`, `:145`, `:175`, `:198-200`, `:259-261`, `:285-287`). Step 01-01 also has the explicit workspace all-target `integration-tests` compile gate. Executing those nine commands in Lima selected all 24 mapped RED functions, plus four schema-evolution tests, and the compiler gate completed successfully. |

### Mandatory roadmap checks

#### 1. External validity — PASS

The roadmap closes the feature through the real `overdrive serve` and
`overdrive deploy <SPEC>` product path. E08 is the sole walking skeleton, and
E08-E13 drive checked-in examples through the built default-feature binary on
native metal. The local harness now captures repository-pinned evidence while
the individual runner owns remote dispatch and output return.

#### 2. Acceptance-criterion implementation coupling — PASS

The named `ServiceSpecV3`, replacement `ProbeRunner::start_alloc` signature,
and mandatory `VmDriver::new` parameter order repeat the already-accepted
ADR-0090/0091 contract. They constrain crafters to the approved public API
rather than inventing implementation structure. Remaining criteria state
observable parser, lifecycle, traffic, evidence, and cleanup outcomes.

#### 3. Step decomposition ratio — PASS

Eight steps target eleven unique production/example files: `8 / 11 = 0.73`.
The repeated static expectation modes remain batched in 02-03, and every step
adds production, example, or expectation-runner behavior.

#### 4. Implementation code in roadmap — PASS

The roadmap contains accepted public signatures and executable verification
commands, but no method bodies, algorithms, loops, or pseudocode. Exact public
shapes are design-contract declarations, not implementation inventions.

#### 5. Concision and precision — PASS

JSON string values contain 1,421 words, below the 1,500-word limit for eight
steps. Every description is at most 16 words, every step has at most five
criteria, the longest criterion is 24 words, and every note is at most 14
words. Verification commands are concrete without expanding explanatory prose.

#### 6. Unit/acceptance boundary — PASS

The 24 Rust scenarios stay in-process at core, worker, sim, reconciler,
control-plane, and CLI boundaries. E08-E13 remain black-box built-product
expectations and invoke neither Rust tests nor Overdrive crates. Peer VM Jobs
remain the Service-traffic oracle where required.

### Remediation evidence

#### D2-R2 evidence direction

`verification/harness/run-expectation.sh` still derives the repository root,
HEAD, dirty state, and evidence paths from the local Git checkout. The roadmap
now invokes that harness directly and requires each runner to perform the
bounded metal call and return its captured output. This matches the delivered
E07 direction: local harness and evidence, remote product execution. The
current E08-E13 runners remain honest exit-75 DISTILL scaffolds; activating the
runner-side metal dispatch is work owned by their roadmap steps, not a
precondition the unimplemented baseline could already satisfy.

#### D4-R2 executable mutation gate

The command at `roadmap.json:61` has these independently verified properties:

- exactly one `cargo xtask mutants --diff origin/main` invocation;
- Lima wrapping whenever `integration-tests` participates;
- wrapper status and absolute `>= 80` enforcement;
- a checked-in numeric baseline (`mutants-baseline/main/kill_rate.txt = 100.0`);
- a hard post-wrapper predicate requiring `kill_rate_pct >= baseline - 2`;
- a single summary artifact at `target/xtask/mutants-summary.json`; and
- placement only after all eight step reviews and E08-E13 evidence audits.

The mutation suite was deliberately not run during roadmap review; repository
discipline reserves that expensive invocation for the one final DELIVER-wave
gate. Syntax and predicate behavior were exercised without generating mutants.

#### D8 command execution

The exact roadmap Rust commands were run inside the repository Lima VM:

| Verification owner | Selected tests | Result |
|---|---:|---|
| 01-01 core + CLI Service-VM acceptance | 10 | PASS |
| 01-01 schema evolution | 4 | PASS |
| 01-01 workspace all-target `integration-tests` check | workspace | PASS |
| 01-02 worker TCP projection | 2 | PASS |
| 01-03 worker + control-plane supervision/composition | 3 | PASS |
| 02-01 worker HTTP projection | 3 | PASS |
| 02-02 reconciler startup ownership | 2 | PASS |
| 03-01 sim terminal + reconciler readiness | 2 | PASS |
| 03-02 reconciler liveness/restart ownership | 2 | PASS |

The 24 acceptance tests currently pass by their checked-in RED-scaffold
`#[should_panic]` contract; this execution validates command syntax, target
resolution, and exact scenario selection, not implementation GREEN. The Lima
instance was returned to its original stopped state after validation.

### Regression revalidation

- **D1 atomic cut:** PASS. V3, all direct compiler consumers, authoritative
  admission/describe, and both CLI lanes remain in 01-01 before any
  `ProbeRunner` or `VmDriver` change.
- **D3 executable map:** PASS. There are 32 rows, 32 unique IDs, and 32 unique
  artifact/executable pairs. Every Rust function exists, every expectation
  mode resolves, step ownership is exact, and all 32 Contract Shapes match the
  DISTILL technical map.
- **D5 concision:** PASS at 1,421 words with all local limits satisfied.
- **D6 estimate:** PASS. Phase, step, and roadmap totals all equal 32 hours;
  the E07 source measurement remains the recorded 26 seconds and the 02-03
  matrix estimate remains eight hours.
- **D7 exclusions:** PASS. The read-only exclusions name the actual ADR-0090,
  ADR-0091, feature-delta, DESIGN, DISTILL, and target paths; scope lists remain
  guidance that permits tightly related compiler and harness fallout.
- **Exact API:** PASS. The roadmap preserves the complete accepted V3 shape,
  one replacement `start_alloc(&AllocationSpec)`, mandatory fifth VmDriver
  runner, existing driver unions/errors/hooks, and no `Vm + None` behavior.
- **Scope:** PASS. GH #257 remains HTTP/TCP-only; VM Exec is rejected before
  intent commit with GH #280 guidance. No guest Exec protocol, runner registry,
  persistence subsystem, public state, or parallel deploy surface appears.
- **Lifecycle:** PASS. Running/Beacon, startup/Stable,
  readiness/`Backend.healthy`, `ServiceLifecycle` liveness termination, and
  `WorkloadLifecycle` restart ownership remain distinct.
- **Product oracle:** PASS. E08, E09, E11, and E13 retain checked-in plaintext
  peer VM Job callers through the Service frontend, never direct host-to-guest
  traffic.
- **Mechanical format:** PASS. DES reports `VALID: 3 phases, 8 steps`; roadmap
  integrity reports no errors; estimates sum to 32; validation status remains
  honestly `pending` until this review is accepted.

### Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

### Final disposition

**APPROVED.** All iteration-1 and iteration-2 findings are closed. The roadmap
is executable without changing the accepted public API, lifecycle ownership,
HTTP/TCP-only boundary, scenario oracles, evidence separation, or single final
mutation discipline. It is ready for its validation metadata to be marked
approved by the owning workflow before DELIVER step 01-01 begins.

---

## Iteration 4 — focused ADR-0101/E09-v2 amendment review

- **Review ID:** `roadmap_rev_20260908_amendment_adr0101_e09v2_iteration_1`
- **Reviewer:** `nw-solution-architect-reviewer` (selected GPT5.6 Luna, maximum thinking)
- **Reviewed:** 2026-09-08
- **Artifacts:** `docs/feature/service-kind-vm-workloads/deliver/roadmap.json` and `docs/feature/service-kind-vm-workloads/deliver/roadmap-amendment-adr-0101-e09-v2.md`
- **Review boundary:** Focused amendment review of mappings, dependencies, executable sequencing, evidence boundaries, and DELIVER history. The original iterations above remain verbatim and are not re-opened.
- **Final verdict:** **APPROVED**

### Review basis and scope

The effective contract is accepted ADR-0101 revision 3, the accepted backend
eligibility ruling revision 3, independent DESIGN review iteration 3, and the
consolidated DESIGN+DISTILL review iteration 2. The acceptance handoff and RED
classification were checked for the BE-01 through BE-12/BE-P1 mappings, the
S-SVM-25/26/29 reassignment, and the stated partial-RED limits. This review
does not re-open the accepted architecture, acceptance-test design, existing
production implementation, native captures, or unrelated pre-amendment
roadmap prose.

The amendment is treated as a delta, not as a replacement execution history.
Completed step identities and their ownership remain unchanged. The existing
`execution-log.json` was read without modification: it contains six historical
`02-03` RED events (`execution-log.json:139-178`), no `02-03` GREEN or COMMIT,
and no events yet for `02-04` or later pending steps. The amendment correctly
states that those RED events belong to the former verification-only contract.
A new crafter must independently run and append its own `02-03` RED/GREEN/
COMMIT phases; no historical phase is counted as the new implementation
contract. The current integrity failure is therefore an honest pending state,
not a reason to rewrite, delete, or fabricate log entries.

### Amendment contract and sequencing assessment

Step `02-03` is coherently repurposed from the uncompleted verification shape
to the single atomic implementation boundary required by ADR-0101. Its five
criteria require ServiceLifecycle to publish every current listener's complete
row, use the exact State/Fact/Identity/View and ordering contract, retire the
entire BackendDiscoveryBridge surface in the same change, and preserve the
accepted async consumer and failure boundaries. The criteria do not add a
public method, type, variant, persistence mechanism, compatibility path,
migration, upgrade path, dual-writer interval, or restart policy. Bridge-test
transfer/retirement and the seed-257209 control remain explicitly owned by the
acceptance designer.

The new `02-04` is a separate native verification step. It owns S-SVM-25,
S-SVM-26, and S-SVM-29, while `03-01` now depends on `02-04` and `03-02`
continues to depend on `03-01`. The nine-step count, phase estimates, and
dependency graph are internally consistent. The final mutation gate remains a
single wave-level gate after all nine step reviews and E08/E09-v2/E10-E13
audits; no per-step mutation work is introduced.

### Mapping and evidence-boundary assessment

The roadmap has 45 unique mappings: the original 32 DISTILL IDs plus all 13
approved ADR-0101 IDs. All BE-01 through BE-12 and BE-P1 rows belong to
`02-03`, point to the approved Sim integration/source-local artifacts, and
retain their declared Contract Shapes. S-SVM-25/26/29 point to the E09-v2,
E10, and E13 runner artifacts and are owned by `02-04`. Static path and
locator checks resolve all 13 amendment-owned Rust functions (with BE-P1's
qualified module path resolved to its leaf test) and all three expectation
modes.

The Rust verification entries for `02-03` are in-process Nextest and workspace
compile commands. The expectation entries for `02-04` are local
`verification/harness/run-expectation.sh` invocations; each runner owns its
bounded `cargo xtask metal run --` product dispatch and returns output to the
local pinned evidence directory. The E09-v2 runner uses the distinct
`examples/service-kind-vm-workloads-v2/run-example.sh` and
`run tcp-truthfulness-100` mode. The legacy E09 runner uses the unversioned
example and its separate ledger markers. The harness's exact-prefix lookup
and bare-E09 legacy fallback therefore distinguish explicit `E09-v2` from
legacy `E09`; the host-safe selector branch test passed.

The E09-v2 acceptance remains bounded to one default-feature build, one
preparation, one persistent control-plane identity, controlled concurrent
Service cohorts, and exactly 100 honest pairs with no retries or discarded
trials. E10 and E13 remain independent black-box expectations. Rust tests do
not spawn the production binary, and expectations do not invoke Cargo tests or
link an `overdrive-*` crate. No native expectation, Rust test, production
binary, or mutation run was performed for this review.

### Mandatory focused roadmap checks

| Check | Result | Evidence / disposition |
|---|---|---|
| 1. External validity | **PASS** | `02-03` retains the production ServiceLifecycle/action-dispatch path; `02-04` drives the checked-in v2 example through the local evidence harness and built product. E10/E13 remain separate black-box paths. |
| 2. Acceptance-criterion coupling | **PASS** | Exact implementation names repeat accepted ADR-0101 shapes and retirements only. No new public surface or private API choice is delegated by the amendment. |
| 3. Step decomposition | **PASS** | Nine steps target 17 unique production-scope entries (`9/17 = 0.53`); the expectation-only `02-04` is a bounded verification step, not a duplicated production implementation step. |
| 4. Implementation code | **PASS** | Descriptions and criteria state accepted outcomes, ownership, and boundaries; they contain no method bodies, pseudocode, or invented algorithm. |
| 5. Concision and precision | **PASS** | The current nine-step roadmap is 2,061 whitespace-delimited words across JSON string values (`jq -r '.. | strings' | wc -w`), below the 3,000-word 9–15-step ceiling; descriptions, criteria counts, and local limits remain within bounds. |
| 6. Unit/acceptance boundary | **PASS** | The amendment preserves in-process Rust versus built-product black-box expectations, peer-VM product oracle, local pinned evidence, and independent audits. |

### Mechanical verification evidence

| Command / check | Result |
|---|---|
| `jq empty docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | **PASS** |
| `python3 -m json.tool docs/feature/service-kind-vm-workloads/deliver/execution-log.json >/dev/null` | **PASS** |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | **PASS** — `VALID: 3 phases, 9 steps` |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.verify_deliver_integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | **PASS** — roadmap validator reports no errors |
| `bash -n` on the harness and E09-v2/E09/E10/E13 runners | **PASS** |
| `bash verification/harness/test-run-expectation.sh` | **PASS** — `run-expectation harness branch tests passed`; host-safe fixtures only |
| Static mapping/dependency/history audit | **PASS** — 9 unique steps, no missing dependency references, 45 unique IDs, completed ownership unchanged, only S-SVM-25/26/29 moved to `02-04`, and amendment-owned locators resolve |
| Full `des.cli.verify_deliver_integrity` (exit 1) | **EXPECTED PENDING** — `02-03` has historical RED-only entries; `02-04`, `03-01`, and `03-02` have no entries yet. This is not production execution evidence. |

### Findings and dispositions

No blocking, critical, high, medium, or low finding remains within this
focused amendment scope. The historical six RED events are preserved rather
than reclassified as new execution; the new crafter's independent DES trace is
still a required future DELIVER action. The prior eight-step approval remains
provenance only, and `roadmap.validation.status = "pending"` is correct until
the owning workflow records this fresh approval. This review does not change
that metadata or authorize implementation completion.

### Amendment verdict

**APPROVED.** The amendment is mechanically executable for the next DELIVER
boundary: `02-03` is an atomic ADR-0101 sole-publisher/bridge-retirement step,
`02-04` is the separately mapped E09-v2/E10/E13 verification step, and
`03-01` follows it. Mappings, selectors, evidence boundaries, final-gate
placement, and historical-log treatment agree with the accepted package. The
approval is limited to this roadmap amendment and makes no claim of production
GREEN, native 100-pair completion, or mutation completion.

---

## Iteration 5 — focused ADR-0101 revision-4 D7 / BE-02 singleton alignment review

- **Review ID:** `roadmap_rev_20260908_amendment_adr0101_e09v2_revision4_iteration_1`
- **Reviewer:** `nw-solution-architect-reviewer` (independent roadmap continuation; selected GPT5.6 Luna, maximum thinking)
- **Reviewed:** 2026-09-08
- **Artifacts:** `docs/feature/service-kind-vm-workloads/deliver/roadmap.json`, `docs/feature/service-kind-vm-workloads/deliver/roadmap-amendment-adr-0101-e09-v2.md`
- **Prior history:** Iterations 1–4 above are preserved verbatim. Iteration 4 remains the approval of the preceding revision-3/E09-v2 amendment; this iteration is a fresh review of the revision-4 delta only.
- **Final verdict:** **APPROVED**

### Review mandate and boundary

This is a narrow roadmap-alignment review. It covers only the changed
revision-4 D7 mapping for BE10's local direct-VIP action selection and the
independently approved BE-02 singleton correction. The review checks the
roadmap's mappings, dependency graph, executable sequencing, acceptance and
expectation boundaries, and treatment of DELIVER history. It does not reopen
ADR-0101 revision 3, the accepted lifecycle ownership, the acceptance-test
design, the E09-v2 runner diagnosis, or unrelated dirty production and test
work.

No production implementation, Rust test, native-metal expectation, mutation
run, DES phase, commit, or additional agent was started for this review. The
roadmap remains an implementation plan; this verdict is not implementation or
native execution evidence.

### Authoritative review basis

The current ADR is **Accepted — revision 4 amendment; independent DESIGN
review APPROVED** and states that D7 is the only revision-3 exception, with no
implementation or native unhealthy-routing claim
(`adr-0101-service-backend-health-observed-convergence.md:3-12`). Its D6 text
keeps all consumer, acknowledgement, persistence, retry, and lifetime
boundaries unchanged
(`adr-0101-service-backend-health-observed-convergence.md:235-258`), while D7
pins the existing Register/Deregister action choice, full local candidate
vector, private helper rename, and unchanged public surface
(`adr-0101-service-backend-health-observed-convergence.md:260-300`).

The focused BE10 DESIGN amendment is independently **APPROVED** and explicitly
limits the change to the existing `ServiceMapHydrator` helper and action
variants, with no public API or new acknowledgement/retry mechanism
(`design/review-amendment-be10-local-backend-withdrawal.md:96-135`). Its
review records the same independent **APPROVED** disposition and leaves native
and implementation work to downstream owners
(`design/review-amendment-be10-local-backend-withdrawal.md:177-249`). The
separate BE-02 singleton acceptance review is also **APPROVED**: one
production-scheduled allocation retains all three listeners, while meaningful
multi-allocation ordering is explicitly deferred to issue #282
(`distill/review-be02-single-allocation.md:15-23,85-115,141-153`).

The amendment under review records this as a delta at
`roadmap-amendment-adr-0101-e09-v2.md:39-58`. The previous review ID and
scope remain recorded at `:33-37`; they are provenance for the preceding
revision-3/E09-v2 amendment and are not silently reused for this review.

### Revision-4 delta assessment

#### BE10 / ADR-0101 D7 mapping

The only new production-scope entry in `02-03` is the existing
`crates/overdrive-reconcilers/src/service_map_hydrator.rs`
(`roadmap.json:237-247`). The related test entry names existing local
action-selection unit tests and private-helper fallout, while the acceptance
designer retains detailed BE tests and bridge-test transfer or retirement
(`roadmap.json:248-258`). This is a path-level addition required by the
approved D7 contract, not a new component, seam, public API, persistence
boundary, retry protocol, acknowledgement, migration, upgrade path, or
dual-writer interval.

The implementation note is mechanically precise: it pins only the private
`push_register_local_backend_actions` to
`push_local_backend_actions` rename and selects the existing
`RegisterLocalBackend` for a healthy local candidate or
`DeregisterLocalBackend` for an unhealthy still-present candidate. It also
requires the complete local candidate vector and fingerprint and preserves
remote, mesh, lifecycle, async-port, and public-API boundaries
(`roadmap.json:260-266`). Those statements match D7's accepted action table
and exact private signature; the roadmap delegates no invented API choice.

The D7 mapping remains inside the existing one-Running normal-convergence
boundary. It does not smuggle BE-02 replica arbitration, local-map readback,
effect acknowledgement, failure retry, or established-flow revocation into
`02-03`.

#### BE-02 singleton correction

The BE-01 through BE-12/BE-P1 scenario IDs and their existing acceptance
artifact mapping remain unchanged. The amendment and `02-03` implementation
note now state the approved singleton input: one genuinely
production-scheduled allocation with all three listener facts. They also state
that multi-allocation membership and order remain a #282 follow-up, without
adding a replica scheduler or a test-only allocation row. This exactly matches
the independent BE-02 review's boundary and keeps its historical two-replica
failure as evidence rather than rewriting it.

#### E09-v2, E10, E13, and sequencing

The `02-04` step, its S-SVM-25/26/29 mappings, estimate, scope, and verification
commands are unchanged (`roadmap.json:269-303`). The exact commands remain:

```text
verification/harness/run-expectation.sh E09-v2
verification/harness/run-expectation.sh E10
verification/harness/run-expectation.sh E13
```

The harness selector uses the exact ID-prefix glob and an explicit versioned
ID path; when bare `E09` is ambiguous it filters only `E09-vN-*` directories
and retains the unversioned directory (`verification/harness/run-expectation.sh:27-51`).
The current directories therefore resolve explicit `E09-v2` to
`E09-v2-vm-service-tcp-truthfulness-100` and bare `E09` to
`E09-vm-service-tcp-truthfulness-100`. This is an actionable distinction, not
a legacy fallback that can capture the v2 runner. `03-01` still depends on
`02-04`; no executable sequencing change is hidden in the delta.

The E09-v2 criterion remains one default-feature build and preparation, one
persistent control-plane identity, controlled concurrent Service cycles, and
exactly 100 honest pairs with no retries or discarded trials. E10 and E13
remain separate black-box expectations. No Rust test is promoted to an
expectation runner, and no expectation invokes Cargo tests or links an
`overdrive-*` crate.

### DELIVER history and actual execution state

`execution-log.json` was read without modification. It contains six earlier
`02-03` RED entries at `execution-log.json:139-178`, followed by a later
`02-03` RED PASS, GREEN FAIL, and COMMIT SKIPPED sequence at
`:181-200`. The skipped-commit detail records the then-unresolved two-running
BE-02 precondition and the then-unchanged-consumer BE-10 boundary. Those
entries are prior attempt/history, not completion of this revision-4 contract;
the new crafter must independently execute and log its own RED, GREEN, and
COMMIT phases. No phase is inherited, removed, relabeled, or fabricated.

The current full integrity check honestly reports no entries for `02-04`,
`03-01`, or `03-02` and exits nonzero for those missing phases. That result is
compatible with this roadmap approval: it is evidence that downstream DELIVER
work has not run, not evidence that the roadmap's new D7 alignment has been
executed. `02-03` is likewise not declared complete by this review.

### Focused roadmap checks

| Check | Result | Evidence and disposition |
|---|---|---|
| External validity | **PASS** | `02-03` names the real ServiceLifecycle/ServiceMapHydrator/action-shim path; `02-04` retains checked-in E09-v2/E10/E13 black-box runners. |
| Acceptance-criterion coupling | **PASS** | The new D7 note repeats only the accepted private helper rename, existing action variants, health mapping, and preserved vector/fingerprint; BE-02 is the approved singleton correction. |
| Step decomposition | **PASS** | Nine steps cover 18 unique production-scope entries (`9/18 = 0.50`); the added hydrator path is the one existing consumer path required by D7, not a duplicate implementation step. |
| Implementation code | **PASS** | The delta contains outcomes, ownership, exact names, and boundaries only; it contains no method body, pseudocode, or invented mechanism. |
| Concision and precision | **PASS** | The current roadmap has 2,175 whitespace-delimited words across JSON string values, below the 3,000-word ceiling for a 9–15-step roadmap. |
| Unit/acceptance boundary | **PASS** | D7 tests remain in-process/source-local acceptance work; E09-v2/E10/E13 remain built-product black-box expectations with no production execution claim. |

### Mechanical verification evidence

| Command or audit | Result |
|---|---|
| `jq empty docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | **PASS** |
| `python3 -m json.tool docs/feature/service-kind-vm-workloads/deliver/execution-log.json >/dev/null` | **PASS** |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate docs/feature/service-kind-vm-workloads/deliver/roadmap.json` | **PASS** — `VALID: 3 phases, 9 steps` |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.verify_deliver_integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | **PASS** — roadmap-only validation reports no errors |
| `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.verify_deliver_integrity docs/feature/service-kind-vm-workloads/deliver` | **EXPECTED PENDING** — exit 1 only for missing `02-04`, `03-01`, and `03-02` phase entries |
| `bash -n` on `run-expectation.sh` and the E09-v2, legacy E09, E10, and E13 runners | **PASS** |
| Static E09/E09-v2 directory-selector audit | **PASS** — explicit v2 resolves one v2 directory; bare E09 resolves one unversioned directory |
| Static roadmap delta audit against the preceding approved roadmap | **PASS** — 9 step IDs, dependencies, estimates, completed step objects, 45 mapping rows, and all `02-04` commands are unchanged; dependency graph is acyclic and all references resolve |
| Focused path audit | **PASS** — existing hydrator, BE artifact, and four expectation runner paths resolve |

No test, native-metal run, mutation run, production execution, DES event, or
commit was performed for these checks.

### Findings and dispositions

No blocker, critical, high, medium, or low finding remains within this narrow
roadmap delta.

| Observed boundary | Disposition |
|---|---|
| BE-02 no longer requests the unsupported two-allocation precondition | **Approved correction.** One production-scheduled allocation and all three listeners preserve the observable singleton contract; multi-allocation membership/order remains explicitly owned by #282. |
| BE10 needs local unhealthy action selection after an authoritative health withdrawal | **Approved D7 mapping.** The existing hydrator selects the existing deregistration action while retaining the full candidate vector; no new consumer protocol or lifecycle gate is required. |
| Prior `02-03` RED/GREEN-failed/skipped-COMMIT history is present | **Historical disposition.** Preserve it exactly; a new crafter independently reruns RED/GREEN/COMMIT under revision 4. No implementation completion is inferred. |
| Full integrity still lacks downstream entries | **Expected pending state.** This review validates roadmap readiness only and does not authorize fabricated DES phases or execution claims. |

The #282 deferral, unexecuted BE10 recovery suffix, and native unhealthy-routing
boundary are accepted limits already pinned by the approved DESIGN and DISTILL
artifacts. They do not authorize a new roadmap step or architectural mechanism.

### Approval metadata disposition

At review start, `roadmap.validation.status` was correctly `pending` for this
fresh delta. With no finding requiring remediation, this review authorizes the
owning workflow to mark that metadata **approved** using this review ID and
timestamp. The update preserves the pre-amendment approval provenance, the
revision-3/E09-v2 amendment review ID and scope, and the exact former pending
amendment text as historical metadata. The focused amendment's status is also
updated to point to this review; neither update claims implementation, native
verification, or mutation completion.

### Amendment verdict

**APPROVED.** The revision-4 roadmap delta is mechanically coherent and
actionable: `02-03` adds only the approved existing ServiceMapHydrator
healthy-Register/unhealthy-Deregister selection and its bounded path/test
fallout; BE-02 now honestly exercises one production-scheduled allocation with
all three listeners; `02-04` and its E09-v2/E10/E13 commands are unchanged;
and `03-01` remains sequenced after `02-04`. Prior approval provenance and
execution history remain intact. This approval is limited to the focused
roadmap amendment and makes no production, native, DES, or mutation completion
claim.
