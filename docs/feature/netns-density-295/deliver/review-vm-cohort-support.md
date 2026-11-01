# DISTILL review — 64-VM correctness fixture support

## Metadata

- **Feature:** `netns-density-295`
- **Review type:** fresh independent `nw-acceptance-designer-reviewer`
- **Review iteration:** 1
- **Review date:** 2026-11-01
- **Approved source parent:** `a2cd9691de4da949389cfe884a044ca9cc60367d`
- **Reviewed source:** `crates/overdrive-testing/bin/fwd_guest_suite.rs`
- **Reviewed handoff:** `docs/feature/netns-density-295/deliver/vm-cohort-support.md`
- **Preserved delivery change:** the two pending-marker removals in
  `crates/overdrive-cli/tests/integration/vsock_density.rs`
- **Final verdict:** **APPROVED**

## Review boundary and authority

This review covers only the uncommitted GuestKit fixture support that makes the
two existing 64-VM correctness bodies directly selectable inside a disposable
outer guest. It does not approve a product result, a benchmark receipt, a new
test identity, or a change to either acceptance body.

The controlling contract is roadmap step `14-05`, scenario `P6-S15`, V-9's
correctness half, R5-25, R5-55 (a), (d), (e), and NO-SURVIVE in
`feature-delta.md`. U-2 keeps every density/cost measurement in GH #326. The
review also applies the repository's exact-API, typed-error, native-falsification,
test-boundary, domain-naming, and no-contract-weakening gates.

No native acceptance body, VMM, GuestKit, kernel module, BPF program, cgroup,
namespace, route, or nft table was started by this reviewer. Mutation testing,
KASAN, Lima, and physical-host kernel or VM mutation were not used.

## Verdict

**APPROVED.** The package adds private fixture sizing and direct selection for
exactly the two existing identities, stages and executes the existing
`overdrive-cli--integration` ELF through the established `cgroup-default`
GuestKit boundary, rejects vacuous and runtime-skipped runs, and preserves the
canonical 119-body `cgroup-default` and 46-body `mtls-relay` accounting.

The native execution remains pending. Approval means the reviewed support may
now be prepared and used for the two focused correctness runs. Only those runs
can establish semantic RED or GREEN for the existing bodies.

## Findings

No blocking, high, medium, or low findings remain.

## Exact identity and contract-shape audit

| Required identity | Review result |
|---|---|
| `integration::vsock_density::every_vm_at_density_round_trips_through_its_own_attachment` | PASS — appears once in `VM_COHORT_CLI`, remains an existing `#[tokio::test]`, and retains `/// CONTRACT_SHAPE: bounded-change.` |
| `integration::vsock_density::serve_down_at_density_is_fail_closed_and_the_next_boot_reclaims_every_vm` | PASS — appears once in `VM_COHORT_CLI`, remains an existing `#[tokio::test]`, and retains `/// CONTRACT_SHAPE: bounded-change.` |

The diff against the approved parent removes exactly two `#[ignore = "pending
DELIVER step 14-05"]` attributes from `vsock_density.rs`: zero body lines and
zero assertions change. The source table adds no alias, prefix selector, new
scenario, new lane, or third identity.

Contract-shape traceability is coherent: the feature delta and roadmap describe
the 64-VM correctness journey; the two Rust identities name per-attachment
round trips and fail-closed restart reclamation; the bodies assert those same
outcomes. No event-sourced aggregate is involved, so an event-sequence table is
not applicable.

## Outcome and observable-universe audit

