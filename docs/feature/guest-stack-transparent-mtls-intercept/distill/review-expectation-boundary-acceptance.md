# Acceptance Review: Expectation Boundary Restoration

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review type | Fresh isolated acceptance-design review |
| Reviewed commit | `9558af759e049564b7149bea6459961979899e96` |
| Parent commit | `c33f0396edf86c1db888a4c36b751911258c48fb` |
| Review iteration | 1 |
| Verdict | **NEEDS_REVISION** |

## Scope and method

This review evaluates the acceptance boundary restored by the reviewed commit: one public, reply-dependent E07 expectation; exact Rust ownership for D7 and the contracts formerly assigned to E08/E09; checked-in example ownership; Contract Shape declarations; and honest roadmap/evidence state.

The review compared the commit with its parent, read the changed DISTILL, roadmap, example, expectation, and index artifacts, and traced every removed E08/E09 behavior to its declared executable Rust owner. It also applied the repository's acceptance-design criteria, BDD rules, and test-design mandates. Existing dirty working-tree files outside this review artifact were treated as pre-existing user work and were not modified.

## Boundary and trace evidence

The intended boundary is mostly restored correctly:

- E07 now owns one stakeholder-visible black-box journey: a VM caller resolves the named service, sends a byte-distinct request, and reaches ordinary success only after receiving the exact reply. Its README explicitly excludes netlink, nftables, packet capture, counters, original-destination recovery, TLS/kTLS, generation, and private cleanup assertions.
- The E07 runner is an honestly labelled pending stub. It checks that the four source/spec inputs are checked in, does not narrate successful product evidence, and is paired with a roadmap task to replace the stub.
- The example bundle is checked in under `examples/guest-stack-transparent-mtls-intercept/`; the caller's zero exit is causally dependent on the exact response, while timeout or a different response fails.
- The expectation index lists only E07 for this feature and states that private lifecycle, kernel, wire, and cleanup contracts remain Rust-owned.
- The roadmap is valid JSON, marks itself stale and regeneration-required, retains `validation.status = pending`, and therefore does not silently authorize DELIVER execution.
- All 15 stakeholder scenarios have exactly one Contract Shape declaration. The supporting-contract map also preserves the exact source path, function identity, and shape for D7, pre-READY failure, post-READY exit 78, diagnostic totality, reclamation, stop/idempotency, sibling preservation, interruption/concurrency, illegal-event properties, replay contracts, and cleanup complements.

### Former E08/E09 disposition

| Removed expectation responsibility | Retained executable owner |
|---|---|
| Fresh mesh-guard install failure refuses the workload | S-GTI-05, `when_the_mesh_guard_cannot_be_installed_the_workload_is_refused` |
| Same-identity restart/reclaim succeeds before execution | S-GTI-06a, `a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again` |
| Failed re-enrolment remains fail-closed | S-GTI-06b, `failed_re_enrolment_after_platform_reclamation_stays_closed` |
| Resolver failure before READY is a boot failure with complete cleanup and sibling preservation | S-GTI-08a plus `P-GTI-PRE-READY-ERROR-CLOSURE` |
| Operator exit 78 after READY is an ordinary result | S-GTI-08b plus `P-GTI-JOB-EXIT-CLASSIFIER` and `C-GTI-08-EXIT78` |
| Stop removes the target guard without harming siblings | S-GTI-12a, `a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind` |
| Repeated stop remains successful when no guard exists | S-GTI-12b, `job_stop_without_a_guest_egress_guard_is_idempotent` |
| Exact diagnostic, reconciliation, lease/preflight, illegal-event, replay, interruption, concurrency, and cleanup contracts | Named supporting Rust contracts in the roadmap and DISTILL maps |

No former E08/E09 outcome was found to be silently deleted. No removed internal contract was reassigned to E07.

