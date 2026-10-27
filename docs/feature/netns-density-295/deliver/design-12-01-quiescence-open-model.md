# STEP 12-01 approved DESIGN contract — quiescence `OpenFlow` enabledness

## Status and boundary

**Status: QUIESCENCEOPEN1 APPROVED by the user 2026-10-26; post-approval fold
review, formal alignment and evidence remain pending.** Exact authority: “i
approve the rest of the decisions.” Technical review found zero open findings.
This approval authorizes only the bounded guard below; STEP 12-01 remains
non-executable until its remaining gates pass.

The bounded correction adds one existing-state premise to the existing
`quiescence.OpenFlow` action. It adds no state, action, API, production
mechanism, scenario, hazard, check, driver filter, or observable product
outcome.

The separate Trace-1 old-child postmortem failure in the same raw log is
excluded. It is private conformance-fixture process ownership: a test reuses
one Tokio runtime after simulated process death, while real process death
destroys that runtime. It supplies no production reachability or authority for
a production drain/cleanup mechanism and remains DISTILL-owned test support.

## Reproduced first divergence

Metal run `f4e6751b-6063-4ba3-b8fb-579adcaf1e21` replayed randomized
`quiesce_conf` with `QUINT_SEED=0x8703c2cd`. The complete log is
`.context/netns-density-295/12-01/quiescence-random-8703c2cd-verbose1-backtrace.log`.

Trace 2 reaches:

```text
... -> RestartWith(true)
    -> Damage("C1")
    -> OpenFlow
```

Immediately before `OpenFlow`, model and implementation agree on every
projected field: the process is up, table complete, count zero, latch open,
both recoveries idle with zero attempts, and no flow open. The model also has
its existing stimulus fact `dmg("C1") = true` (raw lines 823-861).

The current guard is only:

```quint
up and not(latch) and flowsOpen == 0
```

It therefore enables `OpenFlow` and sets `flowsOpen = 1`. Production remains
at zero; that is the sole state difference (raw lines 863-902).

## Exact production/action correspondence

The conformance identity mapping is already fixed:

- model component `C1` is
  `SharedGuestNetworkComponent::FlowListeners`, the stream listener's accept
  task (`shared_network_test_ports.rs:829-836`);
- `Damage("C1")` calls the real composed fault path
  `fail_listener(FlowListener::Stream, ConnectionAborted)` and settles until
  that accept task has exited (`shared_network_test_ports.rs:1152-1157`);
- the model preserves that fact in its existing `dmg` map; and
- the existing `OpenFlow` driver call queues a complete current-launch
  `TcpConnect` request and settles
  (`shared_network_test_ports.rs:1334-1367`). Its documented/modelled meaning
  is not "a guest connect is queued": it is "a guest-opened flow is admitted,
  read, attached and paired" (`quint_quiescence.rs:29-38`, `:232-236`).

With the stream accept task dead, the queued connection has no production
caller that can accept, read, admit, attach, or pair it. `flowsOpen` projects
only flows the production owner holds in `guest_flows`, `dials`, and
`datagrams`, not kernel/sim accept-queue children
(`quint_quiescence.rs:59-82`, `:530-555`). Production correctly remains at
zero.

This is a real action-enabledness mismatch. Rejecting the action in the driver
after Quint selected it would be an identity-dependent trace filter and would
leave the formal transition relation false.

## Exact approved formal contract

The model already has every needed fact. Change only `canOpenFlow`:

```quint
def canOpenFlow: bool =
  up
  and not(latch)
  and flowsOpen == 0
  and not(dmg.get("C1"))
```

`OpenFlow` continues to use `canOpenFlow` as its sole guard and retains every
existing effect and its existing `conf_step` record. No other action changes.

The new term means exactly: the action that represents a newly admitted and
paired guest-opened flow is enabled only while the component that accepts new
guest flow connections is not damaged. It does not assert that the kernel
accept queue is empty, add a listener-health projection, or turn `dmg` into an
owner observation. `dmg` remains the model's existing stimulus produced by
the real `Damage` action.

### Why only `C1`

`C2` is `ControlListener`, whose accept task admits **new control sessions**.
Its failure does not end an already-live control session and therefore does
not by itself prevent a guest-opened flow from being admitted through the
still-running stream listener. `OpenFlow` abstracts a flow using the live
session; it does not include creation of a new control session. Any lazy
fixture setup needed to construct that precondition is DISTILL-owned test
support, not authority to add `not(dmg("C2"))` to the protocol.

