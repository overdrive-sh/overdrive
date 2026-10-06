# Spike findings — Quint model of the guest-flow owner protocol (netns-density-295)

**Verdict.** The ordering rules the design pins hold in the model, and each one is
load-bearing: removing any one of them produces a counterexample. The model also
finds **two defects in the design text** and **one unvalidated kernel assumption**:

1. **D8a / D23 — a failed `IntakeAdmission::revoke` leaves a stale
   `intake_listeners` element.** The design's error handling closes the listener
   anyway. A marked connection (probe or leg-S) to that port then reaches a
   wildcard host service on the same port. The stale element also survives
   quiescence, teardown and lease release. No runtime path removes it, because
   `revoke(self)` consumes the only handle. A candidate fix (keep the listener
   bound and retry the revoke) passes every check.
2. **D16 (next-fit, M-9) — a CID-clash retry can receive the same CID while
   other offsets are free.** ADR-0156 claims "different unless every other offset
   is held". That claim is false when other placements and releases run
   concurrently, and after a `serve` restart. Both cases follow from the
   cursor's wrap and reset rules.
3. **K-A2 has no V-item.** CID attribution assumes the kernel resets a dead VM's
   connections, including those still in a host accept queue, before the CID is
   leased again. If that assumption is false, a stale control session from the
   previous VM is attributed to the next allocation on the same CID.

Gate recommendation: **PROCEED to DISTILL after the user rules on items 1 and 2.
Add a validation item for item 3.** Also pin the underspecified points in
§ *Underspecified*.

- Probe: `spike-scratch/netns-density-295-quint-owner/` (`spec/`, `scripts/`,
  `evidence/`).
- Precedent copied: the pinned tool versions, the Apalache-plus-TLC split, the
  per-check evidence files and the bug-flag hazard instances of
  `raft-corrosion-vs-gossip-log/spike-scratch/flat-cluster-intent-log/increment-{a,b}`.
- No quint-connect drivers: no implementation exists yet. Conformance belongs to
  DISTILL.

## Environment

| Item | Value |
|---|---|
| Host | Lima VM `overdrive`, aarch64, 8 vCPU, 16 GiB |
| `uname -r` | `7.0.0-34-generic` |
| Quint | 0.32.0 standalone (`.tools/bin/quint`) |
| Apalache | 0.56.1, build `70cdaf4` (`QUINT_HOME`) |
| TLC | TLC2 2.19 (08 Aug 2024), via `quint verify --backend tlc` |
| JVM | Temurin 21.0.12.1, `JVM_ARGS=-Xmx6g` |
| Spec SHA-256 | `owner_flows.qnt` `541059fa…0b76` · `udp_slots.qnt` `225759f0…939c` · `cid_lease.qnt` `225b1c91…f7e9` · `steering.qnt` `a6a648f7…4b3a` · `steering_extra.qnt` (finding-1 witness) `f7e410e6…e06d` (every evidence file records the full hash; all match) |

**VM state.** The Lima root filesystem is mounted `emergency_ro` (ext4 remounted
read-only). `/var/tmp` refused writes. `/tmp` (tmpfs) and the virtiofs workspace
were writable, so every run used them. The VM was **not** recreated: doing so
would kill runs in other workspaces that share it. The orchestrator should
recreate it (`.claude/rules/testing.md` § *An unusable VM is recreated*).

## Method

Four Quint modules. Each models the design text at the abstraction the design
pins.

| Module | Scope | Design sources |
|---|---|---|
| A `owner_flows.qnt` | Control session per CID (atomic claim, loss); guest-opened flow admission; request read → install; non-blocking connect → `Paired`; provision / activate / teardown / release; quiesce / restore. One CID, reused by successive allocations (`gen`) | § *Control session* (D18a, ADR-0166); § *Per-kind total orders* TcpConnect 3–6 and Datagram 3 (D18, H-1); § *Owner and provisioner* (M-7); § *Composition*; ADR-0160 |
| B `udp_slots.qnt` | Guest slot pool: claim, association, `Paired` / `Refused` / `Abort`, release on socket close (every slot), lost report → audit, idle release, late `Paired` → `Abort` | § *Per-kind total orders* (Datagram, UDP slot release); D25 intake model; SLOT-ABORT; P-35 |
| C `cid_lease.qnt` | Lease offsets → CID; next-fit cursor; foreign CIDs (clash); retire → release → retry; `serve` restart | ADR-0156 (D16, M-9); G-V4; R5-19 |
| D `steering.qnt` | Boot steps with a crash between any two; steering rules, shared `local` route and `intake_listeners` elements persisting across crashes; intake listener bring-up / take-down; `ListenState`; session loss; quiesce / restore; teardown / release; where a connection to `workload_addr:port` lands | § *Composition* boot 7–9 (D8a ordering invariant); § *Intake listener mirroring* (D23); § *mTLS forwarded-outbound port*; ADR-0152, ADR-0163; G-V1, G-V5, G-V8 |

