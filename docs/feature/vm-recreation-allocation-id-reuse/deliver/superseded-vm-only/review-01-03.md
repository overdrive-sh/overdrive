# DELIVER Review — VM recreation allocation identity, step 01-03

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-03` — Prove native artifact ownership |
| Commit | `d4db67f6f34b52244ae3a5f215bfc08ef03f4ecc` |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Review date | 2026-09-13 |
| Iterations | 1 |
| Final verdict | **APPROVED** |

## Review scope and authority

This review covers the three files changed by commit `d4db67f6` and the six
step-owned, qualified-metal test bodies:

- `crates/overdrive-cli/tests/integration/vm_reclamation_tier3.rs` —
  S-VM-81 and S-VM-28;
- `crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs` —
  S-VM-48 and S-284-METAL-01; and
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs` —
  S-GTI-06a and S-GTI-06b.

The authority set is accepted ADR-0104, the accepted feature delta, approved
roadmap step 01-03, `AGENTS.md`, `CLAUDE.md`, and the repository's Rust,
testing, debugging, design, development, BPF, and verification rules. The
review preserves the pre-existing dirty `AGENTS.md`, the untracked DES log,
and the already-written step 01-01 and 01-02 review artifacts. No production
or test source was edited by this reviewer; this Markdown artifact is the only
new review output.

## Design-contract checklist

| Contract | Result | Evidence |
|---|---|---|
| Stable logical owner remains `WorkloadId` / `WorkloadLifecycle` | PASS | The commit has no production change and introduces no owner, aggregate, lifecycle state, or compatibility identity. |
| Platform Reclamation retains the predecessor and publishes a fresh VM row | PASS | S-VM-81 and S-VM-28 require a `Terminated / Stopped { by: PlatformReclaimed }` predecessor and a distinct `Running` row with `restart_count == 0` and `last_terminated == None` (`vm_reclamation_tier3.rs:1267-1298`, `1404-1435`). |
| Fresh allocation identity is observed through existing ports | PASS | The tests call `deploy`, `describe`, and `stop` handlers against the real `run_server` composition; they do not construct rows or invoke a new API. |
| SVID and mesh guard behavior | PASS | S-VM-81 boots mTLS-composed serve and checks issued-certificate coverage (`:1218-1239`, `:1295-1298`). S-GTI-06a ties the exact fresh allocation to `poll_until_issued_identity`, observes the D7 rule and kTLS peer wire, and rejects pre-readiness guest frames (`guest_stack_mtls_egress.rs:4088-4220`, `4376-4540`). |
| Clean clone and unchanged operator master | PASS | S-VM-48's marker guest writes and rereads the first clone, then only reaches fresh replacement `Running` when the marker is absent; it fingerprints the operator master before and after (`vm_stop_restart_and_vmm_death.rs:1587-1712`). |
| Typed re-enrolment failure remains closed | PASS | S-GTI-06b requires the fresh row, `MtlsInterceptInstallFailed { stage: "outbound_tproxy_install", ... }`, zero `EXEC`, zero guest-originated frames, and cleanup of the captured VMM (`guest_stack_mtls_egress.rs:4543-4684`). |
| Two real VMMs have distinct successful beacon ownership | PASS | S-284-METAL-01 records two successful real `CloudHypervisorVmm::create` results, asserts two allocation IDs, and independently matches successful predecessor and replacement `bind` syscalls (`vm_stop_restart_and_vmm_death.rs:1765-1795`, `2217-2333`, `2101-2151`). |
| New creation precedes predecessor cleanup | PASS | The second real create is held after success; only after its artifact family is present does the test advance the existing reclamation clock, wait for old-only disposal, and verify replacement survival (`:2335-2346`). The retained strace asserts the same ordering. |
| Old/replacement artifact and process isolation | PASS | The metal universe contains run-dir descendants, beacon/API/vsock/console/kernel paths, clone/index, cgroup, and each captured PID. Replacement presence is asserted while old cleanup runs; both families and both PIDs are absent after operator cleanup (`:1987-2095`, `2366-2388`). |
| No new public surface or architecture | PASS | `git show --stat` contains only test files. No action, type, field, trait, parameter, persistence surface, retry, sleep, lock, cleanup mechanism, or expectation was added to production. |
| Qualified-metal boundary | PASS | All three modules are gated by `integration-tests,kvm-tests`; the nextest `host-kernel-shared` group matches each module. Lima is used only for compile/clippy checks. The metal command is the only execution evidence for the real guest tests. |

## Production composition and reachability audit

