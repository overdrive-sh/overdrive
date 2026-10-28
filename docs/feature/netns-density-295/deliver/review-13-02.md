# DELIVER review — STEP 13-02 PROBE-RUN-COUNT

## Review metadata

| Field | Value |
|---|---|
| Feature / step | netns-density-295 / 13-02 |
| Review ID | STEP13-02-DELIVER-POSTCOMMIT-01 |
| Review date | 2026-10-28 |
| Reviewer | Fresh isolated DELIVER implementation reviewer |
| Iteration | 1 |
| Implementation range | 80f97acfd..60fd13746a9bf1e0f895f4063b0761c24a37de7c |
| DES range | f14a1e8ace0223cd8ad6fb3b3e160c76a82181c1 |
| Final verdict | APPROVED |
| Open findings | 0 blocker; 0 critical; 0 high; 0 medium |

The review applied the repository AGENTS.md, applicable .claude/rules for
development, Rust, testing, verification, and review, nw-review-workflow,
nw-sc-review-dimensions, and nw-quality-framework. Repository-specific
native-Markdown and DELIVER rules take precedence over the installed review
skill's legacy YAML output and iteration defaults.

Only this review artifact was written. No production source, test body, design,
roadmap, execution log, model, lane, host state, or commit was changed. The
pre-existing modified AGENTS.md remains untouched.

## Review boundary and authoritative artifacts

The accepted contract is the PROBE-RUN-COUNT section of the feature delta,
ADR-0191 and the approved PRCI-1 input-shape decision. The exact carrier is
also recorded in ADR-0101 and the independently approved DESIGN and DISTILL
reviews. Their current hashes are:

| Artifact | SHA-256 |
|---|---|
| deliver/design-13-02-probe-input-shape.md | 77e121a1d4eb6df177b88b32a0415ce246badc1ea2f3ad03e086a752ec5d42e2 |
| deliver/review-design-13-02-probe-input-shape.md | 712a9bff6c66364d35dc152781a1ba9b79705b6b7a84c93b0c04da832e72f657 |
| deliver/distill-13-02-legacy-field-alignment.md | 59a34637ddfc347ffdda73f78abe394d93f849ddcd913ed4fc6720a8d383825a |
| deliver/review-distill-13-02-legacy-field-alignment.md | 1b93d1ed4e8b7abead7a2231dea9ca60712761173c975e25ebab2a9e671baf06 |
| feature-delta.md | 66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09 |
| adr-0101-service-backend-health-observed-convergence.md | 300747e7f10269f5b819702498d628cb80246a39b1d4ef99905c85cb00397d70 |

The implementation commit retains the Marcus author and has exactly one
Co-Authored-By: Codex <codex@openai.com> trailer. The DES-only commit records
the same author/trailer and changes only deliver/execution-log.json.

## Exact contract assessment

The implementation has exactly the approved public carrier at
crates/overdrive-reconcilers/src/service_lifecycle.rs:207-213:

    #[doc(hidden)]
    pub probe_rows: BTreeMap<(ProbeRole, ProbeIdx), ProbeResultRow>,

ServiceLifecycleState remains exactly the three ADR-0101 fields at
service_lifecycle.rs:228-243: allocs, service_dataplane, and
observed_backend_rows. The existing hydration return and
compute_backend_healthy four-argument shape remain unchanged. There is no
fourth state field, tuple-return replacement, accessor, wrapper, action, port,
observation API, new scenario, new lane, or new model.

The public ProbeRunSeq, SameStatusRuns, V2 row, aliases, and parse-error
representations were already declared by the accepted feature contract and
the preceding scaffold. This step completes their bodies; it does not invent
public surface. The implementations at
crates/overdrive-core/src/observation/probe_result_row.rs:149-276 preserve
the exact zero/malformed typed errors, validated u64/u32 serde shapes,
decimal Display/FromStr, and saturating successors.

The V1 conversion at probe_result_row.rs:438-455 copies every V1 field and
sets exactly ProbeRunSeq::FIRST and SameStatusRuns::ONE. The envelope
continues to append V2 as discriminant 1 and maps V1 through the approved
conversion. No wire or API JSON field is added.

## Production path and ownership review

The producer keeps run state private to each supervised probe task. At
crates/overdrive-worker/src/probe_runner/mod.rs:632-658, status, run sequence,
and same-status streak are computed before the write, but the task records the
state only after write_probe_result succeeds. A pass/fail status comparison
uses the ProbeStatus variant at :677-693, so changing only a failure detail
continues the streak as required. Capacity, quiescence, adapter errors, and
failed writes do not consume a run number or break the producer streak.