| Accepted outcome | Existing body and oracle | Result |
|---|---|---|
| 64 unique workload attachments each complete TCP and UDP once | The first body deploys 64 Jobs, requires every terminal row to be `Terminated`, requires 64 unique workload addresses, requires exactly 64 unique TCP tags and 64 unique UDP tags, and requires every peer observation to carry the node source address. | PASS |
| Guest-prefix access fails closed with `serve` up and down | The second body first proves a wildcard host listener is live, then requires every workload-address dial to return only `ECONNREFUSED` or `ENETUNREACH`; success, timeout-shaped errors, and any accepted host connection fail the body. | PASS |
| The next boot enforces NO-SURVIVE before replacement | The body kills `serve`, reboots it, requires all 64 former VMM PIDs to be gone, waits for a distinct Running allocation for every persisted Job, and requires the replacement CID set to equal the reclaimed CID set. | PASS |
| U-2 correctness only | Neither body nor the fixture records latency, throughput, memory usage, percentile, density rate, or benchmark credit. The added values are static capacity inputs and a supervision bound. | PASS |

The designed semantic failure at the host-connect interface maps exactly to the
approved OS representations `ECONNREFUSED` and `ENETUNREACH`; the assertion's
closed match rejects every other errno and successful connection. The package
adds no production error path or interface representation. Fixture failures
continue through the existing typed `GuestKitError` family: absent prepared ELF
as `LaneNotPrepared`, launch/peer/wait failures through their existing variants,
and private runner selection/result violations as existing `Staging` errors.
There is no `SPECIFICATION_AMBIGUITY` and no string message replacing a designed
production semantic distinction.

## Staging, discovery, and ELF provenance

The existing staging contract is sufficient and fail-closed:

1. `IN_GUEST_LANES` already declares package `overdrive-cli`, test target
   `integration`, and features `integration-tests,kvm-tests`.
2. `guest-kit-prepare` invokes Cargo with JSON messages and accepts only a
   `compiler-artifact` whose profile is a test and whose target name is
   `integration`; it copies Cargo's reported executable to
   `guest-kit/in-guest/overdrive-cli--integration`.
3. `InGuestTests::prepared("overdrive-cli", "integration")` checks that exact
   contract path and returns `LaneNotPrepared` if it is absent.
4. `GuestRun::boot_with_resources` stages that checked ELF at the existing
   in-guest test path with its shared libraries. No source copy, wrapper,
   alternate build, or second executable-discovery mechanism is introduced.

The source gate intentionally produced no execution ELF. The handoff correctly
requires the later native preparation receipt to retain Cargo's reported source
ELF path, its SHA-256, and the SHA-256 of the staged contract-path copy before
either correctness body is run.

## Direct selection and fail-honest accounting

The direct selector is exact and isolated:

- `OVERDRIVE_FWD_GUEST_EXACT` is accepted only with
  `OVERDRIVE_FWD_GUEST_GROUP=cgroup-default`.
- `focused_selection` compares the full identity for equality and returns one
  `Cell` from `VM_COHORT_CLI`.
- The libtest argv includes `--exact` and only that identity.
- The source manifest must total exactly two bodies before execution.
- The selected process must exit zero and print the required
  `test result: ok. 1 passed; 0 failed;` summary.
- Recognized runtime-SKIP diagnostics are an error. A retained `#[ignore]`
  produces zero passes and therefore cannot receive credit.
- The focused branch returns before any canonical manifest executes and emits
  `canonical_119_credit=0`.

The unchanged arithmetic remains `cgroup-default = 57 + 18 + 5 + 20 + 13 + 6
= 119` and `mtls-relay = 7 + 36 + 3 = 46`. The two cohort identities are kept
outside both canonical manifests, so a focused pass cannot change either count.

## Resource arithmetic

The private sizing is consistent with the existing test declarations and the
production reserve policy:

| Quantity | Independently checked value |
|---|---:|
| Declared RAM per workload | 134,217,728 bytes |
| `reserve_bytes(128 MiB)` | 8,724,152 bytes |
| `MemoryPlan` cgroup ceiling per inner VMM | 142,941,880 bytes |
| 64 inner VMM ceilings | 9,148,280,320 bytes |
| Existing outer kernel/serve/test/tools envelope | 4,294,967,296 bytes |
| Required outer bytes | 13,443,247,616 bytes |
| 12,820 MiB | 13,442,744,320 bytes — short by 503,296 bytes |
| 12,821 MiB | 13,443,792,896 bytes — headroom 545,280 bytes |

