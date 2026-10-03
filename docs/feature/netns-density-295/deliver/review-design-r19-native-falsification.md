# DESIGN Review — R19 native-falsification replacement

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Review | `arch_rev_20261003_netns295_r19_native_falsification` |
| Reviewer | Fresh isolated `nw-solution-architect-reviewer` |
| Model | GPT 6.1 Sol (`gpt-6.1-sol`), extra-high (`xhigh`) reasoning; explicit session override of the definition's legacy Haiku default |
| Iteration | 1 |
| Date | 2026-10-03 |
| Source snapshot | `020255395328b28a4236a65f3610d3f63951d01c`, with the existing dirty native fixtures, execution log, roadmap, AGENTS, and pending DESIGN annotations preserved |
| Bounded proposal verdict | **APPROVED — ordinary-flow R19 conditional withdrawal proposal only** |
| Overall DESIGN / roadmap execution | **PENDING / NON-EXECUTABLE** |
| Findings | 0 critical, 0 high, 0 medium, 1 low documentary note |

The review applies `nw-sar-critique-dimensions` and the bounded checks from `nw-roadmap-review-checks`. Repository correctness, native-falsification, exact-interface, contract-weakening, and evidence-boundary rules govern over generic review templates. This is native Markdown review feedback, not a YAML response stored in a Markdown file.

## Scope and authority

Reviewed [the concrete proposal](../../../../.context/r19-replacement-design.md), the feature delta's current validation and dependency register, and affected clauses in the architecture brief and ADRs 0088, 0089, 0124, 0125, 0139, and 0140. Reviewed the current rule encoder/canonical identity, real serve composition, retained mTLS listener owner, current runtime quiescence, exact-port repair, and killed-mode lifetime path only as needed to verify the proposal's production facts. This is not a new review of unrelated accepted decisions or all roadmap steps.

Primary native evidence is [the completed report](../../../../.context/distill-native-08-01.md), particularly its ordinary-flow follow-up at line 145 and platform-stop closeout at line 243, and the existing receipts:

- `distill-native-08-01-fresh-flow-premise-proof.log`, nextest `5e43658a-438a-40b8-ae5e-3a27a1875bfa`: final ordinary-flow branch evidence.
- `distill-native-08-01-r19-premise-proof.log`, nextest `9d7492d1-e672-4bf3-b7ee-91e5c85cfa00`: earlier independent reproduction of the ordinary R19 passes and R18 exposure.
- `distill-native-08-01-time-wait-ordered-2.log`, nextest `f52ce763-3a0a-4a3d-b796-cebb6a7bd57c`: completed controls and a direct-peer variant; not the accepted named-Service E14(e) disposition.

The reviewer ran no native command, experiment, security test, or new regression. No blocked task was retried, rephrased, or recreated. Source/test/design/roadmap/log files, staging, and commits are outside this review's write ownership; only this artifact was written.

### Prior acceptance verified from committed artifacts

Read commit `b5ef001b` directly. Its ADR-0140 Status records user acceptance on 2026-09-24, independent rounds 4/5 and verification `arch_rev_20260924_netns295_r5_verify`, and the exact condition:

> If native evidence shows that outbound guest TCP to an absent transparent listener is already dropped under the current order, this decision is withdrawn.

The same committed feature delta records:

> D-295-R18 and D-295-R19 are accepted as conditional decisions: each stands only if its native RED (E14) reproduces, and is withdrawn otherwise, exactly as its ADR states.

Its *Conditional parts, and the shape if a RED does not reproduce* table states the shape without R19:

> Stays mark → TPROXY → accept, which E14's listener RED showed already fails closed.

Immediately below, it distinguishes the two ordinary outbound listener-absent cases from E14(e), which is judged separately. The current feature delta retains that authority at its Acceptance record (line 274), conditional acceptance (line 314), and R18/R19 contract section. These are verified quotations from the accepted DESIGN record, not invented quotations of an original chat transcript. No separate user approval to accept a security residual was found or inferred.

## Evidence and production-path verification

