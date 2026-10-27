# STEP 13-01 PFRO-1 independent DESIGN review

## Metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `13-01` |
| Date | 2026-10-27 |
| Role | Fresh independent bounded solution-architecture reviewer |
| Iteration | 1 |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 1 high; 0 critical |
| Native conclusion | Same-flow post-FIN RST is demonstrated; post-EOF read is an invalid reset oracle |
| Execution disposition | Roadmap validation remains pending; this review grants no implementation or DISTILL authority |

Reviewed the current `AGENTS.md`, applicable development and spike rules, the
solution-architect reviewer definition, `nw-sar-critique-dimensions`, and
`nw-roadmap-review-checks`. Repository scope, evidence, Markdown, and iteration
rules take precedence over the installed definition's legacy defaults.

Only this review artifact was edited. No production, test, DES, execution-log,
feature-delta, roadmap, or probe file was modified; no native job, mutation run,
or commit was created.

## Reviewed artifact identity

| Artifact | SHA-256 |
|---|---|
| `deliver/native-13-01-postfin-reset-observation.md` | `f38a9424df3f35f2141f78454c2dfb2de04b5b98a005ce745998000e8a481c54` |
| `deliver/design-13-01-postfin-reset-observation.md` | `ecf8a7305a0ebb1c0f1fb4a24422943fb80be5b77ec4cb4e2f7c866823092f3e` |
| `increment-a/native-endpoint-observation.log` | `48a7520ee61c32223482804b4d2a5658f350b64cc7431add9108d87ef9781d7d` |
| `increment-a/native-production-fin-rst.log` | `726ce9d1fa823c8c4fdc4ce9d7996f45ff84c775ec4e8d1fe73576261c2dbbf5` |

The evidence directory is
`spike-scratch/netns-density-295-postfin-reset/increment-a/`. The review also
read its complete C source, runner and capture scripts, README and raw logs;
the three requested `.context/netns-density-295/13-01/` receipts; the affected
D9/D14/D20/DG-10 contract; STEP 13-01 and its dependencies; the bounded
feature-delta/roadmap diff; and the current production and vendored Linux
paths cited below. The completed STEP 12-01 review's final approval and commit
chain were checked without reopening their implementation.

## Finding PFRO-R1 — owner completion is asserted without an existing completion boundary

**Severity: high. Status: open. Disposition: bounded documentary remediation
required before approval.**

`design-13-01-postfin-reset-observation.md:22-23` requires:

> Dropping the claimed registration completes the existing
> `MtlsEnforcement::teardown` before the owner can finish releasing the flow.

This is stronger than the existing `ForwardedOutbound` contract and does not
describe the current caller/owner path. It conflicts with the proposal's
observation-only scope and unchanged lifecycle/API assertions at `:8-12`,
`:63-86`, and `:123-134`. The native record's `:55-58` similarly compresses
the token drop and the reaper's await into an apparent owner completion step.

The exact current ordering is:

1. `owner_runtime::process_end` removes the pair from the owner map
   (`crates/overdrive-control-plane/src/guest_network/owner_runtime.rs:1309-1316`).
2. It awaits closing the owner-held peer and vsock descriptors and emits
   `guest_flow.closed` (`:1408-1413`). The remaining move-only registration
   field is dropped at the end of that pair's scope (`:1414`;
   `flow_owner.rs:285-296`).
3. `ForwardedOutbound::drop` removes the registration and calls the synchronous
   `ForwardedRegistrationState::close`
   (`crates/overdrive-worker/src/mtls_intercept_worker.rs:1347-1363`).
4. `close` cancels the token, takes an available handle, and submits an async
   teardown task using the current Tokio handle (`:190-199`). It returns no
   completion handle to the guest-flow owner and awaits nothing itself.
5. Alternatively, the already-spawned registration reaper takes the handle
   after cancellation and awaits teardown in its own task (`:1366-1373`,
   spawned at `:3182`). Neither path is joined by `process_end`.

`MtlsEnforcement::teardown` itself is an awaited effect-completion boundary:
its `Ok` postconditions close both legs and stop both pumps
(`crates/overdrive-core/src/traits/mtls_enforcement.rs:825-850`), and the host
implementation awaits `spawn_blocking(reclaim_connection)`
(`crates/overdrive-dataplane/src/mtls/mod.rs:428-446`). That does not make the
separate owner's synchronous token drop an awaited boundary. DG-10 reading 2
and the declared token API say the sockets **then** close; they do not add an
owner join (`feature-delta.md:6524-6530,12409-12416`). The current integration
cell likewise drops the registration and subsequently waits for the remote
observation (`vsock_mesh_egress_mtls.rs:1379-1388`).

**Required remediation:** reconcile item 2 and the native reachability wording
with that exact boundary. PFRO-1 must not silently introduce a new owner
completion guarantee while claiming no lifecycle or production change. Keep
the approved teardown postconditions intact and distinguish the task that
awaits teardown from the owner that requests it. If an owner-level completion
guarantee is intended, that is a separate contract decision requiring its own
proven necessity and exact approved interface; this bounded review does not
authorize it.

This finding is an explicit contradiction in the proposed contract's account
of existing code, not a new production race or cleanup-defect claim. No
hypothetical cancellation schedule, forced abort, new Sim seam, or broader
runtime remediation is proposed. The native evidence proves eventual reset
after the drop; it does not prove the additional owner-return ordering.

