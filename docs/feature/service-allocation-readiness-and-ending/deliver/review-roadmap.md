# Independent DELIVER Roadmap Review

## Metadata

- Feature: `service-allocation-readiness-and-ending`
- Artifact reviewed: [roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json)
- Review dates: iteration 1 at 2026-11-09 16:04 UTC; iteration 2 at 2026-11-09 19:59 UTC; iteration 3 at 2026-11-09 20:03 UTC
- Reviewer: independent roadmap reviewer
- Current verdict: **APPROVED at iteration 3**; iteration 1 was `SPECIFICATION_AMBIGUITY`, iteration 2 was `NEEDS_REVISION`
- Roadmap validation metadata: approved at 2026-11-09T20:03:41Z after this independent review
- Review iteration: 3; full disposition history follows

## Current summary

The upstream contract and approval gaps from iteration 1 are resolved by the approved full DESIGN and DISTILL artifacts. Iteration 2 confirmed the typed error contract, public signatures, S-57 activation, per-test Contract Shape scan, and single final mutation gate. Iteration 3 confirmed that the Phase 05 description and 05-05 scenario name now match D16-G1's current-format policy. The roadmap is approved at validation timestamp 2026-11-09T20:03:41Z.

## Iteration 2 summary

The Phase 05 description states that D16 removes `inferred` in two current-layout steps under existing envelope tags. Step 05-05 names current-layout golden evidence under existing tags. No historical decode requirement remains.

## Iteration 1 summary

The 17-step plan has a coherent dependency order, maps the ignored acceptance and conformance cases to implementation steps, and separates Sim composition from native-metal evidence. The G-10 (9a) test waiver follows the approved DG-13 ruling. The step sequence also places the guest-listener default flip after the host board, guest-listener task, real-boot hooks, and D17 wake.

**praise:** The plan keeps in-process composition tests, real-kernel boot evidence, and the operator-facing closeout distinct. Its rollout order makes the new guest-listener readiness gate the final behavior change.

Approval is blocked by two interface-contract gaps: D11 requires a stream error for observation-read failures without pinning the typed representation and mapping, and D14 introduces `AllocationSpec::tcp_service_ports()` without an exact signature. Additional blocking roadmap issues are missing DISTILL coverage for the `GuestListeners` input restrictions, missing per-test Contract Shape declarations on transitioned bodies, per-step mutation gates that conflict with the final-wave-only rule, and unresolved independent DESIGN/DISTILL approval evidence.

## Iteration 1 findings

### F-01 — D11 leaves the typed observation-read error contract unspecified

**SPECIFICATION_AMBIGUITY — blocking.**

The D11 contract requires an observation read failure, both at stream open and on a `Lagged` relist, to end the stream with `Err`. It cites the existing `workload_event_from_lifecycle_with_state` precedent and the rule that distinct failure modes get distinct variants, but does not state the semantic error set, the exact type/variant/payload at the stream boundary, or the mapping from `ObservationStore::alloc_status_rows` failures to that representation ([feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:859)). D11 also declares that it adds no public surface ([feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:865)).

Step 04-02 repeats only “ends the stream with an error,” while S-42 asserts the same generic outcome. The current stream carrier is `std::io::Error`; the repository’s typed-error gate explicitly says a generic `io::Error` does not resolve an unspecified semantic error contract ([AGENTS.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/AGENTS.md:79), [roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:245)).

**issue (blocking):** return this error path to DESIGN to pin the approved representation and each mapping. DISTILL should then assert that representation for open-read and relist failures. This review does not prescribe a new error type or variant.

### F-02 — D14 does not pin the `tcp_service_ports` method signature

**SPECIFICATION_AMBIGUITY — blocking.**

The accepted D14 contract introduces the cross-crate `AllocationSpec::tcp_service_ports()` accessor and defines its result semantically as the TCP filter of `declared_listeners` in declaration order, but leaves the method signature unspecified ([feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:2401), [feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:2595)). Step 05-01 relies on the name and behavior without supplying the missing receiver/return shape ([roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:300)). The return ownership and type determine the contract available to the TPROXY consumers.

**issue (blocking):** DESIGN must pin the exact signature before step 05-01 is executable. The implementation scaffold or existing field type cannot substitute for an accepted signature.

### F-03 — D14’s platform-only `GuestListeners` restriction has no acceptance scenario