The two const assertions enforce both sides of that ceiling. Because the
calculation calls production `MemoryPlan::derive(...).cgroup_max_bytes()`, a
reserve-policy change that invalidates 12,821 MiB breaks compilation. The
existing 500-millicpu declaration still derives one vCPU per inner workload;
the outer guest keeps the established eight-vCPU nested-execution setting. CPU
overcommit affects elapsed time only and creates no performance claim.

The 1,200-second focused cell bound matches the existing twenty-by-60-second
`vsock_density` nextest envelope. It is an outer cleanup/supervision deadline;
the bodies' 600-second state bounds and two-second immediate-refusal oracle are
unchanged.

## Production composition, confinement, and cleanup

The support preserves the production and isolation boundaries:

- `GuestKit::boot_scripted_with` reaches production
  `CloudHypervisorVmm::create` over the production `VmConfig` renderer.
- The outer VMM retains uid/gid 4200, the device-node-derived KVM and
  vhost-vsock supplementary groups, production seccomp and Landlock, and the
  confined short runtime directory.
- The prepared test ELF runs only inside that disposable outer guest. Its
  production `serve` composition launches all 64 workload VMMs there, so their
  processes and kernel, network, nft, BPF, cgroup, and filesystem effects are
  nested within the disposable boundary.
- The physical Metal host supplies the outer production VMM and KVM transport;
  the focused path performs no physical-host nextest run, module load, cgroup
  delegation change, network setup, or workload cleanup.

On success, `GuestRun::finish` prints the complete console, requests guest
poweroff, and requires the outer VMM to exit within 30 seconds. On body error,
timeout, or panic, unwinding drops `GuestKitVm`; its synchronous Drop path calls
production `CloudHypervisorVmm::terminate` with zero grace, whose implementation
signals immediate kill and waits for the reaper outcome. Dropping the outer VMM
therefore ends every nested VMM without a physical-host workload cleanup path.
Work and support directories remain RAII-owned. The handoff correctly refuses a
native receipt that lacks terminal outer-guest cleanup evidence.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 10/10 | One success journey and one fault/recovery journey; error coverage is 50%. |
| Given-When-Then form | 9/10 | Both Rust bodies have explicit preconditions, one domain journey, and observable outcomes; the restart body intentionally carries the accepted outage-and-recovery sequence. |
| Business/domain language | 10/10 | Identities and assertions use Job, workload address, node source address, `serve`, VMM, and reclaimed CID precisely. |
| Coverage completeness | 10/10 | Both roadmap criteria and every requested outcome map to one of the two exact bodies. |
| Walking-skeleton user-centricity | 10/10 | The production-composed journeys prove externally meaningful round-trip, fail-closed, and restart outcomes rather than component wiring alone. |
| Priority validation | 10/10 | The package addresses only the missing execution capacity/selection boundary; GH #326 retains measurements. |
| Observable behavior assertions | 10/10 | The bodies observe production rows, peer traffic, connect errno, live-listener non-delivery, VMM death, and replacement CIDs. |
| Traceability coverage | 10/10 | P6-S15, V-9 correctness, R5-25, R5-55, and NO-SURVIVE remain linked through roadmap, module docs, identities, and assertions. |
| Walking-skeleton boundary proof | 10/10 | Cargo-reported ELF, production composition root, production VMM adapter, real nested KVM, and real kernel/wire effects are retained. |

All dimensions are at least 7, every mandate below passes, and there are zero
blockers.

## Test-design mandates and three pillars

