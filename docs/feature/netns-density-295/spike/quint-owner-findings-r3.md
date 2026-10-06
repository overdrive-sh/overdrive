# Model-check findings, round 3 — guest-flow owner (netns-density-295)

**Verdict.** With the user's decisions of 2026-10-06 on the round-2 findings
(U-5 confirmed with assumption K-A4 / V-24, D16-PREF, D8a-READMIT, and the
accepted bounded exposures D8a-PROBE and D8a-FLUSH), all 106 checks in
`checks.toml` match their pre-recorded expectation on the final spec bytes:
36 properties hold and 70 expected violations are found (witnesses, teeth,
environment faults, documented consequences). Both round-2 counterexamples
are gone, and each new rule has teeth: switching it off brings the
counterexample back.

No new counterexample contradicts a decision as written. The run shows two
**consequences of D16-PREF** that the current guarantee text (ADR-0156,
R5-19) does not state, and one **ordering point under D8a-READMIT** that the
model had to assume:

1. **D16-PREF: a retry can fail again on an offset it already failed on,
   within one `serve` process.** If every free offset is excluded for the
   workload, the fallback takes one of them, and a foreign CID clashes again.
   The r2 guarantee "never assigned `o` again while another offset is free"
   no longer holds; what holds is "never an excluded offset while an
   unexcluded one is free".
2. **D16-PREF: the fallback can return the offset that just failed** while
   another free (excluded) offset exists, when another workload's assignment
   moved the cursor. This contradicts ADR-0156's sentence "different unless
   every other offset is held" if that sentence is kept.
3. **D8a-READMIT / D8a-FLUSH: the exposure ends only if a recovery's repair is
   not pre-empted by another restore.** With a restore from any source allowed
   while the recovery holds quiescence, quiesce / restore can alternate after a
   flush and the repair never runs. The model states fairness on the repair
   itself; the design should say who may restore while a recovery is in
   progress.

Gate recommendation: **PROCEED to the independent DESIGN re-review**, with
items 1–3 surfaced to the user as wording / ownership confirmations. None is
a safety leak; each concerns what the guarantee text promises.

- Specification: `specs/quint/guest-flow-owner/` (permanent home, ADR-0168).
- Evidence (append-only, round 2 untouched): `specs/quint/guest-flow-owner/evidence/r3/`
  — `full-run.log` (spec hashes, typecheck, the full check), `summary.txt`,
  `checks/` (every check's log and Apalache ITF trace), `traces/` (verbose TLC
  counterexamples with `.condensed` step lists), `sim-*.txt`, `first-pass/`
  (superseded runs, with notes).
- The old `scripts/` runner is superseded by `cargo xtask quint`; it moved to
  `evidence/scripts-r2/` because it produced the r1/r2 evidence.

## Environment

| Item | Value |
|---|---|
| Host | Lima VM `overdrive`, aarch64 |
| `uname -r` | `7.0.0-34-generic` |
| Quint | 0.32.0 (`/usr/local/bin/quint`, provisioned by the Lima template) |
| Apalache | 0.56.1; TLC via `quint verify --backend tlc` |
| JVM | `openjdk version "21.0.12.1" 2026-08-18 LTS` |
| Runner | `cargo xtask quint typecheck`; `cargo xtask quint check --subsystem guest-flow-owner` (sequential, 2026-10-06T17:53Z–19:28Z) |

Final spec SHA-256 (head of `evidence/r3/full-run.log`):

```text
27a22c9bf29f12ca84bc03261b4e05f8fd77e26b6527e3b9b1a8089f8eb9e6b7  cid_lease.qnt
32bab80732b6164cd7e3957bb533cb0757a9a4e1ba91e75e47988a3c006dbbbb  owner_flows.qnt
bf5a9a773aa3667da17feaa518c4c98b618f8aa592f4dab732d328ec4ef7d26e  steering.qnt
b46148378362a12ce7778bbd252841a96082001cb78b67868bbb1af5132ec557  udp_slots.qnt
779fd1b9c082cffe88ca539aa552a57c08ee23809dcc89fa4c9eb0da93d1b952  hazard/cid_lease_hazards.qnt
3bb69466801e4a666d72baf6b9e917ca5588a01d7da3b89d3ec57e40df45e472  hazard/owner_flows_hazards.qnt
2e5f083642bc36a776406a4466ece3b48e44931fcb45de1c0fba1186010e955b  hazard/steering_hazards.qnt
edef8104826e53b50d2db4ef5a9efcf36a51b404f50e9c504c3ad3b9bbe98689  hazard/udp_slots_hazards.qnt
0fe56242ec1a01bae72d70e4e1f9a059f4478ca2ed7dabf10c24ccaf9cad58d0  checks.toml
```