The active bodies are vertical slices through existing owners, not fixture-only
reconstructions. `commands::serve::run_with_dataplane` and `run_with_kek` call
the existing `run_inner` composition (`crates/overdrive-cli/src/commands/serve.rs:155-186`),
which calls `run_server`; the mTLS tests use the existing
`run_with_kek_and_vmm_override` boundary (`:199-206`) only to decorate the
real VMM. `run_server` composes the real cgroup/worker, `VmDriver`, driver
registry, reconcilers, boot reclamation, and convergence loop
(`crates/overdrive-control-plane/src/lib.rs:1608-1765`, `2208-2219`,
`2815-2823`, `3063-3069`).

The two reclamation tests use real Cloud Hypervisor and real cgroup/filesystem
surfaces; S-VM-81 additionally leaves the real `EbpfDataplane` and SVID
composition enabled. S-VM-48 uses only the already-established Sim dataplane
port override because its claim is VM rootfs behavior. S-GTI-06a/b leave real
mTLS/dataplane composition enabled and replace only the VMM port with an
observation/failure decorator around real Cloud Hypervisor. S-284-METAL-01
uses the same real VMM inside `AllocationOwnershipVmm`, `RealCgroupFs`, and
the real filesystem, Unix-socket, cgroup, process, and `strace` surfaces.

Every operator action enters through the existing direct Rust handlers. The
tests do not spawn the built Overdrive binary, invoke a verification
expectation, import an `overdrive-*` crate from a black-box runner, fabricate
an `AllocStatusRow`, or install a missing production network/artifact effect.
Cloud Hypervisor and guest binaries are legitimate Tier-3 child fixtures.

## Qualified-metal evidence

The supplied qualified-metal control run executed the exact six-test selection
from roadmap step 01-03 on a native, non-virtualized x86_64 KVM host and passed
6/6. The run used `cargo xtask metal run --`, not Lima; no Lima execution is
claimed for the KVM-dependent bodies. The retained raw syscall capture is:

`/var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789290158143227525-1773360/strace.raw`

The capture is produced by `AllocationOwnershipStrace::attach` before server
and child creation, stored under a unique mode-0700 directory, and flushed
after both allocations and their cleanup owners finish
(`vm_stop_restart_and_vmm_death.rs:1822-1894`). The test's final oracle is
independent of Rust row assertions: it requires successful bind lines for both
allocation-derived beacon paths, rejects `EADDRINUSE` on the replacement
path, and requires an old-ID `unlink`/`rmdir` after a line carrying the
replacement identity (`:2101-2162`). The live assertions independently require
the replacement's complete artifact family and PID while predecessor cleanup
runs, followed by exact artifact and PID absence for both captures.

This reviewer independently confirmed the six bodies compile and are selected
as non-ignored tests with the Lima-routed `nextest list`; real execution of
those bodies remains the supplied native-metal evidence, as required by the
roadmap and qualified-host boundary.

## Strengths

`praise:` The step keeps the evidence lanes unusually clear: the real
Cloud-Hypervisor/host effects stay in qualified metal, while the direct CLI
handlers and production owners remain the system under test. The metal test's
held second-create cut plus retained raw `strace` gives an independently
observable ownership order instead of relying on a timing explanation.

## Fixture-correction equivalence audit

The test-authoring commit `ca09f160` supplied the complete pre-authored bodies;
the step parent `e7fd61c7` is the accepted 01-02 owner-path proof. The reviewed
commit's source diff is exactly:

| File | Change | Disposition |
|---|---|---|
| `vm_reclamation_tier3.rs` | Remove the two exact `pending DELIVER step 01-03` ignores | Authorized activation only; bodies, rows, history checks, and cleanup remain unchanged. |
| `guest_stack_mtls_egress.rs` | Remove the two exact `pending DELIVER step 01-03` ignores | Authorized activation only; SVID, guard, wire, typed-error, guest-boundary, and cleanup oracles remain unchanged. |
| `vm_stop_restart_and_vmm_death.rs` | Remove S-VM-48 and S-284-METAL-01 ignores; change one helper delay; move/add the predecessor presence assertion | The delay is a fixture correction, not production behavior: `run_server`'s production probe owner uses its separate `SystemClock`, so each logical clock tick must allow the existing one-second startup-probe interval. The old artifact family is now asserted at the predecessor's `Running` boundary, before the Cloud Hypervisor terminal transition removes its vsock. The expected artifact contents/order, strace oracle, replacement barrier, rows, final complements, and bounded waits are retained. |

