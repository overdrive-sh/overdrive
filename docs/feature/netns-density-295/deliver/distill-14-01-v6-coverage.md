# STEP 14-01 V-6 acceptance-coverage reconciliation

## Implementation evidence — reproduced #310 boundary

The iteration-7-approved proposal is implemented in the existing P6-S13
identity and private support. Durable Metal run
`012109f99448d662b35c1760` (source digest
`62f6c1832d836a9536a84d89d821f14bd43357fbb5f8a2125f5f8efb16ca3728`)
reached the exact public Service-VIP UDP action after proving empty UDP,
gateway DNS, mesh-name resolution, issued-VIP TCP and route-disjoint same-CA
remote mTLS through production `serve` and public deploy.

The production UDP action does not complete. Two client VM Service
allocations fail with test protocol exit 21 (`UDP_TIMEOUT`); a third remains
Running after `v6-client: vip-tcp ok` without `v6-client: vip-udp ok` for the
120-second bounded observer. The peer VM Service declares and binds the UDP
listener, but the earlier exact-source run `a5eb43472f5dc625d0c29e86`
records its remote/name/VIP TCP requests and no UDP request. The fixture sends
to the public issued Service VIP and accepts only the byte-distinct peer
reply, so it cannot manufacture or waive the missing production `connect4`
outcome.

The user subsequently approved deferring this exact Service-VIP UDP-to-VM
acceptance action to #310. The run remains production evidence for that
deferral; it is no longer a missing #295 V-6 outcome and does not authorize a
production repair in STEP 14-01. The remaining already-implemented oracles
stay fail-closed and must be reachable without the deferred action after
DISTILL aligns the existing body. DISTILL made no production edit and added no
test, identity, scenario, selector, lane, API, hook or override seam. Raw
durable evidence is retained under
`.context/netns-density-295/14-01/raw/012109f99448d662b35c1760/`.

## Disposition

**Audit reconciled to the user-approved deferral: Service-VIP UDP delivery to
a VM backend is not a #295 V-6 / STEP 14-01 acceptance action.** The current
117-body native receipt remains honest but does not discharge every retained
V-6 / V-LEGB-SERVE outcome. Existing production-composition evidence can be
mapped precisely; the remaining retained outcomes must still run through
`overdrive serve` + public `overdrive deploy`.

No new test body or scenario is required. DISTILL must remove only the
deferred Service-VIP UDP-to-VM action and its gating dependency from the
already-authored P6-S13 / V-6 body, then retain every other existing V-6 and
V-LEGB-SERVE oracle plus the five `guest_flow_through_serve` identities. This
keeps the existing six STEP 14-01 identities and the runner's
`111 + 1 + 5 = 117` count. It changes no production API, hook, lane, selector,
mechanism or model.

This document is an audit and concrete source-change proposal only. No
executable source, production source, DES event, execution log or commit was
changed. The original dispatch prohibited DELIVER from editing DISTILL bodies;
therefore the body completion must be performed by DISTILL and independently
reviewed before the implementation reviewer re-runs STEP 14-01.

## Accepted contract

The controlling obligations are cumulative, not alternatives:

- Feature-delta V-6 requires one production-composition journey through
  `overdrive serve` + `overdrive deploy`: a VM Service dials a mesh peer by
  name, reaches a non-mesh TCP/UDP peer with an empty datagram included,
  reaches the mesh Service frontend/VIP over TCP with mTLS, accepts a remote
  inbound TCP connection through leg C → leg S → `GuestStreamDial`, and
  exposes TCP and HTTP probe outcomes that follow application state. The
  Service-VIP UDP-to-VM action is deferred to #310 under D12
  (`feature-delta.md:13180`).
- Feature-delta V-LEGB-SERVE requires the same-node mesh dial through the
  production leg B, leg C, guest-flow owner and leased `workload_addr` with
  and without a node default route. The round trip must complete; leg C must
  see a `127.0.0.1:<ephemeral>` peer; no packet may leave on the uplink. Its
  accepted evidence scope also records the concurrent
  `IP_BIND_ADDRESS_NO_PORT` set/unset comparison (`feature-delta.md:13174`).