The no-row boundary is explicit at mod.rs:619-655 and the event fields at
:704-729 match ADR-0192 exactly: alloc, role, probe_idx, reason, and detail
only for adapter_error and write_failed. Every such run emits one
probe.not_sampled event at warning level and writes no row. Other typed guest
dial failures become ordinary Fail rows, as ADR-0187 requires.

Both observation adapters compare only run_seq: the Sim merge is at
crates/overdrive-sim/src/adapters/observation_store.rs:516-534, and the local
redb merge is at crates/overdrive-store-local/src/observation_backend.rs:1407-1429.
Equal or smaller rows are silent; a greater row wins even when its observation
time is earlier. The local event is emitted only after commit at
observation_backend.rs:1012-1026; the Sim adapter applies its injected write
failure before mutation and emits only for an accepted merge. The existing
malformed-prior-row replacement policy remains intact.

Hydration reads the allocation's probe rows once and builds both projections
from that vector at service_lifecycle.rs:842-920. Every returned row is kept
in the fact-local map, the index-0 consumer projections use the same selected
rows, and absent rows remain absent. The pure reconciler then consumes the raw
inputs at the existing decision boundaries:

| Consumer | Evidence | Contract result |
|---|---|---|
| Startup | service_lifecycle.rs:603-621 | Uses Startup/index-0 Fail.same_status_runs; missing row consumes zero attempts. |
| Readiness | service_lifecycle.rs:1120-1155 | Uses Readiness/index-0 status and producer streak against the live threshold; no row is ineligible. |
| Liveness | service_lifecycle.rs:974-1025 | Uses Liveness/index-0 status, streak, and run_seq; records liveness_last_acted_run only when the existing stop action is emitted. |

The liveness comparison is strictly newer than the acted row, so the same row
cannot re-emit and a newer crossed row can retry a failed stop while the
allocation remains Running. WorkloadLifecycle remains the restart authority;
the implementation adds no restart or budget path.

## Typed error contract gate

No new semantic failure condition is introduced. The implementation preserves
the following exact mappings:

| Failure or condition | Implemented representation |
|---|---|
| Probe-row hydration read failure | Existing HydrateError::ObservationRead(String) at service_lifecycle.rs:872-876 and :943-947. |
| Prober adapter failure | Existing ProbeRunnerError::ProbeAdapterFailed { alloc_id, probe_idx, source: ProbeFailure }; the same typed cause is diagnostic detail on the no-row event. |
| Observation write failure | Existing ProbeRunnerError::ObservationWriteFailed { alloc_id, probe_idx, source: ObservationStoreError }; the typed error is returned and the event carries its diagnostic display. |
| Capacity or quiescence refusal | No error and no row, with the exact node_capacity or quiesced event reason. |
| Zero or malformed run/streak input | Existing ProbeRunSeqParseError::{Zero, Malformed} and SameStatusRunsParseError::{Zero, Malformed}. |
| Malformed stored envelope | Existing observation-layer decode/log-and-skip or replacement behavior; no new error taxonomy. |

No diagnostic string substitutes for a semantic error. The approved outer
ObservationRead(String) mapping remains the only hydration boundary error,
as PRCI-1 explicitly permits.

## Test and acceptance integrity

The DISTILL iteration-2 review is independently approved. Its raw-row fixture
alignment is present: retained status/timestamp projections are paired with
the exact role/index rows, missing rows remain absent, and the hydration text
golden contains the additive fact-local row. The probe-count fixture now starts
the same shared owners and opens the existing EXEC gate before constructing the
composition, removing unrelated fail-closed boot cleanup from the fresh-run
oracle. No production seam or private-state assertion was added.

The only deleted executable bodies are the private startup-counter property,
the removed liveness recovery-counter property, and the missing-row terminal
case whose premise depended on the deleted View counter. The latter cannot be
retained under the approved rule that an absent row consumes no attempt. The
remaining startup deadline/reason, readiness threshold, liveness threshold,
same-row act-once, newer-row retry, non-Running suppression, restart-authority,
View write-through, reclamation, V1/V2, merge, and no-row assertions remain.
The activated acceptance and Quint bodies use the production composition and
the existing Sim/real store boundaries; they do not spawn the product binary,
duplicate the implementation, or use expectation-runner behavior.

The V1 fixture is unchanged: the 176-character hex constant is 88 bytes, and
its ASCII-hex SHA-256 is
68e7554e15013200c4c5ca7856ccb6721c544155b62d6157fe4a3d41029f2096.
The schema test now passes without regenerating that fixture.

## Verification evidence

The crafter's retained durable receipts and the current review handoff report:

| Gate | Result / receipt |
|---|---|
| Normal CH-filtered union | Accepted seed 94db98ed; durable run d29f875adec94fb4bde6844f; nextest 18c59d4d-09ef-4457-9c85-c9b69bda70f1; 2162/2162 passed, 167 skipped. |
| Probe-count production suite | Seed 6152749d50ab30864e0ff41b; nextest 9ab7995c-e822-4055-95ed-4a427260d1e0; 22/22 passed, including 100 randomized traces. |
| Schema suite | Durable run cd63ca268481ebd9a8c09a73; nextest 2f2b9156-b586-4432-81d3-1160da1e3ddf; 6/6 passed. |
| Workspace check | bd57b3d550a335ca7f3f7fd9 — pass. |
| Workspace clippy | 81954095166c2a2c34055cc2 — pass. |
| Formatting | 8d6dfe19391df2dd8d59f087 — pass. |
| DST validation | a438d297b8e017f1f9fff4c — pass. |
| Conformance lint | d2faf590cc71de516d736d46 — pass. |
| Review-local checks | git diff --check 80f97acfd..HEAD and JSON parsing of roadmap/execution log — pass. |

The retained formal guest-flow-owner run 279 is unchanged and was not rerun
because this step does not affect its inputs. The known quiescence seed
651125aa is the unchanged parent-baseline counterexample recorded by the
STEP 13-01 review, not a STEP 13-02 regression. No mutation testing was run;
the repository reserves that gate for the final DELIVER wave.

## DES phase-history audit

The execution log preserves two RED attempts before GREEN and COMMIT at
docs/feature/netns-density-295/deliver/execution-log.json:1566-1592.
Their evidence has distinct meanings:

| Timestamp | Correct classification |
|---|---|
| 2026-10-27T23:16:04Z | Compiler-only legacy-field fallout from the old test fields. This is not a semantic RED and is not counted as proof that the pre-change behavior failed. The generic historical FAIL entry is preserved rather than rewritten. |
| 2026-10-28T00:19:15Z | Honest semantic RED against isolated baseline 80f97acfd with only the reasoned marker activation: the worker property supplied Capacity, expected one probe.not_sampled, observed zero; durable baseline 7a96aed53937ce3baa7ed5f0 exited 100. |
| 2026-10-28T02:32:56Z | GREEN passed after the bounded producer/store/reconciler and fixture work. |
| 2026-10-28T02:35:13Z | COMMIT passed in DES commit f14a1e8ace0223cd8ad6fb3b3e160c76a82181c1. |

No invented semantic test, altered scenario oracle, hidden expectation runner,
or unrecorded phase claim is needed to explain the final green result.

## Review dimensions and findings

| Dimension | Result | Evidence |
|---|---|---|
| Exact API / contract shape | PASS | One hidden fact-local map; exact three-field State; helper return/arity preserved; no extra public surface. |
| Typed error representation | PASS | Every designed failure maps to an existing typed variant or the explicitly non-error no-row outcome. |
| Implementation bias / scope | PASS | No new owner, persistence, retry protocol, cancellation mechanism, model, lane, or consumer API. |
| Producer correctness | PASS | State advances only after an accepted write; no-row outcomes preserve run/streak state. |
| Merge and persistence | PASS | Both adapters use strict greater run_seq; failure is atomic and event-after-commit. |
| Hydration and consumer completeness | PASS | All rows are carried; role/index 0 consumers use producer streak and run sequence at read time. |
| Test integrity | PASS | Existing production composition, Sim/real stores, Quint traces, and preserved complements; no theater or assertion weakening. |
| Environment / boundary realism | PASS | Tier-1 production composition and real local store evidence are retained; no unnecessary Tier-3 rerun was introduced. |
| DES honesty | PASS | Compiler-only and semantic RED evidence are distinguished, with original history preserved. |

No blocking or nonblocking review finding remains. One editorial observation,
which is not a finding, is recorded without remediation:
crates/overdrive-worker/src/probe_runner/run_numbering_tests.rs:19-20
still says the run-counting cells are pending DELIVER step 9 even though this
step activated them. It has no executable, API, acceptance-oracle, or scope
effect and does not condition approval.

## Iteration history and verdict

### Iteration 1 — 2026-10-28

The fresh postcommit review covered the complete implementation range, the
DES-only commit, the approved DESIGN/DISTILL artifacts, the exact public and
typed-error contracts, changed tests and deletions, and all supplied durable
verification receipts. No remediation was requested.

**Verdict: APPROVED.** STEP 13-02 satisfies the approved PRCI-1 shape and the
PROBE-RUN-COUNT outcomes without weakening retained tests or inventing API
surface. The step may advance to the next roadmap step after the orchestrator
records this artifact.

# APPROVED
