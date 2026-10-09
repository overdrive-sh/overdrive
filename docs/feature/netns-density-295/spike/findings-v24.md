# V-24 — the guest observes the host's close of a `TcpAccept` (K-A4, round-2 model finding r2-3)

**Verdict: HOLDS**, with one wording correction (first read) and one dependency (H-1). 18,930 of 18,930 torn-down flows ended correctly on each of two independent boots.

Probe and raw captures: `spike-scratch/netns-density-295-v24/` (`guest/` static Rust probe, `increment-{g,h}/run.py` + `evidence/`, `analyze.py`, `launch.py`). Increments a–f were harness fixes and back no verdict.

## Environment

- Native x86_64 metal host (AMD EPYC 8024P), `systemd-detect-virt` = none, KVM API 12; fail-closed substrate probes passed; run under the metal lease.
- Host and guest kernel `7.0.0-29-generic` — **not** the ADR-0068 6.18 pin. Re-run on the pin when available.
- Cloud Hypervisor fork: `vendors/cloud-hypervisor`, branch `overdrive/vhost-kernel-vsock`, `41a619d191f6472f79a3f22bcb9576c2a516906c` (v53.0 + 2), launched `--vsock cid=…,backend=vhost-kernel`.
- Guest CIDs 3240 (`increment-g`) and 3241 (`increment-h`).

## What was probed

- **Host:** opens V_h to the guest, sets `SO_LINGER{1,0}`, writes the 16-byte request, then closes for each cause in the row: abort (`Abort` then close), quiescence (close only), control-session loss (kill the session; the guest reconnects), close after the reconnect; plus SIGKILL of the owner process.
- **Guest:** a userspace model of the guest owner — accepts V_g, reads the request, connects C to an in-guest app, holds, observes V_g, tears down on EOF or error.
- **Guest states:** V_g accepted (600 ms hold before `Paired`), and V_g still queued (`accept()` withheld; `vsock_diag` showed 1,000 queued).
- **Two guest modes:** honouring control messages, and ignoring them (isolates the kernel's signal on V_g).
- **Positive control:** payload after `Paired` reached the app 100/100 in both states.

## Verdicts (identical on both boots)

| Sub-claim | Verdict | Evidence |
|---|---|---|
| (i) host closes after the guest accepted V_g, before `Paired` (abort / quiescence / session loss) | HOLDS | 1,000/1,000 per cause per mode, plus SIGKILL and blind-`Paired` races. V_g read → EOF, write → `EPIPE`, poll `POLLIN\|POLLRDHUP`, C reset. |
| (ii) host closes while V_g is queued, then the guest accepts | HOLDS | 1,000/1,000 for abort, quiescence, session loss, close-after-reconnect, SIGKILL; 100/100 after a 10 s settle. `POLLRDHUP` already set at accept 1,000/1,000. |
| (iii) session killed while queued, guest reconnects, then accepts | HOLDS | 1,000/1,000 for both session-loss variants. |
| "First read fails or reads EOF" (literal wording) | FALSIFIED in letter, HOLDS in substance | The first read returns the buffered 16-byte request (1,000/1,000); the next read is EOF, the first write `EPIPE`, `POLLRDHUP` set at accept. `ECONNRESET` never seen on V_g. |
| C sees `ECONNRESET` within 100 ms of the guest's accept | HOLDS per flow; INCONCLUSIVE as a bound under a 1,000-flow burst | Single flow 0.24–0.59 ms. 1,000-flow bursts: p99 up to 112 ms, max 136 ms — guest scheduling on 2 vCPUs, not propagation. |
| No byte of the aborted flow reaches the app | HOLDS only with H-1 | 0 bytes on C across 18,930 flows. If the host writes after the request before `Paired` and then closes, 3,200/3,200 bytes reach C. |
| Zero guest connections 1 s after | HOLDS | `vsock_diag` and `/proc/net/tcp` show 0 leftovers after every case. |
| U-5: `Paired` for a closed flow / `Abort` for an unheld flow is discarded with no reply | HOLDS | Host discarded 1,000/1,000 blind `Paired`; guest discarded 1,000/1,000 `Abort`s; no replies; the flow still ended via V_g 1,000/1,000. |

Close-to-guest propagation for an accepted V_g, single flow: p50 0.07–0.10 ms, worst 0.24 ms.

Edge cases covered: close with no request written (EOF at request read, 100/100); close without linger (100/100 each state); 10 s queue settle; sequential single flows; owner SIGKILL.

## Design implications

- **No control-protocol change.** No falsification condition occurred.
- **DISTILL wording:** R5-28, R5-31 and the TcpAccept rows of G-V5/G-V6 say "after the request read, V_g reads EOF, reports `POLLRDHUP`, and its first write fails `EPIPE`" — not "the first read fails". The guest owner checks for end-of-flow after the request read; a successful 16-byte read does not prove the flow is live.
- **H-1 must stay.** "No byte reaches the app" depends on it: the kernel delivers data sent before the close.

## Limits

- Kernel 7.0.0-29 on both sides, not the 6.18 pin.
- The guest owner is a userspace model; the production sockmap route V_g → C over an already-ended V_g was not probed — DELIVER step 5 checks that such a route tears C down.
- Not probed: host close racing a guest `accept()` while the request write is in flight; accept-queue overflow; guest-owner restart.
- Ports differ from the design's (52420–52424); kernel behaviour is unaffected.

**Gate recommendation:** PROMOTE — V-24 discharged; carry the wording correction and H-1 dependency into DISTILL.
