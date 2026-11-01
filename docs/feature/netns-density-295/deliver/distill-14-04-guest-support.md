# DISTILL handoff — STEP 14-04 existing-test and GuestKit support

## Handoff metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-04` |
| Role | Fresh isolated DISTILL acceptance designer |
| Source reviewed | `db7629972071523535ddbb76983ef1f6e309dd9b` |
| Accepted parent | `92f0c60d4e3001dca7a233b7ec25979bafde1055` |
| DELIVER review | `review-14-04.md`, iteration 1, `NEEDS_REVISION` |
| Scope | Existing test identities, fixtures, GuestKit staging and source gates only |
| Native execution | **NOT RUN — independent DISTILL review is required first** |
| Result | **CURRENT READINESS PRECONDITION ADDED THROUGH THE EXISTING TYPED SNAPSHOT — ITERATION 8** |

No production behavior, public or cross-crate API, typed error, scenario
identity, DES event, roadmap, marker, executor, lane, model, seam or deferral
changed. Two existing acceptance bodies are strengthened at their existing
production boundaries: V-6 now retains its two allocations' public identity
and lifecycle guarantees, and the existing forwarded-outbound worker body now
retains the production same-socket splice guarantee. Iteration 3 corrects only
that worker body's private two-phase peer/observer fixture after the first
authorized native run exposed its one-phase and ptrace-interruption defects.
The user-owned `AGENTS.md`, the accepted HTTP response-disposal fix, the
original crafter's private E23 adapter correction and the recovered STEP 14-04
evidence package are preserved. Iteration 5 keeps the strict natural-Job
completion oracle and corrects V-6's settled Service row to the approved typed
split: reconciler-attributed row/transition reasons, operator-attributed
terminal condition and `exit_code=None`. Iteration 6 keeps the third existing
STEP 14-01 control-plane body's pass and completion contracts unchanged while
making its already-terminal failure path perform the same operator-stop,
inventory-complement and graceful-server cleanup required by its success path.
Iteration 7 preserves that primary relay failure as the first final error even
if one or more cleanup operations also fail. Iteration 8 corrects the existing
third control-plane body's missing precondition: the same one-shot client is
created only while the exact Service allocation's latest typed readiness row
is `Pass` under its unchanged threshold-one configuration.

## Authority and reconciliation

This handoff applies the exact STEP 14-04 dispatch, the frozen 04-03 /
S-ND295-36 suite map, the current replacement feature delta, the current test
sources and retained Git history. The project infrastructure policy is
inherited: Rust tests remain direct `#[test]` / `#[tokio::test]` bodies and no
`.feature` consumer is introduced.

The feature has no DISCUSS, DESIGN or DEVOPS `wave-decisions.md`; only the
SPIKE decision file exists. Missing prior-wave decision files are warnings
under the DISTILL graceful-degradation rule. The present-tense approved
feature delta and its accepted ADRs supply the driving and native boundaries.
Reconciliation passed with zero contradictions among the decision files that
exist. This remediation authors no new scenario.

## Frozen identities and provenance decision

Frozen 04-03 named these two positive bodies in
`guest_stack_mtls_egress.rs`:

- `microvm_dials_a_mesh_peer_by_name_and_receives_the_reply`;
- `the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear`.

Both bodies exist in accepted history through `e98a5382b`; commit
`b8bdf6005` removed the bridge/TAP-era `run_mesh_guest_scenario` composition
and both bodies. The deletion was followed by the accepted replacement
contract, rather than by removal of their user outcomes:

- `END-STATE` makes the guest NIC-less and vsock-only
  (`feature-delta.md:1304`, approved 2026-10-11);
- ADR-0072 dial-by-name remains retained (`feature-delta.md:1603`);
- old roadmap 10-01 is explicitly **Replace — the vsock attachment end to
  end (V-6)** (`feature-delta.md:1723`);
- V-6 is the current production-composition journey through `serve` and
  `deploy` (`feature-delta.md:13180`).

The retained outcomes map to two existing current identities at their proper
boundaries:

- the user journey, per-allocation identity and public lifecycle universe stay
  in
  `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy`;
