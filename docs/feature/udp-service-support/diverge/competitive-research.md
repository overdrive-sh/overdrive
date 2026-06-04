# Competitive / Prior-Art Research — service-update surface & per-service L4 proto

> **Research depth: Lightweight** (Decision 2 — 3+ named references, known
> market, at least one non-obvious alternative). The question is narrow
> and empirical: *how do comparable eBPF / L4 dataplanes shape their
> service-update surface, and where do they carry the per-service L4
> protocol?* The answer is the empirical anchor for the taste score —
> specifically whether the **aggregate-with-proto** shape (option 2) or
> the **positional-proto** shape (option 1) is the industry-validated
> pattern.

## The single discriminating question

Every system below load-balances L4 traffic and must distinguish TCP from
UDP (and often SCTP) on the *same VIP+port*. The design choice each made
is: **is the protocol a field of the service/VIP identity tuple, or a
side-channel argument?** This maps directly onto our option 1 (positional
side-channel) vs option 2 (typed aggregate where proto is a first-class
field of the descriptor).

## Reference 1 — Cilium (`L4Addr.Protocol`; service map keyed by proto) — OBVIOUS, canonical

Cilium's userspace load-balancer model carries protocol as a field of the
**`L4Addr`** abstraction: `L4Addr` is "the backend port with a `L4Type`
(usually tcp or udp) and the Port number." Services and backends are
addressed through structures that bind `(address, port, protocol)` as a
unit. The BPF maps (`cilium_lb4_services_v2`, `cilium_lb4_backends_v3`)
are keyed by structures (`lb4_key`, `lb4_service`) that carry the L4 shape,
and the protocol flows through the service abstraction rather than as a
loose argument.