**issue (blocking):** The design says `GuestListeners` is valid only as the sole startup descriptor of a Service, that `from_submit` and the parser reject it elsewhere, and that operators cannot write it in TOML ([feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:2382)). The DISTILL scenario inventory and RED classification do not include a test for raw API submission in a non-startup role, multiple descriptors, a non-Service workload, or an explicit TOML `GuestListeners` value. Step 05-07 covers default synthesis, opt-out, the dial-bearing probe cap, and the composed unavailable-listener path, but not those rejection rules ([roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:448)).

This leaves an accepted public input constraint without a RED/GREEN obligation. Add the missing pre-authored DISTILL cases and list their activation under 05-07. This is a DISTILL coverage remediation; it does not require new API or architecture.

### F-04 — Several tests to be transitioned lack per-test Contract Shape declarations

**issue (blocking):** The roadmap says crafters remove only the pending marker and do not edit acceptance bodies ([roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:11)). The workspace rule requires a Contract Shape declaration on each new or transitioned test, and the exact `/// CONTRACT_SHAPE: pure-function.` line on each applicable Rust property ([AGENTS.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/AGENTS.md:375)). Several bodies activated by the roadmap have only a module-level `//!` declaration:

- S-55’s pending property is preceded by a module-level `//! CONTRACT_SHAPE: pure-function.` but has no per-test rustdoc declaration ([declared_listeners_tcp_view.rs](/Users/marcus/conductor/workspaces/helios/ouagadougou/crates/overdrive-core/tests/acceptance/declared_listeners_tcp_view.rs:7), [declared_listeners_tcp_view.rs](/Users/marcus/conductor/workspaces/helios/ouagadougou/crates/overdrive-core/tests/acceptance/declared_listeners_tcp_view.rs:49)).
- S-34’s pending descriptor-list property has only a module-level explanatory declaration, not the required per-test line ([startup_deadline.rs](/Users/marcus/conductor/workspaces/helios/ouagadougou/crates/overdrive-reconcilers/tests/acceptance/startup_deadline.rs:9), [startup_deadline.rs](/Users/marcus/conductor/workspaces/helios/ouagadougou/crates/overdrive-reconcilers/tests/acceptance/startup_deadline.rs:65)).
- S-46’s pending property has a module-level `bounded-change` tag but no per-test declaration; DISTILL classifies this predicate as a pure proptest ([service_lifecycle_startup_stop.rs](/Users/marcus/conductor/workspaces/helios/ouagadougou/crates/overdrive-core/tests/acceptance/service_lifecycle_startup_stop.rs:15), [service_lifecycle_startup_stop.rs](/Users/marcus/conductor/workspaces/helios/ouagadougou/crates/overdrive-core/tests/acceptance/service_lifecycle_startup_stop.rs:371)).

**Required action:** return the affected bodies to DISTILL for the required per-test declarations before DELIVER. The roadmap’s crafter rule correctly forbids repairing these bodies during implementation.

### F-05 — Per-step mutation gates conflict with the final-wave-only mutation rule

**issue (blocking):** The roadmap carries mutation thresholds in step criteria and a `MUTATION GATE` command in every step’s notes; for example 01-01 requires an 80% kill rate per step and 04-02 repeats a per-step threshold ([roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:20), [roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:246)). The binding workspace rule makes mutation a single final DELIVER-wave gate after all step reviews. The roadmap has no final wave-level mutation gate outside the 17 steps.

**Required action:** remove mutation execution/kill-rate criteria from individual steps and record one final wave gate after all step reviews. Preserve all accepted targets, including the D17 signal-position and stored-permit mutants that support the S-48(b) waiver; do not weaken that evidence while consolidating it.

### F-06 — Independent DESIGN and DISTILL approval evidence is unresolved on disk

**issue (blocking):** The feature delta says DESIGN was approved at review round 7 ([feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:3057)), but the current DESIGN index still says no independent re-review authorizes DISTILL and records D13-D16 as user-approved but not accepted ([wave-decisions.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/design/wave-decisions.md:5), [wave-decisions.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/design/wave-decisions.md:10)). ADR-0270 and ADR-0271 also still say “not yet Accepted” with review/model-check gates remaining ([ADR-0270](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/product/architecture/adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md:5), [ADR-0271](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/product/architecture/adr-0271-the-probe-descriptor-no-longer-records-whether-it-was-inferred.md:5)).

