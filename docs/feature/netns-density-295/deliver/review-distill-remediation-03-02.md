# DISTILL review — step 03-02 EXEC-oracle remediation

- **Feature:** `netns-density-295`
- **Scenario:** `S-ND295-28`
- **Roadmap step:** `03-02`
- **Review type:** fresh isolated `nw-acceptance-designer-reviewer`
- **Review date:** 2026-09-22
- **Reviewed range:** `ba1d2f583823ed4a633b4068de9adf14a14699a5..7a50786553748608097b3e1fb2e26c5664a689ec`
- **Reviewed commit:** `7a50786553748608097b3e1fb2e26c5664a689ec`
- **Final verdict:** **CHANGES_REQUIRED**

## Review boundary and authority

This review is limited to the acceptance-oracle correction in:

- `crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs`;
- `docs/feature/netns-density-295/distill/test-scenarios.md`; and
- `docs/feature/netns-density-295/distill/red-classification.md`.

The controlling contract is the accepted EXEC-close linearization in
`feature-delta.md`, approved decision `D-295-DELIVER-03-01`, S-ND295-28, and
roadmap step `03-02`. The concrete production path reviewed is
`VmDriver::release_for_exit_emission -> GuestNetworkExecGate::claim_release ->
pending_exec.take -> BeaconWriter::release_exec -> run_beacon_writer`. The wire
authority is the existing line-oriented `BeaconMessage` Published Language.

No production change, API addition, private `active_claims` observation, seeded
writer surrogate, or architecture change is authorized by this review.

## Strengths

- The correction removes the contradicted zero-raw-prefix oracle. It now allows
  a claim linearized before detection to make partial progress or finish, as
  `feature-delta.md:3870-3877` requires.
- The body still drives the real `VmDriver`, production-owned `BeaconWriter`,
  real Unix beacon socket, forced backpressure, Open-to-Recovering transition,
  structured task cancellation, cancellation join, and EOF.
- The test imports the existing production `BeaconMessage` parser and does not
  duplicate the protocol grammar or inspect private gate storage.
- The function identity and reasoned `#[ignore]` marker are unchanged. All three
  pending bodies in the changed file carry the exact
  `/// CONTRACT_SHAPE: bounded-change.` declaration and the repository's
  accepted `/// Outcome anchor: DISCUSS Elevator Pitch` marker.
- The scenario matrix now names all eight S-ND295-28 bodies. The GREEN
  classification explicitly authorizes no production change and states that
  the other step-`03-02` schedules retain independent gates.

## Findings and severity

| ID | Severity | Disposition |
|---|---|---|
| DR-03-02-01 | **Blocker** | Open — the corrected oracle discards complete malformed/interleaved frames, so it can hide the duplicate-writer behavior the remediation must detect. |

### DR-03-02-01 — Complete parser failures are silently removed from the observable universe

**Evidence:** `netns_density_exec_release.rs:248-253` splits the observed stream
at newlines, but then applies two fallible `filter_map` operations:

```rust
.filter_map(|frame| std::str::from_utf8(frame).ok())
.filter_map(|line| line.parse::<BeaconMessage>().ok())
```

The final assertion at lines 254-262 counts only the messages that survived
both filters. A newline-terminated frame that is invalid UTF-8 or rejected by
the production parser disappears, and the `<= 1` assertion can still pass.
That contradicts the self-audit's claims that typed newline-framed messages are
the complete observable universe and that interleaving/duplicate-writer
behavior is not hidden (`test-scenarios.md:790-801`).

**Falsifier:** two independently valid writer payloads such as
`EXEC ["a"]\n` and `EXEC ["b"]\n` can be partially interleaved on one byte
stream as `EXEC ["aEXEC ["b"]\n"]\n`. Both resulting newline-complete frames
are rejected by `BeaconMessage`; the current `filter_map` chain returns zero
messages and the acceptance assertion passes. The existing parser's rejection
of malformed EXEC JSON is independently executable and passed under Lima. This
is an oracle weakness, not a claim that current production has two writers, so
no production remediation is authorized.

