# DESIGN Decisions — phase-2-xdp-service-map

**Wave**: DESIGN (solution-architect)
**Owner**: Morgan
**Date**: 2026-05-05
**Status**: COMPLETE — handoff-ready for DISTILL (acceptance-designer)
**Mode**: propose (user ratified `lgtm` against `proposal-draft.md`)

---

## Key Decisions

One line each, citing the source artifact / file the decision lands in.

- **D1 — Three-map split + HASH_OF_MAPS atomic swap.** SERVICE_MAP
  (outer key `(ServiceVip, u16 port)`), BACKEND_MAP, MAGLEV_MAP per
  Cilium / Katran reference shape. Source: ADR-0040; research § 2.1, §
  2.2, § 6.2; `architecture.md` § 10.
- **D2 — Trait surface signature.** `Dataplane::update_service(service_id:
  ServiceId, vip: ServiceVip, backends: Vec<Backend>)` — three explicit
  args; Q-Sig=A. Source: ADR-0040; `architecture.md` § 5.
- **D3 — Checksum helper choice.** `bpf_l3_csum_replace` /
  `bpf_l4_csum_replace` (kernel helpers); Q1=A. Source: ADR-0040;
  research § 4.1.
- **D4 — Reverse-NAT egress hook.** TC egress (`tc_reverse_nat`); Q2=A.
  Kernel-floor compatibility (5.10 LTS). Source: ADR-0041;
  `architecture.md` § 5.
- **D5 — Sanity-prologue strategy.** Shared `#[inline(always)]` Rust
  helper in `overdrive-bpf::shared::sanity`; Q3=C. Source: ADR-0040;
  research § 8.2.
- **D6 — Maglev parameters.** M=16_381 default; M ≥ 100·N rule;
  weighted permutation via Eisenbud + multiplicity expansion in
  `BTreeMap` order; ship weighted directly (no vanilla-then-weighted
  progression per DISCUSS Decision 8). Q5=A inner-map size 256;
  Q6=A operator surface deferred. Source: ADR-0041; research § 5.2,
  § 5.3.
- **D7 — Endianness lockstep.** Wire = network-order; map storage =
  host-order; conversion site = `crates/overdrive-bpf/src/shared/sanity.rs`
  (`reverse_key_from_packet` / `original_dest_to_wire`). Tier 2
  roundtrip + userspace proptest. Source: ADR-0041; `architecture.md`
  § 11.
- **D8 — `DropClass` slot count locked at 6.** `MalformedHeader=0,
  UnknownVip=1, NoHealthyBackend=2, SanityPrologue=3, ReverseNatMiss=4,
  OversizePacket=5`; Q7=B. Source: ADR-0040; `architecture.md` § 6.
- **D9 — `ServiceMapHydrator` reconciler is the J-PLAT-004 closer.** Sync
  `reconcile`, runtime-owned hydration per ADR-0035/0036, View persists
  `RetryMemory` inputs (not deadlines), per-target keying on
  `ServiceId`, ESR pair `HydratorEventuallyConverges` /
  `HydratorIdempotentSteadyState`. Source: ADR-0042; `architecture.md`
  § 8.
- **D10 — `Action::DataplaneUpdateService` + `service_hydration_results`
  observation table.** New typed Action variant (Q-Action=A); new
  observation table for `actual` projection (Drift 2); new
  `ServiceHydrationDispatchError` shim error; failure surface is
  observation, NOT `TerminalCondition` (preserves ADR-0037
  invariant). Source: ADR-0042; `architecture.md` § 7, § 12.

---

## Architecture Summary

- **Pattern**: Hexagonal (ports & adapters) — inherited from
  `brief.md` § 1; Phase 2.2 fills the empty body of one port
  (`Dataplane::update_service`) and adds one reconciler against it.
