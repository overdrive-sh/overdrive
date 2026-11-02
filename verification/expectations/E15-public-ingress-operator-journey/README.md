# E15 — public hostname served through the gateway, kernel TLS on both legs

Status: `pending` — authored at DISTILL; no evidence exists, nothing has been captured or reviewed
Surface: E — built-product end to end (with D observations of kernel TLS state)
Execution substrate: `native-metal`
Walking skeleton: yes; the operator-visible half of the public-ingress-gateway journey

## Expectation

An operator who supplies a public hostname, a leaf-first public certificate chain
and its matching PKCS#8 key can run the checked-in `examples/public-ingress-gateway/`
journey against the **built default-feature `overdrive` binary** on native x86_64
metal, and an external client then reaches the deployed VM Service through the
gateway:

- `curl --resolve <PIG_E15_PUBLIC_HOSTNAME>:443:<PIG_E15_GATEWAY_ADDRESS> https://<PIG_E15_PUBLIC_HOSTNAME>/stream`
  verifies the gateway's certificate for that hostname against **normal system
  trust** (no `--cacert`, no `--insecure`), negotiates TLS 1.3, and receives
  status 200, `X-Workload: api` and the guest's streamed body. `--resolve` is the
  client-local address override only; the hostname stays SNI, Host and the
  certificate identity.
- While the response is streaming, the gateway holds the public TCP/443
  connection and its upstream connection toward the leg-C listener with the
  kernel TLS record layer armed (an independent `ss -tni` sample shows
  `tcp-ulp-tls version: 1.3` with transmit and receive configured on both, and
  `/proc/net/tls_stat` shows the corresponding installs), i.e. the body is not
  carried by a userspace TLS stream.
- `serve` started the gateway through the relay probe: its log carries no
  `health.startup.refused` event with reason `gateway.relay_probe` or
  `gateway.connect_probe`, and TCP/443 was bound before the request.
- After the example's owned stop and cleanup, no gateway-owned residue remains:
  nothing listening on TCP/443, no gateway upstream sockets, and the example's
  owned VM, network and cgroup deltas are zero.

The expectation makes **no** throughput, CPU or latency claim (the spike found no
CPU advantage; `docs/feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md` (c)).

## Anchors

- Anchor: S-PIG-01 / `AT-PIG-E2E-1` in `docs/feature/public-ingress-gateway/feature-delta.md` (DISTILL "Scenario list and tags").
- Anchor: `OUT-PIG-PUBLIC-REQUEST` (registered outcome; DESIGN "Application outcomes").
- Anchor: ADR-0244 (kernel TLS after a userspace handshake) and ADR-0239 (bodies move kernel-side) in `docs/product/architecture/`.
- Anchor: DESIGN "Artifact-boundary handoff to DISTILL" (operator input set, provenance and argv contract) in `docs/feature/public-ingress-gateway/feature-delta.md`.

## Exact public boundary

Operator inputs, exactly (no others; `PIG_E15_OVERDRIVE_BIN` and `PIG_E15_PUBLIC_CA`
are stale configuration and are rejected):

| Input | Meaning |
|---|---|
| `PIG_E15_GATEWAY_ADDRESS` | IPv4 the gateway binds and curl resolves to |
| `PIG_E15_PUBLIC_HOSTNAME` | Public Hostname; also Route `hostname`, SNI, Host and certificate identity |
| `PIG_E15_CERTIFICATE_CHAIN` | absolute path, leaf-first public chain |
| `PIG_E15_PRIVATE_KEY` | absolute path, matching PKCS#8 key |

Outer invocation: `cargo xtask metal run -- verification/harness/run-expectation.sh E15`
(native x86_64 KVM host, uid 0, fail closed on the substrate probes).

The runner (to be authored with the example; see "Runner contract") must, in order:

1. Require the harness-captured clean HEAD (`EXPECTATION_SOURCE_SHA`, `EXPECTATION_SOURCE_DIRTY=false`), run exactly
   `cargo xtask bpf-build`, record the object path and SHA-256, then build
   `cargo build --locked --package overdrive-cli --bin overdrive --target-dir <runner-build-root>/target`
   (default features), record the binary SHA-256, and prove externally that the
   complete BPF object bytes occur exactly once in the binary. Evidence files are
   the fixed `build-provenance/` set named in the DESIGN handoff.
2. Invoke `examples/public-ingress-gateway/run-example.sh <runner-build-root>/target/debug/overdrive <runner-scratch>/product <PIG_E15_GATEWAY_ADDRESS> --public-hostname … --certificate-chain … --private-key …` (production-trust mode).
3. While the example's public request is streaming, sample `ss -tni` and
   `/proc/net/tls_stat` from outside the product (independent oracle) and save them verbatim.
4. Capture the example's stdout/stderr, the `serve` log, the status/TLS/response
   evidence and the cleanup deltas verbatim.

## Observable refutation conditions

The expectation is **refuted** (not merely incomplete) by any of:

- the request fails system-trust verification, negotiates a version other than TLS 1.3, or any step needed `--cacert`, `--insecure`, DNS or an `/etc/hosts` edit;
- status other than 200, a missing `X-Workload: api`, a body that differs from the guest's published stream, or a response that is buffered whole (the first body bytes arrive only after the guest has finished);
- no TCP/443 socket with the TLS ULP at version 1.3 and no 127.0.0.1 upstream socket with the TLS ULP during the stream (the data path was not kernel TLS), or `TlsDecryptError` greater than its pre-run value;
- a `health.startup.refused` event naming `gateway.relay_probe` or `gateway.connect_probe`, or TCP/443 unbound before the request;
- residue after cleanup (a listener on TCP/443, a gateway upstream socket, or a nonzero owned-resource delta);
- provenance failure: dirty or mismatched HEAD, missing or duplicate exact-object BPF embedding, or a digest recomputed before the example differing from the recorded digest.

A capture that is narrated rather than executed, or whose runner was not the checked-in one, cannot support `satisfied`.

## Runner contract (to be authored in the delivery of the example)

`runner.sh` does not exist yet and no evidence exists; the harness reports `pending` until it does. It must not
import or link any `overdrive-*` crate, must not hand-program a map, rule, Route, snapshot or identity, and must
append (never overwrite) every observation. Exit 75 means "pending" per the harness; any other nonzero exit is data.
The capture is a single point-in-time event pinned to its SHA by the harness; a different reviewer decides
`satisfied` after reading the saved output. Per `.claude/rules/verification.md` this is not a regression test; the
recurring half of the same claim is `AT-PIG-E2E-1` in `tests/conformance`.
