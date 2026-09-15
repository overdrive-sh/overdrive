# Adversarial review — DELIVER step 02-02

- **Feature:** `remove-legacy-exec-workload-driver`
- **Step:** `02-02` — migrate current operator artifacts to microVMs
- **Reviewer:** `nw-software-crafter-reviewer` (fresh step-specific reviewer)
- **Review ID:** `code_rev_20260915_02-02_iteration_1`
- **Reviewed commits:** `e36b8eb72b6712ef7b5015168361a8e6e001e4cd` and `910512aea6e740521be2601426c342389c7911f2`
- **Parent:** `e36b8eb7^`
- **Trailer:** `Step-Id: 02-02`
- **Final verdict:** **CHANGES_REQUESTED**

## Executive summary

The retained healthy VM Service source is structurally valid, the VM-only
preparation check passes without a deleted-spelling absence assertion, the
historical ADR/evolution paths and E06/E08 receipts are unchanged, and the
changed current journeys do not add a GH #295 topology claim. The implementation
does not yet satisfy the complete operator-artifact contract, however.

Three blocking/high findings remain:

1. General operator examples were deleted together with host-process examples,
   leaving current UDP, dial-by-name, probe, and EDD consumers pointing at
   missing files or at the removed cross-driver runner mode.
2. The live E12 liveness journey asserts that replacement keeps the original
   AllocationId, directly contradicting the accepted P-105 fresh-successor
   contract and the existing VM Service lifecycle evidence.
3. Active whitepaper and persona claims still present host-process execution as
   supported, despite the current execution boundary being VM/microVM-only.

No mutation run or architecture change is required for this documentation and
example step. The findings are bounded artifact corrections within the approved
feature delta.

## Iteration history

| Iteration | Reviewed commits | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `e36b8eb7`, `910512ae` | **CHANGES_REQUESTED** | 1 blocker, 2 high | Return to the original step crafter; preserve this review as the iteration-1 record. |

## Contract-shape and scope audit

| Contract | Result | Evidence |
|---|---|---|
| Current healthy VM Service source-valid | **PASS** | `examples/service-kind-vm-workloads/prepare.sh check-source` exits 0. It checks positive `[service]`/`[vm]` shape, guest paths, HTTP/TCP fixtures, and VM client specs; it does not assert absence of a deleted spelling. |
| VM/microVM current README and retained example | **PASS** | `README.md` and `examples/service-kind-vm-workloads/README.md` describe the supported VM path; retained Service/Job fixtures use `[vm]`. |
| HTTP/TCP probe contract | **PASS** | Retained Service fixtures and the healthy runner keep TCP startup and HTTP readiness against the guest; the changed product journeys describe HTTP/TCP only. |
| P-105 allocation identity | **FAIL** | `run-example.sh:1014-1017` requires `replacement_alloc == original_alloc`, while the accepted owner path emits a distinct successor. |
| Running → intercept → guest-command ordering | **PASS by scope audit** | This commit changes no production action-shim ordering and the current product guidance continues to describe guest-grounded VM execution. |
| Historical ADR/evolution prose and E06/E08 | **PASS** | `git diff` over `docs/product/architecture/adr-*.md`, `docs/evolution/`, and E06/E08 is empty. |
| GH #295 boundary | **PASS** | No changed artifact selects shared switching, per-tap classification, shared-bridge DNS, replacement mTLS, cross-host routing, density, or throughput work. |
| Current documentation/example completeness | **FAIL** | General current artifacts and active guidance still refer to deleted examples or host-process support; see D1 and D3. |

## Mechanical evidence

- Commit scope is 40 files, with documentation/example deletions and two
  bounded VM-test comment/scenario cleanups; no production Rust source is
  changed.
- `git diff --check e36b8eb7^ 910512aea6e740521be2601426c342389c7911f2` passes.
- `bash -n examples/service-kind-vm-workloads/prepare.sh
  examples/service-kind-vm-workloads/run-example.sh` passes.
- The focused existing production-owner acceptance test passes:
  `cargo test -p overdrive-reconcilers --test acceptance
  acceptance::service_kind_vm_workloads::workload_lifecycle_alone_decides_restart_after_liveness_stop -- --exact`.
  It asserts a `RestartAllocation` with predecessor
  `alloc-service-vm-liveness-restart-0` and successor
  `alloc-service-vm-liveness-restart-1`.
- The stale cross-driver mode is directly unreachable:
  `examples/service-kind-vm-workloads/run-example.sh run
  http-status-cross-driver` exits 1 with the current usage string because
  the dispatch arm was removed while the function body remains.
