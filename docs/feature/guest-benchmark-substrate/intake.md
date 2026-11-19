# Guest benchmark substrate — intake

Feature ID: `guest-benchmark-substrate` · Anchor issue: [#362](https://github.com/overdrive-sh/overdrive/issues/362)
Classification: infrastructure / tooling, brownfield (extends `native-benchmark-program`). Gathered 2026-11-19.

## User requirements (from the request)

1. Lift the limitation recorded in #362: the benchmark program measures only on the
   `native` substrate (closed one-entry registry, DDD-4, ADR-0299). Add a way to
   benchmark components that run **inside a workload VM** in production.
2. Run everything on the Metal box (`cargo xtask metal run --`); no Lima.
3. Subagents run on Sonnet 5.5.

## What #362 says lifting it takes

- A `guest` substrate entry in the registry, run through the guest kit (GuestKit) on Metal.
- A runner half inside the guest, writing the same record schema, with the host half collecting it.
- The guest's view of host settings recorded, since a guest cannot see turbo, SMT or
  frequency. The record must also mark vCPU scheduling, because the guest kit's 2 vCPUs are
  not pinned today.
- Guest results compared only with guest results; the record's substrate field already enforces this.
- Not for VM-lifecycle families: a guest that launches VMs measures nested virtualization,
  which production does not have.

## Related

- #287 / `docs/feature/native-benchmark-program/`: the benchmark program this extends.
- #303: in-guest kernel mTLS, the first consumer (compare with the host proxy).
- #361: regression-gate policy (separate, out of scope).