- the production leg-F kTLS/splice mechanism stays in
  `integration::vsock_mesh_egress_mtls::registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again`.

The outcome mapping is exact at the current architecture boundary:

| Frozen outcome | Current assertion in the existing mapped body |
|---|---|
| A VM Service dials its mesh peer by name and receives the reply | The client is deployed with `peer_name = "v6-peer.svc.overdrive.local"`; its production console must report the gateway nameserver, the unique resolved peer name and `vip-tcp ok` (`vsock_walking_skeleton.rs:144-177`). |
| The selected mesh path is mTLS and never carries the plaintext application markers | Capture is armed before the client Service. `assert_tls13_no_cleartext` requires TLS 1.3 software TX/RX application records on the exact backend tuple, zero plaintext markers, unambiguous parsing and lossless capture (`vsock_walking_skeleton.rs:139-177`; `vsock_support.rs`). |
| Both peer and caller carry their exact per-allocation SPIFFE identity | The existing V-6 body polls the public `issued_certificates` projection for each Running allocation and requires exactly `SpiffeId::for_allocation(workload, alloc)`, once, before crediting the journey (`vsock_walking_skeleton.rs:113-115,163-175,249-287`). |
| The caller's complete public allocation-row universe changes only in the historically permitted lifecycle fields | V-6 reuses the existing complete-row complement through `assert_exact_operator_stopped_service_lifecycle_delta`. The client Running row and its operator-stopped `Terminated` row may differ only in state, reason, workload address, timestamp, exit code, transition and terminal payload. The settled row reason and last-transition reason must be `Stopped { by: Reconciler }`; the terminal claim remains `Stopped { by: Operator }`; `exit_code` is `None` (`vsock_walking_skeleton.rs:219-240`). |
| The exact live kTLS socket owns both production splice directions | The existing forwarded-outbound happy-path body observes the one live TLS 1.3 software-TX/RX socket to its registered backend through `ss`, resolves its sole process fd through `/proc/self/fd`, and requires that exact fd in positive `splice(2)` source and destination records captured by bounded external `strace` (`vsock_mesh_egress_mtls.rs:1053-1235,1242-1289`). |
| Lifecycle and cleanup remain total | Public stop requires both Service allocations to reach `Terminated`; the caller receives the complete-row comparison, both VMMs disappear, the production population is absent, and the ephemeral range is restored (`vsock_walking_skeleton.rs:219-246`). |

The old bodies' TAP queue holder, shared bridge, per-TAP intercept event,
virtio-net header and peer-interface assertions describe the superseded NIC
attachment. Restoring their historical body would require rebuilding the
deleted bridge/TAP fixture or writing a new vsock body under an old name. The
first contradicts approved END-STATE; the second would be a new test body and
scenario identity disguised as restoration. Neither is authorized. The
existing V-6 and forwarded-outbound identities are therefore the precise
current mapping, and the historical bodies are not restored.

The mechanism assertion is intentionally not added to V-6. It belongs to the
existing worker adapter-integration body that directly composes
`MtlsInterceptWorker::register_forwarded_outbound`, `HostMtlsIntercept` and
`HostMtlsEnforcement`. Its observation is external/kernel-visible only: `ss`,
`/proc/self/fd` and `strace`. No production accessor, hook or second owner is
introduced. V-6 receives only the public identity and allocation-row
assertions that are part of its existing peer/client journey.

This decision does not use the two E6 negative fixture failures as positive
credit. They remain the qualified pre-existing baseline only:

- `integration::guest_stack_mtls_egress::fault_fixture::setup_fault_matrix_restores_the_exact_production_object_graph` — injected
  `setup-original-rename-before-intent` expected `Err`, observed `Ok`;
- `integration::guest_stack_mtls_egress::fault_fixture::trapped_fixture_paths_restore_programs_counters_handles_userdata_and_fib` — injected
  `setup-original-rename-after-mutation` remained observable.

## Existing userspace and Sim evidence retained

The original crafter recovered the existing durable records under
`.context/netns-density-295/14-04/`. This remediation did not rerun them.
`selection-exclusion-ledger.md` is the exact evidence map.