| Gate | Result | Evidence |
|---|---|---|
| CM-A — hexagonal/port boundary | PASS | The bodies enter through deploy/serve production composition and observe its public/integration boundary; the support invokes the prepared test binary, not an internal implementation substitute. |
| CM-B — domain-language abstraction | PASS | The exact identities and outcomes preserve the feature's ubiquitous language; technical staging remains in the fixture. |
| CM-C — complete journey | PASS | Deploy, round trip, outage/refusal, restart reclamation, replacement, Job stop, and shutdown close the two journeys. |
| CM-D — pure logic/fixture isolation | PASS | No business logic is reimplemented. Static private sizing reuses production `MemoryPlan`; impure work stays in the existing GuestKit adapter. |
| CM-E — universe-bound assertion | PASS / N/A | These are Tier-3 production-composition tests, where traditional observable assertions are the repository-approved mode. |
| CM-F — layer-dependent PBT mode | PASS | Both real-I/O bodies are explicit examples; no property generator is applied to native execution. |
| CM-G — two-tier rich journey | PASS / N/A | The accepted two-body native set is fixed; control-plane state-space coverage remains in its existing sim/Quint tiers. |
| CM-H — real-I/O sad paths | PASS | The serve-death/refusal/reclamation journey is one named, explicit native example. |

Pillar 1 passes through precise domain names and outcomes. Pillar 2 passes
because the second body chains Running VMs to owner death, fail-closed outage,
boot reclamation, replacement, and cleanup. Pillar 3 passes because the
prepared test target drives the production `serve` composition and production
Cloud Hypervisor adapter inside the disposable GuestKit boundary.

## Completeness taxonomy

| Category | Result | Bounded evidence |
|---|---|---|
| C1 — equivalence and boundary | PASS | The fixed accepted population is exactly 64; workload addresses and CIDs must be unique at that population. Other populations are benchmark scope under GH #326. |
| C2 — state and transition | PASS | Module documentation and bodies cover terminal completion plus Running → owner down → boot reclamation → replacement Running. |
| C3 — count cardinality | PASS / N/A | The contract fixes this recurring correctness case at 64; zero/one/other density populations would change scope into the benchmark. |
| C4 — lifecycle and idempotency | PASS | Both journeys include orderly stop/shutdown; the restart journey proves former VMM death and reclaimed-CID replacement. |
| C5 — mode/decision table | PASS / N/A | No mode flag is introduced. |
| C6 — negative and robustness | PASS | Success, timeout-shaped failure, listener delivery, and every errno outside the approved two-member set fail closed; runtime SKIP and ignored selection also fail. |
| C7 — environment/interruption/concurrency | PASS | The real nested-KVM environment carries 64 actors; the second journey kills `serve` mid-lifecycle and proves recovery plus cleanup. |

### Mechanical 15-item checklist

| Item | Result | Rationale |
|---|---|---|
| C1a | PASS / N/A | The accepted correctness input is the fixed 64-VM cohort; empty density is outside this scenario. |
| C1b | PASS | Exactly 64 is the scenario boundary and is asserted through unique address/CID cardinality. |
| C2a | PASS | The module docstring states the completion and outage/restart lifecycle. |
| C2b | PASS / N/A | This set does not claim exhaustive control-plane state-machine coverage; existing sim/Quint tiers own it. |
| C3 | PASS / N/A | Population variation is explicitly GH #326 benchmark scope. |
| C4a | PASS | Restart re-applies placement after owner loss and must converge on the reclaimed CID set. |
| C4b | PASS / N/A | No inverse-without-prerequisite operation is part of P6-S15. |
| C5a | PASS / N/A | No mode flag exists. |
| C5b | PASS / N/A | No flag orthogonality claim exists. |
| C6a | PASS / N/A | No parser or free-form input surface is introduced by this support. |
| C6b | PASS | The designed connect-failure set is exercised and asserted as `ECONNREFUSED` or `ENETUNREACH`. |
| C6c | PASS | Success, timeout/other errno, listener acceptance, process failure, ignore, and runtime SKIP cannot escape as a pass. |
| C7a | PASS | The static envelope covers all 64 production cgroup ceilings plus the existing outer overhead and fails closed if it cannot launch. |
| C7b | PASS | The second body interrupts the live owner and proves fail-closed outage and restart reclamation. |
| C7c | PASS | The outage/restart body establishes all 64 VMs Running before owner death; the round-trip body drives the full 64-identity cohort. |

