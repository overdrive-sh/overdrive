# STEP 12-01 DISTILL alignment

## Scope and authority

This alignment is limited to existing acceptance-test surface made obsolete by
the approved attachment-cut error reconciliation. The authoritative contract
is the exact post-cut taxonomy in `feature-delta.md` under **Typed error
taxonomy and who branches**, together with the independently approved DESIGN
review in `review-design-12-01-error-variants.md` iteration 2.

Outside the later, explicitly approved formal/test-runner alignments catalogued
below, no scenario, test body, fixture seam, public API, model action, lane, or
accepted assertion was added or weakened. Production implementation and DES
events remain owned by the STEP 12-01 crafter.

The final bounded decisions carry seven explicit user approvals, without
implying roadmap validation or any broader surface:

| Decision | User authority | Exact approved DISTILL boundary |
|---|---|---|
| `FLOWQUOTA240` | “Approve 240-second exception” | Only the randomized quota cell's 60-second period × 4 override; all eight full-capacity traces and bodies unchanged. |
| `BFIFO1` | “i approve the rest of the decisions.” | The beacon model's exact FIFO `bq`, existing-action effects/safety, and removal of the driver's private accept-order shadow. |
| `PABORTERR1` | same | `HostRecvAbort` fixes the pre-Paired payload-free peer abort to existing `Unreachable:ECONNRESET`, with the existing run asserting it. |
| `ACTPRIORITY1` | same | `ActivatePending` is enabled only when the Provisioned CID is not quiesced and has no live session. |
| `ADMISSIONREFUSAL1` | same | The existing `HostAcceptFlow` quiesced branch drives and verifies the real `Refused { Quiesced }` with zero admitted state/place; production reader ownership/cap/fd work remains with the crafter. |
| `QUIESCENCEOPEN1` | same | `OpenFlow` additionally requires the existing C1 `FlowListeners` damage fact to be false. |
| `CHCGROUP55RUNNER1` | same | Three existing-lane catalog entries and one `cgroup-default` group for exactly 55 unchanged bodies in disposable guests. |

These are the canonical approvals at `feature-delta.md:751-752`. Formal
replacement evidence, exact replays, the 55-body guest run and independent
source/evidence review remain required; the current roadmap remains pending.

## Existing-test alignment

| File | Alignment | Reason |
|---|---|---|
| `crates/overdrive-control-plane/src/guest_network/owner_acceptance/errors.rs` | Removed the exhaustive `taxonomy_name` arms for `ScratchCleanupIncomplete` and `StartupProbeCleanup`. | Those variants have no post-cut producer, consumer, wire representation, or persisted representation. The function remains an exhaustive match with no wildcard over the approved exact 13-variant `GuestNetworkError` set, including the single `Forwarder { source: ForwarderError }` carrier with no operation field. |
| `crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs` | Removed the `GuestNetworkScratchComplement` / `GuestNetworkScratchCount` imports, the test-local `scratch_complement` constructor, and `scratch_complement_never_fabricates_zero`; removed the now-unused `proptest` prelude import. | The helper and property exercise only the deleted bridge/TAP/TCX/map/pin/guard scratch complement. They do not observe a retained post-cut owner, forwarder, allocation cleanup, or startup-probe outcome. |

The remaining error coverage preserves the retained contract:

- `taxonomy_name` still makes an unapproved `GuestNetworkError` variant a
  compile-time exhaustiveness failure.
- `a_forwarder_that_cannot_be_opened_or_probed_refuses_boot_typed` still
  requires the exact `ForwarderError` source to cross the owner boundary
  without flattening.
- The phase-1 forwarder property still requires `DeviceAbsent`, `AbiMismatch`,
  and stage-specific probe mismatches to return through
  `GuestNetworkError::Forwarder { source }`, release the probe context, and
  issue no later boot effect.
- Allocation cleanup coverage remains; only the deleted node-level scratch
  complement property leaves.

## Supplemental fail-stop alignment

The independently approved DESIGN supplement closes
`SharedGuestNetworkFailStopCause` over exactly eight post-attachment-cut unit
variants. The existing core acceptance table was aligned without adding or
rewriting a test body:

| File | Alignment | Preserved coverage |
|---|---|---|
| `crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs` | Removed `TapQuiescenceUndetermined` and `VmKillFailed` from `CAUSES`; changed its array length and compile-time length assertion from 10 to 8; changed the generated `FailStop` index domain from `0_u8..10` to `0_u8..8`; replaced the superseded D-295-R14 kill-scope prose with the exact post-cut taxonomy. | `every_fail_stop_cause_is_closed_and_first_request_wins` still crosses every retained cause with every retained shared component. `generated_operation_sequences_match_the_gate_model` still generates every retained cause and checks its receipt, terminal transition, waiter refusal, and first-request-wins behavior. No assertion changed. |
| `crates/overdrive-testing/src/pids_max.rs` | Removed stale references to per-TAP and supervisor kill paths from the E23 test-helper documentation. | The helper still produces the real cgroup-v2 `pids.max` refusal needed by the retained allocation-scoped VM-lifecycle kill evidence. |
| `crates/overdrive-worker/tests/integration/real_cgroup_fs/refused_thread_kill.rs` | Clarified that E23 drives retained `CgroupManager::cgroup_kill` for one allocation, not the deleted supervisor `VmKillCapability`. | The existing body, Contract Shape, pending marker, real-kernel boundary, and typed `io::Error` / no-panic oracle are unchanged. |

The two removed causes had no post-cut producer or branch consumer. Their
deletion therefore removes no executable outcome from the cause cross-product.
The retained eight are, in declaration order:
`RecoveryDeadlineExceeded`, `LeakedQuiescenceHold`,
`ForeignQuiescenceHold`, `SupervisorReturned`, `SupervisorFailed`,
`SupervisorPanicked`, `SupervisorCancelled`, and `RequestChannelClosed`.

The supplied-owner signature change has bounded compiler fallout rather than
a DISTILL test-design change. Existing control-plane integration and
simulation callers of `run_server_with_obs_and_driver(s)` pass a neutral
`CgroupManager` solely for the deleted `vm_cgroups` parameter. STEP 12-01 must
remove that argument and the matching imports while preserving each caller's
driver, owner, EXEC wiring, boot path, and assertions. The production
`CgroupManager::cgroup_kill` and `CgroupPath::for_alloc` uses in VM start,
stop, artifact disposal, and boot reclamation remain outside that fallout.
No caller requires a replacement parameter, fixture seam, or compatibility
overload.

The E23 pending body remains necessary compiler and real-kernel evidence for
the retained VM-lifecycle path. It must not be deleted with the obsolete
supervisor capability. Its pre-fix panic/abort behavior is separate from the
semantic RED receipt for the shared-network attachment cut; the smallest
semantic ordering RED remains the already-authored owner-composition test
below.

## Beacon private-fixture migration

The accepted step-6 beacon contract replaces the private Unix split halves
with the existing `BeaconIo` port's move-only `BeaconSocket`. The production
crafter pinned the corresponding private implementation shape: the one
`BeaconWriter` actor owns `Arc<dyn BeaconIo> + BeaconSocket`, returns its framed
line receiver from `spawn`, and `run_exit_watcher` / `drain_guest_report`
consume that receiver. DISTILL aligned only existing private test support in
`crates/overdrive-worker/src/vm_driver.rs`:

- Removed the test module's direct `UnixStream` socketpair construction.
- Changed private `test_beacon_writer` to bind the existing `SimBeaconIo`,
  queue and guest-close one simulated connection, accept its move-only
  `BeaconSocket` through `BeaconIo`, and call the production writer's agreed
  `spawn(io, socket, vmm, control)` shape. The helper returns the existing
  `Arc` / `Weak` identity pair plus the writer's line receiver.
- `dropped_gate_sender_releases_watcher_without_stranding` now passes that
  line receiver to `run_exit_watcher`. The simulated guest close produces the
  same immediate no-report outcome as the former closed Unix peer, so the
  existing five-second strand detector, matching-session claim, exit-event,
  allocation identity, and `Crashed { exit_code: None, signal: None }` oracles
  are unchanged.
- The direct watcher fixture also passes a fresh private
  `Arc<Mutex<BeaconState::default()>>`, matching the production watcher's
  accepted state-owner input. The fixture has no live launch or claim to
  remove, so this preserves its original gate-release and exit-classification
  universe without manufacturing beacon state.
- Existing `test_beacon_writer` callers in
  `claim_guard_requires_the_originating_live_session_and_preserves_other_entries`
  await the private helper and retain its receiver. Every originating-session,
  replacement-session, Starting, pre-beacon, EndingInFlight, and absent-entry
  assertion is unchanged.
- The helper's new async boundary exposed five `parking_lot::MutexGuard`
  lifetimes that Clippy conservatively carried across a later helper await.
  Each existing map assertion now has its own lexical block and explicitly
  drops its guard immediately after the final assertion, so the guard is both
  tightly scoped and destroyed before the next `test_beacon_writer().await`.
  The assertion order, map contents, beacon ownership drops, and session-claim
  oracles are unchanged; no lint allowance suppresses either ownership check.

This is compiler-required private fixture migration over an already-approved
port. It adds no public API, adapter seam, scenario, test body, or observable
outcome. The fixture intentionally depends on the crafter's coordinated
private writer/watcher signatures and will compile once those production
changes land in the same STEP 12-01 worktree.

## Beacon-bind typed-refusal alignment

The independently approved DG-4 composition mapping adds the exact
`VmmBootError::BeaconBind { source: BeaconIntakeError }` variant. The existing
`a_failed_beacon_bind_refuses_boot` integration cell already drives the
production `run_server` composition with `SimBeaconIo::fail_next_bind`,
requires one `health.startup.refused` reason `vm_driver.beacon_bind`, proves
nothing was bound, and proves boot did not continue to the shared-table write.

That existing body now also requires the refusal to be
`ControlPlaneError::VmmBoot(VmmBootError::BeaconBind { source:
BeaconIntakeError::Bind { port, source } })`, with the exact configured
`VsockListenPort` and injected `AddrInUse` `io::ErrorKind`. This preserves the
typed port and source across both approved error boundaries instead of
accepting any boot error with the same diagnostic event. The body's existing
bind-cleanup, no-later-write, and exactly-one-refusal assertions remain
unchanged. Its `refused_boot` helper already rejects an accidentally published
`ServerHandle`, so the same body still proves the boot server is not returned.

No exhaustive `VmmBootError` match or other existing fixture requires an
alignment: the current matches select individual variants, and the existing
HTTP conversion matches the enclosing `ControlPlaneError::VmmBoot` wrapper.
The Optional-versus-Required rule is production composition behavior over the
same approved typed cause; no second existing DG-4 body exists, and this
bounded alignment does not manufacture one or add a capability seam.

## `mtls_inbound_guest_relay` execution boundary

The existing R5-54 Rust body cannot currently be executed inside a disposable
Cloud Hypervisor guest by an existing checked-in command. Its documented
command runs the `overdrive-dataplane` integration binary directly in the
Metal test process, where the loopback kTLS `TCP_ULP` operations affect the
physical host kernel.

The approved GuestKit components contain the needed lower-level pieces, but
they are not composed for this binary:

- `cargo xtask guest-kit-prepare` stages only the `overdrive-init` unit-test
  binary and the `overdrive-fwd` `integration` test binary. It does not build
  or stage `overdrive-dataplane --test integration` at
  `target/guest-kit/in-guest/overdrive-dataplane--integration`.
- `run_scripted_in_guest_cell` can stage and execute an arbitrary prepared
  `InGuestTests` binary and can load the stock `tls.ko`, but it is a Rust
  fixture function, not a command. No existing host-side body calls it for
  `overdrive-dataplane` or for the `mtls_inbound_guest_relay` cells.
- `overdrive-fwd-guest-suite` is not a generic executor. It hard-codes
  `InGuestTests::prepared("overdrive-fwd", "integration")`, fixed cell names,
  isolation groups and expected pass counts. Replacing its staged binary
  would select absent `overdrive-fwd` test names or violate its result-count
  oracle.

Therefore there is no honest existing command/path that both leaves the
authored test binary unchanged and confines its kernel effects to a disposable
guest. Merely compiling or copying the binary supplies no checked-in host
launcher, while the current `nextest` command executes it on the physical
host. Closing this boundary required adding the dataplane target to the
prepared artifacts and a group to the existing runner. The user explicitly
approved that bounded reuse; the implementation below changes no test body.

### Approved minimum extension

The smallest extension reuses the existing lane setup, `GuestKit`, scripted
guest, module loader, CID band and `overdrive-fwd-guest-suite` executable. It
adds no scenario, test body, public API, runner binary, CID band or execution
framework. The exact implemented changes are limited to two existing files:

1. In `xtask/src/guest_kit_prepare.rs`, one staging descriptor was appended to
   `IN_GUEST_LANES`:

   ```rust
   InGuestLane {
       package: "overdrive-dataplane",
       bin: "integration",
       target: InGuestTarget::Test,
       features: "integration-tests",
   }
   ```

   This builds the existing integration libtest binary unchanged and copies
   it to the already-defined contract path
   `target/guest-kit/in-guest/overdrive-dataplane--integration`. No
   `kvm-tests` feature is named because `overdrive-dataplane` declares only
   `integration-tests`, and that feature already enables the existing
   `tests/integration.rs` target. This is one additional artifact prepared by
   the existing GuestKit setup command, not a new test lane.

2. In `crates/overdrive-testing/bin/fwd_guest_suite.rs`, one exact group
   function was added with the existing dispatch signature:

   ```rust
   fn run_mtls_relay(
       kit: &GuestKit,
       _fwd_tests: &InGuestTests,
   ) -> Result<(), GuestKitError>
   ```

   The function obtains
   `InGuestTests::prepared("overdrive-dataplane", "integration")`, creates a
   temporary module work directory, stages the running guest kernel's
   `net/tls/tls` module at
   `/lib/overdrive-testing/modules/tls.ko` through the existing
   `host_module_file`, and calls the existing `GuestRun::boot` with name
   `"mtls-relay"`, `FWD_GUEST_BOUNDARY.raw(13)`, that one extra module and
   the module's guest path. Slot 13 is unused in the runner and remains inside
   its existing 16-slot registered CID band; no registry edit is needed.

   It then calls the existing executor with exactly:

   ```rust
   guest.exec(
       &[
           "--test-threads=1",
           "--nocapture",
           "integration::mtls_inbound_guest_relay::",
       ],
       7,
       CELL_BOUND,
   )?;
   ```

   The source currently contains exactly seven live tests under that module.
   The module-qualified filter excludes every other integration module, and
   the existing summary oracle requires `7 passed; 0 failed`. Add
   `("mtls-relay", run_mtls_relay)` is in the existing group table and the
   selector is named in that runner's module documentation. No generic
   package, binary, filter or expected-count arguments were added.

The existing image path copies the unchanged libtest binary with
`with_shared_libraries: true`, which stages its ELF interpreter and `ldd`
library closure. The test mints its PKI in-process and needs no guest file or
tool beyond that closure. The scripted guest already mounts `/proc`, `/sys`
and `/dev` and raises loopback before commands; its existing
`run-with-modules` path loads `vsock_loopback.ko` and then the staged stock
`tls.ko` before `exec`. Therefore every `TCP_ULP` attach, kTLS socket,
loopback connection and reset exists only in the disposable guest kernel.

The concrete invocation is:

```text
cargo xtask metal run -- cargo xtask guest-kit-prepare
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=mtls-relay cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

`GuestRun::finish` asks the scripted peer to power off, waits for Cloud
Hypervisor to exit, and the existing `GuestKitVm` drop remains the kill/reap
fallback. Existing `WorkDir` drops remove the image, run and decompressed
module staging directories. The physical Metal host supplies the guest
kernel/module files and launches the VMM; it never opens the test's loopback
kTLS sockets or sets `TCP_ULP`.

### Guest-relay test crypto-provider alignment

The first approved guest run reached the unchanged seven-test binary, then all
seven cells stopped before their contract oracles because Rustls found two
compiled default crypto providers. The workspace's direct Rustls dependency
already disables defaults and selects `ring`; the dataplane's dev-only `ktls`
dependency had retained ktls 6's default `aws_lc_rs` feature, which enabled
AWS-LC beside `ring` in the same test process.

The bounded test-build correction selects the existing workspace provider:

- the workspace dependency is now
  `ktls = { version = "6", default-features = false }`; and
- `overdrive-dataplane`'s existing dev-dependency is now
  `ktls = { workspace = true, default-features = false, features = ["ring"] }`.

The workspace-level `default-features = false` is necessary: a member cannot
disable defaults inherited from a workspace dependency whose defaults remain
enabled. `ktls` has exactly the relevant features (`default = ["aws_lc_rs",
"tls12"]`, `ring = ["rustls/ring", "tokio-rustls/ring"]`), and it is used by
no other workspace manifest. `cargo tree -p overdrive-dataplane --features
integration-tests -e features` now shows only `ktls feature "ring"`,
`rustls feature "ring"`, and `tokio-rustls feature "ring"`; no AWS-LC provider
feature appears in that package graph.

This changes no dependency version, adds no dependency, and changes no
production Rustls or runtime provider choice. It only makes the existing
dev-only kTLS peer use the same `ring` provider already selected by the
workspace so the unchanged GuestKit cells can reach their kTLS assertions.

## `VipRange` property alignment

The independently approved `VipRange` supplement pins
`first_overlapping_range(&self, other: Ipv4Net) -> Option<Ipv4Net>` as the
smallest cross-crate query for the existing D8-PREFIX-VIP invariant. DISTILL
aligned the existing P4-S14 multi-range property in
`crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs`:

- The property derives the first configured CIDR, in the generated vector's
  operator order, that intersects `GUEST_NODE_PREFIX`.
- Through both existing phase-1 entry points, an overlap must now return the
  exact `GuestNetworkError::GuestPrefixOverlapsVipRange { prefix:
  GUEST_NODE_PREFIX, vip_range: expected }` payload. A later overlapping CIDR
  cannot replace an earlier one.
- The existing disjoint pass, no-boot-effect refusal, phase-1 witness, and
  valid-`VipRange` precondition assertions remain unchanged. Reserved VIP
  addresses are additionally generated inside configured ranges and passed
  through both entry points; they do not affect the expected overlap or typed
  payload because the approved query is over configured CIDRs themselves.

The existing `#[ignore = "pending DELIVER step 13"]` marker remains exactly in
place. STEP 12-01 adds the approved `VipRange` query needed for compilation;
step 13 lands and activates the startup refusal behavior. This property is
therefore deferred step-13 evidence and is not a STEP 12-01 semantic RED
receipt. No scenario, property body, strategy, fixture seam, or lane was added.

## `ProbeForwarder` fixture auto-delivery

The post-cut owner startup path now calls the already-approved
`GuestFlowIo::probe_forwarder` operation. `SimGuestFlowIo` deliberately holds
async effects unless a fixture opts into automatic release; its global
defaults remain unchanged so stepped conformance drivers can still control
individual effects. Five existing owner-composition fixtures now opt in to
the expected phase-1 effect:

- `owner_acceptance/support.rs::Composition::with_prefix`, shared by the
  owner acceptance cells and Quint drivers;
- `owner_acceptance/legs_dial.rs::compose`, which constructs that same
  composition directly; and
- `prefix_boot_acceptance.rs::Serve::start`, which drives both phase-1 entry
  points;
- `shared_network_test_ports.rs::ComposedNode::boot`, which drives the
  production boot-seam phase before composing the owner and retains its
  queued exact forwarder-failure script; and
- `action_shim/guest_activation_acceptance.rs::ShimFixture::build`, which
  awaits an expected successful owner startup probe before reaching the
  existing activation journey.

Each calls `io.auto_release(SimEffectKind::ProbeForwarder)` immediately after
constructing its fresh seam. This is fixture scripting for an expected effect,
not a change to `SimGuestFlowIo::new`. Existing
`script_failure(ProbeForwarder, failure)` queues still supply the exact typed
failure when the effect is auto-released, so the failure cases retain their
`ForwarderError` source assertions. Pass paths now complete instead of parking,
while the issued-effect ordering, context-release, no-later-boot-effect, and
positive phase-1 witness assertions remain unchanged. No production default,
test body, seam, API, or model action was added.

## Sim forwarder half-close projection

The crafter's focused owner-acceptance run reproduced a test-support model
defect at seed 61. After `guest_sends(flow, b"abc")` and
`guest_half_closes(flow)`, all bytes reached the destination, exactly one
direction recorded `source_end = Clean`, that direction recorded
`target_shut = true`, the opposite direction remained open, the pair remained
live, and no `ForwardEnd` had been queued or consumed. Despite that live state,
`SimSocket::peer_end()` reported `Some(Eof)`.

`overdrive-sim` is the repository's test binding, and
`SimFlowForwarder::target_half_close` was projecting the one-direction
`SHUT_WR` twice: correctly as `sent_shutdown = Send`, and incorrectly as the
terminal whole-socket fact `peer_end = Eof`. The bounded correction removes
only the second projection. It preserves:

- byte movement before the target shutdown;
- `source_end = Clean` and `target_shut = true` for that direction;
- the live pair and open opposite direction after one half-close;
- `ForwardOutcome::Completed` only after both source directions end cleanly;
- `SHUT_WR` / `0x2` delivery in both directions before full completion; and
- the production owner's later descriptor close, which remains the operation
  that projects terminal `FarEndEvent::Eof` after the completed end record.

The two existing `overdrive-sim` acceptance assertions that previously treated
a directional shutdown as terminal EOF now require `peer_end == None`: once
after the first half-close, and once after the standalone forwarder has emitted
the completed record but before any owner closes the descriptors. Their byte,
direction-state, completed-record, inventory, unhook, and trailing-end
assertions are unchanged. No new test, body, socket state, adapter seam, API,
or production mechanism was introduced, and reset/full-close behavior remains
owned by the existing abort and owner-close paths.

## Pre-attach destination-cause lookup

Two existing owner cells exercise the accepted R5-45(d/e) window after the
non-mesh `ConnectPeer` completes and before `attach`:

- seed 66 resets the destination and requires attach refusal plus the existing
  abort projection; and
- seed 68 sends bytes and FIN, then requires the attach-time run to deliver the
  bytes and carry the half-close without aborting.

The peer socket already exists in `SimState.handles` throughout that window,
but `SimGuestFlowIo::peer_of(flow)` previously consulted only
`flow_sockets[flow].peer`, which is populated at attach. Both existing cause
helpers therefore panicked before applying their stimulus.

