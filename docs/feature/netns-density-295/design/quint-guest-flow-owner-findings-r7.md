# Quint model check, round 7 — guest-flow owner (D8a-LOAD-SWAP, `prefix_boot` only)

Subsystem `specs/quint/guest-flow-owner/`. Design change: D8a-LOAD-SWAP
(user-approved 2026-10-08). It removes the runtime probe and boot `verify`.
Boot step 9 is now load → swap → route. Previous round:
`quint-guest-flow-owner-findings-r6.md`. Tools: Quint 0.32.0, Apalache 0.56.1,
TLC (via Quint), JDK 21, Lima kernel 7.0.0-34-generic.

Recorded run `679777-1791451723.128` (`--record`, default scheduling): **wall
time 33.8 min**. **120 of 120 checks matched `expect`.** `cargo xtask quint
verify-evidence --subsystem guest-flow-owner` reports: "evidence matches the
specs; every outcome met `expect`".

## What changed

- **`prefix_boot.qnt`** went from 295 to 246 lines.
  - Boot step 9 is now `load` → `swap` (→ `pin` for a first attach) →
    `route`.
  - `load` can fail, which stands for a verifier rejection. Boot then refuses
    and nothing is attached.
  - `swap` is atomic on an adopted pinned link (K-L9). For a first link it is
    attach then pin. If it fails, the earlier program stays.
  - `serve` can crash between any two steps and inside swap, between attach
    and pin.
  - These are removed: the probe, `progProbed`, `probeOk`, `verify`/`verOk`,
    the defective-program state (`BAD`, M-DEF), and the `badprog`/`allbad`
    instances.
  - The instances are now `boot_ok` (one scripted failure, unbounded crashes),
    `boot_faulty` (unbounded failures and crashes) and `boot_live` (one of
    each).
- **Invariants.**
  - `LocalOnlyOverLink`: invariant 6 restated. The route is `local` only while
    a link with a loaded program exists. There is also a `-WhileDown` form.
  - `RouteOnlyAfterSwap` replaces `RouteOnlyAfterFreshProbe`.
  - New: `OpenConverged`. A boot opens only with its own program in the
    pinned link and the route `local`. Together with `BootOpens`, this is the
    claim that a crash at any step leaves a state the next boot converges.
  - Kept: `NoWildcardReached`, `NothingDeliveredWhileDown`, `NoElsewhere`
    (U-4) and `NeverDetached`.
  - `Exposure` is removed.
- **Hazards.**
  - Kept: `routeFirst`, `detach` (a non-atomic swap) and `unpinned`.
    `unpinned` now also has an `OpenConverged` check.
  - Added: `ignoreFailure`. A load or swap failure is ignored and boot goes on
    to the route.
  - Removed: `swapBeforeProbe`, `noVerify`, `ignoreVerify` and `staleVerify`,
    and their checks.
- **Witnesses.** These are reachable: a refusal, a load refusal, a program
  update, a crash with `local`, a crash after a swap under `local`, a crash
  between attach and pin, and a failed swap under `local` that keeps the
  earlier program.
- **`prefix_landing.qnt`**: M-DEF and the `defective` steer form are removed.
- **`README.md`**: the module is now described as load → swap → route. K-L8
  and M-DEF are removed. A new row, A-PROG, says the shipped program is
  correct (Tier-3 CI, V-26). The `prefix_boot` row of the properties table is
  rewritten.
- **`checks.toml`**: `udp-slots-hazard-late-paired-by-slot-data` is now
  `ci = false`. It took 133 s in round 6 and 151.5 s in this run.

## Verdicts per module

| Module | Holds | Expected violations | Unexpected | Sum of check seconds |
|---|---:|---:|---:|---:|
| `owner_flows` (A) | 4 | 17 | 0 | 2,013 |
| `udp_slots` (B) | 3 | 11 | 0 | 1,470 |
| `cid_lease` (C) | 4 | 13 | 0 | 142 |
| `prefix_boot` | 9 | 12 | 0 | 132 |
| `intake` | 5 | 13 | 0 | 167 |
| `quiescence` | 7 | 9 | 0 | 122 |
| `shared_table` | 5 | 8 | 0 | 86 |
| **Total** | **37** | **83** | **0** | |

The `prefix_boot` instances are finite and exhaustive under TLC, at about 5 s
per check. The Apalache safety check (`max_steps = 12`) took 9.4 s. Every
prediction written in `checks.toml` before the run was met.

## Teeth, in short

- **`ignoreFailure`** (7 states). Boot 1 (v1) fails `load`, and the failure is
  ignored. `swap` has no loaded program, so no link is created. Boot then
  writes the route `local`. The prefix is now `local` with no steering, and a
  host service is reached.
- **`routeFirst`.** The route is written `local` before the first load, so it
  is `local` with no link.
- **`detach`.** The pinned link is detached under `local`, and `serve` crashes
  before the new attach. The node is left `local` with no link.
- **`unpinned`.** After a crash the process-scoped link vanishes, and the
  route stays `local`. Boot also opens with the link unpinned, which violates
  `OpenConverged`.

## Findings

1. **`ci = true` checks over 2 minutes in this run.** These are in modules
   outside this round. I did not change them, because changing them would
   invalidate the recorded evidence.
   - `udp-slots-release-liveness` took 148.6 s on one attempt. In run
     `671148` it took 109.8 s.
   - `udp-slots-hazard-ignore-late-paired` took 135.8 s. That figure includes
     a retry after the Apalache 120 s start-up bound; the check itself takes
     about 17 s.

   Recommendation: set `udp-slots-release-liveness` to `ci = false` next
   round. The other overrun is the runner's start-up retry being counted in
   `seconds`, as in round 6.
2. **Two full runs, not one.** This was my invocation error, not a tooling
   defect. The first `--record` run (`671148-…`, 32.1 min) completed and
   recorded. While it was still running, I wrongly concluded it had died,
   because my wait loop used `kill -0` on a root-owned PID and so returned at
   once. I then started a second run (`679777-…`). Its first minute overlapped
   with the end of the first run, which inflates a few early durations. The
   second run replaced `evidence/`. Both runs had identical inputs and every
   check passed in both. Nothing was killed.
3. **Stale comments in other modules.** `shared_table.qnt` (lines 58 and 106)
   and `intake.qnt` (line 28) still say "verified, probed" about the
   steering. These modules were out of scope this round. Their semantics are
   unaffected, because they take step 9 as one step that succeeds. Reword the
   comments next round. The same applies to the revision-9 fence form in
   `quiescence`'s `otherRepairsPrefix` hazard (round 6).

## Underspecified points

- **D8a-LOAD-SWAP is not yet in the delta.** The modelled step order comes
  from the dispatch. The delta still carries D8a-PROBE-FIRST, `verify` and
  R5-24's probe/verify cases. When the architect's text lands, compare the
  pin sub-step and the refusal outcome on a pin failure against the module.
- **Unchanged from round 6:** K-D3 partial re-convergence, and a `serve`
  crash during `Vmm::create`.

## Assumptions without a discharging validation item

- **A-PROG** (the shipped program is correct) is discharged by the Tier-3
  steering tests in CI (V-26), per the design change. Confirm that V-26 names
  this once the delta is updated.
- **K-L9** maps to V-27, and V-27's K-L8 half is now unused by the model.
- **K-B2** and **A-FID**: unchanged.
- **A-31**: discharged by the appliance image.
- **M-DEF**: gone.