| Required family | Retained evidence |
|---|---|
| Control-plane `service_kind_vm_workloads` plus all six `mtls_resolve_rekey` properties | Run `123d1d305a59c62621167c6a`, 7/7, exit 0. |
| Worker `service_kind_vm_workloads` | Run `fadba7c7f11b3c5f841961cc`, 7/7, exit 0. |
| S-SVM-14 bounded status-only body | Run `6bb340a111340465b09285c5`, 1/1, exit 0; semantic pre-fix RED retained as `0c7c8f7a010f45d98c85ce10`. |
| `cgroup_accounting_equivalence` | Run `47d5b8e4c15f2130fcfc0efe`, 2/2, exit 0. Both cases are TempDir-backed and open no host cgroup or VMM. |
| Exec-release Sim composition | Run `a195e03fe928822d439da399`, 3/3, exit 0. |
| Guest-network Sim composition | Run `b5a23f152afc770d5530c673`, 7/7, exit 0. |
| DNS semantic / codec acceptance | Run `49c18c3dacf978236f7a8056`, 32/32, exit 0. This is not credited as the real port-53 socket/wire proof. |
| CLI safe Service subset | Run `9caa18ff5d95626ec9416474`, 2/2, exit 0. The real-cgroup-dependent third body remains a recorded permission refusal, not a waiver. |
| Formatting, worker compile and scoped worker Clippy | Runs `c0ff48e32c19090a51fb8596`, `4a9da7d33271efa30c6c970e`, `0e3c5751dd8f206943903e77`, all exit 0. |

The independent review's two evidence-precision findings are corrected in the
ledger without changing any hashed raw record:

1. run `fadba7c7f11b3c5f841961cc` now records the metadata's actual source
   digest `a428484e88188b87386a7da61816088f75694dd5f77e110282a09af50fa35ea8`;
2. the E23 row now distinguishes the original retained run source, which did
   not select E23, from the current prepared and still-unexecuted
   `CGROUP_WORKER` support;
3. the frozen guest-stack row now records the remediated existing-identity
   mapping as prepared but natively unexecuted, rather than claiming the
   current bodies remain absent.

## Exact test-to-GuestKit manifest matrix

Every native effect stays inside the existing disposable GuestKit boundary.
`guest-kit-prepare` builds the current libtest target, reads Cargo's actual
test executable from JSON output, and copies that ELF to
`target/guest-kit/in-guest/<package>--<target>` (`guest_kit_prepare.rs:265-272,
298-320`). The runner checks that staged ELF through
`InGuestTests::prepared`; it does not build in a test, copy source or generate
a launcher.

| Required outcome / exact identity | Existing manifest and staged ELF | Safe execution boundary | Count after this change |
|---|---|---|---:|
| Frozen guest-stack journey, identity and lifecycle outcomes, mapped to the strengthened existing `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy` | `STEP14_01_CLI`; `overdrive-cli--integration` built with `integration-tests,kvm-tests` | Fresh `step14-01-cli` outer member of `cgroup-default`; `run_nested_vm_test_binary` launches the body's workload VMs as nested production `CloudHypervisorVmm` guests | 1 |
| E23 `integration::real_cgroup_fs::refused_thread_kill::a_kill_write_under_thread_refusal_returns_an_io_error_and_never_aborts` | **Added as one exact row to existing `CGROUP_WORKER`**; existing `overdrive-worker--integration` ELF | Existing fresh `cgroup-worker-integration` member of `cgroup-default`, with its private cgroup-v2 mount and `pids` controller enabled | 1 |
| Frozen same-socket splice outcome, mapped to the strengthened existing `integration::vsock_mesh_egress_mtls::registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again` | **Added as one exact row to existing `CGROUP_WORKER`**; the same existing `overdrive-worker--integration` ELF; `strace` is staged as an ordinary external observer | Existing fresh `cgroup-worker-integration` member of `cgroup-default`; real leg F, host enforcement, kTLS socket and production splice pumps remain inside the disposable guest | 1 |
| Real DNS responder | The seven exact `integration::dns_responder_bind::*` rows in `CONTROL_PLANE_NORMAL_NATIVE` | Existing `cgroup-control-plane-integration` outer guest | 7 |
| Exec/process netns and veth | Two `serve_boot_provisions_veth::*`, seven `veth_provision_idempotent::*` and two `workload_netns_provision::*` exact rows in `CONTROL_PLANE_NORMAL_NATIVE` | Same existing control-plane outer guest; netns, veth, XDP, routes and cleanup remain guest-local | 11 |
| Leg-C constant-prefix path | `integration::vsock_constant_guest_prefix_rules::a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways` in `STEP13_04_REMOTE_FIREWALL` | Existing cgroup worker outer guest, after its temporary default route is removed | 1 |
| Leg-C mTLS/kTLS/splice path | `integration::mtls_inbound_enforce::inbound_enforce_origdst_server_mtls_ktls_rx_splice_to_server` in `DATAPLANE_NATIVE` | Existing `mtls-relay` disposable guest | 1 |
| Service dataplane selection | `integration::service_map_forward::ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth` in `DATAPLANE_NATIVE` | Existing `mtls-relay` disposable guest | 1 |