- **Paradigm**: OOP (Rust trait-based) — inherited from `brief.md` § 2.
- **Key components**:
  - `overdrive-bpf` (kernel side): `xdp_service_map`, `tc_reverse_nat`
    + 5 BPF maps + shared sanity / endianness helpers.
  - `overdrive-dataplane` (userspace loader): `EbpfDataplane` impl
    of `Dataplane`, typed map handles, `swap.rs` HASH_OF_MAPS
    primitive, `maglev/{permutation,table}` userspace generators.
  - `overdrive-control-plane::reconcilers/service_map_hydrator/`:
    sync-`reconcile` reconciler, View persisting RetryMemory inputs,
    runtime-owned hydration of `desired` (`service_backends`) +
    `actual` (`service_hydration_results`).
  - `overdrive-control-plane::action_shim::service_hydration`:
    dispatches `Action::DataplaneUpdateService` to
    `Dataplane::update_service`, writes outcome row to
    `service_hydration_results`.
  - `overdrive-core` extensions: 5 newtypes (`ServiceVip`,
    `ServiceId`, `BackendId`, `MaglevTableSize`, `DropClass`); one
    new `Action` variant; `AnyReconciler`/`AnyState` extended.

---

## Reuse Analysis

(HARD GATE — same 20-row table from `architecture.md` § 4. 15
EXTEND/REUSE; 5 CREATE NEW with documented "no existing
alternative" justification.)

| # | Component / surface | Disposition | Rationale |
|---|---|---|---|
| 1 | `Dataplane` trait (`overdrive-core::traits::dataplane`) | EXTEND | Add three method args to `update_service(service_id, vip, backends)`; no new trait. |
| 2 | `EbpfDataplane` (`overdrive-dataplane::ebpf_dataplane`) | EXTEND | Phase 2.1 stub bodies become real implementations; struct shape unchanged. |
| 3 | `SimDataplane` (`overdrive-sim::adapters::dataplane`) | EXTEND | Mirror new method signature; in-memory `BTreeMap` book-keeping. |
| 4 | `Reconciler` trait (`overdrive-core::reconciler`) | EXTEND (new impl, no trait change) | New `ServiceMapHydrator` impl; ADR-0035 trait shape unchanged. |
| 5 | `AnyReconciler` enum | EXTEND | Add `ServiceMapHydrator` variant; runtime hydration `match` arm extended. |
| 6 | `AnyState` enum | EXTEND | Add `ServiceMapHydrator(ServiceMapHydratorState)` variant per ADR-0021/0036. |
| 7 | `Action` enum | EXTEND | Add one new variant `Action::DataplaneUpdateService`. |
| 8 | `ReconcilerName` | EXTEND | Add `service-map-hydrator` const name; no type change. |
| 9 | `EvaluationBroker` | REUSE | Storm-proof keying on `(name, target)` works as-is. |
| 10 | `action_shim::dispatch` match | EXTEND | Add `DataplaneUpdateService` arm; existing match exhaustiveness gates. |
| 11 | Service-backends ObservationStore row shape | REUSE | Already in `traits/observation_store.rs`; no schema change. |
| 12 | `service_hydration_results` ObservationStore table | CREATE NEW (additive-only migration) | Required for `actual` projection to observe what *is*, not what was *predicted* (Drift 2). No alternative. |
| 13 | `RedbViewStore` | REUSE | ADR-0035 `bulk_load` / `write_through` for any typed `View`. |
| 14 | `TickContext` | REUSE | Wall-clock injection works as-is. |
| 15 | `CorrelationKey` | REUSE | The `(reconciler, target, fingerprint)` shape exists. |
| 16 | `ServiceVip` newtype | CREATE NEW | No existing IPv4/IPv6-VIP newtype; required for typed `(VIP, port) → ServiceId` SERVICE_MAP key. No alternative. |
| 17 | `ServiceId` newtype | CREATE NEW | No existing service-identity newtype; required for typed Action variant + per-target keying. No alternative. |
| 18 | `BackendId` / `MaglevTableSize` / `DropClass` newtypes | CREATE NEW | No existing backend-identity, table-size, or drop-class type; required for STRICT-newtype discipline. |
| 19 | `aya::Bpf` loader (Phase 2.1 substrate) | REUSE | `overdrive-dataplane::loader` already loads ELF. |
| 20 | `xtask bpf-build / bpf-unit / integration-test vm` | REUSE (Slice 07 fills `verifier-regress` + `xdp-perf` stubs from #23) | No new subcommand. |

**Summary**: 15 EXTEND/REUSE; 5 CREATE NEW (1 observation table + 4
newtypes plus the unavoidable `ServiceVip`); 0 unjustified CREATE
NEW.

---

## Technology Stack

OSS-only, all already in workspace `Cargo.toml`.

| Dep | Version | License | Role | Why chosen |
|---|---|---|---|---|
| `aya` / `aya-ebpf` | 0.13.x | MIT-or-Apache-2 | Userspace BPF loader + kernel-side primitives | Pure Rust, no `protoc`, ADR-0038 substrate. |
| `redb` | 2.x | MIT-or-Apache-2 | View persistence via `RedbViewStore` (ADR-0035) | Pure Rust embedded ACID KV; fsync per write_through. |
| `libsql` (existing dep) | — | MIT | Reserved for incident memory / DuckLake catalog (Phase 3+); NOT used by hydrator | Hydrator persists via redb per ADR-0035. |
| `rkyv` | 0.8 | MIT | `BackendSetFingerprint` content hash | Archived bytes are canonical → deterministic hashing per development.md. |
| `serde` / `ciborium` | 1.x / 0.2 | MIT-or-Apache-2 | View CBOR encoding (ADR-0035) | Author derives `Serialize + Deserialize`; runtime owns persistence. |
| `proptest` | 1.x | MIT-or-Apache-2 | Newtype roundtrip + permutation determinism + endianness roundtrip | Same discipline as Phase 1 newtypes. |
| `turmoil` | 0.6 (pinned) | MIT-or-Apache-2 | DST harness exercises hydrator ESR | Inherited from Phase 1. |
| `thiserror` | 2.x | MIT-or-Apache-2 | `ServiceHydrationDispatchError` + `DataplaneError` extensions | `#[from]` pass-through embedding. |

No new top-level deps. No proprietary deps.

---

## Constraints Established

The 10 DISCUSS constraints (`architecture.md` § 2) are preserved
verbatim into DESIGN; the only DESIGN-specific addition is:

- **Hydrator-side determinism is structural.** `BTreeMap` —
  not `HashMap` — across `desired`, `actual`, View, and Maglev-input
  iteration. Enforced by dst-lint on the `core`-class crates;
  enforced by review on the `adapter-host`-class crates. The
  `ReconcilerIsPure` DST invariant + the new `HydratorEventuallyConverges`
  / `HydratorIdempotentSteadyState` invariants close the loop.

---

## Upstream Changes

**None.** This feature is additive against the Phase 2.1 substrate.

- Three new ADRs (ADR-0040 / 0041 / 0042) added to the index; no
  existing ADR is superseded or amended.
- `brief.md` extended with a new Phase 2.2 sub-section (§ 44 onward)
  alongside Phase 2.1. Status row updated in place.
- `c4-diagrams.md` extended with an L3 dataplane component diagram;
  L1 / L2 from Phase 2.1 unchanged.
- `proposal-draft.md` retained in this directory for decision-
  provenance traceability (see `architecture.md` § 16). No
  consolidation / deletion this wave.
- DISCUSS Slice 04 budget (1.5d) is acknowledged informationally;
  no edits to DISCUSS artifacts.
- `.nwave/des-config.json` modifications, if any, are committed
  in the same PR as this DESIGN landing per the global rule (the
  `always-include path` discipline in `development.md`).

No edits to `whitepaper.md`, `commercial.md`, `.claude/rules/*`,
or any other SSOT file outside `docs/product/architecture/` and
this feature directory.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial DESIGN wave decisions for `phase-2-xdp-service-map`. Mode = propose; user ratified the proposal-draft with `lgtm`. Seven open-question decisions + three drifts locked. Three ADRs (0040 / 0041 / 0042) authored. `brief.md` § 44+ + `c4-diagrams.md` Phase 2.2 component diagram added. — Morgan. |
