# Independent DISTILL review — service allocation readiness and ending

## Metadata and verdict

| Item | Value |
|---|---|
| Review | Fresh, independent full-suite review, 2026-11-09 |
| Authority | Approved DESIGN iteration 4 in `../design/review.md`; exact D11-E1, D14-A1, D14-E2, D13-S1, D17-S1 and D16-G1 contracts in `../feature-delta.md` |
| Material reviewed | D1–D17 scenario inventory, Rust acceptance bodies and RED scaffolds, six Quint conformance drivers, `red-classification.md`, `upstream-issues.md`, `contract-remediation-audit.md`, and the 17-step roadmap handoff |
| Historical review | The earlier Sentinel round-1 `needs_revision` is recorded in the feature delta. This artifact does not convert that historical report into an approval. |
| Current verdict | **APPROVED at iteration 4.** F-01 is resolved; F-02 and F-03 were withdrawn in iteration 3. |
| Downstream status | The full DISTILL gate is approved. `deliver/roadmap.json` validation remains `pending` for its own independent review; this verdict alone does not authorize a DELIVER step. |

## Iteration 1 — historical disposition

The feature delta records the prior round-1 findings and their subsequent remediation. The restart driver now exercises hydration, reconcile and shim dispatch; the liveness and hook compositions have executable bodies; DG-1–DG-13 and the minor DESIGN gaps have rulings. The current independent review accepts those dispositions as context, then reviews the revised suite against the approved iteration-4 contract. It does not claim an earlier full DISTILL approval.

## Iteration 2 — initial full-suite review

### What the revised suite proves and how it is placed

The scenario inventory maps the designed S-1–S-56 obligations and D14-E2's S-57 to Rust bodies. Composed acceptance tests drive the production observer, reconcilers, action shim, probe runner, submit handler and stream through their entry points, with `Sim*` implementations at driven ports. Pure reconciler and derivation properties supply input-space breadth. The six Quint drivers use `quint-connect` traces and production-owner projections; the guest half of `guest_report` and real kernel effects have separate Tier-3 bodies. The no-test-binary boundary is retained: the feature's Rust tests do not spawn the built `overdrive` binary or invoke a test runner as their SUT.

The revised typed-error evidence is particularly concrete:

| Designed semantic failure | Approved interface representation and mapping | Executable assertion / limit |
|---|---|---|
| Open read or `Lagged` relist returns `ObservationStoreError::{Unreachable, Io, Envelope}` | `ServiceStreamError::ObservationRead { source: original_error }`, once, then end | Both S-42 bodies in `service_stream_recorded_verdict.rs` enumerate all three nested causes, inspect the payload and `Error::source`, require immediate `Err`, permanent end and no later `Timeout`; the store snapshot is unchanged. |
| `emit_service_line` fails | `ServiceStreamError::LineEmission { source: original_io_error }` | The exact variant and source chain are compile-pinned in the approved scaffold. Ordinary in-memory line emission supplies no reachable injected failure; no emitter seam is justified. This is a stated evidence limit, not an untyped fallback. |
| Raw Service `GuestListeners` is in the wrong vector or has a wrong descriptor role | `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }` → `ControlPlaneError::Aggregate` → HTTP 400 `validation` with exact vector/index field | `guest_listeners_input_rejection.rs` varies vector, descriptor role and position, including a later invalid value; `guest_listeners_http_rejection.rs` drives the production submit handler, checks HTTP field and no intent-store delta, then checks the nested cause and response mapper. Other error variants fail the assertions. |
| Correctly tagged raw Service `GuestListeners` is not sole Startup | `AggregateError::GuestListenersNotSoleStartup { count }`, before `ProbeCountExceeded`; HTTP field `startup_probes` | Core cases cover counts 2, 7, 8 and 9; HTTP covers count 2. The sole-startup and zero-descriptor controls are admitted. |
| Canonical case-insensitive Service TOML token appears in any role | `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }` → `CliError::ParseError`, exit 1 before HTTP | Core parser covers all roles, two token cases, index 1 and first-role precedence; the CLI body covers all roles and cases in both deploy modes with an invalid config path as a pre-HTTP litmus. The noncanonical no-underscore spelling remains `UnknownProbeType`. |
| Job/Schedule TOML contains a probe block; raw Job/Schedule JSON adds a probe field | Existing `ProbesNotAllowedOnKind` for TOML; raw payload is rejected structurally by `deny_unknown_fields` before Service admission | Existing CLI/core kind tests exercise the TOML gate. The raw per-kind payloads have no probe vectors. A fabricated raw Job/Schedule `GuestListeners` descriptor would be an impossible fixture and is not required. |

