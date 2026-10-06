# Model-check findings, round 2 — guest-flow owner (netns-density-295)

**Verdict.** With the decisions approved on 2026-10-06 (D8a-REVOKE, D8a-ROUTE,
D16-EXCL, U-1, U-2, U-4, U-6, and U-5 as pinned pending confirmation), every
design property holds in the model, and both round-1 counterexamples are gone.
Every new rule is load-bearing: switching it off brings a counterexample back.

The model finds **two new counterexamples against the pinned text** and **one
unstated assumption** behind U-5:

1. **D16-EXCL — a workload can be refused while usable offsets are free.** The
   exclusion set records every pre-READY exit, not only CID clashes, and is
   cleared only by READY or stop. After non-clash failures on every free
   offset, `assign` refuses with the pool-exhaustion refusal until the
   workload is stopped or `serve` restarts.
2. **D8a-REVOKE restore rule — a listener can serve with no element.** "A
   listener still pending removal whose port is wanted again keeps its
   element" assumes the element still exists. After a foreign flush (K-D3)
   it does not, and nothing re-adds it.
3. **U-5 rests on K-A4: the guest must observe the host's close of `V_h`.**
   The argument "the closing side sends `Abort`; session loss aborts every
   flow" misses a `TcpAccept` whose `V_g` the guest has not accepted yet.
   Without K-A4 such a guest half, connected to the application, is never
   closed. No V-item covers K-A4.

Gate recommendation: **PROCEED to the independent re-review after the user
rules on items 1 and 2 and confirms U-5 with K-A4 named and validated.** None
of the three is a safety leak to a host service.

- Specification (permanent home, ADR-0168): `specs/quint/guest-flow-owner/`
  — four specs, `hazard/` variants, `checks.toml`, `README.md`.
- Evidence: `specs/quint/guest-flow-owner/evidence/` (`summary.txt`,
  `verify-evidence.txt`, one append-only file per check, `traces/`,
  `sim-*.txt`, `typecheck.txt`). Round 1's evidence, specs and scripts moved
  to `evidence/r1/`; `spike-scratch/netns-density-295-quint-owner/` was
  removed. The round-1 findings file still names the old paths.

## Environment

| Item | Value |
|---|---|
| Host | Lima VM `overdrive` (recreated before this run), aarch64, 8 vCPU, 16 GiB |
| `uname -r` | `7.0.0-34-generic` |
| Quint | 0.32.0 standalone (`specs/quint/guest-flow-owner/.tools/bin/quint`, gitignored) |
| Apalache | 0.56.1, build `70cdaf4` (`QUINT_HOME=.tools/quint-home`) |
| TLC | TLC2 2.19 (08 Aug 2024), via `quint verify --backend tlc` |
| JVM | Temurin `openjdk 21.0.12.1 2026-08-18 LTS` |
| Final spec SHA-256 | `owner_flows.qnt` and the other seven files: `evidence/typecheck.txt` (last block) and the header of every evidence file; `verify-evidence.txt` checks that each check's last run used the current bytes |

**Infrastructure notes (not results).** Another workspace's unbounded TLC run
filled the VM's `/tmp` tmpfs (7.1 GB) during this run. Three of my runs
failed or hung because of it, one Apalache server port collided between two of
my own concurrent runner invocations, one Apalache run at depth 22 timed out,
and one TLC guard error (`sess.get(0)`) in an intermediate spec stopped TLC.
Each such entry carries a `NOTE … not a result` line in its evidence file and
in `summary.txt`, and was re-run. The runner now puts every JVM's temp
directory on `/var/tmp`.

## What changed in the model

