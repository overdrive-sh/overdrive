# DISTILL Q7/Q9 platform review — guest-stack-transparent-mtls-intercept

| Field | Value |
|---|---|
| Review ID | `platform_rev_2026-08-29_q7_7255c68e` |
| Reviewer | `nw-platform-architect-reviewer` (fresh isolated review) |
| Reviewed commit | `7255c68e64b7f0f15da5a2ed8a033806a2939e6a` |
| Comparison base | `29ab0bf71ef8c178a04035010d0ad72084b9ce7b` |
| Review iteration | 1 |
| Verdict | **NEEDS_REVISION** |

## Scope and evidence

This review covers the Q7/Q9 DISTILL remediation in `feature-delta.md`,
`distill/test-scenarios.md`, and `distill/red-classification.md`. It evaluates
the package against the approved DESIGN amendment, the real-guest SPIKE, the
repository's metal/KVM and testing policy, the production driver/reconciler/nft
surfaces at the reviewed commit, and the `cargo xtask metal` implementation.
The roadmap is known to retain the superseded downstream text and was not
treated as an authoritative acceptance source. The missing feature-specific
DEVOPS artifacts are recorded as pre-existing input absence, not as a standalone
finding.

The working tree contains later uncommitted implementation work and other review
artifacts. Source evidence below is therefore taken from the immutable reviewed
commit (or its parent, whose source tree is identical for this docs-only commit),
not from those later changes. No production file was modified and no test suite
was run: the commit changes documentation only, while the relevant end-to-end
checks require the designated metal host.

Static verification included:

- `git diff --check 29ab0bf7 7255c68e` — passed;
- changed-file and scenario-map inspection — three documentation files changed;
- commit-parent inspection of the live/scaffolded Rust tests;
- nft expression and `GETRULE` decoder inspection;
- CLI command/handler and workload-reconciler transition inspection;
- `cargo xtask metal`/bootstrap/nextest serialization inspection.

## Platform feasibility summary

| Concern | Assessment |
|---|---|
| Real guest route | Correctly selects `kvm-tests` through `cargo xtask metal run --` and rejects Lima as evidence, but incorrectly calls the required native-metal KVM surface “nested KVM” (P1). |
| Capture placement | The approved after-C3/before-real-VMM-create seam is feasible through the existing VMM override/decorator seam; the all-EtherType, exact-interface, fail-on-drop/truncation contract is technically implementable. |
| Guest suppression | NIC-down admission, per-interface IPv6 disable/write-readback, `arp_notify=0` write-readback, static IPv4/route, resolver, then READY is feasible without Beacon or operator-protocol expansion. |
| Console evidence | An 8-KiB/five-fragment tail from `VmRunDir::console_log()` before run-directory removal is feasible, with bounded stderr fallback. Cleanup preservation is not acceptance-covered (P6). |
| nft lifecycle | Exact rule presence and alloc-scoped removal/peer-rule preservation are feasible. The required per-rule “increment” is not observable from the unchanged production rule (P3). |
| Restart/stop | Same-allocation `RestartAllocation` and ungated teardown exist, but the scenarios name the wrong operator routes (P5). |
| Public surface | The intended implementation can avoid Beacon, `VmmExit`, describe, observation-schema, and public protocol expansion. S-GTI-08 currently tries to force and inspect facts unavailable at its assigned driving port (P4). |

## Findings

### P1 — The specified KVM environment contradicts the native-metal trust boundary

**Severity:** HIGH
**Status:** Open

`test-scenarios.md:25-27,149-154,360-364` and
`red-classification.md:52,62-67` repeatedly name **nested KVM** as the required
execution tier. The repository's metal provisioner says the opposite:
`infra/metal/provision.sh:46-49` requires real virtualization extensions with no
nesting, and `:66-71` says a virtualized result inherits the same trust problem
as Lima. The SPIKE verdict is also explicitly pinned to native `/dev/kvm` on a
bare-metal x86_64 host.

The command route itself is right, but the environmental precondition is not a
terminology-only defect: `provision.sh` currently warns rather than aborts when
`systemd-detect-virt` reports virtualization. An implementer following the
DISTILL wording can therefore run on a nested host and publish evidence the
project explicitly considers non-authoritative.

**Required remediation:** replace every “nested KVM” statement with native KVM
on the non-virtualized x86_64 metal host; retain `kvm-tests` as the Cargo feature
name. Add a deterministic preflight for this gate that fails the feature run
when the host is virtualized, instead of treating the current warning as valid
evidence. Lima remains compile-only/non-signal for these scenarios.

### P2 — The RED classification is not a snapshot of the reviewed tree

**Severity:** HIGH
**Status:** Open

`red-classification.md:34-53` declares all twelve scenarios
`MISSING_FUNCTIONALITY`, and `test-scenarios.md:388-402` maps every scenario to a
RED scaffold. In the immutable commit-parent source, however:

- S-GTI-01, S-GTI-02, S-GTI-03, S-GTI-04, and S-GTI-07 are live metal test
  bodies (`guest_stack_mtls_egress.rs:1083-1450`);
- only S-GTI-05, S-GTI-06, S-GTI-08, and S-GTI-12 remain explicit RED panics
  (`:1455-1478`);
- `derive_vm_tap_plan` is implemented (`veth_provisioner.rs:815-830`) and
  S-GTI-09/10/11 are live properties (`:5075-5235`), not `todo!()` scaffolds.

Historical compile evidence is clearly labeled, but the document then uses it
to assert a current 12/12 RED gate. That makes the DELIVER entry state
non-reproducible and can cause already transitioned tests to be rewritten or
ignored.

**Required remediation:** regenerate the classification against the exact
reviewed tree. Distinguish inherited GREEN tests, still-live RED scaffolds, and
new Q7/Q9 obligations that make a previously green scenario incomplete. Record
fresh compile/discovery evidence or an explicit not-executed status per test;
do not call the whole population RED.

### P3 — “The exact rule increments” is impossible under the simultaneous REUSE-AS-IS constraint

**Severity:** HIGH
**Status:** Open

S-GTI-01/02 require the first original destination to increment the exact
alloc-scoped host-veth rule (`test-scenarios.md:177,191-194`; Q9 at `:61`). At
the same time, `feature-delta.md:88-90,501-505` declares
`install_outbound_tproxy` and its nft rule **REUSE AS-IS — zero change**.

