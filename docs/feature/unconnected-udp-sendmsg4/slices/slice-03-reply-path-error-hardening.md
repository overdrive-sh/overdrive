# Slice 03 — Reply-path error hardening (no backend-IP leak, clean failures)

**Feature:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Story:** US-03 · **Job:** J-OPS-004 (primary), J-PLAT-004

## Goal (one sentence)

When the reply-path mapping is missing, asymmetric, or the kernel is
below floor, the failure is clean and observable — never a reply that
leaks the backend IP and never a silent hang the operator cannot
diagnose — so Ana can tell a misconfiguration from a platform bug with
`dig`, `tcpdump`, and `bpftool`.

## IN scope

- `REVERSE_LOCAL_MAP` miss handling on `recvmsg4`: a reply with no reverse
  entry MUST NOT leak the backend IP as the source. Define the observable
  behaviour (drop with a counted `DropClass`-style reason, OR pass
  unchanged ONLY if that cannot leak — DESIGN decides; the AC pins "no
  backend-IP-sourced reply ever reaches a client").
- Kernel-floor preflight: recvmsg4 requires ≥ 4.20, sendmsg4 ≥ 4.18. State
  the floor check; on the supported 5.10+ matrix this is informational,
  but a host below floor must refuse-or-warn observably, not deliver a
  half-working service (the J-OPS-004 honesty contract — cf.
  ADR-0028/ADR-0034 cgroup-preflight refusal precedent).
- Tier-3 fixture-collision guard codified: the test fixture binds off the
  systemd-resolved-owned UDP 5353 (`.claude/rules/debugging.md` § 11) and
  asserts a clean `bind` rather than papering over `EADDRINUSE`.

## OUT of scope (later / DESIGN)

- Multi-backend selection / health-driven backend removal (rides the
  existing hydrator + Backend.healthy path; not a reply-path concern).
- The happy-path round-trip (Slice 01) and the equivalence pin (Slice 02).
- Conntrack / per-flow state (explicitly rejected — UDP is stateless, D7).

## Learning hypothesis

**Confirms if it succeeds:** the half-working-service failure mode has a
single, observable, non-leaking shape — a missing reverse entry produces
a diagnosable failure, not a backend-IP leak or a silent timeout.
**Disproves if it fails:** if a forced REVERSE_LOCAL_MAP miss produces a
backend-IP-sourced reply (the client discards it and the query times out
with no signal), it disproves "the reply path fails safe" and re-opens
the silent-asymmetry hazard at the error boundary rather than the happy
path.

## Acceptance criteria

- [ ] With the forward entry present but the reverse entry forced absent,
      NO reply reaches the client sourced from the backend IP (verified by
      Tier-3 `tcpdump`); the miss is observable (counter / log), not silent.
- [ ] A host below the recvmsg4 kernel floor (≥ 4.20) refuses or warns
      observably at attach/preflight — it does NOT silently deliver a
      forward-only half-working service.
- [ ] The Tier-3 fixture binds its stub resolver off UDP 5353 and asserts
      a clean `bind`; a collision fails the test loudly rather than being
      swallowed.

## Dependencies

- Slice 01 (the hooks + maps) and Slice 02 (the equivalence surface to
  assert the error-path source identity against).
- DESIGN: the miss-handling decision (drop-with-reason vs pass) in the
  ADR-0053 amendment.

## Effort estimate / reference class

~0.5–1 day. Reference class: the cgroup-preflight refusal path
(ADR-0028 / ADR-0034) for the floor check, and the `DropClass`-counted
drop discipline (`crates/overdrive-core/src/dataplane/drop_class.rs`) for
the no-leak miss handling.

## Elevator-pitch note (value, not infra)

NOT `@infrastructure`: the operator-invocable proof is `dig @<vip>`
against a deliberately-misconfigured service failing CLEANLY (with a
diagnosable signal) plus `bpftool map dump` showing the reverse-entry
absence — and the decision it enables is "this is MY misconfiguration,
not a platform bug," which is the operator-trust outcome J-OPS-004 names.
See US-03 in feature-delta.md.
