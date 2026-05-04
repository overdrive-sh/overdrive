# ADR-0037 — Reconciler emits typed `TerminalCondition`; streaming forwards it; `LifecycleEvent` no longer projects reconciler-private View state

## Status

Accepted. 2026-05-03. Decision-makers: Morgan (proposing); user
ratification 2026-05-03 (single-deliverable codification of the
recommendation in
`docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`).
Tags: phase-1, reconciler-primitive, application-arch, http-shape,
streaming.

**Companion ADRs**: ADR-0035 (collapsed `Reconciler` trait + runtime-
owned `ViewStore`); ADR-0036 (`AnyState` amendment removing per-
reconciler `hydrate`); ADR-0032 (NDJSON streaming + `LifecycleEvent`
+ `TransitionReason`); ADR-0033 (`alloc_status` snapshot enrichment +
`RestartBudget`).

**Lands alongside the ADR-0035 reset**: the in-flight branch
`marcus-sa/libsql-view-cache` is being reset against `main` (per
`docs/feature/reconciler-memory-redb/design/upstream-changes.md`
§D2). This ADR lands in the same wave so the new DELIVER roadmap that
follows the reset wires `terminal` from day one — not "ship without it
and add later." See *Implementation note* below.

## Context

Step 02-04 of the in-flight issue-139 work retired the in-process
`view_cache` shadow above libSQL and stamped a `restart_count_max:
u32` projection of `JobLifecycleView` directly onto the polymorphic
`LifecycleEvent` broadcast payload. The streaming consumer
(`streaming.rs::check_terminal`) read the projection plus the
hard-coded `RESTART_BACKOFF_CEILING` constant to decide whether a
`Failed` event was terminal. A second emission site — the exit
observer — passed the literal `0` because it had no `JobLifecycleView`
in scope.

Three smells surfaced together:

1. The `restart_count_max: 0` literal in
   `crates/overdrive-control-plane/src/worker/exit_observer.rs` was
   structurally meaningless. Every future emission site outside a
   reconciler tick (a planned node-health reconciler, any second
   reconciler that broadcasts on the same channel) would need to
   reproduce the literal-0 puzzle.
2. The streaming layer made the *terminal-or-not* decision —
   `event.restart_count_max >= RESTART_BACKOFF_CEILING` — using a
   policy constant and an aggregated input, not a reconciler-published
   conclusion. This is a layering inversion: business decisions
   belong in the reconciler that owns the View (per ADR-0013 §1, now
   ADR-0035 §1), not in the broadcast subscriber.
3. The projection violated the project's own "Persist inputs, not
   derived state" rule (`.claude/rules/development.md`). The field
   was derived from `restart_counts.values().max()` interpreted
   against the ceiling constant; making the ceiling
   operator-configurable in any future phase silently invalidates
   every previously-broadcast event, because the consumer's
   "exhausted = used >= ceiling" derivation is bound to the ceiling
   in force at decode time, not at emit time.

The follow-up research
(`docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`)
evaluated five candidates against four established precedents
(Kubernetes Conditions, controller-runtime informers, Fowler's
Event-Carried-State-Transfer vs Event Notification, Erlang/OTP
supervisor introspection). The recommendation is **candidate (c)**:
the reconciler decides terminal-or-not from inputs in scope and emits
a typed condition; streaming forwards the condition without
re-deriving from inputs. From §6 of the research:

> "The decision IS a stable contract: once a transition to Failed is
> reported as *terminal* (no further restart attempts will be made),
> that's a property no future policy change can revoke. The terminal
> flag is *interpretive output*, not *cached input*. This is the same
> distinction Vernon makes between Domain Events (raw facts) and
> Integration Events (interpretive, stable-schema facts published
> outside the bounded context)."

### Composition with ADR-0035 — why this is now structural, not convention

ADR-0035 collapses the reconciler trait and moves View persistence
into a runtime-owned `ViewStore` backed by a per-node redb file.
**Under that decision, the View blob is genuinely private to the
reconciler runtime: it lives in redb, it is bulk-loaded at register
time, it is held in an in-memory `BTreeMap<TargetResource, View>`,
and no consumer outside the runtime ever reads it.** The View has
exactly one publication path: through the Actions a reconciler emits.