**Infrastructure notes (not results).** Quint intermittently hung at Apalache
server start or after printing its verdict (three times over the round, once
in the sequential run, with no concurrent load). `cargo xtask quint` has no
per-check timeout, so a hang blocks the run. The sequential run's hung check
(`owner-flows-env-guest-misses-host-close`, 39 min at server start) was
killed by hand and re-run alone (`rerun-guest-misses.log`, `violation`,
9.8 s); a watchdog killing any checker JVM older than 20 minutes ran for the
rest of the run and fired no further (`watchdog.log`). Every hung run shows
`tool-error`, and none is counted as a result.

## What changed in the model

| Decision | Model change | Module |
|---|---|---|
| U-5 (confirmed) | Text: a `Paired`, `Refused` or other control message for an unknown or closed flow id is discarded and counted, no reply, except a late `Paired` for a released UDP slot (`Abort`, module B, unchanged). The discard behaviour was already modelled; the counter is telemetry and is not modelled | A |
| K-A4 (V-24) | New guest action `GuestQueueReset`: the host's close of `V_h` removes a `V_g` still in the guest's accept queue. Kept: an accepted `V_g` observes the close (`GuestSeesHostClose`), and the guest may still accept `V_g` before the reset arrives (the race). `GuestHalfEnds` now covers queued as well as accepted guest halves | A |
| D16-PREF | `assign` = next-fit over free unexcluded offsets; if none, next-fit over free excluded offsets; refuse only when no offset is free. New ghost and invariants `NoExcludedWhileUnexcludedFree`, `NoRefusalWhileFree`; `Safety` = `CidUnique ∧ NoExcludedWhileUnexcludedFree ∧ NoRefusalWhileFree`. New instance `lease_transientTwo` (two workloads, two non-clash pre-READY exits). Teeth: `BUG_REFUSE_WHEN_ALL_EXCLUDED` (the r2 rule) | C |
| D8a-READMIT | A resetting listener whose port is wanted again is re-admitted (element added), not assumed to keep its element. `RulesRepair` requires quiescence (unchanged; now with teeth). Teeth: `BUG_REWANT_KEEPS_ELEMENT` (the r2 rule), `BUG_REPAIR_OUTSIDE_QUIESCE` | D |
| D8a-PROBE | No model change: the reachable resetting-listener state is the witness `steering-witness-revoke-retry`, labelled accepted in `checks.toml` | D |
| D8a-FLUSH | New temporal property `ExposureEnds`: a host service reached after a foreign flush stops being reachable, under fair owner steps, fair boot, fair restart, strong fairness on the recovery's quiesce-on-damage and on its repair, and `serve` eventually staying up. Teeth: the same property without recovery fairness | D |

## Verdicts

Result of record: `evidence/r3/summary.txt` (generated from `full-run.log`
and `rerun-guest-misses.log`). "exhaustive" = TLC explored the complete state
graph of the instance; "depth N" = Apalache bounded search.

