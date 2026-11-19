# ADR-0299 — The service-map hydrator reads the programmed service map

## Status

Proposed. 2026-11-17. Author: Morgan (`nw-solution-architect`), DESIGN wave, APPLICATION
layer, feature `reconciler-convergence-guarantees`. The user approved the direction and the
read mechanism on 2026-11-17 (feature delta D-20, option (a), and the fingerprint read). The
ADR stays `Proposed` until independent design review completes.

Supersedes in part [ADR-0042](adr-0042-service-map-hydrator-reconciler.md): § 3's statement
that the hydrator's `actual` is the `service_hydration_results` row. The rest of ADR-0042
stands, including the table and the executor that writes it.

**Amendment 2026-11-19 — the result row no longer exists.** The "executor's receipt for
operators" in the Decision section was removed: the `service_hydration_results` row, its
types and its store table are deleted, and so is the executor's write of it. Nothing read the
row, and the observation store keeps last-write-wins current state, so the row could not
record that a failure occurred. The `DataplaneUpdateService` executor calls
`Dataplane::update_service`, returns nothing, and on failure emits one structured
`tracing::warn!` event `dataplane.update_service.failed` (`service_id`, `fingerprint`,
`reason`). The hydrator's `actual` and its retry from the programmed-map diff are unchanged.
The Decision, Alternatives and Consequences text below is the original and is not rewritten.

## Context

The `ServiceMapHydrator` emits `DataplaneUpdateService`. It decides whether that effect is
in place from `service_hydration_results`, the executor's receipt of the last update. The
service map is the resource the hydrator manages, and the hydrator never reads it.

After a node reboot, or a restart of `serve` (which recreates the pinned `SERVICE_MAP`), the
result rows persist in redb and the kernel maps are empty. A `Completed` row over the
desired fingerprint reads as converged, so the hydrator emits nothing and the service is
never reprogrammed. The same happens if the map is lost while the process runs. The resync
pass of ADR-0296 re-evaluates the target every period and changes nothing, because the
evaluation reads the same receipt. The hydrator property of the feature's DISTILL work
found this in the reboot world.

Appliance facts: only Overdrive writes the node's maps, so the reachable losses are
Overdrive's own: a reboot, a restart of `serve`, or a crash between a map write and its
receipt. The decision does not defend against foreign mutation of the maps.

Reading the programmed set back from the kernel is possible but not cheap. The inner Maglev
array for a frontend has 16,381 slots and encodes slot ownership, not membership, so a read
of the table must scan it and then look each owner up in `BACKEND_MAP`. Measured on kernel
7.0.0-38-generic with 1,000 frontends of 16 backends, that scan costs 5.41 ms p50 per
frontend (5.84 s for a pass), and the reconstructed set is exact only while every backend
owns at least one slot (a backend with a tiny weight beside a 65,535-weight backend owns
none).

## Decision

The hydrator's `actual` is read from the programmed service map for the service's frontend,
through a narrow read-only port on the dataplane adapter, reached from `hydrate_actual`
through the hydration context. The hydrator emits `DataplaneUpdateService` when the
programmed state differs from the desired programmable set and its retry pacing allows it.
The result row remains the executor's receipt for operators and is no longer an input to
`reconcile`. The retry memory decides only when to act again, never whether the effect is in
place.

The read returns a fingerprint stored in the resource. When `update_service` builds a new
inner `SERVICE_MAP` array, it writes the 64-bit fingerprint of the applied backend slice into
two extra slots of that array (16,381 and 16,382, the low and the high 32 bits) before the
atomic outer swap. The inner capacity is 16,383; XDP still indexes the table modulo 16,381, so
it never selects the extra slots. The read is an outer lookup that returns the inner map id,
`BPF_MAP_GET_FD_BY_ID`, then two element lookups, all under the dataplane's apply-commit guard
for read. The hydrator compares the fingerprint it reads with the fingerprint of the set it
would apply, computed by the same function over the same slice. An absent slot is an empty
`actual`.

The fingerprint covers weights and health, so the full-set read's precondition (every backend
owns a Maglev slot) does not apply. It is not the View-marker anti-pattern of
`.claude/rules/reconcilers.md`: the marker there is a hash the reconciler keeps of what it
emitted, stamped before the effect; this one lives in the kernel resource, is written
atomically with the table, and disappears with it.

The write port and the read port reach the hydrator from one dataplane instance. The control
plane carries them as one value that can be built only from a single concrete instance
implementing both traits, so a read over a different instance from the one the action shim
writes through cannot be constructed. The composition root, every `DataplaneComposer` and
`AppState` hold that value, and `build_hydration_context` reads the view from it.

`update_service` refuses a backend set that contains an IPv6 backend, with `LoadFailed` and
before any write, so the prior programmed state and its fingerprint are unchanged. The
fingerprint covers every backend of the applied slice; a call that returned `Ok` after
dropping a backend would store a fingerprint of a set it did not program, and the hydrator
would judge the service converged. Production builds only IPv4 backends, so the refusal
governs a state no production path reaches; it makes the trait contract unambiguous and the
two adapters equal.

The port's exact contract, the value's shape, its construction sites and the IPv6 contract
are in the feature delta (D-20).