- **What it does well for the job:** the protocol is inseparable from the
  service identity — there is no code path where a service is described
  without its protocol, so a TCP-only reverse-path bug (our #163) is
  structurally hard to write: the key tuple *is* `(addr, port, proto)`.
- **Where it would fail our job:** Cilium's surface is a large aggregate
  (`lb4_service` carries flags, backend count, affinity, scope) — adopting
  the *full* Cilium aggregate would be over-modelled for Overdrive's
  Phase-2 single-node scope. The lesson is "proto belongs in the
  descriptor," not "copy Cilium's whole struct."
- **Assumption it makes about caller behavior:** callers always describe a
  service as a typed frontend, never as positional args.
- Source: [Service Load Balancing | cilium/cilium DeepWiki](https://deepwiki.com/cilium/cilium/2.6-service-load-balancing),
  [Cilium Standalone Layer 4 Load Balancer XDP](https://cilium.io/blog/2022/04/12/cilium-standalone-l4lb-xdp/).

## Reference 2 — Katran (`VipKey { address, port, proto }`) — OBVIOUS, the cleanest analogue

Meta's Katran is the closest structural analogue to Overdrive's XDP path.
Its VIP is described by **`VipKey`** (`lib/KatranLbStruct.h`): a struct of
`std::string address`, `uint16_t port`, `uint8_t proto`. To configure a
TCP VIP toward an HTTP port you supply `{address="10.0.0.1", port=80,
proto=IPPROTO_TCP}`. The `VipKey` struct *is* the BPF-map lookup key in the
forwarding plane — protocol is a co-equal field of the VIP identity, not a
separate argument.

- **What it does well for the job:** Katran cannot express "a VIP" without
  a protocol — `VipKey` makes the `(addr, port, proto)` triple atomic.
  This is exactly the shape our `BackendKey { ip, port, proto }` newtype
  already uses on the REVERSE_NAT side (C4). The forward-path descriptor
  in option 2 mirrors `VipKey` directly.
- **Where it would fail our job:** Katran's API is C++ config-object
  driven; the lesson transfers as "the service descriptor carries proto as
  a typed field," which is precisely option 2.
- **Assumption about caller behavior:** the control plane constructs a
  typed VIP key; there is no positional-argument fast path.
- Source: [katran/USAGE.md](https://github.com/facebookincubator/katran/blob/main/USAGE.md),
  [Open-sourcing Katran — Engineering at Meta](https://engineering.fb.com/2018/05/22/open-source/open-sourcing-katran-a-scalable-network-load-balancer/).

## Reference 3 — Kubernetes `ServicePort.Protocol` — OBVIOUS, the upstream-of-us shape

kube-proxy's `ServicePort` carries `Protocol` (TCP default; also UDP,
SCTP) as a named field of each port entry. A multi-port service is a
**list of `ServicePort`**, each with its own `(name, protocol, port,
targetPort)`. Kubernetes learned the hard way that protocol must be part
of the port's identity — issue #39188 ("Services with same port,
different protocol display wrongly … have wrong merge key") is the *exact*
failure shape #163 is: when protocol is NOT part of the key, two
listeners on the same port (one TCP, one UDP) collide.

- **What it does well for the job:** the `Vec<ServicePort>` shape is the
  direct precedent for our option 3 (per-listener descriptor) AND
  option 5 (descriptor-with-Vec-of-listeners) — multi-listener services
  fan out naturally because each port-entry carries its own protocol.
- **Where it fails the job:** kube-proxy is iptables/ipvs, not our XDP
  path — the *surface* transfers, the *mechanism* does not.
- **Assumption about caller behavior:** services are declared as a list of
  typed port entries; protocol is never a global default applied after the
  fact (C3 — "Proto is NEVER defaulted to Tcp").
- Source: [Protocols for Services | Kubernetes](https://kubernetes.io/docs/reference/networking/service-protocols/),
  [Issue #39188 — kubernetes/kubernetes](https://github.com/kubernetes/kubernetes/issues/39188).

## Reference 4 (NON-OBVIOUS) — loxilb / GLB split-tier: protocol carried but the *surface* can be thin

The non-obvious alternative comes from comparing two eBPF/L4 designs that
deliberately keep their **update surface thin** even though protocol is
still part of the key:

- **loxilb** (eBPF/GoLang L4 LB) models an L4 service and threads protocol
  through its service struct, but its internal map-update path is a
  focused per-rule write rather than a fat aggregate — the protocol rides
  the rule, not a large descriptor object.
- **GitHub GLB Director** is a *split L4/L7* design: the L4 director tier
  is deliberately stateless and minimal; protocol classification happens
  where the packet is, and the control surface that programs the director
  is narrow.

- **Why it's non-obvious / what it teaches:** these systems show that
  "protocol must be in the key" does NOT *force* a fat aggregate at the
  update surface. The minimal-thread option (our option 1) is a
  defensible point on the design spectrum — proto can be a focused field
  threaded into the key-derivation step (our Step 4b) without re-modelling
  the whole call as a descriptor object. This is the empirical
  counterweight that keeps the option study honest: the aggregate is
  *common* (Cilium/Katran/k8s) but not *universal*; a thin surface that
  still carries proto into the key is a real, shipped pattern.
- **Where it fails OUR job specifically:** the split-tier minimalism
  assumes the protocol is recoverable at the dataplane from the packet or
  an adjacent rule. In Overdrive the protocol is *intent* (declared in the
  TOML listener) and must be *carried* from intent → hydrator →
  `update_service` — it cannot be re-derived from the packet at install
  time. So the "thin surface" only works if proto is threaded explicitly,
  which is what option 1 does (and what makes option 4 — "key by VIP+proto
  with proto carried some other way" — fragile, see options-raw.md).
- **Assumption about caller behavior:** the update path is per-rule/narrow;
  the caller does not assemble a service aggregate.
- Source: [loxilb-io/loxilb](https://github.com/loxilb-io/loxilb),
  [github/glb-director](https://github.com/github/glb-director),
  [Introducing the GitHub Load Balancer](https://github.blog/engineering/infrastructure/introducing-glb/).

## Synthesis — what prior art says about option 1 vs option 2

| Finding | Implication for the option study |
|---|---|
| **3 of 4 references (Cilium, Katran, k8s) bind protocol into the service/VIP/port *identity tuple*, not a side-channel arg.** | Strong empirical support for the *aggregate-with-proto* family (options 2, 3, 5). The industry-validated pattern is "proto is a field of the service descriptor / key." |
| **Katran's `VipKey { addr, port, proto }` is byte-for-byte the shape of our existing `BackendKey { ip, port, proto }` newtype (C4).** | Option 2's forward-path descriptor is the *symmetric twin* of a key shape we already ship. This lowers option 2's concept-count cost: the engineer already knows `(ip, port, proto)` from the reverse side. |
| **Kubernetes #39188 is literally #163's failure class** (protocol not in the key → same-port TCP/UDP collide). | The job's O2 (no silent divergence) and the multi-listener requirement (O5, US-05) are *industry-known* failure modes that the aggregate/per-listener shape (options 2/3/5) prevents by construction. |
| **loxilb / GLB show a thin per-rule surface CAN carry proto without a fat aggregate** (non-obvious). | Option 1 (minimal positional proto) is NOT a strawman — it is a real shipped point on the spectrum. It must be scored honestly, not dismissed. Its weakness is O5 (extension/multi-listener), not O1–O3. |
| **No named system re-derives protocol from the packet at install time when protocol is declared intent.** | Kills option 4 (key by VIP+proto with proto carried "some other way") on feasibility — proto must be threaded explicitly from intent; there is no honest side-channel. |

**Bottom line for the taste score:** prior art validates the
*aggregate/key-tuple* family as the dominant pattern (Cilium, Katran,
k8s) while confirming the *thin positional* surface as a real minority
pattern (loxilb/GLB). The empirical anchor therefore does NOT
auto-decide option 2 — it raises option 2's Desirability/longevity score
*and* establishes that option 1 is a legitimate contender on Subtraction.
The discriminator is O4 (scattered args) + O5 (extension cost) +
service_id reconciliation — scored in taste-evaluation.md.

## Gate check

- [x] 3+ real products named with cited behaviors (Cilium, Katran,
  kube-proxy, loxilb/GLB).
- [x] At least one non-obvious alternative (loxilb/GLB split-tier thin
  surface — a *different category*, serving the same "carry L4 proto"
  job with a deliberately minimal surface).
- [x] No generic market claims — every claim cites a named struct
  (`L4Addr.Protocol`, `VipKey`, `ServicePort.Protocol`) and a source URL.