- Roadmap criterion 1 requires the existing CLI walking skeleton to include
  an empty datagram while retaining the node-source and VMM-gone outcomes
  (`roadmap.json:1504`).
- Roadmap criterion 2 requires the five existing owner-observation bodies
  (`roadmap.json:1505`).
- Roadmap criterion 3 repeats name/gateway:53, mesh-VIP TCP, remote inbound
  TCP, TCP/HTTP probes and V-LEGB-SERVE no-uplink through the production
  composition, and records the Service-VIP UDP-to-VM deferral
  (`roadmap.json:1506`).
- Criterion 4 forbids test-installed product state and override seams
  (`roadmap.json:1507`).

The roadmap identifies `P6-S13 / V-6 / V-LEGB-SERVE`, with the CLI body as its
primary executable and the five named `guest_flow_through_serve` bodies as its
owner/probe/Relay complements. Completing the primary body is correction of
that accepted inventory, not a seventh test or a new scenario class.

## Retained evidence identity

The audit read the committed STEP 14-01 source at `HEAD c9f2f6d69` and the
reviewer-owned `review-14-01.md`; it did not run or mutate a host.

| Artifact | SHA-256 |
| --- | --- |
| Pre-deferral feature delta | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| Pre-deferral roadmap | `2a909f49f0edc671c989228def9daac8c3d7fb41763d55d7359ea7471c03875a` |
| STEP 14-01 dispatch | `ef672d8849ff68fde6a1f4d2050e09957373030eeb4b580d7c1999c442d593f7` |
| CLI walking skeleton | `2cc74c7eba4a769149ea930abd74559f2c70dbceac1c36f17aa6c4ee98074b34` |
| CLI vsock support | `8f5e46ce02d32820917e569a0a70c4d352435aabb308f84d75f274edd608ebba` |
| Through-serve owner bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` |
| Guest workload binary | `3d8c8d70da22f64f2c48c4ffd0b14efae503704e3ffbc4424328ee2872d69f39` |
| Guest/remote-peer protocol support | `9539a3388bc2004ab2e19d4b81878851307fc4c35e2a1f2689cbb1a3cc7e591d` |
| Owner datagram acceptance | `2c732a48ed2358886aaa6f215478eb5b81991bd6c7421867df4d283411fed11f` |
| Real worker mTLS suite | `dffc6d6292288197618af686427948cdc241492ca5047930abdfa615ca7e86e0` |
| Real leg-B suite | `b5294cd324f3658d88a46641ebc0c352159d0433ab202a30233d7d5ebaded1c5` |
| Real constant-rule / remote leg-C suite | `853f30dede0a4ad29c8579c585a002e16c158ed52cf228b3c8f6d4c9725dd400` |
| DELIVER review iteration 1 | `f19e0fa48ece9ae8ad289b3dcae3676bb2ab164a532e76386209a1dd648122a7` |

The decisive native record remains
`.context/netns-density-295/14-01/raw/410a62758dd8e62f9f9d6418/`:

- source digest
  `d3fe8c43abf2447f87621f8d6b96ea863525d95c27f7e5784190ee36b7fbe6ce`;
- exit 0, `command_returned`;
- output SHA-256
  `df6b8f0439e8908dd7bd6862967732906245276d0809f05cdb1fff5eb9a273f4`;
- terminal SHA-256
  `22e48c84b552885091dce2c89793901e770027dbaa8ea6f678a1b55eef586e12`;
- 107 successful summaries declaring exactly 117 passed, 0 failed and
  0 ignored, with zero recognized runtime-SKIP diagnostics.

The runner selects only one CLI body and five control-plane bodies
(`fwd_guest_suite.rs:1140-1175`). The raw record proves the CLI identity once
at `output.log:3097-3100` and the five control-plane identities once at
`output.log:3547-3574`. A passing selected body proves only the assertions in
that body; it cannot prove an uninvoked stimulus.

## Exact traceability

| Accepted outcome | Current production invocation and oracle | Receipt | Disposition |
| --- | --- | --- | --- |
| Non-mesh TCP round trip | The CLI body deploys the real VM Job through `deploy`, the guest opens TCP to `RemotePeer`, and the peer asserts one request from the node-side veth address; the Job must be `Terminated` (`vsock_walking_skeleton.rs:43-79`). | `410a…`, exact CLI PASS at `output.log:3097-3100` | **COVERED** |
| Non-mesh non-empty UDP round trip | The same body invokes `ROUND_TRIP`; the guest sends `datagram_request(tag)`, checks the byte-distinct reply, and the peer asserts one tagged datagram from the node-side address (`guest_peer.rs:195-230`; `vsock_walking_skeleton.rs:80-85`). | Same exact CLI PASS | **COVERED for non-empty only** |
| Empty UDP datagram | `datagram_exchange` calls `send(&datagram_request(tag))` once. `RemotePeer::serve_udp` recognizes only `DGRAM-REQ:` and `CHURN:` payloads; `PeerLog` has no zero-length observation (`guest_peer.rs:204-220`; `guest_app.rs:521-535,932-952`). | No empty send or receive appears in the selected body or raw record. | **MISSING** |
| Node source address and VMM gone | The remote peer checks the node-side veth address for TCP and UDP, then the body requires `hypervisor_of(alloc).is_none()` after terminal state (`vsock_walking_skeleton.rs:72-89`). | Same exact CLI PASS | **COVERED** |
| Production TCP probe over `GuestStreamDial` | `deploy_listening_service` submits a real VM Service with a TCP readiness probe. The probe/relay body requires at least three application-observed probe connections and owner `DialPurpose::Probe` accounting (`guest_flow_through_serve.rs:199-235,635-688`). | `410a…`, exact body PASS at `output.log:3559-3562` | **COVERED** |
| Same-node leg-B → leg-C → leg-S → `GuestStreamDial` round trip without a node default route | The same body deploys a client VM Job that holds a byte-moving connection to the Service's leased `workload_addr`; it requires the Service allocation's Paired `Relay` dial and the client allocation's Paired guest flow. The existing disposable GuestKit has no NIC/default route after its bounded route fixtures clean up. | Exact body PASS; retained run records the no-default/no-uplink qualified readback before later fresh STEP 14 guests (`output.log:2077-2152,2651`). | **COVERED for the route-less production round trip and owner state**; it does not observe leg-C's peer or a with-default uplink. |
| Guest DNS by mesh name through gateway:53 | No selected guest command accepts a name. `ROUND_TRIP` parses both destinations as `SocketAddrV4` (`guest_peer.rs:108-120`), and the selected bodies pass raw IP addresses. | No selected identity or raw assertion. | **MISSING** |
| Mesh frontend / issued Service VIP over TCP with mTLS through leg F | The same-node body dials the backend `workload_addr` directly. It neither reads the Service VIP/frontend nor drives the D24a frontend-key branch. | No selected identity or raw assertion. | **MISSING** |
| Service-VIP UDP delivery to a VM backend | Native production composition reached this action after DNS, off-node UDP, mesh-name TCP and TCP-to-VIP, then timed out without a peer-VM UDP request. | `012109f99448d662b35c1760` and the exact-source complement `a5eb43472f5dc625d0c29e86`. | **DEFERRED TO #310 — NOT A STEP 14-01 ACCEPTANCE ITEM** |
| Remote inbound client through leg C → leg S → `GuestStreamDial` | The current relay client is another same-node VM Job. `RemotePeer` is only a TCP/UDP server that guest workloads dial; it never acts as an mTLS client to the VM tuple. | Same-node relay PASS does not supply the required remote stimulus. | **MISSING** |
| HTTP probe follows application state | The only selected probe is `ProbeMechanic::Tcp`; the guest `listen` application does not answer HTTP. | No HTTP descriptor/result in a selected body. | **MISSING** |
| V-LEGB-SERVE with-default route, `127.0.0.1:<ephemeral>` leg-C peer and zero uplink packets | The selected same-node relay body has no default-route fixture, socket-table assertion or packet capture. | No selected assertion or capture. | **MISSING** |
| V-LEGB-SERVE concurrent `IP_BIND_ADDRESS_NO_PORT` set/unset comparison | The production set path is exercised indirectly, but the selected bodies record no concurrent comparison or exhaustion observation. | No selected assertion or diagnostic. | **MISSING accepted evidence item** |

## Why existing mechanism tests do not close the gaps

These tests remain valuable non-regression evidence, but their boundaries are
explicitly narrower than V-6:

- `owner_acceptance/datagrams.rs` proves empty framing/order, gateway:53 policy
  and a scripted `connect4` rewrite at the owner port. It uses the in-memory
  composition and scripted flow I/O; it does not boot `run_server`, a real VM,
  the DNS responder, the cgroup attachment or the real NIC.
- `vsock_mesh_egress_mtls.rs` proves real leg-F mTLS, but its own header says
  it uses `CountingResolve` and plays the guest-flow owner's role at the port;
  it explicitly assigns the full `serve` + `deploy` mesh cell to P6
  (`vsock_mesh_egress_mtls.rs:13-33`).
- `vsock_legb_dial.rs` proves production marking, loopback binding and leg-C
  socket identity at the worker boundary. It calls the enforcement port
  directly and uses a test-side serving target rather than the `run_server`
  owner and deployed VM (`vsock_legb_dial.rs:15-39`).
- `vsock_constant_guest_prefix_rules.rs` proves a remote mTLS client reaches
  production leg C, but it composes the worker fixture directly and supplies
  the serving guest-stream target through private test support rather than a
  `run_server` owner and deployed VM (`vsock_constant_guest_prefix_rules.rs:15-31,152-220`).
- `dns_responder_bind` and the real TCP/HTTP prober suites prove their host
  adapters directly. The selected STEP 14 bodies never cause a deployed guest
  to use those adapters through the guest-flow owner.

Combining green results from these boundaries would establish that components
work separately. It would not establish that V-6 is wired through the accepted
production entrypoint, which is the purpose of this walking skeleton.

## Smallest executable remedy

### Keep the executable inventory fixed

Do not add a seventh `#[test]`, rename an existing identity, add a selector,
create a lane or change the static count. Retain:

1. the existing CLI P6-S13 body; and
2. the five existing `guest_flow_through_serve` bodies.

The complete rerun must still reconcile to exactly 117 passed bodies and zero
ignored/runtime-SKIP diagnostics through the existing `cgroup-default`
GuestKit runner.

### Complete the existing off-node phase

Within the current P6-S13 body and `ROUND_TRIP` helper:

- send a genuine zero-length UDP datagram on the same connected guest socket;
- have the existing remote peer record its source and send a genuine
  zero-length reply;
- require the guest to receive length zero and continue to the existing
  non-empty byte-distinct exchange; and
- assert exactly one empty request, its node-side source address, the guest's
  empty-reply success line, the existing non-empty result and VMM disappearance.

Keep `UDP_CHURN` behavior unchanged by confining the empty exchange to the
existing `ROUND_TRIP` path. Missing zero-length traffic must fail; the fixture
must not manufacture an observation without `recv_from` returning `n == 0`.

### Complete the corrected V-6 Service phase

Extend the same P6-S13 body after preserving its current Job phase. Pin two
distinct production workloads in the same `serve` lifetime:

- **client VM Service** — submitted through the existing public deploy path;
  its guest application performs every required gateway-DNS, mesh-name,
  mesh/frontend-VIP TCP and off-node TCP/UDP dial. It has one private
  valid TCP listener solely to satisfy Service admission and empty probe
  arrays. Every dial outcome is attributed to this Service's allocation and
  console, never to an outer-guest helper or the peer Service.