The reviewed production rule cannot supply the asserted measurement:

- `egress_tproxy_rule_exprs` contains iifname match, L4-protocol match, TPROXY,
  mark, and accept, but no nft `counter` expression
  (`overdrive-netlink/src/nft.rs:461-493`);
- `RuleInfo` and `list_rules` expose only handle and userdata
  (`overdrive-netlink/src/nft.rs:187-195,926-954`);
- `install_outbound_tproxy` recovers rule identity/handle only
  (`overdrive-worker/src/mtls_intercept.rs:487-530`).

Packet capture plus a leg-F accept can prove the composed path, but it cannot
literally prove that this exact nft rule's counter incremented when no such
counter exists. Adding a counter expression/decoder changes an asset the design
marks off-limits; silently substituting a weaker inference violates the exact
Q9 wording.

**Required remediation:** reconcile this upstream contract before DELIVER. Pin
one authorized kernel-observable rule-hit mechanism (and its exact before/after
oracle), or explicitly sanction the tightly bounded internal nft counter change
and update the reuse tally. Do not add a public/protocol/observation-schema field,
and do not let the observation decorator mutate or install the production rule.

### P4 — S-GTI-08 is not executable at its assigned real driving port

**Severity:** HIGH
**Status:** Open

S-GTI-08 requires an operator deploy to make a real guest receive a malformed
platform-owned network token (`test-scenarios.md:238-251`) and then requires the
CLI metal test to compare a private `WorkloadLifecycleView` and exact internal
actions (`:250-251,395-398`). Neither side is available at that port:

- `VmPayload` contains command, args, kernel, and rootfs, not cmdline
  (`overdrive-core/src/traits/driver.rs:469-481`);
- `compose_vm_network` formats the token from typed allocation fields and rejects
  an invalid prefix before spawn (`overdrive-worker/src/vm_driver.rs:111-154`);
- `overdrive deploy`/`workload describe` do not return a private reconciler view
  or action vector.

A config-mutating VMM decorator would no longer be the specified production
deploy path and would conflict with the Q9 decorator's observation-only role.
Publishing private action/view state at the CLI would be the forbidden public or
protocol expansion.

**Required remediation:** use a production-reachable real pre-READY failure for
the metal sample, such as a legitimate custom rootfs whose resolver write fails.
Keep malformed-token parser partitions source-local. Split the exact
`FinalizeFailed`/no-`RestartAllocation`/unchanged-view proof into a focused
reconciler/action-shim test, while the metal scenario asserts port-visible
terminal detail, durable restart count/budget, no second allocation, no
READY/Running/EXEC/operator marker, and no frame.

### P5 — The restart and teardown scenarios name routes that do not drive the claimed transitions

**Severity:** HIGH
**Status:** Open

S-GTI-06 allows `overdrive workload restart` as a genuine same-id
`RestartAllocation` trigger (`test-scenarios.md:220-228`; carry-forward at
`feature-delta.md:809-816`). The production restart handler instead bumps a
desired generation (`handlers.rs:946-969`); the reconciler stops the current
instance and later emits a fresh `StartAllocation` with a newly minted allocation
id (`workload_lifecycle.rs:698-739,1124-1204`). It therefore proves the fresh
install site, not the same-id restart site the scenario exists to lock. Job-kind
natural exit is also final, not restart-budget recovery
(`workload_lifecycle.rs:823-833`).

S-GTI-12 then invokes the nonexistent `overdrive workload stop`
(`test-scenarios.md:253-260`). The actual CLI exposes `job stop`; the workload
namespace has only restart and describe (`overdrive-cli/src/cli.rs:94-109`), and
the direct test handler is `commands::deploy::stop`.

**Required remediation:** pin one deterministic real route that emits a reused-id
`RestartAllocation` for S-GTI-06, such as the existing unclean control-plane
boot/reclamation path while intent remains, and assert same allocation id plus
the durable restart evidence. Remove the generation-replacement CLI from that
scenario. Drive S-GTI-12 through the real `overdrive job stop <id>` / corresponding
`commands::deploy::stop` handler, then assert target-rule absence and byte/handle
preservation of every other allocation rule.

### P6 — The failed-start package does not prove cleanup totality or absence of host residue

**Severity:** HIGH
**Status:** Open

The approved DESIGN requires a console snapshot failure never to mask cleanup or
the typed rejection (`design/wave-decisions.md:156-174`). S-GTI-08 and its bounded
examples specify detail selection and lifecycle output
(`test-scenarios.md:238-296`) but never assert that every console outcome still
runs cleanup or that the failed allocation leaves no host residue. The
completeness audit likewise counts missing/unreadable console only as degraded
diagnostic coverage (`:356`).

This omission matters because `VmDriver` must add an asynchronous filesystem read
before `cleanup_after_start_failure`, and that cleanup is deliberately
best-effort across VMM termination, clone/index removal, cgroup kill/removal,
run-directory removal, and claim release (`vm_driver.rs:918-950`). C3 and D6 add
netns, tap, veth, return-route, capture, and nft resources outside that immediate
function. A correct terminal row can therefore coexist with leaked platform
state unless the full failure path is checked.

**Required remediation:** add focused tests proving absent/empty/unreadable and
read-error console outcomes still invoke cleanup once and preserve the primary
typed rejection. Extend the real S-GTI-08 completion oracle to wait for and assert
no alloc VMM process, cgroup, rootfs clone/index, run directory, netns, tap,
veth, return route, nft rule, raw capture socket, or capture task remains, while
an independently running allocation and its nft rule remain unchanged. Use
bounded polling/explicit deadlines, not sleeps.

### P7 — The metal launcher has no host-wide lease across sync and execution

**Severity:** MEDIUM
**Status:** Open

The nextest `host-kernel-shared` group correctly serializes this module inside a
single nextest invocation (`.config/nextest.toml:454-476`). It does not serialize
independent Conductor workspaces or separate `cargo xtask metal run` processes.
Every launcher syncs with `rsync --delete` into the same `~/overdrive` directory
(`infra/metal/bootstrap.sh:22,125-141`; `xtask/src/main.rs:675-679,737-779`) and
then uses the same node-global cgroup/nft/interface substrate. Concurrent runs can
overwrite the remote tree between sync and compile or collide in the kernel,
creating false failures or, worse, evidence from mixed source revisions.

