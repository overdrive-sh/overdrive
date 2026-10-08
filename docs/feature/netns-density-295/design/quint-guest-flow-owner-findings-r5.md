# Quint model check, round 5 — guest-flow owner (revision 9 + APPLIANCE)

Subsystem `specs/quint/guest-flow-owner/`. Design: `feature-delta.md` § *vsock
Attachment Replacement DESIGN*, revision 9 with item 7 APPLIANCE (A-31) of
2026-10-07, § *Formal protocol model* (the four steering concerns). Tools:
Quint 0.32.0, Apalache 0.56.1, TLC (via Quint), JDK 21, Lima kernel
7.0.0-34-generic.

Recorded run `614030-1791411869.153` (`--record`, default scheduling, 8 jobs):
**wall time 1,278.6 s (21.3 min)**, **120 of 124 checks matched `expect`**,
4 unexpected (all finding r5-1). `cargo xtask quint verify-evidence --subsystem
guest-flow-owner` reports the evidence current, and fails only on those 4
recorded `expect` mismatches.

## What changed

- `steering.qnt` is split into one module per concern. Each module has its own
  small instances, and the shared landing predicate lives in one stateless
  module (`prefix_landing.qnt`).
- Foreign environment faults are removed: other software deleting the nft table
  or steering entries, detaching the link, writing or fencing the route, or
  taking a CID. The runtime fence, `repair_guest_prefix`, the serialized
  guest-prefix order and UP-5/6/11/13/15 are gone, together with their checks.
- `cid_lease`'s only holders outside the pool are VMMs that survived a `serve`
  crash: one from an earlier run at start, plus survivors of every crash.
- Every module now takes `OFF: Set[str]` (rules switched off) and, where it
  needs one, `CFG: str` (a named instance). Every hazard file is one-line
  `import`s with an override. Modules A and B got only this parameter change;
  their content is unchanged.

| File | Before | After |
|---|---:|---:|
| `steering.qnt` | 1,142 | — |
| `prefix_landing.qnt` (shared) | — | 65 |
| `prefix_boot.qnt` (concern 1) | — | 288 |
| `intake.qnt` (concern 2) | — | 272 |
| `quiescence.qnt` (concern 3) | — | 228 |
| `shared_table.qnt` (concern 4) | — | 247 |
| `hazard/steering_hazards.qnt` | 825 | — |
| `hazard/{prefix_boot,intake,quiescence,shared_table}_hazards.qnt` | — | 16 + 16 + 13 + 12 = 57 |
| **steering total (specs + hazards)** | **1,967** | **1,157** |
| `cid_lease.qnt` / `owner_flows.qnt` / `udp_slots.qnt` | 360 / 530 / 393 | 366 / 532 / 390 |
| `hazard/{cid_lease,owner_flows,udp_slots}_hazards.qnt` | 35 / 53 / 41 | 14 / 24 / 20 |
| **all `.qnt`** | **3,379** | **2,503** |
| `checks.toml` (checks) | 1,547 (156) | 1,235 (124) |

## Verdicts per module

| Module | Holds (expected) | Violations (expected: witnesses, hazards, documented) | Unexpected | Sum of check seconds |
|---|---:|---:|---:|---:|
| `owner_flows` (A) | 4 | 17 | 0 | 1,376 |
| `udp_slots` (B) | 3 | 11 | 0 | 705 |
| `cid_lease` (C) | 4 | 13 | 0 | 101 |
| `prefix_boot` (concern 1) | 8 | 12 | **4** | 111 |
| `intake` (concern 2) | 5 | 13 | 0 | 104 |
| `quiescence` (concern 3) | 7 | 9 | 0 | 90 |
| `shared_table` (concern 4) | 6 | 8 | 0 | 67 |
| **Total** | **37** | **83** | **4** | |

Every design rule has a hazard that shows its violation:

- concern 1: `routeFirst`, `noVerify`, `staleVerify`, `removeNotFence`,
  `detach`, `unpinned`;
- concern 2: K-L3 `entryOutlives`, `takedownBeforeClose`, `rollbackPartial`,
  `actOkPartial`, `teardownKeeps`, `quiesceKeeps`;
- concern 3: `anyRestore` (safety and pre-emption), `repairNoHold`,
  `redoNoCount` (UP-7), `otherRepairsPrefix` (UP-12);
- concern 4: `steeringInNft`, `unpinned`, `nonLoopback`, `anyRestore`;
- D16-CLAIM: all three hazards, with surviving VMMs as the only outside
  holders (see below).

Every check except the two 400–570 s Apalache runs of modules A and B finishes
under 2 minutes and carries `ci = true`.

## Unexpected result — finding r5-1 (D8a-FENCE, D8a-ROUTE; invariant 6)

Invariant 6 states: after a crash at any boot step and after any failed
converge / verify / probe, the route is `local` only behind a steering link that
a boot verified and probed after its last change. **The design's own boot order
breaks this.** `GuestPrefixSteering::converge` adopts the pinned link and swaps
in the new program with `BPF_LINK_UPDATE`. That swap happens under the `local`
route left by the earlier boot, and before `verify` and `probe` run.

Counterexample (`evidence/traces/prefix-boot-badprog-nothing-while-down.txt`,
15 states):

