# Slice 03 — A guest gets a definite, prompt failure when no upstream is reachable

> Feature: `guest-public-dns-resolution` (#382). Story: **US-GPD-3**. Job: J-OPS-005. Builds on slice 01.

## Goal

When the node cannot reach any upstream resolver, the guest gets a definite DNS
reply before its own 5 s timeout, mesh names keep answering, and lookups recover
when the upstream returns, with no restart.

## Learning hypothesis

The forwarder can bound its own wait so the guest never sees the silent 5 s
timeout that motivated the feature, and an upstream outage does not take mesh
naming down. Falsified if a guest lookup still takes the full timeout, or if a
mesh answer is delayed by the outage.

## Serve + deploy loop

`overdrive serve` -> `overdrive deploy herdr-box.toml` -> make the upstream
unreachable -> guest lookup of `crates.io` fails with a definite error in under
5 s; `payments.svc.overdrive.local` still resolves; restore the upstream -> next
lookup succeeds.

## Behaviour

- Unreachable upstream: definite reply, guest-observed time under 5000 ms (approved by the user 2026-11-25).
- Mesh names unaffected.
- Recovery needs no restart of the node, the responder or the guest.

## Carpaccio taste tests

- Closes a real loop through production: yes.
- Thinnest: one failure class (all upstreams unreachable) and its recovery.
- No test-only wiring: upstream unreachability is injected at the port boundary.

## Acceptance

AC-10 to AC-12 of US-GPD-3.

## Dependencies

Slice 01. DESIGN question 3 (reply code, timeout budget).