No whole-workspace or direct-VMM test runs are allowed on the physical host.
The outer guest uses production `CloudHypervisorVmm`,
`VmConfinement::confined`, uid/gid 4200, the actual `/dev/kvm` and
`/dev/vhost-vsock` supplementary groups, Landlock/seccomp through the
production launcher, and the existing bounded cleanup (`guest_kit.rs:542-624,
925-940`). Nested execution is used only by the already-declared STEP 14-01
production-composition body.

## Manifest accounting

Adding E23 and the one mapped existing forwarded-outbound body changes only
existing `cgroup-default` accounting:

| Segment | Exact bodies |
|---|---:|
| Retained cgroup baseline | 57 (was 55; E23 and the mapped existing forwarded-outbound body are the two additions) |
| STEP 13-04 | 18 |
| STEP 13-05 | 5 |
| Control-plane normal-native suffix | 20 |
| STEP 13-06 | 13 |
| Complete pre-STEP-14 cgroup boundary | 113 |
| Existing STEP 14-01 nested bodies | 6 (one CLI + five control-plane) |
| **Canonical `cgroup-default` result** | **119** |

The existing `mtls-relay` accounting stays 46: seven inbound-relay baseline
bodies, 36 dataplane integration-native bodies and three dataplane library
native bodies. Focused STEP 14-01/14-02 diagnostic output now names zero
`canonical_119_credit`; no focused diagnostic can be added to the canonical
result.

## Native RED `2572dafb29ff625bdddc3bbb`: causal diagnosis

The current-source GuestKit run advanced through corrected E23, then failed
the existing forwarded-outbound identity. Its complete console records two
observations:

1. the peer-session thread panicked in `watch_termination` when its blocking
   raw TCP read returned `EINTR` (`Interrupted system call`, errno 4);
2. the exact live TLS 1.3 software-TX/RX socket was fd 15, while the positive
   splice sets were `source_fds={15,19}` and
   `destination_fds={14,20}`. The socket was a source but not a destination.

The EINTR is fixture-induced by the syscall observer, not a production
transport failure. The failing stack is the test peer acceptor's session
thread → `run_peer_session` → `watch_termination` → `RecordingTcp::read` →
`std::net::TcpStream::read`; it never enters a production owner or pump.
`SpliceTrace` attaches before the connection exists and detaches immediately
after the first exchange and live-kTLS observation. At that point the
peer-session thread has completed its response and is blocked only in
`watch_termination`. SIGTERM makes the owned strace child detach from all
traced threads; that ptrace stop interrupted the peer fixture's raw read.
The production splice loop is a separate path and already handles `EINTR` as
no-progress interruption before retrying the same blocking splice
(`overdrive-dataplane/src/mtls/splice.rs:513-547`). The raw failure therefore
does not demonstrate a reachable production mTLS defect.

