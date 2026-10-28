# STEP 13-04 DISTILL leg-C fixture alignment

## Status

**ALL 23 PACKAGE-UNION CALLER FIXTURES CORRECTED — ready for independent
DISTILL review iteration 6 before the broad package-union replay.**

The first correction changes only the existing test-local
`vsock_legb_dial::node` fixture shared by
`vsock_constant_guest_prefix_rules`. The execution-support correction extends
only the existing GuestKit `cgroup-default` internal manifest and staging so
the already-authored STEP 13-04 bodies run in its existing three disposable
guests. It changes no production source, acceptance-test body, assertion,
Contract Shape declaration, pending marker, scenario, lane, model, DES event,
or commit. The body count changes only at the runner selection boundary:
the existing 55-body identity remains explicit and 18 existing STEP 13-04
bodies are added, for 73 expected passes.

## Contract and scope

The authoritative STEP 13-04 outcome is the roadmap's firewall single cut and
feature-delta obligations R5-3, R5-25, R5-43, and V-19:

- ADR-0172 keeps the constant output reject and prerouting drop after the
  leg-C divert, so non-diverted guest-prefix traffic reaches neither a host
  socket nor another route;
- every packet of a leg-C-diverted connection reaches leg C in both
  directions; and
- the existing Tier-3 cell
  `a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways` must complete
  its mutual-TLS handshake and observe one established leg-C socket on the
  registered VM tuple.

ADR-0125 and ADR-0184 remain production concerns of STEP 13-04. This fixture
alignment neither implements nor alters them. Feature-delta D24/D24a concern
service-VIP handling and are outside this correction.

## Failure evidence and boundary

The original crafter retained the diagnostic summary at
`.context/netns-density-295/13-04-legc-diagnostic-summary.md`. It records the
pre-correction active cell reaching the successful mutual-TLS handshake and
then failing its unchanged socket-table oracle:

```text
[rules-leg-c-remote][handshake] 2 certificates; ss []
leg-C accepted the remote's connection on the VM's tuple: []
```

That diagnostic ran in the Metal host network namespace. It is causal
diagnostic evidence only and cannot be used as final native proof. The durable
runner retained no raw stdout receipt for it; the summary states that
limitation explicitly. This correction does not repeat that unsafe execution.

The initial raw GuestKit receipt is
`.context/netns-density-295/13-04-metal-guest-kit-f6b8464bd04dc1f93b760008.log`.
GuestKit prepared the lane and launched its disposable Cloud Hypervisor guest,
then Cloud Hypervisor refused creation of its API server socket with
`PermissionDenied` under the existing confined execution identity. The next
raw receipt,
`.context/netns-density-295/13-04-metal-guest-kit-rootfs-permission-41591c6f534f97e5f8d2643f.log`,
showed the same traversal defect on the per-launch rootfs parent.

The original crafter corrected that existing GuestKit boundary privately in
`guest_kit.rs`. The Metal transient service used `umask 0027`, so the
platform-owned staging, runtime and rootfs-parent directories were
`root:root 0750`; the production VMM identity (uid/gid 4200) could not traverse
to its already-chowned child API socket or rootfs clone. GuestKit now assigns
only the existing confined group 4200 to those parents and mode `0710`:
owner access and group traversal, with no group read/write or access for
others. The VMM still runs as uid/gid 4200 with the existing supplementary
device-owner groups, Landlock grants and seccomp filter. No production launch
identity or confinement surface changed.

The corrected existing `cgroup-default` manifest then passed all 55 of its
previous bodies in the same three disposable guests. Durable run
`873eb46d2df0c195e547c3ec`, retained at
`.context/netns-density-295/13-04-metal-guest-kit-cgroup-default-final-873eb46d2df0c195e547c3ec.log`,
records 45 harness invocations, 55 passed bodies, zero failures and exit 0.
That run establishes the recovered disposable-guest boundary; it predates the
18-body STEP 13-04 selection extension below. Neither receipt is evidence
against the leg-C source correction, and neither authorizes a physical-host
nft/netns replay.

## Causal proof

The failure is a test-fixture configuration defect with a complete production
caller path:

1. `Node::start` constructed `SimGuestStreamDial::new()` and erased it behind
   the existing `GuestStreamDial` port when composing
   `HostMtlsEnforcement`.