That removes the ambient option that step 02-04 took — "project a
View field onto the broadcast event." There is no longer a path from
View to `LifecycleEvent` that does not pass through the Action
channel. The recommendation in the follow-up research becomes
structurally inevitable rather than discipline reviewers must
catch:

- View blob carries inputs only (`restart_counts`,
  `last_failure_seen_at`) — already required by *Persist inputs, not
  derived state*.
- `reconcile` recomputes terminal-or-not every tick from the inputs
  plus the live policy.
- When terminal, the emitted Action carries `terminal:
  Some(TerminalCondition::…)`. Action is the publication boundary
  where reconciler-private state crystallises into a stable contract.
- The action shim writes the observation row with `terminal` AND
  echoes `terminal` onto `LifecycleEvent`. Both consumer-facing
  surfaces (the HTTP `alloc_status` snapshot, the streaming event
  bus) read Action-derived state, never the View.
- Streaming never projects the View. `streaming.rs::check_terminal`
  collapses to `if let Some(cond) = &event.terminal`.

The value of pairing ADR-0037 with ADR-0035 is that the layering rule
("no derived projections of reconciler memory on `LifecycleEvent`")
is now structurally enforced — reconciler memory has no consumers,
only Actions are publication-boundary-crossing — rather than a
discipline reviewers must catch on every PR.

## Decision

### 1. Define `TerminalCondition` in `overdrive-core`

A new public enum lives next to the `LifecycleEvent` definition in
`overdrive-core` (per ADR-0032 §"`LifecycleEvent` lives in
`overdrive-core` next to the trait surface"):

```rust
/// Reconciler-decided terminal condition for an allocation.
///
/// Emitted by a reconciler when its `reconcile` body concludes that
/// no further convergence work will be attempted for this allocation.
/// The variant is *interpretive output* — the reconciler's stable
/// claim about what concluded the allocation's lifecycle — and is
/// computed from inputs (`restart_counts`, `last_failure_seen_at`,
/// the live policy) at the moment of the deciding tick.
///
/// Per `.claude/rules/development.md` § "Persist inputs, not derived
/// state": the variants here describe the reconciler's *decision*,
/// never the inputs that fed the decision. Adding an
/// operator-configurable per-job restart-budget policy in a future
/// phase changes which `restart_counts` value triggers
/// `BackoffExhausted`; it does not change the meaning of
/// `BackoffExhausted` itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TerminalCondition {
    /// JobLifecycle: restart budget reached; no further attempts will
    /// be scheduled. `attempts` is the count consumed at the moment
    /// of the deciding tick.
    BackoffExhausted { attempts: u32 },

    /// JobLifecycle: explicit operator stop converged. The
    /// allocation reached `Stopped` because the operator (or the
    /// reconciler itself) requested it, not because of a failure.
    Stopped { by: StoppedBy },

    /// Forward-compat for WASM third-party reconcilers per whitepaper
    /// §18 (*Extension Model*: trait surface identical between Rust
    /// first-party and WASM third-party).
    ///
    /// `type_name` is a CamelCase identifier scoped by the
    /// reconciler's canonical name (e.g.
    /// `"vendor.io/quota.QuotaExhausted"`); `detail` is opaque
    /// rkyv-encoded bytes the reconciler may attach. Streaming
    /// forwards `Custom` verbatim; the well-known first-party
    /// variants above stay in the closed set and are
    /// compile-time-checked at every consumer.
    Custom {
        type_name: String,
        detail:    Option<Vec<u8>>,
    },
}
```

`StoppedBy` is the existing enum from ADR-0032 §"`TransitionReason`
shape" (`enum StoppedBy { Operator, Reconciler }`); the variant is
re-used unchanged.

### 2. `LifecycleEvent.terminal: Option<TerminalCondition>` replaces `restart_count_max`

```rust
pub struct LifecycleEvent {
    // ... existing fields per ADR-0032 §"LifecycleEvent shape" ...

    /// `Some` when the emitting reconciler decided this transition
    /// concludes the allocation's lifecycle. `None` when this is a
    /// non-terminal transition (Pending → Running, Running → Failed
    /// with budget remaining, etc.). Replaces step-02-04's
    /// `restart_count_max: u32` field.
    ///
    /// The exit observer and other emission sites outside a
    /// reconciler tick emit `terminal: None` — *structurally
    /// meaningful* ("I am not making a terminal claim"), not the
    /// step-02-04 `restart_count_max: 0` literal which was
    /// *structurally meaningless* ("no info, downstream
    /// conservatively ignores").
    pub terminal: Option<TerminalCondition>,
}
```

The step-02-04 `restart_count_max: u32` field is deleted. There is
no parallel-fields transitional period — the ADR-0035 reset of the
in-flight branch absorbs the change in the same DELIVER replan.

### 3. `AllocStatusRow.terminal: Option<TerminalCondition>` is the durable home

The reconciler's terminal decision is durable ObservationStore state,
not transient broadcast-only data. Following the same pattern that
ADR-0032 §3 amendment used for `reason` /
`detail`: the action shim writes `terminal` onto the `AllocStatusRow`
when it dispatches the deciding action, and echoes the same value
onto the `LifecycleEvent` it broadcasts after the row write. Both
consumer surfaces read row-derived state:

- The HTTP `alloc_status` handler (ADR-0033) reads the row and
  derives `RestartBudget.exhausted` from
  `row.terminal.is_some_and(|t| matches!(t, TerminalCondition::BackoffExhausted { .. }))`
  rather than from `restart_counts`. This collapses ADR-0033's
  `RestartBudget.exhausted` derivation to a row-field read; the
  `JobLifecycleView` is not consulted.
- The streaming event subscriber reads `event.terminal` directly.
  `streaming.rs::check_terminal` collapses to a single `if let`.

`AllocStatusRow.terminal` is an additive `Option<TerminalCondition>`
field. Per whitepaper §4 *Consistency Guardrails* and the project's
additive-only schema-migration discipline, this is a forward-
compatible addition; existing redb files decode the `Option` as
`None`.

### 4. Action shim integration

The reconciler emits the terminal decision as a typed flag on the
relevant `Action` variant — not as a separate parameter threaded
through the runtime. The two `Action` variants that conclude an
allocation's lifecycle today (`Action::StopAllocation`, the synthetic
`AllocStatusUpdate { state: Failed, ... }` shape per ADR-0023) gain a
`terminal: Option<TerminalCondition>` field. The action shim:

1. Writes `AllocStatusRow { terminal, .. }` to the ObservationStore.
2. Constructs `LifecycleEvent { terminal, .. }` and broadcasts.

Both writes carry the same `TerminalCondition` value — drift between
the snapshot surface and the streaming surface is structurally
impossible. This is the same pattern ADR-0033 §5 established for
`TransitionReason` ("byte-equality is structural across both wire
shapes … no string drift, no re-stringification") applied to the new
field.

The action shim does not synthesise terminal decisions on its own —
all terminal claims originate in `reconcile`. Emission sites outside
a reconciler tick (the exit observer, ADR-0023 action-shim noop
heartbeat) emit `terminal: None`.

### 5. SemVer convention for `TerminalCondition`

Adopting the Kubernetes `Condition.Reason` convention (per the
follow-up research §Knowledge-Gap-3 recommendation):

- **Well-known first-party variants** (`BackoffExhausted`,
  `Stopped`) are stable contract. Once published, the variant tag
  and field shape are pinned across every consumer (CLI, third-party
  SDK, audit log decoder).
- **Renames are major-bump breaking changes.** The `#[non_exhaustive]`
  attribute on `TerminalCondition` ensures consumers must already
  match exhaustively with a wildcard arm, but a rename of a
  well-known variant breaks every consumer that branches on the old
  name. Reserve renames for genuine semantic changes; treat them as
  major-version breaking changes when the public API is versioned.
- **New variants are additive minor.** A future
  `TerminalCondition::DriftLimit { ... }` for a Phase-2 right-sizing
  reconciler is an additive change; consumers handle it via the
  wildcard arm until they branch on it explicitly.
- **`Custom { type_name, detail }`** is the WASM-extensibility
  surface. Third-party reconcilers populate `type_name` with a
  CamelCase identifier scoped by the reconciler's canonical name
  (`vendor.io/quota.QuotaExhausted`). Consumers that don't
  understand a particular `type_name` forward it verbatim and
  display it as-is; they don't crash.

This addresses the research's *Gap 3 — Behavior under reconciler
version skew* directly. The convention is documented here so that
when Phase 2+ adds new variants, the rule is already established
rather than retrofit under pressure.

## Considered alternatives

The follow-up research evaluated five candidates against four
precedents. Rather than re-deriving the matrix here, this ADR cites
the research's §"Candidate Evaluation Matrix" verbatim and summarises
why each non-(c) candidate was rejected:

| Candidate | Why rejected |
|---|---|
| (a) Reconciler-typed event sum-type (per-reconciler `LifecycleEvent` variants) | Closed Rust enum cannot be extended by WASM third-parties without an "untyped variant" escape hatch that defeats the typing benefit. Contradicts whitepaper §18's "trait surface identical between Rust and WASM" claim. Migration cost ~1–2 days for medium principled-cleanup gain. |
| (b) Streaming owns its own JobLifecycle-shaped read model (full CQRS) | Duplicates state for one consumer at v1 scale. Fowler's own CQRS guidance: "only use it on specific portions of a system" where the read model differs substantially from the write model — for `restart_count_max` the read model is one column of the write model. Migration cost ~1 week for no consumer-cardinality benefit. |
| (d) Separate observation row for `RestartBudget` | Schema-level work — Corrosion gossip path, observation-store row type, sweep reconciler. Composes with the recommendation of this ADR (the row carries `terminal` rather than a separate budget shape) but is independent; deferred to Phase 2+ if a real driver appears. Migration cost ~3–4 days. |
| Step-02-04 status quo (`restart_count_max: u32` on the generic event) | Three smells listed in *Context* above. The exit_observer literal-`0` is a textbook layering smell; the field violates *Persist inputs, not derived state*; the streaming layer makes business decisions a consumer should not. Zero migration cost (already shipped) but is the explicit design defect this ADR exists to fix. |
| **(c) Reconciler decides terminal, emits typed condition** (ACCEPTED) | Lowest v1 migration cost (~half a day under the ADR-0035 reset, since the new DELIVER roadmap wires `terminal` from day one). Aligns with K8s `Condition.Reason` shape. Composes structurally with ADR-0035's runtime-owned View. Provides a forward-compat surface for WASM third-parties (`Custom` variant). |

The full evaluation, the precedent-by-precedent justification, and
the candidate-cost analysis live in
`docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`
§§4–6. This ADR pins the decision; the research carries the rationale.

## Consequences

### Positive

- **`streaming.rs::check_terminal` simplifies from ~30 LOC to ~5
  LOC.** The block currently shaped as
  `if matches!(event.to, AllocStateWire::Failed) { let used =
  event.restart_count_max; if used >= RESTART_BACKOFF_CEILING { ... } }`
  collapses to `if let Some(cond) = &event.terminal { ... }`. The
  hard-coded `RESTART_BACKOFF_CEILING` constant disappears from the
  streaming path entirely — it remains a JobLifecycle-internal
  policy, not a streaming concern.
- **`streaming.rs::lagged_recover`'s `restart_count_max_hint`
  parameter is deleted.** The lag-recovery snapshot path reads
  `latest.terminal` directly off the observation row instead of
  re-deriving from a hint passed across the call.
- **`exit_observer.rs`'s `restart_count_max: 0` literal disappears.**
  The site emits `terminal: None`, which is structurally meaningful
  ("I am not making a terminal claim") rather than structurally
  meaningless ("no budget consumed"). Every future second-emission
  site (the planned node-health reconciler, any reconciler that
  broadcasts outside its own tick) inherits the `None` shape without
  the literal-0 puzzle.
- **WASM third-party reconcilers can extend the terminal vocabulary
  without touching core types.** The `Custom { type_name, detail }`
  variant is the K8s-Condition-shaped open vocabulary that whitepaper
  §18 *Extension Model* requires. The runtime never inspects
  `type_name`; it forwards verbatim. Consumers that recognise a
  particular `type_name` branch on it; consumers that don't, display
  it as-is.
- **`RestartBudget.exhausted` (ADR-0033 §1) becomes a row-field read
  rather than a recomputation from `restart_counts`.** The HTTP
  handler reads `row.terminal.is_some_and(|t| matches!(t,
  TerminalCondition::BackoffExhausted { .. }))`. The
  `JobLifecycleView` is not consulted by the snapshot handler at
  all under this ADR. ADR-0033's source-map row for
  `restart_budget.exhausted` is updated accordingly (see
  *Cross-references* below).
- **The §18 layering inversion is closed structurally.** Streaming
  stops making business decisions about budget exhaustion; the
  reconciler owns the decision, the streaming layer just forwards.
  Combined with ADR-0035's runtime-owned View — which makes the
  blob inaccessible to streaming in the first place — the
  "no derived projections of reconciler memory on `LifecycleEvent`"
  rule is enforced by the type system (the View handle does not
  exist on the streaming path) rather than by reviewer discipline.
- **Aligns with the canonical K8s `Condition` shape.** Operators
  familiar with controller-runtime, operator-sdk, Crossplane, Tekton
  recognise this shape on day one. There is an industry-standard
  vocabulary for "the reconciler decided this is terminal."

### Negative

- **One small enum addition + one
  field-rename-with-shape-change on `LifecycleEvent`.** The
  serialised wire shape changes
  (`restart_count_max: u32` → `terminal: Option<TerminalCondition>`).
  Single-cut migration per the project's greenfield convention; no
  parallel-fields transitional period.
- **`Action::StopAllocation` and the synthetic Failed-row action gain
  a `terminal: Option<TerminalCondition>` field.** The reconciler is
  now the source of the terminal claim, so the action shim picks up
  one extra parameter to thread onto both the row and the event.
  Localised to the action shim and the two affected `Action` variants.
- **`AllocStatusRow.terminal` is a new redb column.** Additive
  per the project's additive-only schema-migration discipline.
  Existing `Option<TerminalCondition>` archives decode as `None`.

### What this ADR does NOT do (deferred)

- **Does not introduce a separate observation row for
  `RestartBudget` (research candidate (d)).** The `RestartBudget`
  wire shape on `alloc_status` (ADR-0033) is preserved; only its
  `exhausted` field's source changes (from a `restart_counts`
  recomputation to a `terminal` row-field read). A separate
  observation-row-level schema for budget state is a Phase 2+
  concern and would compose cleanly with this ADR if it lands.
