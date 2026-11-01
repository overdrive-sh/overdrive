# DELIVER review — STEP 14-01 walking skeleton on metal (V-6)

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-01` |
| Review ID | `STEP14-01-DELIVER-POSTCOMMIT-01` |
| Review type | Fresh isolated post-commit implementation review |
| Parent | `822b533950fcfeb51f72444b574fe1f05b93cb97` |
| Implementation commits | Cumulative `fe22d261dfd9cbca444611c125ca3f9d8bdb98e9`, `c9f2f6d6975b236fa4a049093550d3ea8dcf4242`, remediation `bc832a3e0e6b031502165b338ab8c4604efbdf10`, DES commit `aa25cc910380b03e5df7b767773916b4a2105f43` |
| Iteration | 1 historical; 2 current re-review |
| Final verdict | **APPROVED — iteration 2**; iteration 1 **NEEDS_REVISION** is preserved below |
| Open findings | 0 current; historical F-01 closed by the approved contract correction and executed remediation |

This review applies the current workspace `AGENTS.md`, the applicable Rust,
development, testing, verification and workflow rules, the software-crafter
review dimensions, the complete
`.context/netns-density-295/dispatch-14-01.md`, roadmap step `14-01`,
feature-delta V-6 / V-LEGB-SERVE, and the accepted ADRs 0187, 0190–0192 and
0197–0211. The native Markdown review convention takes precedence over the
installed review skill's legacy YAML-only output.

No production source, test source, DES log, roadmap, design artifact, spike,
host state or commit was changed by this review. The user-owned dirty
`AGENTS.md` was preserved. This review artifact is the only reviewer-owned
file.

## Authority and review boundary

The roadmap contract requires:

1. the activated walking skeleton to round-trip TCP and UDP off-node, include
   an empty datagram, observe the node source address and prove the VMM is
   gone;
2. the five existing `guest_flow_through_serve` bodies to prove the owner
   pair, end, token/session, Probe and Relay watch paths; and
3. V-6 through production `serve` + `deploy`: gateway:53 guest DNS, mesh-VIP
   TCP/mTLS, service-VIP UDP, inbound leg-C → leg-S → dial, TCP and HTTP
   probes, and the V-LEGB-SERVE no-uplink result.

The three bullets above are the preserved iteration-1 authority snapshot. The
current controlling criterion 3 replaces only its Service-VIP UDP-to-VM action
with the user-approved #310 deferral; iteration 2's traceability and verdict
apply the corrected contract.

The implementation boundary is limited to the two production corrections named
by the dispatch:

- `overdrive-init` must dial the token's `GuestTransportToken.ports.beacon`
  port under GUEST-PORTS / ADR-0217; and
- the pending guest TcpConnect child must retain a `ReadHangup` until `Paired`
  so EVMOD-PAIR can attach a `CLOSE_WAIT` half and forward its queued bytes and
  FIN.

The existing GuestKit support package was independently DISTILL-reviewed and
approved through five iterations. Its PID-1 production-init fallback, nested
Cloud Hypervisor support, console observer, private R5-41 Service fixture and
E6 compiler-only fallout are therefore reviewed here for honesty and scope,
not redesigned.

## Commit and DES integrity

Both implementation commits have the required parent chain and preserve the
Marcus author. Each has exactly one `Co-Authored-By: Codex
<codex@openai.com>` trailer and one `Step-Id: 14-01` trailer. No Claude,
Anthropic or generated attribution appears. The implementation commit changes
the production owner/init paths and the approved test-support/marker files;
the follow-up commit only appends the COMMIT event to the execution log.

The step's DES history is complete and chronological:

| Phase | Result | UTC timestamp |
|---|---|---|
| RED | FAIL | 2026-10-29 13:12:43 |
| RED | PASS | 2026-10-29 14:59:24 |
| GREEN | FAIL | 2026-10-29 15:18:15 |
| GREEN | PASS | 2026-10-30 15:13:20 |
| COMMIT | PASS | 2026-10-30 15:14:31 |

`des-verify-integrity` reports no missing event for `14-01`; its unrelated
workspace result reports future steps `14-02`–`14-05` and historical `04-02`–
`04-04` incomplete. Roadmap-only validation reports `Roadmap format OK`.
Those unrelated traces were neither changed nor used as evidence for this
step.

`git diff --check` reports only intentional two-space Markdown hard breaks in
the inherited approved DISTILL review artifact. No source or executable test
file has whitespace errors.

## Contract and implementation assessment

### GUEST-PORTS / beacon implementation

`read_transport_token_with` parses the accepted seven-field token and maps
parser failures to `InitError::GuestTransportSetup` at
`crates/overdrive-init/src/main.rs:477-488`. `connect_beacon` then reads the
same token and calls `connect_beacon_once(token.ports.beacon)` at
`main.rs:580-585`; the socket address is constructed from that port at
`main.rs:616-622`. No production path in `overdrive-init` uses the old fixed
beacon constant. Native execution of the public `serve` + deploy composition
passes kernel-assigned ports through the real nested workload VM, which is
direct evidence that the token and host listener agree.

The implementation adds no public method, type, enum variant, parameter, wire
field, configuration field or ownership surface. `GuestTransportToken` and
`GuestHostPorts` remain crate-private test/production-init types with the
accepted field names.

### EVMOD-PAIR pending-child behavior

At `crates/overdrive-init/src/guest_flow/owner.rs:671-693`, a
`GuestHangup::ReadHangup` on the pending outbound child returns while retaining
the `PendingOutbound` entry. The existing `Paired` path then calls
`attach_outbound`; the forwarder performs the attach-time state check and
queues the child's early bytes and FIN. A reset or other hangup still removes
the pending flow, closes both halves and sends the existing `Abort` control
message when the host has not already closed its half. The host-vsock
`ReadHangup` path remains the existing silent-close path. This preserves
ADR-0212's `ESTABLISHED` / `CLOSE_WAIT` acceptance and ADR-0210's ended-half
refusal without a new owner state or public API.

The walking skeleton's TCP application writes its request and calls
`shutdown(Shutdown::Write)` before reading the reply
(`crates/overdrive-testing/bin/guest_peer.rs:168-195`), so the passing native
walking-skeleton receipt exercises the early-FIN path through production
serve/deploy rather than a fixture-only state.

### Typed semantic error mapping

The step adds no new semantic error set. The existing approved mappings remain
complete:

| Designed failure | Approved implemented representation | Evidence |
|---|---|---|
| Missing or duplicate `overdrive.transport=` token | `GuestTransportSetupError::TokenMissing` / `TokenDuplicate`, wrapped by `InitError::GuestTransportSetup` | `transport/mod.rs:255-268`, `main.rs:481-488` |
| Malformed or duplicate-reserved port field | `GuestTransportSetupError::TokenMalformed { field: TransportTokenField::{Beacon, Stream, Control, Extra} }`, wrapped by `InitError::GuestTransportSetup` | `transport/mod.rs:75-156` |
| Beacon socket creation failure | `InitError::Socket(Errno)` | `main.rs:616-623` |
| Beacon connect failure after the bounded retry | `InitError::Connect(Errno)` carrying the last kernel cause | `main.rs:588-610` |
| Guest transport setup operation failure | The existing stage-specific `GuestTransportSetupError` variants (`DummyDevice`, `AddressAdd`, `FwmarkRule`, `FwmarkRoute`, program/module/listener variants and `Forwarder { source: ForwarderError }`) | `transport/mod.rs:213-245,255-378` |
| Pending child half-close before `Paired` | No failure: `ReadHangup` is retained as the approved EVMOD-PAIR state; reset/error follows the existing close/`Abort` path | `owner.rs:671-693` |
| Attach-time ended half | Existing `ForwarderError::HalfEnded { side, errno }`, counted as `forwarder_attach_ended`, with the existing two-half cleanup | `owner.rs:561-593` |

No required distinction is flattened into a string, no generic error is used
as a substitute for an approved semantic cause, and no new public error
variant was invented.

## Test integrity and support scope

The exact acceptance-body comparison against the parent shows only the six
pending-marker removals plus the explicitly approved private R5-41 helper
change. The walking skeleton body and all of its assertions are unchanged.
The five control-plane bodies and assertions are unchanged; only
`a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted`
now calls the approved private `deploy_hold_service` helper. That helper keeps
the same workload id, resources, VM command, exact `hold` arguments, kernel,
rootfs and peer while using an existing Service listener shape so production's
workload-kind replacement contract is not contradicted by a terminal Job.

All six live bodies retain `/// CONTRACT_SHAPE: bounded-change.`. There is no
assertion weakening, test deletion, test skip, mock-dominated SUT, fixture
end-state seeding, or runtime-skip bailout in the changed test boundary.
The E6 fixture change removes the deleted `OutboundTproxyInstall` wrapper and
updates the current shared-table identity from the stale three-set/five-rule/
three-rule shape to the accepted one-set/three-prerouting/nine-output shape.
It does not run or reclassify the two known E6 runtime failures.

