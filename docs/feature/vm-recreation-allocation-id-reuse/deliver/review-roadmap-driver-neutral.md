# Driver-neutral DELIVER roadmap review

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Artifact reviewed | `docs/feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json` |
| Review role | Independent DELIVER-roadmap reviewer |
| Iteration | 1 |
| Date | 2026-09-13 |
| Reviewed repository state | `730b7a397c55050c7d19d254a17c5d79efc9d794` plus the untracked corrective roadmap and initialized execution log |
| Result | `CHANGES_REQUESTED` |

The roadmap validation remains `pending`. This review did not edit production
code, acceptance bodies, the execution log, the superseded VM-only archive,
PR state, or `AGENTS.md`.

## Scope

The review checked the roadmap against the user-ratified P-105-1 through
P-105-7 contract in the canonical feature delta, active ADRs 0105, 0106, 0108
and 0109, withdrawn ADR-0107, historical/rejected ADR-0104, the corrective
DESIGN review record, and the re-DISTILL acceptance inventory. It also checked
the current `Action::RestartAllocation` declaration, the same-ID
`WorkloadLifecycle` baseline, the cleanup-first action-shim arm, every mapped
Rust executable and pending marker, and the roadmap-review/TDD rules.

The review does not reconsider the approved mechanism or promote any adjacent
cleanup, retry, persistence, network, scheduling, or host-hardening proposal.

## Scenario mapping audit

The two roadmap representations agree exactly: the per-step `scenario_ids`
set and top-level `scenario_mappings` set each contain the same 29 unique
re-DISTILL IDs, with no duplicate or omitted ID.

| Step | Scenario IDs | IDs | Executable bodies | Current activation state |
|---|---|---:|---:|---|
| `01-01` | `S-284-PURE-01..10`, `S-284-PROP-01`, `S-VM-26`, `S-VM-27`, `S-284-CORE-P1` | 14 | 14 | 14 pending |
| `01-02` | `S-284-SIM-01..05` excluding `SIM-05` | 4 | 4 | 4 pending |
| `01-03` | `S-284-SIM-05`, `S-284-SIM-P1..P3`, `S-284-SIM-P8` | 5 | 5 | 4 pending, `P8` active |
| `01-04` | `S-284-SIM-P4..P7` | 4 | 4 | 4 pending |
| `01-05` | `S-284-METAL-P1`, `S-284-METAL-P2` | 2 | 6 | 6 pending; `P2` intentionally aggregates five retained metal bodies |
| **Total** |  | **29** | **33** | **32 pending, 1 active** |

All 33 named functions exist. The mapped functions contain complete bodies,
not placeholder-only scaffolds. Every mapped pure/property body carries the
exact line `/// CONTRACT_SHAPE: pure-function.`; every mapped Sim/metal body
carries the declared bounded-change line; the active Service preservation
body carries `unbounded-preservation`. The legacy `S-SVM-17` rustdoc anchor on
the active function is reconciled by the canonical re-DISTILL mapping to
`S-284-SIM-P8` and does not duplicate a roadmap ID.

The mapping is complete but its delivery placement is not executable; see
F-01 through F-03.

## Contract audit

