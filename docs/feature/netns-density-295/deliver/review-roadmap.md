# DELIVER Roadmap Review — `netns-density-295`

## Metadata

- Reviewer: independent `nw-solution-architect-reviewer` (DELIVER roadmap reviewer)
- Iteration: 1
- Reviewed at: 2026-09-20T16:44:36+02:00
- Roadmap: `docs/feature/netns-density-295/deliver/roadmap.json`
- Authoritative inputs: `feature-delta.md`, `docs/product/architecture/brief.md`,
  `distill/test-scenarios.md`, `distill/red-classification.md`,
  `spike/wave-decisions.md`, and the accepted GH #295 ADR set
- Mechanical integrity: `des-verify-integrity --roadmap-only` was reported as
  passed before this review
- Worktree safety: pre-existing `.serena/project.yml` and
  `deliver/execution-log.json` were preserved

## Verdict

**REJECTED_PENDING_REVISIONS.** The roadmap is not executable under the accepted
contract yet. `roadmap.json` validation remains `status: "pending"`; it was not
approved or modified.

The roadmap has a sound overall owner decomposition and dependency order, but it
contains an explicitly unresolved benchmark target shape, one contradictory
public TCX API declaration, broken ADR references, an incomplete non-regression
verification command, and an omitted CLI driving-adapter boundary. Its size also
fails the repository's roadmap concision gate.

## Strengths

- The eleven-step dependency chain is coherent: the dependency-neutral EXEC/API
  cut precedes the provisioner, TCX/nft, listener, recovery, production
  composition, conformance, and receipt work.
- The roadmap preserves the accepted single-owner boundary and repeatedly forbids
  compatibility APIs, second owners, public fault hooks, and product resource
  expansion.
- The scenario mapping covers every DISTILL scenario ID, including `31A` and
  `31B`, and both benchmark receipts. The only mapping defect is duplicate
  prerequisite ownership for `27` and `28`, identified below.
- Most test boundaries are correctly separated: in-process Rust production
  composition, real-kernel/native-metal evidence, direct-handler conformance,
  and non-EDD capacity receipts are named as distinct lanes.
- The roadmap carries the accepted capacity boundary, fixed bridge identity,
  source-honest rollback dispositions, exact listener ownership, and EXEC-close
  ordering into observable criteria rather than substituting a new architecture.

## Findings

### RMR-01 — Blocker: the benchmark executable and receipt location are an unresolved design/API gap

**Severity:** Critical

**Evidence:**

- Step `04-04` introduces the concrete binary
  `crates/overdrive-control-plane/bin/netns_density_benchmark.rs`, a new
  `cargo run --bin netns_density_benchmark` interface, and an output contract in
  `roadmap.json:515-548`.
- The step's own implementation note at `roadmap.json:548` says that the
  feature delta names only “a future DELIVER benchmark target,” explicitly calls
  the crate/bin path a proposal, and asks for architect/user confirmation before
  landing it.
- The accepted design keeps the production constructor and owner private:
  `feature-delta.md:1780-1782` keeps `HostGuestNetworkProvisioner` and plan
  construction private, while `feature-delta.md:1822-1828` keeps the production
  shared-owner implementation and constructor private. The public
  `run_server` composition is the only stated production construction boundary
  (`feature-delta.md:1864-1870`). No accepted benchmark entry point or binary
  signature is specified.
- The roadmap simultaneously lists receipt directories under `target/` at
  `roadmap.json:536-540` and excludes `target/**` at `roadmap.json:578-582`.
  The repository `.gitignore:2` also ignores `/target/`.

**Reachability and consequence:** A standalone Cargo binary cannot call the
private owner/constructor merely because it is in the same package. Without an
approved benchmark driving boundary, the crafter must either invent public
surface, duplicate owner construction, or fail to drive the production owner.
The receipt location is likewise not a deterministic committed artifact path.
This is not a hypothetical implementation concern: the exact step already
declares the proposed binary and the missing contract.

