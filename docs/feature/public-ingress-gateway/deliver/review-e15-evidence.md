# E15 different-fox review — public ingress operator journey

- Expectation: `verification/expectations/E15-public-ingress-operator-journey/`
- Evidence: `evidence/runs/20261113T135825.550600128Z-3340967/`, captured at clean commit `b0f880439` on the native x86_64 metal host (committed in `fa639219d`)
- Reviewer: independent review agent (Sonnet), dispatched with the expectation, the runner and the raw evidence only; it did not see the capturing agent's summary
- Date: 2026-11-13

## Verdict: SATISFIED

| Claim | Verdict | Evidence |
|---|---|---|
| System-trust verification, no `--cacert`/`--insecure` | HOLDS | `product/stream-verbose.err`: `* SSL Trust Anchors: *   CAfile: /etc/ssl/certs/ca-certificates.crt`; `run-example.sh` sets `public_trust_args=()` when `production_trust=1` |
| TLS 1.3 negotiated | HOLDS | `stream-verbose.err`: `* SSL connection using TLSv1.3 / TLS_AES_256_GCM_SHA384 / x25519 / id-ecPublicKey` |
| Status 200, `X-Workload: api` | HOLDS | `product/stream-headers.out`: `HTTP/1.1 200 OK` / `x-workload: api` |
| Body matches the guest's published stream | HOLDS | `product/stream-body.out` is `PIG-STREAM-CHUNK-00` … `07`, matching `examples/public-ingress-gateway/guest_server.rs` `write_stream` (`CHUNKS=8`); the example's own `cmp -s` passed (`example_exit=0`) |
| `--resolve` is address-only; the hostname is SNI, Host and certificate identity | HOLDS | `stream-verbose.err`: `Host: 151-115-99-251.sslip.io`; certificate `subject: CN=151-115-99-251.sslip.io`, subjectAltName matches; no DNS or `/etc/hosts` use |
| Kernel TLS on both the public 443 connection and the upstream leg-C connection, TX and RX configured | HOLDS | `kernel-tls-samples/during-run.txt:1597-1598`: `ESTAB 151.115.99.251:443 151.115.99.251:35928 ... tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw`; `:1601-1602`: `ESTAB 127.0.0.1:34942 100.95.0.2:8080 ... tcp-ulp-tls version: 1.3 ... rxconf: sw txconf: sw`, and the reverse-direction socket likewise |
| `/proc/net/tls_stat` corroborates, attributable to this run | HOLDS | `TlsCurrTxSw`/`TlsCurrRxSw` go 0 (pre-run) → 3 (during, matching the three live kTLS socket endpoints in the same `ss` sample) → 0 (post-run); `TlsDecryptError` stays 0. The cumulative `TlsTxSw`/`TlsRxSw` delta is host-global and not attributable, and the verdict does not rest on it |
| `serve` started the gateway through the relay probe; no `health.startup.refused` for `gateway.relay_probe` / `gateway.connect_probe` | HOLDS | `serve.log` has no match for `health.startup.refused`, `relay_probe` or `connect_probe`; it records `purpose="relay" result=Ok(GuestStream{target: 100.95.0.2:8080, ...})` |
| TCP/443 bound before the request | HOLDS | `observations/0007-gateway-status` (`listener_bound:true`) precedes `0008-public-stream` in control flow and timestamps; `pre-run.txt` has no `:443` socket, `during-run.txt` shows it `ESTAB` |
| Cleanup: no 443 listener, no upstream sockets, zero owned deltas | HOLDS | `product/cleanup-deltas`: `vm=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 public-443-sockets=0 ktls-sockets=0`; `post-run.txt` has no `tcp-ulp-tls` line |
| Provenance: clean tree, SHA, BPF object embedded exactly once | HOLDS | `build-provenance/source.txt`: `source_sha=b0f880439... dirty=false`; `bpf-consumption.txt`: `embedded_exactly_once=true`, digest cross-checked against `overdrive_bpf.sha256`; all files named in `feature-delta.md:7146-7157` present |
| No stale binary or public-CA input | HOLDS | `runner.sh` rejects `PIG_E15_OVERDRIVE_BIN` / `PIG_E15_PUBLIC_CA`; neither appears in the evidence; the binary was built in this run |
| Streaming (not buffered) | NOT DECIDED | The README disclaims deciding this from `time_starttransfer`; the evidence does not over-claim it |

## Gaps noted

1. The README status line still said no evidence existed. Updated with this review.
2. `evidence/run.log` holds only `cp: warning` lines. This is by design: `runner.sh` writes its output into named provenance and product files.
3. The cumulative `/proc/net/tls_stat` counters are not attributable to this run on a shared, internet-facing host. Immaterial: the per-socket `ss` evidence and the run-scoped `TlsCurr*` counters establish the claim.
4. The certificate chain and key paths are not recorded, by design (they reach the box as `env` words). The curl trace shows a Let's Encrypt chain (`CN=YE2`) for `151-115-99-251.sslip.io`, inside the stated operator input set.
