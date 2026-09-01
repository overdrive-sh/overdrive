# DISTILL Q7/Q9 acceptance review — guest-stack-transparent-mtls-intercept

**Review ID:** `accept_rev_2026-08-29_q7_7255c68e`  
**Reviewer:** `nw-acceptance-designer-reviewer` (fresh isolated review)  
**Reviewed commit:** `7255c68e64b7f0f15da5a2ed8a033806a2939e6a` against its first parent  
**Review iteration:** 1  
**Verdict:** **REJECTED_PENDING_REVISIONS**

## Scope and evidence

This review covers the three documentation files changed by the reviewed commit:

- `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md`
- `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- `docs/feature/guest-stack-transparent-mtls-intercept/distill/red-classification.md`

The approved DESIGN Q7/Q9 amendment, SPIKE evidence, product journeys, project ATDD
policy, complete DISTILL package, and the relevant Rust test locations were read as
context. The working tree contains later, uncommitted source changes; they were not
reviewed for correctness and were not modified. Where code placement matters, this
review uses the immutable commit-parent snapshot
`7255c68e64b7f0f15da5a2ed8a033806a2939e6a^`.

No test suite was run: the reviewed change is documentation-only, the Tier-3 tests
require the metal lane, and the working source is intentionally partial. Static
checks covered scenario/tag counts, GWT shape, changed-file scope, contract-shape
declarations, the declared RED reasons, and commit-parent test placement.

## Strengths

- The high-level Q7 lifecycle model correctly removes the former setup-`EXIT` arm:
  all guest network initialization precedes READY, failure powers off before READY,
  and only the post-READY operator command may emit `EXIT`
  (`test-scenarios.md:59`, `test-scenarios.md:63-103`).
- The Q9 packet-observation contract is materially stronger than the parent text:
  it names both interfaces, the full allocation identity tuple, the all-EtherType
  universe, conservative evidence failures, continued capture across EXEC, the
  first five-tuple, rule evidence, leg-F, TLS/kTLS, and the cleartext complement
  (`test-scenarios.md:181-194`).
- The pure-property split is appropriate. Exact exit-code classification ranges
  over every `Option<i32>` plus arbitrary signal, while suppression/admission ranges
  over NIC and read-back states. The package explicitly requires the exact Rust
  declaration `/// CONTRACT_SHAPE: pure-function.` on every live property
  (`test-scenarios.md:282-291`).
- Console selection has a useful bounded matrix: final 8 KiB, five fragments,
  unterminated final fragment, lossy UTF-8, console precedence, stderr fallback,
  and a stable neither-source fallback (`test-scenarios.md:292-296`).
- All twelve scenarios carry a contract-shape tag, the three derivation properties
  are kept at layer 1, and real-guest cases remain example-based on the metal lane.

## Scorecard

| Dimension | Score / 10 | Result | Rationale |
|---|---:|---|---|
| Happy-path bias | 8 | Good | Five scenarios exercise a failure, invariant, interruption, or teardown concern; the package is not success-only. |
| GWT format | 4 | Below standard | S-GTI-01/-02 put post-action state in Given; S-GTI-06 contains mutually exclusive success and failure runs under one When. |
| Business language | 4 | Below standard | Several Then clauses assert private types, internal action names, source-line gates, and hypothetical mutations rather than domain outcomes. |
| Coverage completeness | 3 | Reject | The claimed 15/15 audit does not satisfy the mechanical C1/C2/C4/C6/C7 checks. |
| Walking-skeleton user-centricity | 6 | Acceptable | S-GTI-01 expresses user value, but S-GTI-02 is a second, technically framed skeleton and its setup is not driveable as written. |
| Priority validation | 9 | Excellent | Q7 boot honesty and Q9 born-captured proof are the correct load-bearing risks. |
| Observable behavior | 2 | Reject | S-GTI-08 requires private view/action inspection; S-GTI-02/-06/-12 also name internal calls and code sites. |
| Traceability coverage | 7 | Good | Missing DISCUSS/DEVOPS inputs are explicitly recorded; the approved DESIGN and product journeys are mapped. |
| Walking-skeleton boundary proof | 3 | Reject | The C3/capture-ready Givens require the deploy result before the deploy action, and the S-GTI-08 malformed token is not operator-supplied. |

## Mandate verdicts

| Mandate | Verdict | Evidence |
|---|---|---|
| CM-A — hexagonal boundary | **FAIL** | S-GTI-08 asserts private `WorkloadLifecycleView` and exact reconciler actions; S-GTI-02 asserts `start_alloc` success. |
| CM-B — business language | **FAIL** | Gherkin exposes `:1880`, `:1269`, `:2038`, `DriverType::Exec`, private types, and code-mutation expectations. |
| CM-C — complete user journey | **FAIL** | S-GTI-02 cannot establish its Given through the stated deploy journey; S-GTI-06 combines incompatible outcomes in one run. |
| CM-D — pure extraction | PASS | Classifier and suppression logic are assigned to source-local pure properties. |
| CM-F — PBT layer | PASS | Generative tests are confined to source-local/layer-1 properties; Tier-3 metal tests are examples. |
| CM-H — named layer-3 sad paths | **FAIL** | S-GTI-06 hides reinstall failure inside the happy restart scenario; the setup table collapses distinct token/apply failures. |

Any CM-A/CM-B/CM-C failure is a blocking review result.

## Findings

### AD-Q7-01 — post-deploy C3 and capture state are fixture theater in Given

**Severity:** BLOCKER  
**Dimensions:** GWT format, walking-skeleton boundary, CM-A, CM-C  
**Status:** Open; remediation required

**Evidence**

- `test-scenarios.md:172` says the decorator is already capture-ready on “the
  exact allocation tap and host-veth” before the operator deploys at line 173.
- `test-scenarios.md:183-185` says C3 has already provisioned the allocation and
  the witness already knows its complete identity before the deploy When at line
  186.
- The package's own state machine places “C3 provision complete” before the
  decorator releases real VMM creation (`test-scenarios.md:63-72`), which makes
  those facts outcomes inside deployment, not external preconditions.

The exact allocation, tap, host-veth, and C3 completion do not exist before the
operator submits the deployment. A test can install an observation-only decorator
before the action, but it cannot truthfully put allocation-specific readiness and
correlation in Given without pre-provisioning the end state or adding functional
test wiring.

**Required remediation**

State only that the observation-only decorator is installed and ready to arm in
Given. After the operator deploys, require the witness to prove that it armed the
new allocation's exact tap and host-veth after C3 and before delegating real VMM
creation. Keep the full identity, zero-frame interval, and conservative failures
as witnessed outcomes. Apply the same correction to S-GTI-01.

### AD-Q7-02 — S-GTI-08's malformed-token metal fixture is not reachable through the driving port

**Severity:** BLOCKER  
**Dimensions:** priority, walking-skeleton boundary, CM-A  
**Status:** Open; remediation required

**Evidence**

- Q3 explicitly defines the network token as platform-owned and not an operator
  input (`test-scenarios.md:55`).
- S-GTI-08 nevertheless requires a real guest to receive a malformed platform
  token “through the production deploy path” (`test-scenarios.md:239-241`).
- In the commit-parent production placement, `compose_vm_network` formats the
  token from typed platform fields and validates the prefix before appending it
  (`7255c68e^:crates/overdrive-worker/src/vm_driver.rs:112-154`). There is no
  deploy argument for an operator to provide malformed token bytes.
- The later, read-only partial test placement independently demonstrates the
  production-reachable seam: it corrupts the guest resolver destination and then
  drives normal `deploy` (`guest_stack_mtls_egress.rs:1965-1986`), rather than
  mutating the platform token.

Producing the proposed malformed token requires an internal config/VMM mutator.
That would make the supposedly production-path metal AT pass through test-only
functional wiring and violate its own driving-port contract.

**Required remediation**

Use a production-reachable pre-READY failure for the metal example, such as a
legitimate custom rootfs whose resolver write fails. Keep missing/malformed token
coverage as source-local parser examples. Preserve the same lifecycle, diagnostic,
console, terminal, no-EXEC, no-frame, and restart-accounting assertions across the
metal and focused layers.

### AD-Q7-03 — S-GTI-08 mixes operator outcomes with private reconciler state

**Severity:** BLOCKER  
**Dimensions:** observable behavior, CM-A, CM-B  
**Status:** Open; remediation required

**Evidence**

- The scenario requires `FinalizeFailed` to be the only lifecycle action and
  forbids `RestartAllocation` by internal variant name (`test-scenarios.md:250`).
- It then asserts that the returned **private** `WorkloadLifecycleView` equals its
  input (`test-scenarios.md:251`).
- The executable map assigns the metal scenario to the CLI integration file while
  separately assigning classifier/reconciler support to source-local tests
  (`test-scenarios.md:395-398`).

Neither the private view nor the exact internal action vector is returned by
`overdrive deploy` or `workload describe`. They cannot be part of the metal
driving-port universe. Declaring them in the metal Gherkin forces an internal seam
or a log/call-path oracle.