## Native evidence and reachability assessment

### Production composition and the affected owner path

`run_server` supplies the existing enforcement, intercept and resolve ports to
`compose_guest_network` (`crates/overdrive-control-plane/src/lib.rs:3927-3968`).
That composition constructs one worker and one owner with the same worker
instance (`guest_network_composition.rs:81-117`). The owner's stream task
dispatches `TcpConnect` to `tcp_connect`
(`guest_network/owner_runtime.rs:782-798`). Its mesh branch resolves once,
opens and binds the real loopback peer, registers it before connect, and
attaches the pair (`:1055-1094`). `OwnedPair::_forwarded` retains the returned
token (`:819-858`; `flow_owner.rs:285-296`).

The real worker claims the registered peer before spawning outbound
enforcement (`mtls_intercept_worker.rs:3099-3127,3175-3204`). The host adapter
retains the accepted leg-F descriptor and dialled leg-B descriptor in one
`ConnState`, with both splice pumps (`mtls/mod.rs:82-101`;
`mtls/outbound.rs:83-152`). Linger is armed on both before kTLS and the pumps
(`outbound.rs:105-117`). A source clean end propagates leg B's `SHUT_WR` after
the accepted bytes, without reclaiming the reverse direction
(`mtls/splice.rs:682-743`; `mtls/mod.rs:219-229`). A subsequent owner-held token
drop reaches the existing cancellation/teardown paths described in PFRO-R1.
Reclaim stops the pumps, joins them before descriptor release, and preserves
abortive linger (`mtls/mod.rs:317-352`). No retry or new recovery path is needed
to explain the captured effect.

The Tier-3 cell exercises this existing worker port with real intercept and
enforcement adapters. It plays the guest-flow owner's registered-loopback
role; it does not execute the entire control-plane owner or built product
binary. Its own module contract explicitly identifies that boundary
(`vsock_mesh_egress_mtls.rs:13-33,437-467`). This is appropriate host-adapter
evidence for the TCP effect. It must not be described as a complete
`serve`/`deploy` end-to-end execution or proof of an owner-level await.

### FIN/RST direction and timing

The capture script selects only the existing half-ended cell, starts
loopback FIN/RST capture before nextest, and retains the cell's nonzero exit
separately (`increment-a/capture-production-rst.sh:4-22`). In run
`944d48d864bb9263b521ef70`:

| Boundary | FIN | RST | Assessment |
|---|---|---|---|
| Leg F | `127.0.0.1:33519 -> :35783`, `20:40:17.433660` | `:35783 -> :33519`, `20:40:22.485841` | Reverse direction is correct: the forwarder sends FIN, then the accepted leg-F endpoint resets it. |
| Leg B | `127.0.0.1:39876 -> :43469`, `20:40:17.433683` | `:39876 -> :43469`, `20:40:22.485909` | Same source, destination and four-tuple; the leg-B endpoint emits RST after its own FIN. |

The leg-B interval is 5.052226 seconds. It matches the cell's explicit
five-second `StillOpen` precondition before `drop(registration)`
(`vsock_mesh_egress_mtls.rs:145,1365-1380`), rather than an automatic
five-second production drain deadline. The earlier clean FIN pairs occur
during fixture/probe setup. The target pairs are the only later FIN/RST pairs,
and the capture reports zero kernel drops
(`native-production-fin-rst.log:6-18`). The opposite leg-F direction and the
same leg-B direction must remain distinguished.

### Absent reset versus invalid observation

The unchanged recorder marks `saw_fin`, then repeatedly reads after EOF and
recognizes only a subsequent read error as reset
(`vsock_mesh_egress_mtls.rs:931-954`). The original
`green-half-12.log:44-66` proves the failing observation: local leg B is state
5 during reclaim, while the peer recorder remains `saw_fin: true` /
`Termination::Open`. `debug-half-7.log:49-70` additionally records kTLS
`AF_UNSPEC` failure with errno 95. `linger2-triple.log:29-54` records two passing
siblings and the same half-ended timeout. Those historical failures cannot
establish absence of RST.

The isolated probe correctly separates those questions. It waits for the
peer to consume FIN, checks local `TCP_FIN_WAIT2`, applies each candidate,
then reads and queries the still-owned peer descriptor
(`postfin_reset.c:188-232,284-321`). Run `1c399d41d0b18d5213938a51` records:

| Candidate | Local before close | Peer after action | `SO_ERROR` | Later `recv` |
|---|---:|---:|---:|---:|
| Existing zero linger | 5 | 7 | 32 | 0 |
| Negative `TCP_LINGER2` plus zero linger | 5 | 7 | 32 | 0 |
| `TCP_REPAIR` | 5 | 8 | 0 | 0 |
| Exact-cookie `SOCK_DESTROY` | 5 | 7 | 32 | 0 |

The probe uses directly armed kTLS 1.2 without the production TLS 1.3
handshake/payload. It isolates close and peer TCP semantics; the independent
production-cell capture supplies the real handshake, pumps and payload path.
Its `main` intentionally prints all candidates and ignores their Boolean
return values (`postfin_reset.c:348-354`), so exit 0 is not evidence that every
candidate succeeds. The recorded states and errors are the evidence. Together
the two runs support the proposed oracle correction without inventing a new
close mechanism.

## Linux 7.0 source verification

