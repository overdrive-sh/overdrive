# Slice 01 — Real-iface XDP attach (veth, not `lo`)

**Story**: US-01
**Backbone activity**: 1 (Attach to a real iface)
**Effort**: ½–1 day (Linux developer running in Lima VM via `cargo xtask lima run --`)
**Depends on**: phase-2-aya-rs-scaffolding (`EbpfDataplane`, `xdp_pass`, `PACKET_COUNTER`, `cargo xtask bpf-build`).

## Outcome

Phase 2.1's `xdp_pass` graduates from attaching against `lo` to
attaching against a real `veth0` interface inside the developer's
Lima VM (and inside CI's `ubuntu-latest` runner). The loader's iface-
resolution / native-mode-attach / structured-warning shape is
exercised end-to-end against a non-loopback driver. The existing
`PACKET_COUNTER` (`LruHashMap<u32, u64>`) increments on real frames
pushed through the veth pair, asserted via `bpftool map dump`. A
`tracing::warn!` with structured `iface` field fires on
generic-mode fallback; a typed `DataplaneError::IfaceNotFound`
surfaces on missing iface.

## Value hypothesis

*If* a non-`lo` ifindex changes loader semantics — attach behaviour,
native-mode availability, ifindex resolution errors — every later
slice that touches a real driver will surface two failure modes at
once (slice-specific + driver-specific). *Conversely*, if veth and
`lo` behave identically, the lift confirms ifindex semantics are
uniform across drivers and every later slice attributes failures
cleanly to its own scope.

## Disproves (named pre-commitment)

- **"`xdp_pass` against `lo` is sufficient evidence the loader
  works."** No — `lo` is XDP-generic-fallback territory; native attach
  to virtio-net / mlx5 is a different code path.
- **"ifindex semantics are uniform across drivers."** Expected to be
  true; the slice is the empirical confirmation.

## Scope (in)

- veth pair create/teardown helpers in `crates/overdrive-dataplane/tests/integration/veth_attach.rs` (gated `integration-tests`).
- Loader iface-name → ifindex resolution via `nix::net::if_::if_nametoindex` (or equivalent).
- `DataplaneError::IfaceNotFound { iface }` typed error variant.
- Native-mode attach default + structured `tracing::warn!` on generic-mode fallback.
- Re-target Phase 2.1's `xdp_pass` and `PACKET_COUNTER` to the veth pair.
- Sudo'd in-host execution per #152 — developer's Lima VM via `cargo xtask lima run --`; CI's `ubuntu-latest`.

## Scope (out)

- Any new BPF program (Slice 02+).
- Any new BPF map (Slice 02+).
- Any LB logic (Slice 02+).
- IPv6 frames (future Phase 2 slice).
- Conntrack (#154).
- Kernel-matrix execution (#152).

## Target KPI

- Tier 3 integration test passes: 100 frames sent through veth1 produce `PACKET_COUNTER == 100` on the kernel side.
- 100% of native-attach failures surface as a `tracing::warn!` (not silently fall through to generic mode).
- 0 silent fall-throughs to generic mode without a structured warning.
- Existing Phase 2.1 loopback-iface integration test continues to pass.

## Acceptance flavour

See US-01 scenarios. Focus: real veth attach + counter increment
asserted via `bpftool map dump`; structured-warning emission on
generic-mode fallback; typed error on missing iface.

## Failure modes to defend

- veth pair create fails (CAP_NET_ADMIN missing): integration test
  bails with a skip-message — `cargo xtask lima run --` defaults to
  root, so this should not happen in practice; the bail-out exists
  for defensive coverage.
- Native-mode attach fails: structured `tracing::warn!` fires;
  generic-mode attach is the fallback path. Test still passes.
- Iface name doesn't resolve: returns `DataplaneError::IfaceNotFound`;
  test asserts the typed error variant.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — veth setup helper + iface-resolution helper + structured-warning emission (3) |
| No hypothetical abstractions landing later | PASS — uses existing `EbpfDataplane`, `xdp_pass`, `PACKET_COUNTER` |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — real veth pair, real `ip link`, real `bpftool map dump` |
| Demonstrable in single session | PASS — `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --features integration-tests --test '*'` |
| Same-day dogfood moment | PASS — Linux developer runs the integration test in their Lima VM |
