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

# Iteration 3 re-review

- **Iteration:** 3
- **Review date:** 2026-09-22
- **Remediation commit:** `2179f03cbc7eaa15e82cf8543a69cccb4eeb6a79`
- **Remediation parent:** `a028115a13d4b218f8b70ba4bb2b426786cc48f7`
- **Trigger:** DELIVER review `review-03-02.md` findings D1 and D2
- **Reviewer:** same isolated `nw-acceptance-designer-reviewer`
- **Iteration-3 verdict:** **CHANGES_REQUIRED**

This section preserves iterations 1 and 2 verbatim. It reviews the expanded,
user-authorized S-ND295-28 DISTILL/roadmap remediation, the complete mapped
eight-body evidence set, and the two DELIVER findings. The roadmap's
`validation.status = pending` is the correct pre-review state and is not a
finding.

## Iteration-3 reviewed scope and authority

The remediation commit changes exactly:

- `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs`;
- `docs/feature/netns-density-295/deliver/roadmap.json`;
- `docs/feature/netns-density-295/distill/red-classification.md`; and
- `docs/feature/netns-density-295/distill/test-scenarios.md`.

The accepted authority remains the EXEC-close linearization,
`D-295-DELIVER-03-01`, S-ND295-27 as the opaque gate evidence boundary, and
S-ND295-28 as the production `VmDriver`/writer/action-owner evidence boundary.
No production Rust or API changed in this remediation.

## DELIVER finding dispositions

| Finding | Iteration-3 disposition | Evidence |
|---|---|---|
| D1 — worker-scoped zero-test model selector and ambiguous real-`VmDriver` wording | **CLOSED** | Roadmap description, criterion 1, scenario name, verification, and notes now name the one authoritative overdrive-core model as a prerequisite and the deterministic worker schedules as the real `VmDriver`/`BeaconWriter` evidence. |
| D2 — action-shim body used the historical host-netns composition and never reached release ownership | **CLOSED** | The body now uses the pre-existing post-#295 test composition with `SimSharedGuestNetworkOwner`; the exact selector reaches the held release, remains pending there, and proves cancellation ownership. |

No production failure is asserted by either disposition. Both were evidence and
traceability defects.

## D1 re-review — core-model and real-owner split

The corrected roadmap is exact at the D1 boundary:

- `roadmap.json:390` describes the overdrive-core model prerequisite, the three
  primary real-`VmDriver` schedules, and the existing complements separately.
- Criterion 1 at `:397` assigns `PROPTEST_CASES=1024` to the authoritative core
  property and explicitly forbids a worker duplicate, seeded writer, or
  injected supervisor consequence.
- The exact core selector at `:415` selects the real
  `acceptance::netns_density_exec_gate::generated_operation_sequences_match_the_gate_model`
  body. The stale worker zero-test selector is absent from the roadmap and
  current DISTILL SSOT.
- The three primary schedules remain selected by their module selector at
  `:414`; the writer/stop/cancellation and action-owner complements have their
  own exact selectors at `:416-417`.
- `test-scenarios.md:53` and `red-classification.md:41-76` carry the same split
  and list all eight bodies without presenting the core property as a worker
  test.

The independent core run selected one test and executed 1,024 proptest cases.
No worker PBT, seeded writer, test seam, or production change was added.

## D2 re-review — production action owner and cancellation ownership

### Existing sanctioned composition

`dispatch_with_guest_network_provisioner_for_test` is not surface invented by
commit `2179f03c`. `git log -S` and blame trace its declaration to commit
`dd1a18fe8ed3ce4275fb52e31cd8a86b8aceb08c` on 2026-09-20, before this
remediation. It is doc-hidden and integration-feature/test gated.

The helper preserves the production workflow-intent preflight and calls the
same private `dispatch_with_network_provisioner_and_guest` owner used by
production. Production passes its boot-composed `SharedGuestNetworkOwner` as
`Some(shared_guest_network.as_ref())`; the helper passes the injected
`GuestNetworkProvisioner` at that same parameter. In either case,
`provision_and_inject_netns` selects the `Some(guest_provisioner)` branch,
awaits `GuestNetworkProvisioner::provision`, and returns before the historical
`HostNetworkProvisioner`/slot/netns/veth branch. The nominal host provisioner
argument in the helper is therefore not executed for this allocation path.

