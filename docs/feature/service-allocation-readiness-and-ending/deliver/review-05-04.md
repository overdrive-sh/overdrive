# DELIVER review — step 05-04

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-04` — Remove the inferred flag from the probe descriptor carriers |
| Iteration | 2 — final; iterations 1–2 are recorded below |
| Implementation commit | `8545391f4567811d19ab0427600c5d1a023a2fef` |
| COMMIT audit | `c3057beca1484c45685a2a38f19c7fc3de848560` |
| D1 remediation commits | `ebb7080ab82556f4a561a8282581fdeaa415a984`; COMMIT audit `44d6d5a2eae0124e2412176f112ec95425fdc5d7` |
| Prior step | `05-03` approved at iteration 1 |
| Review scope | D16-G1 descriptor/Service-intent/Service-spec current layouts; D14-E2 typed admission and caller mappings; retained TCP default gate; changed acceptance, schema and fixture evidence |
| Working tree before iteration 1 artifact | Clean |

## Final verdict

**APPROVED — iteration 2.** The 05-04 implementation matches the approved D16-G1 and D14-E2 interface contracts. Iteration 1's D1 test-contract blocker is resolved: DISTILL independently approved removal of the impossible D9 input oracle, and the original crafter reran the focused GREEN selection without that exclusion before recording COMMIT PASS. No production behavior defect, API-shape divergence, typed-error gap, unauthorized compatibility expansion, or disallowed fixture rewrite remains.

The focused verification is not a full workspace suite or final mutation gate. The final verdict is for step 05-04's accepted scope and the required step verification.

## Iteration history

| Iteration | Verdict | Disposition |
|---|---|---|
| 1 | **NEEDS_REVISION** | D1 found an active failing test whose synthetic pre-Stable liveness row contradicted the accepted D9 production owner order. Routed the test-contract correction to DISTILL; no implementation change requested. |
| 2 | **APPROVED** | DISTILL correction was independently approved; the obsolete test and its placement entry were removed while reachable Stable, startup-stop, and post-Stable liveness outcomes remain covered. The original crafter reran GREEN and COMMIT. |

## Contract and production-path review

### D16-G1 current layouts and scope

`ProbeDescriptor` loses only `inferred`; its remaining fields, `ProbeMechanic` order, and the existing `GuestListeners` variant remain intact (`crates/overdrive-core/src/aggregate/probe_descriptor.rs:127-159`). The parser-side Service spec and Service intent continue using their existing V1 envelope families and tag 0. The two new current-layout roundtrip fixtures contain a sole `GuestListeners` startup descriptor and assert the current archive bytes and decoded value (`crates/overdrive-core/tests/schema_evolution/service_spec.rs:43-67`; `crates/overdrive-core/tests/schema_evolution/workload_intent.rs:55-75,104-119`). The existing `FIXTURE_V*` literals were not rewritten; no new version, historical nested type, migration chain, or raw-Service provenance restriction was introduced.

The implementation keeps the working TCP startup synthesis while removing its provenance bit. An omitted startup stanza still synthesizes a TCP probe against the first listener; an explicit empty startup array still yields no startup probe (`crates/overdrive-core/src/aggregate/workload_spec.rs:1178-1220`; `crates/overdrive-core/tests/acceptance/health_check_toml_parse.rs:412-497`). `ServiceLifecycle` still announces Stable for a Running allocation with a startup Pass and retains the explicit empty-list opt-out branch (`crates/overdrive-reconcilers/src/service_lifecycle.rs:578-629`; `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs:162-190`). The Service wire descriptor, CLI `ProbeRenderRow`, and probe section renderer no longer carry or print the suffix; OpenAPI no longer marks `ProbeDescriptor.inferred` required. `ProbeCountExceeded` no longer describes the synthesized probe as inferred.

The two-step rollout is the approved bounded landing in D16-G1: this commit rebaselines the Service intent/spec carriers; 05-05 owns the probe-result and witness carriers. The temporary remaining row/witness `inferred` fields are not treated as a new long-term API, and no replacement field was added. `verification/expectations/E13-vm-service-inferred-tcp-startup/` and `website/` are untouched.

### D14-E2 typed error mapping

The implemented semantic failures map to the exact approved variants and caller surfaces:

| Designed semantic failure | Approved and implemented typed representation | Caller-visible mapping |
|---|---|---|
| Raw Service has `GuestListeners` in Readiness/Liveness, or its Startup descriptor has a non-Startup descriptor role | `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }`; checked in Startup, Readiness, Liveness vector order before sole-startup and probe-count checks (`aggregate/mod.rs:456-463,635-660`) | `submit_workload` retains `ControlPlaneError::Aggregate`; `to_response` returns HTTP 400, `error = "validation"`, and the exact `field` path (`control-plane/src/error.rs:994-1022`). |
| Correctly role-tagged `GuestListeners` shares the Service Startup vector with another descriptor | `AggregateError::GuestListenersNotSoleStartup { count }`, after role validation and before general probe-count validation (`aggregate/mod.rs:662-668`) | Same typed aggregate chain; HTTP 400 `validation` with `field = "startup_probes"`. |
| Service TOML contains canonical `guest_listeners`, case-insensitive, in a probe role | `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }`; parser reports the first role/index and does not add a noncanonical alias (`workload_spec.rs:1263-1303,1355-1358,1452-1455`) | Both `deploy` and `deploy_streaming` preserve `CliError::ParseError(source)`; exit code remains 1 (`cli/src/commands/deploy.rs:223-239,523-539`). |
| Job/Schedule TOML contains a probe block | Existing `ParseError::ProbesNotAllowedOnKind` remains the earlier kind gate | No D14-E2 variant is manufactured. Per-kind raw Job/Schedule submit payloads do not expose Service probe vectors. |

The activated core tests assert exact variant payloads, vector/index ordering, sole-startup count and precedence, canonical case-insensitive parsing, and noncanonical spelling behavior. The HTTP test drives the actual Service submit handler and checks status, body fields, and intent-store non-mutation; it also checks the exact core variants. The CLI test drives both deploy handlers and checks the nested parser error payload and exit code. These boundaries satisfy D14-E2 without flattening the semantic cause into a string.

### Test integrity and Contract Shape Compliance

The changed test bodies fall within the accepted D16-G1 carve-out: field-construction edits, removal of assertions that specifically pinned the deleted `inferred` behavior, replacement of the affected historical-byte decode assertions with current-layout roundtrips, and deletion of the CLI test whose only behavior was rendering the removed `(inferred)` suffix. The default TCP synthesis, explicit-empty opt-out, Stable decision, current descriptor values, typed rejection payloads, and HTTP/CLI outcomes remain asserted. No unrelated assertion was weakened, test skipped, or fixture literal rewritten.

Every newly activated D14-E2 body has a per-test Contract Shape declaration. The two new current-layout archive tests use `/// CONTRACT_SHAPE: pure-function.`; source-local pure properties use the same exact declaration. The activated HTTP and CLI tests declare `bounded-change`. None of the new test names matches the banned `test_*` naming pattern. There is no DISCUSS Elevator Pitch artifact for this feature, so an outcome-anchor check is not applicable to these pre-authored scenarios.