2. `SimGuestStreamDial::new()` has the typed default
   `Refuse(GuestDialError::NotServing)` for every unscripted target.
3. The old `Node::serve_vm` called the production
   `MtlsInterceptWorker::start_alloc`, which registered
   `(VM_ADDR, PORT)` in `inbound_destinations`, but it never configured the
   test-side guest dial to represent the already-serving VM application.
4. The remote connection therefore passed the production nft leg-C divert and
   completed `HostMtlsEnforcement`'s mutual-TLS server handshake. Production
   `inbound::establish_guest` then called
   `guest_dial.dial(orig_dst, DialPurpose::Relay)` through the accepted port.
5. The fixture returned `NotServing`; production correctly reset leg C and
   returned `MtlsEnforcementError::GuestDial`. By the time the unchanged cell
   read `ss`, no established socket remained.

Changing production to keep the socket after `NotServing`, adding a fallback,
or weakening the empty-`ss` assertion would violate the existing fail-closed
guest-dial contract. No public interface or typed-error gap exists.

## Exact fixture correction

`Node` now retains the concrete existing `Arc<SimGuestStreamDial>` that it
passes to `HostMtlsEnforcement` through `Arc<dyn GuestStreamDial>`.
`serve_vm` performs its existing production `start_alloc` call and then
scripts only that exact registered `SocketAddrV4` as:

```rust
SimDialScript::AcceptOpen { reply: Vec::new() }
```

This represents a serving VM application whose guest-stream direction stays
open while the test observes the established leg-C connection. Target-specific
scripting is load-bearing: every undeclared or unregistered tuple retains the
sim port's default typed `NotServing` refusal. In particular, the existing
`a_marked_dial_to_an_unregistered_tuple_is_refused_and_reaches_no_host_socket`
cell remains fail-closed; no default-accept behavior was introduced.

The production path remains unchanged:

```text
remote peer
  -> production nft leg-C divert
  -> HostMtlsEnforcement mutual-TLS handshake
  -> GuestStreamDial::dial(VM_ADDR:PORT, Relay)
  -> existing bidirectional leg-C/GuestStream relay
```

## Existing GuestKit runner extension

The recovered GuestKit boundary did not select the STEP 13-04 cells. The
existing `overdrive-fwd-guest-suite` already stages the
`overdrive-control-plane` and `overdrive-worker` integration binaries and its
selectable `cgroup-default` group already boots exactly three disposable
Cloud Hypervisor guests with private cgroup-v2 and bpffs mounts. The correction
keeps that group, those three guests, their CID slots, the staged binaries,
the scripted-peer executor and the `GuestRun::exec_cgroup` result oracle.
It appends exact filters after the existing manifests in the already-running
control-plane-integration and worker-integration guests. No selectable group,
binary, lane, CLI, hook or alternate executor was added.

The fail-closed count accounting has three independent checks:

| Manifest identity | Harness invocations | Expected passed bodies |
| --- | ---: | ---: |
| Existing `cgroup-default` baseline | 45 | 55 |
| Existing STEP 13-04 extension | 18 | 18 |
| Combined existing group | 63 | 73 |

Every extension filter is exact and expects one pass. The 18 bodies are:

| Existing binary | Existing authored bodies | Count |
| --- | --- | ---: |
| `overdrive-control-plane--integration` | `boot_member_clear_refusal::{a_rejected_boot_member_clear_refuses_the_composed_boot_with_its_own_cause, a_boot_member_clear_that_leaves_members_refuses_with_the_members_it_observed, a_boot_member_clear_that_commits_after_the_refusal_publishes_nothing}` | 3 |
| `overdrive-control-plane--integration` | `shared_element_cleanup_failure::{shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address, shared_element_cleanup_failure_readback_failed_retains_retirement_and_address}` | 2 |
| `overdrive-control-plane--integration` | `shared_guest_network_startup::a_failed_step8_table_write_refuses_boot_and_the_restart_converges` | 1 |
| `overdrive-worker--integration` | `mtls_intercept_install::{shared_program_absence_create_readback_idempotence_and_guard_drop, shared_program_replaces_only_listener_targets_and_preserves_foreign_complement, shared_program_valid_wrong_target_observation_is_non_mutating}` | 3 |
| `overdrive-worker--integration` | `netns_density_shared_owner::{stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner, member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member, policy_route_loss_is_repaired_with_live_members_and_the_prior_guard_is_relinquished, a_differently_targeted_program_is_never_rewritten}` | 4 |
| `overdrive-worker--integration` | `shared_intercept_members::each_deleted_intercept_object_is_restored_exactly_with_live_allocations` | 1 |
| `overdrive-worker--integration` | `vsock_constant_guest_prefix_rules::{a_host_local_connection_to_a_guest_prefix_address_is_rejected_at_once, a_host_local_datagram_to_a_guest_prefix_address_fails_eperm, a_remote_connection_to_an_unassigned_guest_prefix_address_is_dropped, a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways}` | 4 |

