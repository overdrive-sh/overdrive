# E14 — publicly trusted public-ingress operator journey

Status: pending
Surface: E — built-product end to end
Execution substrate: real-kernel Linux with a publicly trusted operator chain
Walking skeleton: no; point-in-time production-trust receipt

## Expectation

An operator starts the built default-feature product with the approved public
gateway arguments and a publicly trusted certificate chain for an
operator-controlled Public Hostname, deploys the checked-in api Service and public-api Route, and
receives the workload response over TLS 1.3 / HTTP/1.1. Gateway status remains
readable over operator mTLS and contains no credential material.

- Anchor: AT-PIG-E2E-1 in the public-ingress feature delta.
- Anchor: OUT-PIG-PUBLIC-REQUEST in docs/product/outcomes/registry.yaml.
- Anchor: ADR-0114, ADR-0127 through ADR-0131, and ADR-0132 through ADR-0142.

## Verification

The runner requires the four operator inputs `PIG_E14_GATEWAY_ADDRESS`,
`PIG_E14_PUBLIC_HOSTNAME`, `PIG_E14_CERTIFICATE_CHAIN`, and
`PIG_E14_PRIVATE_KEY`. It rejects stale binary/public-CA inputs, requires the
harness-captured checkout to be clean, and builds the default-feature product
itself from the captured HEAD with `--locked`. Before that product build it
runs the repository's exact `cargo xtask bpf-build` path, records the BPF
command, complete output, exit status, object path and SHA-256, and rechecks
that HEAD and the clean source state did not change. It records the source SHA,
literal product-build command, exit, toolchain, resolved artifact, and SHA-256
before rechecking both artifact digests immediately before use. A byte-level
receipt proves the built default-feature binary embeds that exact captured BPF
object once; this uses the existing build/include path and adds no attestation
API. It
invokes the checked-in example and retains only stakeholder-visible command
output, redacted gateway status, public TLS headers, and streamed response.
It fails closed unless the observed substrate is verified Lima Linux running
as uid 0, and records the observed Lima marker, OS, hostname, kernel, uid,
cgroup-v2 and bpffs facts. Certificate
subject/issuer/validity, OpenSSL chain verification and curl's
`ssl_verify_result=0` are retained as the point-in-time public-trust receipt.
The captured operator-mTLS gateway-status JSON is parsed for the exact active
application, listener, connect-path, gateway-identity and certified-key
projections and rejected if either supplied credential path or any protected
key/PEM field is serialized.
It performs no BPF-map, socket, kernel, private lifecycle, or cleanup oracle;
those remain the recurring Tier-3 test's responsibility.

The capture never depends on ambient DNS and never edits /etc/hosts. It passes
the configured gateway IPv4 to the checked-in example, whose public client uses
curl --resolve for the operator Public Hostname. The override supplies only the captured
gateway IPv4; neither public HTTPS request uses `--cacert` in this expectation
(the separate operator-mTLS control plane still pins its private CA). The
normal system trust store validates the public chain while
TLS SNI, HTTP Host, and certificate identity remain that Public Hostname. Production
DNS remains operator-managed.
