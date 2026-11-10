# DELIVER review — step 05-05

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-05` — Remove the inferred flag from the row and witness carriers |
| Iteration | 2 — final; iteration 1 is recorded below |
| Implementation commit | `74a30e19101e17cce5da1f3317403ab2a64e015b` |
| COMMIT audit | `a99fd627089d5689f5d5b30c906c44bc681c858f` |
| Prior step | `05-04` approved at iteration 2 |
| Review scope | D16-G1 current layouts for probe-result, allocation-status and allocation-lifecycle-occurrence carriers; wire and CLI removal; default and opt-out Stable witnesses; affected tests and current-layout evidence |
| Working tree before iteration-1 artifact | Clean |

## Verdict

**APPROVED — iteration 2.** The implementation removes the approved fields and preserves the three existing envelope families, tags and aliases. Iteration 1's D1 documentation blocker is closed: the original crafter now distinguishes current variant-tag stability and the V1-to-V2 projection from historical byte compatibility in both source comments and generated OpenAPI. The no-backward-read policy is stated without adding versions, migrations, compatibility readers, or behavior changes.

No production behavior or public API expansion is requested. The review did not rerun tests.

## Iteration history

| Iteration | Verdict | Disposition |
|---|---|---|
| 1 | **NEEDS_REVISION** | D1 found stale backward-read claims in the current source/OpenAPI documentation. Update the descriptions to distinguish retained variant tags and the current V1-to-V2 projection from historical byte compatibility; regenerate OpenAPI. |
| 2 | **APPROVED** | Original crafter commit `c8e43cbd` updates the affected source descriptions and regenerated OpenAPI; D1 is closed. |

## Contract review

### D16-G1 carrier shapes

`ProbeResultRowV1` keeps its five surviving fields in order. `ProbeResultRowV2` keeps `alloc_id`, `probe_idx`, `role`, `status`, `last_observed_at_unix_ms`, `run_seq`, and `same_status_runs` in order. The existing V1 and V2 envelope variants, tags `[0, 1]`, latest aliases, and V1-to-V2 projection remain; the conversion still sets `run_seq = FIRST` and `same_status_runs = ONE` (`crates/overdrive-core/src/observation/probe_result_row.rs:365-455`).

`ProbeWitness` retains `probe_idx`, `role`, and `mechanic_summary` in the approved order. `AllocStatusRowEnvelope::V1` and `AllocLifecycleOccurrenceRowEnvelope::V1` remain under tag 0, with their current aliases and constructors intact. No other public type, variant, error, parameter, wire field or storage envelope was added.

The row JSON projection no longer has an `inferred` field, and the CLI Stable summary prints the witness without the suffix (`crates/overdrive-control-plane/src/api.rs:1193-1230`; `crates/overdrive-cli/src/render.rs:1308-1320`). OpenAPI removes `inferred` from `ProbeResultRowJson` and `ProbeWitness`. The opt-out and default-gate witness constructors no longer populate a flag (`crates/overdrive-reconcilers/src/service_lifecycle.rs:574-629`). The implementation leaves the parser's current TCP startup gate and its Running-plus-Pass Stable behavior intact, as required before D15.

The schema-evolution tests retain the historical `FIXTURE_V*` literals without using them as backward-read gates, and add current-layout bytes for both probe-result tags, an allocation-status row with Stable in both `terminal` and `last_terminated.terminal`, the occurrence row with Stable, and both `StoppedBy` cases (`crates/overdrive-core/tests/schema_evolution/probe_result_row.rs:29-206`; `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs:28-205`; `crates/overdrive-core/tests/schema_evolution/alloc_lifecycle_occurrence_row.rs:23-124`). This matches the five-carrier inventory and D16-G1's current-layout scope. No E13 expectation or website path changed.

### Typed errors and interface shape

This step adds no error condition and changes no typed-error representation or caller mapping. The accepted D14-E2 typed mappings verified in the 05-04 review remain outside this step's changes. The public API change is limited to removal of the approved `inferred` fields.

## Test integrity and Contract Shape Compliance

The diff's test edits match the 05-05 carve-out: field construction/projection edits remove the deleted field; assertions and the CLI test whose sole behavior was rendering the removed `inferred` marker are removed with that behavior; the three carrier groups gain current-layout witness evidence. The other observable assertions remain. Existing `FIXTURE_V*` byte literals are unchanged. Newly added or transitioned schema-evolution bodies carry `/// CONTRACT_SHAPE: pure-function.`; the updated opt-out property retains its `bounded-change` declaration. No test was skipped or broadened to accommodate implementation.