1. Boot 1 runs binary v1: converge, verify, probe, route `local`, open.
2. `serve` crashes. The route and the pinned link persist (U-4).
3. Boot 2 runs binary v2, whose steering program the probe would reject.
   Converge adopts the link and swaps v2 in under the `local` route.
4. A converge sub-step fails. Boot tries to fence, the fence write fails
   (allowed by the design), and startup refuses.
5. `serve` is down with `local` over the unverified v2 program.

A crash right after the swap reaches the same state: the next transition from
state 14 is `Crash`. With the nft rules partial (concern 4: the mTLS worker
failing part-way, or a crash between batches), an unmarked host-local
connection reaches a host service:

- `prefix-boot-badprog-no-wildcard`: violated during the boot window;
- `prefix-boot-badprog-nothing-while-down`: violated while `serve` is down.

With only sound programs (`boot_ok`), no host service is reached. The
structural invariant (`prefix-boot-local-only-over-certified`, `-while-down`) is
still violated after any binary change, because a program no boot has probed
decides lookups until boot 2 reaches `probe`, and for as long as `serve` stays
down when that boot crashes or refuses with a failed fence.

The design text says this cannot happen: the counterexample under G-V1 says
"keeping `local` over a steering a failed converge changed … leaves the prefix
decided by nothing verified", and the error-taxonomy row says "a `local` route
stands only over a steering link an earlier boot attached". Both are true of
the link but not of its program.

Remedies are for the architect; the model only shows the gap. Two directions
would close it:

- probe the newly loaded program object in a private namespace **before**
  `BPF_LINK_UPDATE` and swap only after verify and probe pass; or
- fence before any converge step that changes the program under a `local`
  route.

Either way the invariant then holds as written. Otherwise the design must state
this window as a bounded exposure, with the fence-write failure included. The
model assumes a probe-rejected program decides nothing when attached (M-DEF,
worst case).

Prediction recorded before the run: violated (`checks.toml` comments on the
four checks; the module run on 2026-10-07 23:4x matched).

## Other results the architect should know

- **Concern 4 progress wording.** "The leg-C bypass ends" holds trivially: the
  IpRules recovery's first hold closes every intake listener, so the bypass
  ends at the quiesce. It also holds under the `anyRestore` hazard. What
  D8a-HOLD defends here is that the **repair** is not pre-empted forever.
  `TableRepaired` (the table is complete again, or `serve` stops, under the
  IpRules recovery's own fairness only) holds, and `anyRestore` violates it.
  The § *Formal protocol model* progress clause for concern 4 should name the
  repair, not the bypass.
- **D16-CLAIM hazards on the appliance.** All three still show their violation
  with surviving VMMs as the only outside holders. Each counterexample goes
  through a different case:
  - `checkThenClaim` and `releaseBeforeVmm` show it through a VMM that was
    launching when `serve` crashed and creates its device after the restart.
    The counterexample is `OrphanExit`, assign, launch, `Boot`, assign the same
    offset, launch, `SurvivorCreate`, `CreateOk` clash. `ClaimHeld` is also
    violated directly.
  - `rememberInUse` shows it through `InUse` answers from survivors.
  - No hazard is a rule that is not load-bearing.
- **Tooling defect (xtask quint, record).** The first `--record` run did not
  write evidence. `intake-witness-leg-c` was violated in the initial state. TLC
  prints `Error: Invariant … is violated by the initial state:` with no
  `State 1:` header, so `trace::extract_tlc_trace` returns `None`, and
  `evidence::record` fails with "no TLC counterexample" after a full 21-minute
  run. That message was not visible through the runner's table output. The log
  (`target/quint/guest-flow-owner/intake-witness-leg-c/attempt-1.log`) was
  overwritten by the next run. To reproduce: any TLC invariant check whose
  `init` violates the invariant. The witness itself was weak (it only showed
  that leg-C exists at boot). I replaced it with `WitnessNoLegCWhileServed`
  (unmarked traffic to a served, steered port lands on leg-C) and recorded
  again. No evidence of the first run exists, because the recorder wrote none.

## Underspecified points

1. **D8a-FENCE / D8a-ROUTE — ordering of the program swap against the route.**
   This is finding r5-1. The design does not say whether `probe` runs before or
   after `BPF_LINK_UPDATE`, or whether boot fences before a converge that
   changes the program.
2. **K-D3 / boot step 8 — can re-convergence leave a complete table partial?**
   The design does not say whether `start_shared_owner` ever removes constant
   rules while converging a complete table, which is what a crash between
   batches would need. The model allows any partial state when `serve` crashes
   during step 8 (generous).
3. **D16-CLAIM / K-C2 — a `serve` crash during `Vmm::create`.** The design does
   not state whether a VMM process that has inherited the claimed device
   outlives `serve`. The model allows both: an in-transit claim survives as a
   VMM-held CID or is released.
4. **M-DEF — what a probe-rejected steering program does when attached.** This
   is not a kernel fact. The model takes the worst case (it decides nothing).

## Assumptions without a discharging validation item

- **M-DEF** (above): a modelling choice. It only matters through finding r5-1.
- **K-B2** (R5-14, no kernel unknown) and **A-FID** (the `FlowId` contract):
  unchanged, as the design records.
- **A-31**: discharged by the appliance image (ADR-0068), not by a runtime item.

Every other kernel assumption maps to a validation item in `README.md`: V-10,
V-13, V-19, V-22, V-24, V-25, V-26, V-5(c), P-16 and P-23.