**Required remediation**

Split the contract by layer. The metal scenario should assert only port-visible
facts: typed terminal reason/detail and exact exit information where exposed,
unchanged durable `restart_count`/budget, no second allocation or restart
transition, no Running/EXEC/operator marker, and no frame. A focused in-process
reconciler example may separately assert `FinalizeFailed`, absence of
`RestartAllocation`, and unchanged `WorkloadLifecycleView`; map that supporting
test explicitly and do not present it as a CLI observable.

### AD-Q7-04 — the RED snapshot is factually stale and classifies GREEN tests as RED

**Severity:** BLOCKER  
**Dimensions:** RED-for-right-reason gate, coverage evidence  
**Status:** Open; remediation required

**Evidence**

- `red-classification.md:45-47` says `derive_vm_tap_plan` is a `todo!()` and marks
  S-GTI-09/-10/-11 `MISSING_FUNCTIONALITY`.
- `red-classification.md:49-53` then claims all twelve ATs are RED and none is a
  wrong assertion.
- In the immutable commit parent, `derive_vm_tap_plan` is implemented at
  `7255c68e^:crates/overdrive-control-plane/src/veth_provisioner.rs:815-830`.
- The commit-parent S-GTI-09/-10/-11 tests are live `proptest!` properties with
  the exact contract-shape declaration and no `#[should_panic]`
  (`.../veth_provisioner.rs:5075-5235`).
- Commit-parent S-GTI-01/-02/-03/-04/-07 are also live Tier-3 test bodies, while
  only S-GTI-05/-06/-08/-12 remain explicit RED panics
  (`7255c68e^:crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1076-1478`).
- The remediation is documentation-only and explicitly supplies no fresh compile
  or execution evidence (`red-classification.md:26-30`).

The document may preserve old compile evidence as history, but it cannot call the
current population “12/12 RED.” A wrong snapshot invalidates the DELIVER entry
gate, and the malformed-token/private-view designs above additionally fit
`OBSERVABLE_NOT_AT_PORT` / `WRONG_ASSERTION`, not `MISSING_FUNCTIONALITY`.

**Required remediation**

Regenerate the snapshot against the exact reviewed tree. Mark inherited GREEN
properties/tests as such, identify which amended obligations are newly RED, and
record an actual compile/run result or an explicit not-executed classification per
test. Reclassify impossible/internal-port assertions as wrong-shape until fixed.
Do not use historical evidence to claim a current 12/12 RED result.

### AD-Q7-05 — S-GTI-06 combines mutually exclusive reinstall outcomes

**Severity:** HIGH  
**Dimensions:** GWT format, CM-C, CM-H  
**Status:** Open; remediation required

**Evidence**

- One restart When appears at `test-scenarios.md:223`.
- The Then branch requires successful reinstall and an intercepted first flow at
  lines 224-225, then introduces a second hidden condition — “when the re-install
  fails” — at lines 226-227.

One execution cannot both reinstall successfully and fail reinstall. The hidden
second When makes the scenario non-deterministic and prevents one concrete
Given/When/Then example from satisfying it.

**Required remediation**

Split into named successful-restart and failed-reinstall scenarios (for example,
S-GTI-06a and S-GTI-06b), each with its own fixture, one When, and one observable
outcome. One Rust test function may host two explicitly named subcases only if
failure reporting preserves those identities, but the specification must still
describe two examples.

### AD-Q7-06 — several Then clauses assert implementation paths or hypothetical mutations

**Severity:** HIGH  
**Dimensions:** business language, observable behavior, CM-B  
**Status:** Open; remediation required

**Evidence**

- S-GTI-02 defines intercept-live partly by the internal call result
  “`start_alloc` success” (`test-scenarios.md:189`).
- S-GTI-06 says the `:1880` gate fired (`test-scenarios.md:224`).
- S-GTI-12 names source locations `:2038`, `:1269`, the private discriminator
  `DriverType::Exec`, and asserts that a hypothetical code edit would red the AT
  (`test-scenarios.md:258-260`).

Exact rule presence/removal, packet evidence, terminal state, allocation identity,
and unchanged peer rules are observable. A private call's return, a source-line
gate, and “this mutation would fail” are not driving-port outcomes.

**Required remediation**

Keep implementation coordinates in a rationale/carry-forward table, not Gherkin.
Express intercept-live through the exact alloc-specific rule plus the blocked-
until-live command behavior. Express restart through reused allocation identity,
durable restart count, rule presence, and wire evidence. End S-GTI-12 after the
target rule is absent and every peer rule is unchanged.

### AD-Q7-07 — named token and static-apply failure partitions are collapsed

**Severity:** HIGH  
**Dimensions:** coverage completeness, CM-H  
**Status:** Open; remediation required

**Evidence**

- `network_token_parse_malformed` collapses malformed address, prefix, gateway,
  and DNS into one row (`test-scenarios.md:273`).
- `static_ipv4_apply` collapses address, netmask, link-up, and default-route
  failures into one row (`test-scenarios.md:279`).
- The self-audit nevertheless says each declared error and closed set are covered
  (`test-scenarios.md:353-355`).

These stages have distinct sequencing and diagnostic obligations. A row that says
“A, B, C, or D fails” does not specify which examples will exist, what input
forces each arm, or the exact expected stage for each result.

**Required remediation**

Enumerate stable named subcases for malformed address, prefix, gateway, and DNS,
and for address apply, netmask apply, link-up, and default-route apply. Each row
must pin its forcing input, exact typed/stage result, forbidden later operations,
and required diagnostic. Keep them source-local and table-driven; no extra VM boot
is needed.

### AD-Q7-08 — the 15/15 AT-completeness verdict is mechanically incorrect

**Severity:** BLOCKER  
**Dimensions:** coverage completeness, lifecycle/Q7/C2  
**Status:** Open; remediation required

**Evidence**

- **C1b:** the scenarios cover valid slots 0 through `NET_SLOT_MAX`, but do not
  specify max-1 and max+1 rejection as executable examples
  (`test-scenarios.md:309-328`, audit claim at lines 344-345).
- **C2b:** the audit says S-GTI-08 “rejects every setup failure after READY”
  (`test-scenarios.md:347`), while S-GTI-08 and the approved model place every
  setup failure before READY (`test-scenarios.md:59`, `:242-244`). It therefore
  supplies no illegal-event-from-each-state coverage.
- **C4b:** the canonical item is inverse operation **without** its prerequisite.
  S-GTI-12 expressly Givens a present installed rule (`test-scenarios.md:255-257`),
  so it is the normal inverse path, not uninstall-without-install. The PASS claim
  at lines 349-350 is self-contradictory.
- **C6b:** distinct token/static-apply errors are collapsed as described in
  AD-Q7-07, so every declared arm is not yet a named executable example.
- **C7b:** S-GTI-06 restarts after Running, S-GTI-05 injects install failure, and
  S-GTI-08 performs failure-triggered shutdown. None is an interruption in the
  middle of the mutating operation claimed at `test-scenarios.md:357`.
- **C7c:** per-slot pure uniqueness is not a multi-actor test with two parallel
  invocations, which the canonical check requires when concurrency safety is
  claimed (`test-scenarios.md:358`).

Even granting C3 as N/A because the pure derivation consumes one slot rather than
a collection, the documented evidence supports at most **9/15**, which is
`INCOMPLETE`, not 15/15 `COMPLETE`. The duplicated 15/15 claim in
`feature-delta.md:773-787` is therefore also wrong.

**Required remediation**

Add the missing boundary, illegal-transition, inverse-without-prerequisite,
named-error, interruption, and parallel-actor cases, or mark genuinely
inapplicable items PASS with a valid rationale. Recompute the checklist
mechanically and update both DISTILL artifacts to the same result. Do not count a
different test shape as a semantic substitute for the named checklist item.

### AD-Q7-09 — the package declares two walking skeletons

**Severity:** MEDIUM  
**Dimensions:** walking-skeleton strategy, traceability  
**Status:** Open; remediation required

**Evidence**

- Both S-GTI-01 and S-GTI-02 carry `@walking_skeleton`
  (`test-scenarios.md:168`, `:180`; duplicated in `feature-delta.md:696-697`).
- The package later describes one walking-skeleton journey, with S-GTI-02 merely
  chained onto it (`feature-delta.md:717-729`).
- `nw-distill` requires exactly one walking-skeleton scenario per feature.

**Required remediation**

Retain `@walking_skeleton @driving_port` on S-GTI-01. Keep S-GTI-02 as the focused
born-captured security scenario with `@driving_port @real-io @kvm @property`, or
merge its proof into S-GTI-01 if one scenario remains readable. Update both tag
tables consistently.

## Correctness gates

