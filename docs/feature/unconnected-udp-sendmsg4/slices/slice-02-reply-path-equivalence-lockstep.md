# Slice 02 — Sim≡kernel reply-path equivalence lockstep

**Feature:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Story:** US-02 · **Job:** J-PLAT-004 (primary), J-OPS-004

## Goal (one sentence)

Pin the unconnected-UDP forward rewrite AND the reply-path source identity
as a Sim≡kernel equivalence invariant, so a forward-only or asymmetric
regression fails LOUDLY at PR time and the silent-asymmetry class (O3,
#163-class) cannot reach production undetected.

## IN scope

- Tier-1 `SimDataplane` invariant asserting BOTH: (1) the unconnected-UDP
  forward rewrite `(vip, vip_port, udp) → backend` matches the declared
  frontend, and (2) the reply-path source identity — Sim rewrites the
  reply source to the **VIP**, never the backend. This is the J-PLAT-004
  equivalence twin extended to the reply leg.
- The Sim adapter's `REVERSE_LOCAL_MAP`-equivalent reply rewrite derives
  from the SAME forward `register_local_backend` registration as the
  kernel path (single source of truth for the reverse mapping).
- Tier-3 acceptance pinning the real kernel path against the same
  observable contract (reply source = VIP via `tcpdump`; both maps via
  `bpftool map dump`), meeting Tier-1 at the shared backend identity.

## OUT of scope (later slices / DESIGN)

- Error-path handling (REVERSE_LOCAL_MAP miss, kernel-floor preflight) →
  Slice 03.
- The round-trip itself (delivered by Slice 01).
- In-process both-adapter retarget (infeasible — the real adapter needs a
  kernel + bpffs; this is why the pin is two-pronged, mirroring
  submit-a-udp-service.yaml step 4).

## Learning hypothesis

**Confirms if it succeeds:** the reply-path source identity is provably
equivalent across Sim and the real kernel — the asymmetry that produced
#163 is structurally impossible to reintroduce on the sendmsg/recvmsg
path.
**Disproves if it fails:** if a deliberately-introduced forward-only Sim
mutation (drop the reply rewrite) does NOT fail the invariant, it
disproves "the equivalence surface actually covers the reply leg" — the
invariant is decorative and must be strengthened. (This is the mutation
the slice exists to kill; cf. the §18 ESR discipline.)

## Acceptance criteria

- [ ] A Tier-1 invariant asserts the SimDataplane reply source for an
      unconnected-UDP service is the VIP (not the backend) for the
      declared frontend; the invariant is on the per-PR critical path.
- [ ] Removing the reply-path rewrite from the Sim adapter FAILS the
      Tier-1 invariant (verified by a RED scaffold or a mutation-test
      target, not by inspection).
- [ ] Removing the reply-path rewrite from the kernel path FAILS the
      Tier-3 acceptance (the `tcpdump` reply-source assertion turns red).
- [ ] The reverse mapping in BOTH adapters is derived from the same
      forward registration — no independent reply-path source of truth.

## Dependencies

- Slice 01 (the round-trip + both maps + the hooks must exist to pin).
- DESIGN: the ADR-0053 amendment's atomic-write contract (so the
  equivalence invariant has a single forward registration to assert against).

## Effort estimate / reference class

~1 day. Reference class: the `ReverseNatLockstep` Tier-1 invariant +
Tier-3 acceptance pair shipped for udp-service-support
(submit-a-udp-service.yaml step 4) — same two-pronged shape, same
"meet at the shared backend key set" structure, retargeted to the
cgroup same-host reply path.

## Elevator-pitch note (value, not infra)

This slice is NOT `@infrastructure`: its operator-invocable proof is the
CI gate verdict plus `bpftool map dump REVERSE_LOCAL_MAP`, and the
decision it enables is concrete — Ana (and the dataplane author) can
TRUST that the VIP-sourced reply guarantee will not silently regress,
which is the difference between "it works today" and "it is safe to
depend on." See US-02 in feature-delta.md.