The before/after diff contains no deleted assertion, changed expected value,
weakened predicate, skipped body, altered seed, changed driver composition, or
new fixture state. The 25ms-to-one-second change is bounded by the existing
10 logical-second loop and 30-second reclamation cadence; it supplies wall time
for the real `ProbeRunner` without changing the production clock or its
behavior. Capturing `old` before the failure progression makes the existing
artifact-presence assertion observe the valid pre-terminal state; the later
replacement-survival and final exact-family assertions are unchanged in
meaning and remain in place.

## Contract Shape and test-quality review

All six transitioned functions carry the exact required
`/// CONTRACT_SHAPE: bounded-change.` line, and none matches the banned
implementation-detail test-name patterns. The test bodies use finite,
domain-named universes and assert observable rows, host paths, processes,
guest boundary records, packet captures, kTLS records, and typed outcomes.
There are no domain-layer unit tests, mocks inside the hexagon, fully mocked
SUTs, zero-assertion tests, tautological assertions, or expectation runners.

The six scenario mappings are six distinct fixed Tier-3 behaviors, giving a
test-budget ceiling of `6 × 2 = 12`; six active step-owned tests are within
budget. The one-active-acceptance rule is not applicable because the approved
roadmap explicitly batches these six real-I/O regression bodies in one step.

The S-GTI success body supplies the identity-specific replacement-SVID proof
through `poll_until_issued_identity`; the weaker S-VM-81 non-empty certificate
check is only its real SVID-holder precondition and is not relied on as the
sole fresh-identity oracle. The accepted design deliberately keeps
`IdentityMgr` opaque and adds no test accessor. No unproven concern about
that existing boundary was promoted to a finding.

## DES, commit, and verification evidence

| Check | Result | Evidence |
|---|---|---|
| Roadmap readiness | PASS | `roadmap.json:349-353` is approved by the independent architect review with the exact 21/21 scenario mapping. |
| DES phase order | PASS | `execution-log.json` records 01-03 `RED EXECUTED PASS`, initial `GREEN EXECUTED FAIL`, resumed `GREEN EXECUTED PASS`, then `COMMIT EXECUTED PASS`, chronologically. `des-verify-integrity` independently reports all three steps have complete traces. |
| Commit scope | PASS | `git show --stat d4db67f` reports exactly the three roadmap test files. |
| Attribution | PASS | Author and committer remain Marcus Schack Abildskov; the message has exactly one `Co-Authored-By: Codex <codex@openai.com>`, one `Step-Id: 01-03`, and no Claude, Anthropic, or generated-by attribution. |
| Diff hygiene | PASS | `git diff --check d4db67f^ d4db67f` is clean. |
| Test integrity | PASS | Only the six authorized ignore markers and the documented qualified-metal fixture corrections changed; all expected assertions and bounded universes remain. |
| Mutation testing | PASS — not run | Individual-step mutation testing is expressly prohibited; the final DELIVER-wave gate remains pending until all step reviews complete. |

The reviewer independently ran the following non-metal checks through the
required Lima boundary:

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo check -p overdrive-cli --all-targets --features integration-tests,kvm-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-cli --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS |
| `cargo xtask lima run -- cargo fmt --all -- --check` | PASS |
| `PYTHONPATH=/Users/marcus/.claude/lib/python cargo xtask dst-lint` | PASS |
| `cargo xtask lima run -- cargo nextest list ...` with the exact six-test expression | PASS — all six are compiled and listed as active; no KVM execution was claimed in Lima |

## Findings

| ID | Severity | Reachability / evidence | Remediation disposition |
|---|---|---|---|
| None | — | No proven, reachable, in-scope defect remains. The only source changes beyond marker activation are the documented fixture corrections, and the supplied native-metal run passed the exact six-test selection 6/6. | No remediation required. |

Static concerns that would require changing a pre-authored oracle (for example,
adding a new `IdentityMgr` test surface or broadening the artifact universe)
were not promoted: they are either already covered by the exact S-GTI identity
oracle or require an unauthorized API/architecture expansion, and no failing
current-production regression demonstrates necessity.

## Iteration history

| Iteration | Target | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | `d4db67f6f34b52244ae3a5f215bfc08ef03f4ecc` | **APPROVED** | None | No remediation; step is complete. |

## Final verdict

**APPROVED.** Step 01-03 activates exactly the six authorized qualified-metal
regressions, preserves their authored behavior and bounded universes, drives
the real production owners, and has the required native 6/6 evidence. The
fresh-row/history, SVID and mesh, rootfs, typed-failure, beacon-order,
artifact-isolation, and final artifact/PID obligations are covered without
new API or architecture. DES, attribution, scope, Lima compile/clippy,
formatting, and `dst-lint` evidence are complete. The step may advance to the
final DELIVER-wave gate.