| Gate | Verdict | Notes |
|---|---|---|
| Q7 READY lifecycle | PARTIAL | The state model is correct, but its metal forcing case and private-observable split are not. |
| S-GTI-08 real driving port | FAIL | Malformed platform token is not operator-producible; private view/actions are not port observables. |
| S-GTI-02 born-captured proof | FAIL | The evidence universe is strong, but allocation-specific readiness is placed before deploy and `start_alloc` is asserted internally. |
| S-GTI-05 install refusal | PASS at specification level | Terminal/no-command/no-frame/no-cleartext outcomes are coherent. |
| S-GTI-06 restart | FAIL | Happy and failed reinstall are combined; code-path assertions leak into Gherkin. |
| S-GTI-12 teardown | PARTIAL | Rule removal and peer preservation are good; inverse-without-prerequisite coverage and business-language shape are missing. |
| Pure properties | PASS at specification level | Domain/range and exact contract-shape declaration are pinned at the appropriate layer. |
| Named setup failures | FAIL | Token and static-apply stages need explicit named subcases. |
| RED classification | FAIL | Current tree and historical snapshot are conflated; GREEN tests are called RED. |
| Contract-shape tags | PASS | 12/12 scenarios have a valid contract-shape tag. |

## Verification record

Static commands used:

```text
git diff --name-status 7255c68e^ 7255c68e
rg -n '^Scenario:|^@.*contract-shape:|@walking_skeleton' <DISTILL artifacts>
git show 7255c68e^:<relevant Rust path>
rg -n 'RED scaffold|should_panic|CONTRACT_SHAPE|derive_vm_tap_plan' <relevant Rust paths>
```

Results:

- Commit scope: three documentation files, as declared.
- Scenario count: 12; contract-shape tags: 12.
- Walking-skeleton tags: 2.
- Commit-parent S-GTI-09/-10/-11: live properties, not RED scaffolds.
- Commit-parent metal placement: S-GTI-01/-02/-03/-04/-07 live bodies;
  S-GTI-05/-06/-08/-12 explicit RED scaffolds.
- Review artifact is the only file written by this reviewer.

## Verdict and remediation disposition

**REJECTED_PENDING_REVISIONS.** There are five blocker findings, three high
findings, and one medium finding. CM-A, CM-B, and CM-C fail, the mechanically
recomputed completeness result is below the COMPLETE threshold, and the RED
snapshot is not truthful for the reviewed tree.

All findings are open. After remediation, the same acceptance-review role must
re-review the revised artifacts. Approval requires zero blocker, critical, high,
or medium findings.

---

## Iteration 2 — 2026-08-29

**Review ID:** `accept_rev_2026-08-29_q7_a5bd158e_i2`  
**Reviewer:** same `nw-acceptance-designer-reviewer` role, isolated review  
**Reviewed commit:** `a5bd158edb76308a44ccf99597d349950898f0f7`  
**Compared with:** first parent `cd12725159a6b2a92619f17aa4dc5f0ff621b842`  
**Model/reasoning:** inherited GPT 5.6 Luna / maximum  
**Verdict:** **REJECTED_PENDING_REVISIONS**

### Scope and evidence

Iteration 2 re-evaluated AD-Q7-01 through AD-Q7-09 against the complete target
commit and its immutable parent, then scanned the approved DESIGN/ADR handoff,
the full feature delta, the three DISTILL documents, and E07/E08/E09 for
regressions. The source tree was read only where an immutable-parent fact or the
existing metal launcher behavior was needed. Pre-existing dirty source and
DELIVER work was excluded and not modified.

No test suite was run. The remediation is documentation and pending-EDD work,
all current classifications expressly say `NOT_EXECUTED`, and native runtime
evidence is not yet available. Static checks covered commit scope, scenario and
tag counts, Contract Shape declarations, target-parent test placement, EDD file
modes/status, cross-wave contradictions, and the canonical 15-item audit.

### Strengths retained or added

- S-GTI-01/-02 no longer pre-create allocation identity in Given. The observer
  starts identity-free and learns C3 facts only after the real deploy action
  (`test-scenarios.md:59-62,195-217`).
- S-GTI-08a now uses a production-reachable custom-rootfs resolver failure and
  keeps private lifecycle facts in `C-GTI-08-RECONCILE`
  (`test-scenarios.md:269-280,380-392`).
- Restart success/failure, pre-/post-READY outcomes, and stop with/without a rule
  are distinct examples. All fifteen Gherkin scenarios have one exact Contract
  Shape tag; exactly one actual `@walking_skeleton` tag exists.
- The immutable-parent table no longer converts historical GREEN/RED into a
  current result. Every row is honestly `NOT_EXECUTED`, and the five inherited
  live metal bodies, four panic scaffolds, three live derivation properties, and
  newly incomplete split obligations are distinguished.
- D7 is non-vacuous and fail-conservative: complete normalized program identity,
  strict multipart framing, stable generation brackets, loss notification,
  `C > 0`/`L > 0`, checked equality in both directions, full capture, and
  sibling preservation are explicit (`test-scenarios.md:81-139`).
- E07/E08/E09 are executable pending stubs with no fabricated evidence. Their
  READMEs pin real built-binary journeys, finite deadlines, kernel/wire/state
  evidence, and bounded cleanup.

### Prior-finding dispositions

| Iteration-1 finding | Iteration-2 disposition | Evidence |
|---|---|---|
| AD-Q7-01 — post-deploy facts in Given | **CLOSED** | The harness has no allocation-specific identity in Given; it binds after real C3 and arms capture before VMM spawn (`test-scenarios.md:59-62,195-217`). |
| AD-Q7-02 — unreachable malformed-token metal fixture | **CLOSED** | S-GTI-08a drives a normal deploy-selected custom rootfs whose resolver target makes the production write fail (`test-scenarios.md:269-280`; E08 README `:7-12,26-37`). |
| AD-Q7-03 — private reconciler facts in metal Gherkin | **CLOSED** | Port-visible state remains in S-GTI-08a; exact action/View assertions moved to `C-GTI-08-RECONCILE` (`test-scenarios.md:380-389`). |
| AD-Q7-04 — stale RED snapshot | **CLOSED for the original false claims** | The exact base is named, every current result is `NOT_EXECUTED`, and inherited bodies/scaffolds/new obligations are separated (`red-classification.md:3-39`). AD-Q7-13 records a newly found omission in the supporting-contract inventory. |
| AD-Q7-05 — combined reinstall success/failure | **CLOSED** | S-GTI-06a and S-GTI-06b are separate examples with mutually exclusive fixtures and outcomes (`test-scenarios.md:243-260`). |
| AD-Q7-06 — source paths/private calls in Then | **CLOSED for the cited leaks** | `:1880`, `:1269`, `:2038`, `DriverType::Exec`, and `start_alloc` are absent from Gherkin. AD-Q7-14 records the remaining meta-test language in the walking skeleton. |
| AD-Q7-07 — collapsed token/static failure partitions | **CLOSED** | Address, prefix, gateway, DNS, suppression, address, netmask, link, route, and resolver cases are separate (`test-scenarios.md:339-366`). |
| AD-Q7-08 — false 15/15 completeness | **PARTIALLY RESOLVED; BLOCKER remains** | Boundary, illegal-state, inverse, interruption, and concurrency text was added, but five checklist items still lack mechanically valid AT support; see AD-Q7-08 below. |
| AD-Q7-09 — two walking-skeleton tags | **CLOSED** | Static count is one actual `@walking_skeleton`, on S-GTI-01. The summary-table spelling defect is only LOW (AD-Q7-16). |

### Iteration-2 scorecard

| Dimension | Score / 10 | Result | Rationale |
|---|---:|---|---|
| Happy-path bias | 8 | Good | Error, invariant, teardown, interruption, degraded-resource, and concurrent cases are represented. |
| GWT format | 5 | Below standard | S-GTI-08b uses internal EXEC as When; S-GTI-09/-10/-11 place a second invalid invocation in Then. |
| Business language | 5 | Below standard | The walking skeleton says a named test oracle passes and exposes C3/VMM mechanics rather than the stakeholder-visible security outcome. |
| Coverage completeness | 4 | Reject | The mechanical result is 10/15, not the claimed 15/15; an approved total classifier property is also absent from the handoff. |
| Walking-skeleton user-centricity | 5 | Below standard | The dial/reply value is present, but a core Then is a D7 test-harness verdict. |
| Priority validation | 9 | Excellent | Q7 lifecycle honesty and Q9 born-captured proof remain the correct critical risks. |
| Observable behavior | 6 | Acceptable with gaps | Private View/action checks were separated; S-GTI-08b still drives an internal event rather than an operator port. |
| Traceability coverage | 3 | Reject | The effective DESIGN/ADR texts contradict both the restart route and the runtime substrate selected by DISTILL. |
| Walking-skeleton boundary proof | 9 | Excellent | Identity is learned after real C3, capture is armed before real VMM spawn, and D7 fails closed on ambiguity/loss. |

### Mandate verdicts