This covers the complete STEP 13-04 native manifest: the constant-prefix
positive and negative cells, all three shared-program install/read-back cells,
the four activated shared-owner cells, the real-kernel deleted-object repair
cell, all three boot-member clear refusal cells, both awaited convergent
release cells and the step-8 boot-write refusal/restart cell. Pure source-local
sim and Quint bodies remain in their ordinary userspace test route; adding
them to a real-kernel guest would not strengthen their evidence.

The existing worker guest image now also stages `nft`, `ss` and `uname`, plus
the running kernel's exact tproxy/reject expression modules and their declared
IPv4/IPv6 defragmentation and reject dependencies. They are copied into and
loaded only inside that disposable guest by the existing `run-with-modules`
payload; the two control-plane guest payloads retain their prior tool and
module set. The physical host remains launch substrate and receives no nft,
netns, cgroup or module mutation from this suite.

## CH73 fixture boot correction

The first complete 73-body replay reached the extended manifest through the
intended disposable boundary. Durable run `c13d2413f408e84f38d6453c` booted
`cgroup-control-plane-acceptance`, preserved the 55-body baseline, and then
reached the first new control-plane body,
`shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address`.
The retained handoff is
`.context/netns-density-295/13-04-ch73-failure.md`; the durable record is
collectable by the same run id. The body failed before its cleanup-fault
stimulus:

```text
internal error: entered unreachable code:
a successfully booted shared owner has reported its bound flow ports
at crates/overdrive-control-plane/src/action_shim/mod.rs:1468:13
```

The production invariant is correct. `provision_and_inject_netns` receives the
flow ports captured from the composed `AppState` and treats their absence as
unreachable after a successful boot. `AppState::new` deliberately captures
ports only when its injected shared guest-network owner already reports them.
`SimSharedGuestNetworkOwner` deliberately reports `None` until its first
successful `converge_shared`; on success it records `ForwarderOpen`, marks
itself converged and reports the accepted fixed ports.

The existing `Harness::boot` already used one `Arc<RecordingGuestNetwork>` as
both the `AppState` owner and the later action-shim provisioner, and one
`GuestNetworkExecWiring`. It started the shared mTLS owner and opened that
wiring's EXEC gate, but omitted the accepted shared guest-network convergence
step. The correction awaits `network.converge_shared()` on that exact owner
after the reconciler runtime is registered, before constructing `AppState`
and before `wiring.supervisor().open_after_boot()`.

This is the same established fixture order used by
`mtls_install_fail_closed::SeamFixture` and the action-shim admission fixture:

```text
start shared mTLS owner
  -> converge the exact shared guest-network owner
  -> construct AppState from that owner and wiring gate
  -> open that wiring's EXEC gate
  -> dispatch allocation actions
```

No state is assigned manually and no fallback port is fabricated. The
`RecordingGuestNetwork::converge_shared` implementation delegates through the
existing `SharedGuestNetworkOwner` port to its same inner sim instance. It
does not append to, clear or slice the fixture's allocation-effect/lease
journal. The cleanup model remains unarmed during boot and predecessor start;
each authored body still arms its `LowerCleanupFault` only immediately before
the normal stop, with all failure, retry, retirement, address-retention,
unrelated-member and complete-journal assertions unchanged. The manifest
therefore remains 55 + 18 = 73 bodies.

## CH73 guest-route precondition correction

Durable replay `ada3ff7709ff6cde475adca0` confirmed the prior boot correction:
both `shared_element_cleanup_failure` bodies passed all 17 unchanged verdicts.
It then reached the worker guest and failed the first constant-firewall cell.
The raw receipt is
`.context/netns-density-295/13-04-metal-guest-kit-ch73-network-failure-ada3ff7709ff6cde475adca0.log`;
the bounded diagnosis is
`.context/netns-density-295/13-04-ch73-network-route-failure.md`:

```text
[rules-host-tcp][connect][+0ms] Network is unreachable (os error 101)
assertion failed: left NetworkUnreachable, right ConnectionRefused
```

This is a valid no-route outcome under V-19, but it is not the authored cell's
subject. That cell requires a routable main-table lookup so the production nft
output hook can project its exact `ECONNREFUSED`; the UDP sibling likewise
requires the output hook's exact `EPERM`. The qualified host had a node default
route. The disposable worker guest had only loopback and no main-table path to
`100.95.0.0/16`, so route lookup failed before the hook. Accepting
`ENETUNREACH` in those bodies would make the constant output reject vacuous and
weaken their exact errno contract.

The existing runner now separates the eight route-independent STEP 13-04
worker bodies from the four unchanged constant-firewall bodies. Only around
the latter suffix it:

1. proves `ip route get 100.95.255.201` fails before setup;
2. loads the stock dummy driver already carried by the GuestKit image;
3. creates test-owned `ovd-gk-route`, assigns documentation address
   `192.0.2.1/32`, and installs a main-table default route with that preferred
   source and a fixture-specific metric;
4. proves route lookup selects exactly `dev ovd-gk-route src 192.0.2.1`;
5. runs only the two host-local output-reject bodies with their exact errno and
   no-host-socket assertions;
6. removes the default route and dummy link, then proves the target is again
   unroutable; and
7. runs the remote-drop and bidirectional leg-C bodies in that restored
   no-node-default-route environment.

The dummy device is a guest-local sink: it gives local output the same
routable lookup precondition as the qualified node without giving packets an
external path. The bounded default exists only for the two with-route
host-local cells. It adds no guest-prefix `/16`, no gateway `/32`, no
local-table entry and no host address inside the guest prefix, so it does not
pull forward STEP 13-05's production gateway convergence. The no-route state
exists before and after those two cells; the earlier replay retains its
observed `ENETUNREACH` evidence. Existing with/without-default-route,
serve-up/down and gateway-exclusion obligations are neither collapsed nor
relabelled.

On a routed body failure the runner still attempts the explicit route/link
cleanup before returning the authored failure. If cleanup also fails, dropping
the disposable Cloud Hypervisor guest destroys the entire test-owned network
namespace and is the fail-closed complement. No route or link is created on
the physical Metal host. The same 18 exact STEP 13-04 filters and combined
73-body accounting remain unchanged.

## CH73 worker filesystem compatibility correction

Durable replay `603782e98f2f5fd2ee6b8821` proved both host-local output cells,
the two cleanup-failure bodies with all 17 verdicts each, and the step-8
boot-write refusal/restart cell. After route cleanup it reached the first
remote/no-default-route body and failed only its namespace-handle precondition.
The raw receipt is
`.context/netns-density-295/13-04-metal-guest-kit-ch73-netns-failure-603782e98f2f5fd2ee6b8821.log`;
the bounded diagnosis is
`.context/netns-density-295/13-04-ch73-netns-path-failure.md`:

```text
precondition: netns /var/run/netns/ovt-rules-remote-drop-00210 opens:
No such file or directory (os error 2)
```

`PeerNetns::create` correctly uses `ip netns add`; iproute2 publishes the
namespace handle under `/run/netns`. Its unchanged `spawn_in` then opens the
traditional Linux spelling `/var/run/netns/<name>`. A normal Linux rootfs has
`/var/run -> /run`; the minimal scripted GuestKit image did not. Changing the
fixture path would make the same authored topology depend on which rootfs runs
it, so the compatibility belongs to the existing guest payload.

Before any worker body runs, the runner now prepares `/run/netns`, proves
`/var/run` is neither an existing path nor a symlink, creates the standard
`/var/run -> /run` alias, and proves both the symlink's canonical destination
and `/var/run/netns` directory. Only the existing worker image gains the
already-installed `ln` userspace tool. The alias lives for that disposable
guest's lifetime and is destroyed with its rootfs; no physical-host path or
mount changes.

The immediately related prerequisite audit was kept to the selected 18 bodies:

| Surface used by the selected bodies | Existing disposable-worker support |
| --- | --- |
| `ip netns`, `/run/netns`, `/var/run/netns` | `ip` was already staged; `/run/netns` and the standard alias are now prepared and verified before the worker manifest. |
| nft program/read-back and policy routing | `nft` and `ip` are staged; nfnetlink/nf_tables, tproxy/reject dependencies and veth are loaded only in the worker guest. |
| socket-table and kernel identity evidence | `ss` and `uname` are staged with their shared-library closures. |
| lock/lease state | The rootfs already provides `/tmp`; `KernelStateLock` and `TestCidrLease` use their unchanged `/tmp` paths. |
| cgroup, bpffs, procfs, sysfs and devices | The existing group mounts private cgroup-v2 and bpffs, boots procfs/sysfs/devtmpfs, sets the existing RLIMIT and preserves the confined device permissions. |
| dummy route precondition | The stock dummy module already carried by GuestKit is loaded only for this worker guest and removed with the guest. |

The unselected later `mtls_intercept_install` cells that invoke `sysctl` or
other tools are outside the exact 18-body manifest; no speculative staging was
added for them. No authored namespace path, body, assertion, filter, group,
guest, lane or count changed.

## Four-package union caller-fixture audit

The required four-package union then ran at durable Metal run
`673f34b829f67b06388dd5bc` (`QUINT_SEED=0x94db98ed`): 1,715 selected,
1,692 passed, 23 failed and 148 skipped, exit 100. The complete receipt is
`.context/netns-density-295/13-04-metal-package-union-remediation-run.log`.
The raw traces do not support one undifferentiated “baseline” or
“missing-flow-port” label:

| Exact failure partition | Count | Raw failure | Causal classification |
| --- | ---: | --- | --- |
| `e09_v2_failure_stream_overlap_spike` | 2 | `action_shim/mod.rs:1468`, absent bound flow ports | Owner existed but had never converged; mTLS owner also had not started and EXEC opened early. |
| `e10_vm_early_exit_spike` | 1 | `action_shim/mod.rs:1468` | Same missing boot sequence in its one composed fixture. |
| `netns_density_node_admission` | 1 | `action_shim/mod.rs:1468` | Exact `LeaseLedger` owner entered `AppState` before convergence and EXEC opened first. |
| `netns_density_reclaim` | 2 | `action_shim/mod.rs:1468` | Exact `RecordingOwner` entered `AppState` before convergence. |
| `service_backend_projection` | 11 | `action_shim/mod.rs:1468` | Each `World` used a fresh unconverged owner and an unstarted mTLS owner, with EXEC already open. |
| `vm_finalize_failed_ownership_spike` | 2 | `action_shim/mod.rs:1468` | Same missing boot sequence in its one fixture. |
| `driver_neutral_allocation_replacement` | 3 | Preserved D-UP-15 assertion sees `Provision` / `Teardown` workload-netns calls for VM specs | These tests call the legacy process-netns dispatcher, not the accepted guest-owner path. They never reached line 1468. |
| `service_kind_vm_terminal_invariant` seed 25717 | 1 | accepted probe event timeout | Its exact owner was already converged before `AppState`; no line-1468 panic exists in the raw trace. |

The 19 direct missing-port cases were migrated only through their existing
fixtures. Each now starts its existing shared mTLS owner, converges the same
guest-network owner later stored in `AppState`, constructs `AppState` so it
captures that owner's ports, and only then opens the same EXEC wiring. No owner
instance, pool, seed, scheduler, fault injection, lifecycle action, assertion
or body count changed.

Their probe fixtures also had to express the already-accepted VM transport:
wildcard VM probes use the existing `GuestStreamDial` port once a transport is
correctly assigned. Existing `SimGuestStreamDial` scripts now mirror each
fixture's existing TCP refusal or HTTP status stimulus; no new port or adapter
was added. The readiness-threshold fixture advances the existing producer once
per expected Pass streak step, preserving the producer-row contract rather
than re-reading one row as three passes.

### Replacement-fixture placement

The three replacement failures were a different, directly reproduced caller
error. Their external `overdrive-sim` fixture called
`dispatch_with_network_provisioner`, which supplies `guest_network: None` to
the private dispatch core. Under `integration-tests`, that legacy branch
allocates an exec/process netns and therefore records the exact
`Provision`/`Teardown` calls the D-UP-15 oracle forbids for VMs. Production
calls the same private core with the guest owner, pool, bound flow ports and
EXEC gate present.

