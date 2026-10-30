# STEP 14-01 — existing GuestKit native-support handoff

## Current disposition — R5-41 workload-kind fixture correction

Iteration-4 finding F-02 is closed by one private DISTILL fixture correction.
R5-41's existing long-running `hold` workload is now submitted as a Service,
whose accepted lifecycle places a successor after the first launch is killed,
instead of as a terminal Job. The private `deploy_hold_service` helper preserves
the existing workload id, replica count, resources, VM command, exact `hold`
arguments, kernel, rootfs and remote peer. It declares the existing
non-reserved TCP `SERVICE_PORT` listener required for Service admission and
leaves startup, readiness and liveness probes empty. The guest still opens only
the same outbound held flow observed by R5-41.

The R5-41 body changes only its private submission-helper call. Its real VMM
kill and every assertion remain byte-identical: distinct successor allocation,
new launch token, new session on CID reuse, no dead-launch unread connection,
exactly one live pair, current forwarder generation and empty owner state after
stop. The other four control-plane bodies retain their original Job/Service
fixtures. Production Job no-restart and Service restart behavior are unchanged;
no public API, scenario, lane, selector, probe outcome or production mechanism
was added.

All final receipts have source digest
`d3fe8c43abf2447f87621f8d6b96ea863525d95c27f7e5784190ee36b7fbe6ce`:

- scoped Metal source gate `41dbac5813ccba60893161ad` exited 0 after
  `cargo fmt --all -- --check`, the kvm-gated control-plane integration
  `cargo check`, and Clippy with `-D warnings`;
- existing-runner Metal execution `410a62758dd8e62f9f9d6418` exited 0 with
  107 successful summaries declaring exactly 117 passed, 0 failed and
  0 ignored bodies: the retained 111, CLI walking skeleton and all five
  control-plane cells;
- the exact R5-41 identity passed once in 5.60 seconds, after its predecessor
  cell and before the remaining three control-plane cells; recognized runtime
  SKIP diagnostics were zero; and
- read-only physical-host audit `be0ea50bafad180437fa1117` exited 0 with no
  live Cloud Hypervisor process, physical-host nft table or STEP 14-01 workdir.

The complete durable records are retained under
`.context/netns-density-295/14-01/raw/<run-id>/`. No DES phase, commit or
mutation run was created. The same independent DISTILL reviewer approved the
correction in iteration 5 with F-02 closed and no new findings. The complete
117-body acceptance boundary is ready to return to the STEP 14-01 crafter.

## Current disposition — accepted-cleanup console observation

The walking skeleton is GREEN through production TCP, UDP, terminal cleanup
and VMM disappearance after one bounded DISTILL correction to its existing
test-local console observer.

The failing post-terminal path read was incompatible with the accepted
production lifecycle. On a natural guest exit, `run_exit_watcher` reads the
guest report, passes the Running-confirmed gate, claims the unique `LiveVm`,
and calls `cleanup_driver_artifacts`. That helper kills/removes the cgroup,
removes the complete `VmRunDir` (including `console.log`), then removes the
rootfs clone and index link. Only after all four cleanup calls return does the
watcher send `ExitEvent`; the allocation's `Terminated` row is downstream of
that event. Therefore `console_of` reading
`/run/overdrive/vm/<alloc>/console.log` after observing `Terminated` must see an
absent path. Retaining that file would weaken the accepted cleanup gate.

`crates/overdrive-cli/tests/integration/vsock_support.rs` now starts a
test-local read-only console capture with `Lane::boot`, before any workload is
deployed. It reads the existing production console file while it exists and
retains the longest byte snapshot per allocation in process memory.
`console_of` still prefers the live file and uses the retained bytes only after
production removes it. `ConsoleCapture::drop` stops and joins its observer
thread. The correction creates no production file, persistence path, API,
hook, adapter override or cleanup owner.

The authored walking-skeleton body and every assertion are unchanged. Its
source diff from `822b53395` remains the single removed STEP 14-01 ignore
marker. The console assertions still require the actual guest-emitted
`round-trip: tcp ok` and `round-trip: udp ok` lines; independent remote-peer
source-address assertions and `hypervisor_of(&alloc).is_none()` still execute.

Durable run `d42014b2202f9df175307503` proves the correction:

- the retained existing boundary passed all 111 bodies;
- the exact CLI walking skeleton passed `1 passed; 0 failed; 0 ignored` in
  3.71 seconds;
- its pass includes byte-exact TCP and UDP, both remote-peer source-address
  assertions, both retained console-line assertions and the no-live-VMM
  assertion; and
