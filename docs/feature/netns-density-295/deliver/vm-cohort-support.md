# VM cohort GuestKit support handoff

## Status and scope

This package prepares the existing disposable GuestKit boundary for the two
already-authored 64-VM correctness bodies. It contains test-support source and
one bounded correction to the existing round-trip body's address oracle. It
does not add a scenario, production change, public API, test seam, model, lane,
benchmark, DES phase or commit.

The approved source parent is
`a2cd9691de4da949389cfe884a044ca9cc60367d`. The delivery crafter's two pending
attribute removals and DES log events, the user-owned `AGENTS.md`, the approved
runner support, and the independent review artifact are preserved.

## Exact existing identities

Both bodies remain in the existing Cargo test target
`overdrive-cli::integration` and retain `CONTRACT_SHAPE: bounded-change`.
`guest-kit-prepare` must freshly build and stage that target after review;
there is currently no staged ELF containing the corrected source.

| Exact libtest identity | Existing outcome |
| --- | --- |
| `integration::vsock_density::every_vm_at_density_round_trips_through_its_own_attachment` | Each of 64 guest applications completes its own TCP and UDP round trip exactly once, the remote peer sees the node source address, and all workload addresses are unique. |
| `integration::vsock_density::serve_down_at_density_is_fail_closed_and_the_next_boot_reclaims_every_vm` | Every guest-prefix dial is refused immediately and reaches no live host listener with `serve` up or down; the next boot ends every unsupervised VMM and places every persisted Job on the reclaimed CID set. |

Both identities, tags, `ATTACHMENTS = 64`, workload command, resource
declarations, terminal-state checks, exact TCP/UDP counts, node-source oracle,
fail-closed dial oracle and restart/cleanup assertions remain unchanged. The
only body correction moves the first identity's address cardinality assertion
from terminal rows to the same public rows while each allocation is Running.

## Native evidence and terminal-row contract correction

The initially reviewed runner support was sufficient and the recovery body is
GREEN in durable receipt `23dcb4e230df8d1fe0dd182e`: one selected pass, zero
ignored/runtime-SKIP bodies, zero canonical-119 credit and clean outer-guest
shutdown.

The round-trip body remains RED in receipts
`88a2a3893cd926d4078cceef` and `4c191dd7f199e4fce463edef` because different
workloads reached `WorkloadCrashedImmediately`. That is a production outcome
for the delivery crafter; this DISTILL correction does not classify, change or
claim to fix it.

Diagnostic receipt `8f71d1cfe5460b053d0902ad` temporarily enlarged a private
production drain budget and then reached the address assertion with only 62 of
64 terminal rows carrying `workload_addr`. The temporary production change was
reverted; the current worktree contains no production diff. The diagnostic is
useful only because it exposes an independent oracle defect.

ADR-0098 records the accepted lifecycle: a Running row carries the allocation's
observed `workload_addr`, while a genuine terminal row deliberately removes it
because a dead allocation is not a live backend. The implementation does that
explicitly in both terminal authoring paths: `FinalizeFailed` supplies the
prior address only for the non-terminal Stable result and otherwise supplies
`None` (`action_shim/mod.rs:2089-2093`); `StopAllocation` supplies `None` to the
Terminated row (`action_shim/mod.rs:3331-3336`). A correctness oracle requiring
64 addresses from terminal rows therefore conflicts with the accepted
contract, regardless of the separate workload crashes.

The existing round-trip body now starts one private observer for each deployed
Job. Each observer drives the same public `describe` handler and returns only
from a `Running` row with a present workload address. Reaching `Terminated` or
`Failed` first is an immediate failure that prints the exact row, so a crashed
workload cannot be converted into missing-address success. The body requires
exactly 64 observed Job ids and 64 distinct live addresses before its existing
terminal and peer-traffic assertions. `JoinSet` owns the observers, so a body
failure aborts any unfinished observer with the test task.

No terminal value is reconstructed, persisted or recovered; no address is read
from terminal history. No guest behavior, production timing, address owner,
public response shape or lifecycle rule changes.

## Existing staging and execution boundary

`cargo xtask guest-kit-prepare` already declares
`overdrive-cli --test integration --features integration-tests,kvm-tests` in
`IN_GUEST_LANES`. It asks Cargo for JSON compiler-artifact output and copies
the actual reported ELF to the existing
`target/guest-kit/in-guest/overdrive-cli--integration` contract path.
`InGuestTests::prepared` reads that exact file and fails with
`LaneNotPrepared` when it is absent. No staging file, source copy, generated
wrapper or build script is added.

The existing `cgroup-default` runner remains the only executor:

1. `OVERDRIVE_FWD_GUEST_EXACT` resolves one declared exact identity.
2. `run_nested_vm_test_binary_with_resources` creates a fresh member of the
   existing `cgroup-default` disposable-guest group.
3. `GuestKit::boot_scripted_with` launches the outer guest through production
   `CloudHypervisorVmm::create` and its production `VmConfig` renderer.