The splice-set mismatch is a real oracle-setup defect, not evidence that the
required production direction is absent. The first request can be captured
during establishment and passed to the forward encrypt pump as its in-memory
`prelude`; that pre-arm buffer is deliberately delivered by the pump thread's
blocking `write_all` because it has no source fd. The one-phase fixture then
observed only the return steady-state splice: live kTLS fd 15 appeared as a
positive source, while no post-establishment request existed to make it a
positive destination (`overdrive-dataplane/src/mtls/outbound.rs:121-147`).
Retrying the same one-phase run could never establish the missing direction
honestly.

The bounded correction stays in the same existing test identity and private
peer fixture:

- the primary body selects a two-phase peer mode;
- after the first byte-distinct request/reply completes, a second distinct
  request/reply crosses the already-established pumps, forcing the forward
  and return steady-state splice directions;
- the peer records and requires that second request before the test may credit
  its syscall evidence;
- only `ErrorKind::Interrupted` on the peer fixture's raw termination read is
  retried, within the unchanged stop flag and 60-second deadline. An
  interrupted blocking read consumed no bytes; EOF, reset, timeout and every
  other error keep their existing meanings and fail-closed handling.

No timeout is lengthened, no syscall error is broadly ignored, and no
production hook, accessor, API or retry policy is added. The oracle still
requires the exact live kTLS inode's sole process fd to appear in both positive
splice sets. V-6's identity and complete-row complement assertions remain
unchanged; the terminal-kind expectation is corrected below.

## Native RED `5e5f472e4191780088cc1a96`: lifecycle-oracle diagnosis

After the splice-fixture correction was approved, the current cgroup-default
run passed all 113 pre-STEP-14 bodies and failed the nested V-6 body at its
complete lifecycle comparison. The client Service terminal row had
`exit_code=None`; the reused historical helper required `Some(0)`.

This is a fixture-oracle mismatch, not a production failure. V-6 stops both
Services through the existing public `stop` verb. `WorkloadLifecycle` emits
`StopAllocation` with
`TerminalCondition::Stopped { by: StoppedBy::Operator }`; the action shim
awaits driver stop, retirement and cleanup, then writes the typed terminal.
`VmDriver` drains the stop writer and reaps both VMM processes with status 0,
but an operator stop deliberately does not reinterpret the VMM process status
as a guest application `EXIT` record. The public handler projects
`exit_code` only from `Completed` and `Failed`; approved `Stopped` therefore
projects `None`. The raw path contains no guest error, peer error or EXIT
beacon before the assertion. The later bounded serve teardown follows the
assertion failure and is not its cause.

The existing `assert_exact_lifecycle_delta` helper originated with the frozen
natural Job-completion body. Its `Some(0)` assertion remains strict and
unchanged for that path. The bounded correction adds a sibling private helper
for the already-existing operator-stopped Service journey. It reuses the same
complete-row complement comparison, then additionally requires all approved
public meanings exactly:

- terminal state `Terminated`;
- `reason == TransitionReason::Stopped { by: Reconciler }`;
- the last transition carries that same settled compatibility reason;
- `terminal == TerminalCondition::Stopped { by: Operator }`; and
- `exit_code == None`, proving the reaped VMM status was not misreported as
  an application exit.

V-6 calls this Service-specific helper only for its caller allocation. The
exact per-allocation identity, complete lifecycle complement, same-fd splice,
cleanup and natural Job `Some(0)` guarantees remain intact. No wire field,
public API, state meaning, timeout, retry or test identity changes.

## E23 activation boundary

The E23 body and its `/// CONTRACT_SHAPE: unbounded-preservation.` declaration
remain unchanged. After support iteration 2 was approved, the original STEP
14-04 crafter removed its pending marker and corrected the private production
adapter. Run `2572dafb29ff625bdddc3bbb` advanced beyond E23 without the prior
Tokio panic. This remediation preserves that active body and correction; it
changes neither marker nor production source.

The test creates the allocation scope before the cap, moves its own test
process into a scratch child, caps `pids.max` at its current task count, calls
the retained allocation-scoped `CgroupManager::cgroup_kill`, and accepts only
`Ok(())` or the existing typed `io::Error`. `AllocCleanup` and the refusal
guard retain cleanup ownership. No new error representation or test seam is
needed.