- The E07 runner reaches a missing product path:
  `verification/expectations/E07-vm-job-calls-exec-service/runner.sh` exits
  127 at `examples/guest-stack-transparent-mtls-intercept/run-example.sh`.
- No native-metal execution or mutation testing was required by this
  documentation-only step and neither is used as a finding.

## Blocking and high findings

### D1 — Blocker: general examples and their current consumers were deleted without migration or retirement

**Locations:**

- Deleted in the reviewed commit: `examples/dns-resolver.toml`,
  `examples/dial-by-name-responder/{a.toml,b.toml,ping_pong.py}`,
  `examples/quick-bind-service.toml`,
  `examples/liveness-{absent,fails,holds}-service.toml`, and the other
  general fixtures listed in the commit.
- `docs/product/journeys/submit-a-udp-service.yaml:67` and
  `docs/product/journeys/reach-an-unconnected-udp-service.yaml:92` still
  command `dns-resolver.toml`.
- `docs/product/journeys/dial-a-mesh-peer-by-name.yaml:117` still commands
  the deleted `dial-by-name-responder/a.toml + b.toml` pair.
- `verification/expectations/O03.../runner.sh:35`, `O06.../runner.sh:29`,
  `E02.../runner.sh:20`, `O02.../runner.sh:7`, `O07.../runner.sh:53-55`,
  and `E05.../runner.sh:30-32` still drive deleted fixtures.
- `verification/expectations/E07.../runner.sh:7,35` still drives the deleted
  `guest-stack-transparent-mtls-intercept` bundle.
- `verification/expectations/E10.../runner.sh` still validates eight
  Exec/VM cells, while the reviewed `run-example.sh` no longer dispatches
  `http-status-cross-driver` and its `http-exec-*.toml` fixtures are deleted.

The accepted feature delta distinguishes host-process-only examples from
general workload examples: host-process-specific outcomes are deleted, while
general journeys/examples are migrated to the existing VM artifact preparation
path (`feature-delta.md:760-762`). UDP reachability, dial-by-name, probe
semantics, and VM/HTTP status behavior are general contracts, not host-process
outcomes. The current files now either fail at a missing path or silently lose
their operator-runnable journey. E07's failure is reproduced by the runner's
exit 127; E10's entry point is reproduced by the removed-mode usage failure.

**Required remediation:** migrate the general fixtures and their current
journey/EDD consumers to the supported VM bundle and `[vm]` specs, or explicitly
retire/supersede only the host-process-specific expectations. Remove the stale
E10 cross-driver execution mode and E07 host-process Service expectation from
the active catalogue/harness (historical receipts must remain immutable where
the repository contract requires them). Ensure every retained current command
resolves to a checked-in, operator-runnable example. Do not add deleted-name
absence tests and do not touch E06/E08 evidence.

### D2 — High: E12 asserts the opposite of the accepted P-105 successor identity

**Locations:** `examples/service-kind-vm-workloads/run-example.sh:1000-1017`
and `:1065-1081`.

After observing the liveness-stopped predecessor, E12 captures the new
allocation, then requires:

```bash
[[ "$replacement_alloc" == "$original_alloc" ]] \
  || die "E12 restart changed the allocation identity unexpectedly"
```

The accepted feature contract requires `RestartAllocation.alloc_id` to remain
the accepted predecessor while `RestartAllocation.spec.alloc` is a distinct,
durably reserved fresh successor. The existing production-owner acceptance
test at `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:469-498`
passes and asserts exactly that predecessor/successor pair. The composed
projection property also states that every liveness replacement receives a
fresh physical identity (`crates/overdrive-sim/tests/integration/service_backend_projection.rs:624-632`).
Therefore the current native-metal `run liveness-restart` journey will fail at
the identity assertion when the real liveness replacement follows P-105. The
same-ID wording in the ledger and PASS line (`:1076-1081`) would also misstate
the stakeholder outcome even if the assertion were removed.

**Required remediation:** retain the predecessor row as immutable history and
assert that the replacement row carries a distinct fresh AllocationId, while
preserving the existing liveness probe attribution, restart count, terminal
history, and cleanup assertions. Keep the exact P-105 identity language in the
journey/ledger; do not change production lifecycle behavior or introduce a
same-ID exception.

### D3 — High: active current guidance still claims host-process execution

**Locations:**

- `docs/whitepaper.md:9` says the platform currently unifies virtual
  machines, processes, unikernels, and WASM; `:87-88` repeats processes as a
  first-class current workload type.
