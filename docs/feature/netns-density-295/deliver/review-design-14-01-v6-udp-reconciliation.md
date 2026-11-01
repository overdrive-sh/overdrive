# Independent DESIGN review — STEP 14-01 V6 Service-VIP UDP reconciliation

## Current disposition

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261030_step_14_01_v6_udp_reconciliation` |
| Date | 2026-10-30 |
| Role | Fresh independent solution-architecture reviewer |
| Iteration | 2 |
| Reviewed proposal | `deliver/design-14-01-v6-udp-reconciliation.md` |
| Reviewed roadmap edit | `deliver/roadmap.json` validation block only |
| Verdict | **APPROVED FOR USER DECISION** |
| Blocking findings | 0 — F-01 through F-03 closed in iteration 2 |
| Production, test, DES or accepted-contract authority | **None** |
| Roadmap authority | **PENDING / NON-EXECUTABLE** |

This review applies the accepted V6 and D12 feature-delta contracts, ADR-0145,
ADR-0149, ADR-0164 and ADR-0172, the repository's production-reachability,
native-falsification, no-unrequested-scope, exact-interface and no-weakening
rules, and the nWave architecture and roadmap review checks. The repository's
required native-Markdown artifact governs over the reviewer definition's
legacy YAML response shape.

The review is bounded to the Service-VIP UDP clause of V6 and STEP 14-01
criterion 3. It independently reads the real public submission, allocation,
backend publication, hydrator, cgroup `connect4`, and guest datagram-owner
paths. It does not change the proposal, roadmap, feature delta, ADRs,
production/test source, DES state or evidence, and it does not run Metal,
Lima, mutation testing or a new acceptance body.

## Review result at the contract boundary

The proposal is correct on four central points:

1. V6 binds **the dialing client** to a VM Service. It does not require the
   Service-VIP UDP target to be the same peer VM Service used by the mesh-name,
   TCP/mTLS, remote-inbound and probe outcomes. The exact accepted sentence is
   at `feature-delta.md:13180`; roadmap criterion 3 preserves the same grammar
   at `deliver/roadmap.json:1506`.
2. The iteration-7 DISTILL topology added an unauthorized identity equality by
   making the peer VM Service's issued VIP and UDP listener the V6 UDP target
   (`review-distill-14-01-native-support.md:1547-1559,1650-1656`). D12 and
   ADR-0164 make that target invalid: inbound UDP service to VMs is deferred,
   and a connected UDP peer inside `GUEST_NODE_PREFIX` is refused `Policy`.
3. Receipt `012109f99448d662b35c1760` proves that exact public-deploy topology
   produces no UDP reply after the client VM has already completed DNS,
   off-node TCP/UDP, mesh-name TCP and issued-VIP TCP. It does **not** expose
   the owner's post-connect peer or typed refusal, so it does not uniquely
   prove that ADR-0164's post-`connect4` `Policy` branch was the stage that
   ended the attempt.
4. No source or test change may be inferred from that failure. The accepted
   contracts provide no UDP-to-VM path, and the native receipt is not authority
   to invent one.

The proposal is not yet decision-ready. The independent production audit
resolves its conditional “if an existing path is found” branch in the
negative, and it finds that “outside `GUEST_NODE_PREFIX`” is a necessary but
insufficient target predicate for the accepted `connect4` proof. A user choice
is therefore unavoidable, but the current options require correction before
they are surfaced.

## Independent production-reachability audit

### Public Service submission is VM-only

The public Service payload carries `WorkloadDriverV1`, whose only variant is
`Vm(Vm)` (`crates/overdrive-core/src/aggregate/mod.rs:133-159,345-367`). The
HTTP/JSON submission shape flattens the same `DriverInput`, whose only accepted
table is `[vm]` (`aggregate/mod.rs:836-848`; `api/submit.rs:51-73`). The CLI's
public Service deploy path projects only `ParserDriverInput::Vm` into that
wire shape (`crates/overdrive-cli/src/commands/deploy.rs:601-665`). There is no
public host-process, external-endpoint, static-backend or other non-VM Service
variant.

The current P6-S13 body follows that exact path. It deploys the peer as a VM
Service with a UDP listener, reads that Service's issued VIP and leased
`workload_addr`, then passes the issued VIP to the separately deployed client
VM Service (`vsock_support.rs:282-333`; `vsock_walking_skeleton.rs:102-159`).

### Every production Running allocation publishes the guest transport address

`WorkloadLifecycle` initially emits `AllocationSpec.transport = None` because
the reconciler is transport-neutral (`workload_lifecycle.rs:1307-1333`). The
production action shim does not leave it that way. Before `Driver::start`, it
assigns and provisions a `GuestNetworkPlan`, then sets
`spec.transport = Some(plan.assignment().clone())`; without the production
guest owner, production dispatch fails closed
(`action_shim/mod.rs:1445-1484,2186-2245`). `AllocationSpec` itself documents
that `None` is for sim-driver fixtures and production `StartAllocation` always
sets the transport (`overdrive-core/src/traits/driver.rs:322-326`).

After a successful start, the Running row writes
`spec.transport.workload_addr()` (`action_shim/mod.rs:2445-2475`). Under the
accepted guest transport that address is in `GUEST_NODE_PREFIX`. The raw native
receipt confirms the real rows as `100.95.0.4`, `.5` and `.6`; it does not
contain a production Running allocation with `workload_addr = None`.

### The backend publisher has no second production source

`ServiceLifecycle` projects each allocation row's address, using `host_ipv4`
only for `workload_addr = None` (`service_lifecycle.rs:852-919`). It publishes
only Running allocations as each listener's `Backend`
(`service_lifecycle.rs:1027-1103`). Repository-wide production-source search
finds this as the only producer of `Action::WriteServiceBackendRow`; the other
`ServiceBackendRow` constructors and direct writes are test fixtures or
consumers. Consequently the fallback cannot materialize a public host backend:
the public model cannot submit one and the production start path does not
produce the required `None` row.

### An outside-prefix backend is not by itself a `connect4` target

The V6 clause requires the cgroup `connect4` path and specifically confirms
that the attachment at `overdrive.slice` rewrites owner sockets
(`feature-delta.md:13180`). The production hydrator registers a backend into
the cgroup hook's `LOCAL_BACKEND_MAP` only when its IPv4 address equals the
node's `host_ipv4` (`service_map_hydrator.rs:336-371,442-475`). Other
outside-prefix backends enter the remote XDP/REVERSE_NAT projection instead
(`:373-421`).

The `cgroup_connect4_service` program consults only `LOCAL_BACKEND_MAP`; on a
miss it permits the original VIP connect unchanged
(`crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:48-101`). It
does not consult the XDP `SERVICE_MAP`. Therefore a backend merely outside
`GUEST_NODE_PREFIX` does not satisfy the accepted owner-socket rewrite. For the
unchanged current architecture, the required target must be a publicly
materialized Service backend at the node's `host_ipv4`, with its listener
registered through the normal publisher/hydrator path. A different
outside-prefix selection rule would itself be a separate architecture change.

### Reachability conclusion

No current `serve` + public-deploy path materializes that target. The only
public Service driver is VM; every production VM start publishes a guest-prefix
address; the only production backend publisher consumes that address; and the
hydrator's local cgroup path requires `host_ipv4`.

The proposal's conditional topology-only DISTILL route is therefore not an
available next step. DISTILL cannot discover private fixture HOW that changes
those production facts without either manufacturing product state forbidden
by criterion 4 or introducing an unapproved product contract. STEP 14-01 must
remain stopped for a user choice.

## Findings

### F-01 — blocking: the conditional no-approval branch is disproved and its target predicate is too weak

**Severity:** Critical / blocking  
**Location:** proposal lines 23-37, 208-237 and roadmap pending scope line 1718  
**Status:** Open

The proposal correctly says its own audit found no public non-VM Service path,
but still delegates a second search to DISTILL and authorizes topology-only
correction if it finds any backend outside `GUEST_NODE_PREFIX`. The independent
audit above closes that condition as false in the current production graph.
It also proves that “outside the guest prefix” is insufficient: the accepted
owner-socket `connect4` proof needs a backend in the hydrator's local partition,
which today means `backend.ip == host_ipv4`.

Leaving the condition open creates two invalid handoffs. A DISTILL author could
manufacture `workload_addr = None` or a backend row and call it topology, or
could select a remote outside-prefix address that never enters
`LOCAL_BACKEND_MAP`. Both would satisfy the proposal's words while failing the
accepted public-composition outcome.

**Required remediation:**

1. Remove the conditional “DISTILL may proceed if it finds an existing path”
   authority and state that the production audit found no such path.
2. Name the missing contract exactly: no public Service submission can produce
   a Running backend at `host_ipv4` for the ServiceLifecycle →
   ServiceMapHydrator → `LOCAL_BACKEND_MAP` → owner-socket `connect4` path.
3. Amend the roadmap `pending_scope` accordingly. Keep status `pending` and
   preserve all prior approvals, but require the user choice rather than a
   further DISTILL search.
4. Do not authorize a direct map write, injected observation row,
   `workload_addr = None`, private publisher, adapter override or fabricated
   reply as a substitute.

### F-02 — blocking: the #310 option prescribes an unproven mechanism and uses the wrong logical dependency

**Severity:** High / blocking  
**Location:** proposal lines 82-97, option B at line 247  
**Status:** Open

ADR-0149 says carrying a peer address in each frame is needed to **multiplex
peers behind one association**, and rejects that alternative for this feature
(`adr-0149...md:48-52`). It does not prove that every possible inbound-UDP
design must use that frame or multiplexing shape. A future #310 design owns the
choice of association identity, peer-address representation, intake owner,
reply routing and lifecycle. This reconciliation may not choose those contracts
in advance.

The proposal also says reaching the peer VM requires relaxing D12/ADR-0164
“or” adding the deferred mechanism. A mechanism cannot replace permission to
cross the accepted security/scope boundary. Option B necessarily requires an
explicit supersession of D12 and ADR-0164 **and** a separate DESIGN that proves
the exact #310 mechanism. Whether that later design changes the existing frame
is not established here.

**Required remediation:** describe option B only at its approved decision
boundary: include inbound UDP service to VM backends in this feature, explicitly
supersede the two prohibitions if the user selects it, and return the exact
mechanism, interfaces, typed errors, ownership, lifecycle, formal model and
native proof to a separate DESIGN. Remove the claim that ADR-0149 has already
selected the necessary wire mechanism.

### F-03 — blocking: the user options lack a recommendation and exact normative weakening quotes

**Severity:** High / blocking  
**Location:** proposal lines 239-253  
**Status:** Open

The option table usefully separates four outcome directions, but it is not yet
safe to present as an approval request. It gives no recommendation, and its
weakening options summarize rather than quote the exact terms the user would
authorize changing. Under the repository's no-weakening rule, words such as
“through `serve` + `deploy`,” “No test installs,” and “Inbound UDP service to
VMs deferred” cannot disappear behind option labels.

The corrected decision surface must make these consequences explicit:

| Option | Exact disposition required before user presentation |
|---|---|
| **A — production host-local Service backend path** | **Recommended** because it is the only listed direction that preserves both accepted V6 and D12. State that the target must be publicly materialized at `host_ipv4` through the production publisher/hydrator so the existing `LOCAL_BACKEND_MAP` path rewrites the owner socket. Choosing A authorizes only a separate DESIGN; it does not approve a driver variant, endpoint resource, lifecycle/API shape or typed error set. |
| **B — include #310** | Quote D12's “Inbound UDP service to VMs deferred” and ADR-0164's “If the connected socket's kernel peer lies in the guest prefix, the association is refused (`Policy`, #310)” as the exact accepted prohibitions to supersede. State that a separate DESIGN must choose the mechanism; do not promise a wire change before that design. This preserves V6 but relaxes an accepted security/scope boundary and is materially larger than A. |
| **C — weaken evidence** | Quote V6's “Walking skeleton through `overdrive serve` + `overdrive deploy`” and roadmap criterion 4's “No test installs, binds or programs anything production does not; ... no override seam.” State exactly that a direct map/backend fixture would replace those clauses for Service-VIP UDP and would no longer validate the public Service lifecycle. |
| **D — defer/remove the outcome** | Quote and remove V6's “and a service VIP over UDP (`connect4`; includes the confirmation that the ADR-0053 attachment at `overdrive.slice` rewrites owner sockets in `overdrive.slice/control-plane.slice`)” and the corresponding roadmap criterion-3 clause. This is the smallest implementation scope and a direct observable-outcome weakening. |

The recommendation follows the repository's correctness precedence, not an
implementation-cost preference. If the user prefers bounded scope over the
accepted V6 outcome, option D is the honest smaller choice, but only the user's
exact approval can make that weakening valid. Option C retains nominal outcome
text with weaker evidence and therefore carries the highest evidence-integrity
risk. Option B expands #310 and changes the largest security, ownership and
protocol surface.

## Actor, object, state and boundary audit

| Concern | Review disposition |
|---|---|
| Dialing actor | **Exact:** the client VM Service guest application owns the V6 DNS, mesh-name, mesh-VIP TCP, Service-VIP UDP and non-mesh dials. |
| Peer VM Service | **Exact but not universal:** it may remain the target for mesh-name/TCP, remote inbound, probes, state control and retained TCP population. V6 does not require its UDP listener to be the Service-VIP UDP target. |
| UDP target object | **Correction required:** it is a distinct Service/VIP/listener/backend relation, but under the unchanged production cgroup path its selected local backend must be `host_ipv4`, not merely outside `GUEST_NODE_PREFIX`. |
| Allocation state | **Exact:** only Running allocations enter the backend row; production VM Running rows carry their guest transport `workload_addr`. |
| Publication owner | **Exact:** `ServiceLifecycle` owns the production `ServiceBackendRow`; the observation store and hydrator consume it. A test or DISTILL helper is not a second owner. |
| Selection boundary | **Exact after F-01:** `ServiceMapHydrator` owns local-vs-remote classification; `LOCAL_BACKEND_MAP` alone feeds the cgroup `connect4` rewrite. |
| Datagram owner | **Exact:** the guest-flow owner opens and connects the host UDP socket, reads its kernel peer, and refuses a guest-prefix peer. It has no TCP resolver/leg-S branch for datagrams. |
| Evidence boundary | **Exact:** native Metal through the built product, real `serve` and public deploy. A private map/programming seam cannot inherit this credit. |

## Error, lifecycle, formal-model and security gates

| Gate | Assessment |
|---|---|
| Typed semantic errors | No new product error is authorized by this reconciliation. Test-protocol `UDP_TIMEOUT` is receipt evidence only. Any future option-A/B interface must pin its own complete typed error set before DISTILL. |
| Lifecycle Gate Ownership | Not applicable while this artifact only stops the invalid topology and requests a user direction. A selected option that adds or changes a production mechanism requires its own gate matrix and failure projection. |
| Formal protocol model | Not applicable to the stop/disposition itself. Option B necessarily returns to DESIGN for concurrent association/ownership analysis; whether it changes the current wire is not pre-decided here. |
| Security | D12, ADR-0164's post-connect guest-prefix refusal and ADR-0172's constant fail-closed rules remain exact. No implementation may relax them without the user's option-B approval followed by an exact replacement design. |
| Native falsification | The receipt falsifies the DISTILL topology/execution premise. It is consistent with, but does not independently prove every internal stage of, the accepted D12/ADR boundary. No speculative post-`connect4` fix is authorized. |

## Roadmap review checks

| Mandatory check | Result |
|---|---|
| External validity | **FAILED for execution; correctly pending.** With the current public model there is no invocable target for the Service-VIP UDP action. F-01 must replace the conditional path with the user-decision gate. |
| AC implementation coupling | **Pass.** Criteria 3 and 4 state the accepted observable production journey and evidence boundary. The cgroup `connect4` mechanism is already an accepted V6 contract, not a new private algorithm. |
| Step decomposition ratio | **Pass / unchanged.** No roadmap step or production file list is added by the pending validation edit. |
| Implementation code | **Pass.** The scoped validation block contains no algorithm or source prescription. |
| Concision and precision | **Needs F-01 correction only.** The validation block is concise, but “outside-guest-prefix” is not precise enough for the existing cgroup path and its conditional DISTILL route is no longer factual. |
| Unit/integration boundary | **Pass.** The accepted P6-S13 boundary is the real production composition. This review rejects private state injection and test-owned product programming. |

The validation status must remain `pending`. The nested `prior_approval` chain
correctly preserves STEP 13-02, PFRO-1 and completed STEP 12-01 authority; this
bounded V6 defect does not revoke them. No `approved_at` may be set for the
current V6 disposition.

## Architecture-review dimensions

| Dimension | Assessment |
|---|---|
| Architectural bias | No technology-preference or novelty bias. The proposal correctly avoids inventing a UDP-to-VM mechanism to make one test green. |
| Decision quality | The accepted ADR context and consequences are sufficient. F-02 blocks the proposal from turning ADR-0149's rejected alternative into a preselected #310 mechanism. |
| Completeness | F-01 and F-03 block: the real missing local-backend contract and decision-ready normative diffs are incomplete. |
| Feasibility | The conditional existing-path route is infeasible in the current production graph. Each user option other than D requires a later exact design or explicit evidence-boundary amendment. |
| Priority | Correct. The reconciliation is limited to the one native-falsified V6 Service-VIP UDP premise and preserves earlier approved work. |
| Testability / effect isolation | Correctly refuses a production seam whose only producer would be a test. No new component contract shape arises until the user chooses an option. |

## Iteration 1 verdict

**CHANGES_REQUESTED — USER DECISION REQUIRED.** The actor reading, D12/ADR
boundary, native non-uniqueness and stop disposition are correct. The fresh
production audit proves there is no existing public path for a topology-only
DISTILL correction. It also narrows the missing target from a generic
outside-prefix backend to a publicly materialized `host_ipv4` backend that the
existing hydrator registers in `LOCAL_BACKEND_MAP` for the owner-socket
`connect4` rewrite.

Remediate F-01 through F-03 in the proposal and roadmap validation block, then
surface the corrected A-D decision to the user. Recommend A as the sole listed
direction that preserves both accepted V6 and D12, while making clear that A
starts a separate DESIGN and authorizes no API or implementation. Quote every
normative weakening for B, C and D. Until the user explicitly chooses a
direction and any required replacement design is independently reviewed,
STEP 14-01 remains stopped at Service-VIP UDP and no affected source, test,
DES event or accepted contract may change.

# Iteration 2 re-review — decision package

## Iteration 2 identity and scope

| Field | Value |
|---|---|
| Proposal SHA-256 | `6d21f6e973de5fa81f62d9229c7d1d0fb7a99d7785194066c03a5f55c630f1b0` |
| Roadmap SHA-256 | `ff2c3dbf2b308f65d32bfbd9bf363296879f548149ceba64a3c6b46a235760e8` |
| Re-reviewed findings | F-01, F-02 and F-03 only |
| New blocking findings | 0 |
| Verdict | **APPROVED FOR USER DECISION** |

The hashes match the exact artifacts supplied for iteration 2. This re-review
checks only the requested remediation and the roadmap validation disposition.
It does not reopen accepted V6/D12 semantics, earlier roadmap approvals or the
production-reachability proof established in iteration 1. No executable source,
test, DES state, evidence, accepted contract or commit changed in this review.

## F-01 remediation — exact production target and no conditional DISTILL route

**Disposition: CLOSED.**

The revised proposal now names the exact missing contract at lines 23-35 and
229-242:

- the Service-VIP UDP target must be a publicly materialized Service backend
  at the node's `host_ipv4`;
- `ServiceLifecycle` must publish that backend;
- `ServiceMapHydrator` must place it in `LOCAL_BACKEND_MAP`; and
- the existing owner-socket `connect4` hook must consume that entry.

It explains why the exact address matters at lines 127-144: the hydrator's
local partition is `backend.ip == host_ipv4`, while every other outside-prefix
backend enters the remote XDP/REVERSE_NAT projection and does not feed the
cgroup hook. A generic outside-prefix target is no longer presented as
sufficient.

The proposal also closes the conditional branch. Lines 30-35 state that the
completed audit found no `serve` + public-deploy materialization path and that
“There is no remaining topology-only DISTILL route.” Lines 231-242 name the
absence again and forbid a direct map write, injected `ServiceBackendRow`,
test-only `workload_addr=None`, private publisher, adapter override or
fabricated reply. No remaining clause authorizes DISTILL to search for or
manufacture a substitute.

The roadmap validation block carries the same exact boundary. It remains
`pending`; its `pending_scope` names the
`ServiceLifecycle -> ServiceMapHydrator -> LOCAL_BACKEND_MAP -> owner-socket
connect4` path, requires the user's A-D choice, and rejects every fixture-only
substitute. F-01 is fully remediated.

## F-02 remediation — #310 permission and mechanism remain separate

**Disposition: CLOSED.**

Lines 92-98 now use the correct conjunction: making UDP reach a peer VM
requires **both** explicit approval to supersede D12 and ADR-0164's
guest-prefix refusal **and** a separate DESIGN for #310's mechanism. The text
states that ADR-0149 records only one rejected peer-address/multiplexing
alternative and does not select the future design.

Option B repeats that boundary at lines 271-292. It quotes the two accepted
prohibitions, defines approval as scope/security supersession plus authority to
run a separate DESIGN, and assigns association identity, peer-address
representation, intake ownership, reply routing, lifecycle, interfaces, typed
errors, formal modeling and native proof to that later design. It expressly
states that this record does not preselect a frame, peer-address or
multiplexing mechanism. No wire or association architecture is invented by
this reconciliation.

## F-03 remediation — recommendation and exact weakening surface

**Disposition: CLOSED.**

The decision package now recommends option A because it is the only listed
direction that preserves exact V6 and D12. Lines 246-269 limit approval of A to
authority for a separate DESIGN of the missing host-local Service-backend
capability. They explicitly withhold selection or approval of a driver variant,
external-endpoint resource, lifecycle, owner, API, wire/persisted shape, typed
error set, implementation, test or roadmap step.

Options B-D expose their normative changes directly:

| Option | Exact quoted disposition | Assessment |
|---|---|---|
| B | D12: “Inbound UDP service to VMs deferred”; ADR-0164: a connected guest-prefix peer is refused `Policy` | Exact prohibitions to supersede are visible; later mechanism remains unselected. |
| C | V6: “Walking skeleton through `overdrive serve` + `overdrive deploy`”; roadmap criterion 4: “No test installs, binds or programs anything production does not; ... no override seam.” | Exact evidence weakening and loss of public-Service-lifecycle proof are visible. |
| D | V6's complete Service-VIP UDP / `connect4` owner-socket clause and criterion 3's “and a service VIP over UDP” | Exact observable outcome removal and required canonical amendments are visible. |

The trade-offs are explicit: A preserves both accepted contracts; B preserves
the V6 outcome by relaxing the D12/ADR-0164 security/scope boundary and has the
largest protocol/ownership impact; C retains nominal behavior with the
highest evidence-integrity risk; D has the smallest implementation scope and
directly weakens the observable outcome. The user can approve or reject one
direction without any mechanism being silently bundled into that choice.

## Roadmap validation re-review

| Check | Iteration 2 result |
|---|---|
| Targeted scope | **Pass.** Only STEP 14-01 criterion 3 and feature-delta V6 Service-VIP UDP are pending. |
| Execution gate | **Pass.** Status remains `pending`; no `approved_at` is set and no execution authority is implied. |
| Prior approvals | **Pass.** The nested chain preserves STEP 13-02 PRCI-1, PFRO-1 and completed STEP 12-01 exactly. |
| External validity | **Pass for the decision gate.** The block states that no current invocable public target exists and requires a user direction before any replacement design or contract amendment. |
| Precision | **Pass.** `host_ipv4` and the complete production owner/map path replace the insufficient outside-prefix predicate. |
| Test boundary | **Pass.** Direct map programming, injected backend rows, test-only fallback state, private publishers, adapter overrides and fabricated replies are expressly forbidden. |

The roadmap remains intentionally non-executable. Approval of this decision
package does not restore roadmap approval; the selected option's required
design or canonical amendment must complete its own approval and review gates.

## Normative contract and authority audit

| Concern | Iteration 2 disposition |
|---|---|
| Existing V6 and D12 | Unchanged until the user explicitly chooses B, C or D. |
| ADR-0164 and ADR-0172 security boundary | Fail-closed and unchanged; no GREEN exception or guest-prefix tuple allowance is authorized. |
| Public/cross-crate API | No new surface selected or approved. |
| Typed errors | No new product error selected; any option-A/B contract must pin its semantic error set in its later DESIGN. |
| Lifecycle ownership | No gate or state meaning changes in this decision package; later production mechanisms must supply their own exact gate analysis. |
| Formal model | No protocol changes in this package; option B returns concurrent association/ownership choices to a later DESIGN. |
| Evidence | The native receipt proves failure of the peer-VM topology but still does not identify a unique internal stage. |
| Downstream authority | User decision only. No production, test, DISTILL, DES, roadmap-execution or commit authority follows from this verdict. |

## Iteration 2 verdict

**APPROVED FOR USER DECISION.** F-01, F-02 and F-03 are closed. The revised
proposal accurately states that no current public-deploy path can materialize
the required `host_ipv4` Service backend; removes the conditional DISTILL
escape; separates #310 permission from mechanism design; recommends A without
selecting an API or architecture; and quotes the exact normative changes for
B, C and D. The roadmap remains narrowly pending and preserves every earlier
approval.

The next valid action is to present options A-D exactly as recorded and obtain
the user's explicit choice. This verdict approves that decision request only.
STEP 14-01 remains stopped at Service-VIP UDP until the selected option's
required separate DESIGN or canonical amendment is approved and independently
reviewed.
