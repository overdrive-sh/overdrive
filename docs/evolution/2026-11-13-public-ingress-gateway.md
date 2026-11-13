# Evolution — public ingress gateway

**Feature:** `public-ingress-gateway` (GH #54). **Finalized:** 2026-11-13. **Status:** DELIVER complete, 26 of 26 roadmap steps committed.
The [feature delta](../feature/public-ingress-gateway/feature-delta.md) is the single wave record (DESIGN and DISTILL sections, user rulings R-1 to R-69, decision tables).
The [approved roadmap](../feature/public-ingress-gateway/deliver/roadmap.json) and the [execution log](../feature/public-ingress-gateway/deliver/execution-log.json)
are the delivery authorities. This record summarizes them and links the permanent artifacts.

## Feature summary

`overdrive serve` can now terminate public HTTPS and relay requests to a VM Service. An operator starts `serve` with four gateway flags
(address, certified-key id, certificate chain, private key) and declares one Route with `overdrive deploy <ROUTE_SPEC>`.
The gateway terminates TLS 1.3 on TCP/443 for the Route's hostname and parses HTTP/1.1 request and response heads.
It selects the backend in the kernel with a dedicated `cgroup/connect4` BPF program over the existing Service maps. It reaches the backend over the mesh with its own
SPIFFE identity (a dedicated gateway SVID) and pins the exact peer the BPF program chose. The guest sees plaintext HTTP.
After a userspace rustls handshake on each leg, the kernel holds the TLS record layer (kTLS), and bodies move between the two sockets with `splice(2)` and never enter gateway memory.

## Business context

GH #54 is titled "Gateway subsystem (node-agent-embedded; hyper + rustls; in-process BPF map access)" and describes the gateway as infrastructure, not a platform job
(feature delta, "Prior-wave and architecture consultation"). Overdrive's mTLS model covers traffic between workloads, where workloads hold no certificates and the node agent
runs mutual TLS for them. It did not cover a client on the public internet reaching a workload. The gateway closes that gap without a second identity system,
a second backend list or a userspace load balancer.

The feature had no DISCUSS wave. The delta records that current intent came from issue #54 plus explicit user corrections (public north-south ingress,
operator-supplied public certificate first, gateway SVID on the mesh hop, existing BPF Service dataplane preserved), and that no stories, story map or KPIs were invented.
There was also no separate spike `findings.md` at DESIGN start. The spike evidence arrived as four findings files described below.

## Key decisions

The 24 gateway ADRs [0220](../product/architecture/adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md) to
[0243](../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md) trace to the 2026-09-13 DESIGN, reconstructed on `main` at `549d78fa6`, plus later additions such as 0242, 0243 and 0248 (see
[adr-renumbering.md](../feature/public-ingress-gateway/adr-renumbering.md)). The kernel data-path ADRs
[0244](../product/architecture/adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md) to
[0254](../product/architecture/adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md) were added by the G1 ruling of 2026-11-02 and the rulings that followed.
[0274](../product/architecture/adr-0274-gateway-svid-issuance-failure-is-a-recorded-occurrence-on-the-application-status-row.md) and
[0275](../product/architecture/adr-0275-workload-svid-issuance-failure-is-a-recorded-occurrence-on-its-own-row.md) record the two SVID issuance-failure decisions (R-45, R-46, 2026-11-04).

