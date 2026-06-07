# Evolution — workflow-primitive

**Finalized:** 2026-06-06 (work delivered 2026-06-05)
**Wave arc:** DIVERGE → DISCUSS → DISTILL → DESIGN → DELIVER (13 steps / 3 slices) → FINALIZE
**Anchor:** GH #39; whitepaper §18; roadmap [3.2]; job J-PLAT-005
**Authoritative design records:** [ADR-0066](../product/architecture/adr-0066-workflow-journal-redb-second-table-layout.md),
[ADR-0064](../product/architecture/adr-0064-workflow-trait-ctx-and-engine-reconciler-boundary.md)
**Feature workspace (preserved):** `docs/feature/workflow-primitive/`

---

## Summary

Built the `Workflow` durable-execution primitive — the §18 peer to the
pure-sync `Reconciler`. A workflow is an ordinary `async fn run(ctx)`
whose progress is journaled in redb so it resumes after a crash from the
first incomplete step and drives to a single terminal result, with
effects exactly-once on the replay path. The author writes normal control
flow (`ctx.run` / `ctx.sleep` / `ctx.wait_for_signal` / `ctx.emit_action`);
the platform owns the journal, the cursor-based replay, and the
crash-resume correctness proof (a DST replay-equivalence invariant on the
CI critical path).

**Ratified direction: B′** — a distinct durable-async `Workflow`
primitive journaled in redb (Option B's ordinary-control-flow authoring
model, redb store instead of libSQL). This upheld the two-primitive
doctrine (whitepaper §18 / `reconcilers.md`): the discriminator is the
await/suspension/signal surface, not termination. Reuse verdict: 6
EXTEND/REUSE, 2 CREATE NEW (`JournalStore` port, `WorkflowEngine`),
**no new external dependency** (redb + ciborium already in the graph).

## Business / technical context

### The validated job (J-PLAT-005)

> When a platform subsystem must perform a finite, ordered, multi-step
> operation whose steps take externally-visible side effects unsafe to
> repeat (issue a cert, quiesce a region, snapshot a microVM, ratify a
> rollout), express the sequence as ordinary control flow and have the
> platform persist progress, resume after a crash from the first
> incomplete step, and drive it to a single terminal result **exactly
> once** — without hand-rolling a state machine, a crash-resume path, and
> a correctness proof for each one.

Load-bearing outcomes: **O1** no repeated side effect on resume, **O2**
no lost committed step, **O3** fast to author, **O4** resumed terminal ==
uninterrupted terminal, **O5** provable resume-equivalence before ship,
**O6** minimize the number of distinct persistence/recovery mechanisms.

### How the direction was chosen

DIVERGE scored six options and *recommended* Option C
(reconciler-as-step-machine) on the two heaviest criteria (mechanism
reuse + zero new concepts), explicitly **contingent** on DISCUSS
ratifying a doctrinal amendment to run terminal sequences on the
reconcile loop. The post-DIVERGE design dialogue instead **ratified B′**:
the two-primitive doctrine was judged load-bearing beyond mechanism
(suspension/await ergonomics, parent-child composition, the future WASM
extension surface), and version-skew was reframed as an SDK-era concern
rather than an architectural driver — withdrawing the penalty that had
ranked the replay options low. The journal moved to redb (not libSQL),
matching the peer reconciler primitive's ADR-0035 substrate decision and
serving O6.

## Key decisions (DESIGN, locked over B′)