- **peer VM Service** — submitted separately through the same public deploy
  path; it exposes the TCP, probe and state-control endpoints, is the
  remote-inbound target and owns the issued VIP/name the client Service uses.

The existing off-node Job remains roadmap criterion 1's run-to-completion
proof. It does not substitute for the required client Service actor.

Then complete these actions:

1. Deploy the peer Service with the TCP application listener, separate TCP
   and HTTP probe listeners and one private state-control listener. Deploy the
   client Service only after all required captures are armed.
2. Read the peer's issued Service VIP, listener declarations and probe results
   from the existing public describe snapshot; derive the accepted mesh name
   `<peer-id>.svc.overdrive.local` rather than injecting a frontend address.
3. Require the client Service guest to read its production
   `/etc/resolv.conf`, resolve the mesh name, and report the exact configured
   gateway nameserver before completing TCP by name, TCP to the issued VIP
   and its retained off-node TCP/UDP actions.
4. Require the peer Service guest application to observe each request and
   return a byte-distinct reply. For TCP-to-VIP, use the closed mTLS oracle
   below; a successful application reply alone receives no mTLS credit.
5. Drive the exact healthy → unhealthy → healthy transition and require the
   public probe-result surface to show both TCP and HTTP follow it, as pinned
   below. Do not substitute a direct `TcpProber`/`HttpProber` call.