| Case | Completed evidence | Review conclusion |
|---|---|---|
| E14(c): external leg-F destruction | Final receipt lines 57–69: ifindex 59503, 44 frames, 36 actual post-fault gateway/external SYNs during witnessed TAP-up exposure, Intercept delta 42, program/source membership and fwmark/local route present before/after, unchanged bridge default-drop counter, no correlated SYN-ACK or wildcard accept. Native report lines 218–235 records complete lossless accounting. Actual TAP down was approximately 180 ms after the fault. | The ordinary exposure does not reproduce on the tested current composition. The owner's later quiescence is measured, not hidden; no zero result is inferred from empty ingress or an already-down TAP. This does not prove the future non-quiescing listener classification. |
| E14(d): killed serve owner | Final receipt lines 40–53: ifindex 59500, 3993 frames, 3942 post-kill SYNs, Intercept delta 3982, unchanged owned program/members and route objects before/immediately after/after, no correlated reply or wildcard accept. Same TAP remains live through the complete window. | Independently supports the ordinary conditional withdrawal without relying on runtime quiescence after owner death. |
| E14(a)/(b): IP-program loss | Final receipt lines 24–36: peer ifindex 59496, sender 59497, 1978 peer-target and 3956 host-local SYNs, 1972 forwarded SYNs, matched gateway/actual-host replies, Intercept delta 13916, zero peer capture drops and unchanged bridge drop counter. The zero-forwarded assertion fails after ingress prerequisites pass. | Genuine R18 premise reproduction. The R18 guard decision, field, error, effects, and zero outcomes remain required; no R18 withdrawal follows from R19. |
| E14(e): original named-Service guest TIME_WAIT journey | Closeout lines 253–270 and receipt `f52ce763…` distinguish a completed direct-peer variant from the original named-Service destination. Host controls passed; the direct-peer variant recorded `reopened=false` and zero wildcard accepts. The original named-Service correction did not apply or run. | **Unresolved required gate.** Neither host controls, earlier failed prerequisites, nor the different direct-peer tuple establishes a positive or negative result for the original accepted guest journey. |

The final ordinary receipts are consistent with the existing fixture's enforced controls. The closed-listener body preserves the exact-ifindex capture through the real down notification, selects fault-period guest requests during witnessed up intervals, correlates replies to those requests, requires both destination kinds, checks the Intercept delta and bridge counter, and compares exact program/member state (`intercept_mark_fail_closed.rs:1707`, `:1724`, `:1762`, `:1781`, `:1810`, `:1827`). The killed-mode body retains the same controls and additionally refuses a down/removed TAP (`:1878`, `:1894`, `:1904`, `:1930`, `:1940`, `:1966`, `:1986`). Fresh source-port behavior and request/reply correlation are declared at `:180`, `:818`, and `:832`. The no-SYN-ACK oracle is independent of whether a bare SYN completes a wildcard handshake; the zero-accept observation is retained as its additional accepted check.

### Current owner, state, and ordering