| Mandate | Verdict | Evidence |
|---|---|---|
| CM-A — hexagonal boundary | **FAIL** | S-GTI-08b's When is internal `EXEC`, not deploy or another driving-port action (AD-Q7-10). |
| CM-B — business language | **FAIL** | S-GTI-01 asserts that “the D7 oracle passes” and names harness/VMM mechanics instead of an observable security outcome (AD-Q7-14). |
| CM-C — complete user journey | **FAIL** | S-GTI-08b starts after READY and never names the operator action that creates the run being classified (AD-Q7-10). |
| CM-D — pure extraction | **PARTIAL** | Token/suppression/D7 logic is assigned to pure properties, but the approved total exit classifier property is absent from the executable/classification handoff (AD-Q7-13). |
| CM-F — PBT layer | PASS | Generative obligations remain source-local/layer 1; real-I/O scenarios are concrete examples. |
| CM-H — named layer-3 sad paths | PASS | Fresh install, reinstall, and resolver failure are separate, named real-I/O outcomes; detailed parser/apply failures remain source-local. |

### Canonical AT-completeness recomputation

The canonical checklist is evidence-based: prose mentioning a condition is not
an AT unless the package defines a driveable example/property and maps it to an
executable obligation. On that basis the current score is **10/15 —
ACCEPTABLE_WITH_DOCUMENTED_GAPS**, not `15/15 → COMPLETE`.

| Item | Verdict | Iteration-2 evidence |
|---|---|---|
| C1a | PASS | S-GTI-09/-10/-11 include slot zero and one. |
| C1b | **FAIL** | Each Given is restricted to a valid slot, its When derives that valid plan, and only Then introduces `max + 1`; no action invokes the rejected boundary (`test-scenarios.md:290-313`). |
| C2a | PASS | The legal lifecycle is explicit (`test-scenarios.md:38-79`). |
| C2b | **FAIL** | The seven-row matrix has no named test/property identity, executable map, or immutable-baseline classification, so it is not mechanically an AT per state (`test-scenarios.md:394-404,464-479`; `red-classification.md:41-54`). |
| C3 | PASS | Source-local netlink properties specify zero, one, and duplicate/many exact targets (`test-scenarios.md:406-416`). |
| C4a | **FAIL** | Same-tag adoption and repeated stop cover two operations, not “each mutating op”; no apply-twice inventory or mapped AT exists for the C3/guest-apply/install mutations (`test-scenarios.md:139,449`). |
| C4b | PASS | S-GTI-12b repeats stop when no target guard was installed. |
| C5a | PASS | The materially distinct mesh/non-mesh and fresh/reclamation success/failure outcomes are separated. |
| C5b | PASS | No independent user flag matrix is introduced; teardown is checked with and without the guard prerequisite. |
| C6a | PASS | The platform token's address/prefix/gateway/DNS malformed partitions are distinct. |
| C6b | **FAIL** | Minimal-root initialization is a declared pre-READY error stage (`test-scenarios.md:64-68`) but is absent from the closed failure table and executable map; the existing immutable-base test is not cited as coverage. |
| C6c | **FAIL** | The table names stages and “typed stage-specific diagnostic” generically, but does not enumerate a closed typed error set, and the omitted minimal-root stage disproves “every sanctioned stage” (`test-scenarios.md:339-366,453-455`). |
| C7a | PASS | Resolver, console-read, capture-loss, netlink-loss, and strict timeout cases fail conservatively. |
| C7b | PASS | M-GTI-INTERRUPT-BOOT interrupts the real VMM between capture-ready and READY and requires total cleanup. |
| C7c | PASS | M-GTI-CONCURRENT-DEPLOY specifies two parallel built-binary deploys and allocation-scoped deltas. |

### Findings

#### AD-Q7-08 — the canonical completeness claim remains mechanically false

**Severity:** BLOCKER  
**Kind:** `at_gap_in_delivery_scope`  
**Status:** Open after partial remediation

The target repeats `15/15 → COMPLETE` in both `test-scenarios.md:442-461` and
`feature-delta.md:936-949`. The recomputation above supports 10/15. In
particular, the invalid `max + 1` action sits in Then, the illegal-state matrix
is not a mapped AT set, only two of the feature's mutating operations have any
repeat contract, and minimal-root failure is missing from the supposedly closed
error inventory.

**Required remediation:** create driveable/mapped boundary-rejection examples,
give each illegal-state case a stable test/property identity and immutable-base
classification, inventory every mutating operation and add apply-twice coverage
or a justified N/A, and add/map minimal-root failure plus the closed typed-error
set. Recompute the checklist from those artifacts and update both duplicate
claims to the same honest score.

#### AD-Q7-10 — S-GTI-08b begins after the operator action and uses internal EXEC as When

**Severity:** HIGH  
**Dimensions:** GWT temporal honesty, CM-A, CM-C  
**Status:** Open

`test-scenarios.md:282-288` Givens a Job that has already completed setup and
reported READY, then uses “EXEC runs” as the action. EXEC is an internal control
message, not an operator driving port; the real deploy that created READY is
outside the scenario. E08 already states the honest executable journey:
`overdrive deploy <ready-then-exit-78-spec>` followed by describe
(`E08 README:26-46`).

**Required remediation:** Given only an available composed serve and observation
harness; When the operator deploys a VM Job whose command exits 78; Then prove
READY precedes EXEC/EXIT and describe reports ordinary exit 78 without setup
rejection or restart. Keep any direct READY/EXEC mapping check in the separately
named component example.

#### AD-Q7-11 — the corrected same-allocation scenario contradicts its authoritative D6 sources

**Severity:** HIGH  
**Kind:** `specification_ambiguity`  
**Dimensions:** cross-wave traceability, restart-loop safety  
**Status:** Open

DISTILL correctly states that natural Job exit/crash finalizes, workload restart
mints a fresh allocation, and only platform reclamation with standing intent
re-drives the same id (`test-scenarios.md:73-79,243-260`). But the artifact calls
DESIGN authoritative while effective D6 still says restart budget, crash
recovery, and workload restart are “all live for VM”
(`design/wave-decisions.md:18`; ADR-0089 `:56-64`;
`feature-delta.md:570-578`). The current feature delta therefore contains both
opposite contracts, despite the reconciliation result saying there is no
unresolved DESIGN ambiguity (`test-scenarios.md:34-36`).

**Required remediation:** amend D6, ADR-0089, and the DESIGN section of the
feature delta to select platform reclamation with standing intent as the sole
same-allocation VM Job route; preserve the two-site gate requirement and
explicitly classify natural exit and generation replacement. Then rerun the
cross-wave reconciliation.

#### AD-Q7-12 — native-only execution is contradicted by effective DESIGN text requiring nested KVM

**Severity:** HIGH  
**Kind:** `specification_ambiguity`  
**Dimensions:** environment traceability, evidence validity  
**Status:** Open

The remediated contract and E07/E08/E09 correctly reject virtualized/nested
hosts and require native non-virtualized x86_64 KVM
(`test-scenarios.md:141-153`; E07 README `:19-26`). Effective DESIGN still says
the same tests “require nested KVM” (`design/wave-decisions.md:68-77`;
`feature-delta.md:640-647`). Because DISTILL explicitly names those sources as
authoritative, a runner author can follow the older text and publish evidence
from a host the current preflight forbids.

**Required remediation:** align every effective DESIGN/ADR/feature-delta
execution-surface statement with native, non-virtualized x86_64 KVM. Historical
nested-KVM wording may remain only when explicitly marked superseded.

#### AD-Q7-13 — the total Job classifier property is missing from the executable and RED handoff

**Severity:** HIGH  
**Dimensions:** Q7 coverage, Contract Shape, immutable-baseline classification  
**Status:** Open

Approved DESIGN requires a source-local property over every `Option<i32>` exit
code plus arbitrary signal with exact
`/// CONTRACT_SHAPE: pure-function.` (`design/wave-decisions.md:164-166`;
`feature-delta.md:214-218`). The supporting section now describes only one
seeded reconciler example (`test-scenarios.md:380-389`); its general Contract
Shape sentence does not state the classifier's domain. The adapter map has no
total classifier property, and `red-classification.md:43-54` has no row recording
that this property is absent at immutable base `cd127251`. Nevertheless the
feature delta claims the classifier property carries the exact declaration
(`feature-delta.md:982-983`). Static inspection of the immutable base finds the
classifier function but no such property.

**Required remediation:** add a named source-local total classifier property to
the scenario/adapter map, pin every `Option<i32>` plus arbitrary signal and the
exact pure-function rustdoc line, and classify it `NOT_EXECUTED /
NEWLY_INCOMPLETE` against `cd127251`. Assign exact Contract Shapes to the named
component/native supporting examples as part of their executable handoff.

#### AD-Q7-14 — the walking skeleton still asserts test machinery instead of behavior

**Severity:** HIGH  
**Dimensions:** business language, user-centricity, CM-B  
**Status:** Open

