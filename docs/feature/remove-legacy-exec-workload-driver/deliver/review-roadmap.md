# Independent DELIVER Roadmap Review — Remove legacy Exec workload driver

## Metadata

| Field | Value |
|---|---|
| Review ID | `remove-legacy-exec-workload-driver-roadmap-20260914-iterations-1-2` |
| Reviewer | `nw-solution-architect-reviewer` |
| Roadmap | `docs/feature/remove-legacy-exec-workload-driver/deliver/roadmap.json` |
| Review date | 2026-09-14 |
| Iteration | 1–2 — fresh review plus remediation re-review |
| Requirements authority | GH #293 body and empty comment thread; user-approved P-293-1 through P-293-6 |
| Roadmap validation field | Remains `pending`; this review does not modify the reviewed artifact |
| Verdict | **APPROVED after iteration 2** |

## Scope and authority

This review evaluates implementation readiness only. It does not revise the
accepted architecture, authorize GH #295 work, modify production/tests, run a
DELIVER step, capture E14, execute mutation testing, or approve the roadmap's
pending validation field.

The review read GH #293 with comments, the complete feature delta, ADR-0110
through ADR-0113, the approved DESIGN review, the corrected DISTILL acceptance
specification and RED classification, and the executable DISTILL delta in
commit `2944643fd25398aff0a0f5a597360723efe841ba`. It applied the repository
rules and the mandatory `nw-sar-critique-dimensions` and
`nw-roadmap-review-checks` skills. The user-approved greenfield contract
overrides generic compatibility assumptions.

## Mechanical evidence

- `des.cli.roadmap validate` with the required
  `PYTHONPATH=/Users/marcus/.claude/lib/python` returned
  `VALID: 3 phases, 6 steps`.
- The JSON parses and declares six steps. Dependencies are the acyclic strict
  chain `01-01` → `01-02` → `01-03` → `02-01` → `02-02` → `03-01`; every
  dependency resolves to an earlier step.
- No `TODO`, `TBD`, or `FIXME` remains. Uses of “placeholder,” “compatibility,”
  and “fallback” are prohibitions, not unfinished work.
- The roadmap contains 1,481 words across JSON string values, within the
  1,500-word limit for four to eight steps. Step descriptions are 24–30 words,
  each step has five criteria, the longest criterion is 21 words, and notes are
  68–87 words.
- The roadmap states 37 affected production Rust files, giving `6 / 37 = 0.16`.
  An independent conservative symbol search found 39 current production Rust
  files; either denominator is comfortably below the `2.0` step/file limit.
  No three-step substitution pattern exists.
- Every primary `test_file` and `scenario_name` resolves to an existing path
  and function/mode. E14's directory is intentionally future output of
  `03-01`, not a missing precondition.
- Commit `2944643f` contains the approved minimal executable delta: one new
  generic unsupported-key property and four existing-test transitions across
  three existing Rust test files. It adds no feature-specific test file or
  pending scaffold.

## Mandatory roadmap checks

| Check | Verdict | Evidence and disposition |
|---|---|---|
| External validity | **PASS** | `01-01` retains parser/HTTP no-write evidence, `01-02` reaches the real server composition boundary, `01-03` retains production action-shim ordering, and `03-01` drives the checked-in VM Service example through the built default-feature binary on native metal. The result is invocable through `overdrive serve` plus `overdrive deploy`, not only isolated components. |
| Acceptance-criterion implementation coupling | **PASS under the exact DESIGN contract** | Criteria name `WorkloadSpecInput::exec_command`, `DriverRegistry`, `AllocDriverIndex`, `RestartAllocation`, and the four V1 envelopes only where accepted DESIGN pins retain/delete/reset behavior. No criterion invents a private decomposition, alternate signature, type, variant, parameter, or renamed public surface. F-01 concerns evidence-boundary wording, not exact-contract naming. |
| Step decomposition | **PASS** | The six steps isolate the two schema cuts around their coupled enum changes, then documentation and black-box evidence. The dependency graph is executable and the 0.16 conservative ratio is well below the limit. |
| Implementation code | **PASS** | The roadmap contains exact accepted type/owner names and observable ordering, but no method body, loop, algorithm, conditional, pseudocode, or implementation snippet. |
| Concision and precision | **PASS** | Total and local word counts pass. Criteria are testable and single-interpretation under the referenced feature delta. Adding the required final gate must be offset by consolidating the six repeated “No mutation testing” sentences so the roadmap stays at or below 1,500 words. |
| Unit/acceptance boundary validation | **FAIL — F-01** | Rust evidence otherwise remains in-process and E14 remains external built-product evidence. The `01-02` and `02-01` test-paradigm notes incorrectly instruct tests to assert private exit-observer task ownership that DISTILL expressly assigns to reviewer diff audit. |

