# V-31 findings: R2 — kernel-classified half-close, no HalfClose/HalfCloseAck, no drain deadline

Probe increment: `spike-scratch/netns-density-295-v31-r2-halfclose/` (`bpf/`, `probe/`, `scen/`, `plans/`, `evidence/` — 1118+ files, `out/`, aggregate tables in `analysis.txt`). Question source: `.context/over-engineering-audit-review.md` finding 4; `docs/research/networking/vsock-sockmap-half-close-and-drain-research.md` (R1, R2); `findings-drain-t1-t6.md`; ADR-0180. Recorded by the orchestrator from the spike agent's report (the agent's Write tool was refused). The run was interrupted once (lease deadlock with V-28) and resumed.

**PROMOTE R2** — drop `HalfClose`/`HalfCloseAck` and `FLOW_DRAIN_DEADLINE` — on two conditions:
- **A5 (new):** attach the SHUTDOWN/RST classifier as a **raw tracepoint** (`raw_tp` / `tp_btf`), not a perf-attached tracepoint.
- **A6:** a TCP-source end check (below).

Probe 2a holds on loopback only with A5; all microVM runs used the perf-tracepoint build (see Not run).

## Verdict table

| Question | Verdict | Evidence |
|---|---|---|
| 1. Clean half-close, loopback | HOLDS | 120,000 trials (240,000 flows) on an older build plus 54,000 on the final classifier build: 0 truncation, 0 stuck, 0 misclassified. The survivor's forwarded count equals the peer's received count in every sampled direction. |
| 1. Clean half-close, microVM | HOLDS | `vmclean3`: 15,000/15,000 (8,000 mixed, 2,500 FIN-only at 300 ms, 1,500 FIN-on-data at 3000 ms, 3,000 small). |
| 2a. Guest app crash | HOLDS in microVM; loopback only with A5 | microVM: 0 violations in 4,800 trials (crash-fin, crash-rst). Loopback with the perf tracepoint: stuck flows in 7 of 1,173 reps (finding below). |
| 2b. Host destination RST | HOLDS | `hunt3`: 0 clean EOF after a reset in 12,000 trials (5,931 resets); microVM 0 in 1,200. The first build returned clean EOF after a reset in 6 of 12,000 (fixed by A6). |
| 2c. Guest, host and `serve`-side owner kills | HOLDS | microVM: 6,400 held + 24,000 busy trials per owner, 0 violations; every far end got a reset (29,393 and 29,047 `cli:reset`); the survivor aborted on `peer_close_while_open`; the guest sees 0x3 and aborts when the `serve`-side owner dies. Negative control with classification off: 781 + 90 clean short EOFs on loopback, 512 in the microVM. |
| 3. Control-session loss | HOLDS | `c2-loss` (40 runs, 100,000 trials): 5,021/5,021 paired flows finished clean, 0 violations. Split-process runs on loopback and microVM: 0 violations. Only the abort-everything policy kills paired flows (765/765). |
| 4. Forced recursion misses | HOLDS; the backstop is load-bearing | 30 % of calls skipped: per-call counters 0/3,000, cumulative alone 1,936/3,000, cumulative + backstop 3,000/3,000. Also 3,000/3,000 at 70 % and 90 % skip, incl. 4.3M injected misses on 2 MiB flows. Real misses at volume (77M fexits): 96,000 flows, 0 stuck. 5 GiB flows (32-bit wrap): 6/6 exact. |
| Lost SHUTDOWN record (new) | perf-attached tracepoint FALSIFIED; raw tracepoint HOLDS | 7 of 1,173 reps with perf; 0 of 1,312 reps (~393,600 trials) with raw. |

## The finding: perf tracepoint programs can be skipped

- **Symptom:** in loopback `k-crash-fin` (30 % of trials SIGKILL the client), a few flows never finish. The client waits its full 15 s timeout; the host vsock shows `IN|HUP|RDHUP` (the kernel received the 0x2 SHUTDOWN); no ring record exists (`ringbuf_output_fail` and `shut_ring_drop` both 0). Both directions.
- **Rate:** 7 of 1,173 reps (perf) vs 0 of 1,312 (raw). At the perf per-rep rate (~0.6 %), 0 hits in 1,312 reps has probability ~4e-4.
- **Mechanism evidence:** perf tracepoint programs are skipped, counted only as a miss, when `bpf_prog_active != 1` (`kernel/trace/bpf_trace.c:109-127 @ v7.0`); raw tracepoints run regardless (`bpf_trace.c:1568 @ v7.0`). With `kernel.bpf_stats_enabled=1` polled over a 353-rep run, `vsock_recv` showed `recursion_misses` in exactly 2 reps (245 and 139); the 245-miss rep is the one with the hang. What held `bpf_prog_active` was not identified; a 12-core `BPF_MAP_UPDATE_ELEM` stressor (11.3M updates) did not reproduce it. The causal link rests on that coincidence, the kernel path and the raw-vs-perf A/B.
- **Correction:** `findings-drain-t1-t6.md` T5 ("record always precedes the wake") holds for the raw form only.