The approved support changes stay test-local:

- `guest_peer.rs` selects the existing embedded production init when the
  existing workload image carries it, while preserving explicit
  `overdrive.testinit=` precedence and scripted-peer images;
- `vsock_support.rs` observes the existing console file before production
  removes the `VmRunDir`, retaining only bytes already exposed by production;
- `fwd_guest_suite.rs` and `guest_kit_prepare.rs` reuse the existing
  cgroup-default GuestKit executor and exact identities, with nested kernel
  effects inside disposable outer Cloud Hypervisor guests; and
- no override seam, new production flag, test lane, scenario, model, selector,
  executor or public API was added.

## Executed evidence

### Native GuestKit boundary

The final native receipt is
`.context/netns-density-295/14-01/raw/410a62758dd8e62f9f9d6418/`.
Its metadata pins the source digest
`d3fe8c43abf2447f87621f8d6b96ea863525d95c27f7e5784190ee36b7fbe6ce` and the
implementation base `822b533950fcfeb51f72444b574fe1f05b93cb97`. The raw output
hash is `df6b8f0439e8908dd7bd6862967732906245276d0809f05cdb1fff5eb9a273f4`;
the terminal hash is
`22e48c84b552885091dce2c89793901e770027dbaa8ea6f678a1b55eef586e12`.

Independent accounting of the raw output finds 107 successful libtest
summaries declaring exactly 117 passed, 0 failed and 0 ignored bodies. The
runner's selected manifest reconciles as 111 retained bodies + 1 CLI body + 5
control-plane bodies. The six exact STEP 14-01 identities each start once and
finish successfully. The line-oriented runtime-skip detector finds zero
recognized runtime-skip diagnostics. The three retained cgroup-default guests,
the CLI nested guest and the control-plane nested guest all complete inside
the GuestKit boundary.

The six exact bodies are:

- `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy`
- `integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve`
- `integration::guest_flow_through_serve::a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted`
- `integration::guest_flow_through_serve::the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`
- `integration::guest_flow_through_serve::a_dropped_probe_stream_is_an_abort_at_the_guest_through_serve`
- `integration::guest_flow_through_serve::a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve`

The walking-skeleton assertions prove the production TCP round trip, one
non-empty UDP round trip, node-side source address, console success and VMM
disappearance. The five control-plane assertions prove the owner pair,
generation, quota, Probe/Relay watch, abort, VMM-death reset and teardown
paths described by their existing scenarios.