I count ten observable outcomes for the step: two current-layout archive shapes, default TCP synthesis, explicit-empty opt-out, raw-Service role rejection, raw-Service cardinality rejection, valid raw-Service admission, operator-TOML rejection, HTTP mapping, and CLI mapping. The generic `2 × behavior` budget is 20 methods; the 12 step-relevant test bodies, including retained controls and the current-layout tests, fit within it. Parametrized loops count once. This is not a recount of all fixture tests mechanically touched across the workspace.

## Finding D1 (iteration 1) — stale D9 same-tick liveness test blocked the green gate

**Severity:** Blocker — acceptance-test contract / verification gate  
**Location:** `crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs:1649-1707`  
**Disposition:** Route to DISTILL for test-contract reconciliation; no 05-04 production change is requested.

The crafter reported that `stable_this_tick_suppresses_liveness_restart_same_tick` failed and was excluded from the passing focused selection. It fabricates `stable_committed = false`, a startup Pass, and a liveness Fail row with a crossed streak in the same reconcile input, then requires only a Stable action. The 05-04 production diff does not change the liveness branch that this test exercises.

The accepted D9 owner order makes that fixture unreachable. `ProbeRunner::start_alloc` retains Liveness descriptors instead of spawning their tasks (`crates/overdrive-worker/src/probe_runner/mod.rs:335-357`); `VmDriver::on_alloc_stable` is the owner that calls `start_liveness` (`crates/overdrive-worker/src/vm_driver.rs:2111-2114`); and the production action shim writes the Stable lifecycle row before calling `on_alloc_stable` (`crates/overdrive-control-plane/src/action_shim/mod.rs:2181-2189`). The runner then spawns the retained task from `start_liveness` (`probe_runner/mod.rs:396-414`). ADR-0261 says the liveness streak starts at Stable and explicitly records the same-tick skip as unreachable and removable (`docs/product/architecture/adr-0261-liveness-probing-begins-when-the-service-is-stable.md:67-72`). The failing direct reconciler fixture therefore does not reproduce a reachable production defect.

