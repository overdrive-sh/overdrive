# Prioritization — workload-kind-discriminator

## Changed Assumptions

- 2026-05-10 — folded in GH #164 (service listener spec shape) as Slice 06
  with US-08. Re-ranked: Slice 06 lands at order 5 (after Slice 04, before
  Slice 05). New KPI K6 (listener round-trip byte-equality). The runtime
  allocator for `vip = None` is tracked at #167 and is not part of this
  feature.

## Release Priority

| Priority | Slice | Target Outcome | KPI | Rationale |
|---|---|---|---|---|
| 1 | Slice 01 — Parser kind discriminator | Operators can express the kind they intend in TOML | K2 — spec-validation latency p95 < 50ms; mixed-kind specs rejected with named guidance 100% | Enabler — every other slice depends on this abstraction landing first. |
| 2 | Slice 02 — Job submit terminal | Operators trust the streaming submit's verdict for Jobs | K1 — honesty rate (Job submits whose CLI verdict matches kernel exit code) moves from 0% to ≥99% on `examples/coinflip.toml` over 100 trials | The bug under audit IS this KPI. The slice IS the fix. |
| 3 | Slice 03 — alloc status Job render | Operators can post-hoc see why a Job failed without reading control-plane logs | K3 — comprehension rate (operators correctly identify exit code from `alloc status` output for a Failed Job) — target ≥95% in usability check | The user's explicit framing journey: *"it should show that it failed during execution"*. |
| 4 | Slice 04 — Service preservation | Existing Service workflows are unaffected by the rename | K4 — regression rate; 0 broken existing Service tests; existing `streaming_submit_happy_path` continues to pass | Regression guard. **Prerequisite for Slice 06.** |
| 5 | Slice 06 — Service listener spec shape | Operators can declare `[[listener]]` triples and round-trip them through submit + alloc status | K6 — 100% byte-equality between submit echo and `alloc status` Listeners sections across 100 Service submits with pinned VIPs | **Folded in from #164.** Forward-compatible with #167's eventual runtime allocator decision. |
| 6 | Slice 05 — Schedule parsing | Operators can write a Schedule spec and receive honest deferral | K5 — submit echo URL == alloc status URL, byte-identical, in 100% of cases | Lowest priority; could split into its own feature without harming the rest. |

## Backlog Suggestions

| Story | Slice | Priority | Outcome Link | Dependencies |
|---|---|---|---|---|
| US-01 | 01 | P1 | Honesty KPI K1 (enabler) + K2 | None |
| US-02 | 02 | P1 | Honesty KPI K1 (mover) | US-01 |
| US-03 | 03 | P1 | Comprehension KPI K3 | US-01, US-02 |
| US-04 | 04 | P2 | Regression KPI K4 | US-01 |
| US-08 | 06 | P2 | Listener round-trip KPI K6 | US-01, US-04 |
| US-05 | 05 | P2 | Deferral consistency KPI K5 | US-01 |
| US-06 | 01 | P2 | (Tech task) Anti-pattern grep gate for `"live"` literal | US-01 |
| US-07 | 01 | P1 | (Migration) `examples/coinflip.toml` shape change | US-01 |

> US-06 is a technical-task story (per LeanUX template's "Technical Task" type) — it
> implements the grep / dst-lint gate that prevents the literal `"live"` from being re-
> introduced. It must link to a user story (US-02 — the bug fix) per task-type rules.
>
> US-07 is the migration story for the existing example.
>
> US-08 (folded in 2026-05-10 from #164) ships the Service listener spec shape;
> the runtime allocator for `vip = None` is tracked at
> [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167)
> and is not part of this feature.

## Notes on priority assignment

All stories carry **P1 or P2** by default per CLAUDE.md ("All priorities by default" —
the orchestrator told me to address everything; I kept everything in scope).

Effort_hours estimates per story are advisory per CLAUDE.md ("No effort/time budget
cuts"). They are recorded in each `slice-NN-*.md` for orientation but do not gate
landing the slice.