### Source identity

Current source hashes match the approved DISTILL handoff and native source
identity for all implementation/support files that the handoff pins:

| File | SHA-256 |
|---|---|
| `crates/overdrive-init/src/main.rs` | `35d5b02d6fa1f119a05806aad4ab665bbfeb6c2becd76ff456a49676f9b37941` |
| `crates/overdrive-init/src/guest_flow/owner.rs` | `6c6f0e9e02a56ddc7ff91244dcbd4e24820c696fb9c8656d969f4dcb26e02932` |
| `crates/overdrive-testing/bin/guest_peer.rs` | `3d8c8d70da22f64f2c48c4ffd0b14efae503704e3ffbc4424328ee2872d69f39` |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| `crates/overdrive-cli/tests/integration/vsock_support.rs` | `8f5e46ce02d32820917e569a0a70c4d352435aabb308f84d75f274edd608ebba` |
| `crates/overdrive-cli/tests/integration/vsock_walking_skeleton.rs` | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| `crates/overdrive-control-plane/tests/integration/guest_flow_through_serve.rs` | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` |
| `xtask/src/guest_kit_prepare.rs` | `e436cea4685646eb044606ccf5af1e8fe3f6f9e78581ad7cda3dac16d87a0e69` |

The final source-gate receipt is
`.context/netns-density-295/14-01/raw/41dbac5813ccba60893161ad/`; it exits
zero after format, kvm-gated checks and Clippy. Its output hash is
`492a535214e853805686b7db6ff6d1c26dde91496c81a44ea7a3e6d4ff1df6dd` and its
terminal hash is
`e65ab97152d01187b307bf0391a36ac89f52099afecfddfb8291267f0dc1b380`.

### Cleanup and ordinary evidence

The read-only physical-host audit
`.context/netns-density-295/14-01/raw/be0ea50bafad180437fa1117/` exits zero
with `live_cloud_hypervisor=0`, empty physical-host nft output and no
STEP14-01 work directory. Kernel-sensitive effects therefore remain inside
the disposable GuestKit guests; no physical Metal kernel mutation is used as
the product-test substrate.

The ordinary unprivileged source check `794f24c2c6701ed17193a95f` exits zero
after BPF build, format and package checks. The targeted Job terminality
receipt `8e2dc8fe0ae5242845a89b85` and seeded restart-SSOT receipt
`21b992e20519e96eeb2b7d58` both exit zero. The known clean-parent Quint seed
`0x1127719f` is retained as a reproduced baseline counterexample in the
DISTILL review; it is neither waived nor changed, and no model or test was
edited to make it pass. Mutation testing was not run, as required for an
individual DELIVER step.

## Findings

### F-01 — Blocker: V-6 production acceptance is incomplete and the empty datagram is unproved

**Kind:** acceptance-completeness / evidence-boundary gap
**Status:** open
**Severity:** blocker

The accepted V-6 contract is broader than the six bodies that passed. The
feature-delta explicitly requires a production `serve` + `deploy` walking
skeleton with an empty datagram, guest DNS through gateway:53, mesh-VIP TCP
mTLS, service-VIP UDP, inbound leg-C → leg-S → `GuestStreamDial`, TCP and HTTP
probes, and the no-uplink V-LEGB-SERVE observation
(`feature-delta.md:13180`). The roadmap repeats those requirements at
`roadmap.json:1504-1507`.

The exact executed walking skeleton does not send or observe an empty
datagram. Its guest application calls `datagram_exchange` once, and that
function sends only the non-empty `datagram_request(tag)` at
`crates/overdrive-testing/bin/guest_peer.rs:204-217`; the remote peer's UDP
server only recognizes the `DGRAM-REQ:` payload. The native output therefore
proves one non-empty UDP request/reply, not UDP-EMPTY through the production
serve/deploy path. The lower-level EVMOD datagram suite is a different
adapter/module boundary and cannot substitute for this roadmap criterion.

The five through-serve bodies likewise use a raw leased `workload_addr` and a
TCP `ProbeMechanic::Tcp` readiness probe
(`guest_flow_through_serve.rs:639-676, 711-749`). They contain no gateway:53
guest DNS stimulus, service-VIP UDP flow, HTTP probe, or remote-client
leg-C/inbound stimulus. The exact runner manifest contains only the six
identities listed above; no V-LEGB-SERVE capture or uplink observation occurs,
and no test reads a packet capture or equivalent no-uplink oracle. Existing
VIP, DNS, inbound-relay and worker-level cells are separate simulations or
adapter compositions, not the required production `serve` + `deploy` boundary.

This is a bounded, reproducible coverage failure: the source test body,
guest application, exact native selector and retained raw output can be read
without inference, and the required subclaims have no executed assertion or
receipt. The production entrypoint is reachable — `Node::boot` composes
`run_server`, and its `deploy_*` helpers submit through the public HTTPS
handler — but the current exact scenarios do not drive the missing V-6 paths.
No control-plane ordering, timing or concurrency defect is being asserted, so
the seeded-simulation falsification rule is not implicated.

The implementation crafter cannot close this finding by changing the bodies:
the dispatch and user instruction explicitly preserve DISTILL-authored
stimuli/assertions and forbid new tests, scenarios, lanes, mechanisms or
override seams. The finding must therefore return to the owning DISTILL/DESIGN
decision for an exact reconciliation of the V-6 contract and authored
scenario set. No production code change or contract weakening is authorized
by this review.

**Disposition: OPEN — blocks approval.**

## Review dimensions

| Dimension | Result | Evidence |
|---|---|---|
| Exact public/cross-crate API shape | PASS | Dynamic beacon uses the approved token field; no public surface changed. |
| Typed semantic errors | PASS | Every affected token, socket, setup and attach failure maps to an approved typed representation. |
| EVMOD-PAIR lifecycle | PASS | `ReadHangup` is retained until `Paired`; reset/error still closes and aborts; native TCP half-close path passes. |
| Test integrity | PASS | Marker-only activation plus the independently approved private R5-41 fixture correction; no weakened oracle. |
| Native boundary | PASS | Five disposable GuestKit lifecycles, nested CH, exact 117 pass accounting, zero runtime skips and clean host audit. |
| Source equivalence | PASS | Current hashes match the approved native handoff for all pinned files. |
| V-6 acceptance completeness | **BLOCKER** | Empty datagram, DNS, VIP UDP, HTTP, inbound leg-C and no-uplink production proof are absent. |
| Scope / architecture | PASS for landed changes | No override seam, public API, new lane, model or security weakening landed. |
| Mutation policy | PASS | No mutation run occurred. |

## Remediation disposition

No source remediation was performed. The sole finding is upstream acceptance
coverage reconciliation, not an implementation detail that can be repaired
inside the approved crafter scope. A later iteration may only review an exact
DESIGN/DISTILL disposition and independently refreshed evidence; it must keep
the accepted API, typed errors, production ownership and safety boundary
unchanged.

## Iteration 1 verdict

**NEEDS_REVISION.** The landed production changes match GUEST-PORTS and
EVMOD-PAIR, the typed-error contract is preserved, the support package is
honest and independently approved, and the complete native six-body receipt
is non-vacuous and safely confined. The step still cannot be approved because
the explicit V-6 production criterion is not evidenced by the authored
scenarios or the exact native ledger, including the required empty datagram.

# Review iteration 2 — post-remediation re-review

## Iteration 2 scope and authority

This iteration reviews the cumulative implementation through `HEAD
aa25cc910380b03e5df7b767773916b4a2105f43`, including the original `fe22d261d`
and `c9f2f6d69` commits and the remediation in `bc832a3e0`. The review also
reads the accepted roadmap, V-6 feature-delta row, dispatch, ADRs 0187,
0190–0192 and 0197–0211, the approved UDP-deferral DESIGN records and the
DISTILL native-support/alignment reviews.

The user-approved correction is exact: Service-VIP UDP delivery to a VM
backend is deferred to #310 under D12. The approval quote is recorded in
`deliver/design-14-01-udp-deferral.md`:

> “yeah, let's not do that now. add all the relevant info to the github issue,
> and then we keep it deferred to #310 and correct the contradictory
> acceptance checklist.”

The correction removes only that Service-VIP UDP acceptance action. It keeps
off-node TCP and UDP, the genuine empty datagram, gateway:53 DNS, mesh-name
TCP, issued-VIP TCP/mTLS, remote inbound TCP, TCP and HTTP probe state,
V-LEGB-SERVE loopback/no-uplink evidence, the set/unset
`IP_BIND_ADDRESS_NO_PORT` comparison and cleanup. The independent DESIGN and
DISTILL reviews approve that exact boundary. No new scenario, identity,
selector, lane, public product API, production hook, override seam or
mechanism is authorized by the correction.

The current source also contains the user-approved and independently reviewed
DISTILL test-only audit checkpoint and its `node.crash().await` fixture fence.
Those changes are assessed for honesty and lifecycle ownership here; they are
not treated as production API.

## Finding F-01 disposition

Iteration 1's F-01 said the V-6 production journey was incomplete because the
then-current body had only the original Job TCP/non-empty-UDP path and no
empty datagram, DNS, VIP TCP/mTLS, remote inbound, HTTP probes or
V-LEGB-SERVE observations. That finding was accurate for the iteration-1
source and receipt and remains preserved above.

The accepted contract was then corrected by the user-approved deferral, and
DISTILL completed the existing P6-S13 body and private support. The current
body is still the one selected CLI identity. It now executes the retained
outcomes through the production `serve` composition and public `deploy` path;
it does not execute the deferred Service-VIP UDP action. The current exact
native run passes the complete 117-body ledger with zero runtime skips.

F-01 is therefore **CLOSED**. The closure is a contract correction plus the
approved existing-body DISTILL alignment, rather than an unreviewed weakening
or a test-only substitution.

## Current V-6 and V-LEGB-SERVE traceability

| Retained obligation | Current production stimulus and oracle | Evidence |
|---|---|---|
| Job off-node TCP and UDP, including an empty datagram | The original Job still goes through `Lane::deploy_job` and the public handler. `datagram_exchange` sends `send(&[])`, requires a zero-length reply, then performs the existing byte-distinct non-empty exchange (`crates/overdrive-testing/bin/guest_peer.rs:239-287`). The remote peer records only an actual `recv_from` with `n == 0` (`crates/overdrive-testing/src/guest_app.rs:970-987`). | `vsock_walking_skeleton.rs:50-100`; current native 117 receipt `9aac137871ce16772bac79f1` passed the exact CLI identity. |
| Node source and Job VMM disappearance | The remote peer checks both TCP and UDP source addresses against its node-side address. The terminal Job row must be `Terminated`, and `hypervisor_of` must find no VMM (`vsock_walking_skeleton.rs:72-100`). | Current exact CLI body and native ledger pass. |
| Peer VM Service and public identity | `deploy_v6_peer` submits a separate Service through `deploy`, with the declared application, TCP probe, HTTP probe and bounded population listeners (`vsock_support.rs:324-353`). The body reads the public VIP, leased `workload_addr` and listener snapshot (`vsock_walking_skeleton.rs:102-126`). | Current source and native 117 receipt. |
| Gateway:53 guest DNS and mesh-name TCP | The client Service reads its production `/etc/resolv.conf`, requires `nameserver=100.95.0.1`, resolves `v6-peer.svc.overdrive.local`, then performs a byte-distinct TCP exchange by the resolved name (`guest_peer.rs:630-671`; `vsock_walking_skeleton.rs:151-158`). | Current source; current native body passed through the exact selected identity. |
| Issued Service VIP TCP through mTLS | The client opens the peer's public issued VIP and application port, exchanges `V6_VIP_REQUEST`/`V6_VIP_RESPONSE`, and keeps the stream for state/population observations (`guest_peer.rs:673-720`). The body correlates one eligible `ss` row to the peer backend and requires TLS 1.3 kTLS in both directions with no cleartext markers (`vsock_walking_skeleton.rs:159-160,187-194`; `vsock_support.rs:1370-1434`). | Current source and current native pass. |
| Remote inbound TCP through leg C → leg S → `GuestStreamDial` | `remote_mtls_round_trip` enters a route-disjoint disposable namespace, builds a client certificate from the production CA material, verifies the peer chain/SPIFFE URI, dials the deployed VM backend tuple and exchanges byte-distinct records. The peer console must record the request (`vsock_support.rs:199-247,503-555`; `vsock_walking_skeleton.rs:128-133`). | Current source and current native pass; this is a real remote client stimulus, not a worker-only stand-in. |
| TCP and HTTP probe state follows healthy → unhealthy → healthy | The peer has public TCP and HTTP readiness descriptors. The client sends the ordinary state command, the peer changes its shared health state, and `wait_probe_state` checks descriptor role/index/mechanic, same allocation id and strictly newer Pass/Fail/Pass timestamps for both probes (`guest_peer.rs:371-415`; `vsock_walking_skeleton.rs:125-126,162-179,220-271`). | Current source and current native pass. |
| V-LEGB-SERVE no-uplink and loopback leg-C identity | `UplinkGuard` and `PacketCapture` are created inside the disposable outer GuestKit guest. A positive control proves capture is live. While the client holds the production population, the body requires exactly one backend socket with a loopback leg-C peer and zero matching uplink tuple frames with no packet-loss/truncation ambiguity (`vsock_walking_skeleton.rs:135-139,181-195`; `vsock_support.rs:1094-1155,1186-1434`). | Current source and current native pass. |
| `IP_BIND_ADDRESS_NO_PORT` set/unset comparison | The test-only unset cohort reads back option 0 and proves bounded ephemeral exhaustion. The client Service opens the production set cohort; the external socket/inode oracle requires option 1, exact backend tuples, fewer source ports than destinations, one fd owner per inode and post-close absence (`vsock_support.rs:648-800,1045-1092`; body `vsock_walking_skeleton.rs:135,181-217`). | Current source and current native pass. |
| Service-VIP UDP delivery to a VM backend | No current action or assertion. The feature-delta, roadmap and approved DESIGN explicitly defer this action to #310 under D12; D24's general guest UDP `connect4` contract remains unchanged. | Correctly excluded; no credit is claimed and no #310 mechanism was added. |
| Five through-serve owner bodies | The runner keeps the exact five existing identities: held pair, R5-41 replacement, Probe/Relay watches, dropped Probe abort and VMM-exit Relay reset. The approved private `deploy_hold_service` fixture keeps replacement workload-kind neutral. | `fwd_guest_suite.rs:1143-1173`; native ledger `step14_01_control_plane_passed=5`. |
| Production composition / no override | `Lane::boot` composes the existing in-process `serve` root and every workload is submitted through the public deploy handler. The outer native runner invokes the existing `run_nested_vm_test_binary`/GuestKit boundary; it does not install product maps, listeners, routes or selectors. | `vsock_support.rs:254-299,306-375`; `fwd_guest_suite.rs:1215-1266,2369-2550`. |

The current acceptance body therefore supplies the missing production
composition proof while preserving the six exact identities and `111 + 1 + 5
= 117` inventory. The deferred action is recorded as deferred, rather than
silently passing through a stand-in or being counted as an unexecuted
success.

## Production implementation review

### GUEST-PORTS and beacon ownership

`read_transport_token_with` parses the seven-field token and maps all parser
failures to the existing `InitError::GuestTransportSetup` carrier
(`crates/overdrive-init/src/main.rs:457-488`). `connect_beacon` reads the
same token and passes `token.ports.beacon` to every retry attempt; the
`VsockAddr` uses that exact value (`main.rs:574-623`). The old fixed beacon
port is not used on the production path. The change adds no public type,
method, variant, wire field or configuration surface.

### Pending `ReadHangup` and EVMOD-PAIR

When the pending outbound child is the socket that receives
`GuestHangup::ReadHangup`, the owner returns while retaining its
`PendingOutbound` entry (`crates/overdrive-init/src/guest_flow/owner.rs:680-693`).
The later `Paired` path attaches the retained child; the forwarder's attach-time
logic carries queued bytes and FIN. Reset/error paths still remove the pending
entry, close both halves and issue the existing `Abort` path. The current
walking skeleton writes its request, calls `shutdown(Write)` and reads the
reply (`crates/overdrive-testing/bin/guest_peer.rs:203-230`), so the
early-FIN behavior is exercised through production `serve`/`deploy`.

### HTTP GuestStream lifetime and body drain

`probe_guest_stream` drains the response body before classifying status
(`crates/overdrive-worker/src/probe_runner/http_prober.rs:246-265`).
`GuestStreamIo` retains the owned stream inside in-flight read, write and
shutdown futures and returns it only when the future completes
(`http_prober.rs:298-469`). The probe runner wraps the full dial, request and
body-drain operation in its existing timeout (`crates/overdrive-worker/src/probe_runner/mod.rs:565-611`).
Cancellation therefore drops the operation and its stream under the existing
owner abort/lifetime path; it does not spawn detached work or add a second
public method. Status remains the approved semantic outcome; response body
bytes are deliberately outside the HTTP probe contract.

The native V-6 probe transition exercises the actual production HTTP probe,
including initial Pass, newer Fail and newer Pass. The earlier HTTP reset
history and DISTILL fixture request-consumption correction remain retained;
they are not relabelled as unrelated green evidence.

### Outbound mTLS transport death and reclaim

Outbound and inbound pumps receive adapter-private roles in the existing
`PumpState` (`crates/overdrive-dataplane/src/mtls/splice.rs:57-92`, assigned in
`outbound.rs` and `inbound.rs`). On a transport-death exit, the detached reaper
identifies whether the terminal pump was an outbound leg and sets the existing
`reset_peer_on_reclaim` flag before removing the connection
(`crates/overdrive-dataplane/src/mtls/mod.rs:263-310`). Reclaim keeps the
existing stop/join/close ownership order and applies `SO_LINGER{1,0}` on the
abortive branch (`mod.rs:315-390`). The current native output contains both
outbound terminal-role/transport-death records and guest-side reset outcomes;
the current native run completes without a reaper hang.

The review also checked the lifetime edge explicitly described by
`PumpHandle::request_stop`: a blocking encrypt splice needs a socket wake
before join (`splice.rs:263-287`). The existing bounded blocked-splice test
proves the `reset_peer_on_reclaim == false` wake path
(`mod.rs:1425-1514`). The `true` branch is an existing abortive-close path and
does not have an independent current production failing reproduction for a
blocked sibling. Under the repository reachability gate this remains an
unproven hypothesis, not a finding or a remediation request: no production
caller/owner path and bounded failure proof establishes the suspected hang.
No new shutdown mechanism is prescribed.

The read-only diagnostics added around this path call only
`getsockname`, `getpeername`, `TCP_INFO` and read-only socket options while
the connection entry is still owned (`mod.rs:392-573`). They do not consume
socket state, change lifecycle ordering, or branch product behavior. Broad
diagnostic output is therefore an evidence-quality concern only; the current
native output and cleanup remain bounded.

### SharedAudit revalidation

`audit_shared` snapshots owner pair/watch counts and generation, pauses only
at the test-only checkpoint, reads forwarder inventory and revalidates a
non-snapshot match against the current owner state and unchanged generation
before accepting it (`crates/overdrive-control-plane/src/guest_network.rs:1316-1369`).
The normal mismatch maps to the existing typed
`GuestNetworkError::PostconditionMismatch` and marks the existing Forwarder
repair component. The user-approved async checkpoint is `cfg(test)` and does
not hold an owner-state or fixture mutex across its await
(`guest_network.rs:711-760`).

The pre-fix seeded production-owner regression uses seed
`11400714819323198485` and offset `989 ms`; it observed a Forwarder recovery,
one quiescence hold, `Quiesced` admission and zero surviving watches instead
of the normal held Probe watch. The current source passes all eight seeds in
`bdfb7d8e8df254218c78958a0`, with zero seam-leak/panic reports after the
approved fixture fence. This establishes both reachability and necessity for
the revalidation; it does not claim a generic race fix beyond this invariant.

## Typed semantic-failure mapping

The current implementation preserves the approved semantic taxonomy. The
following table maps each affected designed failure to the representation the
caller actually branches on.

| Designed failure | Approved implemented representation | Current owner/caller evidence |
|---|---|---|
| Missing or duplicate `overdrive.transport=` token | `GuestTransportSetupError::TokenMissing` / `TokenDuplicate`, wrapped by `InitError::GuestTransportSetup` | `transport/mod.rs:84-93`; `main.rs:477-488` |
| Malformed, missing, duplicate or reserved token field/port | `GuestTransportSetupError::TokenMalformed { field: TransportTokenField }`, wrapped by `InitError::GuestTransportSetup` | `transport/mod.rs:95-156` |
| Guest transport setup operation failure | Existing named `GuestTransportSetupError` variants (`DummyDevice`, `AddressAdd`, `FwmarkRule`, `FwmarkRoute`, `TcpFwmarkAccept`, `FwmarkReflect`, `ReservedPorts`, `IntakeBind`, `ProgramLoad`, `ProgramAttach`, module/listener/forwarder variants) | `transport/mod.rs:159-247,252-280` |
| Beacon socket creation or connect failure after retry | `InitError::Socket(Errno)` or `InitError::Connect(Errno)` carrying the last kernel cause | `main.rs:588-623` |
| Mesh resolution failure / unavailable leg-F / peer setup / connect / attach | Existing `FlowRefusal::ResolveFailed`, `FlowRefusal::IntakeUnavailable`, or `FlowRefusal::MeshUnreachable`; no diagnostic string is used as the branch carrier | `owner_runtime.rs:1098-1211`; `wire.rs:386-455` |
| Node forwarding quiesced, no session, not serving, pairing deadline, capacity, refused/timed-out/unreachable VM dial | Existing `GuestDialError::{Quiesced,NoSession,NotServing,PairingDeadline,Capacity,Refused,TimedOut,Unreachable { errno }}` | `dial.rs:226-302`; owner end projection `owner_runtime.rs:1457-1469` |
| Pending child half-close before `Paired` | No error: retained `ReadHangup` is the approved EVMOD-PAIR state. A reset/error follows existing `Abort`/close behavior. | `overdrive-init/src/guest_flow/owner.rs:680-693` |
| Attach-time ended half | Existing `ForwarderError::HalfEnded { side, errno }`; owner counts `forwarder_attach_ended`, aborts both halves and returns the existing `FlowOutcome::Aborted` path | `owner_runtime.rs:1177-1206`; ADR-0210 |
| Invalid HTTP target | `ProbeFailure::InvalidTarget { reason }` remains the adapter error before a guest dial | `http_prober.rs:219-240`; `mod.rs:572-581` |
| HTTP status or transport result | Existing `ProbeOutcome::Pass` / `ProbeOutcome::Fail { reason }`; body-drain failure does not introduce a new semantic status because the accepted probe contract is status-based | `http_prober.rs:162-180,246-265` |
| Shared owner/forwarder inventory mismatch | Existing typed `GuestNetworkError::PostconditionMismatch { operation: SharedAudit, expected, observed }`, followed by existing Forwarder recovery ownership | `guest_network.rs:1339-1368`; `lib.rs:1197-1305` |

No new semantic condition is flattened into a string, and no unapproved public
error variant or payload was introduced.

## Test integrity, scope and contract shape

The V-6 body and private support changes are the independently approved
DISTILL alignment, not an implementation-side weakening of a failing oracle.
The current source retains all original required assertions and adds the
approved retained-outcome assertions; it removes only the user-deferred
Service-VIP UDP action. The five through-serve bodies remain the existing
identities, with only the approved private R5-41 Service fixture adjustment.
All six selected bodies retain `/// CONTRACT_SHAPE: bounded-change.`.