| ID | Decision | Record |
|---|---|---|
| **D1** | Journal store = a **second redb table layout** behind a distinct `JournalStore` port — NOT an extension of `RedbViewStore`. Shares the redb file, `Arc<Database>`, codec, fsync-ordering, and Earned-Trust probe; differs in trait surface + table layout (append-only-ordered vs single-blob-overwrite). The central reuse call. | ADR-0066 §1 |
| **D2** | Journal codec = **CBOR (`ciborium`)**, not the ADR-0048 rkyv envelope. It is mutable runtime memory, not content-addressed; replay needs deterministic decode, and additive per-slice entry variants ride `#[serde(default)]`. | ADR-0066 §2 |
| **D3** | Engine ↔ lifecycle-reconciler boundary: the **reconciler stays pure-sync; the engine runs the async body off the action-shim.** The workflow-lifecycle reconciler emits `Action::StartWorkflow` and observes terminal rows (never `.await`); the engine is to workflows what `Driver` is to allocations. The subtlest boundary. | ADR-0064 §5 |
| **D4** | Replay = engine cursor + `ctx.*` check-then-record. Re-execute `run` from the top; replay returns recorded results without re-firing effects (exactly-once on replay, K1); live performs + appends (fsync-before-suspend). Step identity is positional; `name` is a diagnostic label + a fail-closed replay-determinism check. Honest semantics: **at-least-once effect, exactly-once on replay**. | ADR-0064 §3 |
| **D4a** | `ctx.run<T>(name, f)` (Restate `ctx.run` model — wrap any side-effecting future, journal its `T`, replay without re-running `f`) **replaces** the slice-01 `ctx.call(CallRequest) -> CallResponse` surface, which was hardcoded to a single transport datagram and could not return a value. `Transport` stays reachable via a `ctx.transport()` accessor. Greenfield single-cut — no shim, no migration. (User-pinned 2026-06-05.) | ADR-0064 §3 |
| **D5** | Crate placement mirrors the reconciler split: trait + ctx in `overdrive-core` (no tokio), engine + journal in `overdrive-control-plane`, sim journal + replay invariant in `overdrive-sim`. | ADR-0064 §1 |
| **D6** | `WorkflowCtx` surface grows additively per slice (run<T> → sleep → signal+emit); the machinery lands whole in slice 01. | ADR-0064 §4 |
| **D7** | `WorkflowResult` is distinct from `TerminalCondition` — inherits the SemVer convention, not the type. | ADR-0064 §2 |

## Steps completed

13 steps across 3 vertical slices, each RED → GREEN → COMMIT, green via
Lima. Outside-In TDD: most steps drove GREEN through a port-to-port
acceptance test and skipped a redundant RED_UNIT decomposition (recorded
as `NOT_APPLICABLE` with rationale — avoiding Testing Theater).

**Slice 01 — Walking skeleton: one durable step that survives a crash**

| Step | Commit | What landed |
|---|---|---|
| 01-01 | `272d556b` | `Workflow` trait, `WorkflowCtx`, `WorkflowResult`, concrete `WorkflowSpec` in core |
| 01-02 | `252e5fd3` | K6 no-step-machine + D-INH-4 ctx-only body compile-scans |
| 01-03 | `d4bc47a4` | `JournalStore` port + `SimJournalStore` + `ProvisionRecord` promotion |
| 01-04 | `fca40f24` | `RedbJournalStore` on the shared redb substrate |
| 01-05 | `41c5d784` | `WorkflowEngine` + journal-cursor replay; dispatch `StartWorkflow` off the shim |
| 01-06 | `0c037365` | engine terminal row + workflow-lifecycle reconciler |
| 01-07 | `08bd35f9` | three workflow DST invariants + walking-skeleton crash-resume |
| 01-08 | `274ee3b0` | full engine boot composition + e2e |

**Slice 02 — Durable `ctx.sleep` across a crash**

| Step | Commit | What landed |
|---|---|---|
| 02-01 | `70f4c52b` | `ctx.sleep` through `Clock`; `SleepArmed` journal variant records the deadline |
| 02-02 | `95caf0f8` | replay-equivalence across durable sleep + crash-resume invariants |
| — | `cad2fd4a` | refactor: replace `ctx.call` with the general `ctx.run<T>` durable-step primitive (D4a) |

**Slice 03 — Typed signals + workflow→cluster Action emission**

| Step | Commit | What landed |
|---|---|---|
| 03-01 | `0bd2dc8c` | `ctx.wait_for_signal` + `ctx.emit_action`; additive journal variants |
| 03-02 | `c94f9f6b` | `wait_for_signal` genuinely blocks on an absent signal; crash-while-blocked re-blocks on the SAME signal |
| 03-03 | `dea5cafe` | production emit → Raft drain wiring + e2e |

DELIVER opened with the wave docs (`25e65187` DIVERGE ratify, `a4c84a38`
DISCUSS, `74bbc75e` DESIGN/ADRs, `f5f5d27e` DISTILL 20 GWT scenarios + 21
RED scaffolds, `64e229a2` roadmap). Post-slice polish landed an L1–L6
cleanup pass (`fb18ebc5`) and honesty/robustness fixes (`b6c07ed9`
at-least-once docs; `9fbe8103` action-shim per-action isolation;
`bcc769d3` converge a panicked workflow to a `Failed` terminal +
unconditional live-instance teardown).

