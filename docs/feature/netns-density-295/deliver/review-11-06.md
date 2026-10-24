# DELIVER Review — Step 11-06

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `11-06`, guest transport and guest flow owner |
| Reviewer | Fresh, isolated DELIVER reviewer; GPT-6.1 Sol, explicitly requested by the user |
| Date / iteration | 2026-10-24 / 1 |
| Implementation | `2f92c925d374a21f41f74e34bd9f58767d53429e` |
| DES completion record | `04002b431884169f1b6e7b4c579061884c033842` |
| Approved formal alignment and evidence | `9d7463e8bf1c8bb89926dff8b9394cd68a9ad12f` |
| Current route authority | `34588a68d`, independently approved GUEST-TRANSPARENT-ROUTE correction |
| Branch | `marcus-sa/wellington-v3` |
| Verdict | **NEEDS_REVISION** |

The review started after the actual implementation commit. It covers the
45 implementation/acceptance/build files in that commit, the step's DES
completion, and the separately committed two Quint models plus 76 evidence
artifacts. The vDPA/SR-IOV research, the separate 11-02 and 11-04 corrections,
frozen phases 01–04, and retired steps 05–10 are excluded.

The governing instructions are `AGENTS.md`, the applicable repository rules
for development, Rust, testing, BPF and verification, the prepared dispatch,
the current feature delta and roadmap, and ADRs 0038, 0150, 0159, 0174, 0180,
0199, 0201, 0202, 0205, 0206, 0210, 0212, 0215 and 0217. The installed
`nw-software-crafter-reviewer` role and its `nw-sc-review-dimensions`,
`nw-tdd-review-enforcement` and `nw-tdd-methodology` skills were applied
subject to those repository and user overrides. In particular, there is no
review-iteration cap. The explicit user override forbids mutation testing
anywhere in this workflow; no final mutation gate is due.

Only this Markdown artifact and bounded reproduction material under
[`spike-scratch/netns-density-295-review-11-06/increment-a/`](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/README.md)
were authored by the reviewer. The probe sources, runner and every original raw
capture are retained for commit under the repository spike discipline; build
outputs are excluded.
No production source, acceptance body, model, roadmap, DES event, attribution,
branch or existing dirty work was changed. No commit was created. The
pre-existing dirty `AGENTS.md` is preserved.

## Result and acceptance mapping

Three reachable failures were reproduced through the real guest kernel and
the production `overdrive-init` owner. The existing native TCP-correlation
cell also leaves required parts of its accepted verification contract
unexercised. These issues block approval despite the supplied green suites.

| Accepted requirement | Assessment |
|---|---|
| Exact transport token, seven programs, two embedded objects, ordered setup and typed setup stages | Parser, setup dispatch, labels, build/embedding and program ordering match the accepted shape. The guest forwarder probe executes additional host-only stages: F1. |
| Exact private mark `0x295`, priority 32765/full mask/table 295, local-default route/proto boot/scope host, both mark sysctls before READY | Matches the approved replacement in `transport/kernel.rs` and `transport/mod.rs`; the current native route and application-connect evidence supports it. No old `lookup local` fallback remains. |
| Process-owned links/maps; temporary ACTIVE/CHILD pins absent before program load/attach/READY | The two-object loader shares the child map and removes both pins and their directory before the seven kernel program loads. The current native cell checks the bpffs complement before and after service. No runtime adoption mechanism was added. |
| Control session, TcpAccept, TcpConnect, fixed-record I/O, forwarder records and resident UDP slot ownership | The serialized owner and typed handles implement the designed seams; the supplied source-local suites pass. Native UDP cannot start with the kit's supplied kernel support: F2. The BPF release hook bypasses the designed slot-release order: F3. |
| `GuestSlotAttachRefused { error, vsock }`, V_g returned/closed, S resident, association cleanup, exact HalfEnded count, no guest pair/record/control | The carrier and both adapters have the approved shape. The owner closes returned V_g, ends the association without releasing S, and increments the count only for `HalfEnded`. The added source-local cell covers host/guest end-record distinction and later reassociation. |
| Guest Quint replay through the production owner and approved observation surfaces | Current named/randomized replay evidence is green. Owner projections use the pinned observation and named Sim arbiters; the applied VMM-exit boundary is retained. Formal correspondence is assessed below. It does not establish the missing real BPF slot behavior. |
| Current native guest-kit cells | Supplied lane: 12/12, zero skipped. Its production-init cases explicitly enable the optional loopback module; none is a UDP capture/release cell. The default workload kit fails before READY, F1. |
| Candidate negative, terminal/delete monotonicity and established-data invocation observations in the existing parameterized cell | Incomplete in the checked-in cell, F4. The valid live-open Fast Open/shared-quota case is exercised; that does not exercise the other required branches. |
| Bounded compiler/build fallout and deletion boundary | The C/CO-RE source, Clang 21/BTF/header tooling, existing CI/Lima surfaces, guest object target, parser metadata and rustdoc correction are related. No new lane or production API is required by this review. The pending 12-01 token/TAP attachment cut is not demanded here. |

The actual `main.rs` already calls `configure_guest_network` and starts
`GuestFlowOwner` before READY. This commit changes its constructor call only
to propagate the fallible `KernelGuestOwnerIo::new`. The review uses that
actual caller path and does not force the roadmap's stale “not yet called by
main” wording into the implementation.

## Exact interfaces and typed error mapping

The token/port structs, `GuestTransportOps`, seven `GuestProgram` variants,
`prepare_guest_programs`, move-only socket handles, `GuestOwnerIo`, owner
driving/observation methods and fatal enum were compared with DG-5 and the
accepted amendments. Stream refusal remains the two-owned-half
`GuestAttachRefused`; the datagram refusal returns only `HostConn` V_g. There
is no slot arm or fabricated slot handle in `GuestOwnedSocket`. The candidate
is the approved single C/CO-RE exception; the other six guest programs remain
Rust/Aya. The private netlink helpers and owner state organization are HOW,
not missing DESIGN interfaces.