| Decision | ADR | What and why |
|---|---|---|
| Embed single-node ingress in `serve` | [0220](../product/architecture/adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md) | The gateway runs inside the node agent and adds no backend-programming mechanism. Spike D4 showed a VM backend already reaches `SERVICE_MAP`, `BACKEND_MAP` and `REVERSE_NAT_MAP` through the ordinary hydrator path. |
| One Route set, Gateway Application owner | [0221](../product/architecture/adr-0221-singleton-public-route-set-aggregate.md), [0222](../product/architecture/adr-0222-public-ingress-route-and-gateway-application-owner.md) | A singleton Route Set aggregate and one owner that publishes Route, frontend and resolver admission atomically. |
| Producer-neutral certificate custody | [0223](../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md), [0231](../product/architecture/adr-0231-preserve-last-usable-public-certified-key-replacement.md) | Manual input is the only producer now. Custody has one install boundary, so ACME (#57) can later publish the same protected input. A bad replacement never displaces the last usable key. |
| Dedicated gateway SVID | [0225](../product/architecture/adr-0225-dedicated-gateway-svid-identity-lifecycle.md), [0234](../product/architecture/adr-0234-exact-peer-gateway-client-mtls.md) | The gateway holds its own identity slot, so no allocation's identity is borrowed. The mTLS hop pins the exact peer the BPF selection chose. |
| Separate BPF selection program | [0224](../product/architecture/adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [0232](../product/architecture/adr-0232-gateway-connect-selected-backend-receipt.md), [0233](../product/architecture/adr-0233-commit-gated-backend-identity-publication.md), [0241](../product/architecture/adr-0241-dataplane-applied-backend-identity-association.md) | See below. |
| Kernel TLS after a userspace handshake | [0244](../product/architecture/adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md), [0249](../product/architecture/adr-0249-gateway-legs-offer-only-aes-gcm-tls13-suites.md), [0250](../product/architecture/adr-0250-gateway-never-originates-a-tls-keyupdate.md) | See below. |
| Gateway owns HTTP/1.1 framing | [0245](../product/architecture/adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md), [0251](../product/architecture/adr-0251-gateway-rejects-ambiguous-http-framing.md) to [0254](../product/architecture/adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md) | Parsing heads and deciding where each message ends moved into the gateway when hyper left the data path. |
| Splice discipline | [0246](../product/architecture/adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md), [0247](../product/architecture/adr-0247-gateway-exchange-is-full-duplex.md), [0239](../product/architecture/adr-0239-stream-public-proxy-bodies-with-backpressure.md) | The empty-pipe rule of at least 64 KiB exists because of a kernel defect found in the spike. |
| Typed unavailability at the consuming port | [0242](../product/architecture/adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md), [0243](../product/architecture/adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md), [0248](../product/architecture/adr-0248-connector-reserves-its-cleanup-ledger-slot-before-preparation.md) | A missing backend is a typed result at the gateway, and the dataplane adapter prepares upstream sockets with a cleanup ledger. |
| Listener gating, status, generations | [0226](../product/architecture/adr-0226-public-listener-tls13-only.md), [0227](../product/architecture/adr-0227-redacted-operator-gateway-status.md), [0228](../product/architecture/adr-0228-dataplane-selection-receipt-ownership.md), [0229](../product/architecture/adr-0229-keep-gateway-and-service-dataplane-generations-independent.md), [0230](../product/architecture/adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md) | The gateway's gate does not redefine workload Service readiness. Status is redacted. |
| Request policy | [0236](../product/architecture/adr-0236-canonical-public-route-match-key.md), [0237](../product/architecture/adr-0237-public-proxy-header-policy.md), [0240](../product/architecture/adr-0240-single-upstream-attempt-per-public-request.md) | One canonical match key, a fixed header policy, one upstream attempt and no retry. |

### From hyper in userspace to kTLS and splice

The first design (ratified 2026-09-13) was the conventional one: a hyper server terminates TLS and parses HTTP, then an HTTP client forwards each request.
Every body byte would pass through gateway memory twice. The user ruling G1 of 2026-11-02 replaced it before it was built.
rustls now performs the handshake, then hands the session secrets to the kernel through `kernel::KernelConnection`. The gateway parses only heads and splices bodies between the two kTLS sockets.
The stated reason is to keep bodies out of the gateway process, and the cost is that the gateway frames HTTP/1.1 itself, so ADR-0239 and ADR-0245 record `hyper` as a rejected alternative.
The spike found no CPU advantage for splice over copying in userspace ([findings-ktls-splice-relay.md](../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md), item c),
and the feature makes no throughput or CPU claim. The spike ran on native metal, Linux 7.0.0-38, and established three things this design rests on:

- Splicing a kTLS-RX socket into a pipe smaller than a TLS record loses 5 bytes per record on 7.0. The relay therefore splices only into an empty private pipe of at least 64 KiB (ADR-0246). The spike notes a kernel fix in v7.3.
- TLS 1.3 KeyUpdate works only through rustls `kernel::KernelConnection` from the unbuffered handshake. `dangerous_extract_secrets` is not enough.
- A `connect4` selection composes with a later handshake and kTLS install on the same file descriptor. A kTLS plus sockmap rejection is absent on 7.0.0-38 and present at v7.3, so the design uses no sockmap.

### A separate BPF selection program

The gateway needs the mesh's backend selection for its own connections. Folding the gateway case into the existing `cgroup_connect4_service` hook was rejected (user ruling P-10, R-1..R-19 of 2026-11-02).
The blog records the measured basis: a prototype of the gateway selection verified at 246 instructions against 28 for the existing hook, about 8.8 times growth on a hook that runs on every workload connect, against a 5% growth rule for hot-path programs.
The gateway therefore attaches a separate `cgroup/connect4` program to its own cgroup and reuses the Service maps. The final object verifies at 125 instructions
([findings-connect-hook-cost.md](../feature/public-ingress-gateway/spike/findings-connect-hook-cost.md)). The same findings file reports a median 117 ns added to a connect of about 28 microseconds over 30 interleaved runs on one host (report-only, GH #287).
The program ignores the `healthy` flag (ruling P-9) because the flag flapped in tests for a reason the team could not establish, so a request in the window between a backend starting and its listener binding gets `502`.
Spike V-GW-1 to V-GW-5 ([findings-gateway-connect-path.md](../feature/public-ingress-gateway/spike/findings-gateway-connect-path.md)) held V-GW-2, V-GW-3, V-GW-4 and V-GW-5 and held V-GW-1 only partly.

### Tracked limitations

Each limitation below has an open GitHub issue, confirmed with `gh issue view` on 2026-11-13.

| Limitation | Where pinned | Issue |
|---|---|---|
| HTTP/1.1 only, anything else answered `505` | [ADR-0235](../product/architecture/adr-0235-public-http11-only.md) | [#341](https://github.com/overdrive-sh/overdrive/issues/341) HTTP/2 and gRPC on the public listener |
| One Route and one hostname per node | [ADR-0221](../product/architecture/adr-0221-singleton-public-route-set-aggregate.md), docs pages | [#352](https://github.com/overdrive-sh/overdrive/issues/352) several Routes and hostnames on one listener |
| Fixed finite runtime limits (128 connections, 32 KiB heads, 100 headers, 100 requests per connection, 16 MiB bodies, 5, 30 and 120 second deadlines) | [ADR-0238](../product/architecture/adr-0238-fixed-finite-public-runtime-limits.md), docs pages | [#353](https://github.com/overdrive-sh/overdrive/issues/353) configurable runtime limits |
| Certificates come from operator files, status reports source `manual` | [ADR-0223](../product/architecture/adr-0223-producer-neutral-public-certified-key-custody.md) | [#57](https://github.com/overdrive-sh/overdrive/issues/57) embedded ACME client |
| No CLI verbs for gateway status and Route withdrawal (HTTP calls on the operator listener) | docs pages | [#355](https://github.com/overdrive-sh/overdrive/issues/355) CLI verbs for gateway status and Route withdrawal |

The feature delta and ADR text cite #341 and #57. #352 and #353 appear in the website pages, and #355 is named in `review-docs.md`. Neither ADR-0221 nor ADR-0238 carries its issue number.

## Work completed

All 26 roadmap steps have a passing COMMIT event in the execution log, dated 2026-11-05 to 2026-11-12. The roadmap was approved on 2026-11-05 by the DELIVER roadmap review.

**Phase 01, core vocabulary.**
01-01 Route value types, canonical match key and generations. 01-02 Gateway identity epoch and SPIFFE subject.
01-03 Observation-store half: workload SVID issue-failure record and gateway status rows. 01-04 HTTP/1.1 exchange state machine and deadline composition.
01-05 Quint conformance, `relay_exchange` driver.

**Phase 02, identity, control plane and Gateway Application owner.**
02-01 `GatewaySvidLifecycle` reconciler transitions. 02-02 Gateway SVID issue-and-audit executor and identity slot. 02-03 Issue-failure recording in the executor.
02-04 Gateway identity hydration and Quint conformance (`gateway_identity`). 02-05 Workload SVID issue-failure occurrence on the allocation surface.
02-06 Gateway Application owner with certified-key custody. 02-07 Lag-aware intent subscription and gateway watch recovery.

**Phase 03, dataplane connect path.**
03-01 Gateway selection `cgroup/connect4` hook and registered-socket connect. 03-02 Selection receipt and commit-guarded applied backend identity.
03-03 Quint conformance, `selection_receipt` driver. 03-04 Upstream socket preparation and cleanup ledger in the connector.

**Phase 04, kernel relay and exchange runtime.**
04-01 Sim relay adapter, HTTP/1.1 head and framing, exchange driver. 04-02 Host kernel relay adapter (rustls handshake, kTLS, splice).
04-03 Lingering close for early and gateway-authored responses. 04-04 Relay Earned-Trust probe. 04-05 Exact-peer gateway client mTLS.
04-06 Seeded Gateway Application invariants. 04-07 Quint conformance, `gateway_application` driver.

**Phase 05, composition and the production loop.**
05-01 Enabled gateway boot, shutdown stage and the consuming-port 503 outcome. 05-02 Route operator surface, gateway status and issuance-failure occurrence.
05-03 Public request end to end through `serve` and `deploy`, which also produced the recurring conformance test `AT-PIG-E2E-1`
([tests/conformance/tests/public_ingress_gateway_end_to_end.rs](../../tests/conformance/tests/public_ingress_gateway_end_to_end.rs)).

The execution log holds 153 events, including three FAIL events (02-04 GREEN, 03-03 RED, 04-07 RED) and the recorded blockers and checkpoints below. Every one of these steps later has a passing COMMIT:

- 01-04 stopped on a `todo!` scaffold (`GatewayLimits::first_slice`) owned by a later step.
- 02-04 stopped on an acceptance-test fixture defect (a shared `SimCa` seed gave a duplicate serial).
- 02-06 stopped because the gateway `NodeId` source was unpinned, which needed a design ruling.
- 03-03 and 04-07 stopped because the Quint model and driver predated a ruling (R-59) or kept a zero-lease Draining state the owner retires eagerly, and the fix belonged to the spec and driver, not production code.
- 04-04 stopped on a gold test that required distinct `RelayProbeError` discriminants the pinned shape did not provide.
- 05-01 stopped because the route and status handlers belonged to 05-02.
- 05-03 recorded two checkpoints on 2026-11-09, a guest image staged without the guest kit and a test oracle that required a `127.0.0.1` peer where production dials the guest address. It committed on 2026-11-12.

The log's own text names each cause. It does not record the day-by-day resolution, and this record does not infer one.

## Verification

**Spike evidence.** Four findings files and [wave-decisions.md](../feature/public-ingress-gateway/spike/wave-decisions.md) record the kTLS and splice relay (PROMOTE with a pipe condition), the VM Service map programming (WORKS), the connect path V-GW-1 to V-GW-5 and the per-connect hook cost. All ran on native x86_64 metal, kernel 7.0.0-38-generic. They are evidence for design claims and not test coverage.

**Formal models.** [specs/quint/public-ingress-gateway/](../../specs/quint/public-ingress-gateway/README.md) holds four state-machine modules (`relay_exchange`, `gateway_identity`, `gateway_application`, `selection_receipt`) with named runs, hazard variants and recorded evidence. Steps 01-05, 02-04, 03-03 and 04-07 deliver their `quint-connect` conformance drivers.

**E15, the operator journey on native metal.** [E15](../../verification/expectations/E15-public-ingress-operator-journey/README.md) was captured at clean commit `b0f880439` on 2026-11-13 on a native x86_64 metal host (run `20261113T135825.550600128Z-3340967`, `execution_status: succeeded`, runner exit 0).
An independent review agent that saw only the expectation, the runner and the raw evidence returned **SATISFIED**
([review-e15-evidence.md](../feature/public-ingress-gateway/deliver/review-e15-evidence.md)). The claims that held:

- `curl` with system trust only (no `--cacert`, no `--insecure`) negotiated TLS 1.3, got `200` with `X-Workload: api`, and received the guest's eight-chunk body.
- `ss -tni` samples taken during the request showed `tcp-ulp-tls version: 1.3` with transmit and receive configured on the public 443 connection and on the upstream leg-C connection. `TlsCurrTxSw` and `TlsCurrRxSw` went 0, 3, 0 across the run and `TlsDecryptError` stayed 0.
- `serve` started the gateway through the relay probe with no `health.startup.refused` event, and TCP/443 was bound before the request.
- After cleanup, VM, network, cgroup, run-directory, mount and loop deltas and public-443 and kTLS socket counts were all zero.

The review decided one claim as NOT DECIDED: whether the body streamed or was buffered. E15 makes no claim about it, and neither does this record. It is one run, so it shows the path works end to end and not how often.

**Gaps the capture exposed, fixed before it passed.** The first metal runs failed on five separate defects, each fixed in its own commit:

1. `15ec00167`: the remote checkout was never clean. The sync excluded `vendors/linux`, so its gitlink read as deleted, and the runner's two marker files were untracked. E15 requires a clean HEAD and refused every run. The sync now creates an empty `vendors/linux` directory and `.gitignore` covers the markers.
2. `8311ae385`: the kTLS sampler's section headers began with `---`, which bash `printf` parses as an option, so the runner exited with status 2 before the example ran.
3. `4780b4200`: the example wrote only the `overdrive-ca-root` credential, but the gateway seals the certificate's private key under a separate `overdrive-public-certified-key` KEK, so `serve` refused with "public certified-key KEK unavailable". Tests injected a `SimKek`, so the credential-file path had never run before E15.
4. `d661ccd5d`: the example's `prepare.sh` staged no guest kernel modules, so `overdrive-init` could not create its transport dummy device ("Not supported"). It now stages `overdrive-init`, `ovd_fwd.ko`, the dummy and `inet/tcp/udp_diag` modules and a static `modprobe`, matching `service-kind-vm-workloads/prepare.sh`.
5. `b0f880439`: the example failed its own check after a successful request. curl writes CRLF header lines and `\r?` is not a carriage-return escape in GNU `grep -E`, so `x-workload: api` never matched. The pattern now carries a literal CR.

**Resource measurements** (one host, the whole `serve` process): 9 descriptors per exchange at peak and 7 at steady state, resident memory growth of about 173 KiB per exchange on a 0.73 MB base, and 0.44 MB total in-use heap growth across six bursts of 128 exchanges.
These come from the blog, which cites a test bound of 24,405,456 bytes at 128 exchanges. They are a single sample. This record did not re-derive them from raw output.

**Not claimed:** any CPU or throughput advantage, behavior on other hosts or loads, streaming versus buffering, and mutation-test scores for this feature (this record read no mutation result).

## Lessons learned

- A seam that tests fill with a simulator hides the real input. Every gateway test injected `SimKek`, so the credential-file path to the key-encryption key had never run until a real example ran it. The E15 capture found it.
- Hand-written example scaffolding drifts from its sibling. The missing kernel modules were already solved in `service-kind-vm-workloads/prepare.sh`.
- A design measured before it is built can change shape cheaply. The hyper design was replaced during DESIGN, and the 5-bytes-per-record kernel defect surfaced in a spike and not in production.
- A silent data-loss defect makes a tuning value a correctness setting. The 64 KiB empty-pipe rule is pinned by ADR-0246 because nothing errors when it is violated.
- Quint drivers that depend on a spec's state model stall when the spec lags a ruling (03-03, 04-07). The log shows spec and driver changes had to leave the crafter's scope.
- A shared remote checkout that is never clean blocks clean-source evidence. Capture preconditions need their own gate before the capture.
- We chose the data path before measuring it. G1 replaced the hyper data path with kTLS and spliced bodies, and the spikes that followed (G3) only showed the new path could work. They also found splicing no cheaper than a userspace copy. The kernel path gained nothing measurable and cost a custom HTTP/1.1 framer, a kTLS splice relay and a workaround for a kernel splice defect. HTTP/2 (#341) needs userspace framing anyway. Benchmark competing mechanisms with a spike before choosing one, and move work into the kernel only for a measured gain or a property userspace cannot provide (`.claude/rules/design.md`, "Choose kernel or userspace by evidence" and "Benchmark competing approaches with a spike"). #341 carries the instruction to revisit the data path first.
- We stopped reading a signal instead of fixing it. The gateway ignores `BACKEND_MAP.healthy` (ADR-0242, R-2) because the flag flapped in a spike for a cause nobody established. Requests in the roughly one-second startup window therefore reach backends that are not ready and get `502`. An unexplained flap is a defect to root-cause, not a reason to drop readiness.
- Reuse is what kept the gateway small. Backend selection reuses the mesh's Service maps, the upstream hop reuses the node's identity issuance, and the selection hook reuses the existing cgroup attachment. Each avoided a second copy that could drift from the first. The two kernel facts that changed the design (the splice byte loss and the KeyUpdate API) both came from the byte relay, the one component built from scratch.
- A decision request needs its evidence and costs in it. The design record lists G1 and R-2 as approved on 2026-11-02, yet G1's cost (no measured gain) was established only after approval, and R-2's cost (`502` to not-ready backends) did not reach the owner as a decision about correctness.

## Issues encountered

- The blocked and checkpoint-pending steps listed under "Work completed".
- The five E15 capture defects listed under "Verification".
- `bun run test:blog` could not run, for an environmental reason described in the finalization record.
- The documentation review took four rounds. Rounds 1 to 3 requested changes (uncited limitations, evidence scoped past one run, banned wording, a how-to that checked status before a Route existed, and a missing KEK step), and round 4 approved.

## Permanent artifacts

- ADRs in `docs/product/architecture/`: [0220](../product/architecture/adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md) to [0254](../product/architecture/adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md), [0274](../product/architecture/adr-0274-gateway-svid-issuance-failure-is-a-recorded-occurrence-on-the-application-status-row.md) and [0275](../product/architecture/adr-0275-workload-svid-issuance-failure-is-a-recorded-occurrence-on-its-own-row.md). The Route and domain context map and the canonical C4 for the gateway are in [c4-diagrams.md](../product/architecture/c4-diagrams.md).
- Quint specification: [specs/quint/public-ingress-gateway/](../../specs/quint/public-ingress-gateway/README.md).
- Verification: [E15](../../verification/expectations/E15-public-ingress-operator-journey/README.md) and its [different-fox review](../feature/public-ingress-gateway/deliver/review-e15-evidence.md).
- Recurring test: [AT-PIG-E2E-1](../../tests/conformance/tests/public_ingress_gateway_end_to_end.rs).
- Example: [examples/public-ingress-gateway/](../../examples/public-ingress-gateway/README.md).
- Docs and blog: [concept page](../../website/content/docs/concepts/public-ingress-gateway.mdx), [how-to](../../website/content/docs/how-to/expose-a-service-publicly.mdx), [blog post](../../website/content/blog/public-ingress-gateway.mdx).
- Delivery history, retained in place: the [feature workspace](../feature/public-ingress-gateway/feature-delta.md), [code reconciliation plan](../feature/public-ingress-gateway/distill/code-reconciliation-plan.md), [ADR renumbering map](../feature/public-ingress-gateway/adr-renumbering.md) and the spike findings.

## Finalization record

Required by `.claude/rules/finalize.md`.

**E15 captured and reviewed before any writing.** E15 was captured at `b0f880439` and reviewed SATISFIED before the blog reported it (commits `fa639219d`, `50321dd68`, `3b57914e7`).

**Documentation and blog paths.**
- `website/content/blog/public-ingress-gateway.mdx`
- `website/content/docs/concepts/public-ingress-gateway.mdx`
- `website/content/docs/how-to/expose-a-service-publicly.mdx`
- Edited pages, as listed in the review artifact: `website/content/docs/concepts/architecture.mdx`, `concepts/identity.mdx`, `concepts/workload-identity-lifecycle.mdx`, `how-to/deploy-a-workload.mdx`, `how-to/run-verification-on-metal.mdx`, `reference/cli.mdx`, and the `meta.json` files.

**Validation results.**
- `bun run build`: passes.
- `bun run test:blog`: could not run. It starts `wrangler dev`, which needs `bunx`, and the local proto install of bun 1.3.5 has no `bunx` binary. The cause is environmental and the test never started a server.

**Reviewer verdict.** APPROVED at `116aa6ba2` after four rounds (the reviewers were independent Sonnet agents given the rules, the content and the sources of truth, not the author's summary). One non-blocking note remains, a long source line in the concept page that does not affect the rendered text.
The record-keeping commit is `c5626b0a0`.

**Review artifacts.**
- [docs/feature/public-ingress-gateway/deliver/review-docs.md](../feature/public-ingress-gateway/deliver/review-docs.md)
- [docs/feature/public-ingress-gateway/deliver/review-e15-evidence.md](../feature/public-ingress-gateway/deliver/review-e15-evidence.md)

This finalization wrote no website deployment, publication, push or pull request. No file was migrated or deleted, because the ADRs, feature workspace, specs and verification catalogue already sit at their permanent homes.