The vendored tree identifies Linux 7.0 (`vendors/linux/Makefile:2-5`). The
review read the cited code and the adjacent transitions:

| Claim | Verified primary-source path |
|---|---|
| kTLS close reaches the saved TCP protocol | `net/tls/tls_main.c:365-395` restores `sk_prot`, then calls the saved close. |
| kTLS socket options and disconnect differ | `tls_main.c:896-910` delegates non-`SOL_TLS` options but rejects `tls_disconnect`; `:1005-1009` installs that disconnect. `net/ipv4/af_inet.c:648-656` uses the active protocol's disconnect for `AF_UNSPEC`. |
| Zero linger can reset an endpoint already in `FIN_WAIT2` | `net/ipv4/tcp.c:3225-3228` invokes disconnect; `:3360-3364` includes `FIN_WAIT2` in `tcp_need_reset`; `:3409-3419` sends active reset. |
| FIN establishes the state that masks a later read error | `net/ipv4/tcp_input.c:4947-4961` sets receive shutdown and `SOCK_DONE`, then enters `CLOSE_WAIT`. |
| Later reset still records a typed endpoint error | `tcp_input.c:4890-4930` records `EPIPE` for reset in `CLOSE_WAIT` and invokes terminal cleanup. |
| Read-after-EOF can remain zero despite that error | `tcp.c:2793-2803` checks `SOCK_DONE` before `sk_err` and receive shutdown. |
| `SO_ERROR` consumes the pending error | `net/core/sock.c:1797-1800` reads through `sock_error`; observation must retain the first nonzero result rather than expect it to persist after consumption. |
| Repair does not add an active reset | `tcp.c:3412-3419` selects the repair branch before active reset. |

The native `TCP_REPAIR` counterexample and the source justify reopening and
rejecting it. Negative linger adds no required effect, `AF_UNSPEC` is blocked
by the current kTLS protocol, and privileged `SOCK_DESTROY` duplicates an
already demonstrated close effect. Withholding FIN and accepting EOF-only
would change the accepted outcomes and remain forbidden. The selected wire or
terminal-state/error observation is supported by the actual production
capture and endpoint semantics.

## Interface, errors, ownership and invariant audit

PFRO-1's intended scope adds no production operation, port, owner, task,
runtime hook, model, config/CLI item, public type, enum variant, wire value or
persisted format. `ForwardedOutbound` remains the same move-only token;
registration, pairing and teardown signatures are unchanged. The outstanding
exception is the unsupported lifecycle ordering in PFRO-R1, not a missing new
typed-error representation.

| Designed condition / result | Approved representation and implementation | Disposition |
|---|---|---|
| Remote receives reset after FIN while reverse direction remains open | Native TCP `CLOSE_WAIT -> CLOSE`, pending `EPIPE` (32), or same-flow FIN/RST packet evidence | Exact endpoint evidence; no Overdrive error wrapping is added. |
| Remote reset before FIN in the existing open-drop case | Existing peer socket reset/error observation; EOF-only is insufficient | Preserved. |
| Clean close in both directions | FIN with no later RST and no pending socket error | Preserved; aligned DISTILL observation must distinguish this from the half-ended case. |
| Enforcement teardown fails | Existing `MtlsEnforcementError::TeardownFailed { id: EnforcedConnectionId, source: io::Error }` (`mtls_enforcement.rs:535-544`; host mapping `mtls/mod.rs:440-445`) | No new mapping or error branch authorized; PFRO-R1 must not imply a new owner-facing result. |
| Missing/duplicate registration and refusal precedence | Existing DG-10 readings 1 and 4, including `NotInterceptLive`, `DuplicateForwardedRegistration`, shutdown and shared-owner causes | Unchanged; no diagnostic string substitutes for a semantic distinction. |

The bounded-change declaration identifies the existing registration and
owned endpoints, the observed peer/wire universe, and unchanged complements.
There is no new unbounded-preservation component, read port with write power,
or broad capability injection. Test-local representation and observation
helpers remain DISTILL's responsibility; no public test-support API is needed
or authorized. The existing scenario/lane remains the only affected scenario.

This is an observed host-adapter TCP effect, supported by preserved real-kernel
receipts. It is not a speculative control-plane convergence or timing defect.
A new concurrency model, seeded invariant or Sim seam would not be a valid
additional gate for this oracle correction. No such expansion is required by
this review.

## Selective invalidation and normative diff

The feature-delta diff changes only the D20 evidence-status annotation and
adds the DG-10 notice. The accepted D20 mechanism and reset outcome, DG-10
readings, D9 one-resolution/registered pairing, and D14 cleartext removal are
not removed or relaxed. Normative diff inspection found no reduction of
`zero`, `never`, `must`, `exactly`, fail-closed, cleanup, security, or reset
outcomes. PFRO-R1 instead identifies an unsupported added completion promise.

The roadmap parses as JSON. Comparison with HEAD shows `validation` is the
only changed top-level value; no step, criterion, dependency, verification
command, or implementation scope changed. `status = pending`,
`approved_at = null`, and the explicit STEP 13-01 `pending_scope` correctly
block execution of the affected step and its dependent execution. The prior
2026-10-27T18:56:09Z approval and its review artifact remain recorded.