**Required disposition:** Keep the finding open as a DESIGN/API gap. Obtain an
explicitly approved non-operator benchmark target shape and an evidence/receipt
location that is compatible with repository tracking before executing `04-04`.
Do not add a public CLI command, owner accessor, second owner, or persistence
mechanism by inference.

### RMR-02 — Blocker: the TCX adapter criterion names the wrong public operation surface

**Severity:** Critical

**Evidence:**

- `roadmap.json:178` says `guest_tcx` exposes exactly five doc-hidden
  operations “(attach/pin/adopt/query/remove).”
- The accepted D-295-DISTILL-6 contract instead names the exact five operations
  at `feature-delta.md:763-790`: `query_attachment`, `detach_pinned_link`,
  `endpoint_present`, `remove_endpoint`, and `read_counter`. It also states at
  `feature-delta.md:793-799` that the endpoint/counter ABI and raw aya types
  remain private.
- The feature delta's component table uses “typed query/detach/endpoint/counter
  adapter operations” (`feature-delta.md:658`), while “attach/pin/adopt/query” is
  only a lifecycle/evidence description (`feature-delta.md:3949`), not the
  accepted five-method doc-hidden API.

**Reachability and consequence:** The criterion is on the exact implementation
  path consumed by the shared owner and S-ND295-37's external mutation body. A
  crafter following it can implement a different public surface (or expose
  attach/pin/adopt operations that the accepted contract keeps behind the
  production owner), violating the repository's exact-API rule even if tests
  pass.

**Required disposition:** Replace the criterion with the exact D6 names and
  signatures, or surface a design amendment before DELIVER. Do not let a
  crafter choose the nearest compiling operation names.

### RMR-03 — High: design references are not traceable to the accepted ADR files

**Severity:** High

**Evidence:** Every ADR path in the roadmap's `design_refs` after the first three
  feature-local documents is missing from the repository:

- `roadmap.json:14` names
  `adr-0085-transparent-mtls-node-shared-guest-network.md`; the actual accepted
  file is `adr-0085-subprocess-free-netlink-mechanism-swap.md`.
- `roadmap.json:15-18` names non-existent ADR-0114, ADR-0115, ADR-0116, and
  ADR-0120 filenames. The actual files are
  `adr-0114-node-local-shared-bridge-guest-network.md`,
  `adr-0115-tcx-endpoint-classification-feeds-transparent-mtls.md`,
  `adr-0116-shared-bridge-gateway-dns.md`, and
  `adr-0120-node-shared-transparent-mtls-listeners.md`.
- `roadmap.json:19-22` maps the wrong titles to ADR-0122 through ADR-0125.
  In particular, the accepted records are
  `adr-0121-fixed-guest-network-admission-cap.md`,
  `adr-0122-guest-network-operation-error-family.md`,
  `adr-0123-node-session-mtls-registration-generation.md`,
  `adr-0124-bounded-shared-network-owner-recovery.md`, and
  `adr-0125-constant-nft-rules-shared-intercept-elements.md`.
- The accepted fixed-MAC decision is ADR-0126 and the accepted address/pool and
  density decisions are ADR-0117/ADR-0118; they are omitted from the roadmap
  references even though their contracts are implemented in steps `02-01` and
  `03-01`.

**Consequence:** A crafter or reviewer following `design_refs` cannot locate the
  authoritative ADRs and may consult the wrong decision number. The feature
  delta remains the primary exact-shape SSOT, but the roadmap's architecture
  traceability gate fails as written.

**Required disposition:** Correct the filenames/titles and include every active
  ADR that supplies a step contract, especially ADR-0117, ADR-0118, ADR-0121,
  and ADR-0126. This is a reference correction, not permission to alter the
  accepted architecture.

### RMR-04 — Blocker: the roadmap fails the concision and precision gate

**Severity:** High (blocking under the roadmap review check)

**Evidence:** A mechanical word count of the JSON gives 8,111 words overall and
  7,661 words in `phases`, versus the roadmap-review threshold of 3,000 words
  for 9–15 steps. Seven of eleven step descriptions exceed the 50-word limit;
  nine of eleven steps exceed the five-AC limit; and every step's
  `implementation_notes` exceeds the 100-word limit (the largest is over 200
  words). For example, step `01-01` spans `roadmap.json:44-97` and has 11
  criteria plus a 166-word description and 210-word note.