- the next control-plane cell
  `the_forwarder_inventory_counts_every_held_guest_flow_through_serve` also
  passed exactly once.

The command later exited 101 because the following independently-authored
control-plane cell timed out waiting for a replacement allocation after its
intentional VMM kill; its row remained `WorkloadCrashedImmediately { signal:
Some(9) }`. That is a production outcome outside this console-observer
remediation. It neither invalidates the exact walking-skeleton pass nor
authorizes another fixture change.

No DES phase was recorded and no commit was created. The same independent
DISTILL reviewer approved this bounded correction in iteration 3 with no new
findings. That approval closes only the console-observation support; the later
control-plane replacement-allocation timeout remains visible and outside this
fixture disposition.

## Disposition after DISTILL review iteration 1

The reviewer-identified fixture entrypoint defect is closed in the existing
test-only PID-1 wrapper. The same native command now reaches production
`overdrive-init` inside the nested workload VM and produces a genuine semantic
RED in production behavior:

```text
overdrive-init: fatal: could not connect the beacon vsock socket to the host
after repeated retries: ECONNRESET: Connection reset by peer
```

The production path is exact and needs no new test mechanism:

1. the CLI body enters production `serve` and the public deploy handler;
2. production `serve` binds kernel-assigned node vsock ports;
3. the production VM driver renders the accepted
   `overdrive.transport=vsock2,...,beacon=<bound-port>,...` token;
4. nested Cloud Hypervisor boots the stock-kernel workload image;
5. the existing PID-1 wrapper loads the reviewed stock-kernel prerequisites
   and `exec`s the embedded `/lib/overdrive/overdrive-init`; and
6. production `overdrive-init::connect_beacon_once` still dials the fixed
   `BEACON_VSOCK_PORT` (`1234`) instead of the token's kernel-assigned beacon
   port, so the connection is reset and the allocation becomes
   `VmGuestExitUnreported` instead of `Terminated`.

This is `MISSING_FUNCTIONALITY`, not a fixture, compile, KVM or infrastructure
failure. DISTILL support stops at that semantic RED. It does not change
production source, the accepted cmdline/API/config surface, any authored test
body or oracle, or the runner's fail-fast behavior. The five control-plane
bodies remain selected but are not entered after the walking skeleton's
semantic failure; DELIVER owns making the activated acceptance boundary GREEN.

No DES phase was recorded and no commit was created.

## Review remediation — closed fixture entrypoint defect

`crates/overdrive-testing/bin/guest_peer.rs` now selects its existing
production-fixture program in this order:

1. an explicit `overdrive.testinit=` value, preserving every direct
   `GuestKit::boot` call; then
2. the existing fixed `OVERDRIVE_INIT_GUEST_PATH` when that file exists.

`GuestKit::image` installs that file only for the existing
`GuestInit::OverdriveInit` workload mode. Scripted-peer images do not contain
it, so their required `overdrive.testpeer=` behavior is unchanged. Both cases
reuse the existing `raise_nofile`, prerequisite-module loading and `exec`
sequence; the wrapper remains PID 1 until it replaces itself with production
init.

The correction is private DISTILL fixture HOW. It adds no public API, flag,
constructor field, lane, selector, executor, scenario, cmdline token or test
seam.

## Bounded native execution support

The support delta remains confined to existing test infrastructure:

- `xtask/src/guest_kit_prepare.rs` stages the existing
  `overdrive-cli::integration` binary and builds the existing
  `overdrive-control-plane::integration` binary with its declared
  `kvm-tests` feature.
- `crates/overdrive-testing/bin/fwd_guest_suite.rs` adds the six exact libtest
  identities to the existing `cgroup-default` selector. The prior 111-body
  accounting and unrelated `mtls-relay` 46-body accounting remain unchanged.
- STEP 14-01 uses fresh disposable outer GuestKit guests. Kernel modules,
  cgroup v2, bpffs, securityfs, nft, routes, namespaces and inner workload
  VMMs remain inside those guests. The physical Metal host launches only the
  confined outer VMM.
- Every STEP 14-01 invocation uses the existing required-execution path:
  exact identity, nonzero-exit failure, exact successful summary requirement
  and line-oriented runtime-SKIP rejection.
- The E6 compiler fallout remains neutral: the deleted D14
  `OutboundTproxyInstall` wrapper is removed around the retained typed
  `NftRuleInstallFailed` cause, and the readback checks the accepted one-set /
  three-prerouting / nine-output identity plus the sole dynamic
  `inbound_destinations` set. The two known E6 runtime failures were not run or
  reclassified.