**Mechanical result: 15/15 — COMPLETE.** Items marked not applicable count as
passing under the canonical completeness skill and are justified above.

## Source verification

| Check | Result |
|---|---|
| Current parent | PASS — `a2cd9691de4da949389cfe884a044ca9cc60367d` matches the handoff. |
| `vsock_density.rs` scope | PASS — `git diff --numstat` reports zero added and two removed lines; the diff is exactly the two ignore attributes. |
| Current source SHA-256 | PASS — `fwd_guest_suite.rs` is `d8ba2e26bb71b9e1ba7483940026154f56b4acb3818bd7ba037c0a597bd4f190`; `vsock_density.rs` is `f06869d85ff3169609c84b948734549544f5c9b0abee7c480ec8a245c5d4f680`, matching the retained source-gate record. |
| Durable unprivileged Metal source gate | PASS — handoff records run `7b4fbc2cf1be5170b6fa4878`, `exit_code=0`, `cause=command_returned`, with fmt, both scoped checks, and both Clippy `-D warnings` checks green. |
| `git diff --check` on the bounded package and preserved ignore removals | PASS. |
| Native acceptance | NOT RUN — correctly deferred until this review approval. |
| Mutation testing / KASAN | NOT RUN — forbidden for this bounded review. |

## Remediation dispositions

No remediation is required. The later native run must follow the handoff's
durable Metal, artifact-provenance, exact-selector, full-output, zero-canonical-
credit, runtime-SKIP rejection, and terminal outer-guest cleanup contract.

---

# Iteration 2 — live-address oracle correction

## Iteration 2 metadata

- **Review type:** same independent `nw-acceptance-designer-reviewer`, bounded
  re-review
- **Review date:** 2026-11-01
- **Reviewed delta:** live-address observation correction in the existing
  round-trip body plus the revised handoff and retained fixture support
- **Current `vsock_density.rs` SHA-256:**
  `85817a7d6abdf7b43abae8d44eed73c895e29f383817a854600923976979be37`
- **Current `fwd_guest_suite.rs` SHA-256:**
  `d8ba2e26bb71b9e1ba7483940026154f56b4acb3818bd7ba037c0a597bd4f190`
- **Iteration 2 verdict:** **CHANGES_REQUIRED**

## Iteration 2 boundary

This iteration preserves the complete iteration 1 record above. It reviews only
the acceptance-designer-owned correction to
`every_vm_at_density_round_trips_through_its_own_attachment` and the handoff's
new native-evidence and artifact-freshness claims. The runner implementation,
resource arithmetic, direct selector, canonical accounting, confinement and
cleanup code are byte-identical to iteration 1.

No native body, GuestKit, VMM, kernel operation, mutation test, KASAN run or Lima
command was executed. No production source, DES event or commit was written.

## Iteration 2 verdict

**CHANGES_REQUIRED.** The live-address oracle correction is technically sound,
non-weakening and ready for native execution. One blocking handoff contradiction
must be removed first: the document calls the corrected test binary
"already-staged" even though it later proves that every staged ELF predates the
correction and forbids reusing those artifacts.

No Rust source remediation is required by this review. The existing
`WorkloadCrashedImmediately` evidence remains an unresolved product RED; this
review neither waives nor attempts to fix it.

## Iteration 2 strengths

- The correction preserves both exact libtest identities and both exact
  `/// CONTRACT_SHAPE: bounded-change.` declarations. It adds one private helper
  and no scenario, lane, product API, seam, hook, error variant, model or
  benchmark mechanism.
- The accepted ADR-0098 lifecycle is observed honestly. A VM address is read
  only from a public `Running` row. `Failed` or `Terminated` before that
  observation is an immediate failure with the complete row, so a crash cannot
  be filtered into a smaller address set and mistaken for success.
- The observer is scheduled immediately after each successful deploy response,
  before the loop submits the next Job. Each task therefore polls concurrently
  throughout its VM's boot rather than beginning after all 64 short-lived
  workloads have ended. Production VM boot supplies substantial scheduling
  headroom before the guest command can complete.
