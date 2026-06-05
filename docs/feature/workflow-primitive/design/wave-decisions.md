# DESIGN Decisions — workflow-primitive

Wave: DESIGN (Morgan / nw-solution-architect) · Date: 2026-06-05 · Mode:
PROPOSE · Density: lean. Architecture **locked to B′** (DIVERGE/DISCUSS);
designed OVER it. GH #39, roadmap [3.2]. Job: J-PLAT-005.

## Key Decisions

- [D1] **Journal store = a second redb table layout, distinct `JournalStore`
  port, NOT an extension of `RedbViewStore`.** Shares the `RedbViewStore` redb
  file + `Arc<Database>` + codec + fsync-ordering + Earned-Trust probe; differs
  in trait surface + table layout (append-only-ordered vs single-blob-overwrite).
  THE central reuse call. (see: ADR-0063 §1; the §17 "second table layout"
  reconciliation.) **RATIFY.**
- [D2] **Journal codec = CBOR (`ciborium`, ADR-0035 §3 discipline), NOT the
  ADR-0048 rkyv envelope.** Mutable runtime memory, not content-addressed;
  replay needs deterministic decode (CBOR), not zero-copy archived-byte
  canonicality; additive per-slice entry-variants ride `#[serde(default)]`.
  (see: ADR-0063 §2.) **RATIFY.**
- [D3] **Engine↔lifecycle-reconciler boundary: the reconciler stays pure-sync;
  the engine runs the async body off the action-shim.** The workflow-lifecycle
  reconciler emits `Action::StartWorkflow` + observes terminal rows (never
  `.await`); the engine is the async executor off the shim, exactly as
  `StartAllocation`→`Driver::start`. The engine is to workflows what `Driver`
  is to allocations. (see: ADR-0064 §5.) **RATIFY — the subtlest boundary.**
- [D4] **Replay = engine cursor + `ctx.*` check-then-record.** Re-execute `run`
  from the top; replay returns recorded results without re-firing effects
  (exactly-once K1); live performs + appends (fsync-before-suspend). All
  non-determinism through `ctx` ⇒ bit-identical replay (K4). (see: ADR-0064 §3.)
- [D5] **Crate placement: trait+ctx in `overdrive-core` (no tokio), engine +
  journal in `overdrive-control-plane`, sim journal + replay invariant in
  `overdrive-sim`.** Mirrors the reconciler split; honors `core`-has-no-tokio.
  (see: ADR-0064 §1.)
- [D6] **`WorkflowCtx` surface additive per slice** (call/01 → sleep/02 →
  signal+emit/03); machinery whole in slice 01. (see: ADR-0064 §4.)
- [D7] **`WorkflowResult` distinct from `TerminalCondition`** — inherits the
  SemVer convention, not the type. (see: ADR-0064 §2.)

## Architecture Summary

- **Pattern:** hexagonal (ports & adapters), single-process — the §18
  durable-async `Workflow` primitive, peer to the pure-sync `Reconciler`.
- **Paradigm:** OOP (Rust trait-based) — unchanged project default.
- **Key components:** `Workflow` trait + `WorkflowCtx` (core) · `WorkflowEngine`
  (control-plane, async, off the shim) · `JournalStore`/`RedbJournalStore`
  (control-plane) · workflow-lifecycle reconciler (pure-sync) · `SimJournalStore`
  + `replay_equivalence_provision_record` invariant (sim).

## Reuse Analysis