**Required remediation:** make the feature's metal gate acquire a host-wide lease
covering sync through command completion, with owner/commit diagnostics and a
finite acquisition timeout. A workspace-specific remote directory alone is not
sufficient because nft/cgroup/interface resources remain node-global. Document
the lease in the eventual DEVOPS artifact.

### P8 — Driving-port scenarios have no EDD expectation stubs

**Severity:** MEDIUM
**Status:** Open

The package maps Rust test bodies but provides no
`verification/expectations/` stubs for the operator/driving-port behaviors. That
omits the stable outcome-to-evidence bridge for terminal state/detail,
same-allocation restart, exact rule lifecycle, peer-rule preservation, and the
born-captured wire proof. This is especially risky while the roadmap remains
intentionally stale and the feature-specific DEVOPS handoff is absent.

**Required remediation:** add deterministic expectation stubs for the driving
scenarios, naming the exact command, state/wire/kernel evidence, timeout, cleanup
postcondition, and metal prerequisite. Keep source-local pure properties in Rust;
do not duplicate them as shell expectations.

## Strengths retained

- The route is correctly centered on the real x86_64 metal runner and a real
  Cloud Hypervisor guest; no Lima/netns substitute is accepted for the guest
  interception claim.
- Q9 closes the packet universe across EtherTypes, VLAN shapes, directions, and
  failure evidence, and it requires capture before real VMM spawn on the exact
  allocation interfaces.
- IPv6 and `arp_notify` write/readback ordering is explicit and precedes NIC-up,
  READY, intercept release, and the operator command.
- Console bounds, unterminated-fragment handling, lossy UTF-8, guest-console
  precedence, and stderr/neither-source fallbacks are concrete and bounded.
- S-GTI-12's target-rule removal plus preservation of other allocations is the
  right teardown oracle once it is connected to the real stop route.
- The package explicitly forbids Beacon, `VmmExit`, describe, observation-schema,
  and public protocol expansion.

## Finding count

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 6 |
| Medium | 2 |

## Verdict

**NEEDS_REVISION.** The package has six open high-severity findings, so it does
not satisfy the wave-gate rule for `APPROVED` or `CONDITIONALLY_APPROVED`.
Remediation must preserve the approved no-public-surface design while making the
metal environment, RED snapshot, rule-hit oracle, failure injection, lifecycle
driving routes, and cleanup proof executable and deterministic. The revised
artifact requires a fresh platform-architect re-review.

## Iteration 2 — remediation re-review

| Field | Value |
|---|---|
| Reviewed commit | `a5bd158edb76308a44ccf99597d349950898f0f7` |
| Comparison base / approved DESIGN | `cd12725159a6b2a92619f17aa4dc5f0ff621b842` |
| Review date | 2026-08-29 |
| Review iteration | 2 |
| Verdict | **NEEDS_REVISION** |

### Scope and verification

Iteration 2 reviewed the immutable remediation commit, the full effective
DESIGN/ADR/architecture text at its parent, the repository metal launcher and
test policy, and the three new expectation stubs. Later dirty DELIVER/source
work was excluded from evidence. The target remains a documentation and pending
expectation change; no metal result is inferred.

Static checks:

- `git diff --check cd127251 a5bd158e` — passed;
- the target parent is exactly `cd127251`;
- all three expectation runners are executable in the target tree and pass
  `bash -n`;
- no production, public API, Beacon, persistence, or observation-schema file is
  changed by the remediation commit.

### Prior-finding dispositions

| Iteration-1 finding | Disposition | Evidence |
|---|---|---|
| P1 — nested KVM contradicted the native-metal trust boundary | **PARTIALLY CLOSED; HIGH remains** | The remediated DISTILL contract now pins a fail-closed native, non-virtualized x86_64 preflight and rejects Lima/nesting (`test-scenarios.md:141-164`; E07 `README.md:21-26`). Effective DESIGN, the architecture brief, and the mandatory test rule still require “nested KVM” (`design/wave-decisions.md:68-77`; `feature-delta.md:591-596,640-647`; `brief.md:10259`; `.claude/rules/testing.md:1447-1468`). |
| P2 — RED classification was not an immutable snapshot | **CLOSED** | The base is explicitly `cd127251`; dirty work is excluded; every result is `NOT_EXECUTED`; inherited live bodies, semantic RED scaffolds, and new/incomplete obligations are distinguished row by row (`red-classification.md:7-54`; `test-scenarios.md:925-934`). |
| P3 — exact counter increment was impossible under REUSE-AS-IS | **CLOSED** | Approved D7 sanctions one anonymous counter plus the internal normalized-program/counter projection, strict complete `GETRULE`/`GETGEN`, notification/loss guard, exact AF_PACKET packet/IPv4-`tot_len` equality, checked arithmetic, and read-only ownership (`test-scenarios.md:81-139`; ADR-0088 §5; ADR-0089 §1). The tally is consistently 8/10/1. |
| P4 — S-GTI-08 used unreachable/private driving-port facts | **CLOSED** | S-GTI-08a now drives a real deploy-selected rootfs resolver failure and keeps metal assertions port-visible; private action/View facts live in `C-GTI-08-RECONCILE`, with S-GTI-08b supplying the READY/EXEC/exit-78 complement (`test-scenarios.md:269-288,380-392`). No public test hook is required. |
| P5 — restart/stop scenarios used the wrong routes | **PARTIALLY CLOSED; HIGH remains** | DISTILL now uses unclean serve restart → boot-epoch reclamation → same-`AllocationId` re-drive and the real `overdrive job stop` route (`test-scenarios.md:73-79,243-260,315-330`). Effective D6/ADR/brief text still calls restart budget, crash recovery, and `overdrive workload restart` live same-allocation VM paths. |
| P6 — cleanup totality/residue was absent | **PARTIALLY CLOSED; MEDIUM remains** | The full VMM/cgroup/clone/index/run-dir/netns/tap/veth/route/nft/capture set, deadlines, diagnostic-read totality, interruption, concurrent deploy, and independent-allocation preservation are now specified (`test-scenarios.md:269-280,368-378,418-433`). The new exact sibling “position/order” clause is not defined across target deletion. |
| P7 — no host-wide metal lease | **PARTIALLY CLOSED; MEDIUM remains** | A host-wide path, timeout, owner metadata, and cleanup/final-probe lifetime are specified and implementation is honestly assigned to roadmap/DEVOPS (`test-scenarios.md:155-164`). The stated boundary begins around the remote command and does not cover the launcher's preceding shared-tree `rsync --delete`. |
| P8 — no EDD stubs | **PARTIALLY CLOSED; MEDIUM remains** | E07/E08/E09 are present and honestly `pending`. E08 lists S-GTI-05 as an anchor but specifies only resolver failure and the exit-78 complement, leaving the distinct fresh production nft-install failure without an EDD command/evidence contract. |