The accepted private core also takes the existing custom
`MtlsInterceptLifecycle`, which the three scenarios need to observe a no-op
successor stop and injected predecessor-stop failures exactly. The public
AppState test helper hard-wires the concrete worker and cannot preserve that
oracle. Rust privacy also prevents the external integration-test crate from
calling the private core. The bounded correction therefore moves exactly the
three existing bodies and their private recording fixtures into
`action_shim::replacement_acceptance`, beside the private core. It adds one
source-local `cfg(test, integration-tests)` module declaration and no exported
helper, public method, parameter or production mechanism.

| Before | After | Count |
| --- | --- | ---: |
| `overdrive-sim::driver_neutral_allocation_replacement successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` | `overdrive-control-plane action_shim::replacement_acceptance::successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` | 1 |
| `overdrive-sim::driver_neutral_allocation_replacement successor_and_cleanup_outcomes_follow_the_ratified_precedence_table` | `overdrive-control-plane action_shim::replacement_acceptance::successor_and_cleanup_outcomes_follow_the_ratified_precedence_table` | 1 |
| `overdrive-sim::driver_neutral_allocation_replacement rejected_successor_publication_fully_unwinds_without_immediate_second_proposal` | `overdrive-control-plane action_shim::replacement_acceptance::rejected_successor_publication_fully_unwinds_without_immediate_second_proposal` | 1 |

The old integration binary retains its two unrelated bodies. All lifecycle,
no-op stop, exact-ID order, failure precedence, retained retry state,
row/index, one-proposal and zero-workload-netns assertions moved unchanged.
The source-local fixture converges its existing `TestSharedOwner`, captures
its bound flow ports, then opens the existing EXEC wiring before calling the
private guest-aware core. `RecordingNetwork` stays connected to the legacy
port and its zero-call assertion remains live.

### Probe-fixture alignment

The two non-line-1468 failures were also fixture-specific and were not hidden
under an owner-startup label:

- E10 reached the guest-stream HTTP parser. Its accepted row carries the
  canonical reason `HTTP 302 (redirect not followed)`, while the obsolete
  host-side `SimHttpProber` script expected `HTTP 302`. The fixture now keeps
  its exact 302 outcome and asserts the canonical production reason. Its
  bounded logical-clock wait observes the already-live startup producer; the
  terminal, reclamation and occurrence assertions are unchanged.
- Seed 25717 called the single-tick helper repeatedly. Every call creates a
  fresh producer state at `ProbeRunSeq::FIRST`; after the baseline readiness
  Pass, the strict `run_seq` LWW store correctly rejected the equal-sequence
  Fail and emitted no event. The existing supervised producer now drives the
  same Pass/Fail/Pass trajectory through `SimGuestStreamDial`, retaining one
  monotonic producer state. The late accepted Pass is an already-started
  `Pending` guest dial completed after the terminal owner cancels the
  supervisor, preserving the authored late-result race without seeding a row
  or restarting a terminal supervisor. The refusal reason is the typed guest
  dial label `refused`.

### Focused evidence

- `5fce29b32fbe103885c9869b`: after owner/EXEC migration, 14 of 19 selected
  cases passed; five reached distinct probe-fixture outcomes rather than line
  1468.
- `a5ef2684206cb67c212f2b97`: Cargo check and Clippy passed; two consumer
  trajectories passed after existing guest-stream stimulus alignment.
- `d1b6495bda8e82c726263634`: readiness producer-streak correction passed;
  seed-25717 and E10 still failed their original probe-row/event oracles.
- `109495dc838a5eb1d64e646d`: the three source-local replacement scenarios
  passed with every original oracle; the two separately diagnosed probe
  fixtures still failed before their final alignment.
- `c3576f41acf3b5318fe3bdc5`: E10 proved one real guest dial completed and
  isolated the only mismatch to its obsolete redirect reason.
- `1b1b503a2262a402160fda2b`: E10 and seed 25717 both passed after their
  production-producer fixture alignment.

These focused receipts close the exact 23-failure inventory without an
exclusion, relaxed oracle, public seam or production fallback. The full
four-package union remains reserved for the original crafter after independent
DISTILL review iteration 6.

