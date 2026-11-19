# ADR-0329 — A guest benchmark boots a fresh guest for every replicate

## Status

**Accepted (2026-11-19).** Approved by the user as PD-5. DDD-5 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md). The cost this decision rests on was measured on the Metal host (validation item V-A, held).

## Context

The methodology says every replicate builds a fresh fixture and no state survives from one replicate to the next ([ADR-0300](adr-0300-the-runner-owns-the-statistical-protocol-and-versions-it-as-the-methodology.md)). For a guest benchmark the fixture includes the guest: the kernel's socket tables, the guest flow owner's flow table and UDP slots, the forwarder module's pair state and the page cache all carry state from earlier flows. The cost is one guest boot and shutdown per replicate. One whole guest lifecycle (image build and copy, launch to peer ready, one exec, shutdown to VMM exit) measured p50 1,078 ms, p95 1,132 ms, max 1,174 ms over 90 sequential cycles on the Metal host (Linux 7.0.0-38-generic), with no failure; sixty fresh guests cost about 65 s.

## Decision

A replicate boots a fresh guest from the benchmark image, runs the in-guest program once, ends the guest and reads the result. No guest outlives its replicate. The record states `guest.boot = "per_replicate"`, the only value the validator accepts. A guest that cannot boot (including an unprepared guest kit, `LaneNotPrepared`, whose cause names `cargo xtask guest-kit-prepare`), dies, or returns no intact result is a failed replicate (counted, with the stated reason); the run continues with the next replicate, and a guest's death never ends the run. An unprepared kit therefore fails every replicate before any guest launches and is not a refusal (ADR-0328, feature delta CA-4); a run-level abort for that one cause would need an abort a body does not have.

## Alternatives

- **One guest per run, a replicate being a fixture inside it**: about 1.2 s of lifecycle for sixty replicates, so it saves about a minute, but state survives between replicates (the owner's tables and the kernel's caches), contradicting the methodology's isolation rule and making replicate 30 differ from replicate 1 for reasons the A/A control cannot separate. Rejected: the saving is small against the cost of a result whose replicates are not independent.
- **One guest per profile**: same objection within a profile. Rejected.

## Consequences

The scripted-peer lifecycle floor is about 1.1 s per guest (V-A). For the production-init guest with a workload and the console return (ADR-0330), a whole replicate cycle measured about 2.75 s at the median (launch to READY 1.10 s, EXEC to EXIT 1.58 s, of which the workload about 0.89 s and the console print about 0.65 s; V-B2), so 30 replicates cost about 83 s. The margin to a 30 s per-cycle bound is about 11-fold. A guest death in replicate 12 of 30 leaves replicates 1 to 11 retained and 12 failed, and the run goes on to 13 to 30 and finishes; a run is "incomplete" only when the benchmark process itself ends without a terminal line (feature delta CA-1). The floor of about 1.1 s of lifecycle per replicate follows from the fresh-fixture rule; a family chooses its `replicates` value knowing it.

## Links

- [feature delta, DDD-5, V-A](../../feature/guest-benchmark-substrate/feature-delta.md)
- [V-A findings](../../feature/guest-benchmark-substrate/spike/findings-guest-boot-cost.md)
- [V-B2 findings](../../feature/guest-benchmark-substrate/spike/findings-production-init-return.md)