- `serve::run_with_kek` calls the actual `run_inner`/`run_server` path; its `ServerConfig` composes `HostMtlsIntercept` and the real guest DNS factory (`commands/serve.rs:193`, `:315`, `:322`). The server starts `state.mtls_worker.start_shared_owner()` before serving (`overdrive-control-plane/src/lib.rs:7594`).
- `start_shared_owner_inner` binds F/C through `MtlsIntercept`, converges the program, obtains the expected identity, and creates the retained listener task owner (`mtls_intercept_worker.rs:2136`, `:2144`, `:2158`, `:2159`, `:2179`). This proposal creates no alternate socket owner or publication boundary.
- The private `tproxy_and_mark_and_accept` currently encodes mark assignment, target address/port loads, TPROXY, then accept (`overdrive-netlink/src/nft.rs:679`). Shared rule 1 selects `0x295a`-marked registered-source TCP; shared rule 3 selects the registered destination tuple, and both call that helper (`:753`, `:767`).
- `SharedIpInterceptIdentity::for_listener_ports` and `from_normalized_parts` use the same exact `SharedProgram` encoder/validator (`nft.rs:2964`, `:2970`, `:3181`, `:3433`). Retaining the present order does not require accepting two canonical identities.
- `HostMtlsIntercept::converge_shared` ensures the existing fwmark rule and local route, then records targets/program (`mtls_intercept_port.rs:1146`, `:1166`). The native witness checks these objects separately rather than presuming them from table presence.
- The live supervisor currently invokes `run_mtls_owner`, which audits every second and awaits `quiesce_managed_taps` on detected mTLS failure before 250-ms recovery attempts (`lib.rs:7872`, `:1581`, `:1584`, `:1596`, `:1602`). The later component-specific `run_shared_network_supervisor` is a RED scaffold (`:1486`). The proposal correctly distinguishes current evidence from that future contract. The live-capability early return at `mtls_intercept_worker.rs:2250` is an observed existing audit limit, not evidence of an additional proposed defect.
- The accepted killed-mode test lifetime path joins userspace owners and releases the last worker without host mutation authority (`ServerHandle::kill_for_test`, `lib.rs:6156`, `:6197`, `:6198`, `:6199`). It is a test-gated abrupt-lifetime seam, not a newly shipped recovery interface or proof of every real crash schedule. Native program/member/route/TAP witnesses establish the relevant measured residue. No recovery owner is presumed after actual process death.
- Exact-port repair retains the recorded address and refuses a mismatching rebind (`mtls_intercept_worker.rs:2269`, `:2293`, `:2297`). No detached completion or new shutdown authority is proposed.

No new production defect is asserted in this review. No theoretical cancellation, imagined interleaving, or test-only state is promoted into a finding or architecture requirement. New control-plane ordering claims would still require the repository's independent failing seeded Sim evidence and real production reachability proof.

## Decision and normative-contract review

| Proposal decision | Disposition | Evidence and scope |
|---|---|---|
| R19-N1: invoke the accepted ordinary conditional withdrawal | PASS | Both ordinary cases satisfy the specified fail-closed oracles under current order. The committed conditional branch expressly distinguishes E14(e). |
| R19-N2: retain exact mark → TPROXY → accept tails and canonical identity | PASS | Proposal lines 151–178 retain one exact current order, the rule predicates, target ports, other six IP rules, sets, userdata, counts, ownership, and strict foreign/noncanonical refusal. No alternate-order recognizer is introduced. |
| R19-N3: preserve R18-B | PASS | Proposal lines 180–201 match the accepted guard identity at feature-delta lines 4100–4135: separate `ip overdrive-mtls-guard`, one filter prerouting chain at priority 0 with policy accept, exactly the marked-TCP drop rule and userdata, no counter. The two existing doc-hidden signatures also match the source scaffolds at `nft.rs:3087` and `:3099`. |
| R19-N4: leave LegF/LegC classification pending E14(e) | PASS AS A HOLD | Ordinary results do not choose the final listener quiescence behavior or accept killed-mode exposure. The separate required guest journey remains unresolved. |
| R19-N5: preserve independent invariants | PASS | No change to zero pre-event frames, no-cleartext escape, single-owned-component catch-or-drop, ownership/generation, Clock bounds, R14 kill partition, R15 whole-audit/restore, held-lease cap, or typed no-panic outcome is proposed. |

### Exact interface and effect boundaries

The proposal preserves the feature delta's eight `MtlsIntercept` methods and their existing shapes (`feature-delta.md:3298`), method-free `InterceptGuard`, three-set `InterceptMembers`, and `{ program, policy_route, intercept_mark_guard, members }` `InterceptState`. It adds no public type, trait method, enum variant, parameter, config/CLI field, persisted format, port, owner, or recovery protocol. Its exact wire change is withdrawal of the planned reordering; current encoding remains the sole canonical shape. The R18 effects are already accepted signatures and RED scaffolds, not invented surface.

The reuse table keeps the encoder/identity pure and return-only. Existing convergence, listener ownership, and shared-network operations retain their accepted bounded effect universes, full read-back, generation brackets, foreign complements, and lifecycle boundaries. Read-only observation remains non-mutating. No new unbounded-preservation procedure or new capability boundary is introduced. Private implementation and test-support choices remain outside the new DESIGN interface contract; this review requires no additional helper or Sim seam.

### Semantic weakening gate