**Required acceptance-test remediation:** retain the existing newline framing
and production `BeaconMessage` parser, but make every newline-terminated frame
part of the fail-closed observable universe. A complete frame must either parse
as the one permitted `Exec` or fail the test as wire corruption/interleaving;
only the final unterminated suffix may be ignored. The body must continue to
permit zero or one complete EXEC, reject a genuine second complete EXEC, reject
malformed/interleaved complete frames and unexpected typed message kinds, and
preserve the cancellation join and EOF assertions. Do not add a parser, hook,
API, private-state oracle, or production change.

After the body is corrected, rerun the same writable-Lima selector and align the
two DISTILL documents so their observable-universe and GREEN claims describe
the fail-closed typed-frame oracle exactly.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 10/10 | Six or more of the eight mapped S-ND295-28 bodies are recovery, fail-stop, cancellation, backpressure, stop-race, or writer-failure schedules; error/edge coverage exceeds 40%. |
| Given-When-Then form | 10/10 | S-ND295-28 retains one Given, one When, and three observable Then clauses. |
| Business/domain language | 8/10 | The title and outcome remain guest-command/recovery language. `EXEC`, fail-stop, ownership, and Published Language framing are established low-level domain terms for this contract. |
| Coverage completeness | 6/10 | The intended 0-or-1 complete-command boundary is present, but malformed/interleaved complete frames are omitted from the oracle. |
| Walking-skeleton user-centricity | 10/10 | Not a walking skeleton; the criterion is not applicable and therefore passes. |
| Priority validation | 10/10 | The change addresses only the reproduced contradiction between partial socket progress and the former raw-prefix count. |
| Observable behavior assertions | 4/10 | The test stays on public task/socket/parser observations, but silently drops part of the observable wire output and is not fail-closed. |
| Traceability coverage | 9/10 | S-ND295-28 maps to the outcome registry, the exact scenario matrix, and roadmap `03-02`; environment-to-walking-skeleton mapping is not applicable to this focused schedule. |
| Walking-skeleton boundary proof | 10/10 | Not applicable; the real production-owner boundary is assessed separately below. |

The score gate fails because observable-behavior assertions are below 5 and a
blocker remains.

## Test-design mandates and three pillars

| Gate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal/port boundary | PASS | The corrected body calls the existing `Driver::release_for_exit_emission` operation on a real `VmDriver` and observes the production beacon port; it does not invoke `run_beacon_writer` or private state directly. |
| CM-B — domain-language abstraction | PASS | The scenario expresses recovery, guest-command release, fail-stop, cancellation, and ownership; parser mechanics remain in the technical mapping/body. |
| CM-C — complete journey | PASS | Open claim, writer transfer/backpressure, recovery detection, cancellation, join, EOF, and command complement form one complete focused journey. |
| CM-D — pure logic/fixture isolation | PASS | No new business algorithm or fixture matrix was introduced; the existing production parser is reused. |
| CM-E — bounded-change universe | **FAIL** | Parser errors and unexpected complete frame kinds are silently excluded rather than asserted fail-closed. |
| CM-F — layer-dependent PBT mode | PASS | The real-I/O owner schedule remains one explicit example; generative operation coverage stays in the existing core model property. |
| CM-G — two-tier rich journey | PASS / N/A | The existing model PBT plus production-owner examples already provide the accepted two evidence layers; no parallel property suite is warranted. |
| CM-H — real-I/O sad paths | PASS | Backpressure/cancellation remains an explicit, named example with no PBT loop over real I/O. |

Pillar 1 (domain language) and Pillar 2 (chained state narrative) pass. Pillar 3
(production composition) passes because `VmDriver`, its retained
`BeaconWriter`, and the real socket are used. CM-E nevertheless blocks handoff.

## Completeness taxonomy

| Category | Result | Bounded S-ND295-28 evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | The accepted message boundary is zero or one complete EXEC, with a second forbidden; partial unterminated progress is a separate allowed class. |
| C2 — state and transition | PASS | The core model names BootClosed/Open/Recovering/FailStop, exercises legal sequences, and has an illegal-event table for every state. |
| C3 — count cardinality | PASS | Generated claim/wait schedules cover zero/one/many actors; deterministic writer schedules cover the command cardinality boundary. |
| C4 — lifecycle and idempotency | PASS | Repeated open/fail-stop/late transitions, claim Drop, cancellation, stop, writer consumption, and owner teardown are mapped. |
| C5 — mode/decision table | PASS / N/A | No mode flag is introduced; closed component/cause tables remain independently exhaustive. |
| C6 — negative and robustness | **FAIL** | Complete malformed/interleaved frames are silently discarded, leaving the wire failure set open. |
| C7 — environment/interruption/concurrency | PASS | Forced socket backpressure, cancellation mid-write, stop races, waiting tasks, and concurrent supervisor transitions are present. |

