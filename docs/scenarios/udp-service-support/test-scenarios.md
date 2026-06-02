<!-- markdownlint-disable MD024 MD013 -->
# Test Scenarios — udp-service-support (DISTILL SSOT)

**Wave:** DISTILL (Sentinel, acceptance-designer). **Date:** 2026-06-02.
**Density:** lean (Tier-1 [REF]).

> **SPECIFICATION ONLY — NOT EXECUTED.** Per `.claude/rules/testing.md`
> § "No `.feature` files anywhere", these GIVEN/WHEN/THEN blocks are the
> acceptance specification. They are **never parsed or executed**. The
> crafter (DELIVER) translates each tier-tagged scenario into a Rust
> `#[test]`/`#[tokio::test]` function at the placement named per scenario.
> The RED scaffolds DISTILL ships (`#[should_panic(expected = "RED
> scaffold")]`) are the machine artifacts; this file is the human-readable
> contract they implement.
>
> **SSOT cross-refs:** ADR-0060 (trait contract — preconditions /
> postconditions / edge cases / cross-adapter invariant);
> `feature-delta.md` (US-01..US-05 + DESIGN [REF] decisions D1a–D8);
> `design/upstream-changes.md` (the two back-prop corrections);
> `docs/product/journeys/submit-a-udp-service.yaml`.

---

## Tier mapping (replaces the generic skill's WS-strategy A/B/C/D)

The four-tier model from `.claude/rules/testing.md` IS the test taxonomy.
Every scenario below is tagged with its tier.

| Tier | Lane / runner | What it proves | PBT mode (Mandate 9) |
|---|---|---|---|
| **Tier 1 (DST)** | default lane, `cargo dst` | In-process `SimDataplane` set-equality + reconciler/projection purity. The lockstep universe guard. | proptest **full** allowed (layer 1–2): the per-proto purge, idempotent re-apply, and `ServiceFrontend::new` V4-validation are proptest targets. |
| **Tier 2 (BPF unit)** | `cargo xtask bpf-unit`, `BPF_PROG_TEST_RUN` | Kernel `xdp_reverse_nat_lookup` rewrites a proto=17 source to the VIP. | **example-only** (layer 3+; Mandate 11). |
| **Tier 3 (real veth)** | `cargo xtask lima run --`, `integration-tests` feature | Real `EbpfDataplane` + real wire: `bpftool map dump` + AF_PACKET/tcpdump capture sourced from the VIP. Walking skeleton + multi-listener. | **example-only** (layer 4+; Mandate 11). Sad paths enumerated. |

> **Mandate 8 (Universe-bound assertion) at Tier 1.** The Sim-side scenarios
> assert via exact `BTreeSet<BackendKey>` set-equality — the universe is the
> port-observable REVERSE_NAT key set (+ forward service map), expected is the
> keys derived from `(frontend.proto, backends)`, and an unexpected extra key
> fails the orphan-direction check (fail-closed). See
> `docs/architecture/atdd-infrastructure-policy.md` § "Mandate 8 mapping".
> Tier 2 / Tier 3 (layer 3+) use traditional kernel-side-observable
> assertions per Mandate 8 (layers 4+ MAY use traditional assertions) and
> Mandate 11.

---

## US-01 — Thread protocol through a `ServiceFrontend` newtype

### S-01-A — `ServiceFrontend::new` accepts an IPv4 VIP and round-trips `vip_v4()` `@property` `@US-01` `@K5` `@in-memory` Tier 1

```gherkin
Property: Any IPv4 VIP constructs a frontend whose vip_v4() round-trips the input
  Given any IPv4 ServiceVip, any non-zero port, and any protocol (tcp or udp)
  When a ServiceFrontend is constructed via ServiceFrontend::new(vip, port, proto)
  Then construction succeeds (Ok)
  And vip_v4() returns exactly the IPv4 address that was supplied
  And proto() returns exactly the protocol that was supplied
  And port() returns exactly the non-zero port that was supplied
```
Proptest target: `@given` over IPv4 octets × `NonZeroU16` × `{Tcp, Udp}`.
Placement: `crates/overdrive-core/tests/dataplane/service_frontend_scenarios.rs`.
Note (D2): `ServiceFrontend` has NO `Display`/`FromStr` (no serde) — there is
**no** newtype string-roundtrip property here; the roundtrip is over the typed
accessors only.

