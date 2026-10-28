# STEP 13-02 PRCI-1 independent DESIGN contract review

## Metadata and verdict

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `13-02` |
| Date | 2026-10-28 |
| Role | Fresh independent solution-architecture reviewer |
| Iteration | 1 |
| Verdict | **APPROVED** |
| Blocking findings | 0 critical; 0 high |
| Scope | Exact PROBE-RUN-COUNT raw-input carrier only |
| Execution disposition | The roadmap's `validation.status` remains `pending` during this review; this artifact does not change it or authorize a different implementation shape. |

The review used the current `AGENTS.md`, applicable `.claude/rules/design.md` and `development.md`, the installed `nw-solution-architect-reviewer` definition, `nw-sa-critique-dimensions`, `nw-sar-critique-dimensions`, and `nw-roadmap-review-checks`. The repository's bounded-review and native-Markdown rules supersede the installed definition's legacy YAML and iteration defaults. I edited only this review artifact. I did not change source, tests, design, roadmap, DES state, or host state, and did not commit.

## Artifact and evidence boundary

| Proposed artifact read | SHA-256 at review |
|---|---|
| `deliver/design-13-02-probe-input-shape.md` | `a9c8c5c88e5e80df9734470a7552392e8c2caf37855a51b471262cd4859e6cb8` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| `adr-0101-service-backend-health-observed-convergence.md` | `b08bfc53df5364ab02a7a730db77e3e3bb2b9f8e73dabe03712b625fc7d5dfd2` |

I read ADR-0191's accepted producer streak, read-time threshold, and act-once rules; ADR-0055's pre-joined `ServiceAllocFact` correction; ADR-0086's reconciler-owned hydration boundary; ADR-0101's exact three-field `ServiceLifecycleState` and helper signatures; the proposed canonical diff; STEP 13-02 and its pending validation; the current producer, both ObservationStore adapters, hydration, pure reconcile, runtime, and existing Quint evidence. `git status`, `git diff --stat`, the full tracked dirty-file inventory, and the public-declaration scan identified the partial STEP 13-02 fourth `ServiceLifecycleState::probe_rows` field as the only added production public field in the dirty tracked diff. `git diff --check` passed. Other dirty source and test work remains the original agents' work and is neither adopted nor approved by this DESIGN review.

The gap is a real interface-shape blocker, not a proposed concurrency failure. The DISTILL alignment record at `deliver/distill-13-02-legacy-field-alignment.md:8-39` records the invalid fourth State field and the native compile diagnostics during the partial implementation. This review did not rerun Metal, Lima, mutation tests, or the production binary. Its conclusion rests on the accepted interface contract and the current caller path, not on a new simulated schedule.

## Contract and production-path assessment

The production factory returns `AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new())` (`crates/overdrive-control-plane/src/lib.rs:5328-5347`). `run_convergence_tick` calls `hydrate_desired` and `hydrate_actual`, then pure `reconcile`, persists the returned View, and dispatches Actions (`reconciler_runtime.rs:1396-1405,1511-1537,1563-1578,1604-1640`). This is the live entry and owner path relevant to the proposed carrier; `reconcile` cannot make another store read under the ADR-0086 and `development.md` pure-sync boundary.

One per-descriptor probe task is spawned for an allocation and not re-spawned on registration (`crates/overdrive-worker/src/probe_runner/mod.rs:293-360`). Its written `ProbeResultRow` contains `run_seq` and `same_status_runs`, and the task advances its in-memory numbering only after `write_probe_result` succeeds (`:632-658,661-689`). Both stores select the greater `run_seq` per `(alloc_id, role, probe_idx)` (`overdrive-store-local/src/observation_backend.rs:1407-1429`; `overdrive-sim/src/adapters/observation_store.rs:516-534`). The existing `ObservationStore::list_probe_results_for_alloc` contract returns one winner per role/index for the requested allocation and a strict read-after-write view (`overdrive-core/src/traits/observation_store.rs:2211-2249`); the host and Sim implementations perform the allocation-scoped read (`observation_backend.rs:1029-1069`; `observation_store.rs:537-549,890-895`).