The existing GuestKit runner still selects exactly one CLI identity and five
control-plane identities in `fwd_guest_suite.rs:1143-1173`. Its cgroup-default
path runs those binaries through `run_nested_vm_test_binary` and fresh outer
GuestKit guests (`fwd_guest_suite.rs:2369-2550`), so the V6 `ip netns`, route,
AF_PACKET and ephemeral-range effects are inside the disposable outer guest.
The physical Metal host receives only the existing outer GuestKit VMM. No
Lima run, mutation run, host-kernel product test or new execution lane was
used.

The test-only `SharedAuditCheckpointControl` remains `cfg(test)` and
crate-local. The current workspace Clippy receipt `8e5bd0bf1fe2bea79b626f76`
exited zero with an explicit ledger for the two reviewed DISTILL-only lint
exceptions (`expect_used` and `unchecked_time_subtraction`). The initial
warnings-denied run `43fd05b2faf8633971212f86` exited 101 on exactly those two
locations; it is retained. The final receipt is therefore reported as
ledgered Clippy, not as an unqualified warning-free claim.

## Native, source and cleanup evidence

The current canonical native receipt is
`.context/netns-density-295/14-01/raw/9aac137871ce16772bac79f1/`:

Its retained `output.log`, `source-identity.v1`, `source-files.sha256`,
`skip-ledger.v1`, `terminal.v1`, `cleanup.v1` and `host-audit.log` are the
full raw, identity, ledger, terminal and cleanup evidence used below.