| Decision | Model change | Module |
|---|---|---|
| D8a-REVOKE | A failed element removal leaves the listener bound and resetting (`rst`): element present, accepted connections reset; the removal is retried; the current item still completes (quiesce reports `unconfirmed`, activation returns `Err`). A resetting listener wanted again serves normally. `pool.release` requires no listener and no element | D |
| D8a-ROUTE | `converge_shared` keeps or adds the route only if the rules are present; otherwise it removes the tagged route and startup refuses | D |
| K-D3 as environment | `ForeignFlush` removes the rules and every element at any time; the audit's recovery quiesces, repairs the rules, and restore re-binds (G-V5 ordering) | D |
| U-4 | The route is never removed by `serve` (crash and shutdown alike) | D |
| U-6 | Activation is a multi-step item: begin → bring-up per wanted port → `Ok` (all admitted) or, after a bring-up failure, rollback → `Err` (CID Provisioned; a failed rollback revoke leaves that listener resetting) | D |
| D16-EXCL | Per-workload in-memory exclusion of offsets whose launch ended before READY (clash or any other pre-READY exit); `assign` skips them; refuses when every free offset is excluded; cleared by READY or stop; empty after a restart | C |
| U-2 | After `Refused`, `Abort` or the guest's pairing deadline, the slot's parked frames are discarded and only a new application datagram re-associates | B |
| U-1 | Already modelled in round 1 (atomic admission; step-6 stop for a closed flow); kept, with teeth | A |
| U-5 | New: host-opened `TcpAccept` on both sides. `Paired` / `Refused` for an unknown or closed flow is discarded with no reply, by host and guest; the closing side sends `Abort` while the session is live; session loss aborts every flow of that session on both sides | A |
| D25-BIND | Not modelled (a kernel bind fact; R5-15) | — |

## Verdicts

All 90 checks in `checks.toml` matched their expectation on the final specs
(`evidence/verify-evidence.txt`, pasted below). "Exhaustive" = TLC explored the
complete state graph of the instance; "depth N" = Apalache bounded search.