| Existing component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `Action::StartWorkflow` placeholder | `reconcilers/mod.rs:373` | lifecycle trigger | EXTEND | Already the locked D-INH-3 shape |
| `WorkflowSpec` placeholder | `reconcilers/mod.rs:562` | the spec | EXTEND (concrete) | Already in core; replace empty struct |
| `ReplayEquivalentEmptyWorkflow` | `overdrive-sim` invariants | replay invariant | EXTEND (graduate) | Placeholder explicitly says Phase 2 replaces it |
| `RedbViewStore`/`ViewStore` | `view_store/` | redb durable memory + discipline | REUSE substrate; CREATE NEW port | Substrate shared; trait+layout differ (ADR-0063 §1) |
| action-shim + reconciler runtime | `action_shim/mod.rs:446` | async-effect pipeline | EXTEND | Engine off the same shim |
| `Clock`/`Transport`/`Entropy` | `traits/` | injected non-determinism | REUSE | `WorkflowCtx` wraps existing ports |
| `CorrelationKey`/`HttpCall` | `id.rs:538` | call + correlation | REUSE | `ctx.call` reuses them |
| `TerminalCondition` | core | terminal modelling | DO NOT REUSE (relate) | Different thing; convention inherited |
| `TickContext` | core | injected bundle | DO NOT REUSE (analogue) | Full ctx surface vs time-only |
| `JournalStore`/`RedbJournalStore`/`SimJournalStore` | NEW | journal layout | CREATE NEW | No trait hosts append-only-ordered point-access |
| `WorkflowEngine` | NEW | async executor | CREATE NEW | No component runs journaled async; reconciler pure-sync |

**Verdict: 6 EXTEND/REUSE, 2 DO-NOT-REUSE-(relate), 2 CREATE NEW (justified).**

## Technology Stack

- Rust 2024; `tokio` (engine only, control-plane); `async_trait` (core trait).
- `redb` 2.x (shared substrate) + `ciborium` (CBOR codec) — both already in
  the dep graph. **No new external dependency.**
- `turmoil` + `Sim*` adapters; K4 invariant on the CI critical path.
- No proprietary deps; no contract tests this phase (no external boundary).

## Constraints Established

- Journal on redb, distinct `JournalStore` port, shared substrate, CBOR codec
  (no libSQL journal — K5; no rkyv envelope for the journal).
- `reconcile` stays pure; all workflow async lives in the engine off the shim.
- `core` carries no tokio; engine + journal in control-plane.
- All workflow non-determinism through `ctx`; `ctx.emit_action` → Action
  channel → Raft (no IntentStore bypass).
- Single-node crash-resume only (D3); cross-node resume (#205) not precluded.
- No `overdrive workflow` CLI verb invented (#206).
- No design element hinges on code-graph hashing (R1/D-INH-6).

## Upstream Changes

None to DISCUSS/DIVERGE artifacts (architecture locked from DIVERGE). The
pre-DIVERGE whitepaper "per-primitive libSQL" journal phrasing is superseded
by the redb decision (R2) — already reconciled in the *current* whitepaper
§17/§18 text (the "second redb table layout" wording is present); ADR-0063
records the decision formally. No ADR's existing content modified; ADR-0013
not further superseded (it is already Superseded by 0035). ADR-0063 and
ADR-0064 are additive.

## Outcome Collision Check

`docs/product/outcomes/registry.yaml` not present and the
`nwave-ai outcomes check-delta` CLI was not invoked — **registry not present,
skipped** (no fabrication). When the registry is introduced, the candidate
outcomes for this feature (durable exactly-once terminal sequence; provable
replay-equivalence) should be checked against any future OUT-N rows.

## Peer Review Recommendation

**Recommend deferring to the consolidated DISTILL gate** (the mandatory
4-wave parallel review at end of DISTILL), NOT an optional per-wave architect
review. Rationale: the architecture is *locked from DIVERGE* (no contested
style/pattern choice remains to litigate); the two new ADRs *extend* well-
established precedents (ADR-0035 substrate/codec/probe, ADR-0023 shim) rather
than introducing a novel pattern; there is no unverified performance budget
(K4 is a correctness gate, not a latency target) and no security-boundary
change (workflow→cluster goes through the existing Raft path). The per-wave
review triggers (contested ADR / novel pattern / unverified perf budget /
security boundary change) are all absent. The three sub-decisions flagged
RATIFY (D1 journal-store, D2 codec, D3 engine↔reconciler boundary) are
surfaced to the user for ratification in the DESIGN return summary — that is
the appropriate gate for a locked-direction design, not a fresh adversarial
review pass.
