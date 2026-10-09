# Guest-flow owner — Quint specification

This directory holds the formal model of the guest-flow owner protocol of the
vsock attachment replacement (feature `netns-density-295`, ADR-0145 to
ADR-0181). The specification is a design artifact and the DISTILL conformance
oracle (ADR-0168, decision QUINT): DISTILL drives the implementation against
these modules through quint-connect. The models encode the design text of
**revision 11** (revision 9, the APPLIANCE narrowing of 2026-10-07,
D8a-LOAD-SWAP of 2026-10-08, which removed the runtime probe and boot `verify`,
and D8a-CAP of 2026-10-08, intake-port admission against the steering capacity)
with the **spike remediation decisions of 2026-10-09** (ATTR-TOKEN, ADR-0177, and the
measured kernel behaviour K-A2′ in `owner_flows`; INTAKE-BIND, ADR-0178, in
`prefix_landing`) and **their round 2** (ATTR-LIVE-CLAIM — the live launch token
compared in the claim's step, for the control session and the VM driver's beacon
— and HALFCLOSE-SIGNAL, ADR-0180, in `owner_flows`; CELL-RECREATE in `udp_slots`) in
`docs/feature/netns-density-295/feature-delta.md` § *vsock Attachment
Replacement DESIGN* (§ *Revision 9 decisions*, item 7 APPLIANCE; § *Revision 10
decisions*; § *User decisions — 2026-10-08*; § *Spike remediation decisions —
2026-10-09* and *round 2*; § *Formal protocol model*, round 9). On the appliance only Overdrive writes the kernel objects the
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
(D8a-CAP in `intake`, M-4 in `prefix_boot`, M-2 in `quiescence`). Round 9
(this revision: K-A2′, ATTR-TOKEN with ATTR-LIVE-CLAIM, the beacon and
HALFCLOSE-SIGNAL in `owner_flows`; CELL-RECREATE in `udp_slots`; INTAKE-BIND in
`prefix_landing`) is recorded in `evidence/summary.md`. The conformance projection round
(2026-10-09: QPROJ-1 to QPROJ-8 and the rulings U-QP-1 to U-QP-3 of the feature-delta, spec changes S-1 to S-7)
adds the projection table below, the five-offset and per-side conformance instances, narrows the shared
nft table to its two single-transaction states, and drops the adopted-link `pin` step. Its first run found a
counterexample against ATTR-TOKEN's session claim (a dead launch's queued control
connection, accepted while the CID was Provisioned and read after teardown, took
the claim, which outlived `pool.release`); the design answered it with
ATTR-LIVE-CLAIM, and that counterexample is now the teeth of the hazard
`noClaimRecheck`.

## Layout