| Item | Result |
|---|---|
| Terminal | exit 0, `command_returned`; `runtime_skips=0` |
| Visible ledger | 117 passed, 0 failed, 0 ignored; 111 retained + 1 CLI + 5 control-plane |
| Exact STEP 14-01 credit | `canonical_117_credit=117`; six exact identities each selected once |
| Source identity | digest `50de2d4dccd558cace346402efd7768254b3f2a62b3ca9a0561245c3c4746c22`; output SHA `38ee4b41f55f3eaaa3a14d2a2f7f2fbff12db476e660ec55ccfcb8d1f5e54442` |
| Terminal SHA | `5c879581c441ae96f082554bab33b3d409cb2e60b574f8bd494f32f05523859b` |
| Cleanup | `b1885ef1a18e5bf2fe0671a80`, exit 0; no live Cloud Hypervisor, nft tables or STEP14 workdirs; `ovd-ns-0000` is the documented pre-existing namespace |

The raw source identity declares base commit `c9f2f6d69` and pins its ordered
source-file hashes. Current-source comparison matches the pinned
`guest_network.rs`, `flow_owner.rs`, `lib.rs` and `fwd_guest_suite.rs` hashes.
The only current worktree difference in that receipt's five-file list is the
approved one-line `node.crash().await` fixture fence in
`shared_network_recovery_acceptance.rs` (receipt hash `f16a377b...`, current
hash `df15ced7...`). Review-distill iteration 28 proves that this statement
comes after every behavioral assertion, reconstructs the prior fixture bytes
when removed, and changes no stimulus, seed, schedule or oracle. No native
behavioral credit is attributed to the cleanup-only line.