No scenario, production API, config, hook, model, oracle, privileged bypass or
deferral was added.

## Exact selected identities and body integrity

| Binary | Exact identity | Count |
| --- | --- | ---: |
| `overdrive-cli::integration` | `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy` | 1 |
| `overdrive-control-plane::integration` | `integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve` | 1 |
| `overdrive-control-plane::integration` | `integration::guest_flow_through_serve::a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted` | 1 |
| `overdrive-control-plane::integration` | `integration::guest_flow_through_serve::the_forwarder_inventory_counts_probe_and_relay_watches_through_serve` | 1 |
| `overdrive-control-plane::integration` | `integration::guest_flow_through_serve::a_dropped_probe_stream_is_an_abort_at_the_guest_through_serve` | 1 |
| `overdrive-control-plane::integration` | `integration::guest_flow_through_serve::a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve` | 1 |

Run `bcaa2904fa55d407c296ea8e` collected exactly the five control-plane
identities. The runner manifest contains those five plus the one CLI identity.
All six tests retain `/// CONTRACT_SHAPE: bounded-change.`. Their source diff
from `822b53395` is exactly six deleted STEP 14-01 ignore markers and no body,
fixture call or assertion edit.

## Decisive semantic RED

The decisive command was:

```text
cargo xtask metal run -- bash -c 'set -eu; cargo xtask guest-kit-prepare; OVERDRIVE_FWD_GUEST_GROUP=cgroup-default cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests'
```

Durable run `25c755136da086ec4ce18c0a`, source digest
`8c81ee137d589a74d8c961656d2634c0648390888e82c3016741abc5b257894b`,
exited 101 by `command_returned`.

Before entering STEP 14-01, 101 successful libtest summaries declared exactly
111 passed bodies, zero failed, zero ignored, and zero recognized runtime-SKIP
diagnostics. This re-proves the retained
`55 + 18 + 5 + 20 + 13 = 111` boundary.

The walking skeleton then entered once and failed its unchanged observable
assertion:

```text
expected allocation state: Terminated
observed allocation state: Failed
reason: VmGuestExitUnreported { vmm_exit_code: Some(0), vmm_signal: None }
inner console:
  Run /sbin/init as init process
  NET: Registered PF_VSOCK protocol family
  overdrive-init: fatal: could not connect the beacon vsock socket to the host
  after repeated retries: ECONNRESET: Connection reset by peer
```

The earlier `overdrive-guest-peer: no overdrive.testpeer=` failure is absent.
The wrapper correction therefore reaches and executes production init exactly
as reviewed. The new failure belongs to production's incomplete dynamic-beacon
attachment cut, already identified as RED in the accepted feature delta; it
does not justify another fixture change.

## Safe boundary and cleanup

All kernel-sensitive and real-VM effects occurred inside the disposable outer
GuestKit guest. On assertion failure, the existing teardown bound killed the
empty workload-scope complement, the test process ended, and GuestKit/WorkDir
drop destroyed the outer guest and its staged artifacts.

Read-only durable audit `8a64bea495949838e9a0c174` exited zero and found:

- `live_cloud_hypervisor=0` on the physical Metal host;
- no physical-host nft tables; and
- no `*step14-01*` work directory under `/srv/vm/overdrive-testing`.

The support did not run a production test binary, netns operation, nft command,
module operation or cgroup mutation directly on the physical host.

## Refreshed verification and receipts

Full durable records are retained byte-for-byte under
`.context/netns-density-295/14-01/raw/<run-id>/`.

| Purpose | Run ID | Source digest | Exit | Outcome |
| --- | --- | --- | ---: | --- |
| Initial remediation gate | `1cc65bc1e9d2d1695f8f0fa7` | `fbadae0b208fafcd38e8f20cad0eb369c918bfd48b6104166348a41d4f749430` | 101 | Clippy exposed the 100-line ceiling after the inline correction |
| Final format/check/Clippy | `85de600c150b2c60d426ff53` | `8c81ee137d589a74d8c961656d2634c0648390888e82c3016741abc5b257894b` | 0 | Guest peer and guest-suite runner formatted, compiled and Clippy-clean with `-D warnings` |
| Semantic RED | `25c755136da086ec4ce18c0a` | `8c81ee137d589a74d8c961656d2634c0648390888e82c3016741abc5b257894b` | 101 | Retained 111 pass; production init executes; fixed-beacon connect is reset |
| Physical-host cleanup audit | `8a64bea495949838e9a0c174` | `8c81ee137d589a74d8c961656d2634c0648390888e82c3016741abc5b257894b` | 0 | No live VMM, nft table or STEP 14-01 workdir |

