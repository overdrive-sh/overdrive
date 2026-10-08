# Quint model check, round 8 — guest-flow owner (revision 11)

Subsystem `specs/quint/guest-flow-owner/`. Design input: revision 11 of
`feature-delta.md` (D8a-CAP / ADR-0176, D19-LINKS, D5a-IFACE, D8-PREFIX-CFG,
invariant 12, § *Formal protocol model* "Round 8") and review
`review-design-vsock-replacement-r10.md` (M-2, M-4, L-3). Previous round:
`quint-guest-flow-owner-findings-r7.md`. Tools: Quint 0.32.0, Apalache 0.56.1,
TLC (via Quint), JDK 21.

Recorded run `704554-1791465879.649` (`--record`, default scheduling): **wall
time 40.4 min** (2,424 s). **132 of 132 checks matched `expect`.**
`cargo xtask quint verify-evidence --subsystem guest-flow-owner`: "evidence
matches the specs; every outcome met `expect`".

## What changed

- **`intake` (D8a-CAP).**
  - Admission reserves the allocation's declared ports against `CAP` (the
    steering map's capacity). It refuses `IntakeCapacityReached` with no state
    change. A Retiring lease keeps its reservation until release.
  - One other allocation (two declared ports) competes for the same capacity
    (`CAP = 2`). It is admitted, steers, closes and is released by the
    environment.
  - `steer` has four outcomes:
    - `Ok`.
    - `KernelMemory`: the listener is closed and the bring-up retried.
    - A defect variant: reported, the listener closed, not retried. The port
      is "parked" until it stops serving (its next `ListenState` change,
      session loss, quiescence), then is wanted anew.
    - The kernel's capacity refusal. This is unreachable with admission.
  - **A-MEM** is the fairness on a successful `Steer(1)` in
    `ServingReachable`. Each failed attempt closes the listener, so the steer
    is enabled only intermittently. The fairness is therefore strong (SF) on
    that action; weak fairness on the retry would not carry. This is a
    modelling note, not a design finding.
  - `ServingReachableNoMemRecovery` is the assumption-off form.
  - Invariant 12 is added: `ReservedWithinCapacity`, `ListenersWithinReserved`,
    `EntriesWithinListeners`, `NoCapacitySteerFailure`. Also added:
    `ParkedOnlyWhileServing`. All are in `Safety` and checked separately.
  - New hazards: `noIntakeAdmission`, and `noIntakeAdmission` + `capRetry`
    (the revision-10 capacity-driven retry).
  - New witnesses: a capacity refusal, a refusal while our lease is Retiring,
    the map exactly full, a defect.
  - "verified, probed" is removed from the comments.
- **`prefix_boot`.**
  - M-4: `LocalOnlyOverLink` and `LocalOnlyOverLinkWhileDown` now check
    `link == "pinned"`.
  - M-2: the refusal is named `GuestPrefixSteeringConverge`, and A-PROG is
    renamed A-33 in the comment.
- **`quiescence` (M-2).**
  - The route has the single form `local`. The `route` variable is replaced
    by `prefix ∈ {booted, written}`. `otherRepairsPrefix` is now any runtime
    write of the route or steering.
  - `FailStopLeavesPrefixLocal` is renamed `FailStopLeavesPrefixAsBooted`.
  - The check `quiescence-hazard-fail-stop-fences` is renamed
    `quiescence-hazard-fail-stop-after-runtime-prefix-write`.
  - No instance named `SockOpsLink` / `DrainCounterLink` / `UnframeLink`, so
    nothing was dropped.
- **`shared_table`.** Comments only.
- **README.**
  - K-L9 → V-26 (viii); K-L3 → V-26 (iii), (x).
  - A-PROG → A-33 (R5-1, R5-2, R5-3, R5-25 in CI); A-MEM added.
  - D8a-CAP is in the `intake` rows.
  - "Not modelled" gains D5a-IFACE, D19-LINKS and D8-PREFIX-CFG, and loses
    `ForeignGuestPrefixRoute`.
- **`checks.toml`.**
  - These are now `ci = false`: `udp-slots-release-liveness` (L-3) and
    `udp-slots-hazard-ignore-late-paired` (both over 2 min in r7), and
    `intake-safety-apalache` (166.8 s while iterating, 198.5 s in the
    recorded run).
  - 12 checks are new.
- Unchanged: `owner_flows`, `udp_slots` (except its ci flags), `cid_lease`,
  `prefix_landing`.

## Verdicts per module

| Module | Holds | Expected violations | Unexpected | Sum of check seconds |
|---|---:|---:|---:|---:|
| `owner_flows` (A) | 4 | 17 | 0 | 1,748 |
| `udp_slots` (B) | 3 | 11 | 0 | 1,453 |
| `cid_lease` (C) | 4 | 13 | 0 | 135 |
| `prefix_boot` | 9 | 12 | 0 | 136 |
| `intake` | 10 | 20 | 0 | 376 |
| `quiescence` | 7 | 9 | 0 | 146 |
| `shared_table` | 5 | 8 | 0 | 80 |
| **Total** | **42** | **90** | **0** | |

The TLC instances of `intake`, `prefix_boot` and `quiescence` are finite and
exhaustive. `intake-safety-apalache` has `max_steps = 12`. Every prediction
written in `checks.toml` before the run was met.

### Invariant 12 and `ServingReachable`

- All four invariant-12 checks **hold** on `intake_two`, and also inside
  `Safety` on `intake_ok`, `intake_two` and Apalache.
- **`ServingReachable` holds.** It holds on `intake_live` (no defects) and on
  `intake_ok` (with defects; a parked port is excluded until it stops
  serving). In round 7 it was violated on the design instance.
- **`intake-steer-forever-unreachable`** now runs
  `ServingReachableNoMemRecovery` (A-MEM off) and is violated as expected:
  `KernelMemory` refuses every retry.
- **Teeth.**
  - `noIntakeAdmission` violates `ReservedWithinCapacity` (ours 1 + other 2 >
    2) and `NoCapacitySteerFailure`. The other allocation fills the map, then
    our steer is refused for capacity.
  - `noIntakeAdmission` + `capRetry` violates `ServingReachable`. The other
    allocation holds both slots forever, and our listener is bound → refused
    for capacity → re-bound, without end.

## Findings

1. **A CI check over 2 minutes in this run.**
   `udp-slots-witness-late-paired` (`ci = true`) took 138.1 s. That figure
   includes the runner's Apalache start-up retry: attempt 1 hit the 120 s
   start-up bound, and attempt 2 took 18 s. This is the same pattern as r7's
   `udp-slots-hazard-ignore-late-paired`. I did not change it after the
   recorded run, because that would invalidate the evidence. The check
   itself is far under 2 minutes.
2. **The first full run died at start-up (environment, not tooling).** Run
   `704033-…` failed reading its own `attempt-1.log`. The workspace's
   `target/` directory had been deleted from outside this session while the
   run started. `evidence/` was untouched and no Quint or JVM process
   remained. I started the recorded run once more after that; it is the only
   run that recorded.
3. **No design finding.** No unexpected result.

## Underspecified points

- None new. A-MEM's "weak fairness on the retry" is stated in the delta at
  the level of "a retry eventually succeeds". The model's strong fairness on
  a successful steer is that statement for an action re-enabled once per
  retry.

## Assumptions without a discharging validation item

- None new. A-MEM is an environment assumption (node memory, #261). Its
  exposure is observable (`guest_intake.bind_failed { retried: true }`, V-9).
- Unchanged from round 7: K-B2 (R5-14) and A-FID (a design property).
  A-31 is discharged by the appliance image.
