# DELIVER Review — Step 08-01

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `08-01` — intercept-mark R18 guard-table and native E14 fail-closed evidence |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning (explicit user override) |
| Iteration | 1 |
| Commits reviewed | `e98a5382b7cd7d2efd472f83e0a3d6a9ba65c2fe`, `17942ef5c24416534338b95efe8e4d723a237a44`, `4795906a61116ffb5732905ec35df5f72a2c47e4` |
| Verdict | **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the three step commits against approved roadmap step 08-01, the accepted R18 contract in `feature-delta.md`, ADR-0139, ADR-0140, and the R19 finalization record. The review covered the exact guard observations and effects, retained mark → TPROXY → accept order, host and Sim state production, production listener recovery, native E14 scenarios S-ND295-62/63/64, test-support ownership, Contract Shape declarations, TDD evidence, and reported gates. Later 08-02/08-03 boot and audit behavior and 09-01 supervisor work were not treated as 08-01 requirements.

The accepted 08-01 scope is `roadmap.json:1248-1291`; the state-production boundary is `feature-delta.md:3377-3452`; the worker’s typed error mapping is `:3902-3957`; and the R18-B guard identity and effect contract is `:4178-4210`. E14 and its positive controls are specified at `feature-delta.md:6327`; the active S-ND295-62/63/64 stimulus and oracles are in `distill/test-scenarios.md:1255-1318`.

## Contract Shape Compliance

The public and cross-crate API shape matches the accepted design. `InterceptState.intercept_mark_guard` and `InterceptError::InterceptMarkGuardAbsent` remain unchanged. The two implemented doc-hidden nft effects have exactly the accepted signatures: `observe_intercept_mark_guard() -> Result<bool, NetlinkError>` and `converge_intercept_mark_guard() -> Result<(), NetlinkError>` (`crates/overdrive-netlink/src/nft.rs:3078-3097`). No port method, public type, enum variant, parameter, error field, wire format, or lifecycle contract was added or reshaped.

The private nft codec recognizes only IPv4 table `overdrive-mtls-guard`, its exact `prerouting` filter chain at priority 0 with accept policy, and the single mark-`0x295a` TCP-drop rule with the pinned userdata and no counter (`nft.rs:3117-3153`). It brackets the complete observation by nft generation, returns `false` only for an absent table, rejects partial or conflicting present state, creates missing components in one batch, and requires a positive read-back (`nft.rs:3155-3312`). This matches the accepted rule identity and fail-closed error behavior.

`HostMtlsIntercept::converge_shared` converges the guard through the accepted effect and maps its failure to the existing `NftRuleInstallFailed` error (`crates/overdrive-worker/src/mtls_intercept_port.rs:1141-1169`). The host `InterceptState` producer reads the guard after member convergence (`:1265-1284`); the Sim producer reports its modeled guard state (`crates/overdrive-sim/src/adapters/mtls_intercept.rs:544-548`). The guard field is not falsely removed or made optional.

The production intercept-rule construction is unchanged. E14 runs against the retained mark → TPROXY → accept tail; the R19 reorder remains withdrawn on native evidence. The test and implementation changes do not add R19-only cleanup or alter the listener-loss no-quiescence outcome.

The expanded control-plane and worker changes remain private. They consume the existing listener-task terminal event before exact-port replacement, prioritize a finished listener task over a simultaneous address-read error, avoid re-running the boot-only zero-member observation while active capabilities exist, and keep Leg-F/Leg-C/DNS loss out of managed-TAP quiescence (`crates/overdrive-control-plane/src/lib.rs:1497-1537, 1544-1755`; `crates/overdrive-worker/src/mtls_intercept_worker.rs:381-402, 2220-2331`). These changes preserve the current composed owner’s exact listener-recovery path; they add no shared supervisor or public recovery API.

## Test integrity, authorship, and coverage

The seven active native bodies are `marked_guest_tcp_is_neither_forwarded_nor_delivered_without_the_intercept_program`, `the_intercept_program_still_catches_marked_tcp_without_the_guard_table`, the two outbound listener-loss cases, `inbound_tcp_to_a_closed_listener_is_dropped`, the independent TIME_WAIT controls, and the named-Service guest door. Each carries `Outcome anchor: OUT-ND295-BORN-CAPTURED` and `CONTRACT_SHAPE: bounded-change`. The seeded source-local recovery regression `current_mtls_owner_keeps_the_active_tap_up_after_leg_c_listener_loss` has the same declarations. No new pure-function property was authored, so the exact `CONTRACT_SHAPE: pure-function.` declaration does not apply. No name matches the banned `test_.*(returns_…|exit_code|calls_…_once|status_code|http_…)` pattern.

I counted nine distinct behavior groups across the eight step-owned tests: R18 program loss; guard-only loss; ordinary leg-F listener loss; killed-mode leg-F loss; inbound leg-C loss; the TIME_WAIT negative control; the TIME_WAIT positive control; the killed-mode named-Service TIME_WAIT door; and the active-TAP Leg-C recovery branches. The budget is `2 × 9 = 18`; actual tests are 8, within budget.