| Designed semantic failure | Approved and implemented representation |
|---|---|
| No transport token / multiple tokens | `GuestTransportSetupError::{TokenMissing, TokenDuplicate}` in `parse_transport_token`. |
| First invalid/missing positional token field, invalid/repeated port, or extra field | `TokenMalformed { field: TransportTokenField::{Version, Addr, Dns, Launch, Beacon, Stream, Control, Extra} }`; later repeated port identifies that later field. |
| Dummy device or workload address setup | `DummyDevice { source: io::Error }` / `AddressAdd { source: io::Error }`. |
| Guest rule / local-default route / accepted-child mark inheritance / kernel reply reflection | `FwmarkRule`, `FwmarkRoute`, `TcpFwmarkAccept`, `FwmarkReflect`, each retaining its own `io::Error`. The four method-to-variant mappings are separate. |
| Reserved port range write / one specific intake bind | `ReservedPorts { source }` / `IntakeBind { addr, source }`. |
| Active-storage create/BTF/pin, Rust object/map preparation or shared staging unlink | `ProgramLoad { program: TcpCorrelation, source: aya::EbpfError }`; map causes are retained as `MapError` where applicable. |
| Candidate object/map reuse/CO-RE/running BTF/FEntry load | `ProgramLoad { program: TcpCorrelationCandidate, source: aya::EbpfError }`. |
| Candidate link attach | `ProgramAttach { program: TcpCorrelationCandidate, source: ProgramError }`. The other six program load/attach errors retain their own program identity. |
| Missing module file | `ForwarderModuleMissing { path }`. |
| Module/running release mismatch | `ForwarderModuleRelease { module: KernelRelease, kernel: KernelRelease }`, before `finit_module`. |
| Other module read/vermagic/load refusal | `ForwarderModuleLoad { source: io::Error }`; `EEXIST` remains successful. |
| Guest context DeviceOpen/Version failure | `Forwarder { source: ForwarderError }`. **F1:** the current call also admits later full-host-probe stages, and the native refusal is `Probe { stage: StreamRoundTrip, ... }`. A typed outer envelope does not make that extra guest requirement compliant. |
| Inbound listener bind | `InboundListenerBind { source: io::Error }`. |
| Readiness wait failure / listener failure outside the retry set | `GuestOwnerFatal::EventWait { source }` / `ListenerFailed { listener, kind }`. |
| Forwarder context read/decode or abort failure | `GuestOwnerFatal::Forwarder { source: ForwarderError }`. The kernel seam wraps an end-read error in `io::Error`; `dispatch_next` downcasts the retained typed source rather than parsing its Display. |
| Session connect/read/write/end failure | Per DG-5-F, close session, abort its flows and reopen at `CONTROL_RECONNECT_BACKOFF`; not a new fatal variant. |
| Application connect refusal / timeout | Existing `ControlMessage::Refused { refusal: ApplicationRefused { errno }, flow }` / `TimedOut { errno: ETIMEDOUT, flow }`; close both unattached halves. |
| Stream / resident-slot attach refusal | Original `ForwarderError` and payload in `GuestAttachRefused { error, vsock, outer }` / `GuestSlotAttachRefused { error, vsock }`. Only `HalfEnded` raises `forwarder_attach_ended`; cleanup does not fabricate a different typed cause. |
| Missing/malformed capture correlation | Private `Rejected`, reset-close and no owner capture event/flow/control. Candidate/promotion replacement uses `BPF_EXIST`; deletion is not re-created by those writers. |
| TCP pending exhaustion / no free UDP slot | Reset before owner admission / capture refusal `EPERM`, respectively. **F3:** the release hook makes a still-attached slot appear free, so the new datagram is admitted into the old slot lifecycle instead of waiting for release or receiving the valid exhaustion refusal. |
| Slot counter/live-cookie diagnostic read failure | Existing `io::Result` seam, retried at audit. **F2:** the kit omits the diagnostic support needed even for the initial `slot_owner`; the initial real read is `ENOENT` and no association starts. |

No new semantic error variant or string carrier is requested. F1 must be
closed against the already-approved guest probe contract. If remediation
actually requires an interface-contract decision, its exact shape must go
through DESIGN; this review does not authorize inventing a probe method,
mode parameter, or failure class.

## Iteration 1 findings

### F1 — High, blocking: the default supported workload kit is refused by an extra guest probe stage

**Contract.** The forwarder probe section expressly assigns the full
DeviceOpen-through-Cleanup probe to the host and says that the guest runs
DeviceOpen and Version before READY (`feature-delta.md:4889`, `:4910`;
DG-5 `open_forwarder` at `:11257`). The supported kit includes the native
fixture's stock kernel (ADR-0206). `GuestImageSpec::workload()` deliberately
does not include the optional vsock loopback module (`guest_kit.rs:352`).

**Reachability and failure.** The real production path is
`main::run` (`main.rs:117`) → `complete_pre_ready_init` (`:213`) →
`configure_guest_network` (`:445`) → `configure_guest_transport` →
`KernelGuestTransportOps::open_forwarder` (`transport/kernel.rs:331`). That
method calls the *entire* `KernelFlowForwarder::probe` at `:335`.
`overdrive-fwd/src/lib.rs:1007` runs StreamRoundTrip and the remaining host
stages after Version. A stock-kernel workload image contains the forwarder,
dummy and vhost-vsock prerequisites but no optional loopback module.
StreamRoundTrip fails. The setup error propagates before the owner starts or
READY is sent; `main.rs:99` prints the cause and powers off. There is no retry
or cancellation schedule involved.

**Reproduction.** `native-default-kit.log` retains a fresh GuestKit image
built from `GuestImageSpec::workload()`, booted through the production
`CloudHypervisorVmm` and production init as PID 1 on guest Linux
`7.0.0-38-generic`. It exits with:

```text
overdrive-init: guest transport setup failed: opening the guest forwarder failed: forwarder probe stage StreamRoundTrip failed
```

The bounded host driver exits 4 after the beacon closes without READY.
Changing only the already-existing fixture option `loopback_module` to true
allows the same production init to reach READY (`native-udp-diagnostic.log`).
The supplied 12-test lane sets that option true in
`guest_lane.rs:283` and therefore does not test the default workload image.