**Checkers.**

- **Apalache** (`quint verify`) runs bounded symbolic search to depth N.
- **TLC** (`--backend tlc`) explores the complete reachable state graph of each
  finite instance, with no step bound. Liveness is checked only with TLC, under
  weak fairness, as in increment-b.
- **`quint run`** simulates random traces. It cannot reach paths longer than
  about 10 specific steps, so it is a cross-check only.

**Expectation record.** Each instance encodes its expected verdict in its name,
written before the official runs: `*_ok` is the design model; `*_bug*` switches
off one design rule (teeth); `*_env*` and `steer_revokeFails` model a fault or
the literal design text. The `scripts/run-all.sh` header gives the label
scheme: **OK** = expected to hold; **W** = non-vacuity witness, expected to be
violated (the behaviour is reachable); **T** = teeth, expected to be violated;
**F** = finding (a fault or the design text), predicted to be violated. Each
check's hypothesis, prediction and falsification are below. They were set when
the instance was written and are written out here after the runs.

**Process notes (honest record).**

- A scratch simulation first reported `OneLiveSession` violated in the design
  model. The cause was a spec error: `claim' = claim or x` parses as
  `(claim' = claim) or x`. It was fixed before any official run.
- Module D's first at-rest invariants were checked before the take-down that
  belongs to the same serialized item (M-7). Events are now gated until that
  take-down finishes.
- Three official runs were disturbed by an orphaned Apalache server after a
  tool-cap kill. Each such file carries a `NOTE` / `KILLED` line saying it is
  not a result, and the check was re-run as `*-rerun`.

## Invariants and verdicts

"Bound" is Apalache's search depth or TLC's complete state graph. A TLC "holds"
covers every reachable state of that instance.