## Acceptance-design scorecard

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias resistance | 9/10 | Failure, restart, reclamation, resolver, exit-classification, interruption, concurrency, stop, and idempotency paths remain represented. |
| Given/When/Then integrity | 9/10 | Scenarios use a single principal stimulus and observable outcomes; repeated stop is intentionally the idempotency stimulus. |
| Business-language discipline | 9/10 | The public E07 journey is stakeholder-readable; implementation-specific oracles remain focused Rust contracts. |
| Coverage completeness | 9/10 | Removed E08/E09 responsibilities retain exact executable owners; the sole EDD expectation has a bounded public claim. |
| Walking-skeleton quality | 8/10 | S-GTI-01 is marked as the single walking skeleton and exercises the public journey, but its declared ownership is internally inconsistent. |
| Priority alignment | 9/10 | The public proof is kept minimal while higher-risk private guarantees remain exact Rust acceptance/integration work. |
| Observable-behavior focus | 9/10 | E07 observes only operator commands and the caller's reply-dependent result. |
| Traceability | 6/10 | File/function/shape mappings are exact, but S-GTI-01 is simultaneously described as reply-only and specified to own protection/no-cleartext outcomes already assigned to S-GTI-03. |
| Contract Shape completeness | 10/10 | Every live stakeholder scenario carries one declaration and mapped supporting properties retain exact shapes. |

## Test-design mandate assessment

| Mandate | Result | Evidence |
|---|---|---|
| Driving-port / hexagonal boundary | PASS | E07 drives the built product through public CLI commands; private guarantees are assigned to production-composition Rust tests. |
| Business language at the acceptance boundary | PASS | E07 names the caller, named callee, request, exact reply, and ordinary terminal result without exposing implementation mechanics. |
| User-journey proof | PASS | The checked-in example supplies both endpoints and makes caller success depend on the exact reply. |
| No implementation duplication in the expectation | PASS | The future runner may compile/materialize checked-in fixtures but may not synthesize source, specs, or product logic. |
| Independent verification layer | PASS | E07 does not invoke `cargo test`, a Rust test binary, or an `overdrive-*` crate. |
| Honest evidence state | PASS | E07 and the roadmap remain pending; no captured success is claimed. |
| Exact ownership without overlap | **FAIL** | S-GTI-01's ownership summary contradicts its own Then clauses and overlaps S-GTI-03's declared wire/no-cleartext outcome. |

## Findings

### AEB-01 — S-GTI-01's declared ownership contradicts its specified Then universe

**Severity:** Medium
**Status:** Open

The D7 ownership note in `distill/test-scenarios.md` says S-GTI-01 “states only the stakeholder-visible named-peer reply” and assigns the complete D7 oracle to S-GTI-02 and the Rust decoder/oracle properties. `distill/red-classification.md` likewise says that all D7/wire mechanics remain in S-GTI-02/S-GTI-03. However, S-GTI-01 itself additionally requires that the connection be protected end-to-end and that no cleartext request or response exist on the peer path, including before `Running`. S-GTI-03 separately owns the TLS/no-cleartext invariant.

This is not merely editorial: S-GTI-01 and S-GTI-03 map to different exact Rust functions. A crafter following the scenario text can duplicate the wire/no-cleartext oracle in both tests, while a crafter following the ownership summary can omit two explicit S-GTI-01 outcomes. Either interpretation defeats the commit's stated exact, non-duplicative ownership boundary.

**Required remediation:** choose and document one consistent ownership model. The preferred correction is to retain S-GTI-01's broad stakeholder-level protection outcome, but revise the D7 ownership note and red-classification summary to say so explicitly and distinguish it from S-GTI-02's exact accounting oracle and S-GTI-03's detailed wire/TLS oracle. Alternatively, if S-GTI-01 is genuinely reply-only, remove its protection/no-cleartext Then clauses and leave those outcomes solely in S-GTI-03. Update all affected handoff summaries together so one reading governs the roadmap and future implementation.

## Verification performed

The following non-mutating checks passed against the reviewed tree:

- `git diff --check 9558af759e049564b7149bea6459961979899e96^ 9558af759e049564b7149bea6459961979899e96`
- JSON parsing of the changed roadmap, including confirmation of `validation.status = pending` and regeneration-required state
- exact count of 15 stakeholder scenarios and 15 Contract Shape declarations
- exact trace audit of the former E08/E09 responsibilities against stakeholder and supporting Rust mappings
- `bash -n verification/expectations/E07-guest-stack-mtls-named-peer-call-succeeds/runner.sh`
- executable-bit check for the E07 runner
- `rustc --edition=2024 -D warnings` for both checked-in example helper sources
- `rustfmt --check` for both checked-in example helper sources

Mutation testing was not run, as required for this review stage.