- `docs/whitepaper.md:573-599` presents a current VM/process shared-volume
  path and process cgroup right-sizing, without marking process execution as
  deferred. These statements sit outside the edited VM-only driver table.
- `docs/product/personas/ana-platform-engineer.yaml:37` still names Ana as
  running “host-process and VM-backed Service workloads,” and `:130` still
  compares VM Service behavior to a “host-process Service.”

The reviewed commit updates the whitepaper's driver table and mTLS section,
but leaves these active claims in the authoritative current whitepaper and
persona guidance. They contradict the current VM/microVM-only execution
contract and the step criterion that current product guidance use VM/microVM
execution. This is not a historical ADR/evolution exception: the cited
paragraphs are active abstract/principle/architecture and persona content.

**Required remediation:** update only active current claims to state that
VM/microVM is the shipped execution path and that other workload families are
future/deferred until independently delivered. Update Ana's role and VM
Service success signal accordingly. Preserve historical changed-assumption
entries, ADRs, evolution records, and E06/E08 receipts; do not make any GH
#295 topology selection or performance claim.

## Verification and unchanged surfaces

| Check | Result |
|---|---|
| VM Service `check-source` | **PASS** |
| Shell syntax for retained example scripts | **PASS** |
| Focused P-105 owner test | **PASS**, and it exposes D2's contradictory same-ID oracle |
| Historical ADR/evolution and E06/E08 diff | **PASS — unchanged** |
| HTTP/TCP current documentation/fixtures | **PASS** for retained VM Service bundle |
| Running → intercept → guest-command production ordering | **PASS by unchanged production scope** |
| GH #295 non-implementation/non-overclaim boundary | **PASS** |
| Native-metal E14 capture | Not run; owned by step 03-01 |
| Mutation testing | Not run; final wave gate only |

## Remediation disposition

All three findings are reachable through the current documentation/example
boundary or the existing production owner path. They require bounded example,
expectation, and active-guidance corrections only. No new persistence,
architecture, lifecycle owner, public API, mutation exclusion, or GH #295
mechanism is implicated.

## Iteration-1 verdict

**CHANGES_REQUESTED.** D1 is a blocking operator-artifact failure; D2 and D3
are high contract violations. The step must return to its original crafter for
remediation and a fresh re-review before DELIVER can advance to step 03-01.

## Iteration 2 — remediation re-review

### Review metadata

- **Review ID:** `code_rev_20260915_02-02_iteration_2`
- **Reviewed commits:** `e36b8eb72b6712ef7b5015168361a8e6e001e4cd`,
  `910512aea6e740521be2601426c342389c7911f2`, and remediation commit
  `2d330816cad4887c7f3fcc51b37d78b4b4eadbde`
- **Review basis:** roadmap step `02-02`, feature delta, iteration-1 findings,
  current tree at the remediation commit, and the protected historical
  surfaces named by the step contract
- **Reviewer:** fresh step-specific DELIVER reviewer

### Executive conclusion

The remediation closes the E12 identity inversion and materially repairs the
general VM fixture paths. It does not close D1 or D3, and it introduces a
mechanical diff failure. The active E10 cleanup harness still invokes a runner
that now exits successfully without validating anything; the harness therefore
fails on its residual-resource control. Active E05/O02/O01 guidance and the
verification harness still retain current Exec vocabulary, while the
authoritative whitepaper still presents a Process driver and process-backed
workload paths as live. Historical ADR/evolution prose and E06/E08 evidence
remain unchanged, and no GH #295 mechanism entered the remediation.

**Final verdict: CHANGES_REQUESTED.** DELIVER must return this step to the
original crafter for bounded artifact cleanup and another independent review.

### Finding dispositions

#### D1 — NOT CLOSED: active consumers and harnesses still retain retired Exec paths

The remediation correctly restores VM-shaped current fixtures:

- `examples/dns-resolver.toml`, `examples/quick-bind-service.toml`, the three
  liveness fixtures, and both dial-by-name service specs now carry `[vm]` and
  checked-in VM kernel/rootfs fields.
- `verification/expectations/INDEX.md:21,25` marks E07 and E10 out of scope,
  and the E07/E10 runners explicitly report retirement. E06/E08 evidence is
  byte-for-byte unchanged.

The current consumer boundary is still inconsistent, however:

1. `verification/harness/test-e10-cleanup-oracle.sh:35-40,84-91` still calls
   the retired E10 runner as the cleanup/probe oracle. Running
   `bash verification/harness/test-e10-cleanup-oracle.sh` against the
   remediation prints the retirement message three times and then fails with
   `E10 cleanup oracle accepted a residual allocation cgroup`. The runner's
   unconditional success means the residual-resource and probe-preservation
   controls no longer execute; this is both an active consumer failure and a
   testing-theater regression.
