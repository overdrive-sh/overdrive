# Guest-flow owner — Quint specification

This directory holds the formal model of the guest-flow owner protocol of the
vsock attachment replacement (feature `netns-density-295`, ADR-0145 to
ADR-0176). The specification is a design artifact and the DISTILL conformance
oracle (ADR-0168, decision QUINT): DISTILL drives the implementation against
these modules through quint-connect. The models encode the design text of
**revision 11** (revision 9, the APPLIANCE narrowing of 2026-10-07,
D8a-LOAD-SWAP of 2026-10-08, which removed the runtime probe and boot `verify`,
and D8a-CAP of 2026-10-08, intake-port admission against the steering capacity) in
`docs/feature/netns-density-295/feature-delta.md` § *vsock Attachment
Replacement DESIGN* (§ *Revision 9 decisions*, item 7 APPLIANCE; § *Revision 10
decisions*; § *User decisions — 2026-10-08*; § *Formal protocol model*). On the appliance only Overdrive writes the kernel objects the
model covers (A-31), so no action stands for other software deleting the nft
table, deleting steering entries, detaching the steering link, writing the
route or taking a CID. Crash and restart at every step and Overdrive's own
partial state are modelled. The models do not show that any implementation
conforms.

Model-check results: `docs/feature/netns-density-295/spike/quint-owner-findings.md`
(round 1), `quint-owner-findings-r2.md`, `quint-owner-findings-r3.md`,
`docs/feature/netns-density-295/design/quint-guest-flow-owner-findings-r4.md`
(revisions 8 and 9 before APPLIANCE) and
`docs/feature/netns-density-295/design/quint-guest-flow-owner-findings-r5.md`
(revision 9 + APPLIANCE: the steering model split into four concerns),
`docs/feature/netns-density-295/design/quint-guest-flow-owner-findings-r6.md`
(probe before attach in `prefix_boot`),
`docs/feature/netns-density-295/design/quint-guest-flow-owner-findings-r7.md`
(load → swap → route in `prefix_boot`, D8a-LOAD-SWAP) and
`docs/feature/netns-density-295/design/quint-guest-flow-owner-findings-r8.md`
(this revision: D8a-CAP in `intake`, M-4 in `prefix_boot`, M-2 in `quiescence`).

## Layout

| Path | Content |
|---|---|
| `owner_flows.qnt` | Module A: control session per CID, guest-opened flow admission, host-opened `TcpAccept` pairing, allocation lifecycle, quiescence (as the latch flows see) |
| `udp_slots.qnt` | Module B: guest UDP slot pool, datagram associations, slot release |
| `cid_lease.qnt` | Module C: lease offsets → CID, next-fit walk with the atomic host-kernel claim (D16-CLAIM), claim handoff to the VMM, release, `serve` crash with surviving VMMs (the only CID holders outside the pool) and restart |
| `prefix_landing.qnt` | Shared, stateless: where a connection to a workload address lands, per client class (used by the four steering modules) |
| `prefix_boot.qnt` | Steering concern 1: boot convergence of the pinned steering and the prefix route (D8a-ROUTE, D8a-LOAD-SWAP, U-4) |
| `intake.qnt` | Steering concern 2: intake-port admission against the steering capacity, intake listeners and their steering entries (D8a-LOOKUP, D8a-CAP, D23, U-6, U-3, M-7) |
| `quiescence.qnt` | Steering concern 3: named quiescence holders and component recoveries (D8a-HOLD, ADR-0124, UP-7, UP-12) |
| `shared_table.qnt` | Steering concern 4: the shared nft table left partial by Overdrive (K-D3), the steering's independence from it, the leg-C bypass window and the table repair under the IpRules recovery's hold |
| `hazard/*.qnt` | Hazard instances: each is a main module instantiated with one design rule switched off through its `OFF` constant (a one-line `import`) |
| `checks.toml` | Every check: spec, instance, property, backend, bound, expected verdict, CI flag |
| `evidence/` | Written only by `cargo xtask quint check --subsystem guest-flow-owner --record`; checked by `cargo xtask quint verify-evidence`. Earlier runs live in git history. |

Every module takes `OFF: Set[str]` (the design rules switched off; empty in
the design instances) and, where it has several instances, `CFG: str` (a named
instance configuration whose bounds are defined in the module). A hazard is
therefore `import <module>(OFF = Set("<rule>"), CFG = "<cfg>").* from "../<module>"`.