`ServiceLifecycleReconciler::hydrate_actual` reaches `hydrate_service_lifecycle_actual`, which calls `list_probe_results_for_alloc` once while building each allocation fact (`crates/overdrive-reconcilers/src/service_lifecycle.rs:669-678,856-897,906-931,936-973`). The current partial implementation places the returned rows in a fourth State field (`:221-243,965-973`) and changes `hydrate_service_alloc_facts` to a tuple return (`:856-868`), contrary to ADR-0101 D2/D3. The accepted `ServiceAllocFact` status fields carry only `Option<ProbeStatus>` and the startup observation stamp (`:112-118,143-154,176-187`); none carries current `run_seq` or `same_status_runs`. The persisted `liveness_last_acted_run` supplies the previous acted sequence only (`:332-338`). ADR-0191's `streak_bugReconstruct` counterexample and the coalescing broker rule exclude recovering the producer streak from reconciliation cadence. Thus zero new carrier surface cannot honestly implement the accepted outcome.

PRCI-1 pins precisely `#[doc(hidden)] pub ServiceAllocFact::probe_rows: BTreeMap<(ProbeRole, ProbeIdx), ProbeResultRow>` (`design-13-02-probe-input-shape.md:57-96`; `feature-delta.md:7008-7034`; ADR-0101 D2). `ProbeResultRow` already aliases `ProbeResultRowV2` with the required status, sequence, streak, and identity (`overdrive-core/src/observation/probe_result_row.rs:422-438,461-467`); `ProbeRole` and `ProbeIdx` are ordered keys. The enclosing fact supplies the allocation ID. The producer row and the three retained status projections can be built from the same allocation-scoped result vector, preserving one probe-row snapshot per fact and the pre-joined allocation boundary without another ObservationStore call. The design explicitly requires identity agreement, absence as absence, all returned rows retained, and index-0-only consumer policy (`design-13-02-probe-input-shape.md:71-84`). It leaves GH #319's other-index policy untouched.

The State remains exactly `allocs`, `service_dataplane`, and `observed_backend_rows`; the existing ADR-0101 `hydrate_service_alloc_facts -> Result<BTreeMap<AllocationId, ServiceAllocFact>, HydrateError>` and four-argument `compute_backend_healthy` signatures are pinned unchanged (`adr-0101:141-170`; proposal `:86-96`). The partial State-level map and tuple-return form are correctly rejected: they violate that explicit shape and split a row from its enclosing allocation fact. The design retains the existing status fields and their types, so established callers still see those projections. The additional struct-literal updates are bounded compiler fallout; a repository-wide `ServiceAllocFact {` search found production hydration and existing source-local, acceptance, integration, and Sim fixtures across the current crates. Neutral fixtures can use an empty map; behavioral fixtures must supply rows consistent with their claimed probe observation. No port, Action, wrapper, persisted schema, or test seam is needed.

## Outcome, error, and ownership checks

| Condition or decision | Approved representation / result | Review disposition |
|---|---|---|
| Latest startup `Fail` and producer streak reaches live `max_attempts` after the existing deadline | Existing `StartupProbeFailed` path using the row's `status` and `same_status_runs` | Preserved; no consumer counter is added. |
| Latest readiness `Pass` reaches the live threshold | Existing backend-eligibility result, subject to the existing startup terminal veto and no-readiness rule | Preserved; no persisted `healthy` output. |
| Latest liveness `Fail` reaches the live threshold on Running allocation, with `N > A` | Existing `StopAllocation`; `ServiceLifecycleView::liveness_last_acted_run` receives `N` only on emit | Preserved; the next newer crossed row can retry a failed stop, while a re-read cannot. |
| `list_probe_results_for_alloc` fails | Existing `HydrateError::ObservationRead(String)` (`service_lifecycle.rs:881-885`; `overdrive-core/src/reconcilers/hydration.rs:103-123`) | The already-approved hydration semantic category is explicit. No new failure distinction or variant is required. |
| No stored row for a key | Absent nested map entry and existing no-row decision | Not an error; no invented default. |
| Row below threshold or of the other status | Existing no-action or ineligible decision | Not an error; no diagnostic string substitutes for typed cause. |