2. `verification/README.md:112,122` still presents
   `test-e07-session-lifecycle.sh` as a current harness and a runnable E07
   command, even though E07 is now explicitly out of scope. The harness still
   sources the host-process bundle's `session-lifecycle.sh` at
   `verification/harness/test-e07-session-lifecycle.sh:11,21,111`.
3. The retained current E05/O02/O01 artifacts are not fully migrated: E05's
   active pending README still says the dial-by-name specs are `[exec]` and its
   runner comment still names `ExecDriver` (`verification/expectations/E05...`),
   O02 still recommends a future `exec` probe and describes it as the only
   truthful production mechanic (`verification/expectations/O02.../README.md:58-65`),
   and O01 still creates three `[exec]` tables in its current runner
   (`verification/expectations/O01.../runner.sh:13,26,38`). These are current
   expectation/runner inputs, not immutable E06/E08 receipts, and they either
   need VM/generic migration or explicit retirement.

**Required remediation:** retire or migrate the active E07/E10 harness entry
points without modifying their historical evidence; update the remaining
current E05/O02/O01 expectation prose and inputs to the VM/generic contract (or
mark a genuinely host-process-specific expectation out of scope); and rerun the
cleanup harness so its negative controls are live. Do not preserve the old
runner as an unconditional-success stub.

#### D2 — CLOSED: E12 now asserts a distinct fresh P-105 successor

`examples/service-kind-vm-workloads/run-example.sh:866-874` captures the
replacement allocation and requires `replacement_alloc != original_alloc`.
The ledger records the predecessor on the `before`/`terminal` rows and the
distinct replacement on the `after` row (`:917-933`), while retaining the
liveness attribution, prior terminal observation, restart count, and timestamp
non-reuse checks. The focused production-owner test and composed P-105 property
from iteration 1 remain untouched; the remediation contains no Rust/API
changes. This matches the accepted predecessor-ID / fresh-successor-ID
contract without changing lifecycle production behavior.

#### D3 — NOT CLOSED: active whitepaper still claims host-process execution

The persona correction is present: Ana's role is VM/microVM-backed and its VM
Service success signal no longer compares against a host-process Service
(`docs/product/personas/ana-platform-engineer.yaml:37,130`). The whitepaper
still contains active current claims that contradict the VM/microVM-only step:

- the architecture diagram still includes a live `Process Driver`
  (`docs/whitepaper.md:138-142`);
- the current workload-driver section still opens with “every workload type as
  a first-class citizen” (`docs/whitepaper.md:505-524`);
- current right-sizing/scale-to-zero prose still says “Process-driver
  workloads opt out” (`docs/whitepaper.md:1756-1760`);
- current storage prose still describes a “process workload” sharing a volume
  with a VM (`docs/whitepaper.md:1990-1996`); and
- the active schematic example enables `process = true`
  (`docs/whitepaper.md:2748-2758`).

These are not historical ADR/evolution records and are not framed as
superseded assumptions. Updating only the abstract, principle, VM table, and
mTLS paragraphs did not remove the active host-process execution claims.

**Required remediation:** update the remaining active whitepaper diagram,
workload-driver framing, right-sizing/storage claims, and schematic example to
state the shipped VM/microVM path; describe other families only as future work.
Keep historical changed-assumption prose, ADRs, evolution records, and E06/E08
receipts unchanged. No GH #295 topology or performance claim is needed.

#### S1 — HIGH: remediation commit fails the repository diff check

`git diff --check 2d330816^ 2d330816` reports added blank lines at EOF in:
`examples/dial-by-name-responder/a.toml`, `b.toml`, `ping_pong.py`,
`examples/dns-resolver.toml`,
`examples/guest-stack-transparent-mtls-intercept/session-lifecycle.sh`,
`session-wrapper.sh`, `examples/liveness-absent-service.toml`, and
`examples/quick-bind-service.toml`. This is a direct structural failure in the
remediation diff and must be cleaned before approval.

### Contract and scope verification