### Mechanical 15-item checklist

| Item | Result | Rationale |
|---|---|---|
| C1a | PASS | Zero complete commands is allowed and observed when cancellation wins before newline completion. |
| C1b | PASS | The 0/1/2 command boundary is explicitly stated; 0/1 pass and 2 must fail. |
| C2a | PASS | The S-ND295-27/28 documentation and core `ModelState` define the four-state model used by the generated schedule. |
| C2b | PASS | `illegal_event_from_every_gate_state_is_rejected` covers BootClosed, Open, Recovering, and FailStop. |
| C3 | PASS | Operation-sequence PBT generates zero/one/many active and waiting claims; the finite real-owner schedules cover writer cardinality. |
| C4a | PASS | Repeated and late gate operations are covered by the model and explicit idempotence assertions. |
| C4b | PASS / N/A | A claim cannot be dropped without first owning the opaque RAII value; the type makes the inverse-without-prerequisite operation unrepresentable. |
| C5a | PASS / N/A | No mode flags exist in this bounded correction. |
| C5b | PASS / N/A | No flag orthogonality claim exists. |
| C6a | **FAIL** | A complete malformed/interleaved wire frame is not asserted; it is filtered out. |
| C6b | PASS | Recovery, FailStop causes, cancellation, stop, writer completion/error/EOF/absence, and action-owner cancellation have named schedules. |
| C6c | **FAIL** | The complete observed-frame result set is not closed because parser failures and unexpected typed kinds are omitted. |
| C7a | PASS | A 4 KiB receive buffer against a 16 MiB EXEC forces degraded-resource backpressure. |
| C7b | PASS | Cancellation and stop are exercised during the backpressured write. |
| C7c | PASS | Release, writer, supervisor, waiter, stop, and dispatch owners execute concurrently in the mapped set. |

**Mechanical count: 13/15 — COMPLETE.** The deterministic threshold does not
override the explicit C6 blocker or failed CM-E universe gate.

## Port, universe, complement, and test budget

### Production boundary

The changed body uses the accepted production path. `release_for_exit_emission`
acquires `_exec_claim` before taking `pending_exec` and transfers the command to
the retained `BeaconWriter`. Aborting and joining the release task drops that
task-owned claim. EOF proves that no production write half remains able to
write to this beacon session. No private `active_claims` field, test hook, or
surrogate writer is used.

The task join plus EOF therefore support the claim-lifetime and no-surviving-
socket-writer conclusions. They do not repair the typed-frame complement:
because complete parser failures are discarded, the current count cannot
exclude wire corruption caused by an accidental second writer.

### Observable universe and complement

The intended universe is release-task completion, the entire production beacon
stream through EOF, and the typed result of parsing every complete frame. The
changed implementation observes the first two but retains only successful
members of the third. The complement is therefore incomplete until all
newline-complete frames are asserted.

### Test budget

The bounded S-ND295-28 set expresses five port-level behavior groups: gate
model/transition closure; recovery and FailStop release behavior; claim-before-
detection cancellation; stop/writer lifetime and backpressure; and action-shim
ownership. The budget is `5 × 2 = 10` bodies. The scenario matrix maps eight
bodies, so the set remains within budget. This commit adds no test body or
parallel PBT suite.

## RED-classification honesty and artifact scope

Current production genuinely passes the corrected pending body, so a GREEN
classification is honest in the narrow sense that this body does not require a
production change. GREEN does not prove that the oracle is non-weakening; the
blocker above must be corrected in test code and rerun.

The classification does not silently waive the rest of roadmap `03-02`:
`red-classification.md:32` says the body remains pending for activation and that
the other S-ND295-28 schedules retain independent gates; roadmap criterion 6
still requires every reasoned-ignore body in the file to be activated with no
weakening and no remaining ignore marker.

