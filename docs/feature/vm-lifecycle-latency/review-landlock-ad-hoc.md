# Ad Hoc Landlock Fix Review

## Metadata

| Field | Value |
|---|---|
| Review role | Fresh isolated `nw-software-crafter-reviewer` |
| Review date | 2026-09-11 |
| Reviewed commit | `f60495a32b7fafdebacc06d0336d73132734821d` |
| Required parent / approved design base | `624e02745085a4990bdd8115d5b4304759a01f91` |
| Worktree | Detached review worktree `/tmp/overdrive-landlock.Al0wJR` |
| Iteration | 1 |
| Verdict | **APPROVED** |

## Authorized ad hoc scope

This review covers only the committed reconciliation that moves the already
necessary selected-allocation-TAP sysfs read grant into the existing
`VmConfig::landlock_rules()` composer, transitions the two named native-metal
oracles, and adds the named source-local core property. The implementation is
reviewed against the approved iteration-2 Landlock DESIGN review in
`review-design-remediation-landlock.md` and its amended ADR-0082, ADR-0089,
and architecture-brief contract.

The authorized result is deliberately narrow:

- private `LandlockAccess::{ReadOnly, ReadWrite}` only;
- private
  `VmNetworkAttachment::tap_sysfs_landlock_grant(&self) -> LandlockRule`;
- unchanged public `LandlockRule`, `VmRunDir`, `VmConfig`, `Vmm`, error,
  lifecycle, wire, and persistence surfaces;
- networked explicit rules exactly selected TAP sysfs leaf `access=r`, then
  allocation run directory `access=rw`;
- non-networked explicit rules exactly the allocation run directory
  `access=rw`;
- no adapter-local TAP-rule formatter or duplicate rule authority;
- no parent `/sys/class/net` grant, sibling TAP, glob, alternate path, caller
  access choice, TAP write grant, third rule, or adjacent hardening mechanism.

No production, test, design, roadmap, execution-log, or commit content was
edited during review. This Markdown artifact is the reviewer's only write and
is intentionally uncommitted.

## Design, API, and security analysis

### Exact contract shape

The implementation matches the approved private shape exactly.
`LandlockAccess` is a private two-variant enum and its private renderer maps
only `ReadOnly -> "r"` and `ReadWrite -> "rw"`
(`crates/overdrive-core/src/vm/config.rs:537-551`). `LandlockRule` remains
public with private fields and its existing public `to_rule_arg` and `path`
methods; the change adds only the private access discriminator
(`config.rs:553-583`). No new public function, method, constructor, field,
type, trait, enum variant, or parameter appears in the production diff.

The fixed producers are the only two struct-literal construction sites:

- `VmRunDir::landlock_grant` preserves its public signature and constructs
  only the run-directory `ReadWrite` rule (`config.rs:867-878`);
- private `VmNetworkAttachment::tap_sysfs_landlock_grant(&self) ->
  LandlockRule` constructs only `/sys/class/net/<self.tap>` with `ReadOnly`
  (`config.rs:899-905`).

`VmConfig::landlock_rules()` retains its public signature and is the sole
complete ordered composer. It reserves exactly the required capacity, adds
the private TAP grant only for `network: Some`, then always appends the
run-directory grant (`config.rs:957-975`). This yields cardinality two in the
networked case and one in the non-networked case, in the required order.

### One authority and least privilege

A repository-wide production-source scan found exactly the two sanctioned
`LandlockRule` producer literals above. The former adapter-local
`network_tap_sysfs_landlock_rule` function is absent. The only production
`--landlock-rules` placement remains the host adapter loop over
`config.landlock_rules()` (`crates/overdrive-host/src/vmm.rs:238-283`). The
host adapter therefore owns argv placement only; core owns every explicit
rule value, access mode, cardinality, and order.

The TAP producer accepts neither a path nor an access parameter. Its only
input is the existing `VmNetworkAttachment.tap`, also rendered into `--net`.
The output is the one selected leaf with read-only access. Exact-vector tests
reject the broader `/sys/class/net` parent, an additional or sibling TAP, an
alternate/glob path, TAP write access, a third rule, or an order change. The
allocation run directory remains the only explicit read-write rule. This is
the approved least-privilege split; no new validation API or security
subsystem is introduced.

