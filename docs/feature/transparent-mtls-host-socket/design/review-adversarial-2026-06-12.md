# Adversarial Design Review - transparent-mtls-host-socket

**Artifact reviewed**: ADR-0069, feature-delta DESIGN section, C4 diagrams, upstream-change note, product journey, J-SEC-003, slices 00-05.
**Reviewer stance**: adversarial architecture/security review.
**Verdict**: **rejected pending revisions**.

The central pivot to a universal agent-light L4 proxy is plausible and well supported by the spike findings, but the design is not yet safe to hand to DELIVER. The ADR is much stronger than the surrounding handoff artifacts, and several remaining gaps can lead to the wrong system being implemented or to a weaker security property than the operator-facing claims imply.

## Findings

### F1 - Critical - DELIVER can still implement the superseded in-band model

The design explicitly says J-SEC-003 and slices 00-05 need re-grounding and that the architect did not edit them (`feature-delta.md:1413`, `upstream-changes.md:58`). Those files are still stale and continue to specify the old model:

- `docs/product/jobs.yaml:366`-`371`: node agent installs keys into the workload's own socket and leaves the data path.
- `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml:56`-`70`: goal is still host-socket only, workload-owned socket, agent exits.
- `docs/feature/transparent-mtls-host-socket/slices/slice-00-spike-inband-ktls-one-flow.md:1`-`21`: slice 00 is still the in-band kTLS spike.
- `docs/feature/transparent-mtls-host-socket/slices/slice-03-ktls-install-agent-exits-wire-capture.md:1`-`20`: slice 03 still requires kTLS on the workload socket and agent exit.
- `docs/feature/transparent-mtls-host-socket/slices/slice-05-restart-survival-and-wasm-variant.md:7`-`21`: slice 05 still carries restart-survival assumptions from workload-owned fds.

That is not harmless documentation debt. These are the executable acceptance handoff artifacts. A crafter following the slice files can satisfy the wrong acceptance criteria while violating ADR-0069.

**Required revision**: before DELIVER, update J-SEC-003, product journey, discuss journey, and slices 00-05 to the proxy model, or mark the stale files as superseded/non-authoritative in their headers and create replacement slices. The composed proxy walking skeleton must become slice 00 in the actual slice file, not only in `upstream-changes.md`.

### F2 - High - The port contract is still marked "proposal pending user approval"

The handoff says OQ-1 is a blocker requiring exact signature approval (`feature-delta.md:1337`-`1348`). Later, the proposed `MtlsEnforcement` contract is detailed, but it is still labelled "STATUS: PROPOSAL - PENDING USER APPROVAL" (`feature-delta.md:1418`-`1437`). `wave-decisions.md:64`-`68` also carries OQ-1 as a blocker.

That leaves the crafter in an impossible state: the design says "implement to this exact contract" and "this contract is not approved" at the same time.

**Required revision**: resolve OQ-1 before implementation. Either approve the four-method contract verbatim and change the status everywhere to accepted, or replace it with the approved alternative. DELIVER should not improvise this surface.

### F3 - Critical - The peer/inbound half of the proxy is not designed

ADR-0069 designs the originating side: workload connect is intercepted to leg F, the agent dials leg B, presents the workload SVID, and sends TLS records to the real peer (`adr-0069...md:136`-`143`). The C4 diagram says the peer is another Overdrive workload that "presents its own SVID" (`c4-diagrams.md:25`), while also saying workloads are identity-unaware and hold no key (`c4-diagrams.md:20`).

The missing piece is the receiving side: who accepts the incoming mTLS connection, selects the peer workload SVID, verifies the client SVID, terminates/decrypts TLS, and delivers plaintext to the identity-unaware server workload? The stale DISCUSS material had passive-side language, but the accepted proxy ADR does not replace it with an inbound proxy topology.

Without this, "between two workloads" is underspecified. Either the real peer must itself be TLS-aware, contradicting workload-holds-nothing, or a peer-side transparent proxy exists but is not designed.

**Required revision**: add the inbound/passive proxy flow as a first-class design path: listener intercept/bind model, inbound leg naming, server-side SVID selection, client-auth verification, plaintext delivery to the server workload, tests, and failure semantics. If v1 is outbound-only, narrow the claims and acceptance criteria accordingly.

### F4 - Medium - Guest-stack evidence exists, but the DELIVER handoff does not expose the guest-stack adapter shape

