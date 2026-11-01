# STEP 14-01 UDP-deferral DISTILL alignment review

## Review metadata

| Field | Value |
| --- | --- |
| Feature / roadmap step | `netns-density-295` / `14-01` |
| Review date | 2026-10-31–2026-11-01 |
| Role | Fresh independent DISTILL acceptance-source and evidence reviewer |
| Review iterations | 1–28 |
| Reviewed alignment | User-approved deferral of Service-VIP UDP delivery to a VM backend from #295 to #310 |
| Design authority | `design-14-01-udp-deferral.md`, independently approved by `review-design-14-01-udp-deferral.md` |
| Canonical authority | Feature-delta V-6 and roadmap STEP 14-01 criterion 3 |
| Verdict | **APPROVED — PRIVATE SEEDED-FIXTURE PROCESS CLEANUP IS EXACT, LIFECYCLE-CORRECT AND EVIDENCE-CLEAN; PRODUCTION AUDIT REMEDIATION REMAINS OUTSIDE THIS REVIEW** |
| Open findings | 0 DISTILL source findings; iteration 27's RED remains historical necessity evidence, while the current production audit remediation is not reviewed here |
| Roadmap disposition | Validation is approved after independent DESIGN release. Private fixture findings do not falsify DESIGN or change that status; this reviewer changed no roadmap state |

This review applies the current `AGENTS.md`, the repository's development,
testing, debugging, verification and spike rules, and the nWave DISTILL,
acceptance-review, completeness, test-design and BDD criteria. The
repository's Rust acceptance bodies and native-Markdown review convention
control over generic reviewer examples.

The reviewer created only this review artifact. No production source, test or
fixture source, DESIGN/DISTILL handoff, roadmap, DES event, execution log,
receipt, host state or commit was changed.

## Reviewed artifacts and identities

| Artifact | SHA-256 |
| --- | --- |
| Approved UDP-deferral design | `56d8f7585160c5c7f77b5a304b29a0ae855abc9e0b59c05c6e27b218dd697b39` |
| Independent DESIGN review | `c8bf839ecd5c8440597fe2196dcd10fb6783b9ac0dc78561dbbb43b50f105588` |
| DISTILL alignment handoff | `dbf485e2de42ce1c2bcdeeb9fc643b94af57bbb99472c79325ac6f2b9b3504a1` |
| Canonical feature delta | `e58b0019e05cf894e6febde04bd21d3a49cbb15e14deed05b87828f11fcb8f74` |
| Canonical roadmap | `acee972da92cb6dfa357b24e821acd179c49b70d29f145ed7b93a979d81c32a6` |
| Existing P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` |
| CLI private native support | `08d07e0336f8e6c86902bd082e98541c7ef09b5f3840cf1b14762412cebff2e1` |
| Existing guest workload program | `a83191b2e9b817dfa7da27bf806fc406fec2a361688fba81a03705675e4e4136` |
| Shared test protocol / remote peer | `395c3fc937de129f66a751edf1226be5f928f592d003fb98ab11219d2c80f4c7` |
| Unchanged existing GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| Unchanged five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` |

`roadmap.json` parses as JSON and the current workspace diff passes
`git diff --check`. The reviewed executable source contains no production
crate edit.

## Authority and exact contract boundary

The approved design records the user's explicit option-D decision:

> “yeah, let's not do that now. add all the relevant info to the github issue,
> and then we keep it deferred to #310 and correct the contradictory
> acceptance checklist.”

That authority removes exactly one #295 acceptance outcome: delivery of UDP
through a Service VIP to a VM backend. Canonical feature-delta V-6 removes
the former Service-VIP UDP clause and names its deferral to #310 under D12.
Roadmap criterion 3 removes the corresponding action and retains the
production `serve` plus public `deploy` boundary. Criterion 4 continues to
forbid test-installed product state and override seams.

The alignment does not interpret the decision as a global UDP removal:

- off-node non-empty and zero-length UDP remain mandatory;
- gateway-DNS traffic remains a real datagram association;
- D24's general connected-host-socket/`connect4` contract remains
  canonical;
- the D24/ADR-0164 guest-prefix `Policy` refusal remains fail-closed;
- no non-VM backend is inserted to make the removed action pass; and
- #310 receives no association identity, peer-address encoding, reply-routing
  mechanism, owner, lifecycle, wire shape, typed error or public API from this
  alignment.

The feature-delta diff changes only the V-6 row. D24 and D24a remain
unchanged. No BPF, dataplane, control-plane production, reconciler, host or
worker source changed. The existing general UDP and D24 mechanism tests remain
outside this source delta.

## Exact executable alignment

### Deferred action removal

**Result: PASS.**

The four P6-S13/test-support files contain no `V6_UDP` or `vip-udp`
symbol. The aligned source removes the complete test-owned representation of
the deferred action:

- no Service-VIP UDP port or request/response records in `guest_app.rs`;
- no peer-Service UDP listener in
  `vsock_support.rs:293-321`;
- no peer-guest Service-VIP UDP server;
- no client-guest socket connected to the issued VIP over UDP;
- no `vip-udp ok` success marker; and
- no P6-S13 assertion or `paths-ready` dependency on that marker.

`paths-ready` now follows the retained mesh-name and issued-VIP TCP
byte-exact exchanges. The source does not replace the removed UDP action with
a fabricated peer reply, direct map write, private backend publisher, adapter
override or alternate Service kind.

At accepted-outcome level, no second action was removed. The former
test-private state-control port was folded into the held issued-VIP TCP stream:
the client sends the same acknowledged healthy/unhealthy commands through the
ordinary production mesh path, and the public probe oracle is unchanged. That
is private DISTILL fixture HOW, not removal of a public outcome, scenario or
product interface.

### Retained oracle audit

| Retained outcome | Current stimulus and fail-closed oracle | Review |
| --- | --- | --- |
| Job off-node TCP, non-empty UDP and genuine empty UDP | Public deploy starts the VM Job. `RemotePeer` records exactly one TCP request, one tagged datagram and one real `recv_from(n == 0)` datagram, all from the node address. The guest console must contain all three success markers, the row must be `Terminated`, and its VMM must be absent (`vsock_walking_skeleton.rs:50-100`). | **Preserved.** |
| Peer VM Service identity | A distinct peer Service is submitted through public deploy. Public describe must supply its issued VIP, leased workload address, application listener and exact readiness descriptors (`vsock_walking_skeleton.rs:102-126`; `vsock_support.rs:293-321`). | **Preserved.** |
| Initial and changing TCP/HTTP readiness | Descriptor index 0 must be TCP/18082 and index 1 HTTP `/healthz`/18083 on the same allocation. The body requires initial Pass, newer Fail after the acknowledged unhealthy command, and newer Pass after the acknowledged healthy command. Missing, partial, stale or replacement-allocation evidence fails (`vsock_walking_skeleton.rs:125-179,220-271`). | **Preserved and currently semantic RED.** |
| Route-disjoint remote inbound TCP | A remote client SVID is issued from the same persisted production workload CA. The client verifies chain/signature/time and a SPIFFE URI, then exchanges distinct records through the registered backend tuple; the peer console must record the request (`vsock_walking_skeleton.rs:128-133`; `vsock_support.rs:148-223,463-566`). | **Preserved.** |
| Client VM Service gateway DNS, mesh name and off-node TCP/UDP | A separate client Service reads production `/etc/resolv.conf`, resolves the derived peer name, requires the unique name reply, and performs retained off-node TCP, non-empty UDP and empty UDP. Final peer cardinality and node-source assertions require both the Job and client Service contributions (`vsock_walking_skeleton.rs:135-158,197-200`; `guest_peer.rs:522-619`). | **Preserved.** |
| Issued-VIP TCP mTLS/no cleartext | The client dials the public issued VIP and requires a byte-distinct reply. Pre-armed loopback capture plus the held exact backend tuple must show one fd/inode with `tcp-ulp-tls`, TLS 1.3 software TX/RX, application-data records in both directions, no plaintext markers, zero drops/truncation and unambiguous parsing (`vsock_walking_skeleton.rs:139-160`; `vsock_support.rs:1069-1312`). | **Preserved.** |
| V-LEGB-SERVE with-default no-uplink | A disposable default route/uplink is read back, a positive-control datagram proves capture is live, the held exact backend has one loopback leg-C peer, and the sealed capture must contain zero backend-tuple uplink frames with zero loss. Route/link/netns cleanup remains bounded (`vsock_walking_skeleton.rs:136-139,187-195`; `vsock_support.rs:977-1048,1069-1251`). | **Preserved.** |
| Bounded `IP_BIND_ADDRESS_NO_PORT` comparison | The unset cohort reads option 0, derives `N=C+1` from the narrowed range, requires unique in-range sources, accepted exhaustion and exact live-row absence. The production cohort requires all exact peer backends, option 1 on one fd per inode, in-range sources, fewer source ports than destinations and post-shutdown absence (`vsock_walking_skeleton.rs:135,181-186,215-217`; `vsock_support.rs:618-975`). | **Preserved.** |
| Public stop and cleanup | Both Services stop through the public verb, their named allocations become terminal, both VMMs disappear, serve shuts down, population rows are absent, and the original ephemeral range is restored and read back (`vsock_walking_skeleton.rs:202-217`). | **Preserved.** |

None of these oracles accepts “some” where the approved contract says exact,
turns a required state into an optional state, converts an absence assertion
to a best effort, or credits a console marker without the corresponding
public/kernel/wire observation.

### Private support corrections

The aligned source also contains private corrections that preserve the same
retained scenario:

- `V6ClientSpec` packages private deploy arguments without changing the
  submitted Service shape;
- the held issued-VIP TCP stream carries the state commands, preserving the
  production mesh path while eliminating a test-private control listener;
- HTTP waits for a real request before replying, and TCP readiness uses
  `SO_REUSEADDR` so the accepted unhealthy-to-healthy rebind can be
  observed honestly;
- population connections retry only within one fixed 30-second deadline and
  still require every exact response;
- unset-control sockets reset on drop and re-prove the measured capacity;
- `ss -i` continuation lines are retained so the kTLS metadata oracle
  cannot lose its evidence; and
- longer outer waits remain bounded and do not bypass any success predicate.

These are test-support decisions owned by DISTILL. They do not add or remove
an accepted action, product owner, model, lane or identity. Each retry still
requires the same exact response; each timeout still fails closed.

The Clippy correction in `RemotePeer::wait_for`
(`guest_app.rs:692-705`) changes only the test-support timeout carrier from
`PeerLog` to `Box<PeerLog>`. Success still returns the same log value;
timeout still returns the same last snapshot, now boxed. Existing callers
format or inspect the dereferenced log unchanged. The change adds no waiver,
skip, product error or semantic branch.

## Executable identity and accounting

**Result: PASS.**

The GuestKit runner and five `guest_flow_through_serve` bodies retain their
iteration-9 hashes. The runner still selects:

- one exact CLI identity,
  `integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy`;
  and
- the same five exact control-plane identities.

The constants remain one CLI body plus five control-plane bodies, with the
existing `111 + 1 + 5 = 117` accounting and line-oriented runtime-SKIP
rejection. All six bodies retain
`/// CONTRACT_SHAPE: bounded-change.`. No identity, selector, count, lane,
test binary or scenario was added or renamed.

The native receipt is not a 117-pass receipt. It honestly stops in the CLI
body before the runner launches the five control-plane bodies. The fixed
source inventory remains 117; native completion remains pending production
remediation and a later exact-source run.

## Durable source-gate evidence

**Result: PASS.**

The final source gate and native runner records both name commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242` and identical synchronized
source digest
`f4fac4fe3c0f99241c5ad3d8757780df6a2cc2df0b1128889fa6a399303ad6da`.

| Purpose | Run | Exit | Output SHA-256 | Terminal SHA-256 |
| --- | --- | ---: | --- | --- |
| Final aligned source gate | `50d017156bfcb85944a1bd32` | 0 | `60d76a98d61479ef92439a8e7776fe7d7b79a9a757590e522c06bcb779bdfdfc` | `836d4897578c1c1f22c71e40ea2d81be0af87a1d9a400ecd7cfc225e7b09f01c` |
| Existing cgroup-default runner | `a6dd517d4325327656fd47a6` | 101 | `5080fc263aee729183c28e77765da823a17504945a64357bc49d537deb2c15fd` | `b16149f022d8e8478d5152880036c41e628fec414373d03d933ef011ce75a0a3` |
| Read-only physical-host audit | `d87ba6b4e1fb0bb400f79e39` | 0 | `9950f1bb72eff3d71a6cc0fb74fea3983a487b4808299a494995890264b46f9e` | `3420959aa65ee3f2a6e7441cdd8388b454deed004c09073967b22d86794cb4a3` |
| Pre-existing namespace diagnosis | `41f74f715171fd6cfb5cdb4f` | 0 | `59ef9ab082a8f30b72538d7fcfaf4b18bfd6cb73e8fdcc8085a05828b47addb9` | `0dfa1efde6a6fd11d2cb44fc2fb63df0b61977c3b59a64b99d42fe0919d9e96a` |

The source gate ran rustfmt, guest-workload check/Clippy and CLI integration
check/Clippy with the required feature sets and `-D warnings`. It exited
zero. The identical digest proves the native semantic RED was not produced by
a later or earlier source variant.

## Native semantic RED review

**Disposition: honest retained production-boundary semantic RED; route to the
STEP 14-01 crafter without weakening the fixture.**

The native record first retains 101 successful libtest summaries totaling
exactly 111 passed, zero failed and zero ignored. It then invokes the exact
P6-S13 identity once. The existing Job phase has already passed because
control reaches peer-Service deployment: the off-node TCP, non-empty UDP,
empty UDP, node-source, terminal-row and VMM-gone assertions all precede that
deployment.

The peer VM Service is submitted through the public deploy path. The guest
boots production init and prints `v6-peer: ready population=65`. Public
describe exposes:

- inferred startup TCP/18080, with `Pass`;
- readiness index 0 as TCP/18082; and
- readiness index 1 as HTTP `/healthz`/18083.

At the 30-second deadline both explicit readiness results on
`alloc-v6-peer-0` are still
`Fail { last_fail_reason: "unreachable" }`. Neither required initial
`Pass` exists, so `wait_probe_state(..., true, ...)` panics at
`vsock_walking_skeleton.rs:261`. The client Service, remote-inbound action,
mTLS capture, later probe transitions, population/no-uplink observations and
success cleanup are not reached and receive no native credit.

The failure is not manufactured by the deferred UDP removal. It occurs before
the former Service-VIP UDP action's position and on a retained criterion.
The peer application binds the HTTP listener synchronously before its ready
marker, attempts the TCP readiness bind on its dedicated thread, and logs bind
failure; the retained 30-second public observer does not see a successful
production probe. The inferred application-port startup probe passing confirms
that this is beyond guest boot and startup admission. This is sufficient
production-boundary reproduction; the review does not assign a root cause or
prescribe a product mechanism.

The test must not be made green by deleting the initial Pass requirement,
substituting direct `TcpProber`/`HttpProber` calls, accepting console
readiness, moving the criterion again, or manufacturing a result row. The
original STEP 14-01 crafter owns the bounded product investigation against the
retained design.

The raw output contains four lowercase `skipped` occurrences, all kernel
“Calibrating delay loop” boot messages, and zero recognized runtime-SKIP
diagnostics. This corrects the alignment handoff's incidental count of three;
it does not change its zero-runtime-SKIP conclusion.

## Cleanup and confinement

**Result: PASS for failed-run containment; success cleanup remains unproved.**

The failure teardown kills the remaining `alloc-v6-peer-0.scope` rather than
hanging. The read-only physical-host audit reports zero live Cloud Hypervisor
processes, no nft table and no STEP 14-01 work directory. It reports
`ovd-ns-0000`; the follow-up diagnosis proves that namespace predates this
alignment (mount mtime 2026-10-26), contains no process and retains the
pre-existing `ovd-wl-0000` veth/address. The alignment neither claims it
created nor removes it.

All real-VM, cgroup, nft, namespace, route and packet-capture effects remain
inside the qualified Metal/GuestKit boundary. Lima was not recreated. Because
the body stops at initial readiness, public Service stop, graceful serve
shutdown, population absence and range-restore assertions receive no
success-path credit.

## Traceability corrections and non-blocking notes

The handoff's semantic conclusions are sound, with two evidence-label
corrections recorded by this review:

1. the raw native output contains four, rather than three, incidental
   lowercase `skipped` kernel lines; and
2. the current mTLS/capture implementation is at
   `vsock_support.rs:1069-1312`; the handoff's
   `vsock_support.rs:568-756` range covers identity/range support and is not
   the complete capture oracle.

Neither correction changes an outcome, source behavior, receipt identity or
verdict. The mappings in this review are controlling for acceptance.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Success, state failure/recovery, route mode, exhaustion and cleanup complements remain explicit; current evidence stops at the first retained failure. |
| GWT / one behavior | 9/10 | One cumulative P6-S13 journey retains clear Job, peer Service and client Service phases. |
| Domain language | 10/10 | Service, issued VIP, mesh name, readiness result, remote inbound, leg C, uplink and option-state terms match the canonical contract. |
| Coverage completeness | 10/10 in source | Every retained V-6/V-LEGB-SERVE outcome has a concrete production-boundary stimulus and oracle; only the explicitly deferred outcome is absent. |
| Walking-skeleton user value | 10/10 | The body drives the public operator journey and fails on the first missing retained platform outcome. |
| Priority validation | 10/10 | The alignment removes only the user-approved #310 deferral and preserves all higher-priority security/cleanup invariants. |
| Observable behavior | 10/10 in source | Presence, absence, confidentiality, state transition, loss and exhaustion checks remain fail-closed. |
| Traceability | 10/10 | Canonical clauses, source identities, current line mappings and durable receipts are reconciled. |
| Walking-skeleton boundary | 10/10 | Real production owners remain composed; fixtures provide only workloads, environment and external observation. |

## Test-design mandates and completeness

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Public deploy/describe/stop and production `serve` own product state; no adapter or map override is introduced. |
| CM-B — domain abstraction | PASS | P6-S13 expresses the retained operator outcomes; kernel parsing/capture remains private support. |
| CM-C — complete user journey | PASS in source / RED in product | Every retained phase is present; initial TCP/HTTP readiness blocks native completion. |
| CM-D — fixture isolation | PASS | Test-owned CA/topology/range/capture effects remain disposable and GuestKit-confined. |
| CM-E — universe-bound assertion | PASS / layer N/A | Exact public rows, guest bytes, kernel tuples and loss/accounting complements bind the observation universe. |
| CM-F — layer-dependent PBT | PASS | No generative native body is added; this remains an explicit Tier-3 example. |
| CM-G — two-tier rich journey | N/A | No new scenario or in-memory tier is introduced. |
| CM-H — explicit integration paths | PASS in source | Every retained named path has a positive, negative/loss and cleanup oracle appropriate to its boundary. |

The C1-C7 completeness categories remain populated by retained success and
boundary values (including empty UDP), application-state failure/recovery,
route and option modes, mTLS/identity robustness, bounded concurrency and
cleanup. The explicitly deferred Service-VIP UDP-to-VM action is not counted
as #295 coverage and no substitute evidence is invented.

## Finding dispositions

### Blocking findings

None.

### Non-blocking evidence-label corrections

The skip-occurrence count and mTLS source range are corrected above. No author
remediation is required because this review supplies the exact controlling
evidence labels and neither statement participates in the alignment decision.

### Production semantic RED

The missing initial TCP/HTTP readiness Pass pair is not a DISTILL finding to
weaken or redesign. It is reproduced through the retained production boundary
and is routed to the STEP 14-01 crafter.

## Verdict

**APPROVED.** The alignment implements the exact user-approved deferral:
Service-VIP UDP delivery to a VM backend is removed from #295 P6-S13 and no
other accepted outcome is removed or weakened. General D24/global UDP,
guest-prefix refusal, the same six identities, fixed 117 accounting,
production composition and all retained fail-closed oracles remain intact.

The boxed `PeerLog` timeout carrier and the other private support
corrections are neutral or strengthen observation of the same accepted
scenario. They add no production mechanism, public product API, lane,
scenario or selector.

The exact-source native run is an honest semantic RED at the retained initial
TCP/HTTP readiness outcome. DISTILL must not weaken that oracle to obtain
GREEN. The alignment is approved for handoff to the STEP 14-01 crafter, with
later complete native evidence and independent DELIVER review still required.

# APPROVED



# Review iteration 2 — health recovery control-channel assessment

## Iteration 2 metadata

| Field | Value |
| --- | --- |
| Scope | Current post-HTTP-fix P6-S13 recovery failure only |
| Production candidate | `crates/overdrive-worker/src/probe_runner/http_prober.rs` |
| Native run | `3cb8fe14a208f2f315e8f6e5`, exit 101 |
| Diagnostic run | `48a5b24dd855d5fa22e72d82`, exit 101 |
| Current failure | Client emits `state=unhealthy`, then cannot emit `state=healthy`; P6-S13 panics at `vsock_walking_skeleton.rs:175` |
| Crafter hypothesis | Readiness withdrawal resets the held issued-VIP TCP state-control stream and therefore implies a production lifecycle gap |
| Assessment | **Hypothesis rejected as a production finding; private fixture channel is outside the accepted contract** |
| Iteration verdict | **NEEDS PRIVATE DISTILL REMEDIATION** |

The reviewer changed only this review artifact. No production, acceptance or
test-support source, DESIGN artifact, roadmap, DES event, execution log,
receipt or commit was changed. No Metal, native, mutation or local execution
was launched by the reviewer.

This iteration supersedes only iteration 1's routing of the then-current
initial-probe semantic RED. The HTTP adapter candidate moves the same body
past that boundary. Iteration 1's UDP-deferral approval, retained-oracle audit,
identity/accounting review and evidence-label corrections remain controlling.

## Current source and evidence identity

| Artifact | SHA-256 / evidence |
| --- | --- |
| Production HTTP prober candidate | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` |
| Current guest workload fixture | `a83191b2e9b817dfa7da27bf806fc406fec2a361688fba81a03705675e4e4136` |
| Current P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` |
| Current CLI private support | `08d07e0336f8e6c86902bd082e98541c7ef09b5f3840cf1b14762412cebff2e1` |
| Native output | `3cb8fe14a208f2f315e8f6e5`; output SHA-256 `cae076fadab7addc08bcdfd4c9e8f1ef0ccba84a0816833555e20609d87ea4e3` |
| HTTP diagnostic output | `48a5b24dd855d5fa22e72d82`; output SHA-256 `c570fecfc07ee6f5132c06b87755cb30816d137dedf1c459e480a33bf8d660aa` |

The locally retained `3cb8…` and `48a5…` collections contain the complete
command output and collection terminal text but not the canonical
`metadata.v1` source-digest record. This review therefore uses them for the
observed behavioral sequence and does not invent a source-digest claim. The
crafter-reported ordinary gate `45dd5c9e1d43a3f6ae99cf390` is not present
in the retained raw tree and receives no independent durable-evidence credit
here.

## HTTP prober correction disposition

**Disposition: bounded production correction remains supported.**

The pre-fix diagnostic `48a5…` reached a real paired
`GuestStream` and captured the complete guest HTTP response:

`HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK`.

The then-current adapter nevertheless wrote HTTP readiness Fail with the
diagnostic response bytes. The current candidate retains the in-flight
`GuestStream` read/write/shutdown futures across Hyper polls instead of
recreating and cancelling them on every poll
(`http_prober.rs:298-471`) and drains the returned body before classifying
the status (`:242-265`). It changes no `HttpProber` public trait, status
classification, timeout, URL, probe row or lifecycle contract.

Native `3cb8…` progresses beyond the initial TCP/HTTP readiness gate that
failed before this correction. It reaches retained off-node UDP, gateway DNS,
mesh-name TCP, issued-VIP TCP, initial readiness Pass and the later readiness
Fail. That is non-vacuous boundary evidence that the HTTP 200 is now observed
as Pass. This review does not give the uncommitted product candidate final
DELIVER approval; it records that the later failure does not falsify or require
reverting the bounded HTTP adapter fix.

## Accepted readiness behavior

The canonical retained V-6 outcome is public
`Pass -> newer Fail -> newer Pass` for TCP and HTTP readiness on the same
allocation. It does not specify that an already-established Service-VIP stream
survives eligibility withdrawal.

The accepted Service/readiness contracts are more specific:

- `ServiceLifecycle` derives each Running backend's `healthy` flag from
  the current readiness producer row
  (`service_lifecycle.rs:1027-1155`).
- A Fail makes the backend unhealthy; the service-map hydrator emits
  `DeregisterLocalBackend` for an unhealthy local backend
  (`service_map_hydrator.rs:601-647`).
- New frontend/VIP/backend TCP classification selects only healthy backends
  and returns `MeshUnreachable` when the known Service has none
  (`mtls_resolve_adapter.rs:524-655`).
- The accepted E11 outcome proves traffic with separate client Jobs before,
  during and after the failed window. Its application changes readiness by a
  fixed `204 -> 503 -> 204` sequence; recovery does not depend on traffic
  through the withdrawn Service.
- Readiness changes backend eligibility only. Running, Stable and restart
  count remain unchanged; ProbeRunner supervision is cancelled on terminal
  allocation lifecycle, not by readiness withdrawal.
- The governing service-kind feature delta states explicitly:
  “Existing connections and historical-listener cleanup are excluded”
  (`docs/feature/service-kind-vm-workloads/feature-delta.md:888-892`).

No accepted contract promises connection retention, graceful drain, or
continued delivery on an established frontend/VIP connection after readiness
withdrawal. In fact, requiring the health-recovery stimulus itself to traverse
the failed Service creates a circular test dependency that E11 deliberately
avoids.

## Actual current fixture path and failure

The current test-owned client opens one issued-VIP TCP stream, verifies the
byte-distinct VIP response and retains that stream
(`guest_peer.rs:581-595`). It then sends both state-control commands over the
same stream:

1. `V6-STATE:unhealthy` is acknowledged; the peer application sets its real
   health flag false and the client prints `state=unhealthy`
   (`guest_peer.rs:336-392,597-603`).
2. P6-S13 observes both newer public readiness Fail rows on the unchanged peer
   allocation (`vsock_walking_skeleton.rs:162-170`).
3. After twelve seconds the client attempts
   `V6-STATE:healthy` on that same stream
   (`guest_peer.rs:603-607`).
4. The stream is no longer usable, the client returns its TCP-I/O failure
   path at `guest_peer.rs:603-606`. The fixture application exits; production
   init reports group completion and powers that client workload down. The
   body then times out waiting for `v6-client: state=healthy` at
   `vsock_walking_skeleton.rs:171-175`.

The `3cb8…` console proves the retained prefix of this sequence:
`nameserver=100.95.0.1`, real empty UDP, off-node success, mesh-name
resolution to `10.98.0.1`, `vip-tcp ok`, `paths-ready`, then
`state=unhealthy`; it contains no `state=healthy`. The guest then reports
group completion and powers off. This is a real failure, but the failed
operation is the fixture's private recovery command, not a promised product
connection-lifecycle outcome.

## Production-lifecycle hypothesis assessment

**Disposition: REJECTED as an implementation or DESIGN finding.**

Promoting the reset of the held stream would require a new observable
contract: an established issued-VIP/mTLS flow must survive backend
ineligibility and remain capable of application traffic during the readiness
failed window. The accepted contracts do not say this; the service-kind
feature delta expressly excludes existing connections. The E11 traffic
contract checks new clients and only promises withdrawal/restoration of
eligibility and traffic, with Running/Stable and restart count unchanged.

No existing seeded Sim invariant requires established-flow survival across a
readiness transition. The existing readiness invariants prove
`Backend.healthy true -> false -> true` and new traffic withdrawal/recovery,
not flow retention. Under the workspace reachability gate, the native reset
therefore cannot be promoted into retention architecture, a new drain rule,
or a production remediation finding.

If established-stream survival were desired, DESIGN would first have to pin
its exact semantics and a seeded production-owner invariant. This review does
not request that expansion because it is unnecessary to satisfy retained V-6.

## Finding F-01 — High: recovery depends on a channel whose survival is outside the contract

**Finding owner: DISTILL test-support author.**

The recovery stimulus is currently sent over the same issued-VIP stream whose
backend has just become unhealthy. The test can therefore fail before it asks
the peer application to recover, even when production correctly withdraws the
backend and would correctly restore it after a new Pass. Treating that private
stream as durable weakens test honesty by converting an unspecified
connection-lifecycle behavior into an accidental precondition of the retained
probe outcome.

This is a fixture-HOW defect, not a DESIGN gap. It is reproduced by
`3cb8…` at the exact source path above and requires no production API,
override, seam, lane, scenario or owner change.

### Exact bounded remediation

Keep the existing P6-S13 identity, body-level public probe assertions, two VM
Services, public deploy/describe/stop composition, ports, mTLS/no-cleartext
oracle, no-uplink oracle, population comparison and cleanup gates. Change only
the existing test guest's private application-state control:

1. On the acknowledged `V6-STATE:unhealthy` command, the peer guest sets the
   real shared health flag false and emits its existing unhealthy marker.
   Independently of the control stream's later lifetime, the same peer handler
   waits the existing twelve-second failed interval, sets that same health
   flag true and emits a peer healthy marker. This is the same real
   application-state sequence as the accepted E11 timed
   `204 -> 503 -> 204` workload, not a fabricated probe row.
2. The client guest sends only the existing unhealthy command on the initial
   VIP stream. It must not send recovery on that stream or require the stream
   to survive withdrawal.
3. After the existing failed interval, the client performs a bounded fresh
   connection to the same public issued VIP and application port, using the
   existing byte-distinct VIP request/response. It retries only within the
   existing bounded window. Only after that real post-recovery round trip may
   it emit the existing `v6-client: state=healthy` marker and hold the fresh
   stream for the downstream exact leg-C/socket observations.
4. Leave P6-S13's public assertions unchanged: it must still observe the
   initial Pass pair, newer Fail pair after the unhealthy marker, and newer
   Pass pair after the fresh restored-traffic marker on the same peer
   allocation. Missing application recovery, missing traffic restoration, a
   replacement allocation, stale rows or a partial probe pair still fails.

This correction uses only the existing test workload, issued VIP, request
records and production path. It adds no test, public product surface, direct
prober, backend write, private publisher or alternate lane. No user approval
is required because the canonical outcome is unchanged and test-support HOW
belongs to DISTILL.

## Evidence disposition

| Evidence | Disposition |
| --- | --- |
| `48a5b24dd855d5fa22e72d82` | Valid diagnostic that a real paired GuestStream carried HTTP 200 while the pre-fix adapter failed classification; not a P6-S13 pass |
| Current HTTP-prober source | Bounded production candidate supported by diagnostic and later native progress; final DELIVER review remains required |
| `3cb8fe14a208f2f315e8f6e5` | Valid native reproduction of the private recovery-channel failure after initial Pass, retained network outcomes and public Fail |
| Crafter's lifecycle-gap hypothesis | Rejected; no accepted retention invariant and no seeded Sim failure |
| Remaining P6-S13 suffix | Unexecuted; population, no-uplink and success cleanup receive no new credit |

The current failure must not be made green by weakening
`Pass -> newer Fail -> newer Pass`, accepting only console state, skipping
traffic restoration, changing the peer allocation, injecting a backend row or
preserving the held stream in production. The bounded private workload
correction must be independently re-reviewed before the semantic RED is
reclassified.

## Iteration 2 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 after remediation | Initial health, real failed window, application recovery and restored public traffic remain required. |
| GWT / one behavior | 9/10 | The correction stays inside the existing cumulative P6-S13 journey. |
| Domain language | 10/10 | Backend eligibility, readiness state, issued-VIP stream and application recovery are kept distinct. |
| Coverage completeness | 9/10 current / 10/10 prescribed | The current recovery stimulus is circular; the bounded correction closes it without removing coverage. |
| Walking-skeleton user value | 10/10 | The corrected test still proves public traffic withdrawal and restoration on one Service allocation. |
| Priority validation | 10/10 | It preserves canonical readiness behavior and rejects unapproved retention architecture. |
| Observable behavior | 7/10 current / 10/10 prescribed | Current “healthy” depends on a dead private channel; remediation requires real application recovery plus fresh byte-exact traffic. |
| Traceability | 10/10 | Accepted E11 exclusions, current source sequence and native failure map directly to the owner disposition. |
| Walking-skeleton boundary | 10/10 | All stimuli remain guest application actions through production serve/deploy. |

## Iteration 2 verdict

**NEEDS PRIVATE DISTILL REMEDIATION.** The new P6-S13 failure is not a proven
production lifecycle defect and does not justify retention/drain architecture.
The accepted readiness contract excludes existing connections and proves
withdrawal/restoration with fresh traffic.

Return F-01 to the DISTILL test-support author for the exact bounded workload
correction above. The original crafter's HTTP-prober candidate remains the
appropriate bounded production correction for the earlier HTTP polling
defect; no additional production lifecycle work is authorized by this review.

# NEEDS REVISION



# Review iteration 3 — F-01 private recovery-channel remediation

## Iteration 3 metadata

| Field | Value |
| --- | --- |
| Scope | Exact iteration-2 F-01 remediation only |
| Finding owner | DISTILL test-support author |
| Remediated file | `crates/overdrive-testing/bin/guest_peer.rs` |
| Source-gate run | `9055199a935c497303fcaea5`, exit 0 |
| Native attempts | `100cce0a3d7d451c293de4b1` and `cdead78197b4a7221fe4fa36`, both exit 101 before F-01 |
| Cleanup audit | `a3f6d3ee50385104fd330145`, exit 0 |
| Iteration verdict | **APPROVED — F-01 CLOSED IN SOURCE** |

The reviewer changed only this review artifact. No production, test-support or
acceptance source, roadmap, DESIGN artifact, DES event, execution log, receipt
or commit was changed. No Metal, native or mutation execution was launched by
the reviewer.

The roadmap's current validation status is `approved`, with the independent
DESIGN and DISTILL UDP-deferral reviews recorded as authority. Iteration 2's
private fixture finding did not contradict the approved deferral or any public
interface/lifecycle contract. This iteration neither reopens nor changes
roadmap validation.

## Exact source identity and scope

| Artifact | SHA-256 / disposition |
| --- | --- |
| Remediated guest workload | `9bf7e7e46f3ace97ef2c9dcad216e67ef1646f4617082fdd82107e9dead2a7fa` |
| P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` — unchanged |
| CLI private support | `08d07e0336f8e6c86902bd082e98541c7ef09b5f3840cf1b14762412cebff2e1` — unchanged |
| Production HTTP-prober candidate | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` — unchanged |
| GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` — unchanged |
| Five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` — unchanged |

Only the existing test guest's private V-6 application-state protocol changed.
No body, assertion, marker identity, public product API, production owner,
lane, selector, runner count, scenario or override was added or removed. The
original crafter's HTTP-prober candidate remains byte-identical to iteration 2.

## F-01 remediation audit

### Peer application recovery is independent of stream retention

**Result: PASS.**

`v6_state_control` now reads exactly one command and accepts only
`V6-STATE:unhealthy` (`guest_peer.rs:371-383`). It then:

1. sets the real shared health flag false;
2. writes the existing acknowledgement and emits
   `v6-peer: state=unhealthy`;
3. waits the existing twelve-second failed interval without further control
   stream I/O;
4. sets the same shared health flag true; and
5. emits `v6-peer: state=healthy`
   (`guest_peer.rs:384-389`).

The true transition is owned by the real peer guest application. It is not a
console-only claim, injected probe result, host file, direct prober or backend
publisher. A reset/close of the old issued-VIP stream after the acknowledgement
cannot prevent the application from restoring its actual TCP/HTTP probe state.
This exactly removes iteration 2's unapproved connection-retention
precondition.

### Client drops the withdrawn stream and proves fresh restored traffic

**Result: PASS.**

The client still obtains the initial issued-VIP stream by a bounded,
byte-distinct `V6_VIP_REQUEST` / `V6_VIP_RESPONSE` exchange and emits
`vip-tcp ok` only after success
(`guest_peer.rs:578-588,619-625`). It then:

- sends only the unhealthy command and requires the existing acknowledgement;
- emits the existing `v6-client: state=unhealthy` marker;
- explicitly drops the initial VIP stream;
- waits the same twelve-second failed interval;
- retries a **fresh** connection to the same public issued VIP and application
  port within the existing thirty-second bound;
- repeats the same byte-distinct VIP request/response; and
- emits the existing `v6-client: state=healthy` marker only after that fresh
  production-path traffic succeeds
  (`guest_peer.rs:589-605`).

The fresh stream is retained with the population streams
(`guest_peer.rs:607-616`), preserving the downstream exact app-port leg-C
socket and no-uplink observations. A stale, reset or merely console-healthy
state cannot satisfy this sequence.

### Public probe and downstream oracles remain exact

**Result: PASS.**

The P6-S13 body is byte-identical to iteration 2. It still requires on the same
peer allocation:

- both exact TCP/HTTP readiness descriptors at initial Pass;
- both newer Fail rows after the client unhealthy marker; and
- both newer Pass rows after the fresh restored-traffic marker
  (`vsock_walking_skeleton.rs:125-179,220-271`).

Its timestamp ordering, descriptor role/index/mechanic checks, same-allocation
constraint and partial-pair failure behavior are unchanged. The remote inbound
TCP, DNS/name, mTLS/no-cleartext, off-node TCP/UDP including empty UDP,
population, leg-C identity, no-uplink, public stop, VMM disappearance and
cleanup assertions are also unchanged. The existing one CLI plus five
control-plane identities and `111 + 1 + 5 = 117` accounting remain fixed.

The remediation therefore does not weaken
`Pass -> newer Fail -> newer Pass`; it makes the recovery stimulus independent
of the connection lifecycle that the accepted contract expressly excludes.

## Durable source-gate evidence

All four decisive records identify commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242` and synchronized source digest
`2fe5f3057dcfdea7d005198b9edd44f8d7cff563e4a6f5fb5930e6650127940b`.