`SimSharedGuestNetworkOwner` implements both the exact
`GuestNetworkProvisioner` supertrait and `SharedGuestNetworkOwner`; it is the
existing socket-free adapter for the accepted production port, not an alternate
domain mechanism. The test also starts and later shuts down the real
`MtlsInterceptWorker` shared listener owner, so `start_alloc` follows its
accepted healthy-owner path.

### Reachable owner sequence

The corrected body constructs the existing `AppState`, installs the existing
`HoldingReleaseDriver` and healthy `MtlsInterceptWorker`, and invokes the
pre-existing post-#295 helper with `SimSharedGuestNetworkOwner`. The real action
owner then executes:

1. workflow-intent preflight;
2. `GuestNetworkProvisioner::provision` through the accepted port;
3. driver start and Running observation;
4. real `MtlsInterceptLifecycle::start_alloc` success; and
5. the production action-shim call to
   `driver.release_for_exit_emission(handle).await` before
   `driver.on_alloc_running(&spec)`.

The `HoldingReleaseDriver` is an external Driver-port test double used only to
make that await/cancellation boundary observable. The independent selector
proved `release_entered`, proved the dispatch task stayed unfinished inside the
held release, and observed both `release_completed` and
`on_alloc_running_called` false. Aborting and joining that same dispatch task
dropped the same held release future, set `release_cancelled`, and left both
later facts false. Because the production action owner directly awaits the
hook, there is no detached release future for the fixture to miss.

This closes criterion 5 without fixture theater and without claiming a
production defect. The real production reachability is the ordinary
`dispatch_with_workflow_intent -> dispatch_with_network_owner ->
dispatch_with_network_provisioner_and_guest -> dispatch_single ->
release_for_exit_emission` path over the boot-composed owner. The test replaces
only driven adapters at already-sanctioned ports.

## Complete S-ND295-28 selector matrix

All authoritative test selectors ran independently in writable Lima target
space and selected a nonzero intended count:

| Evidence group | Exact intended bodies | Independent result |
|---|---:|---|
| Authoritative core model prerequisite with `PROPTEST_CASES=1024` | 1 | PASS — run `3fd7a21b-473c-4747-b90b-0d7dca101c23`; 1 passed, 528 skipped. |
| `netns_density_exec_release` real-`VmDriver` schedules | 3 | PASS — run `f5f2eb99-ecd5-40b6-ad66-b71414f5915e`; 3 passed, 99 skipped. |
| `vm_driver_stop_totality` writer lifetime, stop deadline, and cancellation schedules | 3 | PASS — run `13a0c7ff-2d06-49f0-8504-509ea45ee0cd`; 3 passed, 99 skipped. |
| Post-#295 action-owner await/cancellation schedule | 1 | PASS — run `ad45eec2-60ef-420d-b109-efcc69cf3acb`; 1 passed, 208 skipped. |

The complete mapped total is eight distinct bodies. No selector is empty and no
body is counted through a different crate or surrogate process.

## Identity, Contract Shape, universes, and non-expansion

All eight scenario identities are unchanged. The authoritative core property,
three primary worker schedules, two focused backpressure/cancellation schedules,
and action-owner schedule retain explicit
`/// CONTRACT_SHAPE: bounded-change.` and Outcome anchors. The existing
writer-decision-table schedule retains the accepted named `S-VLL-08` outcome
and its exact bounded-change declaration.

The three primary worker bodies are now active with no ignore marker. The
approved fail-closed typed-frame oracle is byte-for-byte unchanged from its
approved form; the only post-approval diff in that file is removal of the three
step-owned ignore attributes. It still requires valid UTF-8 and the production
`BeaconMessage` parser for every newline-complete frame, accepts only an empty
slice or one `Exec`, ignores only a final unterminated suffix, joins the
cancelled release task, and reads through EOF.

The complete observable set remains layered rather than duplicated:

- the core PBT owns public gate returns, transitions, wait/wake/refusal, and
  recovery projections;
- the primary real-`VmDriver` bodies own recovery-before-release,
  claim-before-detection writer transfer/cancellation, FailStop refusal, EOF,
  and typed frame cardinality;
- the three `vm_driver_stop_totality` bodies own writer consumption, the
  existing stop deadline, fail-closed VMM termination, exit-event ordering,
  supervision state, and socket closure; and
