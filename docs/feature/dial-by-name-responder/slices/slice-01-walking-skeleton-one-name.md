# Slice 01 — Walking skeleton: responder answers ONE name → ONE running backend

> Reviewed brief (DISCUSS, 2026-06-24; gated to Slice 00). Feature: `dial-by-name-responder` (#243). Story: **US-DBN-2**.
> Job: J-MESH-001. **The walking skeleton.** Gated by Slice 00.

## Goal (one line)

A deployed workload's `getaddrinfo("<server>.svc.overdrive.local")` resolves to the
server's **running-AND-healthy** `service_backends` addr and the connection is
intercepted + mTLS'd — driven end-to-end through `overdrive serve` + `overdrive
deploy`.

## Learning hypothesis

The in-agent listener can answer A (IPv4) from `service_backends ∩
running-and-healthy` as a **sibling name-keyed reader over the SAME
`service_backends` rows** (same `ObservationStore`, same List-then-Watch pattern as
`ServiceBackendsResolve`, third reader of the surface, D-TME-11 — NOT the addr-keyed
intercept struct), returning the SAME addr `MtlsResolve.resolve` recognizes and
classifies `Mesh` (D-TME-10), and the existing intercept path then mTLS's the hop.
**Predicted:** the resolved addr is byte-identical to the intercept path's source and
the peer wire goes TLS 1.3.

## Thinnest serve+deploy loop

`overdrive serve` (one node) + `overdrive deploy server.toml` (→ running-and-healthy,
gets a backend addr) + `overdrive deploy client.toml` (workload does
`getaddrinfo("server.svc.overdrive.local")` → resolves → connects → intercept mTLS's).
**A→B direction only** — one name, one running-and-healthy backend.

## Behavior (DESIGN owns API)

- Add the **name-answering listener** as a third **sibling** reader over the SAME `service_backends` rows (the `ObservationStore` surface, NOT the addr-keyed `ServiceBackendsResolve` struct).
- Answer `A` for `<job>.svc.overdrive.local` from `service_backends ∩ running-and-healthy` with the running-and-healthy **IPv4** backend addr (`SocketAddrV4`, headless, D-TME-10; the index gates `Backend.healthy == true`); answer `AAAA` as **NODATA** (v1 substrate is IPv4).
- Single-source: the answered addr == the addr `MtlsResolve.resolve` recognizes and classifies `Mesh`.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — `serve` + `deploy` ×2; the intercept landing is the proof. NO test installs a rule/binds a socket/supplies an addr production doesn't (CLAUDE.md vertical-slice rule).
- **Thinnest?** Yes — one direction, one name, one backend.
- **No `#[test]`-only composition?** Driven through `start_alloc`/`accept_loop`/`run_server`, not a hand-rolled harness (the 05-01 lesson).

## Acceptance (= US-DBN-2 ACs)

- [ ] `getaddrinfo("server.svc.overdrive.local")` from a deployed workload → the server's `running`-and-healthy backend addr.
- [ ] Answered addr byte-identical to the `MtlsResolve`-recognized addr AND `resolve` classifies it `Mesh` (single source; an unhealthy addr would classify `MeshUnreachable`, so it is never answered).
- [ ] Subsequent connection intercepted + mTLS'd (Tier-3 capture: TLS 1.3 `0x17`, zero payload cleartext on the peer leg).
- [ ] Resolve read is a sibling reader over the SAME `service_backends` rows — no second source of backend truth; the addr-keyed intercept struct is untouched.
- [ ] Driven by `overdrive serve` + `overdrive deploy`, not a `#[test]`.

## Dependencies

- **Slice 00 PROMOTE** (one-listener-many-netns validated).
- SHIPPED: resolv.conf injection (D-TME-9), resolve index (D-TME-11), intercept + `MtlsResolve` (arc).
