# ADR-0138 — Compose the intercept and guest-DNS owners from required serve-boundary ports

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R16. Proposed 2026-09-23; the exact shape was reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. The shape class is fixed by a user ruling of
2026-09-23: required `ServerConfig` ports composed at the `serve` boundary like
the KEK, not optional override fields. This ADR decides which ports. It
reverses ADR-0072's "no DNS port trait / no sim adapter" sub-decision; ADR-0072
states that amendment explicitly. Exact signatures live only in
the #295 feature delta.

## Context

Several shared-network supervisor components cannot be driven through the real
composition root in-process. The seeded supervisor proof found five
unreachable components, `IpRules`, `IpSets`, `LegF`, `LegC`, and `Dns`, plus the
listener and DNS task-loss classes. Three causes:

- The mTLS worker is built only when the test-only `dataplane_override` field is
  unset. Tests that set it therefore silently lose the production mTLS, DNS, and
  supervisor composition. This is the test-only-state trap recorded for #248.
- The worker is always built over the real host intercept adapter.
- The DNS owner always binds the real `:53`, a process-global resource.

Constructor-declared, non-optional dependencies are how Cilium's hive wires its
agent components, and Cilium places its DNS proxy behind an explicit component
contract. Whether DNS must be present at all is a product decision here
(ADR-0072), not a convention: Kubernetes' NodeLocal DNSCache is optional.
(Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Finding 10.1.)

## Decision

`ServerConfig` requires two ports at construction, alongside the KEK:

- the transparent-intercept port;
- a guest-DNS responder factory.

`overdrive serve` composes the host adapters. The mTLS worker, the DNS task
owner, and the shared-network supervisor are composed unconditionally in every
composition. No optional field can disable them.

The DNS responder gains a doc-hidden port covering probe, serve, audit, and
stop. The existing `DnsResponder` implements it, and so does the sim adapter.
The pure `answer_for` and encoder seam from ADR-0072 remains. Real wire and
source-pin evidence stays Tier 3.

The mTLS enforcement adapter stays composed internally over the real identity
holder. It is not a supervisor component, and it depends on state created
inside `run_server`.

## Alternatives considered

### Intercept port only

Rejected. `Dns` recovery and DNS task-loss classes stay unreachable in-process,
and in-process compositions still collide on `:53`.

### Also require an mTLS-enforcement factory port

Not proposed. No supervisor component depends on it. Enforcement needs the
in-process identity holder, and its real probe already runs in the Lima lane.

### Optional override fields

Rejected by the user ruling. An optional override lets a test configuration
silently change the production composition.

## Consequences

Positive: every supervisor component and task-loss class becomes reachable
through the real composition root. The production composition can no longer be
disabled by a test field.

Negative:

- Every `ServerConfig` construction must supply both ports.
- ADR-0072's no-port DNS stance is reversed. The DNS wire contract is not.
- The `dns_probe_fault` test field is removed, because the sim DNS adapter
  expresses that fault.
- Every in-process `run_server` composition includes the real enforcement
  probe, which is host kTLS I/O. Those lanes therefore run under Lima as root
  and are not deterministic simulation. Seeded simulation of the supervisor
  runs source-locally over sim adapters, outside `run_server`.
- The action shim's intercept lifecycle becomes a required parameter. No
  composition can raise a guest TAP without intercept-live.
