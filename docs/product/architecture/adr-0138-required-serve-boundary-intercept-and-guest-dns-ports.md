# ADR-0138 — Compose the intercept and guest-DNS owners from required serve-boundary ports

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R16. Proposed 2026-09-23; the exact shape was reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. The shape class is fixed by a user ruling of
2026-09-23: required ports composed at the `serve` boundary like the KEK, not
optional override fields. Where the ports travel and how the mTLS enforcement
is built follow the user rulings NO-OVERRIDE-SEAMS and its scope ruling
(2026-10-19): the ports are members of the driven-adapter bundle the
composition root passes to `run_server` (ADR-0219). Not yet implemented. This
ADR decides which ports. It reverses ADR-0072's "no DNS port trait / no sim
adapter" sub-decision; ADR-0072 states that amendment explicitly. Exact
signatures live only in the #295 feature delta, § *[REF] Composition*,
"Driven adapters of `run_server`".

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

The mTLS enforcement depends on state that exists only inside the boot (the
identity holder and the guest dial), and its Earned-Trust probe refusal could
be exercised only through a probe-fault configuration field.

Constructor-declared, non-optional dependencies are how Cilium's hive wires its
agent components, and Cilium places its DNS proxy behind an explicit component
contract. Whether DNS must be present at all is a product decision here
(ADR-0072), not a convention: Kubernetes' NodeLocal DNSCache is optional.
(Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Finding 10.1.)

## Decision

The composition root passes three required ports to `run_server`, as members
of its driven-adapter bundle (ADR-0219), alongside the KEK:

- the transparent-intercept port;
- a guest-DNS responder factory;
- an mTLS-enforcement factory, which receives the boot's identity holder,
  limits, guest prefix and guest dial and returns the enforcement the boot
  then probes.

`overdrive serve` passes the host adapters. The mTLS worker, its enforcement,
the DNS task owner, and the shared-network supervisor are composed
unconditionally in every composition. No optional field can disable them, and
no configuration field carries one of these ports or injects a fault into it.

The DNS responder gains a doc-hidden port covering probe, serve, audit, and
stop. The existing `DnsResponder` implements it, and so does the sim adapter.
The pure `answer_for` and encoder seam from ADR-0072 remains. Real wire and
source-pin evidence stays Tier 3.

## Alternatives considered

### Intercept port only

Rejected. `Dns` recovery and DNS task-loss classes stay unreachable in-process,
and in-process compositions still collide on `:53`.

### Keep the mTLS enforcement composed inside the boot

Builds the enforcement over the real identity holder with no port. Its probe
refusal is then reachable in-process only through a fault field in the
configuration, which the NO-OVERRIDE-SEAMS scope ruling removes, and a test
that needs another identity has no way to supply it. Rejected; the factory
receives the boot-only state, so it covers both.

### An identity factory instead of an enforcement factory

Substitutes the identity the enforcement reads but leaves its probe refusal
unreachable without a fault field. Rejected.

### Optional override fields

Rejected by the user ruling. An optional override lets a test configuration
silently change the production composition.

## Consequences

Positive: every supervisor component and task-loss class becomes reachable
in-process through the server composition that `run_server` delegates to. The
production composition can no longer be disabled by a test field. A test
scripts an enforcement probe failure on a simulation factory.

Negative:

- Every adapter bundle must supply all three ports.
- ADR-0072's no-port DNS stance is reversed. The DNS wire contract is not.
- The `dns_probe_fault` test field is removed, because the sim DNS adapter
  expresses that fault; likewise no enforcement probe-fault field exists.
- A composition that passes the host enforcement factory runs the real
  enforcement probe, which is host kTLS I/O; those lanes run under Lima as
  root and are not deterministic simulation. A composition with a simulation
  factory does not.
- Seeded simulation of the supervisor runs source-locally in
  `overdrive-control-plane`, outside the server composition. It uses the sim
  adapters of ports declared in `overdrive-core` and `overdrive-worker`, and
  test-local implementations of the shared-owner and guest-DNS ports that
  `overdrive-control-plane` declares. `overdrive-sim` depends on
  `overdrive-control-plane`, so its implementations of those ports cannot be
  used by that crate's own source-local tests; they serve its integration
  suites and other crates.
- The action shim's intercept lifecycle becomes a required parameter. No
  composition can raise a guest TAP without intercept-live.