## Verdict

**NEEDS_REVISION**

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 0 |
| Medium | 1 |
| Low | 0 |

The expectation boundary itself is sound: E07 is minimal and black-box, the removed E08/E09 contracts remain exact Rust responsibilities, Contract Shape coverage is complete, and pending evidence is honestly represented. Approval is withheld solely because the S-GTI-01 ownership contradiction leaves a medium-severity ambiguity in the exact non-duplicative acceptance map.

---

## Iteration 2 — Remediation re-review

### Review metadata

| Field | Value |
|---|---|
| Reviewed commit | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Parent commit | `9558af759e049564b7149bea6459961979899e96` |
| Compared with | Iteration 1 above |
| Review type | Same-reviewer acceptance remediation re-review |
| Review iteration | 2 |
| Verdict | **APPROVED** |

### Verdict summary

Iteration 1's sole medium finding is closed. S-GTI-01 now ends at the exact reply-dependent stakeholder outcome, while S-GTI-02/S-GTI-03 and the named Rust properties exclusively retain protection, wire-confidentiality, D7, and kernel-oracle ownership. The summary, scenario, classification, feature handoff, expectation identity, index, and pending roadmap now express the same boundary.

The feature has exactly one active expectation, E07. Its one checked-in operator example deploys exactly one `[service]` + `[exec]` callee and exactly one `[job]` + `[vm]` caller; caller success is causally dependent on the callee's exact reply. E08/E09 have no active expectation directories, identities, mappings, or runtime gates. Their former failure, recovery, lifecycle, stop, cleanup, kernel, and wire contracts remain executable Rust obligations.

E07 remains honestly pending. Its executable stub validates only the checked-in association and exits 75; the harness records `execution_status: pending` and returns nonzero. This approval concerns the DISTILL boundary and executable example contract. It is not native-metal runtime evidence and does not authorize the still-pending roadmap.

### Finding counts

| Severity | Open | Resolved from Iteration 1 |
|---|---:|---:|
| Blocker | 0 | 0 |
| High | 0 | 0 |
| Medium | 0 | 1 |
| Low | 1 | 0 |

### Iteration 1 remediation disposition

| Finding | Disposition | Evidence |
|---|---|---|
| AEB-01 — S-GTI-01 ownership contradicts its Then universe | **RESOLVED** | S-GTI-01 in `distill/test-scenarios.md` now has one Then: the first named-peer connection receives the byte-distinct reply. The D7 ownership note and `distill/red-classification.md` consistently describe it as reply-only. S-GTI-02/S-GTI-03 retain the full protection, no-cleartext, TLS/kTLS, capture, counter, generation, and rule-identity universe. `feature-delta.md` propagates the same exclusive ownership. |

### Unique ownership and executable-boundary evidence

- The target tree contains one feature expectation directory only: `verification/expectations/E07-vm-job-calls-exec-service/`. The old E07 slug and E08/E09 expectation identities have no active references outside historical review artifacts.
- The sole E07 roadmap mapping points to that runner with `bounded-change`; `pending_edd_expectations` remains one. Roadmap validation remains `pending` and regeneration-required.
- The active example directory contains two workload specs and two workload helper sources. `callee.toml` declares one Service/Exec allocation with one replica. `caller.toml` declares one Job/VM allocation and names that callee's service DNS name and listener port.
- The callee explicitly selects `health_check.startup = []`. The operator entry point waits for public `Stable` before deploying the caller, avoiding the known-unreachable inferred host-namespace TCP probe.
- The checked-in preparation entry point binds all unavoidable materialized paths to checked-in source/specs, statically builds the helpers, creates a qualified private appliance image, installs the guest caller, uses same-filesystem staging, and delivers the KEK through the production credential contract.
- The checked-in caller has finite DNS, connect, write, read, retry, and total deadlines. It exits zero only after reading the exact byte-distinct reply; timeout, mismatch, or failed timeout installation exits nonzero.
- The operator entry point drives the built default-feature binary through public `serve`, `deploy --detach`, `workload describe`, and `job stop` commands. Its setup/teardown mechanics are fixture hygiene and do not claim D7, TLS, nft, generation, counter, lifecycle-vector, or private-cleanup evidence.
- The E07 README expressly prohibits inspecting or reimplementing internal D7/wire oracles. Strict D7, boot-failure, diagnostic, C4a, restart/reclamation, stop/idempotency, sibling, nft/FIB, cleanup, generation, counter, illegal-event, and replay contracts remain Rust-only in the active DISTILL and roadmap maps.
- All 15 stakeholder scenarios still have exactly one Contract Shape declaration. Removed E08/E09 responsibilities retain their exact Rust source path, function identity, and shape.

