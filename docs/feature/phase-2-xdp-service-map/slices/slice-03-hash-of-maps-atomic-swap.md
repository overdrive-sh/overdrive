# Slice 03 — HASH_OF_MAPS atomic per-service backend swap

**Story**: US-03
**Backbone activity**: 3 (Swap backends atomically)
**Effort**: 1 day
**Depends on**: Slice 02 (single-VIP forward path).

## Outcome

`SERVICE_MAP` restructures from `BPF_MAP_TYPE_HASH` to
`BPF_MAP_TYPE_HASH_OF_MAPS` per Cilium reference (research § 2.1, §
3.3). Outer map keyed by `ServiceId`; inner map per service is
`BPF_MAP_TYPE_ARRAY` of `BackendId`. New flat `BACKEND_MAP`
(`BPF_MAP_TYPE_HASH`, key `BackendId`, value `Backend { ip, port,
weight, flags }`) decouples backend address resolution from per-VIP
slot tables. XDP program lookup chain: hash 5-tuple → modulo
inner-array-size → BackendId → BACKEND_MAP → backend address →
DNAT-rewrite + `XDP_TX`. Random slot selection here is intentional
simplicity (Slice 04 replaces with Maglev).

`EbpfDataplane::update_service` performs the five-step atomic swap:
1. Insert / update relevant rows in `BACKEND_MAP`.
2. Allocate fresh inner map populated with the new backend slot
   table.
3. `bpf_map_update_elem(SERVICE_MAP, &service_id, &new_inner_fd)` —
   single 64-bit pointer swap (atomic on all supported architectures).
4. Garbage-collect orphaned BACKEND_MAP entries (no service
   references them).
5. Release the old inner map (kernel refcounts; auto-released once
   no XDP invocation references it).

`SimDataplane::update_service` mirrors atomic-swap semantics — one
mutex acquisition; the mutation is a single BTreeMap reassignment so
DST replay matches production.

Tier 3 integration test under `xdp-trafficgen` 100 kpps load asserts
ZERO packet drops across an atomic backend-set swap. New DST
invariant `BackendSetSwapAtomic` (always: every observation sees
either the pre-swap or post-swap backend set, never a mixed state).

## Value hypothesis

*If* Cilium's HASH_OF_MAPS atomic-swap shape doesn't actually deliver
zero drops at 100 kpps in our setup, the § 15 "weighted backends"
claim is performative. *Conversely*, if it does — and the DST
`BackendSetSwapAtomic` invariant proves it — Maglev (Slice 04)
inherits a known-good substrate for its weighted-canary commitment.

## Disproves (named pre-commitment)

- **"Atomic backend swap requires per-flow conntrack to be
  zero-drop."** No — swap atomicity is at the map level; conntrack
  pins individual flows but is not necessary for swap atomicity.
- **"Multiple-backend round-robin can wait until Maglev."** No — the
  two-level shape is the structural prerequisite for Maglev; random
  round-robin in this slice is intentional simplicity proving atomic-
  swap mechanics independent of algorithm choice.

## Scope (in)

- SERVICE_MAP restructure to `BPF_MAP_TYPE_HASH_OF_MAPS`; outer key `ServiceId`, inner `BPF_MAP_TYPE_ARRAY` of `BackendId` (default size 256).
- `BACKEND_MAP` declared as `BPF_MAP_TYPE_HASH` keyed by `BackendId`, value `Backend { ip, port, weight, flags }`.
- STRICT newtype `BackendId` in `overdrive-core`.
- `EbpfDataplane::update_service` 5-step atomic swap implementation.
- `SimDataplane::update_service` atomic-swap mirror (single mutex, BTreeMap reassignment).
- DST invariant `BackendSetSwapAtomic` in `overdrive-sim::invariants`.
- Tier 3 zero-drop integration test under `xdp-trafficgen` 100 kpps.
- BACKEND_MAP orphan-GC integration test.

## Scope (out)

- Maglev consistent hashing (Slice 04).
- REVERSE_NAT (Slice 05).
- Sanity prologue (Slice 06).
- Perf gates (Slice 07).
- Per-flow stickiness / conntrack (#154).
- Kernel matrix (#152).

## Target KPI

- 0 dropped packets across an atomic backend-set swap under 100 kpps `xdp-trafficgen` load.
- 100% `BackendSetSwapAtomic` invariant pass rate on every PR.
- 0 BACKEND_MAP orphans after the GC pass.

## Acceptance flavour

See US-03 scenarios. Focus: zero-drop assertion under sustained
load; orphan-count assertion; SimDataplane atomic-swap mirror under
DST.

## Failure modes to defend

- Inner-map allocation fails (ENOMEM, EPERM): `update_service`
  returns `DataplaneError::MapAllocFailed`; existing inner map is
  unchanged; subsequent traffic continues to forward against the
  old set.
- BACKEND_MAP orphan after backend removal: GC pass walks the map
  and removes IDs not referenced by any service.
- Verifier rejects the `HASH_OF_MAPS` lookup (NULL-check missing on
  inner-map pointer): the program won't load; the slice's Tier 2
  test catches this at PR time.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — HASH_OF_MAPS restructure + BACKEND_MAP + `BackendId` newtype + 5-step swap impl (4) |
| No hypothetical abstractions landing later | PASS — Cilium reference architecture is the proven precedent |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — real `xdp-trafficgen` 100 kpps + zero-drop assertion |
| Demonstrable in single session | PASS — Tier 3 test exits with PASS/FAIL on the developer's Lima VM |
| Same-day dogfood moment | PASS — Linux developer iterates the swap mechanics + invariant |