`test-scenarios.md` and `red-classification.md` are the minimum authoritative
documentation updates for this oracle correction. `feature-delta.md` already
permits pre-detection completion/partial progress and already assigns
claim-lifetime/cancellation evidence to S-ND295-28. Roadmap `03-02` already
requires the real writer/cancellation schedules and activation of every body.
Neither `feature-delta.md` nor `roadmap.json` needs amendment for this bounded
remediation.

## Verification

| Check | Result |
|---|---|
| Writable-Lima exact pending selector from `red-classification.md` | PASS — nextest run `daa64d78-3f7d-4002-9984-4f6af2427ab6`; 1 passed, 101 skipped, 0 failed. |
| Writable-Lima production-parser malformed-EXEC selector | PASS — nextest run `77215740-1c30-41e6-a0b9-7de80eb27984`; 1 passed, 528 skipped, confirming malformed complete EXEC input is rejected by the reused parser. |
| `cargo fmt --all -- --check` | PASS. |
| `git diff --check ba1d2f58..7a507865` | PASS. |
| `jq -e . docs/feature/netns-density-295/deliver/roadmap.json` | PASS. |
| Reviewed changed-file set | PASS — exactly the three authorized files. |
| Reviewed commit author/trailer | PASS — Marcus remains author; exactly one `Co-Authored-By: Codex <codex@openai.com>`; no Claude, Anthropic, or generated-by attribution. |
| Native macOS overdrive-core test listing | ENVIRONMENT-LIMITED — `linux-keyutils` requires Linux libc symbols. All authoritative Rust execution above used writable Lima. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

## Remediation dispositions

| Item | Disposition |
|---|---|
| Replace the contradicted zero-prefix assertion | Correct direction; preserve. |
| Use existing newline framing and `BeaconMessage` parser | Correct direction; preserve. |
| Silently discard complete frame decode/parse errors | Reject; make them fail-closed so interleaving cannot hide. |
| Cancellation join, EOF, at-most-one typed EXEC | Preserve. |
| Production/API/design/roadmap changes | Not authorized and not required. |
| DISTILL document claims | Realign only after the fail-closed oracle passes independently. |

# CHANGES_REQUIRED

# Iteration 2 re-review

- **Iteration:** 2
- **Review date:** 2026-09-22
- **Remediation commit:** `e74cf14c5ea799565abb30a4df447fd36f2d41d9`
- **Remediation range:** `484a7fdbb810dbe093f2d59813156b54d4c105af..e74cf14c5ea799565abb30a4df447fd36f2d41d9`
- **Cumulative implementation range:** `7a50786553748608097b3e1fb2e26c5664a689ec..e74cf14c5ea799565abb30a4df447fd36f2d41d9`
- **Reviewer:** same isolated `nw-acceptance-designer-reviewer`
- **Iteration-2 verdict:** **APPROVED**

This section is additive. It preserves iteration 1's evidence and
`CHANGES_REQUIRED` verdict as the historical record, then records the independent
re-review of the bounded remediation.

## Iteration-2 scope and authority

The re-review independently inspected the remediation to DR-03-02-01 in the
same three authorized files:

- `crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs`;
- `docs/feature/netns-density-295/distill/test-scenarios.md`; and
- `docs/feature/netns-density-295/distill/red-classification.md`.

The accepted authority remains the EXEC-close linearization,
`D-295-DELIVER-03-01`, S-ND295-28, and roadmap step `03-02`. No production,
API, architecture, design, roadmap, or execution-log change is part of the
remediation commit.

## Finding disposition

| ID | Iteration-1 severity | Iteration-2 disposition |
|---|---|---|
| DR-03-02-01 | Blocker | **CLOSED** — every newline-complete frame is now decoded and parsed fail-closed, and the complete typed slice is exactly empty or one `Exec`. |

### DR-03-02-01 closure evidence

The remediation replaces both fallible `filter_map` calls with a total review
of every complete frame (`netns_density_exec_release.rs:248-265`):

1. `split_inclusive` retains the existing newline framing;
2. only a final frame without `\n` is filtered out;
3. every newline-complete frame must decode as UTF-8 or the test panics;
4. every decoded complete frame must parse through the production
   `BeaconMessage` parser or the test panics; and
5. the collected typed slice must match exactly `[]` or
   `[BeaconMessage::Exec { .. }]` (`netns_density_exec_release.rs:266-271`).

This closes each falsifier from iteration 1:

| Frame class | Result after remediation |
|---|---|
| Invalid UTF-8 ending in newline | Fails during UTF-8 decoding. |
| Malformed or interleaved newline-complete frame | Fails through the existing `BeaconMessage` parser. |
| Unexpected typed message such as `Shutdown` | Parses, then fails the exact slice-shape assertion. |
| Two valid complete `Exec` messages | Parses both, then fails the exact slice-shape assertion. |
| Empty stream | Accepted as zero complete messages. |
| One valid complete `Exec` | Accepted as the sole complete message. |
| Final unterminated prefix | Ignored as permitted pre-detection partial progress. |
| One valid `Exec` plus a final unterminated prefix | Accepted; only the complete `Exec` enters the typed slice. |

A focused ephemeral Rust matrix linked against the workspace's production
`overdrive_core::vm::beacon::BeaconMessage` implementation and verified all
eight rows: four accepted classes completed normally and all four forbidden
classes failed closed. The original interleaving byte stream now terminates at
the production parser's `MalformedArgv` error rather than disappearing from the
oracle. The matrix created no repository file or permanent test body.

## Preservation and non-expansion check

The original schedule remains intact:

- same function identity
  `claim_before_detection_backpressure_and_cancellation_do_not_create_a_second_writer`;
- same reasoned `#[ignore]` marker for step `03-02`;
- exact `/// CONTRACT_SHAPE: bounded-change.` declaration and accepted Outcome
  anchor;
- real `VmDriver`, real retained `BeaconWriter`, Unix beacon socket, 16 MiB
  command, 4 KiB receive buffer, and forced backpressure;
- Open admission followed by the real Open-to-Recovering transition;
- release-task abort and joined cancelled `JoinHandle`;
- complete socket read through EOF; and
- no private `active_claims` observation.

The production path remains
`VmDriver::release_for_exit_emission -> GuestNetworkExecGate::claim_release ->
pending_exec.take -> BeaconWriter::release_exec -> run_beacon_writer`. The
remediation adds no parser, helper API, hook, seam, private counter oracle, test
body, production behavior, owner, or architecture. It changes only the
acceptance assertion over the already-observed stream.

The cancellation join continues to end the task-owned opaque claim lifetime;
EOF continues to prove no write half survives on the production beacon
session. The now-closed typed-frame universe additionally prevents malformed
interleaving or a second writer from being hidden.

## DISTILL SSOT and GREEN classification

`test-scenarios.md:752-806` now states the exact final contract: zero or one
complete typed `Exec`, fail-closed UTF-8/parser/kind/cardinality handling, and
only a final unterminated suffix excluded as partial progress. Its outcome,
Contract Shape, production boundary, observable universe, complement, and test
budget descriptions agree with the executable body.

`red-classification.md:20-39` records the same oracle, the iteration-1
falsifier as an oracle weakness rather than a production defect, the exact
pending selector, and an honest GREEN/already-present result. It explicitly
authorizes no production change and keeps every other S-ND295-28/step-`03-02`
schedule independently mandatory. No criterion is waived or silently skipped.

`feature-delta.md` and `roadmap.json` still require no amendment: the former
already permits pre-detection progress/completion and assigns production
claim/writer lifetime evidence to S-ND295-28; the latter already requires the
real writer/cancellation schedules plus activation of every reasoned-ignore
body without weakening.

## Iteration-2 acceptance dimensions

| Dimension | Score | Re-check |
|---|---:|---|
| Happy-path bias | 10/10 | Recovery, fail-stop, cancellation, backpressure, stop-race, and writer-failure coverage remains above 40%. |
| Given-When-Then form | 10/10 | One Given, one When, and observable Then clauses remain coherent. |
| Business/domain language | 8/10 | The user-valued title remains stable; low-level EXEC/framing vocabulary is the accepted Published Language contract. |
| Coverage completeness | 10/10 | Zero, one, second, malformed/interleaved, invalid UTF-8, unexpected kind, and unterminated-suffix classes are now distinguished. |
| Walking-skeleton user-centricity | 10/10 | Not applicable to this focused schedule. |
| Priority validation | 10/10 | Only the proven acceptance-oracle gap was remediated. |
| Observable behavior assertions | 10/10 | The entire newline-complete public socket output now participates fail-closed in the assertion. |
| Traceability coverage | 9/10 | S-ND295-28 remains mapped to its registered outcome and roadmap `03-02`; walking-skeleton environment mapping is not applicable. |
| Walking-skeleton boundary proof | 10/10 | Not applicable; the real production-owner boundary is preserved. |