The S-46 → 02-04, S-34 → 04-01, S-42 → 04-02, S-55 → 05-01 and S-57 → 05-04 activations follow their production dependencies. S-57 belongs in 05-04, where the aggregate/parser variants and field mapping are implemented; the 05-07 default-gate flip is later. D16-G1 uses the user's explicit greenfield waiver: the five affected current carrier layouts are rebaselined under their existing tags, with current-layout evidence, and no historical-byte or backward-read requirement. The roadmap retains one final mutation gate after all 17 approved step reviews, a minimum 80% kill rate and the D17 signal-position/stored-permit waiver evidence.

### Blocking finding F-01 — per-test Contract Shape declarations remain missing

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`. **Severity:** blocker.

The workspace rule in `AGENTS.md` requires a per-test Contract Shape declaration on every new or transitioned test and the exact `/// CONTRACT_SHAPE: pure-function.` rustdoc line on each live source-local pure property. The **156** identities counted here are existing, pre-authored `#[ignore = "pending DELIVER: ..."]` bodies scheduled for activation in the 17-step roadmap; they have **not yet transitioned**. The roadmap's test paradigm says each crafter removes only the pending marker and must not complete or edit an acceptance body. Thus the declaration must already be present for the planned transition. A mechanical scan of the current tree found **122** of those 156 lack `/// CONTRACT_SHAPE:` in their own doc/attribute block. Module-level `//!` declarations do not satisfy the per-test rule. All **46** pending Quint conformance identities are among the 122; the other **76** are scheduled acceptance/composed/Tier-3 identities. The recent S-46 property (`service_lifecycle_startup_stop.rs:381`), S-34 extracted-deadline property (`startup_deadline.rs:69`) and S-55 property (`declared_listeners_tcp_view.rs:51`) were fixed, as were all six new S-57 test identities and both amended S-42 identities; the remaining omissions are outside that bounded audit.

Concrete pure-property examples are `progress_deadline_derivation.rs:60` (S-34), `service_lifecycle_startup_stop.rs:217,299` (S-21/S-29), `workload_lifecycle_ceiling_classification.rs:429` (S-15/S-23) and `beacon_listen_wire_form.rs:22,39` (D13). Concrete composed/conformance examples are `convergence_loop_wake.rs:174` (S-48), `guest_listener_probe_rows.rs:154` (S-53), and `quint_stream_conformance/stream_snapshot.rs:501` (D11). Each has a scenario description or a module-level classification, but no per-test declaration immediately before its test attributes. This is the same class of defect the prior roadmap review found for three bodies; the current rule applies to the whole pending suite.

**Required DISTILL remediation:** add the per-test declaration to every affected pending body, with the exact rustdoc line for pure properties. Preserve bodies, assertions, ignore markers, and scenario activation. Re-run the mechanical scan over all 156 pending identities, not just the previously named three. This is documentation on tests, not a change to production behavior or public API.

The complete location inventory is reproducible without altering tests:

```sh
python3 - <<'PY'
from pathlib import Path
total = missing = 0
for root in ('crates/overdrive-core', 'crates/overdrive-control-plane',
             'crates/overdrive-cli', 'crates/overdrive-worker',
             'crates/overdrive-reconcilers', 'crates/overdrive-sim'):
    for path in Path(root).rglob('*.rs'):
        lines = path.read_text(errors='replace').splitlines()
        for at, line in enumerate(lines):
            if '#[ignore' not in line or 'pending DELIVER:' not in line:
                continue
            total += 1
            start = at - 1
            while start >= 0 and (lines[start].strip().startswith(('///', '#['))
                                  or not lines[start].strip()):
                start -= 1
            if not any(x.strip().startswith('///') and 'CONTRACT_SHAPE:' in x
                       for x in lines[start + 1:at]):
                missing += 1
                print(f'{path}:{at + 1}')
print(f'{missing}/{total} pending tests lack a per-test declaration')
PY
```

### Initial finding F-02 — positive IPv4-mapped non-loopback bind (withdrawn in iteration 3)

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`. **Severity:** blocker.

The approved G-10 boundary (10) and DG-8 ruling in `feature-delta.md` require a real guest bind to `::ffff:<non-loopback>` on a declared port to produce a report and a startup `Pass` (`feature-delta.md:2928–2943`). The existing Tier-3 `a_loopback_only_bind_never_satisfies_a_declared_listener` (`guest_listener_real_boot.rs:381–454`) covers `127.0.0.1`, `::1` and `::ffff:127.0.0.1` as negative cases, with `0.0.0.0` as an ordinary positive control. `every_declared_listener_a_real_workload_binds_is_reported_before_it_exits` (`:324–378`) uses ordinary TCP/UDP wildcard binds. Neither binds a mapped non-loopback address. The held V39 spike is partial and explicitly says mapped IPv6 binds were not exercised (`spike/findings-listen-hooks.md:79`). The current negative case would still pass if the guest rejected **all** IPv4-mapped binds, contrary to the accepted contract.

`upstream-issues.md` acknowledges that this positive half was not authored because the current guest `bind` fixture accepts literal socket addresses and does not receive the leased workload address. That observation does not waive the approved Tier-3 outcome. **Required DISTILL remediation:** author one real-boot, production-composition positive case at the existing guest test-support boundary that obtains a genuine non-loopback address and observes the production startup `Pass` while the allocation is `Running`. Keep the production `Service` admission and guest hook path; do not add a public production seam. If the address cannot be obtained through legitimate test support, surface the exact testability boundary for a separate DESIGN ruling instead of silently omitting the case. No production defect is asserted by this finding; the accepted observable remains untested.

### Initial finding F-03 — S-53 cancellation ordering (withdrawn in iteration 3)

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`. **Severity:** blocker.

