# DNS negative-control skip-hole reproduction

This bounded GuestKit diagnostic runs only the existing
`probe_refuses_when_no_bindable_port` body before remediation. The qualified
gateway route is present. The harness requires libtest success together with
the old uppercase `SKIP:` diagnostic, proving that the redundant second holder
returns before the existing typed `DnsResponderError::Bind` assertion while
the current runner accepts the result.

No production behavior, scenario, selector, lane, or physical-host network
state is changed.

## Result

Durable Metal `15c30cfa1bb75e34e45f9637` exited zero while retaining:

```text
SKIP: could not bind stand-in gateway :53 at 100.95.0.1
test result: ok. 1 passed; 0 failed; 0 ignored
DNS53_NEGATIVE_OLD_SKIP_HOLE_CONFIRMED
```

The first non-reuse wildcard holder had already occupied every overlapping
address, making the second holder impossible. The old runner's documented
patterns did not include uppercase `SKIP:`, so this infrastructure bailout was
accepted as a pass before the typed negative assertion.

Raw receipt: `evidence/metal-15c30cfa1bb75e34e45f9637.log`, SHA-256
`f15a1b4460260a0cdaac04d75d0361804a57a0857d0e93e08b6757aaa9f54ab6`.
