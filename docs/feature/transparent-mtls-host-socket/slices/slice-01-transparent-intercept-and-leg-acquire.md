# Slice 01 — transparent intercept (outbound rewrite + inbound TPROXY), agent accepts the workload leg, no recursion

> **Re-grounded to ADR-0069 (the agent-light L4 proxy).** Productionises the
> Slice-00 composed walking skeleton's intercept + leg-acquire step for BOTH
> directions. This replaces the old "sockops detect + pidfd_getfd the workload's
> own socket" framing — in the proxy model the workload's traffic is
> transparently REDIRECTED to an agent-owned leg, not adopted on the workload's
> own fd.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-01

## Goal (one sentence)

The workload's traffic is transparently brought under the agent's control before
it can reach the real peer un-encrypted — OUTBOUND via `cgroup_connect4` rewrite
to the agent's leg-F listener (the agent `accept()`s and drains the pre-arm
plaintext losslessly), INBOUND via nft-TPROXY redirect to the agent's
`IP_TRANSPARENT` leg-C listener (`getsockname()` recovers the original
destination) — and the agent's OWN leg-B dial is **not** re-intercepted (no
recursion), with the bypass **agent-private** so a workload cannot self-exempt.

## IN scope

- **Outbound intercept**: a `cgroup_connect4`-rewrite program redirects a
  host-socket workload's `connect()` to the agent's node-local leg-F listener
  (reusing the established `cgroup_connect4` program family); the agent
  `accept()`s the leg and `recv()`s the workload's pre-arm plaintext into a
  bounded userspace buffer **losslessly** (route the splice/relay by `local_port`
  only — `findings-userspace-relay.md` Unknown 1 found a `local_ip4` byte-order
  disagreement).
- **Inbound intercept**: an nft-TPROXY prerouting rule (`ip daddr <virt> tcp
  dport <port> tproxy to <agent>` + `ip rule fwmark` + `ip route local … table`
  triple) redirects a connection aimed at the server workload's logical address
  to the agent's `IP_TRANSPARENT` leg-C listener; the agent recovers the original
  destination via `getsockname()` (NOT `SO_ORIGINAL_DST` — under TPROXY the
  kernel keeps the intercepted socket's local addr as the orig dst).
- **The F5 intercept-recursion exemption (mechanism)**: the agent's own leg-B
  dial to the real peer is NOT re-intercepted by the workload `cgroup_connect4`
  program — via a narrowly-scoped, agent-private bypass (`SO_MARK` the program
  checks-and-skips, OR cgroup scoping so the agent's egress is outside the
  workload attach subtree). Reference the existing `cgroup_connect4_service`
  attach boundary (the program attaches to the **workload** cgroup subtree, not
  the agent's).
- `CAP_NET_ADMIN` for the `IP_TRANSPARENT` listener + nft-TPROXY setup — the
  host-side agent runs privileged; the workload is unprivileged and holds nothing.

## OUT scope

- The rustls handshake on leg B / leg C → Slice 02.
- The kTLS arm + agent-light forward splice + return agent-light splice + wire
  capture → Slice 03 (outbound) / Slice 04 (inbound). (D-MTLS-13: forward is
  agent-light, not the retired sockmap egress redirect.)
- The F5 negative tests (the agent's dial is provably not re-intercepted; the
  workload provably CANNOT self-exempt) → Slice 05 (the negatives; the MECHANISM
  is here).
- Resource limits / pump supervision → Slice 05.

## Learning hypothesis

- **Disproves if it fails**: "the platform can transparently intercept the
  workload's outbound connect AND inbound arrival to an agent-owned leg, drain the
  outbound pre-arm plaintext losslessly, recover the inbound original destination,
  and keep the agent's own dial from recursing." If the intercept loses pre-arm
  bytes, cannot recover the inbound orig-dst, or the agent's dial recurses, the
  proxy's foundation has a hole no downstream slice can close.
- **Confirms if it succeeds**: the intercept + leg-acquire foundation is sound;
  the handshake (S02), outbound enforce (S03), and inbound enforce (S04) build on
  a connection that is agent-controlled before any cleartext can escape.

## Acceptance criteria

- [ ] A `cgroup_connect4`-rewrite program redirects a host-socket workload's `connect()` to the agent's leg-F listener; the agent `accept()`s and `recv()`s the pre-arm plaintext **losslessly** (no dropped bytes), routing by `local_port` only. Anchor: `findings-userspace-relay.md` Unknown 1.
- [ ] An nft-TPROXY rule redirects a connection aimed at the server workload's logical address to the agent's `IP_TRANSPARENT` leg-C listener; `getsockname()` on the accepted leg-C socket recovers the original destination (NOT `SO_ORIGINAL_DST`). Anchor: `findings-inbound-intercept.md` §1 (`ORIG_DST=127.0.0.2:18443` recovered via `getsockname`).
- [ ] The agent's own leg-B dial to the real peer is NOT re-intercepted by the workload `cgroup_connect4` program (no recursion) — via an agent-private `SO_MARK`/cgroup-scoping bypass, referencing the `cgroup_connect4_service` attach boundary. Anchor: ADR-0069 § "intercept-recursion / agent-leg-B exemption".
- [ ] The `IP_TRANSPARENT` listener + nft-TPROXY setup succeed under `CAP_NET_ADMIN` (the agent is privileged); the workload is unprivileged and holds nothing. Anchor: `findings-inbound-intercept.md` § "Mechanics" #1.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the intercept + leg-acquire acceptance test (real kernel, not `--no-run`).

## Dependencies

- Slice 00 (the composed WS validated the intercept + leg-acquire compose).
- The `cgroup_connect4` program family (already in `overdrive-bpf`) + `nft_tproxy`
  / `xt_TPROXY` on the 6.18 kernel (ADR-0068).
- The bpffs-pin discipline (`pinning = ByName`, already used for HASH_OF_MAPS) for
  the intercept program/maps where needed.
- The `MtlsEnforcement` port's intercept surface (ADR-0069 / feature-delta DESIGN).

## Effort estimate

~1 day (≤6h). Reference class: the `cgroup_connect4` rewrite + bpffs pin mirror
the existing dataplane discipline; the TPROXY triple + `getsockname` orig-dst are
proven in `findings-inbound-intercept.md`; the agent-private bypass mirrors the
`cgroup_connect4_service` attach boundary.

## Pre-slice SPIKE

Not needed — Slice 00 (the composed WS) and the 6 committed spikes validated the
intercept + leg-acquire mechanism on the real kernel (outbound rewrite +
loophless `accept`/`recv` in `findings-userspace-relay.md`; inbound TPROXY +
orig-dst in `findings-inbound-intercept.md` §1). This slice productionises them.

## Taste-test note

A thin vertical cut: ships the bidirectional transparent intercept + leg-acquire +
the intercept-exemption mechanism. Production-data observable (the workload's
connect is rewritten / its inbound arrival is TPROXY-redirected on a real kernel;
the pre-arm bytes arrive losslessly; the orig-dst is recovered — Tier-3).
Disproves a real assumption (the workload's traffic is agent-controlled before it
can escape, both directions, and the agent's own dial does not recurse). Carries
one value story (US-MTLS-01); the observable is the agent-controlled, losslessly-
captured connection — the necessary first step of the enforcement the user
verifies end-to-end by S03/S04. Not infra-only.