### P1 — the native-metal correction was not reconciled into the effective execution contract

**Severity:** HIGH  
**Status:** Open

The new preflight is internally coherent and matches the actual provisioner's
bare-metal purpose: `infra/metal/provision.sh:46-71` requires real extensions,
states “NO nesting,” and warns that a virtualized result inherits Lima's trust
problem. However, DISTILL names DESIGN/ADRs as authoritative while the active
DESIGN and repository rule still say the opposite:

- `design/wave-decisions.md:68-77` requires the restart and Slice-1 tests on a
  nested-KVM metal surface;
- the DESIGN portion of `feature-delta.md:591-596,640-647` repeats that
  requirement;
- `docs/product/architecture/brief.md:10259` says the same;
- `.claude/rules/testing.md:1447-1468` calls the box “bare-metal” while saying
  the real guest needs nested KVM.

This is an active trust-boundary contradiction, not harmless history. A runner
author can either reject the DESIGN-approved environment or weaken the new
preflight and accept evidence the provisioner declares untrusted.

**Required remediation:** choose and reconcile one execution contract across
the mandatory testing rule, D6/walking-skeleton DESIGN text, architecture brief,
DISTILL, and E07-E09. For the native-metal decision now encoded by DISTILL,
replace the remaining nested-KVM requirements and make the provisioner's
virtualization warning fail closed. Preserve `kvm-tests` as the Cargo feature
name and Lima as compile-only.

### P5 — corrected same-id behavior is still contradicted by D6, ADR-0089, and the brief

**Severity:** HIGH  
**Status:** Open

The remediated scenarios use the reachable path and explicitly distinguish it
from generation replacement. Yet current authoritative text still says that
restart budget, crash recovery, and `overdrive workload restart` are all live
VM same-allocation routes (`design/wave-decisions.md:18`; ADR-0089 `:56-62`;
the DESIGN portion of `feature-delta.md:570-577`; `brief.md:10140-10143`). The
same feature delta later says natural Job exit is final and the workload restart
verb mints a fresh allocation (`feature-delta.md:810,895-898,992-994`).

Following the stale source can create a Job retry loop or exercise only the
fresh-start gate while falsely claiming coverage of the reused-id restart gate.
It also makes `test-scenarios.md:34-36`'s “no unresolved DESIGN ambiguity” claim
false.

**Required remediation:** amend D6, ADR-0089 §1, the DESIGN section of the
feature delta, and the architecture brief to state that standing-intent
boot-epoch platform reclamation is the same-`AllocationId` VM Job route; natural
Job result/crash finalizes, and `overdrive workload restart` creates a fresh
allocation. Retain both production install-gate flips and S-GTI-06a/06b.

### P6 — exact sibling “position/order” has no deterministic teardown oracle

**Severity:** MEDIUM  
**Status:** Open

Approved D7 requires the quiescent sibling snapshot to remain equal and the
target handle alone to be deleted (`design/wave-decisions.md:332-337`). DISTILL
strengthens this to exact sibling “position” (`test-scenarios.md:134-139`) and
“order” (`:315-330`; E09 `README.md:54-57`) without defining absolute ordinal
versus relative order among surviving rules. If the deleted target precedes a
sibling, that sibling's absolute chain ordinal necessarily shifts even though
its rule and relative ordering are untouched.

**Required remediation:** compare the ordered sequence of surviving sibling
identity/program/counter snapshots after filtering the exact target handle, or
pin another explicit relative-order oracle. Do not require an impossible
unchanged absolute ordinal.

### P7 — the lease does not yet cover the shared `rsync --delete` boundary

**Severity:** MEDIUM  
**Status:** Open

The original collision starts before the remote test process. `cargo xtask metal
run` calls `metal_sync` first (`xtask/src/main.rs:737-746`), and bootstrap writes
every worktree into the same `~/overdrive` using `rsync --delete`
(`infra/metal/bootstrap.sh:22,125-141`). Only afterward does xtask start the
remote command (`xtask/src/main.rs:747-779`).

The remediation says to acquire `/run/lock/...` “before the remote command” and
hold it through preflight/execution/evidence/cleanup (E07 `README.md:28-43`;
`test-scenarios.md:155-164`). That does not state how the same lock is held over
the preceding sync, so two worktrees can still overwrite the shared checkout
before either in-command holder protects it. The claim that this already
serializes independent worktrees is therefore not established.

**Required remediation:** assign the lease to an outer metal-run boundary that
serializes sync plus remote execution as one ownership epoch, or use a
workspace/commit-isolated remote tree while retaining a host-global kernel
lease for execution. Specify acquisition/release mechanics that actually span
both phases. Implementing the pinned mechanism may remain a roadmap/DEVOPS
task; defining the correct boundary may not.

### P8 — E08 has a false S-GTI-05 anchor

**Severity:** MEDIUM  
**Status:** Open

S-GTI-05 is a distinct fresh-install failure: the production nft install must
return a real kernel error, the command must remain blocked, no cleartext/frame
may escape, and allocation cleanup must complete (`test-scenarios.md:234-241`).
E08 names S-GTI-05 as an anchor (`README.md:14-18`) but its expectation,
eventual command, and evidence cover only the custom-rootfs resolver failure and
post-READY exit 78 (`:5-12,26-59`). E09 covers a failed same-id reinstall, not
the fresh path.

