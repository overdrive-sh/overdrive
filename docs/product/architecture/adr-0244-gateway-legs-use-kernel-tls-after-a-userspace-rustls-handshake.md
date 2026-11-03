# ADR-0244 — Hand each gateway leg's TLS record layer to the kernel after a userspace rustls handshake

## Status

Proposed — set by user ruling G1 (2026-11-02); pending independent review.
The transmit-lock exclusion has no Quint model (R-19, 2026-11-02). The cipher set
([ADR-0249](adr-0249-gateway-legs-offer-only-aes-gcm-tls13-suites.md), R-8) and the
no-KeyUpdate-origination rule
([ADR-0250](adr-0250-gateway-never-originates-a-tls-keyupdate.md), R-13) are
separate decisions.

## Context

The user ruling G1 (2026-11-02) keeps as much of the gateway data path in the
kernel as the accepted L7 feature set allows. The public leg and the upstream
leg are both TLS 1.3. Their handshakes carry gateway policy that only
userspace can apply: Web-PKI key custody and exact SNI on the public leg
([ADR-0226](adr-0226-public-listener-tls13-only.md)), the Gateway SVID and the
exact-peer SPIFFE equality on the upstream leg
([ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md)). After a handshake
completes, the record layer can run in the kernel (kTLS).

Kernel TLS exposes post-handshake control records only through `recvmsg`, and a
peer's KeyUpdate pauses the receive direction until userspace installs the new
key. A TLS 1.3 recipient must update its receiving keys when a KeyUpdate arrives
(RFC 8446 §4.6.3). Main's transparent-mTLS arm
(`overdrive-dataplane/src/mtls/ktls.rs`) installs kTLS from rustls's
`dangerous_extract_secrets` and has no rekey path. The spike
([findings (b)](../../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md))
showed rekey works through rustls `kernel::KernelConnection` (unbuffered
handshake API, one record read at a time) and not through
`dangerous_extract_secrets`.

## Decision

Complete every gateway TLS 1.3 handshake in userspace with rustls's unbuffered
API, reading one TLS record at a time so that nothing past the handshake is
consumed. Finish every policy check (public SNI and ALPN; upstream exact-peer
SPIFFE equality) before arming. Then install kernel TLS for transmit and
receive on the socket from the connection's `KernelConnection`, and keep that
object for the connection's life. A leg is never usable unarmed.

Handle every post-handshake record in userspace through the `KernelConnection`
and never forward it to the other leg: a KeyUpdate from the peer is answered and
both directions re-keyed on that leg alone; NewSessionTicket is discarded;
close_notify ends that direction; any other record type is fatal and closes both
legs. A transmit re-key is serialised with every writer of that socket.

A connection that cannot be armed is closed. A node whose kernel cannot arm the
leg and relay as this decision requires does not bind the public listener (the
relay probe, feature delta Gate G-1).

The exact arming, control-record and rekey contract (kernel calls, record
handling, locking) is in the feature delta "Contract of the legs".

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Arm kTLS from `dangerous_extract_secrets` (main's transparent-mTLS pattern) | It has no rekey path (spike (b)); a conformant peer's KeyUpdate would stall or kill a long-lived connection. |
| Keep the record layer in rustls for the connection's life (a userspace TLS stream) | Every body octet is decrypted, held and re-encrypted in gateway memory; user ruling G1 moved the record layer into the kernel. |
| Refuse the peer's KeyUpdate with an alert | RFC 8446 §4.6.3 obliges the recipient to update its receiving keys; refusing breaks conformant peers. |

## Consequences

- Plaintext application bytes between the two sockets never reside in gateway
  userspace memory; only message heads and framing lines do.
- Unsupported kernel capability fails closed at the probe, not per request.
- The kTLS plus sockmap combination is never used: the kernel rejects it from
  v7.3 and accepts only one order at v7.0 (findings item (e)).
- No CPU advantage is claimed (findings item (c)).
- The transmit-lock exclusion is covered by the assumption that the kernel does
  not serialise a multi-call logical write against a re-arm and by the Tier-3
  KeyUpdate-during-splice check, not by a model (R-19).

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0226](adr-0226-public-listener-tls13-only.md), [ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0246](adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md), [ADR-0249](adr-0249-gateway-legs-offer-only-aes-gcm-tls13-suites.md), [ADR-0250](adr-0250-gateway-never-originates-a-tls-keyupdate.md)
- [Spike findings](../../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