| Purpose | Run | Exit | Output SHA-256 | Terminal SHA-256 |
| --- | --- | ---: | --- | --- |
| Decisive fixture/CLI source gate | `9055199a935c497303fcaea5` | 0 | `a7b8d638ddeb49aba592b193e7d85b4dcc23dbf6cc2e8f900fdfa9df41e50a3d` | `f77e32cf920ef410c01853e4e1284bde35ff7860850cf6db5e3e2bf218913502` |
| Exact existing runner, attempt 1 | `100cce0a3d7d451c293de4b1` | 101 | `46a17977776e57e0b97d6c053f4323e7a58a78ff9a8fa0189353fd415e9ad902` | `85ee9cb442626e918d773271aa3e07066443004a037ae9728903c5a4bd2a6c9f` |
| Exact-source retry | `cdead78197b4a7221fe4fa36` | 101 | `911cc1c631c952b96336189f19ddad6d30b5fa4a47fbaf396f1371503dcc3902` | `d9df5a0177448d812b4435872bd768d71b66fc818281ddde4ce5a8563249c808` |
| Read-only cleanup audit | `a3f6d3ee50385104fd330145` | 0 | `986b2c2e29b7b2b706e956f3dd3aa2bf51b41e5ae5667c602b761ea8e2b31de0` | `800ccb994554cacaf9a3fc86bc905d9a82c48b701320a35b3e80ee1adde592c7` |

The source gate ran the scoped rustfmt, check and Clippy requirements with
`-D warnings` and exited zero. The identical source digest across both native
attempts prevents a later source variant from being credited.

## Honest native-evidence disposition

**F-01 receives no native execution credit.**

Both exact-source native attempts retain 101 successful libtest summaries
totaling 111 passed, zero failed and zero ignored, then invoke the exact
P6-S13 identity once. Both stop at the earlier initial public readiness Pass
gate, before the client Service is deployed:

- `100cce…`: TCP readiness is `unreachable`; HTTP readiness is
  `connection reset`.
- `cdead…`: TCP readiness reaches Pass; HTTP readiness remains
  `connection reset`.

Neither attempt can execute the unhealthy command, twelve-second application
recovery, fresh VIP connection or `state=healthy` marker. The five
control-plane identities are not launched after the fail-closed CLI exit.
This review does not relabel source inspection as runtime proof and does not
credit any downstream oracle.

The repeated HTTP readiness failure is independent of F-01 and occurs before
the remediated sequence exists. It remains with the original crafter's
unchanged HTTP-prober candidate and DELIVER investigation. This DISTILL review
does not weaken the initial Pass pair, prescribe another HTTP change, or move
the production failure into the fixture.

## Cleanup and confinement

The read-only `a3f6…` audit found zero live Cloud Hypervisor processes, no
nft table and no STEP 14-01 work directory. It lists only the already
documented pre-existing `ovd-ns-0000` namespace. No source mutation or native
execution was performed by this reviewer, and no success-path cleanup credit
is claimed from the failed native attempts.

## Finding dispositions

### F-01 — recovery depends on a channel whose survival is outside the contract

**Disposition: CLOSED IN SOURCE.** The peer application now recovers
independently of the old connection; the client drops that connection and
requires a fresh, bounded, byte-exact issued-VIP round trip before its healthy
marker. No connection-retention assumption remains.

### New iteration-3 findings

None.

## Iteration 3 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Real unhealthy state, failed interval, independent application recovery and fresh restored traffic are all required. |
| GWT / one behavior | 9/10 | The correction stays within the existing cumulative P6-S13 identity. |
| Domain language | 10/10 | Application health, backend eligibility, stream lifetime and restored traffic remain distinct. |
| Coverage completeness | 10/10 in source | The retained probe journey has non-circular stimuli and every downstream oracle remains present. |
| Walking-skeleton user value | 10/10 | Fresh traffic through the public issued VIP proves recovery rather than a private marker alone. |
| Priority validation | 10/10 | F-01 closes without retention architecture or contract weakening. |
| Observable behavior | 10/10 in source | Both state transitions and post-recovery traffic are concrete, bounded guest observations followed by public rows. |
| Traceability | 10/10 | Every iteration-2 remediation clause maps to current source and unchanged body assertions. |
| Walking-skeleton boundary | 10/10 | All traffic remains through production serve/deploy with no seam or override. |

## Iteration 3 verdict

**APPROVED.** F-01 is closed by the exact private DISTILL correction. The
remediation removes the accidental established-stream survival assumption
while preserving the same P6-S13 body, public
`Pass -> newer Fail -> newer Pass` oracle, same allocation, production
composition, six identities, 117 inventory and all downstream assertions.

This is source-level approval only. The native attempts stop earlier at HTTP
readiness and prove nothing about F-01 execution. The original crafter's
unchanged HTTP candidate remains the owner of that earlier production-boundary
failure; this review authorizes no fixture weakening and no additional
production mechanism.

# APPROVED

# Review iteration 4 — bounded HTTP request-consumption correction

## Iteration 4 metadata

| Field | Value |
| --- | --- |
| Scope | Test-private HTTP handler correction only |
| Remediated file | `crates/overdrive-testing/bin/guest_peer.rs` |
| Pre-fix native evidence | `5edb0decd8f9dcd2e0da458d`, exit 101 at HTTP readiness connection reset |
| Source-gate run | `8c190158fb86f85a81fb1474`, exit 0 |
| Corrected native run | `17e58b1b7e76e89bdc58cd4f`, exit 101 later at population |
| Cleanup audit | `7ea0663ef77f63755d502eb8`, exit 0 |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support or
acceptance source, roadmap, DESIGN artifact, DES event, execution log, receipt
or commit was changed. No Metal, native or mutation execution was launched by
the reviewer.

Roadmap validation remains `approved`. This private fixture correction
neither changes nor falsifies the independently reviewed UDP-deferral design,
and this review does not alter roadmap status.

## Exact source identity and bounded change

| Artifact | SHA-256 / disposition |
| --- | --- |
| Corrected guest workload fixture | `0a1c76962a370f010b39337d8a97ea15fd469043aaba6639129835383b5feb3f` |
| Production HTTP-prober candidate | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` — unchanged |
| P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` — unchanged |
| CLI private support | `08d07e0336f8e6c86902bd082e98541c7ef09b5f3840cf1b14762412cebff2e1` — unchanged |
| GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` — unchanged |
| Five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` — unchanged |

Only the existing peer guest's private HTTP request reader changed. No body,
identity, public product API, probe descriptor, state marker, health sequence,
timeout, status/body, port, scenario, lane, selector, runner count, production
owner or override changed.

## HTTP fixture correction audit

**Result: PASS.**

Before the correction, the peer HTTP handler waited only until one byte was
peekable and then wrote its response. The pre-fix native record `5edb…`
reached the exact HTTP readiness path but reported
`Fail { last_fail_reason: "connection reset" }`. Responding before the
Hyper client finished its request write allowed the guest to close while the
request was still in flight; this is a private server-fixture framing defect,
not evidence that the production HTTP status contract should change.

The corrected handler keeps the existing one-second socket read deadline and
calls `consume_http_request_head`
(`guest_peer.rs:392-440`). That helper:

- reads through the exact HTTP header terminator `\r\n\r\n`;
- accepts at most 8,192 bytes;
- returns `InvalidData` if the head reaches the bound;
- returns `UnexpectedEof` if the peer ends before the terminator; and
- propagates the existing one-second read timeout.

Only after successful request-head consumption does the handler read the same
shared health flag, select the unchanged `200 OK` or
`503 Service Unavailable` status/body, write it and half-close its write side
(`guest_peer.rs:401-410`). It does not parse or manufacture a probe result,
extend the deadline, accept a partial request, change status classification,
or bypass the real paired `GuestStream`.

The correction is private DISTILL HOW. It makes the test server obey the
ordinary HTTP request/response boundary while preserving the original
crafter's production `http_prober.rs` candidate byte-for-byte.

## Public and downstream oracle preservation

**Result: PASS.**

The P6-S13 body and CLI support hashes are unchanged. The body still requires:

- initial Pass for both exact readiness descriptors;
- both strictly newer Fail rows after real application unhealthiness;
- independent peer-application recovery and fresh byte-exact issued-VIP
  traffic; and
- both strictly newer Pass rows on the same peer allocation
  (`vsock_walking_skeleton.rs:125-179,220-271`).

Iteration 3's F-01 remediation remains intact: the peer restores the real
shared health flag after twelve seconds, the client drops the withdrawn
stream, and `state=healthy` appears only after a fresh bounded issued-VIP
round trip. The HTTP correction changes none of that code or timing.

The remote inbound, off-node TCP/UDP including genuine empty UDP, DNS/name,
mTLS/no-cleartext, exact socket identity, no-uplink, population, public stop,
VMM disappearance and cleanup assertions remain present. The same one CLI
plus five control-plane identities and `111 + 1 + 5 = 117` inventory remain
fixed.

## Durable evidence

The decisive source gate, corrected native run and cleanup audit identify
commit `c9f2f6d6975b236fa4a049093550d3ea8dcf4242` and synchronized source
digest
`1fb156b1957d7e79fac0d76b5c5a89ddb00d4f75e2dcde77c4ee3ec0bd3e5b1c`.
The pre-fix native record carries the earlier digest
`1aac57075541e2d8c0855e00b7fcabb241d4aaa425a05fa0fbfda7da6ccd83ef`.

| Purpose | Run | Exit | Output SHA-256 | Terminal SHA-256 |
| --- | --- | ---: | --- | --- |
| Pre-fix native HTTP failure | `5edb0decd8f9dcd2e0da458d` | 101 | `1d717f1af6b361c23390ed149952b4dcda113023aa86a651b8dfcca8f58f53e7` | `fadf7682098b2b331f86d0a22a4f3934b2654f0ce94ccede750f032a330ae616` |
| Corrected fixture/CLI source gate | `8c190158fb86f85a81fb1474` | 0 | `7cb1b2c1cc599ea3c7249a68861e8fe799ce2d021599ea26a8584222cf1c970f` | `28845fc7a88131b6c369f47450e290a6bade661b0321a84af347d46d454cdd94` |
| Corrected exact-source native run | `17e58b1b7e76e89bdc58cd4f` | 101 | `77a0319ee59d76f19067c11a508efb808c1bd1100827063bef0b84217a15496b` | `d7366cc04270e3bd4fc1d50fbed8fe2643ead397815d63348cd1d6188f31eb73` |
| Read-only cleanup audit | `7ea0663ef77f63755d502eb8` | 0 | `e6a396ee856c37da3d91c3ccc9341268a232a49dbba680e29195b0f9c4df65eb` | `9df0843288d913a2ef34e09c319a547284442589e57f1b5a103d1d07db5eb5a6` |

The source gate exited zero. The corrected native record uses the identical
source digest, so its later progress is not attributed to a different source
variant.

## Honest corrected-native disposition

The corrected native run retains 101 successful libtest summaries totaling
111 passed, zero failed and zero ignored, then invokes the exact P6-S13
identity once. It proceeds non-vacuously through:

- both initial TCP/HTTP readiness Pass rows;
- route-disjoint remote inbound TCP;
- client off-node TCP, non-empty UDP and empty UDP;
- gateway DNS, mesh-name resolution and issued-VIP TCP;
- the mTLS/no-cleartext gate;
- real `state=unhealthy` and both newer readiness Fail rows;
- independent peer recovery;
- a fresh byte-exact issued-VIP TCP round trip and
  `v6-client: state=healthy`; and
- both newer readiness Pass rows on the unchanged peer allocation.

Control reaches the unchanged population wait at
`vsock_walking_skeleton.rs:181-185`. The client then reports
`population: Connection reset by peer (os error 104)` and never emits
`population-held`, so the body panics at line 185. This proves the HTTP
fixture correction and iteration-3 recovery path execute honestly. It does
not prove the population, exact option/fd/socket observations, no-uplink
absence, final peer cardinalities, public stop or success-path cleanup.

The five control-plane identities do not run after the fail-closed CLI exit;
the receipt is not a 117-pass result. The population failure is a separate
remaining semantic RED outside this bounded HTTP review. This iteration does
not assign its owner, prescribe remediation, weaken `population-held`, or
authorize another fixture or production change.

## Cleanup and confinement

The read-only `7ea0…` audit records zero live Cloud Hypervisor processes, no
nft table and no STEP 14-01 work directory. It lists only the already
documented pre-existing `ovd-ns-0000` namespace. The failed body teardown
kills its remaining peer/client scopes. No success-path cleanup credit is
claimed.

## Finding dispositions

### Bounded HTTP fixture correction

**Disposition: APPROVED.** Full bounded request-head consumption closes the
connection-reset fixture defect without changing health semantics, timeout,
body, production code or acceptance surface.

### Iteration-3 F-01

**Disposition: CLOSED IN SOURCE AND EXECUTED NON-VACUOUSLY.** The corrected
native run reaches independent application recovery, fresh issued-VIP traffic
and the newer public Pass pair before the later population failure.

### New iteration-4 findings

None. The later population semantic RED is recorded without scope expansion.

## Iteration 4 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | HTTP 200/503, initial health, failed window and recovery all retain real request boundaries. |
| GWT / one behavior | 9/10 | The correction is one private framing fix inside the existing P6-S13 journey. |
| Domain language | 10/10 | HTTP request completion, application health and public readiness remain distinct. |
| Coverage completeness | 10/10 for bounded scope | The corrected run executes the complete probe-state transition before the separately retained population RED. |
| Walking-skeleton user value | 10/10 | Real public readiness and restored issued-VIP traffic advance without fabricated results. |
| Priority validation | 10/10 | The fix preserves every accepted outcome and does not disturb the crafter's product candidate. |
| Observable behavior | 10/10 | A complete bounded request precedes the unchanged response, and public rows prove all three states. |
| Traceability | 10/10 | Pre-fix failure, exact source change, source gate and corrected native progress are linked. |
| Walking-skeleton boundary | 10/10 | Production serve/deploy and real GuestStream remain the exercised path. |

## Iteration 4 verdict

**APPROVED.** The private HTTP fixture now consumes one complete bounded
request head before returning the unchanged health-dependent response. No
body, API, scenario, lane, selector, product mechanism, timeout, health
sequence or public `Pass -> newer Fail -> newer Pass` assertion changed.

The exact-source native run proves the correction and F-01 recovery sequence
through the newer public Pass pair, then fails honestly at the separate
population-held gate. No downstream or 117-pass credit is granted, and this
review prescribes no population change.

# APPROVED

# Review iteration 5 — source-only population diagnostics

## Iteration 5 metadata

| Field | Value |
| --- | --- |
| Scope | Diagnostics for the existing P6-S13 population reset only |
| Changed support | Existing guest population client/peer logging and existing CLI trace/timeout output |
| Source-gate run | `d24a016595077e7e3e546489`, exit 0 |
| Source digest | `9a4bbeed97c932c1f2e460bbfbc97fd084e6e2b19da1ee45adf0735bf2beee35` |
| Native execution | None in this review iteration |
| Iteration verdict | **APPROVED — ONE EXISTING-RUNNER NATIVE REPRODUCTION AUTHORIZED** |

The reviewer changed only this review artifact. No production, test-support or
acceptance source, roadmap, DESIGN artifact, DES event, execution log, receipt
or commit was changed. No native, Metal or mutation execution was launched by
the reviewer.

Roadmap validation remains `approved`. Diagnostic source does not change the
accepted design, an outcome, or roadmap authority.

## Exact source identity and bounded scope

| Artifact | SHA-256 / disposition |
| --- | --- |
| Guest workload diagnostics | `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec` |
| CLI private diagnostics | `0b0adae1e6c0085573b5877b0d1c25bb044118182bca2f2359ed37f4c0b983a4` |
| P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` — unchanged |
| Production HTTP-prober candidate | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` — unchanged |
| GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` — unchanged |
| Five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` — unchanged |

No public product API, production mechanism, body, assertion, marker, retry
policy, timeout, workload kind, scenario, lane, selector or runner count
changed. The diagnostics are test-local observation of the already reproduced
population reset.

## Client population diagnostic audit

**Result: PASS.**

`hold_population` still:

- computes one fixed thirty-second deadline before the population loop;
- iterates the same `0..population` indices;
- targets the same public issued VIP and
  `V6_POPULATION_BASE_PORT + index`;
- writes the same `V6-POP:<index>\n` request;
- requires the same exact `V6-POP-ACK:<index>\n` reply;
- pushes only fully acknowledged streams into the same held vector; and
- returns the same error to `v6_client`, which preserves the existing
  `exit::TCP_IO` outcome
  (`guest_peer.rs:702-710,723-757`).

The correction only annotates each failure:

- `CONNECT`, `WRITE` or `READ` carries the exact index, target
  VIP:port, stage, raw errno when present and source error;
- an ACK mismatch carries the same exact index/target, stage `ACK`,
  `errno=none`, expected bytes and received bytes; and
- `population_error` preserves the original `io::ErrorKind` while adding
  the diagnostic text
  (`guest_peer.rs:732-775`).

`retry_until` is unchanged and still retries every error only until the
single precomputed deadline. No stage is accepted as success, no error is
converted to an ACK, and no failed stream enters the held cohort.

## Peer population diagnostic audit

**Result: PASS.**

The peer still binds the same fixed listeners and spawns one handler per
accepted population connection. It now records:

- exact index, declared port, local tuple and peer tuple at `ACCEPT`;
- request read errno or exact request bytes/mismatch at `READ`/`REQUEST`;
- ACK write errno or exact ACK byte count at `ACK`; and
- final EOF, unexpected data or read error with tuple and errno at `CLOSE`
  (`guest_peer.rs:514-603`).

After a valid request and ACK, the handler blocks in a one-byte closure read
while owning the same `TcpStream`. The prior implementation held that stream
forever in the handler after ACK. While the client is live and holds every
acknowledged socket, both shapes retain exactly one peer stream per successful
index. The new read merely makes the eventual close/reset externally visible;
it does not close, duplicate, replace or remove a live acknowledged stream.

The client sends no post-ACK population bytes, so observing closure cannot
alter request/ACK behavior or consume application data required by an
assertion.

## CLI diagnostic audit

**Result: PASS.**

`Lane::boot` now initializes the repository's existing
`tracing-subscriber` formatter once with:

- global INFO;
- control-plane INFO; and
- worker/dataplane DEBUG
  (`vsock_support.rs:96-108,250-270`).

It uses `try_init`, adds no production subscriber/hook, and supplies no
value consumed by P6-S13. Production `serve` remains composed through the
same existing function and adapters. Trace presence or absence cannot satisfy
an acceptance predicate.

`Lane::wait_console` still delegates to the same allocation-specific
poller, bound and predicate. On timeout only, it prefixes the returned
diagnostic with the current allocation id
(`vsock_support.rs:444-453,1411-1428`). No row selection, completion marker,
timeout or assertion changed.

These additions expose the current in-process production owner sequence and
the exact guest endpoints without a new API, seam, expected-value injection or
test outcome.

## Source-gate evidence

The decisive record names commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242`, source digest
`9a4bbeed97c932c1f2e460bbfbc97fd084e6e2b19da1ee45adf0735bf2beee35`,
privileged execution and terminal `command_returned` exit 0.

| Run | Output SHA-256 | Terminal SHA-256 | Disposition |
| --- | --- | --- | --- |
| `d24a016595077e7e3e546489` | `6903f11fff588e4299ef898bf04ff3d7c5addbf779be2a126e14624102ccc845` | `f847d917d33979857d8fd67e1a4eb37e5e0314094be6899e52a2db562c5d37ec` | Scoped rustfmt/check/Clippy source gate passed |

The gate compiles and lints the exact diagnostic source. It is not runtime
evidence for the population failure and receives no behavioral credit.

## One native reproduction decision

**Disposition: AUTHORIZED.**

One run of the unchanged existing cgroup-default GuestKit runner is warranted
to classify the already-reproduced population reset with the approved
diagnostics. The run must:

- use this exact synchronized source;
- select the existing one CLI and five control-plane identities through the
  unchanged runner;
- preserve every fail-closed assertion and the fixed 117 inventory;
- collect the client stage/index/target/errno, peer
  ACCEPT/REQUEST/ACK/CLOSE context, in-process production traces and current
  allocation timeout text;
- grant credit only through the last assertion actually executed; and
- retain the normal read-only cleanup audit.

This authorization is for observation, not a product or fixture correction.
The run must not be repeated merely to seek a favorable schedule. Its result
must be returned for owner classification before any remediation is proposed.

## Finding dispositions

### Diagnostic source

**Disposition: APPROVED.** The diagnostics expose existing failures and
closure without changing success, retry, held-stream cardinality or any
acceptance outcome.

### New iteration-5 findings

None.

## Iteration 5 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Every success condition remains exact; diagnostics cover all failure stages and closure. |
| GWT / one behavior | 9/10 | The one existing P6-S13 journey is unchanged. |
| Domain language | 10/10 | Population index, issued-VIP target, stage, errno and peer tuple are precise. |
| Coverage completeness | 10/10 for diagnostics | CONNECT/WRITE/READ/ACK and ACCEPT/REQUEST/ACK/CLOSE are fully distinguished. |
| Walking-skeleton user value | 10/10 | The next run can classify the real production boundary without changing it. |
| Priority validation | 10/10 | Observation precedes remedy and preserves the approved roadmap. |
| Observable behavior | 10/10 | Client, peer, production trace and allocation timeout evidence are independent surfaces. |
| Traceability | 10/10 | Every added diagnostic maps to the reproduced population failure path. |
| Walking-skeleton boundary | 10/10 | Existing serve/deploy/GuestKit composition remains the only execution path. |

## Iteration 5 verdict

**APPROVED.** The source changes are diagnostic only. They preserve the exact
population deadline, targets, request/ACK protocol, retry/failure outcome,
held-stream count, P6-S13 body, production HTTP candidate, six identities and
117 inventory.

One exact-source execution of the existing runner is authorized to classify
the current population reset. No source, outcome, API, mechanism, roadmap or
test inventory change is authorized by this verdict.

# APPROVED

# Review iteration 6 — population evidence and final timeout rendering

## Iteration 6 metadata

| Field | Value |
| --- | --- |
| Scope | Evidence from the one iteration-5-authorized reproduction plus source-only timeout rendering |
| Authorized native run | `b2d55cadd3db0611ef5a7d59`, exit 101 |
| Final source gate | `49909e2115de1404ed681a1b`, exit 0 |
| Cleanup audit | `229b05df3d17597c7c5d1bfd`, exit 0 |
| Additional native runs | **None** |
| Iteration verdict | **APPROVED — DIAGNOSTICS COMPLETE IN SOURCE; ROOT CAUSE UNPROVEN** |

The reviewer changed only this review artifact. No production, test-support or
acceptance source, roadmap, DESIGN artifact, DES event, execution log, receipt
or commit was changed. No native, Metal or mutation execution was launched by
the reviewer.

Roadmap validation remains `approved`. The diagnostic result and final
timeout rendering change neither the approved design nor roadmap authority.

## Current source identity and bounded final correction

| Artifact | SHA-256 / disposition |
| --- | --- |
| Guest population diagnostics | `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec` — unchanged from iteration 5 |
| Final CLI timeout diagnostics | `ad5d80cd7e535afa49f5d6f6916a8d51773563630ffa7af852531f5d8ad6fca9` |
| P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` — unchanged |
| Production HTTP-prober candidate | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` — unchanged |
| GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` — unchanged |
| Five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` — unchanged |

The post-run source change is limited to timeout rendering. The same
allocation-specific console observer, bound and predicate still control
`wait_console`. On error, the returned diagnostic now appends every console
already retained by the existing `ConsoleCapture`, with an allocation
heading for each (`vsock_support.rs:91-108,458-470`). It does not change
which allocation is polled, whether the predicate succeeds, the timeout,
console persistence, cleanup order or any assertion.

This rendering closes the observed evidence gap: a future separately
authorized timeout would include both client and peer private stages. It is
not retroactive evidence for the already completed run.

## One authorized native receipt

The authorized receipt identifies commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242`, synchronized source digest
`6a20d5054ce1907c7c9fe768c0fe4ad79824cb57b67cfcdeff11adc2cfd3c0f1`
and terminal `command_returned` exit 101.

It retains 101 successful libtest summaries totaling 111 passed, zero failed
and zero ignored, then invokes the exact P6-S13 CLI identity once. The five
control-plane identities do not run after the fail-closed CLI exit, so this is
not a 117-pass receipt.

The timeout names the actual requested allocation:

`allocation=alloc-v6-client-0`.

The client diagnostic is exact:

`population index=37 target=10.96.0.2:18137 stage=READ errno=104 error=Connection reset by peer (os error 104)`.

The unchanged population loop can reach index 37 only after each earlier
index returned the exact expected ACK and its stream was pushed into the held
vector. The receipt therefore establishes:

- indices 0 through 36 completed the exact request/ACK exchange;
- those 37 client streams were retained when index 37 was attempted;
- index 37 connected and wrote its request far enough to enter the exact ACK
  read;
- the failure is a kernel `ECONNRESET` during `READ`; and
- the failure is not CONNECT refusal, WRITE failure or ACK-content mismatch.

No later index and no `population-held` marker receives credit.

## Production trace disposition

The test-local tracing subscriber captured the in-process production path for
the current client allocation. Around the population failure the trace shows:

- guest-originated `tcp_connect` flows for
  `alloc-v6-client-0`;
- several `peer_eof` terminations;
- a subsequent burst of `peer_reset` terminations;
- one `control_session_lost`; and
- continuing peer-allocation TCP/HTTP probe dials in the same interval.

These observations prove that the failure traversed the real production owner
and identify the affected client allocation. They do **not** establish which
endpoint initiated the first reset, why the reset begins at index 37, whether
one event causes another, or which production component—if any—violated an
accepted invariant. Temporal adjacency is not causation.

No seeded Sim invariant or second native reproduction isolates an ordering,
capacity or lifecycle defect. Under the workspace reachability gate, this
review therefore records the population reset as a reproduced semantic RED
with an unproven root cause. It does not infer retention architecture,
capacity changes, retry policy, owner changes or a remediation.

## Missing peer-console evidence

The authorized receipt was produced before the all-console timeout rendering.
It contains the requested client allocation console and the in-process
production trace but does not render the peer VM's private population console.
Accordingly, it proves no peer-side per-index
`ACCEPT`, `REQUEST`, `ACK` or `CLOSE` event.

The absence is material to root-cause classification but does not make the
existing client/production evidence false. The final source now has the
required observation for any future owner-authorized failure. This review
does not manufacture the missing peer rows, request another run, or treat
source capability as executed evidence.

## No second native reproduction

The final source digest is
`84b1033ab7c90d64c28953b783d306da1e4da864e755cfaf1c3e148239ec632f`.
The retained raw tree contains exactly two records at that digest:

- source gate `49909e2115de1404ed681a1b`, exit 0; and
- read-only cleanup audit `229b05df3d17597c7c5d1bfd`, exit 0.

There is no native product-run receipt at the final digest. The one authorized
reproduction remains `b2d55…`; the changed synchronized digest reflects the
subsequent timeout-rendering source and documentation state. No schedule was
sampled twice to seek a different result.

## Final source-gate and cleanup evidence

| Purpose | Run | Source digest | Exit | Output SHA-256 | Terminal SHA-256 |
| --- | --- | --- | ---: | --- | --- |
| Authorized diagnostic native run | `b2d55cadd3db0611ef5a7d59` | `6a20d5054ce1907c7c9fe768c0fe4ad79824cb57b67cfcdeff11adc2cfd3c0f1` | 101 | `a641c0a1f5ce8e75d3ce77d283d2b6a91f91f251a020c5dfdd94704cf59ff0d1` | `dfc5c01849cd15de0aa84733369fb3bd07b21916dc5c543cbdfb8903941dba45` |
| Final timeout-render source gate | `49909e2115de1404ed681a1b` | `84b1033ab7c90d64c28953b783d306da1e4da864e755cfaf1c3e148239ec632f` | 0 | `3e54b542bd962e49621c2da36592bd3e4345d14172dea5d308096e92353b27b8` | `29c5445752e84e3cbb417d97a92c6c9870315d6f8626591acb62174e59326e99` |
| Read-only cleanup audit | `229b05df3d17597c7c5d1bfd` | `84b1033ab7c90d64c28953b783d306da1e4da864e755cfaf1c3e148239ec632f` | 0 | `221fb54f718e1a345106c3a30ac8f2edad0b14e4be2598899db50b16f456fc52` | `e2f62d6ab1935cc6ec1843b5af562f40af404f21f2779e8fb4fd338f460372d6` |

The final source gate passed. The cleanup audit reports zero live Cloud
Hypervisor processes, no nft table and no STEP 14-01 work directory; it lists
only the documented pre-existing `ovd-ns-0000` namespace. No success-path
cleanup credit is claimed from the failed native body.

## Evidence-credit boundary

The authorized native run preserves the iteration-4 credit through initial
Pass, newer Fail, independent recovery/fresh VIP traffic and newer Pass. It
adds non-vacuous credit for exactly acknowledged-and-held population indices
0 through 36 and the classified index-37 read reset.

It does not prove:

- peer-side population stages;
- `population-held`;
- the exact external option/fd/source-port assertions;
- leg-C/no-uplink observations after population;
- final remote-peer cardinalities;
- public Service stop or VMM disappearance for this body;
- success-path socket/range/topology cleanup; or
- the five control-plane bodies / complete 117 result.

The existing assertions remain fail-closed. No unexecuted suffix is promoted
to evidence.

## Finding dispositions

### Population diagnostic evidence

**Disposition: APPROVED WITH HONEST LIMITATION.** Client and in-process
production evidence precisely locate the observed reset. Peer private stages
are absent from the receipt and the root cause remains unproven.

### Final all-console timeout rendering

**Disposition: APPROVED IN SOURCE.** It adds only retained-console context to
the existing timeout error and passes the final source gate.

### New iteration-6 findings

None. The population semantic RED remains for a separately authorized owner
classification; this review neither assigns cause nor remedy.

## Iteration 6 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Exact ACK progress and failure stage are distinguished without weakening success. |
| GWT / one behavior | 9/10 | The existing P6-S13 journey and inventory are unchanged. |
| Domain language | 10/10 | Allocation, index, VIP target, stage, errno and owner termination causes remain precise. |
| Coverage completeness | 10/10 for diagnostics / incomplete runtime evidence | Final source can render all consoles; the one receipt honestly lacks peer stages. |
| Walking-skeleton user value | 10/10 | Evidence locates the real failure without inventing a remedy or bypass. |
| Priority validation | 10/10 | One authorized run was used, and observation remains prior to remediation. |
| Observable behavior | 10/10 in source / 8/10 receipt | Client and production surfaces are exact; peer private execution is absent. |
| Traceability | 10/10 | Source hashes, two source digests, receipt boundaries and evidence credit are explicit. |
| Walking-skeleton boundary | 10/10 | Production serve/deploy and the unchanged GuestKit runner remain the execution path. |

## Iteration 6 verdict

**APPROVED.** The population diagnostics are complete in source and the one
authorized receipt is accepted with its peer-console limitation. The receipt
proves exact held progress through index 36 and an index-37
`READ/ECONNRESET` on the current client allocation, plus real owner
termination observations. It does not prove the reset's initiator or root
cause.

No second reproduction, production change, fixture weakening, API/mechanism
change, roadmap/DES edit or remediation is authorized by this verdict. The
population failure remains an honest semantic RED for a separately authorized
owner-classification step.

# APPROVED

# Review iteration 7 — leg-B comparison scope after D-socket exhaustion

## Iteration 7 metadata

| Field | Value |
| --- | --- |
| Scope | Population failure ownership and V-LEGB-SERVE comparison scope |
| Decisive native evidence | `72ae8b505399ad534921bb7f`, exit 101 |
| Rejected production experiment | `44af1e6c63566bcd47aeb915`; D socket with no-port produced duplicate `127.0.0.1:0` registration and was reverted |
| Reported cleanup | `b4c8fe95eb54c7db6f1a07ae0`; not present in the retained raw tree, so no independent receipt credit |
| Assessment | **Private global-range population HOW is over-constrained; canonical DESIGN is not contradicted** |
| Iteration verdict | **NEEDS PRIVATE DISTILL REMEDIATION** |

The reviewer changed only this review artifact. No production, test-support or
acceptance source, DESIGN artifact, roadmap, DES event, execution log, receipt
or commit was changed. No Metal, native or mutation execution was launched by
the reviewer.

Roadmap validation remains `approved`. The reproduced failure falsifies a
private DISTILL population construction, not the accepted leg-B mechanism or
roadmap contract.

## Normative contract scope

The accepted contract is precise about the socket that owns
`IP_BIND_ADDRESS_NO_PORT`:

> “Before `connect()`, when the peer address lies inside the guest prefix,
> leg-B sets the socket mark to `MTLS_TPROXY_MARK`, allows a bind without a
> port (`IP_BIND_ADDRESS_NO_PORT`), and binds `127.0.0.1:0`.”

That is ADR-0190's `HostMtlsEnforcement` leg-B dial. Current production
implements it in `dial_leg`: create the leg-B fd, set `SO_MARK`, set
`IP_BIND_ADDRESS_NO_PORT`, bind loopback without a concrete port, then
connect to the VM backend
(`overdrive-dataplane/src/mtls/mod.rs:724-800`).

Feature-delta V-LEGB-SERVE states:

> “Also in scope: concurrent same-node leg-B dials with
> `IP_BIND_ADDRESS_NO_PORT` set and unset, recording whether the unset run
> exhausts the ephemeral range on `127.0.0.1` at its size and the set run
> does not.”

It requires a production leg-B option/comparison result. It does not require
every loopback socket in the full `serve` composition to reuse a port, and it
does not assign `IP_BIND_ADDRESS_NO_PORT` to the guest-flow owner's intake
socket D.

D9 is independently normative: a guest mesh flow is registered by its
concrete forwarder-local tuple before D connects to leg F. The owner must know
that tuple before registration; D is not the leg-B socket whose reuse
ADR-0190 specifies.

The earlier DISTILL proposal's 64-port global window and
`N = C + 1` production population were private HOW
(`distill-14-01-v6-coverage.md:363-402`), not a canonical outcome. Native
evidence now falsifies its assumption that the whole production composition
can fit `C+1` flows merely because leg-B defers source-port assignment.
Under the native-falsification rule, that private derivation and its prior
review approval are invalid; ADR-0190 and V-LEGB-SERVE remain intact.

## Complete production owner/socket path

The actual same-node guest TCP path has two different host sockets:

1. The guest client asks the host guest-flow owner for a TCP connection to the
   Service VIP.
2. Resolution returns `MtlsResolution::Mesh(backend)`.
3. The owner obtains leg F's loopback intake address and creates socket **D**
   with `PeerSocketSpec.bind = Some(127.0.0.1:0)`
   (`owner_runtime.rs:1032-1089`).
4. `HostGuestFlowIo::open_peer` creates D and performs an ordinary concrete
   `bind()` before returning it
   (`flow_io/host.rs:642-672`).
5. The owner reads D's concrete local tuple and registers
   `(D.local, original destination, backend)` with the mTLS worker before D
   connects to leg F
   (`owner_runtime.rs:1090-1118`). This is D9.
6. Leg F claims that registration. `HostMtlsEnforcement` then creates the
   separate **leg-B** socket toward the VM backend using ADR-0190's mark,
   no-port option and loopback bind.

D carries the guest-to-leg-F half and remains live with the flow. It reserves
one concrete ephemeral port. Leg-B is the later socket whose source port may
be shared across distinct backend four-tuples.

## Decisive native falsification

**Result: private test over-constraint reproduced.**

The `72ae…` trace reports, for `alloc-v6-client-0`:

`flow=46 destination=127.0.0.1:60941 stage="open_peer" ... code: 98 ... "Address already in use"`.

The destination is leg F's loopback intake, not the VM backend. The failure is
inside `open_peer` while binding D, before `local_addr`,
`register_forwarded_outbound`, `connect_peer` and the worker's leg-B
`dial_leg` can run. Subsequent retries fail at the same stage.

The guest observes the consequence at population index 38:

`target=10.96.0.2:18138 stage=READ errno=104`.

That reset is downstream of the owner refusing the flow because D could not
bind. It is not evidence that the production leg-B no-port socket exhausted.