The accepted authority remains one probe-task producer, ObservationStore as latest-row arbiter, ServiceLifecycle as threshold consumer, and WorkloadLifecycle as sole restart authority (ADR-0191 decisions 1-7; proposal `:158-172`). The added field is transient State; the View remains limited to the approved acted-run input and announcement sets (`service_lifecycle.rs:275-339`). The canonical normative diff adds the carrier and strengthens identity/absence language; it does not relax `zero`, `never`, `must`, `exactly`, fail-closed, or required outcomes. The quoted standing user ruling in the proposal covers this outcome-preserving shape completion; this review does not treat reviewer agreement as approval for a new outcome.

## Reuse, subtraction, and required DESIGN gates

| Review dimension | Result |
|---|---|
| Reuse and contract shape | Reuses `ProbeResultRowV2`, the allocation-scoped ObservationStore read, the pre-joined `ServiceAllocFact`, the pure `Reconciler` transition, and the existing View. Hydration is a bounded transient projection; reconciliation stays pure; no new effectful port is exposed. |
| Simpler alternatives | Status/time fields cannot supply `N` or `s`; three full-row replacement fields and six scalar fields enlarge or destabilize the accepted API; a fourth State map violates ADR-0101; persisted verdicts violate read-time policy. The one nested map is the least additional public shape that carries the full approved raw row while retaining every existing field. |
| Subtractive check | The proposal removes the partial fourth State field and tuple-return shape, adds no retry, cache, policy, test lane, model, or extra read, and keeps the existing pending deletion of four legacy View counter/time fields in STEP 13-02. It does not absorb unrelated dirty changes. |
| Lifecycle Gate Ownership | `design-13-02-probe-input-shape.md:174-178` correctly records N/A. PRCI-1 changes no state owner or gate; the startup, readiness, and liveness predicates remain the already-approved ADR-0191 predicates. |
| Formal protocol model | `design-13-02-probe-input-shape.md:180-185` correctly records no new model obligation: the in-process carrier changes no concurrent, ordered, or crash-sensitive transition. `specs/quint/probe-count/checks.toml` already names the relevant runs and invariants, including `NoReactOnSameRow`, `NoMissedTerminate`, and the reconstruction hazard; its evidence summary records them. |
| Quality attributes | One already-read row collection per allocation adds bounded transient memory but no I/O, persistence, or runtime scheduling. Deterministic `BTreeMap` order and existing raw-row observability remain. No security boundary changes. |

## Selected roadmap checks

| Check | Result for PRCI-1 |
|---|---|
| External validity | Pass. STEP 13-02 retains the production `run_convergence_tick`/ServiceLifecycle path, producer/store adapters, and composed acceptance/conformance evidence; PRCI-1 supplies the missing input at that live boundary. |
| AC implementation coupling | No new criterion or step is added. The existing STEP 13-02 criteria name accepted public types and required observable probe outcomes; PRCI-1 pins the interface contract in DESIGN rather than adding an algorithm to AC. |
| Decomposition ratio | One carrier correction within the existing STEP 13-02, with no extra roadmap step or production component. The changed public fact requires bounded existing literal fallout, not a new workstream. |
| Implementation code | No new roadmap code or private algorithm is proposed. The exact public field is a DESIGN interface contract; private threading is left to DELIVER. |
| Concision and precision | The proposed addition is one field with explicit row identity, absence, snapshot, and consumer scope. The existing broader roadmap prose is not rewritten in this bounded review. |
| Test boundary | Existing pure, integration, seeded Sim, and conformance cells remain in their established lanes; PRCI-1 adds no test support API, binary-invoking Rust test, or new validation lane. |

## Findings and disposition

No blocking finding remains. The only observed interface divergence is the already-identified uncommitted fourth `ServiceLifecycleState::probe_rows` field and changed private helper signatures. **Disposition:** PRCI-1 rejects that partial shape and pins the bounded replacement for the STEP 13-02 crafter; this review does not assess that crafter's future implementation or tests as complete. The phrase “Its public constructor” in the proposal at line 89 is imprecise because the State has `Default` and the reconciler has `new`; it does not authorize a constructor and has no contract effect. It may be corrected editorially when the proposal is finalized.

**Final verdict: APPROVED for the exact PRCI-1 DESIGN contract only.** Canonical feature-delta and ADR-0101 text agree on the field, retained State shape, and hydration boundary. Roadmap validation remains pending in the reviewed tree; the orchestrator owns any subsequent validation transition under its normal gate. Implementation and DISTILL alignment must be reviewed separately against this exact contract.