## Native RED `129e9c3d78f087fc13e8868e`: EXEC completion and relay diagnosis

The current exact-119 cgroup-default run did not complete the third
`STEP14_01_CONTROL_PLANE` invocation. The apparent green summary at the end of
the ordinary outer console is not evidence for that invocation:

- `test result: ok. 1 passed; ... 146 filtered out` belongs to the preceding
  V-6 `overdrive-cli--integration` binary;
- inside the timed-out control-plane guest, the first and second exact
  invocations each printed `1 passed; ... 219 filtered out` and returned;
- the third exact invocation started at 12:28:30Z and printed no test-result
  summary.

The command and process boundary are unambiguous. GuestKit sent `EXEC` to the
outer guest's persistent PID-1 peer. That peer uses `Command::status` for
`PeerCommand::Exec`, while `run-with-modules` and the cgroup shell both replace
themselves with `exec`; an `EXIT` reply is therefore emitted only after the
integration-test process exits. The third test's own log proves receipt and
start. The absence of both a libtest summary and an EXEC reply proves the
process remained live; this is not lost reply framing and cannot be credited
as a completed test.

The primary failure happened earlier than the 600-second GuestKit bound. The
Service listener was live and prior readiness probes reached it. At
12:28:39.702Z, readiness flow 2147483655 returned
`Unreachable { errno: KernelErrno(104) }`. The production service lifecycle and
service-map hydrator projected that readiness failure into no healthy backend.
At 12:28:39.769Z, the client allocation's one-shot TCP flow followed the real
owner path `handle_connection` -> `tcp_connect` -> `MtlsResolve`; the retained
resolver correctly classified the claimed but currently unhealthy backend as
`MeshUnreachable`, the owner sent the typed refusal, and the guest application
reported `terminal=reset`. The existing `wait_flowing` assertion waited to its
120-second bound and panicked at `guest_flow_through_serve.rs:568` at
12:30:39Z. This body never reached the relayed-pair assertions or its normal
operator-stop sequence.

The secondary EXEC hang is private fixture cleanup. The panicking `Node`
dropped its `ServerHandle`; the production type explicitly says Drop does not
stop the server. Consequently the server owners and their blocking tasks were
not driven through `ServerHandle::shutdown`. Service VMM PID 555 and client VMM
PID 583 have create and READY records but no stop or reap receipt. Runtime
cancellation produced the final probe-store and shared-audit warnings at
12:30:43Z, after which the test printed no further byte through the outer
GuestKit timeout at 12:38:40Z. The DNS responder's blocking `recvmsg` owner is
stopped only by the retained graceful shutdown path; a dropped Tokio runtime
waits for blocking tasks and cannot turn this panic into a child exit.

The bounded fixture correction changes neither the pass oracle nor the
production classification. A private sibling wait observes the existing guest
terminal line. If the one-shot client is already terminal, the same test body:

1. retains the exact primary failure text and diagnostics;
2. operator-stops the client and Service through their existing public verbs;
3. requires every CID to be gone and the forwarder inventory to be exactly
   `(0, 0)`;
4. gracefully shuts down the existing `ServerHandle`; and
5. reports the original failure.

There is no retry, timeout increase, forced pass, runtime skip, abandoned
cleanup, new test identity, lane, executor, production hook or public API. A
future recurrence now returns a normal failing test/EXEC exit instead of
masking the production path as a 600-second support timeout.

Iteration 7 closes the cleanup-reporting gap without changing any cleanup
effect. The existing test-support panic-evidence pattern catches each cleanup
operation independently, so a client-stop panic cannot bypass Service stop,
the empty-CID observation, the exact `(0, 0)` inventory check or awaited
graceful shutdown. A Service-stop or observation panic likewise cannot bypass
the later attempts. The exact inventory assertion runs only on the required
empty-CID snapshot; if that snapshot cannot be obtained, the final report
records both the observation failure and that the inventory complement was
unavailable, rather than implying it passed. The final panic text begins with
the unchanged `primary_failure`; any stop, observation, inventory or shutdown
panic follows under `cleanup failures after the primary relay failure`. Clean
cleanup retains the prior primary-only report.

