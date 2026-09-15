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