### Acceptance-design scorecard

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias resistance | 9/10 | The public expectation is intentionally one canonical success journey; all feature failure/recovery alternatives remain separately mapped to exact Rust tests. |
| Given/When/Then integrity | 10/10 | S-GTI-01 now states one action and one complete public outcome without private-oracle clauses. |
| Business-language discipline | 9/10 | E07 and S-GTI-01 describe the operator, named service, request, reply, and public result; focused Rust scenarios carry technical detail. |
| Coverage completeness | 9/10 | Every removed E08/E09 responsibility retains an executable Rust owner and no outcome was silently deleted. |
| Walking-skeleton quality | 10/10 | The sole walking skeleton is a complete, stakeholder-demonstrable service-name call through the production composition root. |
| Priority alignment | 9/10 | One qualitative end-to-end witness is retained without spending EDD scope on private deterministic contracts. |
| Observable-behavior focus | 10/10 | E07 accepts only public Stable/Succeeded observations whose success depends on the exact reply. |
| Traceability | 10/10 | Scenario, DISTILL, feature handoff, expectation, index, and roadmap ownership are now mutually consistent. |
| Contract Shape completeness | 10/10 | Fifteen scenarios have fifteen declarations; the sole E07 mapping and all supporting Rust shapes remain explicit. |

### Test-design mandate assessment

| Mandate | Result | Evidence |
|---|---|---|
| Driving-port / hexagonal boundary | PASS | The runnable journey uses the built production binary and public CLI surface; it imports or links no `overdrive-*` crate. |
| Business language at the acceptance boundary | PASS | The public claim is the caller's named-service request, exact reply, and ordinary successful result. |
| User-journey completeness | PASS | Checked-in preparation, callee availability, VM caller deployment, reply-dependent completion, and public observation form one runnable journey. |
| No implementation duplication in the expectation | PASS | Fixture scripts compile/materialize checked-in assets but do not synthesize specs/source or reproduce product/netlink/TLS logic. |
| Production composition | PASS | The example uses one real Service/Exec and one real Job/VM through `serve` and `deploy`; the E07 runner does not invoke the Rust test harness. |
| Example-only real-I/O boundary | PASS | E07 is one canonical native-metal example, not generated PBT at the expensive E2E layer. |
| Honest evidence state | PASS | Stub exit 75 maps to pending and nonzero; no executed product evidence is claimed. |
| Exact ownership without overlap | PASS | The prior S-GTI-01/S-GTI-03 overlap is removed; all internal contracts have Rust-only ownership. |

### AEB-02 — Catalogue navigation still describes every runner as Lima-only

**Severity:** Low

`verification/README.md` correctly defines declared execution substrates and E07's `native-metal` metadata, but its directory-tree note still says `run-expectation.sh` “runs runner.sh in Lima.” `verification/expectations/INDEX.md` also tells all future expectation authors to use the Lima-only `od` helper. E07's own README, substrate file, runner, and feature index block are unambiguous, so this stale general navigation text does not weaken the reviewed feature boundary.

**Recommended follow-up:** generalize those two navigation notes to the declared-substrate model and make the `od` helper recommendation conditional on Lima expectations.

### Verification performed

The following checks passed against the reviewed target tree:

- complete comparison of commit `f064e0566611b1b8c4da7775fc169b929bccbca3` with its parent and full reread of Iteration 1
- `git diff --check` for the target commit
- exact target-tree searches for the old E07 slug and active E08/E09 expectation identities
- roadmap JSON parse and confirmation of one EDD mapping, `validation.status = pending`, and regeneration-required state
- exact count of 15 stakeholder scenarios and 15 Contract Shape declarations
- `bash -n` and `shellcheck` on both example scripts, the E07 stub, and the changed expectation harness
- `prepare.sh check-source` and `run-example.sh check-source`
- direct E07 stub execution, confirming exit 75 and pending-only output
- Rust 2024 compilation with warnings denied and `rustfmt --check` for both helper sources
- TOML parsing for both workload specs