## Committed journal-oracle audit (`254cb4a98`)

The crafter removed `Step::InterceptInstalled` from four expected journals in
`mtls_install_fail_closed.rs`. That alignment is honest and needs no source
reversal:

- the exact fixture allocation spec declares `service_ports: Vec::new()`;
- STEP 13-04 deleted outbound set installation, so that spec now installs zero
  allocation nft elements;
- `JournalIntercept::install_inbound` still records
  `Step::InterceptInstalled` for every actual destination install; and
- `steps_for` still retains that step (collapsing only adjacent duplicates),
  so any unexpected inbound installation remains an extra journal entry and
  fails all four exact arrays.

The change therefore removes only an expectation for an operation the
destination-only, zero-port fixture no longer performs; it does not suppress
or clear observed journal entries. Focused durable run
`2b8daeedbd89f5500bb292ed` passed the four exact bodies. Their ordering,
activation, recovery wait, cancellation and release assertions remain intact.

## Preserved coverage

| Existing evidence | Preserved contract |
| --- | --- |
| Host-local TCP into the guest prefix | Exact `ECONNREFUSED` within the existing bound; wildcard listener accepts nothing. |
| Host-local UDP into the guest prefix | Exact `EPERM` at `send`; wildcard socket receives nothing. |
| Remote connection to an unassigned guest-prefix address | Exact timeout/drop; wildcard listener accepts nothing. |
| Remote connection to the registered VM tuple | Existing trust roots, client chain, mutual-TLS handshake, presented-certificate assertion, exact `VM_ADDR:PORT`/remote socket-tuple assertion, release ordering, and cleanup. |
| Same-node registered tuple in `vsock_legb_dial` | Existing production `enforce` call and leg-B/leg-C socket-table assertions; the exact serving target now has an honest guest-stream fixture. |
| Same-node undeclared tuple in `vsock_legb_dial` | Default `NotServing` remains; the existing constant-rule `ECONNREFUSED`, latency, and no-host-accept assertions are unchanged. |

No gateway exclusion, default-route behavior, exact tuple, security boundary,
cleanup complement, nft read-back, or errno assertion was removed or relaxed.

## Mechanical verification

The following source checks passed after the correction:

- `cargo fmt --all -- --check`;
- scoped `git diff --check` for the fixture, existing runner, staging manifest
  and this handoff;
- POSIX `sh -n` accepted the exact `/var/run -> /run` preparation script;
- Metal durable run `f014f7fe86ec4da1081c6f48`:
  `cargo check -p overdrive-worker --test integration --features integration-tests`;
- Metal durable run `32114f92ed04dc30ea9668f7`:
  `cargo clippy -p overdrive-worker --test integration --features integration-tests -- -D warnings`;
- Metal durable run `0674ac52277fa1b0196ef774`: checked the existing
  `overdrive-fwd-guest-suite` binary and `xtask`, restaged the existing
  in-guest binaries, and used their libtest `--list` surface to verify that
  every one of the 18 exact manifest names resolves;
- Metal durable run `c7b2a147bb442f59c0a5a604`: final Cargo check and
  Clippy with `-D warnings` passed for the existing runner binary and `xtask`;
- Metal durable run `489cf650ec149c7dfc6b227b`: read-only `modinfo`
  confirmed the pinned kernel's tproxy/reject expression dependency graph,
  including the linked IPv4 and IPv6 tproxy/defragmentation modules;
- Metal durable run `16f853ecdecaa7674eb0054b`: read-only staging
  preflight confirmed that the pinned running kernel supplies every exact
  tproxy/reject/defragmentation module path added to the disposable image;
- Metal durable run `f86ac248f0df8706ea0c7c74`: scoped Cargo check passed
  for the `overdrive-control-plane` integration binary after the CH73 fixture
  correction;
- Metal durable run `de69af6d9807559116f847d1`: scoped Clippy with
  `-D warnings` passed for that integration binary;
- Metal durable run `39c1f5e6464fa1d377862ea1`: final scoped Cargo check
  and Clippy with `-D warnings` passed for the existing guest-suite runner
  after the route-lifetime correction;
- Metal durable run `109495dc838a5eb1d64e646d`: all three moved
  replacement scenarios passed with their unchanged assertions;
