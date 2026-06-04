# Slice 01 — Unconnected-UDP same-host round-trip (WALKING SKELETON)

**Feature:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Story:** US-01 · **Job:** J-OPS-004, J-PLAT-004 · **WALKING SKELETON**

## Goal (one sentence)

A same-host client that calls `sendto(VIP:port)` without `connect()`
reaches one local UDP backend AND receives a reply sourced from the VIP,
so `dig @<vip> example.com` against a single same-host DNS-shape service
returns a correct answer.

## IN scope

- `cgroup/sendmsg4` (`BPF_CGROUP_UDP4_SENDMSG`) program attached to
  `overdrive.slice`: forward rewrite `VIP → backend` via a
  `LOCAL_BACKEND_MAP` lookup keyed `(vip, vip_port, proto=UDP)`, proto
  read zero-translation from `bpf_sock_addr.protocol` (ADR-0053 Amd 2).
- `cgroup/recvmsg4` (`BPF_CGROUP_UDP4_RECVMSG`) program: reply-source
  rewrite `backend → VIP` via a new `REVERSE_LOCAL_MAP` lookup.
- `REVERSE_LOCAL_MAP` written **atomically** alongside the forward
  `LOCAL_BACKEND_MAP` entry by the same `register_local_backend` action
  (one logical write, two entries; NOT a conntrack table).
- ONE local UDP service, ONE backend, ONE listener. Production-shape
  client: a real `dig`/`sendto` round-trip (NOT a synthetic connected-UDP
  test).
- `user_port` low-16-NBO-in-u32 handling on both hooks (cast to u16, then
  `to_be`/`from_be` — `.claude/rules/development.md`).
- Tier-3 acceptance: real unconnected round-trip through `overdrive.slice`
  with `bpftool map dump` (both maps) + `tcpdump` (VIP-sourced reply)
  evidence. Fixture avoids the systemd-resolved UDP 5353 collision
  (`.claude/rules/debugging.md` § 11).

## OUT of scope (later slices / DESIGN)

- Sim≡kernel reply-path equivalence invariant → Slice 02.
- Error paths: REVERSE_LOCAL_MAP miss, kernel-floor preflight, asymmetry
  guard → Slice 03.
- Multiple backends / weighted selection (one backend only here).
- The ADR-0053 amendment text (architect's job in DESIGN).
- Any change to connect4 / the forward `LOCAL_BACKEND_MAP` shape / the
  hydrator classifier (UNCHANGED — pure addition).

## Learning hypothesis

**Confirms if it succeeds:** the unconnected `sendto` idiom is reachable
end-to-end with a VIP-sourced reply — `dig @<vip>` answers — proving the
cgroup sendmsg4+recvmsg4 pair over LOCAL_BACKEND_MAP + REVERSE_LOCAL_MAP
closes the #200 gap for the canonical client.
**Disproves if it fails:** if the round-trip hangs after the forward
rewrite lands, it disproves "forward rewrite is sufficient" — i.e. it
empirically reproduces the half-working-service trap and proves recvmsg4
is load-bearing (kernel commit `983695fa6765`). The internal milestone
boundary (forward-rewrite-only) is where this disproof is observable
during development; it is NOT a shipped boundary (shipping forward-only
is the J-OPS-004 operator-trust violation the DIVERGE rejected).

## Acceptance criteria

- [ ] `dig @<vip> example.com` against a single same-host DNS-shape UDP
      service returns a correct answer (the resolver uses unconnected
      `sendto`, never `connect()`).
- [ ] A Tier-3 `tcpdump` capture shows the reply's source address is the
      **VIP**, never the backend IP.
- [ ] `bpftool map dump LOCAL_BACKEND_MAP` shows the forward
      `(vip, vip_port, udp) → backend` entry; `bpftool map dump
      REVERSE_LOCAL_MAP` shows the reverse `backend → vip` entry; both
      present after a single `register_local_backend`.
- [ ] The forward and reverse entries are written by ONE action (atomic):
      no observable window where the forward entry exists without the
      reverse entry.

## Dependencies

- Shipped: ADR-0053 connect4 path, `LOCAL_BACKEND_MAP` forward shape,
  `register_local_backend` action (proto-carrying, Amd 3), hydrator
  classifier emitting `RegisterLocalBackend` for UDP local backends.
- DESIGN: ADR-0053 amendment defining `REVERSE_LOCAL_MAP` shape +
  atomic-write contract (architect; forward-pointed only from DISCUSS).

## Effort estimate / reference class

~1–1.5 days. Reference class: ADR-0053 connect4 sendmsg-family hook
landing (same program type, same map-lookup shape, same NBO care-point).
**Risk flag:** the full forward+reply+atomic-write+Tier-3 round-trip in
one slice may exceed 1 day. This is the irreducible walking skeleton —
forward-only is NOT a valid ship boundary (DIVERGE dissent verdict). If
DESIGN/DELIVER finds the Tier-3 fixture (real stub resolver in Lima,
avoiding the 5353 collision) is the long pole, that is a candidate
pre-slice SPIKE — surface to user, do not split into a forward-only ship.

## Pre-slice SPIKE (conditional)

IF the Tier-3 real-stub-resolver fixture (a same-host DNS responder in
the Lima VM, bound off the systemd-resolved-owned ports) is judged
high-uncertainty during DESIGN, run a ≤2h SPIKE to stand up the fixture
and confirm an unconnected `dig`/`sendto` round-trip is observable via
`tcpdump`/`bpftool` BEFORE the hook work. Learning: is the test harness
viable given there is no Tier-2 `BPF_PROG_TEST_RUN` for cgroup_sock_addr?