```text
# scripts/verify-evidence.py — 2026-10-06T16:05:50Z — uname -r 7.0.0-34-generic
owner-flows-safety-tlc                               tlc      holds      expect=holds     OK          12s  exhaustive, 220868 states
owner-flows-safety-apalache                          apalache holds      expect=holds     OK         201s  depth 12
owner-flows-guest-half-ends                          tlc      holds      expect=holds     OK          20s  exhaustive, 220868 states
owner-flows-witness-paired                           apalache violation  expect=violation OK          32s  depth 14
owner-flows-witness-reuse                            apalache violation  expect=violation OK           9s  depth 14
owner-flows-witness-quiesce-abort                    apalache violation  expect=violation OK          13s  depth 14
owner-flows-witness-late-completion                  apalache violation  expect=violation OK          17s  depth 14
owner-flows-witness-accept-paired                    apalache violation  expect=violation OK           9s  depth 14
owner-flows-witness-phantom-connect                  apalache violation  expect=violation OK          10s  depth 14
owner-flows-hazard-split-claim                       apalache violation  expect=violation OK          10s  depth 14
owner-flows-hazard-install-early                     apalache violation  expect=violation OK          11s  depth 14
owner-flows-hazard-admit-split                       apalache violation  expect=violation OK          19s  depth 14
owner-flows-hazard-paired-no-recheck                 apalache violation  expect=violation OK         123s  depth 14
owner-flows-hazard-no-abort-on-loss                  apalache violation  expect=violation OK          13s  depth 14
owner-flows-hazard-teardown-skips-flows              apalache violation  expect=violation OK          31s  depth 14
owner-flows-hazard-late-paired-resurrects            apalache violation  expect=violation OK          11s  depth 14
owner-flows-env-stale-conn                           apalache violation  expect=violation OK          15s  depth 14
owner-flows-env-guest-misses-host-close              tlc      violation  expect=violation OK          10s  exhaustive (stopped at counterexample)
owner-flows-alt-tombstone-misses-close               tlc      violation  expect=violation OK           9s  exhaustive (stopped at counterexample)
udp-slots-safety-tlc                                 tlc      holds      expect=holds     OK          31s  exhaustive, 702839 states
udp-slots-release-liveness                           tlc      holds      expect=holds     OK          72s  exhaustive, 702839 states
udp-slots-safety-apalache                            apalache holds      expect=holds     OK         462s  depth 8
udp-slots-witness-late-paired                        apalache violation  expect=violation OK          10s  depth 12
udp-slots-witness-routed                             apalache violation  expect=violation OK          10s  depth 12
udp-slots-witness-two-slots-one-socket               apalache violation  expect=violation OK           9s  depth 12
udp-slots-witness-reassociation                      apalache violation  expect=violation OK          13s  depth 12
udp-slots-hazard-claim-split                         apalache violation  expect=violation OK          12s  depth 12
udp-slots-hazard-keep-frames                         apalache violation  expect=violation OK          11s  depth 12
udp-slots-hazard-late-paired-by-slot-route           apalache violation  expect=violation OK          35s  depth 12
udp-slots-hazard-late-paired-by-slot-data            apalache violation  expect=violation OK          93s  depth 12
udp-slots-hazard-ignore-late-paired                  apalache violation  expect=violation OK          10s  depth 12
udp-slots-hazard-keep-frames-on-end                  apalache violation  expect=violation OK           9s  depth 12
udp-slots-hazard-no-audit-fairness                   tlc      violation  expect=violation OK          10s  exhaustive (stopped at counterexample)
cid-lease-safety-tlc                                 tlc      holds      expect=holds     OK           5s  exhaustive, 3441 states
cid-lease-two-foreign-safety-tlc                     tlc      holds      expect=holds     OK           6s  exhaustive, 564 states
cid-lease-single-safety-apalache                     apalache holds      expect=holds     OK           5s  depth 9
cid-lease-eventually-placed                          tlc      holds      expect=holds     OK           6s  exhaustive, 3441 states
cid-lease-boot-no-reclash-within-boot                tlc      holds      expect=holds     OK           6s  exhaustive, 2056 states
cid-lease-boot-cid-unique                            tlc      holds      expect=holds     OK           5s  exhaustive, 2056 states
cid-lease-boot-retry-same-foreign                    tlc      violation  expect=violation OK           5s  exhaustive, 32 states
cid-lease-boot-reclash-ever                          tlc      violation  expect=violation OK           4s  exhaustive (stopped at counterexample)
cid-lease-transient-safety                           tlc      holds      expect=holds     OK           4s  exhaustive, 63 states
cid-lease-transient-starved                          tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
cid-lease-transient-eventually-placed                tlc      violation  expect=violation OK           5s  exhaustive, 63 states
cid-lease-witness-clash                              apalache violation  expect=violation OK           4s  depth 9
cid-lease-hazard-no-exclusion                        tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
cid-lease-hazard-no-exclusion-foreign                tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
cid-lease-hazard-no-exclusion-reclash                tlc      violation  expect=violation OK           7s  exhaustive (stopped at counterexample)
cid-lease-hazard-first-fit-no-exclusion              tlc      violation  expect=violation OK           7s  exhaustive, 5 states
cid-lease-first-fit-with-exclusion                   tlc      holds      expect=holds     OK           6s  exhaustive, 3248 states
steering-safety-tlc                                  tlc      holds      expect=holds     OK           5s  exhaustive, 4830 states
steering-two-safety-tlc                              tlc      holds      expect=holds     OK           8s  exhaustive, 14578 states
steering-two-activation-completes                    tlc      holds      expect=holds     OK           8s  exhaustive, 14578 states
steering-two-quiesce-completes                       tlc      holds      expect=holds     OK           7s  exhaustive, 14578 states
steering-two-witness-err-with-resetting              tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-safety-apalache                             apalache holds      expect=holds     OK          64s  depth 16
steering-quiesce-completes                           tlc      holds      expect=holds     OK           6s  exhaustive, 4830 states
steering-listener-follows-guest                      tlc      holds      expect=holds     OK           7s  exhaustive, 4830 states
steering-resetting-resolves                          tlc      holds      expect=holds     OK           7s  exhaustive, 4830 states
steering-activation-completes                        tlc      holds      expect=holds     OK           5s  exhaustive, 4830 states
steering-boot-opens                                  tlc      holds      expect=holds     OK           5s  exhaustive, 4726 states
steering-witness-admitted                            apalache violation  expect=violation OK           7s  depth 18
steering-witness-crash-after-route                   apalache violation  expect=violation OK           5s  depth 18
steering-witness-quiesce-take-down                   apalache violation  expect=violation OK          13s  depth 18
steering-witness-second-allocation                   apalache violation  expect=violation OK           7s  depth 18
steering-witness-revoke-retry                        apalache violation  expect=violation OK          15s  depth 18
steering-witness-activation-err                      apalache violation  expect=violation OK          10s  depth 18
steering-hazard-revoke-fail-closes                   tlc      violation  expect=violation OK           5s  exhaustive (stopped at counterexample)
steering-hazard-revoke-fail-closes-names             apalache violation  expect=violation OK          17s  depth 18
steering-hazard-revoke-fail-closes-quiesce           tlc      violation  expect=violation OK           7s  exhaustive, 4254 states
steering-hazard-release-with-element                 tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-env-flush-safety                            tlc      holds      expect=holds     OK           6s  exhaustive, 23279 states
steering-env-flush-admitted-has-element              tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-env-flush-route-implies-rules               tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-env-flush-host-service                      tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-env-flush-witness-refusal                   apalache violation  expect=violation OK           7s  depth 18
steering-hazard-converge-ignores-rules               tlc      violation  expect=violation OK           5s  exhaustive (stopped at counterexample)
steering-hazard-route-first-boot-opens               tlc      violation  expect=violation OK           6s  exhaustive, 5 states
steering-hazard-route-first-safety                   tlc      holds      expect=holds     OK           5s  exhaustive, 5 states
steering-hazard-route-first-no-check                 tlc      violation  expect=violation OK           6s  exhaustive, 4 states
steering-hazard-route-first-no-check-host-service    apalache violation  expect=violation OK           6s  depth 18
steering-hazard-admit-first                          tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-hazard-close-first                          tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-hazard-quiesce-keeps                        tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-hazard-act-err-early                        tlc      violation  expect=violation OK           5s  exhaustive (stopped at counterexample)
steering-hazard-act-ok-partial                       tlc      violation  expect=violation OK           6s  exhaustive (stopped at counterexample)
steering-hazard-no-owner-fairness                    tlc      violation  expect=violation OK           6s  exhaustive, 4830 states
owner-flows-safety-apalache-deep                     apalache holds      expect=holds     OK         493s  depth 14
steering-safety-apalache-deep                        apalache holds      expect=holds     OK         321s  depth 20
steering-two-safety-apalache                         apalache holds      expect=holds     OK          94s  depth 16
# exit code: 0
```