The existing private lookup now preserves both phases. It returns the attached
peer when present. Before attach, after confirming that the request already
established the flow's vsock entry, it requires the held `ConnectPeer` effect
to have been released and resolves the sole unattached peer from the existing
handle table. This covers the intentional boundary where the effect's oneshot
has been released but the owner task has not yet been polled to store the peer
address. It refuses to choose when zero or more than one unattached peer exists,
so parallel in-flight flows cannot silently receive one another's cause. No map,
correlation field, effect,
method, handle, port, or test seam was added.

The two existing bodies, held-connect ordering, `destination_*` stimuli,
attach-time state classification, byte-ordering, typed abort, and clean
half-close assertions remain unchanged. This completes the semantics already
claimed by the existing `peer_of(flow)` test-support API for an unambiguous
single in-flight pre-attach peer.

## Unclassified attached-vsock end

Seed 65 exercises the existing broken-invariant stimulus
`vsock_ends_unclassified(flow)`: an attached V_h enters
`SimVsockState::Closing` with no recorded shutdown class and an empty receive
queue. The accepted EVMOD-END contract requires one
`ForwardOutcome::Aborted { cause: Unclassified }`, release by the owner, one
`guest_flow.unclassified_end` alarm, and one `unclassified_eof` inventory
increment. `SimFlowForwarder::source_abort` previously recognized only socket
errors and an explicitly recorded abort, so the pair remained live.

The adapter-sim classification now returns `AbortCause::Unclassified { side }`
only when the source is a vsock stream in `Closing`, no class was recorded,
and its receive queue is empty. Socket errors and recorded aborts retain their
existing precedence; recorded clean ends still take the clean path; queued
bytes are still forwarded before any unclassified EOF can be classified.
`set_vsock_state` now schedules the already-installed socket hook after the
state transition, exactly as the other attached-socket stimuli do; a
pre-attach socket has no hook and retains its existing attach-refusal path.
After accepting the final queued byte, the existing direction run now returns
`Again` when the source is still the classless `Closing` state; the next run
reaches `source_abort` and emits the existing `Unclassified` outcome. This
adds no timer or event and never classifies before all queued bytes are
accepted.
Existing end-record, owner-release, peer-reset, warning-field, and inventory
assertions are unchanged. The correction uses the already-approved state and
error variant and adds no API, seam, scenario, test body, or mechanism.

## Phase-1 and serving-context observation

Seed 61 of the existing forwarder-recovery cell observed three rows in the
device's context table where its stale count expected two. This is not a Sim
leak. QPROJ-8 defines `forwarder_contexts()` as every context the seam's device
opened, in open order, including released rows. Phase 1 intentionally opens,
probes, and releases a context of its own before boot opens the serving
context. The separate phase-1 witness correctly requires exactly that one
released empty row immediately after phase 1.

The recovery cell runs later over the same `SimForwarderDevice`. Its complete
honest table is therefore:

1. released, empty phase-1 probe context;
2. released, empty original serving context; and
3. open, initially empty replacement serving context.

The existing assertion now requires those three rows explicitly. It does not
filter the probe row, clear history, create another device, or count it in the
owner's serving `forwarder_generation`; that owner observation still advances
only from the original serving context to its replacement. The existing
abort-all → close-old → open-new ordering, stream termination, generation
increment, clean post-repair audit, restore, and new-dial assertions remain
unchanged. This aligns a pre-authored count oracle with the approved
observation contract and preserves both the phase-1 and recovery witnesses
without weakening leak detection.

## VMM-exit callback ordering

Seed 60 exercises the existing EVMOD-STREAM-END schedule where a live
`GuestStream` observes the V_h reset while its watch record is still in flight,
then the orphan-sweep callback queues that record and resolves the read as
`ConnectionReset`. `vm_stop_before_callbacks` correctly set `sk_err` on the
host vsocks, but it also auto-released the live control connection's
`peer_gone` read. During the intervening `settle`, the owner observed session
loss and aborted the watch first, changing the test to the separate valid
owner-abort schedule.

The split Sim lifecycle now keeps accepted-connection loss unreleased in
`vm_stop_before_callbacks`. That method represents only the first sweep step:
socket error/state is readable, while callbacks and accepted-connection loss
remain independently orderable. `finish_sweep` still reports each socket error
and runs the existing forwarder callbacks. The convenience `vm_stop` path,
which performs both steps without a driver-controlled gap, restores the prior
auto-delivery behavior for accepted connection loss after the sweep. Existing
Quint drivers retain their explicit `release_peer_gone` control.

The seed-60 body and its in-flight-record assertion are unchanged. No new
state, method, event, API, seam, or schedule mechanism was added; the existing
`peer_gone_released` bit and release operation now reflect the two already
authored sweep phases honestly.

## Multi-recovery EXEC-gate property model

The existing generated gate property still modeled the pre-cut single
recovery snapshot. The approved QPROJ-6 / update-table row 12 contract permits
`begin_recovery(component)` to add a distinct component while the gate is
already Recovering, and `recovery_progress()` returns every active component
in `Ord` order.

The property model now mirrors that exact transition without adding a test:

- Recovering state holds an ordered component-to-attempt map, the set that
  joined since the previous completed attempt, and the deterministic latest
  component used by the existing fail-stop receipt.
- Generated `CompleteAttempt` operations use a nine-bit mask, exercising every
  possible `BTreeSet<SharedGuestNetworkComponent>` rather than zero or one
  component only.
- A distinct `begin_recovery` while Recovering adds attempts `0`, marks the
  component joined for the current cycle, and makes it latest; a duplicate
  begin remains refused.
- `complete_attempt` removes tracked components absent from `remaining`,
  increments retained components except those that joined during the cycle,
  never invents a recovery for an untracked member of `remaining`, clears the
  joined set, chooses the first retained component in `Ord` as latest, and
  reopens only when no tracked component remains.
- The property compares the complete ordered `recovery_progress()` vector and
  every attempt count after each generated operation. Fail-stop still checks
  the exact cause, first-request-wins behavior, and the selected latest
  component's attempts; claims remain closed for every non-Open state.

The existing illegal-transition cell now recognizes adding `Dns` during a
`Forwarder` recovery as legal and checks that re-adding the already tracked
`Forwarder` is refused. BootClosed, duplicate-open, complete-outside-recovery,
and post-fail-stop refusals remain unchanged. No API, fixture, seam, strategy
family, scenario, or production state was introduced by DISTILL.

## Netlink route-interface fixture alignment

The independently approved netlink supplement replaces the control-plane's
private `HostRouteType` duplicate with `overdrive_netlink::FibRouteType` and
changes `GuestFlowIo::route_type` to preserve `NetlinkError` directly. The
existing D26 and R5-11 acceptance surfaces already express the required
observable boundary, so no new scenario or body is required:

- `tcp_flows_to_host_internal_or_excluded_destinations_are_refused_by_class`
  already drives local, broadcast, anycast and static refusal classes; requires
  `Refused(HostInternal)`; proves the already-accepted guest record socket is
  closed; preserves the CID/session and unrelated-flow universe; and requires
  zero `ConnectPeer` effects for every refusal.
- The existing datagram refusal cells likewise require ordinary guest-slot
  cleanup and no retained association/quota place.
- Existing boot route cells preserve the exact `LocalRouteConverge` operation,
  route-before-listener ordering, idempotent keep/replace/add outcome and
  failed-write non-effect. Their assertions do not depend on the removed
  private route-type enum.

The bounded compiler/test-fixture fallout is mechanical:

- owner policy, datagram, activation, shared-recovery and Quint fixtures now
  script `FibRouteType` values directly;
- `SimGuestFlowIo` stores and returns `FibRouteType`, defaulting to `Unicast`;
  and
- its existing `RouteType` effect now returns the trait's typed
  `NetlinkError`: an injected I/O failure remains a typed `Connect` failure,
  while the existing netlink-failure stimulus represents the approved
  source-less `FibLookupMissing { destination }` outcome.

No refusal assertion, cleanup expectation, effect count, marker, scenario,
adapter seam, or failure-injection surface changed. Production ownership of
`Client::converge_ipv4_host_local_route`, `Client::ipv4_fib_route_type`, the
host adapter mapping and the owner branch remains with the crafter. DISTILL
found no further existing-test gap and did not implement the separate pending
netlink work beyond its independently approved contract.

## QHOLD supplied-owner fixture alignment

The independently approved QHOLD supplement adds the exact required
`#[must_use] fn leaked_quiescence_holds(&self) -> u32` method to
`SharedGuestNetworkOwner`, with no default. The method reads the cumulative
leaked count of the same owner ledger that issues and redeems that owner's
holds. The production trait declaration, production host-owner implementation,
and audit-tick fail-stop branch remain the crafter's implementation ownership.

DISTILL aligned the existing test and simulation implementations required by
the trait's no-default compiler gate:

- `TestSharedOwner` returns `self.ledger.leaked()`. This is the exact ledger
  already used by its `quiesce` and `restore` methods and by the existing
  R5-32 `LeakHold` stimulus.
- `SimSharedGuestNetworkOwner` returns
  `self.quiescence.ledger.leaked()`, matching its existing inherent
  observation and its hold issue/redeem path.
- `ActivationFaultOwner`, the mTLS journal owner,
  `RecordingGuestNetwork`, the activation-order owner, the retiring-cleanup
  journal owner, `LeaseLedger`, and `RecordingOwner` delegate to their one
  existing inner owner. They do not create a ledger, cache a value, or derive
  the count from their journals.

The existing R5-32 external-owner leak cell is unchanged. Its composed
`TestSharedOwner` still issues and drops the exact hold, observes holds `2`
and leaked `1`, restores the listener recovery to holds `1`, proves admission
remains `Quiesced`, and requires the next audit tick to request the existing
typed `LeakedQuiescenceHold` fail-stop. The new trait dispatch now exposes
that same external owner's ledger to the supervisor, preserving the original
leak witness without adding a scenario, body, constant, fixture seam, or
alternative source of truth.

The approved supplied-owner entrypoint now takes the five exact inputs
`(config, boot, driver, shared_guest_network, guest_network_exec)`; the retired
supervisor `vm_cgroups` parameter is absent. The three remaining existing
fixture callers (`netns_density_boot_order`, `vm_reclamation`, and the direct
conformance harness) no longer construct or pass a trailing `CgroupManager`.
Their boot bundles still carry the existing simulated cgroup/VM-host adapters,
so VM lifecycle and reclamation evidence is unchanged. Imports used solely by
the deleted argument were removed, and the conformance composition comment now
states the current signature.

A subsequent all-target pass found five more neutral callers in
`vm_lifecycle_latency_283_spike.rs`; each now passes the same five exact
arguments and no longer constructs a trailing test `CgroupManager`. The file's
`SimCgroupFs` remains live in its `NodeAdapters::host` boot bundle, preserving
all VM-lifecycle latency stimuli and oracles. A repository-wide caller audit
then found no remaining six-argument invocation. The same compiler pass left
one unused `provision_workload_netns` import in
`alloc_netns_lifecycle.rs`; only that import was removed, with its retained
netns plan, provisioning-double and teardown bodies unchanged.

## Sim adapter compiler fallout for retired VM attachment state

The accepted attachment cut removes `VmTapPlan` from
`WorkloadNetworkProvisioner::provision` and removes the early R18
`intercept_mark_guard` observation field. Three existing simulation support
sites were aligned to those exact interfaces:

- `NoopNetworkProvisioner` and `SimPartialFailureNetwork` now implement the
  one-plan `provision(&WorkloadNetnsPlan)` method and no longer import or
  accept `VmTapPlan`.
- The provision-failure invariant's logical artifact universe retains the
  allocation-owned namespace, host/workload veths, resolver directory and
  transit route. Its obsolete guest TAP, guest route and return-route entries
  were removed, and its non-empty proper-subset bound changed from eight to
  five. The seeded subset selection, typed failure, exact structural teardown,
  zero-residue assertion, failure-before-cleanup ordering and smallest-free
  slot reuse remain unchanged.
- `SimMtlsIntercept::state_snapshot` no longer initializes the removed
  `intercept_mark_guard`; the retained program, policy-route and dynamic-member
  projections are unchanged.

These are existing test/simulation adapter compiler fallouts only. They add no
fixture action, production API, scenario or assertion over a retired
mechanism.

## Remaining attachment-cut fixture fallout

The next compiler and harness pass exposed four further existing-support
alignments. Each stays within an already-authored body or removes a body whose
only subject was deleted by the cut:

- `action_shim/guest_activation_acceptance.rs` no longer imports or passes
  `VmTapPlan`; its recording exec/process netns provisioner implements the
  retained one-plan `WorkloadNetworkProvisioner::provision` signature. Its
  call count and all guest-activation assertions are unchanged.
- The final source-local module in `veth_provisioner.rs` retained its existing
  `assigned_workload_address_inverse_accepts_only_exact_second_usable_hosts`
  body and the assertions over the workload netns plan. Assertions and four
  properties whose only subjects were `VmTapPlan`, persistent TAP identity,
  the guest-side `/30`, VM NIC MAC, or TAP convergence steps were deleted with
  those production types. The retained inverse still covers representative
  slots including both boundaries and rejects the netns network, host address
  and addresses outside the assigned-workload domain.
- The obsolete `guest_tcx_egress_lifecycle` and `guest_tcx_inventory`
  integration modules and their two declarations were deleted. Both imported
  the removed `overdrive_dataplane::guest_tcx` classifier and exclusively
  defended its TCX programs, TAP endpoints, pins, maps, counters and
  identities. Other dataplane integration modules remain declared unchanged,
  including the unchanged `mtls_inbound_guest_relay` binary now staged by the
  approved GuestKit extension.
- `shared_network_test_ports.rs` dropped only the now-unused `PathBuf` and
  `SimCgroupFs` imports left by supplied-owner fixture migration.

The existing `overdrive-init` owner-thread cell still requires the exact
`GuestOwnerFatal::EventWait` display prefix and now expects the standard
`io::ErrorKind::Interrupted` display text `operation interrupted`; the prior
`Interrupted` literal was its debug spelling. Its start-before-READY,
no-control-session and fatal-handler oracles are unchanged.

The companion READY-port failure is not a stale test oracle. The core
`BeaconMessage::Ready` contract says `port` echoes the port the guest dialed,
and ADR-0217 requires `overdrive-init` to connect only to the token's beacon
port. Therefore the existing expectation `port=50234` for a token carrying
`beacon=50234` remains unchanged. The production helper currently hard-codes
the default `1234`; the crafter accepted that as production fallout and owns
changing `ready_message` to `token.ports.beacon`.

Five Sim failures shared one fixture omission: their external `AppState::new`
compositions skipped the production boot step that converges the owner before
the private guest-flow-port `OnceLock` is captured. The activation-order and
retiring-cleanup fixture composers, plus the service-kind VM terminal fixture,
now converge their one existing owner before constructing `AppState`. This
covers the three activation-order cases and the one case in each other file.
The crafter owns the fixture-convenience constructor's internal capture of an
already-converged owner's `flow_ports()` into its existing private `OnceLock`;
production continues to use `new_with_workflow_engine` and its post-step-9
single set. No setter, parameter, public field, test seam or synthetic port was
added, and the five behavioral bodies and assertions are unchanged.

### Broad test-target compiler fallout

The combined worker/control-plane/core gate then compiled test targets that
the focused owner and Sim runs had not built. DISTILL aligned the remaining
existing support to the same approved deletions:

- Twenty control-plane fixture provisioner implementations and three
  additional Sim spike provisioners now implement the one-argument
  `WorkloadNetworkProvisioner::provision(&WorkloadNetnsPlan)` interface. Their
  existing call counts, typed injected failures, teardown journals and
  lifecycle assertions remain unchanged. The one fixture that retained the
  plan in a journal now records `WorkloadNetnsPlan` alone.
- The VM-TAP-only C3 restart-convergence and incompatible-link scenarios were
  removed from `alloc_netns_lifecycle.rs`, together with their TAP observation
  helpers and the wrapper cell in `veth_provision_idempotent.rs`. The retained
  structural workload-netns failure and `FinalizeFailed` gate bodies remain.
- `terminal_netns_cleanup_from_observed_addr.rs` retains its exact prior-row
  address recovery and teardown assertion for the workload-netns transit
  address. Its second row derived solely from deleted `VmTapPlan::guest_addr`
  and was removed.
- The mTLS activation-refusal fixture still injects the retained typed
  `PostconditionMismatch { operation: ForwardingActivate, .. }`; its obsolete
  link-up fact now uses the retained, activation-relevant
  `ControlSession { alloc, cid, live: true }` expected fact and the same fact
  with `live: false` observed. Its durable failure-stage and cleanup oracles
  are unchanged.
- Existing worker/control-plane `InterceptState` doubles now project only the
  retained program, policy route and dynamic members. The test-local R18
  `intercept_mark_guard` bit, initialization and successful-converge write
  were removed.
- Three test files dropped unused `PathBuf` imports left by the compiler
  fallout.

Repository search now finds none of `VmTapPlan`, `derive_vm_tap_plan`,
`vm_tap_converge_steps`, `ObservedVmTap`, `VmTapStep`,
`GuestNetworkFact::LinkUp`, or `intercept_mark_guard` in Rust sources.
Targeted rustfmt and diff checks pass. These edits do not change a retained
production interface or add a compatibility shim.

### Retired netlink TAP/bridge test inventory

The workspace all-target check later reached the netlink crate after its
approved production API deletion and found only existing tests for those
removed mechanisms. DISTILL removed the exact obsolete defenders:

- `client.rs` no longer imports the deleted TAP/link-identity decoder types or
  retains the two pure tests for persistent-TAP classification and complete
  TAP/bridge identity projection. Its post-cut local-route classifier and all
  fwmark/table rule tests remain unchanged.
- `ethtool.rs` no longer retains the debug-message-mask UAPI-constant test.
  The TX-checksum command pin, payload property, target selection, active-state
  observation, endian readers and nested-attribute tests remain unchanged.
  Its retained payload parser keeps a test-local UAPI tag
  `ETHTOOL_A_BITSET_BIT_VALUE = 3` solely to assert that the FEATURES_SET
  encoder emits no such attribute and therefore requests every named bit off;
  the removed TAP debug API/decoder surface is not restored.
- The real-kernel integration entrypoint and its four modules
  (`bridge_guard_lifecycle`, `managed_link_address`, `scratch_tap`, and
  `tap_debug_msg_mask`) were deleted. Every cell in those files constructs or
  observes the removed bridge, persistent TAP, bridge nft guard or TAP debug
  mask; none exercises the retained route/rule/FIB or TX-offload APIs.

No compatibility shim or removed API was restored. Repository search under
`overdrive-netlink` now finds no live Rust reference to the deleted TAP/link
identity types, bridge guard, persistent-TAP creation/observation, or ethtool
debug-mask surface, while topology-neutral unit coverage remains in place.

### Remaining Sim harness outcomes

Seed `25717` in the service-kind VM terminal invariant failed before its
`Running` oracle because its fixture constructed the existing shared mTLS
worker but never started the worker's node-shared listener owner. Production
boot starts that owner before allocation dispatch, and `start_alloc` correctly
fails closed without it. The fixture now calls the existing
`start_shared_owner()` before `AppState` construction and the first tick,
beside its already-required guest-owner convergence. The `Running` assertion
is unchanged.

Seed `186054989670514689` of the retiring-cleanup invariant passed every
retire, no-reuse-while-held, cleanup, release and exact-once assertion. Its
last assertion nevertheless required the first allocation after release to
receive the predecessor's address immediately. That contradicts the accepted
M-9 next-fit contract: `assign` starts at the cursor, advances the cursor past
each selected address, and `release` frees an offset without rewinding the
cursor. In the reproduced schedule `.2` was released after `.2` and `.3` had
been handed out, so `.4` is the exact next-fit result.

The existing final oracle now derives the cursor's next address from the last
pre-next-start `Provision` entry and requires both the new lease and the
driver's injected transport assignment to carry that address. It still proves
that the predecessor stays unavailable while Retiring, is released only after
successful cleanup, and becomes eligible when next-fit wraps. No pool behavior
or release ordering changed.

### R5-54 wrong-CA client observation

The latest disposable-guest run reaches all seven unchanged R5-54 cells; six
pass. In the remaining wrong-CA cell, the accepted server-side outcomes all
pass: `HostMtlsEnforcement::enforce` returns the typed
`PeerVerificationFailed`, and the guest dial and leg-S event journals both
stay empty. The sole failure is the additional `!join(client)` assertion.

That boolean is not a valid TLS 1.3 full-refusal oracle at the point it is
sampled. The test client's blocking `Client::connect` can finish after it
processes the server Finished and sends its own final flight; the server then
processes that flight, rejects the rogue client certificate, and closes. The
reproduced production path returns the typed refusal without flushing a queued
TLS alert. The accepted R5-54 wording requires peer-verification refusal to
make no dial; the existing mTLS guardrail requires the distinct server refusal
and zero bytes at leg S. Neither pins `Client::connect` itself to return `Err`
before the server has processed the client's final flight.

Independent DISTILL review iterations 18–19 approved the bounded replacement.
The same client thread now reports the result of a one-byte-bounded
`read_up_to(1)` after an initially successful `Client::connect`; it retries an
empty interrupted read. A connection error from `Client::connect` is already
a terminal refusal. The assertion requires the accumulated application
plaintext to be empty and rejects `WouldBlock`, `TimedOut`, or a terminal
`Interrupted` result; EOF and other terminal TLS/TCP errors are accepted.
The socket's existing `WAIT` read timeout bounds the observation.

The exact server-side `PeerVerificationFailed`, zero `GuestStreamDial` calls,
and zero leg-S events remain unchanged. This preserves eventual client refusal
and makes zero client-visible plaintext explicit without requiring production
to flush a TLS alert or treating the client's early local traffic-state
transition as server authentication success. No helper, seam, API, scenario,
connection mode or production path changed.

### Broad-run fixture and production dispositions

The 2,218-test combined run compiled after the prior alignment and exposed
three further categories. DISTILL changed only the two fixture/input-domain
cases that were stale against accepted contracts.

The source-local admission-refusal fixture now converges its existing
`TestSharedOwner` before `AppState::new`, so the fixture-convenience
constructor captures that owner's existing flow ports exactly as boot does.
Because convergence is a boot precondition rather than an allocation effect,
the fixture records its owner-journal baseline. Lease-event stamps subtract
that baseline, the no-effect assertion compares against it, and the restart
assertion reads calls after it. The original oracles remain exact: no
successor effect before cap refusal; predecessor retire → one teardown →
release; no successor driver start, row, occurrence or lifecycle event.