### S-01-B — `ServiceFrontend::new` rejects an IPv6 VIP `@property` `@error` `@US-01` `@D1a` `@in-memory` Tier 1

```gherkin
Property: Any IPv6 VIP is rejected by ServiceFrontend::new
  Given any IPv6 ServiceVip, any non-zero port, and any protocol
  When ServiceFrontend::new(vip, port, proto) is called
  Then it returns Err(ParseError)
  And no ServiceFrontend value is produced
```
Proptest target: `@given` over IPv6 segments. Pinned `@example`: `::1`.

### S-01-C — The frontend carries the declared protocol end-to-end from intent `@US-01` `@in-memory` Tier 1

```gherkin
Scenario: A udp listener's protocol reaches the dataplane as Udp, never defaulted to Tcp
  Given a Service desired projection sourced from a listener-bearing fact whose listener declares protocol udp on port 5353
  When the ServiceMapHydrator emits the dataplane update action and the action-shim builds the ServiceFrontend
  Then the ServiceFrontend reaching update_service carries proto Udp
  And no Proto::Tcp literal on the intent→hydrator→frontend→dataplane path is reached for this service
```
Placement: `crates/overdrive-core/tests/reconcilers/service_frontend_provenance_scenarios.rs`
(or the existing hydrator test module). Driving port: `ServiceMapHydrator.reconcile`.

---

## US-01 / C3 GUARD — proto provenance (ATLAS-1 carried-forward)

### S-01-D — Proto is sourced from a listener-bearing fact (happy arm) `@US-01` `@C3` `@in-memory` Tier 1

```gherkin
Scenario: The desired projection sources protocol from the listener fact, not from service_backends
  Given an observed listener fact declaring (vip 10.96.0.10, port 5353, protocol udp)
  And a service_backends row that carries neither port nor protocol
  When the desired projection for the service is computed
  Then the resulting ServiceFrontend proto is Udp
  And the protocol was read from the listener-bearing fact (ListenerRow / BackendDiscoveryBridge per-listener projection), not synthesised from a default
```

### S-01-E — Unresolvable listener proto is a structured error, NEVER a silent `Tcp` default (negative arm) `@US-01` `@C3` `@error` `@in-memory` Tier 1

```gherkin
Scenario: A desired projection with no resolvable listener protocol fails structured, not silently defaulting to Tcp
  Given a service whose desired projection has no resolvable listener-bearing protocol fact
  When the desired projection attempts to determine the service protocol
  Then it produces a structured error (Failed), surfaced as an operator-visible failure
  And it does NOT emit a ServiceFrontend or update_service action with a silently-defaulted Proto::Tcp
```
This is the load-bearing C3 defense (ATLAS-1 b). Placement: same module as S-01-C/D.
The negative arm asserts the **absence** of a `Tcp`-defaulted action AND the
**presence** of the structured Failed signal — both, not either.

---

## US-01 / D1a — IPv6 rejection at the operator-visible site

### S-01-F — IPv6 VIP rejected at the action-shim with the existing operator-visible Failed row `@US-01` `@D1a` `@error` `@in-memory` Tier 1

```gherkin
Scenario: An IPv6 service VIP is rejected at the action-shim as an operator-visible Failed, not a late opaque dataplane error
  Given an action carrying a service whose VIP is IPv6
  When the action-shim dispatch builds the ServiceFrontend via ServiceFrontend::new
  Then construction is rejected at the action-shim (the existing ipv4_from_vip rejection site)
  And an operator-visible ServiceHydrationStatus::Failed observation row is written with reason Ipv6Unsupported
  And the rejection is NOT demoted to a late opaque DataplaneError deep in an adapter
```
Driving port: action-shim `dispatch`. Placement:
`crates/overdrive-control-plane/tests/acceptance/service_frontend_ipv6_rejected.rs`
(in-process; the action-shim is exercised directly, the Failed row is the
port-observable outcome). Mirrors the existing `dataplane_update_service.rs:160`
`ipv4_from_vip` → `ServiceHydrationStatus::Failed` behaviour — preserved unchanged.