### Public state, errors, and persistence

The commit changes three files only: core transient configuration, the host
argv adapter, and the native-metal test file. It changes no persisted or wire
type, no trait signature, no allocation state or transition reason, no error
variant or mapping, no cleanup owner, and no lifecycle gate. `VmConfig` and
`VmNetworkAttachment` remain transient values. Adding a private field to the
already non-constructible-outside-core `LandlockRule` does not add public API
or persistence layout.

## Production and reproducer evidence

### Real caller and owner path

The selected identity is production-derived, not test-authored:

1. `provision_and_inject_netns` assigns the production slot, derives one
   `VmTapPlan`, provisions it, and passes that same plan into
   `inject_workload_network`
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1184-1218`).
2. `inject_workload_network` copies `VmTapPlan.tap` into
   `AllocationSpec.guest_tap` before `Driver::start`
   (`action_shim/mod.rs:1221-1238`). Both fresh-start and same-ID restart
   production arms execute this C3 seam before the driver
   (`action_shim/mod.rs:1824-1849,2306-2326`).
3. `compose_vm_network` transfers `AllocationSpec.guest_tap` without a second
   derivation into `VmNetworkAttachment.tap`
   (`crates/overdrive-worker/src/vm_driver.rs:123-195`). The resulting
   attachment is placed on the complete `VmConfig`, then the existing
   `Vmm::create(&config)` port is awaited
   (`vm_driver.rs:1296-1344`).
4. `CloudHypervisorVmm::create` prepares the already-approved confined paths,
   builds the command, and spawns it. `build_confined_command` uses the same
   attachment for `--net` and consumes only `VmConfig::landlock_rules()` for
   explicit Landlock values
   (`crates/overdrive-host/src/vmm.rs:238-283,400-460`).

This path is the real in-process `serve -> deploy -> C3 provision ->
VmDriver -> Vmm::create -> Cloud Hypervisor` path. No test hand-creates the
TAP used by the native assertions.

### Test integrity and Contract Shape

The two transitioned native tests contain the exact standalone declaration
`/// CONTRACT_SHAPE: bounded-change.` and the core property contains the exact
standalone declaration `/// CONTRACT_SHAPE: pure-function.`. None is skipped,
renamed to a banned implementation-result pattern, assertion-free,
tautological, mock-dominated, or fully mocked.

The native tests retain their production composition and strengthen the stale
one-rule oracle to exact equality over the complete explicit-rule vector:

- `hypervisor_runs_bounded_nonroot_and_landlock_confined` first proves the
  live Cloud Hypervisor has nonzero real/effective uid and gid and both named
  rlimits below the `serve` process, then asserts the exact two-rule sequence
  (`crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs:1796-1875`);
- `networked_vm_landlock_rules_are_exact_tap_read_then_run_dir_write` first
  checks run-directory exclusivity, then asserts the same exact sequence and
  no third rule (`vm_stop_restart_and_vmm_death.rs:1947-2027`).

Both obtain the expected TAP identity by parsing the live hypervisor's
production `--net` value (`vm_stop_restart_and_vmm_death.rs:1732-1745`). They
do not duplicate `VmTapPlan`'s slot/name derivation. Exact equality over every
explicit `--landlock-rules` value is the closed-world oracle: an unexpected
parent, sibling, alternate, write, ordering, or cardinality value changes the
vector and fails the assertion. The existing production stop path is awaited
after each successful oracle; `poll_until_terminated` requires the durable
terminal result before server shutdown.

The source-local property
`landlock_rules_are_exact_and_ordered_for_network_presence` ranges over both
network-presence states and generated valid four-lowercase-hex TAP suffixes.
It independently asserts the complete paths vector and complete rendered
values vector, including count and order
(`crates/overdrive-core/src/vm/config.rs:1234-1283`). Its universe is empty:
it performs no filesystem, process, network, TAP, clock, or persistence I/O.
It would fail if the network branch were removed, either access mode changed,
the order changed, or any rule were added. The property repeats only the
approved output specification; it does not reimplement the production
composition loop.