| # | Invariant (design clause) | Instance | Verdict | Bound / evidence |
|---|---|---|---|---|
| 1 | `OneLiveSession` — a CID has at most one live control session (D18a, L-3) | A `flows_ok` | **Holds** | TLC exhaustive: 13,592 states, depth 30. Apalache depth 14 (`Safety` conjunction). `tlc-OK-A-flows-Safety.txt`, `verify-OK-A-flows-Safety-14.txt` |
| 2a | `SlotHeldOnce` — a slot is held by at most one (socket, destination); one slot per pair (D25) | B | **Holds** | TLC exhaustive on the small instance (2 sockets, 1 destination, 2 slots, 3 association ids): 122,519 states, depth 19. Apalache depth 8 on the 2-destination instance. Simulation: 50,000 traces × 40 steps |
| 2b | `ReleasedSlotCarriesNoData` — a free slot holds no frame, route or flow; an owned slot holds only its owner's frames; no frame reaches another (socket, destination)'s host association (D25, P-35) | B | **Holds** | Same as 2a |
| 2c | `RouteCurrent` — a routed slot routes to its own current association | B | **Holds** | Same as 2a |
| 2d | `LatePairedAborted` — a late `Paired` for a released slot is answered with `Abort` (SLOT-ABORT) | B | **Holds** | Same as 2a |
| 3 | `NoInstallBeforeRead` — no flow installed before its full 16-byte request is read (H-1) | A | **Holds** | TLC exhaustive; Apalache depth 14 |
| 4 | `NoAdmitWhenClosed` — no registered flow and no `Paired` while quiesced, without a live session, or for a non-Active CID (G-V3, G-V5, M-7) | A | **Holds** | TLC exhaustive; Apalache depth 14 |
| 4+ | `CurrentGeneration` — every live session and registered flow belongs to the allocation currently leasing the CID, and that allocation is not released (§ *Composition*: "the CID is never freed while a flow, listener or control session … exists") | A | **Holds under K-A2**; **counterexample without it** (finding 3) | TLC exhaustive; `F-A-staleConn-*` |
| 5 | `RetryNeverSameCid` — a CID-clash retry never receives the same CID (as the task states it) | C | **Counterexample** (permitted by ADR-0156's exception, but also reached outside it) | `F-C-concurrent-RetryNeverSameCid` (Apalache) |
| 5′ | `RetryDifferentUnlessAllHeld` — ADR-0156's own claim | C | **Counterexample** (finding 2) | TLC and Apalache: `F-C-concurrent-RetryDifferentUnlessAllHeld` |
| 5″ | `RetryNeverSameForeignCid` — ADR-0156 consequence "does not hit the same foreign CID again" | C | **Counterexample** (finding 2), concurrent and after restart | `F-C-concurrent-*`, `F-C-singleBoot-*` |
| 5‴ | `RetryNeverSameCid` with one allocation and no restart | C `lease_single` | **Holds** | TLC exhaustive (11 states); Apalache depth 9 |
| — | `CidUnique` — leased CIDs unique | C | **Holds** | TLC exhaustive: 9,741 states, depth 35. Apalache stalled on the next-fit `fold` encoding (killed at 28 min; depth 9 timed out at 180 s) |
| 6 | `RouteImpliesRules` — the shared route is never present without the guest-prefix rules, after a crash at any boot step (D8a, H-4) | D `steer_ok` | **Holds** | TLC exhaustive: 1,110 states, depth 27, crash allowed between every pair of steps. Apalache depth 22 |
| 7 | `NoHostServiceReached` — a connection to a workload address reaches only a bound intake listener or leg-C, never another host service (D8a, B-2, R5-1) | D `steer_ok` | **Holds** with no revoke failure; **counterexample under the design's revoke-failure handling** (finding 1) | TLC exhaustive; Apalache depth 22; `F-D-revokeFails-*` |
| 7+ | `ElementNamesListener` — while `serve` is open, every element names a listening intake listener (§ *Architecture enforcement*) | D | **Holds** / **counterexample under revoke failure** | as 7 |
| 7++ | `ElementsMirrorState` — at rest, an element exists only while Active ∧ ¬quiesced ∧ session ∧ listening ∧ declared (G-V8) | D | **Holds** | TLC exhaustive; Apalache depth 22 |
| 7+++ | `NoListenerWhileQuiesced` — at rest, no intake listener and no element while quiesced (G-V5, M-7) | D | **Holds** / **counterexample under revoke failure** | as 7 |

### Liveness (TLC, weak fairness on the owner's steps)

| Property | Instance | Verdict | Evidence |
|---|---|---|---|
| `ReleasedSlotBecomesFree` — a slot held by a released socket returns to the pool, under fair report handling and a fair audit (D25) | B small | **Holds** (122,519 states) | `tlc-OK-B-slotsSmall-ReleasedSlotBecomesFree.txt` |
| `QuiesceCompletes` — once quiesced, every intake listener closes and every element is removed (G-V5) | D `steer_ok` | **Holds** | `tlc-OK-D-steer-QuiesceCompletes.txt` |
| `ListenerFollowsGuest` — a wanted declared port is eventually admitted (D23) | D `steer_ok` | **Holds** (admit failure allowed once) | `tlc-OK-D-steer-ListenerFollowsGuest.txt` |
| `QuiesceCompletes` with revoke failure (design text) | D `steer_revokeFails` | **Violated** — the element is never removed (finding 1) | `tlc-F-D-revokeFails-QuiesceCompletes.txt` |
| `QuiesceCompletes` with the candidate fix | D `steer_revokeFailsKeep` | **Holds** | `tlc-OK-D-revokeFailsKeep-QuiesceCompletes.txt` |

### Non-vacuity witnesses (each expected to be violated, and it is)

`Paired` is reached (W-A-paired, 12 steps). A CID is reused by a second
allocation (W-A-reuse). Quiesce aborts a fully-read flow (W-A-quiesceAbort). A
connect completes after its flow was aborted (W-A-lateCompletion). A late
`Paired` is answered with `Abort` (W-B-latePaired). A slot is routed
(W-B-routed). One socket holds two slots (W-B-twoSlotsOneSocket). A slot
re-associates after its association ended (W-B-reassociation). A CID clash
happens (W-C-single-clash). An intake listener is admitted (W-D-admitted).
`serve` crashes with the route present (W-D-crashAfterRoute). A quiesce
take-down is in progress (W-D-quiesceTakeDown). A second allocation becomes
Active on the reused address (W-D-secondAllocation).

## Teeth — each design rule is load-bearing

Every hazard variant switches off exactly one rule. Each was **predicted to
violate** its invariant, and each does (Apalache; module C and D teeth also
with TLC).

| Variant | Rule removed | Invariant violated | Counterexample in plain words |
|---|---|---|---|
| `flows_bugSplitClaim` | L-3: the session check and the claim are one operation | 1 `OneLiveSession` | Two connections from one CID both see "no live claim", then both claim. Two live sessions (8 steps) |
| `flows_bugInstallEarly` | H-1: read the request, then install | 3 `NoInstallBeforeRead` | V_h is installed with 0 request bytes read. The request would go to the verdict and be dropped |
| `flows_bugAdmitSplit` | Admission check and registration in the CID's flow table are one step | 4 | A flow passes the check, teardown runs (Retiring) and misses it, then it registers as accepted on a Retiring CID |
| `flows_bugPairedNoRecheck` | Step 6 stops for a flow already Closed | 4 | Mid-connect, the session is lost and the flow aborted. The connect completes and `Paired` is emitted anyway, on a CID with no session (13 steps) |
| `flows_bugNoAbortOnLoss` | Session loss aborts the CID's flows | 4 | An accepted flow survives session loss |
| `flows_bugTeardownSkipsFlows` | Teardown aborts every flow | 4+ | The lease is released while an accepted flow from it still exists |
| `slots_bugClaimSplit` | The slot claim is atomic | 2a | Two destinations pick the same free slot and both take it (holders = 2) |
| `slots_bugKeepFrames` | Release discards parked frames (P-35) | 2b | A socket closes and its slot returns to the pool with its frame still parked (3 steps) |
| `slots_bugLatePairedBySlot` | A late `Paired` is matched by flow id, never by slot | 2b, 2c | Socket 1 → dest 2 opens fid 1. Socket 1 closes and the slot is released. Socket 2 → dest 1 claims the same slot. The late `Paired(1)` routes that slot to fid 1's association, so socket 2's datagram for dest 1 goes to dest 2 (7 steps). This is the P-35 hazard, now cross-destination |
| `slots_bugIgnoreLatePaired` | SLOT-ABORT | 2d | A late `Paired` is dropped silently |
| `lease_single_bugFirstFit` | Next-fit (M-9) | 5 | With lowest-free assignment, a single allocation's clash retry gets the same foreign CID (4 steps) |
| `steer_bugRouteFirst` | D8a boot order: rules before route (revision 4's order) | 6, 7 | Boot adds the route at step 2, before the rules (violation at the second step). From then until `start_shared_owner`, the prefix is local with no rules and every connection reaches the host service. If `serve` crashes there, the window never closes |
| `steer_bugAdmitFirst` | Bring-up: bind → listen → register → admit | 7 | The element exists before the listener. A marked connect reaches the wildcard host service |
| `steer_bugCloseFirst` | Take-down: revoke → unregister → close | 7 | The listener is closed while the element remains. Same leak |
| `steer_bugQuiesceKeeps` | Quiescence closes intake listeners (M-7) | 7+++ | A listener is still admitted at rest while quiesced |
| `slots_live`, no audit fairness | Level-triggered audit backstop (K-B2) | `ReleasedSlotBecomesFree` | A lost `sock_release` report strands the slot forever |
| `steer_liveTeeth`, no owner fairness | — (the liveness property is not vacuous) | `QuiesceCompletes` | The owner never runs its take-down |

## Counterexamples against the design

### Finding 1 — revoke failure leaves a stale element (D8a, D23; affects R5-1, V-19, G-V5, G-V8)

The design text is in § *mTLS forwarded-outbound port*: "Intake admission errors
(`admit_intake_listener`, `revoke`) are handled by the owner exactly like an
intake bind failure: the listener is closed, `guest_intake.bind_failed` is
emitted … retried at the audit cadence while the guest still reports
listening." The model applies this to a revoke failure.

Trace (`evidence/traces/F-D-revokeFails-NoHostServiceReached.txt`, Apalache,
14 steps; also found by TLC):

1. Boot steps 1–4: the rules, then the route; `serve` opens.
2. An allocation is provisioned. Its control session opens and it is activated.
3. The guest application listens on declared port 1. The owner receives
   `ListenState` and runs bind → register → admit: element 1 is present and the
   listener is admitted.
4. Forwarding is quiesced, which requires take-down. `revoke` fails, so the
   owner closes the listener (design text). Element 1 stays in
   `intake_listeners`.
5. `serve` is open and marked senders exist (leg-S, probes). A marked connect to
   `workload_addr:1` passes the marked-reset rule, because the element admits
   it. The local route delivers it, and the only socket on port 1 is the host's
   wildcard service.

This violates invariant 7 and the pinned "every element names a listening
listener" invariant. The same trace with teardown in place of quiescence
reaches `Release`, and the lease is freed with the element present: the
guard names flows, listeners and sessions, not elements. Checked: witness
`F-D-revokeFails-releaseWithElement`, Apalache, 15 steps
(`traces/F-D-revokeFails-releaseWithElement.txt`, spec
`steering_extra.qnt`). Under the candidate fix the same property holds (TLC
exhaustive, `tlc-OK-D-revokeFailsKeep-NoReleaseWithElement.txt`). The next allocation on that
address then inherits a pre-admitted port, and its TCP probe passes against
the host service before its application listens (exactly R5-1). TLC also shows
`QuiesceCompletes` is violated: nothing ever removes the element, because
`IntakeAdmission::revoke(self)` consumes the admission handle on `Err`. The
only remover is the next boot's convergence.

**Candidate fix, checked.** On a revoke failure, keep the listener bound, keep
the admission, and retry the revoke. Teardown and release wait for success.
Instance `steer_revokeFailsKeep`: `Safety` holds (TLC exhaustive, 2,142
states; Apalache depth 16) and `QuiesceCompletes` holds. This needs
`revoke(&self)` (or a returned handle on `Err`) and a pinned retry, which is a
change to the interface contract. **User decision needed (D8a).**

### Finding 2 — next-fit does not guarantee a different CID on retry (D16 / M-9; affects R5-19, G-V4)

**Concurrent placements** (`traces/F-C-concurrent-RetryDifferentUnlessAllHeld.txt`,
8 steps; offset 0 is a foreign CID):

1. A1 gets offset 0 (cursor → 1). A1's launch clashes on the foreign CID.
2. A3 gets offset 1 (cursor → 2). A1's lease is released.
3. A3 runs. A2 gets offset 2 (cursor wraps to 0). A3 stops, so offset 1 is free.
4. A1 retries and next-fit returns offset 0 again: the first free offset at or
   after the cursor. Offset 1 is free.

ADR-0156's "different unless every other offset is held" and its consequence
"next-fit means the retry does not hit the same foreign CID again" are both
false under concurrent assignment and release.

**`serve` restart** (`traces/F-C-singleBoot-RetryNeverSameForeignCid.txt`,
4 steps): a single allocation clashes on offset 0, `serve` restarts, the
cursor resets to the first offset (as specified), and the retry clashes on
offset 0 again.

Impact: a clash is a non-terminal launch failure, so each recurrence costs one
more launch attempt. Repeated recurrence is possible but needs the cursor to
wrap or a restart each time. R5-19 as written ("a CID-collision launch failure
followed by placement retry gets a different CID") is not an unconditional
property, so a seeded-sim oracle for it would be flaky.

Options for the user:

- (a) Weaken the ADR claim to "the next assignment after a failure does not
  return the failed offset unless the cursor has wrapped". Restate R5-19 the
  same way.
- (b) Make it true: keep an in-memory exclusion of offsets whose launch failed
  with a CID clash until a later successful launch or a bounded count. This is
  internal structure, but the ADR claim changes.

**User decision needed (D16 / M-9).**

### Finding 3 — CID attribution across allocations rests on an unvalidated kernel assumption (K-A2; D17, D18a, ADR-0166)

Trace (`traces/F-A-staleConn-CurrentGeneration.txt`, 8 steps):

1. Allocation 1 provisions the CID, and its VM connects a control session,
   still in the host accept queue.
2. The VM exits, and the allocation is torn down and released.
3. Allocation 2 provisions the same CID (P-16: reusable at once).
4. The owner accepts the queued connection. Attribution is by peer CID only, so
   it becomes allocation 2's live session, from a dead VM.

The design is safe only if the kernel resets every connection of a released
vhost device, including accept-queue entries, before the CID can be reassigned.
P-16 proves CID reuse; nothing proves the reset. The same applies to flow
connections on 1240/1241 and to the beacon listener (D17). **No V-item
discharges K-A2.** Proposed: extend V-10 (or add a V-item) with "connect from
the guest, kill the VMM before the host accepts, relaunch a VM with the same
CID, then accept: the accepted socket must fail at first I/O".

### Environment fault (not a design defect) — something else flushes the nft table (K-D3)

`steer_envFlush`: if other host software removes the shared table, for example
a host firewall reload, the persistent route makes the guest prefix locally
delivered with no steering. Every connection then reaches host wildcard
services (`F-D-envFlush-*`). This lasts until the audit repairs the rules while
`serve` is up, or indefinitely while `serve` is down.

The trace also shows that `converge_shared` adds the route without checking
that the rules exist: the flush lands between boot steps 2 and 3. Cheap
hardening: `converge_shared` refuses or repairs when the steering rules are
absent before it adds or keeps the route. K-D3 ("not removed by other
software") has no V-item. Surface to the user as a residual risk of D8a / H-4.

## Underspecified — needs a decision ID pinned

| # | Gap | Decision | Why it matters (model evidence) |
|---|---|---|---|
| U-1 | Flow events are not in the per-allocation order (M-7 lists provision, activate, teardown, quiescence / restore, session open / loss and `ListenState` only). The design pins the *outcomes* (G-V6: "install racing quiesce or teardown; `Paired` after Retiring → aborted") but not the two mechanisms the model shows are required: (i) the admission check and registration in the CID's flow table are one step relative to quiesce / teardown / session loss; (ii) the step-6 continuation stops for a Closed flow. | D18 / D18a / M-7 | Removing either one violates invariant 4 (`T-A-admitSplit`, `T-A-pairedNoRecheck`). State both as contract lines so DISTILL can write the oracle. |
| U-2 | After `Refused` / `Abort` / pairing timeout of a datagram association whose slot is still owned: are the parked frames discarded, is the slot released, and what triggers re-association? (A-18 covers only the connected-socket case.) | D25, SLOT-ABORT, A-18 / V-5(c) | If parked frames are kept, cell readiness re-triggers association at once. To a `Policy` / `HostInternal`-refused destination this loops (two vsock connects plus one refusal per attempt). The model discards them; re-association is reachable (W-B-reassociation). |
| U-3 | The release guard ("the CID is never freed while a flow, listener or control session … exists") does not name `intake_listeners` elements. | D8a | Finding 1's stale element survives `pool.release`. |
| U-4 | Is the shared route removed at graceful shutdown? Roadmap new step 13 says "removal at graceful shutdown". ADR-0152 and the ownership table say it is never removed by `serve` and persists across restarts, graceful or not. | D8a / H-4 | Either reading satisfies invariant 6 (removing the route only helps), but the two texts contradict each other. |
| U-5 | The host's handling of `Paired` / `Refused` for an unknown or Closed host-opened flow id (a `TcpAccept` aborted by quiescence while the guest's `Paired` is in flight). SLOT-ABORT covers only the guest datagram side. | D18a | Not modelled; probably "ignore, V_h already closed", but unpinned. |
| U-6 | `activate`'s all-or-nothing intake bring-up ("a bind or element failure … returns `Err` with the CID still Provisioned") versus the post-activation retry-at-audit-cadence path. | D23 / G-V3 | The model treats activation and bring-up as separate steps, so activation atomicity is **not** checked. |
| U-7 | D15-R3 (connect bound to a dead non-mesh destination) changes deadlines only. | D15-R3 | Time is abstract in the model; the user's 2-minute ruling does not affect any invariant here. |

## Modelling assumptions and what discharges them

| ID | Assumption (kernel / environment) | Discharged by |
|---|---|---|
| K-A1 | Peer-CID attribution is authentic (vhost stamps the device CID) | **V-10** (bind level P-17; packet level open) |
| K-A2 | A released vhost device's connections, including accept-queue entries, are reset before the CID is reassigned | **None** — finding 3 |
| K-A3 | A socket installed before its request is read hands the request to the verdict, which drops it | P-23 controls; **R5-9** (Tier-3) |
| K-B1 | Closing T_g / Q_g is observed by the host as the end of the association, even when the host accepts after the close | P-22 (bounded); no V-item names it |
| K-B2 | `sock_release` reports the socket, or the level-triggered `sock_diag` audit finds orphan slots | **R5-14** (Tier-3 + seeded sim); D25 not yet approved at writing |
| K-B3 | Emptying or recreating the framing and reassembly cells discards every parked frame (no residue in a psock backlog) | **None** directly; R5-14 partially. The P-35 hazard depends on it |
| K-C1 | A duplicate CID is refused at device creation and the first VM is unaffected; a CID is free right after VMM exit | P-16 (proven, bounded) |
| K-C2 | Foreign vhost CIDs are fixed while the model runs | Environment; none |
| K-D1 | With the `local` route and no rules, a connection reaches a wildcard host listener unless a listener bound to `workload_addr:p` exists | **V-19** |
| K-D2 | With the rules: a marked connection not in `intake_listeners` is reset; other non-diverted traffic is dropped / rejected; a leg-C divert with no socket (`serve` down) falls through to that drop | **V-19** |
| K-D3 | The rules, route and elements persist across a `serve` crash; nft batches apply atomically; no other software removes the table | **V-19** (persistence "while `serve` is down"); "no other software" — **none** |
| K-D4 | Without the route, no guest-prefix address is locally delivered | **V-19** |
| K-D5 | Closing a listener resets children still in its accept queue | ADR-0160 claim; **V-13** (owner-kill resets) |

Not modelled, so this spike gives no evidence on them: D26 / V-21 (host-internal
deny set and its output rule), V-20 (intake-child tag identity), D5 / D5a
framing and unframe, D24 / D24a VIP resolution, K2 half-close drain, the
2 ms listen-state lag, D15-R3 deadlines, and the beacon (D17) beyond its shared
claim discipline.

## Limits

- Small instances: one CID or address; 2 control connections; 2 flows; 2 slots;
  3 association ids; 3 lease offsets; 2 ports. Generations are bounded at 2.
  Time is abstract.
- Module B's 2-destination instance was too large for TLC (more than 13.6 M
  distinct states and still growing at 15 min, in a scratch feasibility run
  whose output was not retained). It is covered by Apalache to depth 8 and by 50,000
  simulated traces of 40 steps. The 1-destination instance is exhaustive. Its
  two tags (socket 1, socket 2) still exercise cross-owner misdelivery.
- Module C's next-fit `fold` defeats Apalache. TLC exhaustive is the evidence
  for C's holds; Apalache found every C counterexample within 9 steps.
- Apalache depths are bounds, not proofs. TLC holds are complete only for the
  finite instance checked.
- The models encode the design text. They do not show that any implementation
  conforms; quint-connect drivers belong to DISTILL.

## Evidence index

- `evidence/typecheck.txt`: four specs, exit 0, with hashes.
- `evidence/verify-summary.txt` and `verify-<label>.txt`: Apalache. Each file is
  append-only with command, hash, versions, `uname -r`, raw output and exit
  code.
- `evidence/tlc-summary.txt` and `tlc-<label>.txt`: TLC.
- `evidence/sim-summary.txt` and `sim-<label>.txt`: simulator.
- `evidence/traces/*.txt`: condensed per-step diffs of every F / T
  counterexample (`scripts/condense-trace.py`).
- `evidence/00-install-tools.log`: tool installation, `uname -r`.
- Reproduce inside Lima: `scripts/install-tools.sh`, then
  `scripts/run-all.sh <section>` (sections `typecheck`, `A-verify`, `A-tlc`,
  `B-verify`, `B-rerun`, `B-sim`, `B-tlc`, `C-verify`, `C-tlc`, `C-tlc2`,
  `D-verify`, `D-tlc`, `D-tlc2`).