| Check | Result | Evidence |
|---|---|---|
| D1 general fixtures use VM shape | **PASS** | All restored UDP, dial-by-name, probe, and liveness specs inspected; `[vm]`, kernel, rootfs, and VM command are present. `examples/service-kind-vm-workloads/prepare.sh check-source` exits 0. |
| D1 active E07/E10 retirement boundary | **FAIL** | E07 README/runner and E10 README/runner are marked out of scope, but `verification/README.md` and `test-e07-session-lifecycle.sh` remain active; `test-e10-cleanup-oracle.sh` still calls the no-op E10 runner and fails its residual control. |
| D1 current expectation consumers | **FAIL** | E05/O02/O01 retain current `[exec]`, `ExecDriver`, or exec-probe vocabulary in live README/runner paths. |
| D2 E12 successor identity | **PASS** | Distinct-ID assertion at `run-example.sh:866-869`; predecessor/successor ledger at `:917-933`; no production Rust/API changes in remediation. |
| D3 Ana persona | **PASS** | VM/microVM role and VM-only success signal at `ana-platform-engineer.yaml:37,130`. |
| D3 active whitepaper | **FAIL** | Process Driver diagram, first-class framing, process scale-to-zero/volume claims, and `process = true` schematic remain at the cited lines. |
| Historical ADR/evolution and E06/E08 | **PASS** | `git diff --quiet 2d330816^ 2d330816 -- docs/evolution docs/product/architecture verification/expectations/E06-vm-job-deploy-reaches-running verification/expectations/E08-vm-service-guest-health`. |
| GH #295 boundary | **PASS** | No shared-switch, per-tap, shared-bridge, replacement-mTLS, cross-host, density, or throughput terms occur in the remediation diff. |
| Shell/example structural checks | **PARTIAL** | `bash -n` and VM `check-source` pass; `git diff --check` fails on eight added EOF blank lines. |
| Expectation harness smoke checks | **PARTIAL** | `verification/harness/test-run-expectation.sh` passes; E10 cleanup harness fails as described. E07 host-safe harness is not a product-path signal on this macOS sandbox because `ps` is unavailable/denied. |
| Native-metal E14 / mutation gate | **NOT RUN** | Owned by step 03-01 and final DELIVER gate respectively. |

### Review conclusion

D2 is closed. D1 remains a blocking current-artifact/harness failure, D3
remains a high active-documentation contract failure, and S1 is a direct
mechanical quality-gate failure. The remediation is bounded and does not call
for architecture, public API, production lifecycle, persistence, mutation
exclusion, or GH #295 changes. Until D1, D3, and S1 are corrected and the
current E10 cleanup oracle is green, this step cannot advance to 03-01.

### Iteration-2 verdict

**CHANGES_REQUESTED.** Return to the original step crafter for remediation and
repeat the fresh isolated review. Historical evidence remains protected; no
approval is granted for the current artifact set.

## Iteration 3 — final independent re-review

### Review metadata

- **Review ID:** `code_rev_20260915_02-02_iteration_3`
- **Reviewed commits:** `e36b8eb72b6712ef7b5015168361a8e6e001e4cd`,
  `910512aea6e740521be2601426c342389c7911f2`, remediation commit
  `2d330816cad4887c7f3fcc51b37d78b4b4eadbde`, remediation commit
  `183cfd40f1135b75650908fa20decbf357f4e7a3`, final remediation commit
  `f1de51be31631f7c4954ffda62dc640f9a73dfa3`, and current review tree at
  `HEAD` `71c23f52a13646fa8c17e3950628d32363fe9265`
- **Review basis:** roadmap step `02-02`, feature delta and DISTILL P-105
  contract, iterations 1–2, the current implementation/example tree, and
  protected historical E06/E08 and ADR/evolution surfaces
- **Reviewer:** fresh step-specific DELIVER reviewer
- **Scope note:** no implementation files were changed and mutation testing was
  not run.

### Executive conclusion

The final remediation closes the E05/O01/O02 active-fixture vocabulary,
whitepaper-current-guidance, and whitespace findings; the `run-example.sh`
E12 assertion also requires a distinct successor. The step is not yet
approvable at the complete current artifact boundary:

1. The active E12 expectation README and runner still require and describe a
   same-ID replacement, so the current fresh-successor product journey and its
   current black-box oracle disagree.
2. E10's runner and cleanup harness now only print retirement messages and exit
   `0`. `run-expectation.sh` therefore records a successful execution for a
   retired, non-validating runner; the E07 host-safe harness and its catalogue
   references also remain active after E07 was marked out of scope.

The current VM source check, VM-shape examples, E05/O01/O02 corrections,
current whitepaper guidance, complete step-range diff check, protected
historical surfaces, and GH #295 non-implementation boundary pass. The
unresolved D1/D2 contracts are bounded artifact corrections, but they prevent
advancement to step 03-01.

### Finding dispositions

#### D1 — NOT CLOSED: retired E10/E07 execution entry points still report success

