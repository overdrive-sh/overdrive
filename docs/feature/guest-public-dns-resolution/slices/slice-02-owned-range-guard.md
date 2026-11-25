# Slice 02 — Forwarded answers never point a guest into Overdrive-owned address space

> Feature: `guest-public-dns-resolution` (#382). Story: **US-GPD-2**. Job: J-OPS-005. Builds on slice 01.

## Goal

An address in an Overdrive-owned range that arrives in a forwarded answer does
not reach the guest as an address, while other private answers pass unchanged.

## Learning hypothesis

A guard keyed on one SSOT of owned ranges is narrow enough to keep split-horizon
answers (`10.20.4.7`) working and strong enough that a name cannot alias a mesh
frontend (`10.98.0.7`). Falsified if a benign private answer is blocked or an
owned-range address is delivered.

## Serve + deploy loop

`overdrive serve` -> `overdrive deploy herdr-box.toml` -> guest lookup of
`build-cache.corp.example` against a controlled upstream that answers
`10.98.0.7` -> the guest does not receive it. Same name answering `10.20.4.7`
-> the guest receives it.

## Behaviour

- Owned-range answer: not delivered as an address; the guest gets a definite result, never silence.
- Non-owned private answer: delivered unchanged.
- Mixed answer set: the owned address is not delivered.
- The set of owned ranges has one SSOT shared with the allocators (frontend `10.98.0.0/16` today; `10.99.0.0/16` is DESIGN's call given #303 D4).

## Carpaccio taste tests

- Closes a real loop through production: yes, through the same entry points as slice 01.
- Thinnest: one guard, three scenarios.
- No test-only wiring: the controlled upstream is a `Sim*` adapter at the port boundary.

## Acceptance

AC-6 to AC-9 of US-GPD-2.

## Dependencies

Slice 01 (forwarding exists). DESIGN question 4 (mechanism, on-hit result,
range membership).