Kernel facts the decision rests on, read in `vendors/linux` at `v7.0`: a userspace lookup on
a `HASH_OF_MAPS` returns the inner map id (`kernel/bpf/syscall.c:329-330`,
`kernel/bpf/hashtab.c:2626-2643`, `kernel/bpf/map_in_map.c:131-134`);
`BPF_MAP_GET_FD_BY_ID` needs `CAP_SYS_ADMIN` (`kernel/bpf/syscall.c:4859-4888`), which the
node process has on the appliance; the outer slot is replaced under the hash bucket lock
(`kernel/bpf/hashtab.c:2646-2667`); the kernel rejects an inner array whose `max_entries`
differs from the prototype's (`kernel/bpf/arraymap.c:594-601`), so the prototype and every
inner map carry 16,383.

## Alternatives considered

| Option | Why it loses |
|---|---|
| Keep the result row as `actual` and invalidate the rows at boot | Fixes a reboot and a restart. Misses a map lost while the process runs, and gives a second owner (boot) the job of making a receipt true. The user rejected it. |
| Read the full table: batch scan of the 16,381-slot inner array plus `BACKEND_MAP` lookups | Measured 5.41 ms p50 and 8.21 ms p99 per frontend, 5.84 s for 1,000 frontends, about 17% of a core at the 30 s period. The kernel's batch lookup walks the array one key at a time (`kernel/bpf/syscall.c:2093-2108 @ v7.0`). It is exact only while every backend owns a Maglev slot, a precondition the contract would have to state and a future non-uniform weight would break. |
| Make the inner maps `BPF_F_MMAPABLE` and read the table through `mmap` | Measured 107 µs p50 (112 ms for 1,000 frontends) before the `BACKEND_MAP` lookups, about 80 µs more at 16 backends. About 20 times the fingerprint's cost, and it keeps the slot-starvation precondition. The kernel allows it for arrays (`kernel/bpf/arraymap.c:66-68, 578-591 @ v7.0`) but the prototype and every inner map must carry the flag (`kernel/bpf/map_in_map.c:90 @ v7.0`), and the reader must open the map read-write (`mm/mmap.c:465`, `kernel/bpf/syscall.c:1095-1096 @ v7.0`). |
| Read only whether the outer slot exists and keep the result row as the diff for content | Cheaper per read. Leaves a receipt as the diff for whether the right backends are programmed, which is the defect class `reconcilers.md` names. |
| Add the read method to the `Dataplane` trait | `Dataplane` is the write port the executor uses. Hydration reads only through narrow read-only ports on the hydration context (ADR-0086 D5); a read method on the write port would let the hydrator reach writes through the same object. |
| Give `AppState` and the composers a dataplane and a view as two independent values | A view over an instance other than the one the shim writes through reads a different resource, and nothing in the types prevents it. One value built from one concrete instance makes the mismatch unrepresentable. |
| Skip an IPv6 backend in `update_service` and return `Ok` | The fingerprint would cover a backend the dataplane never programmed, so the hydrator would read equal and stop. The host adapter refuses; the sim adapter matches it. |
| Derive `actual` from the dataplane's in-process tracker of what it last wrote | The tracker is a record of the dataplane's own writes, a receipt held in memory. It does not see a map lost while the process runs. |
| Have the dataplane replay programming from observation at boot | Two owners of one effect, and a boot-time path that does not run for a map lost while running. The hydrator already owns the diff (`reconcilers.md` § "Not a candidate"). |

Measurements: V-6 findings,
`docs/feature/reconciler-convergence-guarantees/spike/findings-programmed-service-map-read.md`
§ "Comparison of read mechanisms", all three mechanisms on one host, kernel 7.0.0-38-generic,
1,000 frontends of 16 backends. Fingerprint: read p50 4.7 µs, p99 6.3 µs, sweep 6.2 ms,
apply p50 13.5 ms (baseline apply 17.2 ms).

## Consequences

Positive:

- The reboot, restart and lost-map worlds converge through the existing resync pass, with no
  new wake and no boot step.
- The hydrator follows the rule it was in breach of: a View decides when to act, never
  whether an effect is in place.
- The hydrator's `should_dispatch` loses its result-row arms.
- The read costs about 4.7 µs per service and about 6 ms for a pass over 1,000 services,
  against 5.84 s for the full-table read.
- A weight-only or health-only change is detected, with no precondition on slot ownership.

Negative and bounds:

- A `BACKEND_MAP` entry lost while its inner map survives reads as in place. Both maps are
  created and destroyed together, and only the appliance's own code writes either, so only
  an Overdrive bug produces it. The design does not defend it.
- The inner `SERVICE_MAP` array grows from 16,381 to 16,383 slots, and the Maglev table size
  and the inner capacity become two constants. The existing XDP dataplane tests and every
  fixture that builds an inner map for the production outer map must use the new capacity;
  the claim that XDP never selects the extra slots is a Tier-3 obligation (the spike ran no
  XDP program).
- `update_service` writes the fingerprint slots; its postcondition gains the fingerprint, and
  both adapters implement it. A reader that resolves the inner map id outside the apply-commit
  guard can see `ENOENT` or a torn state (unguarded reads tore in 63% of 2,734 reads in the
  spike), so the guard is part of the contract.
- A programmed service whose backend row is gone has no desired state, so the hydrator does
  not purge it. Teardown belongs to GH #211.
- ADR-0042 is edited only by this supersession pointer, which the architect adds on
  acceptance.

Links: [feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md)
(D-20, D-21, V-6), ADR-0042, ADR-0086, ADR-0296, GH #211.
