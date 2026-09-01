# Mutation Baseline TAP-Isolation Remediation Review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review scope | Recurrent mutation-baseline S-GTI-08a timeout remediation only |
| Reviewed commit | `87da1f17ea3e39f023380bc86d84d2da1ae55515` |
| Parent | `01cd03dfd75180ac690aa99ba0446259620acf2c` |
| Range | `01cd03dfd75180ac690aa99ba0446259620acf2c..87da1f17ea3e39f023380bc86d84d2da1ae55515` |
| Subject | `test(mutation): serialize recurrent VM baseline owner` |
| Required trailer | `Feature-Id: guest-stack-transparent-mtls-intercept` — present and exact |
| Review iteration | 1 |
| Verdict | **APPROVED** |

## Review boundary

This review is limited to the diagnosed collision between
`describe_round_trip::submit_then_describe_round_trips_spec_and_digest` and
S-GTI-08a, the nextest isolation change, and panic/error-safe teardown of the
resolver-failure fixture. It does not re-review the feature implementation or
require unrelated whole-workspace baseline remediation. No mutation testing
was run.

The range changes exactly two files: `.config/nextest.toml` (12 insertions, 7
deletions) and
`crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs` (159
insertions, 96 deletions). There is no production-code, public-API, dependency,
architecture, retry-policy, or mutation-exclusion change.

## Root-cause ownership and timing

The two failed unmutated baselines show the same overlap and symptom:

| Evidence | Long-lived owner | S-GTI-08a | Failure |
|---|---|---|---|
| Attempt 2 log | Started as test 801 at line 2024 and had not completed when S-GTI-08a started | Started as test 2306 at line 5022; timed out at 120.013s | `gti-resolver-sibling` reached `Failed` with `VmGuestExitUnreported { vmm_signal: Some(9) }` |
| Attempt 3 `baseline.log` | Started as test 801 at line 1996 and had not completed when S-GTI-08a started | Started as test 2306 at line 4994; timed out at 120.014s | The same sibling and `vmm_signal: Some(9)` result |

The code identifies the owner rather than merely correlating names:

- `submit_then_describe_round_trips_spec_and_digest` runs 256 property cases.
  Every case calls `spawn_server`, which creates fresh empty data/config
  directories and invokes production `run_server` with `RealCgroupFs`.
- Production `run_server` unconditionally awaits
  `vm_reclamation_boot::converge` before restart adoption. That drive hydrates
  desired allocations from the server's own empty store, observes node-global
  VM run-dir/cgroup state, and executes reclamation actions, including
  `kill_scope`, for allocations absent from its desired/supervision view.
- The proptest therefore repeatedly booted an empty owner while S-GTI-08a held
  the live `gti-resolver-sibling` Cloud Hypervisor. The observed signal 9 is
  consistent with that production reclamation path and the exact two-test
  pairing reproduced the failure before this remediation.

The Cloud Hypervisor diagnostic `Tap ovd-tp-0000 already exists. IP
configuration will not be overwritten.` is not the owner. ADR-0089 section 4
and the accepted feature delta explicitly define attaching the pre-created
persistent TAP by name, with that warning as the expected benign handoff. The
warning was captured in both failures because it is emitted during the normal
boot path; the terminal signal-9 row is the causal evidence.

## Configuration review

The nextest change is the smallest cross-process isolation boundary for the
recurrent owner:

- It adds only the exact package/binary/test filter for
  `submit_then_describe_round_trips_spec_and_digest` to the existing
  `host-kernel-shared` group.
- The group already has `max-threads = 1` and already contains the whole
  `guest_stack_mtls_egress` module, so the two processes cannot overlap.
- `cargo nextest show-config test-groups --profile mutants` resolved both the
  exact proptest override and S-GTI-08a's module override into
  `host-kernel-shared`. This independently verifies that the `mutants` profile
  inherits the default-profile assignments used by the mutation baseline.