Test modification was detected. The only behavior assertion/test removed is the one whose entire oracle is the deleted `inferred` marker; other assertion changes remove only that field from expected construction or projection. This is within the explicit D16-G1 carve-out. No zero-assertion, tautological, mock-dominated, circular, or always-green test was identified.

There is no pending acceptance test assigned to this step. The implementation did not add a test method; it extended existing schema-evolution test bodies and removed the inferred-only render case. The observable outcomes reviewed are: current V1 and V2 probe-row layouts and the V1 conversion; current allocation-status and occurrence Stable-witness layouts; the two stopped-by cases; stable envelope tags/offsets; row and Stable wire shape; describe and CLI output; opt-out rendering; and the retained default TCP Running-plus-Pass Stable path. Test budget is not exceeded by the step's test additions.

For the test-budget check, I count 22 distinct observable outcomes across the schema, wire/render, opt-out, and retained default-gate criteria; the maximum is 44 test methods. The reported focused selections comprise 42 passing test cases across schema (14), storage (8), wire/render/probe/lifecycle (18), and parser (2). This includes preservation controls from adjacent behavior, and no new test method was added in this step. Parametrized properties count as one method.

### Iteration 1 — TDD phase and quality-gate evidence

| Gate | Result | Evidence and limit |
|---|---|---|
| RED → GREEN → COMMIT | **PASS** | `execution-log.json` records RED FAIL, GREEN PASS, then COMMIT PASS for 05-05, in order. The isolated crafter's report says it reran its own RED before implementation; no inherited phase is claimed. |
| G1–G4 | **PASS / N/A** | No pre-authored ignored acceptance test was assigned; schema fixtures use pure-Rust archive/read paths and add no in-hexagon mocks. |
| G5 | **PASS** | The step retained the existing behavior scenarios and only removed wording/assertions specific to the deleted flag. |
| G6–G7 | **PASS, scoped** | The crafter reported the focused test groups passing: schema (14), storage (8), wire/render/probe/lifecycle (18), and parser (2). Workspace all-target check and workspace Clippy with `-D warnings` also passed as reported. These are not a claim that the full workspace test suite passed. |
| G8 | **PASS** | No new test methods were added; existing schema tests were extended with the required current-layout cases. |
| G9 | **PASS under the explicit carve-out** | The field-removal, inferred-only assertion/test removal, and current-layout evidence fall within the approved D16-G1 carve-out. No unrelated acceptance body was weakened. |
| OpenAPI | **PASS, with D1 documentation finding** | The crafter reports generator-backed regeneration and validation. The resulting schema removes the fields but still contains a historical decode claim described in D1. |
| Mutation testing | **Not run** | Correctly reserved for the single final DELIVER-wave gate. |
| Broad verification | **Not established** | No full workspace test suite or whole-workspace formatting gate is claimed. The crafter reported pre-existing `xtask` format drift; it is outside this bounded step. |

The reviewer inspected the implementation, test, roadmap, execution-log, and generated-schema diffs and ran `git diff --check`, which passed. The reviewer did not rerun tests, Clippy, or OpenAPI generation.

### Iteration 1 — RPP cascade

L1 documentation was scanned and stopped at D1, the stale archived-byte compatibility claim. L2–L6 were not scanned after the L1 finding. The step adds no speculative abstraction or architecture surface.

## Finding D1 — iteration 1 — stale backward-read claims contradict the accepted greenfield policy

**Severity:** Blocker — design/documentation contract contradiction  
**Locations:** `crates/overdrive-core/src/observation/probe_result_row.rs:365-370`; `crates/overdrive-core/src/transition_reason.rs:602-647`; generated `api/openapi.yaml:2913-2945,3023-3033,3053-3063`  
**Iteration 1 disposition:** Open; returned to the original 05-05 crafter for bounded documentation correction and OpenAPI regeneration. **Iteration 2 disposition:** Closed.