| Path | Content |
|---|---|
| `owner_flows.qnt` | Module A, five instances (`flows_half_host` / `flows_half_guest` are the per-side conformance instances of `flows_half`, S-4). `flows_ok` (CFG `core`): control session per CID, launch attribution by peer CID and live launch token (ATTR-TOKEN, ATTR-LIVE-CLAIM) under the measured K-A2′, guest-opened flow admission, host-opened `TcpAccept` pairing, allocation lifecycle, quiescence (as the latch flows see). `flows_beacon` (CFG `beacon`): the VM driver's beacon — live launch, READY attribution and the beacon claim, `driver.stop`. `flows_half` (CFG `half`): one paired TCP flow's half-close handshake (HALFCLOSE-SIGNAL), owner death per side, session loss |
| `udp_slots.qnt` | Module B: guest UDP slot pool, datagram associations, slot release, cell generations and readiness dispatch (CELL-RECREATE) |
| `cid_lease.qnt` | Module C (design instances on three offsets; conformance instances `lease_conf_ok` / `lease_conf_live` on the five offsets of the /29 test pool, compared where `walkIdle`, S-2): lease offsets → CID, next-fit walk with the atomic host-kernel claim (D16-CLAIM), claim handoff to the VMM, release, `serve` crash with surviving VMMs (the only CID holders outside the pool) and restart |
| `prefix_landing.qnt` | Shared, stateless: where a connection to a workload address lands, per client class (used by the three steering modules that import it); an intake listener is reached only through its steering entry (INTAKE-BIND) |
| `prefix_boot.qnt` | Steering concern 1: boot convergence of the pinned steering and the prefix route (D8a-ROUTE, D8a-LOAD-SWAP, U-4) |
| `intake.qnt` | Steering concern 2: intake-port admission against the steering capacity, intake listeners and their steering entries (D8a-LOOKUP, D8a-CAP, D23, U-6, U-3, M-7) |
| `quiescence.qnt` | Steering concern 3: named quiescence holders and component recoveries (D8a-HOLD, ADR-0124, UP-7, UP-12) |
| `shared_table.qnt` | Steering concern 4: the shared nft table, a two-state variable (`absent` / `complete`; single-transaction writes, U-QP-1) left absent by Overdrive's own failed boot-step-8 write (K-D3), the steering's independence from it, the leg-C bypass window and the table repair under the IpRules recovery's hold |
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
| A (`core`) | Owner CID table (`Absent / Provisioned / Active / Retiring`), the lease leasing the CID (its launch token, live while Provisioned or Active), the launch of the last VM, VM up/down, admission latch, the `ClaimSet` claim and its holder, control connections (each with the launch that opened it: queued, being read, live, dead, closed), guest-opened flows (each with its launch; registered awaiting the token check, or attributed), host-opened flows on both sides, control messages per direction | provision, VM start / stop (K-A2′: accepted connections fail, queued ones survive with their buffered first record), activate, teardown (ends the live token, closes the session, releases the claim; leaves connections being read), release, quiesce, restore; session connect / reconnect, accept (early close on the CID state), read + live-token check + claim in one step, loss; flow connect, check-and-register, request read + live-token check, install, connect, completion, discard, end (with or without a control message); `TcpAccept` open, guest accept, `Paired` / `Refused`, `Abort`, guest cleanup |
| A (`beacon`) | The allocation lifecycle, VM up/down, the driver's live launch (started, not stopped), the beacon claim and its holder, beacon connections (each with its launch and a buffered READY: queued, being read, live, closed) | provision, VM start / exit, teardown, `driver.stop` (ends liveness, closes the beacon session, releases the claim — one step; leaves connections being read), release (after `driver.stop` and teardown); beacon dial / re-dial, accept (early close with no live launch), READY read + live-launch check + claim in one step |
| A (`half`) | One paired TCP flow, per side (guest owner, host owner): pairing, the outer leg's recorded EOF, the announcement (none / HalfClosing / done), the acknowledged peer direction, the outer shutdown, the far end's first terminal event (ghost), owner alive, observed session loss; the control session's two FIFO directions and the flow vsock's end-of-stream per direction, as independent channels | far-end half-close (before or after `Paired` on the opener's side), `Paired`, `HalfClose` (after the drain), `HalfCloseAck` (at once; U-5 discard), vsock shutdown after the acknowledgement, vsock end arriving (announced → outer shutdown; unannounced → abort with reset, `UnannouncedVsockEnd`), clean close, session-loss observation (after the peer's in-flight messages); faults: outer reset, flow error / teardown / quiescence, owner death per side (linger resets its far end; its vsock end reaches the survivor as its own action, `VsockEndReachesSurvivor`), session drop, a defect ending a vsock without the handshake |
| B | Socket liveness, pending `sock_release` reports, slots, host associations, control messages, queued readiness events (each registered for its slot's current or an earlier cell generation), residue (hazard only) | claim, send, readiness dispatch (an earlier generation's event discarded; a current one starts an association), pair / refuse / abort, late `Paired`, pairing deadline, socket close (report or lost report), report handling, audit, idle release; every end of association recreates the cells (a new generation) |
| C | The kernel's CID holder table (free / surviving VMM / our claim), lease per offset, next-fit cursor, the `assign` walk (pool lock), per-allocation state and where its claim's file is (slot / in transit / VMM) | assign begin, one claim per step (claim / `InUse` skip / non-`InUse` error), `take_claim`, `create` Ok / Err, VMM exit before READY, stop, `pool.release`, redeploy, surviving-VMM exit, `serve` crash + restart (a VMM being created survives iff it had inherited its device) |
| boot | `serve` up / boot phase / why down, this boot's program version, whether it loaded and whether its swap returned Ok, the link (none / attached unpinned / pinned) and its program, the tagged route (`absent` / `local`) and `estab` | boot steps 1–8 (one step), load (the kernel verifier may reject), swap (`BPF_LINK_UPDATE` of the adopted link, then straight to route; or a first attach, then pin, then route: S-3), route; a failure of each of these and of the route write (→ refuse, route not written, earlier program kept); crash between any two steps and inside swap (between attach and pin); restart with either program version |
| intake | `serve` up, intake listeners, steering entries, allocation state (its reservation of the declared ports while held, Retiring included: D8a-CAP), session, last `ListenState`, quiescence, ports parked by a reported steer defect, one other allocation's reservation and steered listeners (the map's capacity CAP = 2) | admission (reserve, or refuse `IntakeCapacityReached` with no state change) of ours and the other allocation, per-allocation events (each closes every listener whose port stops serving, in the same step: K-L3), bind / listen and their failure (retried), `steer` Ok / `KernelMemory` (retried, A-MEM) / a defect (reported, not retried: the port is parked until it stops serving) / the kernel's capacity refusal (unreachable with admission), activation begin / Ok / rollback, teardown, release, quiesce / restore, the other allocation's steer / close / release, crash, restart |
| quiescence | holders (the SET of current holders, D8a-HOLD, U-QP-3), latch, per-component recovery state (idle / held / repaired), damage (IpRules: the table is absent, drawn at `init` and at `Restart` only; FlowListeners: a listener-task exit, recurring), attempts, the wanted intake listener, whether the route (single form `local`) and the pinned steering are as boot converged them or a runtime step wrote one of them (the `otherRepairsPrefix` hazard: any runtime route or steering write) | FlowListeners damage (recurring; there is no runtime IpRules damage action, S-6), quiesce under the component's holder, repair (that component only), repair failure, failed post-repair audit, restore with the hold, fail-stop, owner bring-up, crash, restart (boot step 8's write fails and leaves the table absent, or succeeds) |
| shared_table | boot phase, the table (`absent` / `complete`: one variable, written and replaced only in single nft transactions, U-QP-1), steering link, route, listeners and entries, holders and recoveries (IpRules, FlowListeners) | boot step 8's transaction succeeds (`complete`) or fails (`RulesFail`: the table stays as it was), step 9, the mTLS worker's failed replacement transaction (`MtlsReplaceFail`, unbounded: the table stays as it was), listener-task exit, owner bring-up, recovery steps, crash (the table as it was), restart |