The E10 implementation no longer dispatches `http-status-cross-driver`, and
the current E10 cleanup harness no longer invokes the deleted matrix. However,
both active entry points are unconditional-success stubs:

- `verification/expectations/E10-vm-service-http-cross-driver-status/runner.sh:1-6`
  prints a retirement message and exits `0`.
- `verification/harness/test-e10-cleanup-oracle.sh:1-6` does the same.

Direct execution at `HEAD` produced the two retirement messages with exit `0`.
The repository harness maps runner exit `0` to `execution_status: succeeded`
(`verification/harness/run-expectation.sh:120-139,170-175`), so a caller can
still create a successful E10 execution receipt without any product or cleanup
assertion. This is the exact testing-theater gap identified in iteration 2;
renaming the no-op output did not retire the executable success path.

The E07 boundary is also incomplete. `verification/README.md:112,122` still
lists `test-e07-session-lifecycle.sh` as a current harness,
`verification/harness/test-e07-session-lifecycle.sh:11,21,111` still sources
the host-process session fixture, and a direct invocation fails with
`could not capture the direct wrapper child` (exit `1`) on this host. The E07
expectation runner is marked out of scope, but its host-process harness remains
an active catalogue entry point.

**Required remediation:** retire/remove the E10 and E07 executable harness
entry points (or make them fail closed as out-of-scope/pending rather than
success), remove their current catalogue/harness listings, and preserve the
historical E07/E10 evidence trees unchanged. Do not recreate the cross-driver
matrix or add a replacement mechanism.

#### D2 — NOT CLOSED: the current E12 expectation still asserts same-ID replacement

The product example is corrected: `examples/service-kind-vm-workloads/run-example.sh:866-869`
requires `replacement_alloc != original_alloc`, and its ledger records the
predecessor on `before`/`terminal` and the distinct successor on `after`
(`:917-933`).

The active E12 operator-facing expectation was not corrected with it:

- `verification/expectations/E12-vm-service-liveness-restart-describe/README.md:15`
  says the ledger records the same allocation identity before, at, and after
  restart; `:40-41` calls the replacement “same-ID”.
- `verification/expectations/E12-vm-service-liveness-restart-describe/runner.sh:95`
  rejects any fresh successor by requiring all three row IDs to be equal, and
  `:109-110` requires the stale same-ID PASS string.

The current production-owner acceptance evidence already proves the inverse
contract: a liveness replacement retains the predecessor in the restart action
and reserves a distinct successor. The existing E12 receipt at
`evidence/verification.yaml` is a historical SHA-pinned capture whose ledger
does show the old same-ID behavior; it must remain immutable and must not be
used as evidence for the current post-cut contract. As written, a future
current E12 capture would fail its own runner against the corrected
`run-example.sh` output.

**Required remediation:** update only the current E12 expectation prose,
ledger predicate, and PASS wording to require the accepted
predecessor/fresh-successor identity while preserving all liveness, terminal,
timestamp, and cleanup assertions. Keep the old E12 evidence receipt
immutable and do not change production lifecycle behavior.

#### D3 — CLOSED: active whitepaper now states the VM/microVM boundary

The final remediation corrects the remaining active current guidance:

- `docs/whitepaper.md:139-143` labels the shipped VM driver and marks
  Unikernel/WASM as future in the architecture diagram.
- `docs/whitepaper.md:508-526` makes the shipped VM/microVM path the current
  driver boundary and describes other families as future work.
- `docs/whitepaper.md:2762-2766` removes `process = true` and enables the
  shipped VM/microVM drivers while marking other drivers future.

The earlier scale-to-zero/storage corrections remain present at
`docs/whitepaper.md:1760-1762,1996-2000`, and the Ana persona remains VM-only.
No historical ADR/evolution prose or E06/E08 receipt changed, and no GH #295
topology or performance claim entered the remediation. D3 is closed.

#### S1 — CLOSED: the complete step diff passes `git diff --check`

The final remediation removes the eight added EOF blank lines identified in
iteration 2. Both `git diff --check 2d330816^ f1de51be` and the complete
implementation/remediation range `git diff --check e36b8eb7^ f1de51be` exit 0.
S1 is closed.

### Contract and scope verification

