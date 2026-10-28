# DNS/53 qualified fixture confirmation

This bounded GuestKit proof reruns the fresh DNS fallback case and the exact
two-adoption-plus-DNS prefix after correcting only the private positive
fallback stimulus and installing the qualified node's exact
`local 100.95.0.1/32 dev lo proto 247` route.

The positive stand-in binds the appliance-shaped specific loopback resolver
address `127.0.0.53:53`. It still makes the production wildcard bind fail,
while it does not occupy the distinct shared-gateway fallback address. The
negative no-bindable-port control remains unchanged. No private namespace,
prefix-wide route, default route, production API, or bind behavior is added.

## Result

Durable Metal `61d8273016ed5daa3b3cf1cb` exited zero. In both fresh and
two-adoption-prefix guests:

- no UDP/53 owner existed before each DNS case;
- the exact local gateway route was present with proto 247;
- the private holder occupied only `127.0.0.53:53`, with reuse disabled and
  CLOEXEC set;
- the production responder owned a distinct `100.95.0.1:53` socket after
  probe; and
- both existing positive fallback bodies reported one pass, zero failed/
  ignored, and no skip.

Both adoption cases also passed before the prefix DNS cases. The proof ended
with `DNS53_QUALIFIED_FIXTURE_PASS`.

Raw receipt: `evidence/metal-61d8273016ed5daa3b3cf1cb.log`, SHA-256
`c9e09ba097cadab006ba7627a4869220d1022f11427be38c39a80d686ff58edc`.