## Accepted-contract coverage

| Contract | Roadmap result |
|---|---|
| Pure greenfield cut; no compatibility surface | **PASS.** Old readers, aliases, disabled variants, retired diagnostics, fallbacks, migrations, old fixtures, and renamed public equivalents are prohibited in criteria, notes, and excluded patterns. |
| `[vm]` is the sole driver grammar; generic invalid behavior remains | **PASS.** `01-01` covers VM roundtrips, missing supported-driver input, bounded generated unsupported keys, and production no-write behavior without naming a removed spelling as the test subject. |
| Delete production symbols and dedicated tests together | **PASS.** `01-01`, `01-02`, and `02-01` explicitly pair parser/prober/driver/lifecycle deletion with deletion of their dedicated tests and forbid replacement source-shape/absence suites. |
| Exactly four incompatible current V1 resets | **PASS.** `01-01` owns `ServiceSpecEnvelope` and `WorkloadIntentEnvelope`; `02-01` owns `AllocStatusRowEnvelope` and `AllocLifecycleOccurrenceRowEnvelope`; unrelated envelopes remain unchanged. |
| Preserve VM-only tagged unions, registry/index and per-driver observers | **PASS on implementation contract; evidence wording requires F-01 remediation.** `01-02` retains `Driver`, `DriverRegistry`, `AllocDriverIndex`, `VmDriver`, `Vmm`, optional VMM composition, VM exit provenance, supervision release, and clean shutdown. |
| Preserve HTTP/TCP probes | **PASS.** `01-01` retains target projection, role/index, thresholds, publication, cancellation, re-registration, and the Earned-Trust probe gate. |
| Preserve P-105 and current VM effect order | **PASS.** `01-03` retains accepted predecessor/distinct durably reserved successor identity, successor-first result precedence, one exact-old cleanup attempt, and Running → interception → guest-command release. |
| Delete `WorkloadSpecInput::exec_command` without replacement | **PASS.** `01-01` repeats the exact public-API disposition and forbids an accessor substitute. |
| GH #295 is constraint-only | **PASS.** Shared switching, per-tap classification/interception, shared-bridge DNS, replacement transparent mTLS, cross-host routing, density, and throughput are excluded from code, tests, examples, and claims. |
| Schedule-racy P-105 oracle is outside #293 | **PASS.** `01-03` and the global exclusions prohibit both a test change and a production fix. |
| E06/E08 historical; E14 post-cut only | **PASS.** `02-02` and `03-01` preserve the historical receipts and constrain E14 to the checked-in VM Service example, built binary, stakeholder-visible outcome, cleanup, SHA/substrate/dirty-state evidence, and independent review. |
| Current documentation names the supported microVM model | **INCOMPLETE — F-02.** Operator examples, README, product jobs, journeys, and one persona are named, but the whitepaper's active Exec claims are outside the explicit acceptance/file boundary despite being named by the accepted feature contract. |
| Single final mutation gate after all step reviews | **INCOMPLETE — F-03.** Per-step mutation is correctly forbidden, but the final gate has no executable command, threshold, or structured result requirement. |

## Compatibility and unnecessary-work audit

No roadmap step preserves or manufactures a compatibility reader, legacy
alias, disabled/deprecated arm, migration bridge, old-byte fixture, retired
driver error, special deleted-spelling branch, replacement accessor, or renamed
host-process driver. `AllocationAttemptEvent::Dispatch` and
`guest_command_release_permitted` are the exact approved private semantic
renames; the roadmap correctly gives them no name/shape test.

The roadmap does not recreate the rejected deletion suite. It retains the one
approved bounded unsupported-key property and uses existing VM/parser/codec/
probe/lifecycle evidence. `03-01` does not inline a spec, invoke Rust tests,
link an `overdrive-*` crate, or duplicate integration assertions.