## Kernel citations (`@ v7.0`)

- `shutdown(SHUT_WR)` sends flags 0x2: `net/vmw_vsock/af_vsock.c:1179-1222`, `net/vmw_vsock/virtio_transport_common.c:1084-1096`.
- Close sends 0x3, or RST if the peer already sent the full mask: `virtio_transport_common.c:1288-1295`.
- The tracepoint fires before any socket lookup: `virtio_transport_common.c:1622`.
- Cumulative quantities advanced on the send's accept path, valid at `skb_send_sock` fexit: TCP `write_seq` (`net/ipv4/tcp.c:1390`), vsock `tx_cnt` (`virtio_transport_common.c:504`); `skb_send_sock` at `net/core/skbuff.c:3422`.
- `bpf_prog_active` is raised by map syscalls (`kernel/bpf/syscall.c:270,317,1862,1974`); sockmap/sockhash updates return before it (`:262-264`).

## Design implications

- **A5:** attach the classifier as `raw_tp` or `tp_btf`. A skipped SHUTDOWN leaves a flow stuck forever, and R2 has no deadline to rescue it.
- **A6:** treat an unclassified TCP-source EOF as an abort when any holds: `tcp_state == CLOSE` with our write side still open; forwarded count equals `tcpi_bytes_received` (no FIN sequence); forwarded count greater than `tcpi_bytes_received`. Takes the clean-EOF-after-destination-RST defect from 6/12,000 to 0.
- **A7:** with the raw tracepoint, a full ring is the only remaining loss path. Ring drops were 0 in every run, but the owner needs the drop counter and a defined response (e.g. abort the affected flows).
- Control-session loss aborts only unpaired flows; no flow depends on the session for half-close ordering.
- The cumulative read repairs any skipped middle call; only a skipped last call strands a goal, which the backstop re-evaluates (on a rise of `recursion_misses`, and once at arm time on a short read).

## Reused vs re-run

- **Reused:** the earlier agent's ablation, clean volume, control-loss, real-miss and 5 GiB runs, plus superseded builds.
- **Completed from the interrupted run:** `hunt3`, `b3-*`, `loopkill3` (pulled from the host); `vmclean3`/`vmkill3` (V3b microVM batch).
- **New:** crash-fin diagnosis with events, instrumented rebuild, raw-tracepoint implementation and raw-vs-perf A/B, `bpf_stats` polling, map-update stressor.
- **Superseded:** `c-loss-*` predates the judge fix and the 4 MiB intake `SO_RCVBUF`; `c2-*` replaces it.

Lease handling on resume: at 15:42Z the H3b lease holder was stale (launcher ssh died mid-run, sshd session kept the lease), a duplicate stale V3b holder was queued behind it, and an orphaned V-28 holder (dead launcher) grabbed the lease when freed. The two stale v31 holders and the V-28 orphan were terminated; V3b then ran to completion.

## Not run or limited

- All microVM runs used the perf-tracepoint build; the raw form is proven on loopback only. The microVM sample (~160 reps) would catch a 0.6 % per-rep rate only ~2 times in 3, so **one microVM batch on the final build is needed before DELIVER**.
- The `bpf_prog_active` holder is unidentified.
- Production tokio owner, kTLS, IPv6, UDP and hostile-guest over-credit are out of scope, as in T1–T6.

## Environment

Host and guest `7.0.0-29-generic` (guest stock bzImage `b51367c7…`). VMM: vendored Cloud Hypervisor fork `41a619d19`. Binary pins: perf-tracepoint build `1c0c4ad2…` with `drain-bpf.o` `0f873511…`; final build `cb34aee8…` (raw default; `V31_PERFTP=1` selects perf).

Cleanup: host `/tmp/v31-work` and `/tmp/v31-crash-*` removed, `kernel.bpf_stats_enabled=0` restored, no v31 processes left. Pre-existing `ovd-veth-*` links and `/sys/fs/bpf/overdrive` untouched. Local `evidence/` and `out/` kept.
