# Guest-flow owner — Quint specification

This directory holds the formal model of the guest-flow owner protocol of the
vsock attachment replacement (feature `netns-density-295`, ADR-0145 to
ADR-0171). The specification is a design artifact and the DISTILL conformance
oracle (ADR-0168, decision QUINT): DISTILL drives the implementation against
these modules through quint-connect. The models encode the design text of
**revision 9** in `docs/feature/netns-density-295/feature-delta.md`
§ *vsock Attachment Replacement DESIGN* (§ *Revision 8 decisions*,
§ *Revision 9 decisions* items 1–8 as of 2026-10-07, including D8a-FENCE in
its round-4d form: the steering and the tagged prefix route are one component
(`GuestPrefixSteering`) with one runtime repair, `repair_guest_prefix(&hold)`
(fence → converge → verify → probe → `local`); no other recovery's repair
writes the steering or the route; every fence, every lift and the verify
before a lift are serialized in the owner's guest-prefix order; every
`quiesce_forwarding` call and every intake listener close verifies first and
fences on failure; the audit observes the route form (UP-5); a failed
post-repair audit is a failed attempt (UP-7); UP-11, UP-12). They do not show
that any implementation conforms.

Model-check results: `docs/feature/netns-density-295/spike/quint-owner-findings.md`
(round 1), `quint-owner-findings-r2.md`, `quint-owner-findings-r3.md`
(revision 7), and `docs/feature/netns-density-295/design/quint-guest-flow-owner-findings-r4.md`
(this revision: D16-CLAIM, D8a-HOLD, D8a-LOOKUP, D8a-ROUTE and U-6 restated;
its part *Round 4b* adds D8a-FENCE and the D16-CLAIM wording; its part
*Round 4c* adds item 7 of revision 9; its part *Round 4d* models the design's
answer to round 4c: one owner and one repair of the guest prefix).

## Layout

| Path | Content |
|---|---|
| `owner_flows.qnt` | Module A: control session per CID, guest-opened flow admission, host-opened `TcpAccept` pairing, allocation lifecycle, quiescence (as the latch flows see) |
| `udp_slots.qnt` | Module B: guest UDP slot pool, datagram associations, slot release |
| `cid_lease.qnt` | Module C: lease offsets → CID, next-fit walk with the atomic host-kernel claim (D16-CLAIM), claim handoff to the VMM, release, foreign vhost users, `serve` crash with surviving VMMs and restart |
| `steering.qnt` | Module D: boot steps (steering converge → verify + probe → `local` route, else fence + refuse), the tagged route (`local` / fence `prohibit`), the pinned `sk_lookup` steering and its socket-map entries, constant nft rules, intake listeners, named quiescence holders (the first hold verifies the steering and fences if it fails, before closing listeners) (every quiesce call verifies the steering and fences if it fails, before the first holder closes listeners) and three per-component recoveries — IpRules, FlowListeners (finding → quiesce → component repair only → clean audit → restore) and GuestPrefixSteering (finding → quiesce → `repair_guest_prefix`: fence → converge → verify + probe → `local`, the lift serialized against every fence → clean audit → restore; a failed post-repair audit is a failed attempt and the next attempt begins with the fence) — with a per-episode attempt budget and fail-stop, all-or-nothing activation, teardown / release, where a connection lands |
| `hazard/*.qnt` | Instances of the same modules with one design rule switched off (`BUG_*`, the teeth) or an environment fault switched on (`ENV_*`) |
| `checks.toml` | Every check: spec, instance, property, backend, bound, expected verdict, CI flag |
| `evidence/` | Written only by `cargo xtask quint check --subsystem guest-flow-owner --record` (summary of the last recorded run, spec hashes, counterexample traces); checked by `cargo xtask quint verify-evidence`. Earlier runs live in git history. |

## What is modelled