No GH #295 implementation is hidden behind a broader label. The retained
netns/veth/TAP/address/MAC/gateway/prefix/DNS fields are the current VM-used
mechanism required by #293; the roadmap neither deletes them prematurely nor
selects their replacement.

## Findings

### F-01 — High: private exit-observer ownership is incorrectly assigned to tests

**Roadmap locations:** `roadmap.json:70,97,149,179`

`01-02` says bounded composition tests must declare “observer tasks” and assert
their complement. `02-01` similarly directs bounded owner tests to assert
“row/observer/supervision deltas.” This goes beyond the approved executable
boundary. `ServerHandle.exit_observer_tasks` and the shared shutdown token are
private, and no driving port exposes task-vector cardinality or token identity.
DISTILL explicitly adds no test-only accessor or large composition suite and
assigns exact one-task-per-registry-entry, clone-one-token, and join-all
ownership to implementation-review audit (`distill/test-scenarios.md:271-283`;
`feature-delta.md:937-941`). Existing tests prove only the behavioral boundary:
registry `∅ | {Vm}`, VM exit publication/provenance, supervision release, and
clean bounded shutdown.

Following the roadmap literally would either manufacture an unapproved test
seam/private-module test or expand the deliberately minimal DISTILL delta.

**Required remediation:** Rewrite the `01-02` and `02-01` test-paradigm notes
so executable tests assert only registry outcomes, trusted-runner delivery, VM
exit publication/provenance, lifecycle rows, supervision release, and clean
bounded shutdown. State that exact observer-task cardinality, shared-token
identity, and join-all ownership are reviewer diff checks only. Add no test
function, test accessor, source-shape check, or private-module import. The
implementation criteria may continue to require the accepted ownership, but
must not present its private shape as an executable test oracle.

### F-02 — High: the active whitepaper migration is missing from the explicit step boundary

**Roadmap locations:** `roadmap.json:184-220,255-269`

The accepted documentation contract expressly requires current `README.md`,
the whitepaper, architecture brief, product jobs/journeys, outcome registry,
OpenAPI, crate/module rustdoc, and current examples to name the microVM family
as supported (`feature-delta.md:764-768`). DESIGN already updated the focused
brief/outcome records, but the current whitepaper still lists `exec` as a live
driver and says the universal enforcement mechanism covers “process/exec”
workloads (`docs/whitepaper.md:505-524,730-744`).

`02-02` explicitly accepts examples, README, jobs, journeys, and persona
guidance and lists their paths, while neither its criteria/files nor global
source scope names `docs/whitepaper.md`. The note that paths are guidance
allows tightly bounded fallout, but it does not make the omitted authoritative
SSOT an acceptance obligation. A fresh crafter can satisfy every written
criterion while leaving the active platform design claiming support for the
deleted driver.

**Required remediation:** Add the current whitepaper to `02-02`'s observable
documentation criterion and `files_to_modify`, and include it in the roadmap
source scope. Limit the edit to active current-support claims; historical ADR
and evolution prose remains excluded. Keep other newly discovered current-doc
files as tightly bounded fallout under the existing “paths are guidance” note,
including any current persona whose active promise still names host-process
execution.

### F-03 — High: the final DELIVER mutation gate is narrative, not executable

**Roadmap locations:** `roadmap.json:227,250`

The roadmap correctly forbids mutation testing in each implementation step and
says one final gate belongs to the orchestrator. It does not provide the gate's
command, required feature surface, ≥80% result, or structured evidence. The
repository's canonical diff-scoped command is
`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`
on this macOS workspace (`.claude/rules/testing.md:1020-1040`), and the
authoritative result is `target/xtask/mutants-summary.json` with a required
kill rate of at least 80% (`testing.md:1174-1188`). Repository DELIVER rules
place this single gate after every roadmap step and its review.

Without an executable wave-level gate, all six steps and E14 review can finish
while mutation remains an unverified prose reminder.

**Required remediation:** Add one roadmap-level `final_mutation_gate` after
all six step approvals and the independent E14 evidence review. Name the
canonical Lima-wrapped `cargo xtask mutants --diff origin/main --features
integration-tests` command, require a passing structured summary and ≥80% kill
rate, and keep mutation forbidden during individual steps. Consolidate the six
repeated “No mutation testing” sentences into this single gate so the revised
roadmap remains within the 1,500-word concision ceiling. Do not edit mutation
exclusions as feature work.