Three positive health-check parsing properties now generate only valid
listener ports: `1..61000` or `62025..=65535`, excluding the approved guest
intake reservation `61000..=62024`. Probe-target-only generators retain the
full nonzero port domain. The positional-index property still generates every
role count from its existing ranges but discards the sole combination whose
total exceeds `GUEST_PROBE_SLOTS` (3 + 3 + 3). Dedicated
`vsock_deploy_validation` properties remain unchanged and continue to require
`ListenerPortReserved` throughout the reserved range, acceptance immediately
outside it, acceptance of eight probes and typed `ProbeCountExceeded` for nine
and above. No refusal coverage moved into a positive parser property.

The six worker probe failures are production STEP 12-01 row-19 gaps, not
fixture fallout. The existing bodies already retain and script one
`SimGuestStreamDial`, require `DialPurpose::Probe`, exercise TCP and HTTP over
accepted streams, cover refusal, timeout/drop, all eight allowed probes and
zero calls to the ordinary probers for guest-prefix targets. Current
`ProbeRunner::probe_tick` still receives only TCP/HTTP probers and dispatches
every mechanic directly through them; its injected `guest_dial` remains
unused. The crafter owns the approved guest-prefix dial branch. No worker
probe body or oracle changed.

The guest-activation bodies concurrently start an allocation and open its
control session through the existing Sim seam. Their former pre-session
barrier was `SimGuestFlowIo::settled()`, a paused-Tokio 365-day sleep that
completes only when no task is runnable. After the crafter corrected
`activate_guest_network` to yield between `ControlSessionPending` retries, the
activation future correctly stayed runnable; paused time therefore could not
advance to the idle signal, so the fixture future never queued the session.

The fixture now uses a private bounded scheduler barrier over observations it
already owns. Within 256 `yield_now` polls it requires: the pool lease exists,
the recording driver has started that allocation, the owner's CID is
`Provisioned` with no session, and EXEC has not been released. Only then does
the existing body queue `ControlOpen`. The quiesced cell uses the same barrier
before and after restoring its hold, proving the lease and token remain stable.
Later `settled()` calls that observe actual Sim I/O completion remain
unchanged. This preserves the intended session-pending witness and every
zero/once/teardown/VMM-death assertion without an arbitrary sleep, timeout
increase, production API or new wait seam.

The focused four-cell rerun then passed, but the two positive cells that end
with an allocation still Active emitted the Sim seam's destructor panic: one
record handle survived until fixture drop. This is fixture cleanup, not an
owner leak: the teardown and VMM-death cells already ended their allocations,
while `exec_is_released_only_after_the_allocation_is_active` and the enabled
half of `an_allocation_without_a_guest_transport_runs_as_today` intentionally
stopped immediately after their positive assertions. Each now dispatches the
existing `StopAllocation` after its last assertion. That drives the ordinary
driver-stop, owner teardown, descriptor closes and lease release before the
Sim seam is dropped. The destructor oracle remains enabled; no panic is
suppressed, and every outcome assertion remains before cleanup unchanged.

### `quint_fwd_pair` serving-context projection baseline

A current-tree verbose replay isolated the first conformance divergence at
Trace 1, Step 0 (`InitWith { m: "watch" }`): every projected field matched
except `oldCtx`. The model expected `none`; the driver reported `released`.
The Sim device history was honest: row 0 was phase 1's released, empty probe
context and row 1 the open serving context. `FwdPairDriver::project` had treated
the second-to-last device row as the serving context's predecessor even though
the owner had not performed a modeled recovery.

The projection now uses the already-observed owner
`forwarder_generation` as the exact count of serving contexts and projects
`ctx`, `oldCtx`, queued records and generation-indexed pair facts from that
suffix of the device table. It does not discard or clear phase history. It
requires the prefix to contain exactly one phase-1 row and requires that row
to be released with zero live pairs, zero live watches and no queued end
record; an extra row or leaked resource is an explicit conformance error.

At generation 1 the serving suffix contains only the current context, so
`oldCtx = none`. After recovery it contains the released prior serving context
and the replacement, so the existing `released`/`open` projection remains.
Generation-to-row indexing inside the owner projection now addresses serving
history rather than the phase probe. No Sim state, model input, recorded trace,
production observation, API or seam changed.

A later watch trace selected the other `CtxRelease(h)` branch: with a live
watch, the host owner dies and `exit_process` closes its seam/context. The
post-step projection correctly omits owner facts, but it first called
`owner.flow_observation()` to calculate the serving-context suffix. That call
performs `GuestFlowIo::inventory()` and is illegal after process exit, so the
fixture panicked before comparing the retained kernel/socket facts.

Immediately before `exit_process`, `owner_death` now captures the owner's
actual `forwarder_generation`. Post-mortem projection uses that snapshot only
to split the retained device context table into phase-1 and serving rows; it
does not call any owner port. While the owner lives, projection continues to
read the current owner observation. The snapshot supplies no modeled outcome
or synthetic state: generation was already the QPROJ-8 source before death,
and the model makes owner death final. Context-release, quiescence, remaining
socket facts and the `owner = None` oracle are unchanged.

### `quint_fwd_pair` orphan-sweep error projection

After the serving-history correction, the same verbose replay advanced to
Trace 1, Step 1 (`VmmExit`) and isolated the next divergence. The model and
driver agreed that the host vsock remained established, had `sk_err`, and had
sent no SHUTDOWN. The driver alone projected peer-shutdown flags `{R,S}`.
`SimSocket::set_sk_err` had conflated the independent pending socket error and
received-SHUTDOWN facts by setting `received_shutdown = Both` for every vsock
error.

The accepted orphan-sweep action makes the error readable before
`sk_error_report`; it does not receive a peer SHUTDOWN. `set_sk_err` now sets
only `sk_err`. Its existing callers remain exact: `vm_stop_before_callbacks`
stores the sweep error before `finish_sweep` reports it;
`dial_vsock_error` stores and reports an I/O error; TCP, UDP and destination
socket-error callers already depend only on `sk_err`. Attach/watch refusal,
local holder reads, forwarder abort classification and `report_error` each
read `sk_err` directly, so none depends on fabricated peer bits. A real vsock
reset still calls `peer_reset` → `peer_shutdown(Both)`, preserving the actual
received-SHUTDOWN and peer-abort paths.

This is a one-field Sim adapter correction. It changes no Quint state or
trace, test body, oracle, public API or seam, and keeps the two-step VMM-exit
ordering: readable error first, callback/end record second.

The next replay step exposed an independent fixture handoff at
`HolderDrop`. In the trace `InitWith("watch") → VmmExit → HolderDrop`, the
guest had not sent `Paired`, so the driver's holder was still the spawned
production dial future and `self.stream` was `None`. The former mapping
dropped only `self.stream.take()`, making the modeled action a no-op: the
watch stayed live, no `Detached` record was queued and `V_h` stayed open.

The existing production cancellation contract already supplies the exact
pre-handoff effect. `DialCancelGuard` cancels `run_dial`; its error cleanup
aborts the watch, queues the `Detached` record and closes `V_h`. The authored
owner-acceptance cell
`a_dropped_dial_future_ends_the_dial_like_the_pairing_deadline` drives the
same existing `JoinHandle::abort` plus settle ordering. The conformance
driver's `holder_drop` now drops the returned `GuestStream` when present and,
before `Paired`, aborts its existing dial task instead.

The first alignment called `JoinHandle::abort` and immediately entered the
ordinary settle barrier. A Metal replay proved that insufficient: Tokio had
not yet polled the aborted outer task, so `DialCancelGuard` had not been
dropped and `run_dial` had received no cancellation. The driver now awaits
the aborted handle to completion (and drops its result) before settling. This
is the existing task-cancellation completion boundary, not a yield count or
sleep: it guarantees the guard has cancelled `run_dial`; settle then observes
the already-authored cleanup effects. If the task completed concurrently, its
returned stream is dropped by the same result drop. This aligns the fixture
across the dial-future → stream ownership handoff without changing production,
the model, an oracle, or any API/seam.

The first non-verbose replay after that change was reported with the same
whole-state mismatch, so DISTILL did not add another wait. A throwaway
diagnostic (removed immediately after its one run) proved the exact completed
path: the dial branch ran, its join ended `Cancelled`, and after settle the
forwarder had zero live pairs, zero live watches and one queued end. The
subsequent verbose replay confirmed the owner projection now matched exactly:
`ended`, `Detached`, queued, `nrec = 1`, with no live context generation. The
only remaining difference was the closed `V_h` socket fact: the model had
`sent = {R,S}` / `sshut = true`; the Sim socket still had neither.

That difference came from an existing Sim adapter omission, not another
cancellation step. Record-socket close and handed-off stream drop recorded a
logical `VsockShutdownOp::Close` but did not apply the close to the shared
`SimSocket` that QPROJ-8 reads. `end_handle` now calls
`record.vsock.close_from_owner(abortive)` for every owned record socket, and
`SimGuestStreamIo::drop` calls `close_from_owner(false)` on its retained
socket. This covers a pre-pair guest-opened `V_h`, a pre-pair host dial and a
handed-off stream through their existing owner-close paths. A vsock owner
close sets only its sent SHUTDOWN to `Both`; it does not fabricate received
peer bits or an error, so the preceding VMM-exit correction and the real
reset/abort paths remain independent. Existing logical close recording,
watch abort, `Detached` record, dial-attempt log and owner cleanup are
otherwise unchanged, with no new helper, state, seam, API or test body.

A later trace, `InitWith("stream") → VmmExit → TcpRst("h") →
CtxRelease("h")`, showed the pre-attach flow still `Connecting` with
`ConnectPeer` held, no sent SHUTDOWN and both forwarder contexts closed and
empty. DISTILL first attributed this to the raw `converge_shared` Forwarder
branch and handed it to DELIVER as missing production cleanup. That premise
was wrong and is explicitly withdrawn. The production entry is the runtime
supervisor: after the audit finding it calls
`acquire_shared_network_recovery_hold`, whose `owner.quiesce()` latches
admission, clears the flow tables, cancels and awaits every `flow_task`, aborts
forwarder work, drains records and closes sockets. Only then does
`run_shared_network_recovery_attempt` call `converge_shared`. No duplicate
cleanup belongs in that primitive.

The conformance driver had bypassed this accepted owner protocol by calling
`audit_shared()` and `converge_shared()` directly. It now reproduces the real
composition: the scripted audit finding is observed first; `CtxRelease(h)`
then calls `quiesce()` and retains its counted move-only hold while the repair
closes the old context and parks at the held open. `Reopen(h)` releases that
open, awaits the repair, requires a clean owner audit, and restores exactly
that hold. The hold is private driver state only and never crosses an owner or
API boundary. The existing quiesce path supplies the required pre-attach task
cancellation and record-socket close, while the bounded Sim alignment above
projects `{R,S}` and the production owner observation projects `ended` /
`byRel = true`. The model/action remains unchanged, and no production cleanup,
seam or synthetic abort was added.

With the recovery protocol aligned, that trace's only remaining difference was
`tcpErr(h)`: the model retained the `ECONNRESET` applied by its preceding
`TcpRst(h)`, while the driver projected `0` after quiescence removed the live
peer handle. Before attach the Sim seam has not yet associated the peer socket
with the flow (`flow_sockets.peer` is populated at attach); the driver's
existing pre-attach stimulus therefore selects the unambiguous live peer
socket directly. Cleanup correctly removes the owner handle, but the socket's
already-observed `sk_err` remains a terminal kernel fact.

The private driver now retains a clone of the exact `SimSocket` selected by a
pre-attach peer stimulus and reads that same socket as QPROJ-8 evidence after
cleanup. Attached flows still prefer the seam's flow-correlated peer, and a
live unattached socket remains the final fallback. No error value is copied or
synthesized: `TcpRst` mutates the retained Sim socket itself, and projection
reads its actual `sk_err`. The retained reference changes no owner lifetime,
forwarder state, cleanup or production behavior and adds no API or seam.

The conformance lint requires its arbiter exemption on the exact Sim field
read, not only above the surrounding fallback chain. The retained
`host_peer.clone()` line now carries the same QPROJ-8 justification: it is a
clone of the actual host-peer `SimSocket` chosen by the pre-attach stimulus,
whose shared socket state is the kernel-fact arbiter after owner cleanup. Its
Tier-3 twins are P-69's reset classification and P-71's destination reset
before attach. This marker does not exempt a copied value or fixture shadow;
the existing `peer_of_opt` preferred source and `latest_peer_socket` live
fallback remain unchanged. No accessor, seam, projection behavior or test was
added.

The next randomized watch trace exposes a formal handoff gap rather than a
fixture correction: `InitWith("watch") → Abort("h")`. At `InitWith`, the
production dial is still awaiting the guest's `Paired`; the driver owns a dial
future and has no `GuestStream`. Calling the real owner `quiesce()` therefore
takes R5-53's in-flight-dial cancellation path: abort the watch, then close the
still-owner-held `V_h`. The exact observation is a queued `Detached` record,
sent SHUTDOWN `{R,S}`, and no stream to mark owner-aborted.

The pre-correction model enabled `Abort(h)` solely because the watch was live
and unconditionally applied EVMOD-STREAM-ABORT. Independent DESIGN/formal
review approved an exact reconciliation that preserves both accepted outcomes
without changing production. `fwd_pair` now records `strmHeld`, false at
`InitWith`, set only when a successful guest `Attach(g)`/`Paired` returns the
real move-only stream, and cleared only by actual holder drop. The host driver
projects it directly from `self.stream.is_some()` as a top-level fact, including
after owner death; it keeps no shadow phase.

Existing actions branch on that ownership cut. Pre-handoff `Abort(h)` aborts
the watch, closes owner-held `V_h`, leaves `strmAbort` false and writes the
existing guest Abort. Held-stream `Abort(h)` retains WATCH-ABORT unchanged:
`V_h` untouched and `strmAbort` true. Dialer writes, FINs, reads and end
observations require `strmHeld`. `Attach(g)` retains late-Paired ordering: the
driver sends the real control reply and accepts the already-fixed dial error,
so a late local guest attach never manufactures a stream. `GuestAbort(g)` now
selects only from the actual guest phase and always clears its message,
independent of host holder ownership.

The first independent source review found one omitted existing outcome in
that handoff mapping: `InitWith("watch") → TcpRst("g", false) →
Attach("g")`. The guest attach must send the real `Refused` reply, and
production `run_dial` then takes its ordinary dial-error cleanup: abort the
still-live host watch, queue one `Detached` record, close owner-held `V_h`,
remove the dial and return `GuestDialError::Refused { errno: ECONNREFUSED }`.
Because `Refused` is the reciprocal reply, this path sends no guest `Abort`.

The corrected existing `Attach`/`AttachHook` effect composes exactly that
cleanup when the watch was live and guest attach returned `halfEnded`. It
leaves `strmHeld = false`, `strmAbort = false`, `hres = none` and
`guestAbortSent = false`; the refused guest half retains the existing
pre-pair close and zero-record outcome. The host driver still sends the real
control reply and now checks the live refusal's fixed typed errno before
accepting the completed dial. A late refusal after an already-ended host dial
continues to preserve that dial's earlier fixed error. This closes F44-01
without a new action, run, scenario, API, seam, state or production change.

The first approved-source Metal typecheck stopped before every replay with
Quint QNT101 because the helper's private local name `next` shadows Quint's
built-in `next`. No check or evidence ran. That local is now named `attached`;
its definition, every use and all transition guards/effects are otherwise
byte-for-byte equivalent. This is a compiler-required model-only identifier
correction; the formal sequence restarts from typecheck, and the final
independent evidence review includes its alpha equivalence.

After that correction, the exact seed advanced to Trace 18:
`InitWith("watch") → CtxRelease("h") → Write("gh") → Attach("g")`.
`CtxRelease(h)` selected the existing host-owner-death branch, whose
`exit_process()` correctly destroys every host-process handle, including the
accepted host control-session record. The model still permits the guest's
local application write and attach before it observes that session end. The
host conformance driver represents those guest actions as private stimulus,
but `Attach(g)` incorrectly tried to deliver its locally produced `Paired` or
`Refused` through the already-dead host record, so the Sim seam rejected the
missing accepted connection before any state comparison.

The driver now sends that reply through the real host control connection only
while the host owner lives. After the modeled owner death it performs the
same local guest attach/refusal stimulus and no host I/O; `owner_dead` is the
existing exact process-lifetime fact, and no owner projection is read after
death. A living owner still exercises the real dial, including the exact
Refused errno and successful `GuestStream` handoff. This changes no model
guard or outcome, does not recreate the destroyed connection, and adds no
state, action, API, seam or production mechanism.

The next exact compile-time-seeded replay stopped on a genuine formal-contract
divergence rather than another fixture failure. Metal run
`07069ac6-5be7-47a5-9f05-26b0cc412ff8` replayed `QUINT_SEED=0xee26ec37` with
verbose output captured at
`.context/netns-density-295/12-01/quint-fwd-pair-seed-ee26ec37-verbose1.log`.
Trace 14 is `InitWith("watch") → TcpRst("g", false) → VmmExit →
FwdErr("h") → CtxRelease("h")`. Immediately before the final action, the
host watch is ended with one queued `SocketError` record, the actual dial
still owns no `GuestStream`, the old context is open, the VM is gone, and the
owner's end-record task has not consumed the record. The model and driver
agree on every projected field at that point.

`CtxRelease(h)` correctly selects Forwarder recovery because no pair or watch
is live. Its required pre-close quiescence still finds the pending
`DialPhase::AwaitingReply`: `quiesce` records `Quiesced`, cancels and awaits
the flow task, and `run_dial`'s existing error cleanup closes its still-owned
`V_h`. The driver therefore projects sent `{R,S}` and `sshut = true`; the
approved model's holder-free `recoveryClose` leaves both empty/false. Every
other field matches, including the released context, `strmHeld = false`, the
queued `SocketError` record, and absence of an owner-release record.

This evidence falsifies the approved premise that every holder-free ended
watch's pending dial has already completed its typed error cleanup before
Forwarder recovery. A queued end record and its still-pending dial task are a
reachable intermediate state, and quiescence owns its cleanup. The smallest
replacement can reuse the existing model state and `shutAll`: holder-free
recovery of an ended watch must ensure owner-held `V_h` is closed while still
leaving `strmAbort = false`, `hres = none`, and sending no guest Abort when
the VM/session is gone. If earlier typed-error cleanup already closed it,
`shutAll` is idempotent. Because the canonical design and feature delta
expressly say that holder-free recovery adds no socket effect, DISTILL did not
edit the model to choose this replacement. The affected formal design and
roadmap evidence remain pending until DESIGN records and independently reviews
the corrected existing-action contract; no later seed, named suite, job-3
record or evidence verification ran.

Independent DESIGN/formal review iteration 3 approved that exact bounded
replacement. `recoveryClose` now uses only the existing ended-watch,
`strmHeld`, V_h sent-flags and `vmUp` facts. Its holder-free branch records
whether socket cleanup is pending, applies the existing idempotent `shutAll`,
keeps `strmAbort = false` and `hres = none`, and preserves the terminal watch
record unchanged. It sets `guestAbortSent` only when cleanup was newly needed
while the VM was live, otherwise preserving any existing message state. The
Trace-14 VM-gone state therefore changes only V_h's sent/own-shutdown facts.
No driver change, state, action, run, scenario, hazard, API or mechanism was
needed for this replacement. Both exact compile-time seeds, the unchanged
40-cell conformance suite, full job-3 record and evidence verification remain
blocked on independent source review of this diff.

The first source review found one omitted clause from the approved selector:
the holder-free ended-watch branch also requires `ep(h) = alive`. Without it,
a completed held-stream `HolderDrop` left `strmHeld = false`, `ep(h) = dead`
and a retained reported `hres`, then a later recovery entered the pending-dial
branch and erased that outcome to `none`. The branch now includes the exact
alive-dialer guard. Trace 14 remains selected because its owner-held V_h
endpoint is alive; completed holder-drop states are excluded and preserve
their terminal result. The non-watch branch, held-stream branch, cleanup
selector and every effect remain unchanged. This is F47-01's sole source
correction and adds no semantics beyond the independently approved selector.

The required exact replay of seed `0xea0ef1a4` then stopped at another
reachable pending-dial cleanup cut. Metal run
`76bf4c31-9881-4fd7-aea7-c7d60e5686ea`, log
`.context/netns-density-295/12-01/quint-fwd-pair-seed-ea0ef1a4-attempt2-verbose1.log`,
reached Trace 47:
`InitWith("watch") → CloseUnattached("g") → CtxRelease("g") → VmmExit
→ FwdErr("h") → VCtl("gh") → TakeEnd("h")`. Before `TakeEnd`, the
holder-free host watch has one queued `SocketError` record and its
`AwaitingReply` dial still owns V_h. The model and driver agree on every
projected fact.

The real host `TakeEnd` releases the record to `process_end`. That dispatch
recognizes the `AwaitingReply` dial, fixes `Unreachable { ECONNRESET }`, and
cancels its task. `run_dial` then performs every dial error's existing
cleanup: abort the already-ended watch, remove the dial and close owner-held
V_h. The model's current watch-mode `TakeEnd` only changes `rec` from
`queued` to `read`, based on its former assumption that watch V_h is always
holder-owned. Actual V_h consequently has sent `{R,S}` and `sshut = true`,
while the model retains empty/false; every record, error, context, endpoint
and holder fact matches.

This is the same accepted pending-dial cleanup class as Trace 14, reached by
record consumption rather than recovery quiescence, and it falsifies the
current `TakeEnd` holder-ownership premise. The bounded replacement requires
no state or action: after marking the record read, watch-mode `TakeEnd` with
`strmHeld = false` and `ep(h) = alive` must apply the same existing
`cleanupPending`/idempotent `shutAll` selector, preserve `strmAbort = false`
and `hres = none`, and record the existing pending-dial guest Abort only when
cleanup was newly required while the VM was live. A held stream retains the
current no-close result. Because the canonical `TakeEnd` action currently
says the watch holder owns V_h and does not pin this holder-free branch,
DISTILL made no model change. Formal execution remains stopped pending an
exact DESIGN amendment and independent review.