The final DISTILL source handoff independently pins the current V-6 source
that is not enumerated in the runner's five-file identity list:
`vsock_walking_skeleton.rs` is `f64f7beb8a25bbe34aec9b73d4b292926151514a4bf3224cdaaceffb8e95971d`,
`vsock_support.rs` is `59a3211b55f920fc25bb1cbb7fc2d6129672cf8e7fe233030e216e665701b63b`,
`guest_peer.rs` is `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec`,
and `guest_app.rs` is `395c3fc937de129f66a751edf1226be5f928f592d003fb98ab11219d2c80f4c7`.
Those hashes match the current worktree and the approved DISTILL alignment;
the exact native selection is the existing runner's direct six-identity
manifest, not a stale source-copy launcher.

The current source-gate receipt `5bec214df78304c714ae32a0` and ordinary
workspace check `a480b64007c06dc92774c716` both exit 0. The initial and final
Clippy receipts, source hashes and exception ledger are retained above. The
ordinary checks ran without native tests. `git diff --check` reports only
inherited intentional Markdown hard-break whitespace in approved review
artifacts; source and executable test files are clean.

The entire DES log contains unrelated incomplete future-step entries, but the
14-01 sequence itself has actual RED failure and pass, GREEN failure/pass,
COMMIT pass, the retained remediation attempts, and the final RED failure,
GREEN passes and COMMIT pass. No DES phase was edited or invented by this
review. The repository-wide verifier reports only the unrelated `04-02`,
`04-03`, `04-04`, `14-02`–`14-05` gaps; it does not report a 14-01 phase gap.