| Check | Result | Evidence |
|---|---|---|
| Current healthy VM Service source-valid | **PASS** | `bash examples/service-kind-vm-workloads/prepare.sh check-source` exits 0; it validates positive VM shape without a deleted-spelling absence assertion. |
| Retained example shell syntax | **PASS** | `bash -n` passes for the retained VM runner/preparation scripts and restored example helper scripts. |
| E12 product runner successor identity | **PASS** | `run-example.sh:866-869` rejects same-ID replacement; ledger rows at `:919-931` retain predecessor history and emit the successor. |
| E12 current expectation oracle | **FAIL** | E12 README `:15,40-41` and runner `:95,109-110` still require same-ID replacement. |
| E10/E07 retirement boundary | **FAIL** | E10 runner and cleanup harness both exit 0 without assertions; E07 remains listed and its host-process harness is active and fails direct execution. |
| E05/O01/O02 current vocabulary | **PASS** | E05 uses `[vm]` current specs and labels the host-process sketch historical; O01 generates only `[vm]` specs; O02 limits the active mechanic check to HTTP/TCP and calls host-process mechanics historical. |
| Active whitepaper VM/microVM guidance | **PASS** | Final remediation marks the diagram's non-VM families future, makes the driver framing VM/microVM-only, and sets the schematic to `vm=true`, `microvm=true`, future `unikernel`, and `wasm=false`. |
| Historical ADR/evolution and E06/E08 | **PASS** | `git diff --quiet e36b8eb7^ f1de51be -- docs/evolution docs/product/architecture verification/expectations/E06-vm-job-deploy-reaches-running verification/expectations/E08-vm-service-guest-health` exits 0. |
| GH #295 boundary | **PASS** | The remediation contains no implementation/selection claim for shared switching, per-tap interception, shared DNS, replacement mTLS, cross-host routing, density, or throughput; the one deferred-boundary mention remains explicitly non-selecting. |
| Complete step diff whitespace | **PASS** | `git diff --check 2d330816^ f1de51be` and `git diff --check e36b8eb7^ f1de51be` both exit 0. |
| Native-metal E14 / mutation gate | **NOT RUN** | E14 belongs to step 03-01; mutation testing is the final DELIVER-wave gate. |

### Review conclusion

D2 is only closed in the product example, not in the current E12 expectation
oracle. D1 remains a blocking retirement/testing-theater failure. D3 and S1
are closed by f1de51be. The remaining remediation is bounded to current
expectation/harness retirement and E12 oracle wording; it does not require
architecture, public API, production lifecycle, persistence, mutation
exclusion, or GH #295 changes.

### Iteration-3 verdict

**CHANGES_REQUESTED.** Return step 02-02 to the original crafter for bounded
remediation and repeat the independent review. Do not advance to step 03-01
until D1 and D2 are closed; D3 and S1 are already closed and the complete
step-range diff check is clean.

## Iteration 4 — final approval review

### Review metadata

- **Review ID:** `code_rev_20260915_02-02_iteration_4`
- **Reviewed commits:** the prior reviewed range through `f1de51be31631f7c4954ffda62dc640f9a73dfa3`, review commit `71c23f52a13646fa8c17e3950628d32363fe9265`, and current remediation commit `028033556472d5790acef651b3bdf81797791904` (`HEAD`)
- **Review basis:** roadmap step `02-02`, feature delta and DISTILL P-105 contract, iterations 1–3, current operator/expectation artifacts, and protected historical evidence
- **Reviewer:** fresh step-specific DELIVER reviewer
- **Scope note:** no implementation files were changed or reviewed as remediation; no mutation run was performed. This review covers the bounded artifact and verification-entry-point corrections only.

### Executive conclusion

The latest remediation closes both remaining iteration-3 findings. E12 now
requires the predecessor to remain the `before`/`terminal` identity and a
distinct fresh successor after restart in both its current README and runner.
The retired E10 product runner and cleanup oracle both fail closed with a
nonzero exit, and the obsolete E07 host-safe harness has been deleted along
with its verification README entry. Current E05/O01/O02 artifacts contain no
active Exec-driver or host-process execution claim; the one O02 reference
explicitly labels host-process mechanics historical. The active whitepaper and
Ana persona remain VM/microVM-only, and the complete step-range diff is clean.

Protected historical evidence remains unchanged, including the E06, E08, E07,
E10, and E12 evidence trees and the architecture/evolution records. No
GH #295 topology, persistence, API, lifecycle-owner, or mutation-scope change
was introduced.

### Finding dispositions

#### D1 — CLOSED: retired E10/E07 entry points no longer report successful evidence

The current E10 runner at
`verification/expectations/E10-vm-service-http-cross-driver-status/runner.sh:1-7`
prints its explicit pre-cut retirement message to stderr and exits `2`. The
current E10 cleanup oracle at
`verification/harness/test-e10-cleanup-oracle.sh:1-7` has the same fail-closed
shape and exits `2`. Direct execution of both scripts produced the retirement
message and returned the recorded nonzero status, so the expectation harness
cannot turn either retired path into a successful execution receipt.