The completed read-only assessment shows the production refusal was legitimate
for the state the fixture created. The listener was ready at 12:28:32.458Z and
the first three probes succeeded, but the latest probe before client accept,
flow 2147483655, failed with ECONNRESET at 12:28:39.702Z. The client became
ready at 12:28:39.712Z and was refused `mesh_unreachable` at 12:28:39.769Z;
the next probe passed at 12:28:40.738Z. The body previously required only a
listener line, five seconds of inventory agreement and at least three
historical accepted probes. None required the latest readiness result to be
`Pass`. Production `ServiceLifecycle` and the service-map hydrator correctly
withheld a backend whose latest readiness row was `Fail`.

The bounded correction uses the existing public `GET /v1/allocs` typed
`AllocStatusResponse`; no new accessor or seam is introduced. Immediately
before creating the same one-shot Job, a predicate wait requires:

- exactly the same Running Service allocation;
- exactly one declared readiness probe at index zero, with the existing TCP
  target, one-second interval and `success_threshold=1`; and
- that allocation's single LWW `ProbeResultRowJson` for readiness index zero
  to be `ProbeStatusJson::Pass`.

That typed LWW row is the public current readiness observation consumed by
`ServiceLifecycle`; with the asserted threshold of one, `Pass` is exactly the
backend-eligibility input. A historical console count cannot satisfy it. The
wait uses the existing `SETTLE_BOUND` and condition polling; it adds no blind
sleep or longer deadline. Timeout or snapshot panic remains a primary failure
and reuses the approved cleanup/error aggregation, with no client stop when no
client was created. The same FLOWING, relay-watch, quota, inventory and
historical-probe assertions remain unchanged after the precondition.

This correction does not explain the probe's ECONNRESET. The retained run does
not identify whether that failed probe ended in `connect_guest`, watch setup,
frame write or another internal stage. That mechanism remains unproven and the
qualified raw evidence is retained without labeling the reset expected or
waiving it. The current fallible joined `RealCgroupFs` write remains excluded:
E23 had already passed, both affected VMMs completed cgroup setup and reached
READY, and no cgroup write error, joined-writer wait or thread-refusal event is
on this path.

## Marker sweep

The active feature markers are exactly:

| Marker | Disposition |
|---|---|
| Two `vsock_density.rs` bodies at lines 146 and 200 | Preserved for fresh STEP 14-05. They are sequencing, not an issue deferral. |
| `quint_cid_lease.rs:2108`, `pending #325` | The one approved enduring issue-linked ignore. |

No old step ID is reused and no additional ignore or deferral is introduced.

## Source gates completed

Iteration-1 Metal durable run `b27ef0ffbbe5d2bb222b8857` remains the accepted
E23/runner-only source gate. Iteration-2 preparation runs
`36c263a22f061d3b6901af00` and `515e7d414482ab9ef30a5891`, followed by final
source run `5a7dfdadd33367183c0f0a9e`, used `--no-sudo` and the task-owned
`CARGO_TARGET_DIR=target/distill-14-04-guest-support-r2`. The final run
completed with exit 0 after:

1. `cargo fmt --all --check`;
2. checks for the `overdrive-cli` integration target with
   `integration-tests,kvm-tests`, the `overdrive-worker` integration target,
   the GuestKit runner and xtask;
3. the matching four scoped Clippy commands with `-D warnings`.

Iteration-3 source gate `6dc054f21cde74be787ce6f5` found only three scoped
Clippy defects in the first fixture draft (common enum postfix and two
redundant clones); it ran no tests. After those source defects were corrected,
final unprivileged Metal run `b6e452ada75abd89da0d5047`, using task-owned
`CARGO_TARGET_DIR=target/distill-14-04-guest-support-r3`, passed format, the
worker-integration and GuestKit-runner checks, and both matching Clippy
commands with `-D warnings`. Final source-identity run
`d045bb4c47f30618da6d9085` then passed format plus the worker-integration
check and Clippy `-D warnings` after the completed fixture documentation.