- The pre-existing 240-second cold-scratch timeout for the 256-case proptest is
  retained. Retry count remains zero. No sleep, retry, random backoff, broader
  module match, unrelated serialization, profile override, or mutation
  exclusion was added.

The group is semantically appropriate: its established purpose is to
serialize processes that observe or mutate node-global VM/kernel state.
Assigning the one repeatedly booting production server uses that existing
ownership boundary without creating a second lock domain or changing
production recovery behavior.

## Fixture-cleanup and test-integrity review

The resolver-failure fixture now puts its live-owner observation under
`catch_live_owner_observation`, then always invokes
`finish_after_authoritative_cleanup`. The resulting ownership order is exact:

1. Cleanup is armed before the sibling deploy request can persist intent.
2. The observation future owns the VMM cut receivers; an unwind drops those
   receivers before teardown so a blocked VMM decorator is released.
3. Through the still-live control plane, cleanup stops only the fixed
   `gti-resolver-sibling` identity and accepts only `Stopped` or the idempotent
   `AlreadyStopped` outcome.
4. It waits for the sibling terminal row, parses that row's exact allocation
   ID, and rejects any still-live PID in that allocation's production cgroup.
5. Server shutdown runs independently even if sibling cleanup returns an error
   or panics; observation, sibling cleanup, and server cleanup are reported
   together and any non-success fails the test.

Moving the helper inputs from `&TempDir` to `&Path` is mechanical and permits
the server-root path to move into the unwind-caught observation future without
transferring the tempdir owner. All call sites retain the same paths.

The original behavioral evidence remains intact inside the protected
observation: exact typed pre-READY failure and errno detail, durable row,
target VMM/cgroup/clone/index/run-dir/netns/TAP/veth/route/nft cleanup, guest
beacon/frame boundary, complete packet-path baseline equality, target
`Stopped` then `AlreadyStopped`, and byte-exact sibling row and rule
preservation. The S-GTI-08a Outcome anchor and exact
`CONTRACT_SHAPE: bounded-change.` declaration are unchanged. The shared closure
also makes its cleanup-focused and terminal-stop-idempotence callers safer
without weakening their contracts. No assertion was removed or relaxed.

## Verification

All runtime commands used the repository's native metal runner with
`OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel` and
`OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4`.

| Command | Result |
|---|---|
| `git rev-parse 87da1f17^`; range `diff --name-status`, `--numstat`, and `--check` | PASS — exact supplied parent, only the two scoped files, no whitespace errors |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask metal run -- cargo nextest show-config test-groups --profile mutants -p overdrive-control-plane -p overdrive-cli --features integration-tests,kvm-tests --groups host-kernel-shared --no-pager` | PASS — group has `max threads = 1`; exact proptest and S-GTI module resolve into it |
| Focused two-test metal run under `--profile mutants`, repeated three times | PASS — 2/2 each time, zero retries; 65.358s, 65.951s, and 65.399s |
| Same focused two-test metal run with pass timing enabled | PASS — S-GTI-08a 17.415s, proptest 48.395s, 2/2 in 65.820s, proving sequential execution |
| Focused S-GTI-08a-only metal run, repeated twice | PASS — 1/1 each time, zero retries; 17.418s and 17.425s |

A preliminary metal invocation without the required kernel/rootfs environment
failed closed during native preflight before running tests and changed no
state. The correctly parameterized authoritative runs above all passed.

## Findings

No defects found. The root-cause attribution follows the production boot and
reclamation ownership path and matches two recurrent baseline timelines; the
TAP warning is an accepted handoff diagnostic, not a collision cause. The
configuration uses the existing minimal single-writer domain and is selected
by the mutation profile. The fixture teardown is fail-loud, sibling-specific,
panic/error safe, and verifies process quiescence while preserving all
contract, outcome, and exact-state assertions.

## Verdict

**APPROVED.** Commit `87da1f17ea3e39f023380bc86d84d2da1ae55515`
remediates the bounded recurrent mutation-baseline collision without masking,
unrelated serialization, weakened evidence, or production redesign.