| Contract | Result | Evidence and disposition |
|---|---|---|
| Driver-neutral identity policy | PASS | `01-01` requires both Exec and VM to use the existing `RestartAllocation` predecessor/successor relation and confines driver matching to payload projection. The roadmap rejects a same-ID compatibility branch and the archived VM-only action split. |
| Exact action and port shape | PASS | The current public action remains `RestartAllocation { alloc_id, spec, kind }`; the roadmap points to the feature delta's exact helper/action meanings and blocks any new action, API, port, store, schema, error, driver-policy method, cleanup owner, retry, lock, or network mechanism. |
| Successor-first ordering | PASS | `01-02` assigns SIM-01 through SIM-04 to the current restart arm and requires predecessor fence/capability use, complete successor outcome, successor unwind where applicable, then one exact-old driver → mTLS → structural-network cleanup attempt and the ratified result precedence. |
| Cleanup scope and index preservation | PASS | The mapped 16-cell scenario owns cleanup short-circuiting and exact identities. Roadmap notes retain the existing index/allocator/port owners; accepted successor state is not rolled back for predecessor cleanup failure. |
| SystemGc preservation | PASS | `S-284-PURE-06` is mapped to `01-01`, whose criteria preserve SystemGc resubmit as fresh `StartAllocation` rather than reactivating ADR-0104's VM-only replacement branch. |
| Pure, Sim, and metal evidence lanes | FAIL for step placement | The correct bodies and substrates are named, but three lanes are delayed into validation-only roadmap steps and one runtime criterion is separated from its proving scenario. See F-01/F-02. |
| Qualified-metal boundary | PASS | Lima is compile/clippy-only. The six real-VM bodies execute only through `cargo xtask metal run` with `integration-tests,kvm-tests`; the roadmap preserves substrate probes, strace, fixtures and exact host-artifact assertions. |
| Mutation testing | PASS | The user-directed exclusion is explicit for every step and the final DELIVER gate; no mutation command or exclusion edit appears. |
| Compiler-required fallout | PASS | The accepted contract changes no public field, variant, trait, dependency feature or persisted schema. The three named production files cover the expected semantic/rustdoc cuts, while the scope rule permits only acceptance-required, documented compiler fallout. |

## Mandatory roadmap checks

| Check | Result | Assessment |
|---|---|---|
| External validity | PASS | The correction remains reachable through the existing `overdrive deploy` → `overdrive serve` handler → intent → convergence path, with composed Sim evidence and real Cloud Hypervisor evidence. No new operator surface is invented. |
| AC implementation coupling | PASS | Criteria name approved domain actions, owners and observable outcomes but do not prescribe a new signature or private decomposition. Exact private signatures remain in the feature delta, referenced rather than copied. |
| Step decomposition | **FAIL** | The nominal `5 / 3 = 1.67` ratio passes, but steps `01-03` through `01-05` are validation-only activation steps, prohibited by the roadmap methodology and incompatible with the repository's per-step DES cycle. |
| Implementation code in roadmap | PASS | No implementation body or pseudocode is embedded. Ordering and checked allocation language restate ratified observable contract, not an invented algorithm. |
| Concision and precision | PASS under repository override | Step descriptions, criteria counts, criterion lengths and notes stay within their per-field limits. The extra volume is exact 29-ID traceability and runner commands; repository guidance makes such scenario specificity non-blocking. |
| Unit/test boundary | PASS | Pure tests drive the public reconciler boundary, Sim tests drive the production runtime/action-shim owners through existing ports, and metal tests retain real process/filesystem/socket/cgroup/rootfs/mesh effects. |

## Findings

### F-01 — BLOCKER: three validation-only steps cannot execute RED → GREEN → COMMIT

**Proven conflict.** Roadmap lines 50 and 137-222 deliberately put all
production changes in `01-01`/`01-02`, then define `01-03`, `01-04` and
`01-05` only as pending-marker removal plus test execution. Their
`files_to_modify` lists contain test files only. `01-03` says a non-green body
is a blocker, `01-04` says the step adds no mechanism, and `01-05` forbids
production edits.

The roadmap methodology's No-Op Prevention rule says every step must add
production code and validation belongs to the preceding step's review. The
repository additionally requires every roadmap step to run with a fresh
crafter through RED → GREEN → COMMIT. The acceptance activation rule requires
that crafter to remove its marker, obtain semantic RED, and then change
production.

Because all three steps depend on `01-02`, both possible executions are
invalid: if the two earlier production cuts satisfy the bodies, the fresh
crafter obtains GREEN rather than RED and has no production change; if a body
is still RED, the roadmap orders the crafter to stop rather than make the
production change that would turn it GREEN. Prior DISTILL RED evidence cannot
be claimed as a later isolated crafter's RED phase.

**Required disposition.** Re-decompose the plan so every roadmap step that
owns pending markers also owns the bounded production change that makes those
bodies turn from semantic RED to GREEN. Fold pure/Sim/metal validation into
the production-changing step that satisfies it, or split the two production
cuts into smaller real production slices if the existing design and bodies
support that without new API. Do not retain test-activation-only steps.