- `JoinSet` owns every observer. A joined task must return the exact Job id and
  one address; a task panic fails the body, and unwinding aborts unfinished
  observers. No detached observation survives the test task.
- The body still requires exactly 64 Job ids, 64 observed live addresses, 64
  distinct live addresses, terminal `Terminated` for every Job, exactly 64 TCP
  tags, exactly 64 UDP tags and the node source address on every peer
  observation. No outcome is removed or relaxed.
- The terminal-row correction is supported by the accepted architecture and
  both production authoring paths. ADR-0098 states that `Running` carries
  `workload_addr` and a genuine terminal removes it. `FinalizeFailed` forwards
  the address only for its non-terminal Stable result, while `StopAllocation`
  writes `None` for its Terminated row.
- The known product failure is kept distinct from the oracle correction. Two
  prior runs failed different VMs with `WorkloadCrashedImmediately`; the
  temporary private drain-budget diagnostic was reverted and is not presented
  as a remedy or production result.

## Iteration 2 finding

| ID | Severity | Disposition |
|---|---|---|
| VMCS-R2-01 | **Blocker** | Open — the handoff simultaneously claims an already-staged corrected binary and proves that no corrected execution ELF exists. |

### VMCS-R2-01 — Corrected ELF provenance is internally contradictory

**Evidence:** `vm-cohort-support.md:18-19` says both bodies remain in the
"already-staged `overdrive-cli::integration` test binary." The same document
then states at `:242-250` that this correction did not run
`guest-kit-prepare`, produced no corrected execution ELF, and that every prior
staged artifact was built from the pre-correction source hash
`f06869d85ff3169609c84b948734549544f5c9b0abee7c480ec8a245c5d4f680` and must
not be reused. The corrected test source is instead
`85817a7d6abdf7b43abae8d44eed73c895e29f383817a854600923976979be37`.

This is load-bearing rather than editorial. The next authorized action is to
prepare and execute a fresh ELF. Describing the only extant staged artifacts as
the current binary permits exactly the stale-artifact execution that the later
freshness gate forbids. The prior recovery pass remains valid historical
evidence for its unchanged identity and for the runner boundary; its old ELF is
not evidence that the corrected source was prepared.

**Required bounded remediation:** replace "already-staged
`overdrive-cli::integration` test binary" with wording that names the existing
Cargo test target or the binary that `guest-kit-prepare` will freshly stage.
Keep the explicit statement that no corrected ELF exists, retain both old ELF
hashes as historical provenance, and keep the requirement to record Cargo's
new reported executable plus the freshly staged copy's SHA-256 before native
execution. No Rust, production, runner, scenario or lane change is required.

## Oracle and lifecycle audit

| Claim | Evidence | Result |
|---|---|---|
| Address comes from a live public state | `observe_live_workload_address` calls the existing public `describe` handler and accepts only `AllocStateWire::Running` with `Some(workload_addr)`. | PASS |
| Terminal state cannot supply or reconstruct an address | The helper panics on `Terminated` or `Failed`; no terminal row or historical value is read after that branch. | PASS |
| Observer starts early enough to observe a short-lived workload | Each observer is spawned directly after that Job's deploy POST returns, while the following deploy and the real VM boot still have to occur. All observers run concurrently. | PASS |
| Observer failure cannot become success | Describe failure, missing Running address, terminal-first state, deadline, task panic, duplicate id or fewer than 64 completions all panic the body. | PASS |
| Existing terminal-success oracle remains | After all live addresses are captured, every Job must reach terminal and its first row must be exactly `Terminated`. | PASS |
| Existing traffic oracle remains | Exact TCP/UDP counts, complete tag sets and node-source assertions are unchanged. | PASS |
| Existing product RED remains visible | A `Failed` row containing `WorkloadCrashedImmediately` now fails at the observer with that exact row before any address-cardinality assertion. | PASS |

The correction strengthens test honesty. The former oracle required a value the
accepted terminal contract deliberately removes and used `filter_map`, so a
genuine terminal absence reduced the set without identifying the lifecycle
cause. The corrected oracle closes that gap and does not permit a previously
forbidden state.