Final spec hashes (`evidence/typecheck.txt`, last block):

```text
quint typecheck cid_lease.qnt -> exit 0 (sha256 d168176c354d8a91bbd7f899eaa02ddea7fbedd0f7513b022172de741fe0b0a4)
quint typecheck owner_flows.qnt -> exit 0 (sha256 f24b49feaf5286a575cc7c49875d2ce00aadef8d1ff8e38f3f905956c2918da3)
quint typecheck steering.qnt -> exit 0 (sha256 dc840b3501bc08f0588a0a22b3cee13e25e85b434f65d5302e3ef731ba29338a)
quint typecheck udp_slots.qnt -> exit 0 (sha256 b46148378362a12ce7778bbd252841a96082001cb78b67868bbb1af5132ec557)
quint typecheck hazard/cid_lease_hazards.qnt -> exit 0 (sha256 07ab6db7727494beb035d45660f5633f8e5030f0c1e9536ba42f393d6ecf499e)
quint typecheck hazard/owner_flows_hazards.qnt -> exit 0 (sha256 3bb69466801e4a666d72baf6b9e917ca5588a01d7da3b89d3ec57e40df45e472)
quint typecheck hazard/steering_hazards.qnt -> exit 0 (sha256 2ba84281f36056936482acaeb4c2b544565293df2a8e85723eb76cd31ea5fd1c)
quint typecheck hazard/udp_slots_hazards.qnt -> exit 0 (sha256 edef8104826e53b50d2db4ef5a9efcf36a51b404f50e9c504c3ad3b9bbe98689)
```