This is not evidence that the implementation needs a same-tick suppression mechanism. The pre-authored test contradicts the approved D9 contract and should be reconciled by DISTILL against the already accepted reachable S-32 trajectory. Since it was excluded only after failing, the reported focused results do not establish G6/G7. Do not call the full suite green, and do not expand 05-04 into liveness implementation work.

## Verification evidence

I reviewed the implementation and COMMIT-audit diffs without rerunning tests.

| Evidence | Result | Source / limitation |
|---|---:|---|
| 05-04 RED → GREEN → COMMIT | Original sequence: RED fail, GREEN pass twice, COMMIT pass. After the DISTILL correction: GREEN pass and COMMIT pass. | `deliver/execution-log.json`; the remediation COMMIT audit `44d6d5a2` contains only the new COMMIT event. |
| Focused selection | Iteration 1 reported 173 passing core/control-plane/CLI tests but excluded the failing D9 test. Iteration 2 reported **182/182 passed** with no exclusion. | Original crafter report; iteration 2 included the retained core/composed D9 controls and OpenAPI gate. |
| Focused worker selection | 18 passed were reported | Crafter report. |
| Workspace all-target check and focused Clippy | Passed as reported | Not independently rerun. |
| Changed-file format, JSON and `git diff --check` | Passed as reported / checked | The working tree was clean before this artifact. |
| OpenAPI validation | Generator-backed live schema comparison passed against updated `api/openapi.yaml`; the iteration-2 OpenAPI gate also passed. The direct metal `openapi-gen` invocation did not run because it failed to acquire its lease. | Do not report the direct generator command as passed. |
| Full-workspace test, Clippy, formatting | Not established | The focused validation omitted the failed test. The crafter reported a BPF `no_std` unwind diagnostic from full-workspace Clippy and existing `xtask` formatting drift; I did not rerun or classify those as baseline/unrelated. |
| Mutation testing | Not run | Correctly reserved for the final DELIVER-wave gate. |
| Commit attribution and scope | Pass | The 05-04 implementation/audit and D1 remediation/audit commits retain the existing Git author and carry exactly `Co-Authored-By: Codex <codex@openai.com>`; no E13 or website path changed. |

## Iteration 1 disposition

The D16-G1 and D14-E2 implementation is otherwise acceptable. D1 remains a blocker to approval because the active acceptance-test inventory conflicts with the accepted D9 contract and a failing test was excluded from the reported GREEN selection. Complete the bounded DISTILL reconciliation, rerun the required green verification, and submit this step for reviewer re-evaluation. No production code, new API, or broader liveness mechanism is requested by this review.

## Iteration 2 — D1 re-review

The DISTILL remediation at `docs/feature/service-allocation-readiness-and-ending/distill/liveness-stable-tick-oracle-remediation.md` removes only `stable_this_tick_suppresses_liveness_restart_same_tick` and its `host-pure` placement entry. Its independent review artifact, `distill/review-liveness-stable-tick-oracle-remediation.md`, approves that bounded correction and confirms the production owner path makes the test's pre-Stable liveness row unreachable. The test's reachable outcomes remain covered: Running plus startup Pass announces Stable; no liveness task or row exists before Stable and the first liveness row follows the committed Stable hook; a crossed post-Stable liveness streak stops the allocation without re-announcing Stable; and a startup stop still suppresses a competing pure-input liveness stop. No production/API code, test seam, model, test threshold, or reachable assertion changed.

The original crafter reran the focused GREEN selection without the removed test and reported **182/182 passed**, including the retained core/composed D9 controls and OpenAPI gate. The two named remediation control logs also record four core controls and two composed controls passing. `execution-log.json` appends GREEN PASS and COMMIT PASS after the original step sequence; no new RED event is claimed for the test-only DISTILL correction. Mutation testing remains reserved for the final DELIVER-wave gate. I reviewed the remediation diff and artifacts without rerunning broad tests.

**Iteration 2 disposition: APPROVED.** D1 is closed. The step's exact D16-G1/D14-E2 contract mappings and verification limits recorded above remain valid; no further remediation is required for 05-04.