The DISTILL artifacts say the independent review is pending, and the only recorded review outcome is Sentinel round 1 `needs_revision`; no subsequent approval is recorded ([upstream-issues.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/distill/upstream-issues.md:5), [feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:3218)). I found no checked-in DESIGN or DISTILL review artifact in this feature directory. The round-7 sentence may be stale summary prose; it does not reconcile the current status records or establish the independent approvals needed for DELIVER.

**Required action:** reconcile the DESIGN status with its actual independent review evidence, obtain/record the independent DISTILL approval, and leave the roadmap pending until both gates are supported by their review artifacts.

## Iteration 1 positive findings and closed hypotheses

- The 17 steps form an acyclic dependency graph and the ordering for D13-D15 is explicit. The real-boot Tier-3 cases are assigned to 05-06, while the composed Sim case for an omitted `LISTEN` line and the final default flip are assigned to 05-07. The criteria preserve the design’s TCP/UDP set, empty-list opt-out, and production composition boundaries.
- The individual ignored acceptance-test names in `distill/red-classification.md` are all referenced by a roadmap step’s implementation notes. The conformance families are assigned to their corresponding D1/D3, D6/D7, D9, D10/D11, D13/D14, and D17 steps. The named `G-10 (9a)` omission is the approved DG-13 waiver, not an orphan.
- V41 and V42 are labelled held in the DESIGN record; V42’s result is recorded as `PROMOTE PR-2` and the hook scope is accepted in contract shape. V39 is explicitly partial and carried, not a hidden DELIVER gate. I did not treat those labels as evidence of an unrecorded redesign.
- S-19 EarlyExit rendering already passes at HEAD in the DISTILL scenario matrix. The existing `execution-log.json` contains only 01-01 RED/FAIL and GREEN/PASS events, with no COMMIT. It remains untracked and unchanged; no step is reported complete.
- The roadmap’s file lists are treated as guidance per workspace policy. Necessary mechanical fallout across struct literals and production callers is already described in 05-01 and D16 notes, so no restrictive-allowlist finding is raised.

## Iteration 1 acceptance-scenario audit

The Rust acceptance bodies are the repository convention; no `.feature` files are expected. The production composition evidence is credible for the claimed boundaries: observer/shim/runner in-process tests for lifecycle ownership, `run_server` compositions for stream/readiness outcomes, and native-metal real-boot tests for guest kernel and beacon effects. The design’s error/boundary cases are materially represented, including failed observation writes, retries, lagged streams, malformed LISTEN input, no-op/duplicate reports, and deadline outcomes.

| Dimension | Score | Assessment |
|---|---:|---|
| Success/error balance | 9 | Substantial failure, retry, malformed-input, timeout, and unaffected-state coverage. |
| Scenario structure | 8 | Rust tests use setup, one action/sequence, and explicit observable oracles; Gherkin is intentionally absent. |
| Domain language | 8 | Scenario names largely use allocation, startup, Stable, listener, and deploy terminology; API identifiers stay in implementation notes where appropriate. |
| Coverage completeness | 6 | The D14 restriction on accepting `GuestListeners` only as the platform-generated sole Service startup descriptor has no matching test obligation. |
| User-journey focus | 8 | The default-gate steps include a complete service start/deploy path and native production proof. |
| Priority validation | 8 | The steps follow approved decisions and recorded production/spike evidence; out-of-scope items remain excluded. |
| Observable behavior | 8 | Scenarios mostly assert public rows, stream events, driver ports, and kernel/wire effects at the appropriate tier. |
| Traceability | 9 | Scenario names and conformance obligations map to decision groups and roadmap steps. No DISCUSS wave exists by design. |
| Production-composition boundary | 9 | The plan distinguishes in-process production composition, Sim fault cases, and real-kernel Tier-3 evidence. |

### Design mandates

| Mandate | Status | Evidence |
|---|---|---|
| CM-A — hexagonal boundary | Pass | Composed cases drive the production observer, shim, reconcilers, probe runner, and `run_server`; test doubles sit at driven ports. |
| CM-B — business-language abstraction | Pass with Rust adaptation | Rust scenario names use domain outcomes; `.feature` files are not part of this repository convention. |
| CM-C — user-journey completeness | Pass with F-03 exception | The default-readiness journey is composed through `serve`/`deploy`; the invalid `GuestListeners` input restriction lacks a scenario. |
| CM-D — pure-function extraction | Pass | Deadline, backend-eligibility, and declared-listener filter properties are separated from real-I/O checks. |