- the action-owner body owns the action shim's direct await, same-future
  cancellation, and the false completion/`on_alloc_running` complement.

No private `active_claims` storage, new parser, API, hook, writer seam,
production owner, persistence mechanism, recovery mechanism, or later-step
`03-03` behavior was added. The four changed files are the minimum SSOT/test
scope needed to correct D1 and D2.

## New blocking finding

### I3-D3 — Roadmap criterion 4 describes a state the named body never enters

- **Severity:** Blocker
- **Dimension:** acceptance traceability / observable-boundary honesty
- **Location:** `docs/feature/netns-density-295/deliver/roadmap.json:400`

Criterion 4 says
`cancelling_backpressured_release_cannot_leave_an_exec_sender_running` proves
cancellation while awaiting `claim_release` in Recovering/BootClosed and proves
no leaked Notify registration. The named body does neither.

The independently executed real path is concrete:

1. `build_driver` performs `open_after_boot` before constructing the driver
   (`vm_driver_stop_totality.rs:167-178`).
2. The body starts the VM, spawns
   `VmDriver::release_for_exit_emission`, and forces the 16 MiB EXEC write
   against a 4 KiB receive buffer (`:1328-1373`).
3. In production, `release_for_exit_emission` therefore obtains its Open claim,
   takes the pending EXEC, and transfers it to `BeaconWriter::release_exec`.
4. The body aborts the release only after the writer is genuinely
   backpressured (`:1373-1375`), then observes fail-closed VMM termination,
   exit-gate ordering, EOF, and no complete parsed EXEC (`:1377-1432`).

The body has no supervisor capability and never calls `begin_recovery`; it
cannot enter Recovering or BootClosed. Because the Open claim returns
immediately, this schedule also creates no parked `Notify` waiter whose
registration could be observed. The other seven mapped bodies do not turn this
named test into the criterion claimed: the recovery waiter is completed rather
than cancelled, and the core PBT aborts residual waiters without asserting a
Notify-registration complement. Wait registration is private synchronization
under the accepted Contract Shape in any case.

This is not a production defect. The selector passed and proves the accepted
post-claim/writer-transfer cancellation behavior. It is a remaining roadmap
traceability defect and repeats the class D1 was meant to eliminate.

**Required bounded correction:** replace criterion 4 with wording that matches
the accepted and executable schedule, for example:

> `cancelling_backpressured_release_cannot_leave_an_exec_sender_running`
> proves that cancelling the structured release future after an Open claim has
> transferred the command to the production beacon writer synchronously signals
> writer cancellation, closes the beacon through EOF, completes fail-closed VMM
> termination before releasing the exit-event gate, and leaves no detached EXEC
> sender.

Do not add a new test, expose Notify state, inspect private gate storage, or
change production. `test-scenarios.md` and `red-classification.md` already state
the accepted cancellation-after-writer-transfer boundary and need no change for
this finding. Keep roadmap validation pending until this sentence is corrected
and independently re-reviewed.

## Iteration-3 acceptance dimensions

| Dimension | Score | Re-check |
|---|---:|---|
| Happy-path bias | 10/10 | Recovery, fail-stop, backpressure, cancellation, writer error/EOF/absence, and action-owner cancellation remain the majority. |
| Given-When-Then form | 10/10 | S-ND295-28 retains one Given, one When, and observable outcome/complement clauses. |
| Business/domain language | 8/10 | The title and outcome remain stable; EXEC/framing terms are the accepted low-level Published Language. |
| Coverage completeness | 10/10 | All eight mapped bodies execute and the typed frame classes remain closed. |
| Walking-skeleton user-centricity | 10/10 | Not applicable to this focused scenario. |
| Priority validation | 10/10 | The remediation addresses only the reproduced D1/D2 evidence failures. |
| Observable behavior assertions | 10/10 | Public returns, task completion, socket EOF/frames, VMM/exit ordering, supervision, and action-owner flags are asserted at their accepted layers. |
| Traceability coverage | 6/10 | D1 and D2 are closed, but criterion 4 still assigns Recovering/BootClosed/Notify evidence to an Open/post-transfer cancellation body. |
| Walking-skeleton boundary proof | 10/10 | Not applicable; production composition is assessed directly. |

The reviewer score gate fails because traceability remains below 7 and I3-D3
is a blocker.

## Iteration-3 mandate re-check