Independent DESIGN/formal review iteration 4 approved that exact amendment.
`TakeEnd` still performs the queued-to-read update first. It then reuses the
iteration-3 cleanup selector only for side h, watch mode, `strmHeld = false`
and `ep(h) = alive`, applying it to the post-consumption state. Thus `rec =
read` and the original outcome, cause, errno and `nrec` pass through
unchanged; incomplete owner-held V_h cleanup becomes sent `{R,S}` / `sshut =
true`, `strmAbort` remains false, `hres` remains none, and the existing guest
Abort fact is added only when cleanup was newly required while the VM was
live. Held streams, dead endpoints, non-watch pairs and guest-side records
retain their existing exact effects. No driver, state, action, run, scenario,
hazard, API or mechanism changed. Both exact seeds and the full formal gates
remain blocked on independent source review of this amendment.

`HolderDrop` keeps its single action: with a stream it performs the existing
stream drop and clears both holder facts; before handoff it cancels a still-live
pending dial without setting the stream-drop ghost; with neither holder nor
live watch it is disabled. Recovery marks an ended watch owner-aborted only
while its stream remains held. Existing properties now pin
`strmAbort ⇒ strmHeld`, holder-only calls, and held-stream recovery. No action,
named run, scenario, public API, seam or production mechanism was added. The
exact seed, unchanged conformance inventory, typecheck and full job-3 evidence
record remain mandatory after independent source review.

### `quint_flow_quota` timeout disposition

The stale broad run timed out the six observed `quint_flow_quota` cells after
the common composition seed was printed and before any later trace-step
evidence. The common driver `init` constructs the full production limits to
realize the model's `Q = 1`, `CAP = 2`: 4,095 background flows for each of
three model allocations plus filler until 65,534 live pairs exist. With
`BATCH = 64`, that is more than one thousand owner settles per test before the
first modeled action, repeated independently by each named trace.

This evidence does not identify a wrong model action, projection or production
ordering; it stops during the deliberately full-scale fixture precondition.
At the time this disposition was made, the stale broad job did not distinguish
slow full-scale initialization from a particular blocked effect. The stale
broad job also predates the crafter's current production fixes. It did not
justify increasing `BATCH`, injecting counters, lowering limits, caching a
composition, or adding a capacity seam.

#### Subsequent user-approved test budget exception (2026-10-26)

After the Sim forwarder's incremental inventory-count correction, one complete
65,534-flow quota trace passed in approximately 19 seconds. The randomized
quota test still performs all eight independent full-capacity traces; the
measured total is above Nextest's default 120-second per-test ceiling. The user
explicitly approved: “Approve 240-second exception” for
`overdrive-control-plane::quint_flow_quota_randomized_traces` only. Its scoped
`profile.default.overrides` entry uses a 60-second period and four periods
(240 seconds). The override preserves all eight samples, full 65,534-flow
initializations, existing actions and assertions; it changes no quota, model,
test body, or other timeout. Exact-seed and full-suite results are recorded in
the crafter's raw Metal logs under `.context/netns-density-295/12-01/`.

Validation after the approved override:

- `QUINT_SEED=0x68e1cd8a` randomized replay: Nextest run
  `142e70db-25a4-466a-bc60-7e94d0438402`, 1/1 passed with all 8 traces in
  145.961 seconds; raw log
  `.context/netns-density-295/12-01/quint-flow-quota-randomized-seed-68e1cd8a-approved-240s.log`.
- All 8 `quint_flow_quota` cells: Nextest run
  `c63c2e1e-f27b-48e2-9f64-badf14f31c36`, 8/8 passed in 290.638 seconds;
  the randomized cell took 151.460 seconds and was the single slow test. Raw
  log `.context/netns-density-295/12-01/quint-flow-quota-suite-approved-240s.log`.

### `quint_cid_lease` Launch effect boundary

The current 23-cell CID-lease gate reached all activated bodies, but 22
stopped at their first ordinary `Launch` with the driver's generic
“hazard-only ... device held” error. Read-only diagnostics, removed after the
focused reproduction, proved this was not a launch clash: the Sim VMM had no
device holder, the host recorded no PID, the convergence tick was unfinished,
and there was no allocation row. The driver had returned from its wait while
the allocation's original `Held::Provision` call was still in the
provisioner's waiting list.

`HeldProvisioner::release` grants the held call and wakes its task; that task
removes the waiting entry only when it is next polled. The Launch wait accepted
`held_for(a).is_some()` as completion, so it immediately re-observed the
just-granted Provision entry before the dispatch could consume the grant,
create the VMM or reach any production refusal. It then interpreted the
absence of a VMM as the model's clash branch. This made every ordinary launch
look hazard-only and never exercised `VmDriver::start`.

The existing wait now treats only `Held::Teardown` as its alternate held
completion. That is the already-authored post-launch boundary: a restart or
failed start can enter successor/predecessor cleanup and park at teardown.
The just-released Provision entry no longer satisfies the wait, so the
production dispatch must either create a VMM, finish, or reach that existing
teardown boundary. The later launch logic, real device-holder observation,
READY delivery, row projection and actual clash classification are unchanged.
No model action, refusal, API, seam, timeout or test body was added.

After that correction, the fixed trace reached the real VM start and READY
path but parked at teardown with the VMM gone. The final read-only diagnostic
(also removed) showed the exact ordering: the allocation already had its
`Running { reason: Started }` row, PID `1000000` was no longer live, the device
holder map was empty, and teardown of that same allocation was waiting. This
is the action shim's post-start fail-closed cleanup, not a VMM launch clash.
The CID fixture had constructed its existing `MtlsInterceptWorker` without
starting its node-shared owner; the action shim therefore failed
`mtls_lifecycle.start_alloc` after the VM driver succeeded, stopped the VMM and
entered guest-network teardown.

The composition now calls the existing worker's `start_shared_owner()` before
building `AppState`, matching the production boot precondition and the other
VM action-shim fixtures. It still uses the same worker instance in `AppState`.
No production error is bypassed: the positive Launch reaches the already
approved mTLS allocation lifecycle after its shared owner is healthy, while
the existing fail-closed mTLS fixtures retain their explicit unstarted/failing
owners. This is fixture setup only, with no API, seam, scenario or expected
state change.

The three-offset pool run then reached its designed full-pool refusal. The
action shim correctly treated `PoolExhausted` as an admission refusal, cleaned
the predecessor at its held teardown, and preserved the same typed refusal as
the convergence tick's final error. The driver had treated every tick error as
an unexpected production failure when `Release` reaped that completed tick.
The existing `AssignRefused` mapping removes `placement[a]` only for this
path; `Release` now uses that existing distinction to reap the tick and accept
exactly `ConvergenceError::Shim(ShimError::GuestNetwork(
GuestNetworkError::PoolExhausted { .. }))`. Any `Ok` or different error still
fails the conformance step. Predecessor cleanup, lease release, retry count and
the subsequent placement remain projected normally; no production refusal is
swallowed outside the one modeled `AssignRefused` path.

Six remaining named cells fail before driver execution because their Quint
runs end in `.then(Action).fail()`. Quint emits a terminal empty ITF state for
the deliberately disabled action; quint-connect 0.1.2 unconditionally tries
to extract `lease_conf_ok::cid_lease::conf_step` from every emitted state and
fails on `{}`. The six are exactly the disabled-action witnesses:
`ReleaseWhileTheVmmRunsIsRefused` and the five runs whose last `Assign` must be
disabled (`ServeCrashOrphanIsReclaimedBeforeAnyAssign`,
`StoppedWorkloadStaysStoppedAcrossServeCrash`,
`FailureAtCeilingExhaustsTheBudget`,
`ServeCrashDuringFinalizeFinalizesAfterRestart`, and
`StopOfAnEndedWorkloadReclaimsItsLease`). Their evaluator checks pass in the
recorded evidence; no production or projection state has been reached in the
Rust failures.

This is a genuine conformance-input incompatibility, not a driver action to
filter or fabricate. After independent review approved the exact bounded
correction, the model now factors the existing `Assign` and `Release` guards
verbatim into side-effect-free, state-reading `canAssign(a)` and
`canRelease(a)` predicates; the actions
use those predicates as their guards. The six named runs end by requiring the
corresponding predicate to be false instead of asking Quint to emit a failed
transition. Their state sequence, enabled actions, refusal outcome, invariants,
scenario names and conformance mains are unchanged. The remaining `.fail()` is
the separate pending-#325 model-only guard witness, which has no live Rust
`#[quint_test]`.

No trace state is filtered and no refusal action is manufactured. This changes
the formal source hash, so source review must approve this exact diff before
the mandatory job-3 record and evidence verification. A pre-review static
typecheck through the required Lima command did not start because Lima reset
its SSH connection. The first approved Metal typecheck then rejected the
initial `pure def` spelling: Quint forbids a `pure` definition from reading
state variables. Both helpers now use ordinary `def`, which is the exact
state-reading, no-primed-write form required for an enabled predicate; their
bodies and every action/run use are unchanged. The failed typecheck changed no
evidence. This compiler-required syntax correction was independently reviewed
before the Metal sequence resumed.

Independent source review approved that qualifier correction, after which the
required Metal sequence completed in order:

```text
cargo xtask metal run -- cargo xtask quint typecheck
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(quint_cid_lease_failure_at_ceiling_exhausts_the_budget) | test(quint_cid_lease_release_while_the_vmm_runs_is_refused) | test(quint_cid_lease_serve_crash_during_finalize_finalizes_after_restart) | test(quint_cid_lease_serve_crash_orphan_is_reclaimed_before_any_assign) | test(quint_cid_lease_stop_of_an_ended_workload_reclaims_its_lease) | test(quint_cid_lease_stopped_workload_stays_stopped_across_serve_crash)' --no-fail-fast --no-capture
cargo xtask metal run -- cargo xtask quint check --subsystem guest-flow-owner --jobs 3 --record
cargo xtask metal run -- cargo xtask quint verify-evidence --subsystem guest-flow-owner
```

The typecheck passed all seven specs; the six formerly empty-terminal cells
passed 6/6. The full job-3 record passed 279/279 under run id
`2043112-1792964018.889`, with no residual Quint/JVM process, and
`verify-evidence` passed. Its one informational timing warning is unchanged in
meaning: `udp-slots-hazard-late-paired-by-slot-route` took 181.93 seconds with
`ci = true`, and still produced its expected `violation`.

The recorder initially created its remote evidence subtree as root, so the
wrapper could not sync it back. The repair changed ownership only for
`/home/ubuntu/overdrive/specs/quint/guest-flow-owner/evidence`, then synced
that exact subtree; generated bytes and run id were preserved. The record
changed 80 evidence files (the two summaries and 78 trace artifacts) plus the
one `cid_lease.qnt` input. Local summary consistency is exact: all 279 entries
have `pass = true` and `outcome == expect`; recorded/local SHA-256 values match
for `cid_lease.qnt`
`c0e3b2b077067ae1fff8c1e61f055bdc18c384a64af4d2f7592747e556464481`
and unchanged `checks.toml`
`6915260626ab7238a55da721f0dbe26c262da6e00c20d76c214551b3096470a4`.
The synced summary hashes are `summary.json`
`9e1e6cb36e818f54c805feaf77dfee64e5cc1c72ae11afe6531f470e1836db76`
and `summary.md`
`57e754363dad80508025c5b0109592ed809c134d7c612b852f375f9defec9038`.

### Final fixture-lint alignment

The workspace Clippy pass exposed four mechanical test-support lints after
the deletions shortened or shifted existing bodies. DISTILL removed the stale
`too_many_lines` expectation from the now-shorter boot-order seed helper,
renamed the private Forwarder open-effect and quiescence-hold bindings to keep
their two distinct ownership roles explicit, replaced `last()` with
`next_back()` on the existing double-ended provision-address iterator, and
factored the admission-refusal body's unchanged lifecycle-bus drain loop into
a private fixture helper. The body is now 95 physical lines after rustfmt,
below Clippy's 100-line threshold; the helper preserves the same receive
order, empty/closed termination and lag panic. The ethtool test-local
`FEATURES_SET` rustdoc identifier remains backtick-quoted. No lint allowance,
assertion, event ordering, iterator filter or behavioral outcome changed.

## Broad default-gate failure grouping and bounded alignment

The first current-tree broad default gate ran on Metal as nextest run
`6ffbaccb-6e3b-4896-854f-a60b2c35a10a`; its full output is
`.context/netns-density-295/12-01/default-worker-control-core.log`. The exact
command excluded only `quint_fwd_pair` while that source was under independent
review. It compiled, then ran 2,178 tests: 2,052 passed, 114 failed, 12 timed
out and 159 were skipped. The failures form bounded common-cause groups; they
are not 126 independent oracle changes.

- Eight properties generated inputs outside approved ingress domains: four
  produced nine probes against `GUEST_PROBE_SLOTS = 8`, and four produced a
  listener port inside `guest_reserved_ports()`. Their positive round-trip and
  equivalence subjects require valid services; separate refusal bodies retain
  maximum-probe and reserved-port sad paths. The existing generators now
  filter only on those production constants. Per-role probe sizes, arbitrary
  descriptor fields, non-empty cases, protocols, ordering and distinctness
  coverage remain unchanged.
- Eighteen action-shim failures reached the fixture-only `AppState::new`
  invariant with no reported guest flow ports. Six async builders constructed
  an unconverged owner immediately before the convenience constructor, while
  production boot converges it first. The netns-density seam, runtime
  convergence, service workload, injected-clock runtime, lifecycle-backoff and
  mTLS fail-closed builders now call that same owner's existing
  `converge_shared()` and pass the same instance onward. No port is injected,
  guessed or defaulted; `AppState::new` captures the owner's reported ports.
  Existing per-action trace marks occur after fixture construction.
- Six `quint_quiescence` failures were executable-fixture teardown defects.
  The runner replaces one driver between traces and drops it after the last;
  a still-live composed process then reached the Sim seam destructor with one
  or three unclosed handles. Explicit `Crash` also exited the seam before
  aborting and joining server tasks, allowing a surviving task to make an
  owner call after exit. The private composed fixture now drops its retained
  owner/dial references, aborts and joins the existing
  `ServerHandle::kill_for_test` boundary, and only then releases the seam
  context. `QuiescenceDriver` uses the same boundary before replacing a live
  node and on normal drop; panic unwinding performs no secondary teardown.
- Four fail-stop timeouts (two Quint quiescence and two shared-recovery cells)
  observed `shutdown_requested()` nonblockingly and then awaited the same
  one-shot request again in `end_after_fail_stop`. `ComposedNode` now caches an
  observed request and consumes that exact value when ending the process; an
  unobserved request retains the original await. No timeout, retry or recovery
  budget changed.
- The remaining named quiescence failure never reached the driver. Its run
  ended with `.then(OpenFlow).fail()`, so Quint emitted the deliberate
  disabled transition's empty terminal `{}` and quint-connect tried to read
  `conf_step` from it. This is the same proven tool incompatibility as the six
  CID-lease cells. The model now factors `OpenFlow`'s guard byte-for-byte into
  state-reading `canOpenFlow`; the action uses that guard and the unchanged
  run ends with `.expect(not(canOpenFlow))`. Enabled actions, states,
  invariant and refusal outcome are identical. This formal-input change needs
  independent source review and the already-required full job-3 record.
- Exact randomized quiescence seed `0x8703c2cd` (Metal run
  `f4e6751b-6063-4ba3-b8fb-579adcaf1e21`, log
  `.context/netns-density-295/12-01/quiescence-random-8703c2cd-verbose1-backtrace.log`)
  separates two further causes. In Trace 1, an old `control_session` child
  survives `ServerHandle::kill_for_test`; after `ComposedNode` marks that Sim
  seam exited, the child reaches `GuestFlowIo::close` and trips the seam's
  postmortem-call assertion. The real OS process death kills the whole Tokio
  runtime, so this is a private fixture process-scope gap, not authority for a
  production drain. The existing helper aborts/joins top-level server tasks
  but owns no handle for owner-runtime child tasks, while the conformance
  driver deliberately reuses one Tokio runtime across process restarts. No
  existing accepted cleanup API can end those children without turning abrupt
  death into graceful owner teardown; DISTILL did not suppress the Sim
  assertion or add a task-control seam.
- In Trace 2 of the same seed the first compared difference is only
  `flowsOpen`: the model enables `OpenFlow` after `Damage("C1")`, while C1 is
  the production FlowListeners component and its accept task is dead. The
  driver queues a connection, but no owner can admit it, so production
  correctly remains at zero. Current `OpenFlow` means an admitted/paired flow,
  not a mere queued guest connect, yet its guard has no flow-listener health
  premise. Adding `not(dmg[C1])` would remove a production-infeasible formal
  action, but it changes model enabledness and is not authorized as a trace
  filter. DISTILL left the model unchanged and records the exact formal
  contract gap for review.

### Proposed private per-process runtime ownership for abrupt-crash conformance

The Trace-1 postmortem has a bounded test-support correction, but it requires
an ownership refactor rather than another settle or production drain. The
node-global fixture facts already live outside the process: `ComposedNode`
owns the injected `SimClock`, `SimMtlsIntercept`, `GuestAddressPool`, durable
temporary directories and boot count. Only `QuiescenceDriver` owns the Tokio
runtime on which each `boot_server` process and every detached owner child task
runs. Reusing that runtime across `Crash`/`RestartWith` is the sole reason an
old `control_session` can run after its process's seam exited.

The proposed two-file private refactor is:

1. In `shared_network_test_ports.rs`, split `ComposedNode`'s existing abrupt
   end into two internal phases. Phase A drops the retained owner/dial
   references, invokes and awaits the existing `ServerHandle::kill_for_test`
   abort/join fence, removes `process`, records `NodeStop`, and returns the old
   process's `Arc<SimGuestFlowIo>` **without** calling `exit_process`. Provide
   the same phase-A shape for fail-stop after consuming its cached shutdown
   request. The existing ambient-runtime acceptance helpers keep their current
   public behavior by calling phase A and then `io.exit_process()` themselves.
   No production type or Sim method changes.
2. In `quint_quiescence.rs`, treat its existing `tokio::runtime::Runtime` as
   the current Sim process incarnation. `Crash`, fail-stop, trace reset and
   normal driver drop first run phase A on that runtime, then replace and drop
   the old runtime. Dropping it cancels every remaining detached accept,
   control-session and flow child exactly as OS process death does. Only after
   the runtime is gone does the driver call the returned seam's existing
   `exit_process()`, releasing its context with no task able to call it. A
   later `RestartWith` boots on the fresh runtime while reusing the same
   clock/table/pool/directories. A refused boot likewise rotates its finished
   runtime before the next restart, because a typed boot refusal ends that
   process incarnation. Panic unwinding performs no blocking teardown; runtime
   destruction itself cancels tasks and the Sim destructor's existing
   panicking guard remains the diagnostic-safe fallback.

Compared state does not change. `Crash`/fail-stop still project only `up =
false`, the existing stop cause and the retained shared table; restart still
reopens the same stores and node-global kernel-table arbiter; ordinary live
steps use the same production ports. There is no graceful owner teardown,
task-control hook, new Sim seam/API, assertion suppression, yield count,
sleep, model state or production cleanup. This plan needs independent source/
ownership review before implementation.

Independent plan review 56 approved that ownership shape with zero findings,
and the two private test-support files now implement it exactly.
`ComposedNode` exposes two test-local phase-A endings: abrupt crash and
fail-stop consume/remove the current `Process`, retain the shared table,
drop owner/dial references, await the existing server abort/join fence, record
the same `NodeStop`, and return the old `SimGuestFlowIo` without exiting it.
Its existing ambient-runtime `crash` and `end_after_fail_stop` methods call the
same phase A and then immediately `exit_process`, preserving their prior
observable behavior.

`QuiescenceDriver` now creates its current-thread paused runtime through one
private constructor and treats it as one process incarnation. Crash,
fail-stop, normal trace replacement and normal driver drop obtain the old
seam through phase A, synchronously replace and drop the old runtime, then
call `exit_process` on the retained seam. Runtime destruction is outside every
Tokio future and therefore completes before seam exit; every detached accept,
control-session and flow child is destroyed first. A typed boot refusal also
rotates the ended runtime before the next restart. Panic unwinding performs no
blocking teardown. The node-global clock, intercept/table, pool, boot counter
and directories remain on `ComposedNode` and are unchanged across the fresh
runtime.

Ownership audit: no production or Sim trait/type/method changed; both new
phase-A calls live under the existing `#[cfg(test)]` module; no graceful owner
teardown, task hook, assertion suppression or timing loop was introduced.
`QuiescenceState::from_driver`, action dispatch, model variables, stop causes,
table projection, pool/store reuse and every normal ambient caller remain
unchanged. Rustfmt and scoped/global diff checks pass. Exact-seed execution is
blocked until independent source review.

Source review F57-01 found one early-return hole: `InitWith` mapped a boot
refusal with `?`, returning `BootDiverged` before rotating that failed
process incarnation's runtime. `ComposedNode` now offers a test-local deferred
refusal-exit wrapper around the same boot: success retains the same `Process`;
refusal records the same `NodeStop::Refused`/`refused_io` but returns the
typed refusal together with that seam. Ambient callers preserve prior behavior
by exiting it immediately. `QuiescenceDriver::InitWith` and `RestartWith`
capture that result, replace and drop the refused runtime, then exit the
returned seam before propagating the same `BootDiverged` or expected typed
refusal outcome. Successful boot and optional initial-flow setup retain their
prior order and runtime. This closes every failed-boot path with the required
runtime-drop-before-seam-exit ordering and unchanged projection/error
semantics.

The approved source was then exercised with compile-time seed
`0x8703c2cd` and `QUINT_VERBOSE=1` through Metal (run
`eb0154ed-83ec-46cf-8311-2d98d2c695d2`; raw output
`.context/netns-density-295/12-01/quiescence-random-8703c2cd-verbose1-after-runtime-fix.log`).
The previous postmortem `SimGuestFlowIo` call and panic are absent. The run is
not a conformance pass: its first remaining divergence established the now
implemented, source-review-pending QUIESCENCEOPEN1 model correction, where
`Damage(C1) -> OpenFlow` projects `flows_open = 1` in the model and `0` in
production while every other printed field agrees. This is partial evidence for the process-runtime fix
only and is not formal GREEN. The separately approved guard correction for
`NoFlowOpensWhileCountedHoldRemains` passed its focused named replay 1/1
(Metal run `a6e24f20-786f-4562-8eca-b108f2640383`; raw output
`.context/netns-density-295/12-01/quiescence-no-flow-named-after-qnt-guard.log`).