6. From a route-disjoint peer namespace, open a real mTLS client connection to
   the registered VM tuple and exchange bytes with the deployed guest. The
   accepted production composition has no other path from that tuple's leg C
   to the VM application; the byte exchange plus held socket identities is the
   external leg-C → leg-S → `GuestStreamDial` oracle. The peer credential must
   chain to the same production workload CA. Private fixture code may use the
   existing CA persistence/`Ca` contracts; it must not add a `ServerHandle`
   identity accessor, inject an `IdentityMgr`, replace the host CA adapter or
   bypass peer verification.
7. Reuse an external socket-table observation to require leg C's same-node
   peer to be `127.0.0.1:<ephemeral>` while the client-Service flow is held.
8. Exercise the same-node dial first in GuestKit's existing no-default-route
   topology and then with a disposable test-owned default-route/uplink
   control. Seal an AF_PACKET capture before final drain and require zero
   packets for the same-node tuple on that uplink. Remove the control route and
   interface before teardown and audit the complement.
9. Run the exact bounded `IP_BIND_ADDRESS_NO_PORT` set/unset comparison pinned
   below. The production path remains the set path; the unset comparison is a
   test-local kernel control and never a production flag or override seam.
10. Stop every Service/Job through the existing public operator verb and keep
    the existing empty-owner/VMM/host cleanup complements.

All topology, capture and remote-client effects remain inside the disposable
outer GuestKit guest. No kernel, route, nft, BPF, cgroup, namespace or real-VM
effect may occur directly on the physical Metal host.

### Fail-closed mTLS oracle

Before deploying the client Service, the body knows and records the exact
accepted client-to-peer destination as `(peer issued VIP, peer TCP application
port)` and the peer allocation's leased backend tuple. It then arms the
existing loss-accounted AF_PACKET/TCP reassembly support on loopback before
the client application can send its unique byte-distinct request and response
markers.

While the TCP-to-VIP connection is held:

1. Poll `ss -e` plus `/proc/self/fd`/socket-inode ownership and require exactly
   one eligible leg-F socket whose tuple correlates bidirectionally with the
   captured peer flow. Zero or multiple candidates fail.
2. Require its socket record to contain `tcp-ulp-tls`, TLS version `1.3`,
   `txconf: sw` and `rxconf: sw`; require the inode to have exactly one live
   in-process fd owner. This reuses the closed predicate already implemented
   by `record_has_bidirectional_tls13_ktls` and the candidate correlation in
   `guest_stack_mtls_egress.rs`.
3. Seal the capture after the byte-exact application reply, drain it, and
   require `PACKET_STATISTICS.drops == 0`, no `MSG_TRUNC`/`MSG_CTRUNC`, no
   wire/snapshot-length mismatch, no malformed or ambiguous TCP reassembly,
   and the exact tuple on loopback only.
