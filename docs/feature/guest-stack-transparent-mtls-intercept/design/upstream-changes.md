# DESIGN → DISTILL upstream changes — bounded terminal and network corrections

**Feature:** guest-stack-transparent-mtls-intercept (GH #222)
**Date:** 2026-08-31
**Authority:** feature delta, ADR-0037, ADR-0083, and ADR-0089

This amendment replaces the rejected replay-oriented terminal-race design with
three bounded corrections to production paths that already exist. It changes no
DISTILL artifact, roadmap, public API, observation schema, probe schema, broker
contract, or recovery protocol.

## Source evidence

| Boundary | Current production evidence | Correction |
|---|---|---|
| Stop versus exit observer | `Driver::stop` sets `intentional_stop` before signalling and awaits the exit watcher (`crates/overdrive-worker/src/driver.rs:660-706`). The observer then writes a dominating `Terminated { by: Operator }` row through the same compound LWW method (`crates/overdrive-control-plane/src/worker/exit_observer.rs:458-549`). Action dispatch itself awaits actions sequentially (`crates/overdrive-control-plane/src/action_shim/mod.rs:777-887`), so that observer is the concrete concurrent writer. `StopAllocation` currently re-reads and retries in an unbounded loop (`:2606-2695`). | Make exactly two fresh-read proposals. One rejected proposal covers the expected observer race; a second rejection ends the action successfully after the existing supervision/route tail, with no fabricated event. |
| Post-assignment provision failure | The C3 seam assigns a slot before netns/veth/TAP provisioning (`action_shim/mod.rs:1113-1148`), while the current failure arm records `Failed` without first invoking structural teardown (`action_shim/mod.rs:1762-1783`). The existing teardown helper derives the held slot, tears down, and releases only after success (`action_shim/mod.rs:1317-1333`); the provisioner removes netns, stranded owned TAP, host veth, and resolver state idempotently (`crates/overdrive-control-plane/src/veth_provisioner.rs:2171-2212`). | After an assigned provision fails, invoke that existing helper before returning. A cleanup failure retains the slot because release follows successful teardown. |
| Same-id restart ordering | Restart already awaits every resolved prior-driver stop (`action_shim/mod.rs:2136-2158`) but currently begins replacement provisioning immediately afterward (`action_shim/mod.rs:2168-2188`). Existing mTLS and structural teardown operations are `worker.stop_alloc(...).await` and `teardown_and_release_netns` (`action_shim/mod.rs:1357-1370`). | Await prior mTLS teardown, then complete prior structural teardown/slot release, before any replacement provision, identity issue, or driver start. |
| Production cancellation model | Graceful shutdown cancels and then joins the convergence task, explicitly waiting for its active tick to finish through action dispatch (`crates/overdrive-control-plane/src/lib.rs:1396-1417`); the loop observes cancellation only between drained batches (`lib.rs:3355-3396`). Hard process loss also destroys the process-local driver route. | No action-level cancellation partition, detached completion owner, tail replay, or cancellation recovery contract is designed. |

## Contract 1 — bounded terminal contention

`StopAllocation` keeps the existing cleanup-first order and compound
`ObservationStore::write_alloc_lifecycle` boundary. After cleanup it performs
at most two proposals, each built from a fresh current-row read and a dominating
logical timestamp:

1. `Ok(Some(occurrence))`: run the existing no-await local tail — release
   supervision, remove the existing `AllocDriverIndex` entry, and best-effort
   broadcast that accepted occurrence.
2. A fresh read already has the exact requested terminal: release supervision,
   remove the route, emit nothing, and return `Ok(())`.
3. First `Ok(None)`: another author won LWW; re-read once and make the second
   proposal.
4. Second `Ok(None)`: the bound is exhausted. Release supervision, remove the
   existing route, emit no occurrence/event, and return `Ok(())`. The competing
   accepted row remains the durable truth.
5. Read/write `Err`: preserve the existing typed error and atomic no-partial-
   commit behavior.

There is no sleep, configurable budget, WorkloadLifecycle replay branch,
generation fence, terminal-tail target, route hydration, receipt, outbox,
detached future, or retry subsystem. `AllocDriverIndex` remains the existing
per-boot routing cache; its miss already broadcasts a best-effort stop to every
composed driver (`action_shim/mod.rs:717-742`).

## Contract 2 — total post-assignment provision unwind

Slot exhaustion remains a pre-assignment failure and needs no cleanup. Once
`NetSlotAllocator::assign` has succeeded, every
`network_provisioner.provision` error invokes the existing
`teardown_and_release_netns_raw` path, captures that result, and then records
the existing Failed disposition:

- teardown removes only the allocation's existing slot-derived resources;
- benign absence remains success;
- the slot is released only if teardown succeeds;
- teardown failure retains the slot and returns the existing typed
  `VethProvisionError`/`ShimError`;
- the durable `Failed` row continues to carry the original
  `WorkloadNetnsProvisionFailed` cause; no aggregate error or cleanup schema is
  added; and
- after the Failed write, a store error takes its existing precedence;
  otherwise a captured teardown error is returned through its existing
  `ShimError`, and full success returns `Ok(())`.

No boot-GC state machine, namespace-path classification, marker, scanner, or
new persistence is part of this correction.

## Contract 3 — prior protection is gone before replacement work

The same-id `RestartAllocation` sequence is:

```text
read existing attempt fence
  -> await every resolved prior Driver::stop (NotFound is absence)
  -> await prior MtlsInterceptWorker::stop_alloc
  -> teardown prior netns/veth/TAP/resolver state and release its slot
  -> provision/inject the replacement network
  -> ensure replacement identity
  -> start replacement driver
  -> retain the existing Running-write/intercept-install/EXEC-release tail
```

Failure of prior mTLS or structural teardown returns its existing typed error
before replacement work starts. Any failure after the replacement receives a
slot uses Contract 2's same resource-specific unwind. No
`RestartNetworkDisposition`, replay token, or second cleanup protocol is
sanctioned.

## Explicitly removed from the normative design

- WorkloadLifecycle terminal/tail replay and replay-specific generation
  placement changes.
- `AllocDriverRouteView`, route hydration, and durable/process-local route
  correlation.
- New broker self-enqueue, watch, Lagged, or periodic-relist requirements.
- ServiceLifecycle terminal/route facts, liveness-attempt state, and cross-
  reconciler handoff machinery.
- `ProbeResultRow` V2, logical-attempt persistence, and probe/driver signature
  propagation.
- Arbitrary action-cancellation partitions and cancellation-tail repair.
- Expanded boot-GC/netns path-state machinery.

The historical review artifact remains unchanged; its next reviewer-owned
iteration must record that those expanded proposals were rejected and
superseded by this amendment.