## Historical evidence and limitations

| Historical receipt | Retained fact | Current disposition |
|---|---|---|
| `af7d3395010cef474da2c000` | Healthy CLI setup later reset with an unknown-host/errno presentation; the generic `ETIMEDOUT` label does not identify a root cause. | Retained as a failure with zero control-plane credit; no waiver or relabeling. |
| `727f0a30d20dc6ae67b3d3b1` | First Forwarder-recovery control-plane run exited 101 without a proven initiating cause. | Retained as an unknown-cause failure with zero credit; no invented diagnosis. |
| `69697656241418f6e7342b7c` | Third control-plane body stopped after 114 visible passes at the typed watch mismatch; expected 0 and observed 1 remain historical failure evidence. | Retained; later seeded overlap RED and current green fix establish the bounded reachable race, but history is not erased. |
| `d9366f8860945f3880e26ac6` | Seed `11400714819323198485`, offset `989 ms`, production owner/supervisor Probe overlap failed the held-state invariant. | Necessity and reachability proof for the audit remediation. |
| `918b8c78ca620f96ad184c0c` / `bdfb7d8e8df254218c78958a` | Initial seeded fixture passed assertions but emitted eight seam-leak/panic-hook reports; the corrected run passed all eight seeds with zero reports. | Historical failure retained; one-line owner fence approved and closed by DISTILL review iteration 28. |
| Clean-parent seed `0x1127719f` | Reproduced baseline `exit 100`, 0 pass, 1 fail and 189 skip, with SHA `6092244d...`. | Retained limitation only; no waiver, model change or current 117 credit. |
| `9aac137871ce16772bac79f1` | Current canonical runner: exactly 117 passed, no ignored/runtime skips, cleanup exit 0. | Current evidence for this implementation; it does not erase any preceding failure. |