**Required disposition.** Make the existing guest setup satisfy the approved
guest probe contract and boot the supported default workload kit. Do not turn
the host's extra probe stages into an undocumented guest-kernel prerequisite,
weaken READY, or add an unapproved public probe API. This is an implementation
failure, not a falsification of the accepted two-stage guest requirement.

**Status:** OPEN; no reviewer remediation.

### F2 — High, blocking: the kit reaches READY without the socket-diagnostic support needed to start UDP

**Contract.** The guest image carries the socket-diagnostics support used by
the slot owner/audit (`feature-delta.md:7160`–`:7162`).
UDP sockets and first datagrams must reach the actual guest owner, not only
the source-local Sim implementation.

**Reachability and failure.** An ordinary unconnected IPv4 UDP `send_to` runs
the production `sendmsg4` capture, claims a slot and redirects its datagram.
`KernelGuestOwnerIo::poll_once` marks that slot's readiness reported
(`kernel_io.rs:475`). `GuestFlowOwner::start_datagram` calls `slot_owner`
(`owner.rs:758`), which calls `diagnostic_source_port` (`kernel_io.rs:975`).
`diagnostic_sockets` requests the TCP and UDP inet-diag dumps and returns a
netlink error at `:1280`–`:1287`. The kit's image/module loader does not supply
`inet_diag`, `tcp_diag` or `udp_diag` (`guest_kit.rs:483`,
`guest_peer.rs:691`). On the supported stock kernel those dump handlers are
not available. The real reply is `NLMSG_ERROR`, payload `-2` (`ENOENT`).
`start_datagram` returns without an association, while the seam has suppressed
that slot's readiness. Its owner is not published, so the later owner audit
also has no owner cookie to repair. The owner remains alive; no flow request
is sent. This is not shutdown or an injected read error.

**Reproduction and necessity contrast.** With only F1's existing optional
loopback fixture prerequisite enabled, the production PID-1 guest reaches
READY. Its first four-byte UDP send succeeds, then receives no reply;
`HOST_REQUESTS=0`, `INITIAL_UDP_FAILURE at=0`, guest `EXIT 2`.
`native-udp-diagnostic.log` also records the actual module inventory and
independent TCP/UDP inet-diag requests, both returning `NLMSG_ERROR(-ENOENT)`.
In a fresh image, staging and loading the same stock kernel's three
diagnostic modules before the first send makes all 1,024 initial socket
round trips work (`native-udp-recycle.log`, `ALL_1024_SLOTS_ECHOED`). The
production init, owner, BPF and forwarder code are unchanged in this contrast.

**Required disposition.** Supply the diagnostic capability already required
by the accepted guest-image contract for the supported stock-kernel kit.
Close this proven prerequisite omission within the existing kit/setup
boundary. This finding does not authorize a new diagnostic recovery
subsystem, a runtime capability gate, a new error variant, or generalized
transient-I/O hardening.

**Status:** OPEN; no reviewer remediation.

### F3 — Critical, blocking: `sock_release` returns a UDP slot before its old association has been aborted, discarded and disconnected

**Contract.** The release order is abort pair (or close unpaired V_g),
discard S's queue, disconnect S, then return the slot
(`feature-delta.md:7106`, ADR-0201). The current approved model keeps
`kernelSlots` unchanged in `SockClose` and returns it only in the owner's
release action (`udp_slots.qnt:379`, `:400`). The kernel claim and owner
claim must refer to the same owner while the owner holds a slot.

**Reachability and exact ordering.** After an ordinary application UDP
socket closes, production `guest_capture::sock_release` deletes its
`SLOT_OWNER`, clears `SLOT_META` and pushes the index onto `FREE_SLOTS`
**before** publishing the release report (`guest_capture.rs:219`–`:233`).
The still-running guest owner has not handled that report or its forwarder
end record yet; its resident S is still connected and its old association is
still attached. A new ordinary application socket immediately sends. When
the 1,024-slot pool is otherwise full, `alloc_slot` necessarily pops that
prematurely returned index and writes the new cookie/destination (`:104`).
The new packet is directed to the old slot lifecycle. The legitimate owner
release subsequently runs `end_slot_association` (`owner.rs:800`), aborts the
old pair, waits for its end, and eventually discards/disconnects S and calls
`release_kernel_slot` (`owner.rs:843`, `kernel_io.rs:670`). That method now
reads the *replacement* metadata. There is no cancellation, process restart,
raw-fd event injection or forced test-only state in this sequence.

**Reproduction.** The production init is PID 1, running its normal owner
thread. With the stock diagnostic support present, a guest workload opens
1,024 ordinary UDP sockets and waits for one exact echo on each, establishing
that every slot and guest datagram pair works. It then performs 32 bounded
close/new-socket/first-send cycles with the other slots held. The independent
host peer performs only the existing ControlOpen/Paired exchange and canonical
OVD1 frame echo over the real vhost-vsock connection; no host-kernel forwarder
or host capture map is substituted for the guest under review.

The final reproduction deliberately permits the contract's correct
`EPERM` exhaustion result while cleanup is still pending. It counts a failure
only when `send_to` **succeeds** and that admitted datagram is lost. Current
source admits and loses 32/32 first datagrams, with zero EPERM refusals.
All 32 later retries echo correctly. The host sees 1,056 requests: 1,024
initial associations plus 32 replacements, with the 32 admitted first
replacement datagrams missing. `native-udp-admitted-loss.log` records guest
`EXIT 3` and all individual observations.

The necessity contrast uses the same unmodified product and same full pool,
adding only a 100 ms workload wait after each old socket closes. It reports
`RECYCLE_RESULT lost=0/32`, guest `EXIT 0`, and the same 1,056 host requests
(`native-udp-recycle-wait-control.log`). There is one four-byte packet in
flight per socket exchange; this is not a UDP receive-queue overload or a
performance claim. The claim is the designed release-order violation and the
loss of already-admitted data, not a requirement that a still-full pool must
accept a new socket immediately.