S-53 explicitly calls for `stop_alloc` both **mid-wait and between the insertion and the write** and requires no row write to start after cancellation (`feature-delta.md:1852`; G-10's `NoPassAfterEnding` invariant at `:2909`). `guest_listener_probe_rows.rs::no_row_is_written_after_stop_alloc` (`:264–293`) stops the target at line 278, then inserts its declared listener at line 282. That proves stop-before-insertion. The randomized S-53 schedule (`:316–424`) has only `Insert`, `Tick` and `FailNextWrite` operations; it never stops the allocation. The other S-53 tests stop only during cleanup. Thus a task that begins a `Pass` write after a report insertion but after `stop_alloc` is not distinguished by these bodies. The production-reachable owner path is guest `LISTEN` → shared report board → `ProbeRunner` guest-listener task, while the exit observer calls `on_alloc_terminal` → `stop_alloc`; the accepted design explicitly allows the two effects to order this way.

**Required DISTILL remediation:** add a bounded, seeded acceptance schedule at the existing report/runner ports that inserts the completing report, stops the allocation before the task's row write, and asserts no target row or post-cancellation write while an unstopped control still writes. Preserve the already covered mid-wait and ordinary `Pass` cases. If the existing scheduling/port surface cannot expose that ordering without a new seam, record the specific gap for DESIGN instead of inventing production API. This is missing proof of an already accepted invariant, not a claim that current production has a new cancellation defect.

### Completeness and mandate audit

The canonical seven categories were applied to the full feature. C1 (equivalence/boundary) covers empty lists, 1/N probe counts, the cap and timer edges; its mapped non-loopback positive partition is missing (F-02). C2 (state/transition) is represented by the accepted Quint state machines and the Rust gate/state grids. C3 (0/1/N) covers listener and probe collections. C4 (lifecycle/idempotence) covers double terminal hooks, same-row retries, restart/redeploy and inverse/no-op cases. C5 (mode/decision table) covers Service/Job/Schedule, opt-out, probe roles, CLI modes and raw/TOML ingress. C6 (negative/closed errors) is covered by typed nested error matches and the stated uninjectable line-emission limit. C7 (environment/interruption/concurrency) includes observation write/read faults, lag, exits, restart and seeded schedules, but misses S-53's specified cancellation ordering (F-03).

| Canonical 15-item check | Result | Evidence or reason |
|---|---|---|
| C1a empty/minimum | Pass | Empty probe/listener lists and zero GuestListeners controls. |
| C1b every material partition boundary | **Gap** | F-02: mapped non-loopback positive half of G-10 (10). Other count/deadline boundaries are present. |
| C2a state model documented | Pass | Feature's named Quint modules and Rust module-level state narratives. |
| C2b illegal event from relevant states | Pass | Terminal/no-op, stale/superseded, wrong-role and post-close cases. |
| C3 zero/one/many | Pass | Startup descriptors, listener sets and report cardinalities. |
| C4a repeat mutating operation | Pass | Repeated terminal hook, same-row retry and unchanged redeploy. |
| C4b inverse without prerequisite | Pass | Unknown/ended allocation no-op and stop/restart boundary cases. |
| C5a mode combinations | Pass | Kind, role, case, opt-out and both CLI deploy modes. |
| C5b flag orthogonality | Pass | CLI mode preserves typed parse cause; opt-out preserves existing behavior. |
| C6a malformed inputs | Pass | Beacon line, parser token and raw Service role/cardinality cases. |
| C6b declared typed errors | Pass with limit | All reachable typed read/input causes; `LineEmission` has no ordinary in-memory injected failure and is compile-pinned. |
| C6c closed error set | Pass | Exact variant/payload matches reject alternatives. |
| C7a degraded resource | Pass | Observation `Unreachable`/`Io`, failed row writes and lagged bus. |
| C7b interruption | **Gap** | F-03: completing insertion followed by cancellation before write. |
| C7c concurrent actors | Pass | Seeded board, hook, broker and restart schedules. |

**Checklist count: 13/15.** The numeric taxonomy threshold is met, but the two gaps are explicit accepted scenarios, and F-01 is a binding mechanical declaration gate. Therefore the full DISTILL suite is not approvable.

Project conventions replace the reviewer role's Python/Gherkin examples: `.claude/rules/testing.md` requires executable Rust tests and forbids `.feature` files. The Rust test names, docstrings and assertions provide the Given/When/Then narrative; direct pure-function calls are explicitly part of the approved S-15/S-21/S-34/S-46/S-55 obligations and have composed complements. The acceptance tests use port-exposed rows, actions, stream lines and handler outcomes rather than private component fields. The significant user journeys are represented by composed crash/restart, never-ready, recorded-verdict, backend-eligibility and guest-listener gate paths. The absent DISCUSS/DEVOPS directories and their default-environment treatment were already disclosed in the feature delta; this review does not invent user-story IDs or a new lane.

| Review dimension | Result |
|---|---|
| Happy/sad path balance | Pass — the suite has crash, failed write/read, timeout, lag, invalid input, stop and recovery paths. |
| Rust scenario narrative and domain terms | Pass — scenario names and assertions retain Service allocation, terminal, startup row and declared listener meanings. |
| Design scenario traceability and priority | Pass subject to F-02/F-03 — the inventory maps the accepted S-IDs, Quint runs, gates and DELIVER steps. |
| Observable outcomes and production composition | Pass subject to F-02/F-03 — composed paths use production owners and driven-port doubles; no test-only terminal or impossible raw Job/Schedule probe is manufactured. |
| Hexagonal boundary / pure-function complement | Pass — direct pure-property calls have the required composed entry-path tests. |
| Per-test Contract Shape | **Fail, F-01.** |

### Verification and disposition

- Read the approved DESIGN review and exact authoritative contract sections, scenario map, bounded typed-error audit, RED classification, upstream gap dispositions, relevant Rust acceptance bodies, scaffolds, conformance projection and Tier-3 fixtures. Checked the five changed scenario-to-step mappings against actual dependencies.
- `git diff --check` passed. Both `deliver/roadmap.json` and `infra/metal/test-placement.json` parsed as JSON. A local read-only scan produced the 156/122 pending-test declaration count. No cargo, Quint, native, mutation, DES, or test-runner command was run in this independent iteration; the author's focused Metal RED/control and clippy/fmt results remain recorded evidence, not results claimed for this review. The recorded whole-workspace `cargo fmt --all -- --check` failure in unchanged `xtask/tests/acceptance.rs:22` is a limitation, not a finding against these DISTILL changes.
- The current approved DESIGN contract and the user's D16 greenfield waiver are retained. No public API, production code, test body, normative DESIGN artifact, DES event, roadmap validation, or commit was changed by this reviewer.
- **Iteration-2 verdict: NEEDS_REVISION.** The proposed routing of F-02 and F-03 was rechecked and superseded in iteration 3 below. This is retained as a historical review record, not a current instruction to expand DISTILL.

## Iteration 3 — scope and production-reachability correction

The initial F-02 and F-03 were rechecked against the accepted handoff, exact S-53 oracle and current production owner path before routing them. This iteration supersedes their blocking classifications above; it changes no tests, production code or DESIGN contract.

| Initial finding | Corrected disposition |
|---|---|
| F-02, mapped non-loopback positive Tier-3 case | **Withdrawn as a DISTILL blocker.** The original roadmap 05-06 implementation notes expressly say “Not covered and not required: the `::ffff:<non-loopback>` Pass case (no seam gives the guest app its leased address).” `upstream-issues.md` also records that exact omission. The feature delta still states the desired mapped non-loopback behavior, but the accepted test handoff excludes this one recurring Tier-3 proof. No later user-approved test obligation or existing legitimate guest fixture boundary was identified that reverses that exclusion. The negative mapped-loopback and ordinary positive bind cases remain required and authored. This is a documented evidence limit, not permission to weaken the production behavior. No new test-support or production seam is mandated by this review. |
| F-03, cancellation between insertion and row write | **Withdrawn as an implementation or coverage blocker.** The S-53 oracle is “no row write is started after cancellation.” `guest_listener_probe_rows.rs:264–293` cancels the target through `ProbeRunner::stop_alloc` and then injects a completing report; it asserts no target row while an unstopped control writes. The 05-03 roadmap criterion requires “no row after `stop_alloc`,” which this body checks. The narrower proposed insertion-before-stop schedule was inferred from the S-53 stimulus prose; it is not a reproduced failure. Current `ProbeRunner::start_alloc` (`probe_runner/mod.rs:313–367`) spawns the probe task, `stop_alloc` (`:380–385`) cancels its supervisor token, and `VmDriver::on_alloc_terminal` (`vm_driver.rs:2067–2069`) calls it. The D14 `GuestListeners` task still panics in its RED scaffold (`probe_runner/mod.rs:643–648`), so there is no implemented task whose exact post-insertion scheduling or write behavior can be reproduced as defective. No seeded `overdrive-sim` failing invariant with a printed seed or complete production timing trace was supplied. An imagined interleaving or test-only abort cannot become a remediation requirement under `AGENTS.md`; no new Sim seam/API is authorized. |

**Revised canonical completeness:** C1b passes for the accepted DISTILL scope with the mapped non-loopback evidence exclusion explicitly recorded; C7b passes on the authored stop-before-report, stop/restart, crash and timeout interruptions. The other 13 items retain their iteration-2 dispositions, so the scoped checklist is **15/15 pass or inapplicable**. Its score does not override the binding per-test declaration gate. F-01 remains a concrete mechanical handoff blocker: the 156 pending bodies are the roadmap's future transitioned set and 122 lack their required declarations. The inventory command above reproduces the count and paths. The user's rule forbids advancing a DELIVER step on a missing per-test declaration while crafters are instructed to remove only the pending marker.

**Current verdict: NEEDS_REVISION, F-01 only.** Return only the per-test declarations to DISTILL, then re-review that bounded remediation and the complete suite. F-02 and F-03 are nonblocking recorded limits; do not dispatch tests, design changes, a Sim seam, or production changes for them from this review. `deliver/roadmap.json` validation remains `pending`.

## Iteration 4 — F-01 remediation and final full-suite verdict

**Remediation reviewed.** The acceptance designer added per-test `/// CONTRACT_SHAPE:` declarations to the previously missing pending DELIVER identities in 30 Rust files. The exact inventory command in F-01 now reports **0/156 missing**. All 46 pending Quint identities have a declaration at the final attribute boundary; the other pending properties and acceptance/composed/Tier-3 bodies have a declaration in their own test block. The exact `/// CONTRACT_SHAPE: pure-function.` line appears on applicable pure properties, including S-34's `progress_deadline` property and the D13 beacon wire-form properties. State/action-producing tests use `bounded-change` for their declared observable universe. The pending markers and 156-test activation set remain present.

**F-01 disposition: resolved.** I inspected the changed Rust hunks for the remediation: they add or reposition Contract Shape rustdoc lines around test attributes. The amended S-42 bodies and the new S-57 core/HTTP/CLI cases retain the exact typed variant, nested payload, field, exit and no-delta assertions reviewed in iteration 2. No accepted error representation, public API, scenario mapping or prior RED classification was changed by this metadata remediation. The six new Metal placements, D16-G1 greenfield scope, and final-wave-only mutation gate remain as reviewed. F-02 and F-03 remain withdrawn; no mapped-nonloopback case, speculative schedule, production seam or new design demand was added.

**Verification and provenance.** My read-only scan returned `missing/total 0 156`; `git diff --check` passed. I reviewed the test metadata diffs and the updated `contract-remediation-audit.md`, `red-classification.md` and feature-delta status evidence. The author reports changed-file `rustfmt --check` and Metal Clippy across `overdrive-core`, `overdrive-control-plane`, `overdrive-cli`, `overdrive-worker` and `overdrive-reconcilers`, with all targets, `integration-tests` and warnings denied, passing. I did not rerun cargo, Quint, native boots or mutation in this iteration and do not promote the author's reported commands to independent run results. The previously recorded whole-workspace `cargo fmt --all -- --check` failure remains limited to unchanged `xtask/tests/acceptance.rs:22`.

**Final verdict: APPROVED — full independent DISTILL review.** The accepted D1–D17 and D14-E2 obligations have executable Rust bodies or explicit scope/evidence limits, typed error causes and mappings are pinned and asserted at their reachable boundaries, the per-test declaration gate now passes, and no unresolved `SPECIFICATION_AMBIGUITY` or in-scope acceptance blocker remains. This approval records the actual independent DISTILL gate after iteration-4 remediation; it does not approve the pending roadmap, execute a DELIVER step, run mutation testing or alter the user's D16 waiver.
