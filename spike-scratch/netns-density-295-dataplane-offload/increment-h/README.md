# Gateway route ownership proof

This bounded GuestKit proof exercises the control-plane native suffix route
preparation in two fresh guests:

- missing route: setup creates the exact GW-LOCAL `/32`, DNS positive cases
  pass, and cleanup removes only that fixture-created route;
- preexisting valid route: setup borrows the identical route, the exact two
  adoption cases and DNS positive cases pass, and cleanup preserves the
  borrowed route.

Both paths require the exact kernel rendering
`local 100.95.0.1 dev lo proto 247 scope host`; a differing route fails closed.
No prefix-wide/default route, interface address, production ownership protocol,
selector, lane, scenario, or physical-host network state is added.

## Result

Durable Metal `678b2e2aba65af9927e27fe0` exited zero.

- Fresh path: setup recorded `ownership=created`; both positive DNS fallback
  bodies passed; cleanup recorded `gateway_after=''`.
- Preexisting path: setup recorded `ownership=borrowed` for exact kernel
  rendering `local 100.95.0.1 dev lo proto 247 scope host`; both adoption and
  both positive DNS bodies passed; cleanup retained the identical route.
- Both paths retained distinct resolver-holder and production-gateway sockets,
  zero skips, and no route covering a non-gateway prefix address.

The proof ended `DNS53_GATEWAY_OWNERSHIP_PASS`.

Raw receipt: `evidence/metal-678b2e2aba65af9927e27fe0.log`, SHA-256
`274e4541d3e9bab3a99106709995f6ff17f8d47237ab9c1a59538fca904f2033`.