Native-metal product execution was not run, and no E07 evidence was claimed. Mutation testing was not run, as required for this stage.

### Final verdict

**APPROVED**

Iteration 1's medium finding is fully resolved. There are zero blocker, high, or medium findings. The sole low-severity catalogue-navigation wording issue is non-blocking and does not alter E07's exact ownership, runnable example, or pending evidence boundary.

---

## Iteration 3 — Runtime-boundary remediation re-review

### Review metadata

| Field | Value |
|---|---|
| Reviewed commit | `5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Cumulative review range | `9558af759e049564b7149bea6459961979899e96..5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Intermediate remediation | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Compared with | Iterations 1 and 2 above |
| Review type | Same-reviewer acceptance remediation re-review |
| Review iteration | 3 |
| Verdict | **APPROVED** |

### Verdict summary

Both prior acceptance findings are closed with no regression. AEB-01 remains resolved: S-GTI-01 and E07 own only the reply-dependent public journey, while S-GTI-02/S-GTI-03 and named Rust properties exclusively own D7, wire, kernel, lifecycle, and cleanup guarantees. AEB-02 is now resolved: the catalogue layout and authoring instructions consistently route runners through their declared substrate and make the Lima helper conditional.

The active feature boundary still contains exactly one expectation, E07. Its one checked-in example still contains exactly one `[service]` + `[exec]` callee with one replica and exactly one `[job]` + `[vm]` caller. The public journey is now more precise: it waits for the Service allocation table to report `Running` with replicas `1/1`, then accepts the Job only at `Terminated` with the public `Succeeded` verdict. The caller can produce that success only after the exact reply.

E08/E09 remain absent from the expectation catalogue and runtime gates. Their former D7, boot-failure, diagnostic, reclamation, failed-reinstall, stop/idempotency, sibling-preservation, nft/FIB, cleanup, generation, counter, illegal-event, and replay contracts remain mapped exclusively to Rust tests.

E07 remains fail-closed and honestly pending. No native-metal run or product evidence was produced by this review, and the roadmap remains pending and regeneration-required.

### Finding counts

| Severity | Open | Resolved in this iteration | Previously resolved and retained |
|---|---:|---:|---:|
| Blocker | 0 | 0 | 0 |
| High | 0 | 0 | 0 |
| Medium | 0 | 0 | 1 |
| Low | 0 | 1 | 0 |

### Prior-finding dispositions

| Finding | Iteration 3 disposition | Evidence |
|---|---|---|
| AEB-01 — S-GTI-01 ownership contradicts its Then universe | **REMAINS RESOLVED** | S-GTI-01 still ends at the byte-distinct named-peer reply. The D7 ownership note, RED classification, feature handoff, E07 README, index, and roadmap consistently reserve protection, no-cleartext, TLS/kTLS, capture, counters, generation, rule identity, and cleanup for Rust. |
| AEB-02 — catalogue navigation describes every runner as Lima-only | **RESOLVED** | `verification/README.md` now says the harness runs each runner on its declared substrate. `verification/expectations/INDEX.md` makes `od` Lima-specific and directs native-metal runners through the qualified metal path with checked-in substrate metadata. |

### Acceptance-boundary regression audit