**Abstraction.** One CID (or one workload address) reused by two allocations,
one launch each, so a launch token is the lease's generation; two control
connections (reused after they close: the guest reconnects); two guest-opened
flows and one host-opened flow (instance `core`); two beacon connections
(instance `beacon`); one paired TCP flow whose opener — guest (TcpConnect) or host
(TcpAccept) — is chosen at init (instance `half`); a connection's first record
(`ControlOpen`, READY, request) is buffered at connect, and an accepted connection
whose VM dies before its first record is read is closed (K-A2′); two
UDP slots, two sockets, one or two destinations, a cell generation per slot
abstracted to current / earlier (generations only grow, and dispatch compares an
event's generation only for equality with its slot's current one); three lease offsets and two or
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
damage flag (the table absent) and the listeners to one wanted listener; `shared_table` takes step
9 as one step that always succeeds. The shared table is a two-state variable (`absent`, `complete`):
the mTLS worker writes and replaces it only in single nft transactions (D26-PINS F-2, boot step 8), so a
failed first write leaves it absent, a failed replacement and a crash leave it as it was, and no
partial table exists (U-QP-1); `prefix_landing`'s `lands` stays total over any `Net`, but the
`shared_table` instances reach only `divert = reject`. The shipped steering program is assumed
correct (A-33): a program that loads is a correct program,
so no module has a defective-program state. Per-allocation serialization (M-7)
is one atomic step per event that also closes every listener whose port stops
serving. The `assign` walk is stepwise (one claim ioctl per step) and holds the
pool lock; surviving VMMs exit between its steps.

## Projection classification (S-1, QPROJ-1 to QPROJ-8)

The spec is the DISTILL conformance oracle: a `quint-connect` driver replays it against the production
owner and compares state after every step (`.claude/rules/testing.md` § "Quint conformance"). Every
variable of every module is exactly one of: **projected** (compared; read from a pinned public read-only
owner accessor, a pinned sim arbiter paired with a Tier-3 scenario, an independent kernel query, or the
caller's own observation of a driving-port call it issued), **ghost** (exists to state an invariant; a
function of the compared trajectory and the action, so a divergence surfaces in the compared variables
or in the action's enabledness), **model-internal** (state inside one atomic production operation;
compared only where that operation is not in progress), **stimulus** (state of what the driver plays:
applications, placement intent, the peer owner, fault budgets) or **hazard-only** (constant in the
design instances). Authoritative source: `docs/feature/netns-density-295/feature-delta.md` § *Conformance
observation surface and action map* (approved 2026-10-09); it also carries the action-to-driving-port
map. Identity relabelling (model ids to `AllocationId`, `LaunchToken`, `FlowId`, socket cookie, ...) is a
bijection the driver fixes, never a state source. Owner accessors (all `pub`, read-only, owned
snapshot): O2 = `GuestAddressPool::lease_observation()`; O3 = `HostSharedGuestNetworkOwner::flow_observation()`;
O4 = `VmDriver::beacon_observation()`; O5 = the `overdrive-init` guest owner's `guest_observation()`;
O6 = the runtime supervisor's `recovery_observation()` (the SET of current recovery holders, U-QP-3).
S = sim arbiter (`held_cids`, `is_live`, `shared_table_state`, `socket_observation`,
`steering_observation` / `steering_effects`, `beacon_socket_observation`, `guest_socket_observation`);
K = independent kernel query; C = the caller's observation of its own driving-port call.