The two distinct contract behaviours are networked and non-networked rule-set
composition. The maximum unit-test budget is four. The relevant unit-level
evidence is the new core property plus the pre-existing host command-value
test, so the budget passes. The two native-metal tests are independent Tier-3
production-composition evidence, not duplicate unit cases.

The test edits are requirement-authorized by parent commit `624e0274` and do
not weaken, delete, skip, or reduce the old assertions: the stale exact
`[run-dir rw]` expectation becomes the approved exact
`[selected-TAP r, run-dir rw]` expectation. The new core property would fail
against the parent's core composer for every generated network-present case,
because the parent returned only the run-directory rule; production code,
not a fixture, supplies the GREEN transition. No DES phase event applies to
this explicitly authorized ad hoc reconciliation, and the approved design
states that no roadmap or execution-log amendment is required.

## Independent verification

The metal target was sourced from the primary workspace's gitignored `.env`
and is not recorded here. The qualified guest artifacts were
`/srv/vm/overdrive-testing/kernel` and
`/srv/vm/overdrive-testing/rootfs.ext4`. Every metal invocation passed the
runner's fail-closed native x86_64, non-virtualized, KVM and source-identity
preflight.

| Verification | Result |
|---|---|
| Commit identity and parent | `HEAD` is `f60495a3`; sole parent is the required `624e0274`. |
| Scope | Exactly the three authorized files changed; 183 insertions and 93 deletions. |
| `cargo fmt --all --check` | PASS. |
| `git diff --check 624e0274 f60495a3` | PASS. |
| `cargo xtask dst-lint` | PASS. |
| Core + host focused run | PASS: 2 tests; nextest run `d5781afa-56b6-42d5-8f79-98feb1fd736b`. |
| Core property with `PROPTEST_CASES=1024` | PASS: 1 property; nextest run `13985676-f6bb-41dd-9ada-f87f8faaba46`. |
| Qualified native-metal Landlock run | PASS: both named tests; nextest run `82392141-3391-4127-a6cb-dde8e968e8b0`; both real VMs reached `Running`, so the selected leaf grant is functional under real Cloud Hypervisor while the non-root and exact-rule assertions hold. |
| `cargo check -p overdrive-core --all-targets` | PASS on qualified metal. |
| `cargo check -p overdrive-host --all-targets --features integration-tests` | PASS on qualified metal. |
| `cargo check -p overdrive-cli --all-targets --features integration-tests,kvm-tests` | PASS on qualified metal. |
| Clippy for the same three package/feature surfaces with `-D warnings` | PASS on qualified metal. |
| Post-verification worktree state | Clean before this review artifact was added; no production/test/design diff was introduced. |

## Commit attribution

The Git author and committer remain `Marcus Schack Abildskov
<work@marcus-sa.dev>`, matching the parent. The commit message contains
exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer. It contains no
Claude Code, Claude, Anthropic, generated-by, or other forbidden attribution.

## Findings and dispositions

No critical, high, medium, or low defect was established. In particular, no
reachable production path, executable test, API diff, or repository-wide
authority scan supports a claim that the implementation grants a broader
sysfs target, grants TAP write access, constructs another TAP identity,
retains duplicate host authority, changes public state/errors/persistence, or
depends on a test-only composition.

| Gate | Result |
|---|---|
| Exact approved API shape | PASS |
| Least privilege / one authority | PASS |
| Selected-TAP production identity | PASS |
| Contract Shape declarations | PASS |
| Closed-world and non-vacuous oracles | PASS |
| Test integrity / no testing theater | PASS |
| External validity / real production composition | PASS |
| Unit-test budget | PASS |
| RPP scan | PASS through L6; no in-scope smell requiring a finding |
| Compile, lint, format, and focused runtime evidence | PASS |
| Commit scope and attribution | PASS |

## Final verdict

**APPROVED.** Commit `f60495a32b7fafdebacc06d0336d73132734821d`
implements the authorized Landlock reconciliation without public API,
lifecycle, persistence, error, or security-boundary expansion. It preserves a
functional non-root Cloud Hypervisor launch and gives the selected allocation
TAP only the exact read access Cloud Hypervisor requires, with the allocation
run directory remaining the sole read-write explicit grant.