This is a real BPF/socket adapter effect. The correct Sim/model release
ordering cannot expose the production hook's premature map/queue mutation;
no invented control-plane schedule or new Sim seam is needed to reproduce it.

**Required disposition.** Preserve the accepted owner-controlled release
order and ownership. A slot must not enter the free pool or acquire a new
kernel owner before its previous association's abort/discard/disconnect has
completed. Use the existing owner/seam lifecycle; this finding does not
authorize another ownership table, generation protocol, persistence boundary,
new lane or model change.

**Status:** OPEN; no reviewer remediation.

### F4 — High, blocking verification gap: the current candidate cell does not exercise all of the accepted native cases

This finding is about the explicit test/evidence acceptance criterion, not
an allegation that the candidate program is broken. No hypothetical kernel
or control-plane defect is promoted into an implementation requirement.

The current 11-06 criterion (`roadmap.json:928`, third criterion) and the
TCP-CORRELATION contract require the existing parameterized cell to prove
live direct Fast Open `CandidateRejected → Rejected`, terminal monotonicity
with established sends, preterminal reset/delete finality, and zero candidate
invocations on established data.

The checked-in executable table is exactly `DirectUncorrelated`, `Ordinary`,
`ImmediateApplicationClose`, `ForcedSyncookie`, `DataInSynFastOpen`
(`guest_flow/real_kernel.rs:749`). `DirectUncorrelated` calls ordinary
`connect`, not MSG_FASTOPEN (`:852`). The only Fast Open runner
(`:779`–`:841`) exercises a valid producer, the shared pending quota and
recovery. Its pending producer is not reset before validation; it sends no
parallel established-data cohort while checking terminal/deleted state.
The cell observes child-map **cardinality**, not candidate/terminal
transitions, and contains no candidate invocation-count observation. The
12 wrapper cases only invoke these bodies. The existing native spike's
different diagnostic programs are useful design evidence, but do not turn
these absent assertions into checks of the committed production composition.

**Required disposition.** Reconcile and complete the already-approved
existing-cell correction and its evidence through DISTILL. Do not add a
cell, lane, model, production accessor, or fresh performance/ARM claim. A
passing 12-test summary must not be described as proving these absent
oracles. This requirement is already in the accepted step, rather than new
coverage invented by this review.

**Status:** OPEN; no reviewer test-body change.

## Formal correspondence and test integrity

Commit `9d7463e8bf` was included in the review. Independent recomputation
matched all **24/24** `summary.json.inputs` SHA-256 values to current files.
All **279/279** recorded checks have `pass = true`; outcomes consist of 209
holds and 70 expected hazard violations. The retained global
`quint-verify-evidence-metal-final.log` verifies guest-flow-owner, probe-count
and tcp-capture-pending. No redundant model record was run.

The owner-flow edits factor existing guards and replace disabled-action
`.fail()` runs with explicit disabled-guard expectations. The UDP model
separates the kernel claim from the owner's later SlotReady publication and
retains release ownership until the release action. These changes preserve
the reviewed invariant rather than permitting F3. The production-owner
projections, Sim socket input/output and closed/reset facts, record-consumed
observation and applied-VMM-exit boundary were inspected. The green formal
record is not evidence that the production BPF hook implements that model;
F3 demonstrates the missing correspondence at the actual kernel boundary.

The default source-local owner group contains 59 tests (session 10,
TcpConnect 9, TcpAccept 15, flow end 9, UDP 13, newtypes 3), and transport has
14. These represent distinct accepted outcomes/error classes, with the 97
named/randomized conformance entries counted separately, not as additional
unit-behavior duplicates. Each owner/transport test has its Contract Shape
declaration; live pure-function properties retain the exact required rustdoc
line. No banned test name was found. The supplied declaration-checker script
is absent in this installation; declarations were inspected directly rather
than claiming that unavailable CLI ran. Legacy untouched tests elsewhere and
neutral compiler-only struct-literal updates were not turned into a new
documentation migration.

The test-body changes were compared with the approved DISTILL correction
record, including the fixed-record stimulus order, separated consumed-record
and owner-close observations, real SimFlowForwarder HalfEnded refusal,
AppClaim/GuestReady projection alignment, current route assertions, executable
build-tool fixtures and the native R5-28 positive-control ordering. They were
not rejected merely because their approved edits share the implementation
commit. The original reset/state/end assertions precede the new PAUSED
positive-control handshake. No acceptance body was edited by this reviewer.

The bounded native reproductions are Tier-3 investigation material, not EDD
expectations, permanent tests or benchmarks. They run the production guest
entry point and real guest-kernel effects through the existing GuestKit and
production VMM confinement. There is no Rust expectation runner, spawned host
`overdrive` CLI, new permanent test lane or production Sim workaround.

## Verification evidence and mechanical gates

| Evidence | Actual result and limit |
|---|---|
| `11-06/init-nextest-metal-final-serial.log` | 187/187 passed, 3 later-step tests skipped; 496.725 s. Earlier similarly named failing logs are retained history, not current GREEN. |
| `11-06/in-guest-lane-final-poll-fix-metal.log` | 12/12 passed, zero skipped; 66.078 s. No UDP capture/release cell; optional loopback enabled. |
| `11-06/quint-fwd-pair-stimulus-handlers-with-toolchain-metal.log` | Final corrected driver: 38/38, 152 filtered/skipped tests; 33.907 s. |
| `11-06/workspace-check-metal-final-clean.log` | Workspace/all-target integration-feature check passed. |
| `11-06/workspace-clippy-metal-final-approved.log` | Workspace/all-target integration-feature Clippy with warnings denied passed. |
| `11-06/rustfmt-metal-final-approved.log` | Current format check passed. |
| `11-06/dst-lint-metal-after-ff.log` | Corrected dst-lint passed; the older `dst-lint-metal-final.log` is a superseded failure. |
| `11-06/bpf-clippy-metal-final-native.log`, `conformance-lint-metal-final-approved.log` | Guest BPF Clippy and conformance lint passed. |
| `11-06/quint-verify-evidence-metal-final.log` | Three subsystems verified; independently checked 24 input hashes and 279 expected outcomes. |
| [native-default-kit.log](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/evidence/native-default-kit.log) | Reproduced F1, no READY, guest powers off; host reproduction exits 4. |
| [native-udp-diagnostic.log](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/evidence/native-udp-diagnostic.log) | Reproduced F2; READY then first admitted UDP datagram stalls, no flow request, inet-diag returns ENOENT, guest EXIT 2. |
| [native-udp-admitted-loss.log](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/evidence/native-udp-admitted-loss.log) | Final F3 oracle accepts legitimate EPERM refusal; actual 32/32 admitted first datagrams lost, 32/32 retries echo, guest EXIT 3. |
| [native-udp-recycle-wait-control.log](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/evidence/native-udp-recycle-wait-control.log) | Same product/pool with workload wait: zero losses, EXIT 0. |
| [artifact-identity-cleanup.log](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/evidence/artifact-identity-cleanup.log) | Native artifact identity retained; the completed review host process is absent. |