The completed chain `7105b5807`, `9f7befe04`, `83419d308`, `f613be8b3` exists,
and `review-12-01.md:268-272` records its final approval. Nothing in the new
native evidence contradicts that chain, unrelated D9/D14 obligations,
leg-C/`GuestStream`, LEGS-DIAL, other DG-10 readings or other D20 paths. Their
requirements and prior approvals remain valid. Existing sibling pass receipts
are preserved; they are not a replacement for the proposed DISTILL alignment
and fresh triple/full-suite verification with an honest post-FIN oracle.

## Architecture and roadmap checks

| Required check | Result for this bounded amendment |
|---|---|
| Architectural bias | Pass: existing close effect is measured; added privileged mechanisms are rejected for lack of need, with native/source evidence. |
| Decision context, alternatives, consequences | Pass: exact observed problem, multiple reopened alternatives and bounded quality-attribute consequences are recorded. |
| Completeness | One blocking lifecycle-account contradiction, PFRO-R1; the observation contract itself is precise. |
| Feasibility / testability | Pass: same-flow wire evidence and peer state/error inspection exist at the current Tier-3 boundary, without production seams. |
| Priority validation | Correct problem and scope: fix the falsified evidence premise, supported by two native runs; no production mechanism work is justified by the timeout. |
| External validity | Pass: current registered-loopback production adapter path is exercised; the proposal does not claim a new invocable feature or replace the existing V-6 product proof. |
| Acceptance-criterion coupling | Pass for the amendment: observable FIN/RST/error distinctions; test-support implementation remains DISTILL-owned. No roadmap criterion changed. |
| Step decomposition | Unchanged: no new step or production-file decomposition. STEP 13-01 still depends on 12-01; no duplicated scenario family is added. |
| Implementation code in roadmap | No added implementation code or algorithm. |
| Roadmap concision / precision | Bounded validation/status notice only; no expansion of step descriptions, criteria or notes. Existing frozen roadmap bulk is outside this review. |
| Test boundaries | Pass: in-process production adapter test and isolated native probe; no product expectation runner is invented, no integration test is converted into a built-binary runner, and no model seam is added. |

## Verification and remediation disposition

Read-only verification included native source/log reconciliation, exact tuple
and direction checks, production caller/owner tracing, vendored-source checks,
roadmap JSON/structural comparison with HEAD, commit-chain inspection, and
`git diff --check` on the bounded canonical diff. Those checks succeeded; no
new native run was necessary because the preserved evidence is internally
consistent. No test or evidence result was fabricated or inferred from an
exit code alone.

| Finding | Iteration 1 disposition | Re-review requirement |
|---|---|---|
| PFRO-R1 | Open: proposal conflates the reaper's awaited teardown with the guest-flow owner's token-drop completion | Reconcile the two affected descriptions with the existing boundary and observation-only scope. No production change or broader lifecycle finding is authorized. |

## Iteration 1 verdict

**CHANGES_REQUESTED.** The post-FIN read oracle is conclusively falsified and
the required production RST is demonstrated. The chosen replacement
observation preserves D20/DG-10 and adds no justified runtime mechanism.
Approval is blocked only by PFRO-R1's unsupported owner-completion guarantee.
After that bounded documentary correction, the proposal can receive
re-review; user approval, independently reviewed DISTILL alignment, and the
existing STEP 13-01 native verification remain subsequent gates.

---

# Review iteration 2 — PFRO-R1 remediation

## Metadata and reviewed identity

| Field | Value |
|---|---|
| Date | 2026-10-27 |
| Iteration | 2 |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 other blocking findings |
| Native record SHA-256 | `491f0ab4efd3c745351e49786128cc88c8948f05f6f5404bbf8380202dd35716` |
| Replacement design SHA-256 | `922d264b741d218adc5a7812d4b837651bd7e374ed6dfba1257c509b7166842b` |

Re-read both complete revised artifacts and rechecked the exact current
token-drop, registration cancellation/reaper, `process_end`, and existing
cell observation paths. Reconstructing the prior two passages and final blank
line reproduces both iteration-1 artifact hashes exactly. Thus the only
changes since iteration 1 are the PFRO-R1 passages and final blank-line
normalization. Both native raw-log hashes remain unchanged.

## Finding closure and evidence

| Finding | Remediation | Evidence and disposition |
|---|---|---|
| PFRO-R1 — unsupported owner completion guarantee | PFRO-1 item 2 now specifies synchronous registration removal/cancellation, a separate task taking the retained handle and awaiting teardown, and expressly no guest-flow-owner join or new completion guarantee. Item 3 specifies the resulting same-flow RST within the existing cell observation deadline. Native reachability steps 5–6 distinguish the same task boundary. | `design-13-01-postfin-reset-observation.md:22-27`; `native-13-01-postfin-reset-observation.md:55-63`. **Closed.** |

The correction matches the current production path:

- `ForwardedOutbound::drop` removes the matching registration and calls
  `close` synchronously (`mtls_intercept_worker.rs:1347-1363`).
- `ForwardedRegistrationState::close` cancels and may submit a task that takes
  the handle and awaits teardown (`:190-199`); the existing cancellation
  reaper can instead take it and await the same teardown (`:1366-1373`).
- The guest-flow owner's `process_end` awaits its peer/vsock closes and emits
  `guest_flow.closed`; it does not acquire or join a registration teardown
  task (`owner_runtime.rs:1408-1415`).