## What is modelled

| Module | State | Actions |
|---|---|---|
| A | Owner CID table (`Absent / Provisioned / Active / Retiring`), generation of the allocation leasing the CID, VM up/down, admission latch, the `ClaimSet` claim, control connections, guest-opened flows, host-opened flows on both sides, control messages per direction | provision, VM start / stop, activate, teardown, release, quiesce, restore; session open / accept / loss; flow accept, read, install, connect, completion, discard; `TcpAccept` open, guest accept, `Paired` / `Refused`, `Abort`, guest cleanup |
| B | Socket liveness, pending `sock_release` reports, slots, host associations, control messages | claim, send, associate, pair / refuse / abort, late `Paired`, pairing deadline, socket close (report or lost report), report handling, audit, idle release |
| C | The kernel's CID holder table (free / surviving VMM / our claim), lease per offset, next-fit cursor, the `assign` walk (pool lock), per-allocation state and where its claim's file is (slot / in transit / VMM) | assign begin, one claim per step (claim / `InUse` skip / non-`InUse` error), `take_claim`, `create` Ok / Err, VMM exit before READY, stop, `pool.release`, redeploy, surviving-VMM exit, `serve` crash + restart (a VMM being created survives iff it had inherited its device) |
| boot | `serve` up / boot phase / why down, this boot's program version, whether it loaded and whether its swap returned Ok, the link (none / attached unpinned / pinned) and its program, the tagged route (`absent` / `local`) and `estab` | boot steps 1–8 (one step), load (the kernel verifier may reject), swap (`BPF_LINK_UPDATE` of the adopted link, or a first attach then pin), route; a failure of each of these and of the route write (→ refuse, route not written, earlier program kept); crash between any two steps and inside swap (between attach and pin); restart with either program version |
| intake | `serve` up, intake listeners, steering entries, allocation state (its reservation of the declared ports while held, Retiring included: D8a-CAP), session, last `ListenState`, quiescence, ports parked by a reported steer defect, one other allocation's reservation and steered listeners (the map's capacity CAP = 2) | admission (reserve, or refuse `IntakeCapacityReached` with no state change) of ours and the other allocation, per-allocation events (each closes every listener whose port stops serving, in the same step: K-L3), bind / listen and their failure (retried), `steer` Ok / `KernelMemory` (retried, A-MEM) / a defect (reported, not retried: the port is parked until it stops serving) / the kernel's capacity refusal (unreachable with admission), activation begin / Ok / rollback, teardown, release, quiesce / restore, the other allocation's steer / close / release, crash, restart |
| quiescence | holders, latch, per-component recovery state (idle / held / repaired), damage, attempts, the wanted intake listener, whether the route (single form `local`) and the pinned steering are as boot converged them or a runtime step wrote one of them (the `otherRepairsPrefix` hazard: any runtime route or steering write) | damage (recurring), quiesce under the component's holder, repair (that component only), repair failure, failed post-repair audit, restore with the hold, fail-stop, owner bring-up, crash, restart (boot may leave the table partial again) |
| shared_table | boot phase, the table's two parts (`divert`, `reject`), steering link, route, listeners and entries, holders and recoveries (IpRules, FlowListeners) | boot step 8 complete / stopping part-way / a crash between its batches, step 9, the mTLS worker's runtime convergence stopping part-way (unbounded), listener-task exit, owner bring-up, recovery steps, crash, restart |