The native artifact identity receipt records:

| Artifact | SHA-256 |
|---|---|
| Production static `overdrive-init` | `e76704f5246793eeaecfec704489162c26ed818549fba6c870730c04a1f06efa` |
| Rust guest BPF object | `06d93a651df42484639044c6cf302805afcba4d07694dd509300c9980aa6851c` |
| Candidate C/CO-RE object | `84c8d7730fe591eb50ff98504e7ab0e3c3da027b034d44393bff7e6023d89094` |

The retained reproduction sources, isolated Cargo manifest, exact commands and
case runner are in
[`spike-scratch/netns-density-295-review-11-06/increment-a/`](../../../../spike-scratch/netns-density-295-review-11-06/increment-a/README.md).
All ten original raw logs are byte-identical in its `evidence/` directory; the
retention manifest records every original file and SHA-256. The original final
host source is also preserved in `source-at-capture/host.rs`; the runnable host
source changes only console-output path plumbing for the relocation.
The guest is compiled with `rustc --target x86_64-unknown-linux-musl`; the
host runner imports the existing GuestKit. Commands use `cargo xtask metal
run --` and the canonical Metal lease. The host arguments are: no optional
flags for F1; `with-loopback` for F2; `with-loopback with-diag` for F3; and
the additional `after-release-wait` for its positive contrast. The final
guest oracle accepts `EPERM` while a full pool is still being released and
only rejects successful sends whose admitted data disappears. Initial
scratch compile and host-process nofile mistakes were harness authoring
failures and were not classified as product failures.

Every module, BPF/map, socket-diagnostic and routing effect under investigation
ran **inside disposable Cloud Hypervisor guests**, not in the physical
host's module state or shared Lima. Each observed guest ended by normal
poweroff, or by the fixture's scoped cleanup on failure. No product source changed
between the native failing/control cases. No timing distribution, performance
receipt, new ARM coverage or native UDP coverage for the original 12-test
lane is claimed.

The final effective DES entries for this step are RED EXECUTED/PASS at
`2026-10-23T21:13:09Z`, GREEN EXECUTED/PASS at
`2026-10-24T08:43:00Z`, and COMMIT EXECUTED/PASS at
`2026-10-24T08:43:43Z`. Earlier malformed/intermediate RED outcomes remain
append-only history. The genuine 11-04 lint-remediation events in the shared
log were preserved and are not claimed as this reviewer's or step 11-06's
phases. Implementation, corrected DES and formal commits preserve Marcus as
author and carry the exact Codex co-author and Step-Id trailers. Unrelated
dirty AGENTS and the separate research/lint commits are excluded.

## Gate disposition and iteration 1 verdict

| Gate | Disposition |
|---|---|
| Semantic RED → GREEN → COMMIT and attribution/scope mechanics | PASS for the recorded implementation cycle. |
| Production composition, real adapters, no fixture implementation of guest behavior | PASS for the exercised boundaries; F1–F3 are reproduced failures of that real composition. |
| Contract Shape declarations, domain naming and bounded test count | PASS for this step's activated/changed acceptance group. |
| Existing suite results and formal record | Retained and assessed as passing at their stated boundaries. |
| Exact guest setup/error/invariant compliance | FAIL — F1, F2, F3. |
| Complete approved candidate-cell verification | FAIL — F4. |
| Mutation testing | Not run. The user explicitly forbids mutation testing anywhere in this workflow; no final mutation gate is due. |
| Additional architecture, generalized recovery, permanent tests or lane expansion | Not authorized or implemented by this review. |

**NEEDS_REVISION.** F1–F3 have bounded native reproductions and necessity
contrasts; F4 is an unmet, already-approved verification obligation. Return
implementation remediation to this step's original crafter and existing-cell
acceptance work to DISTILL. Re-review this same step and append the exact
remediation/evidence dispositions here. Do not start 12-01 until this review
returns **APPROVED**. The separate final full-slice Sol review is still due.


## Compliance follow-on before iteration 2

The reviewer explicitly read `.claude/rules/spike.md`. Its retained-source and
raw-evidence requirements take precedence over generic review guidance. The
former ignored probe directory was moved into the retained standalone increment
linked above. Every one of its 13 original files is preserved, including all ten
failure/control raw logs; `evidence/retention-manifest.json` records and verifies
the original byte lengths and hashes. The new runner keeps future captures in
separate timestamped directories. The original capture commands remain verbatim
in `evidence/commands-at-capture.txt`. No runtime rerun was needed for relocation,
and no finding, original oracle, wire exchange, guest setup or product source
was changed.

F1/F2/F3 remain deviations from the already-approved implementation/prerequisite
contract, not DESIGN falsification or authorization for new architecture. The
original crafter may remediate F1 and F3 through the existing approved seams
only. This review requires no new public probe API, slot-ownership mechanism or
model. If the implementation audit uncovers a genuine interface-contract gap,
stop that affected path and return its exact shape for DESIGN rather than
inventing a method, parameter or ownership protocol. DISTILL owns F2's existing
kit/test-support correction and F4's existing approved cell obligation. The
same reviewer will re-review after their completed handoffs.

