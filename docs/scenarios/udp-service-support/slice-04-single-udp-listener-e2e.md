# Slice 04 — Single UDP listener forward+reverse e2e (walking skeleton)

**Walking skeleton by journey coverage.** The thinnest slice exercising
every backbone activity; the operator-verifiable proof.

**Story:** US-04
**Priority:** P1
**KPI:** K1 (UDP reverse-path success)
**Job:** J-OPS-004
**Effort:** ~1 day
**Dependencies:** Slice 01 + Slice 02 (`ServiceFrontend` newtype + production fan-out)

## Goal the operator can verify

Ana runs `overdrive deploy dns-resolver.toml` (single udp/5353
listener), sends a real UDP datagram to the VIP, and a `tcpdump` capture
on the client veth shows the reply sourced from `10.96.0.10:5353` (the
VIP) — NOT the backend `10.244.0.20`.

## Learning hypothesis

If a single UDP listener service's full chain (CLI → control-plane →
reconciler → EbpfDataplane) is exercised end-to-end, then the reverse
path actually rewrites proto=17 responses to the VIP on real wire — the
core #163 fix is operator-verifiable, not just unit-asserted.

## IN scope

- Tier 3 e2e (real veth, behind `integration-tests`): submit a
  single-UDP-listener Service through the real chain.
- A real UDP datagram client→VIP; capture the reply.
- Assert the reply source == VIP.
- Distinguish "no response" (backend down) from "response with backend
  source" (the #163 defect).

## OUT scope

- Multi-listener TCP+UDP (US-05 / Slice 05).
- The lockstep gate (US-03 / Slice 03) — that is the unit/invariant
  guard; this is the wire-level proof.
- Forward-path-only assertions (forward already works pre-feature).

## Acceptance criteria

- [ ] Tier 3 e2e submits a single-UDP-listener Service through the real CLI→control-plane→reconciler→EbpfDataplane chain.
- [ ] A capture on the client side shows the reply sourced from the VIP, not the backend.
- [ ] Gated behind `integration-tests`; runs via `cargo xtask lima run`.
- [ ] The test distinguishes "no response" from "response with backend source".
- [ ] Deterministic across seeds (≥99/100).

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --features integration-tests -E 'test(udp_reverse_nat_e2e)'` green. Manual: submit `dns-resolver.toml`, `dig @<vip> -p 5353`, observe the reply source in a `tcpdump`.

## Pre-slice SPIKE

**Not required.** `reverse_nat_e2e` and `service_map_forward` (crates/overdrive-dataplane/tests/integration/)
are the exact Tier 3 shape; the `overdrive-testing` `ThreeIfaceTopology`
fixture supplies the veth/netns plumbing. UDP datagram send + capture is
a known pattern (swap the TCP connect for a UDP sendto).
