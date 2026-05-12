# Walking skeleton — rkyv-envelope-evolution

**Feature**: `rkyv-envelope-evolution`
**Strategy**: A — Full real-adapter path (in-process rkyv + real redb)
**Date**: 2026-05-12

---

## Purpose

The walking skeleton answers one question: **can an operator who
upgraded the control-plane binary read yesterday's in-flight allocation
status from disk without `subtree pointer overran range` exploding the
convergence tick?**

That is the failure mode that surfaced 2026-05-12 (commits `6ffa9270`
and `e7b40282` appended fields to `AllocStatusRow`, breaking every
pre-existing redb file). The walking skeleton is the minimum E2E shape
that proves the new versioned-envelope mechanism prevents the
recurrence.

---

## Scope — what's in the skeleton

The skeleton lands one of the five envelope boundaries end-to-end:
`AllocStatusRowEnvelope`. The other four follow the same pattern in
DELIVER but do not block stakeholder demo-ability of the mechanism.

| Slice | Component | Real / Sim |
|---|---|---|
| `overdrive-core::codec::envelope` (trait + `EnvelopeError`) | new module | pure-Rust; no I/O |
| `AllocStatusRowEnvelope { V1(AllocStatusRowV1), V2(AllocStatusRowV2) }` | new enum + `pub(crate)` payload structs | rkyv-archived (real codec) |
| `AllocStatusRow::latest()` constructor | new constructor | pure-Rust |
| `Envelope::into_latest()` reader | new method | pure-Rust |
| `LocalObservationStore::alloc_status_rows` (with envelope decode) | updated host adapter | **real redb via `tempfile::TempDir`** |
| `LocalStore::open` (with envelope decode + refuse-to-start) | updated host adapter | **real redb via `tempfile::TempDir`** |
| `xtask::dst_lint::scan_for_envelope_variant_construction` | new scanner clause | pure-Rust AST scan |
| `xtask::dst_lint::scan_for_envelope_fixture_coverage` | new scanner clause | pure-Rust AST scan + directory listing |
| `IntentStoreError::Envelope` / `ObservationStoreError::Envelope` | new error variants | thiserror `#[from]` |

**No `Sim*` substitution.** The skeleton exercises the real codec
(rkyv) against the real adapter (redb) — the only "fake" is the
`tempfile::TempDir` that holds the redb file for the lifetime of the
test. Per Strategy A this satisfies Dimension 9c (every driven adapter
has a real-I/O integration test).

---

## Walking skeleton scenario (operator-framed)

```gherkin
@walking_skeleton @driving_port @real-io
Scenario: Operator restarts control-plane and observes yesterday's allocation status
  Given a node with a redb observation file containing one V1-shape AllocStatusRow
    | alloc_id     | workload_id  | node_id  | state    |
    | alloc-svc-01 | svc-payments | node-001 | Running  |
  And the control-plane binary has been upgraded to a build that knows V2
  When the control-plane boots against that redb file
  And a convergence tick runs
  Then the alloc-status row is read as the latest envelope shape
  And the row's state field still reads "Running"
  And the control-plane logs contain zero "subtree pointer overran range" entries
  And the convergence tick completes without error
```

**Litmus test verdict** (per Dimension 5):
- Title describes operator goal ("restarts control-plane and observes ...") — not a technical flow. ✓
- Then steps describe observable outcomes (state value, absence of log lines, tick completion) — not internal side effects. ✓
- Non-technical stakeholder can confirm: "an operator who upgrades expects yesterday's data to still be there" — yes. ✓

---

## What the skeleton does NOT include

- The other four envelopes (`NodeHealthRowEnvelope`,
  `ServiceHydrationResultRowEnvelope`, `ServiceBackendRowEnvelope`,
  `JobEnvelope`) — same pattern, deferred to post-WS DELIVER slices.
- V2 payload differs from V1 — for the WS, V1 and V2 can be
  structurally identical with a single field added (e.g. a new
  `pub(crate) terminal_v2: Option<TerminalCondition>` on `V2` that
  defaults to `None` from `V1::into_latest()`). The point is to
  exercise the *envelope* mechanism, not a real V2 schema bump.
- xtask coverage gate (S-EV-06) — the WS lands four envelopes' worth
  of fixtures; S-EV-06 closes the loop that enforces "every future
  envelope has a fixture" but is not required to prove the mechanism.

---

## Adapter coverage proof

Per Dimension 9c, the WS includes at least one real-I/O scenario per
driven adapter:

| Driven adapter | WS real-I/O coverage |
|---|---|
| `LocalObservationStore` | Inside the WS Gherkin above — `alloc_status_rows` reads through real redb |
| `LocalStore` | S-EV-03 (intent refuse-to-start) is the WS coverage for the intent adapter — included in the WS sequence |

Litmus test 9d ("if I deleted the real adapter, would this WS still
pass?"): no — the WS Gherkin explicitly asserts on `redb` file
contents, log output from the real `LocalObservationStore::alloc_status_rows`,
and process exit code from the real `LocalStore::open`. Replacing
either adapter with an InMemory double would break the WS. ✓

---

## Exit criteria — WS GREEN

The WS is GREEN when:
1. `AllocStatusRowEnvelope` and supporting types compile (no `todo!` left for the WS surface).
2. The Gherkin scenario above passes as a Rust integration test in `crates/overdrive-store-local/tests/integration/envelope_walking_skeleton.rs` (gated behind `integration-tests`).
3. The intent refuse-to-start scenario (S-EV-03 against `JobEnvelope` V1-only) passes.
4. `cargo nextest run -p overdrive-core --test schema_evolution` runs the `alloc_status_row` fixture and decodes the V1 golden bytes.
5. `cargo xtask dst-lint` passes (after the new `scan_for_envelope_variant_construction` clause lands).
6. Pre-existing tests still pass.

Once GREEN, the four remaining envelopes follow the same per-envelope
pattern; each new envelope adds one test file, one fixture, one enum,
and one adapter scenario.