The current fixture globally narrows the outer guest's ephemeral range, then
sets the production population to `N=C+1`, where `C` is the measured free
concrete-port capacity. The production path necessarily needs one concrete D
port per flow before leg-B exists. Therefore `N=C+1` makes D exhaustion
inevitable even if every leg-B socket shares one source port. The fixture is
testing global composition capacity, not isolating ADR-0190's leg-B option.

## Rejected D-socket remedy

Adding `IP_BIND_ADDRESS_NO_PORT` to D is not an authorized or compatible
repair. The `44af…` experiment left D.local as `127.0.0.1:0` at the
pre-connect registration point and reached
`DuplicateForwardedRegistration { forwarder_local: 127.0.0.1:0 }`.

That result follows D9's required ordering: registration must exist before D
connects to leg F, while a no-port bind receives its concrete port only at
connect. Reordering registration, changing its key, or teaching D to share
port zero would redesign the forwarded-registration protocol. No such
production mechanism is authorized, and reverting the experiment was correct.

The test must accommodate D's accepted concrete-port ownership; production
must not be changed to satisfy a private global-range assumption.

## Finding F-02 — High: the unset/set comparison ignores the production D-socket cost

**Finding owner: DISTILL test-support author.**

The current control models only one option-unset source socket per logical
dial, while the production cohort holds both concrete D and the measured
leg-B socket. Returning `N=C+1` from that control guarantees the production
cohort cannot create all D sockets. The later option/fd/backend assertions are
unreachable for a reason outside their intended leg-B subject.

This is a test-HOW defect. It does not create an API gap or an
outcome-changing DESIGN choice.

### Exact invariant-preserving correction

Keep the existing P6-S13 body, two Services, public serve/deploy composition,
64-port bounded window, exact backend set, byte acknowledgements, external fd
option checks, source-sharing assertion, post-close absence and all other
oracles. Correct only the private population comparison:

1. After measuring `C` currently free concrete loopback ports, choose a
   bounded logical population `N` such that
   `2 <= N <= 65`, `N < C`, and `2N > C`; the smallest canonical choice
   is `N = floor(C / 2) + 1` when the preconditions hold.
2. Shape the **unset control** like the real two-socket path. First hold `N`
   option-unset loopback-bound reservation sockets representing D's concrete
   port cost. Then attempt `N` option-unset connections to the existing
   distinct fixed listeners. Require exactly the remaining `C-N` source
   ports to succeed and the next connection to fail with the accepted
   ephemeral-exhaustion errno; no failure row may remain.
3. Close/reset both control socket classes, externally prove their absence and
   re-read the same free capacity before production starts.
4. Run the production cohort with that same `N`. Real D now consumes its
   accepted `N` concrete ports. Require all `N` guest request/ACK exchanges
   to remain held.
5. Observe only the exact production **leg-B** rows: their peers must equal the
   exact registered backend set; every correlated live fd must report
   `IP_BIND_ADDRESS_NO_PORT == 1`; their source ports must lie in the bounded
   window; and their distinct source count `S` must satisfy
   `S < N` and `N + S <= C`. Together with `2N > C`, this proves the set
   leg-B cohort succeeded by sharing where the same D cost plus unset leg-B
   sockets cannot.
6. Preserve the existing production-population absence and original-range
   restore/readback gates.

This correction grants no false capacity credit: success requires every real
production flow, exact ACK, exact leg-B fd/option/backend tuple and capacity
inequality. It reduces only the private population chosen from the same
measured range; V-LEGB-SERVE pins no value of 65. It is not the #326 density
benchmark and adds no test, scenario, lane, product seam or API.

The correction can be implemented with the existing
`EphemeralRangeGuard`, unset-control listeners, raw socket helpers,
`socket_rows`, `inode_fds` and option readback. No production registration
or socket-order change is needed.

## Diagnostic source disposition

The private `owner_runtime` stage trace may remain for bounded diagnosis. It
projects existing stages/errors only and does not change D9 ordering, refusal
mapping, socket creation or owner behavior. The original HTTP-prober candidate
and every previously reviewed acceptance body remain outside this finding.

The reported cleanup run `b4c8fe95eb54c7db6f1a07ae0` is not present under
the retained raw tree, so this reviewer grants it no independent receipt
credit. That evidence availability issue does not affect the socket-owner
classification, which is established by the durable `72ae…` receipt and
source path.

## Native-falsification disposition

| Item | Disposition |
| --- | --- |
| ADR-0190 leg-B mark/no-port/bind contract | **REMAINS VALID** |
| D9 concrete D.local registration ordering | **REMAINS VALID** |
| V-LEGB-SERVE option set/unset validation outcome | **REMAINS REQUIRED** |
| Private 64-port `N=C+1` production construction | **FALSIFIED** |
| D no-port / `127.0.0.1:0` registration experiment | **REJECTED AND REVERTED** |
| Production registration redesign | **NOT AUTHORIZED** |
| Roadmap validation | **REMAINS APPROVED** |

No DESIGN contradiction exists unless the contract is broadened to require
the entire composition to admit more live flows than its global ephemeral
range. The accepted language is explicitly about concurrent leg-B dials and
their option; it does not impose that broader outcome.

## Iteration 7 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 after remediation | Both unset exhaustion and set sharing remain exact, with D cost represented. |
| GWT / one behavior | 9/10 | The existing cumulative P6-S13 body remains the sole scenario. |
| Domain language | 10/10 | D, leg F, leg B, concrete registration and ephemeral capacity are distinguished. |
| Coverage completeness | 7/10 current / 10/10 prescribed | The current set oracle is unreachable behind D exhaustion; the correction restores its intended universe. |
| Walking-skeleton user value | 10/10 | The corrected comparison still runs through production serve/deploy and real owners. |
| Priority validation | 10/10 | Native evidence narrows the fixture instead of inventing registration architecture. |
| Observable behavior | 7/10 current / 10/10 prescribed | Current failure observes D; corrected assertions observe exact leg-B option/source sharing and control exhaustion. |
| Traceability | 10/10 | Normative quotes, owner call order and native stage/errno evidence align. |
| Walking-skeleton boundary | 10/10 | No direct product state or alternate mechanism replaces the production path. |

## Iteration 7 verdict

**NEEDS PRIVATE DISTILL REMEDIATION.** The current population RED is caused by
the fixture's globally narrowed range exhausting D's required concrete binds
before the intended leg-B comparison. It does not falsify ADR-0190,
V-LEGB-SERVE or D9, and it does not authorize `IP_BIND_ADDRESS_NO_PORT` on D,
registration reordering or a new production mechanism.

Return F-02 to the DISTILL test-support author for the exact capacity-accounted
control and production population derivation above. No production, DESIGN,
roadmap or API change is required or authorized.

# NEEDS REVISION

# Review iteration 8 — capacity-accounted leg-B comparison

## Iteration 8 metadata

| Field | Value |
| --- | --- |
| Scope | Exact F-02 private DISTILL fixture remediation |
| Source gate | `cb3d36129bb6cf7f62c6683c`, exit 0 |
| Exact native runner | `ea80ce9b99a8cfb3d906ee37`, exit 101 after the P6-S13 pass |
| Read-only cleanup | `11fd70ae687507e276cf8713`, exit 0 |
| Synchronized source digest | `b2d214a3f653232497f811210c4dcc58965bff66f4bb6d656747ba49c5676338` |
| Finding disposition | **F-02 CLOSED** |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production source,
test-support source, acceptance body, DESIGN or DISTILL handoff, roadmap, DES
event, execution log, receipt, host state or commit was changed. The reviewer
launched no native, Lima or mutation execution.

Roadmap validation remains `approved`. This iteration closes the private
fixture defect identified in iteration 7. It does not change ADR-0190, D9,
V-LEGB-SERVE, the approved UDP deferral or any production contract.

## Exact reviewed source identities

| Artifact | SHA-256 | Disposition |
| --- | --- | --- |
| CLI private native support | `59a3211b55f920fc25bb1cbb7fc2d6129672cf8e7fe233030e216e665701b63b` | F-02 correction only |
| Existing P6-S13 body | `f64f7beb8a25bbe34aec9b73d4b292926151514a4bf3224cdaaceffb8e95971d` | Same identity and scenario; private plan is threaded to existing helpers |
| Guest workload and diagnostics | `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec` | Unchanged from iterations 5–7 |
| Production HTTP prober | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` | Unchanged original crafter candidate |
| Existing GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` | Unchanged |
| Existing five control-plane bodies | `04029e0d53d6d665a5073dba1008759e9de32c2eeecb799892cc52109e7b9b18` | Unchanged |

The retained owner-runtime stage tracing remains private diagnostic output. It
does not change D creation, forwarded registration, D9 ordering, refusal
mapping or any public interface. F-02 introduces no production source, API,
selector, lane, Service, workload, scenario, registration mechanism or #326
density promise.

## F-02 correction audit

### Measured plan and unset control

`EphemeralRangeGuard::run_unset_control` measures the currently free concrete
loopback-port capacity `C` inside the existing 64-port disposable range. The
private `population_for_capacity` derives

```text
N = floor(C / 2) + 1
2 <= N <= 65
N < C
2N > C
```

and asserts every bound before returning `PopulationPlan { count: N,
capacity: C }` (`vsock_support.rs:685-702`). This is the exact bounded plan
prescribed in iteration 7.

The unset control then holds exactly `N` raw IPv4 stream sockets. Each socket
reads `IP_BIND_ADDRESS_NO_PORT == 0`, binds `127.0.0.1:0`, receives a distinct
concrete in-window port, and remains live as the control representation of
production D (`vsock_support.rs:703-712`, `865-887`). With those reservations
held, the helper attempts the existing distinct option-unset connections. It
requires exactly `C-N` successful connections with distinct in-window source
ports and requires the next attempt to fail with `EADDRINUSE` or
`EADDRNOTAVAIL` (`vsock_support.rs:714-752`). The exact failed destination has
no live TCP row, so exhaustion cannot receive credit from a residual failed
connection (`vsock_support.rs:753-761`).

The successful client and accepted sockets are reset-closed, all control
destinations must lose every live row, every D-reservation port must disappear,
and `free_capacity()` must return the same `C` before production starts
(`vsock_support.rs:762-788`). The control therefore accounts for both socket
classes without leaking capacity into the production measurement.

### Production set cohort

The existing P6-S13 body passes the same `PopulationPlan.count` through the
existing public Service deployment and guest workload. It still waits for the
exact `population-held=N` marker before inspecting the host socket table, and
it passes the complete plan to the existing external option oracle
(`vsock_walking_skeleton.rs:135-186`). No product state, override seam or
alternate dial path supplies the result.

`assert_population_option_set` selects rows only when their peer tuple belongs
to the exact expected set
`{(peer_addr, V6_POPULATION_BASE_PORT+i) | 0 <= i < N}`. It requires exactly
`N` rows, every expected destination, one live fd for each inode,
`IP_BIND_ADDRESS_NO_PORT == 1` on every fd, and every source inside the same
bounded range (`vsock_support.rs:1045-1077`). For the distinct source count
`S`, it fail-closes on all three capacity facts:

```text
S < N
N + S <= C
2N > C
```

(`vsock_support.rs:1078-1086`). Thus all `N` real production flows and their
exact request/ACK holds must exist while the production leg-B sockets share
source ports; the same `N` concrete D sockets plus option-unset leg-B sockets
cannot fit. The assertion grants no capacity credit merely because fewer
flows were attempted.

### Preserved P6-S13 outcomes and boundaries

The existing body retains its same public `serve` plus `deploy` composition,
separate peer and client Services, and all earlier oracles: Job traffic,
non-empty and empty off-node UDP, DNS and issued name/VIP, same-CA remote mTLS,
initial TCP/HTTP probe Pass, newer Fail, independent recovery, fresh VIP
round-trip, newer Pass, exact leg-C identity, positive-control plus loss-
accounted no-uplink capture, remote-peer cardinalities, public stop, terminal
allocations, VMM disappearance and serve shutdown. After shutdown it still
requires the exact production backend set to have no live row and restores and
reads back the original ephemeral range (`vsock_walking_skeleton.rs:187-217`).

Only the private population plan and its option-unset comparison changed. The
same six STEP 14-01 identities and the existing 117-runner accounting remain;
no body, test identity, scenario, lane, public API, production selector or
production socket/registration mechanism was added or weakened.

## Source-gate evidence

The scoped source gate `cb3d36129bb6cf7f62c6683c` used source digest
`b2d214a3f653232497f811210c4dcc58965bff66f4bb6d656747ba49c5676338`
and exited 0. Its retained output records the scoped checks completing without
a compiler, formatting or lint failure. The native and cleanup receipts carry
the identical source digest, so their result is attributable to the reviewed
source identities.

## Exact native evidence

The one exact runner `ea80ce9b99a8cfb3d906ee37` exited 101. Before that
later runner failure, the existing P6-S13 identity passed in 47.43 seconds.
A passing body is non-vacuous evidence that every sequential assertion above
executed, including exact unset exhaustion and cleanup, all `N` byte-exact
production ACK/holds, exact backend/fd/option/window observation, all three
capacity inequalities, the retained probe and traffic outcomes, public stop,
both VMM disappearances, production-row absence and range restoration.

The receipt contains 102 successful libtest summaries totaling 112 passed,
zero failed and zero ignored. Those are the retained 111 pre-extension bodies
plus P6-S13. It contains no recognized runtime-SKIP result.

After P6-S13 passed, the runner entered the first unchanged control-plane
identity,
`the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`. Its
separate disposable guest did not reply to the existing EXEC within the
600-second bound, and the runner failed before any of the five control-plane
bodies received pass credit. No retry was launched. This receipt therefore is
not a full 117-pass result. The later independent outer EXEC timeout neither
invalidates the already completed P6-S13 body nor proves a control-plane
product defect, and this bounded review assigns no remediation for it.

## Cleanup evidence

Read-only cleanup receipt `11fd70ae687507e276cf8713` used the same source
digest and exited 0. It records zero live Cloud Hypervisor processes, no nft
table and no STEP 14-01 work directory. The only listed namespace is the
documented pre-existing `ovd-ns-0000`.

## Finding disposition

| Finding | Disposition | Evidence |
| --- | --- | --- |
| F-02 — unset/set comparison ignored D's concrete-port cost | **CLOSED** | Exact two-socket unset control, same-`N` production cohort, exact capacity inequalities and passing P6-S13 native body |
| Complete 117-runner proof | **PENDING EVIDENCE, NOT A NEW FINDING** | 112 passes followed by an independent outer EXEC timeout before the five unchanged control-plane bodies |

No source-only or product remediation is required for F-02. No retry or
control-plane timeout change is prescribed or authorized by this review.

## Iteration 8 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The exact unset exhaustion and production set success are both mandatory and capacity-accounted. |
| GWT / one behavior | 9/10 | The existing cumulative P6-S13 scenario and identity are retained. |
| Domain language | 10/10 | D, leg F, leg B, concrete capacity, source sharing and backend tuples remain distinct. |
| Coverage completeness | 10/10 for F-02 | Every prescribed control, production and cleanup oracle executed in the passing P6-S13 body. |
| Walking-skeleton user value | 10/10 | Production `serve`, public `deploy`, real VMs and real host socket state remain the observation path. |
| Priority validation | 10/10 | The correction fixes only the falsified private population HOW and preserves canonical outcomes. |
| Observable behavior | 10/10 | Exhaustion, absence, ACK holds, fd option state, exact tuples, capacity inequalities and cleanup are externally observed. |
| Traceability | 10/10 | Each iteration-7 prescription maps to current source and exact-source native evidence. |
| Walking-skeleton boundary | 10/10 | No direct state injection, product hook, alternate mechanism or oracle weakening was introduced. |

## Iteration 8 verdict

**APPROVED.** F-02 is closed. The capacity-accounted fixture represents D's
accepted concrete-port cost, proves the same-cost option-unset cohort exhausts,
and proves all `N` production flows complete and remain held through exact
ADR-0190 leg-B sockets with source sharing. P6-S13 passes non-vacuously under
the exact reviewed source.

The retained native receipt establishes 112 passes, not the complete 117. The
later first-control-plane outer EXEC timeout remains a separate evidence
limit; it does not reopen F-02 and receives no speculative fix in this review.
Roadmap validation remains approved.

# APPROVED

# Review iteration 9 — control-plane wait-flowing diagnostics

## Iteration 9 metadata

| Field | Value |
| --- | --- |
| Scope | Private diagnostics for an existing control-plane `wait_flowing` failure |
| Diagnostic support source | `8600a93f36600c88d2b90d827fa58598ae229d528e69e24550179804336f1380` |
| Staged owner diagnostic source | `6e79e166df9eda1f35b6be01a5a48b704868185d4c1f051f26adfa3a5d2cf021` |
| Source gate | `f244eae46ec860c1c71fad85`, exit 0 |
| One exact native reproduction | `4dbc4f792d41a57f393b4308`, exit 101 at a later outer EXEC timeout |
| Read-only cleanup | `745c909c3d357acdfe60e273`, exit 0 |
| Synchronized source digest | `cd7d76112dc454fbf4b73754d41326a4580c607be3d0571ce66ace59a63211a9` |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production source,
test-support source, acceptance body, DESIGN or DISTILL handoff, roadmap, DES
event, execution log, receipt, native state or commit was changed. No native,
Lima or mutation execution was launched by the reviewer.

Roadmap validation remains `approved`. These diagnostics observe an existing
fixture and existing private owner events. They do not change any accepted
STEP 14-01 outcome or reopen the approved UDP-deferral design.

## Exact diagnostic-source audit

### Live console capture and cleanup-safe retention

The test-local `ConsoleCapture` starts as part of the existing `Node::boot`
fixture before any workload is submitted. It scans the existing production
VM run root while the `Node` remains live, reads each allocation's existing
`console.log`, and retains the latest non-empty byte sequence under the exact
allocation identifier (`guest_flow_through_serve.rs:96-178`, `195-235`). It
does not create a workload, change a VM path or keep a production resource
alive. Its only output is test-process memory used for failure rendering.

`console_of` still reads the live `VmRunDir` console first. It consults the
retained bytes only when the production path is absent or empty
(`guest_flow_through_serve.rs:451-466`). Successful predicates therefore
continue to observe the live production console while it exists. Accepted
cleanup may remove `VmRunDir`; after that removal the same helper can render
the bytes already observed while the Service and client allocations were
live.

The capture thread stops and joins when the existing `Node` fixture drops.
The retained map is cleared at the next `Node` start, so allocation output
from one exact body cannot satisfy or diagnose a later body. The capture is
never queried by an owner assertion, deployment selector or product path.

### Unchanged `wait_flowing` contract

`wait_flowing` retains the exact `FLOWING_LINE` success predicate, existing
`BOOT_BOUND` of 120 seconds and existing 200 ms poll interval
(`guest_flow_through_serve.rs:87-90`, `468-482`). Only its fail-closed panic
text is expanded: it renders the requested client allocation through
`console_of` and then all retained allocation consoles. The latter is ordered
by allocation identifier and labels every section, so a future reproduction
can distinguish the Service VM from the client VM without altering the
predicate or choosing a different allocation.

No assertion, stimulus, deadline, test body, identity, allocation selector,
lane or outcome changed. The diagnostic output is not consumed on a success
path and cannot turn a missing `hold: flowing` marker into a pass.

### Existing in-process trace output

`init_owner_trace_output` installs the repository's existing tracing
subscriber before `run_server`. Its filter retains INFO globally and enables
control-plane, worker and dataplane DEBUG output
(`guest_flow_through_serve.rs:115-128`, `195-221`). This exposes the already
staged owner-runtime stage events in the existing test output. It adds no
production hook, API, port, seam or outcome dependency.

The staged owner source preserves the same `IntakeUnavailable` branches and
ordering while adding exact stage context around `leg_f_addr`, `open_peer`,
`local_addr`, `register_forwarded_outbound`, `connect_peer` and attach refusal.
It does not change socket construction, D9 forwarded registration, connect or
abort/close behavior, error taxonomy, caller result or public interface.

## Identity, inventory and boundary audit

The unchanged GuestKit runner remains
`45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9`.
Its manifest still names the same five control-plane identities, each with an
expected count of one, and retains the one CLI P6-S13 identity. The accounting
remains exactly 111 retained bodies plus one CLI body plus five control-plane
bodies: 117 only after the entire fail-closed manifest reconciles. The
diagnostic source adds no test, scenario, lane, selector or runner count.

All five body declarations retain `CONTRACT_SHAPE: bounded-change.` and their
existing stimuli and assertions. The diff is confined to imports, private
console/trace helpers, the private `Node` fixture field/startup, live-first
`console_of` fallback and `wait_flowing` failure rendering. No production
composition or test oracle is bypassed.

## Source-gate evidence

Scoped Metal source gate `f244eae46ec860c1c71fad85` used source digest
`cd7d76112dc454fbf4b73754d41326a4580c607be3d0571ce66ace59a63211a9`
and exited 0. Its retained output SHA-256 is
`26de759c4834447b828f4b1cb3842f2d7fe397f2c00eecf94982df4c0c110fb0`.
The native and cleanup receipts carry the same digest, so their evidence maps
to the exact reviewed diagnostic source.

## One-reproduction evidence and limits

The existing runner was executed once as `4dbc4f792d41a57f393b4308` and exited
101. The sequential control-plane manifest reached its fifth identity,
`a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve`.
Therefore this schedule advanced through the earlier third
`the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`
identity and the fourth identity without reproducing the original `d3a9…`
third-body `wait_flowing` failure.

This is a non-reproduction, not evidence that the original failure was fixed
or that its owner is known. The new diagnostic path did not execute its
failure rendering for that earlier condition, so this review grants no
Service/client console or owner-stage evidence for it.

At the fifth identity, the outer disposable guest did not reply to the
existing EXEC within its unchanged 600-second bound. The retained tail shows
the Service listener and probe-reset traffic, but the missing outer reply does
not establish a body-level cause. This bounded review neither classifies that
timeout as a production failure nor prescribes a fixture or product remedy.
No retry was launched to seek another schedule.

The raw output contains 102 successful libtest summaries totaling 112 passed,
zero failed and zero ignored. It contains no recognized runtime-SKIP result.
The runner did not complete its five-control-plane-body reconciliation, so no
117-pass credit is claimed and the partial progression is not counted as five
additional libtest summaries.

## Cleanup evidence

Read-only cleanup receipt `745c909c3d357acdfe60e273` used the synchronized
source digest and exited 0. It reports zero live Cloud Hypervisor processes,
no nft table and no STEP 14-01 work directory. The only listed namespace is
the documented pre-existing `ovd-ns-0000`.

## Findings and remediation disposition

No blocking finding is present in the bounded diagnostic source.

| Item | Disposition |
| --- | --- |
| Console retention and live-first fallback | **APPROVED** |
| `wait_flowing` predicate, deadline and allocation selection | **UNCHANGED** |
| Existing tracing subscriber and staged owner events | **APPROVED AS PRIVATE DIAGNOSTICS** |
| Original third-body failure | **NOT REPRODUCED; NO NEW EVIDENCE OR REMEDIATION** |
| Fifth-body outer EXEC timeout | **UNCLASSIFIED EVIDENCE LIMIT; NO REMEDIATION PRESCRIBED** |
| Complete 117-runner proof | **PENDING** |
| Roadmap validation | **REMAINS APPROVED** |

The diagnostic source is ready to preserve the required consoles and owner
stages if the original failure recurs under a separately authorized run. This
review does not authorize another native execution or any timeout, product,
body, selector or contract change.

## Iteration 9 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for diagnostic scope | Diagnostic output exists only on the unchanged fail-closed path and cannot create success. |
| GWT / one behavior | 10/10 | The five existing behaviors and identities remain unchanged. |
| Domain language | 10/10 | Service allocation, client allocation, owner stage and outer EXEC are kept distinct. |
| Coverage completeness | 10/10 for source design | Requested client and all observed allocation consoles plus owner stages are available on a future reproduction. |
| Walking-skeleton user value | 10/10 | The fixtures continue through production `run_server`, public submission and real VMs. |
| Priority validation | 10/10 | The change records evidence for the reproduced failure class without proposing a cause or mechanism. |
| Observable behavior | 10/10 | Diagnostics retain existing production console bytes and tracing events; no direct state replaces them. |
| Traceability | 10/10 | Every support addition maps to the missing evidence identified in the earlier `wait_flowing` receipt. |
| Walking-skeleton boundary | 10/10 | No hook, API, selector, lane, scenario or outcome dependency was added. |

## Iteration 9 verdict

**APPROVED.** The bounded control-plane diagnostic source is honest and
fail-closed. It preserves live Service/client console bytes across accepted
run-directory cleanup, keeps `console_of` live-first, leaves `wait_flowing`'s
predicate and 120-second deadline unchanged, and exposes existing private
owner events through the test process's tracing subscriber. The same five
control-plane identities and 117-body inventory remain intact.

The one reproduction did not reproduce the original third-body failure and
later stopped at the fifth body's outer EXEC boundary. It provides 112-pass
evidence only. Neither condition supports a product finding or remediation in
this review. Roadmap validation remains approved.

# APPROVED

# Review iteration 10 — fifth-control-plane-body phase diagnostics

## Iteration 10 metadata

| Field | Value |
| --- | --- |
| Scope | Source-only diagnostics for the existing fifth STEP 14-01 control-plane body |
| Synchronized source digest | `d68140dde187ee885c14880a15e71e92b1128b5fefff29ae377dca12d9c469bd` |
| Combined production diagnostic compile | `1df2bc8dc4f9768a40d59d26`, exit 0 |
| Format, CP integration check and CP Clippy | `adb0e7c4afe13418a0e773bf`, exit 0 |
| Forwarder Clippy | `e3eb13387a680bbdacdbcd20`, exit 0 |
| Native execution | None |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support or
acceptance source, DESIGN/DISTILL handoff, roadmap, DES event, execution log,
receipt, native state or commit was changed. No native, Lima or mutation
execution was launched.

Roadmap validation remains `approved`. This iteration reviews output-only
source diagnostics. It establishes no cause or product defect for the earlier
fifth-body outer EXEC timeout and authorizes no product remedy or new harness.

## Exact source identities

| Source | SHA-256 | Reviewed role |
| --- | --- | --- |
| Existing CP bodies and private fixture support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` | Fifth-body phase markers plus previously approved console support |
| Owner snapshot diagnostics | `9eae807a7d2225435353d2ac2d80b89da0f1ac6e83f366f30d5da1ec772abd7a` | Owner-state lock and inventory-call boundaries |
| Host inventory diagnostics | `be7e03a2a5229d6cbd9d5427b40fc93484c58aaff43d0ad53d377a7301bf7b00` | Host forwarder acquisition and inventory call |
| Owner task shutdown diagnostics | `4d8356d7c35b14a3c09c4cf6307334a75e6714f43c3936ef9a8ede6344eec9bd` | Existing owner cancellation/join sequence |
| ServerHandle shutdown diagnostics | `2a4d10aa084478a67a18c4bd6fc97e42af6be560206d48de8d992f2902bc3107` | Existing serve shutdown stages |
| Kernel inventory diagnostics | `e5cb0c94fb42ad807713f2540546d5e5774ff3e8f648b01a0c712d588d6144d4` | Typed ioctl result before unchanged mapping |
| Forwarder tracing dependency | `e4b9e5a1bd7069bdaf61bfb8acd819374bfeddf88af2bf6c524618ba22c6f0cb` | Existing workspace `tracing` dependency only |
| Unchanged GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` | Five CP identities and 117 accounting |

The lockfile identity is
`fdebdb7b566a7698ae6615e0e1fc5664c9550b44285db83c2a064a5e202d9834`.
The reviewed forwarder fallout adds the existing workspace `tracing`
dependency; it adds no feature flag, public type, method, error or runtime
control.

## Fifth-body marker audit

The marker helper is private to
`guest_flow_through_serve.rs`. Every `step14_01.cp_phase` event carries the
exact identity
`a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve`, the
current Service allocation identifier and the current client allocation
identifier (`guest_flow_through_serve.rs:115-129`). No predicate, assertion or
fixture reads an event.

The fifth body preserves its existing action and await sequence and adds
start/complete markers at these exact boundaries:

1. Immediately before and after the existing `wait_flowing(&client_alloc)`
   call.
2. Immediately before and after the existing observation whose predicate is
   exactly one Paired `DialPurpose::Relay` for the Service allocation.
3. Immediately before the real `libc::kill(..., SIGKILL)` request and after
   the unchanged `rc == 0` assertion. Both kill events additionally carry the
   actual `cloud-hypervisor` PID selected for the Service allocation.
4. Immediately before the existing client-terminal polling loop and after it
   returns an observed terminal event.
5. Immediately before and after the existing observation whose predicate
   requires the dead Service allocation to be absent from every owner CID.
6. Immediately before and after the existing `Node::shutdown().await`.

The exact source order remains:

```text
client flowing
relay held
pre-kill client-is-open assertion
Service VMM SIGKILL
client terminal polling and Reset assertion
dead Service launch absent
inventory agreement
client and Service stop
empty-owner teardown and zero inventory
Node shutdown
```

The markers do not move a statement across another statement and do not add an
await, sleep, retry, timeout or branch. `BOOT_BOUND` remains 120 seconds,
`SETTLE_BOUND` remains 30 seconds, the polling intervals remain 200 ms, the
Service and client selectors remain the same allocation values, and the kill
stimulus remains the actual Service VMM `SIGKILL`. All assertions and
predicates are byte-identical around the new output statements.

The helper emits INFO into the already-installed test subscriber. Its fields
are formatting-only values; it does not retain a production object, query a
new state source, mutate allocation state or supply any success input. Missing
completion output therefore locates the last entered existing stage without
changing that stage's result.

## Companion owner and forwarder diagnostic audit

### Owner snapshot lock and inventory call

`HostSharedGuestNetworkOwner::flow_observation` emits DEBUG immediately before
and after the same owner-state lock acquisition. The owner still holds that
same guard while it decides whether a context exists, calls the existing
`GuestFlowIo::inventory`, builds `state.observation`, installs the inventory
read into that snapshot and explicitly drops the guard
(`flow_owner.rs:430-459`). The new inventory begin/complete events surround
only the pre-existing call. The returned `Result` is mapped exactly as before:
`Ok` becomes `ForwarderInventoryRead::Read`, every error becomes
`ForwarderInventoryRead::Unreadable`.

No lock acquisition, guard drop, inventory condition, output snapshot or error
mapping changed. The statements add observability inside the already existing
lock scope; they do not introduce a timeout, retry, await or alternate lock.

### HostGuestFlowIo forwarder acquisition and call

`HostGuestFlowIo::inventory` emits DEBUG before calling the same private
`forwarder()` helper and after its `Arc<ForwarderSlot>` has been acquired
(`flow_io/host.rs:839-870`). `forwarder()` still takes the host state lock only
to clone the current forwarder `Arc` and releases that temporary guard before
returning. The diagnostic code then invokes the same
`forwarder.context.inventory()` once and returns its exact typed result.

The seam and generation fields describe the already selected forwarder. There
is no second lookup, alternate generation, held state-lock guard, retry,
conversion or changed error. The begin/complete events therefore distinguish
host forwarder acquisition from the kernel inventory call without changing
either scope.

### Kernel inventory ioctl and typed result

`KernelFlowForwarder::inventory` initializes the same zeroed
`OvdFwdInventory`, executes the same single `inventory_ioctl` against the same
context fd and stores its existing typed result for DEBUG output
(`overdrive-fwd/src/lib.rs:288-318`). The `Err(Errno)` path still passes that
same error to `io_failure(ForwarderOp::Inventory, error)`. The success path
still validates `reserved == 0` and constructs the same
`ForwarderInventory` fields.

The diagnostic exposes the typed ioctl `Result`, including the exact errno on
failure. It changes no ioctl arguments, return mapping, UAPI validation,
public error taxonomy or timeout.

### Owner and ServerHandle shutdown

Owner-task shutdown retains the existing order: cancel under the owner-tasks
lock, join stream/control/end-reader root tasks, collect and cancel child
flows under the owner-state lock, join those child tasks, then wait for
refusal readers to drain (`owner_runtime.rs:117-218`). Naming root tasks and
enumerating child joins only supplies stage fields; every optional task is
consumed once in the same array/vector order, and each join result remains
ignored after its success boolean is logged.

`ServerHandle::shutdown` likewise retains the accepted order and the same
`drain_deadline`: convergence, emit drain, interest router, axum graceful
shutdown, server task, exit observers, shared-network supervisor, mTLS worker
owner, guest-flow owner and mTLS resolve owner (`lib.rs:2838-2940`). Indexing
exit-observer joins only labels their existing order. The same cancellations,
awaits, worker-error capture and final `Result` remain.

These shutdown events do not introduce a deadline, await, cancellation,
branch, public hook or error conversion. They expose which already existing
synchronous lock or asynchronous join boundary was last entered.

## Test identity and accounting audit

The GuestKit runner manifest remains byte-identical. It names the same five
control-plane bodies, each with count one, and the same one CLI P6-S13 body.
The inventory remains 111 retained bodies plus one CLI body plus five CP
bodies, for 117 only after the complete fail-closed manifest reconciles.

The fifth body's `CONTRACT_SHAPE: bounded-change.` declaration, identity,
lane, production `run_server` composition, public submission path, Service and
client workload shapes, and all stimuli and assertions remain unchanged. No
test, scenario, selector, API, production mechanism or output-dependent oracle
was added.

## Source-gate evidence

All three retained Metal source-only receipts use synchronized source digest
`d68140dde187ee885c14880a15e71e92b1128b5fefff29ae377dca12d9c469bd`:

| Receipt | Evidence | Result |
| --- | --- | --- |
| `1df2bc8dc4f9768a40d59d26` | Combined diagnostic compile | exit 0; output SHA-256 `d44135316b18ca018e446af86ffa139b8cf67d05c8f3354ad7d8b23d0cabb7a4` |
| `adb0e7c4afe13418a0e773bf` | Format, CP integration check and CP Clippy with warnings denied | exit 0; output SHA-256 `9a66fee8277634969737aea8c2f27d2e4c393e048710fba8de583e2b99941ec3` |
| `e3eb13387a680bbdacdbcd20` | Forwarder Clippy with warnings denied | exit 0; output SHA-256 `c202da562890c95e2aaa7d0e67f4cd1b9fef7bc75f411026ab5324b030e3ac90` |

Each terminal record declares `kind=command`, `exit_code=0` and
`cause=command_returned`. These are compilation, formatting and lint receipts;
they do not execute an acceptance body, GuestKit VM, kernel mutation or
cleanup path. No runtime-SKIP, native pass count or behavioral GREEN credit is
claimed.

## Findings and remediation disposition

No bounded source finding remains.

| Item | Disposition |
| --- | --- |
| Fifth-body phase markers | **APPROVED AS OUTPUT-ONLY PRIVATE DIAGNOSTICS** |
| Exact test, Service/client allocation and VMM PID fields | **APPROVED** |
| Predicates, assertions, deadlines, stimuli and order | **UNCHANGED** |
| Owner/host/kernel inventory diagnostics | **APPROVED AS OUTPUT-ONLY PRIVATE DIAGNOSTICS** |
| Owner and ServerHandle shutdown diagnostics | **APPROVED AS OUTPUT-ONLY PRIVATE DIAGNOSTICS** |
| Five CP identities and 117 inventory | **UNCHANGED** |
| Earlier fifth-body timeout cause | **UNPROVEN; NO PRODUCT REMEDY PRESCRIBED** |
| Roadmap validation | **REMAINS APPROVED** |

This source is suitable for a separately authorized existing-runner
reproduction. This review itself does not authorize or perform native
execution and does not prescribe a product change or new harness.

## Iteration 10 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for diagnostic scope | Every marker brackets a fail-closed existing stage and supplies no success value. |
| GWT / one behavior | 10/10 | The existing fifth behavior and all other CP identities remain unchanged. |
| Domain language | 10/10 | Service allocation, client allocation, VMM kill, owner snapshot, kernel inventory and shutdown stages remain distinct. |
| Coverage completeness | 10/10 for source design | Every bounded fifth-body stage and each suspected existing observation/shutdown boundary has start/completion output. |
| Walking-skeleton user value | 10/10 | Diagnostics remain on the real production composition, public submission and real VMM path. |
| Priority validation | 10/10 | Source records evidence for an unclassified timeout without inventing a cause or remedy. |
| Observable behavior | 10/10 | Exact identities, PID, typed ioctl result and existing stage boundaries are emitted without becoming oracles. |
| Traceability | 10/10 | Every event maps to the fifth-body stage or companion production boundary named by the handoff. |
| Walking-skeleton boundary | 10/10 | No API, hook, selector, scenario, lane, timeout or alternate mechanism was introduced. |

## Iteration 10 verdict

**APPROVED.** The fifth-control-plane-body markers and companion
owner/forwarder/shutdown tracing are bounded output-only diagnostics. They
preserve the exact body identity, Service/client allocations, real VMM kill,
predicates, assertions, deadlines, order, selectors, error mappings and
shutdown behavior. The same five CP identities and 117-body inventory remain.

All source gates pass at one synchronized digest. No native evidence is
claimed, the earlier timeout remains unclassified, and this review prescribes
no product remedy or new harness. Roadmap validation remains approved.

# APPROVED

# Review iteration 11 — isolated exact-fifth control-plane boundary

## Iteration 11 metadata

