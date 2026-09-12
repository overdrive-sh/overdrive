# Landlock DESIGN Remediation Review

## Metadata

| Field | Value |
|---|---|
| Review role | `nw-solution-architect-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 1 |
| Base | `dbb5dbb79fc7ba236c44075d8eb7214d8a38c32d` |
| Reviewed files | `docs/product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md`; `docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md`; `docs/product/architecture/brief.md` |
| Verdict | **CHANGES_REQUESTED** |

## Authorized scope

This review covers only the uncommitted Landlock DESIGN reconciliation in the
three files above. It assesses the selected allocation TAP sysfs read grant,
the exact private producer/composer shape, least privilege, public-API
preservation, the two named native-metal test transitions, and consistency
between ADR-0082, ADR-0089, and the architecture brief.

All other dirty files, the active step `01-02` implementation, tests, roadmap,
DES log, staging area, commits, and branch state were excluded and preserved.
No production or design source was edited by this reviewer.

## Production and reproducer evidence

### Current production owner path

The claimed failure is reachable through the real production composition:

1. `provision_and_inject_netns` assigns one `NetSlot`, derives
   `VmTapPlan`, converges the TAP, and injects the same `tap.tap` into
   `AllocationSpec.guest_tap`
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1180-1218,
   1225-1238`).
2. `derive_vm_tap_plan` alone renders the production TAP name as
   `ovd-tp-<slot.to_hex4()>`; the prefix plus four lowercase hexadecimal
   characters is statically inside IFNAMSIZ
   (`crates/overdrive-control-plane/src/veth_provisioner.rs:752-757,
   802-830`). The provisioner creates that exact persistent TAP, moves it into
   the allocation netns, sets owner uid `4200`, addresses it, and observes the
   same name before `Driver::start`
   (`veth_provisioner.rs:2726-2803, 2934-2957`).
3. `VmDriver::provision_vmm` converts `AllocationSpec.guest_tap` without a
   second derivation into `VmNetworkAttachment.tap`, then places the complete
   attachment on `VmConfig.network`
   (`crates/overdrive-worker/src/vm_driver.rs:123-195, 1134-1155,
   1296-1319`).
4. The driver calls the existing `Vmm::create(&VmConfig)` port
   (`vm_driver.rs:1343-1363`). `CloudHypervisorVmm::build_confined_command`
   uses `VmNetworkAttachment.tap` for `--net tap=...` and, at the reviewed
   base, independently formats the same value into
   `/sys/class/net/<tap>,access=r` before adding the run-directory rule from
   `VmConfig::landlock_rules()`
   (`crates/overdrive-host/src/vmm.rs:238-298, 313-324, 447-463`).

The proposed move therefore changes the authority and representation of an
already-required argument; it does not create a new production state, caller,
TAP identity, lifecycle gate, persistence boundary, or security subsystem.

### Cloud Hypervisor behavior

Cloud Hypervisor v53's `net_util::open_tap` checks the selected
`/sys/class/net/<tap>` entry and reads
`/sys/class/net/<tap>/tun_flags`; read failure is mapped to
`ReadSysfsTunFlags` with the message `Failed to read the TAP flags from
sysfs`. Its Landlock adapter maps `r` through `AccessFs::from_read(ABI)` and
adds file/directory rules through `path_beneath_rules`. This independently
matches the retained production diagnostic and the proposed read-only rule:

- [Cloud Hypervisor v53 `open_tap.rs`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/open_tap.rs#L42-L50)
- [Cloud Hypervisor v53 `landlock.rs`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/landlock.rs#L44-L67)
- [Cloud Hypervisor v53 path-rule application](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/landlock.rs#L90-L104)

### Qualified native-metal validation

The clean detached worktree `/tmp/overdrive-landlock.Al0wJR` was verified at
the exact review base with the qualified kernel and rootfs under
`/srv/vm/overdrive-testing/`. The target was sourced from the workspace
`.env`; it is not recorded in this artifact.

Fail-closed native preflight passed. Two focused runs exercised the real
in-process `serve` + `deploy` path, real TAP/netns provisioning, real Cloud
Hypervisor, guest READY, and the live VMM argv:

| Nextest run | Test | Observed result |
|---|---|---|
| `51d9c23e-82c9-4b74-82d9-9835d80d82f1` | `hypervisor_runs_bounded_nonroot_and_landlock_confined` | The VM reached `Running` in about 2.2 s. The test failed only at its stale one-rule oracle. Observed explicit rules were exactly selected TAP `/sys/class/net/ovd-tp-0000,access=r`, then the allocation run directory `access=rw`. Non-root and rlimit checks had already passed. |
| `98d3448b-fc06-48ab-88a2-35e7cff2d96e` | `vsock_landlock_grant_is_the_run_directory_scoped_to_nothing_else` | The VM reached `Running` in about 2.2 s. Directory-exclusivity checks passed; the test failed only because its stale oracle expected one rule instead of the same exact ordered two-rule sequence. |

These runs validate both the positive production dependency and the two stale
test oracles against a clean source identity. The supplied bounded negative
population removes only the TAP grant and reaches
`Failed / VmGuestExitUnreported` with Cloud Hypervisor's
`ReadSysfsTunFlags(EACCES)` diagnostic after the 90-second observation bound.
Together with the upstream call site above, that is sufficient reachability
and necessity evidence for this narrow DESIGN amendment.

## API, composition, and security analysis

### Exact API and feasibility

The selected implementation is mechanically feasible without new public
surface:

- `LandlockAccess { ReadOnly, ReadWrite }` and its
  `const fn as_str(self) -> &'static str` can remain private in the existing
  `overdrive_core::vm::config` module.
- `LandlockRule` can retain its existing public type, derives, and
  `to_rule_arg` / `path` methods while adding only the private access field.
- `VmRunDir::landlock_grant` can preserve its exact public signature and fixed
  run-directory producer while choosing private `ReadWrite`.
- `VmNetworkAttachment::tap_sysfs_landlock_grant(&self) -> LandlockRule` is
  callable by `VmConfig::landlock_rules` in the same module without export.
- `VmConfig::landlock_rules()` already returns `Vec<LandlockRule>` and can be
  the sole complete ordered composer: optional selected-TAP read rule first,
  mandatory run-directory write rule second.
- The host adapter's existing loop already accepts any returned rule count and
  preserves iteration order, so deleting
  `network_tap_sysfs_landlock_rule` requires no replacement adapter API.

No public constructor, trait method, enum variant, parameter, persisted field,
wire field, or lifecycle result is needed. The design correctly rejects a
second public method, general rule constructor, caller-controlled access, TAP
fd redesign, and a second security subsystem.

### One producer/composer authority

The selected authority is coherent: production TAP identity has one source;
there is one public fixed run-directory producer, one private fixed TAP
producer, one complete ordered composer, and one value renderer. The host
adapter owns only placement of `--landlock-rules` tokens. This removes the
review-base duplication where `overdrive-host` formats the TAP rule while
`overdrive-core` formats the run-directory rule.

`network: None` remains the exact legacy `Vec` containing only the run
directory. Although the current `serve` + `deploy` VM path supplies a complete
network channel before `VmDriver::start`, `None` remains an existing valid
`Vmm`/`VmConfig` boundary value and a host-adapter equivalence case; preserving
it does not invent a new production mode.

### Least privilege and path identity

The production value channel makes path injection unreachable: the action shim
does not accept an operator TAP string; it injects the slot-derived
`ovd-tp-<4 lowercase hex>` value after successful provision of that exact TAP.
The same `VmNetworkAttachment.tap` value drives both `--net tap=` and the
private Landlock producer. Direct test construction of the public transient
struct is not a production producer and is not grounds for a new validator or
newtype in this bounded remediation.

The chosen target is one selected sysfs class entry, not `/sys/class/net`, a
glob, a sibling interface, or a caller-provided alias. Linux exposes the class
entry as a kernel-managed symlink, but Cloud Hypervisor's read and its Landlock
`path_beneath_rules` application resolve the same selected entry; the clean
metal boot demonstrates that identity at the real boundary. The `r` mapping
does not grant write access. The independent allocation run directory remains
the only `rw` rule. No broader path is required or sanctioned.

### Public state and lifecycle

The amendment changes no `Vmm` port signature, `VmConfig` public field,
`VmNetworkAttachment` public field, `LandlockRule` public method, error type,
allocation state, retry behavior, cleanup ownership, TAP provisioning, guest
addressing, interception order, READY/EXEC boundary, persistence, or operator
surface. `Lifecycle Gate Ownership: Not applicable` is supported by the
production path: the rule is consumed inside the existing `Vmm::create` launch
before the unchanged boot race.

### Roadmap and DES disposition

No roadmap or DES-log amendment is required. The active roadmap's `01-02`
verification already runs the full `vm_stop_restart_and_vmm_death` native-metal
test file and the `overdrive-host` VMM equivalence suite. The amendment fixes an
SSOT/test-oracle mismatch in an already-required launch dependency; it adds no
acceptance outcome, step, gate, public API, or lifecycle mechanism. Necessary
`overdrive-core` implementation and pure-test fallout is permitted by the
repository's acceptance-criteria-over-file-list rule and does not expand the
roadmap contract.

## Findings and dispositions

### F-01 — The older exact `LandlockRule` shape remains operative-looking and contradicts the proposed private discriminator

**Severity:** High — blocking exact-design ambiguity.

**Evidence.** The proposed amendment pins `LandlockRule { path,
access: LandlockAccess }`, two access values, and a private TAP producer
(`ADR-0082:68-106`). The older 2026-08-17 amendment later in the same accepted
ADR still pins the mutually exclusive shape `LandlockRule { path }`, states
that `access=rw` is rendered unconditionally and is *not a field*, and says the
rule is built only by `VmRunDir::landlock_grant`
(`ADR-0082:334-395`). A second historical clause still says
`landlock_grant()` is the only producer and that no access parameter can be
wrong (`ADR-0082:1384-1391`).

The new text explicitly supersedes only the older unqualified phrase “sole
producer” (`ADR-0082:103-107`). It does not explicitly supersede the older
exact no-access-field representation or unconditional-`rw` renderer. A crafter
bound to the full ADR therefore has two incompatible exact private contracts,
even though the public signatures agree.

**Required disposition.** Make the 2026-09-11 amendment explicitly supersede
the 2026-08-17 `(a)`/`(b)` no-access-field, unconditional-`rw`, and
run-directory-only exact-shape clauses wherever they remain, while retaining
their still-valid run-directory necessity and exclusivity rationale. This is
documentation reconciliation only; it does not authorize a different API or
mechanism.

**Disposition:** Open.

### F-02 — Reuse Analysis omits mandatory contract-shape declarations for several touched components

**Severity:** High — blocking Effect Isolation review gate.

**Evidence.** The Reuse Analysis labels `LandlockRule::to_rule_arg` and
`VmConfig::landlock_rules` as pure-function / return-only, but does not declare
an exact contract-shape class for `VmRunDir::landlock_grant`, the private
`VmNetworkAttachment::tap_sysfs_landlock_grant`, or
`CloudHypervisorVmm::build_confined_command` (`ADR-0082:171-179`). These are
all touched components in this amendment. The reviewer contract requires every
component in Reuse Analysis to declare its effect shape, including unchanged
or deletion-only boundaries.

All three functions are described elsewhere as return-only composition with no
external effect; the gap is documentation, not implementation feasibility.
The out-of-scope TAP provisioning/lifecycle row may state that its existing
effect boundary is unchanged rather than re-designing it.

**Required disposition.** Add explicit contract-shape classifications to
every Reuse Analysis row touched by the amendment, including universe/delta
only if a row is not pure. Do not introduce a new capability, port, or effect.

**Disposition:** Open.

### F-03 — The test transition omits the repository-mandated per-test Contract Shape declarations

**Severity:** High — blocking test-contract handoff gap.

**Evidence.** The amendment explicitly transitions
`hypervisor_runs_bounded_nonroot_and_landlock_confined`, renames and changes
`vsock_landlock_grant_is_the_run_directory_scoped_to_nothing_else`, and adds
pure configuration coverage (`ADR-0082:142-155`). The two current native-metal
tests have no per-test `CONTRACT_SHAPE` declaration at their definitions
(`crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:1782-1804,
1926-1944`). The repository requires every new or transitioned test to carry
its declaration, and requires the exact rustdoc line
`/// CONTRACT_SHAPE: pure-function.` on every live source-local pure-function
property.

The proposed oracles themselves are implementable and remain honest: the
clean metal runs show both tests already reach their final argv assertion and
observe the exact two-rule order while preserving the earlier non-root,
rlimit, and directory-exclusivity checks. The gap is the missing effect-shape
handoff, not the selected assertions.

**Required disposition.** Classify each transitioned native-metal test and
the added pure configuration test/property in the DESIGN handoff, and require
the corresponding exact per-test declarations. Preserve the existing real
production entry point and cleanup universe; do not add a test-only API or a
new expectation runner.

**Disposition:** Open.

No other critical, high, medium, or low finding was established. In
particular, no reachable path requires a broader sysfs grant, a second TAP
identity, validation API, persistence, TAP-provisioning redesign, or lifecycle
change.

## Validation results

| Validation | Result |
|---|---|
| `git diff --check -- <three reviewed files>` | Exit 0; no whitespace errors. |
| Read-only source trace at base `dbb5dbb7` | Confirmed one real production TAP identity from `VmTapPlan` through `AllocationSpec` and `VmNetworkAttachment` into both `--net` and the Landlock grant. |
| Cloud Hypervisor v53 source check | Confirmed the exact `/sys/class/net/<tap>/tun_flags` read, typed `ReadSysfsTunFlags` failure, `r` access mapping, and `path_beneath_rules` application. |
| Qualified clean native-metal run `51d9c23e-82c9-4b74-82d9-9835d80d82f1` | Native preflight passed; VM reached `Running`; stale one-rule oracle failed against exact TAP-read then run-dir-write argv. |
| Qualified clean native-metal run `98d3448b-fc06-48ab-88a2-35e7cff2d96e` | Native preflight passed; VM reached `Running`; directory exclusivity passed; stale one-rule oracle failed against the same exact order. |
| Roadmap/DES review | No amendment needed; existing `01-02` verification owns both affected executable surfaces and the change adds no outcome or gate. |
| Production changes, test edits, roadmap/DES edits, commit/staging | Not performed by this reviewer. |

## Iteration history

### Iteration 1 — fresh independent review

The proven production dependency, exact selected mechanism, public-API
non-change, producer/composer ownership, least-privilege boundary, and two test
oracle transitions are sound. Three documentary gaps remain: the accepted ADR
still presents a contradictory exact private representation, several touched
Reuse Analysis rows omit required contract-shape classes, and the transitioned
tests lack the mandated per-test Contract Shape handoff.

## Final verdict

**CHANGES_REQUESTED.** Close F-01 through F-03 and return this same bounded
Landlock DESIGN remediation for independent re-review. Do not broaden the
grant, add public API, change TAP provisioning or lifecycle behavior, amend the
roadmap/DES log, or invent a second security mechanism.

---

## Iteration 2 — Remediation Re-review

### Iteration 2 metadata

| Field | Value |
|---|---|
| Review role | `nw-solution-architect-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning |
| Review date | 2026-09-11 |
| Iteration | 2 — F-01 through F-03 remediation re-review |
| Base | `dbb5dbb79fc7ba236c44075d8eb7214d8a38c32d` |
| Re-reviewed files | `docs/product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md`; `docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md`; `docs/product/architecture/brief.md` |
| Current verdict | **APPROVED** |

### Remediation examined

The re-review remained limited to the same authorized Landlock DESIGN
reconciliation. The actual documents contain none of the stray words from the
dispatch message. No production, test, roadmap, DES-log, staging, commit, or
branch change was reviewed as part of the remediation.

The selected contract remains unchanged from iteration 1:

- networked VM: exactly selected allocation TAP sysfs leaf `access=r`, then
  allocation run directory `access=rw`;
- non-networked `VmConfig`: exactly the legacy run-directory rule;
- unchanged public API;
- private `LandlockAccess { ReadOnly, ReadWrite }`;
- private
  `VmNetworkAttachment::tap_sysfs_landlock_grant(&self) -> LandlockRule`;
- existing `VmConfig::landlock_rules()` as sole complete ordered composer;
- no broader sysfs grant, public constructor, second identity, persistence,
  TAP redesign, lifecycle change, or second security subsystem.

### Finding dispositions

#### F-01 — Closed

ADR-0082 now explicitly supersedes the 2026-08-17 exact private
`LandlockRule { path }`, unconditional-`rw`, unqualified-sole-producer, and
run-directory-only cardinality clauses (`ADR-0082:20-31`). The retained
2026-08-17 section is rewritten as historical evidence and points exclusively
to the proposed 2026-09-11 discriminator and cardinality
(`ADR-0082:394-418`). D2.2 likewise labels the older Slice-01 deferral as
historical and states the current fixed run-directory producer in terms of
private `LandlockAccess::ReadWrite` (`ADR-0082:1391-1418`).

There is now one operative candidate representation: private
`LandlockAccess`, private access-bearing `LandlockRule`, one private selected
TAP producer, and one complete `VmConfig` composer. The search hits for the old
shape occur only inside clauses that explicitly identify and supersede that
history; they no longer compete with the implementation contract.

**Disposition:** Closed without API or mechanism change.

#### F-02 — Closed

Every touched Reuse Analysis row now carries an exact contract-shape
declaration and universe (`ADR-0082:217-225`):

- selected-TAP identity flow, `LandlockRule::to_rule_arg`, both rule producers,
  `VmConfig::landlock_rules`, and
  `CloudHypervisorVmm::build_confined_command` are explicitly
  `CONTRACT_SHAPE: pure-function.` over universe `∅`;
- the reused TAP/network/interception/lifecycle family is explicitly
  `CONTRACT_SHAPE: bounded-change.` over its unchanged one-allocation universe
  and records that this amendment adds no delta.

`build_confined_command` is correctly classified as return-only: it constructs
and returns `Command`; the existing `Vmm::create` owner performs the spawn.
No read port gained write capability and no capability-god-object was added.

**Disposition:** Closed without effect-boundary expansion.

#### F-03 — Closed

ADR-0082 now pins the final executable handoff (`ADR-0082:155-201`):

- `hypervisor_runs_bounded_nonroot_and_landlock_confined` retains its exact
  name and receives the exact standalone rustdoc declaration
  `/// CONTRACT_SHAPE: bounded-change.`;
- the stale vsock-only name is replaced exactly by
  `networked_vm_landlock_rules_are_exact_tap_read_then_run_dir_write`, also
  with `/// CONTRACT_SHAPE: bounded-change.`;
- the core property is named exactly
  `landlock_rules_are_exact_and_ordered_for_network_presence` and receives the
  required `/// CONTRACT_SHAPE: pure-function.` declaration.

The oracle ownership is non-overlapping. The first native-metal test owns the
live process's non-root/rlimit and complete explicit-rule argv contract. The
second owns run-directory exclusivity and precise path/access scope. The core
property independently owns composer representation, cardinality and order;
host tests own argv token placement. Both bounded tests name the existing
one-allocation universe, return it to absence through the existing production
stop path, and preserve operator-master and sibling-resource complements.
Those declarations strengthen verification honesty over effects the tests
already create and clean up; they add no production outcome or adjacent
hardening mechanism. The architecture brief carries the same names and exact
declarations (`brief.md:8619-8631`).

**Disposition:** Closed without test-only API, hand-created TAP, expectation
runner, or product-scope expansion.

### API, security, and scope revalidation

The exact public/private split remains implementable against the reviewed base.
`LandlockRule` retains its public type and existing `to_rule_arg` / `path`
surface; the only new representation is private. `VmRunDir::landlock_grant`
retains its public signature. The new TAP producer is private to
`overdrive_core::vm::config`. `VmConfig::landlock_rules` retains its signature
and deterministically owns the complete sequence. The host adapter deletes its
parallel formatter and only places returned values on argv.

Least privilege remains explicit and internally consistent. The real producer
channel is still slot-derived `VmTapPlan.tap -> AllocationSpec.guest_tap ->
VmNetworkAttachment.tap`; the same value drives `--net tap=` and the private
sysfs rule. The private producer accepts neither path nor access input. The
design rejects the `/sys/class/net` parent, sibling TAPs, aliases, alternate or
glob paths, a third rule, and TAP write access. Linux's kernel-managed sysfs
class symlink and Cloud Hypervisor v53's `path_beneath_rules` use resolve the
same selected entry, as confirmed by iteration 1's clean metal boot. No
production-reachable injection path requires a new validator or public
newtype.

The remediation introduces no state, error, persistence, cleanup ownership,
READY/Running/intercept-live/EXEC-release ordering, stop behavior, retry,
reclamation, TAP provisioning, guest addressing, wire, or operator-surface
change. ADR-0089 and the architecture brief continue to defer the exact
Landlock shape to ADR-0082 and are mutually consistent with it. The historical
record and proposed/pending status are clear. No roadmap or DES-log amendment
is needed.

### Qualified evidence revalidation

The remediation after iteration 1 changes documentation only, so the clean
base source identity and qualified artifacts used by the native-metal evidence
are unchanged. Runs `51d9c23e-82c9-4b74-82d9-9835d80d82f1` and
`98d3448b-fc06-48ab-88a2-35e7cff2d96e` remain directly probative: both real
VMs reached `Running`, both observed exactly the selected TAP read rule followed
by the allocation run-directory write rule, and both failed only at their stale
one-rule oracle. The bounded negative population remains corroborated by Cloud
Hypervisor v53's direct `/sys/class/net/<tap>/tun_flags` read and typed
`ReadSysfsTunFlags` permission failure. Re-running real guests after prose-only
remediation would not add a new source/runtime/input population.

### Iteration 2 validation results

| Validation | Result |
|---|---|
| `git diff --check -- <three reviewed design files>` | Exit 0; no whitespace errors. |
| Historical-contract scan | Old `LandlockRule { path }`, unconditional-`rw`, and sole-producer strings remain only in clauses explicitly marked historical/superseded. |
| Reuse Analysis scan | Seven touched rows; every row has exact `CONTRACT_SHAPE` classification and an explicit universe/delta disposition. |
| Executable-handoff scan | Both final native-metal test names, both exact bounded-change declarations, the exact core property name, and its exact pure-function declaration are present in ADR-0082 and summarized consistently in the brief. |
| API/security/source trace | One selected TAP identity, no caller path/access input, one complete composer, no public API addition, no broader grant, no new effect owner. |
| Qualified metal evidence | Iteration 1 clean-base runs remain current because iteration 2 remediation is documentation-only. |
| Roadmap/DES, production/tests, commit/staging | Unchanged and not edited by this reviewer. |

### Iteration 2 findings

No new critical, high, medium, or low finding was established. F-01, F-02,
and F-03 are closed.

### Final verdict after iteration 2

**APPROVED.** The bounded Landlock DESIGN remediation now has one exact current
representation, complete Effect Isolation declarations, executable test
handoff, least-privilege selected-TAP identity, unchanged public API, and no
scope or lifecycle expansion. It is ready to become implementation authority;
the pending-review labels may be transitioned by the owning architect/orchestrator.