```text
owner-flows-safety-tlc                             tlc      got=holds      expect=holds     MATCH       15.2s  exhaustive ci=true
owner-flows-safety-apalache                        apalache got=holds      expect=holds     MATCH      248.2s  depth 12   ci=false
owner-flows-guest-half-ends                        tlc      got=holds      expect=holds     MATCH       19.8s  exhaustive ci=true
owner-flows-witness-phantom-connect                apalache got=violation  expect=violation MATCH       13.5s  depth 14   ci=true
owner-flows-witness-queue-reset                    apalache got=violation  expect=violation MATCH       10.5s  depth 14   ci=true
owner-flows-env-guest-misses-host-close            tlc      got=violation  expect=violation MATCH        9.8s  exhaustive ci=true
owner-flows-alt-tombstone-misses-close             tlc      got=violation  expect=violation MATCH       15.6s  exhaustive ci=true
cid-lease-safety-tlc                               tlc      got=holds      expect=holds     MATCH        5.0s  exhaustive ci=true
cid-lease-transient-starved                        tlc      got=holds      expect=holds     MATCH        4.8s  exhaustive ci=true
cid-lease-transient-eventually-placed              tlc      got=holds      expect=holds     MATCH        4.8s  exhaustive ci=true
cid-lease-transient-two-safety-tlc                 tlc      got=holds      expect=holds     MATCH        5.1s  exhaustive ci=true
cid-lease-transient-two-eventually-placed          tlc      got=holds      expect=holds     MATCH        6.3s  exhaustive ci=true
cid-lease-fallback-reclash                         tlc      got=violation  expect=violation MATCH        4.8s  exhaustive ci=true
cid-lease-fallback-same-foreign                    tlc      got=violation  expect=violation MATCH        4.8s  exhaustive ci=true
cid-lease-retry-different-unless-all-held          tlc      got=holds      expect=holds     MATCH        4.9s  exhaustive ci=true
cid-lease-transient-two-retry-different            tlc      got=violation  expect=violation MATCH        4.9s  exhaustive ci=true
cid-lease-hazard-no-exclusion-preference           tlc      got=violation  expect=violation MATCH        5.1s  exhaustive ci=true
cid-lease-hazard-refuse-starved                    tlc      got=violation  expect=violation MATCH        5.1s  exhaustive ci=true
cid-lease-hazard-refuse-while-free                 tlc      got=violation  expect=violation MATCH        5.1s  exhaustive ci=true
cid-lease-hazard-refuse-eventually-placed          tlc      got=violation  expect=violation MATCH        5.2s  exhaustive ci=true
steering-safety-tlc                                tlc      got=holds      expect=holds     MATCH        5.2s  exhaustive ci=true
steering-env-flush-safety                          tlc      got=holds      expect=holds     MATCH        7.4s  exhaustive ci=true
steering-env-flush-admitted-has-element            tlc      got=holds      expect=holds     MATCH        7.4s  exhaustive ci=true
steering-hazard-rewant-keeps-element               tlc      got=violation  expect=violation MATCH        7.1s  exhaustive ci=true
steering-hazard-repair-outside-quiesce             tlc      got=violation  expect=violation MATCH        7.0s  exhaustive ci=true
steering-env-flush-host-service                    tlc      got=violation  expect=violation MATCH        6.8s  exhaustive ci=true
steering-env-flush-exposure-ends                   tlc      got=holds      expect=holds     MATCH        8.8s  exhaustive ci=true
steering-hazard-exposure-no-audit-fairness         tlc      got=violation  expect=violation MATCH        8.2s  exhaustive ci=true
steering-witness-revoke-retry                      apalache got=violation  expect=violation MATCH       17.9s  depth 18   ci=true
...
# 106 of 106 match; ci=true: 101
```

(Excerpt, copied from `evidence/r3/summary.txt`: the rows this round changed
or added, plus anchors. The full 106-row table is in that file.)

**CI subset.** 101 of 106 checks finished under 2 minutes in the sequential
run and carry `ci = true`. Not in CI: `owner-flows-safety-apalache` (248 s),
`owner-flows-hazard-paired-no-recheck` (158 s), `udp-slots-safety-apalache`
(578 s), `owner-flows-safety-apalache-deep` (658 s),
`steering-safety-apalache-deep` (393 s). Each of those properties also has a
TLC exhaustive check or a shorter hazard in CI. (The `checks.toml` header
said the opposite — "ci = false only for checks under 2 minutes"; corrected.)

Simulation cross-check (`quint run`, seed 20261006; not a proof):
`lease_transientTwo` `Safety` (20,000 traces, 30 steps), `flows_ok` `Safety`,
`steer_envFlush` `SafetyUnderFlush` and `AdmittedHasElement` (20,000 traces,
40 steps each) — all `[ok] No violation found` (`evidence/r3/sim-*.txt`).

