# Upstream Changes — workload-kind-discriminator (DESIGN wave)

**Wave**: DESIGN
**Date**: 2026-05-10
**Author**: Morgan
**Audience**: Luna (PO) for review; orchestrator for relay.

This file records DESIGN-wave decisions that change assumptions
recorded in earlier-wave artifacts. Per the skill spec, the
architect must NOT edit DISCUSS / DISCOVER / DIVERGE artifacts
inline — the changes go here for PO review and the upstream artifact
is updated in a follow-up dispatch.

The three changes below are **non-blocking refinements**, all of
which the DISCUSS reviewer (`review-discuss.md`) explicitly flagged
for DESIGN to resolve.

---

## Change 1 — K3 measurement cadence pinned

**Source**: `outcome-kpis.md:54` Measurement Plan table, K3 row.

**Current text** (DISCUSS):
> Manual check on first release; automated assertion that the
> rendered Exit column matches the persisted exit_code | Once at
> feature release; ongoing automated | PO + DELIVER wave

**Proposed text** (DESIGN):
> Manual usability check: pre-release one-shot, 5-10 operators read
> a `Failed (backoff exhausted)` alloc-status fixture and state the
> exit code. ≥95% pass = release; <95% = iterate render layer and
> retest. Automated parsing-from-fixtures regression test runs on
> every CI. | Pre-release manual gate (one-shot at first release);
> automated test continuous | PO (manual) + DELIVER wave (automated)

**Rationale**: see `wave-decisions.md` § "Reviewer-flagged decisions
resolved" / [D9].

**Action requested**: PO updates the K3 row in
`outcome-kpis.md`'s Measurement Plan table. No other K3 references
in DISCUSS artifacts need updating (US-03's KPI block already
reads "usability check (small sample, 5–10 operators) before/after
this slice" — compatible with pre-release one-shot).

---

## Change 2 — `${listener_triple}` consumer #3 pinned shape

**Source**: `shared-artifacts-registry.md:160-162` (the
`${listener_triple}` artifact's Consumer #3 entry).

**Current text** (DISCUSS):
> 3. `AllocStatusRow` listener fields denormalised at write time
>    (architect to confirm shape).

**Proposed text** (DESIGN):
> 3. `AllocStatusRow` listener fields denormalised at write time
>    as `listeners: Vec<ListenerRow>` embedded on the row (NOT a
>    separate `service_listener` table). `ListenerRow` shape:
>    `{ port: NonZeroU16, protocol: Proto, vip: Option<ServiceVip> }`
>    — same field set as the spec-layer `Listener`; name
>    `ListenerRow` distinguishes the observation-side type from
>    the intent-side `Listener` per ADR-0011's intent-vs-
>    observation type-distinctness rule. See ADR-0047 §4a.

**Rationale**: see `wave-decisions.md` § "Reviewer-flagged decisions
resolved" / [D5].

**Action requested**: PO updates Consumer #3 of
`${listener_triple}`. The Validation field below it ("round-trip
property test asserts `JobSpecInput` ↔ `Job` ↔ TOML/JSON preserves
listener order and triple values bit-equivalently") is unaffected.

---

## Change 3 — Slice 06 ships as one slice (no split)

**Source**: `slice-06-service-listener-fields.md` § "Carpaccio
shape — single slice, defended" (already pre-anticipated this
verdict; see also `dor-validation.md:145` and `review-discuss.md`
non-blocking suggestion #1).

**Current text** (DISCUSS):
> If the architect later determines the alloc status render
> extension is non-trivial (e.g. requires denormalising listener
> triples onto `AllocStatusRow`), splitting at the AllocStatusRow
> boundary is the natural fault line — Slice 06a (parser + types
> + submit echo + OpenAPI + property test) and Slice 06b (alloc
> status listeners section). DISCUSS leaves that decision to
> DESIGN since the persistence shape is theirs to pin.

**DESIGN verdict**: KEEP AS ONE SLICE. The denormalisation IS
required (ADR-0047 §4a / Change 2 above), but the shape chosen
(embedded `Vec<ListenerRow>`) is mechanical from the spec-layer
`Listener` slice — adding a column type and a write-side copy is
~2h, not the multi-day effort that would warrant splitting. Total
slice effort holds at ~1.5d.

**Action requested**: no edit needed in `slice-06-*.md` — the
"Carpaccio shape" section already documents the kept-whole verdict
under the conditions ("If the architect later determines the alloc
status render extension is non-trivial"); the architect's
determination is now recorded as "the extension is mechanical, kept
whole." A one-line note can be appended:

> **DESIGN verdict (2026-05-10)**: kept whole; the alloc-status
> render extension is mechanical against the embedded `Vec<ListenerRow>`
> shape ratified by ADR-0047 §4a. See
> `design/wave-decisions.md` [D8].

---

## Summary

| Change | Affected artifact | Severity | PO action |
|---|---|---|---|
| 1 | `outcome-kpis.md` Measurement Plan K3 row | Low | One-line edit |
| 2 | `shared-artifacts-registry.md` `${listener_triple}` Consumer #3 | Low | Two-line edit + add Validation cross-ref to ADR-0047 §4a |
| 3 | `slice-06-service-listener-fields.md` Carpaccio shape section | Trivial | One-line append (optional) |

All three changes are clarifying refinements; none invalidate any
UAT scenario, AC, journey step, or KPI baseline. No DISCUSS user
story is contradicted. No DISCUSS journey is invalidated. No
DISCUSS slice is renumbered.