**CI subset.** 85 of the 90 checks finished under 2 minutes and carry
`ci = true`. Not in CI: `owner-flows-safety-apalache` (201 s),
`owner-flows-hazard-paired-no-recheck` (123 s), `udp-slots-safety-apalache`
(462 s), and the two deep runs (493 s, 321 s). Each of those properties also
has a TLC exhaustive check (or a shorter hazard) that is in CI.

Simulation cross-check (`quint run`, seed 20261006, 40 steps; not a proof):
`slots_ok` 50,000 traces, `flows_ok` / `steer_ok` / `steer_two` 20,000 traces,
`lease_ok` 20,000 traces of 30 steps — all `[ok] No violation found`
(`evidence/sim-summary.txt`).

### Expectations recorded before the runs, and the two that changed

Every `expect` in `checks.toml` was written before the recorded run. Two
results differed and are reported, not hidden:

- `steering-env-flush-admitted-has-element`: predicted `holds`, TLC found a
  counterexample. It is finding 2; `expect` is now `violation`, with a comment
  in `checks.toml`.
- `steering-hazard-act-err-early`: predicted `violation`, TLC reported
  `holds`. The cause was vacuity: with one declared port a failed activation
  has nothing to roll back. The hazard and a new design instance `steer_two`
  now declare two ports; the hazard is violated and `steer_two`'s properties
  hold. The witness `WitnessNoErrWithResetting` needs about 20 steps, so it
  moved from Apalache (depth 18, a bound artefact `holds`) to TLC.

## Round-1 counterexamples — gone

**Round-1 finding 1 (stale element after a failed revoke).** Under
D8a-REVOKE, `steer_ok` and `steer_two` hold `NoHostServiceReached`,
`ElementNamesListener`, `NoReleaseWithElement`, `NoListenerWhileQuiesced`
and `ElementsMirrorState` exhaustively, with one bind failure and one revoke
failure allowed; `QuiesceCompletes` and `ResettingResolves` hold. The
round-1 text as a hazard (`steer_bugRevokeFailCloses`) brings the
counterexample back in 15 steps (activate, session loss, revoke fails, the
listener closes with its element present —
`traces/steering-hazard-revoke-fail-closes.txt`). The element guard on
`pool.release` is defence in depth: it only matters if an element can
outlive its listener, so its teeth need both rules off
(`steering-hazard-release-with-element`).

**Round-1 finding 2 (CID-clash retry returns to the failed offset).** Under
D16-EXCL, `lease_ok` (three workloads, concurrent placement and release),
`lease_twoForeign` and `lease_single` hold `RetryNeverSameCid`,
`RetryDifferentUnlessAllHeld`, `RetryNeverSameForeignCid` and
`NoReclashWithinBoot` exhaustively. Without the exclusion
(`lease_bugNoExclusion`) the round-1 concurrent trace returns in 8 steps
(`traces/cid-lease-hazard-no-exclusion.txt`). With the exclusion, next-fit is
no longer what carries the retry guarantee: first-fit plus the exclusion also
holds (`cid-lease-first-fit-with-exclusion`).

### The CID-retry guarantee after a `serve` restart

The exclusion set and the cursor are in memory, so a restart empties both.
The checked guarantee is exactly ADR-0156's:

- **Within one `serve` process:** a workload's retry never receives an offset
  on which its launch already ended before READY while another offset is free
  (`RetryDifferentUnlessAllHeld`, `NoReclashWithinBoot` hold, including in
  `lease_boot` across restarts for the within-process part).
- **After a restart:** the retry may receive the same offset again. In
  `lease_single_boot` a workload clashes on offset 0, `serve` restarts, and
  the first `assign` returns offset 0 (4 steps,
  `traces/cid-lease-boot-retry-same-foreign.txt`). Each restart costs at most
  one more failed launch per excluded offset per workload (`NoReclashEver`
  violated, `NoReclashWithinBoot` holds). A different workload can receive an
  offset another workload excluded and fails there at most once.

