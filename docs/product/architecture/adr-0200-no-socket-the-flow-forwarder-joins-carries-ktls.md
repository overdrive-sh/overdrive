# ADR-0200 — No socket the flow forwarder joins carries kTLS

## Status

**Accepted — EVMOD-KTLS, approved by the user 2026-10-17.** GH #295. Answers
the user's design input of the data-path ruling (ADR-0197, accepted
2026-10-16): "TLS-record-aware peek (cmsg) with close_notify → FIN on the
other side, if the module's TCP half carries kTLS". The attach refusal is in
the #295 feature delta, § *[REF] vsock data path — the in-kernel
event-driven forwarder (EVMOD)*, "kTLS".

## Context

The evmod follow-up spike installed kTLS (TLS 1.3, software) on the module's
TCP legs (`spike/findings-evmod-followup.md`, item 4):

- With kTLS installed before the module attached, peek and consume worked
  byte-exact through the record layer and the bound held, but a
  `close_notify` alert aborted the pair: `tls_sw_recvmsg` handles `MSG_PEEK`
  (`net/tls/tls_sw.c:2062 @ v7.0`) and returns `-EIO` when the next record is
  not application data and the caller passed no control buffer
  (`tls_record_content_type`, `:1844-1860`). Software kTLS cost 30–45 % of
  single-flow throughput.
- With kTLS installed after the module attached, the TLS ULP saved the
  module's callback and called it from its own path (`:2511-2516`,
  `:2693-2694`); unloading the module then panicked the guest. ADR-0198 keeps
  a module reference for such a socket for the life of the boot, which
  removes the panic but not the record handling.

Which sockets the forwarder joins in #295 (feature delta, "Sides and
pairs"): on the host the flow vsock and the destination TCP socket, the
loopback TCP socket to leg-F, or the connected UDP socket; in the guest the
intake child, the connection to the application, the UDP slot socket and the
flow vsock. None carries kTLS. leg-C's kTLS socket is relayed by leg-S in
userspace through a `GuestStream` (ADR-0186), which the forwarder only
watches; that relay is deleted when #303 lands. After #303 the guest's kTLS
socket reaches the host through a loopback TCP peer that carries
ciphertext (END-STATE), and the kernel already refuses a psock on a socket
with a ULP (`net/core/skmsg.c:756-758 @ v7.0`) for the same reason.

## Decision

The forwarder refuses to attach or watch a TCP socket that has an upper
layer protocol installed (`inet_csk_has_ulp`,
`include/net/inet_connection_sock.h:349 @ v7.0`), with a distinct error, and
no Overdrive owner installs a ULP on a socket after attaching it. No
forwarded byte is a TLS record the forwarder must parse; TLS stays on the
sockets that terminate it (leg-C, leg-F, and the guest workload after #303),
and the forwarder moves their ciphertext or plaintext as an opaque stream.

## Alternatives considered

- **Forward leg-S in the kernel: attach leg-C's kTLS socket to the flow
  vsock**, with a record-aware peek: `recvmsg` with a control buffer reads
  `TLS_GET_RECORD_TYPE` (`net/tls/tls_sw.c:1854 @ v7.0`), application-data
  records are forwarded, a `close_notify` alert becomes `shutdown(SHUT_WR)`
  on the vsock, any other alert aborts, and kTLS is installed before attach.
  It would delete the one userspace payload relay early and recover the
  relay's measured cost (V-28: +0.5 ms per connection, 12–37 % less
  client-to-guest throughput). Not chosen: it needs a contract-shaping spike
  (peek of non-data records, partial records, alert ordering against FIN)
  for a path #303 deletes, and the forwarder then parses TLS for that path
  only.
- **Allow kTLS installed after attach.** Safe for memory only because such a
  socket keeps its module reference for the life of the boot (ADR-0198), and
  every alert still aborts. Rejected.

## Consequences

- The forwarder has no TLS code and no dependency on the `tls` module.
- leg-S remains the temporary userspace relay of ADR-0186, with its measured
  cost, until #303; it learns how the guest ended its direction from a
  forwarder watch on its flow vsock (ADR-0180), not from a TLS record.
- A future decision to forward a kTLS socket is a new ADR with its own
  spike; nothing in #295 depends on it.
- ADR-0204's late-callback rules depend on this decision: with no ULP on
  an attached socket, no other subsystem saves a copy of a forwarder
  callback, so an override entered after its pair ended can call the
  socket's current callback without leading back into the forwarder. A
  decision to allow a ULP on a forwarded socket must revisit ADR-0204.
- Measured on Linux 7.0 (P-70, `spike/findings-ev-ctl.md` § 3.6–3.7):
  attach refuses a TCP socket with a ULP or with kTLS TX+RX
  (`EPROTONOSUPPORT`), before checking its state; kTLS installed on an
  attached socket and its far end was byte-exact with one end record for
  four endings and no oops, and `delete_module` stayed refused. The
  installed-after-attach case clobbers the saved callbacks at unhook, which
  no Overdrive owner produces.
