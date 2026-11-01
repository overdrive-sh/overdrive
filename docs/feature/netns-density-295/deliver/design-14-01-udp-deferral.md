# STEP 14-01 Service-VIP UDP deferral to #310

## Status

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-01` |
| Date | 2026-10-30 |
| Role | Bounded DESIGN contract correction after the reviewed V-6 UDP reconciliation |
| User decision | **OPTION D APPROVED — DEFER THE SERVICE-VIP UDP ACCEPTANCE ACTION TO #310** |
| Canonical amendment | Written in `feature-delta.md` V-6 and `deliver/roadmap.json` criterion 3 |
| Validation | **PENDING fresh independent DESIGN review** |
| Execution authority | **None until that review and the resulting DISTILL alignment complete** |
| Production/API change | **None** |

This record applies the user's selected option D from
`design-14-01-v6-udp-reconciliation.md`, after the independent review in
`review-design-14-01-v6-udp-reconciliation.md` approved the A–D decision
package for user choice. It changes only #295's V-6 acceptance obligation to
deliver UDP through a Service VIP to a VM backend. It does not add a
replacement backend type, endpoint resource, transport, API, wire field,
error variant or test seam.

## Explicit user approval

The exact approval is:

> “yeah, let's not do that now. add all the relevant info to the github issue,
> and then we keep it deferred to #310 and correct the contradictory
> acceptance checklist.”

This approval selects option D and authorizes the exact observable-outcome
removal quoted in that reviewed option. It does not authorize implementation
of #310 or any relaxation beyond the two clauses below.

## Normative contract diff

### Feature-delta V-6

The pre-decision V-6 row required:

> “and a service VIP over UDP (`connect4`; includes the confirmation that the
> ADR-0053 attachment at `overdrive.slice` rewrites owner sockets in
> `overdrive.slice/control-plane.slice`)”

That complete clause is removed from #295 V-6. The corrected V-6 row keeps
the production `overdrive serve` + `overdrive deploy` boundary and explicitly
records that the Service-VIP UDP acceptance action remains deferred to #310
under D12. D24's general contract for a guest datagram to a Service VIP is not
deleted or changed: the owner still uses the connected host UDP socket and the
existing `connect4` rewrite, and a selected kernel peer inside the guest prefix
is still refused `Policy`.

### STEP 14-01 roadmap criterion 3

The pre-decision criterion required:

> “and a service VIP over UDP”

That clause is removed. The corrected criterion states that Service-VIP UDP
delivery to a VM backend is excluded from STEP 14-01 and remains deferred to
#310 under D12. Criterion 4's production-composition and no-override boundary
is unchanged.

### Normative-term audit

The approved weakening is limited to the two quoted clauses. No other
`must`, `never`, `zero`, `exactly`, `required`, `forbidden` or fail-closed
term is removed or relaxed. In particular:

- D12, “Inbound UDP service to VMs deferred,” remains accepted;
- ADR-0164's guest-prefix `Policy` refusal remains fail-closed;
- the VM remains NIC-less and vsock-only;
- the production-composition evidence boundary remains `serve` + public
  `deploy`, with no direct map write, injected backend row, private publisher,
  adapter override or fabricated reply; and
- cleanup, source identity and loss-accounting requirements remain mandatory.

## Outcomes that remain mandatory in #295

The deferral is not a global removal of UDP and does not convert V-6 into a
TCP-only journey.

| V-6 / STEP 14-01 outcome | Disposition |
|---|---|
| VM Service dials a mesh peer by name through guest DNS at `gateway:53` | **Required, unchanged** |
| VM Service reaches a non-mesh address over TCP and UDP through the real NIC | **Required, unchanged** |
| A genuine empty UDP datagram round-trips and the remote peer observes the node source address | **Required, unchanged** |
| VM Service reaches the mesh Service VIP/frontend over TCP with mTLS through leg F | **Required, unchanged** |
| Remote client reaches the VM over inbound TCP through leg C → leg S → `GuestStreamDial` | **Required, unchanged** |
| TCP and HTTP probes over the dial follow the application's healthy → unhealthy → healthy state | **Required, unchanged** |
| V-LEGB-SERVE proves the same-node TCP path with and without a default route, leg C sees `127.0.0.1:<ephemeral>`, and zero matching packets leave on the uplink | **Required, unchanged** |
| Concurrent `IP_BIND_ADDRESS_NO_PORT` set/unset comparison | **Required, unchanged** |
| Public stop, VMM disappearance, owner/socket/route/interface cleanup and the existing 117-body accounting | **Required, unchanged** |
| Service-VIP UDP delivery to a VM backend | **Deferred to #310; not a #295 V-6 / STEP 14-01 acceptance action** |

The client and peer may remain distinct VM Services for the remaining
mesh-name, TCP/mTLS, inbound-TCP, probe-state and population outcomes. No
non-VM Service backend is introduced to preserve the removed UDP action.

## #310 follow-up contract boundary

[GH #310](https://github.com/overdrive-sh/overdrive/issues/310) remains the
owner of “UDP service delivery to VM workloads (inbound UDP from many
clients).” The existing issue already requires a kernel-only path, native
Metal validation and no userspace payload relay. The failed STEP 14-01 native
run adds evidence that a publicly deployed VM Service's issued VIP and UDP
listener do not supply that path under #295: after DNS, off-node UDP,
mesh-name TCP and TCP-to-VIP succeeded, the peer VM observed no UDP request and
the client timed out.

The future #310 DESIGN must resolve these contract questions before any
implementation or acceptance test is authorized:

1. **Association identity:** what exact tuple or owned identity distinguishes
   one inbound client association, how it is created, and when it is released
   or reused.
2. **Peer-address representation:** how the remote client's source address is
   carried across the VM boundary so the guest application's ordinary UDP
   receive semantics are preserved.
3. **Reply routing:** how a guest reply selects the correct remote client and
   preserves the Service VIP as the client-observed reply source without an
   ambiguous or test-owned route.
4. **First datagram:** how the first inbound datagram creates or finds its
   association and is delivered once, without requiring a second datagram,
   being silently dropped, duplicated or delivered to a later owner.
5. **Many clients:** how simultaneous clients of the same Service VIP and UDP
   listener remain isolated, including collision, capacity, late-datagram,
   teardown and reuse behavior.
6. **Interface and failure contracts:** which production owner holds the
   association, the exact lifecycle states and cleanup authority, and the
   semantic typed errors/counters callers and operators branch on.

Those questions are constraints, not a selected architecture. ADR-0149 and
ADR-0147 record peer addressing inside a datagram frame as one alternative
rejected for #295; they do not preselect it for #310. This correction likewise
does not choose one association per client, one multiplexed association, a new
frame, a new port, a new owner method or any public surface.

The #310 solution must preserve the accepted transport boundary unless a
later user-approved design explicitly changes it:

- packet and application payload forwarding is kernel-only on both sides;
- the VM gains no guest NIC, TAP, bridge port or per-allocation host netdevice;
- Overdrive userspace may manage fixed-size control and lifecycle state but
  does not relay UDP application payload; and
- native Metal evidence covers first-datagram delivery, reply identity,
  multi-client isolation and cleanup through the production composition.

This artifact records the material that must be added to #310. The GitHub
issue update is owned by the parent orchestrator and is not claimed as
completed here.

## Acceptance-document reconciliation

The current acceptance checklist is corrected as follows:

- `deliver/distill-14-01-v6-coverage.md` marks the Service-VIP UDP row
  deferred to #310, removes it from the client/peer Service action list and
  retains every other V-6 and V-LEGB-SERVE oracle.
- `deliver/distill-14-01-native-support.md` retains the failed native run as
  production evidence but no longer directs the STEP 14-01 crafter to invent
  or repair a UDP-to-VM path.
- Historical DELIVER and DISTILL review artifacts remain unchanged as records
  of the contract and evidence that existed when those iterations ran. Their
  Service-VIP UDP blocker is superseded only by this explicit user-approved
  contract correction; their other findings and evidence remain live.

No executable source, test body, DES event or execution log is changed by
this DESIGN correction. After fresh independent DESIGN review, DISTILL owns
the bounded test/checklist alignment, then the original STEP 14-01 crafter
and reviewer resume against the corrected contract.

## Interface, error, lifecycle and review gates

- **Public/cross-crate API:** unchanged for #295.
- **Wire and persisted formats:** unchanged for #295.
- **Typed errors:** unchanged for #295. `UDP_TIMEOUT` in the native fixture is
  evidence, not a new product error contract. #310 must pin its semantic error
  set before implementation.
- **Lifecycle Gate Ownership:** not applicable to this deferral; it changes no
  state owner, transition or gate. #310 must define association ownership and
  cleanup before implementation.
- **Security:** D12, ADR-0164 and ADR-0172 stay fail-closed.
- **Validation:** `pending`. A fresh independent reviewer must verify the
  exact two-clause removal, preserved outcomes, #310 boundary and absence of
  invented API or mechanism. This record does not self-approve the roadmap.