- The test drops the token and subsequently waits on its existing
  `PEER_TERMINATION_DEADLINE` observation (`vsock_mesh_egress_mtls.rs:1379-1388`).
  The revised item 3 uses that existing test deadline; it adds no production
  timer, drain deadline, completion method or ordering guarantee.

`MtlsEnforcement::teardown` retains its own previously reviewed awaited
effect-completion postconditions. The remediation neither weakens those
postconditions nor promotes the separate guest-flow owner's token drop into
an awaited interface. No implementation change is needed to close this
documentary finding, and none is authorized by its disposition.

## Preserved contract and scope gates

| Gate | Iteration 2 assessment |
|---|---|
| Native reset and oracle conclusion | Preserved: the same leg-B four-tuple sends FIN then RST; endpoint state/error distinguishes reset from EOF. Both raw receipts are byte-identical to iteration 1. |
| D20 / DG-10 outcomes | Preserved: open/half-ended registration drop requires reset; clean completion retains FIN with no later RST; EOF-only after abort remains insufficient. |
| Normative contract diff | No accepted `zero`, `never`, `must`, `exactly`, fail-closed, security, cleanup or reset requirement is removed or relaxed. The removed promise was an unsupported proposed owner-completion assertion, not an accepted invariant. |
| Public/cross-crate API and typed errors | Unchanged. No new completion API, semantic error, payload, mapping, runtime hook, owner or mechanism. Linux `EPIPE` remains endpoint evidence, not a new project error representation. |
| DESIGN / DISTILL ownership | Unchanged: one existing scenario and lane; DISTILL owns test-local observation representation. The correction does not pin helper signatures or authorize public test-support surface. |
| Selective invalidation | Unchanged: STEP 13-01 post-FIN evidence remains pending. The completed STEP 12-01 chain, unrelated D9/D14, remaining DG-10 readings, leg-C/`GuestStream`, LEGS-DIAL and other D20 paths retain their prior approvals and obligations. |
| Roadmap and dependencies | JSON revalidation succeeds. Only `validation` differs from HEAD; status remains `pending`, `approved_at` remains null, prior approval is recorded, and the STEP 13-01 pending scope and existing dependencies are unchanged. |
| Alternatives and primary-source explanation | Unchanged and supported by the already-reviewed native receipts and vendored Linux 7.0 source. No rejected runtime mechanism has been reintroduced. |
| Model / seed gate | No new control-plane schedule claim is made. This remains an observed host-adapter TCP effect; no new model, seeded invariant, or Sim seam is required. |
| Six roadmap checks | Iteration-1 results remain applicable: invocation path and test boundaries are intact, no criterion or decomposition changed, and no implementation code or verbosity expansion was added to the roadmap. |

Read-only checks also repeated the bounded canonical `git diff --check` and
roadmap structural comparison. No new native execution, test edit, production
edit, DES event, evidence rewrite, canonical status change, or commit was
performed. Only this review artifact was appended.

## Iteration 2 final verdict

**APPROVED.** PFRO-R1 is fully remediated. The proposed observation correction
now accurately distinguishes synchronous token removal/cancellation from the
separate task's awaited teardown, while preserving the required same-flow
post-FIN RST and every accepted scope and invariant boundary. No technical
blockers remain.

This approves the bounded DESIGN proposal for the next authority gate. It
does not supply user approval or roadmap execution approval. The proposal
remains proposed and the roadmap remains pending until the recorded user
approval and independently reviewed DISTILL alignment gates are satisfied;
STEP 13-01 still requires its existing native triple/full-suite verification
and subsequent DELIVER review.

---

# Review iteration 3 — canonical PFRO-1 fold

## Metadata and reviewed identity

| Field | Value |
|---|---|
| Date | 2026-10-27 |
| Iteration | 3 |
| Role | Fresh independent replacement solution-architecture reviewer |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 other blocking findings |
| Native record SHA-256 | `694f919056d5b39bf40d0c5468130c0a57a22415f02264ac81bfbb5ba55fc672` |
| Replacement design SHA-256 | `0b9319831636577ca1ce85cc79f6214325d73c94ac92ef6a494c42cfdbfef2ba` |
| Feature delta SHA-256 | `f70569fb7479368d5aab9d74de2e4f898d7daaed55bd9ff3af9542376efc4e9b` |
| Roadmap SHA-256 | `c7d8c8f15dabafaf6ad6e3ea337829aba3dd1821e1403b7c9fe4a0f54bddf309` |
| Native endpoint log SHA-256 | `48a7520ee61c32223482804b4d2a5658f350b64cc7431add9108d87ef9781d7d` |
| Native production capture SHA-256 | `726ce9d1fa823c8c4fdc4ce9d7996f45ff84c775ec4e8d1fe73576261c2dbbf5` |

The original reviewer was no longer addressable, so this iteration was
performed as the repository-authorized fresh replacement review. It read the
current `AGENTS.md`, applicable DESIGN, development, spike and testing rules,
the solution-architect reviewer definition, `nw-sar-critique-dimensions`,
`nw-roadmap-review-checks`, all four canonical artifacts named above, and the
complete retained `increment-a` probe source, scripts and raw logs. Repository
requirements for native Markdown and unlimited remediation iterations override
the installed reviewer definition's legacy YAML and iteration limits.

Only this review artifact was appended. No production source, test, probe,
native evidence, feature delta, roadmap, DES log, execution log or commit was
changed, and no new native or Metal job was run.