**Abstraction.** One CID (or one workload address) reused by two allocations;
two control connections; two guest-opened flows and one host-opened flow; two
UDP slots, two sockets, one or two destinations; three lease offsets and two or
three workloads; two guest ports, one or two declared; a steering capacity of
two entries shared with one other allocation declaring two ports. Time is abstract: every
deadline is a nondeterministic action. A connection's landing place
(`prefix_landing.qnt`) is a state predicate over three client classes
(Overdrive's marked leg-S / probes, other host-local clients, remote clients)
and every port; a wildcard host service is assumed bound on every port; the
outcomes are intake, leg-C, refused, wildcard (exposure) and elsewhere (no
tagged route: the packet leaves by another route). Each steering module fixes
what the other concerns own: `prefix_boot` has no listeners and takes the nft
rules as absent (the steering alone); `intake` takes boot as one step, the
steering converged and the rules complete; `quiescence` abstracts the table to a
damage flag and the listeners to one wanted listener; `shared_table` takes step
9 as one step that always succeeds. The shipped steering program is assumed
correct (A-33): a program that loads is a correct program,
so no module has a defective-program state. Per-allocation serialization (M-7)
is one atomic step per event that also closes every listener whose port stops
serving. The `assign` walk is stepwise (one claim ioctl per step) and holds the
pool lock; surviving VMMs exit between its steps.

## Assumptions and what discharges them

| ID | Assumption | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic (vhost stamps the device CID) | V-10 (bind level proven, P-17) |
| K-A2 | A released vhost device's connections, accept-queue entries included, are reset before the CID is reassigned (`flows_envStaleConn` drops it) | V-22 |
| K-A3 | A socket installed before its request is read hands the request to the verdict, which drops it | P-23 (+ R5-9 regression) |
| K-A4 | The host's close of `V_h` of an aborted or session-lost `TcpAccept` tears down the guest-side connection, including one still in the guest's accept queue (`flows_envGuestMissesHostClose` drops it) | V-24 |
| A-FID | Host flow ids are not reused within one pairing window | `FlowId` contract (design property) |
| K-B1 | Closing `T_g` / `Q_g` is observed by the host as the association's end | V-22 |
| K-B2 | `sock_release` reports the socket, or the audit finds orphan slots | R5-14 (no kernel unknown; no V-item) |
| K-B3 | Emptying or recreating a slot's cells discards every parked frame | V-5(c) |
| K-C1 | A duplicate CID is refused at device creation and the first holder is unaffected; a CID is free right after its holder's last reference closes | P-16 |
| K-C2 | The only CID holders outside the pool's leases are Overdrive VMMs that survived a `serve` crash; they exit at any time | Environment, modelled as actions (`Boot`, `OrphanExit`); the absence of other holders is A-31 |
| K-C3 | A claim on an unowned instance is exclusive while any reference to its file is open, survives the handoff to the VMM, and is released at the last close | V-25 |
| K-D1 | With the `local` route and no lookup decision, a connection reaches a wildcard host listener unless a listener bound to `workload_addr:p` exists | V-19 |
| K-D2 | With the nft rules: unmarked host-local prefix traffic is diverted to leg-C or rejected; remote prefix traffic diverted to leg-C or dropped; a divert with no leg-C socket falls through to the drop | V-19 |
| K-D3 | The rules, the route and the pinned steering persist across a `serve` crash or shutdown; each nft batch applies atomically, so a partial table arises only between batches (the mTLS worker failing part-way, or `serve` crashing between them) | V-19 (persistence, U-4); V-26 (iv), (vii) (steering independent of the table) |
| K-D4 | Without a tagged route the prefix is not local: a packet to it is routed elsewhere unless the nft rules divert or reject it first | V-19 |
| K-D5 | Closing a listener resets the children in its accept queue | V-13 |
| K-L1 | `sk_lookup` runs for a loopback-ingress connection to a `local`-route address that carries no socket, before the listener / wildcard lookup; a drop yields a TCP reset | V-26 |
| K-L2 | A prerouting TPROXY-assigned packet (leg-C divert) bypasses `sk_lookup` | V-26 |
| K-L3 | Closing a listening socket removes it from the socket map in the same step, and its slot is free for a new entry (D8a-CAP) (`intake_bugEntryOutlives` drops it) | V-26 (iii), (x) |
| K-L4 | The pinned link and map keep running with no process; after the owner exits the map holds no live socket | V-26 |
| K-L5 | Ingress ifindex is loopback for host-local connects, the receiving device otherwise | V-26 |
| K-L9 | `BPF_LINK_UPDATE` swaps the link's program atomically, and on error the link still runs the earlier program (`prefix_boot`: `swap` of an adopted link is one atomic step; a failed `swap` keeps the program) | V-26 (viii) |
| A-MEM | A `steer` refused for kernel memory (`KernelMemory`) succeeds on a later retry: memory pressure is not permanent (`intake`: strong fairness on a successful `Steer` in `ServingReachable`; `ServingReachableNoMemRecovery` drops it) | Environment, not a kernel unknown (node memory capacity: #261); the exposure is observable (`guest_intake.bind_failed { retried: true }`), V-9 reports any occurrence at density |
| A-33 | The shipped steering program decides as specified: a program that loads is a correct program (`prefix_landing`: an attached program is `ours`) | The Tier-3 steering scenarios R5-1, R5-2, R5-3 and R5-25, run in CI on the pinned kernel (ADR-0068); not checked at runtime |
| A-31 | Only Overdrive writes the node's kernel objects (APPLIANCE) | The appliance image configuration (ADR-0068); not a runtime validation item |

## Properties and teeth

| Module | Safety (design instances) | Progress (TLC) | Teeth (`OFF`) |
|---|---|---|---|
| `prefix_boot` | `LocalOnlyOverLink` (invariant 6, M-4: `local` only while a pinned link with a loaded program exists), `LocalOnlyOverLinkWhileDown`, `RouteOnlyAfterSwap`, `NoWildcardReached`, `NothingDeliveredWhileDown`, `NoElsewhere` (U-4), `NeverDetached` (K-L9), `OpenConverged` (a boot opens only with its program in the pinned link and the route `local`) | `BootOpens` | `routeFirst`, `ignoreFailure`, `detach` (non-atomic swap), `unpinned` |
| `intake` | `NoWildcardReached`, `ReachOnlyServing`, `EntryNamesServingListener`, `NothingDeliveredWhileDown`, `RemoteNeverDirect`, `UnmarkedOnLegC`, `ActivationAllOrNothing`, `ProvisionedHasNoListener`, `NoReleaseWithListener`, `QuiescedClosesListeners`, invariant 12 (D8a-CAP): `ReservedWithinCapacity`, `ListenersWithinReserved`, `EntriesWithinListeners`, `NoCapacitySteerFailure`; `ParkedOnlyWhileServing` | `ServingReachable` (under A-MEM; a port parked by a reported defect is excluded), `ActivationCompletes` | `entryOutlives` (K-L3), `takedownBeforeClose`, `rollbackPartial`, `actOkPartial`, `teardownKeeps`, `quiesceKeeps`, `noIntakeAdmission` (D8a-CAP), `noIntakeAdmission` + `capRetry` (the revision-10 capacity-driven retry) |
| `quiescence` | `LatchIffHolders`, `HeldIsHolder`, `NoReopenBeforeRepair`, `NoRepairWithoutHold`, `QuiescedClosesListeners`, `RepairNeverWritesPrefix`, `FailStopLeavesPrefixAsBooted` (UP-12) | `IpRulesEpisodeEnds`, `FlowListenersEpisodeEnds` (UP-7), `IpRulesRepaired` — each under its own recovery's fairness only | `anyRestore`, `repairNoHold`, `redoNoCount` (UP-7), `otherRepairsPrefix` |
| `shared_table` | `NoWildcardReached` (invariant 11), `RemoteNeverDirect`, `NothingDeliveredWhileDown`, `NoReopenBeforeRepair`, `QuiescedClosesListeners` | `TableRepaired` (the IpRules recovery's repair completes under its hold, or `serve` stops) — under that recovery's own fairness and boot only | `steeringInNft`, `unpinned`, `nonLoopback`, `anyRestore` |
| C | `CidUnique`, `ClaimHeld`, `NoLaunchClash`, `NoSkipWhileClaimable`, `NoLeakedClaim`, `RefusalEachClaimInUse` | `EventuallyPlaced` | `checkThenClaim`, `releaseBeforeVmm`, `rememberInUse` |

Modules A and B are unchanged in content; their properties and teeth are
listed in `checks.toml`.

Not modelled: D25-BIND (R5-15), D26 / V-21, V-20, framing and unframe (D5 /
D5a), VIP resolution (D24 / D24a), half-close drain, the listen-state lag
bound, D15-R3 deadlines, the beacon beyond its claim discipline, the unframe
program's attachment set (D5a-IFACE), the forwarder's process-owned links
(D19-LINKS), the boot prefix-configuration check (D8-PREFIX-CFG: it runs before
any boot effect), the map-shape
replacement at `load` (a new map at every boot), the ADR-0124 time bound itself (the model checks
ordering and step counts; the bound is V-9).

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
parallelism. Full logs land in
`target/quint/guest-flow-owner/<run-id>/<check>/` (one directory per run, with
the run's `summary.json`); only `--record` writes `evidence/`.