---

## US-02 / US-03 — Sim per-proto fan-out + lockstep set-equality (Tier 1)

### S-03-A — Sim installs EXACTLY the declared-`frontend.proto` key set `@US-03` `@K2` `@property` `@in-memory` Tier 1

```gherkin
Property: The SimDataplane REVERSE_NAT key set for a service equals exactly the keys derived from (frontend.proto, backends)
  Given any service with a frontend declaring a single protocol P and any set of IPv4 backends
  When update_service(frontend, backends) is applied to the SimDataplane
  Then the REVERSE_NAT key set for that service equals exactly { BackendKey{ ip, port, P } : backend in backends }
  And it contains NO key for any protocol other than P
```
This narrows the `[Tcp, Udp]` hardcode (`sim/.../dataplane.rs:277`,
`reverse_nat_lockstep.rs:161`) to `frontend.proto`. Universe = the
`BTreeSet<BackendKey>`; expected = exact set-equality (fail-closed via the
orphan check). Placement: retarget
`crates/overdrive-sim/src/invariants/reverse_nat_lockstep.rs` +
`crates/overdrive-sim/tests/invariant_evaluators.rs` driving it.

### S-03-B — NEGATIVE: a dropped Sim fan-out key fails the lockstep `@US-03` `@K2` `@error` `@in-memory` Tier 1

```gherkin
Scenario: A SimDataplane fan-out that drops the declared-proto key fails the lockstep gate
  Given a udp service whose SimDataplane fan-out is mutated to install no key (or a tcp-only key)
  When the ReverseNatLockstep invariant evaluates the key set
  Then the invariant FAILS, naming the missing (ip, port, udp) key
```

### S-03-C — NEGATIVE: an EXTRA (phantom) Sim key fails the lockstep `@US-03` `@K2` `@error` `@in-memory` Tier 1

```gherkin
Scenario: A SimDataplane fan-out that installs a phantom extra-proto key fails the lockstep gate
  Given a tcp-only service whose SimDataplane fan-out installs both a tcp AND a phantom udp key (the pre-US-01 over-broad behaviour)
  When the ReverseNatLockstep invariant evaluates the key set
  Then the invariant FAILS via the orphan-direction check, naming the phantom (ip, port, udp) key as not present in the declared layout
```
S-03-B + S-03-C together pin the fail-closed universe guard in BOTH directions
(missing key AND extra key) — this is the #163 structural defense.

### S-03-D — The #163 regression is structurally caught `@US-03` `@K2` `@error` `@in-memory` Tier 1

```gherkin
Scenario: The exact #163 shape — Ebpf installs only tcp for a udp service — is the failure the gate exists to catch
  Given a udp service for which the production-mirroring fan-out would install only the tcp key (the #163 bug shape)
  When the lockstep set-equality is evaluated against the declared udp frontend
  Then the gate FAILS, proving the divergence that shipped #163 cannot recur silently at Tier 1
```
(The production-adapter half of #163 is caught at Tier 3 — S-04-A's `bpftool`
dump asserts the udp key is present in the real `EbpfDataplane`.)

---

## US-02 / D4 — Per-proto purge (Tier 1)

### S-02-A — Empty backends purges ONLY `frontend.proto`'s keys `@US-02` `@D4` `@property` `@in-memory` Tier 1

```gherkin
Property: update_service(frontend_P, []) purges only protocol P's REVERSE_NAT keys for the VIP
  Given a VIP carrying a udp frontend (installed keys for udp) and a co-resident tcp frontend on the same VIP (installed via a separate update_service call)
  When update_service(frontend_udp, []) is applied with an empty backend set
  Then every udp REVERSE_NAT key for that VIP is removed
  And every co-resident tcp REVERSE_NAT key for the same VIP survives
```
Proptest target: `@given` over backend sets × the two protos. Universe = the
full `BTreeSet<BackendKey>` for the VIP; expected = udp keys `set_to` empty,
tcp keys `unchanged`. This is the per-proto-purge correction (D4 /
upstream-changes Correction 1 — "removes both protos" → per-proto).