| Boundary | Result | Evidence |
|---|---|---|
| Exactly one feature expectation | PASS | The target tree, feature index, DISTILL map, and roadmap expose only `E07-vm-job-calls-exec-service`. The old E07 slug and E08/E09 expectation identities have no active references. |
| Exactly one caller/callee journey | PASS | `callee.toml` contains one `[service]` + `[exec]` with `replicas = 1`; `caller.toml` contains one `[job]` + `[vm]` and calls that service's DNS name and port. |
| Public, reply-dependent outcome | PASS | The built CLI's Service `Alloc / State` and replica count establish callee availability; the Job's `Attempt / State` and `Verdict` establish ordinary success; caller zero exit requires the exact byte-distinct reply. |
| Correct public render parsing | PASS | Separate parsers are anchored on the production Service and Job table headers, reject the opposite table shape, and are exercised by the host-safe `check-source` route. |
| Production composition | PASS | The example builds the default-feature product and drives only `serve`, `deploy --detach`, `workload describe`, and `job stop`; it invokes no Rust test binary or `overdrive-*` crate. |
| Public/private cleanup separation | PASS | Cleanup requires bounded public stop commands, owns the exact started serve process, and removes only the per-invocation marker tree. It no longer enumerates, asserts, kills, or deletes product-private processes, run directories, cgroups, namespaces, links, nft state, or capture state. |
| Bounded public cleanup | PASS | Caller and callee stops each have finite command deadlines and must succeed for a successful run; the exact serve process has a finite TERM grace period followed by KILL; marker-owned preparation cleanup has bounded unmount, loop-detach, and outer cleanup deadlines. |
| Credential isolation | PASS | `serve` runs under a fresh anonymous session keyring, verifies the production KEK description is initially absent, resolves the per-run credential through `CREDENTIALS_DIRECTORY`, and never purges or overwrites an ambient key. |
| Preparation ownership | PASS | Traps and process-local ownership are established before the fixed output tree is created. A per-invocation token guards normal check/removal, and unmarked or foreign-token trees are refused. |
| Fail-closed pending state | PASS | The E07 stub performs only the checked-in association check and exits 75. The harness records `execution_status: pending`, returns nonzero for pending/failed/absent runners, and has host-safe branch tests. |
| E08/E09 remain Rust-only | PASS | No E08/E09 expectation directory, index row, EDD mapping, or runtime gate exists; all former responsibilities retain named Rust owners and Contract Shapes. |
| Roadmap authority | PASS | `validation.status` remains `pending`, `requires_regeneration` remains true, and the roadmap contains exactly one pending EDD mapping. |

### Acceptance-design and mandate status

| Area | Result | Assessment |
|---|---|---|
| Happy-path bias resistance | PASS | E07 is intentionally the one public success witness; error, recovery, reclamation, interruption, stop, and cleanup complements remain explicit Rust scenarios. |
| Given/When/Then integrity | PASS | The walking skeleton retains one operator action and one stakeholder-visible result. |
| Walking-skeleton user value | PASS | A VM workload calls a named service and receives the reply through the production composition root. |
| Observable behavior | PASS | E07 consumes only public Service/Job renders and process results; internal mechanisms do not determine its acceptance claim. |
| Traceability and Contract Shape | PASS | Fifteen stakeholder scenarios retain fifteen declarations; scenario, DISTILL, example, index, and roadmap ownership agree. |
| Hexagonal driving boundary | PASS | The built operator binary is the sole product-driving interface. |
| No implementation duplication | PASS | Preparation compiles/materializes checked-in workload assets but does not reproduce product, D7, nft, TLS, or lifecycle logic. |
| Layer-appropriate input mode | PASS | The native-metal journey remains one canonical example, not generated property testing at the E2E layer. |
| Evidence honesty | PASS | Pending is distinct from succeeded, substrate is explicit, and only successful runner exit can make the harness return zero. |

### Changed cross-catalogue surfaces

The generic harness changes do not create a feature-boundary regression. Absent, pending, failed, invalid-substrate, Lima-success, and native-metal-success branches now have a host-safe executable check. E06's checked-in `native-metal` declaration corrects fresh substrate manifests while preserving its historical pinned evidence as history. Neither change creates another expectation for this feature or expands E07's claim.

### Verification performed

The following host-safe checks passed against the reviewed target tree:

- full reread of the latest acceptance artifact and comparison of both remediation commits with the cumulative target
- `git diff --check` for `f064e056..5279a561` and inspection of every changed active artifact
- target-tree searches for the old E07 slug and active E08/E09 expectation identities
- roadmap JSON parse, one-EDD inventory, and pending/regeneration-required validation audit
- exact count of 15 stakeholder scenarios and 15 Contract Shape declarations
- `bash -n` and `shellcheck` for both example scripts, the E07 stub, the harness and harness branch test, and the changed E06 runner
- `prepare.sh check-source` and `run-example.sh check-source`, including the Service/Job render-parser self-checks
- `verification/harness/test-run-expectation.sh`, covering succeeded, pending, failed, absent-runner, invalid-substrate, Lima, native-metal, and other branches
- direct E07 stub execution, confirming exit 75 and pending-only output
- Rust 2024 helper compilation with warnings denied and `rustfmt --check`
- TOML parse and exact top-level section audit for the one caller and one callee specs

