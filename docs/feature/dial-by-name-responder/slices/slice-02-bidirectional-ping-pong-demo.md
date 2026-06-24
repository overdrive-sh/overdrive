# Slice 02 — Bidirectional ping-pong demo

> Reviewed brief (DISCUSS, 2026-06-24; gated to Slice 00). Feature: `dial-by-name-responder` (#243). Story: **US-DBN-3**.
> Job: J-MESH-001. The operator-runnable proof. Builds on Slice 01.

## Goal (one line)

Two services ping-pong by name: A calls `b.svc.overdrive.local`, B calls
`a.svc.overdrive.local`; each call increments a counter + stamps a fresh date on a
~10s cadence; each hop resolved through the responder then intercepted + mTLS'd —
runnable with two `overdrive deploy` commands.

## Learning hypothesis

The walking-skeleton path (Slice 01) generalises to **bidirectional** resolution
between two real deployed workloads, and the behavior is **operator-observable**
(counters/dates advance) — not just assertable in a Tier-3 test.

## serve+deploy loop

`overdrive serve` + `overdrive deploy examples/dial-by-name-responder/a.toml` +
`overdrive deploy examples/dial-by-name-responder/b.toml` → observable advancing
ping-pong.

## Behavior

- Two specs `examples/dial-by-name-responder/{a,b}.toml` — `[service]`/`[exec]`/`[resources]`/`[[listener]]` (the `overdrive deploy` schema). Introduces the `examples/<feature>/` subdir convention.
- A small **ping-pong program**: resolve peer by name → call on a ~10s loop; on inbound call, increment a counter + set a fresh date + reply.
- `command` MUST point at a **real on-disk binary** in the deploy env (no phantom paths). Ports avoid dev-VM collisions (NOT 5353 — `systemd-resolved` owns it, per `dns-resolver.toml`).
- **Program shape DECIDED (user, 2026-06-24): a tiny Rust bin staged into the VM** (the `coinflip-helper` precedent — clean HTTP/TCP + counter/date), built and staged at a real on-disk `command` path before the demo runs.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — the demo IS two `overdrive deploy`s against `serve`; it cannot run until the responder answers, so it's scoped inside this feature.
- **Thinnest for its outcome?** It's the largest slice but still one deliverable (the operator-runnable proof); no sub-split buys independent value.
- **No `#[test]`-only composition?** The demo runs against the production binary; graduates to an EDD expectation, not a `#[test]`.

## Acceptance (= US-DBN-3 ACs)

- [ ] `a.toml` + `b.toml` exist with the accepted schema; `command` → a real on-disk binary.
- [ ] A calls `b.svc.overdrive.local`, B calls `a.svc.overdrive.local`, each via the in-agent responder.
- [ ] Each call increments a counter + stamps a fresh date; cadence ≈ 10s.
- [ ] Each hop intercepted + mTLS'd (tcpdump/`ss -tie` on the peer leg).
- [ ] Driven by two `overdrive deploy`s against `overdrive serve`.
- [ ] Graduated to `verification/expectations/` (proposed `E05-dial-by-name-ping-pong-mtls`), anchored to the US-DBN-3 scenario + K-DBN-3; honest `pending` if the full-system EDD harness (#227/#75) hasn't landed (mirror E04).

## Dependencies

- **Slice 01** (the A→B skeleton).
- The ping-pong program — a tiny Rust bin (decided 2026-06-24) — built + staged at its `command` path in the deploy env.
- Real on-disk `command` target staged in the deploy env.