Inspected the actual tracked DESIGN diff. It consists of additive pending-validation annotations in the eight DESIGN paths, plus the root's pre-existing relabeling of the old feature-delta status as historical. No operative zero/never/must/exactly/forbidden/required outcome has been removed or relaxed.

The proposed normative amendment changes the conditional R19 mechanism to its already-approved no-change branch. Removal of the **R19-induced** old-order conflict/one-time cleanup consequence follows because no R19 schema change occurs; genuine malformed, foreign, partial, and noncanonical objects still refuse startup. The proposal does not waive startup refusal or broaden accepted identity. Its pending LegF/LegC annotation does not replace the existing `never` clause with weaker operative behavior. A later listener replacement or accepted residual must undergo its own semantic diff and any required exact user approval; this review supplies neither.

## Dependency invalidation and alternatives

The feature delta's register at line 40 covers the affected order amendment, ordinary outbound exposure assertion, observe/converge/boot identity value, listener-only non-quiescence rationale, stale-order cleanup consequence, reuse and threat/evidence projections, required downstream changes, environment/step/scenario projections, and summary/C4 references. The annotations in ADR-0088/0089 apply only to their #295 listener-loss correction; they do not claim native evidence about the historical per-workload netns topology. ADR-0124 preserves DNS and independent EXEC/Clock/kill behavior. ADR-0125 preserves cardinality, ownership, and R10/R12/R15. ADR-0139 invalidates only its R19 cross-reference while retaining the independently reproduced R18 decision. The brief maps its summaries and index. These are correctly scoped pending dependencies, not silent continuation of the falsified R19 rationale.

The rejected mark-before-TPROXY alternative is genuinely reopened: ADR-0140's rejection rests on ordinary guest traffic reaching wildcard listeners, which the completed required cases do not reproduce. Listener quiescence is reopened but not selected; the independent R18 guard is not claimed to close a TIME_WAIT path. Keeping the old reordering solely because it was once accepted has no execution authority. No new persistence, replay, scheduler, hydration, cancellation, or ownership architecture is introduced to make a hypothesis consistent.

C4 relationships and components are unchanged. The existing L2 order description (`c4-diagrams.md:1651`) and listener-only supervisor prose (`:2070`) are explicitly pending through the central register. Their wording must be reconciled when recording the replacement; a duplicate diagram or fabricated component is not required.

## Findings and remediation dispositions

### L1 — Evidence summary predates the completed TIME_WAIT documentary closeout

**Severity:** Low. **Location:** proposal lines 43 and 283–302; feature-delta initial status at line 15. **Status:** Open documentary note; not a blocker to the bounded ordinary R19 withdrawal.

The proposal describes the earlier guest TIME_WAIT prerequisite failures and correctly says the required gate is unresolved. The subsequently appended native closeout, however, now records completed host controls and one completed direct-peer variant (`distill-native-08-01.md:253–270`; receipt `f52ce763…`). That variant is negative only for its concrete direct-peer tuple. The original named-Service destination did not run and remains unresolved. A blanket summary of the earlier setup failure is now incomplete evidence provenance, although its gate disposition remains correct.

**Disposition:** Incorporate that precise distinction during documentary replacement recording: controls completed; direct-peer variant observed; original named-Service E14(e) unresolved. Do not infer the original door is closed, mark the mandatory gate complete, alter its required journey, or retry the blocked experiment. This is documentation reconciliation, not a request for source, test, security, or architecture changes.

### Mandatory pending gates — existing requirements, not new defect findings

The missing E14(e) result and final listener classification are pre-existing execution blockers deliberately retained by the proposal. They are not evidence that the bounded ordinary withdrawal is incorrect. No unproven hypothesis has been turned into a mandatory new mechanism.

## Architecture dimensions and bounded roadmap checks