The failed completeness dimension and the contract ambiguities block approval. `@sizing-review-needed` is informational for steps 01-02, 02-01, 02-02, 02-03, 03-01, 04-01, 04-02, 05-02, and 05-03, which each map to at least eight named acceptance/conformance runs. These are sizing signals only; their detailed scenario coverage is legitimate and does not independently block approval.

## Iteration 1 verification

- `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/service-allocation-readiness-and-ending/deliver/ --roadmap-only` — **PASS** (`Roadmap format OK; validator: no errors; execution-log skipped`).
- Read-only structural audit — **PASS**: 17 step IDs match `total_steps`; all declared dependencies point to an earlier step; every individually listed ignored acceptance-test name in `red-classification.md` appears in a roadmap implementation note.
- No implementation tests, mutation runs, or DES phase commands were run.
- No roadmap validation metadata was changed. `validation.status` remains `pending` because this review does not approve the roadmap.

## Iteration 1 remediation disposition

No roadmap or design edits were made. The typed error and public signature gaps return to DESIGN. The missing `GuestListeners` rejection scenarios and Contract Shape declarations return to DISTILL. The mutation policy and upstream review evidence must be reconciled before the roadmap can be approved. The untracked historical execution log is preserved.

## Iteration 1 verdict

**SPECIFICATION_AMBIGUITY — not approved.** D11’s typed stream-error contract and D14’s `tcp_service_ports()` signature are not fully pinned. The independent DESIGN/DISTILL gates, DISTILL coverage, Contract Shape declarations, and mutation-gate placement also require resolution before DELIVER.

## Iteration 2 — upstream remediation and current-roadmap re-review

### Disposition of iteration-1 findings

| Finding | Iteration-2 disposition | Evidence |
|---|---|---|
| F-01 — D11 typed stream errors | **CLOSED** | D11-E1 pins `ServiceStreamError::{ObservationRead { source: ObservationStoreError }, LineEmission { source: std::io::Error }}`, the stream item signature, and the open/relist/source mappings. S-42 checks `Unreachable`, `Io`, and `Envelope` at both read sites, the nested payload, one error followed by stream termination, and the absence of a later timeout. |
| F-02 — D14 `tcp_service_ports` signature | **CLOSED** | D14-A1 pins `pub fn tcp_service_ports(&self) -> Vec<NonZeroU16>` as an owned, declaration-order TCP filter. S-55 and step 05-01 match that behavior. |
| F-03 — GuestListeners input restriction coverage | **CLOSED** | D14-E2 pins the exact aggregate/parser variants and ingress mappings. S-57 covers raw Service wrong roles, non-sole startup cardinality/precedence, canonical TOML tokens in every role/case, HTTP mapping/no-write, CLI mapping in both deploy modes, and the valid raw-Service control. Step 05-04 activates S-57; step 05-07 only changes default synthesis. |
| F-04 — per-test Contract Shape declarations | **CLOSED** | The read-only scan across all workspace pending DELIVER identities reports **0/156 missing**. The exact pure-function line is present on applicable properties. |
| F-05 — per-step mutation gates | **CLOSED** | Step criteria and notes no longer execute mutation checks. The roadmap has one final mutation gate after all 17 reviewed steps, 17 preserved target sources, an 80% minimum, and D17 S-48(b) signal-position/stored-permit evidence. |
| F-06 — DESIGN/DISTILL approval provenance | **CLOSED** | The current on-disk full DESIGN review is APPROVED at iteration 4; the current on-disk full DISTILL review is APPROVED at iteration 4. The feature delta and wave index point to those actual artifacts. The former unsupported round-7 claim is explicitly withdrawn. |

The full DESIGN and DISTILL approvals are now verified against their artifacts and current status documents. The user’s explicit D16 greenfield compatibility waiver covers only historical persisted-byte/backward-read compatibility for this feature’s pre-D16 archive shapes. D16-G1 retains all five envelope families and existing tags, current-format correctness and all other lifecycle/readiness outcomes. I do not revive the withdrawn mapped-nonloopback test demand or the speculative S-53 insertion-before-stop schedule.

### F-07 — D16 roadmap summary still describes an envelope bump and historical decode