- Eight `quint_flow_quota` cells originally timed out during the
  production-limit fixture initialization. A temporary private diagnostic
  proved steady progress with no held Sim effect: at the timeout it had
  reached filler allocation 9 at 768/4,096 flows (45,821 total background
  flows and 274,550 issued effects), and the last settle took about 338 ms
  (Metal run `d259ddac-a3a4-4a02-affd-31d7ac903f2c`; raw output
  `.context/netns-density-295/12-01/quint-flow-quota-init-diagnostic-seed3952.log`).
  Source inspection then found the quadratic test-adapter operation:
  `SimFlowForwarder` rescanned every live pair after each attach. The original
  crafter replaced that scan with exact derived counters maintained at every
  existing live-map insertion/removal; the Sim adapter suite passed 19/19 and
  the full 65,534-flow admission cell passed in 19.7 s.

  The resulting eight-cell quota run passed 2 and exposed six honest
  projection failures (Metal run `4bef8717-b131-4cf1-9a1f-81b5bf8c3585`;
  raw output
  `.context/netns-density-295/12-01/quint-flow-quota-suite-after-incremental-summary.log`).
  Compile-time seed `0x80618105` reaches `init -> Admit(1) -> HalfClose(1) ->
  Abort(1)`, where the model expects place 1 ended and `endq = [1]` (Metal run
  `5d518ddf-b572-4a8a-9475-4954c88bcdfe`; raw output
  `.context/netns-density-295/12-01/quint-flow-quota-failure-seed-80618105-verbose.log`).
  A removed read-only diagnostic proved the owner had consumed no end record
  (`last_consumed_end = None`, `consumed_ends = 0`): pair 65,535 remained in
  the open serving row's queue, while driver generation 1 selected closed,
  empty row 0, phase 1's retained probe context.

  `QuotaDriver::project` now applies the already established projection
  contract: `forwarder_generation` names the exact suffix of serving contexts;
  exactly one non-serving phase-1 row must precede it, and that row must be
  closed with zero live pairs, zero live watches and no queued end. Pair state
  and `endq` use only that serving suffix with their original generation/pair
  identifiers and loss checks. This does not filter a retained phase-1
  resource, manufacture a record or alter the model/body/bounds. All temporary
  diagnostics are removed; the source awaits independent review before a
  rerun.

  After that projection source was approved, exact seed `0x80618105` passed
  its complete trace. The eight-cell suite passed six; the two remaining
  failures reached quint-connect's empty terminal `{}` for the sole active
  `flow_quota.qnt` `.fail()` witness (Metal run
  `bcd93b98-66b9-49da-ae66-8a5d9e7b0cb4`; raw output
  `.context/netns-density-295/12-01/quint-flow-quota-suite-after-projection-fix.log`).
  The live witness is exactly `EndRecordsAreReadInQueueOrder`: after queue
  `[2, 1]`, `ReadEnd(1)` must be disabled because it is not the head. No other
  `.fail()` remains in this formal input.

  The bounded formal-source proposal factors `ReadEnd`'s existing conjunction
  byte-for-byte into state-reading `canReadEnd(s)` and makes the action use
  that predicate. The named run asserts `not(canReadEnd(1))` in the identical
  pre-transition state instead of asking Quint to emit a failed transition.
  Slot/end-queue state, action enabledness, action sequence, refusal outcome,
  scenario name, conformance main and Rust driver are unchanged. The README
  records the compatibility reason. This model-input change awaits independent
  source review before typecheck or replay; after approval it joins the single
  final job-3 full record and evidence verification rather than triggering a
  stale intermediate record.
- All thirteen host `quint_udp_slots` cells stop after their first
  `GuestReady`. QPROJ-3 requires `cids[cid].datagrams[flow] =
  AwaitingPaired`, but production `datagram_associate` inserts a datagram row
  only after attach as `Paired`. While route lookup is held, the connection is
  neither queued nor closed and no owner row exists, so the driver correctly
  refuses to fabricate model state. This is a production owner-observation
  gap for the crafter; DISTILL leaves the model and bodies unchanged.
- Eight `quint_owner_flows` cells expose production semantic mismatches. Two
  share an exact path: with no live session or a Retiring CID,
  `register_flow` computes `tracked = false` but still starts `handle_flow`
  and drops its local `JoinHandle`, detaching an untracked socket task. O has
  no row and S has no owner close, producing “flow 2 has no model state.” The
  accepted boundary closes that connection. The other six are seeded
  invariant divergences (`0xf23a4188`, `0x174c2bd4`, `0xafd56990`,
  `0x2ccf5f3f`, `0xf4ff9824`, `0x3baf34c8`) and require compile-time verbose
  replay before any source change. No driver/model/oracle was changed from the
  broad output.

One of those owner-flow seeds is now reduced to an exact typed-error contract
gap. Metal run `5c554937-a39a-40dc-b2aa-b64121b30f76`, compile-time seed
`0x3c09cb16`, log
`.context/netns-density-295/12-01/owner-flows-seed-3c09cb16-verbose1.log`,
reaches `GuestAcceptedError(1) → HostRecvAbort(1)`. Guest and host phases
both close exactly as specified; only the dial result differs. The formal
action fixes `Unreachable:io`, while production returns
`GuestDialError::Unreachable { errno: ECONNRESET }`.

The mismatch is not a projection spelling issue. `GuestAcceptedError`
delivers `Hangup { Error }` on accepted V_g, but the guest's wire
`ControlMessage::Abort { flow }` carries no errno. Production
`abort_guest_flow` handles an `AwaitingReply` dial by deliberately inserting
the fixed typed payload `Unreachable { ECONNRESET }`; the driver separately
projects ENODEV, ECONNRESET and other I/O errors, so normalizing ECONNRESET to
the generic bucket would erase the distinct VMM-exit contract. Before
PABORTERR1, the feature delta pinned `dl.res` as the real dial result and named
this guest action but did not pin the exact errno mapping for an `Abort`
received before `Paired`; the formal model's `Unreachable:io` was an abstract
label, not an approved public payload. DISTILL therefore made no model,
projection or production change. PABORTERR1 now pins this condition to fixed
`ECONNRESET` and requires the existing formal action to record that exact
result. A wire/error-contract change, action-specific driver shadow, or
arbitrary generic errno remains invalid.

After PABORTERR1 approval, the existing `HostRecvAbort` formal transition now
uses `dlEndOne(h, "Unreachable:ECONNRESET")`, and the existing
`AcceptedVgErrorAbortsWithAbort` run requires that exact result. No driver
normalization, variant, payload, wire field, action or run was added.

The teardown seed is also reduced without changing test support. Metal run
`1ace9abe-e4db-44f1-8bb5-ad7530806dcd`, seed `0x864f5500`, log
`.context/netns-density-295/12-01/owner-flows-teardown-864f5500-afterfix.log`,
reaches `GuestOpenSession(1) → HostAcceptSession(1) → Teardown` before
the first record is read. `spawn_control` has already inserted the key into
the owner's `unread` set and registered its control task before making that
task runnable, so there is no accept/registration fixture race. Teardown now
correctly retains that unread task, but its final postcondition excludes
unread control tasks from forwarding residue and then unconditionally removes
the whole CID. The accepted socket is still open and the task is still live,
while O has lost `unread` and S is neither queued nor closed; the driver
therefore has no honest projection state.

This is production lifecycle ownership. Teardown must retain the Retiring CID
when only accepted-unread control tasks remain, without treating them as live
forwarding residue; `finish_connection` must remove that Retiring CID after
the final retained unread/control task closes and all session, flow, dial,
datagram and pair state is empty. The existing body must continue to observe
`reading` after teardown and `closed` after `HostReadOpen`. No Sim, projection,
model, barrier or body change is warranted.

Two adjacent stale-launch bodies had a separate fixture mismatch after the
owner correctly began closing truly unknown CIDs at accept. Their stated
subject is a queued connection from the previous launch of the **same CID**,
so the host accepts it under the current CID row and rejects its launch token
at the first-record read. They used the default `/16` pool, whose next-fit
cursor assigns the next lease a different CID after release; the old key was
therefore genuinely unknown and correctly closed at accept, making the later
`release_read` invalid. Both existing bodies now compose the owner through its
existing `Composition::with_prefix` path with a `/30`, gateway `.1`: exactly
one usable address/CID, the same production `GuestAddressPool` behavior used
by the owner-flow conformance driver. No CID is forced and no seam, API,
state, action, timing or assertion changes. The old launch token still differs,
so the first-record attribution/refusal oracle retains its teeth.

The focused rerun passed the stale-session cell and 87 of 88 owner-acceptance
cells. The remaining batched stale-flow cell had one more missing authored
precondition: after proving the old control session stale, it accepted the old
flow/datagram/empty records before the current launch had opened a live
control session. Production correctly applies the no-session boundary at
accept, so those sockets never reach a request read. The body now accepts and
reads the old session first, opens and claims the new launch's real control
session, then accepts and reads the three already-queued stale flow records.
They remain the old launch's same-CID connections with their original tokens;
the current live session changes only admission reachability, allowing the
intended ATTR-TOKEN comparison at read. The closed-at-read, zero message,
zero connect/attach, current-session claim and activation assertions are
unchanged.

Source review F53-01 found that releasing the current `ControlOpen` read and
immediately releasing the first stale-flow accept still let the accept race
the claim. The body now crosses the existing Sim idle boundary after the
current read and observes the production owner reporting `session =
new_session` before it releases any old flow accept. Each stale connection's
existing accept → settle → record-read boundary remains explicit. This
uses owner observation as the completion witness, with no yield count, sleep,
new seam or timing assumption.
- One worker failure was a fixture stimulus outside the real lifecycle: the
  body terminated the VMM and only then wrote `EXIT 7`, but an accepted
  connection's next Sim I/O correctly resets after VMM death and a real guest
  cannot write after its process ended. The unchanged intent is EXIT-received
  before subsequent VMM teardown. The body now writes while the VMM is alive
  and waits on the existing `SimBeaconIo::parked()` observation until the
  production writer has consumed the line and is awaiting another; only then
  does it terminate the VMM. This is an observed receipt boundary, not a
  yield-count timing proxy or direct queue mutation, and the authoritative
  EXIT-7 assertion is unchanged. Three worker failures remain production/
  conformance evidence: a backpressured exec release prevents BeaconWriter
  close from making the complete guest stream readable; one beacon named run
  violates its state invariant; and the beacon randomized model selects
  connection 1 while the real FIFO has connection 2 first.

The FIFO mismatch is now reproduced exactly: Metal run
`50c02457-e6a3-49f4-a701-d22376d53bcc`, compile-time seed `0x2331472b`, log
`.context/netns-density-295/12-01/quint-flows-beacon-randomized-seed-2331472b-verbose1.log`.
The trace opens beacon 2, then beacon 1, then chooses
`HostAcceptBeacon(1)`; `SimBeaconIo` and AF_VSOCK accept correctly keep beacon
2 at the FIFO head, so the driver refuses the infeasible action. Current
`owner_flows.qnt` has no accept-queue/order variable: `bcn` records only each
connection's phase and generation, and D-UP-19/README expressly say any
pending beacon may be accepted because order is not modelled. With two
same-generation pending connections the existing state cannot identify which
opened first. A head-match guard cannot be expressed honestly without an
approved queue/order fact. Accepting FIFO while applying the model's chosen id
would fabricate a cross-connection step; ordering by numeric id would filter a
real open order. DISTILL therefore made no model or driver change. BFIFO1 now
pins the minimal beacon accept-queue state updated by the existing open/accept
actions.

The approved formal/test-support package now implements the exact six folded
decisions without adding actions, runs or public surface:

- BFIFO1 adds model-only `bq`, empties it at `InitWith`, appends at
  `GuestOpenBeacon`, restricts/removes its head at `HostAcceptBeacon`,
  preserves it across every other transition, adds it to `allVars`, and adds
  `BeaconQueueExact` to existing `BeaconSafety`. The worker driver removes its
  private `accept_order`; one real accept and the existing connection-state
  projection establish identity.
- ACTPRIORITY1 adds only `not(quiesced)` to `ActivatePending`; its result and
  no-state-change effect remain unchanged.
- ADMISSIONREFUSAL1 retains the formal `HostAcceptFlow` state transition, but
  its existing conformance handler now recognizes the real Active/live-session
  Quiesced branch, releases accept and the complete buffered request in the
  same action, and requires exactly the production-emitted
  `Refused { flow, Quiesced }`. The existing R5-26 body additionally confirms
  zero guest-flow state, zero held place and zero attach after that refusal.
  The handler never injects or shadows a result. Production refusal-reader,
  bound/rearm and fd-budget implementation remains the crafter's ownership.
- QUIESCENCEOPEN1 adds only `not(dmg.get("C1"))` to the existing
  state-reading `canOpenFlow` predicate; the driver and action effects remain
  unchanged.
- PABORTERR1 is the exact transition/assertion change recorded above.
- The README records the bounded action/result/identity meanings, including
  the D-UP-19 beacon exception and `BeaconQueueExact`.

Static source formatting and diff checks pass. No Quint typecheck, seed
replay, conformance suite, formal record or evidence verification starts
before independent source review. All formal inputs join the one final
three-job guest-flow-owner record and `verify-evidence` gate after source
approval.

Iteration 61 found four bounded source defects. Three are closed in the
current package:

- PABORTERR1 now changes only `HostRecvAbort` to fixed
  `Unreachable:ECONNRESET`; `HostAbortAccept` is restored to generic
  `Unreachable:io`. The existing public `GuestDialError::Unreachable`
  documentation records that the payload-free pre-Paired peer `Abort` maps to
  fixed `ECONNRESET`; no variant or runtime behavior changed.
- The Quiesced `HostAcceptFlow` handler now releases accept and crosses the
  existing settle boundary until production owns a record handle and has
  parked its read; only then does it release the complete buffered request and
  settle again. Both fixture phases remain one formal action, with the same
  exact outbox-delta and state comparison.
- The README's opening D-UP-19 inventory now states the order-insensitive
  default and names BFIFO1's bounded beacon FIFO exception, while leaving
  reclamation unordered.

The existing authored admission evidence is strengthened as far as its
current bodies honestly reach. The R5-26 body retains stable Quiesced refusal
and zero state/place/attach. The existing stale-token body now runs while
Quiesced and still requires silent `StaleLaunch`, proving attribution before
the snapshotted refusal. The existing malformed body runs while Quiesced and
checks both exact `Malformed` and `Unsupported` responses, proving decode
classification priority. Those two readers run concurrently: the source
asserts exactly two messages and exactly one matching identity/payload for
each flow, without sorting, dropping duplicates or imposing cross-flow
response order. The first focused admission run observed the valid completion
order `Unsupported(flow 2), Malformed(flow 1)`; its cap body and fd cell passed
(Metal run `fbd52880`). The existing fd-budget body requires exactly
`FdBudget { required: 376_832, available: 1_024 }` rather than accepting any
value above 1,024.

### Exact private admission validation body

Independent authority review 62 confirmed that the existing user approval of
ADMISSIONREFUSAL1 already authorizes its required private validation, without
a new user decision. One source-local body is now added in the existing
`owner_acceptance/admission.rs` module:

`admission_refusal_readers_are_bounded_rearmed_and_cancelled_without_cross_lifecycle_response`

It carries `/// CONTRACT_SHAPE: bounded-change.` and adds no scenario id,
public hook, Sim seam, model action or lane. Its helpers remain test-local and
exercise the production-composed owner and the crafter's actual private HOW:

- It seeds the production `OwnerState::insert_flow_task` counter to 65,535
  with exact refusal identities, then proves an ordinary stream
  (`owner_launch = None`, `flow = None`) and a decoded datagram
  (`owner_launch = None`, `flow = Some`) do not count. The latter is the
  exact production shape installed after request decode.
- It releases two real current-launch connections through the existing stream
  accept port. The first actual `spawn_flow` refusal reader reaches exactly
  65,536 under the production lock; the second remains in the real accept
  queue. Releasing the first complete request makes the actual reader emit
  one Quiesced refusal, close, run `finish_flow_task`, remove itself and
  publish the production `Notify`; only that real completion/rearm lets the
  second connection become the accepted, read-pending refusal reader. The
  synthetic entries are never used to emulate notification.
- With the second reader demonstrably accepted, classified and parked on its
  read, a third connection waits at saturation. Root shutdown and the second
  request are then made ready together. The production biased cancellation
  branch closes/removes the reader without a response, its real removal
  publishes capacity, and the shutdown-priority accept loop leaves the third
  connection unaccepted.
- Fresh production compositions drive a read-pending refusal reader through
  the real teardown port and through real control-session EOF followed by a
  replacement session. Both paths complete cancellation, remove the count,
  close the socket and send no response across lifecycle/session identity.
- Post-assertion synthetic cleanup uses only `remove_flow_task` after root
  shutdown, when no capacity waiter exists; it is not the rearm evidence.
  Real teardown/session tasks remain awaited by their production lifecycle
  owners, holds are restored where the process remains live, and each fresh
  composition exits its seam normally.

This closes the count identity, exact cap, atomic saturation, actual
completion notification, lossless rearm, biased root-shutdown priority and
teardown/session-replacement obligations against present production branches,
without prescribing or manufacturing a production helper.

The focused Metal execution passed this body 1/1 (run
`7ecf729f`, as recorded by the original crafter). The subsequent workspace
Clippy gate found only four `significant_drop_tightening` findings on its
private `OwnerState` mutex guards. Each guard now has an explicit `drop`
immediately after its last assertion/mutation and before the next return,
socket observation or async boundary. No assertion, ordering, lifecycle,
counter, notification, cleanup or Contract Shape changed; no lint allowance
was added.

### RELEASEPROJ1 — pool-authoritative lifecycle projection

The user explicitly approved RELEASEPROJ1 (“Approve RELEASEPROJ1”), and the
canonical fold at `feature-delta.md:753,9589-9637,10432-10433` is complete.
The reproduced Trace 44 keeps a real accepted-unread control reader after
`Teardown -> Quiesce -> Release`; QPROJ-2 proves the pool lease absent while
QPROJ-3 retains a `Retiring` cleanup row. The previous owner-row-first driver
therefore projected a fictitious Retiring lease/generation.

`OwnerFlowsDriver::project` now applies the approved four cases over the real
pool observation first:

1. no pool lease projects `Absent` with no compared generation, validating
   any remaining owner row only as the strict cleanup tombstone;
2. a Retiring pool lease projects `Retiring` and that pool launch, while an
   owner row may be absent or the same strict tombstone;
3. an Admitted pool lease must match the driver's CID/allocation/launch and a
   QPROJ-3 `Provisioned` or `Active` row; and
4. absent/mismatched/Retiring owner state under an Admitted lease is a
   conformance failure.

The private tombstone assertion uses current production state, not driver
history: same allocation/CID/launch and `Retiring`; public/private session
authority absent; public unread equals private unread; guest flows,
flow-tasks/ids, dials/replies/cancels/requests, datagrams, pairs and dial
watches empty; held flows zero; `control_tasks` keys exactly equal `unread`;
and every retained task's `owner_launch` is the tombstone launch. Private
non-resource result caches remain outside the projection. Existing session
projection still reads `unread`, so a retained old reader remains `reading`;
same-CID replacement and late finish retain their existing production/model
actions and generation checks.

No formal state/action/run/check, production behavior, API, wire value,
release callback, cleanup owner or test seam changed. Rust formatting and diff
checks precede independent source review; exact seed `0xf23a4188`, unchanged
teardown/stale-launch/owner-flow targets and final formal evidence remain the
post-review gates.

### Queued same-launch control open after VMM death — approved formal alignment

Metal run `0ca90d92`, compile-time seed `0x6539f0dd`, Trace 33 (raw log
`.context/netns-density-295/12-01/owner-flows-6539f0dd-verbose-current.log`)
reaches `Provision -> VmStart -> GuestOpenSession(2) -> ActivatePending ->
VmStop -> Quiesce -> HostAcceptSession(2) -> ActivateQuiesced ->
HostReadOpen(2)`. Immediately before the last action, model and production
agree on Provisioned allocation/lease generation 1, quiesced admission, no
claim and session 2 `reading`. After it, the model reports claim 2 / `dead`;
production reports claim 0 / `closed`. Every other compared field agrees.

This is not a lost buffered record. K-A2-prime marks the still-queued child
reset without removing its buffered `ControlOpen`. On accept, Sim constructs
the real record handle with those bytes followed by the reset. The driver's
existing `release_read` releases the complete record and its following end.
`read_record` returns the bytes first; production `control_session` validates
the matching launch and takes the claim, then its immediately-ready next read
returns `ConnectionReset`. The existing `session_lost` path clears the claim,
closes the socket and finishes the task before the driver's settle boundary.
No stable port-observable claimed/dead state remains.

CONTROLOPENRESET1 is now approved under the user's exact 2026-10-26 authority,
“Approve CONTROLOPENRESET1”, and is canonical in the feature delta and
`design-12-01-control-open-reset.md`. The existing `HostReadOpen` action now
preserves its token-invalid, held-claim and `BUG_SESSION_SPLIT_CLAIM` branches.
Its valid/free branch keeps the existing `live`/claimed result while
`writerUp(s.gen)` is true. While the writer is down it reuses the existing
`sessionLoss(c)` action effects, producing the stable closed/no-claim result
and the same `NoSession` flow/dial/message cleanup that production completes
before the driver's settle boundary. Moving `keepSessRest` into the four
unchanged branches is necessary because the writer-down branch must apply the
existing session-loss cleanup rather than constrain those variables to remain
unchanged.

The README's `HostReadOpen` action mapping records that exact partition and
retains the separate live-session `VmStop -> HostDeadSessionLoss` transition.
No formal state, action, run, scenario, hazard, witness or check changed. The
production owner, Sim adapter, conformance driver, test bodies and public API
are unchanged. Exact seed `0x6539f0dd`, all 46 unchanged `owner_flows` targets,
the final three-job guest-flow-owner record and `verify-evidence` remain gated
on independent source review; this source alignment is not formal GREEN
evidence.

### Current `fwd_pair` non-watch abort closure mismatch — existing-contract model omission