## Iteration 2 — replacement independent review

| Field | Value |
|---|---|
| Reviewer | Fresh, isolated GPT-6 Sol DELIVER reviewer, explicitly authorized by the user for the same step and criteria |
| Reviewed committed chain | Original `2f92c925d374a21f41f74e34bd9f58767d53429e`; formal `9d7463e8bf1c8bb89926dff8b9394cd68a9ad12f`; remediation `13b6a2d4d5b522af718474361b25688709a09cca`; DES `7a8f86763c4ec807aa85cf3014372fdd5f2b59e7` |
| Date / iteration | 2026-10-24 / 2 |
| Verdict on committed tree | **NEEDS_REVISION** — F1–F4 closed; new proven F5 remains on this committed tree |
| Subsequent local work | Original crafter's private `kernel_io.rs` change and C/D captures are in progress and are **not** counted as committed remediation in this verdict |

Two attempted turns by the original GPT-6.1 Sol reviewer and one fresh
GPT-6.1 Sol replacement turn failed before a verdict. They are neither review
iterations nor approvals. The user then authorized this GPT-6 Sol replacement
without changing acceptance criteria. The iteration-1 findings and their raw
evidence remain part of this artifact. I inspected the actual changed
production, fixture, test and CI sources against the approved roadmap,
feature delta and relevant ADRs, the retained native captures, the independent
DISTILL iteration-22 record and the precise guest owner path. The user's
unrelated dirty `AGENTS.md` and the original crafter's current production and
DES edits were preserved. No production, test, model or DES source was edited
by this reviewer; no mutation test was run.

### F1–F4 remediation dispositions

| Finding | Disposition and direct evidence |
|---|---|
| F1, extra guest forwarder probe | **CLOSED.** `KernelGuestTransportOps::open_forwarder` now opens the process-owned context and performs only DeviceOpen and Version (`transport/kernel.rs:388–400`). Its private ioctl and descriptor checks retain the existing `ForwarderError::Probe { stage, source }` representation (`:48–103`); the host's full `FlowForwarder::probe` is no longer called by guest setup. The default workload image, without the optional vsock loopback module, reaches READY in `increment-b/evidence/default-kit-f1-f3-post-fix.log`. This is the existing guest setup contract, not a new public probe method or knob. |
| F2, stock socket-diagnostic prerequisites | **CLOSED.** `GuestKit` stages the stock kernel's `inet_diag`, `tcp_diag` and `udp_diag` modules (`guest_kit.rs:91–95,490–496`), and the existing PID-1 guest fixture loader loads staged prerequisites before it execs production init (`guest_peer.rs:668–718`). With this default image the real owner receives exact echoes for all 1,024 initial UDP slots; the earlier `NLMSG_ERROR(-ENOENT)` startup failure is absent (`increment-b/evidence/default-kit-f1-f3-post-fix.log`). The source still uses the approved `io::Result` diagnostic seam; there is no new public recovery/error interface. |
| F3, premature kernel slot reuse | **CLOSED for the original admitted-loss trace.** `sock_release` now retains `SLOT_OWNER`, `SLOT_META` and the free-pool claim and reports the released cookie (`guest_capture.rs:214–232`). The owner releases after abort/end, discard and disconnect (`owner.rs:798–865`; `kernel_io.rs:684–711,1055–1069`). The default-kit native retry workload observes 1,024 initial echoes, 32 legitimate `EPERM` refusals while slots remain owned, 32 later echoes, zero admitted replacement losses, 1,056 host requests and guest EXIT 0 (`increment-b/evidence/default-kit-f1-f3-post-fix.log`). The C/D trace below exposes a distinct publication gap in this same accepted lifecycle; F3 closure alone cannot approve the step. |
| F4, incomplete existing native correlation cell | **CLOSED.** The same parameterized cell now includes live direct `MSG_FASTOPEN` rejection, real candidate-program invocation observations, a sender cohort with a completed send while the child is Pending that continues across gate release and Ready, exact terminal metadata, post-terminal zero invocations, one-time deletion, preterminal reset/delete finality and active/child-map cleanup (`guest_flow/real_kernel.rs:948–1359`). The quota Fast Open target remains separate and sends no post-handshake payload before capture. The independent DISTILL review records iteration 22 **APPROVED** with zero open F2/F4 findings, and the existing native wrapper passed 1/1 in `i21-f01-correlation-overlap-native.log`. The cell uses the production adapter and running BPF program/map observations, not a fixture counter or new public accessor. |

The accepted typed-error set remains as mapped in iteration 1. In the changed
paths, a guest DeviceOpen or Version refusal remains
`GuestTransportSetupError::Forwarder { source: ForwarderError::Probe { stage,
source } }`, with Version's ABI mismatch or descriptor failure retained as the
typed source. Missing, mismatched and other module-load failures still map to
`ForwarderModuleMissing`, `ForwarderModuleRelease` and
`ForwarderModuleLoad`, respectively; staging a matching diagnostic module
adds no owner error variant. The C/CO-RE candidate still maps object/BTF/load
failure to `ProgramLoad { program: TcpCorrelationCandidate, source: EbpfError }`
and attach failure to `ProgramAttach { program: TcpCorrelationCandidate,
source: ProgramError }`; malformed/missing correlation remains private
Rejected with reset and no flow/control. UDP pool exhaustion is the designed
capture refusal `EPERM`, and a diagnostic read error remains the existing
`io::Result` path. F5 is the non-error `Ok(None)` source-port case at that seam;
its missing cleanup requires no new typed error or public interface.

The bpftool packaging correction is also within the approved build setup.
CI and Lima test `apt-cache policy bpftool` for a concrete candidate; on Noble's
virtual package they install `linux-tools-common` and `linux-tools-generic`,
copy the actual executable under `/usr/lib/linux-tools-*/bpftool` to
`/usr/local/bin/bpftool`, check its version, then dump the declared readable
target BTF. The existing dev-setup probe now verifies an actual dump. The
retained isolated Ubuntu 24.04 Noble proof records the virtual-package result,
the packaged generic executable, `bpftool v7.4.0`, and a successful dump from
Metal's read-only BTF with an Azure runner release simulated. It is **local
Noble validation**, not a GitHub-hosted CI rerun; the branch has no upstream,
open PR or Actions run. No new public configuration was added.

