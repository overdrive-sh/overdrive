# Findings: public-name DNS from a VM guest under `serve` + `deploy` (#382)

**Verdict: DOESN'T WORK.** A VM guest cannot resolve public names through its configured resolver. The node responder at the guest gateway does not answer non-mesh queries; the guest's lookup times out. Egress to an external resolver by IP works.

- Kernel: `7.0.0-38-generic` (native metal, x86_64). Source: `fe0c3adcf` (clean except unrelated docs). Metal run id `b20bcb2fb8028a614e91755c`, 2026-11-25.
- Harness: `src/main.rs` boots the real `serve` composition in-process (`run_with_adapters`, `NodeAdapters::host`, SimKek) and deploys one VM Service through the public `deploy` handler. Nothing is hand-installed. Guest program: `guest/probe.rs` (static musl, std only).
- Raw output: `out/run2/evidence.log`, `out/run2/run.log` (gitignored).

## Prediction (written before the run)

- Hypothesis: the node responder drops non-mesh queries, so the guest's default resolver times out on public names.
- Predicted: A default lookup fails after ~5-10 s; B raw query to gateway:53 for a public name gets no reply; C raw query to 1.1.1.1:53 answers; D gateway answers a mesh-name query; E TCP to 1.1.1.1:443 connects.
- Falsification: A resolves, or B gets a reply.

## Observed (guest console, verbatim)

```
probe: resolv.conf = "nameserver 100.95.0.1\n"
probe: begin gateway=100.95.0.1
probe: A1 default-resolver example.com FAILED err=failed to lookup address information: Try again elapsed_ms=5005
probe: A2 default-resolver api.anthropic.com FAILED err=failed to lookup address information: Try again elapsed_ms=5005
probe: B  raw gateway example.com try=1 NO-REPLY kind=WouldBlock elapsed_ms=3001
probe: B  raw gateway example.com try=2 NO-REPLY kind=WouldBlock elapsed_ms=3005
probe: B  raw gateway example.com try=3 NO-REPLY kind=WouldBlock elapsed_ms=3005
probe: C  raw 1.1.1.1 example.com try=1 REPLY from=1.1.1.1:53 bytes=61 rcode=0 ancount=2 elapsed_ms=9
probe: D  raw gateway mesh-name try=1 REPLY from=100.95.0.1:53 bytes=94 rcode=3 ancount=0 elapsed_ms=7
probe: D2 default-resolver mesh-name FAILED err=failed to lookup address information: Name does not resolve elapsed_ms=8
probe: E  tcp 1.1.1.1:443 CONNECTED elapsed_ms=2
probe: done
```

Host control (same box, same moment): the host resolves both names (`getent hosts example.com api.anthropic.com` returned addresses via `151.115.125.3` / `151.115.125.11`).

## Reading

- A1/A2 match the prediction: musl's resolver gives up after its 5 s timeout with `EAI_AGAIN`.
- B: three 3 s waits, no datagram back. The responder receives non-mesh queries and stays silent (`responder.rs:461`, `.ok()?` on `WireError::NotMeshName`).
- D: the responder is alive and replies promptly to a mesh-name query. It returned NXDOMAIN (rcode 3) for the probe's own name, most likely because the probe does not listen on its declared port 8081 and so has no healthy backend; not investigated, and not material to this finding.
- C and E: guest UDP and TCP egress to external addresses work through the ADR-0162 path.

## Design implications

Public-name resolution needs a fix at the responder or in the guest's resolver config (#382 lists the options). Egress already carries UDP/53 to an external resolver, so the forwarding responder has a working path to an upstream.

## Gate

DOESN'T WORK confirmed; the gap in #382 is real.