Native-metal execution, feature runtime evidence, Rust feature suites, and mutation testing were not run.

### Final verdict

**APPROVED**

There are zero open blocker, high, medium, or low findings. The sole E07 boundary is public, runnable by contract, uniquely owned, bounded in its public cleanup, and fail-closed while pending. E08/E09 remain exclusively Rust-owned.

---

## Iteration 4 — Launch-lifecycle remediation re-review

### Review metadata

| Field | Value |
|---|---|
| Reviewed commit | `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Cumulative review range | `9558af759e049564b7149bea6459961979899e96..f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Compared with | Iterations 1–3 above |
| Review type | Same-reviewer acceptance remediation re-review |
| Review iteration | 4 |
| Verdict | **APPROVED** |

### Verdict summary

The launch-lifecycle remediation preserves the approved acceptance boundary. The feature still has exactly one active expectation, E07, and its stakeholder claim remains exactly one VM Job calling one Exec Service and succeeding only after receiving the byte-exact, byte-distinct reply. The public success oracle remains Service `Running` with replicas `1/1`, followed by Job `Terminated` with `Verdict: Succeeded`; no wrapper, PID, process-group, keyring, teardown, kernel, wire, or private-cleanup fact has become a stakeholder outcome.

The new session wrapper, lifecycle helper, and host-safe fault harness are fixture-preparation and bounded-cleanup machinery. Their references appear in runtime/preparation contracts and roadmap verification obligations, outside the S-GTI-01 Then clause and outside E07's Expectation section. `keyctl session -` remains only the preparation mechanism that isolates production KEK delivery before `serve`; neither keyring presence nor keyring behavior is included in the evidence success oracle.

E08/E09 remain absent from the expectation catalogue, index, roadmap EDD mappings, and runtime gates. Their former D7, boot-failure, diagnostic, reclamation, reinstall, stop/idempotency, sibling-preservation, nft/FIB, generation, counter, illegal-event, replay, and cleanup responsibilities remain exclusively assigned to named Rust scenarios and properties.

E07 remains honestly pending. Its runner still performs only the checked-in association check and exits 75, the harness still maps that result to non-successful pending evidence, and the roadmap remains `validation.status = pending` with regeneration required. No native-metal product execution or evidence capture was performed in this review.

### Finding counts

| Severity | Open | Resolved in this iteration | Previously resolved and retained |
|---|---:|---:|---:|
| Blocker | 0 | 0 | 0 |
| High | 0 | 0 | 0 |
| Medium | 0 | 0 | 1 |
| Low | 0 | 0 | 1 |

### Prior-finding dispositions

| Finding | Iteration 4 disposition | Evidence |
|---|---|---|
| AEB-01 — S-GTI-01 ownership contradicts its Then universe | **REMAINS RESOLVED** | S-GTI-01 still has one Then outcome: the named-peer connection succeeds and receives the byte-distinct reply. D7, protection, no-cleartext, TLS/kTLS, capture, counters, generation, rule identity, and cleanup remain outside that scenario and outside E07's public claim. |
| AEB-02 — catalogue navigation describes every runner as Lima-only | **REMAINS RESOLVED** | The catalogue still runs each expectation on its declared substrate, E07 still declares `native-metal`, and the Lima helper remains conditional. The new host-safe lifecycle-test navigation does not alter expectation execution routing. |

### Acceptance-boundary regression audit