| Gate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal/port boundary | PASS | Core capabilities, `Driver`, `GuestNetworkProvisioner`, production beacon, VMM, and action-owner paths are entered only through their accepted ports. |
| CM-B — domain-language abstraction | PASS | Scenario vocabulary remains recovery, command release, fail-stop, cancellation, and ownership. |
| CM-C — complete journey | PASS | The mapped set closes gate, writer, stop, and action-owner journeys at distinct observable boundaries. |
| CM-D — pure logic/fixture isolation | PASS | No new business logic or fixture-defined outcome; pre-existing sim adapters implement the production ports. |
| CM-E — bounded-change universe | PASS | Each schedule declares its allowed delta and complement; the typed-frame universe remains fail-closed. |
| CM-F — layer-dependent PBT mode | PASS | The sole PBT stays at the core model; real-I/O/integration schedules remain finite examples. |
| CM-G — two-tier rich journey | PASS / N/A | The core model and production-owner examples remain the accepted evidence split without a parallel suite. |
| CM-H — real-I/O sad paths | PASS | Recovery, cancellation, backpressure, stop, and FailStop remain explicitly named examples. |

All three pillars pass for the executable scenarios. The blocker is roadmap
traceability, not a mandate or production-composition failure.

## Iteration-3 completeness audit

| Category | Result | Complete S-ND295-28 evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | Empty/one/second complete EXEC and unterminated frame classes remain distinct. |
| C2 — state and transition | PASS | The core model exercises legal and illegal events across BootClosed/Open/Recovering/FailStop. |
| C3 — count cardinality | PASS | Zero/one/many claims and zero/one/two complete commands are represented. |
| C4 — lifecycle and idempotency | PASS | Repeated/late gate operations, claim Drop, release cancellation, writer consumption, stop, and teardown are covered. |
| C5 — mode/decision table | PASS / N/A | No new mode flag; existing component/cause and writer-outcome tables remain closed. |
| C6 — negative and robustness | PASS | Invalid UTF-8, malformed/interleaved frames, unexpected kinds, fail-stop causes, writer errors, and cancellation fail closed. |
| C7 — environment/interruption/concurrency | PASS | Forced socket backpressure, mid-write cancellation, stop races, gate waiters, writer task, and action dispatch are exercised. |

### Iteration-3 mechanical 15-item checklist

| Item | Result | Evidence |
|---|---|---|
| C1a | PASS | Zero complete command and minimum operation-sequence cases are accepted. |
| C1b | PASS | Zero/one pass, second complete EXEC fails; partial final frame is separate. |
| C2a | PASS | `ModelState` documents all four gate states. |
| C2b | PASS | Illegal-event coverage exists for every gate state. |
| C3 | PASS | PBT and deterministic schedules cover zero/one/many claims and command cardinality. |
| C4a | PASS | Repeat/late transitions and writer lifecycle outcomes are exercised. |
| C4b | PASS / N/A | Opaque RAII claim Drop without acquisition is unrepresentable. |
| C5a | PASS / N/A | No mode flag is introduced. |
| C5b | PASS / N/A | No flag-orthogonality contract exists. |
| C6a | PASS | Malformed/invalid/interleaved complete frames fail closed. |
| C6b | PASS | Every declared recovery/fail-stop/writer/cancellation failure class has a named schedule. |
| C6c | PASS | Parser/kind/cardinality results are closed; no other complete frame escapes. |
| C7a | PASS | The 4 KiB receive buffer against 16 MiB EXEC forces degraded backpressure. |
| C7b | PASS | Stop and cancellation interrupt an in-flight backpressured write. |
| C7c | PASS | Gate, release, writer, stop, and dispatch actors execute concurrently. |

**Mechanical result: 15/15 — COMPLETE.** Completeness does not override the
roadmap traceability blocker.

The test budget remains five observable behavior groups, permitting ten bodies.
Eight bodies are mapped and executed; no extra body or duplicate PBT was added.

## Iteration-3 verification

