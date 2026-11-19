# ADR-0325 — The `guest` substrate joins the closed benchmark registry

## Status

**Accepted (2026-11-19).** Approved by the user as PD-1. Amends [ADR-0299](adr-0299-benchmark-substrates-are-a-closed-registry-and-the-first-entry-is-native.md), whose decision is operative. DDD-1 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

ADR-0299 registers one substrate, `native`, and makes `substrate = guest` a build error ("guest substrates are not available to benchmarks"), tracking the gap as [#362](https://github.com/overdrive-sh/overdrive/issues/362). A component that runs inside a workload VM in production (the `overdrive-init` guest flow owner) cannot be measured by a native number. The registry is closed and checked by constant evaluation; that mechanism stays.

## Decision

The registry is `["native", "guest"]`. `guest` means: the benchmark's measuring half runs inside a guest-kit guest that the benchmark process launches directly on the exclusive Metal host, not inside an outer test guest. The attribute takes the substrate as a bare identifier. The "guest substrates are not available" refusal does not exist; an identifier outside the registry is a build error naming it and listing the registered substrates (`native, guest`). `native` is unchanged. A guest-group name in `smoke(guest = "<group>")` is a test placement, not a substrate, and stays in the ADR-0294 registry.

A `guest` benchmark declares no `bpf_stats`: the runner's `BPF_ENABLE_STATS` descriptor is held by the host process and does not cover a guest kernel, so the record would claim statistics that are not in effect. Like the D9 rule (a family that launches VMs stays `native`), this is stated in the catalogue and the methodology and enforced by review; no mechanical check exists. Reading BPF program statistics inside a guest is a scope limitation tracked by [#371](https://github.com/overdrive-sh/overdrive/issues/371) ("Benchmark program: read BPF program run statistics inside a guest").

## Alternatives

- **A `substrate = guest("<group>")` form naming a placement group**: conflates a measuring environment with a test environment; a test guest group is exactly the outer guest a benchmark must not run in (nested virtualization, which production does not have). Rejected.
- **A third entry `guest_nested`**: no family measures in a nested guest and ADR-0299's reason for refusing it stands. Rejected.
- **A build error for `bpf_stats` on a `guest` benchmark**: one constant-evaluated message and one compile-fail fixture for a state no declared family reaches. Not added.

## Consequences

ADR-0299's "scope limitation tracked by #362" paragraph no longer applies to the substrate registry. Records of different substrates are never compared (no command takes two records). VM-lifecycle families stay `native` (a guest that launches VMs measures nested virtualization; [#347](https://github.com/overdrive-sh/overdrive/issues/347)). The compile-fail fixture `guest_substrate` is replaced by one asserting the acceptance; `unknown_substrate` asserts the refusal with the new list in its message.

## Links

- Interface contract: [feature delta, DDD-1](../../feature/guest-benchmark-substrate/feature-delta.md)