### Expectations recorded before the runs, and the one that differed

Every `expect` in `checks.toml` was written before the first r3 run. One
first-pass result differed and is reported, not hidden
(`evidence/r3/first-pass/`):

- `steering-env-flush-exposure-ends`: predicted `holds`; the first pass found
  a lasso. The first formulation put strong fairness on one action,
  "quiesce on damage OR repair". After a flush, quiesce and restore alternate
  forever; every quiesce step satisfies the fairness of the disjunction, so
  the repair is never forced. The defect was in the property's fairness
  assumption, not in the model of the design. Fairness is now stated on each
  recovery step separately, and the property holds. The lasso is the basis of
  item 3 above.

## Round-2 counterexamples — gone

**r2-1 (D16-EXCL refused a workload while usable offsets were free).** Under
D16-PREF, `lease_transient` holds `NoStarvedOnUsableOffset` and
`EventuallyPlaced` exhaustively, and `lease_transientTwo` holds `Safety` and
`EventuallyPlaced`. The r2 rule as a hazard (`lease_transient_bugRefuse`)
brings the r2 trace back (10 states) (`traces/cid-lease-hazard-refuse-starved.condensed`):
clash on 0, non-clash pre-READY exits on 1 and 2, then all three offsets free
and the workload refused.

**The within-process guarantee holds.** `NoExcludedWhileUnexcludedFree`
("`assign` never returns an excluded offset while an unexcluded one is free")
holds (as part of `Safety`) in `lease_ok` (three workloads), `lease_twoForeign`,
`lease_single` (Apalache depth 9), `lease_transient`, `lease_transientTwo`, and
with first-fit (`cid-lease-first-fit-with-exclusion`). `lease_boot` is checked
only for `CidUnique` and `NoReclashWithinBoot`. Without the exclusion
(`lease_bugNoExclusion`) it fails (`cid-lease-hazard-no-exclusion-preference`).
`NoRefusalWhileFree` holds in the same instances and fails under the r2
rule (`cid-lease-hazard-refuse-while-free`).

**r2-2 (a resetting listener wanted again served with no element after a
flush).** Under D8a-READMIT, `steer_envFlush` holds `AdmittedHasElement`
exhaustively. Teeth:

- The r2 restore rule (`steer_envFlushBugRewantKeeps`) reproduces the r2
  trace (20 states): admit, quiesce, revoke fails (resetting), foreign flush,
  repair, restore, the listener serves with no element
  (`traces/steering-hazard-rewant-keeps-element.condensed`).
- Repair outside quiescence (`steer_envFlushBugRepairOutsideQuiesce`) fails
  (16 states) without any revoke failure: an admitted listener loses its
  element at the flush, the rules are repaired while it keeps serving, and
  nothing re-adds the element (`traces/steering-hazard-repair-outside-quiesce.condensed`).
  This is U-r2-4, now closed by D8a-READMIT's "repair only inside
  quiescence".

## U-5 with K-A4 (V-24)

With K-A4 as decided — the host's close of `V_h` of an aborted or
session-lost `TcpAccept` tears down the guest-side connection, including a
`V_g` still in the guest's accept queue — `GuestHalfEnds` holds exhaustively
(`owner-flows-guest-half-ends`): every guest half whose host side closed,
accepted or still queued, is eventually torn down. Both removal paths are
reachable: the queue removal (`owner-flows-witness-queue-reset`) and the
accept-then-observe path.

Without K-A4 (`flows_envGuestMissesHostClose`), the guest half is orphaned.
Trace (17 states, then stuttering; `traces/owner-flows-env-guest-misses-host-close.condensed`):
the host opens a `TcpAccept`, the allocation is torn down (the host closes
`V_h` and the session) while `V_g` waits in the guest's accept queue, and
then nothing ever ends `V_g`. A guest `Abort` tombstone still does not
replace K-A4 (`owner-flows-alt-tombstone-misses-close`, unchanged).