| Check | Result |
|---|---|
| All four corrected test selectors | PASS with intended nonzero counts — 1 + 3 + 3 + 1 = 8 bodies. |
| Workspace `cargo check --all-targets --features integration-tests` in writable Lima target | PASS. |
| Workspace clippy roadmap command | NON-BLOCKING BASELINE FAILURE — unchanged `overdrive-netlink/src/nft.rs:5121` `clippy::print_stderr`; no remediation file is implicated. |
| Focused control-plane integration clippy retry | ENVIRONMENT-LIMITED — the Lima `/tmp` target exhausted space after the full workspace run; executable selector and workspace check had already passed. |
| `cargo fmt --all -- --check` | PASS. |
| `git diff --check a028115a..2179f03c` | PASS. |
| Roadmap JSON parse | PASS. |
| `des-verify-integrity --roadmap-only` | PASS — format OK; pending validation is intentionally preserved. |
| Remediation commit scope | PASS — one existing acceptance body plus roadmap and two DISTILL SSOT documents; no production file. |
| Remediation commit attribution | PASS — Marcus remains author; exactly one Codex co-author trailer and no prohibited attribution. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

The current Lima guest's default cargo target was read-only, so authoritative
test and check executions used the isolated writable target
`/tmp/codex-netns-density-target`. This changes only build-cache location, not
the selected binaries or test filters.

## Roadmap validation disposition

No approval-field update is recommended in iteration 3. Leave:

- `validation.status = pending`;
- the reviewer field as a pending-review marker; and
- `approved_at = null`.

After the one criterion-4 sentence is aligned and independently approved, the
original acceptance designer may apply the final reviewer text and timestamp
mechanically. No production, test, DISTILL, feature-delta, design, or execution
log change is required for I3-D3.

## Iteration-3 remediation dispositions

| Item | Disposition |
|---|---|
| D1 | **CLOSED.** Correct core selector, wording, and evidence split; stale worker zero-test command removed. |
| D2 | **CLOSED.** Existing sanctioned post-#295 composition reaches and owns release cancellation. |
| Approved typed-frame oracle | **PRESERVED.** No assertion or parser change after iteration 2. |
| I3-D3 | **OPEN blocker.** Roadmap criterion 4 must describe the actual Open-claim/post-transfer cancellation evidence. |
| API/architecture/production | **PASS.** No new surface or behavior. |
| Mutation testing | **NOT RUN.** Deferred to the final DELIVER gate. |

# CHANGES_REQUIRED

# Iteration 4 re-review

- **Iteration:** 4
- **Review date:** 2026-09-22
- **Remediation commit:** `8b407cb21ec499720f53e7ff75eec0805ae3d0b3`
- **Remediation parent:** `cd3217701df84509ea0d16715a53479d955e25a8`
- **Trigger:** iteration-3 finding I3-D3
- **Reviewer:** same isolated `nw-acceptance-designer-reviewer`
- **Iteration-4 verdict:** **APPROVED**

This section is additive and preserves iterations 1-3 as the complete review
history.

## Iteration-4 scope

The remediation commit changes one line in one file:

- `docs/feature/netns-density-295/deliver/roadmap.json` — criterion 4 of step
  `03-02`.

`git diff --numstat` reports `1` insertion and `1` deletion. No other
criterion, selector, description, scenario identity, implementation note,
estimate, validation field, test, DISTILL artifact, design artifact,
`feature-delta.md`, production/API file, or `execution-log.json` changed.

The roadmap remains intentionally pending until this verdict is mechanically
applied.

## I3-D3 disposition

**I3-D3 is CLOSED.** Criterion 4 now states exactly the behavior independently
executed in iteration 3:

> `cancelling_backpressured_release_cannot_leave_an_exec_sender_running`
> proves that cancelling the structured release future after an Open claim has
> transferred the command to the production beacon writer synchronously signals
> writer cancellation, closes the beacon through EOF, completes fail-closed VMM
> termination before releasing the exit-event gate, and leaves no detached EXEC
> sender.

This matches the body and production sequence:

1. `build_driver` opens the EXEC gate;
2. `release_for_exit_emission` acquires the Open claim and transfers the pending
   command to the production `BeaconWriter`;
3. the 16 MiB write is observably backpressured;
4. aborting the structured release synchronously signals the retained writer;
5. fail-closed VMM termination completes before the exit event becomes
   observable; and
6. the guest reads EOF, proving no write half or detached EXEC sender survives.

The former false claims about cancellation while awaiting `claim_release` in
Recovering/BootClosed and about a leaked `Notify` registration are absent from
the current roadmap. A repository cross-reference scan finds no such claim in
the active step-`03-02` contract.

The correction is documentation-only. It does not add a test, expose gate
storage, widen API, change production, or require a new schedule.