- **Does not introduce per-reconciler payload variants on
  `LifecycleEvent` (research candidate (a)).** The `Custom { type_name,
  detail }` variant is the deliberate concession to WASM
  extensibility; per-reconciler enum variants are the closed-world
  shape that contradicts whitepaper §18 *Extension Model*.
- **Does not introduce a streaming-side materialised view (research
  candidate (b)).** Streaming reads `event.terminal`; no second
  persistent state.

### Quality-attribute impact

- **Maintainability — modifiability**: positive (small).
  `streaming.rs::check_terminal` shrinks; the hard-coded
  `RESTART_BACKOFF_CEILING` reference disappears from the streaming
  path. Future operator-configurable per-job ceiling policy lands
  inside `JobLifecycle::reconcile` only.
- **Maintainability — testability**: positive. The terminal decision
  is computed inside `reconcile`, which is a pure synchronous
  function under ADR-0035; the existing `ReconcilerIsPure` invariant
  (ADR-0017) already constrains it. A new property test asserts
  that the terminal decision is a function of the View inputs and
  the live policy, not of any latent state.
- **Reliability — surface coherence**: positive. `LifecycleEvent.terminal`
  and `AllocStatusRow.terminal` are populated by the same action-shim
  call site with the same value. Drift between the snapshot surface
  (HTTP `alloc_status`) and the streaming surface
  (`SubmitEvent::ConvergedFailed` derived from `LifecycleEvent`) is
  structurally impossible.