## Exact API and typed-error gate

No public or cross-crate surface changes. `DescribeArgs` and `describe` are the
existing operator-facing driving adapter already used by `Lane::snapshot` and
`Lane::wait_rows`; the new helper does not reach an internal store or add a
test-only accessor. `PathBuf`, `JoinSet` and the helper itself are private test
implementation.

No production typed-error contract changes. A public describe transport,
status or decode error fails the acceptance body with its existing `CliError`
text; a `Running` row without an address, terminal-first row or observation
deadline is a test-oracle failure. The approved host-connect semantic error set
in the unchanged recovery body remains exactly `ECONNREFUSED | ENETUNREACH`.
There is no `SPECIFICATION_AMBIGUITY`.

## Acceptance-review dimensions — iteration 2 delta

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 10/10 | The success cohort retains terminal/traffic checks; terminal-first and public-observation failures are explicit. |
| Given-When-Then form | 9/10 | Deploy begins observation, each allocation must become publicly Running with an address, then complete its terminal and traffic outcomes. |
| Business/domain language | 10/10 | Job, allocation, Running, terminal, workload address, round trip and node source remain precise. |
| Coverage completeness | 10/10 | The corrected oracle now covers the lifecycle point at which the address is contractually present. |
| Walking-skeleton user-centricity | 10/10 | The body still drives the production composition and proves the same 64-VM outcome. |
| Priority validation | 10/10 | The change addresses only the independently proven terminal-address oracle defect. |
| Observable behavior assertions | 10/10 | The observation is through the public describe driving adapter and peer traffic, with no private state read. |
| Traceability coverage | 10/10 | P6-S15, R5-55(a), V-9 correctness and ADR-0098 align with the corrected body. |
| Walking-skeleton boundary proof | 10/10 | The correction preserves the prepared production test target and disposable GuestKit boundary. |

CM-A through CM-H and all three pillars continue to pass. In particular, CM-A
improves because the address oracle now uses the public CLI driving adapter;
CM-C retains all 64 complete journeys; CM-D introduces no business algorithm;
CM-F/H keep the Tier-3 case example-based. The iteration 1 completeness result
also remains 15/15: C2 now observes the valid Running-to-terminal lifecycle more
precisely, C6 fails closed on every observer outcome, and C7 retains 64
concurrent actors plus owner interruption.

## Native evidence and freshness audit

| Evidence | Iteration 2 disposition |
|---|---|
| `23dcb4e230df8d1fe0dd182e` | Historical PASS for the unchanged recovery identity: one pass, zero ignored/runtime-SKIP, clean GuestKit and zero canonical-119 credit. It does not contain the corrected test source. |
| `88a2a3893cd926d4078cceef` | Historical semantic RED: one VM reached `WorkloadCrashedImmediately`. Product defect remains open. |
| `4c191dd7f199e4fce463edef` | Independent historical semantic RED on a different VM: `WorkloadCrashedImmediately`. Product defect remains open. |
| `8f71d1cfe5460b053d0902ad` | Diagnostic only: temporary private drain-budget change exposed 62/64 terminal addresses; production change reverted. It proves the old terminal-address oracle was invalid, not that product correctness passed. |
| `236628f7df0df257b41c8a0e` | PASS source gate for the corrected source: fmt, both scoped checks and both Clippy `-D warnings` checks, exit 0. No native execution ELF was produced. |

The current hashes match the revised handoff and `git diff --check` passes. The
test-source diff is the two pending-marker removals plus the private live-address
observer and replacement of the terminal-address cardinality read. No
production file is part of the bounded package. A corrected native result must
use a newly prepared ELF and record both source and staged-copy provenance; no
earlier ELF is eligible for reuse.

## Iteration 2 remediation disposition

Remediate only VMCS-R2-01 in `vm-cohort-support.md`, then return this same
package to the same reviewer. Do not run the corrected native body, change
production, alter either identity or outcome, add a scenario/API/seam/lane,
write DES, or commit while this iteration 2 verdict remains open.