**Consequence:** The roadmap duplicates large portions of the feature delta and
  DISTILL SSOT, obscures the bounded step contract, and makes it harder for
  isolated crafters/reviewers to identify which acceptance bodies are actually
  owned by a step. This is a roadmap-readiness failure independent of whether
  the copied design prose is correct.

**Required disposition:** Reduce each step to a concise observable outcome,
  exact design references, scenario IDs, bounded file guidance, and verification
  commands. Keep exact API fences in `feature-delta.md`; link to them rather
  than copying the full algorithm and implementation notes into the roadmap.

### RMR-05 — High: scenario ownership is ambiguous for S-ND295-27 and S-ND295-28

**Severity:** High

**Evidence:** Step `01-01` lists `S-ND295-27` and `S-ND295-28` in
`roadmap.json:62-66` while its scenario name says they are only compile-time
prerequisites and its criteria state that neither is activated. The same IDs
are then the actual activation IDs for steps `03-01` and `03-02` at
`roadmap.json:297-301` and `roadmap.json:333-337`. The roadmap therefore has
43 scenario entries but only 41 unique IDs; `27` and `28` are duplicated.

**Reachability and consequence:** The delivery protocol at `roadmap.json:24-30`
says each step activates only the bodies named by its `scenario_ids`. An
isolated `01-01` crafter/reviewer can therefore receive ownership of bodies
that the same step explicitly must not activate, while the later activation
steps also claim them. This is a real orchestration ambiguity, not a test
failure hypothesis.

**Required disposition:** Remove the prerequisite IDs from `scenario_ids` or
introduce a distinct non-activation prerequisite field. Keep each scenario's
activation ownership unique in the roadmap.

### RMR-06 — High: S-ND295-36 is not linked to executable verification commands

**Severity:** High

**Evidence:** Step `04-03` promises every mapped non-regression body at
`roadmap.json:485-489`, but its verification at `roadmap.json:504-510` does not
run the named package/test targets:

- `-p overdrive-cli --test integration` is asked to match
  `service_kind_vm_workloads` and `mtls_resolve_rekey`, but the former is
  registered in the CLI **acceptance** target (`crates/overdrive-cli/tests/acceptance.rs:74`)
  and the latter in the control-plane acceptance target
  (`crates/overdrive-control-plane/tests/acceptance.rs:108`).
- The mapped production-runner bodies are in
  `crates/overdrive-control-plane/tests/acceptance/service_kind_vm_workloads.rs:76`
  and `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:442`,
  not in the CLI integration target used by the command.
- The actual host and dataplane tests are separate targets at
  `crates/overdrive-host/tests/integration/cgroup_accounting_equivalence.rs:51`
  and `crates/overdrive-dataplane/tests/integration/service_map_forward.rs:133`,
  which the roadmap does run, but that does not cover the omitted control-plane,
  worker, CLI-acceptance, and resolver suites.

**Consequence:** The command can be green while the required S-ND295-36 test
families were not executed. The scenario-to-test matrix therefore does not
provide independent evidence for the stated non-regression criterion.

**Required disposition:** Add correctly scoped nextest commands for each mapped
package/target (or narrow the criterion to the bodies actually run), and list
the existing test homes in the step linkage. Do not create duplicate #295 tests
for already-owned behavior.

### RMR-07 — High: the CLI-verb driving-adapter boundary is omitted

**Severity:** High

**Evidence:** The accepted DISTILL policy explicitly separates the in-process
Rust walking skeleton from the built-product CLI path. It says at
`feature-delta.md:721-727` that `overdrive deploy <spec>` argument parsing,
spec-file loading, and exit codes are not covered by the direct-handler tests
and are instead proved by a Tier-3 native-metal driving-adapter check. The
roadmap's `04-01` criteria at `roadmap.json:416-421` claim real `serve + deploy`
entry points, but the step only modifies Rust integration files at
`roadmap.json:428-431` and verifies nextest at `roadmap.json:432-436`; it has no
built-binary driving-adapter command, expectation artifact, or example-run
verification.

