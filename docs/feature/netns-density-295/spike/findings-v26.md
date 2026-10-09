# V-26 — the kernel facts of the guest-prefix steering (D8a-LOOKUP, D8a-LOAD-SWAP, A-30, K-L1–K-L5, K-L9)

**Verdict: all seven claims HOLD** on Lima (7.0.0-34, aarch64) and metal (7.0.0-29, x86_64). K-L9 holds, so D8a-LOAD-SWAP stays. **One design finding outside the claims (F-1)** contradicts a design precondition. Not yet run on the 6.18 pin.

Probe and raw logs: `spike-scratch/netns-density-295-v26/` — `ebpf/` (aya-rs `sk_lookup` programs `v26_steer` and `v26_ifx`), `probe/` (userspace, raw `bpf(2)` for link update), `run.sh`, `build.sh` / `build-x86.sh`, `out/run-lima-aarch64-full.log`, `out/run-metal-x86_64-full.log`.

## Environment and model

- **Lima:** `7.0.0-34-generic`, aarch64 (usable, not recreated). **Metal:** `7.0.0-29-generic`, x86_64, `systemd-detect-virt` = none; static binary scp'd to `/tmp/v26-spike` and removed after.
- Both: 30 checks, 0 failed. All resources `v26-`-prefixed and cleaned up.
- **6.18 caveat:** the mechanisms are much older than 6.18, but the claim is about 6.18 — re-run `./run.sh` unchanged on a 6.18 image.
- Netns `v26-host` stands in for root, with a peer netns over a veth for remote clients. Prefix `100.95.0.0/16`, gateway `.1`, tagged `local` route on `lo`; production's fwmark policy routing and `tproxy ip to 127.0.0.1` leg-C installed. A wildcard "sshd" on `0.0.0.0:22022`. Marked connects use mark `0x2`. The "constant nft rules" are a model; the real rules are PORT-295-C's.
- **Positive controls:** with no program, 200/200 local and 200/200 remote connects reach the wildcard; UDP reaches it. A non-atomic detach-then-attach replace under the same storm leaked 285/532 (Lima), 429/532 (metal).

## Verdicts

1. **K-L1 — HOLDS.** No entry: marked and unmarked connects reset 3000/3000 before the wildcard lookup (wildcard 0, intake 0). An exact-bound unsteered listener is never reached (2000/2000 reset); with an entry, assigned 1000/1000. The assigned socket is the one the map names (entry `100.95.0.3:22022` → listener on `127.0.0.1:34567` received 200/200). UDP → ECONNREFUSED, no wildcard hits. With the constant nft rules present, marked connects behave the same.
2. **K-L2 — HOLDS.** TPROXY-assigned packets never reach `sk_lookup` (counter +0 while leg-C accepts 1000/1000; rule removed → +200/200). leg-C still gets 1000/1000 with the program attached and an entry present.
3. **K-L3 (close) — HOLDS.** Key gone as soon as close returns (1 → 0); 2000/2000 later connects reset; an already-accepted child still exchanges ping/pong. 1000-cycle listen/steer/connect/close/connect race: 0 connects after close, 0 lingering keys.
4. **K-L4 — HOLDS.** Owner SIGKILLed, and again after graceful exit: `bpftool link show pinned` still names the program; `bpftool map dump` → `Found 0 elements`; 4000/4000 connects reset; wildcard saw none.
5. **K-L5 — HOLDS.** ifindex 1 for 2000/2000 host-local connects; veth index for 1000/1000 remote. Remote connect to a steered address refused 1000/1000; to an unassigned prefix address with nft rules deleted also refused 1000/1000 (wildcard 0).
6. **K-L9 — HOLDS; D8a-LOAD-SWAP is not returned to DESIGN.** Swap under a 10 ms connect loop leaves the link on the new program, every connect reset. Storm of 5.45M swaps (3 s, 4 connect threads, Lima): 618,512 connects, all reset, none reached a host socket; a sampler saw the link running both programs. Metal: 3.43M swaps, 598,199 connects, 0 leaks. Failing updates each left the link on the earlier program:

   | Failure case | Result |
   |---|---|
   | Program of another attach type (XDP) | `EINVAL` |
   | `expected_old` mismatch | `EPERM` |
   | Replacement fd is a pipe | `EINVAL` |
   | Replacement fd is -1 | `EBADF` |

   `BPF_F_REPLACE` is `1 << 2`. aya 0.13.1 has no `BPF_LINK_UPDATE` wrapper for `SkLookup` — DELIVER needs a raw `bpf(2)` call or its own wrapper.
7. **Capacity (K-L3 / D8a-CAP) — HOLDS.** 262,144 listeners inserted in 0.8 s (aarch64); the 262,145th returned `E2BIG`; 1000/1000 close-one-then-insert cycles succeeded with the map exactly full; closing all left 0 keys.

## Findings for the design

- **F-1 (contradicts a design precondition).** `bind(100.95.0.2:22022)` returns `EADDRINUSE` while a wildcard `0.0.0.0:22022` listener exists, even with REUSEADDR, REUSEPORT and FREEBIND set on the intake side only. So the `steer` precondition ("bound to exactly `intake`") and R5-1's setup ("host service on `0.0.0.0:p`, VM declares `p`") cannot both hold on a real node; an intake listener created first would also block the host service from starting. `sk_lookup` does not need the exact bind: listeners bound to `A:0` and to `127.0.0.1:34567` were assigned and received traffic. Needs a design decision.
- **Reply source address.** A host-local connect to `100.95.0.2` has source = destination (`ip route get` → `src 100.95.0.2`), so replies are addressed into the prefix and unmarked. The first nft model timed out 300/300 until `ct state established,related accept` was added. The constant guest-prefix drop/reject rules must not catch replies.
- **Reset latency.** p50 0.011 ms, p99 ~0.03 ms; scheduler outliers 4–10 ms (Lima), 2.7 ms (metal). Pin the outcome plus a percentile, not a hard per-reset bound.

## Reproduce

```
cargo xtask lima run --no-sudo -- bash -lc 'cd spike-scratch/netns-density-295-v26 && ./build.sh'
cargo xtask lima run -- bash -lc 'cd spike-scratch/netns-density-295-v26 && ./run.sh'      # or ./run.sh p4
```

**Gate recommendation:** PROMOTE the seven claims; F-1 and the reply-path rule go back to DESIGN before the steering scenarios are distilled.