## Independent native-evidence assessment

The production capture is exact about direction, ordering and identity. On leg
B, `127.0.0.1:39876 -> 127.0.0.1:43469` emits FIN at
`20:40:17.433683`, then that same source, destination and four-tuple emits RST
at `20:40:22.485909`. The receipt reports zero packet drops. The target flow is
therefore not an ambiguous EOF or a reset from the opposite endpoint: the
existing production leg-B socket itself emits RST after its forwarded FIN.

The isolated endpoint receipt independently records the existing
`SO_LINGER{1,0}` candidate at local TCP state `5` (`TCP_FIN_WAIT2`), followed by
peer state `7` (`TCP_CLOSE`), `SO_ERROR=32` (`EPIPE`) and
`post_fin_recv=0`. The negative-`TCP_LINGER2` and exact-cookie `SOCK_DESTROY`
candidates produce no stronger endpoint result, while `TCP_REPAIR` leaves the
peer in state `8` (`TCP_CLOSE_WAIT`) with no pending error. The evidence thus
supports retaining zero linger and rejects every proposed extra runtime
mechanism.

The vendored Linux 7.0 paths corroborate those receipts:

- kTLS close restores and calls the saved TCP close implementation
  (`net/tls/tls_main.c:365-395`);
- zero linger reaches TCP disconnect, and `tcp_need_reset` includes
  `TCP_FIN_WAIT2` (`net/ipv4/tcp.c:3225-3228,3360-3364,3400-3419`);
- FIN sets receive shutdown and `SOCK_DONE` and moves an established peer to
  `TCP_CLOSE_WAIT` (`net/ipv4/tcp_input.c:4947-4961`);
- RST in `TCP_CLOSE_WAIT` records `EPIPE` and closes the endpoint
  (`net/ipv4/tcp_input.c:4890-4930`); and
- with no copied bytes, receive checks `SOCK_DONE` before `sk_err`, so a later
  receive may continue returning EOF while the typed socket error remains
  available through `SO_ERROR` (`net/ipv4/tcp.c:2793-2803`;
  `net/core/sock.c:1797-1800`).

The raw production and endpoint evidence therefore prove both parts of the
canonical correction: the required reset already occurs, and a second read
after consumed FIN cannot prove or disprove it.

## Canonical contract-fold audit

| Gate | Iteration 3 assessment |
|---|---|
| DG-10 and D20 outcome | Preserved exactly. An open or half-ended claimed loopback flow still requires reset; only a loopback flow ended cleanly in both directions clears linger and closes leg B with FIN and no later RST. EOF-only is not accepted as an abort outcome. |
| Exact post-FIN observation | Pinned without ambiguity: the same leg-B four-tuple emits RST after FIN, and Linux 7.0 exposes the peer transition `TCP_CLOSE_WAIT -> TCP_CLOSE` with pending `SO_ERROR=EPIPE`. A post-EOF read is explicitly excluded as evidence either way. |
| Production mechanism | Unchanged. Existing `SO_LINGER{1,0}` remains the mechanism; no `TCP_LINGER2`, `TCP_REPAIR`, `SOCK_DESTROY`, withheld FIN, retry or fallback is added. |
| Public and cross-crate API | Unchanged. `ForwardedOutbound`, `register_forwarded_outbound`, `OwnedPair` and `MtlsEnforcement::teardown` retain their exact signatures and meanings. No accessor, hook, completion method, port, type or enum variant is introduced. |
| Typed errors | Unchanged. No Overdrive semantic error or mapping is added. Peer `EPIPE` is typed Linux endpoint evidence and does not enter an Overdrive error envelope. |
| Ownership and lifecycle | Unchanged. The guest-flow owner drops the owner-held registration token; the separate registration-owned task takes any retained handle and awaits the existing enforcement teardown. No guest-flow-owner join or completion guarantee is introduced. |
| Body, scenario, lane and model | Unchanged by the fold. The same authored STEP 13-01 cell, Given/When/outcome and Tier-3 suite remain authoritative. DISTILL owns the observation representation needed to align that existing cell; no additional body, scenario, lane, Sim seam, seeded invariant or Quint model is authorized. |
| Contract shape | The bounded-change declaration is complete: the existing registration and two owned endpoints are the mutation universe; peer TCP state/error and same-flow FIN/RST are the verification universe; all named complements remain unchanged. |
| Normative weakening | None. The feature-delta diff removes or relaxes no `zero`, `never`, `must`, `exactly`, fail-closed, security, cleanup, ownership or reset requirement. It replaces only the falsified read-after-EOF evidence premise. |
| Selective invalidation | Correctly bounded to DG-10 reading 2's STEP 13-01 post-FIN observation. The completed STEP 12-01 approval and commits, D9, D14, other DG-10 readings, leg-C/`GuestStream`, LEGS-DIAL and other D20 proofs remain valid. |

The typed-error contract gate is satisfied because PFRO-1 creates no new
project failure condition or caller branch. The lifecycle-gate and formal-model
sections are not reopened because this correction changes neither a lifecycle
gate nor a concurrent protocol. Native evidence discharges the host-adapter TCP
fact directly, without adding a test-only production seam.

## Standing authority and iteration-2 gate correction

The controlling user ruling is exactly:

> “Stop only for a gap that changes the outcome; for anything else, take the design's evident intent and report it after the step.”