The current exact seed `0xea0ef1a4` replay (Metal run `90540a6f`; raw log
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-current.log`) advances to
Trace 51, not a watch trace. Its exact suffix is
`InitWith(stream) -> Attach(h) -> ShutRd(gh) -> VmmExit -> Fin(hg) ->
Abort(h)`. After `Abort(h)`, model and implementation agree on host owner
phase `ended`, `Detached`, one queued record, `byRel = false`, the V_h
`{R,S}` shutdown, `sshut` and `sk_err`. The sole difference is
`pr(h).closed`: model false, implementation true.

The driver is projecting the accepted fact exactly. `abort_h` calls the real
`HostSharedGuestNetworkOwner::quiesce`; the host owner is alive
(`gh.hostDead = false`) even though `VmmExit` made the guest side gone. The
first hold clears the owner tables, calls `abort_all` (queuing the existing
Detached end), awaits flow tasks, then closes both owned pair descriptors in
`guest_network.rs:1324-1450`. The Sim V_h consequently records `{R,S}` and
the owner no longer holds the flow. The canonical QPROJ-8 definition is
`closed` iff the flow left QPROJ-3 and the shared Sim vsock half is closed, so
`closed = true` cannot honestly be filtered in the driver.

The current formal `ownerAbort` does the opposite for every non-watch side:
it applies `abortSide(..., release = false)`, queues the record and returns
without `shutAll` or `pr.closed = true`. This is a model-transition mismatch
on a reproduced production entry, not postmortem I/O, a holder shadow, an
owner-death stimulus or a projection bug. DESIGN re-read the existing
contract and found it already complete: ordinary owner abort closes the
descriptors (`feature-delta.md:5886-5891`), QPROJ-8 `closed` is the absent
owner row plus closed vsock (`:10573`), and only the watch contract preserves
the holder socket (`:4877-4896`, `:5155-5166`). No new decision is required.

The model's non-watch branch now applies `abortSide`, sets that side's
`pr.closed = true`, then performs `shutAll`; both watch branches remain
byte-for-byte unchanged. The existing `OwnerAbortResetsBothFarEnds` run now
also requires `closed`, `{R,S}` sent and `sshut` immediately after `Abort(h)`.
No state, action, scenario, production behavior, API or driver projection was
added. This formal-source change awaits independent source review before the
exact seed, unchanged FwdPair targets and final three-job evidence record.

### Same-CID second VMM stop — generation-isolated unread delivery

Compile-time seed `0xab174c14` (raw
`.context/netns-density-295/12-01/owner-flows-ab174c14-verbose-20261026.log`)
reaches an old launch-1 control child that survived its VMM's death, teardown,
release and same-CID reprovision, then was accepted as the replacement row's
generation-safe unread cleanup task. A later launch-2 `VmStop` changed only
that old session from expected `reading` to actual `closed`.

The first attempted driver correction filtered releases with
`control_tasks[key].owner_launch`. Exact rerun still failed. The authorized
temporary diagnostic (removed after capture) proves why: the old child was
queued under launch 1 but accepted only after launch-2 reprovision, so the
replacement row and task correctly record accepted launch 2 while the socket
still contains launch-1 bytes and its launch-1 reset. Raw log
`.context/netns-density-295/12-01/owner-flows-ab174c14-vmstop-diagnostic-20261026.log`,
lines 9965-9969, records first/buffered 24 bytes, zero ready bytes,
`end=Reset`, `end_released=false`, old launch-1 record token and
`peer_gone=false`; CID-wide `vm_stop_before_callbacks` changes that old record
to `peer_gone=true`. The task launch is acceptance-row attribution identity,
not physical VMM identity. That premise is retracted.

Selecting by the decoded wire launch would also be wrong: the payload is
untrusted, and malformed, unsupported or wrong-token connections still
physically belong to the VMM that opened them. The independently approved
private test-support alignment now binds trusted physical identity at the
existing fixture connection-creation boundary:

- `QueuedConn` has a private trusted physical-launch field supplied by the
  fixture's `Lease`, separate from the record bytes; `RecordState` receives it
  unchanged at accept. The existing `queue_connection` test-support signature
  carries that optional internal identity; it is not a production port.
- `Composition::{open_session_with,open_tcp_flow_with,open_datagram,
  open_raw_flow}` and the corresponding existing action-shim/shared-network
  fixture creators pass the physical lease launch independently of whatever
  valid, foreign or malformed payload they buffer. Low-level socket-only Sim
  tests use no selective launch stop and retain their CID-wide helper path.
- The shared host-vsock registry tags the actual `SimSocket` at record
  creation/accept with that same private identity; dialled host connections
  use the existing identity-free current-device branch because they cannot
  survive the owner teardown/replacement boundary.
- Existing `vm_stop(cid)` keeps its CID-wide low-level semantics through one
  internal matcher. Existing launch-aware `vm_stop_before_callbacks` receives
  the stopped trusted launch from its three current driver callers and marks
  only queued/accepted/vsock resources tagged with that physical launch. No
  payload is parsed to decide death ownership.
- The owner-flow driver releases accepted loss only for the intersection of
  QPROJ-3 `unread` and the Sim arbiter's now-launch-exact `peer_gone` set; it
  removes the disproved task-owner filter. The old child retains `reading`
  until its own held record read exposes launch-1 bytes followed by launch-1
  reset, then existing stale-token/session-loss handling closes it.

Existing wrong-token and raw/malformed fixture paths provide the negative
source proof: their trusted physical launch remains the lease launch while
their payload is foreign or undecodable. This changes no production behavior,
public API, formal state/action/run, QPROJ meaning or test body. It awaits
independent source review before exact seed replay; no compile, test or model
job ran while the source package was under review.

### Abort-class `VCtl(gh)` — carry out the existing queued first run

The later exact FwdPair replay (raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-closed-model-20261026.log`)
reaches Trace 63:
`InitWith(stream) -> TcpRst(g) -> CtxRelease(g) -> Write(hg) -> Attach(h) ->
Fin(hg) -> VCtl(gh)`. The `{R,S}` close packet reaches attached V_h and the
Sim socket records the abort-class peer bits, but actual owner state remains a
live pair with no record because the driver disabled global auto-run and did
not execute the run the callback queued. The model expects the accepted
EVMOD-END first-run result: `PeerAbort`, one queued record, pair descriptors
closed and V_h `{R,S}` sent.

The driver now reads the live pair's actual `(ForwarderGeneration,
ForwardId)` from QPROJ-3 before applying an abort-class `Packet::Close` or
`Packet::Rst`, applies the existing socket stimulus, and executes exactly one
already-queued `SimFlowForwarder` run through the existing private run API. It
fails rather than synthesizing a result if that identified live pair has no
queued runnable work. `Packet::Shut` remains callback-only so the existing
`HopEnd` actions carry clean half-close progress. Unattached, ended and
post-owner-death abort packets have no live QPROJ-3 pair and therefore run
nothing, preserving their current behavior. No auto-run default, Sim adapter,
production path, model action, test body, API or seam changed. Both private
driver corrections await independent source review before exact seed replay.

### Datagram pre-attach environment — park at the existing UDP connect effect

After the abort-class `VCtl` correction, exact seed `0xea0ef1a4` reaches
Trace 65 `InitWith(dgram) -> UdpBlock(gh)` and the driver reports no owner peer
socket (raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-vctlqueued-approved-20261026.log`,
lines 14585-14642). The model intentionally permits the destination reader to
be blocked before H attaches: this is an external UDP peer condition, not a
pair phase, and `UdpBlock` changes only `dg(gh).blk`.

The fixture currently parks the owner at `RouteType`, before production
`datagram_associate` has opened its UDP `PeerSocket`. That makes every
pre-attach peer stimulus unplayable even though the existing Sim support was
designed to carry it: `latest_peer_socket` retains an opened unattached peer,
and `peer_of(flow)` permits UDP causes while `ConnectUdp` is held (it rejects
only a held stream `ConnectPeer`). Production order is route lookup ->
`open_peer` -> `connect_udp` -> peer validation -> synchronous attach, so the
smallest faithful fixture boundary is the already-existing asynchronous
`ConnectUdp` effect.

The bounded correction changes only the private FwdPair driver:
`InitWith(dgram)` now holds `ConnectUdp` instead of `RouteType`; route lookup and
peer creation complete, then the owner parks at connect with the exact peer
socket available for `UdpBlock`, `UdpUnblock`, early destination datagrams and
errors. `Attach(h)` releases that held `ConnectUdp`; the existing production
task completes connect, validates the kernel peer and attaches synchronously
in the same modeled attach action. No pending blocked-state shadow, forced
socket state, model guard, production yield, Sim API, action or scenario is
added. Stream mode retains `ConnectPeer`, and every post-attach datagram action
is unchanged. The driver's action map records that exact route/open/held-
connect boundary and the release/validate/synchronous-attach continuation;
the prior claim that connect completed before a held peer-validation step is
removed. This source alignment awaits independent review before replay.

### Pair `FwdErr(h)` — carry out the callback's existing queued first run

The next exact replay reaches Trace 67
`InitWith(dgram) -> Attach(h) -> SendDg(gh, 0) -> UdpErr(h) -> VmmExit ->
FwdErr(h)` and differs only because H remains a live pair with no record or
owner shutdown (raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-udpblock-driver-approved-20261026.log`,
line 15893). The input facts are exact: `UdpErr(h)` records the connected UDP
socket error without reporting it; `VmmExit` records V_h's orphan-sweep error;
the existing driver gives the sweep callback priority at `FwdErr(h)`.

This is the same existing first-run boundary as abort-class `VCtl`, now on
`sk_error_report`. `SimSocket::report_error` reads the installed pair hook and
calls `record_socket_end`; for `HookKind::Pair`, that function stores the
`SocketError` abort cause and enqueues both pair directions. With global
auto-run disabled, `finish_sweep_of` and the UDP-error branch currently return
after that enqueue, so projection correctly still sees the live pair. The
accepted `FwdErr` action includes the callback and first run and therefore
expects `SocketError`, one queued end record, and pair-socket abort effects.

The bounded correction uses the same actual QPROJ-3 pair identity as the
VCtl fix, snapshots it before the callback, invokes the existing sweep or UDP
`report_error`, then executes exactly one of the callback's already-queued
runs for stream/datagram pairs, failing if none is runnable. The watch branch
remains unchanged because a
watch callback itself ends the watch and queues its record without a pair
work run. The direct existing `run_h` branch remains one run. `UdpErr` itself
stays nonterminal, the resident UDP socket and zero-length datagram behavior
are unchanged, and no Sim adapter, production path, model, API, state, action
or scenario changes. This source alignment awaits independent review before
the exact replays.

### Datagram error after recovery closed the unattached peer — formal enabledness alignment

The next exact replay reaches Trace 69
`InitWith(dgram) -> CtxRelease(h) -> VmmExit -> UdpErr(h)` and the driver
correctly reports that the owner holds no peer socket (raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-fwdErr-first-run-approved-20261026.log`,
lines 16876-16991). At init, the corrected fixture created the real UDP peer
and parked its owner task at `ConnectUdp`. `CtxRelease(h)` found no attached
pair, so the real recovery path took its first quiescence hold. Cancellation
wins the held connect's select in `datagram_associate`, aborts the peer handle,
joins the flow task, removes the owner row and closes its descriptors before
recovery closes the context. `VmmExit` changes no part of that host ownership.
With no earlier peer stimulus the driver retained no observation-only socket
clone; `latest_peer_socket` is therefore correctly empty.

The model agrees that H is ended by `Detached`, `byRel=true`, its context is
released and its owned vsock was closed, but `UdpErr(h)` remains enabled
because its current guard checks only datagram mode, `sideUp`, the prior
`terr` bit and the global fault bound. This contradicts the action's own
meaning, “the connected UDP socket at side s reports an error”: after recovery
the owner has no connected S to report through. `terr(h)` is not projected and
no later `FwdErr(h)` is enabled from the ended phase, so retaining a cloned
`SimSocket` solely to set its error would inject an event into a closed owner
descriptor; silently accepting the action as a driver no-op would instead
hide an invalid formal transition.

DESIGN/formal review confirmed that the existing R5-7 contract is already
complete: `UdpErr` is `SO_ERROR` on an association-owned connected UDP socket.
The model now factors that meaning as `ownsUdpSocket(t, s)`: datagram mode,
the side's context open, phase `unattached` or `live`, and descriptors not
closed. `UdpErr` retains its existing `sideUp`, `!terr` and `canFault` guards.
The predicate is symmetric for H and the guest association. On the guest side,
slot S remains resident after an association ends, but the required end order
disconnects it; residency does not make that ended association a connected
`SO_ERROR` producer. A later association returns to `unattached`/`live` and
can report its own error normally. An ended pair with a queued record may still
have owner-held descriptors, but the forwarder has removed the association's
hook and fixed its terminal cause, so a later socket error is not a new
`UdpErr -> FwdErr` transition. No state, action, run, scenario, driver, Sim,
production path or API changed. The source alignment awaits independent
review before replay.

### Datagram send blocking after owner closure — same owned-socket guard

The guarded replay then reaches the same recovery-closed state as Trace 69
with `UdpBlock(gh)` instead of `UdpErr(h)` (raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-udp-guard-final-20261026.log`).
The driver again correctly finds no peer: recovery cancelled the held connect,
closed and removed H, and released the context.

The existing action meaning settles the object identity. `UdpBlock(d)` is the
target UDP socket's send returning `EAGAIN` because its send buffer/far reader
cannot accept, and the driver maps it to `peer_stop_reading` on that real
association socket. It is not an independent external-reader state after the
local socket is gone. `UdpUnblock(d)` is the dual transition of the same
socket; without the same ownership guard it could clear a stale model-only
`blk` bit after recovery closed the descriptor.

Both actions now require `ownsUdpSocket(cur, dstOf(d))`. This preserves both
sides' pre-attach and live connected-socket stimuli, including the resident
guest slot while it is association-owned. Once an association ends, the guest
slot may remain resident but is disconnected and cannot block/unblock sends
for the ended association; a later association makes it eligible again. The
existing `CFG != dlive` and `!blk` block guard and the existing `blk` unblock
guard are unchanged. `SendDg`, queue/drop accounting, zero-length datagrams,
driver, Sim and production behavior are untouched. No new state, action, run,
scenario or API is added; source awaits independent review.

### Datagram send after source-socket ownership ended — source-side guard

After the shared target-socket guard, exact seed `0xea0ef1a4` reaches Trace 28
`InitWith(dgram) -> VmmExit -> CtxRelease(h) -> Reopen(h) -> SendDg(hg, 2)`
(raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-udpblock-guard-approved-final-20261026.log`).
The old flow is gone, the reopened context owns no pair or UDP descriptor and
the VM is down. The driver correctly finds no H peer socket.

The action's existing meaning pins the missing selector. `SendDg(d, sz)` is
the source application (G) or destination application (H) sending a datagram
that becomes queued on the association-owned source UDP socket, represented by
`dg(d).dq0`. It is not a free external packet retained after the local socket
has disappeared: such a packet has no association queue, and a later guest
slot send belongs to a new association after the resident slot was
disconnected.

`SendDg` now requires `ownsUdpSocket(cur, srcOf(d))`. The source direction is
intentional and differs from `UdpBlock`/`UdpUnblock`, whose target send socket
uses `dstOf(d)`. Every prior SendDg condition remains: source `sideUp`, the
per-direction `dnmax` bound, selected size (zero included), WU queue capacity,
and source-queue drop/count/order accounting. Pre-attach and live association
sends on both sides remain enabled. No driver, Sim, production code, state,
action, run, scenario or API changes; source awaits independent review.

### TCP reset callback — carry out the queued side-B first run

Exact seed `0xee26ec37` later reaches
`InitWith(stream) -> Fin(hg) -> ShutRd(hg) -> CloseUnattached(g) ->
Attach(h) -> HopEnd0(hg) -> TcpRst(h, after_fin=true)` (raw
`.context/netns-density-295/12-01/fwd-pair-ee26ec37-udp-guard-final-20261026.log`).
Immediately before the reset, the attached H pair and its side-B TCP socket
are actual; `HopEnd0(hg)` observed the earlier FIN, so the socket is
`CLOSE_WAIT`. `peer_reset` consequently stores `EPIPE` and calls the installed
side-B `sk_error_report` hook. The callback stores
`SocketError { side: B, errno: EPIPE }` in the live pair and enqueues both
directions, exactly as the model's `TcpRst` action requires.

The driver currently routes H through generic `on_peer`, which applies that
callback, wakes the owner and settles, but global auto-run is disabled; it
never executes the queued pair work. QPROJ therefore honestly reports
`tcp_err=EPIPE` while the pair remains live with no record and only the prior
half-close `{S}`. This is the same private first-run omission as abort-class
VCtl and FwdErr, not a Sim run-order defect: `run_pair` checks the pair's
callback-set `abort` before either direction's already-clean source-end state,
so the first side-B run deterministically ends `SocketError` and preserves
the after-FIN `EPIPE` distinction.

The bounded correction snapshots the actual QPROJ-3
`(ForwarderGeneration, ForwardId)` before H's existing `peer_reset`, applies
the reset, then executes exactly one already-queued side-B run and fails if
none is runnable. With no actual pair, the existing pre-attach reset stimulus
runs without a forced pair run. Side G remains guest stimulus. Before-FIN resets retain
`ECONNRESET`; after-FIN resets retain `EPIPE`; buffered bytes and every direct
Hop run remain unchanged. No Sim adapter, production path, model, API, state,
action or scenario changes. Source awaits independent review before replay.

### Late abort packet after the kernel pair ended — actual-live pair identity

Exact seed `0xea0ef1a4` later reaches Trace 97
`InitWith(stream) -> Write(gh) -> CloseUnattached(g) -> Attach(h) ->
TcpRst(h, before FIN) -> CtxRelease(g) -> VCtl(gh)` (raw
`.context/netns-density-295/12-01/fwd-pair-ea0ef1a4-senddg-guard-approved-final-20261026.log`).
The corrected TCP reset callback already executed the pair's first run. The
Sim forwarder removed and unhooked the pair and queued its one end record, but
QPROJ-3 intentionally retains `GuestFlowPhase::Paired { generation, pair }`
until the owner consumes that record and releases the flow place.

The driver's former `observed_pair` mistook that owner-held identity for a
live kernel pair. Late `VCtl` legitimately updates the retained V_h's peer
bits; with the hook removed it queues no work or second record. Requiring a
run from the QPROJ-only identity therefore fails even though Sim and model
agree on one-ended-pair semantics.

The private correction retains QPROJ-3 as the identity source but returns it
as runnable only when its generation equals the owner's current forwarder
generation and the existing Sim arbiter `forwarder_pair_state(id)` still
reports a live kernel pair. This prevents an id of a later context from being
mistaken for the retained old identity. The cached
`ids`/generation remain unchanged for queued-record projection, so quota/place
ownership and unread end identity are preserved. VCtl, FwdErr and TcpRst then
assert a queued first run only for an actually live pair; a late packet on an
ended/unhooked half updates socket facts without fabricating a run or record.
No projection is filtered, and no Sim, production, model, API or state changes.
Independent review iteration 81 approved the behavior, and iteration 82
closed its source scope without further changes.

### Destination-originated TCP reset — endpoint-specific Sim oracle

The focused Metal rerun (`6b04e060-2c6a-4ec1-8f00-063d401f8040`, seed 8;
`.context/netns-density-295/12-01/sim-destination-reset-focused-20261026.log`)
reproduces the sole seam-unit mismatch: the end record is correctly
`SocketError { side: B }`, but the body expects
`socket_observation().far_end[flow] = Reset` and observes no row.

The objects are distinct. `destination_resets(flow)` is the external TCP
destination injecting RST into the attached local TCP half. `peer_reset`
stores that local half's independent `sk_err` and invokes its error callback;
the forwarder ends the pair with the side-B `SocketError`. During abort,
`abort_pair_sockets` resets any TCP half that does not already carry an error,
but deliberately does not reflect a second reset back through the TCP half
whose peer initiated this one. It shuts the vsock half both ways, so the other
far end — the guest application — receives the required reset.

QPROJ-7 `far_end` is the external destination peer's first terminal event from
the local owner (`SimSocket::peer_end`), or an owner close of an unattached
peer. The destination initiated this reset and received no local owner close,
so `None` is exact. Restoring an unconditional `peer_end=Reset` would conflate
received `sk_err` with a sent terminal event and reintroduce the independence
defect fixed earlier. The existing body now requires the exact side-B
`SocketError { errno: ECONNRESET }` record, `SimShutdown::Both` sent on the
shared V_h to reset the opposite guest far end, and no QPROJ-7 event at the
initiating destination. Its comment and the corresponding DISTILL scenario
wording name those endpoints precisely. This preserves P-69's canonical
mapping — a reset reaches the **other** far end as reset — without inventing a
reset echo to its source. No production, Sim adapter, API or new test body is
added; source awaits independent review.

### Watch records become authoritative only when the owner reads them

The full 40-cell FwdPair run reached
`VmmExitWithWatchRecordInFlightResolvesFromRecord` through `VmmExit`, the
holder's raw error observation and `FwdErr`; it then timed out in
`ReadEof(gh)` (raw
`.context/netns-density-295/12-01/fwd-pair-full40-seed-ee26ec37-final-20261026.log`).
The callback had queued the `SocketError` record, but the driver deliberately
holds end delivery. Production `next_end` consumes the record before
`process_end` publishes the watch outcome, and the Sim publishes the same
resolution in `ends_taken` only when `next_end` takes it. A queued record is
therefore not yet authoritative to the `GuestStream`.