4. The outer guest retains uid/gid 4200, the actual KVM and vhost-vsock device
   groups, Landlock, seccomp and the existing confined run directories.
5. The outer guest mounts its private cgroup v2, bpffs, securityfs and XFS
   staging volume, loads its own module copies, and runs the prepared test ELF
   there. The test body launches its 64 nested workload VMs through the
   production `serve` composition.

The physical Metal host supplies transport and KVM to the outer production
VMM. It is never the test kernel and receives no nextest invocation, module
load, cgroup delegation change, route, namespace, nft, BPF or process cleanup
from this support.

## Static resource configuration

The existing test source declares each of the 64 workloads with
`cpu_milli = 500` and `memory_bytes = 128 * 1024 * 1024`. Production derives
one vCPU for each 500-millicpu workload. The outer guest retains the existing
eight-vCPU nested-execution configuration; CPU overcommit affects elapsed
time, which this correctness suite does not measure or assert.

The previous 4,096 MiB outer guest cannot contain 8,192 MiB of declared inner
guest RAM. The focused cohort uses the existing `ScriptedBootExtra.memory_mib`
configuration with a compile-time-checked 12,821 MiB envelope:

| Component | Static amount |
| --- | ---: |
| One inner guest's declared RAM | 134,217,728 bytes (128 MiB) |
| Production `MemoryPlan` reserve for that guest | 8,724,152 bytes |
| One inner VMM cgroup ceiling | 142,941,880 bytes |
| 64 inner VMM ceilings | 9,148,280,320 bytes (8,724.48 MiB) |
| Existing outer node/kernel/userspace envelope | 4,294,967,296 bytes (4,096 MiB) |
| Required total | 13,443,247,616 bytes (12,820.48 MiB) |
| Configured outer guest | 12,821 MiB |

The runner derives the inner ceiling from production
`MemoryPlan::derive(128 MiB).cgroup_max_bytes()`. Two const assertions require
12,821 MiB to cover the sum and 12,820 MiB to be insufficient. A future
production reserve-policy change therefore breaks compilation instead of
silently under-sizing the fixture. The 4,096 MiB outer component is the
runner's established standard guest envelope, retained for the outer kernel,
production `serve`, the integration process, 64 Cloud Hypervisor control
processes and staged tools. This is capacity arithmetic only; it collects no
memory, latency, throughput, percentile or density measurement and grants no
benchmark credit.

The focused cell supervision bound is 1,200 seconds, matching the existing
twenty 60-second periods declared for `vsock_density`. It is a runner cleanup
bound outside the product assertions. Both bodies retain their original
internal bounds and immediate-refusal oracle.

## Selection, skip rejection and accounting

The source table contains exactly the two identities above and requires two
total source-manifest bodies. Each invocation passes only one exact identity
to libtest and requires:

- process exit zero;
- the exact `1 passed; 0 failed;` summary;
- no recognized runtime-SKIP diagnostic; and
- one selected body, so a still-ignored marker produces zero passes and is
  rejected rather than credited.

The focused result label is `VM-COHORT-FOCUSED-RESULT`. It reports the exact
identity, one pass, the two-body source-manifest count, the static outer guest
memory and vCPU configuration, and `canonical_119_credit=0`.

The direct-selector branch returns before the canonical manifests execute.
The existing `cgroup-default` count remains 119 and `mtls-relay` remains 46.
No focused run can claim either canonical manifest's credit or change its
coverage.

## Cleanup and failure containment

On success, both test bodies stop every Job and shut down `serve`.
The runner prints the complete outer-guest console, requests guest poweroff
and requires the outer VMM to exit within 30 seconds. Its work directories
and staged support directories are RAII-owned.

On a test error, timeout or panic, unwinding drops `GuestRun` and
`GuestKitVm`; production `CloudHypervisorVmm::terminate` ends the outer VMM,
which contains every nested VMM and every kernel/network/cgroup effect. No
physical-host process kill or kernel/network cleanup fallback exists. A
native receipt is incomplete unless it records terminal outer-guest cleanup.

## Reviewed execution contract

The initial runner package has an independent approval in
`review-vm-cohort-support.md`. The live-address oracle correction changes that
reviewed body evidence, so the same independent support reviewer must append a
new approval before the round-trip identity is executed again. After approval,
the delivery crafter must rebuild the exact Cargo test target, stage the
resulting corrected ELF through the existing `guest-kit-prepare` producer,
prove the Cargo-reported source executable and staged contract-path copy are
byte-identical, and only then use the existing group/direct selector. The
complete future durable Metal command is:

```bash
cargo xtask metal run -- bash -c '
set -eu
export CARGO_TARGET_DIR=target/vm-cohort-native
sha256sum \
  crates/overdrive-cli/tests/integration/vsock_density.rs \
  crates/overdrive-testing/bin/fwd_guest_suite.rs

source_elf="$(
  cargo test --no-run --package overdrive-cli --test integration \
    --features integration-tests,kvm-tests --message-format=json |
  jq -sr '\''
    [ .[]
      | select(
          .reason == "compiler-artifact"
          and .profile.test == true
          and .target.name == "integration"
          and .executable != null
        )
      | .executable
    ]
    | unique
    | if length == 1
      then .[0]
      else error("expected exactly one Cargo-reported overdrive-cli integration test executable")
      end
  '\''
)"

cargo xtask guest-kit-prepare
staged_elf="$CARGO_TARGET_DIR/guest-kit/in-guest/overdrive-cli--integration"
test -x "$source_elf"
test -x "$staged_elf"
cmp -s "$source_elf" "$staged_elf"
printf "CARGO_REPORTED_SOURCE_ELF=%s\nSTAGED_ELF=%s\n" "$source_elf" "$staged_elf"
stat -c "ELF_STAT path=%n size=%s mode=%a" "$source_elf" "$staged_elf"
sha256sum "$source_elf" "$staged_elf"

OVERDRIVE_FWD_GUEST_GROUP=cgroup-default \
OVERDRIVE_FWD_GUEST_EXACT=integration::vsock_density::every_vm_at_density_round_trips_through_its_own_attachment \
cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite \
  --features integration-tests
'
```

The `cargo test --no-run` selector is the same package, test target and feature
set declared by `IN_GUEST_LANES`. `jq` rejects zero or multiple matching Cargo
executables. `guest-kit-prepare` remains the sole staging producer; `cmp`
proves its contract-path copy is exactly the Cargo-reported executable, and
both paths and SHA-256 values are printed before the corrected body can start.
Any failure before the exact selector prevents native execution.

Each execution must retain the opaque Metal run id, collect that same run
after a route loss, and preserve complete output without `head` or `tail`.
The receipt must identify the source commit and dirty diff, Cargo-reported
source ELF path and SHA-256, exact selector, static resource envelope, process
exit, selected/passed/failed/ignored/runtime-SKIP counts, full console and
outer-guest cleanup. A busy lease is ordinary serialization. Physical-host
nextest, a kernel fallback, Lima, mutation testing, KASAN, delegation changes,
force unload, reboot and host cleanup remain forbidden.

The two existing `WorkloadCrashedImmediately` receipts remain the current
semantic RED evidence. Missing staging, compilation failure, ignored-marker
vacuity, resource refusal, observer failure or another fixture failure is
infrastructure/test evidence and must never replace that product RED.

## Scoped source gates

This DISTILL task may run only unprivileged source checks on Metal, with a
task-owned Cargo target directory:

```text
cargo fmt --all -- --check
cargo check -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
cargo check -p overdrive-cli --test integration --features integration-tests,kvm-tests
cargo clippy -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests -- -D warnings
cargo clippy -p overdrive-cli --test integration --features integration-tests,kvm-tests -- -D warnings
```

These commands compile source only. They do not prepare GuestKit artifacts,
run a test binary, start a VMM, load a module or mutate a kernel. Their durable
run id, exact exit and source digest are appended here after collection.

## Source-gate result

Durable unprivileged Metal run `236628f7df0df257b41c8a0e` completed with
`exit_code=0`, `cause=command_returned`. The same run was collected once with
`cargo xtask metal collect 236628f7df0df257b41c8a0e`; collection also exited
zero and returned the complete retained output.

All five commands above passed against the task-owned
`target/distill-vm-cohort-support` Cargo target directory. Source identities
at the gate were:

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | `d8ba2e26bb71b9e1ba7483940026154f56b4acb3818bd7ba037c0a597bd4f190` |
| `crates/overdrive-cli/tests/integration/vsock_density.rs` | `85817a7d6abdf7b43abae8d44eed73c895e29f383817a854600923976979be37` |

The test-file diff from the approved parent is exactly the two pending
attribute removals plus the bounded live-address observer and replacement of
the terminal-address cardinality read. The existing identity, stimulus,
terminal-state checks, traffic oracles and cleanup are unchanged. No production
file differs from the approved parent.

This correction task did not run `guest-kit-prepare`, so it intentionally
produced no corrected execution ELF and claims no new ELF identity. Every prior
native receipt used pre-correction test source
`f06869d85ff3169609c84b948734549544f5c9b0abee7c480ec8a245c5d4f680`;
for example, the diagnostic staged ELF was
`6e80b8ea42a56d0bf2639740c40c3895e9a2fef11358fda022a5866306b92353`
and the recovery-pass staged ELF was
`816ca3794c4c03d45c4f1ac187c178b599d6b40a83b7e8b9c35fb59d8924b227`.
Those artifacts predate the corrected test source and must not be reused. The
later reviewed preparation must record Cargo's actual reported source ELF and
the SHA-256 of its staged contract-path copy before native execution.

No test, VMM, GuestKit, module, BPF program, cgroup, namespace, route, nft
table or kernel job ran in this source gate.