**issue (blocking):** The accepted D16-G1 policy keeps the five existing envelope families, tags, aliases, and constructor selection; it rebaselines the current payload layouts in place and explicitly adds no new V2/V3 variant, historical nested type, migration reader, or backward-read promise ([feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:2781), [feature-delta.md](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/feature-delta.md:2800)). The roadmap still describes D16 as “two envelope-bump steps” in Phase 05 ([roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:254)), and step 05-05’s `scenario_name` still promises “prior versions decode through the new envelope” ([roadmap.json](/Users/marcus/conductor/workspaces/helios/ouagadougou/docs/feature/service-allocation-readiness-and-ending/deliver/roadmap.json:377)). Those descriptions can cause DELIVER to add the historical migration machinery the user explicitly waived and the accepted design forbids for this feature.

**issue (blocking):** align the Phase 05 description and 05-05 scenario name with D16-G1’s current-layout evidence under existing tags. Keep the current-layout archive/roundtrip criteria and the explicit waiver; do not add historical decode requirements.

### Current checks and disposition

- The roadmap has 17 steps for `total_steps = 17`; the dependency graph has no missing or forward dependencies.
- All individually listed ignored acceptance-test names in the RED classification are referenced in roadmap implementation notes. S-57 is explicitly assigned to 05-04. `raw_service_guest_listeners_as_sole_startup_is_admitted` is a passing control named in the 05-04 notes; it is not a pending activation.
- The final mutation gate is the sole gate. It keeps 17 target-source records, a minimum 80% kill rate, the D17 S-48(b) signal-position/stored-permit mutants, and the production-only target rule.
- The Contract Shape scan returned `0/156 missing`; no source/test body or existing pending marker was changed during this review.
- `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/service-allocation-readiness-and-ending/deliver/ --roadmap-only` — **PASS** (`Roadmap format OK; validator: no errors; execution-log skipped`).
- `git diff --check` on the reviewed feature design, roadmap and review artifacts — **PASS**.
- No tests, mutation run, DES phase command, production edit, or commit was made.

The 8+ scenario sizing flags remain informational on 01-02, 02-01, 02-02, 02-03, 03-01, 04-01, 04-02, 05-02, and 05-03. The untracked execution log remains unchanged with 01-01 RED/FAIL and GREEN/PASS, no COMMIT.

**Iteration-2 verdict: NEEDS_REVISION.** F-01–F-06 are closed. F-07 remains a roadmap-contract contradiction under the approved D16-G1 policy. `roadmap.validation.status` remains `pending`; no validation metadata was changed. After the two D16 descriptions are reconciled, a fresh independent roadmap review can approve the current artifact if no additional mismatch remains.

## Iteration 3 — D16 roadmap wording disposition

**Revision inspected:** the Phase 05 description now says “two current-layout steps under existing envelope tags,” and step 05-05 now names “current-layout golden fixtures under existing envelope tags.” The D16 implementation notes continue to require current-layout evidence across the five affected carriers, preserve existing envelope tags/aliases, and exclude historical migration readers. No earlier envelope-bump or prior-version decode requirement remains in the active roadmap text.

| Finding | Iteration-3 disposition | Evidence |
|---|---|---|
| F-01 through F-06 | **CLOSED** | D11-E1, D14-A1, D14-E2, per-test declarations, the final-wave mutation gate, and full DESIGN/DISTILL approval provenance remain exactly as verified in iteration 2. |
| F-07 — stale D16 envelope-bump/history wording | **CLOSED** | Phase 05 and step 05-05 now describe the approved D16-G1 current-layout evidence under existing tags. `rg` found no `envelope-bump` or `prior versions decode through the new envelope` strings in `roadmap.json`. |

**Verification:** `des-verify-integrity docs/feature/service-allocation-readiness-and-ending/deliver/ --roadmap-only` passed with no errors. `git diff --check` passed for the roadmap and review artifact. The 17-step graph, final mutation gate, D17 waiver evidence, S-57 activation, and `0/156` Contract Shape scan were unchanged. No tests, mutation runs, DES events, production/test edits, or commits were performed. The untracked historical execution log remains unchanged and has no COMMIT event.

**Iteration-3 verdict: APPROVED.** Findings F-01 through F-07 are closed. The reviewer updated only `roadmap.validation` to `status = approved`, reviewer `Codex (independent roadmap reviewer)`, and `approved_at = 2026-11-09T20:03:41Z`. This approval makes the roadmap eligible for DELIVER; it does not report any implementation step as complete.