The model's `watchOutcome` now classifies a watch record only when `rec ==
"read"`; both `queued` and `none` retain `wait`, while owner-abort and
context-release precedence are unchanged. The existing named run now proves
the queued state still waits after `FwdErr`, performs its existing
`TakeEnd(h)` action, proves the record is read and the `GuestAbort` outcome is
available, then performs the unchanged `ReadEof(gh)` assertion. No end peek,
automatic delivery, new action, state, scenario, API or production path was
added. Independent source review approved this correction, and the focused
six-watch run passed the VMM-exit cell.

That focused run then exposed the same required read boundary in the existing
base `WatchedStreamSeesGuestResetAsAbort` run: after the guest reset, its
watch record was queued and the raw holder EOF observed, but the run called
`ReadEof(gh)` without letting the owner read the record. The base run now
retains its queued/woken proof, proves the stream still waits after
`HolderEnd(eof)`, performs the existing `TakeEnd(h)`, proves the record is
read and `GuestAbort` is authoritative, then retains the original
`ReadEof(gh)` outcome assertion. `RecoveryEndsHeldStreamWhoseWatchEnded`
inherits this exact base run, so the one correction covers both failed cells.

A static audit found no other missing owner-read boundary. The VMM-exit run
already has `TakeEnd(h)`. `OwnerAbortOfWatchLeavesVhAndStreamReportsAborted`
classifies by `strmAbort`, while the two context-release watch runs classify
by `byRel`; both precedences intentionally resolve without a record read.
Non-watch `ReadEof` runs use their direction state rather than `watchOutcome`.
No action, run, scenario or assertion was removed; this bounded base-run
fallout awaits independent source review.

The same full run reached the exact final owner-death state in
`HostOwnerDeathIsFinal`, then quint-connect could not extract `conf_step` from
the empty terminal state produced by `.then(Reopen("h")).fail()`. The model
now factors `Reopen`'s unchanged guard as `canReopen` and the same run asserts
that predicate false. The canonical action, state sequence and final-death
outcome are unchanged. Independent source review approved this correction,
and the focused six-watch run passed the owner-death cell.

### Native holder read after owner `SHUT_RDWR`

Two existing watch runs intentionally select the two holder-read results the
native V-EV-CTL evidence permits after the forwarder releases a context and
shuts watched V_h both ways: EOF while the socket is still established, or
`ENOTCONN` after its local shutdown transition. P-70 observed both results
under the same release shape. The Sim already proves the real close effect as
`sent_shutdown=Both`, but its deterministic `local_read` has no result for
that collapsed state and returned `WouldBlock` in both runs.

The crate-private `OwnerShutdownRead` input and
`SimGuestFlowIo::owner_shutdown_read` helper now resolve exactly one such
holder read. The helper first requires an actual vsock stream with both local
shutdown bits sent, empty input, no received shutdown, no `sk_err`,
`Established` state, and an underlying `WouldBlock`. It then returns only the
existing step's selected EOF or `ENOTCONN` result. It stores no shadow state
and changes no socket, owner, model or projection fact. Every non-collapsed
read continues through `SimSocket::local_read`, including queued bytes,
received peer shutdown, orphan-sweep error and guest-reset paths.

`FwdPairDriver::holder_end` maps only its existing `k = eof | err` action input
through that helper; it still performs one read and compares the observed
result with the action. A missing owner close remains `WouldBlock` and fails,
so this fixture-only selector cannot conceal the production/Sim close effect.
No public port, API, action, state, scenario or lane was added. Source awaits
independent review before the two named runs execute.

### Default-gate fixture follow-up

The 2,124-body non-CH/non-FwdPair default union completed as 2,107 passed,
16 failed and one timed out (run `22d83634`; raw
`.context/netns-density-295/12-01/default-worker-control-core-exact55-excluded-retry-20261026.remote.log`).
Three bounded DISTILL alignments follow from exact existing contracts:

- The composed shared-network rig's `provision_with_session` creates the
  control connection before it records `process.cid = ActiveCid`; deriving
  physical launch identity inside generic `deliver` therefore panicked in the
  quiescence/recovery bodies. `deliver` now receives the trusted launch from
  each existing creation caller. Control-open creation passes the plan launch;
  flow creation passes the active plan launch. No payload is trusted and no
  production surface changes.
- The `quiesce_conf` randomized main could select formal `FailStop` solely
  after driver-owned `ForeignRestore`, although its driver and module docs
  correctly reject that transition as undrivable. The `ForeignQuiescenceHold`
  production path requires the supervisor itself to receive `ForeignHold` and
  remains covered by the model-checking instances and FHOLD-CHECK supervisor
  cell. Canonical `FailStop` is unchanged; only `CFG == conf`'s randomized
  `recStep` selection excludes a foreign-only transition unless leak/deadline
  is independently drivable. Direct evaluator/model-check actions,
  leak/deadline fail-stop and all formal safety state remain unchanged.
- The address test allocated predecessor `.2`, kept unrelated `.3` held, then
  released `.2`. The accepted pool assigns next-free from its cursor, which
  advanced past `.3`, so successor `.4` is exact; immediate `.2` reuse would
  contradict QPROJ-2 next-fit. The existing body now pins `.4` and the matching
  next CID while preserving release-last, the freed predecessor and every
  teardown/order assertion. Its name/docs say next-fit continuation rather
  than immediate reuse.

The next-fit body's final owner-journal oracle reads `sim_owner.calls()` over
the fixture's full lifetime. The independently approved same-owner bootstrap
now converges that owner before `AppState` construction, so the exact journal
begins with one real `ForwarderOpen` and then retains the authored workload
sequence: three attach/activate pairs and the predecessor's failed plus retry
aborts in their original positions. The oracle now includes that leading
startup operation instead of slicing or clearing it. Every teardown,
held-lease, release-last, exact next address/CID and per-start activation
assertion is unchanged; an extra or missing startup/workload call still fails
the whole-journal equality.

The acceptance `runtime_reconcile_is_idempotent_across_simulated_control_plane_restart`
fixture likewise passed an unconverged test owner into `AppState::new`; it now
uses the same existing precondition as adjacent fixtures: converge the owner,
then pass that exact instance so `flow_ports()` is present.

Iteration 81's provenance audit removed the accidental convergence insertion
from `eval_dispatch_runs_only_the_named_reconciler`; that passing body again
uses its original fresh fixture owner. It retains convergence in the named
restart-idempotence body above. It also retains the convergence in
`build_converged_state`: supplemental review iteration 50 independently
approved that exact site as the “runtime-convergence helper” among the six
`AppState` fixture builders reproduced with missing owner ports. That earlier
approval requires the helper to converge and pass the same owner instance, so
reverting it would undo reviewed fixture alignment rather than narrow this
iteration's accidental edit.

The R5-58 audit-period timeout is a cancellation-unsafe counter in the
existing DNS audit gate, not a supervisor serialization defect. Seed 2
(`4354685564936845354`, two components, offsets 605 ms and 604 ms) advances
the injected clock by the production one-second call bound before releasing
the second held detecting audit. `with_clock_bound` cancels that parked
future. The fixture incremented `pending` before awaiting its semaphore but
decremented it only after permit acquisition, so the cancelled audit left a
stale count and `release_audits` looped forever. Production keeps audit and
repair attempts in separate task sets and does not serialize this overlap.

`PendingAudit` now owns the private counter increment and decrements it on
drop. Normal permit acquisition explicitly drops that guard at the same point
as the former decrement; cancellation drops it with the future. The held
audit, one-period detection, overlapping recovery and call-bound behavior are
unchanged. No timeout, repair ordering, supervisor code, scenario or public
test surface changes. Independent source review approved this correction; the
exact seed replay remains pending.

### Leaked-hold fail-stop precedes a new recovery audit

Focused randomized quiescence seed `0x94db98ed` reached
`LeakHold -> Damage(C1) -> RecQuiesce(C1)` with `leaked=1`. The model started a
new FlowListeners recovery and raised the count from one to two; the composed
production supervisor started none (raw
`.context/netns-density-295/12-01/default-focused-16-after-fixture82-raii83-20261026.log`).
This is the accepted QHOLD priority: at every due periodic audit the
supervisor reads `leaked_quiescence_holds()` first and requests
`LeakedQuiescenceHold` fail-stop before it spawns that audit. The lack of a
recovery was therefore correct production behavior, not a missing fixture
release or repair.

`RecQuiesce` now factors `canRecQuiesce`, which retains its existing
up/idle/damaged guard and additionally requires no leaked hold on normal
instances. The explicit `leakIgnored` hazard bypasses only that added ledger
priority, preserving the accepted counterexample where the supervisor does
not consult the leak. Existing recovery actions remain available after a leak
when the recovery already began: the named dropped-hold run still repairs,
audits and restores its own hold before fail-stop. `ForeignRestore`, its
canonical evaluator run, and the conformance-only exclusion of a foreign-only
fail-stop remain unchanged. No trace was filtered and no production, driver,
fixture, action, state or scenario changed. Independent source review approved
the guard; the replay passed its former `RecQuiesce` failure.

### `OpenFlow` starts from a live control session

The continued seed `0x94db98ed` then reached
`InitWith(0) -> Damage(C2) -> OpenFlow`. The model's `OpenFlow` is a flow on an
already Active allocation; `flowsOpen=0` says only that no flow is held. The
fixture instead created its first allocation and control session lazily inside
`OpenFlow`. Because C2 is the control-listener accept task and the trace had
already damaged it, that late `ControlOpen` could not be claimed and
`activate` correctly returned `ControlSessionPending` (raw
`.context/netns-density-295/12-01/quiescence-seed-94db98ed-after-leak-priority88-20261026.log`).

Every successful `InitWith`, including `fl=0`, now establishes one allocation
through the existing pool, production `provision`, real `ControlOpen` delivery
and `activate -> Activated` path before the trace can damage a component.
`fl=1` then opens its one modeled flow on that allocation. Every successful
`RestartWith(true)` establishes the same invisible allocation/session
precondition for the new process; refused boots establish none. The model
projection still counts only guest flows, dials and datagram associations, so
the baseline control session leaves `flowsOpen=0`. No state is injected and no
owner outcome, action, model guard, scenario, public API or seam changes. The
fix preserves the valid case where the control listener later dies but an
already-live session continues to admit a flow. Independent source review
approved the fixture correction; the replay passed this former activation
failure.

### A periodic audit starts the first damaged idle component

The continued seed then reached both `Damage(C1)` and `Damage(C2)` followed by
`RecQuiesce(C2)`. Production correctly started no C2 recovery: the full audit
selects the first component by enum order, so with both recoveries idle its
finding was C1 (raw
`.context/netns-density-295/12-01/quiescence-full10-after-recrestore91-20261026.log`).
The model's former guard checked only that the chosen component was damaged
and idle, allowing a later finding that production had not selected.

`nextDamagedIdle` now selects C1 before C2 among damaged components whose
recovery is absent, and `canRecQuiesce(c)` requires that exact finding in
addition to the reviewed leak-first guard. Skipping an already-active C1 is
load-bearing: a following audit may then start damaged C2, preserving the
existing overlapping-recovery runs and production behavior. ForeignRestore
remains driver-only stimulus and changes neither ordering nor eligibility; the
direct foreign-hold evaluator coverage is unchanged. No trace filter, action,
state, scenario, driver branch or production change was added by that ordered
finding correction. Source awaits
independent review before replay.

The full default-union replay later exposed the fixture's exact audit-gate
boundary at seed `0x82af4af4`: after `RecRepair(C1)`, C1's post-repair full
audit was pending when randomized `RecQuiesce(C2)` released the shared DNS
gate. `release_audits` necessarily released both the pending post-repair audit
and the detecting audit; production processed the combined result by restoring
C1 and starting C2, so projection was C1 idle, C2 held and count one. The
model step expected an isolated periodic detection, C1 still repaired and
count two (raw
`.context/netns-density-295/12-01/quiescence-seed-82af4af4-after-union-failure-20261026.log`).

This does not prove production serializes its separate audit/attempt task
sets. It proves only that the existing conformance fixture cannot drive an
isolated periodic audit while any repaired episode's full audit is parked on
the same gate. The `quiesce_conf` randomized selector therefore uses
`conformanceRecQuiesceDrivable` to select `RecQuiesce` only when no episode is
`repaired`; the combined state remains reachable through the existing
`RecRedo` / `RecRestore` actions and their first-finding behavior. The
canonical `RecQuiesce` action, direct named runs, every non-`conf`
model-checking/hazard instance and the named held-overlap sequence are
unchanged. This routing is explicit fixture drivability, not a suppressed
production outcome; source awaits independent review before replay.

### A post-repair audit can discover another recovery

The same seed later reached
`RecQuiesce(C2) -> LeakHold -> Damage(C1) -> RecRepair(C2) -> RecRestore(C2)`.
The model restored C2. Production's full post-repair audit selected C1, the
first damaged component in enum order. Because C1 precedes C2, C2's attempt
was unhealthy and counted, while the same attempt result started C1's own
recovery and hold. Projection therefore correctly showed the leaked hold plus
two held recoveries (`cnt=3`, C1/C2 held, C2 attempts 1), not the modeled
single leaked hold (raw
`.context/netns-density-295/12-01/quiescence-seed-94db98ed-after-active-session-fixture89-20261026.log`).

The model now derives the full audit's first finding as C1 before C2. For a
repaired component, a finding at or before it enables the existing
`RecRedo(c)`: the current attempt becomes held and increments once; if the
finding is a different idle component, that component becomes held under one
new count in the same action. A later finding keeps the existing
`RecRestore(c)` result for the current component while starting the later
idle recovery, so the net count is new hold minus restored hold. A finding
whose recovery already exists adds no second hold. The existing `boolLatch`
hazard still drops every hold and reopens early; all leak, foreign-hold and
failed-own-audit runs retain their actions and outcomes. No new action, phase,
state, scenario, driver branch or production change was added. Source awaits
independent review before replay.

### RECROUND1 — model-only recovery-round membership

The full default union now has one remaining failure, randomized quiescence
seed `0x2355dc58`. Its trace starts C1 recovery, then starts C2 while C1's
first repair attempt is already parked, and immediately asks for
`RepFail(C2)`. Production correctly has no C2 repair to fail: the supervisor
starts a round only while its attempt set is empty, so C2 joined after the
round began and waits for C1's result. The existing gate's joined-since-round
rule then admits C2 to the next round without charging it an attempt (raw
`.context/netns-density-295/12-01/quiescence-seed-2355dc58-after-selector93-20261026.log`).

QPROJ-6 exposes only each episode's `Held` / `Repaired` phase and completed
failed-attempt count. In this state both C1 and C2 are `Held` at attempts zero;
those fields cannot distinguish which recovery is in the current round and
which joined afterward. The reverse production schedule is also reachable:
C2 may start first, then a later C1 failure joins while C2's attempt is
parked. Component order therefore cannot reconstruct round membership.

The user approved exact RECROUND1 with the reply **“proceed”** after its
independent DESIGN review; the canonical fold was independently approved with
zero findings. The model now adds only `roundState(c)` with closed values
`outside`, `pending`, `keep` and `restore`. A first `RecQuiesce` starts the
round from the then-held recoveries; a later recovery remains outside.
`RecRepair`, `RepFail`, `RecRedo` and `RecRestore` require a pending member,
so the retained false `RepFail(C2)` is disabled while C1 remains drivable.

Unhealthy and healthy results are collected as `keep` / `restore`. Projected
attempt, phase, count and restore effects wait until the last pending result;
that action applies one barrier to every member. Keeps become held and gain
one attempt, restores become idle and consume one hold, while outside joiners
keep their phase, hold and attempt count. Remaining held recoveries become the
next round's pending members. A finding discovered by a result action takes
its hold but remains outside that completing round. The existing `boolLatch`
hazard retains its first-restore defect.

Canonical `FailStop(c)` also covers a held joiner or collected result waiting
behind another pending member, preserving the existing per-component deadline
and `OwnFair(c)` liveness. The recording conformance selector does not add
this undrivable deadline branch. `RoundStateIsRecovery` constrains the closed
values, requires every member to be a holding recovery, permits `restore`
only for a repaired episode, and requires a pending member whenever a
collected result is visible. The existing overlap run now pins C1 pending/C2
outside after the two quiesce actions and C2 pending after C1's barrier.

`roundState` is deliberately absent from `QuiescenceState`, both state
decoders, QPROJ-6, production state and driver bookkeeping. Existing
`held_repair(c)` / `DriverError::NotEnabled` remains the action-boundary
proof. No production/API/error/fixture/handler/action/run/scenario/check/lane
was added. Source awaits independent DISTILL review before typecheck or replay.

### UDP-slot route-check identity — bind the actual held effect ticket

Random seed `0x1ee49158` reproduces two host associations with the same
destination IP and distinct flows 1/2. Each `GuestReady` reaches and parks at
its own real `RouteType` call. `HostRefuse(2)` currently searches held effects
by `(kind=RouteType, target=destination IP)` and releases the first match, so
it releases association 1's ticket and closes the wrong flow (raw
`.context/netns-density-295/12-01/udp-slots-1ee49158-verbose-final-20261026.log`).
Destination identity is insufficient; choosing by model ordinal would be a
second shadow identity.

The existing private seam already exposes the required trusted operation
identity: `held()` returns `(SimEffectId, SimEffect)` in issue order. The
bounded driver-only correction now:

- immediately before `GuestReady` opens association `i`, snapshots the held
  `RouteType` ticket set;
- after the real owner settles at its new route check, requires exactly one new
  held `RouteType` ticket and require its target to equal the association's
  actual destination;
- retains `i -> SimEffectId` only as stimulus bookkeeping for the later
  `HostPair(i)` / `HostRefuse(i)` release; it is neither projected nor model
  state;
- at release, removes that exact ticket from the driver map, verifies the seam
  still holds the same id/kind/target, then release it. Missing, already-used
  or ambiguous tickets fail the action.

The reproduced identical-destination pair is the negative proof: both tickets
share their effect target yet remain distinct by the seam-issued ids. This
uses no public `GuestFlowIo` parameter, production task-local id, model action,
state, projection, test body or new seam. Independent review iteration 81
approved the behavior, and iteration 82 closed its source scope without
further changes.

- One healthy-supervisor cell observed audits at 1.01 through 9.01 seconds,
  nine calls over a ten-second window. Production schedules the next deadline
  from audit completion (`clock.now() + period`), so fixed-step completion
  delay accumulates. The accepted cadence remains once per one-second period;
  this is a production scheduling defect, not an oracle or clock adjustment.
- Fifty-five failures require real cgroup state: 48 control-plane boots fail
  at `overdrive.slice/cgroup.subtree_control`, and seven worker real-cgroup
  bodies fail the same `/sys/fs/cgroup/overdrive.slice` delegation
  precondition. The broad physical-host invocation did not provide the
  required disposable cgroup namespace. These effects belong in the already
  approved disposable Cloud Hypervisor boundary. DISTILL did not mutate the
  shared physical host, suppress bootstrap, inject a fake into production
  composition, skip bodies, or add a runner/lane.

### CHCGROUP55RUNNER1 — approved existing-GuestKit execution for the 55 cgroup failures

**User-approved 2026-10-26.** After technical review of this exact extension,
the user said: “i approve the rest of the decisions.” The authority covers
only the three catalog entries, one existing-runner group, exact 55-body
selection/pass counts, fresh-guest cgroup-v2 boundary and invocation below.
It adds no runner framework, body/assertion change, physical-host cgroup, or
production/test-support API.

The existing GuestKit machinery is generic enough to execute these unchanged
libtest binaries: `guest-kit-prepare` already builds and stages an
`InGuestLane`; `InGuestTests::prepared` resolves any staged package/target;
and the existing `overdrive-fwd-guest-suite` runner stages dynamic libraries,
boots a fresh scripted Cloud Hypervisor guest, executes a selected libtest
binary, requires an exact pass count, powers the guest off and drops its
work directory. The current lane catalog and runner group list simply do not
yet name the control-plane and worker binaries or mount cgroup v2. There is no
existing command that honestly executes these 55 bodies in a disposable
guest today.

The minimal reviewable extension is two files and the existing group mechanism,
with no new runner or framework:

1. Add three `InGuestLane` entries in `xtask/src/guest_kit_prepare.rs`:
   `overdrive-control-plane--acceptance` and
   `overdrive-control-plane--integration`, both with `integration-tests`, and
   `overdrive-worker--integration` with `integration-tests`. The existing
   JSON artifact discovery and `guest-kit/in-guest/<package>--<target>` paths
   remain unchanged. No core binary joins this guest group because none of the
   55 cgroup failures is in overdrive-core; its corrected pure properties stay
   in the ordinary non-kernel gate.
2. Add one `cgroup-default` group to
   `crates/overdrive-testing/bin/fwd_guest_suite.rs`. It boots one fresh guest
   per staged binary, stages the existing `cat`, `mkdir`, `mount` and shell
   tools with their libraries plus the same GuestKit-verified `ovd_fwd.ko` at
   the unchanged bodies' existing FWD-PREREQ workspace contract path, creates
   `/sys/fs/cgroup`, mounts `cgroup2`, and fails before libtest unless root
   `cgroup.controllers` contains `cpu`, `memory`, `io` and `pids`. It then runs the unchanged
   bodies single-threaded. The two control-plane binaries select the three
   exact `acceptance::job_stop_*` cells and eleven module-qualified
   integration filters with required pass counts
   `8+7+2+1+2+6+5+1+5+3+5 = 45`; the worker binary runs the exact readonly
   cgroup cell and six exact `real_cgroup_fs` cells, each requiring one pass.
   Total required summary is exactly 55 passed, zero failed. Each binary gets
   a fresh guest, so its cgroup tree, processes and kernel state disappear at
   poweroff; the physical host loads no module and creates no cgroup.

The control-plane integration filters are, in order: the eight exact
`backend_discovery_bridge::boot_composition` Tier-3 cells listed in the table,
`built_in_ca_operator_composition::serve_persistent_ca`,
`cgroup_isolation::alloc_scope_has_writable_cpu_weight_and_memory_max`,
`cgroup_isolation::subtree_control_delegation_is_idempotent`,
`concurrent_submit_toctou`, `describe_round_trip`, `idempotent_resubmit`,
`serve_kernel_assigned_vsock_ports`, `server_lifecycle`,
`shared_guest_network_startup`, and `submit_round_trip`. The worker selections
are the exact failing names from `cgroup_manager::write_to_readonly_cgroup_file`
and `real_cgroup_fs::{cgroup_kill_terminates_pids,controller_validation,
procs_pid_movement,pseudo_file_synthesis,rmdir_auto_reap,
subtree_control_ebusy}`. Exact test names, rather than a broad worker prefix,
avoid silently adding unrelated real-kernel effects.

| Staged binary | Libtest filter | Required passes |
|---|---|---:|
| `overdrive-control-plane--acceptance` | `acceptance::job_stop_idempotent::stop_on_already_stopped_job_returns_already_stopped_outcome` (`--exact`) | 1 |
| same | `acceptance::job_stop_intent_key::stop_writes_separate_intent_key_preserving_spec` (`--exact`) | 1 |
| same | `acceptance::job_stop_unknown::stop_on_unknown_job_returns_404` (`--exact`) | 1 |
| `overdrive-control-plane--integration` | `integration::backend_discovery_bridge::boot_composition::attach_mode_fallback_emits_structured_event_on_dummy_iface` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::boot_composes_ebpf_dataplane_and_attaches_xdp_to_both_ifaces` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::boot_refuses_when_client_iface_does_not_exist` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::boot_refuses_when_earned_trust_probe_fails` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::boot_refuses_when_iface_has_no_ipv4_address` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::boot_resolves_host_ipv4_via_getifaddrs_on_client_iface` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::boot_succeeds_when_earned_trust_probe_round_trips_backend_map` (`--exact`) | 1 |
| same | `integration::backend_discovery_bridge::boot_composition::graceful_shutdown_detaches_xdp_and_removes_bpffs_pin` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::refuse_to_start_does_not_remint_the_root` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::serve_first_boot_generates_seals_and_persists_root` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refuses_on_absent_kek` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refuses_on_corrupted_envelope` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refuses_on_wrong_kek_material` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::serve_restart_adopts_same_root_no_remint` (`--exact`) | 1 |
| same | `integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refusal_causes_are_pairwise_distinct` (`--exact`) | 1 |
| same | `integration::cgroup_isolation::alloc_scope_has_writable_cpu_weight_and_memory_max::` | 2 |
| same | `integration::cgroup_isolation::subtree_control_delegation_is_idempotent::` | 1 |
| same | `integration::concurrent_submit_toctou::` | 2 |
| same | `integration::describe_round_trip::` | 6 |
| same | `integration::idempotent_resubmit::` | 5 |
| same | `integration::serve_kernel_assigned_vsock_ports::` | 1 |
| same | `integration::server_lifecycle::` | 5 |
| same | `integration::shared_guest_network_startup::` | 3 |
| same | `integration::submit_round_trip::` | 5 |
| `overdrive-worker--integration` | `integration::cgroup_manager::write_to_readonly_cgroup_file::write_to_readonly_cgroup_events_propagates_real_eacces` (`--exact`) | 1 |
| same | `integration::real_cgroup_fs::cgroup_kill_terminates_pids::cgroup_kill_terminates_every_pid_within_two_seconds` (`--exact`) | 1 |
| same | `integration::real_cgroup_fs::controller_validation::cpu_weight_out_of_range_returns_einval` (`--exact`) | 1 |
| same | `integration::real_cgroup_fs::procs_pid_movement::writing_pid_to_cgroup_procs_moves_the_process_into_scope` (`--exact`) | 1 |
| same | `integration::real_cgroup_fs::pseudo_file_synthesis::cgroup_events_appears_at_mkdir_time` (`--exact`) | 1 |
| same | `integration::real_cgroup_fs::rmdir_auto_reap::empty_scope_rmdir_succeeds_kernel_reaps_pseudo_files` (`--exact`) | 1 |
| same | `integration::real_cgroup_fs::subtree_control_ebusy::no_internal_process_rule_returns_ebusy_dispatches_to_typed_variant` (`--exact`) | 1 |

The approved invocation reuses the native-metal GuestKit setup and
never calls host `kmod-load`:

```text
cargo xtask metal run -- bash -lc 'cargo xtask guest-kit-prepare && \
  OVERDRIVE_FWD_GUEST_GROUP=cgroup-default cargo run \
  -p overdrive-testing --bin overdrive-fwd-guest-suite \
  --features integration-tests'