### S-02-B — Cross-service shared key survives a per-proto purge `@US-02` `@D4` `@in-memory` Tier 1

```gherkin
Scenario: A REVERSE_NAT key shared with another live service survives an empty-backends purge
  Given a backend key (ip, port, udp) referenced by both service A and service B
  When service A scales to zero backends via update_service(frontend_udp_A, [])
  Then the (ip, port, udp) key survives because service B still references it (the live_keys difference check)
```
Mirrors the existing `sim_dataplane_reverse_nat_cross_service.rs` precedent,
extended to the per-proto frontend shape. Placement:
`crates/overdrive-sim/tests/sim_dataplane_reverse_nat_cross_service.rs` (extend)
or a new `sim_dataplane_reverse_nat_per_proto_purge.rs`.

### S-02-C — Idempotent re-apply yields the same post-state `@US-02` `@property` `@in-memory` Tier 1

```gherkin
Property: Applying update_service(frontend, backends) twice with identical arguments is idempotent
  Given any frontend and any backend set
  When update_service(frontend, backends) is applied, then applied again with identical arguments
  Then the REVERSE_NAT key set after the second application equals the set after the first
```
Proptest target. Mandate 8: universe = the key set; expected =
`idempotent_after` the first apply.

---

## US-02 / boundary — unsupported / edge inputs

### S-02-D — A non-IPv4 backend address contributes no REVERSE_NAT key `@US-02` `@error` `@in-memory` Tier 1

```gherkin
Scenario: A backend with an IPv6 address is silently skipped from the REVERSE_NAT key set
  Given a udp frontend and a backend set containing one IPv4 backend and one IPv6 backend
  When update_service(frontend_udp, backends) is applied to the SimDataplane
  Then the REVERSE_NAT key set contains the IPv4 backend's (ip, port, udp) key
  And the IPv6 backend contributes no key (parity with reverse_nat_keys_for's IPv4-only filter, GH #155 deferral)
```

### S-02-E — SCTP is rejected at the parse boundary (confirm #164 boundary) `@US-02` `@error` `@in-memory` Tier 1