| Field | Value |
| --- | --- |
| Scope | Source-only review of the isolated launcher for the existing fifth CP cell |
| Isolated source | `spike-scratch/netns-density-295-cp-14-01/increment-a/` |
| Current canonical runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| Existing fifth body and fixture support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` |
| Final source gate | `301fd2d5f02795b5091c5b9f`, exit 0 |
| Final synchronized source digest | `21cf0e148603d9d11e86094c02bb13a235b4e04b721504d4c27cea4966751e6f` |
| Native execution | None |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, execution log, receipt, host state or commit was changed.
No native, Lima or mutation execution was launched.

Roadmap validation remains `approved`. This boundary can produce isolated
diagnostic evidence for the already-authored fifth body. It is not a new test
tier, scenario or product path and cannot supply any part of the canonical
117-body result.

## Spike isolation and source inventory

The increment is self-contained under the required `spike-scratch` boundary.
It has its own workspace, manifest, lockfile, build script, entrypoint, README
and retained raw source-gate evidence. Its only direct dependency is a one-way
path dependency on the existing `overdrive-testing` crate with that crate's
existing `integration-tests` feature. No workspace manifest names the
standalone crate, no file under `crates/` imports it, and no canonical test
runner, selector or CI lane refers to it. The repository ignore rules exclude
only this increment's future `target/` and `out/` build products; its source
and evidence remain reviewable.

| Isolated artifact | SHA-256 |
| --- | --- |
| `Cargo.toml` | `951508fa477bdbc061cacd22637829f621a5d99855b1e9ee6b60cd7a07abe5fb` |
| `Cargo.lock` | `7559a9949ba69490d0073682c29deea69ecd18ded49987a15d1b1bb4ad339937` |
| `build.rs` | `5745bca11c41b70eedd3d2283422e053d241da4b2fa81510954243715e19b1ad` |
| `src/main.rs` | `9ae793c327f7b707a9a7870a0d90eb8016f7031463201a2ab1a41b726e1f42b1` |
| `README.md` | `fa06896b14f9cb35592dc6e53a9e96a95fc5c4ab7dd8d684051976bf74f0abc5` |

The three retained evidence directories contain byte-identical copies of each
Metal record also held under `.context/netns-density-295/14-01/raw/<run-id>/`.
No runtime output is present because every retained record is a source gate.

## Canonical-source inclusion audit

The current canonical runner begins with file-level inner documentation and
two crate attributes, followed by its first Rust item, `use std::fs;`. The
build script reads that current file at build time, declares it as a Cargo
rerun input and slices from that exact first item. Mechanical inspection of
the current prefix confirms that the removed 28 lines contain only blank
lines, `//!` file documentation and the two `#![...]` file attributes. No
constant, import, type, function, manifest cell or executable statement is
removed or rewritten.

From `use std::fs;` through the canonical file's final item, the generated
body is byte-for-byte the current runner suffix. Its SHA-256 is
`5d79d96f283e0ea185b6160e4252f4ac5eff6bd97a4942a333ccf1b24379a171`.
The build script appends only the private
`run_exact_fifth_control_plane_cell` wrapper. The included canonical `main`
remains an inert private function inside the standalone module; the
standalone entrypoint calls only the appended wrapper.

`src/main.rs` wraps the generated items in a private module. Its `allow`
attribute suppresses only lint classes created because the exact included
runner contains unused groups in this one-cell executable. It changes no
included item, selection or runtime branch.

This mechanism is fail-closed against runner drift: any removal or renaming of
the current first import fails the build script, and Cargo reruns generation
whenever the canonical runner changes. The review approves the current exact
source identity; it does not treat a future changed runner as pre-approved.

## Exact fifth-cell selection

The canonical `STEP14_01_CONTROL_PLANE` remains a five-element slice. Element
index four is exactly:

```text
integration::guest_flow_through_serve::a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve
expected passed = 1
exact filter = true
```

The private wrapper takes only `[4..5]` from that existing slice and invokes
the included canonical `run_step14_01_binary` with:

```text
package = overdrive-control-plane
target = integration
name = step14-01-control-plane-fifth-diagnostic
slot = 15
cells = existing STEP14_01_CONTROL_PLANE[4..5]
```

Slot 15 is the same registered outer-guest slot used by the canonical
STEP 14-01 control-plane invocation. The private diagnostic name only isolates
its work-directory and console label; it does not enter the test arguments,
workload specification or product configuration. The wrapper requires the
existing helper to return exactly one passed body. That count is the existing
cell's manifest count, not an injected product expected value.

No string copy of the body identity, replacement predicate, expected console
line, fixture success value or product state appears in the standalone
wrapper. Selection remains tied to the current canonical manifest cell, and
the body itself remains the existing production-composition body reviewed in
iteration 10.

## GuestKit boundary equivalence

Because the standalone includes and calls the same helper and all its private
callees, it retains the canonical fifth-cell boundary rather than recreating
it:

- `InGuestTests::prepared("overdrive-control-plane", "integration")` stages
  the existing integration binary at the existing in-guest path.
- `step14_01_nested_support` stages the same guest peer, prepared production
  init, forwarder/BPF artifacts, nested Cloud Hypervisor binary, kernel,
  reflink XFS volume, host tools and exact module closure.
- `GuestRun::boot_with_vcpus` builds the same `GuestInit::ScriptedPeer` image
  with the forwarder and loopback modules, 4096 MiB memory and eight vCPUs,
  then boots it at registered slot 15. The wrapper supplies no NIC, uplink,
  cmdline override or extra payload.
- The same preconditions load the same modules, set `/dev/kvm` and
  `/dev/vhost-vsock` to the existing qualified mode, report virtualization,
  mount cgroup v2, bpffs and securityfs, enable the same cgroup controllers,
  preserve `/var/run -> /run`, mount the XFS reflink volume and prove a real
  reflink.
- The existing cell's `exact = true` value routes through
  `exec_required_cgroup_cells_with_bound`, which constructs the unchanged
  `--test-threads=1 --nocapture --exact <existing identity>` filter.
- `exec_cgroup_required` uses the unchanged `CGROUP_NOFILE_EXEC`, requiring
  both hard and soft `RLIMIT_NOFILE` to be at least 376832 before executing
  the exact integration binary.
- `STEP14_01_CELL_BOUND` remains 600 seconds. A nonzero exit, missing exact
  one-pass summary, recognized runtime-SKIP line or missing EXEC reply fails
  closed.
- `GuestRun::finish` prints the outer console, requests poweroff and requires
  the disposable guest to exit within the existing 30-second bound. The same
  helper then drops its support work directory; error paths retain the same
  RAII owners as the canonical invocation.

The workload body, `run_server` composition, public submission, phase markers,
console capture, owner/inventory/ioctl/shutdown tracing and real VMM stimulus
are unchanged. Kernel, cgroup, security, nested-VM and cleanup effects stay
inside the same disposable GuestKit boundary. The standalone adds no public
or canonical selector, environment group, API, hook, alternate fixture,
scenario, lane or expected-value seam.

## Canonical 117-body accounting

The canonical runner and its manifests are unmodified. Its full result remains
111 retained bodies plus one CLI body plus all five CP bodies. The standalone
calls a private included helper outside that canonical manifest reconciliation;
even a passing isolated fifth cell is diagnostic evidence only. It cannot be
added to a partial canonical run, substituted for a missing CP result or
reported as a 117 pass.

The prior full-runner receipt `af7d3395010cef474da2c000` remains unchanged and
failed honestly. It contains 101 successful summaries totaling 111 passed,
zero failed and zero ignored. The activated existing CLI body then failed at
`vsock_walking_skeleton.rs:166` because `alloc-v6-client-0` did not emit the
existing `v6-client: state=unhealthy` marker within its existing bound. It has
zero `step14_01.cp_phase` events and no control-plane result. This isolated
source review neither retries nor reclassifies that receipt and grants it no
CP or 117 credit.

Its cleanup receipt `df620b468fcb43b6e4a94f61` remains exit 0 with no live
Cloud Hypervisor process, nft table or STEP 14-01 work directory; only the
documented pre-existing `ovd-ns-0000` is listed.

## Compiler-failure and correction audit

The retained initial record `9431daa0352f84a9fce5b7ed` exited 101 before any
runtime. It directly included the binary source in a module, so Rust rejected
its inner `//!` documentation and `#![...]` crate attributes. The output
contains the exact E0753 and inner-attribute diagnostics; it contains no guest
boot or test result.

The correction stayed within the same pre-runtime increment. `build.rs`
removes only those file-level lines, retains every executable runner item and
appends the private wrapper. Intermediate compile record
`496ace560d7f1928c945afc3` exited 0 and exposed only unused-item warnings from
including the full canonical runner. The current private module lint allowance
addresses those inclusion-only warnings without editing copied items.

Final locked Clippy gate `301fd2d5f02795b5091c5b9f` used synchronized source
digest `21cf0e148603d9d11e86094c02bb13a235b4e04b721504d4c27cea4966751e6f`,
exited 0 with warnings denied, and has output SHA-256
`bf8c58ad43e85113e178ea12e547087396519ded47b432d2bd6e66c8c1252833`.
Its terminal record declares `kind=command`, `exit_code=0` and
`cause=command_returned`.

The failed direct include, neutral compiler correction and final Clippy gate
are all retained. Since none executed the diagnostic, correcting the source in
this one increment does not overwrite or rerun a runtime probe. No native
behavioral credit is inferred from compilation.

## Findings and remediation disposition

No bounded source or boundary finding remains.

| Item | Disposition |
| --- | --- |
| Spike isolation | **APPROVED** |
| Canonical runner inclusion | **APPROVED; ALL EXECUTABLE ITEMS BYTE-IDENTICAL** |
| Exact fifth-cell and slot-15 selection | **APPROVED** |
| GuestKit payload, prerequisites and confinement | **UNCHANGED THROUGH THE EXISTING HELPER** |
| Exact filter, nofile limit, 600-second bound and skip rejection | **UNCHANGED** |
| Cleanup behavior | **UNCHANGED** |
| Public/canonical selector, API, hook, scenario or lane | **NONE ADDED** |
| Prior `af7d…` CLI failure | **RETAINED AS FAILURE; ZERO CP CREDIT** |
| Isolated result contribution to 117 | **FORBIDDEN** |
| Roadmap validation | **REMAINS APPROVED** |

This review approves the source and exact diagnostic boundary only. It does
not execute or authorize a native run, prescribe production remediation or
broaden the standalone launcher.

## Iteration 11 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for boundary scope | Nonzero exit, absent summary, runtime skip, timeout and cleanup failure all remain fail-closed. |
| GWT / one behavior | 10/10 | The launcher selects only the already-authored fifth CP behavior. |
| Domain language | 10/10 | Canonical runner, outer GuestKit, existing fifth body and 117 reconciliation remain distinct. |
| Coverage completeness | 10/10 for source boundary | Payload, modules, mounts, cgroup/nofile, exact filter, timeout, skip rejection and cleanup all reuse the canonical helper. |
| Walking-skeleton user value | 10/10 | The isolated launcher still reaches the existing production `run_server`, public submission and real nested VMM path. |
| Priority validation | 10/10 | Isolation bypasses only an unrelated manifest prefix and adds no product or test mechanism. |
| Observable behavior | 10/10 | The launcher observes the existing body and diagnostics without injecting an expected product value. |
| Traceability | 10/10 | Every isolated statement maps to a current canonical runner item or the private one-cell wrapper. |
| Walking-skeleton boundary | 10/10 | No crates wiring, public selector, canonical manifest change, lane or alternate fixture was introduced. |

## Iteration 11 verdict

**APPROVED.** The isolated increment includes every current canonical runner
item unchanged after removing only file-level documentation and crate
attributes, then appends one private wrapper selecting the exact existing
fifth CP cell at slot 15. It inherits the canonical GuestKit payload,
prerequisites, confinement, cgroup/nofile execution, exact filter, 600-second
bound, runtime-SKIP rejection and cleanup.

The earlier `af7d…` CLI failure remains the full-runner result and supplies no
CP credit. Any future isolated result is diagnostic-only and cannot count
toward 117. No native evidence is claimed, no production remedy is prescribed,
and roadmap validation remains approved.

# APPROVED

# Review iteration 12 — isolated-root pre-execution remediation

## Iteration 12 metadata

| Field | Value |
| --- | --- |
| Scope | Neutral compile-context correction before isolated fifth-cell execution |
| Updated generator | `f5e87ea63a8da7d625cfe47ff042a40ca802700806c5539857b34bb34daa2555` |
| Final source gate | `e96d51565e124918953f6796`, exit 0 |
| Final synchronized source digest | `7e00d96bfe777cf85c9f2b3f4c1a19b4b3ae4f6c1a20fdd8980cbc8f1ef131a4` |
| Reviewed executable | `1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff` |
| Native execution | None |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, host state or commit was changed. No native, Lima
or mutation execution was launched.

Roadmap validation remains `approved`. This iteration changes only how the
isolated compilation context resolves the canonical runner's repository root
and how the already-built launcher will be invoked. It changes no accepted
outcome or test behavior.

## Retained pre-execution failures

### `b8eb…` — guest-kit preparation compiler failure

Privileged preparation run `b8eb7c5200dabe33d95c6ed0` exited 1 while
`cargo xtask guest-kit-prepare` compiled the in-guest binaries. It stopped
before the isolated launcher, outer GuestKit image or fifth body could run.
The focused source-only reproduction `8f3d422621e756caed17ddf4` identifies the
failure as Rust E0282 at `overdrive-dataplane/src/mtls/mod.rs:472`: the private
mTLS diagnostic cast from `&storage as *const _` did not identify its pointee
type.

The crafter's correction names `libc::sockaddr_storage` in that cast. It does
not alter the pointer, target representation, socket read, diagnostic result,
error mapping or any public interface. Current `mod.rs` is
`e56a4a10fb6da5441dc0bac275801b7c6c9bad608ed953279179c5b8ee7fb011`;
the exact dataplane gate `fd3f392aa58852c2a24e48c3` exited 0. This is
compiler fallout for crafter-owned private tracing and grants no acceptance
credit.

The `b8eb…` output contains no test summary, `step14_01.cp_phase` event,
GuestKit boot or Cloud Hypervisor launch. It receives zero fifth-body and zero
117 credit.

### `bafb…` — standalone compile-time workspace root

Privileged run `bafb505ab7a31300e5294938` completed the ordinary
`guest-kit-prepare`, then built and started the standalone launcher. Before
`GuestRun::boot_with_vcpus` could construct or boot the outer image,
`step14_01_nested_support` resolved the unchanged BPF artifact to the wrong
path:

```text
/home/ubuntu/overdrive/spike-scratch/target/bpf/overdrive_bpf.o
```

The copy failed with `ENOENT`. That path came from the included runner's
compile-time `CARGO_MANIFEST_DIR`, which referred to `increment-a` in the
standalone crate rather than `crates/overdrive-testing`. The receipt contains
no test result, CP phase, runtime-SKIP or outer-VM console. It receives zero
body and 117 credit.

The failed attempt used root Cargo to build the standalone binary under the
increment's local target directory. The corrected runtime command does not
repeat that build or use that artifact.

## Exact workspace-root replacement

The build script still removes only the canonical runner's file-level docs and
crate attributes. Within the otherwise byte-identical canonical suffix it
requires exactly one occurrence of the current private helper:

```rust
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("overdrive-testing has a workspace-root grandparent"))
        .to_path_buf()
}
```

Mechanical inspection confirms the current runner contains that exact byte
sequence once and contains no pre-existing replacement helper. The generator
uses `replacen(..., 1)` only after asserting the count is exactly one. The
prefix and suffix around that one helper remain byte-identical. Runner drift
that removes, duplicates or changes the canonical helper therefore fails the
build rather than silently selecting a path.

The replacement remains private to the generated standalone module:

```rust
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("OVERDRIVE_DIAGNOSTIC_WORKSPACE_ROOT"))
}
```

`build.rs` derives the compile-time value by canonicalizing
`increment-a/../../..`. In the reviewed workspace this is exactly
`/Users/marcus/conductor/workspaces/helios/wellington-v2`; on Metal it is the
same `/home/ubuntu/overdrive` root that the canonical helper derives by taking
two parents from `/home/ubuntu/overdrive/crates/overdrive-testing`. The
replacement changes the macro's compilation context, not the resulting
repository-root value.

The canonical `workspace_root()` has one use: appending
`target/bpf/overdrive_bpf.o` in `workspace_bpf_artifact`. The corrected
standalone therefore stages the same repository BPF object as the canonical
runner. Forwarder-module staging continues to use the prepared GuestKit's
exact module path, while kernel, nested-device, netfilter, veth, kTLS, init,
Cloud Hypervisor, guest-peer and XFS payload paths continue through their
unchanged canonical helpers. No guest path, host artifact, module list or
payload ordering changes.

## Existing fifth-cell behavior remains exact

Outside the private workspace-root helper, the generated runner body and the
private exact-fifth wrapper remain the iteration-11 source. The canonical
runner identity is still
`45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9`;
the existing fifth body and fixture support remain
`8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795`.

The wrapper still selects only `STEP14_01_CONTROL_PLANE[4..5]`, calls the
existing `run_step14_01_binary` for `overdrive-control-plane` / `integration`
at slot 15, and requires the existing count of one. The same GuestKit image,
eight-vCPU boundary, exact filter, cgroup/nofile wrapper, 600-second bound,
runtime-SKIP rejection, one-pass summary, production body, diagnostics and
cleanup remain. No selector, test, scenario, API, lane, deadline, oracle,
expected product value or canonical manifest changed.

## Build-once and direct-execution boundary

Final source gate `e96d51565e124918953f6796` ran on Metal as the unprivileged
user with the dedicated external target directory
`/home/ubuntu/overdrive/target-codex-14-01`. It ran locked Clippy with warnings
denied, then a locked build, and exited 0 at synchronized source digest
`7e00d96bfe777cf85c9f2b3f4c1a19b4b3ae4f6c1a20fdd8980cbc8f1ef131a4`.
The gate records:

```text
1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff  /home/ubuntu/overdrive/target-codex-14-01/debug/netns-density-295-cp-14-01-diagnostic
```

The source-gate output SHA-256 is
`c419bcfedad5e1c3285ae13c18ab4b9734048335627f167b15dacfacc0514317`;
its terminal record declares `kind=command`, `exit_code=0` and
`cause=command_returned`. It contains no test, VM or runtime evidence.

The reviewed privileged runtime command first performs the canonical
`cargo xtask guest-kit-prepare`, prints the hash of that exact external binary,
and then uses `exec` on its absolute path. It does not invoke `cargo run` or
build the standalone crate as root. `guest-kit-prepare` may update the
canonical prepared in-guest artifacts under the repository target, but it
cannot rebuild or replace the standalone executable under
`target-codex-14-01`, and no root Cargo output is created under the isolated
increment. Matching the printed runtime hash to the reviewed hash is required
before any isolated result receives evidence credit.

No runtime was launched during this review, so the execution hash has not yet
been re-observed and no fifth-body behavior is claimed.

## Accounting and scope

The isolated executable remains diagnostic-only. It is outside the canonical
runner's fail-closed reconciliation and cannot be added to the earlier
`af7d…` 111-pass prefix or any other partial run. A future isolated pass or
failure can locate the fifth body's stage, but the full 117 result still
requires one canonical runner execution to pass all retained, CLI and five CP
bodies.

The pre-execution failures, compiler correction, workspace-root correction and
source gate do not add a public or canonical selector, test, scenario, API,
hook, lane, deadline or oracle. They do not authorize production remediation
or broaden the isolated launcher.

## Findings and remediation disposition

No bounded source or boundary finding remains.

| Item | Disposition |
| --- | --- |
| `b8eb…` mTLS compilation failure | **RETAINED; ZERO RUNTIME OR BODY CREDIT** |
| `bafb…` workspace-root failure | **RETAINED; ZERO RUNTIME OR BODY CREDIT** |
| Private workspace-root replacement | **APPROVED AS NEUTRAL COMPILE-CONTEXT CORRECTION** |
| BPF/module/payload identities | **CANONICAL PATHS RESTORED; OTHERWISE UNCHANGED** |
| Exact fifth-cell behavior and boundary | **UNCHANGED** |
| Unprivileged locked executable build | **APPROVED** |
| Privileged direct-execution command | **APPROVED SUBJECT TO MATCHING THE REVIEWED SHA-256** |
| Selector/test/scenario/API/lane/deadline/oracle | **NONE ADDED OR CHANGED** |
| Isolated evidence contribution to 117 | **FORBIDDEN** |
| Roadmap validation | **REMAINS APPROVED** |

This review approves the neutral source and boundary remediation only. It does
not execute native behavior, prescribe production remediation or expand the
standalone harness.

## Iteration 12 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for boundary scope | Both preparation failures remain recorded and receive zero behavioral credit. |
| GWT / one behavior | 10/10 | The launcher continues to select only the existing fifth CP body. |
| Domain language | 10/10 | Compile context, repository BPF artifact, outer GuestKit and canonical 117 reconciliation remain distinct. |
| Coverage completeness | 10/10 for source boundary | The exact helper replacement, path equivalence, payload preservation, executable identity and runtime command are all pinned. |
| Walking-skeleton user value | 10/10 | The corrected boundary still invokes the existing production-composition body without an alternate fixture. |
| Priority validation | 10/10 | The correction restores the canonical root instead of adding a product or runner selector. |
| Observable behavior | 10/10 | The executable hash is externally recorded; no compiled result is misreported as runtime evidence. |
| Traceability | 10/10 | Each correction maps directly to the retained E0282 or missing-BPF pre-execution failure. |
| Walking-skeleton boundary | 10/10 | No canonical manifest, lane, API, hook, deadline or oracle changed. |

## Iteration 12 verdict

**APPROVED.** The generator requires one exact canonical private
`workspace_root` helper and replaces only its standalone compile-time context
with a canonicalized path equal to the repository root the canonical runner
already derives. This restores the same BPF path while leaving every module,
payload, helper, fifth-cell action and fail-closed runner property unchanged.

The two preparation failures remain honest zero-credit records. The
unprivileged locked gate builds the reviewed executable, and the privileged
runtime will execute that binary directly after ordinary guest-kit
preparation. No native evidence is claimed, isolated evidence still cannot
count toward 117, and roadmap validation remains approved.

# APPROVED

# Review iteration 13 — durable isolated-artifact staging

## Iteration 13 metadata

| Field | Value |
| --- | --- |
| Scope | Final pre-execution staging correction for the approved isolated fifth-cell launcher |
| Updated README / command boundary | `b6eaeb65559030b080256df5738aead4ec2ca68dfe61d3df6c9eb78599fb1740` |
| Source and install gate | `be330805a138397f3149d764`, exit 0 |
| Synchronized source digest | `99a0e6e6248760d8acefbf5fdedba728692f0bdcdcb278567acb0d7dc41dbc17` |
| Reviewed executable | `1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff` |
| Native execution | None |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, Metal cache, host state or commit was changed. No
native, Lima or mutation execution was launched.

Roadmap validation remains `approved`. This iteration changes only the durable
location of the already-reviewed executable between an unprivileged build and
the privileged diagnostic invocation. It adds no product or test behavior.

## Retained `037cb…` pre-execution failure

Run `037cb50421d19352ac4609ff` used the previously approved direct-execution
command and exited 1. It completed the ordinary `guest-kit-prepare`; the next
operation was the fail-closed SHA-256 read of
`/home/ubuntu/overdrive/target-codex-14-01/debug/netns-density-295-cp-14-01-diagnostic`.
That file no longer existed because the Metal workspace synchronization before
the privileged command removed the noncanonical target directory.

The receipt contains no standalone executable start, `overdrive-init` output,
outer GuestKit boot, Cloud Hypervisor process, test result, runtime-SKIP or
`step14_01.cp_phase` event. The exact fifth body did not enter. The receipt is
retained as an artifact-lifetime failure and receives zero body, CP and 117
credit.

Its terminal record declares `kind=command`, `exit_code=1` and
`cause=command_returned`; its output SHA-256 is
`13602fcbbb2a44d33e213f2f47be3876d36969a807cfd5eeb21415522e2092d2`.
The raw record is preserved byte-for-byte under both evidence roots.

## Unchanged reviewed executable

The isolated executable source is unchanged from iteration 12:

| Source | SHA-256 |
| --- | --- |
| Manifest | `951508fa477bdbc061cacd22637829f621a5d99855b1e9ee6b60cd7a07abe5fb` |
| Lockfile | `7559a9949ba69490d0073682c29deea69ecd18ded49987a15d1b1bb4ad339937` |
| Runner-source generator | `f5e87ea63a8da7d625cfe47ff042a40ca802700806c5539857b34bb34daa2555` |
| Diagnostic entrypoint | `9ae793c327f7b707a9a7870a0d90eb8016f7031463201a2ab1a41b726e1f42b1` |
| Included canonical runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| Existing fifth body and support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` |

The only source-level update is the README's exact build/install/runtime
commands. The compiled program still contains the approved private
workspace-root correction, the byte-identical canonical runner items and the
private wrapper selecting `STEP14_01_CONTROL_PLANE[4..5]` at slot 15. The
executable SHA-256 remains exactly
`1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff`.

No test body, payload, manifest cell, selector, fixture, production API,
scenario, lane, deadline, skip policy, oracle or cleanup behavior changed.

## Unprivileged build and content-addressed install

The source command continues to run through the Metal launcher's unprivileged
mode with the external target directory
`/home/ubuntu/overdrive/target-codex-14-01`. It performs the same locked Clippy
with warnings denied and locked build. It then uses `install -D -m 0755` to
copy the resulting executable to:

```text
/home/ubuntu/.cache/overdrive-step14-01/
  1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff/
  diagnostic
```

That cache path lies outside `/home/ubuntu/overdrive`, the workspace tree
replaced by Metal synchronization. The directory name pins the reviewed
content identity, and the source command immediately recomputes the installed
file's SHA-256 and requires exact equality. A failed copy, permission change or
digest mismatch terminates the gate before approval.

Gate `be330805a138397f3149d764` used source digest
`99a0e6e6248760d8acefbf5fdedba728692f0bdcdcb278567acb0d7dc41dbc17`
and exited 0. Its output records both locked compilation stages completing;
the successful `install` and shell `test` are intentionally silent. The
output SHA-256 is
`46eaa7c8e03f0de2fa2163101df43ddaffd2a7316a45e2294b2917fda2aca1d8`,
and the terminal declares `kind=command`, `exit_code=0` and
`cause=command_returned`.

The installed cache artifact is a durable copy of the reviewed standalone
tool, not a new executable implementation, test harness or selector. The
source gate creates it as the unprivileged Metal user; no privileged Cargo
build or `cargo run` creates the standalone artifact.

## Privileged invocation boundary

The reviewed privileged command performs the ordinary canonical
`guest-kit-prepare`, then resolves only the content-addressed cache file. It
recomputes that file's SHA-256 and requires exact equality with
`1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff`
before using `exec` on the same absolute path.

The workspace synchronization and guest-kit preparation cannot remove the
user-cache artifact and do not build or replace it. A missing file or digest
mismatch fails before executable entry. After the check, `exec` introduces no
wrapper process or alternate arguments; the reviewed program enters its
existing private exact-fifth wrapper directly.

The privileged command invokes Cargo only for the ordinary canonical
`guest-kit-prepare` prerequisite. It does not run `cargo build`, `cargo run` or
Clippy for the standalone crate as root and creates no root-owned Cargo output
under the isolated increment. This is the same GuestKit preparation already
required by the canonical runner; content-addressed staging adds no execution
mechanism beyond preserving the reviewed standalone binary across workspace
synchronization.

No runtime has occurred after this correction. The runtime digest check and
body behavior therefore receive no execution credit in this source-only
review.

## Preserved behavior and accounting

Once invoked, the executable still calls the existing
`run_step14_01_binary` with the exact fifth existing CP cell and slot 15. The
same prepared integration binary, GuestKit payload, BPF/module closure,
security and cgroup mounts, nested-KVM confinement, no-uplink boundary,
`CGROUP_NOFILE_EXEC`, exact filter, 600-second bound, required one-pass
summary, runtime-SKIP rejection and cleanup apply.

The cache location is never passed into the test body, outer guest, production
server or workload VM. It cannot alter a predicate or inject an expected
value. The canonical runner remains unchanged at five CP identities and 117
only after its complete 111 + CLI + five-CP reconciliation. Any isolated
result remains diagnostic-only and cannot be combined with `af7d…`, `037cb…`
or another partial receipt.

## Findings and remediation disposition

No bounded source or staging finding remains.

| Item | Disposition |
| --- | --- |
| `037cb…` missing workspace artifact | **RETAINED; ZERO EXECUTION OR BODY CREDIT** |
| Reviewed executable identity | **UNCHANGED** |
| Unprivileged content-addressed install | **APPROVED** |
| Post-prepare digest requirement | **APPROVED; FAILS BEFORE ENTRY ON ABSENCE OR MISMATCH** |
| Privileged standalone Cargo build/run | **ABSENT** |
| Test/body/payload/selector/lane/API/deadline/oracle | **UNCHANGED** |
| Durable staging mechanism | **BOUNDED TO THE REVIEWED STANDALONE TOOL** |
| Isolated evidence contribution to 117 | **FORBIDDEN** |
| Roadmap validation | **REMAINS APPROVED** |

This review approves the final artifact-staging correction only. It does not
execute native behavior, prescribe production remediation or broaden the
standalone launcher.

## Iteration 13 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for boundary scope | Missing or mismatched cache content fails before executable entry; `037cb…` stays zero-credit. |
| GWT / one behavior | 10/10 | The staged binary still selects only the existing fifth CP behavior. |
| Domain language | 10/10 | Source build, content-addressed cache, privileged prerequisite, body entry and canonical 117 remain distinct. |
| Coverage completeness | 10/10 for staging | Build identity, durable location, pre-exec digest, direct entry and evidence limits are all pinned. |
| Walking-skeleton user value | 10/10 | Staging preserves access to the same reviewed production-composition diagnostic body. |
| Priority validation | 10/10 | The correction addresses rsync lifetime only and adds no test or product mechanism. |
| Observable behavior | 10/10 | Exact content identity is checked before staging completion and again before execution. |
| Traceability | 10/10 | The cache correction maps directly to the retained missing-file failure. |
| Walking-skeleton boundary | 10/10 | No canonical selector, API, lane, deadline, oracle or payload changed. |

## Iteration 13 verdict

**APPROVED.** The updated source gate builds the unchanged reviewed executable
as the unprivileged Metal user, installs it outside the rsync-managed workspace
under its exact SHA-256, and validates the installed bytes. The privileged
command performs ordinary guest-kit preparation, requires that same digest and
directly executes the reviewed artifact without building the standalone crate
as root.

`037cb…` remains an honest pre-entry failure with zero credit. No native
evidence is claimed, the isolated result still cannot count toward 117, and
roadmap validation remains approved.

# APPROVED

# Review iteration 14 — isolated fifth-body runtime classification

## Iteration 14 metadata

| Field | Value |
| --- | --- |
| Scope | Actual isolated execution of the existing fifth STEP 14-01 CP body |
| Runtime receipt | `154337257e1bebd3531ce423`, exit 101 |
| Runtime source digest | `39f8917bd798c9661895aa5a436f4e657fd5f0d70f93c89135a3e7859915cf91` |
| Reviewed executable | `1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff` |
| Cleanup receipt | `407aefabec02418e0234bea6`, exit 0 |
| Body classification | **SEMANTIC RED: CLIENT OBSERVED EOF; ACCEPTED ORACLE REQUIRES RESET** |
| Iteration verdict | **EVIDENCE APPROVED; PRODUCT OUTCOME REMAINS RED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, host state or commit was changed. No native, Lima
or mutation execution was launched by the reviewer.

Roadmap validation remains `approved`. The accepted fifth-body outcome remains
`TerminalEvent::Reset`, never `Eof`. This review classifies the reproduced
mismatch and does not weaken that oracle, infer a repair mechanism or claim a
117-body result.

## Exact reviewed source identities

| Source | SHA-256 |
| --- | --- |
| Existing fifth body and private fixture support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` |
| Shared terminal parser and protocol | `395c3fc937de129f66a751edf1226be5f928f592d003fb98ab11219d2c80f4c7` |
| Guest workload program | `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec` |
| Canonical GuestKit runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| Owner snapshot diagnostics | `9eae807a7d2225435353d2ac2d80b89da0f1ac6e83f366f30d5da1ec772abd7a` |
| Host inventory diagnostics | `be7e03a2a5229d6cbd9d5427b40fc93484c58aaff43d0ad53d377a7301bf7b00` |
| Kernel inventory diagnostics | `e5cb0c94fb42ad807713f2540546d5e5774ff3e8f648b01a0c712d588d6144d4` |
| Owner runtime diagnostics | `4d8356d7c35b14a3c09c4cf6307334a75e6714f43c3936ef9a8ede6344eec9bd` |
| Server shutdown diagnostics | `2a4d10aa084478a67a18c4bd6fc97e42af6be560206d48de8d992f2902bc3107` |
| mTLS transport-death diagnostics | `e56a4a10fb6da5441dc0bac275801b7c6c9bad608ed953279179c5b8ee7fb011` |
| Isolated generator / entrypoint | `f5e87ea63a8da7d625cfe47ff042a40ca802700806c5539857b34bb34daa2555` / `9ae793c327f7b707a9a7870a0d90eb8016f7031463201a2ab1a41b726e1f42b1` |

The exact body still deploys the Service and client through production
`run_server` and public submission, waits for the client flow and held relay,
kills the actual Service VMM, parses the client application's first terminal
console line, and requires `Reset`. No runtime result depends on a new
selector, expected-value seam or diagnostic event.

## Ordered body evidence

The body entered with exact allocations
`alloc-d17-relay-svc-0` and `alloc-d17-relay-client-0`. Its output-only phase
markers establish this order:

| Timestamp | Stage | Non-vacuous result |
| --- | --- | --- |
| `16:16:26.084771` | `client_flowing` start | Existing client allocation selected |
| `16:16:26.088956` | `client_flowing` complete | Existing `hold: flowing` predicate passed |
| `16:16:26.092291` | `observe_relay_held` start | Existing owner observation began |
| `16:16:26.114906` | `observe_relay_held` complete | Existing one-Paired-Relay predicate passed |
| `16:16:26.122446` | `vmm_kill` request | Service VMM PID 248 selected |
| `16:16:26.126014` | `vmm_kill` complete | Existing `libc::kill(..., SIGKILL)` returned success |
| `16:16:26.126876` | `client_terminal` start | Existing client-console polling began |
| `16:16:26.333688` | `client_terminal` complete | Existing parser returned a terminal event |

The complete markers are emitted only after their existing call or loop has
returned. The sequence therefore proves that this execution did not stop in
client startup, relay-held observation, owner snapshot, inventory ioctl or the
kill syscall.

## Pre-kill owner and inventory completion

Between the relay-observation start and completion, the owner emitted:

1. owner-state lock begin at `16:16:26.097844`;
2. owner-state lock acquired at `16:16:26.099676`;
3. owner snapshot inventory begin at `16:16:26.101523`;
4. host forwarder-state lock begin/acquired at `16:16:26.103379` and
   `16:16:26.105116`;
5. kernel inventory ioctl begin at `16:16:26.106722`;
6. kernel inventory ioctl completion at `16:16:26.109968`; and
7. owner snapshot inventory completion at `16:16:26.111571`.

Both completion events carry the same successful typed result:

```text
Ok(ForwarderInventory {
  live_pairs: 1,
  live_watches: 1,
  queued_ends: 0,
  attached: 6,
  completed: 2,
  aborted: 6,
  unclassified_eof: 0,
  dgram_oversize: 0,
})
```

The relay-held phase then completed before the kill request. On this schedule,
the earlier outer timeout is therefore not an owner-state-lock,
host-forwarder-lock or inventory-ioctl stall.

## Actual VMM kill and transport-death observations

The kill request and completion both identify Service VMM PID 248. Production
later records that same PID reaped with `signal=Some(9)`, establishing the
actual VMM death beyond the syscall return.

After the kill, production records the Service guest-flow control session lost
and the following read-only transport snapshots:

- Service connection `alloc-d17-relay-svc-0#1` has terminal role
  `inbound_guest_relay_leg_c_to_s`. Its observed leg C remains TCP state 1,
  connected from `100.95.0.2:8080` to `127.0.0.1:39774`, with zero
  retransmits. The connection is then reported reclaimed.
- Client connection `alloc-d17-relay-client-0#0` has auxiliary terminal role
  `outbound_return_leg_b_to_f`. Leg F remains TCP state 1 and connected;
  leg B has no connected peer (`ENOTCONN`, errno 107), TCP state 7 and one
  total retransmission. That connection is then reported reclaimed.
- At `16:16:26.200262`, the production guest-flow owner records the client
  allocation's guest-originated TCP flow ending with `cause="peer_reset"`.

These records identify current production observations. The owner label
`peer_reset` does not establish that the guest application received
`ECONNRESET`; that application outcome is independently classified by the
console protocol below.

## Exact client outcome and assertion failure

The client workload's unchanged `hold` loop classifies its read side as:

- `TerminalEvent::Eof` only when `read` returns `Ok(0)`;
- `TerminalEvent::Reset` only when the read error maps to `ECONNRESET`; and
- another `Error(code)` for other read errors.

It prints exactly
`observer guest-app: terminal=<event>`. The unchanged
`terminal_event_in_console` finds the first line with that exact prefix and
parses only `eof`, `reset` or `error:<code>` into the corresponding enum.

The body completed its terminal polling 207 milliseconds after the kill and
then failed at the unchanged assertion:

```text
assertion `left == right` failed: the mesh client sees a reset, never EOF
  left: Eof
 right: Reset
```

The `left: Eof` value can reach this assertion only after
`terminal_event_in_console(&console_of(&client_alloc))` parsed the exact
existing `observer guest-app: terminal=eof` line. This is non-vacuous evidence
that the client application observed a clean read-side EOF rather than the
required `ECONNRESET`.

The complete Service and client console files are not present in the retained
outer output. The runner's error contains only the final 16-KiB outer-console
tail, and this assertion does not render `all_console_diagnostics`. The review
therefore does not claim any other private console line, byte sequence or
ordering. The typed parser result proves the exact EOF terminal line and
nothing beyond it.

## Failure boundary and evidence credit

There is no `observe_dead_launch` start/completion event and no body-level
`shutdown` start/completion event. Source order places those stages after the
Reset assertion, so the panic stopped the body before either could run.
Neither post-kill owner disappearance, inventory agreement, public stop nor
body shutdown receives credit from this execution.

The outer GuestKit peer subsequently failed to return the EXEC response within
the unchanged 600-second bound. That no-reply timeout is secondary: the body
panic and exact assertion mismatch were already present in its captured outer
console. It does not reclassify the failure as an inventory, kill or shutdown
stall.

The receipt contains no libtest result summary and no recognized runtime-SKIP
line. The isolated body receives zero pass credit. Because the isolated
launcher sits outside canonical manifest reconciliation, this failure also
supplies no partial 117 credit and cannot be combined with another receipt.

## Evidence limitation and classification boundary

The evidence proves a mismatch at the accepted observable boundary:
production records its client owner path as `peer_reset`, while the unchanged
guest application and parser report `TerminalEvent::Eof`. It does not prove
which socket operation, kernel transition, half-close, teardown action or
owner ordering caused that mismatch. The transport snapshots are point-in-time
read-only diagnostics and do not by themselves establish causality.

Accordingly, this review records a reproduced production semantic RED and
preserves the Reset oracle. It does not prescribe a socket option, shutdown
sequence, mTLS change, owner change, retry, timeout, parser change or fixture
weakening. Any production diagnosis or remedy remains separate work subject
to the repository's reachability and accepted-contract gates.

## Receipt and cleanup integrity

Runtime `154337257e1bebd3531ce423` exited 101 by `command_returned`. Its
21,804-byte output SHA-256 is
`21a1155d882faa4a7e1724ad7a5e8885c1386fed05ecab38a9b9f2ab5a7a8e1e`;
its terminal SHA-256 is
`2bcdc424b78a2b11c87ed317c3ca2ed6490e80ade7ae7188055c566aaffb1a39`.
The raw record is retained byte-for-byte under both evidence roots.

Read-only cleanup `407aefabec02418e0234bea6` exited 0. It reports zero live
Cloud Hypervisor processes, no physical-host nft table and no STEP 14-01 work
directory. The only listed namespace is the documented pre-existing
`ovd-ns-0000`. Its output SHA-256 is
`d3d97964338664b99d620c518861ba71547a397523d069727cc95f0bfca14411`.

No retry, Lima run, mutation run, DES event, commit, production edit or oracle
change followed this evidence.

## Findings and disposition

No DISTILL source or isolated-boundary finding remains. The runtime exposes a
production outcome mismatch:

| Item | Disposition |
| --- | --- |
| Client flowing and relay-held setup | **PASSED NON-VACUOUSLY** |
| Owner lock and inventory ioctl before kill | **COMPLETED SUCCESSFULLY** |
| Actual Service VMM SIGKILL | **EXECUTED; PID 248 REAPED BY SIGNAL 9** |
| Production owner terminal classification | **`peer_reset` OBSERVED** |
| Guest application terminal classification | **`Eof` OBSERVED** |
| Accepted fifth-body oracle | **RED; `Reset` REMAINS REQUIRED** |
| Later dead-launch / shutdown stages | **NOT REACHED; NO CREDIT** |
| Outer 600-second timeout | **SECONDARY TO CAPTURED BODY PANIC** |
| Complete Service/client console rendering | **ABSENT; ONLY EXACT EOF LINE IS PROVEN BY THE PARSER** |
| Runtime skip / body pass / 117 | **ZERO CREDIT** |
| Production remedy | **NOT INFERRED OR PRESCRIBED** |
| Roadmap validation | **REMAINS APPROVED** |

The semantic RED should remain visible to the production crafter for separate
root-cause work. This review supplies the reproduced owner path and exact
observable mismatch without selecting a remedy.

## Iteration 14 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The test fails on clean EOF and preserves the required Reset outcome. |
| GWT / one behavior | 10/10 | One existing VMM-kill behavior executed through the exact production path. |
| Domain language | 10/10 | Owner `peer_reset`, guest read-side EOF, outer EXEC and later shutdown are kept distinct. |
| Coverage completeness | 10/10 for reached prefix | Every reached phase is ordered; later phases are explicitly denied credit. |
| Walking-skeleton user value | 10/10 | Real Service/client VMs, held relay, actual SIGKILL and the guest application terminal line determine the result. |
| Priority validation | 10/10 | Evidence classifies the reproduced mismatch without weakening the oracle or inventing architecture. |
| Observable behavior | 10/10 | The typed parser and exact assertion establish EOF at the guest application boundary. |
| Traceability | 10/10 | Phase markers, owner/inventory records, transport snapshots, parser source and assertion align. |
| Walking-skeleton boundary | 10/10 | No test seam, expected-value injection or alternate product path supplies the failure. |

## Iteration 14 verdict

**EVIDENCE APPROVED; PRODUCT OUTCOME REMAINS SEMANTIC RED.** The isolated
existing body reached a held relay, completed the pre-kill owner snapshot and
kernel inventory ioctl, killed the actual Service VMM, and observed the
client's terminal event. Production labeled the owner-side end `peer_reset`,
but the unchanged guest application and parser returned `TerminalEvent::Eof`.
The exact accepted assertion correctly failed because `Reset` remains
required.

The later dead-launch and shutdown stages were not reached, the outer timeout
is secondary to the captured body panic, full private consoles were not
rendered, and the run earns zero pass or 117 credit. The evidence does not
identify a production remedy. Roadmap validation remains approved.

# SEMANTIC RED CONFIRMED

# Review iteration 15 — panic/timeout allocation-console capture

## Iteration 15 metadata

| Field | Value |
| --- | --- |
| Scope | Private panic/timeout console-capture correction for existing CP bodies |
| Reviewed control-plane integration source | `c19a6351570d768174b297b70b7e5fd22a2bcfe91e1698fd78c31e454583abb8` |
| Scoped source gate | `20085833257fec7f391c461f`, exit 0 |
| Scoped source digest | `a272c673938a6485c2bdd4ae9316d1e3ccf11b1f1d242d6db7bfba5bce586785` |
| Native execution | None |
| Iteration verdict | **NEEDS PRIVATE DISTILL REMEDIATION** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, host state or commit was changed. No native, Lima
or mutation execution was launched.

Roadmap validation remains `approved`. Iteration 14's reproduced semantic RED
also remains unchanged: the client observed EOF while Reset remains required.
This review evaluates only whether the private capture can preserve complete,
allocation-correlated failure evidence on a later execution.

## Prior evidence and correction scope

Isolated receipt `cb98290768fcb273c3bd4525` exited 101 and retained the same
body-level semantic RED, but its final outer-console tail contained only the
Service allocation's guest console. That console showed:

- `listen: conn=1 hello=no terminal=reset`;
- a `forward-end` for `ForwardId(1)` with
  `Aborted { cause: PeerAbort { side: A } }`;
- the corresponding `handle-end`; and
- deferred close with `clear_linger=false`.

It did not render the client allocation's console, exact EOF line, client-side
ForwardId/end/close records or an allocation-correlated relationship between
the Service's `ForwardId(1)` and the held relay. A Service allocation serves
both readiness probes and relay connections, so that lone ForwardId cannot be
credited as the relay without the missing allocation and connection evidence.
The prior receipt therefore remains an evidence limitation, not a root-cause
finding.

The reported cleanup `5561c7b2e0bd3f01922b9f020` is not present in the
current retained `.context` raw tree, so this review grants it no independent
receipt credit. This does not affect the source finding below. No native
credit is claimed for the current correction.

## Capture lifetime and read-only ownership

`ConsoleCapture::start` runs inside `Node::boot` after the production server
and workload image are prepared but before `Node` is returned. Every body
submits its first Service or Job only after that return, so the capture thread
exists before any allocation submission (`guest_flow_through_serve.rs:294-339`).

The thread discovers exact directory names under the existing
`/run/overdrive/vm` root and opens each allocation's existing `console.log`
read-only. It stores one `File` under that exact allocation identifier and
retains the descriptor until capture teardown (`guest_flow_through_serve.rs:153-165`,
`231-260`). On Linux, unlinking `VmRunDir` removes the pathname but does not
invalidate an already-open file description. The correction therefore
survives accepted run-directory removal without keeping a production VMM,
TCP socket or application handle alive.

Each file cursor begins at byte zero. Repeated `read_to_end` calls consume only
newly appended bytes, extend the retained byte vector under the same exact
allocation identifier, and maintain a separate pending line fragment per
allocation (`guest_flow_through_serve.rs:167-198`). On stop, the thread calls
the same capture function once more before closing the descriptors. This is a
correct final drain of bytes visible at that point; no byte is duplicated
because each file cursor advances monotonically.

The capture owns only read-only console-file descriptors and in-process test
memory. It adds no production socket reference, persistence, hook, API,
callback or cleanup dependency.

## Allocation correlation and retained evidence

Every streamed line is prefixed with
`STEP14-01-GUEST-CONSOLE allocation=<exact allocation id>`. Panic rendering
iterates the same `BTreeMap` and labels each allocation section and each
matched line with that identifier. This prevents a client allocation record
from being silently substituted for a Service allocation record or vice
versa.

Within a Service allocation, probe and relay flows still share the same
allocation. The capture honestly retains all matching `listen:` and
`overdrive-init: guest-flow` lines; later classification must still use the
reported ForwardId and `hello=yes|no`/other connection evidence. The new
labels preserve the information needed to reject probe substitution, but an
allocation label alone does not prove which Service ForwardId is the relay.
No source assertion treats it as proof.

`panic_console_diagnostics` also appends a separately headed final-byte tail
bounded to 2 KiB per allocation. The raw tail may contain arbitrary or partial
console data, but its heading distinguishes it from the purported complete
control/end-line section.

## Live-path semantics and test-body audit

`console_of` remains live-path-first. It reads the production
`VmRunDir::console_log()` pathname on every call and uses the complete retained
byte stream only if the live read is absent or empty
(`guest_flow_through_serve.rs:550-565`). The capture's streamed and panic
output is never parsed by `console_of`, `wait_flowing`,
`terminal_event_in_console`, an owner observation or any assertion.

The current diff adds imports and private capture/formatting helpers, starts
the private guard and retains the previously reviewed diagnostics. The five CP
body identities, declarations, stimuli, selectors, predicates, assertions,
120/30-second deadlines, real VMM kill, production `run_server` composition,
public submission and cleanup order remain unchanged. The canonical runner is
still
`45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9`;
the isolated launcher, lane and 117 accounting are unchanged.

## Finding F-03 — Medium: teardown emits an unterminated fragment as a complete line

**Finding owner: DISTILL test-support author.**

The ordinary streaming path correctly emits only newline-terminated data:
`emit_complete_console_lines` finds the last `\n`, drains through it and
leaves every later byte in the per-allocation `pending` vector. The vector
therefore contains no complete line after each call.

At capture-thread teardown, however, the source iterates that remaining
`pending` vector and emits it with the same
`STEP14-01-GUEST-CONSOLE allocation=...` format whenever the fragment contains
a matching substring (`guest_flow_through_serve.rs:250-256`). By construction,
that byte sequence is unterminated. A stop that races a console write can
therefore label a partial `overdrive-init: guest-flow`, `observer guest-app:`,
`listen:` or `hold:` record as though it were one of the complete streamed
control/end lines.

The panic renderer has the same bounded honesty defect. It converts the full
retained byte stream with `String::lines()`. Rust treats a nonempty final
unterminated fragment as a line, so the `control/end lines` section can repeat
that partial fragment as a matched line (`guest_flow_through_serve.rs:123-140`).
The separately labeled 2-KiB tail is the correct place to preserve such a
fragment; it should not also receive complete-line status.

This is reachable whenever the final drain observes a writer between the
matched prefix and its terminating newline. It does not change the test result,
but it can make the diagnostic overstate exactly the control/end record it was
introduced to classify. The source therefore does not yet satisfy the stated
complete-line-only evidence contract.

### Exact bounded remediation

Keep the full retained bytes and bounded tail unchanged. Correct only the two
complete-line renderers:

1. Remove the teardown loop that emits the remaining `pending` fragment as
   `STEP14-01-GUEST-CONSOLE`. The final `capture_console_output` call has
   already emitted every newly completed line. Preserve the remaining bytes
   in `retained_consoles`; the panic tail will show them as raw partial data.
2. In `panic_console_diagnostics`, restrict the matched-line scan to the byte
   prefix through the final newline. If no newline exists, the complete-line
   section is empty. Continue rendering the separately headed final 2-KiB tail
   from the full bytes.

No body, assertion, timeout, production code, runner, lane or new test is
needed. This is a private source-only DISTILL correction and should pass the
same scoped format/check/Clippy gate before any native reproduction.

## Source-gate evidence and `bce…` disposition

Scoped Metal gate `20085833257fec7f391c461f` used source digest
`a272c673938a6485c2bdd4ae9316d1e3ccf11b1f1d242d6db7bfba5bce586785`
and exited 0. It checked formatting for the control-plane package, checked the
exact integration target with `integration-tests,kvm-tests`, and ran scoped
Clippy with dependencies excluded and warnings denied. Its output SHA-256 is
`2bc11d3856f36e910a5f1387eef238c3443a06680c6ec4b6fd61d884eebd9d01`.
This proves the reviewed source compiles cleanly; it does not close F-03's
semantic evidence-labeling defect.

Workspace-wide attempt `bce4777e8fd142b00b57206c` exited 1 and ran no test
or VM. Its retained output is not accurately described as production-only: it
shows rustfmt-only layout differences in both this fixture source and
crafter-owned mTLS diagnostic files. The current fixture has those neutral
rustfmt changes, while DISTILL made no production edit. The unrelated
production formatting remains outside this fixture review; the scoped exit-0
gate is the controlling compiler/lint evidence.

## Findings and remediation disposition

| Item | Disposition |
| --- | --- |
| Capture starts before allocation submissions | **APPROVED** |
| Read-only handles survive run-directory unlink | **APPROVED** |
| Incremental complete-byte retention and final descriptor drain | **APPROVED** |
| Newline-terminated streaming during the capture loop | **APPROVED** |
| Unterminated pending fragment at teardown | **F-03 OPEN** |
| Panic matched-line rendering of final unterminated bytes | **F-03 OPEN** |
| Exact allocation correlation | **APPROVED; SAME-SERVICE PROBE/RELAY STILL REQUIRES FORWARDID/HELLO EVIDENCE** |
| Bounded per-allocation final-byte tail | **APPROVED** |
| Live-first `console_of` | **UNCHANGED AND APPROVED** |
| Production hooks/sockets/persistence/API | **NONE ADDED** |
| Bodies, assertions, deadlines, stimuli, identities and 117 | **UNCHANGED** |
| Iteration-14 EOF-vs-Reset semantic RED | **UNCHANGED** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 15 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 | Panic and timeout paths are covered, but a teardown fragment can be overstated as complete. |
| GWT / one behavior | 10/10 | Existing CP behaviors and identities remain unchanged. |
| Domain language | 10/10 | Allocation, ForwardId, probe/relay and raw tail remain distinct in the review. |
| Coverage completeness | 8/10 current / 10/10 prescribed | Every allocation is retained, but complete-versus-partial line status is not exact at teardown. |
| Walking-skeleton user value | 10/10 | Capture observes the existing real VM consoles without changing the production path. |
| Priority validation | 10/10 | The correction is private diagnostics only and preserves the Reset oracle. |
| Observable behavior | 8/10 current / 10/10 prescribed | Full bytes are honest; one partial fragment can be mislabeled in the matched-line view. |
| Traceability | 10/10 | The source defect maps directly to the claimed complete-line evidence boundary. |
| Walking-skeleton boundary | 10/10 | No product hook, alternate fixture, selector or scenario was introduced. |

## Iteration 15 verdict

**NEEDS PRIVATE DISTILL REMEDIATION.** The capture starts before submissions,
retains read-only per-allocation console handles across unlink, incrementally
preserves full bytes, performs a final drain, labels exact allocations,
renders bounded tails and leaves every body and production boundary unchanged.

It does not yet emit only complete control/end lines: the teardown path and
panic matched-line scan can label a final unterminated fragment as complete.
Close F-03 with the exact source-only correction above before authorizing a
native reproduction. Iteration 14's semantic RED and Reset oracle remain
unchanged; no production remedy is prescribed.

# NEEDS PRIVATE DISTILL REMEDIATION

# Review iteration 16 — F-03 complete-line remediation

## Iteration 16 metadata

| Field | Value |
| --- | --- |
| Scope | Re-review of F-03 only |
| Corrected control-plane integration source | `fc49db5d5c0e2e70cdfe07565db9137faeb39a0acf82c81228854c8c3a1a4743` |
| Scoped source gate | `8f7649f923141f24a5503cd8`, exit 0 |
| Scoped source digest | `0c249fa6c5f77e751a573f3eef8814f2f853fcf92520c93cbf15f7edef30cf6c` |
| Native execution | None |
| Finding disposition | **F-03 CLOSED** |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, host state or commit was changed. No native, Lima
or mutation execution was launched.

Roadmap validation remains `approved`. Iteration 14's reproduced semantic RED
also remains unchanged: the client observed EOF while Reset remains required.
This iteration closes only the private diagnostic evidence-labeling defect.

## Exact F-03 remediation

### Teardown pending fragment

The capture thread still performs the same final
`capture_console_output(&mut files, &mut pending)` after its stop flag is set.
That call reads every byte visible through each retained descriptor, extends
the complete retained byte stream and routes every newly newline-terminated
prefix through `emit_complete_console_lines`.

After that final drain, the thread now exits directly
(`guest_flow_through_serve.rs:238-253`). The former loop over `pending` is
absent. Because `emit_complete_console_lines` drains through the final newline,
each remaining pending vector contains only the unterminated suffix, if any.
No such suffix is emitted with the
`STEP14-01-GUEST-CONSOLE allocation=...` complete-line format.

This is the exact iteration-15 remediation. It removes no complete line:
anything terminated before or during the final drain has already been emitted,
while an unterminated fragment is not a complete control/end record.

### Panic matched-line prefix

`panic_console_diagnostics` now computes `complete_at` as one byte past the
last newline in the full retained byte vector, or zero if the vector contains
no newline. It applies `String::lines()` and the control/end matcher only to
`&bytes[..complete_at]` (`guest_flow_through_serve.rs:123-142`).

Therefore:

- every matched line in the `control/end lines` section is backed by a
  newline-terminated retained record;
- a retained stream with no newline contributes no complete matched line; and
- the final unterminated suffix cannot be repeated under complete-line
  labeling.

F-03's two source paths are both closed.

## Full-byte and raw-tail preservation

The retained byte vectors are unchanged. `capture_console_output` still
extends them with every newly read byte before line rendering, and the final
descriptor drain still uses the same function. The correction discards no raw
console byte from retained test memory.

Panic output still computes the separately headed
`--- retained allocation=<id> final-byte-tail ---` from the full byte vector,
not the newline-truncated prefix. It remains bounded to the final 2 KiB per
allocation. A final partial fragment is therefore preserved as raw tail data
under an exact allocation heading, while no longer being represented as a
complete control/end line.

## Preserved capture and acceptance boundaries

All iteration-15 approved properties remain intact:

- `ConsoleCapture` starts in `Node::boot` before any Service or Job submission;
- exact per-allocation read-only `console.log` handles remain open across
  accepted `VmRunDir` unlink;
- file cursors incrementally retain appended bytes without duplication;
- the stop path joins the watcher after one final descriptor drain;
- every streamed complete line and panic section retains the exact allocation
  identifier;
- same-Service probe and relay evidence still requires ForwardId and
  `hello=yes|no`/connection correlation;
- `console_of` remains live-path-first and falls back to the complete retained
  bytes only when the live file is absent or empty; and
- the capture owns no production socket, persistence, hook, API or lifecycle
  resource.

The correction changes no test body, `TerminalEvent::Reset` assertion,
predicate, 120/30-second deadline, stimulus, VMM kill, workload identity,
runner manifest, isolated launcher, lane, public API, production source or
117 accounting. The phase and production diagnostics reviewed in earlier
iterations are unchanged.

## Source-gate evidence

Scoped Metal gate `8f7649f923141f24a5503cd8` used workspace digest
`0c249fa6c5f77e751a573f3eef8814f2f853fcf92520c93cbf15f7edef30cf6c`
and exited 0. It verified control-plane formatting, checked the exact
integration binary with `integration-tests,kvm-tests`, and ran scoped Clippy
with dependencies excluded and warnings denied. Its output SHA-256 is
`db0a67d3e4a5f4d9b069f8429e1449e20921f18f9004f9fe5c5d372ac6c1f619`.
The terminal declares `kind=command`, `exit_code=0` and
`cause=command_returned`.

No test or VM ran in that source gate. The gate supplies compile/lint evidence
only and grants no behavioral, pass or 117 credit.

## Finding disposition

| Item | Disposition |
| --- | --- |
| F-03 teardown pending-fragment emission | **CLOSED** |
| F-03 panic complete-line scan | **CLOSED** |
| Full retained bytes | **PRESERVED** |
| Separately headed 2-KiB raw tail | **PRESERVED** |
| Capture lifecycle and allocation correlation | **UNCHANGED AND APPROVED** |
| Live-first `console_of` | **UNCHANGED AND APPROVED** |
| Bodies, assertions, predicates, deadlines, stimuli, identities and 117 | **UNCHANGED** |
| Production hooks/sockets/persistence/API | **NONE ADDED** |
| Iteration-14 EOF-vs-Reset semantic RED | **UNCHANGED; RESET REMAINS REQUIRED** |
| Roadmap validation | **REMAINS APPROVED** |

No private fixture finding remains. The corrected capture is suitable for a
separately authorized matching-source isolated reproduction. This review does
not perform native execution or prescribe a production remedy.

## Iteration 16 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for diagnostic scope | Panic and timeout evidence remain fail-closed without overstating a partial line. |
| GWT / one behavior | 10/10 | Existing CP behaviors and identities remain unchanged. |
| Domain language | 10/10 | Complete line, raw tail, allocation, ForwardId and probe/relay evidence remain distinct. |
| Coverage completeness | 10/10 | Both F-03 render paths are corrected while full bytes and tails remain available. |
| Walking-skeleton user value | 10/10 | Capture still observes the existing real VM consoles without changing the production path. |
| Priority validation | 10/10 | The remediation changes only private evidence formatting and preserves the Reset oracle. |
| Observable behavior | 10/10 | Only newline-terminated records receive complete-line status. |
| Traceability | 10/10 | Each iteration-15 prescription maps directly to current source. |
| Walking-skeleton boundary | 10/10 | No product hook, alternate fixture, selector, lane or scenario was introduced. |

## Iteration 16 verdict

**APPROVED.** F-03 is closed. Teardown no longer emits pending unterminated
bytes as complete guest-console lines, and panic matched-line rendering is
restricted through the final newline. Complete retained bytes and the
separately headed 2-KiB raw tail continue to preserve any partial suffix.

All previously approved capture lifecycle, allocation correlation, live-first
lookup and acceptance boundaries remain unchanged. Iteration 14's semantic RED
and Reset oracle remain intact, no native evidence is claimed, and roadmap
validation remains approved.

# APPROVED

# Review iteration 17 — correlated Service/client runtime evidence

## Iteration 17 metadata

| Field | Value |
| --- | --- |
| Scope | Final review of allocation-correlated isolated fifth-body evidence |
| Runtime receipt | `6d39b863859629e23a31ec86`, exit 101 |
| Runtime source digest | `1c03cb00b3b176ccc6e61fe16d2dc380e569f5c97d6f9f8dd8644aeadce707db` |
| Capture source | `fc49db5d5c0e2e70cdfe07565db9137faeb39a0acf82c81228854c8c3a1a4743` |
| Cleanup receipt | `6c1c6640f48c80481c4ca359`, exit 0 |
| Body classification | **SEMANTIC RED: CLIENT PID-1 OWNER CONSUMED SIDE-B PEER ABORT; APPLICATION OBSERVED EOF; RESET REQUIRED** |
| Iteration verdict | **EVIDENCE APPROVED; PRODUCT OUTCOME REMAINS RED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, host state or commit was changed. No native, Lima
or mutation execution was launched by the reviewer.

Roadmap validation remains `approved`. The accepted fifth-body outcome remains
`TerminalEvent::Reset`, never `Eof`. This review accepts the correlated
execution evidence without weakening that oracle or selecting a production
remedy.

## Source and observation boundaries

The corrected private capture is the iteration-16-approved source. It starts
before workload submission, retains read-only handles to each exact
allocation's `console.log`, emits only newline-terminated matched lines,
performs a final drain, and renders full retained bytes plus a separately
headed 2-KiB tail on assertion unwind. `console_of` remains live-first. F-03's
complete-versus-partial distinction is unchanged.

The workload image uses `GuestImageSpec::workload`, which stages production
`overdrive-init` as PID 1. The VM workload command remains the ordinary
`/sbin/overdrive-guest-peer` application. Consequently:

- lines prefixed `overdrive-init: guest-flow` are production PID-1 guest-owner
  observations; and
- `hold:` / `observer guest-app:` lines are the ordinary workload
  application's observations.

The guest application maps a zero-byte read to `TerminalEvent::Eof`, maps
`ECONNRESET` to `TerminalEvent::Reset`, and prints the exact terminal protocol
line. The production PID-1 owner prints a forward-end when it receives the
kernel record, then prints the handle-end state and the deferred close it
applies. These are independent boundaries in the same exact allocation
console.

## Client allocation correlation

The panic guard renders this complete, newline-bounded client section:

```text
--- retained allocation=alloc-d17-relay-client-0 bytes=28685 control/end lines ---
allocation=alloc-d17-relay-client-0 hold: flowing
allocation=alloc-d17-relay-client-0 observer guest-app: terminal=eof
allocation=alloc-d17-relay-client-0 overdrive-init: guest-flow forward-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: B } }
allocation=alloc-d17-relay-client-0 overdrive-init: guest-flow handle-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: B } } tcp_pairs=1 pending_end_closes=0
allocation=alloc-d17-relay-client-0 overdrive-init: guest-flow deferred-close stream id=ForwardId(1) clear_linger=false outer=GuestPairSocket(Handle { seam: SeamInstance(1), id: SocketId(3) }) vsock=GuestPairSocket(Handle { seam: SeamInstance(1), id: SocketId(2) })
```

This proves, in console order and under one exact client allocation:

1. the ordinary `hold` application moved bytes in both directions;
2. its read side later returned zero and printed terminal EOF;
3. production PID 1 consumed a kernel forward-end for `ForwardId(1)` as
   `Aborted { PeerAbort { side: B } }`;
4. PID 1 handled that same `ForwardId(1)` while exactly one TCP pair existed
   and no end-close was already pending; and
5. PID 1 deferred the same pair's close with `clear_linger=false` and then
   closed the named outer/vsock handles through the existing path.

The client Service is a single `hold` Job allocation in this body. The same
allocation identifier, one live TCP pair and repeated `ForwardId(1)` therefore
provide direct non-vacuous correlation to the held guest-originated flow. This
is stronger than the earlier host-only `peer_reset` label: it reaches the
production guest PID-1 owner and the workload application's own read result.

The PID-1 trace does not turn EOF into Reset. It shows an owner-side side-B
PeerAbort followed by a non-linger-clearing close, while the application line
already records EOF. That mismatch remains the semantic RED; its cause is not
selected here.

## Service allocation evidence and probe honesty

The Service section is separately labeled
`alloc-d17-relay-svc-0`. It includes the listener and two application terminal
records, both explicitly `hello=no terminal=reset`. In the existing workload
protocol, `hello=no` identifies readiness-probe connections; the held relay
client sends the `hold` hello and would be `hello=yes` if the Service
application emitted its terminal record.

The section also contains PID-1 owner records for Service `ForwardId(1)` and
`ForwardId(2)`, each with side-A PeerAbort, zero pending end closes and
`clear_linger=false`. The serial writers interleave the second application's
line with its forward-end write:

```text
overdrive-init: guest-flow forward-end id=listen: conn=2 hello=no terminal=reset
ForwardId(2) outcome=Aborted { cause: PeerAbort { side: A } }
```

The raw 2-KiB tail preserves those exact bytes under the Service allocation
heading. The matched-line renderer honestly reproduces the newline-terminated
serial record; it does not repair the interleaving or invent a ForwardId.

Because both visible Service application ends are `hello=no` and the serial
write is interleaved, neither Service ForwardId is credited as the held relay.
The Service VMM was killed before the held `hello=yes` connection could emit a
Service application terminal record. All held-flow correlation in this review
comes from the exact client allocation above.

This distinction prevents the earlier probe/ForwardId substitution. The
Service evidence remains useful probe/owner context, while the client evidence
pins the actual held guest-originated flow.

## F-03 capture integrity

Both exact allocation sections are present in the assertion-panic output.
Their `control/end lines` entries are drawn only from retained bytes through
the final newline. The two separately headed `final-byte-tail` sections use
the full retained bytes and remain bounded to 2 KiB each.

No teardown-pending fragment is emitted with the complete-line prefix. The
Service's interleaved record is not a partial-tail labeling error: it is an
actual newline-terminated serial byte sequence produced by concurrent writers,
and the review treats it as interleaved rather than as a valid ForwardId.
F-03 therefore remains closed and the capture output is honest about its
limits.

## Host/body outcome and retained oracle

The host-side evidence remains consistent with iteration 14: the actual
Service VMM PID 248 was killed and reaped by signal 9, the client mTLS leg B
was unconnected/closed, and the host guest-flow owner recorded the client flow
end as `peer_reset`. The exact body then parsed the client application EOF and
failed the unchanged assertion:

```text
assertion `left == right` failed: the mesh client sees a reset, never EOF
  left: Eof
 right: Reset
```

The Reset oracle remains correct and unchanged. The new guest evidence narrows
the observed path through PID 1; it does not justify accepting EOF, changing
the parser, clearing or retaining linger differently, or selecting any socket,
owner, mTLS or shutdown remedy.

The outer GuestKit peer later failed to return its EXEC response within the
unchanged 600-second bound. That timeout is secondary to the captured body
panic and provides no additional body outcome.

## Pass, skip and 117 accounting

The raw output contains no libtest pass summary and no recognized runtime-SKIP
diagnostic. The body failed its existing Reset assertion and receives zero
pass credit. The isolated launcher remains outside canonical manifest
reconciliation, so this receipt contributes zero to 117 and cannot be combined
with any partial full-runner receipt.

No later success-path dead-launch observation, public stop or shutdown credit
is inferred after the assertion panic.

## Receipt and cleanup integrity

Runtime `6d39b863859629e23a31ec86` exited 101 by `command_returned`. Its
21,862-byte output SHA-256 is
`e3b525cfe98ab2c67f95534175276a6b9056dea8a232b4d1fe027e75b5066bc7`;
its terminal SHA-256 is
`b09c33784b17e3d8f42773655e07bbc82dedf4cca13d9201eb7bac4d7254aa9d`.

Read-only cleanup `6c1c6640f48c80481c4ca359` used the same source digest
and exited 0. It reports zero live Cloud Hypervisor processes, no
physical-host nft table and no STEP 14-01 work directory. The only listed
namespace is the documented pre-existing `ovd-ns-0000`. Its output SHA-256 is
`f85b5c911eae9411e67647c727f81af2445a912866c8d34f0d96b97bf25772f0`.

No retry, production edit, Lima run, mutation run, DES event or commit followed
this evidence.

## Findings and disposition

No DISTILL source or capture finding remains. The reproduced product outcome
mismatch remains visible:

| Item | Disposition |
| --- | --- |
| Both exact allocation consoles on panic | **CAPTURED** |
| F-03 complete-versus-partial labeling | **HONEST; REMAINS CLOSED** |
| Client `hold: flowing` | **OBSERVED** |
| Client application terminal | **`Eof` OBSERVED** |
| Client PID-1 owner end | **`ForwardId(1)`, SIDE-B `PeerAbort` OBSERVED** |
| Client handle state | **ONE TCP PAIR, ZERO PENDING END CLOSES** |
| Client deferred close | **`clear_linger=false` OBSERVED** |
| Service `hello=no` ends | **PROBE EVIDENCE ONLY** |
| Service serial-interleaved ForwardId | **NOT CREDITED AS HELD RELAY** |
| Accepted fifth-body oracle | **RED; `Reset` REMAINS REQUIRED** |
| Runtime skip / body pass / 117 | **ZERO CREDIT** |
| Production remedy | **NOT INFERRED OR PRESCRIBED** |
| Roadmap validation | **REMAINS APPROVED** |

The evidence is sufficient to hand the reproduced mismatch to separate
production diagnosis. This review does not expand that diagnosis or prescribe
its result.

## Iteration 17 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The unchanged Reset assertion fails on the captured EOF and earns no pass credit. |
| GWT / one behavior | 10/10 | One existing VMM-kill behavior runs through the production host and guest owners. |
| Domain language | 10/10 | Client held flow, Service probes, PID-1 owner ends and application EOF remain distinct. |
| Coverage completeness | 10/10 for reached failure | Both allocation consoles, owner records, application terminal and capture limits are present. |
| Walking-skeleton user value | 10/10 | Real workload VMs, production init, real SIGKILL and workload application output determine the result. |
| Priority validation | 10/10 | Probe ForwardIds are denied relay credit and no remedy is invented. |
| Observable behavior | 10/10 | Exact allocation labels and repeated client ForwardId correlate the held-flow mismatch. |
| Traceability | 10/10 | Capture source, PID-1 owner definitions, guest protocol and raw receipt align. |
| Walking-skeleton boundary | 10/10 | No test seam, expected-value injection or alternate production path supplies the evidence. |

## Iteration 17 verdict

**EVIDENCE APPROVED; PRODUCT OUTCOME REMAINS SEMANTIC RED.** The corrected
panic capture preserves both exact allocation consoles. The client allocation
correlates successful flow progress, application EOF, PID-1 side-B PeerAbort,
matching `ForwardId(1)` handle-end, zero pending end closes and
`clear_linger=false`. The Service's visible `hello=no` ends remain probe
records, and its serial-interleaved ForwardId receives no held-relay credit.

The Reset oracle remains required. The run earns zero pass, runtime-skip or
117 credit, cleanup is complete, F-03 remains closed, and no production remedy
is inferred. Roadmap validation remains approved.

# CORRELATED SEMANTIC RED CONFIRMED

# Review iteration 18 — U-D17 probe-watch sampling boundary

## Iteration 18 metadata

| Field | Value |
| --- | --- |
| Scope | Ownership of the new canonical U-D17 watch-agreement failure |
| Canonical receipt | `c325287a325d309b8231305c`, exit 101 |
| Receipt source digest | `00732f1c6e8f582b71789c3b1bb72d4601cc8b9d2b38e7ba89d087ba242e793c` |
| Receipt output SHA-256 | `8e8b0443843fb0025cf59d5c2252891d9f58ddc7b260cc963e934864872e8ed2` |
| Classification | **PRIVATE DISTILL POLLING PRECONDITION IS UNDER-CONSTRAINED** |
| Production finding | **NOT ESTABLISHED** |
| Iteration verdict | **NEEDS PRIVATE DISTILL REMEDIATION** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, acceptance body, DESIGN/DISTILL handoff, roadmap, DES event,
receipt, host state or commit was changed. No native, Lima or mutation
execution was launched.

Roadmap validation remains `approved`. The original crafter's outbound reset
correction and all other dirty work remain untouched. The supplied exact-fifth
receipt now passes the previously RED Reset-vs-EOF body, so iteration 17's
reproduced mismatch is historical evidence rather than the current canonical
failure. The complete 117-body result remains unestablished.

## Accepted contract boundary

R5-57 requires the real forwarder's inventory to count flows that are held and
to return their slots after the corresponding completed end/drop step:

> “with N flows held on the node … `inventory().live_pairs` and
> `live_watches` count every pair and watch those flows hold; each flow's end
> … a dropped `GuestStream` … returns them to the count before it once its end
> record is read … The same stimulus sequence … yields the same counts after
> every step.”

The public `GuestStream` contract is equally explicit about effect completion:

> “Dropping it aborts the watch, then closes `V_h` … in one step. The node
> flow slot … [is] released at the drop or the owner's abort, whichever comes
> first.”

For a probe, the dial contract says it takes a node slot while held and its
drop releases that slot. The probe runner's TCP-connect outcome is healthy
when `dial` returns `Ok`; it then drops that stream at once.

The QPROJ-3 owner observation is callable at any time and reports an owned
copy. Its edge case says a *dropped* `GuestStream` is absent. Its one-way
ForwardId invariant says every ForwardId still present in a phase names a
current pair/watch. Neither rule requires an independently executing caller
to see the completed post-drop state while another thread is still inside the
stream's `Drop` implementation.

The accepted outcomes therefore remain exact:

- while a Paired probe stream is held, the owner has a Paired dial and the
  forwarder inventory has one matching live watch;
- after the stream's drop has completed, both are absent; and
- a persistent disagreement is a failure.

They do not authorize treating an arbitrary observation taken midway through
that concurrent synchronous drop as the post-drop result.

## Current fixture path

The U-D17 body starts a Service with a one-second TCP readiness probe. Its
private `sample_agreement` loop runs for a bounded window, every 100 ms:

```rust
let obs = node.observe().await;
assert_inventory_agrees(&obs);
```

`assert_inventory_agrees` immediately requires:

```text
inventory.live_pairs   == count(Paired guest flows)
inventory.live_watches == count(Paired host dials)
```

and then checks the unchanged per-CID quota equation. The loop has no
precondition that the concurrent probe's `GuestStream` construction or drop
has completed before classifying that snapshot as an R5-57 step boundary.

The body comment and the historical S-19 fixture choice describe “agreement on
every 100 ms snapshot” because probe dials are short. That sampling cadence is
private DISTILL HOW. R5-57's normative wording is agreement after each
completed stimulus step and while a flow is held; it does not make every
mid-operation concurrent snapshot a new linearization contract.

## Exact production owner/drop path

A successful probe dial has these existing stages:

1. The owner inserts `DialPhase::AwaitingReply`.
2. `run_dial` creates a kernel watch and records it in `dial_watches`.
3. After the guest's `Paired` reply, the owner changes the dial to
   `DialPhase::Paired` and returns an `OwnerObservedStream` containing the real
   `HostGuestStream`.
4. The TCP probe reports success and immediately drops that returned stream.
5. `OwnerObservedStream::drop` removes the owner dial and owner
   `dial_watches` entry, then returns from its own Drop body.
6. Rust then drops its `stream` field. The inner `HostGuestStream::drop`
   synchronously calls the real forwarder's `abort(watch)`, removes its host
   watch entry and shuts down `V_h`.

The public `GuestStream` drop is complete only after step 6. A concurrent
observation can acquire the owner-state lock after step 5 while the same Drop
is waiting to perform or complete step 6. Such an observation legitimately
sees the owner entry absent and the kernel inventory watch still live; no
caller has yet observed the public Drop return.

This is the real current caller/owner path. No alternate seam or hypothetical
schedule is needed to explain the native snapshot.

## Decisive receipt chronology

The canonical receipt contains 102 successful libtest summaries totaling 112
passed, zero failed and zero ignored: the retained 111 bodies plus the CLI
body. It contains no recognized runtime-SKIP diagnostic and no control-plane
pass result, so it supplies no 117 credit.

For Service allocation `alloc-d17-dial-svc-0`, the failing probe has this
ordered trace:

| Timestamp | Event |
| --- | --- |
| `19:10:32.624073` | Probe `flow=2147483649` returns `Ok(GuestStream)` |
| `19:10:32.625658` | Concurrent owner observation starts acquiring owner state |
| `19:10:32.630173` | `OwnerObservedStream::drop` reports `stream_dropped` after removing its owner dial/watch entries |
| `19:10:32.632931` | Observation acquires owner state |
| `19:10:32.635721` | Owner snapshot begins its forwarder inventory read |
| `19:10:32.648545` | Kernel inventory returns `live_watches: 1` |
| `19:10:32.656591` | Owner snapshot completes with that inventory |

The same snapshot's CID row is Active but has `dials: {}` and
`held_flows: 0`. Thus `paired_dials(obs) == 0` while
`inventory.live_watches == 1`, and line 802 fails with left 1, right 0.

This chronology is direct evidence of a sample taken after the outer owner
observation was removed but before the inner kernel-watch abort had completed.
It is not evidence of a watch remaining after the public stream Drop returned.
The receipt contains no later stable snapshot proving a persistent mismatch.

## Production-finding disposition

The native kernel inventory effect is classified precisely: one real kernel
watch remained live during the incomplete drop. The receipt proves neither a
leak nor a post-completion owner invariant violation.

Promoting this to a control-plane ordering defect would require the repository's
seeded `overdrive-sim` invariant to reproduce a disagreement after the drop
step has completed, with its seed retained. No such failing invariant is
provided. A new API, observation linearization protocol, owner lock coupling or
forwarder seam is therefore neither necessary nor authorized.

The current evidence instead falsifies the fixture assumption that every
100-ms sample lands only between completed probe operations.

## Finding F-04 — Medium: `sample_agreement` asserts before operation completion

**Finding owner: DISTILL test-support author.**

`sample_agreement` accepts every raw owner observation as an R5-57 assertion
point. It needs to poll to the same required complete-state relation before
running the existing exact assertions. This is a private precondition defect;
it does not change the paired-dial outcome.

### Exact bounded remediation

Keep the same body, Service, probe interval, 5/3/6-second sample windows,
100-ms cadence, `SETTLE_BOUND`, identities and every existing assertion.
Change only the private observation acquisition used by `sample_agreement`:

1. Use the existing `Node::wait_observation` and its existing
   `SETTLE_BOUND` to wait until `forwarder_inventory` is `Read` and both exact
   equalities hold:

   ```text
   inventory.live_pairs   == paired_guest_flows(observation)
   inventory.live_watches == paired_dials(observation)
   ```

2. Pass that returned observation to the unchanged
   `assert_inventory_agrees`. This preserves the pair/watch assertions and the
   per-CID quota assertion rather than replacing them with the predicate.
3. Continue computing `most` from the same returned observation and continue
   the existing 100-ms sleep/window loop.

This predicate accepts either required completed state: a Paired held probe
with one owner dial and one live watch, or a fully dropped probe with neither.
It waits through both pre-Paired watch creation and post-owner/pre-kernel drop
transitions. A persistent production disagreement still fails closed at the
existing settle deadline with the last observation.

No new test, scenario, lane, API, hook, expected-value seam or product change
is needed. The correction does not weaken `assert_inventory_agrees`, the
probe-application evidence, the `most <= 1` assertion, Relay quota semantics
or final teardown-to-zero.

## Current production-reset evidence disposition

The supplied exact-fifth receipt `a625379a6750f4e3c91580340` passes the
previous Reset-vs-EOF body with the client observing Reset. The product reset
change remains owned by the original crafter and is untouched by this review.
The canonical receipt reaches the new U-D17 failure only after the retained
111 and CLI body pass, which is consistent with that earlier failure moving
out of the active boundary.

This review grants no full-suite credit: the canonical run stops on F-04
before any five-control-plane reconciliation can complete.

## Findings and disposition

| Item | Disposition |
| --- | --- |
| R5-57 held-state pair/watch equality | **REMAINS REQUIRED** |
| R5-57 completed-drop release | **REMAINS REQUIRED** |
| Native `live_watches=1`, owner dials empty snapshot | **MID-DROP TRANSITION, DIRECTLY REPRODUCED** |
| Persistent post-drop product mismatch | **NOT PROVEN** |
| Seeded Sim ordering defect | **NOT REPRODUCED; NO PRODUCTION FINDING ACCEPTED** |
| F-04 private polling precondition | **OPEN** |
| Exact bounded remediation | **WAIT FOR THE SAME EQUALITY, THEN RUN ALL EXISTING ASSERTIONS** |
| New architecture/API/linearization mechanism | **NOT AUTHORIZED OR NEEDED** |
| Runtime skip / five-CP / 117 | **ZERO CREDIT** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 18 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 9/10 current / 10/10 prescribed | Exact agreement remains required; only an incomplete operation is excluded. |
| GWT / one behavior | 10/10 | The existing probe/Relay watch behavior and identity remain unchanged. |
| Domain language | 10/10 | Owner dial, kernel watch, public Drop completion and observation sample are distinguished. |
| Coverage completeness | 8/10 current / 10/10 prescribed | Current sampling can fail before the required state is complete; the bounded wait restores the intended universe. |
| Walking-skeleton user value | 10/10 | Real probe dials, the production owner and real kernel inventory remain the evidence path. |
| Priority validation | 10/10 | Native chronology is classified without inventing a production defect or API. |
| Observable behavior | 9/10 current / 10/10 prescribed | The exact equality is retained and moved to a completed-state observation. |
| Traceability | 10/10 | R5-57, QPROJ-3, Drop source and receipt timestamps align. |
| Walking-skeleton boundary | 10/10 | No alternate seam, scenario, lane or product hook is introduced. |

## Iteration 18 verdict

**NEEDS PRIVATE DISTILL REMEDIATION.** The canonical failure is an honest real
kernel snapshot, but it was taken while the probe stream's synchronous Drop
was incomplete: owner bookkeeping had been removed and the inner
`HostGuestStream` had not yet completed the kernel-watch abort. The receipt
does not prove a post-completion product invariant violation.

F-04 belongs to the private U-D17 sampler. Wait for the same required
pair/watch equality with the existing settle deadline, then run every existing
assertion unchanged. No production remedy, seeded-Sim finding, architecture,
API or contract weakening is authorized. The run earns no five-CP or 117
credit; roadmap validation remains approved.

# NEEDS PRIVATE DISTILL REMEDIATION

# Review iteration 19 — F-04 completed-operation sampling

## Iteration 19 metadata

| Field | Value |
| --- | --- |
| Scope | Source-phase re-review of F-04 only |
| Corrected control-plane integration source | `6cbd1247a0986b11880fee5ed02fa16c404df066b4b313a9ad40ba4265ee6a81` |
| Scoped source gate | `abc14bf1ae79f67d51e4723a`, exit 0 |
| Scoped source digest | `dfbd5734fe36d970f8de012563bab2795b4d43ae2571cf43a010b5f027387b57` |
| Existing-Sim evidence | `36ae03f443918412eb24d345`, exit 0 |
| Finding disposition | **F-04 CLOSED** |
| Native disposition | **ONE FULL CANONICAL EXISTING `cgroup-default` 117-BODY RUN MAY PROCEED** |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, acceptance body, DESIGN/DISTILL handoff, roadmap, DES event,
receipt, host state or commit was changed. No native, Lima or mutation
execution was launched by the reviewer.

Roadmap validation remains `approved`. The original crafter's production reset
correction and all other dirty work remain untouched. This iteration closes
the private sampler finding only.

## Exact completion predicate

`sample_agreement` retains the same outer sample window and `most` accumulator.
Each iteration now calls the existing `Node::wait_observation`, whose existing
`SETTLE_BOUND` is 30 seconds and whose poll interval is 200 ms. Its new private
completion predicate accepts only an observation whose forwarder inventory is
readable and whose exact values satisfy both required relations:

```text
inventory.live_pairs   == paired_guest_flows(observation)
inventory.live_watches == paired_dials(observation)
```

The source uses direct integer equality. It adds no tolerance, retry count,
range, eventual-success override, stale cached value or approximate oracle.
`NoContext` and `Unreadable` do not satisfy the predicate. A persistent
mismatch still reaches the existing settle-deadline panic with the last exact
observation.

This predicate correctly admits the two completed states identified in
iteration 18:

- a held Paired probe or Relay whose owner phase and real forwarder pair/watch
  are both present; or
- a fully completed drop whose owner entry and real forwarder pair/watch are
  both absent.

It waits through `AwaitingReply` after the watch appears and through the
post-owner/pre-kernel window of an incomplete synchronous Drop. It does not
accept that transition as a required outcome.

## Existing assertions remain controlling

The observation returned by `wait_observation` is passed unchanged to the
existing `assert_inventory_agrees`. That function still independently:

1. obtains `ForwarderInventoryRead::Read` or panics;
2. asserts exact `live_pairs == paired_guest_flows`;
3. asserts exact `live_watches == paired_dials`; and
4. asserts for every CID that `held_flows` equals guest flows plus datagram
   associations plus Relay dials, never Probe dials.

The predicate is therefore a temporal precondition for a completed operation,
not a replacement for any assertion. The exact pair/watch and quota outcomes
remain fail-closed.

`most` is computed from that same returned observation using the unchanged
`dials_of(..., DialPurpose::Probe)` result. Both AwaitingReply and Paired
counts retain their existing meanings, and the existing `most <= 1` assertion
continues to consume this value. No second or weaker observation supplies it.

## Preserved body and timing boundary

The only behavioral source delta for F-04 is the acquisition of the
`sample_agreement` observation. The following remain unchanged:

- the five-second probes-only sample window;
- the three-second probe/Relay sample window;
- the six-second dropped-probe sample window;
- the 100-ms sleep between returned samples;
- the Service's one-second probe interval;
- `BOOT_BOUND`, `SETTLE_BOUND` and every HTTP/API timeout;
- Service and client workload specifications;
- public submission, production `run_server` composition and real VM lane;
- the probe-application `probes.len() >= 3` and no-client assertions;
- the held Relay predicate and exact inventory/quota assertions;
- the dropped-probe `most <= 1`, reset and zero-watch assertions;
- teardown-to-zero, identities, Contract Shape declarations and runner
  manifest; and
- the canonical 111 + CLI + five-CP accounting for 117.

No production source, Drop order, owner observation, API, selector, lane,
scenario, hook or linearization mechanism changed.

## Existing-Sim evidence

Read-only Metal receipt `36ae03f443918412eb24d345` exited 0 with zero runtime
skips. It runs four existing owner/drop invariants:

- `quint_fwd_pair_dropped_stream_aborts_watch_then_guest_sees_abort`;
- `paired_returns_the_stream_and_its_drop_aborts_the_watch_then_closes_vh`;
- `quint_fwd_pair_owner_abort_sends_guest_abort_then_holder_drop_is_inert`;
- `quint_fwd_pair_released_watch_with_no_record_reads_as_abort`.

All four pass. The Quint cases print seed `0x1127719f`; the host driver prints
composition seed `0xf7a`. The output SHA-256 is
`2f723b2b42c13714c2b68343f6782b8c30595f2d61166b53c9cb581298776f02`.

This receipt corroborates iteration 18's ownership conclusion: existing
modeled drop/abort behavior has no post-completion watch disagreement. It does
not erase, reclassify or grant credit for any separate historical baseline
`quint_fwd_pair` seed failure. It is evidence for these four named current
invariants only and is not native kernel-inventory evidence.

## Source-gate evidence

Scoped Metal gate `abc14bf1ae79f67d51e4723a` used workspace digest
`dfbd5734fe36d970f8de012563bab2795b4d43ae2571cf43a010b5f027387b57`
and exited 0. It verified control-plane formatting, checked the exact
integration target with `integration-tests,kvm-tests`, and ran scoped Clippy
with dependencies excluded and warnings denied. Its output SHA-256 is
`df36c612634c894f1573d0fce0b128a5112c7191ee3493206af2cca14e6597d3`.
The terminal declares `kind=command`, `exit_code=0` and
`cause=command_returned`.

No native body ran in this source gate. The earlier gate attempt that could not
start while the authorized Sim receipt held the Metal lease contributes no
source or runtime credit.

## F-04 disposition

| Item | Disposition |
| --- | --- |
| Readable inventory requirement | **EXACT AND FAIL-CLOSED** |
| Pair equality precondition | **EXACT** |
| Watch equality precondition | **EXACT** |
| Existing pair/watch assertions | **UNCHANGED AND STILL EXECUTED** |
| Existing per-CID quota assertion | **UNCHANGED AND STILL EXECUTED** |
| `most` computation | **SAME RETURNED OBSERVATION** |
| Windows, cadence, probe interval and deadlines | **UNCHANGED** |
| Bodies, stimuli, identities and 117 accounting | **UNCHANGED** |
| Production/API/lane/selector change | **NONE** |
| Existing-Sim current invariants | **PASS; HISTORICAL BASELINE FAILURE NOT RECLASSIFIED** |
| F-04 | **CLOSED** |
| Roadmap validation | **REMAINS APPROVED** |

No bounded private finding remains.

## Native execution disposition

**AUTHORIZED:** one full canonical existing `cgroup-default` runner execution
may proceed at the reviewed source. It must retain the unchanged fail-closed
111 + CLI + five-control-plane reconciliation and may claim 117 only if that
single canonical run reports every body passed with zero ignored/runtime-skip
results. An isolated body result or a partial canonical prefix cannot replace
that evidence.

This authorization is limited to the existing runner. It does not authorize a
new selector, retry-until-pass loop, scenario, lane, fixture weakening,
production change or mutation run.

## Iteration 19 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | A persistent mismatch still fails at the existing settle deadline. |
| GWT / one behavior | 10/10 | Existing probe/Relay watch scenarios and identities remain unchanged. |
| Domain language | 10/10 | Completed operation, owner phase, kernel inventory and sample window remain distinct. |
| Coverage completeness | 10/10 | Both exact equalities, all original assertions and quota accounting remain active. |
| Walking-skeleton user value | 10/10 | Production serve/deploy, real VMs and real kernel inventory remain the native evidence path. |
| Priority validation | 10/10 | F-04 is corrected privately without a production mechanism or contract change. |
| Observable behavior | 10/10 | The same exact returned observation supplies both assertions and `most`. |
| Traceability | 10/10 | Every iteration-18 remediation item maps directly to current source. |
| Walking-skeleton boundary | 10/10 | No API, seam, selector, scenario or lane was added. |

## Iteration 19 verdict

**APPROVED.** F-04 is closed. Each U-D17 sample now waits through the existing
settle boundary for readable inventory and exact pair/watch agreement, then
runs every existing assertion and computes `most` from that same observation.
No count, deadline, stimulus, identity, production path or 117 condition is
weakened.

The current named Sim drop/abort invariants pass without reclassifying any
historical baseline failure. One full canonical existing `cgroup-default`
117-body run is authorized. Roadmap validation remains approved.

# APPROVED — ONE CANONICAL 117-BODY RUN AUTHORIZED

## Iteration 19 execution-evidence addendum

### Addendum metadata

| Field | Value |
| --- | --- |
| Authorized canonical run | `8f6666cab490a60207221780`, exit 101 |
| Runtime source digest | `4fd0fbc034b0281868fc9545eb062f0c1de2fc1ec7da600a16fcd7b3a01dd4fa` |
| Runtime output SHA-256 | `43b9ded0ad0a2d7e4cabc4e6e1c5623f55f2d43ef7278166ac0e97271d269385` |
| Cleanup | `7daa6185c045b7dafaf1d00d`, exit 0 |
| F-04 source disposition | **REMAINS APPROVED AND CLOSED** |
| Full canonical result | **NOT ESTABLISHED** |
| Retry disposition | **NO RETRY AUTHORIZED OR PERFORMED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, acceptance body, DESIGN/DISTILL handoff, roadmap, DES event,
receipt, host state or commit was changed. No native, Lima or mutation
execution was launched by the reviewer.

This addendum reviews the one execution already authorized in iteration 19.
It does not reopen F-04 from unavailable evidence and does not infer a new
fixture or production cause.

### Visible libtest evidence

The durable host output contains 102 visible successful libtest summaries
totaling:

```text
112 passed
0 failed
0 ignored
```

These visible summaries are the retained 111-body boundary plus the CLI body.
The output contains no recognized runtime-SKIP diagnostic. The CLI exact body
is visibly the final successful summary before the control-plane outer guest
failure.

The number 112 is the complete visible summary count in this receipt. It is
not rounded up with inferred CP progression and cannot be reported as 114 or
117.

### Control-plane runner progression

The unchanged `STEP14_01_CONTROL_PLANE` manifest is ordered:

1. `the_forwarder_inventory_counts_every_held_guest_flow_through_serve`;
2. `a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted`;
3. `the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`;
4. `a_dropped_probe_stream_is_an_abort_at_the_guest_through_serve`;
5. `a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve`.

`exec_required_cgroup_cells_with_bound` iterates that slice sequentially. It
cannot begin the next cell unless the previous `exec_cgroup_required` returns
successfully after checking all of the following:

- the exact command returned within its bound;
- exit code was zero;
- the exact required one-pass libtest summary was present; and
- no recognized runtime-SKIP line was present.

The retained failure names the third exact filter. Therefore the first and
second CP commands returned through those existing fail-closed checks. This is
valid runner-progression evidence.

Their inner summaries are not visible in the durable host output. The CP outer
GuestKit console is printed only by `GuestRun::finish` after all five cells
complete; the third command returned an error first, so `finish` and the
five-body reconciliation were never reached. Progression through two commands
must not be conflated with a durable completed CP manifest or full-runner
count.

### Third CP timeout and panic-renderer evidence

The third exact command did not reply to outer EXEC within the unchanged
600-second bound. The runner reports only that timeout plus the outer guest's
fixed final 16-KiB console tail.

That tail is visibly inside `panic_console_diagnostics`: it contains the
unique headed `final-byte-tail` section for exact Service allocation
`alloc-d17-dial-svc-0` and allocation-prefixed complete control/end records.
The records extend through probe connection 66. Every visible Service
application record is `hello=no terminal=reset`, paired with probe-style
ForwardId handle-end and deferred-close records. No `hello=yes` Service record
is visible.

The panic renderer runs only while the CP body unwinds through an assertion or
poll panic. Thus the third body did enter, later panicked, and then the outer
EXEC failed to complete. The 600-second no-reply result is secondary to an
already-entered body panic, but the retained tail does not preserve the
primary panic message.

### Sorted-output displacement and exact evidence limit

The panic renderer iterates its retained console map in `BTreeMap` allocation
order. Any client allocation sorts before `alloc-d17-dial-svc-0`; the ordinary
panic hook also prints before local guards finish unwinding. The complete
client section and primary panic text therefore precede the large Service
section.

The Service's sorted complete-line section contains dozens of probe records
and consumes the final fixed tail. In the retained host output:

- the panic heading itself is displaced;
- no retained client allocation section remains;
- no original assertion/poll panic text remains;
- no `hello=yes` Service terminal record remains; and
- only the latter part of the Service allocation section and its raw tail are
  available.

The visible Service records support only a bounded statement: the retained
Service application ends in the available tail are probe connections, and no
held-client Service terminal record is visible. They do not prove whether the
client never paired, paired but had not ended, or failed at another existing
poll/assertion.

The original F-04 `live_watches=1` / zero-Paired-dial panic is absent. Its
absence is not proof that F-04 recurred, passed in the third body, or was the
primary cause. The corrected source remains approved from iteration 19; this
receipt simply cannot classify the new body's primary panic.

### No third pass or 117 credit

There is no visible third-control-plane libtest pass summary. The third
`exec_cgroup_required` did not return success, so the sequential loop never
incremented its passed count for that cell. The fourth and fifth CP identities
were not entered, and the final checks for exactly five CP bodies and six total
STEP 14-01 bodies were not reached.

Accordingly:

- visible durable summaries remain 112;
- the first two CP command checks completed, but the CP manifest did not;
- the third CP identity receives no pass credit;
- the final two CP identities receive no execution credit; and
- the canonical 117 result is not established.

Per the single-run authorization, no retry was launched to obtain another
schedule, recover the displaced client/panic section or seek a green result.
This addendum does not authorize a second run.

### Cleanup evidence

Read-only cleanup `7daa6185c045b7dafaf1d00d` used the same source digest and
exited 0. It reports:

- zero live Cloud Hypervisor processes;
- no physical-host nft table;
- no STEP 14-01 work directory; and
- only the documented pre-existing `ovd-ns-0000` namespace.

Its output SHA-256 is
`bb01f03188e0c91bbeba9da62f435ca6d547b9de7347206265621f8cd5883f12`.
No cleanup residue qualifies or changes the execution evidence limit.

### Addendum disposition

| Item | Disposition |
| --- | --- |
| F-04 source correction | **REMAINS APPROVED AND CLOSED** |
| Visible retained + CLI summaries | **112 PASSED, 0 FAILED, 0 IGNORED** |
| Runtime skips | **ZERO** |
| First two CP command checks | **PROGRESSED THROUGH EXISTING EXIT/SUMMARY/NO-SKIP GATES** |
| First two CP durable console summaries | **NOT VISIBLE; OUTER `finish` NOT REACHED** |
| Third CP body | **ENTERED, PANICKED, THEN OUTER EXEC TIMED OUT** |
| Third CP primary panic/cause | **DISPLACED; NOT CLASSIFIABLE** |
| Service tail | **PROBE RECORDS ONLY; NO HELD-RELAY CAUSE INFERRED** |
| Third CP pass | **NO CREDIT** |
| Fourth/fifth CP execution | **NOT ENTERED** |
| Full 117 | **NOT ESTABLISHED** |
| Retry | **NOT PERFORMED; NOT AUTHORIZED BY THIS ADDENDUM** |
| Cleanup | **PASS** |
| Production or fixture remedy | **NONE INFERRED OR PRESCRIBED** |
| Roadmap validation | **REMAINS APPROVED** |

### Iteration 19 controlling verdict after execution

**F-04 SOURCE APPROVAL REMAINS; CANONICAL 117 IS NOT ESTABLISHED.** The one
authorized runner visibly retains 112 successful summaries with zero skips and
progresses through the first two CP commands' fail-closed checks. The third CP
body enters and panic-renders Service probe evidence, but its primary panic and
client section are displaced before the secondary outer EXEC timeout.

No honest third-pass, full-117 or primary-cause claim can be made. No retry or
remedy is authorized from this evidence. Roadmap validation remains approved.

# EXECUTION EVIDENCE INCOMPLETE — 117 NOT ESTABLISHED

# Review iteration 20 — full timeout console and CP-prefix boundary

## Iteration 20 metadata

| Field | Value |
| --- | --- |
| Scope | Source/boundary review for failure-console retention and increment-B focused reproduction |
| Corrected canonical runner | `285a49b1c1b8b0f2dc62e7c0056aa84cbed315759cb7df07eafa0bb14db13962` |
| Runner source gate | `ceae28d65a65493fff211881`, exit 0 |
| Increment-B final gate | `19c52e22eeac39c03c48780b`, exit 0 |
| Increment-B executable | `3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6` |
| Native execution | None |
| Focused reproduction | **ONE INCREMENT-B EXECUTION AUTHORIZED** |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, cache artifact, host state or commit was changed.
No native, Lima or mutation execution was launched by the reviewer.

Roadmap validation remains `approved`. F-04 remains closed. The canonical 117
result remains unestablished; the focused boundary exists only to recover the
third CP body's displaced primary panic and cannot replace canonical evidence.

## Canonical runner timeout correction

The canonical runner has two test EXEC implementations:

- `exec_with_skip_policy`, for an arbitrary staged test binary; and
- `exec_cgroup_with_skip_policy`, for the existing cgroup/nofile wrapper.

In each function, only the `peer().exec(..., bound)` error branch changed. The
branch still:

1. receives the original peer error as `source`;
2. returns the same `GuestKitError::Staging` immediately;
3. preserves the original detail text with the exact `test_args` and `bound`;
4. keeps the same nested `io::Error::other` cause; and
5. returns before any summary check, passed-count increment, `finish` or
   manifest reconciliation.

The sole correction replaces:

```text
GuestKitVm::console_tail(16 * 1024)
```

with:

```text
GuestKitVm::console()
```

and labels that attachment as the full disposable guest console. The same
`GuestKitVm` remains live when the getter reads its console path; cleanup and
`GuestRun` drop occur only after the error value is constructed and returned.
Thus the full bytes are captured before VMM/runtime cleanup can remove their
source.

`GuestKitVm::console()` is the existing read-only getter used by successful
`GuestRun::finish`. It reads the complete console file so far and performs no
parsing, filtering, sorting, truncation or state change. Newline-terminated
records and any explicitly raw partial fragment remain byte-ordered as the
outer guest emitted them.

## Preserved error and success paths

Every non-timeout path is unchanged:

- a nonzero child code still returns the same full per-command
  `run_console`;
- a zero code without the exact required pass summary still fails with that
  same full command slice;
- a recognized runtime-SKIP still fails from that same command slice;
- a successful command returns `Ok(())` without printing the console; and
- `GuestRun::finish` still prints the full outer console exactly once after
  all selected cells succeed, then powers off and requires the existing
  30-second exit.

The per-cell deadline, command argv, cgroup wrapper, expected pass value,
runtime-SKIP policy and order are byte-identical. No timeout is extended and no
error is swallowed or reclassified.

Because timeout failure returns before `finish`, full failure-console
attachment cannot duplicate a successful console. On an all-green run only
`finish` prints it. On a timeout run only the error embeds it. No duplicate
pass summary or passed-count increment is introduced, and the canonical 117
accounting is unchanged.

## Canonical runner source gate

The corrected runner SHA-256 is
`285a49b1c1b8b0f2dc62e7c0056aa84cbed315759cb7df07eafa0bb14db13962`.
The complete source diff consists of the two `console_tail` to `console`
replacements and the corresponding diagnostic label change.

Scoped Metal gate `ceae28d65a65493fff211881` used source digest
`59323d89ff4032c85735f10a38e11c86dea05ad0b85b8532623dd15134ebd9c0`
and exited 0. It verified formatting, compile/check and Clippy for the
corrected runner's crate/target. Its output SHA-256 is
`c849de45d9404d657d2cb5f20754ad255eba28328bb271da70fdca055f68e694`.
No test or VM ran in that gate.

## Increment-B spike isolation

The focused launcher is self-contained under
`spike-scratch/netns-density-295-cp-14-01/increment-b/`, with its own
workspace, manifest, lockfile, generator, main and README. Its only direct
dependency is the existing `overdrive-testing` path dependency with the
existing `integration-tests` feature. No workspace member, production crate,
canonical runner, CI lane or public selector refers to it.

| Increment-B source | SHA-256 |
| --- | --- |
| Manifest | `bb2b7bbd63b0cfd9c156bf9c663f967142858adfbcf0936588d99111b1631332` |
| Lockfile | `f9bd75adb6c297d87558dee6ba709b4961e92f5a5106fa2069eb213f6049f600` |
| Generator / private prefix wrapper | `5764df439e1d42b69795f84702e5b8359a70f2186a53e1154236df3fda80ea03` |
| Main | `0a6d5aa5b26a60b5f3be2acec6b969aacc0298090f8a407e2565fd6ce8850822` |
| README and exact commands | `4bcc8c31f3ef94cdaafe06372f506794ac84b38557aed71a30f7c164425987c0` |

The increment obeys the spike rule: source and evidence are isolated from
`crates/`; build output uses the ignored external target/cache locations; no
workspace wiring or shipping dependency is added.

## Exact canonical-source inclusion

At build time the generator reads the current
`fwd_guest_suite.rs`, declares it as a rerun input and includes the byte suffix
from the first Rust item. The removed prefix contains only file-level docs,
crate attributes and blank lines.

Within the included suffix, the generator requires exactly one byte-identical
canonical private `workspace_root()` helper. It replaces only that helper with
the same private compile-time repository-root value independently approved for
increment A. Mechanical comparison confirms all bytes outside that helper are
identical to the current canonical runner suffix.

The replacement root canonicalizes `increment-b/../../..`, equal to the
repository root the original helper derives from
`crates/overdrive-testing`. The staged BPF object, forwarder module, kernel,
init, Cloud Hypervisor, nested modules, guest peer, XFS volume and every guest
path therefore remain canonical.

The generator appends only the private
`run_exact_control_plane_prefix` wrapper. The included canonical `main` is
inert inside the private module; the standalone `main` calls only this wrapper.
Module-level lint allowances suppress unused canonical groups and change no
runtime item.

## Exact prefix selection and GuestKit boundary

The wrapper selects:

```rust
&STEP14_01_CONTROL_PLANE[..3]
```

from the current canonical manifest. That is exactly, in order:

1. `the_forwarder_inventory_counts_every_held_guest_flow_through_serve`;
2. `a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted`;
3. `the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`.

It calls the existing canonical `run_step14_01_binary` with:

```text
package = overdrive-control-plane
target = integration
slot = 15
cells = canonical prefix [0..3]
```

The first two cells are retained as the exact prerequisite prefix that the
canonical run completed before the third failure. The helper preserves the
same prepared integration binary, outer GuestKit image, eight vCPUs, nested
payload, kernel/module closure, cgroup v2, bpffs, securityfs, device modes,
XFS reflink volume, no-uplink boundary, `CGROUP_NOFILE_EXEC`, exact filters,
600-second per-cell bound, one-pass summary checks, runtime-SKIP rejection and
cleanup.

The wrapper's `passed == 3` assertion is the existing three manifest counts
summed by the canonical helper. It is runner reconciliation, not an injected
product expected value. No body, assertion, stimulus, fixture state or product
oracle is copied or altered.

## No public surface or 117 substitution

Increment B adds no test identity, scenario, public/canonical selector,
environment group, API, hook, lane or product mechanism. It reuses existing
private test support to collect one focused diagnostic record.

Even if all three prefix cells pass, that result is outside the canonical
runner's retained-111 + CLI + five-CP reconciliation. It cannot contribute
three bodies to a partial canonical receipt, replace the prior timeout, or be
reported as 117. Its only valid purpose is classification of the third CP
failure with the newly retained full outer console.

## Increment-B gates and executable identity

Compiler gate `1291d1d703631b838dcc7397` exited 0. Final unprivileged locked
Clippy/build/staging gate `19c52e22eeac39c03c48780b` used source digest
`02aa72bb9d1b6bbf4481549c181efd7247c3b91ff3688b09306ea34209102053`
and exited 0. Its output SHA-256 is
`9a303a2ff04291b076d84724d6d9b65d0c830c1e463f0cc0dbd8cf6498eeeb12`.

The gate built and installed executable SHA-256
`3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6`
outside the rsync-managed workspace under that content-addressed directory.
The runtime command performs ordinary GuestKit preparation, requires the same
SHA-256 and directly executes that reviewed binary. It does not build or run
the standalone crate through Cargo in the privileged phase.

No native body ran in these source gates.

## Findings and execution disposition

| Item | Disposition |
| --- | --- |
| Both EXEC timeout branches | **FULL CONSOLE READ BEFORE CLEANUP** |
| Original peer error, deadline and fail-closed return | **UNCHANGED** |
| Nonzero/missing-summary/runtime-SKIP paths | **UNCHANGED** |
| Success path and `finish` output | **UNCHANGED; NO DUPLICATE SUMMARY** |
| Runner counts and 117 reconciliation | **UNCHANGED** |
| Increment-B isolation | **APPROVED** |
| Canonical source inclusion | **APPROVED OUTSIDE THE ONE NEUTRAL ROOT HELPER** |
| First-three manifest selection/order | **EXACT** |
| Slot/helper/payload/security/cleanup | **CANONICAL AND UNCHANGED** |
| Public selector/test/scenario/lane/API | **NONE ADDED** |
| Expected product value injection | **NONE** |
| Increment-B contribution to 117 | **FORBIDDEN** |
| Roadmap validation | **REMAINS APPROVED** |

No bounded source or boundary finding remains.

## Focused execution authorization

**AUTHORIZED:** one matching-source increment-B focused reproduction may
proceed using the reviewed content-addressed executable. It must run the exact
canonical three-cell prefix once, retain all failure output, and receive no
117 credit regardless of outcome.

This authorization does not permit a retry-until-pass loop, a changed prefix,
new selector, new scenario, body modification, fixture weakening, production
change, full canonical rerun or mutation run. Any result must be classified
from the single durable receipt without inventing a cause when the full output
does not prove one.

## Iteration 20 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Timeout, nonzero, missing-summary and runtime-SKIP paths remain fail-closed. |
| GWT / one behavior | 10/10 | The focused launcher preserves the exact existing three-cell prefix and bodies. |
| Domain language | 10/10 | Outer console, CP body, prerequisite prefix and canonical 117 remain distinct. |
| Coverage completeness | 10/10 for source boundary | Both timeout branches, every unchanged result path, prefix selection and staging are reviewed. |
| Walking-skeleton user value | 10/10 | The same production `run_server`, public submission and real VM composition execute. |
| Priority validation | 10/10 | The correction recovers displaced evidence without changing product or acceptance behavior. |
| Observable behavior | 10/10 | Full console bytes are retained before cleanup with no parser or priority rule. |
| Traceability | 10/10 | Every source change and wrapper argument maps to the canonical runner. |
| Walking-skeleton boundary | 10/10 | No public selector, lane, API, scenario or alternate fixture was introduced. |

## Iteration 20 verdict

**APPROVED.** Both canonical EXEC timeout branches now retain the complete
outer GuestKit console before cleanup while preserving every original error,
deadline, return, count and success path. No pass summary is duplicated.

Increment B includes the current canonical runner, neutralizes only its
standalone workspace-root context and invokes the exact existing first-three
CP prefix through the canonical slot-15 GuestKit helper. It cannot count toward
117. One matching-source focused reproduction is authorized; roadmap
validation remains approved.

# APPROVED — ONE INCREMENT-B FOCUSED REPRODUCTION AUTHORIZED