**Required remediation:** add an explicit E08 fresh production guard-install
failure subcase with deterministic kernel arrangement, built command/state,
wire, kernel, cleanup, and sibling evidence, or add a separate minimal pending
expectation and remove the false E08 anchor.

### Platform feasibility and public-surface conclusion

The corrected resolver failure is reachable through a normal operator-selected
rootfs, the same-id route and Job-stop route exist, D7 is implementable as an
internal observer/encoder extension, and the full cleanup set is observable on
the native host. None requires a Beacon, public CLI schema, persistence schema,
or production test-only success seam. A real nft failure can be arranged by
external kernel fixture state while still driving the production installer;
the eventual EDD contract must name that arrangement and prove delta-scoped
restoration rather than introduce an injection flag.

The host-wide lease/preflight implementation and evidence capture are valid
roadmap/DEVOPS work only after P1 and P7 make their cross-wave environment and
ownership boundaries consistent.

### Iteration-2 finding count

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 2 |
| Medium | 3 |
| Low | 0 |

### Iteration-2 verdict

**NEEDS_REVISION.** P2-P4 are closed and the substantive route/cleanup/EDD
remediation is materially stronger, but two HIGH contradictions remain in the
effective native-metal and same-allocation contracts. Three MEDIUM defects also
leave the sibling oracle, lease boundary, and fresh-install EDD mapping
non-deterministic or incomplete. This does not meet the repository rule for
`APPROVED` or `CONDITIONALLY_APPROVED`.

## Iteration 3 — approved-DESIGN alignment and platform feasibility re-review

| Field | Value |
|---|---|
| Reviewed commit | `ed332f8972c6285fb067d995d73f51ee63a5ff01` |
| Comparison base / approved DESIGN | `85550e4a267cbd53ac266fa54f4d8cda164910af` |
| Review date | 2026-08-29 |
| Review iteration | 3 |
| Verdict | **NEEDS_REVISION** |

### Scope and verification

Iteration 3 reviewed the immutable DISTILL commit against its exact parent and
the approved DESIGN amendment at that parent. It re-evaluated P1-P8, the full
DISTILL artifacts, E07-E09, the production nft and lifecycle call sites, D7,
cleanup totality, and the shared metal-tree writer boundary. Later dirty source
and DELIVER work was excluded. The expectation files remain honest pending
stubs; no native-metal execution result is inferred.

Static checks:

- `git diff --check 85550e4a ed332f89` — passed;
- the target parent is exactly `85550e4a267cbd53ac266fa54f4d8cda164910af`;
- `git diff --check a5bd158e 85550e4a` — passed for the intervening approved
  DESIGN amendment;
- all three target expectation runners are executable and pass `bash -n`;
- the DISTILL commit changes documentation and pending expectation assets only;
  it changes no production, public API, Beacon, persistence, renderer, or
  observation-schema file.

### Prior-finding dispositions

| Prior finding | Iteration-3 disposition | Evidence |
|---|---|---|
| P1 — native-metal contract contradicted DESIGN/testing | **CLOSED** | Approved DESIGN now requires native, non-virtualized x86_64 hardware KVM, rejects nesting/Lima as runtime evidence, and retains `kvm-tests` only as the feature name (`design/wave-decisions.md:281-287`; `.claude/rules/testing.md`). DISTILL's fail-closed architecture, virtualization, `/dev/kvm`, API-version, and create-VM probes agree (`test-scenarios.md:151-165`). |
| P2 — RED classification was not immutable | **CLOSED (remains closed)** | The artifact pins base `85550e4a`, excludes dirty work, distinguishes inherited live bodies from semantic RED/new incomplete obligations, and records every current result as `NOT_EXECUTED` (`red-classification.md`; `feature-delta.md:953-964`). |
| P3 — exact counter oracle was impossible under REUSE-AS-IS | **CLOSED (remains closed)** | D7 explicitly sanctions the anonymous production counter and internal read-only projection, complete strict `GETRULE`/`GETGEN`, notification/loss guard, normalized full-program identity, exact AF_PACKET packet/IPv4-`tot_len` equality, and checked arithmetic (`design/wave-decisions.md:289-350`; `test-scenarios.md:81-139`). |
| P4 — S-GTI-08 depended on private/unreachable facts | **CLOSED (remains closed)** | The resolver case is port-visible through a deploy-selected custom rootfs; private reconciliation assertions remain component-scoped, and the post-READY exit-78 complement disambiguates the sentinel (`feature-delta.md:851-865`; E08 `README.md:7-14`). |
| P5 — same-id restart and stop used the wrong routes | **CLOSED** | Effective D6 now pins unclean control-plane restart -> boot-epoch platform reclamation -> `RestartAllocation` with the same allocation id, while natural Job exit/crash finalizes and `workload restart` creates a fresh allocation (`design/wave-decisions.md:18`). S-GTI-06a/06b and E09 use that route, and S-GTI-12 uses the real Job-stop port. The failed-install fixture itself remains defective under P8a, but the lifecycle route is no longer contradictory. |
| P6 — sibling position/order oracle was ambiguous | **CLOSED** | The before sequence contains full target-and-sibling snapshots and the after sequence must equal that exact sequence filtered by the target handle; target absence plus sibling identity, normalized program, counters, and relative order are explicit, with no absolute ordinal claim (`test-scenarios.md:140-149,330-345`). |
| P7 — lease did not cover the preceding shared sync | **PARTIALLY CLOSED; MEDIUM remains** | The outer supervisor now acquires before `rsync --delete` and holds one descriptor through final cleanup/probes (`test-scenarios.md:167-182`; E08 `README.md:24-29`). The named feature-specific lease still does not serialize all other writers to the same canonical remote checkout; see P7 below. |
| P8 — E08 falsely anchored S-GTI-05 without an explicit subcase | **PARTIALLY CLOSED; HIGH remains** | E08 now explicitly names the fresh install-failure command, state, wire, kernel, cleanup, fixture-restoration, and sibling evidence (`README.md:31-97`). Its nominated kernel fixture cannot cause the asserted error, and its Running-state oracle conflicts with the preserved production ordering; see P8a and P8b. |

### P8a — the regular hookless-chain fixture accepts TPROXY instead of rejecting it

**Severity:** HIGH  
**Status:** Open