The existing S-ND295-01 body is an in-process handler composition
(`crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:2930`),
which is the correct Rust evidence lane but cannot establish the CLI verb's
argument/exit-code boundary.

**Consequence:** The internal vertical slice is covered, but the claimed
operator entry point is not independently verified at the boundary the accepted
design assigns to it. This also leaves the external-validity check conditional
on an unrecorded manual action.

**Required disposition:** Add the accepted Tier-3 driving-adapter lane and its
artifact/command, or explicitly remove the CLI-verb claim from the step and
record the resulting coverage boundary. Do not make the conformance suite spawn
the product binary; its direct-handler boundary is intentionally correct.

## Required-check disposition

| Check | Result | Evidence/disposition |
|---|---|---|
| Architecture/design traceability | **FAILED** | RMR-01 and RMR-03: the benchmark target is not pinned and the ADR references do not resolve. |
| Exact public API shape | **FAILED** | RMR-01 and RMR-02: the benchmark entry point is proposed, and the TCX criterion names a different five-operation surface than D6. |
| DISTILL scenario coverage | **CONDITIONAL** | All 39 prose IDs plus both receipts are represented, but RMR-05 gives 27/28 duplicate ownership and RMR-06 leaves S-ND295-36 commands incomplete. |
| Dependency ordering | **PASSED** | The declared sequence `01-01 -> 02-01 -> 02-02 -> 02-03 -> 03-01 -> 03-02 -> 03-03 -> 04-01 -> 04-02 -> 04-03 -> 04-04` is acyclic and matches owner prerequisites. |
| Phase/step sizing | **FAILED** | The owner decomposition is sensible, but RMR-04 violates the mandatory roadmap size/precision thresholds. |
| Scenario/test-file linkage | **FAILED** | RMR-05 and RMR-06; most other scenario groups have appropriate test homes and verification lanes. |
| Production-entry-point vertical slices | **CONDITIONAL** | `04-01` and `04-02` drive the production composition root correctly, but RMR-07 omits the separately required built-CLI driving-adapter lane. |
| Examples/expectations/integration boundaries | **CONDITIONAL** | The Rust/conformance/benchmark separation is good; the CLI-verb boundary and benchmark artifact path remain unresolved. |
| Scope expansion | **FAILED** | RMR-01 proposes an unapproved concrete benchmark binary/CLI shape and conflicts with the target exclusion. |
| Executable without design gaps | **FAILED** | RMR-01 is an explicit unresolved design/API gap; RMR-02 and RMR-03 also prevent exact, traceable execution. |

## Architecture quality assessment

The accepted architecture itself is not rejected for technology bias. The brief,
feature delta, and active ADRs provide context, alternatives, quality
attributes, owner boundaries, recovery semantics, and testability evidence for
the shared bridge, TCX, nft guard, listeners, address pool, and EXEC gate. The
roadmap's failure is contract/traceability and execution readiness, not a new
architecture recommendation. No alternate persistence owner, recovery
protocol, listener model, or network subsystem is proposed here.

## Verification record

- Read the roadmap, feature delta, brief, DISTILL scenarios and RED
  classification, spike decisions, and the active accepted ADRs.
- Checked every `design_refs` path against the repository; all nine ADR paths in
  the roadmap are unresolved or misnamed, as recorded in RMR-03.
- Compared roadmap scenario IDs with the DISTILL SSOT; all intended IDs are
  present, with duplicate `S-ND295-27`/`28` prerequisite entries.
- Counted roadmap words and per-step description/criteria/note sizes for RMR-04.
- Checked every S-ND295-36 verification command against the actual Cargo test
  target homes for RMR-06.
- No production code was changed and no commit was created.

## Final disposition

The roadmap cannot advance to DELIVER execution or set `validation.status` to
`approved`. Resolve the two contract blockers (RMR-01 and RMR-02), repair the
traceability and verification defects, and rerun an independent roadmap review.
