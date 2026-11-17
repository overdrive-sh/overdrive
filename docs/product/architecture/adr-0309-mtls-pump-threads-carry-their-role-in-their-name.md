# ADR-0309 — mTLS pump threads carry their role in their name

## Status

**Accepted.** Approved by the user 2026-11-17, including this production change; independent design review resolved 2026-11-17. DDD-14 in the feature delta.

## Context

The resource story needs forward-path work reported apart from return-path work for
the outbound splice path. The two pump roles exist as diagnostic labels
(`PumpRole::OutboundForward`, `OutboundReturn`), but the pump threads are unnamed
`std::thread::spawn` calls (`crates/overdrive-dataplane/src/mtls/splice.rs:231`
and `:260`), so a per-thread reading from `/proc/<pid>/task/<tid>/stat` cannot
tell them apart. `bytes_spliced` is crate-private.

## Decision

The two synchronous pump constructors take a required thread name, and each
outbound pump thread is created with a name that carries its role:
`mtls-out-fwd` for the forward pump and `mtls-out-ret` for the return pump (each
under the 15-byte limit of a thread name). Every other caller of those
constructors (unit tests) passes its own distinct name, and the inbound
`spawn_async` pump is unchanged. A thread-spawn failure surfaces as the existing
`MtlsEnforcementError::Io` for that connection. The exact signature is in the
feature delta (DDD-14). The benchmark sums per-thread user and
system time by thread name over the sustained-transfer window. This is the only
production change the mTLS family needs; the inbound path is an asynchronous
relay on shared runtime threads, so its split is reported absent. V-3 checks that
the per-thread readings account for the work before approval.

## Alternatives

- **Expose per-role CPU or byte counters on the enforcement trait**: new public
  surface to recompute what the kernel already accounts per thread; rejected.
- **Attribute by cgroup**: the pumps share one cgroup, and softirq and kworker time
  is charged elsewhere (research F3.2); rejected.
- **Leave unnamed and infer by byte flow**: a heuristic around a fact the design
  can make observable; rejected.

## Consequences

Thread names also appear in `top -H` and `ps -L`, which helps an operator. A thread
that exits before the sample is not counted, so the split is defined only over
sustained transfers.

## Links

- [Decision DDD-14 and the names](../../feature/native-benchmark-program/feature-delta.md#ddd-14--mtls-pump-thread-names)