S-GTI-01's stakeholder-facing Then says the deployment “learns and captures its
interfaces before VMM spawn” and that “the ratified D7 ... oracle passes”
(`test-scenarios.md:195-203`). Those are harness/test-mechanism statements, not
an outcome a stakeholder can observe. The actual user value—first connection
protected, no peer-path cleartext, reply received—is available elsewhere but is
not what this skeleton says.

**Required remediation:** express S-GTI-01 in behavior language: the VM Job's
first named-peer connection succeeds, is protected end-to-end, and has no
cleartext peer-path copy before Running. Keep C3/VMM/capture/GETRULE/GETGEN
mechanics in S-GTI-02 and the E07 verification contract.

#### AD-Q7-15 — the lease contract still begins after the shared remote-tree sync

**Severity:** MEDIUM  
**Dimensions:** C7 concurrency, non-vacuous evidence isolation  
**Status:** Open

The target says the descriptor spans preflight through remote command completion
(`test-scenarios.md:155-162`), and E07 says to acquire it “before the remote
command starts” (`E07 README:38-43`). But `cargo xtask metal run` performs
`metal_sync` before creating the remote command (`xtask/src/main.rs:716-746`),
and all workspaces sync with delete semantics into the same `~/overdrive` tree.
Two runs can therefore overwrite the tree before either in-command lease takes
effect, producing mixed-commit evidence.

**Required remediation:** require the global metal-host lease to be acquired
before rsync and held by a supervising session across sync, preflight, execution,
evidence capture, and final cleanup. The owner/commit diagnostics and finite
timeout remain mandatory.

#### AD-Q7-16 — summary tables do not reproduce the actual walking-skeleton tag

**Severity:** LOW  
**Dimensions:** traceability consistency  
**Status:** Open

The Gherkin correctly has one `@walking_skeleton`, but both summary tables write
the non-tag string `walking-skeleton` without `@`
(`test-scenarios.md:176`; `feature-delta.md:868`).

**Required remediation:** render the table entry as `@walking_skeleton` so the
human summaries and mechanically counted tag agree.

### Correctness gates

| Gate | Verdict | Notes |
|---|---|---|
| Given/When/Then temporal honesty | **FAIL** | S-GTI-01/-02 are fixed; S-GTI-08b and max+1 boundary clauses remain structurally dishonest. |
| Production-reachable pre-READY sample | PASS | S-GTI-08a uses a deploy-selected custom-rootfs resolver failure. |
| Private component/port separation | PASS | S-GTI-08a and `C-GTI-08-RECONCILE` own distinct observable universes. |
| Immutable-baseline classification | **PARTIAL** | Current rows and `NOT_EXECUTED` results are honest, but the required total classifier property is omitted. |
| Split reinstall outcomes | PASS downstream | S-GTI-06a/b are split; the authoritative D6 route contradiction remains. |
| Behavior language | **FAIL** | The walking skeleton asserts the D7 oracle/harness mechanics. |
| Named failure partitions | PASS for listed token/apply stages | The iteration-1 collapsed rows are fixed; completeness still omits the declared minimal-root failure. |
| Canonical completeness | **FAIL** | Mechanical score is 10/15, not 15/15. |
| Exactly one walking-skeleton tag | PASS | One actual tag; summary spelling is LOW only. |
| Exact Contract Shapes | **PARTIAL** | 15/15 Gherkin scenarios have exact tags; the total classifier/supporting-example handoff is incomplete. |
| EDD stubs | PASS | E07/E08/E09 are pending, executable, non-fabricating stubs with mapped outcomes. |
| D7 oracle | PASS at specification level | Non-vacuous, strict, complete, mutation/loss conservative, exact packet/byte equality. |
| Cleanup totality | PASS at specification level | Failed-start and stop contracts name bounded full residue sets and sibling preservation. |
| Host-wide lease | **FAIL** | The documented lease does not explicitly cover the pre-command shared-tree sync. |

### Verification record

Static verification established:

- target first parent: `cd12725159a6b2a92619f17aa4dc5f0ff621b842`;
- commit scope: three DISTILL/feature documents, three EDD README+runner pairs,
  and `verification/expectations/INDEX.md`;
- Gherkin scenarios: 15; exact Contract Shape tag lines: 15; actual
  `@walking_skeleton` tag lines: 1;
- E07/E08/E09 runners are mode `100755`, all explicitly pending and
  non-evidentiary;
- immutable-base test placement matches the remediated classification, except
  that the required total classifier property has no base test and no
  classification row;
- D6 and execution-surface contradictions remain in effective upstream text;
- the only file written by this reviewer is this Markdown review artifact.

### Iteration-2 verdict and remediation disposition

**REJECTED_PENDING_REVISIONS.** Six iteration-1 findings are closed, two are
closed with narrower new findings, and AD-Q7-08 remains open. The current open
set is one blocker, five high findings, one medium finding, and one low finding.
CM-A, CM-B, and CM-C fail; the completeness claim is not mechanically supported;
two authoritative cross-wave contracts disagree with the remediated scenarios;
and the host-wide lease does not yet cover the shared-tree sync.

Approval requires zero blocker, critical, high, or medium findings. The same
reviewer must re-review the next remediation after the Markdown artifact is
updated and the contradictions are removed at their authoritative sources.

---

## Iteration 3 — 2026-08-29

**Review ID:** `accept_rev_2026-08-29_q7_ed332f89_i3`  
**Reviewer:** same `nw-acceptance-designer-reviewer` role, isolated review  
**Reviewed commit:** `ed332f8972c6285fb067d995d73f51ee63a5ff01`  
**Compared with:** first parent `85550e4a267cbd53ac266fa54f4d8cda164910af`  
**Model/reasoning:** inherited GPT 5.6 Luna / maximum  
**Verdict:** **REJECTED_PENDING_REVISIONS**

### Scope and evidence

Iteration 3 re-evaluated AD-Q7-08 and AD-Q7-10 through AD-Q7-16 against the
complete target commit, its immutable parent, and the iteration-6-approved
DESIGN correction in that parent. It also rescanned all fifteen stakeholder
scenarios, the supporting-property/component inventory, immutable-baseline
classification, feature-delta DISTILL handoff, and E07/E08/E09.

Source was read only to validate immutable-parent test existence and the
production metal/nft call sequence. Pre-existing dirty source and DELIVER work
was excluded and not modified. No test suite or EDD runner was executed: the
target is a specification/pending-stub change and all current results remain
explicitly `NOT_EXECUTED` / `pending`.

### What is now correct

- Effective D6, ADR-0089, the architecture brief, and the DESIGN feature-delta
  section now agree with S-GTI-06: only boot-epoch Platform Reclamation with
  standing intent re-drives the same VM Job allocation; natural Job exit/crash
  finalizes and workload restart creates a fresh allocation.
- Every effective execution-surface statement now selects native,
  non-virtualized x86_64 KVM and rejects Lima/virtualized/nested evidence.
- S-GTI-08b now has an honest deploy action. READY-before-EXEC-before-EXIT and
  ordinary exit 78 are observed after that action (`test-scenarios.md:298-305`).
- The total Job exit classifier is named, mapped, assigned the exact
  `/// CONTRACT_SHAPE: pure-function.` declaration, and honestly classified
  absent at the immutable parent (`test-scenarios.md:429-451,553-566`;
  `red-classification.md:52`).
- S-GTI-01 is now stakeholder-facing: named-peer success, reply, platform
  protection, and no peer-path cleartext. D7 mechanics remain in S-GTI-02/E07.
- The outer supervising lease is acquired and acknowledged before the shared
  `rsync --delete`, then held on the same remote descriptor through preflight,
  execution, evidence, cleanup, and final probes (`test-scenarios.md:167-182`;
  E07 README `:39-48`).
- The table spelling now matches the single actual `@walking_skeleton` tag.
- D7 remains non-vacuous and loss/mutation conservative. The teardown oracle is
  improved to exact ordered-sequence filtering by target handle rather than the
  impossible claim that absolute sibling ordinals remain unchanged
  (`test-scenarios.md:84-149,329-344`).

### Prior-finding dispositions

