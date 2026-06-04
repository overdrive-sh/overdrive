# Slice 05 — ServiceMapHydrator per-listener fan-out (multi-listener TCP+UDP e2e)

**The richest operator outcome.** Dual-protocol services on one VIP.

**Story:** US-05
**Priority:** P2
**KPI:** K4 (multi-listener both-path success)
**Job:** J-OPS-004
**Effort:** ~1.5 days
**Dependencies:** Slice 01 + Slice 02 + Slice 04

## Goal the operator can verify

Ana runs `overdrive deploy edge.toml` (tcp/8080 + udp/8081); the
accepted line shows both listeners; the ServiceMapHydrator emits one
`update_service` per listener with the spec-declared proto; and BOTH the
TCP forward+reverse path AND the UDP forward+reverse path work on real
wire (two Tier 3 captures, both showing VIP source).

## Learning hypothesis

If the ServiceMapHydrator reads `Vec<Listener>` and emits one
`update_service` per listener (rather than collapsing to one call), then
a service speaking both TCP and UDP on one VIP has both protocols' paths
installed — closing the multi-protocol gap.

## IN scope

- `ServiceMapHydrator` (ADR-0042): read `Vec<Listener>` from the intent
  Service aggregate, emit one `update_service` per listener with the
  declared proto.
- Tier 3 e2e: TCP 8080 + UDP 8081 Service through the real chain; both
  paths captured VIP-sourced.
- Re-submit convergence: adding a listener installs the new path without
  breaking existing ones.

## OUT scope

- New protocols beyond tcp/udp (SCTP etc. — GH #155 / future).
- Per-listener health probes (service-health-check-probes already owns
  probe lifecycle; this slice is wire-path only).
- VIP allocation policy changes beyond what phase-2 SERVICE_MAP
  `(VIP,port)` keying already implies (DESIGN P2-Q4).

## Acceptance criteria

- [ ] `ServiceMapHydrator` emits one `update_service` per listener with the spec-declared proto.
- [ ] Tier 3 e2e: TCP 8080 + UDP 8081 Service has both forward+reverse paths working through the real chain.
- [ ] Both protocols' replies captured with the VIP as source.
- [ ] Re-submitting with an added listener converges without breaking existing paths.

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests -E 'test(multi_listener_tcp_udp_e2e)'` green. Manual: submit `edge.toml`, exercise both `nc -u <vip> 8081` and `curl <vip>:8080`, observe both replies VIP-sourced.

## Pre-slice SPIKE

**Not required for the hydrator emission** (reading `Vec<Listener>` and
looping is straightforward). **One DESIGN question to resolve first
(P2-Q4/P2-Q5):** does each listener get its own `(VIP, port)` SERVICE_MAP
key (the natural shape given phase-2 architecture.md §5 Drift-3)? The
locked decision says multi-listener is a HYDRATOR fan-out concern (one
`update_service(frontend, backends)` per listener), NOT a trait-surface
one — re-open the "fold `Vec<Listener>` into the frontend" question only
if multi-listener becomes a trait-surface concern (the Option-2 dissent
condition). This is a DESIGN decision, not a build-time spike — flagged
in feature-delta.md hand-off questions.