The committed remediation has RED, GREEN and COMMIT entries recorded through
DES and the two remediation commits retain Marcus as author with exactly the
required Codex co-author and Step-Id trailers. The supplied post-remediation
Metal checks report 187/187 package tests with three later-step skips, 12/12
native guest-lane tests, the focused correlation wrapper 1/1, workspace
check/Clippy/format, BPF Clippy, focused xtask 4/4 and unchanged formal
279/279 over 24 matching input hashes. These are valid evidence at their
respective boundaries; the existing 12 native wrappers contain no C/D
immediate-release case. No new permanent test, lane, model or expectation
runner is authorized or requested by this review.

### F5 — High, blocking: an application closed before slot-owner publication can leave a kernel UDP claim permanently unavailable

**Accepted contract and reachability.** A released application's UDP slots are
returned after the owner discards and disconnects their resident slot sockets;
the one-second level-triggered audit recovers a lost release report
(`feature-delta.md:7106`; ADR-0201). Production ordinary unconnected UDP
`send_to` enters `sendmsg4` and `alloc_slot`, which pops `FREE_SLOTS` and
inserts the cookie/destination in `SLOT_META` and `SLOT_OWNER`
(`guest_capture.rs:99–112,130–153`). Immediate socket close enters
`sock_release`, which correctly retains that kernel claim while reporting the
cookie (`:214–232`). The production PID-1 guest owner polls the resident slot,
marks readiness reported and queues `GuestIoEvent::SlotReady`
(`kernel_io.rs:422–435,489–495`). If close preceded its diagnostic lookup,
`diagnostic_source_port(cookie)` returns `None` from the live socket dump
(`:1249–1251`); on the **committed** `13b6` source, `slot_owner` returned
`Ok(None)` without releasing the claim. `GuestFlowOwner::start_datagram` then
returned before publishing `state.owner` (`owner.rs:758–769`). The queued
`SocketReleased` handler selects only published owners (`:282–291`), as does
`audit_slots` (`:879–894`); either ordering leaves this unpublished claim out
of both cleanup paths. The process, control session and slot pool remain alive;
there is no forced abort, canceled future or test-only owner state.

**Native reproduction and necessity contrast.** The retained, standalone C
workload drives the default production guest kit, actual PID-1 init, its BPF
capture and the stock 7.0.0-38-generic kernel inside a disposable Cloud
Hypervisor VM. It performs 1,100 ordinary send-and-close operations: 1,024
sends admitted, 76 legitimate `EPERM` refusals, no prior application sockets
left open. After three one-second audit periods a fresh held socket is still
refused `EPERM`; `HOST_REQUESTS=0`, guest EXIT 2. The exact raw capture is
`increment-c/evidence/reruns/default-kit-20261024T120620Z-2078/runner.raw.log`.
The D control uses the same product, kit, peer, 1,100 sockets and audit wait,
but waits for each exact echo before close, ensuring publication. It records
1,100 echoes, `HOST_REQUESTS=1101`, a further post-audit echo and guest EXIT 0
in `increment-d/evidence/reruns/default-kit-20261024T120950Z-5892/runner.raw.log`.
The claim is **post-audit pool exhaustion with zero live prior application
sockets**, not delivery of fire-and-forget UDP payloads or immediate admission
while cleanup is pending. The C/D source, runner, raw logs and guest consoles
are retained under `spike-scratch/netns-density-295-review-11-06/`; they are
not yet committed and must remain preserved with the original crafter's
remediation record per `.claude/rules/spike.md`.

**Required disposition.** Close the unpublished-claim path through the
already-approved resident-slot and kernel-map lifecycle. The original crafter
currently has a private `slot_owner` change that drains/disconnects the
unassociated resident slot and releases its claim when the diagnostic lookup
returns `None`; its post-change C run records 1,032 admitted sends, 68 valid
`EPERM` refusals while busy, a post-audit exact echo and EXIT 0, and unchanged
D again records 1,100 echoes plus a post-audit echo and EXIT 0. Those captures
support the proposed private HOW but are **not** a committed review closure:
the original crafter still owns final verification, retention and DES commit.
This finding does not prescribe or authorize a new owner table, generation,
retry, persistence protocol, Sim seam, public method, test lane or changed
model. The original UDP release invariant remains intact.

### Iteration 2 gate and verdict

| Gate | Disposition |
|---|---|
| Exact public API, typed failures and accepted ownership | PASS for F1–F4 remediation; F5 uses the existing private cleanup seam. |
| Native F1–F4 and correlation evidence | PASS at the stated case boundaries. |
| CI `bpftool` packaging | PASS for the local Noble package/binary/BTF proof; hosted CI has not run. |
| F5 original committed source | **FAIL** — reproducible retained 1,024-slot exhaustion after all application sockets closed and three audits. |
| Existing tests, Contract Shape and formal record | PASS as reported above; they do not exercise F5's immediate-release ordering. |
| Mutation testing | Not run; the user forbids it anywhere in this workflow. |

**NEEDS_REVISION.** F1–F4 are closed, but F5 is reproduced through the real
production entry point and remains in the current committed tree. The original
crafter's private fix, C/D evidence and checks must be committed before this
same reviewer can record iteration 3. Step 12-01 stays held. The separate final
full-slice Sol review remains due.

## Iteration 3 — F5 remediation review

| Field | Value |
|---|---|
| Reviewer | Same GPT-6 Sol DELIVER reviewer as iteration 2 |
| Date / iteration | 2026-10-24 / 3 |
| Production and retained-evidence commit | `6e8d2df862fdf73a049d217f3c6aa52b3db822e8` |
| Initial DES follow-on | `094305325398680394e2f32ed7b76c2387f12d84` |
| Append-only exact DES outcome correction / reviewed HEAD | `10ff04669aaec5a375073134faf6f9da12b838cc` |
| Verdict | **APPROVED** — F1–F5 closed, zero open findings |