| Boundary | Result | Evidence |
|---|---|---|
| Exactly one feature expectation | PASS | The target tree contains only `verification/expectations/E07-vm-job-calls-exec-service/` for this feature. The index and roadmap contain one E07 mapping; old E07 and E08/E09 identities have no active references. |
| One VM Job to one Exec Service | PASS | `caller.toml` contains one `[job]` + `[vm]`; `callee.toml` contains one `[service]` + `[exec]` with `replicas = 1`. The caller names that callee's service DNS name and port. |
| Exact-reply causality | PASS | The caller exits zero only after reading the exact byte-distinct response. Resolution, connect, write, read, mismatch, or total-deadline failure exits nonzero; the operator accepts only the resulting public Job success. |
| Stakeholder assertion purity | PASS | S-GTI-01 and E07's Expectation section contain only the operator action and reply-dependent public outcome. The lifecycle additions do not appear in either acceptance assertion block. |
| Lifecycle machinery remains fixture-scoped | PASS | Wrapper identity, start time, process group, handshake, TERM/KILL polling, reaping, and marker cleanup are described only as preparation/runtime admissibility and cleanup obligations and are exercised by a separate host-safe harness. They are not product success evidence. |
| `keyctl` remains preparation-only | PASS | `keyctl session -` isolates credential delivery before `serve`. The explicit evidence-success paragraph still depends only on public Service/Job renders and the exact caller reply, not on a keyring assertion. |
| E08/E09 remain Rust-only | PASS | No E08/E09 expectation directory, index row, EDD mapping, or runner exists. The latest commit changes no former E08/E09 scenario or supporting-contract owner. |
| Public/private cleanup separation | PASS | Public stop results remain required for the exact caller and callee; wrapper/serve termination and marker removal govern only the example fixture. E07 continues to forbid inspection or repair of product-private cleanup state. |
| Fail-closed pending state | PASS | The E07 stub exits 75 with pending-only text, and the generic harness branch test continues to distinguish pending from succeeded. |
| Traceability and Contract Shape | PASS | The target retains 15 stakeholder scenarios and 15 Contract Shape declarations. The roadmap retains one EDD mapping, `pending_edd_expectations = 1`, pending validation, and regeneration-required state. |

### Acceptance-design and mandate status

| Area | Result | Assessment |
|---|---|---|
| Given/When/Then integrity | PASS | The sole walking skeleton still has one operator action and one reply-dependent observable result. |
| Walking-skeleton user value | PASS | A VM workload calls a named service and receives its expected reply through the built production surface. |
| Observable behavior | PASS | Acceptance depends on public Service/Job renders and caller result, not lifecycle-helper internals. |
| Hexagonal driving boundary | PASS | Product execution remains through built `serve`, `deploy`, `workload describe`, and `job stop` commands. |
| No implementation duplication | PASS | The wrapper and helper manage fixture lifetime only; they do not reproduce product, netlink, nft, TLS, D7, or lifecycle-transition logic. |
| Layer separation | PASS | The host-safe lifecycle harness verifies example-launch hygiene, while feature lifecycle/kernel/wire guarantees remain Rust acceptance/integration responsibilities. |
| Evidence honesty | PASS | Pending remains non-successful and no runtime result is narrated as evidence. |

### Verification performed

The following host-safe checks passed against the reviewed target tree:

- full reread of Iterations 1–3 and inspection of every artifact changed by `5279a561..f908e0cf`
- `git diff --check 5279a561fcff74f61e8329f07fb6a72af0abe051 f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab`
- target-tree inventory confirming exactly one feature expectation and no active old-E07/E08/E09 identity
- exact count of 15 stakeholder scenarios and 15 Contract Shape declarations
- roadmap JSON parse, one-EDD inventory, `pending_edd_expectations = 1`, pending validation, and regeneration-required audit
- direct extraction of S-GTI-01 and E07's Expectation section, confirming lifecycle/keyring mechanics are absent from the stakeholder assertion
- `bash -n` and `shellcheck` for preparation, operator, session lifecycle/wrapper, E07 runner, generic harness, and both host-safe harness tests
- `prepare.sh check-source` and `run-example.sh check-source`
- `verification/harness/test-run-expectation.sh`
- `verification/harness/test-e07-session-lifecycle.sh`, covering pre-group, pre-readiness, pre-PID, PID-timeout, pre-exec, TERM, KILL escalation, bounded reaping, and unrelated-process preservation paths
- direct E07 stub execution, confirming exit 75 and pending-only output
- Rust 2024 helper compilation with warnings denied and `rustfmt --check`
- TOML parse and exact caller/callee top-level section, identity, replica, and target audit

Native-metal execution, feature runtime evidence, Rust feature suites, and mutation testing were not run.

### Final verdict

**APPROVED**

There are zero open blocker, high, medium, or low findings. The lifecycle remediation strengthens fixture safety without expanding the stakeholder claim: E07 remains the sole public one-VM-Job-to-one-Exec-Service exact-reply journey, `keyctl` remains preparation-only, and every former E08/E09 responsibility remains Rust-owned.