PFRO-1 changes no outcome. It records the already-emitted reset using the
observable Linux endpoint and wire facts that can establish it, and leaves the
DISTILL-owned test-support representation to the next reviewed handoff. The
standing ruling therefore already authorizes this canonical evidence-contract
fold. No separate user approval is required.

Iteration 2's final paragraph introduced an additional recorded-user-approval
gate even though its own technical review found no outcome, mechanism, API,
error, owner, scenario, lane or model change. That additional gate is not
supported by the standing ruling and is superseded by this iteration. The
iteration-2 technical approval and PFRO-R1 closure remain valid.

This authority is narrow. It does not authorize any new runtime mechanism,
production or public test-support API, error representation, owner completion
promise, test scenario, lane, model, or weakening of DG-10/D20.

## Roadmap and review checks

The roadmap parses as JSON and retains `validation.status = pending`,
`approved_at = null`, the prior STEP 12-01 approval record, and pending scope
limited to `STEP 13-01 DG-10 reading 2 post-FIN reset observation only`.
STEP 13-01 and its dependent execution therefore remain non-executable. The
only remaining gates for this scoped correction are:

1. independent review of the DISTILL alignment for the existing cell; and
2. exact existing-cell native validation of the aligned observation, including
   the required STEP 13-01 triple/full-suite evidence.

This DESIGN review does not self-approve either gate or roadmap execution.

| Review dimension | Result |
|---|---|
| Architectural bias and simpler alternatives | Pass. The selected path observes the existing effect; all added mechanisms are rejected by native evidence and unnecessary surface. |
| Decision quality and completeness | Pass. Context, exact evidence correction, alternatives, consequences, contract shape, ownership and unchanged boundaries are explicit. |
| Feasibility and testability | Pass. The current Tier-3 boundary can observe same-flow packets or peer terminal state plus pending error without a production seam. |
| Priority validation | Pass. PFRO-1 corrects the proven invalid oracle and does not enlarge the feature around a mechanism that already works. |
| External validity | Pass. The existing registered-loopback host-adapter path remains the invoked path; no new feature invocation is claimed. |
| Acceptance coupling and test boundary | Pass. DESIGN pins observable outcomes while DISTILL retains test-support shape; the integration cell remains in-process and does not become an expectation or built-binary runner. |
| Decomposition, roadmap code and concision | Pass. No step, dependency, criterion, implementation code or scenario is added; the bounded pending-validation notice is precise. |

Read-only verification also confirmed all six canonical and raw-evidence hashes,
the exact packet tuple and direction, vendored Linux 7.0 semantics,
`jq empty` for the roadmap, and `git diff --check` for the bounded canonical
artifacts. No contradictory or ambiguous EOF acceptance remains.

## Iteration 3 final verdict

**APPROVED.** The PFRO-1 canonical fold is correct, complete and bounded. It
preserves the DG-10/D20 reset outcome, records the exact same-flow FIN-then-RST
and terminal `EPIPE` evidence, rejects post-EOF reads as an oracle without
accepting EOF as the outcome, and adds no API, typed error, owner, mechanism,
body, scenario, lane or model.

There are no open DESIGN findings and no further user-approval gate for this
outcome-neutral correction. Roadmap validation remains pending solely on the
independently reviewed DISTILL alignment and exact existing-cell native
validation described above.

---

# Review iteration 4 — PFRO-1 validation completion

## Metadata and evidence identity

| Field | Value |
|---|---|
| Date | 2026-10-27 |
| Iteration | 4 |
| Scope | Close the two PFRO-1 validation conditions left by iteration 3 |
| Verdict | **APPROVED — PFRO-1 VALIDATION COMPLETE** |
| Open PFRO-1 findings | 0 critical, 0 high, 0 other blocking findings |
| DISTILL review | `deliver/review-distill-13-01-postfin-reset-alignment.md` — **APPROVED** |
| DISTILL review SHA-256 | `e9447d4c6b1aa2e977add31e56421a9d9feaafd9db719aa7af93f39ae8bdf0e8` |
| Reviewed test source SHA-256 | `dffc6d6292288197618af686427948cdc241492ca5047930abdfa615ca7e86e0` |
| Native triple receipt SHA-256 | `f0f4863d0253b60cc15e55b0aa3a67a579bd4c74120f1a15e172811764f91ddb` |
| Native twelve-cell receipt SHA-256 | `80824136dcd141f570a51081f7e2bccba28fa8405ec0adf77f7c8b3189c0048a` |

This iteration reviews the original STEP 13-01 crafter's durable native
receipts. It does not author or rerun the tests, claim their DES phases, or
assess the separate package-lane failures the crafter is classifying against
the existing CH55/E6 evidence. Only the two PFRO-1 conditions named in
iteration 3 are decided here.

## Condition 1 — independent DISTILL alignment review

**PASS.** The fresh independent DISTILL review approved the exact source that
the native receipts subsequently executed. The current hashes remain identical
to the review's recorded identities:

| Reviewed artifact | SHA-256 |
|---|---|
| `deliver/distill-13-01-postfin-reset-alignment.md` | `e17381ed8af704ad569ad5da5d139ac2b54666fe89bbb66d2a68bffac156df0d` |
| `vsock_mesh_egress_mtls.rs` | `dffc6d6292288197618af686427948cdc241492ca5047930abdfa615ca7e86e0` |
| `distill/red-classification.md` | `c05cba54432d848d461d38c93019a5be30ff85dbd2bfa9e65e92f74c906df00c` |
| `distill/test-scenarios-p6-real-kernel-mtls-firewall.md` | `6e8f7681f73744fb74788285c5118a1d2a14777da81804c0fc7d07be542258da` |

