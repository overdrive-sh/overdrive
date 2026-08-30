# E07 — A VM Job calls an Exec Service and receives the expected reply

**Surface:** E (end-to-end) · **KPI:** Q9 · **Status:** `captured — independent review pending`

## Expectation

Using the built default-feature operator binary, deploy exactly one
`[service]` + `[exec]` callee and one `[job]` + `[vm]` caller from the
checked-in `examples/guest-stack-transparent-mtls-intercept/` bundle. The VM
resolves `gti-e07-callee.svc.overdrive.local`, sends its byte-distinct request,
and exits successfully only after receiving the exact reply. Built `serve`,
`deploy`, `workload describe`, and `job stop` commands are the only
product-driving surface.

- Anchor: S-GTI-01 in `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN Q9 in `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Example: `examples/guest-stack-transparent-mtls-intercept/`

## Verification contract

The completed runner must use one `cargo xtask metal run --` invocation so the
canonical global lease spans sync, the fail-closed native/non-virtualized
x86_64 KVM preflight, default-feature build, preparation, product commands,
public cleanup, and final owned-fixture checks. The checked-in
`execution-substrate` file is authoritative: Lima, nested KVM, and compile-only
execution are non-signal.

The runner must invoke the bundle's `prepare.sh` rather than reproduce its
logic. That entry point compiles the checked-in static helpers, reflinks and
mounts a private appliance rootfs, installs the guest caller at the exact spec
path, creates an explicit same-filesystem/traversable serve data directory,
and delivers a per-run KEK only through `CREDENTIALS_DIRECTORY`. It must not
generate source, Cargo manifests, or workload specs. The bounded `serve`
lifecycle must run in a fresh anonymous session keyring whose initial absence
of the production KEK description is checked before credential resolution; no
ambient key may be purged or overwritten.

The runtime sequence is bounded and black-box:

```text
built overdrive serve --bind <isolated-bind> --data-dir <prepared-data-dir>
built overdrive deploy --detach examples/.../callee.toml
built overdrive workload describe gti-e07-callee
built overdrive deploy --detach examples/.../caller.toml
built overdrive workload describe gti-e07-caller
built overdrive job stop gti-e07-caller
built overdrive job stop gti-e07-callee
```

Evidence succeeds only when Service describe reports the callee allocation as
`Running` with replicas `1/1` and Job describe reports the caller as
`Terminated` with the ordinary public `Succeeded` verdict within the deadline.
The checked-in caller's zero exit is causally dependent on a byte-exact,
byte-distinct response; mismatch, DNS/connect/read/write timeout, or exhausted
retry budget exits nonzero.

Traps must be installed before materialization and run on success, error, and
signal. Cleanup stops the exact caller and callee workload IDs through public
commands. Before `keyctl` or `serve` exists, the isolated wrapper must establish
and atomically publish a token-bound private process group; the parent must
track the wrapper PID/start time independently and the child must publish the
serve identity before its final `exec`. Success, failure, signal, and handshake
timeout cleanup must terminate that proven launch unit with bounded TERM/KILL
polling and reap only an observed-exited direct child. It must never perform an
unbounded wrapper wait or signal an unverified/reused PID. Cleanup then
unmounts/detaches/removes only marker-owned preparation paths. The fresh
session keyring dies with the serve lifecycle. E07 must not inspect, assert,
or repair product-private processes, run directories, cgroups, namespaces,
links, or capture state.

The expectation must not inspect or reimplement strict netlink framing,
normalized nft programs, capture/counter equality, original-destination
handling, TLS/kTLS state, wire confidentiality, generation stability, or
private cleanup. S-GTI-02, S-GTI-03, and `P-GTI-D7-*` Rust tests own those
internal guarantees.

## Captured evidence

`verification/harness/run-expectation.sh E07` completed successfully on the
qualified native-metal host on 2026-08-30 UTC. The manifest pins the source
commit, complete dirty patch, harness revision, native-metal substrate, seed,
and zero runner exit. The verbatim product output records:

- the Exec Service reaching `Running` with replicas `1/1`;
- the VM Job reaching `Terminated` with `Verdict: Succeeded` and exit `0`;
- the example's reply-dependent `E07 PASS` result;
- successful public stops for the exact caller and callee workload IDs; and
- removal of the marker-owned example materialization.

The evidence makes no internal D7, kernel-program, capture, counter, TLS/kTLS,
or product-private cleanup claim. This capture is not self-stamped
`satisfied`; an independent reviewer must audit it before that status is used.
