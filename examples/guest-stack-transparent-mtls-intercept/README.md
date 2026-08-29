# Guest-stack transparent mTLS intercept examples

This is the one checked-in operator-runnable journey for the feature. It keeps
the product example, black-box E07 expectation, and Rust integration tests at
separate boundaries. `callee.toml` deploys one `[service]` + `[exec]` callee;
`caller.toml` deploys one `[job]` + `[vm]` caller. The VM resolves the service
name and exits successfully only after receiving the byte-exact reply.

The VM specs name stable appliance paths under
`/var/lib/overdrive/examples/guest-stack-transparent-mtls-intercept/`. The E07
native-metal expectation runner may compile the checked-in Rust helpers and
materialize its rootfs copy at those paths. It must not generate replacement
source, Cargo manifests, or workload specs inline.

After compiling `callee.rs` to the path named by `callee.toml` and installing a
static build of `caller.rs` at `/opt/overdrive/examples/gti/e07-caller` in the
rootfs copy named by `caller.toml`, the operator journey is:

```text
overdrive serve
overdrive deploy examples/guest-stack-transparent-mtls-intercept/callee.toml
overdrive deploy examples/guest-stack-transparent-mtls-intercept/caller.toml
overdrive workload describe gti-e07-caller
overdrive job stop gti-e07-caller
```

The caller exits zero only after the exact response is received; any timeout or
different response exits nonzero.

This example intentionally exposes only stakeholder-visible behavior. Strict
nft/netlink framing, normalized rule programs, counter equality, packet
capture, TLS/kTLS internals, lifecycle action vectors, and private cleanup
mechanics belong to the Rust integration/component tests.