R5-19's oracle must assert the within-process property only.

## New counterexamples

### Finding r2-1 — D16-EXCL refuses a workload while usable offsets are free

Trace (`lease_transient`: one workload, offset 0 foreign, two non-clash
pre-READY exits allowed; `traces/cid-lease-transient-starved.txt`, TLC, 9
steps):

1. The workload gets offset 0 and clashes. Offset 0 is excluded.
2. Its retry gets offset 1. The launch ends before READY for another reason
   (for example a VMM start failure or a guest that exits before READY).
   Offset 1 is excluded: the driver cannot tell this from a clash.
3. Its retry gets offset 2 and fails the same way. Offset 2 is excluded.
4. All three offsets are free, offsets 1 and 2 are usable, and `assign`
   refuses with the pool-exhaustion refusal. `EventuallyPlaced` is violated:
   the refusal persists, because only READY (impossible without a lease) or a
   stop clears the set.

On a node with many free offsets this needs one pre-READY failure per free
offset, so it is reached by a workload that fails before READY repeatedly
for a reason unrelated to CIDs (a bad image, a guest that dies early): after
enough retries it stops being placed and is reported as pool exhaustion. On a
nearly full node two transient failures on the two free offsets are enough.
The refusal also names the wrong cause: operators are told the pool is
exhausted (`.claude/rules/rust.md` § distinct failure modes).

**Decision affected: D16-EXCL (ADR-0156, R5-19).** Options for the user:

- (a) When every free offset is excluded, fall back to the free offsets in
  cursor order instead of refusing (the guarantee becomes "different while a
  non-excluded offset is free"; an all-clash node still converges to the
  refusal only through real exhaustion).
- (b) Bound the set (for example the most recent *k* offsets) or expire
  entries.
- (c) Keep the refusal but give it its own typed variant that names repeated
  pre-READY failure, not pool exhaustion.

### Finding r2-2 — a resetting listener wanted again can serve with no element

Trace (`steer_envFlush`, TLC, 19 steps;
`traces/steering-env-flush-admitted-has-element.txt`):

1. Boot completes; an allocation is activated; the guest listens on declared
   port 1; the listener is admitted (element present).
2. Forwarding is quiesced. The element removal fails, so the listener stays
   bound and resetting with its element (D8a-REVOKE).
3. Other software flushes the shared table (K-D3): rules and elements are
   gone.
4. Recovery repairs the rules (forwarding is already quiesced), then restore
   reopens forwarding.
5. Port 1 is wanted again. Per the pinned rule ("a listener still pending
   removal whose port is wanted again keeps its element and serves
   normally") the listener serves — with no element. Marked clients (leg-S
   inbound mTLS, health probes) are reset at the marked-reset rule
   indefinitely: nothing re-admits a listener the owner believes is admitted.

This is fail-closed, not a leak (`NoHostServiceReached` and
`SafetyUnderFlush` hold), but probes fail and inbound mTLS to that port is
refused until the allocation is torn down. It needs two faults (a failed
revoke and a foreign flush).

**Decision affected: D8a-REVOKE / `restore_forwarding`.** Candidate: when a
resetting listener's port is wanted again, re-admit it
(`admit_intake_listener`, treating `DuplicateIntakeAdmission` as present)
instead of assuming its element; or have the IpRules recovery re-derive
`intake_listeners` from the owner's admitted listeners.

### Finding r2-3 — U-5's discard rule depends on K-A4 (no V-item)

U-5 (pinned 2026-10-06, exact rule awaiting confirmation) discards a
`Paired` / `Refused` for an unknown or closed flow with no reply, because
"the side that closes a flow before `Paired` sends `Abort` while the session
is live, and session loss aborts every flow on both sides". For a host-opened
`TcpAccept` the model finds two paths where neither reaches the guest:

- **Abort before accept** (`traces/owner-flows-env-guest-misses-host-close.txt`,
  18 steps; reachable in the design too: witness
  `owner-flows-witness-phantom-connect`, 10 steps). The host quiesces and
  sends `Abort` for flow 1 before the guest has accepted `V_g`. The guest
  does not know flow 1 and discards the `Abort` (U-5). It then accepts `V_g`,
  connects `C` to the application and sends `Paired`, which the host
  discards.
- **Session loss before accept** (`traces/owner-flows-alt-tombstone-misses-close.txt`,
  19 steps). The host's session for the VM is lost while `V_g` waits in the
  guest's accept queue; the host aborts the flow and can send nothing. The
  guest reconnects; it then accepts `V_g` on the new session, pairs, and the
  host discards the `Paired`. The guest's "abort every flow of the lost
  session" never covered this flow: it was not yet in the guest's table.

In both, only the host's close of `V_h` (reset) tells the guest. With K-A4
assumed, `GuestHalfEnds` holds exhaustively (`owner-flows-guest-half-ends`);
without it the guest half — holding an application connection — is never
closed (`owner-flows-env-guest-misses-host-close`). A guest tombstone for an
`Abort` of an unaccepted flow (`ALT_GUEST_ABORT_TOMBSTONE`) closes the first
path but not the second (`owner-flows-alt-tombstone-misses-close`).

Even with K-A4, the first path means the application accepts a connection
for a client the host already aborted and sees it closed at once.

**Decision affected: U-5 (ADR-0166), awaiting user confirmation.** Confirm
U-5 with K-A4 stated as its assumption — "a guest that accepts `V_g` after
the host closed `V_h`, or before, observes the close at its first I/O" — and
add a validation item for it. P-22 / K-B1 cover only the reverse direction
(guest closes, host observes). V-22 (K-A2) is the nearest existing item and
could be extended.

### K-D3 outcome (foreign flush), checked

With other software allowed to flush the shared table at any time
(`steer_envFlush`):

- `SafetyUnderFlush` holds exhaustively: a host service is reached only
  inside a flush window, which runs from the flush until the rules are
  re-installed — by the audit's recovery while `serve` is up, or by the next
  boot's `start_shared_owner` while it is down. `converge_shared` never keeps
  or adds the route without the rules; the refusal (route removed) is
  reachable (`steering-env-flush-witness-refusal`).
- `RouteImpliesRules` and `NoHostServiceReached` are violated, as expected
  for an environment fault. Shortest trace: a flush between boot steps 3 and
  4 leaves the verified route with no rules until the audit runs
  (`traces/steering-env-flush-host-service.txt`, 4 steps).
- D8a-ROUTE is load-bearing: without it, a flush between steps 2 and 3 makes
  `converge_shared` add the route with no rules (4 steps,
  `traces/steering-hazard-converge-ignores-rules.txt`). With D8a-ROUTE, the
  reversed boot order no longer leaks; it never starts
  (`steering-hazard-route-first-safety` holds, `BootOpens` violated).

The window's length is what V-23 measures; the model only bounds where it can
occur.

## Teeth — every rule is load-bearing