Iteration-4 final source gate `0d2406e019828b0c208d2c0d`, using task-owned
`CARGO_TARGET_DIR=target/distill-14-04-guest-support-r4`, passed format plus
the `overdrive-cli` integration check and Clippy with
`integration-tests,kvm-tests` and `-D warnings`. It ran no tests.

Iteration-5 final source gate `4f8f454c5e786fa142bfd0b5` reused that
task-owned target and passed the same format, CLI integration check and Clippy
commands after the settled typed-reason correction. It ran no tests.

Iteration-6 source run `e5a8f081a342bb1c28dfd80a` was an honest format RED
only; rustfmt required one expression to use its canonical line shape, so the
later check and Clippy commands did not run. After that source-only formatting
correction, unprivileged Metal run `877a7df19197f55043fd02f8`, using task-owned
`CARGO_TARGET_DIR=/home/ubuntu/target-distill-14-04-guest-support-r6`, passed:

1. `cargo fmt --all -- --check`;
2. `cargo check -p overdrive-control-plane --test integration --features
   integration-tests,kvm-tests`; and
3. the matching Clippy command with `-D warnings`.

Neither iteration-6 run executed a test or native effect.

Iteration-7 run `5eb8d4fe3615d642cbe2b5da` was an honest rustfmt RED only;
the later checks did not run. Run `5f16dfa7db0a2ec9ff114149` passed format and
the integration-target check, then Clippy correctly rejected the first report
assembly as a manual-assert shape. After replacing only that expression with
one report value followed by the single final panic, unprivileged Metal run
`9e9aad5c39b2852b4a9d703d`, using task-owned
`CARGO_TARGET_DIR=/home/ubuntu/target-distill-14-04-guest-support-r7`, passed
format, the exact control-plane integration-target check and matching Clippy
with `-D warnings`. Final scoped diff check `3c4868d644c9a7cd83373499` also
passed. None of these runs executed a test or native effect.

Iteration-8 run `219d6e4d94c9d8c0b3216bc1` was an honest rustfmt RED only;
the later checks did not run. After applying only rustfmt's line layout,
unprivileged Metal run `f39c600fd6300f7f41530f03`, using task-owned
`CARGO_TARGET_DIR=/home/ubuntu/target-distill-14-04-guest-support-r8`, passed
format, the exact control-plane integration-target check and matching Clippy
with `-D warnings`. Final scoped diff check `a4ad2a4fa5cbd5117ba3036e` also
passed. Final source-identity run `9943406b681ce601b088feff` repeated and
passed the same format, check and Clippy gate after the completed fixture
wording. None of these runs executed a test or native effect.

Metal run `57784176840fcd2897586a3e` showed the whole-worktree
`git diff --check` remains RED only on concurrent, unrelated E06 expectation
evidence whitespace. Those files were preserved. The scoped check over this
iteration's source, handoff and ledger passed in unprivileged Metal run
`8c43d5a3ea613d06e1068f2e`. No test, kernel, module, nft, BPF, cgroup, netns,
route, sysctl, kTLS or VM effect was executed by this remediation.

## Pending independent review and later native gates

The same independent acceptance-designer reviewer must first approve this
bounded precondition correction. Native execution remains gated. After that
review, the original crafter may restage the current ELFs and run the existing
groups as required by the evidence ledger. No production change is authorized
by the unproven probe-reset mechanism:

```text
cargo xtask metal run -- cargo xtask guest-kit-prepare
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=cgroup-default \
  cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite \
  --features integration-tests
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=mtls-relay \
  cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite \
  --features integration-tests
```

Acceptance requires the actual staged-ELF/source identity, exact 119 and 46
pass accounting, zero failures, zero ignored required bodies, zero runtime
SKIP diagnostics, terminal exit 0, and complete outer-guest cleanup. A busy
Metal lease is ordinary serialization. A lost client route requires
collection of the same run ID. No physical-host cleanup, fallback, process
kill or direct native nextest is authorized.

There is no public interface, typed semantic-error, test-identity or
new-artifact gap in the completion correction. Every retained frozen guarantee
still has an existing exact identity at the current vsock production boundary.
The retained probe-reset mechanism remains unproven; it is not a public
interface or new-artifact gap and is not permission to weaken the scenario.