| Prior finding | Iteration-3 disposition | Evidence |
|---|---|---|
| AD-Q7-08 — false canonical completeness | **PARTIALLY RESOLVED; MEDIUM remains** | Boundary rejection, illegal-state identities, minimal-root/closed-error cases, classifier mapping, and a mutation inventory now exist. One C4a row still declares at-most-once mutations N/A without a correct-non-idempotency AT; see AD-Q7-08. |
| AD-Q7-10 — S-GTI-08b internal EXEC as When | **CLOSED** | The When is now the operator's built deploy; READY/EXEC/EXIT and describe are outcomes (`test-scenarios.md:298-305`; E08 README `:31-48,70-78`). |
| AD-Q7-11 — D6 route contradiction | **CLOSED** | Parent `85550e4a` aligns DESIGN D6, ADR-0089, and feature-delta DESIGN text with Platform Reclamation as the sole same-id route. |
| AD-Q7-12 — native/nested contradiction | **CLOSED** | Parent `85550e4a` and the target consistently select native non-virtualized x86_64 KVM and explicitly supersede historical nested wording. |
| AD-Q7-13 — missing total classifier property | **CLOSED** | `P-GTI-JOB-EXIT-CLASSIFIER` pins every `Option<i32>` plus arbitrary signal, exact mapping/declaration/location, and `NOT_EXECUTED / NEWLY_INCOMPLETE` baseline status (`test-scenarios.md:431-437`; `red-classification.md:52`). |
| AD-Q7-14 — walking skeleton asserted test machinery | **CLOSED** | S-GTI-01 now contains only stakeholder-visible connection, reply, protection, and no-cleartext outcomes (`test-scenarios.md:212-219`). |
| AD-Q7-15 — lease began after sync | **CLOSED** | The remote helper acknowledges ownership before sync and the same fd spans sync through final probes, including signal/error cleanup (`test-scenarios.md:167-182`; E07 README `:39-48`). |
| AD-Q7-16 — walking-skeleton table spelling | **CLOSED** | Both scenario tables use exact `@walking_skeleton` (`test-scenarios.md:194`; `feature-delta.md:881`). |

Iteration-1 AD-Q7-01 through AD-Q7-07 and AD-Q7-09 remain closed; no regression
was found in their original defect classes.

### Iteration-3 scorecard

| Dimension | Score / 10 | Result | Rationale |
|---|---:|---|---|
| Happy-path bias | 9 | Excellent | Distinct fresh-install, reinstall, pre-READY, interruption, degraded-resource, concurrent, inverse, and cleanup paths are specified. |
| GWT format | 6 | Below standard | S-GTI-08b is fixed, but S-GTI-07 and S-GTI-08a each retain deploy plus describe as two When actions. |
| Business language | 6 | Below standard | The walking skeleton is fixed; S-GTI-05 newly exposes an E08-owned nft fixture in stakeholder Gherkin. |
| Coverage completeness | 8 | Good with one gap | Fourteen checklist items are mechanically supported; C4a's at-most-once resource row lacks the required duplicate-attempt contract. |
| Walking-skeleton user-centricity | 10 | Excellent | S-GTI-01 is now a complete, demonstrable operator outcome. |
| Priority validation | 9 | Excellent | The package remains centered on born-captured security and truthful boot/restart behavior. |
| Observable behavior | 8 | Good | Port and private component universes remain separated; E09's failed-reinstall forcing setup is still underspecified. |
| Traceability coverage | 9 | Excellent | D6 and native-metal sources are aligned; the one stale product journey is explicitly recorded and owned. |
| Walking-skeleton boundary proof | 9 | Excellent | The observer remains identity-free before deploy and D7 binds only after real C3. |

### Mandate verdicts

| Mandate | Verdict | Evidence |
|---|---|---|
| CM-A — hexagonal boundary | PASS | Walking/metal journeys use built deploy/describe/Job-stop; private lifecycle facts remain in a separately mapped component example. |
| CM-B — business language | **FAIL** | S-GTI-05 names the E08 test-owned regular `prerouting` fixture and production TPROXY append in Gherkin (AD-Q7-18). |
| CM-C — complete user journey | **FAIL** | S-GTI-07 and S-GTI-08a each place two operator commands in one When rather than one action followed by observations (AD-Q7-17). |
| CM-D — pure extraction | PASS | Slot boundary, error closure, classifier, illegal transitions, and D7 decoder/oracle obligations are assigned to source-local pure properties. |
| CM-F — PBT layer | PASS | Generative obligations remain source-local; real-I/O paths remain named examples. |
| CM-H — named layer-3 sad paths | PASS | Fresh guard failure, same-id reinstall failure, resolver failure, interruption, and cleanup paths are distinct. |

### Canonical AT-completeness recomputation

The remediation closes C1b, C2b, C6b, and C6c and maps the total classifier.
The mechanical result is now **14/15 — COMPLETE by the canonical ≥13 threshold**,
but it is not the claimed `15/15` because C4a is still unsupported for one
declared operation class.

| Item | Verdict | Iteration-3 evidence |
|---|---|---|
| C1a | PASS | S-GTI-09/-10/-11 cover zero and one. |
| C1b | PASS | Valid max-1/max remain in S-GTI-09/-10/-11; `P-GTI-SLOT-BOUNDARY` makes `MAX + 1` the action and requires typed pre-derivation rejection. |
| C2a | PASS | The legal lifecycle is explicit. |
| C2b | PASS | `P-GTI-ILLEGAL-01` through `-07` have stable identities, source, shape, expected disposition, and baseline rows. |
| C3 | PASS | D7 target-selection properties cover zero/one/duplicate target cardinality; the stop/E07/E09 examples cover empty/singleton/multi-rule result sequences. |
| C4a | **FAIL** | The inventory's rootfs clone/run-directory/listener/VMM/capture row calls duplicate creation illegal but maps only an “ownership/state-machine proof” and teardown replay, not an AT that applies/replays the creation and asserts the correct rejection (`test-scenarios.md:475-488`). |
| C4b | PASS | S-GTI-12b applies Job stop without an installed target guard. |
| C5a | PASS (N/A) | No independent user mode-flag parameter is introduced; mesh/restart branches are scenarios, not flags. |
| C5b | PASS (N/A) | With no mode flags, orthogonality is not applicable. |
| C6a | PASS | Malformed address/prefix/gateway/DNS cases are separate. |
| C6b | PASS | The stable table forces every declared pre-READY stage plus install/diagnostic/D7 failures. |
| C6c | PASS | `P-GTI-PRE-READY-ERROR-CLOSURE` pins the exact ten-variant pre-READY set and excludes post-READY variants. |
| C7a | PASS | Read, resolver, loss, timeout, and malformed-resource cases fail conservatively. |
| C7b | PASS | M-GTI-INTERRUPT-BOOT interrupts a real boot between capture-ready and READY. |
| C7c | PASS | M-GTI-CONCURRENT-DEPLOY drives two built deploys under one host lease. |

### Findings

#### AD-Q7-08 — C4a still counts an unmapped at-most-once mutation as covered

**Severity:** MEDIUM  
**Kind:** `at_gap_in_delivery_scope`  
**Status:** Open after substantial remediation

`test-scenarios.md:469-488` now inventories replay behavior, but the row for
rootfs clone, run directory, listeners, VMM, and capture processes says duplicate
creation is illegal and then marks apply-twice N/A. Its evidence is only an
unnamed “ownership/state-machine proof” plus teardown replay. Teardown twice does
not prove that a duplicate create/spawn attempt is rejected without replacing,
leaking, or cross-owning the first resource. `P-GTI-ILLEGAL-01/-04` cover missing
C3 identity and premature EXEC, not duplicate clone/listener/capture creation.
Consequently C4a's “each mutating op” requirement is not fully mapped and the
two `15/15` claims (`test-scenarios.md:531-550`; `feature-delta.md:971-989`) are
one point too high.

**Required remediation:** either define a stable component/state-machine test
that repeats the at-most-once creation request and proves the exact typed/no-
replacement/no-leak result for the grouped resources, or narrow the inventory
to the application-level mutating operation that owns them and map its correct
non-idempotency property. Record the obligation in `red-classification.md` and
publish the honest score. Do not use teardown replay as proof of duplicate-create
behavior.

#### AD-Q7-17 — two scenarios still contain deploy and describe as separate When actions

**Severity:** HIGH  
**Dimensions:** GWT format, temporal honesty, CM-C  
**Status:** Open

- S-GTI-07 says the operator deploys and runs describe in one When
  (`test-scenarios.md:278-283`).
- S-GTI-08a likewise deploys the failing Job and runs describe in one When
  (`test-scenarios.md:285-296`).

Describe is the observation surface already referenced by each Then; it is not
the stimulus under test. Combining both commands makes the action boundary
ambiguous and defeats the one-action GWT rule. The deliberate repeated stop in
S-GTI-12b is different: repetition is the idempotency stimulus itself.

**Required remediation:** make deploy the single When in S-GTI-07/-08a. Phrase
the Then clauses as eventual describe observations, including their bounded poll
where relevant. No scenario split or behavior change is required.

#### AD-Q7-18 — S-GTI-05 leaks the E08 nft fixture into stakeholder Gherkin

**Severity:** HIGH  
**Dimensions:** business language, fixture abstraction, CM-B  
**Status:** Open

`test-scenarios.md:250-257` Givens “the E08 test-owned regular `prerouting`
chain fixture” and “the real production TPROXY append.” These are test-harness
and kernel implementation details, not stakeholder language. The concrete
hookless-chain construction is valuable and non-vacuous, but it already belongs
in E08's verification contract (`E08 README:51-66`).

**Required remediation:** state the Gherkin precondition behaviorally—for
example, that the qualified native host will reject installation of the fresh
VM Job's egress guard. Keep table/chain/sentinel, kernel-hook validation, errno,
baseline, and restoration mechanics exclusively in E08/supporting test design.