- **Compatibility — evolvability**: positive. The K8s
  `Condition.Reason` SemVer convention is documented as part of this
  ADR. Phase 2+ additions are additive minor; renames are major.
  Addresses the research's *Gap 3* explicitly rather than leaving it
  as a known unknown.
- **All other attributes**: neutral.

## Implementation note

This ADR lands in lockstep with the ADR-0035 reset. The DELIVER
replan that follows (per
`docs/feature/reconciler-memory-redb/design/upstream-changes.md`
§D2 — the user has elected reset over refactor-in-place) **must wire
`terminal` from day one** rather than ship the ADR-0035 collapse with
the step-02-04 `restart_count_max` field still in place and add
`terminal` later.

Concretely, the new DELIVER roadmap should sequence the change such
that:

1. The `Action` enum gains `terminal: Option<TerminalCondition>` on
   the relevant variants in the same step that introduces the
   typed-View runtime contract.
2. `JobLifecycle::reconcile` computes `terminal` from
   `view.restart_counts`, `view.last_failure_seen_at`, and the live
   `RESTART_BACKOFF_CEILING` policy, and stamps it on the Action.
3. The action shim writes `AllocStatusRow.terminal` and emits
   `LifecycleEvent.terminal` in the same dispatch.
4. `streaming.rs::check_terminal` reads `event.terminal` directly;
   `streaming.rs::lagged_recover` drops its `restart_count_max_hint`
   parameter and reads `latest.terminal` off the observation row.
