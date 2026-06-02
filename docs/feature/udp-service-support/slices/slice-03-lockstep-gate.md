# Slice 03 — Reverse-NAT lockstep gate (both adapters + Tier 2 UDP triptych)

**The structural defense.** Converts the #163 silent asymmetry into a
loud PR-time failure.

**Story:** US-03
**Priority:** P0
**KPI:** K2 (divergence caught pre-merge) + K3 (kernel rewrite fires)
**Job:** J-PLAT-004
**Effort:** ~1.5 days
**Dependencies:** Slice 02 (production fan-out must exist to be matched)

## Goal the author can verify

After this slice, the lockstep is pinned three ways, meeting at the
shared `BackendKey` set:
- **Tier 1** (`cargo dst`, per-PR critical path): the `ReverseNatLockstep`
  invariant fails loudly if the SimDataplane drops a UDP REVERSE_NAT
  fan-out (Sim set-equality over `frontend.proto`).
- **Tier 3** (integration lane): real `EbpfDataplane.update_service(udp)`
  → `bpftool map dump REVERSE_NAT_MAP` shows `(ip,port,udp)` + VIP-source
  wire capture.
- **Tier 2** (`cargo xtask bpf-unit`): `xdp_reverse_nat_lookup` rewrites a
  proto=17 response source to the VIP.

## Learning hypothesis

If the lockstep is pinned by Tier-1 Sim set-equality AND a Tier-3 real-Ebpf
acceptance meeting at the same `BackendKey` set (a pure in-process
both-adapter retarget being infeasible), then a production-only divergence
like #163 becomes impossible to ship undetected.

## IN scope

- Retarget the `ReverseNatLockstep` invariant
  (crates/overdrive-sim/src/invariants/reverse_nat_lockstep.rs) to assert
  the **Tier-1 Sim set-equality** over `frontend.proto` (narrow the
  `[Tcp,Udp]` hardcode at `reverse_nat_lockstep.rs:158-165` to
  `frontend.proto` — landed in US-01).
- Add a **Tier-3 acceptance test** driving the real EbpfDataplane through
  `update_service(frontend_udp)` and asserting `bpftool map dump
  REVERSE_NAT_MAP` contains `(ip,port,udp)` + a VIP-sourced wire capture.
  (A pure in-process retarget against the real adapter is infeasible — it
  needs a kernel + bpffs; resolved at DIVERGE/H1.)
- Add a **Tier-2 `BPF_PROG_TEST_RUN` triptych** (PKTGEN/SETUP/CHECK)
  driving `xdp_reverse_nat_lookup` with a UDP response packet, asserting
  the source rewrite fires.

## OUT scope

- The behavior fix itself (US-02 / Slice 02).
- Full operator e2e (US-04 / Slice 04).
- Multi-listener (US-05).

## Acceptance criteria

- [ ] **Tier 1 (per-PR critical path):** the `ReverseNatLockstep` invariant asserts the SimDataplane installs exactly the declared-`frontend.proto` `BTreeSet<BackendKey>` for a udp service; a dropped Sim UDP fan-out fails it (proven by a negative test).
- [ ] **Tier 3 (integration lane):** real `EbpfDataplane.update_service(frontend_udp)` shows `(ip,port,udp)` in `bpftool map dump REVERSE_NAT_MAP` + a VIP-sourced wire capture; a dropped Ebpf UDP fan-out fails it.
- [ ] **Tier 2:** a `BPF_PROG_TEST_RUN` triptych asserts `xdp_reverse_nat_lookup` rewrites a proto=17 response source to the VIP.
- [ ] The Tier-1 Sim set-equality is on the per-PR critical path (not nightly-only); the Tier-3 Ebpf acceptance runs in the integration lane.

## Demoable check

`cargo xtask lima run -- cargo dst` (Tier 1) green + the Tier-3 acceptance
green; temporarily revert Slice 02 and confirm the Tier-3 Ebpf acceptance
goes RED; restore. `cargo xtask bpf-unit` shows the UDP triptych passing.

## Pre-slice SPIKE

**Not required — RESOLVED by DIVERGE (H1).** The original DISCUSS draft
recommended a ≤0.5-day spike ("can the Tier 1 DST invariant drive the
real EbpfDataplane in-process?"). The DIVERGE settled it: a pure
in-process retarget against the real adapter is **infeasible** (it loads
BPF programs and needs a kernel + bpffs). The gate is therefore Tier-1
Sim set-equality + Tier-3 Ebpf acceptance + Tier-2 triptych, meeting at
the shared `BackendKey` set. The only DESIGN detail forward-pointed is
the exact `ServiceFrontend` newtype shape the gate projects to
`BackendKey` (P1-Q2) — independent of the gate's set-equality logic.