## Lessons learned

- **The DIVERGE recommendation is an input to DISCUSS, not the verdict.**
  The matrix recommended C; the user ratified B′ on premises the matrix
  did not encode (version-skew is SDK-era, not architectural; the
  doctrine is load-bearing beyond mechanism). The recommendation doc was
  honest about this — it framed the C-vs-F-vs-B choice as a *scope
  decision* belonging to DISCUSS — and that honesty is why the pivot was
  clean rather than a re-litigation.
- **Reuse the substrate, create the port.** D1 is the template for adding
  durable state to the platform without multiplying mechanisms: the
  `JournalStore` shares the `RedbViewStore` file, `Arc<Database>`, codec,
  fsync-ordering, and Earned-Trust probe, but gets its own trait surface
  and table layout because append-only-ordered point-access is a
  genuinely different shape from single-blob-overwrite. O6 served without
  forcing the journal into a port that doesn't fit.
- **The engine-is-to-workflows-what-Driver-is-to-allocations boundary
  (D3) keeps `core` pure.** Putting the async executor off the
  action-shim — with the lifecycle reconciler staying pure-sync and only
  emitting `Action::StartWorkflow` + observing terminal rows — is what
  lets the trait + ctx live in `overdrive-core` with no tokio, and what
  makes the whole primitive DST-replayable.
- **Honest semantics beat aspirational ones.** `ctx.run` is at-least-once
  on the live effect and exactly-once only on the replay path; the docs
  and the `emit_action` fix (`b6c07ed9`) say so plainly rather than
  implying a stronger guarantee the journal does not provide.
- **A latent trap surfaced immediately downstream.** Slice 03 left
  `Started` documented as the journal's first entry while the engine
  never wrote it, and the positional cursor could fall through to the
  live path on a variant mismatch. That was closed by the follow-on
  **`workflow-journal-command-notification-split`** feature (finalized
  2026-06-06, see `docs/evolution/2026-06-06-workflow-journal-command-notification-split.md`)
  — evidence that shipping the walking skeleton early made the structural
  gap discoverable before it could corrupt a real journal.

## Issues / deferrals (verified, cite-by-number — nothing created)

- **HA cross-node resume** — [#205](https://github.com/overdrive-sh/overdrive/issues/205)
  (verified open). Single-node crash-resume only this phase (D3); the
  redb-journal + `JournalStore`-port shape does not preclude a future HA
  adapter.
- **No `overdrive workflow` CLI verb** — [#206](https://github.com/overdrive-sh/overdrive/issues/206).
  The observable surface this phase is the ObservationStore terminal row +
  structured lifecycle events + the DST invariant; no operator verb was
  invented.
- **Determinism Layer 3 (content/digest comparison)** —
  [#214](https://github.com/overdrive-sh/overdrive/issues/214), carried by
  the follow-on split feature; out of scope here.

No design element hinges on code-graph hashing (the deferred version-skew
mitigation was reframed as SDK-era, not architectural).

## Migrated / permanent artifacts

- **ADR-0066** (journal redb second-table layout) and **ADR-0064**
  (Workflow trait/ctx + engine↔reconciler boundary) — authored through
  the architect agent; already in `docs/product/architecture/`, not
  migrated here.
- **Feature workspace** — `docs/feature/workflow-primitive/` is preserved
  (the wave matrix derives status from it). It holds the SSOT trail: the
  DIVERGE option matrix + `recommendation.md`, DISCUSS journey/stories/
  KPIs, DISTILL `test-scenarios.md` (20 GWT) + `red-classification.md`,
  DESIGN `wave-decisions.md` + `feature-delta.md`, the three slice specs,
  and the DELIVER `roadmap.json` + `execution-log.json`.
- **Implementation** — `overdrive-core` (`workflow/`), `overdrive-control-plane`
  (`workflow_runtime/`, `journal/`, workflow-lifecycle reconciler,
  action-shim wiring), `overdrive-sim` (`adapters/journal.rs`, the
  `replay_equivalence_provision_record` invariant) per the commits above.

No `docs/architecture/`, `docs/scenarios/`, or `docs/ux/` standard-map
artifacts were produced (this feature's design lives in the preserved
feature workspace + the two ADRs).