E08 and S-GTI-05 require a regular, hookless chain named `prerouting` and claim
that `nft_tproxy_validate` rejects the real production append because the chain
has no reachable prerouting hook (E08 `README.md:51-60`;
`test-scenarios.md:250-257`; `feature-delta.md:995-999`). That premise is false
in the kernel API the scenario invokes.

Production `ensure_base_chain` sends `NFT_MSG_NEWCHAIN` with create semantics
and treats `EEXIST` as success (`crates/overdrive-netlink/src/nft.rs:842-877`).
The real installer then appends TPROXY to the pre-existing chain
(`crates/overdrive-worker/src/mtls_intercept.rs:487-522`). Upstream Linux
[`nft_tproxy_validate`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_tproxy.c#L294-L303)
does request `NF_INET_PRE_ROUTING`, but
[`nft_chain_validate_hooks`](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c#L10898-L10913)
checks that mask only for a base chain and returns success for a regular chain.
The append therefore succeeds in the proposed chain; the rule is merely
unreachable. E08 cannot drive the required terminal production-install error
and could instead certify a workload whose purported guard never traverses
traffic.

**Required remediation:** replace the fixture with a kernel arrangement whose
rejection is real and reproducible on the pinned native appliance kernel. One
viable shape is a pre-existing base chain with the production chain name at a
non-prerouting hook, so the unchanged production ensure receives `EEXIST` and
the unchanged real TPROXY append receives the hook-validation error. Pin the
actual failing operation and errno with a preflight probe, retain no test-owned
production error seam, and preserve the exact baseline/delta restoration and
post-failure cleanup evidence.

### P8b — E08's “never reaches Running” oracle contradicts the preserved install ordering

**Severity:** HIGH  
**Status:** Open

The fresh guard case requires its bounded poll trail to show terminal Failed
and “never reaches Running” (E08 `README.md:68-78`). The immutable production
path first writes the `Running` allocation row, then calls
`worker.start_alloc`; on install error it stops the driver and supersedes the
already-committed Running row with Failed
(`crates/overdrive-control-plane/src/action_shim/mod.rs:1653-1687,1715-1741`).
The restart arm has the same ordering (`:1993-2031`). The lifecycle event and
EXEC-release remain after successful install, but a concurrent describe read
can observe the durable intermediate row. The approved contract explicitly
preserves this Running-arm `start_alloc` placement and forbids turning the test
decorator into a production ordering rewrite (`feature-delta.md:867-872`).

S-GTI-05 itself states the feasible security invariant: terminal Failed,
operator command not run, no guest frame or cleartext egress, and complete
allocation cleanup (`test-scenarios.md:250-257`). It does not require that the
intermediate row was never written.

**Required remediation:** remove the impossible fresh-case “never reaches
Running” poll assertion. Prove the final Failed state and install detail, no
EXEC/operator marker, no guest-originated or cleartext traffic, and bounded
cleanup. It is valid to prove that no Running lifecycle event is emitted only
if the production event boundary supports that observation. If absence of even
a transient durable Running row is a product requirement, return to DESIGN and
approve a production state-ordering change before assigning it to DELIVER.

### P7 — the feature lease does not serialize every writer of the shared remote checkout

**Severity:** MEDIUM  
**Status:** Open; valid only as an explicit roadmap/DEVOPS precondition before
any runtime E07-E09 execution

The revised ownership epoch correctly starts before the feature run's sync and
ends after final probes. The remaining collision is wider than feature-command
concurrency. `MetalAction::Sync` invokes `metal_sync` directly, and
`MetalAction::Run` invokes the same sync before its remote command
(`xtask/src/main.rs:706-746`). `infra/metal/bootstrap.sh:124-141` writes every
caller into the same `~/overdrive` with `rsync --delete`. The contract only says
that “guest-stack metal/EDD commands” share the feature-specific lock
(`test-scenarios.md:167-181`); a generic `cargo xtask metal sync`, another
feature's metal run, or supported direct bootstrap can still overwrite that
tree during the protected epoch.

**Required remediation:** before any E07-E09 runtime claim, make the canonical
metal writer boundary participate in one shared-tree lease for `Run`, `Sync`,
and every supported direct bootstrap writer, while retaining a host-global
fixture/execution lease; alternatively sync each workspace/commit to an
isolated remote directory and retain the host-global fixture lease. The
roadmap/DEVOPS handoff must name the owner, exact writer set, acquisition order,
timeout diagnostics, and signal/error release. This is a valid downstream
condition because the DISTILL package honestly records the stubs as pending and
assigns harness construction before execution; it is not permission to run the
current commands and claim evidence.

### P9 — the slot-boundary property names a nonexistent constant

**Severity:** LOW  
**Status:** Open

`P-GTI-SLOT-BOUNDARY` repeatedly calls the bound `MAX_NET_SLOT`
(`test-scenarios.md:355-362,531-535`; `feature-delta.md:949`). The production
constant and approved DESIGN name are `NET_SLOT_MAX`
(`crates/overdrive-control-plane/src/veth_provisioner.rs:465`;
`design/wave-decisions.md:116-120`). Taken literally, the specified property
does not compile and does not identify the existing boundary.

**Required remediation:** replace `MAX_NET_SLOT` with `NET_SLOT_MAX` in every
DISTILL occurrence.

### D7, cleanup, EDD, and public-surface conclusion

D7 is implementable without a second nft owner or production test mutation:
the approved internal projection, strict multipart framing, generation and
notification guard, normalized-program identity, and checked exact packet/byte
oracle are coherent. The relative sibling snapshot now composes correctly with
handle-scoped deletion.

The cleanup contract is platform-observable and total over the named VMM,
cgroup, clone/index, run-directory, netns/tap/veth/route/nft, and capture
residue, with bounded deadlines and diagnostic-read failure unable to mask the
primary error or cleanup result. Fixture restoration is correctly separated
from product cleanup. E07-E09 are honest pending EDD stubs with real operator
commands and evidence classes. After replacing P8a's false fixture and P8b's
impossible state claim, the intended failure cases remain feasible without a
Beacon, public CLI schema, persistence schema, observation schema, renderer, or
test-only production seam.

P1, P2, P3, P4, P5, and P6 are closed. P7 is the only acceptable downstream
condition, and only under the explicit before-runtime ownership stated above.
P8a and P8b are acceptance-contract defects in the current DISTILL package and
must be corrected before approval.

### Iteration-3 finding count

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 2 |
| Medium | 1 |
| Low | 1 |

### Iteration-3 verdict

**NEEDS_REVISION.** Native-metal alignment, same-id routing, immutable RED
classification, D7, the relative sibling oracle, cleanup totality, and explicit
S-GTI-05 EDD mapping are materially corrected. Approval is still blocked by two
HIGH feasibility defects in E08: its proposed kernel fixture does not fail, and
its no-Running assertion contradicts the approved production ordering. The
shared-tree lease remains one explicitly valid downstream MEDIUM condition;
the constant-name defect is LOW.

## Iteration 4 — kernel-fixture and execution-boundary re-review

| Field | Value |
|---|---|
| Reviewed commit | `558589a7a3ee14ebea0b8cdb6496b65a5830f777` |
| Comparison base | `ed332f8972c6285fb067d995d73f51ee63a5ff01` |
| Effective approved DESIGN | `85550e4a267cbd53ac266fa54f4d8cda164910af` |
| Review date | 2026-08-29 |
| Review iteration | 4 |
| Verdict | **CONDITIONALLY_APPROVED** |

### Scope and verification

Iteration 4 reviewed the immutable ten-file DISTILL/expectation remediation,
the full effective artifacts, the approved DESIGN amendment, and the unchanged
production paths needed to establish feasibility. Particular scrutiny covered
the wrong-hook nft fixture, fresh/restart Running-row ordering, deferred EXEC,
the canonical metal-tree writer set, E09 same-id forcing and restoration, D7,
cleanup totality, constants, and native-host qualification. Later dirty source
and DELIVER work was excluded. The only file written by this reviewer is this
review artifact.

Static checks:

- the target's exact parent is `ed332f8972c6285fb067d995d73f51ee63a5ff01`;
- `git diff --check ed332f89 558589a7` — passed;
- the target changes three feature/DISTILL documents, E07-E09 README/runner
  pairs, and the expectation index; it changes no production or public schema;
- E07, E08, and E09 runner modes are `100755`, and all pass `bash -n` from the
  immutable target blobs;
- all three expectations remain `pending`, every current classification remains
  `NOT_EXECUTED`, and no runtime result is inferred from the documentation pass.

### Prior-finding dispositions

| Finding | Iteration-4 disposition | Evidence |
|---|---|---|
| P1 — native-metal alignment | **CLOSED (remains closed)** | Approved DESIGN and the DISTILL/EDD contract require native, non-virtualized x86_64 hardware KVM, reject nested/Lima runtime evidence, and pin architecture, `/dev/kvm`, API-12, create-VM-fd, and artifact preflight checks (`test-scenarios.md:160-174`; E07 `README.md:19-27`). The implementation is part of the explicit P7 pre-runtime condition. |
| P2 — immutable RED classification | **CLOSED (remains closed)** | The immutable base remains `85550e4a`, dirty work is excluded, and the new fixture/lease/E09 obligations are classified as incomplete rather than promoted to evidence (`red-classification.md`). |
| P3 — D7 feasibility | **CLOSED (remains closed)** | The anonymous production counter, strict complete `GETRULE`/`GETGEN`, normalized full-program identity, loss-detecting notification guard, checked AF_PACKET packet/IPv4-`tot_len` equality, and read-only ownership remain coherent (`test-scenarios.md:93-158`; E07 `README.md:52-72`). |
| P4 — resolver/private-state boundary | **CLOSED (remains closed)** | The deploy-selected custom-rootfs resolver failure remains port-visible; private View/action assertions stay component-scoped, and E08 retains the post-READY exit-78 complement. |
| P5 — same-id and stop routes | **CLOSED (remains closed)** | E09 drives unclean serve termination, unchanged durable data/intent, boot-epoch Platform Reclamation, no second deploy, and reuse of the same allocation id. Natural crash, restart budget, and `workload restart` are excluded (`E09 README.md:31-81`). Job stop remains the real operator verb. |
| P6 — relative sibling oracle and cleanup | **CLOSED (remains closed)** | The target-filtered ordered full-snapshot equality remains exact, and cleanup retains the bounded VMM/cgroup/clone/index/run-dir/netns/tap/veth/route/nft/capture residue set. |
| P7 — feature-local lease omitted other shared-tree writers | **CLOSED at specification level; MEDIUM downstream condition remains** | The contract now names one canonical host-global lock for every `MetalAction::Run`, `MetalAction::Sync`, and supported direct-bootstrap writer, requires acquisition before any mutation including `rsync --delete`, prohibits raw writers, and retains Run ownership through final probes (`test-scenarios.md:176-197`; E07 `README.md:39-50`). The canonical implementation has not landed; see the condition below. |
| P8a — hookless chain accepted TPROXY | **CLOSED** | E08 now uses a production-named **base** chain at INPUT, not a regular hookless chain, and preflights the exact encoded expression/errno on the appliance kernel before the real production append (`E08 README.md:55-92`). |
| P8b — fresh failure falsely forbade a transient Running row | **CLOSED** | DISTILL and E08 now acknowledge the committed Running row and require its terminal Failed supersession while preserving the actual security boundary: no EXEC, operator marker, guest frame, or cleartext (`test-scenarios.md:67-74,265-272`; E08 `README.md:94-127`). |
| P9 — nonexistent `MAX_NET_SLOT` | **CLOSED** | Every affected DISTILL occurrence now uses the production/approved `NET_SLOT_MAX` name (`test-scenarios.md:368-377,548-552`; `feature-delta.md:957`). |

### P8a closure — the wrong-hook base chain produces the required real kernel error

The corrected fixture is feasible and exercises the unchanged production
port. Production's `ensure_base_chain` treats the pre-existing production chain
name as idempotent `EEXIST` success
(`crates/overdrive-netlink/src/nft.rs:842-877`), after which the unchanged
outbound installer performs its real `append-rule`
(`crates/overdrive-worker/src/mtls_intercept.rs:487-522`). Upstream Linux
[`nft_tproxy_validate`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_tproxy.c#L294-L303)
requires `NF_INET_PRE_ROUTING`, while
[`nft_chain_validate_hooks`](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c#L10898-L10913)
returns `-EOPNOTSUPP` when a **base** chain's hook is outside that mask. An
INPUT-hook base chain therefore has the opposite and correct behavior from the
rejected regular hookless fixture.

E08 makes this non-vacuous: the disposable preflight uses the same encoded IPv4
TPROXY expression and exact typed errno, is removed before the scenario, and is
followed by an exact baseline recheck. The production subcase must identify
`OutboundTproxyInstall`, `append-egress`, `append-rule`, and `-EOPNOTSUPP`; an
absent or successful append, another errno, or an injection seam fails. Product
cleanup precedes separately trapped, delta-scoped fixture restoration.

### P8b closure — transient Running is distinguished from permission to execute

The corrected model matches both fresh and restart production arms. The action
shim commits the Running row before `start_alloc`; a failed install stops the
driver and supersedes that row with Failed before the VM exit-emission/EXEC gate
is released (`action_shim/mod.rs:1653-1755,1993-2060`). The artifacts no longer
equate a transient durable row with command execution. They require final
Failed with typed detail, no deferred EXEC release, no operator marker, no guest
frame or cleartext, and complete cleanup. The resolver subcase remains
correctly stricter because its failure is genuinely pre-READY/pre-Running.

This closes P8b without changing production ordering, the Beacon Published
Language, persistence, describe, observation schema, or any public surface.

### E09 feasibility — same-id forcing, destructive-fixture isolation, and restoration

The failed-reinstall journey is now sufficiently distinct from both fresh
deploy and successful sibling preservation:

- it first records a Running target, standing durable intent, allocation id,
  boot epoch, exact rule handle, and complete normalized nft/typed-FIB baseline;
- it terminates `serve` uncleanly, uses the same data directory, issues no
  second deploy or workload-restart command, and requires the
  Platform-Reclaimed ending plus same-id re-drive. Those facts uniquely force
  the boot-reclamation/restart path rather than the fresh install arm;
- it runs in a fresh sibling-free durable directory, with a quiescent original
  command and capture retained across the destructive window. The successful
  journey separately owns sibling preservation and D7/TLS proof;
- it replaces the captured production table only in that isolated journey,
  installs the same production-named INPUT-hook base-chain fixture, and demands
  the real restart-arm `append-egress` / `append-rule -EOPNOTSUPP` result;
- before mutation it computes the expected normalized post-cleanup state by
  filtering every target-scoped nft/FIB object from the baseline. EXIT/INT/TERM
  traps stop serve/capture, attempt bounded product cleanup, remove only the
  fixture, reconstruct the correct shared state without resurrecting the dead
  target, and require exact normalized nft plus typed-FIB equality even on
  assertion failure (`E09 README.md:51-91,117-126`).

The private `RestartAllocation` identity may be retained as supporting
structured trace evidence; the operator-visible Platform-Reclaimed row, same
allocation id, unchanged intent/data, absence of another deploy, and real
restart-site failure remain the non-vacuous route proof. No public action or
describe field is required.

### P7 condition — canonical writer lease and native preflight must land before evidence

**Severity:** MEDIUM  
**Disposition:** Explicit legitimate roadmap/DEVOPS condition

The specification defect is fixed, but the target intentionally does not
implement the canonical boundary. At the immutable source revision,
`MetalAction::Sync` still enters `metal_sync` directly and `MetalAction::Run`
still invokes it before the remote command (`xtask/src/main.rs:706-746`), while
bootstrap writes every caller into the same `~/overdrive` using
`rsync --delete` (`infra/metal/bootstrap.sh:124-141`). There is no universal
lease implementation yet. The existing metal provisioner also warns rather
than fails when virtualization is detected (`infra/metal/provision.sh:63-72`),
so the expectation-owned fail-closed native qualification remains necessary.

Before any E07-E09 runtime claim, roadmap/DEVOPS must land and verify:

1. one canonical `/run/lock/overdrive-metal-shared.lock` owner across Run, Sync,
   and every supported direct-bootstrap mutation;
2. acquisition acknowledgement before the first shared-tree write, including
   sync, with owner PID/start/action/scenario/workspace/commit diagnostics and a
   120-second timeout that aborts without mutation;
3. the same Run descriptor held through preflight, execution, evidence,
   assertion-safe cleanup/restoration, and final probes on normal, error, and
   signal paths; and
4. fail-closed native x86_64/KVM qualification, including literal
   non-virtualization, no hypervisor flag, hardware extensions, openable KVM API
   12, and a successful create-and-close VM-fd probe.

This is a legitimate downstream condition because E07-E09 and the immutable
classification explicitly invalidate runtime evidence until it lands. The
current unleased commands may not be run and promoted as proof.

### Native metal, D7, cleanup, completeness, and public-surface conclusion

Native execution semantics remain aligned with approved DESIGN. D7 remains a
read-only, loss- and mutation-conservative oracle over one unchanged production
rule. The wrong-hook preflight and fixtures are bounded external kernel state,
not production error injection. E08 and E09 separate product cleanup from
fixture restoration, install traps before mutation, and require final
normalized nft/FIB equality. E09's destructive failure setup cannot claim the
successful journey's sibling proof.

The revised AT audit honestly records the attempt-owned duplicate-create C4a
gap and reports 14/15 rather than fabricating 15/15. That acceptance-delivery
gap remains classified `AT_GAP_IN_DELIVERY_SCOPE`; it does not create a new
platform mechanism, public surface, or evidence claim. The pending EDD stubs
remain honest. No Beacon, REST/OpenAPI, persistence/rkyv, observation, renderer,
crate, daemon, dependency, or public-port expansion is needed.

### Iteration-4 finding count

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 1 |
| Low | 0 |

### Iteration-4 verdict

**CONDITIONALLY_APPROVED.** P8a, P8b, and P9 are closed: the wrong-hook base
chain produces the real production-port kernel error, transient Running is
correctly separated from EXEC permission, and the slot constant is exact. All
earlier platform findings remain closed. E09 now forces the same-id restart arm
and safely isolates/restores its destructive fixture. The sole MEDIUM is the
explicit and legitimate roadmap/DEVOPS prerequisite to implement the universal
pre-sync writer lease and fail-closed native runner before any runtime evidence
may be accepted.