# Review iteration 21 — focused three-CP runtime evidence

## Iteration 21 metadata

| Field | Value |
| --- | --- |
| Scope | Final review of the one authorized increment-B focused execution |
| Focused receipt | `1e232061b80e4a961e094624`, exit 0 |
| Runtime source digest | `c367378501a3c3d6a3f87c0b87c47eb88f172314a8d93844931aae15928745e4` |
| Reviewed executable | `3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6` |
| Cleanup receipt | `c5841add0a5378055dfea2e4`, exit 0 |
| Focused result | **EXACT FIRST THREE CP IDENTITIES PASSED IN CANONICAL ORDER** |
| Canonical 117 result | **NOT ESTABLISHED** |
| Iteration verdict | **FOCUSED EVIDENCE APPROVED; NO CAUSE OR REMEDY INFERRED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, cache artifact, host state or commit was changed.
No native, Lima or mutation execution was launched by the reviewer.

Roadmap validation remains `approved`. F-04 remains closed in source. This
iteration classifies the focused receipt only; it does not replace or retry the
failed canonical run.

## Exact executable and boundary

The focused command verifies and executes the content-addressed increment-B
ELF approved in iteration 20. Its generator still includes the current
canonical runner, neutralizes only the standalone compile-time repository-root
helper and calls the existing `run_step14_01_binary` with:

```text
package = overdrive-control-plane
target = integration
slot = 15
cells = STEP14_01_CONTROL_PLANE[..3]
```

The helper creates one fresh CP outer GuestKit guest, stages the same nested
payload and security/cgroup prerequisites, executes each exact cell
sequentially with the same 600-second bound and summary/SKIP gates, then uses
its normal success `finish` path. No focused source or manifest identity
changed after source approval.

## Exact identities, order and summaries

The durable output shows three separate exact libtest invocations in the
canonical manifest order:

| Order | Existing identity | Summary | Duration |
| ---: | --- | --- | ---: |
| 1 | `the_forwarder_inventory_counts_every_held_guest_flow_through_serve` | 1 passed, 0 failed, 0 ignored | 9.25 s |
| 2 | `a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted` | 1 passed, 0 failed, 0 ignored | 6.23 s |
| 3 | `the_forwarder_inventory_counts_probe_and_relay_watches_through_serve` | 1 passed, 0 failed, 0 ignored | 14.66 s |

The three summaries total exactly:

```text
3 passed
0 failed
0 ignored
```

No recognized runtime-SKIP diagnostic appears. The wrapper's existing
`passed == 3` reconciliation succeeds, the process exits 0 by
`command_returned`, and the outer GuestKit reaches its normal full-console and
poweroff path.

This is non-vacuous focused evidence for the three existing bodies under the
increment-B context. No fourth or fifth CP identity is selected or receives
execution credit.

## Full failure-console branch disposition

All three selected commands return normally with their exact required
summaries. There is no peer EXEC timeout, no `did not return within` error and
no `full disposable guest console` failure attachment.

Therefore the corrected canonical timeout branch approved in iteration 20 is
not exercised by this receipt. The successful `GuestRun::finish` path prints
the outer console once, as designed. The focused pass supplies no runtime
validation of the new full-console error branch beyond its source review and
gates.

## Canonical-versus-focused evidence boundary

The prior canonical receipt `8f6666cab490a60207221780` remains exit 101. It
visibly retained 112 successful summaries, progressed through the first two CP
command checks and then entered the same third CP identity, whose primary panic
was displaced before the secondary outer timeout. That receipt did not
establish a third pass or 117.

Increment B intentionally omits the retained-111 and CLI execution prefix. It
starts the same CP helper in a fresh outer CP GuestKit guest and preserves the
same first-three identities and order, but it executes under that reduced
focused context. The third identity passes there.

The two receipts therefore establish a context/schedule-sensitive difference:

- full canonical prefix present → third body panics, primary panic unavailable;
- focused three-CP prefix only → all three bodies pass.

They do not establish that the canonical failure was fixed, that F-04 recurred,
that the omitted retained/CLI prefix caused the failure, or that any production
owner, kernel, timing or cleanup mechanism is defective. The canonical primary
panic is still unavailable, so no root cause can be selected from the focused
pass.

No production or fixture remedy is inferred or prescribed. No second focused
or canonical run is authorized by this review.

## 117 accounting

Increment B is outside the canonical runner's fail-closed reconciliation. Its
three successful bodies cannot be added to the canonical receipt's 112 visible
summaries or used to replace the canonical third-body failure.

Accordingly:

- focused diagnostic count: 3/3 selected bodies passed;
- canonical durable visible count: remains 112;
- canonical five-CP reconciliation: not completed; and
- canonical 117: not established.

The focused result receives diagnostic evidence credit only.

## Receipt and cleanup integrity

Focused run `1e232061b80e4a961e094624` exited 0. Its 252,899-byte output
SHA-256 is
`c570ad1908b41d66b42716abb880f59225dcbd505fdaa8e53818c25693da20c7`;
its terminal SHA-256 is
`70f81968d3212ba31a28e60a83f675fad184b2af725e1031c0204546e638d168`.
The raw files are retained byte-for-byte under the ordinary evidence root and
increment-B's evidence directory.

Read-only cleanup `c5841add0a5378055dfea2e4` exited 0. It reports zero live
Cloud Hypervisor processes, no physical-host nft table and no STEP 14-01 work
directory. The only listed namespace is the documented pre-existing
`ovd-ns-0000`. Its output SHA-256 is
`4e6a0605225125df50785555f9780c36a781ae94bbed6287a31fbfc7406f50d4`.

The cleanup receipt's workspace digest differs because the completed focused
raw record was copied into increment-B's retained evidence before the audit;
no executable source or behavior changed.

No second focused execution, canonical retry, production edit, Lima run,
mutation run, DES event or commit followed.

## Findings and disposition

No DISTILL source or focused-boundary finding remains.

| Item | Disposition |
| --- | --- |
| Exact first-three identities | **PASSED IN CANONICAL ORDER** |
| Focused libtest summaries | **3 PASSED, 0 FAILED, 0 IGNORED** |
| Runtime skips | **ZERO** |
| Process result | **EXIT 0** |
| Fresh CP GuestKit helper/payload | **CANONICAL AND PRESERVED** |
| Failure-console timeout branch | **NOT EXERCISED** |
| Canonical `8f6666…` failure | **REMAINS A FAILURE; NOT RECLASSIFIED** |
| Focused result contribution to 117 | **FORBIDDEN / ZERO** |
| Canonical 117 | **NOT ESTABLISHED** |
| Root cause | **NOT PROVEN** |
| Production or fixture remedy | **NOT INFERRED OR PRESCRIBED** |
| Cleanup | **PASS** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 21 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for evidence classification | Focused success is accepted without erasing the canonical failure. |
| GWT / one behavior | 10/10 | The exact existing three-cell prefix executes in canonical order. |
| Domain language | 10/10 | Focused context, canonical prefix, timeout branch and 117 remain distinct. |
| Coverage completeness | 10/10 for the selected prefix | Every selected identity has an exact successful summary and zero skip. |
| Walking-skeleton user value | 10/10 | The same production serve/deploy and real VM helper execute. |
| Priority validation | 10/10 | Evidence is bounded without inventing a cause or retry. |
| Observable behavior | 10/10 | Exact identities, summaries, order, exit and cleanup are durable. |
| Traceability | 10/10 | Receipt lines map directly to the approved increment-B manifest. |
| Walking-skeleton boundary | 10/10 | Diagnostic success is denied canonical 117 credit. |

## Iteration 21 verdict

**FOCUSED EVIDENCE APPROVED.** The exact first three CP identities pass in
canonical order with three separate one-pass summaries, zero failures, zero
ignored/runtime skips, normal GuestKit finish, exit 0 and complete cleanup.

The focused context omits the retained-111 and CLI prefix. It does not exercise
the corrected timeout branch, does not reclassify the canonical failure and
contributes nothing to 117. No cause, remedy or additional execution is
prescribed. Roadmap validation remains approved.

# FOCUSED PREFIX PASSED — CANONICAL 117 NOT ESTABLISHED

# Review iteration 22 — full canonical first-CP failure evidence

## Iteration 22 metadata

| Field | Value |
| --- | --- |
| Scope | Final review of matching-source canonical failure with full timeout console |
| Canonical receipt | `727f0a30d20dc6ae67b3d3b1`, exit 101 |
| Runtime source digest | `1238131a47429722724604cf47e29c0a9560fcec62593c44d3cd7bb364e07a06` |
| Runtime output SHA-256 | `fc0b0cc1a8bd68a50580b5c0415ec0198527b997e1076b1bb8001c48d2c04c3c` |
| Cleanup | `476535f2cbe3a342e9224e55`, exit 0 |
| Primary failure | **FIRST CP CLIENT FLOW RESET DURING FORWARDER RECOVERY/QUIESCENCE BEFORE `hold: flowing`** |
| Root cause of recovery | **NOT PROVEN** |
| Canonical 117 | **NOT ESTABLISHED** |
| Iteration verdict | **EVIDENCE APPROVED; NO REMEDY INFERRED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, isolated launcher, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, cache artifact, host state or commit was changed.
No native, Lima or mutation execution was launched by the reviewer.

Roadmap validation remains `approved`. F-04 remains closed in source. This
review accepts the matching-source failure evidence and preserves its limits;
it does not authorize a production change, fixture weakening, retry or new
scenario.

## Full-console reporter exercised

The canonical run reaches the timeout branch corrected in iteration 20. Its
error contains the marker `full disposable guest console` and retains the
complete 108,346-byte outer GuestKit console rather than the former final
16-KiB tail.

That full console preserves the original test panic and the complete exact
allocation console. The primary failure is therefore no longer displaced:

```text
thread '...the_forwarder_inventory_counts_every_held_guest_flow_through_serve' panicked at guest_flow_through_serve.rs:568:9:
allocation alloc-d17-inv-0-0: the guest flow never reported flowing
```

The body is the first canonical control-plane identity. It deploys three held
flows sequentially and fails on the first allocation's existing
`wait_flowing`; no CP body receives a pass result.

The original EXEC peer error, unchanged 600-second bound and fail-closed
runner error remain visible around the full console. The branch still exits
before `GuestRun::finish` and CP manifest reconciliation. This receipt is the
runtime proof that the corrected reporter retains the primary panic before
cleanup.

## Visible pass and skip accounting

The durable output contains 102 visible successful libtest summaries totaling:

```text
112 passed
0 failed
0 ignored
```

Those are the retained 111 bodies plus the CLI body. The output contains no
recognized runtime-SKIP diagnostic. The first CP body has no successful
summary; later CP identities are not entered. The run supplies zero CP pass
credit and no 117 result.

## Ordered production evidence

For exact allocation `alloc-d17-inv-0-0`, the full console retains this order:

| Timestamp | Evidence |
| --- | --- |
| `21:32:39.428483` | VM READY accepted |
| `21:32:39.434878` | allocation mTLS intercept installed |
| `21:32:39.437912` | guest EXEC released after intercept installation |
| `21:32:39.489926` | guest-originated `tcp_connect` enters the production owner |
| `21:32:39.491219` | real forwarder inventory reports `live_pairs: 1`, `live_watches: 0` |
| `21:32:39.510982` | shared guest-network owner enters bounded recovery for component `Forwarder` |
| receipt order after recovery | guest application reports `terminal=reset` |
| `21:32:39.522438` | exact allocation's `tcp_connect` ends with cause `quiesced` |

The client console contains READY and EXEC but no `hold: flowing`. It contains
the reset terminal line and the production PID-1 owner records below. Thus the
body's polling failure is non-vacuous: the real flow was established far
enough to create a kernel pair, then reset while shared-owner recovery held
admission quiesced.

The receipt order does not establish a finer causal order between the
untimestamped guest serial line and adjacent host events. It establishes the
bounded facts that recovery was entered, the owner classified the flow as
quiesced, and the guest application observed Reset before `wait_flowing`
could succeed.

## Exact client PID-1 evidence

The panic renderer retains one exact allocation section:

```text
allocation=alloc-d17-inv-0-0 observer guest-app: terminal=reset
allocation=alloc-d17-inv-0-0 overdrive-init: guest-flow forward-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: B } }
allocation=alloc-d17-inv-0-0 overdrive-init: guest-flow handle-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: B } } tcp_pairs=1 pending_end_closes=0
allocation=alloc-d17-inv-0-0 overdrive-init: guest-flow deferred-close stream id=ForwardId(1) clear_linger=false ...
```

The workload application observes Reset. Production `overdrive-init` PID 1
consumes the matching side-B PeerAbort for `ForwardId(1)`, handles it with one
TCP pair and zero pending end closes, and applies the existing deferred close
without clearing linger. These are the actual production guest owner and
application paths, not an injected fixture result.

The evidence proves the flow end and its local close handling. It does not
identify why the host shared owner entered Forwarder recovery.

## Recovery classification limit

After the one-live-pair inventory observation, the host reports bounded
Forwarder recovery and later reports that recovery reaching its attempt bound.
The receipt does not retain a typed initiating audit error, failing ioctl,
task-exit cause or reproducible seeded-Sim invariant that explains why
recovery began.

Accordingly, this review does not infer that the live pair itself caused
recovery, that the forwarder was damaged, or that any lock/order/API is wrong.
“Flow reset during recovery/quiescence” is the reproduced proximal sequence;
“cause of Forwarder recovery” remains unknown.

No production persistence, ownership, recovery protocol, socket behavior,
retry or timeout change is authorized or prescribed from this evidence.

## Focused-prefix disposition

Increment-B receipt `1e232061b80e4a961e094624` remains valid focused
schedule evidence: the exact first three CP identities pass in order inside a
fresh CP outer GuestKit when the retained-111 and CLI prefix are omitted.

That focused pass does not reclassify this matching-source canonical failure.
The full sequence and focused sequence differ in preceding context and
schedule; no evidence pins which difference is causal. The focused result
cannot be added to the canonical 112 summaries and contributes no 117 credit.

No second focused or canonical execution is used to select a preferred
schedule.

## Earlier `af7d…` lifecycle limits

The separate canonical receipt `af7d3395010cef474da2c000` remains a distinct
CLI health-sequence failure and is not explained by the first-CP evidence.
Its durable facts include:

- the client Service spec declares listener port 18085 and no explicit health
  check section;
- the runtime records a successful `DialPurpose::Probe` dial to port 18085, so
  the listener path was reachable; the receipt must not be described as having
  no runtime probe at all;
- the client reaches `paths-ready` but never emits the expected
  `state=unhealthy` line;
- the client application later completes, PID 1 requests poweroff and emits
  EXIT, and owner quiescence follows; and
- peer application reads and state-control reads observe connection resets.

Those facts rule out a simple absent listener and preserve the observed normal
client-exit/poweroff lifecycle. They do not establish why the unhealthy-state
command was not observed.

The mTLS warning says a pump reached the generic `TransportDeath` exit class
and its canned text mentions “leg error / the (C) kernel-reaped ETIMEDOUT.” It
does not log an actual socket errno or a typed ETIMEDOUT result for that
connection. The word `ETIMEDOUT` in the generic warning is therefore not proof
that a host socket timed out. The receipt remains unresolved and no timeout,
health-withdrawal or recovery cause is inferred from it.

## 117 and retry disposition

The matching-source canonical result remains:

- 112 visible retained + CLI passes;
- zero ignored and zero runtime skips;
- first CP body entered and failed before `hold: flowing`;
- zero CP pass credit; and
- no full 117 reconciliation.

The run was the authorized matching-source full execution. No retry was
launched to seek another schedule, and this review does not authorize one. No
fixture assertion, deadline, identity or oracle is weakened.

## Cleanup evidence

Read-only cleanup `476535f2cbe3a342e9224e55` used the same source digest and
exited 0. It reports zero live Cloud Hypervisor processes, no physical-host
nft table and no STEP 14-01 work directory. The only listed namespace is the
pre-existing `ovd-ns-0000`. Its output SHA-256 is
`975058adf20ba1878cd3df10331d084995b39acee42b627c532156a374db0354`.

No cleanup residue qualifies the failure evidence.

## Findings and disposition

No DISTILL source finding is introduced. The current evidence disposition is:

| Item | Disposition |
| --- | --- |
| Full-console timeout reporter | **RUNTIME-PROVEN; PRIMARY PANIC RETAINED** |
| Visible retained + CLI summaries | **112 PASSED, 0 FAILED, 0 IGNORED** |
| Runtime skips | **ZERO** |
| First CP body | **FAILED BEFORE `hold: flowing`** |
| READY / intercept / EXEC | **COMPLETED** |
| Real kernel pair | **`live_pairs=1` OBSERVED** |
| Forwarder recovery | **ENTERED; INITIATING CAUSE NOT PROVEN** |
| Owner flow result | **`quiesced`** |
| Guest application | **`terminal=reset`** |
| PID-1 owner end | **`ForwardId(1)` SIDE-B PEER ABORT, `clear_linger=false`** |
| Focused three-CP pass | **SCHEDULE-CONTEXT EVIDENCE ONLY** |
| `af7d…` generic ETIMEDOUT warning | **NOT AN ACTUAL ERRNO PROOF** |
| Canonical 117 | **NOT ESTABLISHED** |
| Production remedy | **NOT INFERRED OR PRESCRIBED** |
| Cleanup | **PASS** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 22 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The primary CP failure is retained and receives zero pass credit. |
| GWT / one behavior | 10/10 | The first held-flow body fails through the existing production path and assertion. |
| Domain language | 10/10 | Forwarder recovery, owner quiescence, guest Reset and root cause remain distinct. |
| Coverage completeness | 10/10 for reached failure | READY through PID-1 close and cleanup are retained; unreached CP bodies receive no credit. |
| Walking-skeleton user value | 10/10 | Real workload VM, production intercept, real kernel pair and guest application determine the result. |
| Priority validation | 10/10 | Evidence is classified without inventing a recovery cause or remedy. |
| Observable behavior | 10/10 | Full console preserves the exact panic and allocation-correlated terminal path. |
| Traceability | 10/10 | Receipt timestamps, body source, capture output and runner accounting align. |
| Walking-skeleton boundary | 10/10 | Focused success is denied canonical pass credit and no oracle is weakened. |

## Iteration 22 verdict

**FULL-CONSOLE EVIDENCE APPROVED; CANONICAL 117 NOT ESTABLISHED.** The
matching-source canonical run visibly passes 112 retained/CLI bodies with zero
skips, then fails the first CP body's `wait_flowing`. The exact allocation
reaches READY, intercept, EXEC, `tcp_connect` and one live kernel pair before
the shared owner enters Forwarder recovery; the flow ends quiesced, the guest
application observes Reset, and PID 1 consumes the side-B PeerAbort and closes
with `clear_linger=false`.

The receipt does not establish why Forwarder recovery began. The focused
prefix pass remains schedule-context evidence only, and the earlier `af7d…`
generic warning does not prove ETIMEDOUT. No retry, production remedy or 117
credit is authorized. Roadmap validation remains approved.

# PRIMARY FAILURE RETAINED — ROOT CAUSE UNKNOWN — 117 NOT ESTABLISHED

# Review iteration 23 — direct canonical exact-body filtering

## Iteration 23 metadata

| Field | Value |
| --- | --- |
| Scope | Source/boundary review of the canonical test-support exact-body selector |
| Corrected runner | `75dac61241c3feaaaa2b4b4e165a710074de9eaef861e2d01bd179eed99d56e9` |
| Scoped source gate | `f40221b865095b3427ad9be0`, exit 0 |
| Scoped source digest | `4687074a55157fe9298d8eff7171bfeb563956f31e57b5ffe43093faf64d1b94` |
| Native execution | None |
| Focused execution disposition | **ONE DIRECT FIRST-CP EXISTING-BODY RUN AUTHORIZED** |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, acceptance body, DESIGN/DISTILL handoff, roadmap, DES event,
receipt, host state or commit was changed. No native, Lima or mutation
execution was launched by the reviewer.

Roadmap validation remains `approved`. The first-CP canonical recovery cause
and full 117 result remain unestablished. This iteration approves only a
direct selector in the existing test-support runner; it does not authorize a
product change or new acceptance scenario.

## Test-support-only surface

`OVERDRIVE_FWD_GUEST_EXACT` is read only by
`crates/overdrive-testing/bin/fwd_guest_suite.rs`, a native test-support
executable. No production crate, product binary, public Rust API, CLI command,
wire/config format or runtime composition reads it.

The selector does not add a new group. The existing
`OVERDRIVE_FWD_GUEST_GROUP` must equal `cgroup-default` whenever the exact
environment variable is present. Any other selected group, including the
default `all`, fails before group dispatch. Thus a focused invocation remains
inside the existing cgroup-default Metal/GuestKit lane rather than creating a
new lane or bypassing its prerequisites.

## Input validation and exact match

The selector is obtained with `std::env::var_os` and converted with
`OsString::into_string`. A non-UTF-8 value panics fail-closed. A valid string
is matched by full `&str` equality against only these current manifest slices:

- `STEP14_01_CLI`; then
- `STEP14_01_CONTROL_PLANE`.

The six identities are unique. No prefix, substring, glob, regex, test-module
filter or arbitrary binary/test path is accepted. Empty, partial and unknown
UTF-8 values reach the explicit zero-match `GuestKitError::Staging` and do not
boot a focused guest.

The match returns the existing manifest `Cell` itself, including its exact
identity, expected count and `is_exact` value. The focused function does not
construct or modify a test identity.

## Existing binaries, slots and helper

A matched CLI cell maps to the existing prepared `overdrive-cli` integration
binary and registered slot 14. A matched control-plane cell maps to the
existing prepared `overdrive-control-plane` integration binary and registered
slot 15. Both call the unchanged `run_step14_01_binary` with target
`integration` and a one-element slice containing the existing cell.

Consequently each focused execution retains the existing:

- outer scripted GuestKit image and eight-vCPU boundary;
- complete nested kernel/init/Cloud Hypervisor/module/BPF/tool payload;
- cgroup v2, bpffs and securityfs mounts;
- `/dev/kvm` and `/dev/vhost-vsock` device mode boundary;
- XFS reflink volume and no-uplink confinement;
- `CGROUP_NOFILE_EXEC` limit check;
- `--test-threads=1 --nocapture --exact <existing identity>` invocation;
- 600-second per-cell bound;
- exact required summary and runtime-SKIP rejection; and
- full timeout console, normal success console, poweroff and cleanup behavior.

The focused path adds no direct body call, in-process test invocation,
expected product value, state injection or alternate fixture.

## Exact count and zero canonical credit

The focused function reads `expected` from the selected existing cell. After
`run_step14_01_binary` returns, it requires `passed == expected` or fails
closed. Because all current STEP 14-01 cells have expected count one, a
successful focused execution has exactly one passed body.

It then emits one explicit test-support result:

```text
STEP14-01-FOCUSED-RESULT identity=<exact identity> passed=<exact manifest count> canonical_117_credit=0
```

The zero-credit field is fixed and not caller-controlled. The focused path
returns before retained-111, CLI-plus-five and final 117 reconciliation. It
cannot be combined with a partial receipt or represented as canonical credit.

## Selector-absent canonical path

`run_cgroup_default` checks the focused selector before its original body. If
the environment variable is absent, `focused_exact_selector()` returns
`None` and execution falls directly into the existing canonical sequence.

All original manifests, order, per-family count assertions, retained-111
boundary, CLI body, five CP bodies, final STEP 14-01 count and success output
remain byte-for-byte in that path. The main dispatch loop is unchanged except
for the fail-closed group constraint when the exact variable is present.

No canonical body, assertion, deadline, oracle, summary policy or 117 count is
weakened. The full-console timeout correction from iteration 20 remains active
on both canonical and focused errors.

## No generated/copy launcher

The direct selector executes the checked-in canonical runner source. It does
not generate, include or copy `fwd_guest_suite.rs`; no standalone workspace,
wrapper ELF or cache-staged generated source is required.

Historical increment-A and increment-B sources and raw receipts remain
unchanged as evidence. Their READMEs now identify them as superseded evidence
launchers and direct future focused runs to `OVERDRIVE_FWD_GUEST_EXACT`.
Nothing was deleted or rewritten out of the historical record.

This replacement simplifies the focused boundary without adding a production
or public interface.

## Source-gate evidence

The corrected runner SHA-256 is
`75dac61241c3feaaaa2b4b4e165a710074de9eaef861e2d01bd179eed99d56e9`.
Scoped Metal gate `f40221b865095b3427ad9be0` used source digest
`4687074a55157fe9298d8eff7171bfeb563956f31e57b5ffe43093faf64d1b94`
and exited 0. It verified formatting, compile/check and Clippy for the runner
with warnings denied. Its output SHA-256 is
`f7d3db52e7e8291cbc66bf9f7b0a2f42a164e17c9fef9748081809bc3d9961e3`.

The preceding lease-unavailable, rustfmt-layout and documentation-backtick
attempts were source-only and ran no test or VM. The final gate is the
controlling source evidence.

The separately owned control-plane audit diagnostics compiled under their own
owner. Their presence in the eventual matching source does not change this
selector's behavior and receives no approval here beyond preserving the exact
combined source identity at execution.

## Findings and execution disposition

| Item | Disposition |
| --- | --- |
| Surface ownership | **TEST-SUPPORT ONLY** |
| Required group | **EXACTLY `cgroup-default`; OTHERWISE FAIL-CLOSED** |
| UTF-8 validation | **REQUIRED** |
| Identity match | **FULL EQUALITY AGAINST EXISTING STEP 14-01 MANIFESTS** |
| Zero match | **FAIL-CLOSED BEFORE FOCUSED GUEST BOOT** |
| CLI binary/slot | **EXISTING `overdrive-cli` / SLOT 14** |
| CP binary/slot | **EXISTING `overdrive-control-plane` / SLOT 15** |
| Helper/payload/security/deadline/summary/SKIP/cleanup | **UNCHANGED** |
| Focused expected count | **EXACT EXISTING CELL COUNT** |
| Focused canonical credit | **EXPLICITLY ZERO** |
| Selector-absent canonical path | **ORDER, COUNTS, ORACLES AND OUTPUT UNCHANGED** |
| New test/scenario/lane/API | **NONE** |
| Generated/copied runner source | **NONE** |
| Canonical 117 | **NOT ESTABLISHED** |
| Roadmap validation | **REMAINS APPROVED** |

No bounded source finding remains.

## Direct focused execution authorization

**AUTHORIZED:** one direct canonical focused execution of the existing first
CP identity may proceed with:

```text
OVERDRIVE_FWD_GUEST_GROUP=cgroup-default
OVERDRIVE_FWD_GUEST_EXACT=integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve
```

The run must use the existing runner and matching reviewed combined source,
retain the exact zero-credit result or full failure console, and receive no
117 credit regardless of outcome.

This authorization does not permit a second focused run, a different identity,
canonical retry, new selector, prefix execution, body change, fixture
weakening, production change or mutation run. Its sole purpose is to classify
the existing first-CP failure without the retained/CLI prefix.

## Iteration 23 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | Invalid group, encoding, identity, count, summary and skip all fail closed. |
| GWT / one behavior | 10/10 | One already-authored exact body is selected without a new scenario. |
| Domain language | 10/10 | Focused test-support result and canonical 117 credit remain explicit and distinct. |
| Coverage completeness | 10/10 for selector boundary | Group, encoding, match, binaries, slots, helper, count, output and absent-selector path are covered. |
| Walking-skeleton user value | 10/10 | The same production composition and real VM helper execute. |
| Priority validation | 10/10 | Direct filtering replaces generated launchers without expanding product surface. |
| Observable behavior | 10/10 | Exact identity, pass count and zero canonical credit are printed. |
| Traceability | 10/10 | Every focused value originates from the existing manifest cell. |
| Walking-skeleton boundary | 10/10 | No new test, lane, API, fixture or source copy is introduced. |

## Iteration 23 verdict

**APPROVED.** The direct exact-body selector is private test support, requires
`cgroup-default`, validates UTF-8, accepts only a full existing STEP 14-01
identity and fails zero matches. A match reuses the existing binary, slot,
GuestKit helper, payload, security, deadline, summary/SKIP checks and cleanup,
then reports the exact count with `canonical_117_credit=0`.

With the selector absent, the canonical 117 path remains unchanged. One direct
focused execution of the existing first CP body is authorized; roadmap
validation remains approved.

# APPROVED — ONE DIRECT FIRST-CP FOCUSED EXECUTION AUTHORIZED

# Review iteration 24 — direct selector runtime evidence

## Iteration 24 metadata

| Field | Value |
| --- | --- |
| Scope | Final review of the one authorized direct canonical focused execution |
| Focused receipt | `b98b3333856892b4b3072f46`, exit 0 |
| Runtime source digest | `b30d85377f4148d2e1d54485899183c4ef5f76fe4dbb64161b1cb3d569feb8d7` |
| Runner source | `75dac61241c3feaaaa2b4b4e165a710074de9eaef861e2d01bd179eed99d56e9` |
| Cleanup | `dff177c589372cfb6c00a9df`, exit 0 |
| Focused result | **EXACT FIRST CP BODY PASSED; `canonical_117_credit=0`** |
| Canonical failure | **UNCHANGED AND UNCLASSIFIED BEYOND ITERATION 22** |
| Iteration verdict | **FOCUSED EVIDENCE APPROVED; NO REMEDY INFERRED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, acceptance body, DESIGN/DISTILL handoff, roadmap, DES event,
receipt, host state or commit was changed. No native, Lima or mutation
execution was launched by the reviewer.

Roadmap validation remains `approved`. The full canonical 117 result remains
unestablished, and the Forwarder-recovery cause from the full sequence remains
unknown.

## Exact selected identity

The durable output contains one libtest invocation:

```text
integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve
```

None of the other four CP identities appears as a test invocation, and the CLI
walking-skeleton identity is absent. The focused runner therefore selected
only the exact first CP manifest cell authorized in iteration 23.

The identity string appears once in the libtest line and once in the explicit
focused-result marker; that duplicate text is reporting, not a second body
execution.

## Exact summary, skip and process result

The one body reports exactly:

```text
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured
```

There is one successful summary, zero failed/ignored bodies and no recognized
runtime-SKIP diagnostic. No timeout, failure summary or panic is present. The
runner exits 0 by `command_returned`.

After the normal GuestKit finish path, the canonical runner emits exactly:

```text
STEP14-01-FOCUSED-RESULT identity=integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve passed=1 canonical_117_credit=0
```

The marker matches the selected identity and existing manifest count. Its
zero-credit field is explicit and unchanged.

## Ordinary GuestKit boundary

The output identifies the existing
`step14-01-focused-control-plane` outer GuestKit instance and the normal
nested-device/prerequisite sequence. The body runs through the same canonical
`run_step14_01_binary` helper, prepared control-plane integration binary, slot
15, nested payload, mounts, device/security boundary, cgroup/nofile wrapper,
exact libtest filter, 600-second bound, summary/SKIP checks and normal poweroff
cleanup approved in iteration 23.

No standalone generated launcher, alternate test binary, in-process body call,
new lane, state injection or expected product value supplies the pass.

## Canonical failure remains distinct

The matching-source full canonical receipt `727f0a30d20dc6ae67b3d3b1` remains
exit 101. After 112 visible retained/CLI passes, the same first CP identity
entered Forwarder recovery, its first real flow ended quiesced/reset, and
`wait_flowing` failed. That full-sequence primary panic remains valid evidence.

The direct focused receipt omits the retained-111 and CLI prefix and starts a
fresh focused CP GuestKit. The first body passes in that context. This
establishes only a context/schedule difference:

- direct focused context → exact first CP passes;
- full canonical sequence → exact first CP fails during Forwarder recovery.

It does not prove that the full-sequence failure was fixed, that the omitted
prefix caused recovery, that the focused schedule is canonical, or that a
production/fixture mechanism is defective. The recovery's initiating cause
remains unknown.

No retry or remediation is inferred or prescribed from the focused pass.

## 117 accounting

The direct selector bypasses canonical retained and extension reconciliation
and prints `canonical_117_credit=0`. Its one successful body cannot be added to
the canonical receipt's 112 visible summaries or substituted for the failed
first CP result.

The evidence accounting remains:

```text
focused selected bodies: 1 passed
focused canonical credit: 0
full canonical visible passes: 112
full canonical 117: not established
```

No further execution receives authorization from this review.

## Receipt and cleanup integrity

Focused run `b98b3333856892b4b3072f46` exited 0. Its 72,366-byte output
SHA-256 is
`8fa25c9ab0a9ef72b3f84522c4ff8c936dadc6e6c0d09d5276d06ac3d3b70697`;
its terminal SHA-256 is
`f3a6b87a3eecdcd7d49b04ff9644ff2b86b6b39cfca850c3d18840faa71887e2`.

Read-only cleanup `dff177c589372cfb6c00a9df` used the same source digest
and exited 0. It reports zero live Cloud Hypervisor processes, no
physical-host nft table and no STEP 14-01 work directory. The only listed
namespace is the documented pre-existing `ovd-ns-0000`. Its output SHA-256 is
`f0359a1509fdb798b5e6633052c693484d57552ee60398d48327d54df652c3a8`.

No retry, production edit, lane/API/test change, Lima run, mutation run, DES
event or commit followed.

## Findings and disposition

No DISTILL source or focused-selector finding remains.

| Item | Disposition |
| --- | --- |
| Selected identity | **EXACT FIRST CP BODY ONLY** |
| Focused summary | **1 PASSED, 0 FAILED, 0 IGNORED** |
| Runtime skips | **ZERO** |
| Process result | **EXIT 0** |
| Explicit focused marker | **MATCHED IDENTITY / PASSED=1 / `canonical_117_credit=0`** |
| GuestKit boundary | **ORDINARY CANONICAL HELPER AND CLEANUP** |
| Canonical full-sequence failure | **REMAINS A FAILURE; NOT RECLASSIFIED** |
| Focused pass contribution to 117 | **ZERO / FORBIDDEN** |
| Root cause | **NOT PROVEN** |
| Production or fixture remedy | **NOT INFERRED OR PRESCRIBED** |
| Cleanup | **PASS** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 24 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 for evidence classification | Focused success is accepted without erasing canonical failure. |
| GWT / one behavior | 10/10 | One exact existing body executes. |
| Domain language | 10/10 | Focused pass, canonical failure and 117 credit remain distinct. |
| Coverage completeness | 10/10 for selected body evidence | Identity, summary, skip, marker, exit and cleanup are durable. |
| Walking-skeleton user value | 10/10 | The same production serve/deploy and real VM helper execute. |
| Priority validation | 10/10 | No causal or remediation claim is inferred from schedule-specific success. |
| Observable behavior | 10/10 | The exact zero-credit marker and one summary are present. |
| Traceability | 10/10 | Receipt output maps directly to the existing first manifest cell. |
| Walking-skeleton boundary | 10/10 | Focused success cannot enter canonical reconciliation. |

## Iteration 24 verdict

**DIRECT SELECTOR EVIDENCE APPROVED.** Only the exact first CP identity runs;
it reports one pass, zero failures/ignored/runtime skips, exit 0, the explicit
`canonical_117_credit=0` marker and complete cleanup through the ordinary
GuestKit boundary.

The pass does not fix, classify or replace the full-sequence canonical
failure and contributes nothing to 117. No cause, remedy or additional
execution is prescribed. Roadmap validation remains approved.

# DIRECT FOCUSED BODY PASSED — CANONICAL FAILURE AND 117 REMAIN

# Review iteration 25 — private runner responsibility names

## Iteration 25 metadata

| Field | Value |
| --- | --- |
| Scope | Bounded private helper naming correction |
| Current runner | `6921a47007a208b9106727ab3b7302737c252b517f166874221626622db14a65` |
| Scoped source gate | `a239d7d9a06795bda7277c47`, exit 0 |
| Scoped source digest | `8f16c0d2ebe7b414db768c78b4e93615c6efe3e1100ee93fc27ab0dbf6f892f1` |
| Native execution | None |
| Iteration verdict | **APPROVED** |

The reviewer changed only this review artifact. No production, test-support,
canonical runner, spike source, acceptance body, DESIGN/DISTILL handoff,
roadmap, DES event, receipt, host state or commit was changed. No native, Lima
or mutation execution was launched.

Roadmap validation remains `approved`. The canonical first-CP failure and full
117 result remain unchanged and unestablished respectively. This iteration
reviews names only.

## Responsibility mapping

The three renamed helpers now state their actual reusable responsibility:

| Former private name | Current private name | Actual responsibility |
| --- | --- | --- |
| `step14_01_nested_support` | `nested_vm_test_support` | Stage one outer GuestKit guest with the nested production-serve kernel, module, tool, BPF, VM and filesystem substrate |
| `run_step14_01_binary` | `run_nested_vm_test_binary` | Run one prepared integration binary and selected existing cells inside that nested-VM-capable cgroup-default guest |
| `run_step14_01_focused_exact` | `run_focused_exact_nested_vm_body` | Resolve one exact existing nested-VM manifest cell and execute it with explicit zero canonical credit |

The names accurately describe what the functions do without implying that the
implementation itself belongs only to roadmap step 14-01. The surrounding docs
now describe nested production-serve integration binaries and the existing
nested-VM-capable cgroup-default member rather than naming a roadmap step as
the helper's responsibility.