## Architecture quality dimensions

| Dimension | Assessment |
|---|---|
| Architectural bias | **PASS.** The roadmap deletes surface and reuses the existing VM/registry/codec/probe/action-shim architecture. It adds no technology, service, daemon, store, retry owner, migration mechanism, or GH #295 design. |
| ADR quality | **PASS.** ADR-0110 through ADR-0113 remain focused, alternatives-based, consequence-complete, and user-approved. The roadmap does not modify or reinterpret them. |
| Completeness | **FAIL on F-02/F-03.** Runtime/API/schema scope is complete, but one authoritative current-document target and the executable final mutation gate are absent. |
| Implementation feasibility | **PASS subject to F-01.** Current code exposes all required deletion and VM preservation paths. No new public API or production testability seam is needed; F-01 must be corrected so a crafter is not told otherwise. |
| Priority validation | **PASS.** The roadmap removes live Exec behavior, changes none of P-105, and implements none of GH #295. Scalar-driver collapse and compatibility retention remain rejected. |
| Effect isolation | **PASS for production contracts.** The accepted bounded universes remain represented. F-01 is an evidence-lane mismatch: private ownership is review-audited, while tests observe its public behavior. |

## File-scope and compiler-fallout assessment

The file lists are guidance rather than restrictive allowlists, consistent with
the repository rule. Broad source/test directories permit compiler-required
call-site, import, generated OpenAPI, manifest/dependency, and fixture fallout.
Each step's criteria and negative scope constrain those additions to the
approved deletion/preservation contract; the broad lists are not authority for
adjacent hardening, generalized lifecycle work, a new test seam, or GH #295.

The one exception is F-02: because the accepted whitepaper change is a named
product outcome rather than incidental compiler fallout, it must be visible in
the step's criterion and file scope. Adding it does not authorize rewriting
historical architecture records.

## Verification performed

| Check | Result |
|---|---|
| GH #293 body and comments | Read; issue open, comment list empty, handoff remains constraint-only |
| Roadmap JSON parse and DES validation | PASS — `VALID: 3 phases, 6 steps` |
| TODO/dependency/atomic-chain audit | PASS |
| Word-count and local concision audit | PASS — 1,481 total string words |
| Primary path/function/mode locator audit | PASS |
| Commit `2944643f` approved executable delta audit | PASS — one new property, four existing-test transitions, zero new feature test files/pending scaffolds |
| Compatibility/renamed-equivalent/GH #295 audit | PASS |
| Exact API/schema disposition comparison | PASS |
| Test/verification boundary comparison | FAIL only as F-01 |
| Current-document coverage comparison | FAIL only as F-02 |
| Final mutation-gate readiness | FAIL only as F-03 |
| Production tests, native-metal execution, E14 capture, mutation testing, DES execution, commits | Not run or created; outside this review |

## Finding disposition and next review

F-01 through F-03 are concrete roadmap-document defects within the already
approved scope. Their remediation requires no architecture invention, public
API, production code, test file, compatibility behavior, GH #295 work, or
change to the schedule-racy P-105 oracle. Revise only the roadmap and request a
fresh independent re-review. The roadmap must remain `pending` until that
review returns with zero unresolved high/blocking findings.

## Iteration-1 verdict

**CHANGES_REQUESTED**

There are zero critical/blocker findings and three unresolved high findings.
The implementation sequence and exact runtime/schema contract are otherwise
sound, but the roadmap is not yet ready for DELIVER dispatch.

## Iteration 2 — remediation re-review

### Conclusion

The roadmap remediation closes F-01 through F-03 without changing the accepted
P-293-1 through P-293-6 contract. Exact exit-observer task cardinality,
shared-token identity, and join-all ownership are now explicitly reviewer diff
audit only; executable evidence stays at the public production boundary. The
whitepaper is explicitly part of the current-document migration while
historical ADR/evolution prose and E06/E08 remain excluded. One executable
final mutation gate now runs after every step review and E14's independent
evidence review, requires the canonical Lima-wrapped command, a passing
structured summary, and at least 80% kill rate, and prohibits exclusion changes
without the explicit approval required by repository rules.

No new compatibility surface, deleted-name/source-shape test, public API,
architecture mechanism, GH #295 implementation, P-105 change, or unnecessary
test work entered. The six-step sequence remains atomic and executable for
fresh isolated DELIVER crafters under the authoritative feature delta and
DISTILL evidence map.