4. Require at least one valid TLS 1.3 application-data record in each
   direction on that tuple and require zero occurrences of either unique
   cleartext marker in the reassembled payload. An empty capture, a capture
   with only handshake traffic, or a lossy/ambiguous capture fails.

This proves the production frontend/VIP branch selected real bidirectional
kTLS rather than cleartext. It adds no production observation hook.

### Fail-closed probe-state oracle

The peer Service declares exactly two readiness probes in this order:

- `(role=readiness, idx=0)`: `ProbeMechanic::Tcp` to the dedicated TCP probe
  listener; and
- `(role=readiness, idx=1)`: `ProbeMechanic::Http { path: "/healthz" }` to the
  dedicated HTTP listener.

The peer application starts healthy: the TCP listener is bound and HTTP
returns 200. The client Service sends a byte-distinct state command over the
peer's separately declared control listener. `unhealthy` closes the TCP probe
listener and makes `/healthz` return 503; `healthy` rebinds the TCP listener
and restores HTTP 200. The control connection is an ordinary guest action
through the same production mesh path; no host file, direct prober call or
test hook changes state.

Using only the public describe snapshot, the body must:

1. match the two declared descriptors by role/index/mechanic and every result
   to the unchanged peer allocation id; independently retain the client
   Service allocation id as the state-command actor;
2. observe `Pass` for both within 30 seconds of the peer reaching Running;
3. after `unhealthy`, observe `Fail` for both within three declared probe
   intervals plus their timeout, with each `last_observed_at_unix_ms` strictly
   greater than its prior Pass row;
4. after `healthy`, observe `Pass` for both within the same bound, with each
   timestamp strictly greater than its Fail row; and
5. retain the same peer allocation throughout, so a restart or replacement
   cannot masquerade as recovery.

Absence, a wrong role/index/mechanic, a stale timestamp, only one probe
changing, a console-only claim or a replacement allocation fails.

### Loss-accounted with-default no-uplink oracle

Inside the disposable outer GuestKit guest, create a uniquely named veth
uplink `ovdv6up0` to a sink namespace and a test-owned default route through
it. Before the client-Service same-node dial, read back the interface as UP,
its ifindex/address and the exact default route. Bind an AF_PACKET capture to
that ifindex, then send one test-owned positive-control frame to the sink and
require the capture to observe it; zero later matching frames is therefore not
credited to a blind capture.

The capture is armed before the client-Service dial and filters the exact
outgoing same-node backend tuple and direction. Hold the connection until an
external socket-table read finds exactly one leg-C socket local to the peer
workload tuple whose peer is `127.0.0.1:<nonzero ephemeral>`. Only after the
byte-exact round trip does the fixture seal/drain the capture.

The body requires:

- the positive control was observed;
- zero matching uplink frames for the same-node tuple;
- `PACKET_STATISTICS.drops == 0`;
- no truncated frame, ancillary truncation, length mismatch, malformed IP/TCP
  frame or parser ambiguity; and
- the exact held leg-C socket identity above.

Finally delete the default route and both veth ends/namespace, read back that
the route/interface are absent, and retain the existing global cleanup audit.
Every cleanup runs on assertion failure. Zero frames without the positive
control and loss/statistics complements fails.

### Population-bounded `IP_BIND_ADDRESS_NO_PORT` comparison

Keep the production option-set path unchanged. To make exhaustion bounded by
the existing F7 limit, the fixture performs this entirely inside the
disposable guest before the comparison:

1. Read and retain the guest's original
   `/proc/sys/net/ipv4/ip_local_port_range`.
2. Select a conflict-free contiguous 64-port subrange inside that actual
   range, set it as the disposable guest's current range, and read it back.
   Abort if any port in the selected window is already occupied.
3. After production listeners/API sockets are live, externally enumerate any
   occupied `127.0.0.1` source ports in that window. Let `C = 64 - occupied`
   and pin the comparison population to `N = C + 1`; require `2 <= N <= 65`,
   below `MtlsLimits::default().max_inflight_per_alloc == 128` and the 4,096
   guest-flow quota.