- Metal durable run `1b1b503a2262a402160fda2b`: the E10 and seed-25717
  probe scenarios passed through the accepted guest-stream producer; and
- Metal durable list run `a8fbd70d40605eb26fb4da62`: collected exactly the
  three mapped `action_shim::replacement_acceptance` bodies once; and
- Metal durable run `faa335932e3ff7ff4cc87a54`: final Cargo Clippy with
  `-D warnings` passed for all `overdrive-control-plane` and `overdrive-sim`
  test targets under `integration-tests`.

The focused Sim tests and compilation checks are userspace work on Metal. No
post-correction broad package-union or disposable-guest replay has run, by
design. Final nft/netns/cgroup/kernel evidence must wait for this fresh
DISTILL review and then run only inside the recovered existing disposable
Cloud Hypervisor guest boundary. A Metal command prefix alone is not
authorization to mutate the physical host kernel.

## Review and replay handoff

The prior review artifact approved the earlier bounded fixture corrections but
does not approve the four-package caller migrations. The same independent
reviewer should record iteration 6 against the combined package and verify:

1. executable changes are confined to the test-local `vsock_legb_dial::node`
   fixture, GuestKit traversal support, and the existing guest-suite runner's
   staging/selection;
2. the script is target-specific and occurs only for the exact tuple passed to
   `serve_vm`;
3. `SimGuestStreamDial`'s default remains typed `NotServing`, preserving every
   negative and undeclared-tuple path;
4. all 18 selected cells resolve to existing bodies and every body/assertion
   remains unchanged;
5. the old 55-body identity and the new 18-body identity fail closed
   independently before the combined 73-body result is accepted;
6. `cgroup-default` remains one selectable group using the same three guest
   boots and existing staged integration binaries;
7. the added tools and modules exist only in the disposable image and are the
   dependencies of the existing nft/tproxy/reject tests;
8. the group-4200 traversal change is limited to `0710` on platform-owned
   parents and preserves uid 4200, supplementary device groups, Landlock and
   seccomp; and
9. `Harness::boot` converges the exact `RecordingGuestNetwork` instance before
   `AppState` captures its ports and before the exact wiring's EXEC gate opens;
10. the cleanup fault remains armed only at the authored post-start stop
    boundary and the allocation/lease journal is neither cleared nor sliced
    around boot; and
11. the test-owned default route is absent before and after the exact two-cell
    host-local suffix, uses only a guest-local dummy sink while active, and
    never installs the STEP 13-05 gateway/local route;
12. a routed-cell failure preserves its original error while still attempting
    route/link cleanup, with disposable-guest teardown as the complement; and
13. the worker rootfs owns one standard `/var/run -> /run` alias for its
    lifetime, validates its canonical destination before running tests, and
    leaves the authored `PeerNetns` path unchanged;
14. the immediately related tool/module/filesystem audit is bounded to the 18
    selected bodies and adds no speculative support for unselected cells; and
15. no production fallback, API, owner, lane, scenario, model, or relaxed
    oracle was added;
16. the exact 19 line-1468 callers converge the same owner captured by
    `AppState`, start the same mTLS owner and open the same EXEC wiring only
    after those boot preconditions;
17. the three replacement bodies moved once from the external Sim binary to
    the source-local action-shim module, with the old bodies absent and the new
    IDs collected exactly once;
18. the moved bodies call the existing private guest-aware dispatch core,
    retain the custom lifecycle and `RecordingNetwork`, and preserve every
    lifecycle/no-op-stop/order/failure/retry/D-UP15 assertion;
19. seed 25717 uses its existing supervised producer for monotonic `run_seq`
    and models the terminal-late Pass as an already-started pending guest
    dial, without seeding a row or restarting a terminal supervisor;
20. E10's redirect assertion is the canonical reason returned by the
    production guest-stream HTTP parser; and
21. commit `254cb4a98` removed only the four expectations for absent
    zero-port installs while the observed journal still fails on any actual
    unexpected `InterceptInstalled` operation.

After approval, the original crafter should replay the unchanged four-package
union with the mapped three test IDs and the unchanged STEP 13-04
`cgroup-default` selection. The receipts must retain the exact 55+18
disposable-guest accounting and the four-package selected/pass/skip totals;
no skip, exclusion or moved body may disappear from accounting. This DISTILL
task does not write DES phases or commit the source.