### Finding dispositions

| Finding | Iteration-2 disposition | Independent evidence |
|---|---|---|
| F-01 — private exit-observer ownership assigned to tests | **CLOSED** | `01-02` now limits production-boundary tests to registry outcomes, trusted-runner delivery, VM exit publication/provenance, supervision release, and clean shutdown. It assigns exact observer-task cardinality, shared-token identity, and join-all ownership to reviewer diff audit and forbids a test seam, new function, private import, or source-shape check (`roadmap.json:68-72,97`). `02-01` carries the same split for lifecycle evidence (`:145-149,179`). This matches DISTILL's explicit limitation (`distill/test-scenarios.md:271-283`) and the approved feature delta (`feature-delta.md:937-941`). |
| F-02 — whitepaper absent from current-document migration | **CLOSED** | `02-02` now names the whitepaper in its observable criterion, `files_to_modify`, and global source scope (`roadmap.json:186,200,268`). Its notes constrain edits to active whitepaper claims and continue to prohibit historical ADR/evolution changes, E06/E08 changes, and GH #295 claims (`:221`). This covers the active driver/mTLS claims previously identified at `docs/whitepaper.md:505-524,730-744` without rewriting historical records. |
| F-03 — final mutation gate only narrative | **CLOSED** | The new wave-level gate runs after all six approved step reviews and independent E14 evidence review; invokes `cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`; requires exit 0 plus `target/xtask/mutants-summary.json` with `status=pass`, `kill_rate_pct>=80`, and outcome counts; retains the full summary; runs once; and forbids per-step mutation or exclusion changes without explicit approval (`roadmap.json:294-300`). Under the binding repository rule, that approval is explicit user approval, not crafter/reviewer discretion. |

### Mandatory roadmap checks — iteration 2

| Check | Verdict | Evidence |
|---|---|---|
| External validity | **PASS** | Production admission/no-write, server/VMM composition, action-shim lifecycle ordering, and the E14 built-binary native-metal journey remain intact. No hand-wired test composition substitutes for `serve`/`deploy`. |
| Acceptance-criterion implementation coupling | **PASS under exact DESIGN** | Criteria retain only contractually pinned public/schema/owner names. The remediation adds no new method signature, internal algorithm, private decomposition, or public surface. Private observer structure is now review-audit only. |
| Step decomposition | **PASS** | The acyclic chain remains `01-01` → `01-02` → `01-03` → `02-01` → `02-02` → `03-01`. Six steps over the conservative 37-file baseline remain a 0.16 ratio with no substitution-pattern over-decomposition. |
| Implementation code | **PASS** | The added wording describes evidence boundaries, active-document scope, and one gate command. It adds no production algorithm, body, loop, conditional, pseudocode, or implementation mechanism. The mutation command is a verification invocation, not implementation code. |
| Concision and precision | **PASS** | The remediated roadmap contains 1,497 words across JSON string values, below the 1,500-word limit. Descriptions remain 24–27 words, each step has five criteria, the longest criterion is 21 words, and notes remain 65–76 words. |
| Unit/acceptance boundary validation | **PASS** | Rust tests remain in-process and observe parser, codec, registry, VM exit, lifecycle, supervision, and clean-shutdown behavior. Private task/token structure has no executable/source-shape test. E14 remains a built-product black-box capture with no Rust test invocation or crate import. |

### Exact-contract and residue re-audit