4. Have the peer Service declare and bind `N` distinct TCP destination ports.
   Repeating one four-tuple is forbidden.

For the **unset control**, bind `N` test-owned listeners at distinct fixed
ports on another loopback address, outside the selected source-port window.
Test-local client sockets explicitly read back `IP_BIND_ADDRESS_NO_PORT == 0`,
bind `127.0.0.1:0`, connect one-to-one to those listeners and remain held.
Exactly `C` connections must succeed with `C` distinct source ports in the
selected range; connection `N` must fail with the kernel's
ephemeral-exhaustion errno (`EADDRINUSE` or `EADDRNOTAVAIL`) and must create no
socket row.

After closing and externally proving the unset cohort absent, the **production
set cohort** is opened by the client VM Service through the ordinary mesh/VIP
path to the same `N` registered destination tuples. Require all `N` byte-exact
connections to remain held. Correlate their exact socket rows to live
in-process fds and require every fd's `getsockopt(IP_BIND_ADDRESS_NO_PORT)` to
be `1`; require `N` tuples but fewer than `N` distinct loopback source ports,
which proves safe port sharing beyond the unset capacity. Zero/multiple fd
owners, a tuple outside the registered population or an unobserved option
fails.

Bound cohort creation/observation to 30 seconds, close every socket/listener,
restore and read back the original ephemeral range, and require no cohort
socket, route, namespace or interface remains. The test never toggles a
production option and never exposes an option flag.

### Expected source impact

The bounded implementation may touch only existing test/support surfaces
needed by the accepted scenario:

| Existing file | Required correction |
| --- | --- |
| `crates/overdrive-cli/tests/integration/vsock_walking_skeleton.rs` | Remove only the deferred Service-VIP UDP-to-VM action and its gate from the existing P6-S13 body; preserve the completed empty-UDP assertion and every remaining V-6/V-LEGB-SERVE oracle. |
| `crates/overdrive-cli/tests/integration/vsock_support.rs` | Private Service-deploy/describe, socket-observation, route-control and capture support using existing production entrypoints. |
| `crates/overdrive-testing/bin/guest_peer.rs` | Retain the existing workload modes for genuine zero-length off-node UDP and the remaining V-6 client/peer actions; remove only the deferred Service-VIP UDP-to-VM requirement. |
| `crates/overdrive-testing/src/guest_app.rs` | Retain the existing shared test protocol and honest empty-UDP/Service observations needed by the corrected checklist; remove only now-unused deferred-action protocol state. |
| `crates/overdrive-control-plane/tests/integration/guest_flow_through_serve.rs` | No change expected; its existing TCP-probe and same-node Relay observations remain independent complements. |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | No selector/count change expected. A change is allowed only for bounded per-body topology setup/cleanup if the existing body cannot own it safely. |

No production crate, public API, config/wire format, error variant, model,
roadmap or DESIGN artifact is part of this remedy. If honest remote credential
setup or a required observation cannot be built with private test support and
external kernel/public operator surfaces, stop and report that exact
testability gap; do not add a production hook.

## Required evidence after authorization

The correction is complete only when an independent DISTILL reviewer confirms
the existing scenario was completed without adding a test or weakening an
outcome, followed by:

- a Metal-only format/check/Clippy gate for every changed test target;
- `cargo xtask guest-kit-prepare` rebuilding the existing guest program;
- the unchanged existing GuestKit runner reporting exactly 117 passed, zero
  failed/ignored/runtime-SKIP bodies;
- source/receipt evidence for every retained matrix row above, including an
  actual empty datagram, public DNS/TCP-VIP/probe surfaces, remote inbound TCP,
  socket identity and sealed no-uplink capture;
- full durable run metadata/output/terminal records and source digest;
- read-only physical-host cleanup showing zero VMMs, nft tables and STEP 14-01
  workdirs; and
- a refreshed full STEP 14-01 DELIVER review.

The existing `410a…` receipt remains valid proof of the currently covered
subclaims and of the landed production beacon/ReadHangup corrections. It must
not be relabelled as proof of the missing stimuli.