| Architecture dimension | Result |
|---|---|
| Bias / scope | PASS: the proposal removes an unvalidated mechanism through an accepted conditional branch and adds no technology or subsystem. |
| Decision quality | PASS: exact context, committed authority, reproduced contrary evidence, reopened existing alternatives, consequences, and normative diff are recorded. |
| Completeness | PASS for the ordinary replacement; full listener classification remains expressly incomplete pending E14(e). |
| Feasibility / testability | PASS for retention of existing order/API. Mandatory native evidence availability is an unresolved limitation, not a simulated substitute. |
| Priority / simplicity | PASS: physical evidence addresses the specific R19 premise; the R18 exposure remains independent. No invented alternatives are needed for a template count. |

| Roadmap check | Bounded result |
|---|---|
| External validity | Existing completed ordinary bodies drive `serve::run_with_kek` and deployment with real guests. This does not approve the incomplete E14(e) gate or full roadmap. |
| Acceptance-criterion coupling | No new criteria or implementation structure is authored. Existing exact API/wire contracts are intentional architecture contracts. |
| Decomposition | No new step is proposed; unrelated approved steps are not re-audited. |
| Implementation code | No implementation algorithm is inserted into the roadmap. Exact pre-existing effect signatures in the DESIGN preserve the interface contract. |
| Concision / precision | Ordinary withdrawal, independent R18, and pending TIME_WAIT/LegF/LegC scopes are distinct. L1 records the one stale evidence summary. |
| Test boundaries | Existing in-process native integration and black-box expectation boundaries are preserved. No expectation runner, production binary spawning test, or helper implementation is added. |

The actual roadmap validation is `pending`, with `approved_at: null` (`roadmap.json:1726`). Step 08-01 still describes the old standing reordering branch and remains non-executable (`:1248–1284`). This is an upstream reconciliation requirement before later validation, not permission for the reviewer or crafter to edit it or treat this proposal approval as a replacement roadmap approval.

## Platform limitation and exact resumption requirements

The platform rejected the separate TIME_WAIT follow-up with **“possible cybersecurity risk”**; the root authorized documentary closeout only. The completed report preserves that boundary at lines 243–280. The limitation does not waive the mandatory native gate, authorize a workaround, or make an outcome reduction acceptable. This review does not select any route for repeating the blocked work.

Before any future DELIVER resumption, all of the following remain required:

1. Record the reviewed exact ordinary R19 replacement in the authoritative DESIGN, marking contradicted order/exposure/consequence text historical or withdrawn and reconciling the mapped ADR, brief, C4, DISTILL, environment, and roadmap projections. Preserve R18's accepted exact shape and all independent invariants; incorporate L1's evidence distinction.
2. Obtain admissible completed evidence for the **original accepted named-Service guest E14(e) journey**, including its actual resolved original-destination tuple and all existing tuple/state/order/control/capture prerequisites. The next evidentiary requirement is that exact missing disposition; host controls or the direct-peer result do not satisfy it. No such additional work is authorized by this review or the current read-only closeout.
3. Record and independently review the exact LegF/LegC listener-loss classification on that evidence. Confirmed runtime quiescence cannot retroactively prove zero escape while a faulted TAP was up or close killed-mode residue after process death.
4. If a valid result exposes a forbidden single-loss outcome, record an exact invariant-preserving replacement DESIGN. Any proposed acceptance of a measured residual instead requires the user's explicit approval for that exact weakening and the normative diff gate. No such approval currently exists.
5. Complete upstream reconciliation and required roadmap review/approval. Keep DESIGN validation and roadmap `validation.status` pending until the mandatory evidence and classification gates are complete. The original 08-01 crafter remains before GREEN; no new DES phase, GREEN, or COMMIT is authorized here.

## Iteration 1 verdict

**APPROVED for the bounded ordinary-flow R19 conditional withdrawal proposal:** retain the exact current mark → TPROXY → accept order and canonical identity under the verified pre-approved branch, preserve R18-B, and hold the independent listener-loss classification pending. No blocking defect in that bounded proposal is proven.

**Full replacement DESIGN and DELIVER execution remain PENDING / NON-EXECUTABLE.** The original named-Service E14(e) result, exact reviewed listener classification, any necessary user decision, authoritative artifact reconciliation, and roadmap revalidation remain outstanding. The external platform block is an evidence limitation, not an optional or waived gate. Approval of this review's bounded proposal cannot be interpreted as authorization to execute 08-01 or any later DELIVER step.