---

# Iteration 3 — corrected ELF provenance remediation

## Iteration 3 metadata

- **Review type:** same independent `nw-acceptance-designer-reviewer`, focused
  remediation re-review
- **Review date:** 2026-11-01
- **Finding under review:** VMCS-R2-01 only
- **Rust source change since iteration 2:** none
- **Native execution:** none
- **Iteration 3 verdict:** **APPROVED**

## Iteration 3 verdict

**APPROVED.** VMCS-R2-01 is resolved. The handoff now distinguishes the
existing Cargo test target from an execution artifact, states explicitly that
no corrected staged ELF exists, and requires a fresh, uniquely identified,
byte-verified build and stage before the exact selector can execute.

No open findings remain. The iteration 2 technical approval of the
live-address oracle stands unchanged, and the two historical
`WorkloadCrashedImmediately` results remain unresolved product RED evidence.

## VMCS-R2-01 remediation disposition

| Required correction | Iteration 3 evidence | Result |
|---|---|---|
| Remove the false current-artifact claim | `vm-cohort-support.md:18-21` now says the bodies remain in the existing Cargo test target and that no staged ELF contains the corrected source. | PASS |
| Require an exact fresh build | The future command selects package `overdrive-cli`, test target `integration`, and features `integration-tests,kvm-tests`, matching `IN_GUEST_LANES`. | PASS |
| Refuse ambiguous Cargo discovery | The JSON query admits only test-profile `compiler-artifact` messages for target `integration`, applies `unique`, and errors unless exactly one executable remains. | PASS |
| Keep one staging producer | `cargo xtask guest-kit-prepare` remains the sole producer of `guest-kit/in-guest/overdrive-cli--integration`. | PASS |
| Prove source/staged artifact identity | Both paths must be executable; `cmp -s` requires byte identity; `stat` and `sha256sum` record both paths, sizes, modes and hashes before execution. | PASS |
| Preserve source provenance | The command hashes the corrected `vsock_density.rs` and retained runner source before the build and the receipt still requires the source commit and dirty diff. | PASS |
| Preserve stale-artifact prohibition | The pre-correction source hash and historical diagnostic/recovery ELF hashes remain recorded at `:295-305`, followed by an explicit prohibition on reuse. | PASS |
| Fail closed before native execution | `set -eu`, unique-executable selection, executable checks, `cmp`, and hashing all precede the exact runner invocation; any failure prevents the body from starting. | PASS |

The future source executable and staged ELF use the same task-owned
`CARGO_TARGET_DIR`. The runner inherits that directory, so
`InGuestTests::prepared("overdrive-cli", "integration")` reads the exact staged
path whose bytes were just compared. The command adds no build mechanism,
selector, lane or product surface.

## Iteration 3 verification

| Check | Result |
|---|---|
| `vsock_density.rs` SHA-256 | Unchanged — `85817a7d6abdf7b43abae8d44eed73c895e29f383817a854600923976979be37`. |
| `fwd_guest_suite.rs` SHA-256 | Unchanged — `d8ba2e26bb71b9e1ba7483940026154f56b4acb3818bd7ba037c0a597bd4f190`. |
| Handoff provenance statement | PASS — current target identity and absent corrected ELF are no longer conflated. |
| Future command syntax | PASS — author reports `bash -n`; inspection confirms the nested shell/JQ quoting preserves the exact query and command sequence. |
| `git diff --check` | PASS. |
| Production/test/DES diff since iteration 2 | None. |
| Native acceptance | NOT RUN — correctly deferred until this approval. |
| Mutation testing / KASAN / Lima | NOT RUN. |

## Final disposition

The reviewed package is approved for the next bounded native action. That run
must use the handoff's complete fresh-build/provenance command and retain the
durable receipt. Approval does not mark the round-trip body GREEN and does not
authorize a production change: the existing product RED must be reproduced or
resolved through the delivery crafter's separately governed work.
