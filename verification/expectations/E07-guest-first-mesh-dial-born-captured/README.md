# E07 — A VM Job calls an Exec Service and receives the expected reply

**Surface:** E (end-to-end) · **KPI:** Q9 · **Status:** `pending`

## Expectation

Using the built default-feature operator binary, deploy exactly one
`[service]` + `[exec]` callee and one `[job]` + `[vm]` caller from the
checked-in `examples/guest-stack-transparent-mtls-intercept/`
bundle. The VM resolves `gti-e07-callee.svc.overdrive.local`, sends its
byte-distinct request, and exits successfully only after receiving the exact
callee reply. Built `serve`, `deploy`, `workload describe`, and `job stop`
commands are the only product-driving surface.

- Anchor: the stakeholder-visible slice of S-GTI-01 in
  `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN Q9 in
  `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Example: `examples/guest-stack-transparent-mtls-intercept/`

## Verification contract

The eventual native-metal runner must build the default-feature `overdrive`
binary, compile the checked-in caller/callee helpers, materialize only the
unavoidable binaries and guest rootfs, then execute:

```text
built overdrive serve
built overdrive deploy examples/guest-stack-transparent-mtls-intercept/callee.toml
built overdrive deploy examples/guest-stack-transparent-mtls-intercept/caller.toml
built overdrive workload describe gti-e07-caller
built overdrive job stop gti-e07-caller
```

Evidence must show both deploys accepted and the caller reached its ordinary
successful terminal result within a bounded deadline. Because the checked-in
caller returns success only after a byte-exact response, that public result is
the expected-reply proof. The runner then stops the Job (accepting the public
already-stopped outcome after natural completion), stops the service/server,
and proves its own temporary materialization is removed.

The expectation must not inspect or reimplement strict netlink framing,
normalized nft programs, capture/counter equality, original-destination
handling, TLS/kTLS state, wire confidentiality, generation stability, or
private cleanup. S-GTI-02, S-GTI-03, and `P-GTI-D7-*` Rust tests own those
internal guarantees.

## Evidence

None captured. This is a pending black-box stub; the deleted historical E07
evidence attempted to prove internal D7 mechanics and is not valid evidence for
this contract.