The acceptance-body changes preserve the accepted scenarios and correct their stimuli. The guest probe now emits scripted SYNs, so a completed healthy application handshake cannot satisfy or disturb the wildcard-listener oracle. The inbound control uses an unregistered leased-veth source and the peer Service’s registered tuple, exercising the inbound PREROUTING rule rather than the managed-source outbound rule. The TIME_WAIT door uses the real guest-completed close, verifies the exact original named-Service tuple and true host-side TIME_WAIT state, derives newer sequence/timestamp values from the observed FIN, runs both independent controls first, and measures the full post-kill window. These changes follow the accepted scenarios and the recorded B3(c)/H3 repairs; they do not change a required zero outcome or weaken an assertion.

The E14 forwarding and reply oracles remain non-vacuous: the R18 case records live sender/peer TAPs and forwarding at fault time, captures the exact interfaces through the full window, correlates replies to post-fault SYNs, and requires complete loss-accounted capture. The ordinary and killed-mode outbound cases require exact live ingress, zero replies, no wildcard accepts, and complete capture. The inbound case has a healthy Leg-C positive control, an unregistered source, exact peer/source captures, zero forwards/replies/thief accepts, full loss accounting, and the retained live-TAP assertion. The TIME_WAIT case proves the actual FIN-derived tuple, no pre-fault reconnect, post-fault crafted requests, zero reopen, and zero wildcard accepts.

The killed-mode fixture records the exact program and member sets it owns. Cleanup verifies that same program before deleting those members, preserves an already-existing constant program and its foreign complement, and runs after VM/scope cleanup. The R18 table-loss case restores its saved identity and exact source/destination members only after its complete fault oracle, before ordinary production stop. The `guest_stack_mtls_egress.rs` changes expose the existing capture types and functions to the integration-test module; they do not change capture behavior.

No assertion was weakened, deleted, skipped, or changed to accommodate production. The pending markers were removed only for the bodies assigned to 08-01. No test spawns the built Overdrive binary as the system under test; the native cases drive the in-process `serve`/`deploy` composition and use Cloud Hypervisor as the external guest fixture.

### Unproven hypothesis — guard-only S62 response observation

The guard-only S-ND295-62 control uses the raw-SYN stimulus and asserts that the wildcard listener accepts no completed connection, while checking live ingress and that the intercept table remains present. It does not assert a zero post-fault SYN-ACK count or read the capture’s drop counter. Because a raw SYN does not complete a TCP handshake, this leaves a possible gap in proving that branch’s “intercept program still catches or drops” result. The primary R18 program-loss body independently asserts correlated zero replies and zero capture drops. I did not reproduce a false-green through the current production owner, and I did not use a mutation or add a seam. Under the repository reproduction gate this remains an unproven hypothesis, not a finding or remediation request.

## Verification

The selected native E14 selector passed all seven tests (final reported run `3366a0a7-b241-4e03-9c7d-023dc0b9252e`; the retained complete receipt `distill-08-01-cleanup-luna-native-final-after-observe-runtime-fix.log` records 7 passed / 168 skipped, nextest `67543da8-69df-42cf-8e1a-13f90d3b5178`). The named-Service E14(e) independent controls and guest door passed on metal (nextest `f03c43d5-ffcc-4bec-8bd3-9b98f90e334a`, 2 passed). The seeded current-owner regression passed all three Leg-C branches on metal (seed `0x2953300000000001`, nextest test result 1 passed; `.context/08-01-legc-seeded-sync-final.log`). The Lima worker integration selector passed 41/41 as reported.

The execution log records the final RED at `19:17:47Z`, GREEN passes through `21:31:42Z`, and COMMIT at `21:32:59Z`; earlier failed and skipped attempts remain recorded (`deliver/execution-log.json:1028-1091`). The independent native R18 premise receipt records the non-vacuous table-loss exposure and complete capture before its expected fail-closed assertion (`.context/distill-native-08-01.md:210-235`). The final required native selector passed after the private owner correction. The reported workspace `cargo check --workspace --all-targets --features integration-tests`, workspace clippy with `-D warnings`, and formatting gates passed. This reviewer did not rerun the suites or mutation testing; individual-step mutation testing is prohibited.

I ran `git diff --check` over the reviewed commit range; it passed.

## DES, commit, and workspace evidence

The implementation, execution-log, and scenario-context commits retain Marcus as Git author, each contain exactly `Co-Authored-By: Codex <codex@openai.com>`, and each carries `Step-Id: 08-01`. No Claude, Anthropic, or generated-by attribution is present. Before this review, `AGENTS.md` was the only dirty path and remains untouched. This Markdown review is the only file written by the reviewer.

## Findings and remediation dispositions

No proven in-scope implementation defect, API divergence, invariant weakening, acceptance-body weakening, or production-boundary violation was found. No remediation was required or dispatched. The guard-only S62 observation note above remains explicitly unproven and did not expand scope.

## Iteration 1 verdict

**APPROVED.** R18’s exact guard-table effects and read-back are implemented through the accepted host and Sim boundaries. Native E14 evidence uses the retained rule order and proves the required single-loss outcomes; the listener-recovery code stays within the existing private owner and preserves the accepted no-quiescence behavior. The activated tests retain their assigned stimuli and required native outcomes. The guard-only S62 observation note remains unproven under the repository reproduction gate. No later step or final-wave gate is part of this verdict.