## Caller and documentation audit

Every current runner caller uses the new names:

- `run_nested_vm_test_binary` calls `nested_vm_test_support`;
- the direct exact-body path calls `run_focused_exact_nested_vm_body`, which
  calls `run_nested_vm_test_binary`;
- the canonical STEP 14-01 CLI invocation calls
  `run_nested_vm_test_binary`;
- the canonical STEP 14-01 control-plane invocation calls
  `run_nested_vm_test_binary`; and
- increment-A and increment-B historical wrappers call
  `run_nested_vm_test_binary`.

A repository search outside historical review/handoff prose and raw evidence
finds no current reference to a removed helper name. Current handoff and spike
README text use the responsibility names.

The historical review artifact deliberately retains earlier names in the
iterations that reviewed those exact source identities. The new handoff table
records the rename. This preserves chronological evidence instead of
rewriting past reviews as though the new names existed then.

## Frozen STEP 14-01 attribution

The following constants retain STEP 14-01 names:

- `STEP14_01_CONTROL_PLANE`;
- `STEP14_01_CLI`;
- `STEP14_01_CELL_BOUND`;
- `STEP14_01_CONTROL_PLANE_BODIES`;
- `STEP14_01_CLI_BODIES`; and
- `STEP14_01_NATIVE_BODIES`.

That attribution is correct. These constants define the accepted STEP 14-01
manifest, per-cell execution bound and count reconciliation. They are delivery
inventory, not reusable operation names. Renaming them would erase useful
contract provenance rather than improve responsibility naming.

The focused result marker and cgroup-default count diagnostics likewise retain
STEP 14-01 attribution because they report that frozen manifest's credit.

## Historical spike evidence

Increment A and increment B remain explicitly marked as historical,
superseded evidence launchers. Their source-transforming wrappers now name the
current private `run_nested_vm_test_binary` helper and no longer depend on a
removed identifier. Their main-module docs use the same name.

The README warnings still direct all new focused execution to the canonical
`OVERDRIVE_FWD_GUEST_EXACT` path. No historical raw receipt was removed,
reclassified or regenerated. Earlier source hashes and review descriptions
remain in the chronological handoff/review record, so the old-name evidence is
still honest about the source that produced it.

Updating the retained wrappers for the user-directed private rename keeps the
historical launchers intelligible against current canonical source; it does not
turn them back into an approved execution path.

## Behavioral identity audit

The helper signatures, parameters, return types, bodies and call order are
unchanged apart from identifier/doc text:

- `nested_vm_test_support` stages the same exact tools, kernel/init,
  Cloud Hypervisor, BPF, nested module closure, resolver and XFS volume in the
  same order;
- `run_nested_vm_test_binary` prepares the same package/target, creates the
  same support/work directories, boots the same eight-vCPU outer guest, runs
  the same module/device/mount/reflink prerequisites, executes the same cells
  with `STEP14_01_CELL_BOUND`, then uses the same finish and cleanup path; and
- `run_focused_exact_nested_vm_body` performs the same CLI-then-CP exact lookup,
  uses the same package, slot, name and selected existing cell, checks the same
  expected count and prints the same zero-credit marker.

No branch, manifest, payload, module path, slot, selector, exact filter,
deadline, summary/SKIP check, assertion, expected count, output string,
cleanup or error mapping changed. The selector-absent canonical path and
selector-present focused path are behaviorally identical to iteration 24.

All helpers remain private. No product or public test-support API changes.

## Source-gate evidence

The current runner SHA-256 is
`6921a47007a208b9106727ab3b7302737c252b517f166874221626622db14a65`.
Scoped Metal gate `a239d7d9a06795bda7277c47` used source digest
`8f16c0d2ebe7b414db768c78b4e93615c6efe3e1100ee93fc27ab0dbf6f892f1`
and exited 0. It verified formatting, compile/check and Clippy with warnings
denied. Its output SHA-256 is
`908bdac9ed2f3115395933247b132bec2cde390a59b620114e16181bfcbccc6d`.

No test or VM ran in the gate. Naming alone receives no behavioral or 117
credit and requires no native replay.

## Findings and disposition

No bounded naming or source finding remains.

| Item | Disposition |
| --- | --- |
| `nested_vm_test_support` | **ACCURATELY NAMES STAGING RESPONSIBILITY** |
| `run_nested_vm_test_binary` | **ACCURATELY NAMES EXECUTION RESPONSIBILITY** |
| `run_focused_exact_nested_vm_body` | **ACCURATELY NAMES FOCUSED RESPONSIBILITY** |
| Current callers/docs | **UPDATED** |
| Frozen STEP 14-01 manifest/count names | **CORRECTLY RETAIN ATTRIBUTION** |
| Historical reviews/raw evidence | **PRESERVED AND HONEST** |
| Increment-A/B old-name dependency | **REMOVED; LAUNCHERS REMAIN SUPERSEDED** |
| Behavior, payload, slots, selector, deadline and counts | **UNCHANGED** |
| Output and cleanup | **UNCHANGED** |
| Public API / new lane / new test | **NONE** |
| Native replay | **NOT REQUIRED OR PERFORMED** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 25 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Domain language | 10/10 | Names state nested-VM responsibilities while manifest constants retain delivery provenance. |
| Traceability | 10/10 | Former-to-current mapping and every caller are explicit. |
| Scope discipline | 10/10 | Only private identifiers and their documentation changed. |
| Behavioral preservation | 10/10 | Signatures, bodies, branches, payload and outputs remain unchanged. |
| Evidence integrity | 10/10 | Historical names remain in chronological reviews and raw receipts are untouched. |
| Walking-skeleton boundary | 10/10 | No API, selector, lane, test or production path changed. |

## Iteration 25 verdict

**APPROVED.** The private helpers now name their nested-VM staging, execution
and focused-selection responsibilities accurately. All current callers and
docs use the new names, while frozen STEP 14-01 manifest/count constants and
historical evidence retain correct attribution.

No behavior, output, payload, slot, deadline, assertion, count, cleanup, lane,
test or API changed. No native execution is needed; roadmap validation remains
approved.

# APPROVED — PRIVATE RESPONSIBILITY NAMES ONLY

# Review iteration 26 — SharedAudit overlap testability proposal

## Iteration 26 metadata

| Field | Value |
| --- | --- |
| Scope | Read-only assessment of a proposed seeded regression and private scheduling boundary |
| Latest canonical receipt | `69697656241418f6e7342b7c`, exit 101 |
| Receipt source digest | `985832cb016a0724de91451db58015244f58d0579994ad666a89443ea541726c` |
| Receipt output SHA-256 | `f0c5c35ad9626aa91f8b5de5ab6569846550c048be82dc4f6fc2bc803766fcb6` |
| Proposed test/checkpoint status | **NOT IMPLEMENTED** |
| User authorization | **NOT GRANTED** |
| Production defect | **HYPOTHESIS ONLY; NOT ACCEPTED BEFORE A FAILING SEEDED INVARIANT** |
| Production remedy | **NONE PROPOSED OR APPROVED** |
| Iteration verdict | **PROPOSAL NEEDS BOUNDED TESTABILITY CORRECTION BEFORE APPROVAL REQUEST** |

The reviewer changed only this review artifact. No production, test-support,
runner, acceptance body, DESIGN/DISTILL handoff, roadmap, DES event, receipt,
host state or commit was changed. No native, Sim, Lima or mutation execution
was launched.

Roadmap validation remains `approved`. Historical `727f…` and `af7d…`
failures remain unresolved. This review does not accept a new control-plane
defect or authorize a remedy from the proposal.

## Reproduced native overlap and its exact limit

The latest canonical run visibly retains 114 passed bodies, zero failed
summaries, zero ignored and zero runtime skips before the third CP command
fails. Its full-console diagnostics identify one typed owner audit mismatch:

```text
operation = SharedAudit
expected = Forwarder { generation: 1, live_pairs: 0, live_watches: 0 }
observed = Forwarder { generation: 1, live_pairs: 0, live_watches: 1 }
```

The same trace records a successful host probe dial returning `Ok(GuestStream)`
before the mismatch and the stream's drop immediately afterward. The
supervisor then enters Forwarder recovery. This is real production-owner and
real kernel-inventory evidence of the overlap; it is not a fabricated
component fault.

The receipt does not by itself satisfy the workspace's timing/concurrency
gate for a control-plane finding. It lacks a seeded production-supervisor/real-
owner invariant that deterministically schedules the same overlap and fails
against current source. Therefore:

- the stale expected-count versus current inventory race is a supported
  hypothesis;
- a false-positive recovery remains unaccepted as a production finding; and
- no locking, retry, resnapshot or recovery remedy may be selected yet.

## Actual production owner path

`HostSharedGuestNetworkOwner::audit_shared` performs these current steps:

1. snapshot `repairs_in_flight`;
2. lock owner state and copy `pairs.len()` / `dial_watches.len()` as expected
   counts;
3. release the owner-state lock;
4. read `forwarder_generation` in a separate lock;
5. synchronously call `GuestFlowIo::inventory()`; and
6. compare that inventory with the earlier expected counts, returning typed
   `PostconditionMismatch { operation: SharedAudit }` on inequality.

A normal host dial concurrently performs:

1. insert `DialPhase::AwaitingReply` under owner state;
2. call `io.watch(V_h)`;
3. insert the returned watch into `state.dial_watches`;
4. write the guest request and await `Paired`;
5. replace the dial phase with `Paired`; and
6. return a live `GuestStream`.

`io.watch` necessarily precedes owner watch registration. The audit necessarily
releases owner state before it calls inventory. A normal healthy probe can
therefore change the owner and forwarder facts between the two audit reads.
The proposed regression must drive this exact owner/supervisor/dial path; a
script that merely injects an inventory value would be a test-only unreachable
state.

## Existing test-support coverage gap

The existing composed rig is the correct high-level universe: it boots the
production `HostSharedGuestNetworkOwner` and production shared-network
supervisor over `SimGuestFlowIo`, `SimClock` and the real `OwnerDial` binding.
It does not currently expose the required scheduling point.

The current support cannot deterministically create the overlap:

- `SimGuestFlowIo::inventory()` is synchronous. `SimEffectKind::Inventory` is
  explicitly classified as a synchronous effect, and `hold(Inventory)`
  asserts `test bug` rather than parking it.
- The existing DNS audit gate parks the full audit before
  `shared_guest_network.audit_shared()` begins. It cannot pause after the
  owner expected-count snapshot.
- `TestSharedOwner` scripts generic component outcomes and holds no real owner
  dial/watch lifecycle. It cannot discharge this hypothesis.
- The R5-58 seeded bodies damage tasks or script inventory/repair failures.
  They do not overlap a healthy successful host Probe with a production owner
  audit.
- The R5-61 and `quint_fwd_pair` recovery bodies exercise settled failure,
  abort and reopen order. They do not schedule a healthy watch creation
  between expected-count capture and inventory read.

A new seeded regression is therefore necessary if this timing hypothesis is
to become an accepted finding. Existing support cannot express it honestly.

## Proposal issue P-01 — a synchronous Sim inventory checkpoint can deadlock

The proposal places a private Sim-only checkpoint at inventory entry, then
asks the test to complete a normal host Probe while inventory is paused.
That location is unsafe in the current composed rig.

`SimGuestFlowIo::inventory()` is a synchronous trait call. The recovery
acceptance bodies run on a current-thread Tokio runtime with paused time. If
that synchronous call blocks on a semaphore, condition variable or other
release signal, the runtime thread cannot poll:

- the test driver that would release the checkpoint;
- the host probe dial future;
- the production owner's control-session task that must receive `Paired`; or
- the production supervisor that owns the audit.

The result is a fixture deadlock, not the production schedule. Marking
`Inventory` holdable through the existing `SimEffectKind` gate is also invalid:
`hold` deliberately rejects every synchronous forwarding call, and changing
that rule would generalize the Sim effect model beyond the one required
boundary.

A nonblocking inventory hook that directly inserts a watch or returns scripted
inventory would avoid the deadlock only by manufacturing state outside the
normal host-dial lifecycle. That would not prove reachability through the
production owner path.

P-01 is a proposal-boundary defect. No code has been authored, so it is not an
implementation finding.

## Smallest safe corrected boundary

The minimum honest scheduling point is an **async, test-only audit checkpoint
immediately after expected counts and generation are captured and before the
synchronous `io.inventory()` call**.

Required shape:

1. The checkpoint is compiled only for tests and is disarmed by default.
2. It is private/crate-local to the production owner test support; it adds no
   public or cross-crate trait method, constructor parameter or production
   configuration.
3. It is reached after all owner locks used for expected-count capture are
   released. It must never await while holding owner state.
4. It exposes a cancellation-safe `reached` notification and one release
   permit. Dropping a bounded audit future retires its pending marker rather
   than leaking or blocking later tests.
5. In non-test builds, the checkpoint and await do not exist. No production
   scheduling, timeout or error mapping changes.

This checkpoint belongs at the async owner audit boundary, not inside the
synchronous Sim inventory effect. A small `#[cfg(test)]` crate-private control
on `HostSharedGuestNetworkOwner`/its runtime is sufficient. No new
`SharedGuestNetworkOwner` method, `GuestFlowIo` method, `SimEffectKind`, public
API or architectural mechanism is warranted.

The test-only checkpoint is DISTILL fixture surface, but it is still a new
checkpoint and requires the user's explicit approval before authoring.

## Smallest seeded regression body

One new source-local body beside `shared_network_recovery_acceptance` is the
smallest adequate regression. It must use the existing `ComposedNode`,
production owner, production supervisor and normal Sim guest lifecycle.

The body should:

1. Print the seed and retain an action sequence. Reuse the existing seeded
   helper; let the seed choose a bounded offset within the one-second audit
   period so the triggering trace is reproducible.
2. Boot `ComposedNode` and call the existing `ensure_active_cid`, producing an
   Active allocation with a live control session and declared listener.
3. Arm the private post-snapshot/pre-inventory checkpoint.
4. Advance the injected clock until the production supervisor starts its
   normal audit, then wait for the checkpoint's `reached` signal. At this point
   expected pair/watch counts are captured and no owner lock is held.
5. Through the existing composed `OwnerDial`, dial the Active allocation's
   declared port with `DialPurpose::Probe`. Await the ordinary successful
   `GuestStream` and **retain it live**; do not `map(drop)`. This must execute
   normal `connect_guest → watch → request → Paired` behavior through
   `SimGuestFlowIo`.
6. Release the audit checkpoint and settle the production supervisor.
7. Assert the safety outcome: the healthy overlap creates no Forwarder
   recovery episode, no quiescence hold and no `Quiesced` admission; the
   settled owner and forwarder each show exactly one Probe watch while the
   stream is held.
8. Drop the stream normally, settle, and assert the exact owner/forwarder counts
   return to zero without recovery or quiescence.

Against the current implementation, step 7 is expected to fail after the audit
compares stale expected zero with current inventory one. The failure must print
its seed and action sequence. That RED is the prerequisite for accepting a
control-plane defect.

No reusable `successful_host_probe` helper is necessary: the body can call the
existing `process().dial`/`OwnerDial` directly and retain the returned stream.
If readability warrants a helper, it must remain private and return the live
`GuestStream`; a helper that drops it before audit release would erase the
required overlap.

## Exact invariant and evidence boundaries

The proposed invariant is outcome-only:

```text
A healthy successful Probe watch that overlaps SharedAudit does not cause
Forwarder recovery or quiescence; while held, owner and inventory counts are
exactly one; after normal drop, both are exactly zero.
```

It does not prescribe whether a future remedy uses resnapshot, serialization,
retry or another already-approved internal technique. It adds no public
linearization contract. The regression observes the existing supervisor
recovery projection, owner observation and Sim forwarder inventory.

The test must not:

- inject `ForwarderInventory` directly;
- mutate owner expected counts;
- call a generic scripted owner in place of the production owner;
- add a production timeout/retry or public checkpoint;
- add a new lane or external scenario; or
- assert a specific remedy or internal lock order.

The body is a new seeded scenario and the checkpoint is new test support. Both
require explicit user approval. Neither is currently authorized.

## Proposal disposition

| Item | Disposition |
| --- | --- |
| Native expected/observed mismatch | **REPRODUCED, BUT TIMING DEFECT NOT YET ACCEPTED** |
| Existing seeded coverage | **INSUFFICIENT FOR HEALTHY PROBE/AUDIT OVERLAP** |
| One new seeded real-owner/supervisor body | **NECESSARY, NOT AUTHORIZED** |
| Proposed synchronous Sim inventory checkpoint | **REJECTED: DEADLOCK / UNREACHABLE-STATE RISK** |
| Corrected async post-snapshot/pre-inventory checkpoint | **MINIMAL SUFFICIENT TESTABILITY BOUNDARY, NOT AUTHORIZED** |
| Private successful Probe helper | **NOT NECESSARY; BODY-LOCAL OWNERDIAL CALL IS SMALLER** |
| Public/cross-crate API or trait change | **FORBIDDEN AND UNNECESSARY** |
| Production behavior/mechanism change | **NONE AUTHORIZED** |
| Accepted control-plane defect | **NONE BEFORE FAILING SEEDED INVARIANT** |
| Production remedy | **NONE BEFORE FAILING SEEDED INVARIANT** |
| Historical `727f…` / `af7d…` | **REMAIN UNRESOLVED** |
| Proposal implementation | **NOT IMPLEMENTED** |
| User approval | **NOT GRANTED** |
| Roadmap validation | **REMAINS APPROVED** |

## Iteration 26 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Reachability | 10/10 native / pending seeded | The real overlap is native-reachable; deterministic Sim reproduction is missing. |
| Production-path fidelity | 10/10 corrected proposal | Production owner, supervisor, OwnerDial and normal watch transition remain in path. |
| Synchronization safety | 4/10 original / 10/10 corrected | Sync inventory parking deadlocks; async lock-free audit checkpoint is safe. |
| Scope discipline | 10/10 | One source-local seeded body and one test-only private checkpoint suffice. |
| API discipline | 10/10 corrected | No public, cross-crate or trait surface is needed. |
| Remedy neutrality | 10/10 | The invariant specifies outcome only and selects no production repair. |
| Evidence honesty | 10/10 | No defect or remedy is accepted before a failing seeded trace. |

## Iteration 26 verdict

**PROPOSAL NEEDS BOUNDED CORRECTION AND USER APPROVAL.** A seeded regression
through the production owner and supervisor is necessary. The proposed
healthy Probe overlap is the right outcome boundary, but a synchronous
Sim-inventory checkpoint is unsafe and can only deadlock or fabricate state in
the current current-thread rig.

Use one test-only async checkpoint after owner expected-count capture and
before synchronous inventory, with no locks held, then retain a normal
`OwnerDial` Probe stream across audit release. This is the smallest honest
boundary. It is not implemented or authorized. No production defect or remedy
is accepted until that seeded invariant fails. Roadmap validation remains
approved.

# NOT IMPLEMENTED — NOT AUTHORIZED — NO DEFECT OR REMEDY ACCEPTED

# Review iteration 27 — user-authorized SharedAudit overlap regression

## Iteration 27 metadata

| Field | Value |
| --- | --- |
| Review scope | Implementation of iteration 26's corrected testability boundary and one seeded healthy-Probe/SharedAudit overlap regression |
| Source status | User-authorized DISTILL test support and acceptance source; no production repair is included |
| Source gate | Metal `6dc8a7911ddc7dd9cb4aeb43`, exit 0 |
| Seeded regression receipt | Metal `79a67fcb03ed908614b732a4`, intended RED, command exit 1 / exact test exit 101 |
| Exact failing seed | `11400714819323198485` |
| Generated audit offset | `989 ms` within the one-second audit period |
| Verdict | **APPROVED — TESTABILITY BOUNDARY AND SEEDED RED ARE HONEST** |

This iteration reviews only the user-approved source required to reproduce the
healthy Probe/periodic-audit overlap deterministically. It does not review,
request or authorize a production repair. The reviewer changed only this
review artifact and ran no source gate, test, native workload, mutation,
roadmap/DES operation or commit.

## Reviewed source identity

The failing regression receipt records the complete matching source identity:

| Artifact | SHA-256 |
| --- | --- |
| `crates/overdrive-control-plane/src/guest_network.rs` | `85a5ea33ffafbac8de8df647703ee5a8f2a68e76ba34292b91569c7556617a98` |
| `crates/overdrive-control-plane/src/guest_network/flow_owner.rs` | `0277c865f108806c601efce09831a73203b1d06afae07abac8dda56f78c6d783` |
| `crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs` | `f16a377bec26ddfaba4022d64426513831377b25a899244c09040c8b9bce6173` |
| `crates/overdrive-control-plane/src/lib.rs` | `4d16ae54eb7710bb310f1725e3d440af196ddbca541cfdc9ac2b9c16d71f66cd` |

The `lib.rs` overlap from this increment is rustfmt-only signature layout. Its
existing crafter-owned audit and shutdown diagnostic events remain present
with their prior behavior. Neither the checkpoint nor the new body references
or changes that diagnostic path.

## Checkpoint boundary review

The implementation matches the safe boundary prescribed in iteration 26.

- `SharedAuditCheckpoint`, its pending wait record and
  `SharedAuditCheckpointControl` are all guarded by `#[cfg(test)]` in
  `guest_network.rs:712-758`. The control is at most `pub(crate)` and no
  public trait, constructor, port, cross-crate API or production
  configuration changed.
- `OwnerRuntime` carries and initializes the checkpoint only under
  `#[cfg(test)]` (`flow_owner.rs:54-68`). Its default mutex state is `None`,
  so it is disarmed unless a test explicitly arms it. The call inside
  `audit_shared` is also compiled only for tests. A non-test build has no
  checkpoint field, wait or scheduling effect.
- `audit_shared` clones `repairs_in_flight`, captures exact pair/watch counts,
  explicitly drops the state guard, then reads `forwarder_generation` through
  a temporary guard that ends at the statement boundary. Only after those
  values are complete does it await the checkpoint
  (`guest_network.rs:1316-1330`). The synchronous `io.inventory()` remains
  after release. No owner-state, task or fixture mutex is held across the
  await.
- Arming installs one reached sender and one release receiver. On reaching
  the checkpoint, `pause_if_armed` first removes the pending record from the
  mutex, then signals and awaits without a lock. Dropping the control closes
  the release sender and wakes the waiter. Cancelling the audit after it has
  taken the record drops the release receiver while the armed slot is already
  empty; a later `release` returns false rather than leaving a parked owner.
  Thus cancellation cannot leak a held checkpoint or block a later audit.
- The checkpoint is at the production owner's async audit boundary. It does
  not block the synchronous Sim inventory call, inject inventory state or add
  a new `SimEffectKind`. It therefore avoids the deadlock and unreachable
  state rejected in iteration 26.

No checkpoint source finding remains.

## Seeded body and production-path fidelity

Exactly one new source-local body was added beside the existing R5-58 seeded
acceptance bodies:
`a_healthy_probe_watch_overlapping_shared_audit_does_not_start_recovery`.
It carries the required `/// CONTRACT_SHAPE: bounded-change.` declaration.
The only adjacent source changes are the imports needed for the existing
control message, connection key, dial phase and vsock-port types.

The body uses the existing `ComposedNode`. That rig boots the production
`HostSharedGuestNetworkOwner`, production shared-network supervisor and the
boot-bound `OwnerDial`; only the owner effects, clock and other existing
ports are simulated. The stimulus is the normal path:

1. boot the composed node and establish one Active allocation/control
   session;
2. use the seeded offset within the real audit period;
3. arm the post-snapshot/pre-inventory checkpoint and let the periodic
   supervisor audit reach it;
4. call the existing `OwnerDial::dial` for the allocation's declared port
   with `DialPurpose::Probe`;
5. observe the ordinary emitted TCP-accept request and send the ordinary
   `ControlMessage::Paired` on that allocation's control session;
6. retain the returned live `GuestStream`, release the audit and settle the
   production supervisor; and
7. drop the stream normally only after the held-state oracle.

The body neither inserts an inventory row directly nor scripts an owner
result. The stream remains in scope across audit release. This exercises the
actual `connect_guest -> io.watch -> owner registration -> request -> Paired`
lifecycle against the owner snapshot/inventory comparison identified in
iteration 26.

The oracle is outcome-only. While the Probe is held, it requires no Forwarder
recovery episode, zero quiescence holds, `GuestAdmission::Open`, exactly one
owner-side paired Probe watch and exactly one forwarder inventory watch. After
normal stream drop it requires no recovery, zero holds, open admission and
both counts exactly zero. It does not assert resnapshotting, locking, retry,
serialization or any other possible repair. No selector, lane, public API,
external scenario, production timeout or production mechanism was added.

## Verification and reproducible RED

Source gate `6dc8a7911ddc7dd9cb4aeb43` ran `cargo fmt --all -- --check`,
`cargo check -p overdrive-control-plane --features integration-tests --lib`
and matching scoped Clippy with warnings denied on Metal. It exited 0. This
checks both the production compile boundary, where `cfg(test)` checkpoint
source is absent, and the integration-feature library source. The reviewed
diff is whitespace-clean.

Matching-source regression run `79a67fcb03ed908614b732a4` then established
three separate facts:

| Evidence | Result |
| --- | --- |
| Existing owner-dial watch/request body | PASS, 1 passed / 0 failed / 0 ignored |
| Existing seeded component-exit audit body | PASS for its eight printed seeds, 1 passed / 0 failed / 0 ignored |
| New overlap body at exact seed `11400714819323198485` | RED, exact test exit 101 |

The new body prints the seed and complete action sequence. With the generated
`989 ms` offset it reaches the checkpoint, pairs and retains
`FlowId(2147483649)`, then releases the audit. The exact failing comparison is:

```text
observed: (Some(RecoveryEpisode { phase: Held, failed_attempts: 0 }),
           1, Quiesced, 0, 0)
expected: (None, 0, Open, 1, 1)
```

The receipt therefore satisfies the repository's seeded-Sim prerequisite for
this timing-sensitive control-plane finding. It reproduces the claimed safety
invariant failure through the current production owner, production supervisor
and normal OwnerDial lifecycle. The passing existing bodies show that the
new deterministic scheduling boundary did not break the established dial or
component-exit audit fixtures.

This is intentionally semantic RED. It earns no product GREEN, native-117 or
runtime-skip credit, and it does not identify or approve a production remedy.

## Iteration 27 findings and dispositions

| ID | Severity | Finding | Disposition |
| --- | --- | --- | --- |
| I27-01 | None | The async post-snapshot/pre-inventory checkpoint is test-only, crate-local, disarmed by default, lock-free across its await and cancellation-safe. | **APPROVED** |
| I27-02 | None | The one seeded body drives the production owner/supervisor and normal OwnerDial Probe lifecycle, retains the live stream and asserts exact held/drop outcomes without prescribing a repair. | **APPROVED** |
| I27-03 | Evidence | Exact seed `11400714819323198485` reproducibly violates the outcome, while the two adjacent existing tests pass. | **HONEST SEMANTIC RED; NECESSITY ESTABLISHED** |
| I27-04 | Scope | No public/cross-crate API, production behavior, lane, selector, scenario family or mechanism changed; the `lib.rs` touch is formatting-only for pre-existing diagnostics. | **APPROVED** |
| I27-05 | Product implementation | No production correction is part of this iteration. | **UNREVIEWED; NO REMEDY REQUESTED OR AUTHORIZED HERE** |

## Iteration 27 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Reachability | 10/10 | Exact seed and sequence fail through the current composed production owner/supervisor path. |
| Checkpoint safety | 10/10 | Async, one-shot, no lock across await, cancellation retires or closes the wait. |
| Production-path fidelity | 10/10 | Existing ComposedNode, real owner, real supervisor and normal OwnerDial Probe lifecycle are retained. |
| Oracle quality | 10/10 | Recovery, hold, admission and exact owner/inventory complements are fail-closed. |
| Remedy neutrality | 10/10 | The body states only the accepted safety outcome and exact counts. |
| API/scope discipline | 10/10 | All new support is cfg-test and crate-local; production compilation and behavior are unchanged. |
| Evidence honesty | 10/10 | Intended RED is reported as RED, with no GREEN or 117 credit. |

## Iteration 27 verdict

**APPROVED.** The user-authorized test-only async checkpoint is the minimum
safe scheduling boundary identified in iteration 26. It is crate-local,
disarmed by default, reached only after the owner snapshot and generation are
captured with no lock held, and absent from production builds. The single new
seeded body retains a normal successful Probe stream across the production
supervisor's audit release and preserves exact held/drop outcome oracles.

Matching-source Metal evidence reproducibly fails at seed
`11400714819323198485` with Forwarder recovery, one quiescence hold,
`Quiesced` admission and zero surviving watch counts, while both adjacent
existing acceptance bodies pass. That is the required honest seeded safety
RED. This review approves the DISTILL testability implementation and
regression necessity only; no production remedy is selected, requested or
approved.

# Review iteration 28 — seeded-fixture process cleanup

## Iteration 28 metadata

| Field | Value |
| --- | --- |
| Review scope | One private fixture lifecycle fence after all existing overlap-body assertions |
| Pre-correction receipt | Metal `918b8c78ca620f96ad184c0c`, exit 0 with eight seam-leak panic-hook reports |
| Corrected verification | Metal `bdfb7d8e8df254218c78958a`, exit 0 |
| Corrected fixture identity | `df15ced7c53742fcb90677b1c8c6b1419683cfe361b9995683e24b6c66e0d5f8` |
| Current production audit identity | `3c1b9411d8d54abb2c610b3e66ce743177c0b7042f60b7cc474a88bc9a741173` (outside this review) |
| Verdict | **APPROVED — EXACT PRIVATE LIFECYCLE CLEANUP** |

This iteration reviews only the fixture-owned cleanup added to
`a_healthy_probe_watch_overlapping_shared_audit_does_not_start_recovery`.
The current production audit correction is owned by the crafter and is not
approved, rejected or otherwise classified here. The reviewer changed only
this Markdown artifact and ran no source command, test, Metal workload,
roadmap/DES operation or commit.

## Reproduced cleanup defect

Crafter run `918b8c78ca620f96ad184c0c` used fixture SHA-256
`f16a377bec26ddfaba4022d64426513831377b25a899244c09040c8b9bce6173`
and exited 0. Its three exact tests each reported one pass, zero failures and
zero ignores. The overlap body ran all eight default seeds and completed its
behavioral assertions, but the log then emitted exactly eight copies of:

```text
owner defect: 1 seam handle(s) dropped without abort / close or a consuming call
```

Each report is paired with a `panicked at .../flow_io/sim.rs:1788:9` panic-hook
line. The nominal exit 0 does not erase those reports: they show that each
seed iteration dropped its still-live composed process rather than ending it
through the fixture lifecycle.

The source explains the exact single handle. `ensure_active_cid` establishes
the allocation's production-owner control session, whose Sim seam endpoint is
a process-scoped `RecordSocket`. The body already drops the returned Probe
stream and proves both dial-watch universes are zero, but that does not end
the allocation's control session. Before this correction the loop then let
the live `ComposedNode` fall out of scope. `SimGuestFlowIo::drop` counts the
remaining handles of every non-exited process, invokes its fallback exit path,
then asserts the captured count (`flow_io/sim.rs:1779-1791`). The one report
per seed and the one intentional live control session agree exactly; no
unrelated handle
or behavioral assertion is implicated.

## Exact correction and lifecycle ownership

The source adds exactly one statement after the existing held-state and
post-drop assertions:

```rust
node.crash().await;
```

Removing that single line from the current fixture bytes reconstructs the
iteration-27 fixture SHA-256 exactly:
`f16a377bec26ddfaba4022d64426513831377b25a899244c09040c8b9bce6173`.
The current bytes hash to
`df15ced7c53742fcb90677b1c8c6b1419683cfe361b9995683e24b6c66e0d5f8`.
This byte-level complement proves that no seed, offset, checkpoint, Probe
stimulus, schedule, behavioral assertion, expected value or Contract Shape
declaration changed in the fixture.

`ComposedNode::crash` is the existing process owner for this exact rig. It:

1. takes the live `Process` out of the node;
2. drops the fixture's owner and `OwnerDial` references;
3. calls `ServerHandle::kill_for_test` and awaits the abort/join fence for
   server-owned tasks; and
4. only then calls `SimGuestFlowIo::exit_process`, which consumes every
   process-scoped handle and held effect.

The ordering is important: the seam remains live until server-owned tasks can
no longer use it, then its process-exit operation consumes the intentional
control-session `RecordSocket`. The correction uses no panic catch,
suppression flag, relaxed seam assertion, direct handle deletion or new
fixture path. It closes the existing process through its existing lifecycle
owner.

Because the call is after both exact outcome assertions and there is no later
observation, it cannot make either held-state or post-drop behavior pass. An
assertion failure still fails at its original site; normal successful
iterations now end their composed process before beginning the next seed.

## Scope and source separation

The verification receipt records these matching identities:

| Artifact | SHA-256 |
| --- | --- |
| Production `guest_network.rs` | `3c1b9411d8d54abb2c610b3e66ce743177c0b7042f60b7cc474a88bc9a741173` |
| Test-only owner runtime support | `0277c865f108806c601efce09831a73203b1d06afae07abac8dda56f78c6d783` |
| Corrected recovery acceptance fixture | `df15ced7c53742fcb90677b1c8c6b1419683cfe361b9995683e24b6c66e0d5f8` |
| Crafter diagnostic `lib.rs` | `4d16ae54eb7710bb310f1725e3d440af196ddbca541cfdc9ac2b9c16d71f66cd` |

Only the fixture identity differs from iteration 27 by the proved one-line
cleanup. The current production audit source has changed since iteration 27
under separate crafter ownership; this review does not attribute the now-green
overlap outcome to the cleanup or approve that production change. The cleanup
claim is narrower: the body already passed all assertions in `918b8c…`, then
reported eight lifecycle defects; the final one-line fence ends those live
processes without touching their assertions.

No production type, trait, API, owner, schedule, checkpoint, timeout, selector,
lane or test inventory changed as part of this fixture correction.

## Corrected verification

Metal run `bdfb7d8e8df254218c78958a` ran the following against the recorded
source identities:

- workspace rustfmt check;
- `overdrive-control-plane` integration-feature library check;
- scoped Clippy with warnings denied; and
- the exact existing owner-dial test, existing seeded component-exit audit
  test and corrected seeded overlap test.

The run exited 0. Each of the three exact tests reported 1 passed, 0 failed
and 0 ignored. The overlap body printed all eight established seeds and their
same offsets, including seed `11400714819323198485` at `989 ms`. The complete
verification log contains zero `owner defect:` matches and zero `panicked at`
matches. Its SHA-256 is
`a038bad732a267a30ceccfebe8a8214810bc62bc7010d66dbfdd2aad73520bf9`.

This evidence is sufficient for the bounded cleanup: compilation and lint are
clean, all affected fixture bodies complete, and the exact leak-audit symptom
is absent across every seeded iteration. The current green overlap behavior
belongs to the separately owned production audit remediation and receives no
product approval from this review.

## Iteration 28 findings and dispositions

| ID | Severity | Finding | Disposition |
| --- | --- | --- | --- |
| I28-01 | None | The correction is exactly one `node.crash().await` after every existing behavioral assertion. | **APPROVED** |
| I28-02 | None | `ComposedNode::crash` is the existing process lifecycle fence: abort/join server tasks, then consume process-scoped seam handles. | **APPROVED** |
| I28-03 | None | No panic suppression, seam-audit relaxation, direct handle mutation or new cleanup mechanism was introduced. | **APPROVED** |
| I28-04 | Evidence | Pre-correction all-eight-seed run emitted eight one-handle defects; corrected all-eight-seed verification emits none and all three exact tests pass. | **CLOSED** |
| I28-05 | Scope | Current production audit remediation is present at hash `3c1b9411…` but was not edited or reviewed as part of this cleanup. | **OUTSIDE THIS REVIEW** |

## Iteration 28 acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Cleanup ownership | 10/10 | The existing ComposedNode process owner performs the fence and seam exit. |
| Behavioral preservation | 10/10 | Byte-level reconstruction proves the sole fixture change follows all assertions. |
| Leak-audit integrity | 10/10 | The audit remains unchanged and now observes a properly ended process. |
| Scope discipline | 10/10 | One private fixture statement; no production or public surface change. |
| Evidence honesty | 10/10 | Earlier exit 0 is reported with all eight panic-hook defects; final green is not credited to cleanup as product behavior. |

## Iteration 28 verdict

**APPROVED.** The eight pre-correction seam-leak reports are a fixture
lifecycle defect: each seeded iteration leaves the Active allocation's one
control-session `RecordSocket` alive after completing every Probe-watch
assertion. Adding `node.crash().await` after those assertions invokes the
rig's established process fence, waits for server-owned tasks to end and then
consumes process-scoped seam handles through `exit_process`.

The correction changes no stimulus, seed, schedule, checkpoint, assertion,
expected outcome, public API or production source. Matching-source Metal
verification passes format, check, Clippy and all three exact affected bodies;
all eight overlap seeds complete with zero seam-defect or panic-hook reports.
The bounded fixture cleanup is approved. The separately owned production
audit remediation remains outside this review.