`owner-flows-witness-phantom-connect` is still reachable: the guest can
accept `V_g` and connect the application before the reset arrives, then tears
both down when it does. This matches A-28 / R5-31 ("when the guest accepts
`V_g` it sees it ended … the application's accepted connection is reset").
The model does not represent bytes, so R5-31's "no byte reaches it" is not
checked here; V-24 covers it.

## Consequences of D16-PREF (items 1 and 2)

The decision trades the r2 refusal for a fallback. Two consequences follow
that the current guarantee text does not state.

**Item 1 — a workload can fail again on an offset it already failed on,
within one `serve` process** (`cid-lease-fallback-reclash`,
`cid-lease-fallback-same-foreign`; `lease_ok`, 8 states,
`traces/cid-lease-fallback-reclash.condensed`):

1. Workload 1 gets offset 0, a foreign CID, and clashes. Offset 0 is
   excluded for it.
2. Workloads 2 and 3 take offsets 1 and 2.
3. Workload 1's lease is released. Its only free offset is 0, excluded.
   Under D16-PREF it falls back to 0 and clashes again.

On a real node this means a workload keeps launching onto CIDs it already
clashed on while every other free offset is also excluded. Before, it waited
behind the pool-exhaustion refusal. Each attempt costs a VMM launch, at the
cadence of the placement retry backoff (not modelled). The r2 checks
`NoReclashWithinBoot` and `RetryNeverSameForeignCid` are now expected
violations; they still hold in `lease_boot` (two workloads), where an
unexcluded offset is always free at a retry.

**Decision affected:** D16-PREF's guarantee text in ADR-0156 and R5-19. R5-19
currently says "never assigned `o` again while another offset is free". The
checked guarantee is "never an excluded offset while an unexcluded offset is
free". R5-19's oracle should assert that, not the r2 wording.

**Item 2 — the fallback can return the offset that just failed while another
free excluded offset exists** (`cid-lease-transient-two-retry-different`;
`lease_transientTwo`, 15 states,
`traces/cid-lease-transient-two-retry-different.condensed`):

1. Workload 1 fails before READY on offsets 0 (clash), 1 and 2 (other
   causes), in that order. All three are excluded for it; its last failure
   is offset 2.
2. Workload 2 takes and releases offset 0, then takes offset 1. The cursor
   now points at 2.
3. Workload 1 retries. Free: 0 and 2, both excluded. "First free excluded
   offset in cursor order" is 2, the offset it just failed on, although 0 is
   free.

In `lease_ok`, where only foreign clashes exclude offsets,
`RetryDifferentUnlessAllHeld` still holds. It fails once two workloads and
non-clash failures interleave.

**Decision affected:** ADR-0156's claim "the retry is different unless every
other offset is held". Options for the user:

- (a) Restate the claim to what holds (item 1's wording) and drop "different
  unless every other offset is held".
- (b) In the fallback, skip the offset of the workload's most recent failure
  when another free offset exists. The claim then holds again in this
  instance. This would need a hazard check and a re-run.

## D8a-FLUSH and D8a-PROBE — accepted, bounded

**D8a-FLUSH.** The exposure is checked as documented and bounded:

- `steering-env-flush-host-service` and `steering-env-flush-route-implies-rules`
  are expected violations: a host service is reachable after a foreign flush.
  This is the accepted exposure, not a defect.
- `steering-env-flush-safety` holds: a host service is reached only inside a
  flush window.
- `steering-env-flush-exposure-ends` holds: the exposure ends at the audit
  recovery's repair (serve up) or at the next boot's `start_shared_owner`.
  The property assumes `serve` eventually stays up and that each recovery
  step is fair. Without recovery fairness the exposure can last forever
  (`steering-hazard-exposure-no-audit-fairness`).

The model has abstract time. The bound D8a-FLUSH states (the audit period
plus ADR-0124's recovery bound, else fail-stop) is a timing claim that V-23
measures, not this model.

**Item 3 — who may restore while a recovery holds quiescence.** The first
pass shows that if any party may call `restore_forwarding` between the
recovery's quiesce and its repair, quiesce / restore can alternate after a
flush and the repair never runs
(`first-pass/steering-env-flush-exposure-ends.verbose-trace.txt`, lasso at
states 13–14). D8a-READMIT fixes the recovery's own order (quiesce → repair
→ audit → restore). It does not say whether another component's restore can
reopen forwarding while the rules are still missing. **Decision affected:**
D8a-READMIT / G-V5. Suggested wording: while a firewall recovery holds
quiescence, `restore_forwarding` from any other source does not reopen
forwarding until the recovery's audit has passed. The model assumes this (as
fairness on the repair).

**D8a-PROBE.** `steering-witness-revoke-retry` is an expected violation: a
listener stays bound and resetting while its element removal is retried, so a
marked probe to that port is reset. `steering-resetting-resolves` holds: the
resetting state ends. Both carry the D8a-PROBE label in `checks.toml`.

## Teeth — every new rule is load-bearing

| Hazard instance | Rule switched off | Property violated |
|---|---|---|
| `lease_transient_bugRefuse` | D16-PREF (the r2 refusal rule) | `NoStarvedOnUsableOffset`, `NoRefusalWhileFree`, `EventuallyPlaced` |
| `lease_bugNoExclusion` | D16-EXCL preference | `NoExcludedWhileUnexcludedFree` (and the r2 teeth) |
| `steer_envFlushBugRewantKeeps` | D8a-READMIT re-admission | `AdmittedHasElement` |
| `steer_envFlushBugRepairOutsideQuiesce` | D8a-READMIT repair in quiescence | `AdmittedHasElement` |
| `steer_envFlushNoAuditFairness` | recovery fairness (D8a-FLUSH bound) | `ExposureEnds` |
| `flows_envGuestMissesHostClose` | K-A4 (environment) | `GuestHalfEnds` |

The round-1 and round-2 teeth are unchanged and still match
(`evidence/r3/summary.txt`).

## Remaining underspecified points

| # | Point | Decision | Why it matters |
|---|---|---|---|
| U-r3-1 | The guarantee text for D16-PREF: R5-19 and ADR-0156 still state the r2 / r1 guarantees (items 1, 2) | D16-PREF, ADR-0156, R5-19 | An oracle asserting the old wording fails against a correct implementation |
| U-r3-2 | Whether the fallback should avoid the most recently failed offset (item 2, option b) | D16-PREF | Repeated launches onto the same failing offset |
| U-r3-3 | Who may restore forwarding while a firewall recovery holds quiescence (item 3) | D8a-READMIT, G-V5 | Without it the D8a-FLUSH exposure has no upper bound |
| U-r2-5 | A-FID (host flow ids not reused within a pairing window); unchanged, not modelled | `FlowId` wire contract | Only if the id space is narrowed |
| U-r2-6 | D25-BIND; unchanged, not modelled (kernel fact) | D25 | R5-15 |

## Modelling assumptions and what discharges them

As in `specs/quint/guest-flow-owner/README.md` § *Assumptions*. Changes this
round: **K-A4** is stated as decided (the close reaches a queued `V_g` too)
and is discharged by **V-24** (feature-delta A-28). **K-D3** remains an
environment fault (`ENV_FOREIGN_FLUSH`); its window length is **V-23**'s.

Not modelled, so this run gives no evidence on them: the U-5 discard counter;
bytes on a connection (R5-31's "no byte reaches it"); D26 / V-21, V-20;
D5 / D5a framing; D24 / D24a resolution; half-close drain; the listen-state
lag; D15-R3 deadlines; the beacon beyond its claim discipline; placement
retry backoff.

## Limits

- Small instances: one CID or address; two control connections; two
  guest-opened flows and one host-opened flow; two UDP slots; three lease
  offsets with one to three workloads; two guest ports. Generations bounded
  at 2. Time is abstract.
- TLC holds are complete only for the instance; Apalache depths are bounds.
  Apalache cannot handle module C's next-fit fold beyond `lease_single`; TLC
  is the evidence there.
- Liveness results depend on the stated fairness. `ExposureEnds` in
  particular assumes the recovery's repair is fair (item 3).
- The model encodes the design text, not an implementation.