### F5 disposition and contract fit

**CLOSED.** The sole production edit is in
`KernelGuestOwnerIo::slot_owner` (`kernel_io.rs:968–1007`). It first reads an
existing nonzero cookie and destination from the kernel's `SLOT_META`. If the
successful inet-diag lookup finds no live application socket with that cookie,
it calls the existing `end_association(slot)`, then `release_slot(slot)`, and
returns `Ok(None)` (`:992–1000`). `end_association` removes any slot-pair and
readiness bookkeeping, discards the resident socket's receive queue, then
disconnects it (`:1055–1065`). `release_slot` reads that slot's exact kernel
metadata, removes its `(cookie, destination)` key from `SLOT_OWNER`, clears
`SLOT_META`, and returns its index to `FREE_SLOTS` (`:684–711,1067–1069`).
This is the approved no-pair release order at the existing guest-owner I/O
boundary. It does not make the `sock_release` hook free a slot early or let a
new kernel claim replace an unfinished association.

The path is constrained by actual production ordering. `poll_once` emits a
`SlotReady` only for a resident slot without a `slot_pairs` entry and not
already marked readiness-reported (`:422–435,489–495`). `start_datagram`
calls `slot_owner` **before** it publishes the current claim in `state.owner`,
creates a guest flow or starts the forwarder pairing (`owner.rs:758–784`). The
F5 immediate-close claim therefore has no active guest flow or pair to abort;
an older owner value may remain after a previous association, but its pair and
association have already ended before readiness is re-enabled. A live
diagnostic cookie still follows the unchanged connect-and-return path
(`kernel_io.rs:1002–1007`),
so normal paired and later datagram reuse remain owner-controlled. An absent
kernel claim (`raw_cookie == 0`) still returns `Ok(None)` without trying to
free it; a netlink/map/read failure still propagates through `io::Result`
because the diagnostic call retains `?`. No error is reclassified as socket
absence, no typed error distinction is hidden in a string, and no new public
method, enum variant, map, table, retry or configuration key was introduced.
F1–F4's production paths and the Noble `bpftool` workflow are untouched by
this commit.

The retained [C probe](../../../../spike-scratch/netns-density-295-review-11-06/increment-c/README.md)
and [D control](../../../../spike-scratch/netns-density-295-review-11-06/increment-d/README.md)
keep their source, commands, raw runner output, guest consoles, exit statuses
and ownership receipts under `spike-scratch`, now committed. C on the original
committed adapter has 1,024 admitted sends, 76 valid busy-pool `EPERM`
refusals, no live prior application sockets, and a fresh `EPERM` after three
audits (guest EXIT 2, zero host requests). The identical C workload with the
private correction records 1,032 admitted sends, 68 valid temporary refusals,
and an exact post-audit echo (guest EXIT 0, one host request). D waits for each
of 1,100 exact echoes before closing and passes before and after the change,
including the post-audit echo (guest EXIT 0, 1,101 host requests). The exact
captures are the C `default-kit-20261024T120620Z-2078` and
`default-kit-20261024T121819Z-14810` `runner.raw.log` files, and the D
`default-kit-20261024T120950Z-5892` and
`default-kit-20261024T121945Z-16107` `runner.raw.log` files. The C/D
[analysis and hashes](../../../../spike-scratch/netns-density-295-review-11-06/increment-c/evidence/f3-immediate-release-analysis.md)
identify the sources and raw captures. Both variants drive the real production
PID-1 init through the default GuestKit and the actual guest BPF on disposable
Cloud Hypervisor guests with Linux 7.0.0-38-generic. The post-fix probe was
run on the uncommitted candidate whose production diff is the exact private
diff in `6e8d2df`; there is no intervening production source change.

The follow-on Metal runs after this correction report 187/187 default
`overdrive-init` package tests with three later-step skips
(`init-nextest-increment-c-remediation-metal.log`), 12/12 native guest-lane
tests with zero skips (`in-guest-lane-increment-c-remediation-metal.log`),
and successful `overdrive-init` all-target Clippy, formatting and production
diff checks. The formal inputs and 279-check record are unchanged from the
24/24 hash-matched review in iteration 1. No permanent C/D test, lane,
model change, expectation runner, mutation run or hosted-CI claim was added.
The raw console captures retain their original carriage returns as evidence;
the production Rust diff and this Markdown artifact have no whitespace errors.

The original crafter committed the private source fix and all 37 retained C/D
paths in `6e8d2df` without unrelated source or test-body changes. Marcus
remains author and the commit carries exactly the required Codex co-author
and Step-Id trailers. The latest effective step 11-06 DES events are
`RED EXECUTED/PASS` at `12:17:09Z`, `GREEN EXECUTED/PASS` at `12:32:29Z`,
and `COMMIT EXECUTED/PASS` at `12:32:30Z` on 2026-10-24. The earlier prose
outcome events remain append-only history; `10ff04669` adds only the exact
GREEN/COMMIT outcomes through the CLI. The pre-existing dirty `AGENTS.md`
remains outside the reviewed commits.

### Iteration 3 gate and verdict

| Gate | Disposition |
|---|---|
| Original F1–F4, exact API and typed error mappings | **PASS**, closures from iteration 2 unaffected. |
| F5 reachability, necessity contrast and cleanup order | **PASS**, native before/control/after captures and source path agree. |
| Retained probe integrity and source scope | **PASS**, self-contained C/D sources and raw captures committed; only the private adapter method changed. |
| Native, source-local, format and Clippy gates | **PASS** at their stated boundaries; no new behavior change after the runs. |
| DES phase and attribution gate | **PASS** after append-only exact-outcome correction at HEAD `10ff04669`. |
| Mutation testing | Not run; the user's prohibition applies to the whole workflow. |

**APPROVED.** Step 11-06 now satisfies the accepted guest transport, owner,
UDP release and TCP correlation criteria at the reviewed boundaries. There
are zero open DELIVER review findings. Step 12-01 may start after the
orchestrator's mechanical acceptance of this artifact. The separate final
full-slice Sol review remains due.