#### AD-Q7-19 — E09 does not define a production-reachable failed-reinstall forcing setup

**Severity:** MEDIUM  
**Dimensions:** EDD non-vacuity, fail-for-the-right-reason, cleanup isolation  
**Status:** Open

S-GTI-06b and E09 require a deterministic real kernel rejection after the target
has already reached Running and the control plane is restarted
(`test-scenarios.md:268-276`; E09 README `:39-43`). Unlike E08's fresh-install
case, E09 never states how the correctly hooked, already-populated shared chain
is transformed so the real restart-site append fails, whether the failure
subcase runs without a sibling, or how that fixture is restored. A future runner
can therefore fail at fixture setup, destroy sibling evidence, or use a test-only
error seam while still matching the prose “kernel environment rejects.”

**Required remediation:** give the failed-reinstall subcase its own exact command
sequence and kernel baseline/fixture/restoration contract. If it reuses the
hookless-chain technique, specify when the prior table/rules are snapshotted and
removed, how Platform Reclamation and same-id re-drive are still observed, how
the real production append/errno is identified, whether siblings are excluded
from this destructive subcase, and how exact nft/FIB state is restored after
product cleanup.

#### AD-Q7-20 — two passing completeness rows cite the wrong rationale

**Severity:** LOW  
**Dimensions:** mechanical audit precision  
**Status:** Open

`test-scenarios.md:537` says slot properties cover “one and many allocations,”
although they derive from individual slot values; C3 passes through the actual
collection-shaped rule/sequence cases. Lines 540-541 treat mesh/reclamation
branches as mode flags even though this feature introduces no independent flag
input; C5a/C5b pass as N/A, not via flag orthogonality.

**Required remediation:** cite the D7/stop rule collections for C3 and mark
C5a/C5b explicitly N/A with the no-mode-flag rationale used by this review.

### Correctness gates

| Gate | Verdict | Notes |
|---|---|---|
| Prior Given/When/Then defects | **PARTIAL** | S-GTI-08b and boundary rejection are fixed; S-GTI-07/-08a still combine deploy and describe. |
| Production-reachable pre-READY sample | PASS | Custom-rootfs resolver failure remains a real deploy input and production write. |
| Private component/port separation | PASS | S-GTI-08a and `C-GTI-08-RECONCILE` retain separate universes. |
| Immutable-baseline classification | PASS with AD-Q7-08 addition required | All declared rows are honestly `NOT_EXECUTED`; the one newly required duplicate-create obligation needs a row. |
| Total Job classifier | PASS | Exact domain, mapping, declaration, location, and missing-base classification are pinned. |
| Stakeholder walking skeleton | PASS | S-GTI-01 contains no D7/C3/VMM test machinery. |
| Canonical completeness | **PARTIAL** | 14/15 is COMPLETE by threshold, but C4a and the exact 15/15 claim remain wrong. |
| Exactly one walking-skeleton tag | PASS | One actual tag and both summaries use exact spelling. |
| Exact Contract Shapes | PASS for declared obligations | 15/15 Gherkin tags and all named supporting obligations have assigned shapes; pure properties pin the exact rustdoc line. |
| Upstream D6/native-metal alignment | PASS | Effective DESIGN/ADR/brief/testing sources and DISTILL agree. |
| Pre-sync host lease | PASS at specification level | One acknowledged remote ownership epoch spans sync through final probes. |
| D7 oracle | PASS at specification level | Exact, non-vacuous, bounded, loss/mutation conservative. |
| Cleanup/stop oracle | PASS at specification level | Full residue sets and exact target-filtered sibling sequence are pinned. |
| EDD stubs | **PARTIAL** | E07 and E08 are non-vacuous; E09's failed-reinstall forcing/restoration contract is incomplete. |

### Verification record

Static verification established:

- target first parent is `85550e4a267cbd53ac266fa54f4d8cda164910af`;
- target scope is three feature/DISTILL documents, E07/E08/E09 README+runner
  pairs, and `verification/expectations/INDEX.md`;
- 15 Gherkin scenarios have 15 exact Contract Shape tags and one exact
  `@walking_skeleton` tag;
- 43 baseline rows record `NOT_EXECUTED`; the newly named classifier, boundary,
  closed-error, illegal-state, mutation, and D7 obligations are distinguished;
- E07/E08/E09 runners are executable pending stubs and fabricate no evidence;
- `git diff --check 85550e4a..ed332f89` passes;
- effective D6/native-KVM sources are aligned and DESIGN review iteration 6 is
  APPROVED;
- only this native Markdown review artifact was written by this reviewer.

### Iteration-3 verdict and remediation disposition

**REJECTED_PENDING_REVISIONS.** All iteration-2 High findings are closed, and
the canonical audit has improved from 10/15 to 14/15. The remaining open set is
zero blockers, two High findings, two Medium findings, and one Low finding:
one-action GWT violations, a newly leaked test fixture in stakeholder Gherkin,
one unmapped C4a duplicate-create contract, and an underspecified E09
failed-reinstall fixture/restoration path.

Approval requires zero blocker, critical, High, or Medium findings. The same
reviewer must re-review the next remediation; the Low rationale cleanup may ride
that pass but does not independently block approval.

## Iteration 4 — 2026-08-29

**Review ID:** `accept_rev_2026-08-29_q7_558589a7_i4`  
**Reviewer:** same `nw-acceptance-designer-reviewer` role, isolated review  
**Reviewed commit:** `558589a7a3ee14ebea0b8cdb6496b65a5830f777`  
**Compared with:** first parent `ed332f8972c6285fb067d995d73f51ee63a5ff01`  
**Model/reasoning:** inherited GPT 5.6 Luna / maximum  
**Verdict:** **APPROVED**

### Scope and evidence

Iteration 4 re-evaluated AD-Q7-08 and AD-Q7-17 through AD-Q7-20 against the
complete target commit and its immutable parent. It also rescanned the fifteen
stakeholder scenarios, all named supporting properties/component examples, the
immutable-baseline classification, feature-delta DISTILL handoff, E07/E08/E09,
the effective approved DESIGN contract, and the production nft call sequence.

Source was read only. Pre-existing dirty source and DELIVER work was excluded
and not modified. No test suite or EDD runner was executed: the target is a
specification/pending-stub change, every current baseline result remains
`NOT_EXECUTED`, and E07/E08/E09 remain executable pending stubs rather than
narrated evidence.

Mechanical verification established:

- target first parent is `ed332f8972c6285fb067d995d73f51ee63a5ff01`;
- target scope is the same three feature/DISTILL documents, E07/E08/E09
  README+runner pairs, and `verification/expectations/INDEX.md`;
- `git diff --check ed332f89..558589a7` passes;
- there are fifteen Gherkin scenarios, fifteen exact Contract Shape tags, and
  exactly one actual `@walking_skeleton` tag;
- E07/E08/E09 runner modes remain `100755`, and their bodies still stop as
  pending stubs without fabricating evidence;
- `NET_SLOT_MAX` is the real production constant at the reviewed immutable
  commit; no stale `MAX_NET_SLOT` reference remains in the effective DISTILL
  package;