```gherkin
Scenario: An unsupported L4 protocol (sctp) is rejected before it can reach the dataplane
  Given a listener protocol token "sctp"
  When the protocol is parsed (Proto::try_from / FromStr, shipped by #164)
  Then parsing returns Err(UnknownProto), so sctp can never produce a ServiceFrontend
```
Confirms the existing `Proto` boundary (`backend_key.rs:100-110`,
`ParseError::UnknownProto`) — `Proto` admits only tcp/udp. This is a
*confirmation* scenario (boundary already shipped by #164); it pins that the
udp-support feature does not widen the proto admission set.

---

## US-03 — Tier 2 kernel reverse-NAT triptych (UDP)

### S-03-E — `xdp_reverse_nat_lookup` rewrites a proto=17 response source to the VIP `@US-03` `@K3` `@real-io` `@adapter-integration` Tier 2

```gherkin
Scenario: A UDP backend response is source-rewritten to the VIP by the kernel reverse-NAT program
  Given the REVERSE_NAT_MAP is populated with the key (backend_ip, backend_port, udp) mapping to the VIP   # SETUP
  And a synthetic UDP response packet (IPv4 proto=17) sourced from (backend_ip, backend_port)             # PKTGEN
  When xdp_reverse_nat_lookup runs against the packet via BPF_PROG_TEST_RUN                                # CHECK
  Then the output packet's source 5-tuple is rewritten to (vip, vip_port)
  And the verdict is the reverse-NAT egress verdict (not XDP_PASS with an unmodified frame)
```
Mirrors the TCP triptych at
`crates/overdrive-bpf/tests/integration/xdp_reverse_nat_redirect_neigh.rs`
(PKTGEN `synthesise_backend_response`, the `bpf_prog_test_run` helper,
header-rewrite assertion on `data_out`), with proto byte = 17 (UDP) and a UDP
header instead of TCP. Placement:
`crates/overdrive-bpf/tests/integration/xdp_reverse_nat_udp.rs`.

### S-03-F — Tier 2 boundary: a REVERSE_NAT_MAP miss for a udp packet returns `XDP_PASS` unmodified `@US-03` `@error` `@real-io` Tier 2

```gherkin
Scenario: A UDP response with no matching REVERSE_NAT entry passes unmodified
  Given an empty REVERSE_NAT_MAP
  And a synthetic UDP response packet (proto=17)
  When xdp_reverse_nat_lookup runs via BPF_PROG_TEST_RUN
  Then the verdict is XDP_PASS
  And the output frame is byte-identical to the input (no rewrite, no DROP_COUNTER slot consumed)
```
Mirrors the existing "REVERSE_NAT_MAP miss returns XDP_PASS unmodified" Tier 2
case (sibling test point 4), with proto=17.

---

## US-04 — Walking skeleton: single UDP listener forward + reverse e2e (Tier 3)

### S-04-A — Submit a UDP service via `overdrive deploy`; the reverse path carries the VIP source `@walking_skeleton` `@driving_adapter` `@US-04` `@K1` `@real-io` `@adapter-integration` Tier 3

```gherkin
Scenario: An operator deploys a single-UDP-listener service and the UDP round-trip carries the VIP source
  Given Ana has dns-resolver.toml declaring a udp listener on 5353 and a backend bound on 5353
  When Ana runs `overdrive deploy dns-resolver.toml` and a client sends a UDP datagram to the VIP
  Then the deploy exits 0 and prints "Accepted."
  And `bpftool map dump REVERSE_NAT_MAP` shows the (backend_ip, 5353, udp) key mapping to the VIP
  And a wire capture on the client veth shows the backend's reply sourced from the VIP (10.96.0.10:5353), not the backend IP
```
**Walking skeleton (driving adapter).** `overdrive deploy` is invoked via
**real subprocess** of the built binary (CLI verb `Deploy` — `cli.rs:42`,
`main.rs:63` — NOT `job submit`; see Blocker note in feature-delta DISTILL
[REF]). Asserts: exit code 0 + `Accepted.` stdout (the `workload_submit_accepted`
render shape) + the udp REVERSE_NAT key (`bpftool` dump) + the wire-capture VIP
source. Follows `reverse_nat_e2e` / `service_map_forward` Tier-3 shape; uses
`overdrive-testing` `ThreeIfaceTopology` netns/veth fixtures.
Placement: `crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs`
(wired into `tests/integration.rs`) for the dataplane wire half; the
subprocess-`overdrive deploy` driving-adapter half lands in
`crates/overdrive-control-plane/tests/integration/` or
`crates/overdrive-cli/tests/integration/` per the existing
`exec_spec_walking_skeleton` precedent.

### S-04-B — Every UDP reply is independently source-rewritten `@US-04` `@real-io` Tier 3

```gherkin
Scenario: Multiple UDP datagrams each get the VIP source rewrite
  Given the running single-UDP-listener service
  When the client sends three datagrams to the VIP
  Then all three replies are captured with the VIP as source (UDP is connectionless; each reply is independently rewritten)
```

### S-04-C — A missing-backend response is distinguished from a wrong-source response `@US-04` `@error` `@real-io` Tier 3

```gherkin
Scenario: No reply is not misreported as a source-rewrite failure
  Given a UDP service whose backend is NOT bound on the listener port
  When the client sends a datagram to the VIP
  Then no reply is captured
  And the test reports "no response", NOT a source-rewrite failure (only a reply sourced from the backend IP is the #163 defect)
```
The negative-distinguisher: separates "backend down" (no datagram) from "reverse
path broken" (datagram with backend source). Only the latter is #163.

---

## US-05 — Multi-listener (TCP + UDP) e2e (Tier 3)

### S-05-A — A two-listener service installs both protocols' paths via `overdrive deploy` `@US-05` `@K4` `@real-io` `@adapter-integration` Tier 3

```gherkin
Scenario: A TCP+UDP service has both forward+reverse paths working through the real chain
  Given Ana has edge.toml declaring tcp/8080 and udp/8081 and a backend bound on both ports
  When Ana runs `overdrive deploy edge.toml` and exercises both listeners
  Then the deploy exits 0 and the accepted output reflects the multi-listener service
  And the ServiceMapHydrator emitted one update_service per listener, each carrying its declared proto
  And two wire captures (one tcp, one udp) both show the reply sourced from the VIP
```
Driving adapter: `overdrive deploy edge.toml` (subprocess). Placement:
`crates/overdrive-dataplane/tests/integration/multi_listener_tcp_udp_e2e.rs`
(+ the subprocess deploy half alongside S-04-A).

### S-05-B — Each listener's reverse path is independently VIP-sourced `@US-05` `@real-io` Tier 3

```gherkin
Scenario: Both protocols' replies are source-rewritten to the same VIP
  Given the running two-listener service
  When a client exercises both the tcp and udp listeners
  Then the tcp reply and the udp reply are each captured with the VIP as source
```

### S-05-C — Adding a listener on re-submit converges without breaking existing paths `@US-05` `@real-io` Tier 3

```gherkin
Scenario: Re-submitting with an added udp listener installs the new path and preserves the existing two
  Given a running two-listener service (tcp/8080 + udp/8081)
  When Ana re-submits edge.toml with a third (udp/8082) listener
  Then the hydrator reconciles to three update_service calls
  And the new udp/8082 reverse path works and the existing two paths still work
```

---

## KPI scenario links (`@kpi`)

The feature's KPIs (K1–K5, `feature-delta.md` § Outcome KPIs) are **measured by
the tier assertions themselves**, not by separate metric-emission scenarios —
this feature emits no new metric event (no `kpi-contracts.yaml` entry maps to a
new emittable event here; soft gate). Mapping for traceability:

| KPI | Measured by scenario(s) |
|---|---|
| K1 (UDP reverse-path success) | S-04-A (wire capture source==VIP + `bpftool` dump) |
| K2 (Sim/Ebpf divergence caught pre-merge) | S-03-A + S-03-B + S-03-C + S-03-D (Tier 1) ; S-04-A (Tier 3 Ebpf half) |
| K3 (kernel rewrites proto=17) | S-03-E (Tier 2 triptych) |
| K4 (dual-protocol both paths work) | S-05-A + S-05-B |
| K5 (one typed surface, 0 positional reconstructions) | S-01-A/C (frontend carries proto) + code-review grep (C2 pass condition) |

No standalone `@kpi` observability scenario is warranted: there is no metric
event to assert emittable. Noted per the DISTILL soft gate.

---

## Scenario count by tier

| Tier | Scenarios | Of which error/edge |
|---|---|---|
| Tier 1 (DST / in-memory) | S-01-A, S-01-B, S-01-C, S-01-D, S-01-E, S-01-F, S-02-A, S-02-B, S-02-C, S-02-D, S-02-E, S-03-A, S-03-B, S-03-C, S-03-D = **15** | S-01-B, S-01-E, S-01-F, S-02-D, S-02-E, S-03-B, S-03-C, S-03-D = **8** |
| Tier 2 (BPF unit) | S-03-E, S-03-F = **2** | S-03-F = **1** |
| Tier 3 (real veth) | S-04-A (WS), S-04-B, S-04-C, S-05-A, S-05-B, S-05-C = **6** | S-04-C = **1** |
| **Total** | **23** | **10 (43%)** |

Error/edge ratio = 10/23 = **43%** (≥ 40% target met).
Walking skeleton: **1** (S-04-A), `@walking_skeleton @driving_adapter`.
`@property` (PBT-full, Tier 1 only): S-01-A, S-01-B, S-02-A, S-02-C, S-03-A = 5.