The historical receipts are kept with their original labels and boundaries.
The current full success is not used to infer that an older unexplained
failure never occurred.

## Reachability and finding gate

The only production correctness finding accepted in this iteration is the
seeded SharedAudit mismatch already proved by `d9366f...`: the real composed
owner, supervisor and normal `OwnerDial` Probe path reached the checkpoint and
failed the invariant before remediation. The current code revalidates after
inventory and preserves the existing typed mismatch/repair owner.

The potential blocked-encrypt-pump concern in the abortive reclaim branch was
not promoted to a finding because no bounded current production caller/owner
reproduction proves it. The repository rule rejects static suspicion, a
forced test-only abort or an imagined schedule as a DELIVER defect. The
existing blocked-splice test proves the non-abortive wake path; no source,
test, design or architecture change is requested for the unproved branch.
Likewise, the diagnostic logging additions are output-only and have no
reachable behavior/lifetime failure in the current native receipt.

## Iteration 2 findings and remediation dispositions

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| F-01 (from iteration 1) | Blocker, historical | V-6 production acceptance was incomplete in the earlier source and receipt. | **CLOSED.** The exact user-approved #310 deferral and approved DISTILL alignment remove only Service-VIP UDP-to-VM and execute every retained outcome through the existing production composition. |
| F-02 | None | Beacon dynamic port, pending-child half-close, HTTP stream lifetime/body drain, outbound transport-death reset, and SharedAudit revalidation preserve existing contracts and owners. | **NO FINDING.** Behavior and lifetime paths are bounded and mapped to approved representations. |
| F-03 | None | Test-only support performs route, namespace, capture and ephemeral-range operations. | **NO FINDING.** The existing runner executes the test process inside disposable outer GuestKit; the physical-host audit is clean. |
| F-04 | Evidence | Historical unknown causes and prior typed-watch mismatch remain in the receipt set. | **RETAINED HONESTLY.** The current pass receives current credit only; no waiver or history rewrite. |
| F-05 | Hypothesis only | Abortive `reset_peer` reclaim skips `shutdown` before joining a potentially blocked sibling. | **UNPROVEN, NOT A FINDING.** No reachable production failing proof; no remediation authorized. |

## Contract-shape and quality-gate result

The walking skeleton is an exempt single-shot native-metal integration body,
while the five owner bodies remain existing production-composition acceptance
tests. The selected bodies enter through public/in-process composition roots and
assert user-visible or driven-port outcomes. No mock-dominated SUT, fixture
end-state seeding, assertion deletion, runtime skip, or weakened error oracle
was found. The exact runner fails closed on nonzero exit, missing summaries and
runtime skip diagnostics.

The RED/GREEN/COMMIT phases recorded in DES are actual events, including the
intermediate failures. The final current selection is independent of the
reviewer's artifact and was not run by a Rust expectation harness. No mutation
testing was run, as required for an individual DELIVER step.

## Iteration 2 verdict

**APPROVED.** The current cumulative implementation matches the accepted
public API, ownership, lifecycle, wire and typed-error contracts. The
user-approved Service-VIP UDP deferral is represented exactly and only at the
#310 boundary. The current V-6 body proves every remaining required outcome
through production `serve` + public `deploy`, including empty UDP, gateway DNS,
mesh TCP/mTLS, remote inbound TCP, TCP/HTTP Pass–Fail–Pass, loopback leg-C,
no-uplink capture and the bounded port-sharing comparison. The five
through-serve identities remain exact.

The current native ledger is non-vacuous: 117 passed, zero ignored and zero
runtime skips, with disposable GuestKit confinement and clean cleanup. The
earlier F-01 blocker is closed, the seeded audit correction is necessary and
reachable, and all historical failures remain recorded with their original
limitations. No source, test, design, DES or commit change was made by this
review; only this native Markdown artifact was updated.