The accepted D16-G1 policy keeps the envelope families, tags, aliases and the existing V1-to-V2 projection, but changes the current payload layouts in place. It explicitly says the new build may reject or reinterpret pre-D16 bytes under the same tags and promises no backward-read period (`docs/feature/service-allocation-readiness-and-ending/design/full-review-remediation.md:17-29`).

At iteration 1, source said `ProbeResultRowV1` was retained “for decoding rows written before PROBE-RUN-COUNT” (`probe_result_row.rs:369-370`) even though its same-tag payload layout had changed and those historical bytes were not a supported read contract. The `TerminalCondition::Completed` and `TerminalCondition::Stable` schema descriptions also said “Existing archived rows decode unchanged”; the Stable text specifically said the same `AllocStatusRowEnvelope::V1` shape continued decoding while acknowledging that its fixture was regenerated for the new current layout. The generated OpenAPI carried both claims to API readers. These statements conflated preserved variant discriminants/current conversion behavior with historical-byte compatibility, which the user waived for these pre-D16 layouts.

**Action requested at iteration 1:** revise the affected source comments so they describe only the preserved current envelope tags, variant positions and V1-to-V2 conversion, without promising historical bytes decode; regenerate `api/openapi.yaml` from those source descriptions. Do not add versions, migration readers, old-to-new conversion chains, or compatibility tests; those would contradict the accepted D16-G1 scope. This action was completed and independently rechecked at iteration 2 below.

## Verification and commit audit

- The implementation and audit commits preserve the existing Git author and each carries exactly `Co-Authored-By: Codex <codex@openai.com>`; no Claude/Anthropic attribution is present.
- The implementation diff is bounded to the D16 row/witness removal, direct serialization/rendering fallout, current-layout evidence, and test construction/assertion updates. `verification/expectations/E13-vm-service-inferred-tcp-startup/` and `website/` are untouched.
- `execution-log.json` records this step's RED FAIL, GREEN PASS and COMMIT PASS sequence; the separate audit commit appends the COMMIT result.
- The worktree was clean before writing this review artifact. No production code, tests, DES events, mutation exclusions, or commits were changed by the reviewer.

## Iteration 2 — D1 re-review

The original crafter's remediation commit `c8e43cbd7282a8f349394bd814b31a71193a636b` changes only `probe_result_row.rs`, `transition_reason.rs`, and generated `api/openapi.yaml`. `ProbeResultRowV1` is now described as the current V1 payload with its existing V1-to-V2 projection; the comment explicitly declines a read promise for bytes archived with the pre-D16 V1 layout. The `TerminalCondition::Completed` comment describes retained variant tags in the current declaration. The Stable comments now state that D16-G1 rebaselines the current allocation-status and lifecycle-occurrence layouts under existing tags and that pre-D16 bytes have no backward-read guarantee. The OpenAPI descriptions match those source comments.

The remaining `StoppedBy` note that adding a fieldless variant preserves its enum's existing archived representation is outside this finding: D16-G1 changes `ProbeWitness` and the affected containing carriers, not the `StoppedBy` enum. No compatibility migration, new version, test, or runtime behavior was introduced by remediation.

| Re-review check | Result |
|---|---|
| D1 source claims reconciled with accepted D16-G1 | **PASS** |
| Generated OpenAPI reflects corrected source descriptions | **PASS** (generator-backed gate reported by crafter; diff matches source) |
| Scope limited to docs and generated schema | **PASS** — three files only |
| Author and attribution | **PASS** — existing author retained; exact Codex co-author trailer |
| Verification | Scoped format, OpenAPI gate, and `git diff --check` passed as reported; no tests rerun because this remediation changes only documentation/schema descriptions |

**Iteration 2 verdict: APPROVED.** D1 is closed. The current-layout evidence, implementation review, focused verification limits, and mutation-gate disposition from iteration 1 remain unchanged. No further remediation is required for step 05-05.