That review verified the same accepted peer descriptor/session/tuple is used
for the mTLS exchange, FIN consumption and terminal observation; no read occurs
after the first EOF; `TCP_INFO` is polled until exact `TCP_CLOSE`; and
`SO_ERROR` is consumed exactly once and retained. The half-ended assertion
requires exact `TCP_CLOSE` plus `EPIPE`, while the open-drop control requires
reset and both clean-close controls require exact `TCP_CLOSE` plus zero pending
error. It approved no production API, project error, owner, mechanism,
scenario, lane or model change.

## Condition 2 — exact existing-cell native validation

**PASS.** Both original-crafter Metal receipts contain a registered, started
and completed durable run id, a nextest run id, the exact intended test count,
and `exit_code=0` with `cause=command_returned`.

| Receipt | Durable Metal run | Nextest run | Exact result |
|---|---|---|---|
| `.context/netns-density-295/13-01/final-triple-postclippy.log` | `965310ec95cc6ce433407d28` | `305d03fd-81c6-4e4c-886f-b8380b576e3c` | 3 tests run; **3 passed**; 289 skipped; exit 0 |
| `.context/netns-density-295/13-01/final-mesh-12-postclippy.log` | `1410bdb09ac40587eb35ce6b` | `c7378efd-0016-4094-9b6a-61347268031c` | 12 tests run; **12 passed**; 280 skipped; exit 0 |

The triple's selector names the existing open-drop reset control, PFRO-1
half-ended reset cell and clean-close control. The complete suite selector is
exactly `test(vsock_mesh_egress_mtls)` and runs all twelve existing cells. Both
receipts use `cargo xtask metal run`, record the fail-closed native x86_64/KVM
preflight, and execute after workspace synchronization, so they are native
evidence for the reviewed current source rather than a stale local result.

## Outcome and scope audit

| Gate | Completion assessment |
|---|---|
| Half-ended outcome | Preserved and now native GREEN: the same peer that consumed FIN reaches exact `TCP_CLOSE` with retained `EPIPE`; EOF alone cannot pass. |
| Open-drop control | Preserved and native GREEN: the remote side and leg-F forwarder side observe reset before any FIN-only result can satisfy the cell. |
| Clean-close controls | Preserved and native GREEN: both require exact `TCP_CLOSE` with zero pending error, so the PFRO-1 observer does not manufacture a reset. |
| Same flow and resolver complement | Preserved: the reviewed source records one peer session on the original accepted descriptor and retains zero re-resolution. |
| API, error, owner and mechanism | Unchanged: no public/cross-crate surface, typed project error, lifecycle promise or runtime close mechanism was added. Existing `SO_LINGER{1,0}` remains sufficient. |
| Scenario, lane and model | Unchanged: one existing cell was aligned inside the existing twelve-cell Metal suite; no scenario, lane, Sim seam or Quint model was added. |
| Contract weakening | None: reset remains mandatory for open and half-ended drops; clean two-direction completion remains the sole FIN-only case. |
| Preserved history | STEP 12-01's completed approval and commit chain, D9, D14, other DG-10 readings, leg-C/`GuestStream`, LEGS-DIAL and other D20 proofs remain untouched. |

The native results close the observation gap without changing the accepted
DG-10/D20 outcome. They validate the reviewed endpoint representation; they do
not convert permanent EOF into acceptable reset evidence.

## Roadmap validation disposition

The two conditions that iteration 3 left pending are now independently and
durably satisfied:

1. the DISTILL source alignment is independently **APPROVED**; and
2. the exact existing-cell native triple and complete twelve-cell suite are
   both **GREEN** on the synchronized Metal source with durable exit receipts.

The roadmap's scoped PFRO-1 validation may therefore move from `pending` to
`approved`, with this completion and the two durable runs recorded in its
reviewer field. That status change closes only the roadmap's explicit
`STEP 13-01 DG-10 reading 2 post-FIN reset observation` pending scope. It is
not a claim that STEP 13-01 as a whole is GREEN, reviewed or complete. The
original crafter's separate classification and resolution of the 58
package-lane failures, remaining DELIVER gates, DES completion, commit and
fresh step review remain outside this bounded verdict.

Read-only verification confirmed every hash above, the current reviewed test
source identity, both Metal registration/start/completion chains, both nextest
summaries and exit receipts, `jq empty` for the roadmap, and scoped
`git diff --check`. No source, evidence, DES event or commit was changed.

## Iteration 4 final verdict

**APPROVED — PFRO-1 VALIDATION COMPLETE.** Both pending conditions are met by
the independently approved DISTILL alignment and the original crafter's exact
3/3 and 12/12 native Metal receipts. The corrected proof observes
`TCP_CLOSE` plus one retained `SO_ERROR=EPIPE` on the same peer after FIN,
preserves every reset and clean-close outcome, and introduces no API, typed
error, owner, mechanism, scenario, lane or model.

The scoped roadmap validation gate may be marked approved. No broader STEP
13-01 GREEN or completion verdict is issued by this review.