| Module (instance) | Projected | Ghost | Model-internal | Stimulus | Hazard-only |
|---|---|---|---|---|---|
| `cid_lease` (conformance: `lease_conf_ok`, `lease_conf_live`; compared where `walkIdle`) | `lease` (O2 lease per offset), `cursor` (O2 `next_fit`), `kern` (S `held_cids` with O2: 0 unheld; the allocation if the pool leases that offset; -2 if held and unleased), `al(a).off` / `.loc` / `.st` (O2 with C and S `is_live`) | `g` | `walker`, `todo` (one `assign` under the pool lock) | `al(a).retry`, `bootLeft`, `exitsLeft`, `errsLeft` | `al(a).remembered`, `pend` |
| `owner_flows` `flows_ok`, host side | `alloc` (O3 `phase`, else O2), `gen` (O2/O3 `launch`), `quiesced` (O3 `admission`), `claim` (O3 `session`), `sess(c).st`, `flows(f).st` / `.req` / `.inFlight`, `hf(h).st` (O3 with S `socket_observation`), `toGuestAbort` (S `outbox`) | `badPaired`, `crossMsg`, `heldByOther`, `resurrected`, `abortDiscarded`, `phantom` | - | `vmGen`, `vmUp`, `gf`, `toHostPaired`, `toHostRefused`, `sess(c).gen`, `flows(f).gen` | beacon and `hc` variables (constant in this instance) |
| `owner_flows` `flows_ok`, guest side (`overdrive-init`) | `gf(h)` (O5 `accepted`; S `reset_in_queue`), `toHostPaired`, `toHostRefused` (S `outbox`) | as above | - | every host variable | as above |
| `owner_flows` `flows_beacon` (`overdrive-worker`) | `drvLive` (O4 `live`), `bclaim` (O4 `claims`), `bcn(b).st` (O4, S `beacon_socket_observation`) | `readyCross`, `bHeldByOther` | - | `vmGen`, `vmUp`, `alloc`, `gen` | - |
| `owner_flows` `flows_half`, `flows_half_host`, `flows_half_guest` (per side `s` under test) | `paired(s)`, `outEof(s)`, `ann(s)`, `acked(s)`, `outShut(s)` (O3/O5 `FlowEndObservation`), `open(s)` (owner flow table), `loss(s)` (S `session_ended` with the owner holding no session), `ch(s)` (S `outbox`), `vEnd(s)`, `vEndSent(s)` (S), `far(s)` (S `far_end`), `ctlDown` (S `session_ended`), `dead(s)` (C) | `earlyShut`, `faulted` | - | the other side's fields, `opener`, `farHc` | `held`, `ackDue` |
| `udp_slots`, guest side (`overdrive-init`) | `slots(s).owned` / `.sock` / `.dest` / `.fid` / `.routed` (O5 `slots`), `slots(s).parked` (S), `ready` (S `readiness`, current iff its generation equals O5's), `releasePending` (S `release_reports`), `toHostAbort` (S `outbox`), `toHostEnd` (S `legs_closed_unobserved`) | `misdelivered`, `lateIgnored`, `staleReassoc`, `staleGenStart`, `slots(s).holders`, `endedNoNew` | `nextFid`, `guestEnded` | `sockAlive`, `host`, `toGuestPaired`, `toGuestRefused`, `toGuestAbort` | `picks`, `residue` |
| `udp_slots`, host side (`overdrive-control-plane`) | `host(i).st` (O3 `datagrams`, S `closed_by_owner`), `toGuestPaired`, `toGuestRefused`, `toGuestAbort` (S `outbox`) | as above | - | every guest variable, `host(i).sock` / `.dest` / `.slot` | as above |
| `intake` | `lis` (O3 `intake`), `entries` (S `steering_observation().entries`), `alloc` (O3 `phase`; `Activating` by C; else O2), `gen` (`launch`), `sess` (O3 `session` is some), `lastLS` (O3 `last_listen_state`), `quiesced` (O3 `admission`), `parked` (O3 `parked`), `oRes` (O2 `reserved_ports` of the other allocation), `oEnt` (S entries of the other allocation) | `g` | - | `up`, `bindLeft` | - |
| `prefix_boot` | `link`, `prog`, `route` (S `steering_observation()`), `phase` (C and S `steering_effects()`) | `estab`, `g` | `loaded`, `swapOk`, `adopted` | `up`, `stop`, `bin` (C), `failLeft`, `crashLeft` | - |
| `quiescence` | `holders` (O3 `holders`: component names, the SET of holders), `latch` (O3 `admission`), `rec`, `att` (O6 `phase`, `failed_attempts`), `lis` (O3 `intake`), `dmg("IpRules")` (S `shared_table_state()` is `Absent`), `dmg("FlowListeners")` (not O `flow_listeners_running`), `prefix` (S `steering_effects()`) | `g` | - | `up`, `stop` (C), `repLeft` | - |
| `shared_table` | `table` (S `shared_table_state()`: two states), `link`, `route`, `entries` (S `steering_observation()`), `lis` (O3 `intake`), `holders` (O3), `rec`, `att` (O6), `flDmg` (not O `flow_listeners_running`), `phase` (C: the composition step reached), `up` (C) | `g` | - | `repLeft` | - |
| `prefix_landing` | stateless: the landing function is compared on the real kernel (QPROJ-8: observed landing from sockets and `ss`; inputs from `ip route`, `bpftool`, `nft list table`) | - | - | - | - |

`Net.divert` and `Net.reject` are derived from `table` in `shared_table` (both true iff `complete`); the
`Net` inputs `route`, `steer`, `entries`, `lis`, `divert`, `reject`, `legC`, `marked` are read by
independent kernel queries in the `prefix_landing` driver, never written by it.

**Shared table (S-5, S-7).** The shared nft table is written and replaced only in single transactions
(D26-PINS F-2, boot step 8), so `shared_table` holds it in the two states one transaction leaves, `absent`
and `complete` (the replaced-over table has the same constant rules and is `complete`). `table` is a
compared variable (QPROJ-7 `SimMtlsIntercept::shared_table_state`, paired with R5-43 and R5-48). A failed
first write leaves it absent (`RulesFail`), a failed replacement (`MtlsReplaceFail`) and a crash leave it as
it was. `quiescence`'s `dmg("IpRules")` is the table being absent: there is no runtime `Damage("IpRules")`
action, because no Overdrive write leaves the table absent while serving; `init` draws it from
`{false, true}` and `Restart` keeps its choice (read as "boot step 8's write fails, or succeeds"). Neither
module models recovery progress as a single component: `holders` is the set of current holders and
`rec` / `att` are per component (U-QP-3: the supervisor reports the set of holders with their attempt
counts).

**Closed flows (S-4).** In `owner_flows` `flows_half` (and its per-side instances), once `open(s)` is false
the owner holds nothing of the flow: the fields of side `s` are history and a driver compares nothing else
of `s`. In the per-side instances the side under test never exhibits `HcPeerDefectEnd` (a defect of its own
code): `flows_half_host` enables it for the guest only, `flows_half_guest` for the host only.

## Assumptions and what discharges them

| ID | Assumption | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic (vhost stamps the device CID) | V-10 (bind level proven, P-17) |
| K-A2′ | At VMM death, the host connections of that VM already accepted fail at their next I/O; a connection still in a host accept queue stays queued and is accepted later — after the launch's teardown and the CID's next lease and launch included — with the CID as peer, delivering its buffered first record (failing its first read if it has none). Attribution therefore uses the CID's live launch token, compared in the claim's step (ATTR-TOKEN, ATTR-LIVE-CLAIM), never the peer CID alone. Replaces K-A2 (accept-queue entries reset before CID reuse), which V-22 falsified (`flows_bugPeerCidOnly`, `flows_bugBeaconPeerCidOnly`) | Measured: V-22 (P-37, `docs/feature/netns-density-295/spike/findings-v22.md`). Modelled in round 9 |
| K-A3 | A socket installed before its request is read hands the request to the verdict, which drops it | P-23 (+ R5-9 regression) |
| K-A4 | The host's close of `V_h` of an aborted or session-lost `TcpAccept` tears down the guest-side connection, including one still in the guest's accept queue: after the request read, V_g reads EOF, reports `POLLRDHUP`, first write `EPIPE` (TCPACCEPT-END); "no byte reaches the application" also rests on H-1 (`flows_envGuestMissesHostClose` drops it) | Validated: V-24 (P-38, `spike/findings-v24.md`) |
| A-FID | Host flow ids are not reused within one pairing window | `FlowId` contract (design property) |
| K-B1 | Closing `T_g` / `Q_g` is observed by the host as the association's end, even when the host accepts after the close | Validated: V-22 (P-37: buffered data, then EOF) |
| K-B2 | `sock_release` reports the socket, or the audit finds orphan slots | R5-14 (no kernel unknown; no V-item) |
| K-B3 | Recreating a slot's framing / reassembly cells discards every parked frame; emptying them does not — sender-side queue / psock backlog residue refills the cell (`slots_bugCellsEmptiedOnly`); the design recreates (CELL-RECREATE) | Validated by V-5(c) for recreated cells; falsified for emptied cells (P-41, `spike/findings-v5c.md`) |
| K-B4 | When an owner dies, the surviving owner observes the end of each of its flow vsocks — in the measured case before the control-session loss — and the far ends the dying owner held are reset by linger (`owner_flows`, instance `half`: `HcOwnerDies`, `VsockEndReachesSurvivor`, unordered against the survivor's `HcLossObserved`) | Validated by V-13 (P-42: vsock end first in 415 of 415 deaths; linger resets 110/110 per cell; `spike/findings-v13.md`). Modelled in round 9 |
| K-C1 | A duplicate CID is refused at device creation and the first holder is unaffected; a CID is free right after its holder's last reference closes | P-16 |
| K-C2 | The only CID holders outside the pool's leases are Overdrive VMMs that survived a `serve` crash; they exit at any time | Environment, modelled as actions (`Boot`, `OrphanExit`); the absence of other holders is A-31 |
| K-C3 | A claim on an unowned instance is exclusive while any reference to its file is open, survives the handoff to the VMM, and is released at the last close | Validated: V-25 (P-39, `spike/findings-v25.md`; handoff on a prototype `fd=`) |
| K-D1 | With the `local` route and no lookup decision, a connection to `workload_addr:p` reaches a wildcard host listener on `p` (an intake listener is never bound to `workload_addr:p`, INTAKE-BIND, so it is reached only through its steering entry) | V-19 |
| K-D2 | With the nft rules: unmarked host-local prefix traffic is diverted to leg-C or rejected; remote prefix traffic diverted to leg-C or dropped; a divert with no leg-C socket falls through to the drop | V-19 |
| K-D3 | The rules, the route and the pinned steering persist across a `serve` crash or shutdown; the shared table is written and replaced only in single nft transactions (U-QP-1), so it is `absent` (a failed first write) or `complete`, never partial; a failed replacement and a crash leave it as it was | V-19 (persistence, U-4); V-26 (iv), (vii) (steering independent of the table); R5-43, R5-48 (`nft list table` read-back of both states, QPROJ-7 `SimMtlsIntercept::shared_table_state`) |
| K-D4 | Without a tagged route the prefix is not local: a packet to it is routed elsewhere unless the nft rules divert or reject it first | V-19 |
| K-D5 | Closing a listener resets the children in its accept queue | Not exercised by V-13 (P-42) or V-20 (P-46); kernel behaviour of `inet_csk_listen_stop`. R5-10's host-owner kill with a connect burst in flight is the Tier-3 check |
| K-D6 | The constant guest-prefix output reject and prerouting drop decide only connection-opening packets; `ct state established,related` is accepted before them, so replies of an admitted connection pass whatever their address (PREFIX-REPLY) | P-40 (V-26: replies timed out without the rule, completed with it); R5-43 (Tier-3, production rules). Not modelled: the models decide where new connections land |
| K-L1 | `sk_lookup` runs for a loopback-ingress connection to a `local`-route address that carries no socket, before the listener / wildcard lookup; a drop yields a TCP reset | Validated: V-26 (P-40, `spike/findings-v26.md`) |
| K-L2 | A prerouting TPROXY-assigned packet (leg-C divert) bypasses `sk_lookup` | Validated: V-26 (P-40) |
| K-L3 | Closing a listening socket removes it from the socket map in the same step, and its slot is free for a new entry (D8a-CAP) (`intake_bugEntryOutlives` drops it) | Validated: V-26 (iii), (x) (P-40) |
| K-L4 | The pinned link and map keep running with no process; after the owner exits the map holds no live socket | Validated: V-26 (P-40) |
| K-L5 | Ingress ifindex is loopback for host-local connects, the receiving device otherwise | Validated: V-26 (P-40) |
| K-L9 | `BPF_LINK_UPDATE` swaps the link's program atomically, and on error the link still runs the earlier program (`prefix_boot`: `swap` of an adopted link is one atomic step; a failed `swap` keeps the program) | Validated: V-26 (viii) (P-40) |
| K-L10 | `sk_lookup` assigns the listener an entry names whatever address and port that listener is bound to; binds of one port on distinct specific addresses coexist (`prefix_landing`: an entry reaches its listener, and nothing reaches a listener by its bound address) | Validated: V-26 (P-40: listeners on `A:0` and `127.0.0.1:34567` assigned; capacity fill) |
| A-MEM | A `steer` refused for kernel memory (`KernelMemory`) succeeds on a later retry: memory pressure is not permanent (`intake`: strong fairness on a successful `Steer` in `ServingReachable`; `ServingReachableNoMemRecovery` drops it) | Environment, not a kernel unknown (node memory capacity: #261); the exposure is observable (`guest_intake.bind_failed { retried: true }`), V-9 reports any occurrence at density |
| A-33 | The shipped steering program decides as specified: a program that loads is a correct program (`prefix_landing`: an attached program is `ours`) | The Tier-3 steering scenarios R5-1, R5-2, R5-3 and R5-25, run in CI on the pinned kernel (ADR-0068); not checked at runtime |
| A-31 | Only Overdrive writes the node's kernel objects (APPLIANCE) | The appliance image configuration (ADR-0068); not a runtime validation item |

## Properties and teeth

| Module | Safety (design instances) | Progress (TLC) | Teeth (`OFF`) |
|---|---|---|---|
| `prefix_boot` | `LocalOnlyOverLink` (invariant 6, M-4: `local` only while a pinned link with a loaded program exists), `LocalOnlyOverLinkWhileDown`, `RouteOnlyAfterSwap`, `NoWildcardReached`, `NothingDeliveredWhileDown`, `NoElsewhere` (U-4), `NeverDetached` (K-L9), `OpenConverged` (a boot opens only with its program in the pinned link and the route `local`), `PinOnlyAfterFirstAttach` (S-3: `pin` is a step of a first attach only) | `BootOpens` | `routeFirst`, `ignoreFailure`, `detach` (non-atomic swap), `unpinned` |
| `intake` | `NoWildcardReached`, `ReachOnlyServing`, `EntryNamesServingListener`, `NothingDeliveredWhileDown`, `RemoteNeverDirect`, `UnmarkedOnLegC`, `ActivationAllOrNothing`, `ProvisionedHasNoListener`, `NoReleaseWithListener`, `QuiescedClosesListeners`, invariant 12 (D8a-CAP): `ReservedWithinCapacity`, `ListenersWithinReserved`, `EntriesWithinListeners`, `NoCapacitySteerFailure`; `ParkedOnlyWhileServing` | `ServingReachable` (under A-MEM; a port parked by a reported defect is excluded), `ActivationCompletes` | `entryOutlives` (K-L3), `takedownBeforeClose`, `rollbackPartial`, `actOkPartial`, `teardownKeeps`, `quiesceKeeps`, `noIntakeAdmission` (D8a-CAP), `noIntakeAdmission` + `capRetry` (the revision-10 capacity-driven retry) |
| `quiescence` | `LatchIffHolders`, `HeldIsHolder`, `NoReopenBeforeRepair`, `NoRepairWithoutHold`, `QuiescedClosesListeners`, `RepairNeverWritesPrefix`, `FailStopLeavesPrefixAsBooted` (UP-12) | `IpRulesEpisodeEnds`, `FlowListenersEpisodeEnds` (UP-7), `IpRulesRepaired` — each under its own recovery's fairness only | `anyRestore`, `repairNoHold`, `redoNoCount` (UP-7; since S-6 its counterexample is on `FlowListenersEpisodeEnds`: IpRules damage no longer recurs at runtime, so `IpRulesEpisodeEnds` holds under it, recorded as `quiescence-hazard-redo-not-counted-ip-rules-lost`), `otherRepairsPrefix` |
| `shared_table` | `NoWildcardReached` (invariant 11, over the table in either state), `RemoteNeverDirect`, `NothingDeliveredWhileDown`, `NoReopenBeforeRepair`, `QuiescedClosesListeners` | `TableRepaired` (after the table is left absent, the IpRules recovery's repair completes under its hold, or `serve` stops) — under that recovery's own fairness and boot only | `steeringInNft`, `unpinned`, `nonLoopback`, `anyRestore` |
| C (also `lease_conf_*`, five offsets, S-2) | `CidUnique`, `ClaimHeld`, `NoLaunchClash`, `NoSkipWhileClaimable`, `NoLeakedClaim`, `RefusalEachClaimInUse` | `EventuallyPlaced` | `checkThenClaim`, `releaseBeforeVmm`, `rememberInUse` |

| A (`core`) | `OneLiveSession` (1), `NoInstallBeforeRead` (3), `NoAdmitWhenClosed` (4, U-1), `CurrentGeneration` (4+, ATTR-TOKEN / ATTR-LIVE-CLAIM: every live session and every attributed flow belongs to the lease holding the CID, which is not released), `NoCrossLaunchMsg` (ATTR-TOKEN: no control message on a session for a flow of another launch), `NoCrossLaunchSessionHeld` (ATTR-LIVE-CLAIM: no session refused because a connection of another launch holds the claim), `ClosedIsTerminal` (U-5) | `GuestHalfEnds` (U-5, K-A4), `SessionBecomesLive` (a provisioned launch whose guest keeps reconnecting gets a live session) | `sessionSplitClaim`, `installBeforeRead`, `admitSplit`, `pairedNoRecheck`, `noAbortOnLoss`, `teardownSkipsFlows`, `latePairedResurrects`, `peerCidOnly` (ATTR-TOKEN off → `CurrentGeneration`, `NoCrossLaunchMsg`), `claimBeforeToken` (claim before the token is read → `NoCrossLaunchSessionHeld`), `noClaimRecheck` (the first round-9 design: lease token, state checked at accept only → `NoCrossLaunchSessionHeld`, `CurrentGeneration`); environment `K-A4`; candidates `altTombstone` and `altTeardownClosesReading` (the rejected alternative to ATTR-LIVE-CLAIM: holds in the model) |
| A (`beacon`) | `BeaconCurrentLaunch` (every held beacon claim belongs to the CID's live launch), `BeaconNoCrossLaunchReady` (no READY of one launch released as another's), `BeaconNoCrossLaunchHeld` (no beacon connection refused because one of another launch holds the claim) | `ReadyReleased` (a live launch whose guest keeps re-dialling gets its READY released) | `beaconNoLiveRecheck` (lease token, no live check → `BeaconCurrentLaunch`, `BeaconNoCrossLaunchHeld`), `beaconPeerCidOnly` (→ `BeaconNoCrossLaunchReady`, `BeaconCurrentLaunch`) |
| A (`half`; also `half_host` / `half_guest`, S-4: the per-side conformance instances, in which `HcPeerDefectEnd(s)` — a defect of `s`'s own code — is enabled only for the side not under test) | Invariant 13 (HALFCLOSE-SIGNAL): `NoEofOnPeerLoss` (a far end reads EOF for a direction only if that direction's far end half-closed it — never because an owner died), `FinOnlyAfterAck` (no vsock shutdown before the peer's acknowledgement) | `HalfCloseCompletes` (no fault: an announced half-close reaches the far end), `AnnounceEnds` (a HalfClosing wait ends, faults included), `EarlyEofCarried` (an outer EOF before `Paired` is carried out after it) | `rdhupIsHalfClose` (the round-8 rule → `NoEofOnPeerLoss`), `noAck` (→ `HalfCloseCompletes`, `FinOnlyAfterAck`), `ackWaits` (the acknowledging side waits → `AnnounceEnds`), `earlyEofAborts` (→ `EarlyEofCarried`); candidate `altPendingWait` (V-13 variant C: holds `NoEofOnPeerLoss`, violates `ReceiverPendingEnds`) |
| B | `SlotHeldOnce` (2), `ReleasedSlotCarriesNoData` (2), `RouteCurrent` (2), `LatePairedAborted` (2, SLOT-ABORT), `NoReassociationWithoutNewDatagram` (U-2), invariant 14 `NoStaleGenerationStart` (CELL-RECREATE: no association starts from a readiness event of an earlier cell generation) | `ReleasedSlotBecomesFree` | `claimSplit`, `keepFrames` (P-35), `latePairedBySlot`, `ignoreLatePaired`, `keepFramesOnEnd` (U-2), `cellsEmptiedOnly` (K-B3 as falsified → `NoReassociationWithoutNewDatagram`), `fdDispatch` (raw-descriptor dispatch → `NoReassociationWithoutNewDatagram`, `NoStaleGenerationStart`); no audit fairness |

Not modelled: D25-BIND (R5-15), D26 / V-21 and D26-PINS (rule construction), V-20
and INTAKE-TAG-MAP, framing and unframe (D5 / D5a), USO-NO-SEGMENT,
FRAG-MAP-LIFECYCLE and IFACE-NO-STACK (egress program and boot checks sharing no
state with the modelled protocol), VIP resolution (D24 / D24a), the K2 drain
counters (the half-close handshake itself is modelled; a drain is one step and its
deadline a flow error), the linger clearing on a clean close, the CELL-RECREATE
step order and the quarantine of a slot whose new cell cannot be created (framing;
R5-14), the listen-state lag bound, D15-R3 deadlines, the beacon's EXEC / EXIT /
SHUTDOWN after READY, the guest's token check of a `TcpAccept` request (it
cannot fail in this abstraction: the host opens `V_h` only to the current
launch and a `V_g` dies with its VM), the stale-entry 4-tuple refusal of a new
connect (P-37: a refused connect is retried like any reconnect), INTAKE-PORTS
(a deploy check), PREFIX-REPLY (replies, K-D6), the intake host-port block
conflict check at `probe_startup` (INTAKE-BIND: a boot refusal before any
effect), the unframe
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