The "universal" claim is backed by prior evidence: `findings-userspace-relay.md` explicitly concludes that the lossless path collapses into the #222 two-socket host L4 proxy shape, and `transparent-mtls-recommended-architecture-research.md` recommends a host L4 transparent mTLS proxy for guest-stack workloads at the tap/TPROXY boundary. So the issue is not "no spike/research exists."

The remaining handoff gap is narrower: the concrete `MtlsEnforcement` contract is still host-connect shaped (`cgroup_connect4` rewrite, accepted leg F, `SocketAddrV4` peer, and single-node scope: `feature-delta.md:1524`-`1563`). It does not explicitly say how the guest-stack tap/TPROXY/TC adapter maps a virtio-net/tap flow to an `AllocationId`, recovers the original destination, and produces the same `InterceptedConnection` semantics for microVM/unikernel traffic. The product journey also still says guest-stack workloads are deferred to #222 as a separate feature (`enforce-transparent-mtls-on-the-wire.yaml:236`-`241`), directly contradicting the ADR's #222 fold.

**Required revision**: add a short guest-stack adapter handoff section tying the spike/research evidence to the approved contract: tap/TPROXY/TC intercept source, allocation lookup, original-destination recovery, and conversion into `InterceptedConnection`. Also update the stale product journey so it no longer says #222 is separate/out of scope.

### F5 - High - v1 "mTLS" authenticates only to the bundle, not to the intended peer

The ADR deliberately defers expected-destination identity pinning until #178 and leaves `expected_peer == None` in v1 (`adr-0069...md:210`-`222`; `feature-delta.md:1568`-`1582`). It also reserves the wrong-but-valid-peer test behind #178 (`adr-0069...md:500`-`506`).

That means v1 accepts any peer certificate chaining to the trust bundle. This may be acceptable as a scoped interim, but it is materially weaker than the operator-facing "mTLS with workload identity" story if a routing bug, VIP collision, or malicious in-cluster endpoint can present a valid but unintended SVID.

**Required revision**: choose one of two honest paths: make #178 an upstream prerequisite for the security claim, or rename the v1 guarantee everywhere as "chain-to-bundle transport authentication and encryption only; no intended-peer identity pinning." Add an explicit acceptance guard that prevents docs/tests from calling the wrong-but-valid-peer case protected until #178 lands.

### F6 - Medium - Return-pump failure has observation but no recovery policy

ADR-0069 correctly identifies the return splice pump as a reliability sensitivity point: if it dies, the return path strands (`adr-0069...md:395`-`406`). The contract exposes `liveness()` and `PumpLiveness::Stalled` (`feature-delta.md:1829`-`1852`, `:2032`-`2043`), but it does not define how stalled is detected, what threshold is used, who reacts, or whether reaction is teardown, reconnect, refusal, or health degradation.

**Required revision**: specify the worker supervision policy for pump stalls: progress metric, stall threshold, action, telemetry, and acceptance test. A point query is fine, but an undefined point query is not an operational contract.

### F7 - Medium - Resource limits are required but not concrete enough to test or operate

The review revision adds the right classes of limits: `max_prearm_bytes`, `handshake_deadline`, and `max_inflight_per_alloc` (`adr-0069...md:408`-`434`; `feature-delta.md:1624`-`1653`). But the design says these are "sensible compile-time defaults" and not operator-tunable in v1 (`feature-delta.md:1628`-`1632`) without naming values, sizing rationale, or environment-specific constraints.

That leaves acceptance tests unable to distinguish "bounded by design" from "bounded by arbitrary implementation choice," and it leaves production unable to reason about memory/fd exhaustion.

**Required revision**: pin default values or a sizing formula, plus the expected fd/memory budget per allocation and node. Acceptance should assert those values, not merely the existence of fields.

## Positive Notes

- The ADR does the right thing by downgrading "de-risked" to "primitives de-risked; composition unproven" and making the composed real-intercept test a blocking first gate (`adr-0069...md:43`-`59`, `:477`-`484`).
- The design now explicitly separates authentication/encryption from authorization and avoids duplicating #27/#38 policy evaluation (`adr-0069...md:191`-`228`).
- The F4 resource-limit classes and F5 intercept-recursion test obligations are the correct shape; they need concrete handoff and test integration rather than a different architecture.

## Approval Status

`rejected_pending_revisions`

Minimum bar to proceed to DELIVER:

1. Re-ground or supersede stale J-SEC-003, journey, and slice files.
2. Approve OQ-1 contract or replace it with the approved contract.
3. Add the missing inbound/passive proxy design or narrow the feature scope.
4. Resolve the guest-stack handoff/product-journey contradiction.
5. Make the authn-only v1 boundary impossible to misread as intended-peer mTLS.