```

No execution-support source was changed before approval. The approved source
package now adds exactly the three catalog entries and the one
`cgroup-default` group. The group resolves each staged binary through
`InGuestTests::prepared`, boots one fresh guest per binary, stages `cat`,
`mkdir`, `mount` and `sh` with their shared libraries, mounts cgroup v2,
requires `cpu memory io pids` before libtest, and runs the exact table above
single-threaded. It also reuses the runner's existing
`workspace_forwarder_module` packaging: the exact `GuestKit.forwarder_module`
image built for the guest kernel is staged at
`/home/ubuntu/overdrive/target/kmod/<guest-release>/ovd_fwd.ko`, the path the
unchanged `require_forwarder_module` check already resolves. GuestKit retains
its normal `/lib/overdrive/ovd_fwd.ko` copy and the guest owns module load;
the physical host neither loads nor changes a module. This was required after
Metal run `19734` stopped before body 1 with `ForwarderPrerequisiteError::NotBuilt`
at that workspace path. Every invocation retains its individual required-pass
oracle, and the manifest additionally requires their sum to be exactly 55.
Each guest is powered off before the next binary. No body/assertion,
module build/version, fake prerequisite path or physical-host cgroup/module
state changes. Independent final proof review covers this packaging fallout
with the exact 55-pass execution. Ownership remains DISTILL/test support in
the two named files; the production crafter does not absorb or redesign this
extension.

The next guest reached body 1 but its unchanged production bootstrap failed
`CgroupBootstrap(SubtreeControlWriteFailed { source: ENOENT })`. The removed
read-only diagnostic in
`.context/netns-density-295/12-01/cgroup-default-readonly-diagnostic.log`
proved that the fresh cgroup-v2 root advertised `cpu memory io pids`, while
root `cgroup.subtree_control` was empty; consequently the body-created
`overdrive.slice` had an empty `cgroup.controllers`. Linux 7.0
`cgroup_subtree_control_write` returns `-ENOENT` when an enable request is
absent from `cgroup_control(cgrp)`
(`vendors/linux/kernel/cgroup/cgroup.c:3584-3593`).

The guest-only preparation now verifies the four root controllers are
available, writes exactly `+cpu +memory +io +pids` to the fresh guest root's
`cgroup.subtree_control`, then reads it back and requires all four before
libtest. It creates no `overdrive.slice` or service path, enables no other
controller, and changes no body. The unchanged production bootstrap remains
responsible for creating and configuring its own slice. The shell uses only
its builtin `printf` plus the already-staged `cat`; no new tool, host mutation,
module action, API, lane or mechanism is introduced.

After root controller delegation passed, the unchanged first body reached the
approved fd probe and refused `FdBudget { required: 376_832, available:
65_536 }`. The generic guest peer intentionally keeps its existing 65,536
limit for every other lane. For `cgroup-default` only, the same existing
`run-with-modules` path now execs the staged libtest through the already-staged
guest shell. The diagnostic guest's effective capabilities include
`CAP_SYS_RESOURCE`, so that child raises hard and then soft `RLIMIT_NOFILE` to
376,832, reads both values back, fails below the approved bound, and only then
`exec`s the unchanged binary. The existing loopback-module load and exact
libtest summary/pass-count oracle are retained in the wrapper. The unchanged
phase-1 host-fact read is the production-port verification of the inherited
value. This is process-local inside each disposable guest: it changes no
physical-host limit, guest-global sysctl, generic guest-peer default,
runtime/product API, CLI, body, lane, module, controller or production fd
formula. No third support file is changed.

The next unchanged full-boot body then reached node-health publication and
rejected GuestKit's default hostname `(none)` as a `NodeId`. The same fresh-
guest preparation now writes the valid literal `overdrive-cgroup-test` to
that guest's `/proc/sys/kernel/hostname`, reads it back, and refuses before
libtest unless it matches exactly. This is the disposable guest's UTS state;
it does not change the physical host UTS, add a production fallback or edit a
body/API/CLI/lane. It uses the shell builtin `printf` and the already-staged
`cat`, so no tool is added.

A bounded read-only audit of the other explicit full-boot base prerequisites
found one further existing GuestKit module contract. The next boot reached
`MtlsBoot(KtlsArmRoundTrip)` and the real kTLS probe failed `ENOENT` because
the cgroup group neither staged nor loaded the stock guest-kernel `tls.ko`.
The already-approved `mtls-relay` group has the exact required packaging:
`host_module_file(kit, ..., "net/tls/tls", TLS_MODULE_GUEST_PATH)` and that
path in `run-with-modules` ordering. Each fresh cgroup binary now reuses the
same GuestKit-verified file/path/load sequence; its temporary module workdir
lives through guest completion. No new module build, version, path or host
effect is introduced.

With kTLS present, the next real boot failed the hardwired stale-rule sweep at
`chain-exists` with `EPROTONOSUPPORT` (raw log
`.context/netns-density-295/12-01/cgroup-default-55-with-kTLS-current.log`).
The sweep performs `NETLINK_NETFILTER` `GETCHAIN` before the fixture's Sim
intercept replacement participates. The existing guest-suite `core` group
already stages the exact guest-kernel dependencies in order: `nfnetlink`, then
`nf_tables`. Each cgroup guest now reuses only those two verified
`host_module_file` artifacts and paths before `tls.ko`, in that order. Once
`nf_tables` answers the fresh-boot chain lookup with structural `ENOENT`, the
sweep is a clean no-op. The selected acceptance composition replaces the
later mTLS intercept with `SimMtlsIntercept`, so no reject expression/module
is reached; the core group's four reject modules are deliberately not copied.

The remaining current composition needs no speculative module/service setup:
GuestKit already mounts proc, sysfs and devtmpfs and loads the prepared
forwarder; the acceptance fixtures replace mTLS intercept and guest DNS with
their existing Sim adapters while leaving the real kTLS enforcement probe as
the explicit stock-module prerequisite just supplied. The cgroup group now
provides only the reproduced base facts: workspace module identity, cgroup
root delegation, process fd limit, valid UTS hostname and stock TLS module.
Network, stores, configuration and service-specific state remain owned by
each unchanged body/production composition; the runner does not invent
services or precreate production paths.

The next full runner execution reached the unchanged HTTPS client but failed
to resolve `localhost` (raw log
`.context/netns-density-295/12-01/cgroup-default-55-with-netfilter-current.log`).
GuestKit creates a fresh empty `/etc` and stages no hosts file. The cgroup
guest preparation now writes exactly the standard loopback mappings
`127.0.0.1 localhost` and `::1 localhost` to that disposable guest's
`/etc/hosts`, reads the file back, and fails before libtest unless both exact
lines match. This supplies only libc/reqwest's ordinary localhost substrate;
it changes no production hostname or DNS behavior, generic GuestKit/peer,
physical host file, body, API, CLI, lane or network service.

The selected existing `backend_discovery_bridge::boot_composition` bodies
then exposed two equally concrete fresh-root prerequisites. Their unchanged
`require_cap_net_admin()` invokes `ip`, but the cgroup group had not staged
that already-used GuestKit tool, so spawn failure was misreported as absent
capability. Their `BootFixture` also creates a BPF temporary directory below
`/sys/fs/bpf`, while the fresh image had no bpffs mount. The cgroup extras now
add only the existing `ip` tool with its libraries. Guest preparation creates
`/sys/fs/bpf`, mounts the guest's own bpffs there with the already-staged
`mount`, and verifies `/proc/mounts` reports type `bpf` at that exact path
before libtest. GuestKit already supplies the required dummy module; no module,
service, skip, body, production behavior or physical-host mount is added.

The next execution reached those bodies: 4/10 passed and the other six failed
only when their unchanged `ip link add ... type veth` found no registered veth
link kind (raw log
`.context/netns-density-295/12-01/cgroup-default-55-with-ip-bpffs-current.log`).
The installed pinned guest kernel packages veth as `drivers/net/veth.ko`; GuestKit
already proves/stages stock modules through `host_module_file`. Each fresh
cgroup guest now stages that exact verified image at the runner's module path
and loads it in the existing ordered `run-with-modules` list before libtest.
Absence fails closed in `host_module_file`. No other network/netfilter module,
host load, body, skip, production behavior or API is added.

With veth present, the broad backend prefix ran 10/10 while the approved
manifest row correctly required eight (raw log
`.context/netns-density-295/12-01/cgroup-default-55-with-veth-path-current.log`).
Source inspection proves the two extras are the non-Tier-3 pure/default cells
`boot_refuses_when_dataplane_config_section_missing` and
`dataplane_boot_error_iface_addr_resolution_display_contains_remediation`.
The runner now replaces only that broad prefix with the eight fully-qualified
Tier-3 names in the table above, each `--exact` and requiring one pass. The two
extras remain in their normal test binary/default gate; no body or oracle is
removed. The approved cgroup total remains exactly 55 and cannot be satisfied
by increasing the prefix count.

The next exact-manifest run reached all seven
`serve_persistent_ca` bodies in one libtest process: five passed, while two
failed only because redb reported `DatabaseAlreadyOpen` after an earlier body
in that same process (raw log
`.context/netns-density-295/12-01/cgroup-default-55-exact-approved-current.log`).
Their normal Nextest boundary is one process per test. The cgroup runner now
lists the same seven approved function names as seven `--exact`/one-pass
invocations. Each existing `exec_cgroup` call execs a fresh libtest process in
the same fresh guest and staged binary, restoring that process-isolation
boundary without changing a body, data directory, body set, guest, subtotal
or the 45/55 oracle.

The subsequent run proved that the broad `describe_round_trip` prefix now
selects seven passing bodies although CHCGROUP55RUNNER1 names the six real-
server bodies (raw log
`.context/netns-density-295/12-01/server-shutdown-cgroup55-20261026.log`, line
484). The extra
`describe_service_returns_discriminated_shape_with_vip` body drives the
handlers over an in-process `AppState`; it neither calls `spawn_server` nor
crosses the real cgroup bootstrap and remains covered by the normal gate. The
runner now lists the six bodies from the original broad cgroup-failure receipt
(`default-worker-control-core.log`, lines 5660-5665) as `--exact` one-pass
entries. Their subtotal remains six; no seventh result is hidden or counted.

A read-only audit of every remaining broad selector compared current source
with that same original failure receipt. The two
`alloc_scope_has_writable_cpu_weight_and_memory_max` bodies, one
`subtree_control_delegation_is_idempotent` body, two
`concurrent_submit_toctou` bodies, five `idempotent_resubmit` bodies, one
`serve_kernel_assigned_vsock_ports` body, and five `submit_round_trip` bodies
exactly equal their approved prefix counts, so those selectors remain
unchanged. `server_lifecycle` contains those five real-server bodies plus
`graceful_shutdown_propagates_worker_failure_without_a_retry_capability`,
which uses the supplied Sim composition and is already covered by the normal
gate. The runner consequently selects the five names from the original cgroup
failure receipt (lines 5679-5683) as exact one-pass entries.
`shared_guest_network_startup` now contains six bodies, but only the three
approved real-bootstrap bodies appear in the original cgroup failure receipt:
`serve_boots_through_the_pinned_order_with_the_guest_network_composed`,
`an_rlimit_nofile_below_the_owner_budget_refuses_boot`, and
`a_failed_beacon_bind_refuses_boot` (lines 5684-5686). The runner selects those
three by exact name. The VIP-prefix, node-address and scripted step-8 refusal
bodies remain in the normal gate. This batch alignment preserves the exact
45-control-plane / 55-total manifest without changing, skipping or deleting
any test body.

The next exact-manifest run passed all three acceptance bodies and reached the
unchanged `serve_boots_through_the_pinned_order_with_the_guest_network_composed`
body, where `compose_vm_driver` refused while stat-ing `/dev/kvm` with
`KvmUnreachable(ENOENT)` (raw log
`.context/netns-density-295/12-01/server-shutdown-cgroup55-20261026-r2.log`,
line 484). The body supplies a healthy `SimVmm`, but production composition
still derives the confined VMM's supplementary device group from the real KVM
node; a missing node is therefore a guest-substrate failure before the body's
oracle.

The already-approved 11-05 `nested-boundary` GuestKit path supplies this exact
substrate on the same native Metal host. Its retained run stages and loads, in
dependency order, `ccp`, `irqbypass`, `kvm`, `kvm-amd`, `vhost_iotlb`, `vhost`
and `vhost_vsock`, then records real devtmpfs `/dev/kvm` and
`/dev/vhost-vsock` nodes as root:root mode 0660; all seven boundary bodies pass
(`.context/netns-density-295/11-05/evidence/metal-disposable-nested-boundary.log`,
lines 452-464). The cgroup guest now reuses that single module manifest,
stages it through the existing fail-closed `host_module_file` helper, loads it
before cgroup preparation, applies the same 0660 mode and prints both nodes'
actual uid/gid/mode before any libtest process. The shared manifest also
continues to back `nested-boundary`, preventing the two approved disposable-
guest substrates from drifting. No fake device, chosen gid, host module load,
VM adapter override, production fallback, test-body change or runner lane is
introduced.

The input-domain, owner-port, quiescence teardown and cached-request changes
above are bounded existing-test/support alignment. Production owner,
supervisor and worker defects were handed to the original crafter with their
exact entry paths. Cgroup and quota groups need their approved execution
boundaries, not body changes. No focused rerun was started while the formal
source and Metal lease sequence remained independently coordinated.

## Apalache startup-heap packaging alignment

The required 279-check record run `2580067-1793100798.247` completed 271
checks and stopped eight FwdPair hazard checks as startup `ToolError`s. Each
failed check made two attempts with the fixed 2,048 MiB Apalache server heap;
all 16 servers raised `java.lang.OutOfMemoryError: Java heap space` in
`Tokenizer$TokenIterator.pullQuotedString` while Typesafe Config parsed the
generated module, before pass 0 or TLC. The paired failures are the two imports
in `fwd_pair_hazards.qnt`, `_2`, `_3`, and `_5`. The one-import wrappers `_4`,
`_6`, and `_8` passed under the same heap. Peak group RSS remained 4,046–4,578
MiB under the 6,912 MiB reservation, so this is the compile server's fixed heap
rather than host or group exhaustion. The prior passing record
`2043112-1792964018.889` used the same wrappers, checks, and tools; only the
imported `fwd_pair.qnt` had the smaller earlier hash.

The first independently reviewable falsification package splits only the
wrapper pair containing `pair_bugConsumeOnShortSend`:

- `fwd_pair_hazards.qnt` retains the exact
  `pair_bugConsumeOnShortSend(OFF = Set("consumeOnShortSend"), CFG = "stall",
  REC = false)` instance.
- New append-only wrapper `fwd_pair_hazards_10.qnt` contains the exact moved
  `pair_bugEofAsFin(OFF = Set("eofAsFin"), CFG = "ok", REC = false)` instance.
- Only `fwd-pair-hazard-eof-as-fin-peer-loss.spec` changes, from
  `hazard/fwd_pair_hazards.qnt` to
  `hazard/fwd_pair_hazards_10.qnt`. Its name, main, invariant, TLC backend,
  expected violation, and CI setting remain byte-for-byte unchanged. The
  consume check changes no field.

This changes formal packaging only. It adds no model state, action, scenario,
property, check identity, check, execution lane, or memory allowance; the
subsystem remains 279 checks. After independent source review, the bounded
falsification commands were:

```text
cargo xtask metal run -- cargo xtask quint check --subsystem guest-flow-owner --name fwd-pair-hazard-consume-on-short-send --jobs 1
cargo xtask metal run -- cargo xtask quint check --subsystem guest-flow-owner --name fwd-pair-hazard-eof-as-fin-peer-loss --jobs 1
```

Both deliberately omitted `--record`. Each exited 0 in 43 seconds under the
same 2,048 MiB server heap without OOM and found its unchanged expected
violation. Receipts are
`.context/netns-density-295/12-01/fwd-pair-hazard-consume-on-short-send-jobs1-no-record-packaging-falsification97-20261027.log`
and
`.context/netns-density-295/12-01/fwd-pair-hazard-eof-as-fin-peer-loss-jobs1-no-record-packaging-falsification97-20261027.log`
with their adjacent `.exit` files.

That falsification authorized the identical remaining packaging split:

- `_2` retains `pair_bugCheckThenHook`; new `_11` contains the unchanged
  `pair_bugEndedAttachGracefulClose`, and only the ended-attach check's `spec`
  points to `_11`.
- `_3` retains `pair_bugReleaseKeepsPairs`; new `_12` contains the unchanged
  `pair_bugLossyEndRecord`, and only the lossy-record check's `spec` points to
  `_12`.
- `_5` retains `pair_bugWatchAbortShutsVh`; new `_13` contains the unchanged
  `pair_bugDropWithoutAbort`, and only the drop-without-abort check's `spec`
  points to `_13`.

Every moved module keeps its exact `OFF`, `CFG`, and `REC` instantiation. All
six involved checks keep their identity, main, property, TLC backend, expected
violation, CI setting, bounds, and timeout; the subsystem still contains 279
checks. This completed packaging source requires independent review before the
full jobs-3 record and evidence verification.

## RED assessment

The completed primary `quint_owner_flows` command established that all 46
activated tests collect and reach the production owner. It did not establish a
semantic RED: every case stopped at
`HostSharedGuestNetworkOwner::flow_observation`'s
`Not yet implemented -- RED scaffold` panic before quint-connect compared the
first projected owner state with the model. That result is useful activation
and boundary evidence, but it must not be the semantic RED receipt.

No upstream test-support gap exists. The smallest already-authored semantic
ordering RED is:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(a_boot_opens_and_probes_the_forwarder_before_any_listener_or_route)' --no-fail-fast --no-capture
```

The current pre-cut implementation lets `converge_shared` return successfully
without issuing an effect. This test reaches its existing
`expect("the forwarder opened")` oracle and fails because `OpenForwarder` is
absent. The assertion also pins `OpenForwarder` before `BindListener` and
`GatewayRoute`. It therefore fails on the missing STEP 12-01 behavior, through
the production owner composition, without accepting a production scaffold
panic as semantic RED.

The adjacent existing typed-error oracle can corroborate the exact correction
when needed:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one)' --no-fail-fast --no-capture
```

It scripts the existing forwarder failure cases through both phase-1 entry
points and fails because the current no-effect phase-1 body returns `Ok(())`
instead of the approved typed `Forwarder` refusal. Its existing assertions
also cover context release and the absence of later boot effects.

The focused commands should be run after the STEP 12-01 production deletion
removes the two obsolete enum variants. Before that coordinated deletion, the
newly exact exhaustive taxonomy intentionally cannot compile against the old
15-variant pre-cut enum, which would be a transient contract-alignment build
failure rather than the required semantic RED.

## Verification status

- `git diff --check` passes for the aligned Rust files, and
  `rustfmt --edition 2024 --check crates/overdrive-worker/src/vm_driver.rs`
  passes for the whole beacon implementation and fixture file. A native macOS
  run of `cargo clippy -p overdrive-worker --all-targets --features
  integration-tests -- -D warnings` stops in the Linux-only
  `linux-keyutils` dependency before compiling `overdrive-worker`; the
  coordinated Metal rerun remains the executable proof that the five
  `await_holding_lock` findings are closed.
- Repository search finds no remaining reference to the deleted scratch types,
  helper, property, or obsolete error variants in these two owned test paths.
- Repository search finds neither deleted fail-stop cause, the old ten-cause
  length/range, nor supervisor-kill wording in the aligned fail-stop/E23 test
  paths.
- Repository search finds all ten `SharedGuestNetworkOwner` implementations
  with the required leaked-count method: the production host owner, the two
  direct test/simulation ledger owners, and the seven existing delegating
  adapters. The trait has no default implementation.
- The approved GuestKit runner extensions pass targeted `rustfmt --check`
  and `git diff --check`. The existing mTLS relay group retains its seven-test
  oracle. The new cgroup manifest statically totals 3 acceptance + 45
  control-plane integration + 7 worker integration bodies = exactly 55, and
  every exact/prefix filter and individual pass count matches the canonical
  CHCGROUP55RUNNER1 table. No build, guest boot or Metal job was run before
  independent source review; the approved `cgroup-default` invocation is the
  post-review real-kernel gate.
- No test command was used as a gate while the shared worktree still contains
  the pre-cut production enum; the commands above are the bounded semantic RED
  handoff to the STEP 12-01 crafter.