The obsolete E07 host-safe harness is absent, and its command is absent from
the harness layout and invocation section in `verification/README.md:109-121`.
The E07 and E10 catalogue rows and evidence remain as explicitly out-of-scope,
historical records; they are not current executable entry points.

#### D2 — CLOSED: current E12 oracle enforces predecessor plus fresh successor

The current expectation README states that the ledger retains the predecessor
identity before and at the terminal liveness observation and records a distinct
fresh successor (`verification/expectations/E12-vm-service-liveness-restart-describe/README.md:15-22`).
Its verification contract requires a fresh ordinary successor
(`README.md:40-43`). The runner enforces the same contract at
`runner.sh:93-100`: `before == terminal`, `terminal != after`, distinct startup
and allocation timestamps, and preserved threshold evidence. The current PASS
line at `runner.sh:109-110` names the fresh-successor result. The historical E12
receipt remains byte-for-byte unchanged.

#### D3 — CLOSED: active guidance remains VM/microVM-only

The whitepaper architecture diagram labels the shipped VM and MicroVM drivers
and marks Unikernel/WASM as future (`docs/whitepaper.md:138-143`). The workload
driver section identifies VM/microVM as the current shipped path and defers
future families (`docs/whitepaper.md:506-526`); the scale-to-zero and storage
sections likewise describe supported VM workloads (`docs/whitepaper.md:1762-1764,1998-2002`).
The schematic enables only shipped VM/microVM drivers and marks future drivers
false (`docs/whitepaper.md:2762-2766`). Ana's persona is VM/microVM-backed and
its success signal is guest HTTP/TCP health (`docs/product/personas/ana-platform-engineer.yaml:37,129-130`).

#### S1 — CLOSED: mechanical diff remains clean

`git diff --check e36b8eb7^ HEAD` exits `0`; no whitespace defect remains in the
complete step range.

### Contract and scope verification

| Check | Result | Evidence |
|---|---|---|
| E12 README and runner predecessor/fresh-successor contract | **PASS** | README `:15-22,40-43`; runner `:93-100,109-110`; direct inspection confirms `before == terminal` and `terminal != after`. |
| E10 runner fail-closed retirement | **PASS** | Direct `bash verification/expectations/E10-vm-service-http-cross-driver-status/runner.sh` returned `2`. |
| E10 cleanup oracle fail-closed retirement | **PASS** | Direct `bash verification/harness/test-e10-cleanup-oracle.sh` returned `2`. |
| E07 harness retirement boundary | **PASS** | `verification/harness/test-e07-session-lifecycle.sh` is deleted and its entry/invocation are absent from `verification/README.md`. |
| E05/O01/O02 active Exec claims | **PASS** | No `[exec]`, `ExecDriver`, `host-process`, `process-driver`, or `Process Driver` claim occurs in the current E05/O01/O02 README/runner set; O02's sole host-process mention is explicitly historical. |
| Current VM source and runner syntax | **PASS** | `bash examples/service-kind-vm-workloads/prepare.sh check-source` and `bash -n` over current E05/E07/E10/E12/O01/O02 runners exit `0`. |
| Active whitepaper/persona VM/microVM guidance | **PASS** | Current whitepaper and Ana lines cited above contain no active process-driver claim. |
| Protected historical evidence | **PASS** | `git diff --quiet e36b8eb7^ HEAD -- docs/evolution docs/product/architecture verification/expectations/E06-vm-job-deploy-reaches-running verification/expectations/E08-vm-service-guest-health verification/expectations/E07-vm-job-calls-exec-service/evidence verification/expectations/E10-vm-service-http-cross-driver-status/evidence verification/expectations/E12-vm-service-liveness-restart-describe/evidence` exits `0`. |
| Complete step-range whitespace | **PASS** | `git diff --check e36b8eb7^ HEAD` exits `0`. |
| Native-metal E14 / mutation gate | **NOT RUN** | E14 belongs to step 03-01; mutation testing remains the final DELIVER-wave gate. |

### Review conclusion

D1, D2, D3, and S1 are closed. The current retired entry points fail closed,
the current E12 oracle matches the accepted P-105 predecessor/fresh-successor
contract, and historical evidence is preserved. The implementation and
artifact boundary for step 02-02 is approved without architecture or production
code expansion.

### Iteration-4 verdict

**APPROVED.** Step 02-02 may advance to step 03-01. The final DELIVER-wave
mutation gate remains required at the wave boundary; it is not a prerequisite
for this documentation/example review.