| Hazard instance | Rule switched off | Property violated |
|---|---|---|
| `steer_bugRevokeFailCloses` | D8a-REVOKE (round-1 text) | `NoHostServiceReached`, `ElementNamesListener`, `QuiesceCompletes` |
| `steer_bugRevokeFailClosesNoElemGuard` | D8a-REVOKE incl. the release guard | `NoReleaseWithElement` |
| `steer_envFlushBugConverge` | D8a-ROUTE | `ConvergeNeedsRules` |
| `steer_bugRouteFirst` / `…NoCheck` | D8a boot order (with / without D8a-ROUTE) | `BootOpens` / `RouteImpliesRules`, `NoHostServiceReached` |
| `steer_bugAdmitFirst`, `steer_bugCloseFirst` | bring-up / take-down order | `NoHostServiceReached` |
| `steer_bugQuiesceKeeps` | M-7 quiescence | `NoListenerWhileQuiesced` |
| `steer_bugActErrEarly`, `steer_bugActOkPartial` | U-6 all or nothing | `ActivationAllOrNothing` |
| `steer_liveTeeth` | owner fairness (non-vacuity) | `QuiesceCompletes` |
| `lease_bugNoExclusion`, `lease_single_bugFirstFitNoExclusion` | D16-EXCL (and next-fit) | `RetryDifferentUnlessAllHeld`, `RetryNeverSameForeignCid`, `NoReclashWithinBoot`, `RetryNeverSameCid` |
| `slots_bugKeepFramesOnEnd` | U-2 | `NoReassociationWithoutNewDatagram` |
| `slots_bugClaimSplit`, `…KeepFrames`, `…LatePairedBySlot`, `…IgnoreLatePaired`, `slots_liveTeeth` | round-1 rules (D25, P-35, SLOT-ABORT, audit) | as in round 1 |
| `flows_bugLatePairedResurrects` | U-5 (host acts on a late `Paired`) | `ClosedIsTerminal` |
| `flows_bugAdmitSplit`, `flows_bugPairedNoRecheck` | U-1 | `NoAdmitWhenClosed` |
| `flows_bugSplitClaim`, `…InstallEarly`, `…NoAbortOnLoss`, `…TeardownSkipsFlows` | L-3, H-1, loss abort, teardown abort | as in round 1 |

## Remaining underspecified points

| # | Point | Decision | Why it matters |
|---|---|---|---|
| U-r2-1 | U-5's exact rule awaits confirmation; its argument needs K-A4 (finding r2-3) | U-5, ADR-0166 | Without K-A4 a guest `TcpAccept` half is orphaned |
| U-r2-2 | What `assign` does when every free offset is excluded (finding r2-1) | D16-EXCL | Refusal while usable offsets are free; wrong diagnosis |
| U-r2-3 | Whether restoring a resetting listener re-admits its element (finding r2-2) | D8a-REVOKE | Serving listener with no element after a flush |
| U-r2-4 | That IpRules damage is repaired inside quiescence (quiesce → repair → restore). The model assumes G-V5's ordering applies to it. If the rules are repaired without quiescing, every admitted listener loses its element at the flush and nothing re-adds it (finding r2-2 without the revoke failure) | G-V5, ADR-0124 | `AdmittedHasElement` would fail after any flush |
| U-r2-5 | A-FID: host flow ids are not reused within a pairing window. `FlowId` "skips ids still live in its flow table", so a closed id can be reused after a wrap of the 2^31 space; a late `Paired` for the old flow would then pair the new one. Not reachable in practice; not modelled | `FlowId` wire contract | Only if the id space is ever narrowed |
| U-r2-6 | D25-BIND (bind refused only on ports holding an intake) is a kernel fact; not modelled | D25 | R5-15 |

## Modelling assumptions and what discharges them

As in round 1 (`specs/quint/guest-flow-owner/README.md` § *Assumptions*),
with these changes: **K-A2** is now discharged by **V-22**; **K-D3** "no other
software removes the table" is no longer assumed — it is modelled as
`ENV_FOREIGN_FLUSH` and measured by **V-23**; **K-A4** (the guest observes the
host's close of `V_h`) is new and has **no V-item** (finding r2-3).

Not modelled, so this run gives no evidence on them: D26 / V-21, V-20, D5 /
D5a framing, D24 / D24a resolution, half-close drain, the listen-state lag,
D15-R3 deadlines, the beacon beyond its claim discipline.

## Limits

- Small instances: one CID or address; two control connections; two
  guest-opened and one host-opened flow; two UDP slots; three lease offsets;
  two guest ports (one or two declared). Generations bounded at 2. Time is
  abstract.
- Apalache depths are bounds; TLC holds are complete only for the instance.
  Apalache cannot handle module C's next-fit fold beyond the single-workload
  instance; TLC is the evidence there.
- The model encodes the design text, not an implementation.