All scored dimensions are at least 7, and no blocker remains.

## Iteration-2 mandate re-check

| Gate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal/port boundary | PASS | The test enters through the existing `Driver` operation on real `VmDriver` and observes the production beacon port. |
| CM-B — domain-language abstraction | PASS | Recovery, release, fail-stop, cancellation, and command ownership remain the scenario vocabulary; wire mechanics stay in the mapping/body. |
| CM-C — complete journey | PASS | Claim, backpressure, detection, cancellation, join, EOF, and complete complement remain one focused journey. |
| CM-D — pure logic/fixture isolation | PASS | No new business algorithm or fixture matrix; the existing production parser remains the SSOT. |
| CM-E — bounded-change universe | PASS | Every complete frame is now fail-closed; only the explicitly permitted incomplete suffix lies outside the typed universe. |
| CM-F — layer-dependent PBT mode | PASS | This real-I/O schedule stays example-based; the existing core model remains the sole PBT layer. |
| CM-G — two-tier rich journey | PASS / N/A | Existing model plus production-owner schedules remain the accepted evidence split; no parallel suite was added. |
| CM-H — real-I/O sad paths | PASS | Cancellation/backpressure remains one named example without a generated I/O loop. |

All three pillars pass: domain language remains stable, the state-transition
narrative is unchanged, and the application is exercised through production
composition.

## Iteration-2 completeness re-check

| Category | Result | Evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | Empty, one complete, second complete, and unterminated classes are separated. |
| C2 — state and transition | PASS | The four-state gate model, legal sequences, and illegal-event table remain mapped. |
| C3 — count cardinality | PASS | Zero/one/many claims plus zero/one/two complete command boundaries are represented. |
| C4 — lifecycle and idempotency | PASS | Repeated/late transitions, claim Drop, cancellation, stop, writer consumption, and teardown remain covered. |
| C5 — mode/decision table | PASS / N/A | No new mode flag; closed component/cause tables remain independent. |
| C6 — negative and robustness | PASS | Invalid UTF-8, parser rejection, unexpected typed kind, interleaving, and second EXEC all fail closed. |
| C7 — environment/interruption/concurrency | PASS | Real backpressure, mid-write cancellation, task joins, waiters, and concurrent transitions remain present. |

The 15-item mechanical checklist now passes **15/15**. Iteration-1's C6a and
C6c failures are closed; every other item retains its prior PASS or documented
not-applicable disposition. Deterministic completeness verdict: **COMPLETE**.

The test-budget assessment remains `5 behavior groups × 2 = 10` bodies versus
eight mapped bodies. No new body or PBT suite was added.

## Iteration-2 verification

| Check | Result |
|---|---|
| Writable-Lima exact pending selector | PASS — nextest run `712722cb-02f3-4175-8afd-6b4d78818b35`; 1 passed, 101 skipped, 0 failed. |
| Writable-Lima malformed-EXEC production-parser selector | PASS — nextest run `26771c3c-9871-4487-804b-3ba342725baf`; 1 passed, 528 skipped, 0 failed. |
| Ephemeral production-parser frame matrix | PASS — empty/one/unterminated/one-plus-unterminated accepted; invalid UTF-8/interleaved/`Shutdown`/second-EXEC rejected. |
| `cargo fmt --all -- --check` | PASS. |
| `git diff --check 484a7fdb..e74cf14c` | PASS. |
| `git diff --check 7a507865..e74cf14c` | PASS. |
| `jq -e . docs/feature/netns-density-295/deliver/roadmap.json` | PASS. |
| Remediation changed-file scope | PASS — exactly the acceptance body and two DISTILL documents; no production or other artifact. |
| Remediation commit attribution | PASS — Marcus remains author; exactly one Codex co-author trailer; no Claude, Anthropic, or generated-by attribution. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

## Iteration-2 final disposition

DR-03-02-01 is closed with the smallest authorized change. The final oracle is
exact, fail-closed, uses the production parser, preserves the real owner path,
and does not weaken or waive any remaining step-`03-02` schedule.

# APPROVED
