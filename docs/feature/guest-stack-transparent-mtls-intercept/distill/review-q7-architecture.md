# Q7 DISTILL Architecture Review — guest-stack-transparent-mtls-intercept

| Metadata | Value |
|---|---|
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `7255c68e64b7f0f15da5a2ed8a033806a2939e6a` |
| Parent | `29ab0bf71ef8c178a04035010d0ad72084b9ce7b` |
| Scope | Targeted Q7/Q9 DESIGN → DISTILL remediation |
| Verdict | **NEEDS REVISION** |

## Review scope and evidence

This review compared the three-file target delta against the approved DESIGN
chain `563fe26c` → `5590af67` → `29ab0bf7`, ADR-0088, ADR-0089, the effective
feature delta, and all three iterations in
`design/review-q7-remediation.md`. The roadmap was treated as intentionally
stale downstream work and was not scored.

Feasibility was checked against the target commit's production boundaries in:

- `crates/overdrive-control-plane/src/action_shim/mod.rs`
- `crates/overdrive-core/src/traits/driver.rs`
- `crates/overdrive-core/src/vm/config.rs`
- `crates/overdrive-init/src/main.rs`
- `crates/overdrive-reconcilers/src/workload_lifecycle.rs`
- `crates/overdrive-worker/src/vm_driver.rs`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs`
- `verification/expectations/`

`git diff --check 29ab0bf71ef8c178a04035010d0ad72084b9ce7b
7255c68e64b7f0f15da5a2ed8a033806a2939e6a` passed. No runtime suite was run:
the reviewed commit is documentation-only, and the findings below are static
contract/feasibility defects rather than an implementation verdict.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 2 |
| Medium | 1 |
| Low | 0 |

## Findings

### HIGH-1 — S-GTI-08 selects a malformed token state that the production deploy path cannot produce

**Evidence.** `distill/test-scenarios.md:237-248` requires a real Cloud
Hypervisor guest to receive a malformed platform network token "through the
production deploy path," and `:263-296` makes
`network_token_parse_malformed` the real-metal sample. That conflicts with the
actual input boundary:

- `VmPayload` exposes only command, args, kernel, and rootfs
  (`traits/driver.rs:469-480`).
- `KernelCmdline` is explicitly platform-owned and has no operator cmdline
  surface (`vm/config.rs:700-707`).
- `provision_and_inject_netns` derives the complete network channel from one
  typed, slot-derived `VmTapPlan` (`action_shim/mod.rs:925-954`).
- `compose_vm_network` accepts typed `Ipv4Addr` values, rejects prefixes above
  32, formats the sole token canonically, and appends it only if space-free
  (`vm_driver.rs:111-152`). The live `VmTapPlan` supplies the fixed /30, so even
  the rejected-prefix branch is not operator-reachable.

There is therefore no sanctioned `serve` + `deploy` input that produces the
specified malformed string. Mutating `VmConfig` in the observation decorator
would violate Q9's unchanged-delegation contract and the feature's no-test-only-
production-behaviour rule. Adding an operator cmdline/public override solely to
make the scenario possible would contradict the approved no-public-expansion
design.

**Impact.** The nominated real-metal example cannot be implemented honestly.
It would either remain permanently RED, test a synthetic state that production
cannot reach, or force an unsanctioned configuration/API expansion.

**Required remediation.** Use a real pre-READY failure that can be induced by
an existing production input or real guest environment while keeping
missing/malformed-token partitions at the source-local parser/helper boundary.
The metal case must still prove the complete Q7 outcome: no READY, Running,
EXEC, operator command, or guest `EXIT`; typed pre-READY VMM-exit cause;
bounded console precedence/fallback; exact terminal mapping; and no restart.
Document the precise production producer for the selected failure state.

### HIGH-2 — The approved no-restart/count-preservation example has no executable component/port mapping

**Evidence.** Approved DESIGN requires more than the pure classifier property.
`design/wave-decisions.md:142-157` explicitly requires a reconciler/action-shim
example that seeds nonzero private and durable restart counts, proves
`FinalizeFailed` is the only action, proves the returned
`WorkloadLifecycleView` equals its input, and proves the final row forwards its
prior durable `restart_count` unchanged. The approved review repeats that as a
mandatory downstream obligation (`design/review-q7-remediation.md:350-364`).

DISTILL places those internal assertions inside the real `serve` + `deploy`
scenario (`test-scenarios.md:237-251`), but its executable map
(`:384-402`) names only:

- the metal S-GTI-08 function,
- a source-local pure classifier property,
- guest setup/suppression tests, and
- driver diagnostic-selection examples.

It names no reconciler/action-shim example or other port that can observe both
the returned private view and the action-shim's durable row. A black-box metal
journey can observe the durable allocation result, but it cannot inspect the
reconciler's returned private view or enumerate the exact action vector.
Conversely, the classifier property proves only
`VmGuestExitUnreported → Failed`; it cannot prove `FinalizeFailed` exclusivity
or either count-preservation invariant. The feature delta nevertheless claims
that `S-GTI-08 classifier property + lifecycle example` covers this call site
(`feature-delta.md:738-747`) without defining that lifecycle example in the
DISTILL package.

**Impact.** Exact WorkloadLifecycle behavior, absence of
`RestartAllocation`, and the two independent restart-count preservation
contracts can all regress while every mapped test remains green. This is a
DESIGN → DISTILL fidelity and observability gap at the reconciler/action-shim
ports.

**Required remediation.** Add and map the approved source-local/component
example at the real `WorkloadLifecycle::reconcile` and action-shim observation
store boundaries. It must seed nonzero private and durable counts, assert the
single `FinalizeFailed` action with the exact terminal, assert no
`RestartAllocation`, compare returned and input views exactly, and prove the
superseding durable row retains its prior `restart_count`. Keep the metal case
for the production boot/diagnostic journey; do not add a public observation
field to expose private state.

### MEDIUM-1 — The new operator/end-to-end outcomes have no EDD expectation stub

**Evidence.** S-GTI-01 and S-GTI-02 are explicitly tagged
`@walking_skeleton @driving_port`; S-GTI-05, S-GTI-06, S-GTI-07, S-GTI-08, and
S-GTI-12 are also operator/end-to-end real-I/O outcomes. Repository verification
rules require such DISTILL scenarios to graduate into
`verification/expectations/<ID>/` and call a DISTILL package without a
corresponding stub incomplete. The target tree contains no expectation anchored
to `S-GTI-*` or `guest-stack-transparent-mtls-intercept`. Existing E04 and E05
are anchored to the canonical-address and service dial-by-name features; they
do not cover a real guest's pre-READY lifecycle, born-captured first connect,
restart re-enrolment, or teardown.

**Impact.** The DISTILL package has no black-box evidence contract for its new
operator-visible security and failure outcomes, so DELIVER has no feature-
anchored expectation to capture or audit.

**Required remediation.** Add the minimum non-duplicative E/O-surface
expectation stub or stubs, anchored to the relevant S-GTI scenarios. Keep pure
classifier, suppression, and diagnostic-selection cases in Rust only; the EDD
surface should describe the operator-visible real-binary journey and remain
`pending` until its honest metal capture is available.

## Fidelity matrix

| Required contract | Result | Evidence |
|---|---|---|
| Networking and suppression complete before READY | PASS | Q7 state machine and S-GTI-08 failure table put init/token/NIC-down/IPv6/`arp_notify`/static apply/resolver before READY. |
| Beacon Published Language unchanged | PASS | Q7 compatibility pins explicitly forbid Beacon/`ExitKind` expansion and delete setup `EXIT`. |
| Typed pre-READY VMM-exit cause | PASS with HIGH-1/2 caveats | S-GTI-08 requires `VmGuestExitUnreported` with exact VMM code/signal, but its chosen producer is unreachable and lifecycle proof is unmapped. |
| Bounded console diagnostic and fallback | PASS | 8 KiB/five-fragment, unterminated, lossy UTF-8, precedence, stderr, and neither-source partitions are explicit. |
| Exact Job terminal mapping, no restart, count preservation | FAIL | Pure mapping is pinned; the required reconciler/action-shim example is absent from the executable map (HIGH-2). |
| Zero-L2 boot witness and conservative oracle | PASS | Exact identity tuple, both interfaces, all EtherTypes, drops/overflow/malformed/unknown failures, and exact rule-live boundary are pinned. |
| Intercept-live and first-connect ordering | PASS | Full `capture-ready ≺ ... ≺ operator-first-connect` chain plus first five-tuple → rule → leg-F → TLS/kTLS/no-cleartext is explicit. |
| Both D6 install gates | PASS | S-GTI-01 covers fresh start; S-GTI-06 requires genuine same-allocation restart and covers restart install success/failure. |
| Restart and teardown invariants | PASS with HIGH-2 caveat | S-GTI-06 and S-GTI-12 cover reinstall and ungated teardown; pre-READY no-restart accounting lacks its component proof. |
| No public-field expansion | PASS | The delta is documentation-only and explicitly reuses existing reason/detail/row shapes; diagnostic and observer seams remain private/test-only. |
| Operator expectation coverage | FAIL | No S-GTI-anchored expectation stub exists (MEDIUM-1). |

## Verdict

**NEEDS REVISION.** The Q7 lifecycle prose, Q9 closed packet oracle, D6 fresh
and restart gates, teardown inverse, and no-public-expansion constraints are
substantially faithful to approved DESIGN. Approval is blocked by one
unproducible real-metal premise, one missing reconciler/action-shim proof
boundary, and the missing EDD handoff. Re-review is required after all three
findings are remediated.

---

# Iteration 2 — remediation re-review

| Metadata | Value |
|---|---|
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `a5bd158edb76308a44ccf99597d349950898f0f7` |
| Parent | `cd12725159a6b2a92619f17aa4dc5f0ff621b842` |
| Effective approved DESIGN | `cd12725159a6b2a92619f17aa4dc5f0ff621b842` |
| Scope | Iteration-1 remediation plus full effective DESIGN fidelity |
| Verdict | **NEEDS REVISION** |

## Iteration-2 evidence and prior-finding disposition

The target was reviewed from its immutable Git tree against the approved D7
amendment, ADR-0088/ADR-0089, the repository metal-test contract, and the
production seams needed to establish feasibility. `git diff --check
cd12725159a6b2a92619f17aa4dc5f0ff621b842
a5bd158edb76308a44ccf99597d349950898f0f7` passed. The target is a
documentation/expectation delta, so this is a static architecture and
verification-contract review rather than a runtime verdict.

| Prior finding | Disposition | Evidence |
|---|---|---|
| HIGH-1 — unreachable malformed-token metal state | **CLOSED** | S-GTI-08a now uses an operator-selected rootfs whose resolver target makes the production write fail. `VmPayload.rootfs` is a real deploy input (`traits/driver.rs:480`), the VMM attaches a writable per-launch FICLONE (`overdrive-host/src/vmm.rs:263,386,590-603`), and guest init executes the real `fs::write("/etc/resolv.conf", ...)` (`overdrive-init/src/main.rs:422-424`). A rootfs with that path represented by a directory makes the production call fail without mutating `VmConfig`, adding an operator override, or introducing test-only behavior. Malformed token cases correctly remain source-local. |
| HIGH-2 — missing reconciler/action-shim proof | **CLOSED** | `test-scenarios.md:380-392` now defines `C-GTI-08-RECONCILE` at the existing reconciler/action-shim boundaries: seeded private View and nonzero durable count, exactly one failed finalization, no restart, exact View equality, and durable count preservation. The adapter map names that executable component obligation at `:464-479`; no public observation field is added. |
| MEDIUM-1 — no EDD stubs | **REMAINS OPEN, narrowed** | E07/E08/E09 now exist as honest `pending` stubs, so the absence defect is remediated. E08 nevertheless cites S-GTI-05 without specifying or driving its distinct fresh guard-install failure; see MEDIUM-1 below. |

## Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 3 |
| Low | 0 |

## Iteration-2 findings

### MEDIUM-1 — E08 cites S-GTI-05 but never specifies its fresh guard-install failure journey

**Evidence.** S-GTI-05 is a distinct real-I/O contract: arrange a real kernel
error from the production nft install, deploy a fresh VM Job, require actionable
guard-install detail, forbid command release and egress, and prove bounded
cleanup (`distill/test-scenarios.md:234-241`). E08 lists S-GTI-05 as an anchor
(`verification/expectations/E08-vm-guest-boot-failure-truthful-and-clean/README.md:14-18`),
but its expectation, eventual command, and required evidence contain only the
custom-rootfs resolver failure and the post-READY exit-78 complement (`:5-12,
:26-59`). E07 covers successful fresh install and E09 covers failed reinstall
after reclamation; neither covers the fresh-install failure.

**Impact.** The feature/package index reports S-GTI-05 as graduated while no
EDD contract can drive or audit its separate D6/D-MTLS-18 failure path. A
production regression that releases EXEC or leaves residue after a fresh nft
install error is outside all three stubs.

**Required remediation.** Add the fresh production guard-install failure as an
explicit E08 subcase, including the real command/environment arrangement and
command/state, wire, kernel, and cleanup evidence, or add a separate minimal
pending expectation for S-GTI-05 and remove the false E08 anchor. Do not merge
it semantically with the resolver failure or E09's same-allocation reinstall
failure.

### MEDIUM-2 — DISTILL forbids the nested-KVM execution surface that approved DESIGN requires

**Evidence.** Approved DESIGN requires the restart and Slice-1 Tier-3 examples
on the same nested-KVM metal surface, gated by `kvm-tests` and invoked through
`cargo xtask metal run --` (`design/wave-decisions.md:68-77`; effective
`feature-delta.md:591-596,640-647`). The repository's canonical execution rule
likewise says the real Cloud Hypervisor surface needs `x86_64 + nested KVM` and
routes it through the metal runner (`.claude/rules/testing.md:1447-1468`). The
new DISTILL contract instead says runtime is allowed only on a native,
non-virtualized host and explicitly forbids nested KVM
(`distill/test-scenarios.md:141-153`). It then misattributes that restriction to
DESIGN (`feature-delta.md:818`) and carries it into E07/E08/E09 and the handoff
(`feature-delta.md:1011-1014`; E08 `README.md:20-24`; E09 `README.md:20-25`).

**Impact.** The executable acceptance contract rejects an environment the
approved architecture and repository test policy explicitly nominate. A
conforming nested-KVM metal lane would be reported as blocked before any
scenario runs, while DISTILL would falsely present the rejection as inherited
DESIGN.

**Required remediation.** Align the DISTILL preflight and all three EDD stubs
with the approved canonical `x86_64`, `kvm-tests`, `cargo xtask metal run --`
surface. Keep fail-closed `/dev/kvm`, KVM API, artifact, cgroup, and lease
checks, but remove the non-virtualized-only and nested-KVM rejection unless the
DESIGN and repository testing contract are separately amended and approved.

### MEDIUM-3 — “Exact sibling position” is undefined across deletion of the target rule

**Evidence.** Approved D7 requires exact target-handle deletion and says a
quiescent sibling snapshot remains equal across target teardown
(`design/wave-decisions.md:332-338`). Its internal normalized `RuleInfo` shape
contains rule identity and counter state, not an absolute chain ordinal
(`:268-277`). DISTILL strengthens that to every sibling retaining exact
“position” (`distill/test-scenarios.md:134-139`) and S-GTI-12/E09 require exact
“order” (`:315-330`; E09 `README.md:54-57`) without defining whether this means
absolute ordinal or relative order among surviving siblings. If a deleted
target precedes a sibling, that sibling's absolute chain ordinal necessarily
changes even though the sibling rule is untouched.

**Impact.** A correct exact-handle teardown can fail the acceptance test under
the absolute interpretation, while a harness using a weaker interpretation can
claim success without a shared oracle. The contract is therefore not
deterministically executable.

**Required remediation.** Define sibling preservation as equality of the
ordered sequence of surviving sibling identities and counter snapshots after
filtering out the exact target handle (thereby preserving relative sibling
order), or define another explicit DESIGN-sanctioned comparison. Remove any
claim that an absolute ordinal remains unchanged when deletion ahead of a
sibling can shift it.

## Iteration-2 conformance matrix

| Required contract | Result | Evidence |
|---|---|---|
| Production-reachable custom-rootfs boot failure | PASS | Real deploy input reaches the writable cloned rootfs and production resolver write; no test-only route, cmdline, or success result is added. |
| Private lifecycle/action-shim state coverage | PASS | `C-GTI-08-RECONCILE` owns exact private View, action-vector, finalization, no-restart, and durable-count assertions. |
| Exact D7 packet/counter oracle | PASS | Full userdata+handle+normalized-program identity, strict GETRULE/GETGEN framing, notification-loss guard, quiet cuts, exact AF_PACKET packet/IPv4-`tot_len` equality, checked arithmetic, TLS/no-cleartext, adoption, teardown, and sweep constraints faithfully carry approved D7. |
| READY/EXEC/EXIT lifecycle | PASS | Pre-READY failure remains terminal/no-EXEC; post-READY status 78 remains an ordinary Job result. |
| Route/resource cleanup | PASS with MEDIUM-3 qualification | The bounded residue set covers VMM, cgroup, clone/index, run directory, netns, tap/veth/route, nft rule, and observer resources; the sibling-position oracle needs one precise interpretation. |
| Global lease | PASS | Host-wide path, finite acquisition, owner metadata, command-lifetime ownership, cleanup/final-probe coverage, and cross-worktree serialization are coherent. |
| Execution substrate | FAIL | Native-only/nested-forbidden contradicts approved DESIGN and the canonical repository metal lane (MEDIUM-2). |
| Public contract shape | PASS | Q7 observation remains at existing ports/private component state; D7's internal rule/counter projection is the approved bounded extension. |
| EDD graduation | FAIL | Honest pending E07/E08/E09 stubs exist, but no expectation actually specifies S-GTI-05 (MEDIUM-1). |

## Iteration-2 verdict

**NEEDS REVISION.** Both prior HIGH findings are closed: the failure is now
production-reachable without test-only behavior, and the private
reconciler/action-shim contract is executable at the correct component ports.
The ratified D7 exact-counter oracle, lifecycle mapping, cleanup set, lease, and
public-shape constraints otherwise remain coherent. Approval is withheld for
three medium defects: missing real EDD coverage for fresh guard-install
failure, a direct execution-substrate contradiction with approved DESIGN, and
an undefined absolute-versus-relative sibling-order oracle.

---

# Iteration 3 — remediation re-review

| Metadata | Value |
|---|---|
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `ed332f8972c6285fb067d995d73f51ee63a5ff01` |
| Parent / effective approved DESIGN | `85550e4a267cbd53ac266fa54f4d8cda164910af` |
| Scope | Iteration-2 remediation plus full effective DESIGN and production-feasibility review |
| Verdict | **NEEDS REVISION** |

## Iteration-3 evidence and prior-finding disposition

The target's ten-file documentation/expectation delta was read from its
immutable Git tree. Production feasibility was rechecked at the target commit,
including the nft ensure/append path, the generic metal launcher and sync path,
the guest-init/rootfs path, and the lifecycle/component boundaries. The target's
Linux claim was checked against the upstream kernel implementation it names.
`git diff --check 85550e4a267cbd53ac266fa54f4d8cda164910af
ed332f8972c6285fb067d995d73f51ee63a5ff01` passed, and all three pending runner
stubs passed `bash -n`. No runtime suite was run for this documentation-only
review.

| Prior finding | Disposition | Evidence |
|---|---|---|
| MEDIUM-1 — E08 omitted the fresh guard-install failure | **NOT CLOSED; escalated to HIGH-1** | E08 now contains a distinct command, kernel, state/wire, cleanup, and fixture-restoration subcase. Its nominated hookless-chain producer does not cause the claimed kernel rejection, so the journey is still not executable as specified. |
| MEDIUM-2 — DISTILL rejected approved nested KVM | **CLOSED** | Approved DESIGN commit `85550e4a` corrected the authoritative substrate to native, non-virtualized x86_64 hardware-backed KVM. DISTILL, `.claude/rules/testing.md`, ADRs, and E07-E09 now agree on that boundary and on the canonical metal runner. |
| MEDIUM-3 — absolute sibling position was undefined | **CLOSED** | `test-scenarios.md:141-149`, S-GTI-12a/b (`:329-344`), and E09 (`README.md:55-62`) now define exact target deletion as equality of the ordered surviving snapshot sequence after filtering the target handle. This preserves sibling values and relative order without claiming an impossible absolute ordinal. |
| Iteration-1 HIGH-1/HIGH-2 | **REMAIN CLOSED** | The custom-rootfs resolver failure remains production-reachable without a test-only behavior, and the private reconciler/action-shim proof remains explicitly mapped. |

## Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 1 |
| Medium | 1 |
| Low | 1 |

## Iteration-3 findings

### HIGH-1 — A regular hookless nft chain accepts TPROXY, so E08 cannot produce the specified install failure

**Evidence.** S-GTI-05 says the E08 regular `prerouting` chain makes the real
production TPROXY append fail (`distill/test-scenarios.md:250-257`). E08 makes
the mechanism exact: create `ip overdrive-mtls` with a regular, unreferenced,
hookless chain named `prerouting`; production treats the name as present; then
`nft_tproxy_validate` supposedly rejects the append because the chain has no
`NF_INET_PRE_ROUTING` reachability (`verification/expectations/E08-vm-guest-boot-failure-truthful-and-clean/README.md:51-60`).

That kernel premise is false. Production `ensure_base_chain` issues
`NFT_MSG_NEWCHAIN` with `NLM_F_CREATE` and treats `EEXIST` as idempotent success
without verifying the existing chain's hook/type
(`crates/overdrive-netlink/src/nft.rs:842-877`). Production then reaches its real
append (`overdrive-worker/src/mtls_intercept.rs:487-522`). Upstream
[`nft_tproxy_validate`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_tproxy.c#L315-L324)
does call `nft_chain_validate_hooks`, but
[`nft_chain_validate_hooks`](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c#L11566-L11580)
checks the hook only when the destination is a base chain and returns success
for every regular chain. A hookless regular chain therefore accepts the TPROXY
expression; being unreferenced affects packet traversal, not rule insertion.

**Impact.** The production install returns success instead of the required
error. The allocation can proceed toward Running/EXEC with its rule stranded in
an unhooked chain, so the fixture tests a fail-open environment rather than the
D-MTLS-18 terminal path. The scenario may fail on its later wire assertion, but
it cannot prove truthful fresh-install rejection, terminal detail, or the
associated product cleanup.

**Required remediation.** Replace the fixture with a production-reachable,
deterministic kernel rejection and pin it against the actual appliance kernel.
For example, an existing base chain of the production name bound to an
unsupported hook causes the same idempotent `EEXIST` path followed by the real
TPROXY hook validation failure; the exact fixture must be proven before it is
ratified. Preserve the current no-injection constraint, recorded operation and
errno, clean-baseline precondition, delta-only restoration, and separation of
product cleanup from fixture teardown.

### MEDIUM-1 — The pre-sync feature lease does not exclude other writers of the shared metal tree

**Evidence.** The revised ordering is correct among participating #222 runs:
the remote descriptor is acquired before `rsync --delete` and held through
final probes (`distill/test-scenarios.md:167-182`; E07 `README.md:39-48`). The
contract nevertheless says only “guest-stack metal/EDD commands” share the
feature-named lock while claiming that the epoch serializes the shared remote
tree across worktrees. At the target commit, every generic `cargo xtask metal
run` calls `metal_sync` before executing its command, and `cargo xtask metal
sync` calls it directly (`xtask/src/main.rs:706-746`); neither path participates
in the feature lock. The bootstrap then applies `rsync --delete` to the same
`$HOME/overdrive` tree (`infra/metal/bootstrap.sh:124-142`). An unrelated metal
run or sync from another worktree can therefore replace that tree while E07,
E08, or E09 holds the feature lease.

**Impact.** Source, binaries, or evidence inputs can change during a supposedly
commit-pinned acceptance run even though its lock is held. The asserted
cross-worktree serialization is false unless every writer of the common remote
directory participates.

**Required remediation.** Put one shared-tree lease in the canonical metal
sync/run boundary so every `MetalAction::Run`, `MetalAction::Sync`, and other
supported writer acquires it before the first remote-tree mutation and holds it
through the associated command/final probes, or give each worktree an isolated
remote directory. If a universal xtask/bootstrap change is already intended,
state that explicitly and map the `Sync` and direct-bootstrap writer paths;
feature-runner participation alone is insufficient.

### LOW-1 — The slot-boundary executable contract names a nonexistent constant

**Evidence.** `P-GTI-SLOT-BOUNDARY` repeatedly requires
`MAX_NET_SLOT + 1` (`distill/test-scenarios.md:355-362,531-535` and
`feature-delta.md:949`). Approved DESIGN and production use
`NET_SLOT_MAX`; no `MAX_NET_SLOT` symbol exists
(`design/wave-decisions.md:116-120`; `veth_provisioner.rs:465,508-515`).

**Impact.** A crafter following the exact named action cannot compile it, and
the stable executable mapping drifts from the implementation boundary it is
supposed to pin.

**Required remediation.** Replace every `MAX_NET_SLOT` occurrence in the
DISTILL contract with `NET_SLOT_MAX`.

## Iteration-3 conformance matrix

| Required contract | Result | Evidence |
|---|---|---|
| E08 fresh install-failure graduation | FAIL | The subcase and evidence contract now exist, but the hookless regular-chain fixture accepts TPROXY instead of rejecting it (HIGH-1). |
| Native-metal alignment | PASS | Effective DESIGN `85550e4a`, repository testing rules, DISTILL, and E07-E09 consistently require native non-virtualized x86_64 KVM and reject nested/Lima runtime evidence. |
| Relative sibling order | PASS | `A == filter(B, handle != target_handle)` precisely preserves the ordered surviving full snapshots. |
| D7 exact counter oracle | PASS | S-GTI-02/E07 retain strict complete GETRULE/GETGEN framing, full normalized program identity, loss detection, quiet cuts, exact packet/IPv4-`tot_len` equality, checked arithmetic, TLS/no-cleartext, and conservative mutation failure. S-GTI-01 remains the stakeholder-language view of that same E07 journey. |
| Custom-rootfs failure | PASS | The deploy-selected rootfs still reaches a writable per-launch clone and the production resolver write; malformed-token cases remain source-local and no functional test seam is introduced. |
| Component/private-state mapping | PASS with LOW-1 typo | `C-GTI-08-RECONCILE`, exit-78 classification, pre-READY error partitions, D7 closure, illegal-event properties, and mutation replay all name bounded internal/source-local ports without exposing private View/action state publicly. The slot constant name needs mechanical correction. |
| Lease-before-sync | FAIL | Acquisition precedes sync for participating feature runs, but not all writers of the shared remote tree participate (MEDIUM-1). |
| Cleanup and fixture ownership | PASS conditional on HIGH-1 repair | Allocation residue and fixture restoration are explicitly separate, delta-scoped, bounded, and sibling-preserving; the nominated failure must first become real. |
| Public contract shapes | PASS | No Beacon, REST/OpenAPI, persistence/rkyv, describe, observation, crate, daemon, dependency, or public-port expansion is introduced. The approved internal D7 projection remains bounded. |

## Iteration-3 verdict

**NEEDS REVISION.** Native-metal alignment and relative sibling-order semantics
are now closed, and the D7, custom-rootfs, private component, cleanup, and
public-shape contracts remain coherent. Approval is blocked because E08's new
fresh-install fixture relies on a kernel behavior that is the opposite of the
real nftables implementation, and because the pre-sync lease does not yet cover
every writer of the shared remote tree. The slot constant typo is non-blocking
but should be corrected with the same remediation.

---

# Iteration 4 — remediation re-review

| Metadata | Value |
|---|---|
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `558589a7a3ee14ebea0b8cdb6496b65a5830f777` |
| Parent | `ed332f8972c6285fb067d995d73f51ee63a5ff01` |
| Effective approved DESIGN | `85550e4a267cbd53ac266fa54f4d8cda164910af` |
| Scope | Iteration-3 remediation plus full effective DESIGN and production-feasibility regression review |
| Verdict | **APPROVED** |

## Iteration-4 evidence and prior-finding disposition

The ten-file target delta was reviewed from its immutable Git tree against the
approved D6/Q7/Q9/D7 design, ADR-0088/ADR-0089, the production nft install and
boot-sweep paths, both action-shim install sites, the canonical metal writer
surface, and E07-E09's pending evidence contracts. `git diff --check
ed332f8972c6285fb067d995d73f51ee63a5ff01
558589a7a3ee14ebea0b8cdb6496b65a5830f777` passed, and all three runner stubs
passed `bash -n`. No runtime suite was run: the target changes specification and
pending expectation assets only and continues to classify runtime evidence as
absent.

| Prior finding | Disposition | Evidence |
|---|---|---|
| HIGH-1 — hookless regular chain accepts TPROXY | **CLOSED** | E08 now uses a production-named **base** chain at the INPUT hook (`README.md:55-81`). Production still receives `EEXIST` through `ensure_base_chain` and reaches the unchanged real append (`overdrive-netlink/src/nft.rs:842-897`; `overdrive-worker/src/mtls_intercept.rs:487-522`). Unlike a regular chain, a base chain is checked by `nft_chain_validate_hooks`; INPUT is outside TPROXY's required `NF_INET_PRE_ROUTING` mask, so the append returns `-EOPNOTSUPP`. The contract preflights the same encoded expression on the appliance kernel and requires the exact `OutboundTproxyInstall` → `append-egress` → `append-rule` cause chain. |
| MEDIUM-1 — feature lease did not exclude generic/shared-tree writers | **CLOSED at the specification boundary** | The canonical lease is now `/run/lock/overdrive-metal-shared.lock`; every `MetalAction::Run`, `MetalAction::Sync`, and supported direct-bootstrap writer must acquire it before the first shared mutation, including `rsync --delete` (`test-scenarios.md:176-197`; `feature-delta.md:1014-1021`). Run retains the same remote descriptor through final probes, raw/legacy writers are prohibited, timeout/owner metadata is defined, and E07-E09 explicitly cannot produce valid runtime evidence until this roadmap/DEVOPS prerequisite lands. |
| LOW-1 — nonexistent `MAX_NET_SLOT` | **CLOSED** | The boundary property, audit, and handoff consistently invoke `NetSlot::new(NET_SLOT_MAX + 1)` (`test-scenarios.md:368-377,548-556`; `feature-delta.md:957`). No stale `MAX_NET_SLOT` occurrence remains in the reviewed feature/expectation artifacts. |
| Iteration-1 HIGH-1/HIGH-2 and Iteration-2 MEDIUM-2/MEDIUM-3 | **REMAIN CLOSED** | The custom-rootfs resolver producer, private reconciler/action-shim proof, approved native non-virtualized KVM substrate, and relative sibling sequence oracle are unchanged and remain correctly mapped. |

## Iteration-4 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

## Architecture and fidelity regression matrix

| Contract | Result | Evidence |
|---|---|---|
| Fresh production install rejection | **PASS** | E08 starts from a clean feature nft/FIB baseline, installs restoration traps before mutation, proves the INPUT-hook base-chain rejection with the production encoder, requires the real typed production error, observes product cleanup separately, and restores only the recorded delta (`E08 README.md:55-127`). No injection seam or test-owned success result is allowed. |
| Same-id failed reinstall | **PASS** | E09 separates a sibling-free destructive failure journey from the successful/sibling-preserving journey. It records a Running target, standing intent, allocation id, boot epoch, target handle, and normalized nft/FIB baseline; performs no second deploy; and requires Platform Reclamation plus the exact same-id `RestartAllocation` dispatch before the real restart-arm append fails (`E09 README.md:42-81`). Natural exit, restart budget, generation replacement, and fresh install cannot satisfy the oracle. |
| E09 isolation and restoration | **PASS** | The destructive subcase has exactly one target and no sibling, arms signal-safe restoration before mutation, preserves capture through the unguarded fixture window, proves product cleanup before fixture cleanup, never recreates target-scoped objects, and requires normalized nft plus typed FIB equality to the precomputed `filter_target(baseline)` on success, assertion failure, or signal (`E09 README.md:51-91,117-126`). Target handle evidence is recorded separately from normalized whole-state restoration, so the contract does not claim preservation of a kernel-assigned target handle after deliberate table replacement. Sibling nonmutation remains owned only by the separate non-destructive journey. |
| Durable Running ordering | **PASS** | Both immutable production arms write the Running row before calling `start_alloc`, then call `release_for_exit_emission` only after install success (`action_shim/mod.rs:1687-1755,1993-2060`). DISTILL and E08/E09 now permit that durable transient while requiring the security-relevant result: superseding Failed, typed detail, no EXEC/operator marker, no guest frame/cleartext, and total cleanup (`test-scenarios.md:67-74`; E08 `README.md:96-110`; E09 `README.md:74-105`). The resolver case correctly remains pre-READY and therefore still forbids Running. |
| Universal writer exclusion | **PASS as an explicit prerequisite** | Run, Sync, and supported direct bootstrap share one remote host-global ownership epoch; acquisition precedes every shared-tree mutation and Run spans execution, evidence, cleanup, and final probes. The immutable tree does not yet implement this boundary, and the artifacts truthfully make that a runtime-evidence blocker rather than claiming present isolation. |
| D7 exact-rule witness | **PASS** | Complete strict `GETRULE`/`GETGEN`, full normalized encoder identity, generation/notification loss guard, quiet cuts, exact packet/IPv4-`tot_len` equality, checked arithmetic, original destination, TLS/no-cleartext, adoption, boot sweep, and exact target-handle teardown remain intact. The success observer remains read-only; E08/E09's explicitly owned failure fixture is not substituted for D7 success evidence. |
| Q7 failure and exit semantics | **PASS** | Production-reachable custom-rootfs resolver failure remains pre-READY, exact private no-restart/count behavior remains in `C-GTI-08-RECONCILE`, and READY → EXEC → ordinary exit 78 remains the discriminating complement. Beacon, describe, persistence, and observation shapes are unchanged. |
| D6 restart and stop routes | **PASS** | Unclean boot with standing intent remains the sole same-allocation VM Job re-drive; generation replacement remains fresh-id. E09's success journey retains guard-before-EXEC/D7 protection, and S-GTI-12a/b retain exact target deletion plus ordered surviving-sibling equality after filtering the target handle. |
| Native execution substrate | **PASS** | Effective DESIGN, repository testing rules, DISTILL, and E07-E09 consistently require native non-virtualized x86_64 hardware KVM; nested/Lima execution remains non-signal. |
| Immutable status and AT audit | **PASS** | `red-classification.md` continues to separate inherited bodies, RED scaffolds, and newly incomplete obligations with every current result `NOT_EXECUTED`. The canonical audit now honestly records the grouped duplicate-create C4a gap and reports 14/15, COMPLETE by the defined threshold, rather than presenting teardown replay as create-twice evidence. |
| Reuse and public contract | **PASS** | The authoritative 8 REUSE-AS-IS / 10 EXTEND / 1 CREATE-NEW tally is unchanged. No new crate, daemon, dependency, Beacon field, REST/OpenAPI field, persistence/rkyv field, describe field, or observation-schema surface is introduced. |

## Iteration-4 verdict

**APPROVED.** The corrected INPUT-hook base-chain fixture now reaches a real,
deterministic production TPROXY rejection; the canonical lease contract covers
every supported writer of the shared remote tree before mutation; and the slot
boundary names the real constant. The transient Running clarification matches
both production install sites without weakening no-EXEC/fail-closed behavior.
E09's failed-reinstall fixture is isolated, proves the restart arm rather than a
fresh path, and carries an assertion-safe target-filtered restoration oracle,
while successful reclamation and sibling preservation remain separate. The D7,
Q7, D6, native-metal, cleanup, reuse, and public-shape contracts remain faithful
to effective approved DESIGN. There are zero Blocker, Critical, High, Medium,
or Low findings.