| Module | State | Actions |
|---|---|---|
| A | Owner CID table (`Absent / Provisioned / Active / Retiring`), generation of the allocation leasing the CID, VM up/down, admission latch, the `ClaimSet` claim, control connections, guest-opened flows (queued → accepted → request read → installed → connecting → paired / closed), host-opened flows on both sides, control messages per direction | provision, VM start / stop, activate, teardown, release, quiesce, restore; session open / accept / loss; flow accept, read, install, non-blocking connect, completion, discard; `TcpAccept` open, guest accept, `Paired` / `Refused`, `Abort`, guest cleanup |
| B | Socket liveness, pending `sock_release` reports, slots (owner, association id, routed, parked frames), host associations, control messages | claim, send, associate, pair / refuse / abort, late `Paired`, pairing deadline, socket close (report or lost report), report handling, audit, idle release |
| C | The kernel's CID holder table (free / foreign / surviving VMM / our claim), lease per offset, next-fit cursor, the `assign` walk (pool lock), per-allocation state and where its claim's file is (slot / in transit / VMM) | assign begin, one claim per step (claim / `InUse` skip / non-`InUse` error), `take_claim`, `create` Ok / Err, VMM exit before READY, stop, `pool.release`, redeploy, foreign take / release, surviving-VMM exit, `serve` crash + restart |
| D | `serve` up / boot step / why it is down, which callers' `converge_shared` verified, nft rules, route (`absent` / `local` / `fence`), steering link, steering-map entries, intake listeners, allocation state, session, last `ListenState`, holder set, latch, per-component recovery state and attempts, listener-task health; history variables `estab`, `verSince` (verified since the last fence), `tamper` (root detach / root `local` since the last fence or Ok verify), `rmv` / `rfn` (root removed / fenced the route since Overdrive's last route write), `win` / `winE` / `winF` (recovery steps taken while exposed / while traffic leaves the node / while root-fenced) | boot steps (converge / verify / probe may fail → fence + refuse), crash, restart; per-allocation events (each closes every listener whose port stops serving, in the same step); bind / listen, `steer` and their failures; activation begin / Ok / rollback; first-hold quiesce (verify, fence — which may fail — close); per recovery: finding, fence (steering only; may fail), quiesce, converge (with the component fix), verify + probe, `local`, repair failure (fences), re-finding, restore, fail-stop; foreign nft flush, root entry deletion, root link detach, root route removal / replacement / fence / `local`, listener-task exit |