- Linux's primary source makes the wrong-hook fixture deterministic:
  [`nft_tproxy_validate`](https://raw.githubusercontent.com/torvalds/linux/master/net/netfilter/nft_tproxy.c)
  permits only `NF_INET_PRE_ROUTING`, while
  [`nft_chain_validate_hooks`](https://raw.githubusercontent.com/torvalds/linux/master/net/netfilter/nf_tables_api.c)
  returns `-EOPNOTSUPP` for a base chain whose hook is outside that mask. The
  E08/E09 appliance-kernel preflight additionally proves the actual pinned
  runtime kernel before relying on that errno.

### Prior-finding dispositions

| Prior finding | Iteration-4 disposition | Evidence |
|---|---|---|
| AD-Q7-08 — C4a counted as covered / false `15/15` | **CLOSED AS AN HONEST DOCUMENTED GAP** | The mutation inventory now calls the grouped attempt-owned creation obligation `UNMAPPED C4a`, gives it no executable identity or shape, adds an immutable-baseline `AT_GAP_IN_DELIVERY_SCOPE` row, scores C4a FAIL, and publishes **14/15** in both effective summaries (`test-scenarios.md:484-505,542-568`; `red-classification.md:72`; `feature-delta.md:959,975-997`). The canonical gate is still COMPLETE at ≥13; it does not require pretending the fifteenth item exists. |
| AD-Q7-17 — deploy and describe in one When | **CLOSED** | S-GTI-07 and S-GTI-08a each have deploy as the single When. Bounded describe is now a Then observation (`test-scenarios.md:293-311`). S-GTI-12b's repeated stop remains the intentional idempotency stimulus. |
| AD-Q7-18 — E08 nft fixture leaked into S-GTI-05 | **CLOSED** | S-GTI-05's Given is behavioral: the qualified host rejects fresh-guard installation. The table, INPUT-hook base chain, sentinel, production append, errno, baseline, and restoration mechanics live in E08/supporting design (`test-scenarios.md:265-272`; E08 README `:55-92`). |
| AD-Q7-19 — failed-reinstall forcing path underspecified | **CLOSED** | E09 now gives the failure its own fresh sibling-free durable directory and command sequence, snapshots the Running/same-id/boot-epoch/nft/FIB baseline, installs restoration traps before fixture mutation, performs an unclean stop without another deploy, forces the real restart-arm `append-egress`/`append-rule -EOPNOTSUPP`, requires the exact `RestartAllocation` trace, forbids injection/EXEC/frames, proves product cleanup, and restores to the precomputed target-filtered baseline even on assertion/signal failure (E09 README `:31-126`). |
| AD-Q7-20 — incorrect C3/C5 audit rationales | **CLOSED** | C3 now cites zero/one/duplicate D7 targets and empty/single/multiple allocation-rule sequences. C5a/C5b explicitly pass as N/A because no independent user mode flags exist (`test-scenarios.md:548-564`; `feature-delta.md:977-995`). |

Iteration-1 AD-Q7-01 through AD-Q7-07 and AD-Q7-09, plus iteration-2
AD-Q7-10 through AD-Q7-16, remain closed. No regression was found in their
original defect classes.

### Iteration-4 scorecard

| Dimension | Score / 10 | Result | Rationale |
|---|---:|---|---|
| Happy-path bias | 9 | Excellent | Fresh/restart install failure, pre-READY failures, status-78 complement, interruption, resource degradation, inverse stop, concurrency, and cleanup are distinct. |
| GWT format | 9 | Excellent | Each ordinary scenario has one semantic stimulus. Describe is an observation; deliberate stop replay is one idempotency stimulus. |
| Business language | 9 | Excellent | S-GTI-05 is behavioral and the sole walking skeleton remains stakeholder-confirmable. Fixture/kernel mechanics are confined to focused supporting contracts. |
| Coverage completeness | 9 | Complete | Mechanical score is honestly 14/15, above the canonical ≥13 threshold, with C4a visibly documented rather than waived. |
| Walking-skeleton user-centricity | 10 | Excellent | S-GTI-01 demonstrates named-peer success, byte-distinct reply, platform protection, and no cleartext without test machinery. |
| Priority validation | 9 | Excellent | The package remains centered on born-captured security, truthful boot/restart behavior, and exact cleanup. |
| Observable behavior | 9 | Excellent | Metal scenarios use port-visible outcomes; private lifecycle vectors/views remain in the mapped component example. |
| Traceability coverage | 9 | Excellent | Effective D6/native-metal sources and DISTILL agree; the one stale product-journey sentence remains explicitly recorded and routed. |
| Walking-skeleton boundary proof | 9 | Excellent | Real C3 supplies identity after deploy, capture arms before real VMM spawn, and D7 remains conservative and non-vacuous. |

### Mandate verdicts

| Mandate | Verdict | Evidence |
|---|---|---|
| CM-A — hexagonal boundary | PASS | Walking/metal journeys use built deploy, describe, and Job-stop entry points; implementation-sensitive assertions are mapped to supporting source/component contracts. |
| CM-B — business language | PASS | S-GTI-05 no longer names its test fixture; the walking skeleton contains only stakeholder-visible outcomes. |
| CM-C — complete user journey | PASS | S-GTI-07/-08a now use deploy as one action followed by bounded operator observations. |
| CM-D — pure extraction | PASS | Slot boundary, total error/classifier closure, illegal transitions, reclamation selection, and D7 decoder/oracle logic are mapped to pure properties. |
| CM-F — PBT layer | PASS | Generative obligations remain source-local/layer 1; native real-I/O examples remain explicitly enumerated. |
| CM-H — named layer-3 sad paths | PASS | Fresh guard rejection, same-id failed reinstall, resolver failure, interruption, capture/netlink loss, and cleanup are separately named. |

### Canonical AT-completeness recomputation

The mechanically recomputed result is **14/15 — COMPLETE**. This is a
specification score, not an implementation or execution claim.

| Item | Verdict | Iteration-4 evidence |
|---|---|---|
| C1a | PASS | Slot 0 and 1 are explicit valid representatives. |
| C1b | PASS | Max-1/max are valid; `P-GTI-SLOT-BOUNDARY` applies `NetSlot::new(NET_SLOT_MAX + 1)` and requires typed pre-derivation rejection. |
| C2a | PASS | The full legal lifecycle is explicit. |
| C2b | PASS | `P-GTI-ILLEGAL-01` through `-07` cover each modeled state. |
| C3 | PASS | D7 covers zero/one/duplicate targets; stop/E07/E09 cover empty/single/multiple rule sequences. |
| C4a | **FAIL — documented** | Grouped attempt-owned rootfs/run-dir/listener/VMM/capture creation has no correct-non-idempotency AT and is explicitly unmapped. |
| C4b | PASS | S-GTI-12b stops twice when no guard exists. |
| C5a | PASS (N/A) | No independent user mode flag exists. |
| C5b | PASS (N/A) | Flag orthogonality is therefore inapplicable. |
| C6a | PASS | Malformed address/prefix/gateway/DNS cases are distinct. |
| C6b | PASS | Every sanctioned pre-READY stage and the install/diagnostic/D7 failure families are mapped. |
| C6c | PASS | Pre-READY and D7 decoder/oracle error sets are closed by named properties. |
| C7a | PASS | Resolver/read/loss/timeout paths fail conservatively. |
| C7b | PASS | M-GTI-INTERRUPT-BOOT terminates the real VMM between capture-ready and READY. |
| C7c | PASS | M-GTI-CONCURRENT-DEPLOY drives two built deploys in parallel inside one test-run lease. |

### Regression and non-vacuity gates

| Gate | Verdict | Evidence |
|---|---|---|
| Immutable-baseline honesty | PASS | All 43 existing baseline rows remain `NOT_EXECUTED`; the C4a gap gains its own row; no historical RED/GREEN is promoted. |
| One walking skeleton / stakeholder value | PASS | Exactly one real tag; S-GTI-01 remains free of C3/D7/VMM fixture mechanics. |
| Total Job exit classifier | PASS | Every `Option<i32>` plus arbitrary signal, exact mapping/location/declaration, and missing-baseline status remain pinned. |
| Contract Shape coverage | PASS | Fifteen scenarios have fifteen exact tags; every named supporting obligation has an assigned shape or, for the honest C4a gap, explicitly says unassigned. |
| D6 lifecycle alignment | PASS | Platform reclamation with standing intent remains the sole same-id route; natural Job exit is final and workload restart creates a fresh allocation. |
| Native substrate alignment | PASS | Native non-virtualized x86_64 KVM only; nested/Lima runtime evidence remains forbidden. |
| Universal pre-mutation lease prerequisite | PASS | One canonical lock covers Run, Sync, and every supported direct-bootstrap writer before any shared-tree mutation. Run keeps the descriptor through final probes; raw writers are prohibited and runtime evidence is explicitly invalid until this boundary lands (`test-scenarios.md:160-197`; E07 `:39-50`; E08 `:24-32`; E09 `:20-29`). |
| Fresh guard failure forcing | PASS | The production-named INPUT-hook base chain makes the unchanged real TPROXY append reach typed `append-egress` / `append-rule -EOPNOTSUPP`; preflight, clean baseline, product cleanup, and assertion-safe restoration are exact. |
| Failed reinstall forcing | PASS | Separate sibling-free E09 journey proves unchanged durable intent/data, no second deploy, Platform Reclamation, same id, restart action, the real rejected append, and target-filtered restoration. |
| D7 oracle | PASS | Complete strict netlink framing, generation/notification guard, exact program identity, lossless capture, and bidirectional checked equality remain non-vacuous. |
| Stop/cleanup oracle | PASS | Exact target-handle filtering preserves sibling values and relative order; complete residue sets and finite deadlines remain pinned. |
| EDD honesty | PASS at specification level | E07/E08/E09 contracts are implementable, conservative, and pending; the runners claim no execution evidence. |

### Findings

No blocker, critical, High, Medium, or Low findings remain in the reviewed
DISTILL acceptance package.

The explicit C4a omission is not hidden or counted as passing: it is the one
failed item in the mechanically complete 14/15 score and remains a transparent
DELIVER-scope opportunity. Under the canonical deterministic threshold, that
documented omission does not block DISTILL approval.

### Iteration-4 verdict and remediation disposition

**APPROVED.** AD-Q7-08 and AD-Q7-17 through AD-Q7-20 are closed. All earlier
closed findings remain closed. The reviewed package has zero blocker, critical,
High, Medium, or Low findings; its one known C4a checklist miss is honestly
represented in the 14/15 COMPLETE score and immutable handoff rather than being
misclassified as coverage.