### Lifecycle closure

| State/order | `OpenFlow` meaning |
|---|---|
| Healthy C1, process up, latch open, no flow | Enabled exactly as today; the real owner admits and pairs one flow. |
| `Damage("C1")` before supervisor audit | Disabled. A real guest connection may wait in the existing accept queue, but no admitted/paired flow exists and `flowsOpen` remains zero. |
| `RecQuiesce("C1")` | Still disabled by the existing latch, and the first hold preserves `flowsOpen = 0`. |
| `RecRepair("C1")` before restore | `dmg("C1")` becomes false, but the existing latch still disables admission. Queued connections can be refused by the quiesced owner; they are not `OpenFlow`. |
| `RecRestore("C1")` after clean repair | Both damage and latch premises clear; a later actual admitted/paired flow may take the existing `OpenFlow` action. |
| C1 damaged after a flow is already open | `Damage` preserves that existing `flowsOpen`; the supervisor's first hold aborts it exactly as today. The new guard concerns only a new `OpenFlow`. |
| Crash/fail-stop | Existing actions clear process-scoped damage and flows; `up = false` already disables `OpenFlow`. |

The abstraction intentionally omits queued/refused connections because
`flowsOpen` and the action have always represented owner-held admitted flows.
Adding accept-queue state or a later `AcceptFlow` action would enlarge this
model without changing any QHOLD outcome.

## Preserved checks and outcomes

- `QuiescedHoldsNoFlow`, `LatchIffCount`, recovery holds/attempts, boot-table
  refusal, fail-stop, and every action effect remain unchanged.
- The existing named disabled-flow witness continues to use the exact
  `canOpenFlow` predicate; it gains the same listener-health premise as the
  action rather than a separate trace rule.
- The action/run/check inventory remains unchanged: 19 actions, 11 named
  runs, 6 temporal properties, 14 conformance variants, and the same formal
  checks.
- No product behavior is newly required. The real flow-listener task already
  cannot admit a connection after it exits, and the existing recovery repairs
  it before restoring admission.

## Rejected alternatives

| Alternative | Reason rejected |
|---|---|
| Let the driver reject `OpenFlow` when it knows C1 is damaged | An action shadow/filter; the model would still generate a production-impossible admitted-flow transition. |
| Reinterpret `OpenFlow` as queueing a guest connection | Contradicts its existing admitted/read/attached/paired action map and `flowsOpen` projection. |
| Add queued-connection state plus a later accept action | New state/action/mechanism with no QHOLD outcome that the existing abstraction cannot express. |
| Guard on both C1 and C2 | C2 damage stops new control-session accepts, not use of an existing live session; it would remove production-reachable flow admission and encode fixture setup as protocol. |
| Guard on every damaged component | Treats unrelated component health as a global admission gate before the supervisor has acquired its counted hold; no accepted contract requires that. |
| Make `flowsOpen = 1` when the connection merely queues | Fabricates owner-held state; the real owner observation remains zero. |

## Approval and evidence gates

### QUIESCENCEOPEN1 — approved user decision

Add only `not(dmg.get("C1"))` to `canOpenFlow`, where C1 retains its
existing fixed identity `FlowListeners`. The change removes an admitted-flow
action while the sole production accept task is dead. It changes no runtime,
API, wire, action/state/check inventory, or accepted outcome; its cost is the
additional formal state-space restriction and replacement formal evidence.

User authority is recorded by the exact quote above. Post-approval fold review
must verify exact correspondence before implementation. The bounded alignment
surface is only:

- `specs/quint/guest-flow-owner/quiescence.qnt`: the one guard term;
- `specs/quint/guest-flow-owner/README.md` and the feature delta: the exact
  action-enabledness meaning; and
- no driver behavior change: the current action continues to call the real
  production path.

Because the Quint transition relation changes, evidence must replay exact seed
`0x8703c2cd`, every unchanged named/randomized quiescence conformance target,
the complete guest-flow-owner re-record with exactly three jobs,
`verify-evidence`, and independent proof/source review. Trace 1's private
fixture process-scope remediation remains separate and supplies no production
or formal mechanism.
