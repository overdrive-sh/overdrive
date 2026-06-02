# Slice 02 — Production EbpfDataplane REVERSE_NAT proto fan-out

**The actual bug fix.** The line that was broken in #163.

**Story:** US-02
**Priority:** P0
**KPI:** K1 (UDP reverse-path success 0%→100%)
**Job:** J-OPS-004 + J-PLAT-004
**Effort:** ~1 day
**Dependencies:** Slice 01 (the `ServiceFrontend` newtype supplies the proto)

## Goal the operator can verify

After this slice, `overdrive job submit dns-resolver.toml` (udp/5353)
installs a REVERSE_NAT_MAP entry `(backend_ip, 5353, udp) → vip`. A
`bpftool map dump` of REVERSE_NAT_MAP shows the udp-keyed entry. The
Sim-vs-Ebpf REVERSE_NAT key-set diff for a udp service is empty.

## Learning hypothesis

If the production Step 4b installs REVERSE_NAT entries per
`frontend.proto` (mirroring `reverse_nat_keys_for`'s shape), then a
UDP backend's response will find its entry and be source-rewritten to
the VIP — closing #163.

## IN scope

- `EbpfDataplane::update_service` Step 4b: install REVERSE_NAT_MAP
  entries for `frontend.proto` (per-backend per-proto fan-out).
- Mirror the Sim adapter's cross-service purge logic
  (`difference`/`live_keys`) so empty-backend updates remove the udp
  entry without stale lingering.

## OUT scope

- Lockstep gate (US-03 / Slice 03) — this slice fixes the behavior; the
  next slice guards it.
- e2e (US-04 / Slice 04) — the wire-level proof.
- Hydrator multi-listener (US-05).
- TCP behavior is unchanged (no new tcp entries, no removed ones).

## Acceptance criteria

- [ ] Step 4b installs REVERSE_NAT entries for `frontend.proto` (per-backend per-proto).
- [ ] `bpftool map dump` of REVERSE_NAT_MAP shows the `(ip,port,udp)` entry for a udp service.
- [ ] Sim-vs-Ebpf REVERSE_NAT key-set diff for a udp service is empty.
- [ ] Empty-backend update purges the udp entry (no stale entry).
- [ ] TCP services' REVERSE_NAT entries are byte-identical to pre-slice.

## Demoable check

Stand up the dataplane in Lima, submit `dns-resolver.toml`, run
`bpftool map dump name REVERSE_NAT_MAP` and observe the udp-keyed entry.

## Pre-slice SPIKE

**Not required.** `reverse_nat_keys_for` (crates/overdrive-sim/src/adapters/dataplane.rs:266)
is the exact reference shape; the production Step 4b sites are named in
the issue (lib.rs ~697/1336/1455/1533). The work is a known transcription
of a known shape, driven by `frontend.proto`.