5. `exit_observer.rs` emits `terminal: None`.

Whoever owns DELIVER replanning: do NOT plan a roadmap that lands
ADR-0035 first and `terminal` second. The two changes share a
publication boundary (the Action → row → event path); landing them
separately means the action shim is rewritten twice and the
`restart_count_max` literal-0 smell persists between the two PRs.
One coherent step.

## References

- `docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`
  — the research codified by this ADR (4 precedents × 5 candidates;
  19 sources, avg reputation 0.94; High confidence; recommendation
  in §6 *Recommended Shape*).
- ADR-0013 — Reconciler primitive; superseded by ADR-0035; cited
  here for the §2b 7-step contract context that originally produced
  the `restart_count_max` projection.
- ADR-0021 — `AnyState` enum; amended by ADR-0036.
- ADR-0032 — NDJSON streaming submit; defines `LifecycleEvent`,
  `TransitionReason`, `StoppedBy`, `SubmitEvent::ConvergedFailed`,
  the broker fan-out path. The `LifecycleEvent` shape gains
  `terminal: Option<TerminalCondition>` and loses
  `restart_count_max: u32` per this ADR.
- ADR-0033 — `alloc_status` snapshot enrichment;
  `RestartBudget.exhausted` source changes from a `restart_counts`
  recomputation to an `AllocStatusRow.terminal` row-field read per
  this ADR. The §2 field-source-map row for
  `restart_budget.exhausted` is updated in lockstep; the
  `RestartBudget` wire shape itself is unchanged.
- ADR-0035 — Reconciler memory collapse; the structural decision
  this ADR composes with. Under ADR-0035 the View is genuinely
  private to the runtime; the layering rule this ADR encodes
  ("no derived projections of reconciler memory on `LifecycleEvent`")
  becomes structurally enforced rather than convention.
- ADR-0036 — Amendment to ADR-0021; companion to ADR-0035.
- Whitepaper §18 — *Reconciler and Workflow Primitives*, *Extension
  Model*: the WASM third-party trait-surface-identical claim that
  motivates the `Custom` variant.
- `.claude/rules/development.md` § "Persist inputs, not derived
  state" — the rule the recommendation honours; `TerminalCondition`
  variants are *interpretive output* (a stable contract) rather
  than *cached input* (which would inherit the bug class the rule
  was written to prevent).

## Changelog

- 2026-05-03 — Initial accepted version. Codifies the recommendation
  from `issue-139-followup-streaming-restart-budget-research.md`
  candidate (c). Lands alongside the ADR-0035 reset of the in-flight
  `marcus-sa/libsql-view-cache` branch.