| Audit target | Result | Evidence and disposition |
|---|---|---|
| VM-only live grammar and generic invalid-input behavior | **PASS** | `01-01` remains bounded to successful VM payloads, ordinary missing/unsupported-driver behavior, and no intent write. No removed spelling or retired diagnostic becomes an executable contract. |
| Public/API/schema disposition | **PASS** | `WorkloadSpecInput::exec_command` still deletes without replacement; live tagged unions remain VM-only; registry/index/observer ownership remains; exactly the two specification/intent and two lifecycle envelope owners reset to current incompatible V1. No divergent API is authorized. |
| Dedicated deletion tests or renamed equivalents | **PASS** | The roadmap still forbids `remove_*.rs`, trybuild, compile-pass/fail, source-token/source-shape, absence, old-byte, and deleted-name tests. `AllocationAttemptEvent::Dispatch` and `guest_command_release_permitted` retain no shape test. F-01 remediation removes the only private-shape test ambiguity. |
| Compatibility garbage | **PASS** | Every mention of alias, reader, fallback, disabled/deprecated variant, retired diagnostic, migration, fixture, or compatibility is a deletion/exclusion instruction. No replacement host/native/local/command driver or probe placeholder appears. |
| HTTP/TCP, VM routing, exit and lifecycle preservation | **PASS** | The surviving behavior criteria retain HTTP/TCP target/role/threshold/result/cancellation semantics, `DriverRegistry`, `AllocDriverIndex`, VMM absence/refusal, VM exit provenance, supervision release, and clean bounded shutdown. |
| P-105 and schedule-racy oracle | **PASS** | Accepted predecessor/fresh durably reserved successor, successor-first result precedence, and one exact-old cleanup attempt remain. The pre-existing schedule-racy oracle and any production fix remain explicitly excluded. |
| GH #295 boundary | **PASS** | Current VM netns/veth/TAP/address fields remain; no shared switch, per-tap classification/interception, shared-bridge DNS, replacement transparent mTLS, cross-host routing, density, or throughput mechanism/claim enters. |
| Documentation and history | **PASS** | README, current whitepaper, product jobs/journeys/personas, active examples, generated OpenAPI, and necessary current fallout are covered. Historical ADR/evolution prose and E06/E08 receipts remain unchanged. |
| E14 evidence boundary | **PASS** | `03-01` still drives only the checked-in healthy VM Service example through the built default-feature product on native metal, captures actual outcome/cleanup/SHA/substrate/dirty state, and requires a different reviewer. It neither invokes Rust tests nor imports Overdrive crates. |
| Final mutation gate | **PASS** | Exact command, ordering, structured result, ≥80% kill rate, one-wave execution, and exclusion-change prohibition are explicit. No per-step mutation instruction remains. |

### Architecture quality dimensions — iteration 2

| Dimension | Assessment |
|---|---|
| Architectural bias | **PASS.** The remediation adds only roadmap evidence and documentation scope; it introduces no technology or mechanism. |
| ADR quality | **PASS.** ADR-0110 through ADR-0113 and their exact feature-delta contracts remain unchanged. |
| Completeness | **PASS.** Runtime/API/schema deletion, current documentation, evidence boundaries, E14, and the final mutation gate are all represented. |
| Implementation feasibility | **PASS.** Each step can reach GREEN/COMMIT without a compatibility shim, new public API, private test seam, or work owned by another step. |
| Priority validation | **PASS.** Live Exec removal remains 100%; P-105 semantic change and GH #295 implementation remain 0%. |
| Effect isolation | **PASS.** Production bounded-change universes retain their observable behavioral evidence, while non-port-exposed observer ownership remains reviewer audit. |

### Verification performed — iteration 2

| Check | Result |
|---|---|
| Current roadmap JSON parse and DES validation with required `PYTHONPATH` | PASS — `VALID: 3 phases, 6 steps` |
| Dependency/TODO/atomicity audit | PASS — strict resolved chain; no `TODO`, `TBD`, or `FIXME` |
| Total/local word-count audit | PASS — 1,497 total string words; all local limits pass |
| F-01 evidence-boundary comparison against DISTILL | PASS — private task/token/join structure is review-only |
| F-02 active-document scope | PASS — `docs/whitepaper.md` appears in criterion, step file list, and global source scope |
| F-03 final mutation gate | PASS — exact command, ≥80%, structured summary, final-only ordering, no discretionary exclusion edits |
| Compatibility/deleted-name/renamed-equivalent audit | PASS |
| GH #295 and P-105 boundary audit | PASS |
| Exact API and four-envelope comparison | PASS |
| Roadmap validation status | Correctly remains `pending` for orchestrator recording after this verdict |
| Production tests, native-metal execution, E14 capture, mutation testing, DES execution, commits | Not run or created; outside roadmap re-review |

## Final verdict after iteration 2

**APPROVED**

F-01 through F-03 are closed. There are zero unresolved critical, blocker, or
high findings. The remediated roadmap is implementation-ready for the strict
fresh-crafter → fresh-reviewer DELIVER sequence, with the single final mutation
gate after all step and E14 evidence approvals. The roadmap's
`validation.status` remains `pending` for the root orchestrator to update after
recording this on-disk verdict.