The semantic-RED output is 397,879 bytes with SHA-256
`f618ec0ca1c8462df94c0b82875df9d08a28c686ceaaa7b623fe8e41e7913662`;
its terminal record SHA-256 is
`ebeac03a837887c4ba2030f89d54949276d9f2b26e3a55dace5ae7d258d83e5d`.
The final source-gate output SHA-256 is
`56e9c27277ca1fb38eb640d9ce8c87e0c581026e697c42548f6c55b4ae87429c`.

## Source identity at iteration-2 handoff

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-testing/bin/guest_peer.rs` | `3d8c8d70da22f64f2c48c4ffd0b14efae503704e3ffbc4424328ee2872d69f39` |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| `xtask/src/guest_kit_prepare.rs` | `e436cea4685646eb044606ccf5af1e8fe3f6f9e78581ad7cda3dac16d87a0e69` |
| E6 compiler fixture | `2b67117eff47b1252ab66348dbc0e8f6ad3cac86579bdbab97caee9cc0dbd341` |
| Activated CLI test | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Activated control-plane tests | `c8df77ed2880a3a56b249f6172ab97280155e07149d036c9f1bac4b5afadc937` |

The bounded remediation and refreshed evidence are ready for the same
independent DISTILL reviewer iteration 2. The fixture finding is closed; the
new result is the honest STEP 14-01 semantic RED for DELIVER.

## Console-observer remediation receipts

Full durable records are retained byte-for-byte under
`.context/netns-density-295/14-01/raw/<run-id>/`.

| Purpose | Run ID | Source digest | Exit | Outcome |
| --- | --- | --- | ---: | --- |
| Initial scoped source gate | `387a3ecb5e174249303e8faa` | `6adea20d8968ab56d06d9f3ecea0a298a334f525d3baa0f264bb67a93122e347` | 101 | Clippy requested a tighter console-cache lock lifetime |
| Final scoped format/check/Clippy | `83e0f2f61de179d79994b496` | `7eda43f46739334fd4ee0867bcea2c11b3f8194f03c31a740de1c5ef56a8328d` | 0 | CLI kvm-gated integration binary formatted, compiled and Clippy-clean with `-D warnings` |
| Native GuestKit run | `d42014b2202f9df175307503` | `7eda43f46739334fd4ee0867bcea2c11b3f8194f03c31a740de1c5ef56a8328d` | 101 | Existing 111 pass; CLI walking skeleton pass; first control-plane cell pass; next production cell timeout |
| Physical-host cleanup audit | `fdc735fa7064532ba2d65e49` | `7eda43f46739334fd4ee0867bcea2c11b3f8194f03c31a740de1c5ef56a8328d` | 0 | No live Cloud Hypervisor, physical-host nft table or STEP 14-01 workdir |

The native output contains 103 successful libtest summaries whose declared
counts sum to 113 (`111 + 1 + 1`), with zero recognized runtime-SKIP
diagnostics. Its 446,641-byte output log has SHA-256
`6a3f64583c1e767313edf5c0636efa27c5e26df2133e7aff19f91dab9650fd5b`;
its terminal record has SHA-256
`49365c9dd5cbe44ade0578a7d5bafe2cc52e5d9528f6776ddeaea495186c7e26`.
The final scoped source-gate output has SHA-256
`9c1a7b04d1ad76b5b25a9ea18a4b94d7db6aff952c04b3fde5f0bae7a2999d42`.

## Source identity after console-observer remediation

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-cli/tests/integration/vsock_support.rs` | `8f5e46ce02d32820917e569a0a70c4d352435aabb308f84d75f274edd608ebba` |
| Activated walking-skeleton body | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| Crafter-owned `crates/overdrive-init/src/main.rs` | `35d5b02d6fa1f119a05806aad4ab665bbfeb6c2becd76ff456a49676f9b37941` — preserved, not edited by DISTILL |
| Crafter-owned `crates/overdrive-init/src/guest_flow/owner.rs` | `6c6f0e9e02a56ddc7ff91244dcbd4e24820c696fb9c8656d969f4dcb26e02932` — preserved, not edited by DISTILL |
| `crates/overdrive-testing/bin/guest_peer.rs` | `3d8c8d70da22f64f2c48c4ffd0b14efae503704e3ffbc4424328ee2872d69f39` |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |

The console observer is the only new source change in this remediation. The
crafter's dynamic-beacon and pending-`TcpConnect` half-close implementation,
all marker/DES edits, user-dirty `AGENTS.md`, and the approved production
cleanup order remain untouched.