## D1, D2, and complete evidence preservation

D1 remains **CLOSED**: the roadmap still names the one authoritative
overdrive-core PBT prerequisite and separately names deterministic real-
`VmDriver`/`BeaconWriter` schedules. The corrected core selector, all worker
selectors, and the prohibition on duplicate worker PBT/seeded writer remain
unchanged.

D2 remains **CLOSED**: criterion 5, the action-owner selector, and the
pre-existing post-#295 `dispatch_with_guest_network_provisioner_for_test`
composition remain unchanged. No historical `HostNetworkProvisioner` evidence
claim was reintroduced.

The four test-file blobs are identical before and after commit `8b407cb2`:

- core model: `a017ac897bcd97ae8e5d3d464ff2f119604816d5`;
- primary real-`VmDriver` schedules:
  `2648fd680b1b5e63a810739bc2037eb568b9a3f8`;
- writer/stop/cancellation schedules:
  `d0b716e9e1b943ba2859d559ea58be98a401db1d`; and
- action-owner schedule: `688e9bb819f9cb6e04f35ee7c565fe2e8c7605ca`.

Therefore iteration 3's independently executed nonzero selector matrix remains
applicable without rerun: core `1`, primary worker `3`, writer/stop/cancellation
`3`, action owner `1`; all eight passed.

The approved fail-closed typed-frame oracle is also unchanged. It continues to
parse every newline-complete frame through UTF-8 and the production
`BeaconMessage` parser, accepts exactly zero or one `Exec`, rejects malformed,
interleaved, unexpected, or duplicate complete frames, ignores only the final
unterminated suffix, joins cancellation, and reads through EOF.

The DISTILL scenario and RED-classification blobs, `feature-delta.md`, and the
DES execution log are likewise byte-identical across this remediation. The
five-group/eight-body test budget and the iteration-3 15/15 completeness result
remain unchanged.

## Iteration-4 verification

| Check | Result |
|---|---|
| Remediation changed-file scope | PASS — only `roadmap.json`. |
| Remediation diff size | PASS — one criterion line replaced (`1/1`). |
| Criterion 4 exact body-to-roadmap mapping | PASS. |
| Recovering/BootClosed waiter claim absent | PASS. |
| Notify-registration claim absent | PASS. |
| Other five criteria unchanged | PASS. |
| Description, selectors, scenario identity, estimate, and notes unchanged | PASS. |
| Test and SSOT blob identity | PASS — all listed test/DISTILL/design/log blobs unchanged. |
| `jq -e . roadmap.json` | PASS. |
| `git diff --check 8b407cb2^..8b407cb2` | PASS. |
| `des-verify-integrity --roadmap-only` | PASS — roadmap format OK. |
| Remediation commit attribution | PASS — Marcus remains author; exactly one Codex co-author trailer and no prohibited attribution. |
| Test rerun | NOT REQUIRED — the remediation is roadmap-only and every executable blob is identical to the independently executed iteration-3 evidence. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |

No blocker, high, medium, or low finding remains.

## Final roadmap validation metadata recommendation

The original acceptance designer may now apply this exact metadata
mechanically:

```json
{
  "status": "approved",
  "reviewer": "S-ND295-28 step 03-02 DISTILL/roadmap remediation approved by independent acceptance-design review iteration 4; D1, D2, and I3-D3 closed, eight-body selector matrix and fail-closed typed-frame oracle verified.",
  "approved_at": "2026-09-22T19:07:01Z"
}
```

This metadata update is the only remaining authorized write. It does not alter
the roadmap's substantive contract and requires no further production, test,
DISTILL, feature-delta, design, or execution-log change.

## Iteration-4 final dispositions

| Item | Disposition |
|---|---|
| I3-D3 | **CLOSED.** Criterion 4 now matches the Open-claim/post-transfer cancellation body exactly. |
| D1 | **REMAINS CLOSED.** Core PBT and worker evidence split unchanged. |
| D2 | **REMAINS CLOSED.** Sanctioned post-#295 action-owner composition unchanged. |
| Full eight-body evidence | **PRESERVED and GREEN.** Iteration-3 selector evidence remains applicable by blob identity. |
| Fail-closed typed-frame oracle | **PRESERVED and APPROVED.** |
| API/production/architecture | **UNCHANGED.** |
| Mutation testing | **NOT RUN.** |

# APPROVED