**Abstraction.** One CID (or one workload address) reused by two allocations;
two control connections; two guest-opened flows and one host-opened flow; two
UDP slots, two sockets, one or two destinations, two or three association
ids; three lease offsets and two or three workloads; two guest ports, one or
two declared. Time is abstract: every deadline is a nondeterministic action.
A connection's landing place is a state predicate over three client classes
(Overdrive's marked leg-S / probes, other host-local clients, remote clients)
and every port; a wildcard host service is assumed bound on every port; the
outcomes are intake, leg-C, refused, wildcard (exposure) and elsewhere (no
route and no fence: the packet leaves by another route). Leg-S and probes may
dial at any time, quiesced or not (UP-1 as pinned in revision 9).
`repair_guest_prefix`'s fence, converge, verify + probe and route steps are
separate steps. The serialized guest-prefix order is a lock held from a lift's
verify to its route write: an Overdrive step that would fence waits for it
(`steer_bugUnserializedLift` drops it); root may still act inside it.
Per-allocation serialization (M-7) is modelled as one atomic step per event
that also closes every listener whose port stops serving (closing cannot fail
and removes the steering entry in the same kernel step), preceded in the same
step by the close's verify and fence; `steer_envCloseGap` lets root detach the
link between the passing verify and the close. The activation rollback is one
such step. The sequencer orders every `quiesce_forwarding` call (its verify +
fence included) with an in-flight activation (UP-13). The `assign` walk is stepwise (one claim ioctl per
step) and holds the pool lock; foreign vhost users act between its steps.
In module A, `quiesced` is the projection "the holder set is non-empty"; the
named holders are module D's.

## Assumptions and what discharges them

| ID | Assumption | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic (vhost stamps the device CID) | V-10 (bind level proven, P-17) |
| K-A2 | A released vhost device's connections, accept-queue entries included, are reset before the CID is reassigned (`flows_envStaleConn` drops it) | V-22 |
| K-A3 | A socket installed before its request is read hands the request to the verdict, which drops it | P-23 (+ R5-9 regression) |
| K-A4 | The host's close of `V_h` of an aborted or session-lost `TcpAccept` tears down the guest-side connection, including one still in the guest's accept queue (`flows_envGuestMissesHostClose` drops it) | V-24 |
| A-FID | Host flow ids are not reused within one pairing window | `FlowId` contract (design property) |
| K-B1 | Closing `T_g` / `Q_g` is observed by the host as the association's end | V-22 |
| K-B2 | `sock_release` reports the socket, or the audit finds orphan slots | R5-14 |
| K-B3 | Emptying or recreating a slot's cells discards every parked frame | V-5(c) |
| K-C1 | A duplicate CID is refused at device creation and the first holder is unaffected; a CID is free right after its holder's last reference closes | P-16 |
| K-C2 | Foreign vhost users take and release CIDs at any time | Environment, modelled as actions (`ForeignTake` / `ForeignRelease`, unbounded in the safety instances) |
| K-C3 | A claim on an unowned instance is exclusive while any reference to its file is open, survives the handoff to the VMM, and is released at the last close | V-25 |
| K-D1 | With the route and no lookup decision, a connection reaches a wildcard host listener unless a listener bound to `workload_addr:p` exists | V-19 |
| K-D2 | With the nft rules: unmarked host-local prefix traffic is diverted to leg-C or rejected; remote prefix traffic diverted to leg-C or dropped; a divert with no leg-C socket falls through to the drop | V-19 |
| K-D3 | nft rules and the route persist across a `serve` crash; nft batches are atomic; deletion by other software is a fault (`ENV_FOREIGN_FLUSH`, which removes only the nft rules / divert, never steering) | V-19 (persistence, U-4); V-23 (other software) |
| K-D4 | Without a tagged route the prefix is not local: a packet to it is routed elsewhere (default route) unless the nft rules divert or reject it first | V-19 |
| K-D5 | Closing a listener resets the children in its accept queue | V-13 |
| K-L1 | `sk_lookup` runs for a loopback-ingress connection to a `local`-route address that carries no socket, before the listener / wildcard lookup; a drop yields a TCP reset | V-26 |
| K-L2 | A prerouting TPROXY-assigned packet (leg-C divert) bypasses `sk_lookup` | V-26 |
| K-L3 | Closing a listening socket removes it from the socket map in the same step; later SYNs cannot be assigned to it (`steer_bugEntryOutlives` drops it) | V-26 |
| K-L4 | The pinned link and map keep running with no process; after the owner exits the map holds no live socket | V-26 |
| K-L5 | Ingress ifindex is loopback for host-local connects, the receiving device otherwise | V-26 |
| K-L6 | A tagged `prohibit` route for the prefix in the local table refuses host-local connects at once and refuses remote packets (leg-C TPROXY-diverted ones included) without forwarding them; the `local` ↔ `prohibit` replace is atomic | V-26 |
| K-L7 | `GuestPrefixSteering::verify` (a read-only link query) observes a root detach that happened before it | V-26 |
| F-FENCE | A fence (route replace) can fail; modelled as a fault (`FENCE_FAILURES`). After a failed fence a quiesce goes on (design text) and a close goes on (UP-15: the design does not say) | Environment, modelled as an action (`steer_envFenceFail`) |

Root tampering — deleting a steering entry (`ENV_ROOT_ENTRY_DELETE`),
detaching the pinned link (`ENV_ROOT_DETACH`, also between a close's verify and
the close: `ENV_DETACH_IN_CLOSE_GAP`), removing, replacing, fencing or
setting `local` on the tagged route (`ENV_ROOT_ROUTE`) — is the host's trust
boundary. It is modelled as faults, and the resulting windows and their end
are checked (`steering-env-detach-*`, `steering-env-close-gap-*`, `steering-env-root-route-*`,
`steering-detach-exposure-ends*`, `steering-root-route-removal-ends`,
`steering-root-fence-ends`, `steering-recovery-resolves*`), not assumed away.

## Properties of module D (round 4d)

Safety (invariants; `Safety` on the design instances, `SafetyUnderDetach` under
root link detach, `SafetyUnderRouteTamper` under root route tampering):

| Property | Decision | Statement |
|---|---|---|
| `NoWildcardReached`, `ReachOnlyServing`, `EntryNamesServingListener`, `NothingDeliveredWhileDown`, `RemoteNeverDirect` | D8a-LOOKUP | as round 4b |
| `RouteImpliesSteering`, `ConvergeNeedsSteering`, `RouteLocalOnlyVerified` | D8a-ROUTE, D8a-FENCE | `local` only behind attached steering; every lift (boot or `repair_guest_prefix`) only after its own verify + probe; `local` only if a lift's verify returned Ok after the last fence |
| `OnlyGuestPrefixRecoveryLifts` | D8a-FENCE (one owner, one repair) | nothing but boot and `repair_guest_prefix` ever lifts a fence |
| `FencedWhileSteeringRecovers` | D8a-FENCE | from the repair's fence to its lift the prefix is fenced |
| `NoElsewhere` | D8a-FENCE | prefix traffic never leaves the node once the route exists (no root tampering) |
| `ExposureOnlyInTamperWindow` | D8a-FENCE, item 8 | a host service is reachable only after a root detach / root `local` and before the next fence or lift-verify |
| `OpenExposureWithinTwoSteeringSteps` | D8a-FENCE, item 8 | while `serve` is open the detach window lasts at most two guest-prefix-recovery steps (`ExposureWithinTwoSteeringSteps`, the same bound across a fail-stop, is violated: F-4d-1) |
| `NoOwnerStepWidens` (= `QuiescenceNeverWidens` ∧ `CloseNeverWidens`), `SteeringQuiesceNeverWidens` | item 7 | no quiesce call and no intake listener close of any cause widens a detach exposure |
| `FenceHoldsWhileStopped`, `NothingWhileStopped` | D8a-FENCE, UP-12 | while `serve` is down after a refusal or a fail-stop (no root tampering since the last fence / lift-verify), the prefix is fenced or `local` behind attached steering, and every connection is refused |
| `GuestPrefixFailStopFenced` | D8a-FENCE, UP-12 | the guest-prefix recovery's fail-stop leaves the fence — **violated under root detach (F-4d-1)** |
| `ElsewhereOnlyInRemovalWindow`, `ElsewhereWithinGpsRepair` | UP-5 | traffic leaves the node only between a root removal and the next Overdrive route write, within four guest-prefix-recovery steps |
| `RootFenceWithinGpsRepair` | UP-5 | a root fence is lifted within seven guest-prefix-recovery steps |
| `ActivationAllOrNothing`, `ProvisionedHasNoListener`, `NoReleaseWithListener` | U-6, U-3 | as round 4b |
| `QuiescedClosesListeners`, `LatchIffHolders`, `HeldIsHolder`, `NoReopenBeforeRepair`, `NoRepairWithoutHold` | D8a-HOLD, M-7 | as round 4b |

Documented violations (trust boundary, findings): `RouteNeverWidens` (root
detach between a lift's verify and its route write, item 8);
`NoOwnerStepWidens` on `steer_envCloseGap` (root detach between a close's
verify and the close, F-4d-3); `QuiescenceNeverWidens` / `CloseNeverWidens`
with a failed fence (UP-15); `GuestPrefixFailStopFenced` and
`ExposureWithinTwoSteeringSteps` (F-4d-1).

Progress (TLC): `ServingReachable`, `ActivationCompletes`, `BootOpens`,
`LegCBypassEnds` (also under a concurrent root detach, UP-11);
`DetachExposureEndsFindFence` (with and without another recovery);
`DetachExposureEndsWithOwner`; `SteeringRecoveryResolves` and
`SteeringRecoveryResolvesAllRecoveries`; `RouteTamperEnds`, `RootFenceEnds`.
`DetachExposureEnds` (no fairness of the owner's activation steps) is violated:
UP-13.

Teeth (one rule off each, `hazard/steering_hazards.qnt`): the round-4b set
(D8a-LOOKUP, K-L3, D8a-ROUTE, U-6, U-3, M-7, D8a-HOLD, D8a-FENCE), plus
`steer_bugQuiesceNoFence`, `steer_bugJoinNoVerify`, `steer_bugCloseNoVerify`
(item 7), `steer_bugOtherRepairsPrefix` (one owner and one repair),
`steer_bugUnserializedLift` (the serialized guest-prefix order),
`steer_rootRouteLiveAuditIgnores` (UP-5), `steer_detachLiveSoloRedoNoFence`
(UP-6), `steer_detachLiveRedoNoCount` (UP-7).

Not modelled: D25-BIND (a kernel bind fact; R5-15), D26 / V-21, V-20, framing
and unframe (D5 / D5a), VIP resolution (D24 / D24a), half-close drain, the
listen-state lag bound, D15-R3 deadlines, the beacon beyond its claim
discipline, a foreign untagged route overlapping the prefix
(`ForeignGuestPrefixRoute`), a route of another form than `absent` / `local` /
`fence` (a different route lands as `absent` does), the
ADR-0124 time bound itself (the model checks ordering and step counts; the
bound is V-23 / V-26).

## How to run

From the workspace root on the host (the Lima VM provisions Quint 0.32.0,
Apalache 0.56.1 and JDK 21 on `PATH`):

```bash
cargo xtask lima run -- cargo xtask quint typecheck
cargo xtask lima run -- cargo xtask quint check --ci --subsystem guest-flow-owner   # the CI subset
cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner        # every check
cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --name <check>
cargo xtask lima run -- cargo xtask quint check --subsystem guest-flow-owner --record  # record evidence
cargo xtask quint verify-evidence --subsystem guest-flow-owner                          # evidence matches specs?
```

The runner (`xtask/src/quint.rs`) reads `checks.toml`, runs each check through
`quint verify`, and fails unless every outcome matches `expect`; a tool error or
timeout never counts as a result. It owns timeouts, retries, process cleanup and
parallelism. Full logs land in `target/quint/guest-flow-owner/<check>/`; only
`--record` writes `evidence/`.
