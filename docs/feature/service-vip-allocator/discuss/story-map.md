# Story Map — service-vip-allocator

## Scope Assessment: PASS

- **Story count**: 1 story (US-01).
- **Bounded contexts**: 1 (the dataplane allocator surface in
  `crates/overdrive-dataplane/`; admission boundary in control-plane
  is a downstream caller).
- **Estimated effort**: 1–3 days, depending on DESIGN's resolution of
  Open Questions 1, 2, 4 (reclamation trigger, when admission
  allocates, shared-primitive shape). Comfortably within carpaccio
  bounds.

Elephant Carpaccio check (per `nw-leanux-methodology` § Principle 8):

- **>10 stories?** No (1).
- **>3 bounded contexts?** No (1).
- **Walking skeleton >5 integration points?** N/A (brownfield, no
  walking skeleton).
- **Effort >2 weeks?** No.
- **Multiple independent user outcomes?** No (single outcome:
  transparent platform-issued VIP).

No oversizing signals. Single story is correctly right-sized.

## Backbone (workflow activities)

The operator's end-to-end flow against this feature:

```text
[Author Service spec] → [Submit spec] → [Run / converge] → [Stop / terminal-state]
       |                     |                  |                    |
   no `vip`            platform              operator             allocator
   field needed         allocates,           observes             releases VIP
                         echoes VIP,         VIP via              for reuse
                         admits spec         alloc status
```

Single user activity column; the four stages are micro-steps within
one operator outcome.

## Stories per stage

| Stage | Story | Notes |
|---|---|---|
| Author | (no story — operator behavior, not platform behavior) | The absence of an operator-pinned `vip` field is enforced by AC-06's rejection. |
| Submit | **US-01** (covers AC-01, AC-02, AC-04, AC-05, AC-06) | Allocation happens here OR at first reconciler tick — DESIGN decides per Open Question 2. |
| Run | **US-01** (covers AC-01 echo / alloc status render) | Render surface inherits from upstream slice-06. |
| Terminal-state | **US-01** (covers AC-03) | Reclamation trigger is DESIGN's call per Open Question 1. |

A single story covers all four stages because the platform's behavior
is one continuous lifecycle around one shared artifact (the VIP).
Splitting would manufacture optionality.

## Priority Rationale

- **US-01 is the only story**. No prioritization needed.
- **Carpaccio slicing is deferred to DESIGN/DELIVER**. DESIGN
  produces the roadmap with slices. Candidate slice fault lines
  (architect's call, not DISCUSS's):
  - Slice A: shared-primitive refactor of `BackendIdAllocator` —
    pure refactor, no new behavior, preserves test surface.
  - Slice B: `ServiceVipAllocator` over the shared primitive (allocate
    only, no release).
  - Slice C: reclamation path (release on terminal-state).
  - Slice D: admission rejection for operator-supplied `vip = Some(...)`.
  - Slice E: pool config surface + exhaustion handling.

  This is the architect's territory; DISCUSS does not lock the
  slicing.

## Walking skeleton

N/A. This is a brownfield refactor of an existing isolated primitive
(`BackendIdAllocator`). The "walking skeleton" of Phase 1 already
ships at the workspace level; this feature is one primitive deepening
within it.

## Deferred to DESIGN/DELIVER

- All five open questions from `wave-decisions.md`.
- Roadmap with concrete slices and per-slice acceptance criteria.
- Persistence shape, factor of the shared primitive, admission-layer
  decisions.

## Cross-references

- `user-stories.md` — the single story with embedded ACs and UAT
  scenarios.
- `wave-decisions.md` — open questions, changed assumptions, scope.
- `outcome-kpis.md` — measurable targets.
- SSOT: [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
