# Spike wave decisions: public-ingress-gateway

## ktls-splice-relay (2026-11-02)

Findings: `findings-ktls-splice-relay.md`. Kernel 7.0.0-38 on native metal.

- (a) splice kTLS-RX -> kTLS-TX: PROMOTE, conditional on the empty-pipe rule (>= 64 KiB private pipe, splice-in only into an empty pipe, `len = min(remaining, cap)`) or a kernel carrying the v7.3 `copied` fix in `tls_sw_splice_read`. Defect: v7.0 loses 5 bytes per record when the pipe cannot map the record.
- (b) TLS 1.3 kTLS with KeyUpdate: PROMOTE. rustls `kernel::KernelConnection` via the unbuffered handshake; `dangerous_extract_secrets` is insufficient.
- (c) exact-CL / chunked splice, backpressure, keep-alive: PROMOTE. No CPU-gain claim.
- (d) connect4 selection composes with later TLS handshake and kTLS install on the same fd: PROMOTE.
- (e) kTLS+sockmap rejection absent on 7.0.0-38, present at v7.3: INFORMATIONAL; G1 uses no sockmap.
