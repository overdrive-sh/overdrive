# DNS negative-control typed-assertion proof

This bounded GuestKit proof runs only the existing
`probe_refuses_when_no_bindable_port` body after removing its redundant second
holder. The first non-reuse wildcard holder remains the real all-address block.
The harness requires the original typed `DnsResponderError::Bind` assertion to
pass, zero skip diagnostics, and libtest success.

No production bind behavior, expected error, scenario, selector, lane, or
physical-host network state is changed.

## Result

Durable Metal `a9a3ea8eb54fc1ee4697ed0d` exited zero. It first applied the new
line-oriented skip detector to the retained old raw receipt and printed
`old-uppercase-SKIP-colon=new-detector-reject`. The exact fixed native body
then retained:

```text
[diag] DNS53 negative typed error: Bind {
  addr: 100.95.0.1:53,
  source: ... AddrInUse ...
}
test result: ok. 1 passed; 0 failed; 0 ignored
DNS53_NEGATIVE_TYPED_ASSERTION_PASS
```

No skip diagnostic occurred. Raw receipt:
`evidence/metal-a9a3ea8eb54fc1ee4697ed0d.log`, SHA-256
`de3b767616128c2285e2906260e203a805f52c965c1cfe2476f8d6e229368cf5`.
