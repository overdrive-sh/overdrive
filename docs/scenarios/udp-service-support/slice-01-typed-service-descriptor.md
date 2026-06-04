# Slice 01 — `ServiceFrontend` newtype trait migration

**Load-bearing abstraction.** Every subsequent slice composes onto the
new `update_service(frontend, backends)` signature. Sequenced first AS a
behavior-preserving refactor (ship-the-abstraction-first, done honestly).

**Story:** US-01
**Priority:** P0
**KPI:** K5 (typed-surface adoption) — guardrail for K1
**Job:** J-PLAT-004
**Effort:** ~1 day
**Dependencies:** None (foundation)
**Decision basis:** DIVERGE Option 6 (`ServiceFrontend` newtype, 4.17). See `../recommendation.md` + `../diverge/taste-evaluation.md`.

## Goal the operator/author can verify

After this slice, `Dataplane::update_service` takes
`(frontend: ServiceFrontend, backends: Vec<Backend>)` where
`ServiceFrontend` carries `(ServiceVip, port, Proto)` — replacing the
shipped option-C signature `update_service(vip: Ipv4Addr, backends)`
(`dataplane.rs:101`). The existing TCP e2e (`service_map_forward` Tier 3)
stays green — proving the migration changed no PRODUCTION behavior. A
udp-listener service's frontend carries `proto: Udp`.

## Learning hypothesis

If we thread the protocol into a `ServiceFrontend` newtype on the trait
surface, then both SimDataplane and EbpfDataplane can derive REVERSE_NAT
keys from a single source — eliminating the structural cause of #163's
divergence — without touching PRODUCTION reverse-NAT behavior yet.

## IN scope

- Replace the shipped `Dataplane::update_service(vip: Ipv4Addr, backends)`
  (option C) with `update_service(frontend: ServiceFrontend, backends)`,
  where `ServiceFrontend` carries `(ServiceVip, port, Proto)`. **`backends`
  stays a SEPARATE positional arg** (not folded into the frontend).
- The frontend **re-absorbs `ServiceVip`** (locked-A's typed-VIP intent —
  shipped-C dropped it to raw `Ipv4Addr`). **`service_id`/`correlation`
  STAY on the `Action::DataplaneUpdateService` envelope** (`validate.rs:288`)
  — NOT folded into the frontend (action-routing, not a dataplane key).
- Migrate ALL call sites in the same PR (single-cut, C6).
- Both adapters consume the frontend; SimDataplane's `reverse_nat_keys_for`
  reads `frontend.proto` instead of the hard-coded `[Tcp, Udp]` (this is
  where Sim's over-broad fan-out is CORRECTED to the declared proto — Sim
  installs both today; after this it installs exactly what the service
  declares).
- **H2 — update any existing test/invariant asserting the two-proto Sim
  fan-out in this same single-cut PR** (a TCP-only service's Sim key set
  shrinks `{tcp,udp} → {tcp}`).

**Blast radius — 5 sites:** trait (`dataplane.rs`), EbpfDataplane,
SimDataplane, action-shim dispatch (`validate.rs`), ReverseNatLockstep
invariant. **Hydrator UNCHANGED** for US-01/US-04 (multi-listener fan-out
is US-05, a hydrator concern).

## OUT scope

- Production EbpfDataplane proto fan-out change (US-02 / Slice 02).
- Lockstep gate (US-03 / Slice 03).
- Any e2e (US-04/05).
- Hydrator per-listener emission (US-05) — hydrator UNCHANGED in this slice.
- Folding `service_id`/`correlation` into the frontend (stays on the Action by design).

## Acceptance criteria

- [ ] `update_service` takes `(frontend: ServiceFrontend, backends)` where `ServiceFrontend` carries `(ServiceVip, port, Proto)`; the shipped `(vip: Ipv4Addr, backends)` (option C) is gone. `backends` stays separate.
- [ ] The frontend re-absorbs `ServiceVip`; `service_id`/`correlation` remain on the `Action::DataplaneUpdateService` envelope (NOT in the frontend).
- [ ] Both adapters consume the frontend; existing TCP Tier 3 tests green.
- [ ] A udp-listener frontend carries `proto: Udp` end-to-end from intent.
- [ ] **C2 pass condition:** no call site reconstructs `(vip, port, proto)` from separate positional args (grep-verified); `service_id` travelling separately on the Action is explicitly permitted and is NOT a violation.
- [ ] SimDataplane `reverse_nat_keys_for` derives keys from `frontend.proto`, not the hard-coded `[Tcp, Udp]`; any existing two-proto-Sim-fan-out assertion is updated in the same PR (H2).
- [ ] Zero PRODUCTION proto behavior change in the Ebpf reverse-NAT path (REVERSE_NAT entries identical to pre-migration for the TCP case).

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --features integration-tests -E 'test(service_map_forward)'` stays green; the existing TCP path is unchanged.

## Pre-slice SPIKE

**Not required.** The trait shape is locked by DIVERGE (Option 6,
`ServiceFrontend` newtype family). The only remaining uncertainty — exact
newtype field names/derives, module location, whether `port` is
`NonZeroU16` — is a DESIGN P1 question (P1-Q2), not a build-time unknown.

## Forward pointer

DESIGN authors the ADR amendment to phase-2 architecture.md §5 Q-Sig
(**C → `ServiceFrontend`**, superseding the paper locked-A) and pins the
final newtype shape. Do NOT edit the ADR in this wave.
