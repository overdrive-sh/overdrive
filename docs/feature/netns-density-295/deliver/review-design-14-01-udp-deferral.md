# STEP 14-01 Service-VIP UDP deferral independent DESIGN review

## Metadata and verdict

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-01` |
| Date | 2026-10-30; release reconciliation 2026-10-31 |
| Role | Fresh independent bounded solution-architecture reviewer |
| Iteration | 1 — DESIGN approval; Iteration 2 release decision appended below |
| Reviewed decision | User-approved option D: defer the Service-VIP UDP-to-VM acceptance action to #310 |
| Verdict | **APPROVED** |
| Blocking findings | 0 critical; 0 high |
| Roadmap disposition | Iteration 1: `pending`; Iteration 2: **approved after independent DISTILL alignment review** |
| Execution authority | Iteration 1: none; Iteration 2: **original STEP 14-01 crafter may resume RED/GREEN against the retained semantic RED** |

This review used the current `AGENTS.md`, the applicable repository DESIGN and
development rules, the installed `nw-solution-architect-reviewer` definition,
`nw-sar-critique-dimensions`, and `nw-roadmap-review-checks`. The repository's
bounded-scope, explicit-user-authority, native-Markdown, no-iteration-cap, and
roadmap-pending rules supersede the installed reviewer's legacy YAML and
iteration defaults.

In Iteration 1 I edited only this review artifact. I did not change the feature
delta, roadmap, DESIGN or DISTILL records, executable source, tests, DES state,
execution log, GitHub issue, or host state. I created no commit and ran no
build, test, mutation, Lima, Metal, or native execution job.

## Reviewed artifact identity

| Artifact | SHA-256 at review |
|---|---|
| `feature-delta.md` | `e58b0019e05cf894e6febde04bd21d3a49cbb15e14deed05b87828f11fcb8f74` |
| `deliver/roadmap.json` | `acee972da92cb6dfa357b24e821acd179c49b70d29f145ed7b93a979d81c32a6` |
| `deliver/design-14-01-v6-udp-reconciliation.md` | `c7504a8b61c1457b3a8abfa74946af81141149b9e1f51750b7325286c2cac6a4` |
| `deliver/design-14-01-udp-deferral.md` | `56d8f7585160c5c7f77b5a304b29a0ae855abc9e0b59c05c6e27b218dd697b39` |
| `deliver/distill-14-01-v6-coverage.md` | `90a5e64b4d0bbdb372d6dff0c9cb75c958b5e0f08149bf7727f01e8436867e0c` |
| `deliver/distill-14-01-native-support.md` | `18725930557c6aac5d705ee7234ac1ebd11025aa5c2cd768329a0722c83c6e9d` |
| ADR-0164 | `c5a697a29944e4c486928816d813f572132f9392ed8ee74d8b9ecc3417c9a1e7` |

The review read the complete bounded correction, the prior reviewed V-6 UDP
reconciliation and its independent review, the changed canonical V-6 row and
STEP 14-01 roadmap block, both current DISTILL handoff records, D12, D24/D24a,
ADR-0164, V-LEGB-SERVE, and the roadmap validation history. Other dirty source,
test, review, and workspace-rule changes remain their owners' work and are
neither adopted nor approved here.

`roadmap.json` parses as JSON and `git diff --check` passes. No executable
verification is needed for this documentary contract correction; the retained
native failure is evidence for #310 and the executable acceptance alignment is
the next DISTILL gate.

## Explicit user authority and exact normative diff

The correction records the exact user approval:

> “yeah, let's not do that now. add all the relevant info to the github issue,
> and then we keep it deferred to #310 and correct the contradictory
> acceptance checklist.”

That statement is explicit authority for reviewed option D. It authorizes the
following two removals and no other outcome relaxation:

| Canonical location | Pre-decision clause | Corrected contract | Review result |
|---|---|---|---|
| Feature-delta V-6 | “and a service VIP over UDP (`connect4`; includes the confirmation that the ADR-0053 attachment at `overdrive.slice` rewrites owner sockets in `overdrive.slice/control-plane.slice`)” | Removes the Service-VIP UDP-to-VM acceptance action, records its deferral to #310 under D12, and states that D24's general guest-datagram `connect4` contract remains unchanged. | **Exact approved weakening** |
| STEP 14-01 criterion 3 | “and a service VIP over UDP” | Excludes Service-VIP UDP delivery to a VM backend from STEP 14-01 and defers it to #310 under D12. | **Exact approved weakening** |

The roadmap description's change from generic “mesh, VIP, inbound and probe”
to “mesh-name, TCP/mTLS VIP, inbound TCP and probe” is an accurate editorial
projection of those same two removals. The implementation note permits only
the independently reviewed DISTILL alignment that removes the deferred action;
it grants no production edit or #310 mechanism authority.

The normative-term audit found no additional removal or relaxation of
`zero`, `never`, `must`, `exactly`, `required`, `forbidden`, fail-closed,
source-identity, loss-accounting, cleanup, production-composition, or
security-boundary language. The design record quotes the removed clauses,
quotes the user's authority, and explicitly limits the amendment to them.

## Preserved STEP 14-01 outcomes

| Required outcome | Preserved contract evidence | Review result |
|---|---|---|
| Off-node TCP and non-empty UDP through the real NIC | V-6 retains both protocols; roadmap criterion 1 retains their native-metal round trip. | **Preserved** |
| Genuine empty UDP datagram | V-6 and criterion 1 retain the empty datagram; the DISTILL record retains the zero-length request, reply, source, and guest-success oracles. | **Preserved** |
| Node source identity | Criterion 1 still requires the peer to observe the node address; the DISTILL coverage keeps the node-side source assertion. | **Preserved** |
| Mesh-name resolution through guest DNS at `gateway:53` | Criterion 3 and the DISTILL contract retain the production gateway-DNS association and derived mesh name. | **Preserved** |
| Mesh Service VIP/frontend over TCP with mTLS through leg F | V-6, criterion 3, and the fail-closed mTLS oracle retain the TCP/VIP path and bidirectional TLS evidence. | **Preserved** |
| Remote inbound TCP through leg C -> leg S -> `GuestStreamDial` | V-6, criterion 3, and the route-disjoint remote-client action remain mandatory. | **Preserved** |
| TCP and HTTP probes follow healthy -> unhealthy -> healthy application state | V-6, criterion 3, and the fail-closed probe-state oracle remain mandatory. | **Preserved** |
| V-LEGB-SERVE with and without a default route, `127.0.0.1:<ephemeral>`, and zero matching uplink packets | V-LEGB-SERVE is unchanged; criterion 3 and the loss-accounted no-uplink oracle retain it. | **Preserved** |
| Concurrent `IP_BIND_ADDRESS_NO_PORT` set/unset comparison | V-LEGB-SERVE and the population-bounded DISTILL comparison are unchanged. | **Preserved** |
| Public stop, VMM disappearance, owner/socket/route/interface cleanup, and loss-accounting complements | Criterion 1, criterion 4, the deferral record, and the DISTILL checklist retain these outcomes. | **Preserved** |
| Production composition | V-6 remains through real `overdrive serve` plus public `overdrive deploy`; criterion 4 still forbids test-installed product state and override seams. | **Preserved** |
| Executable inventory and accounting | The corrected DISTILL checklist retains the existing one CLI identity plus five `guest_flow_through_serve` identities and `111 + 1 + 5 = 117`. | **Preserved** |

The correction therefore does not turn the journey into TCP-only coverage and
does not remove general guest UDP. It removes one destination/backend outcome:
Service-VIP UDP delivery to a VM backend.

## D12, D24, guest-prefix refusal, and #310 boundary

D12 remains accepted as “Inbound UDP service to VMs deferred.” ADR-0164 still
requires datagrams to a Service VIP to use the guest-flow owner's connected
host UDP socket and the existing ADR-0053 `connect4` rewrite. It still requires
the owner to refuse the association as `Policy` when the post-rewrite kernel
peer lies inside the guest prefix. The feature delta's D24 and D24a rows are
unchanged, and the corrected V-6 row names D24's general guest-datagram
contract explicitly. The correction neither deletes the general Service-VIP
datagram contract nor creates an exception for a guest-prefix peer.

No non-VM Service backend is introduced to preserve the removed action. The
design rejects a direct map write, injected backend row, private publisher,
adapter override, fabricated reply, and any new production API or test seam.
It also does not select a #310 association identity, peer-address encoding,
reply-routing mechanism, first-datagram protocol, ownership model, lifecycle,
wire format, public port, enum variant, or typed-error representation.

The six #310 items are properly stated as questions the future DESIGN must
resolve. They identify the unresolved semantic contract without deciding its
representation. The retained kernel-only payload boundary, no guest NIC/TAP,
native-Metal evidence requirement, and no userspace application-payload relay
come from the accepted product boundary; they are constraints on #310, not a
new #295 mechanism.

The parent orchestrator reports that GH #310 was updated with the relevant
failure evidence and open contract questions and that the final issue body was
read back exactly. This reviewer performed no duplicate external write. The
repository record contains the same bounded questions and does not claim that
their answers are selected.

## History and prior-approval preservation

The prior `design-14-01-v6-udp-reconciliation.md` now places the user's option-D
selection and the new canonical correction before its preserved pre-selection
analysis. It explicitly marks later “user decision required” language as
historical and superseded for current disposition. The current DISTILL records
use “then-required, now-deferred” for the failed action and preserve the native
run as evidence. Historical review artifacts remain unchanged records of the
contract in force when their iterations ran. No stale current instruction
still requires STEP 14-01 to make UDP reach the peer VM Service.

The roadmap changes current validation from `approved` to `pending` and carries
the former STEP 13-02 PRCI-1 approval into `prior_approval`. That object retains
its exact approved carrier and review artifact, and nests the earlier STEP
13-01 PFRO-1 approval beneath it. Completed STEP 12-01, STEP 13-01, STEP 13-02,
GUEST-PORTS, EVMOD-PAIR, and other earlier approvals are not revoked or
relabelled. At Iteration 1, no current `approved_at` or execution claim was
present.

## Architecture and roadmap review checks

| Review dimension | Result |
|---|---|
| Architectural bias and subtraction | The correction removes an impossible current acceptance action instead of adding a backend kind, fallback, map injection, protocol, or API. It follows the user's selected smallest bounded outcome change. |
| Decision quality | The exact authority, before/after clauses, consequences, retained outcomes, rejected expansions, and downstream gates are explicit. The previously reviewed A-D package supplies the alternatives analysis; this record applies the selected option without reopening it. |
| Completeness | D12, D24/D24a, guest-prefix refusal, production composition, source identity, loss accounting, cleanup, security, typed-error, lifecycle, and future #310 questions are all addressed. |
| Implementation feasibility and testability | #295 needs no production mechanism. The existing P6-S13 body and private support already contain the action; DISTILL can remove only it and its gate while preserving the established external/public oracles. Any need for production surface remains a blocker rather than implied authority. |
| Priority validation | The correction addresses the reproduced contradiction directly: public VM Service UDP delivery is #310 while STEP 14-01 required it. The user selected deferral after the prior review presented the alternatives. |
| External validity | **Pass.** The retained journey remains invocable through real `serve` and public `deploy`; the client VM Service, peer VM Service, remote client, DNS, TCP/mTLS, probes, off-node UDP, and cleanup paths remain observable. |
| Acceptance-criterion coupling | The modified criterion states observable production behavior and the exact excluded outcome. It adds no private helper, algorithm, return type, or new implementation structure. |
| Step decomposition | No step, dependency, estimate, scenario identity, selector, lane, or executable count is added. The correction remains inside STEP 14-01. |
| Implementation code in roadmap | None added. The implementation note constrains authorization and points DISTILL to an already-authored body; it does not prescribe a #310 implementation. |
| Roadmap precision | The modified description and criterion distinguish TCP/mTLS VIP delivery from the deferred UDP-to-VM action. The broader existing roadmap is outside this bounded review and is not rewritten. |
| Test boundary | The same black-box production-composition body and five in-process owner complements remain distinct. No production binary invocation is moved into a Rust unit boundary and no expectation or new lane is introduced. |

## Iteration 1 findings and dispositions

| Finding | Severity | Disposition |
|---|---|---|
| No blocking design, authority, scope, typed-error, lifecycle, security, roadmap-history, or contract-preservation defect was found in the scoped correction. | — | **Closed by review; no remediation required.** |

The historical documents contain the old requirement and alternatives because
they are evidence of how the contradiction was found and how option D was
chosen. Their current-disposition sections unambiguously supersede that old
instruction. Deleting those records would lose review history and is neither
required nor authorized.

## Release to DISTILL

This approval completes only the fresh independent DESIGN review named in the
roadmap's `pending_scope`. DISTILL may now perform the bounded acceptance
alignment, subject to all of these conditions:

1. Remove only the Service-VIP UDP-to-VM action and the dependency that gates
   later retained assertions on its `vip-udp ok` / `paths-ready` success.
2. Preserve off-node TCP, non-empty UDP, genuine empty UDP, gateway DNS,
   node-source identity, TCP/mTLS VIP, remote inbound TCP, TCP/HTTP probe-state,
   V-LEGB-SERVE no-uplink/socket-identity, `IP_BIND_ADDRESS_NO_PORT`, public
   stop, VMM disappearance, and cleanup/loss-accounting oracles.
3. Preserve the existing six exact STEP 14-01 identities, selectors and lanes,
   and the `111 + 1 + 5 = 117` accounting. Add no seventh test or scenario.
4. Add no non-VM backend, direct map state, injected production row, fabricated
   reply, production hook, public/cross-crate API, wire/persisted format, enum
   variant, error mapping, owner, lifecycle, or #310 mechanism.
5. Keep historical native failure evidence intact and describe the action as
   deferred rather than repaired or passed.
6. Obtain a fresh independent DISTILL review of the aligned acceptance body and
   checklist before the roadmap validation can leave `pending` or STEP 14-01
   can regain execution authority.

If the bounded alignment cannot preserve every retained outcome with private
test support and the existing public/operator/kernel observations, DISTILL must
return that exact testability gap. It may not create production surface or
weaken another oracle.

## Iteration 1 final verdict

**APPROVED for the exact user-authorized STEP 14-01 Service-VIP UDP-to-VM
deferral only.** The canonical V-6 and criterion-3 changes match the quoted
approval; D12, ADR-0164's guest-prefix `Policy` refusal, D24's general connected
host-socket `connect4` contract, every retained STEP 14-01 outcome, the six
identities/117-body accounting, and prior approvals remain intact. No #310
architecture or new API surface is selected.

Iteration 1 concluded that the roadmap must remain `pending` until the DISTILL
alignment and its independent review satisfy the release criteria above. That
iteration did not self-approve roadmap execution or authorize the STEP 14-01
crafter to resume. Iteration 2 below supersedes only that release disposition
after independently verifying that both named conditions completed.

## Iteration 2 — reviewed DISTILL release decision

### Evidence and disposition

| Field | Value |
|---|---|
| Date | 2026-10-31 |
| Trigger | Completed independent DISTILL review of the bounded UDP-deferral acceptance alignment |
| DISTILL alignment | `deliver/distill-14-01-udp-deferral-alignment.md` (`dbf485e2de42ce1c2bcdeeb9fc643b94af57bbb99472c79325ac6f2b9b3504a1`) |
| Independent DISTILL review | `deliver/review-distill-14-01-udp-deferral-alignment.md` (`93b2adcf1b0607cb9eefbe36873b514701a3dbea0e152efdde2dcb820caa26e5`) |
| DISTILL verdict | **APPROVED — 0 blocker, 0 high, 0 medium, 0 low findings** |
| Release verdict | **APPROVED FOR THE ORIGINAL STEP 14-01 CRAFTER TO RESUME RED/GREEN** |
| Native implementation state | **RED** — initial TCP/HTTP readiness does not reach the required `Pass` pair |
| Native runner state | **Not 117 GREEN** — the CLI identity fails before the five control-plane identities run |

The independent DISTILL reviewer read the aligned source and durable exact-
source evidence, verified the canonical option-D boundary, and approved the
alignment for DELIVER handoff. Its two evidence-label corrections — four
incidental lowercase kernel `skipped` lines and the controlling mTLS source
range — are non-blocking and do not change a contract, source identity,
receipt, or verdict.

Iteration 2 changes only this review artifact and the roadmap's validation
state. It does not edit executable source, tests, DESIGN/DISTILL handoffs, DES
state, the execution log, receipts, host state, or prior-approval records, and
it creates no commit or additional execution evidence.

### Release-criteria reconciliation

| Iteration-1 release condition | Independent evidence | Result |
|---|---|---|
| Remove only the Service-VIP UDP-to-VM action and its gate | The four P6-S13/private-support files contain no `V6_UDP` or `vip-udp`; the port, records, peer listener/server, client exchange, marker, assertion, and `paths-ready` dependency are removed. The held issued-VIP TCP stream carries the existing acknowledged state commands as private DISTILL HOW. | **SATISFIED** |
| Preserve every retained outcome and fail-closed oracle | The review maps off-node TCP/non-empty UDP/empty UDP, node source, distinct peer and client VM Services, gateway DNS, mesh name, issued-VIP TCP mTLS, remote inbound TCP, TCP/HTTP readiness transitions, no-uplink/socket identity, `IP_BIND_ADDRESS_NO_PORT`, public stop, VMM disappearance, and cleanup complements to exact source and oracles. | **SATISFIED** |
| Preserve the six identities, lanes, selectors, and `111 + 1 + 5 = 117` inventory | The existing one CLI identity and five `guest_flow_through_serve` identities remain, all six retain `CONTRACT_SHAPE: bounded-change.`, and the runner/count/selector/lane hashes remain unchanged. | **SATISFIED** |
| Add no non-VM backend, product state injection, production hook, API, wire/persisted format, error, owner, lifecycle, or #310 mechanism | The source delta is test/fixture-owned; D24 and D24a are unchanged; no production crate or general UDP mechanism test changed; no substitute UDP path or public product surface was added. | **SATISFIED** |
| Preserve the failed UDP evidence as deferred rather than passed | The handoff and review keep the earlier timeout as #310 evidence, remove it from #295 credit, and make no pass claim for Service-VIP UDP-to-VM delivery. | **SATISFIED** |
| Obtain fresh independent DISTILL approval before roadmap release | `review-distill-14-01-udp-deferral-alignment.md` returns `APPROVED` with no open finding and explicitly releases the aligned body to the original STEP 14-01 crafter. | **SATISFIED** |

The aligned source hashes match the approved DISTILL record: P6-S13 body
`2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89`,
CLI private support
`08d07e0336f8e6c86902bd082e98541c7ef09b5f3840cf1b14762412cebff2e1`,
guest workload program
`a83191b2e9b817dfa7da27bf806fc406fec2a361688fba81a03705675e4e4136`,
and shared test protocol/remote peer
`395c3fc937de129f66a751edf1226be5f928f592d003fb98ab11219d2c80f4c7`.
The source gate is GREEN for formatting, check, and Clippy at the scoped
targets. The exact native runner is deliberately not relabelled GREEN.

### Retained semantic RED and DELIVER boundary

The exact-source native runner preserves 101 successful libtest summaries
totalling 111 passed bodies, then invokes the existing P6-S13 CLI identity.
The public peer VM Service reaches Running, production init starts its guest,
the inferred TCP startup probe reaches `Pass`, and public describe exposes the
exact TCP readiness descriptor at index 0 and HTTP `/healthz` readiness
descriptor at index 1. Both retained readiness rows remain
`Fail { last_fail_reason: "unreachable" }` at the 30-second deadline, so the
required initial pair of `Pass` rows is absent.

That failure is an honest production-composition semantic RED on a retained
STEP 14-01 outcome. It occurs before the former Service-VIP UDP action's
position and is neither a fixture/contract ambiguity nor a reason to weaken
the oracle. The later remote-inbound, client-Service, mTLS capture,
probe-transition, population, no-uplink, comparison, and success-cleanup
assertions did not run and receive no native credit. The five control-plane
identities also did not run in that failed runner invocation. DELIVER must make
the retained initial readiness outcome GREEN and then obtain the complete
exact-source 117-body evidence and independent implementation review.

No additional DESIGN or DISTILL approval gate is created for investigating or
fixing this reproduced RED within the already accepted architecture. If the
original crafter finds that satisfying it requires a new public/cross-crate
API, owner, lifecycle, typed-error representation, persistence/wire contract,
or other architecture absent from the accepted design, the existing
specification-ambiguity rule still applies; this release does not pre-authorize
such surface.

### Roadmap validation restoration

The roadmap's validation gate may now return to `approved` because the two
conditions recorded in its pending scope are complete:

1. the exact user-approved deferral received independent DESIGN approval in
   this artifact; and
2. the exact executable acceptance alignment received independent DISTILL
   approval in `review-distill-14-01-udp-deferral-alignment.md`.

The restoration cites the user's exact 2026-10-30 approval, both independent
reviews, the retained RED, and the bounded DELIVER authority. It preserves the
full nested `prior_approval` chain for STEP 13-02 PRCI-1 and STEP 13-01 PFRO-1.
It does not claim a production implementation is GREEN, a native 117-pass
receipt exists, or the current DELIVER review is complete.

### Iteration 2 final decision

**RELEASED TO DELIVER RED/GREEN.** The original STEP 14-01 crafter may resume
at the reproduced initial TCP/HTTP readiness RED under the corrected V-6
contract. Only Service-VIP UDP delivery to a VM backend remains deferred to
#310. D12, D24/D24a, the guest-prefix `Policy` refusal, every retained outcome,
the same six identities/117 inventory, and all prior approvals remain intact.

This is an evidence-based roadmap restoration after independent DESIGN and
DISTILL review. It is not self-approval, implementation approval, a GREEN
verdict, or native completion evidence.
