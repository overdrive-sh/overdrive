# Quint model check, round 6 — guest-flow owner (revision 10, D8a-PROBE-FIRST)

Subsystem `specs/quint/guest-flow-owner/`. Design: `feature-delta.md` § *vsock
Attachment Replacement DESIGN*, revision 10 — D8a-PROBE-FIRST (2026-10-08),
D8a-ROUTE as restated for it, D8a-FENCE removed, § *Formal protocol model*
("Next round"). Previous round: `quint-guest-flow-owner-findings-r5.md`
(finding r5-1). Tools: Quint 0.32.0, Apalache 0.56.1, TLC (via Quint), JDK 21,
Lima kernel 7.0.0-34-generic.

Recorded run `657887-1791447428.618` (`--record`, default scheduling, 8 jobs):
**wall time 1,832 s (30.5 min)**, **124 of 126 checks matched `expect`**,
2 unexpected (both the predicted "verify is not load-bearing" result below).
`cargo xtask quint verify-evidence --subsystem guest-flow-owner` reports the
evidence current against the specs and fails only on those 2 recorded `expect`
mismatches.

## What changed

Only `prefix_boot` changed in content (abstraction: "probed before attach"; the
probe mechanism is not modelled):

- Boot step 9 is `load` → `probe` → `attach` (→ `pin` for a first attach) →
  `verify` → route. `load` and `probe` change no modelled kernel object.
  `attach` is the only step that changes the link's program: atomically for an
  adopted pinned link (K-L9), or attach then pin for a first link. It runs only
  for a candidate whose probe passed. A failed `attach` keeps the earlier
  program.
- The attached program carries `progProbed` (probed before its attach). M-DEF
  (a defective program decides nothing) applies only to a program attached
  unprobed.
- The route has two forms, `absent` and `local`. The fence form, the fence
  write and its failure action are deleted. A failure of `load`, `probe`,
  `attach`, `pin`, `verify` or the route write refuses startup with the route
  unwritten. A crash can happen between any two steps and between attach and
  pin.
- Invariant 6, restated (`LocalOnlyOverCertified`): the link, if it exists,
  runs a program probed before its attach, and the route is `local` only while
  such a link exists. `RefusalFences`, `FenceLifted`, the fence witnesses and
  `SafetyOutsideUpdateWindow` are deleted. `NoElsewhere` remains under U-4
  (the route is never removed).