### F-02 — BLOCKER: `01-01` claims runtime fsync behavior that its own evidence cannot prove

**Proven mismatch.** Criterion 5 at roadmap line 86 requires View fsync before
dispatch, consumption without a successor effect/row, and later advancement
above the gap. `01-01` maps only pure reconciler/core bodies and runs no
`ReconcilerRuntime`/redb reopen scenario. The re-DISTILL adapter matrix assigns
that production-owner proof to `S-284-SIM-05`; the roadmap places SIM-05 in
`01-03`, after `01-01` must already have received an independent `APPROVED`
review and after the action-shim cut.

The pure tests can prove that `next_view` contains the issued key, but they
cannot prove runtime fsync-before-effect, close/reopen hydration, or requeue to
a higher unrowed successor. A step reviewer cannot approve criterion 5 on a
pending later-step test, and the strict sequence forbids advancing past an
unproved criterion.

**Required disposition.** Align each criterion with evidence executed in the
same production-changing step. Either narrow `01-01` to the pure returned-View
reservation promise and place the runtime criterion with its actual production
cut, or restructure the production steps so SIM-05 legitimately turns RED to
GREEN in the step that claims fsync/reopen behavior. Do not count a pending
future scenario as current-step evidence.

### F-03 — HIGH: none of the 32 pending markers names its activating step

**Proven mismatch.** The roadmap protocol says each step removes only its own
markers, but all 32 pending attributes still use descriptions of the form
`pending corrective re-DELIVER roadmap: ...`; zero names `01-01` through
`01-05`. This was valid before a roadmap existed, but it is not the final
DELIVER handoff. `.claude/rules/testing.md` requires every pending marker to
name the activating DELIVER step so unfinished work is discoverable from the
source plus roadmap and so a crafter cannot claim another step's body.

**Required disposition.** After fixing F-01/F-02 and stabilizing the step IDs,
bind each of the 32 pending attributes to exactly one activating step. Change
only the `#[ignore = ...]` reason; preserve every body, fixture, oracle, seed,
replay sidecar and Contract Shape declaration. The active SIM-P8 body remains
active and receives no marker.

## Verification

- `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate ...` → `VALID: 1 phases, 5 steps`.
- Independent set comparison → 29 unique feature-delta IDs, 29 unique
  per-step references, 29 unique top-level mappings, with identical sets.
- Source resolution → 28 one-body mappings plus five bodies aggregated under
  `S-284-METAL-P2`, for 33 existing Rust functions.
- Activation audit → 32 pending markers and one active preservation body;
  zero pending markers name a concrete DELIVER step.
- Contract Shape audit → all mapped pure/Sim/metal declarations match the
  canonical re-DISTILL table.
- Production-path read → current `WorkloadLifecycle` still emits same-ID
  restart and admits `Draining`; current action shim performs predecessor
  driver/mTLS/network cleanup before successor work. These are the two bounded
  production cuts named by the roadmap, not authority to add another
  mechanism.
- Superseded-roadmap comparison → no rejected VM-only `StartAllocation`
  replacement, VM-specific allocator/policy branch, or Exec same-ID
  compatibility requirement is reactivated.

No Rust test was rerun for this artifact review. The complete bodies and their
semantic RED/compile evidence are already pinned in re-DISTILL commit
`d00b1658`; the blocking findings are deterministic roadmap/activation
contradictions, not suspected production failures requiring a new regression.

## Review iteration

### Iteration 1 — 2026-09-13

The content contract, scenario inventory, test bodies, evidence substrates,
action/port preservation, driver-neutral policy, successor-first exact-old
cleanup scope, SystemGc preservation, metal gate, mutation exclusion and
compiler-fallout rule all pass. The roadmap is nevertheless not executable as
a strict DELIVER plan because three steps have no RED-to-GREEN production cut,
the first step claims runtime behavior proved only by a later pending
scenario, and pending markers do not identify their final owners.

## Verdict

`CHANGES_REQUESTED`

Leave `validation.status = pending`. Re-review is required after F-01 through
F-03 are remediated; no DELIVER step may start from this roadmap.
