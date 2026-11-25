# Slice 01 — A guest resolves a public name and connects (walking skeleton)

> Feature: `guest-public-dns-resolution` (#382). Story: **US-GPD-1**. Job: J-OPS-005.

## Goal

A VM guest deployed with `overdrive deploy` on a node started with
`overdrive serve` resolves a public name through its default resolver
configuration and connects to it, with nothing installed by hand.

## Learning hypothesis

The node responder can forward non-mesh queries to the host's resolvers
without changing mesh answers, so a guest whose only nameserver is the gateway
resolves public names and connects. Falsified if the guest still waits out its
5 s timeout, or if any mesh answer changes.

## Serve + deploy loop

`overdrive serve` -> `overdrive deploy herdr-box.toml` -> guest
`getaddrinfo("github.com")` -> TCP connect on 443.

## Behaviour

- Public names resolve (`api.anthropic.com`, `github.com`, `crates.io`, `example.com`).
- A non-existent public name returns NXDOMAIN promptly.
- Mesh names answer exactly as today; a mesh name with no healthy backend is NXDOMAIN and is never sent upstream.
- No name under `svc.overdrive.local` is forwarded, malformed or not.
- The guest's `resolv.conf` is unchanged.

## Carpaccio taste tests

- Closes a real loop through production: yes, from deploy to connect.
- Thinnest: one forwarding arm, no guard and no failure handling beyond what the three issue ACs force.
- No test-only wiring: the upstream may be a `Sim*` adapter at the port boundary; nothing is hand-installed in the guest.

## Acceptance

AC-1 to AC-5 of US-GPD-1 (issue ACs 1-3 plus unchanged resolver config and the
namespace boundary).

## Dependencies

Shipped responder, gateway resolver and guest egress. DESIGN answers questions
1, 2 and the P2 items this slice needs (record types, TCP/UDP) first.