- Hazards: `removeNotFence` is deleted. `swapBeforeProbe` is added (revision
  9's order: `attach` accepts an unprobed candidate). `routeFirst`, `detach`
  and `unpinned` are kept. `noVerify` and `staleVerify` are re-run. I added
  `ignoreVerify` (a verify failure only is ignored) to separate verify from the
  probe refusal, which `noVerify` switches off together.
- New witnesses: a crash right after a swap under `local`, and a failed attach
  under `local` that keeps the earlier program. Both show that the r5-1 window
  is still reachable in the design and is now harmless.
- `prefix_landing`: the fence case and K-L6 are removed.
- `shared_table`: the trivial `LegCBypassEnds` property and its two checks are
  removed. Concern 4 progress is now `TableRepaired`: the IpRules recovery's
  repair completes under its hold, or `serve` stops. It is checked on
  `table_live` (`shared-table-repaired`, unchanged) and on `table_ok`
  (`shared-table-ok-repaired`, which replaces `shared-table-ok-leg-c-bypass-ends`).
- `README.md`: revision 10, K-L8 and K-L9 (V-27) replace K-L6, the `prefix_boot`
  rows are updated, and the log path is corrected to
  `target/quint/guest-flow-owner/<run-id>/<check>/`.
- `intake`, `quiescence`, `owner_flows`, `udp_slots` and `cid_lease` are
  unchanged.

## Verdicts per module

| Module | Holds (expected) | Violations (expected: witnesses, hazards, documented) | Unexpected | Sum of check seconds |
|---|---:|---:|---:|---:|
| `owner_flows` (A) | 4 | 17 | 0 | 1,757 |
| `udp_slots` (B) | 3 | 11 | 0 | 1,225 |
| `cid_lease` (C) | 4 | 13 | 0 | 137 |
| `prefix_boot` (concern 1) | 11 | 14 | **2** | 273 |
| `intake` (concern 2) | 5 | 13 | 0 | 144 |
| `quiescence` (concern 3) | 7 | 9 | 0 | 117 |
| `shared_table` (concern 4) | 5 | 8 | 0 | 78 |
| **Total** | **39** | **85** | **2** | |

`prefix_boot` instances are finite and exhaustive under TLC. Each design check
took about 5 s. The Apalache safety check (`max_steps = 12`) took 10.7 s.

## r5-1 is gone, and `swapBeforeProbe` reproduces it

The four checks that failed in round 5 now hold on the design:

- `prefix-boot-local-only-over-certified` and `-while-down`;
- `prefix-boot-badprog-no-wildcard`;
- `prefix-boot-badprog-nothing-while-down`.

So do `prefix-boot-safety` (TLC and Apalache), `prefix-boot-badprog-safety`,
`prefix-boot-allbad-safety` and `prefix-boot-opens`. A rejected candidate is
never attached, so the link only ever runs a probed program. A failed or
interrupted `attach` keeps the earlier probed program under `local`, and the
witnesses show that this state is reached.

With `swapBeforeProbe` (the only change is that `attach` accepts an unprobed
candidate, in revision 9's order), all four checks are violated. The
counterexample is `evidence/traces/prefix-boot-hazard-swap-before-probe-badprog-nothing-while-down.txt`
(15 states), the same shape as r5-1:

1. Boot 1 runs v1: load, attach, pin, verify, probe, route `local`, open.
2. `serve` crashes. The route and the pinned link persist.
3. Boot 2 runs v2, which the probe would reject. `attach` swaps v2 into the
   pinned link under `local` before any probe.
4. The pin sub-step fails, and startup refuses.
5. `serve` is down with `local` over unprobed v2, so a host service is reached.

No fence-write failure is needed now, because the fence no longer exists.

## Unexpected result — finding r6-1: `verify` is not load-bearing in the model (D8a-ROUTE)

The rule is D8a-ROUTE: "the route is written only after this boot's program
was … swapped into the pinned link and verified". The prediction, written in
`checks.toml` before the run, was that it would hold. Two hazards that remove
`verify` find no observable violation. Both are judged on `Exposure`
(invariant 6, no-wildcard, nothing-delivered-while-down, no-elsewhere) rather
than on the `RouteOnlyAfterFreshProbe` ghost, which they break by definition.

- `prefix-boot-hazard-ignore-verify` (`badprog`: a verify failure, including a
  scripted query failure, is ignored and boot writes the route) holds.
- `prefix-boot-hazard-stale-verify` (`badprog`: a boot that adopted a pinned
  link skips verify) holds.

Why: in the model, after a successful `attach` (K-L9: atomic, earlier program
kept on error) and a successful `pin`, the link is pinned and runs this boot's
probed program. Every way to reach `verify` with a different link state is
already a refusal at `attach` or `pin`. Verify is a read-back of a fact the
kernel has just guaranteed. This is what § *Formal protocol model* predicted
("under K-L9 `verify` is a read-back of the kernel's own swap, discharged by
V-27").

`prefix-boot-hazard-no-verify` (`allbad`: verify AND probe failures ignored)
still shows a violation, but only through the ignored probe failure:

1. The probe fails, so there is no probed candidate.
2. `attach` keeps the link (there is none).
3. Verify fails, and that failure is ignored.
4. The route becomes `local` with no link, so the wildcard is reached.

Verify and the probe refusal each catch this alone. Neither is load-bearing
without the other in the model.

**For the architect** (no design change was made; this is routed back):
either

- state `verify` as defence in depth against a K-L9 / V-27 falsification
  (the model takes K-L9 as true, so it cannot show verify's value), and keep it
  without claiming the model defends it; or
- drop it from D8a-ROUTE's ordering claim.

If V-27 falsifies K-L9 (a failed update that leaves a different program or
no program), verify becomes the only check, and the model would need an
assumption-off variant to show it. That variant was not run this round.

## Other results

- **Tooling (xtask quint), not a design result.** In two checks the Apalache
  server did not reach its first pass within the 120 s start-up bound:
  `udp-slots-witness-reassociation` and `prefix-boot-badprog-no-wildcard`.
  The runner killed the process group and retried, and both passed on attempt 2.
  The recorded `seconds` includes the failed attempt, so these `ci = true`
  checks show 125–133 s. The check itself takes about 5 s.
  `udp-slots-hazard-late-paired-by-slot-data` (`ci = true`, unchanged module)
  ran 133 s with no retry, which is just over the 2-minute CI bound. I did not
  change its flag, because the module is outside this round.
- **Full-run wall time beyond the Bash tool cap.** The recorded run took
  30.5 min (round 5: 21.3 min). The foreground command was cut at the tool's
  30-min cap at ~1,813 s, but the run inside the VM continued and completed
  normally. I waited for its PID to exit and killed nothing. `summary.json`
  shows `interrupted_by_signal: null` and every check finished. A future full
  `--record` will need a shorter run or a longer tool cap.
- **`quiescence` keeps a revision-9 route form.** Its `otherRepairsPrefix`
  hazard writes `route = "fence"` to stand for "a repair writes the route".
  `quiescence` does not use `prefix_landing`, and its check is ghost-based
  (`RepairNeverWritesPrefix`), so the result is unaffected. I left it as
  instructed ("unchanged"). The README notes it.

## Underspecified points

1. **D8a-ROUTE — what `verify` defends** under K-L9. This is finding r6-1.
2. **D8a-PROBE-FIRST — a probe failure versus a probe I/O error.** The model
   refuses on both, which matches the delta's "boot refuses with a typed
   error". The delta does not say whether a test-run I/O error (as opposed to a
   deviation) is retried before refusing. Retrying would not change safety.
3. **M-DEF** remains a modelling choice. Under D8a-PROBE-FIRST it is reached
   only by `swapBeforeProbe`.

r5's other underspecified points are unchanged: K-D3 partial re-convergence,
and the `serve` crash during `Vmm::create`.

## Assumptions without a discharging validation item

- **M-DEF**: a modelling choice (above).
- **K-B2**, **A-FID**: unchanged, as the design records.
- **A-31**: discharged by the appliance image (ADR-0068).

K-L8 and K-L9, the new `prefix_boot` assumptions, map to V-27. K-L6 is gone.
Every other kernel assumption maps to its validation item in `README.md`.
