# V-21 — host-internal deny enforcement in the output path (D26, ADR-0167, B-1, A-25)

**Verdict: the core claim HOLDS on both kernels.** Five conditions the design does not pin (F-1–F-5, plus minor F-6); F-1 (rule order vs the conntrack accept) and F-2 (single-transaction replacement) open real holes if DELIVER gets them wrong. UDP's reject surfaces at `send()`, not `connect()` (F-4). Not run on 6.18.

Probe and logs: `spike-scratch/netns-density-295-v21/` (`run.sh`, `probe.py`, `peer.py`, `out/run-lima-aarch64-full.log`, `out/run-metal-x86_64-full.log`, `out/run-lima-aarch64-r2.log`). No eBPF: D26 specifies nftables only.

## Environment

Lima `7.0.0-34-generic` aarch64; metal `7.0.0-29-generic` x86_64, bare metal; nft v1.1.6. Both full runs: 123 checks, 122 as predicted (the other is F-1, identical on both). Metal ran from `/tmp/v21-spike/` over ssh under `sudo flock -w 1500 /run/lock/overdrive-metal-shared.lock` (`cargo xtask metal run --no-sync` refused the source on a digest mismatch).

Topology: netns `v21-host` (root stand-in), `v21-peer` (off-host). Gateway `100.95.0.1/16` on bridge `v21-br0`, tagged `local 100.95.0.0/16 dev lo proto 251` (ADR-0152), production fwmark policy routing, host veth `10.77.21.1/30`, operator locals `192.0.2.55/32` and `169.254.1.1/32` on `lo`. Off-host `198.51.100.7`, `169.254.169.254:80`, neighbour `10.77.21.2`. Host services: wildcard `0.0.0.0:22022` (TCP+UDP), loopback-only `127.0.0.1:22023`, DNS stand-in `100.95.0.1:53`, leg-F stand-in `127.0.0.1:22024`.

Rule under test (model of the constant D26 rule), table `ip v21-mtls`, chain `output` (`hook output priority filter`): (1) accept owner-marked to `100.95.0.1` UDP/TCP 53; (2) reject owner-marked to `127.0.0.0/8` or `169.254.0.0/16`; (3) reject owner-marked with `fib daddr type local`. Owner mark `0x4` (`meta mark 0x4`), distinct from TPROXY `0x1` and leg-S `0x2`. Oracle: server-side hit counter. Positive control P0 (no table): all 26 marked dials reach their server.

## Verdicts

1. **Core deny — HOLDS.** 0 of 20 marked dials reached a server (`127.0.0.1` both services, `127.0.0.2`, `0.0.0.0`, `10.77.21.1`, gateway `:22022`, `100.95.0.7`, `169.254.1.1`, `192.0.2.55`, `169.254.169.254:80`). TCP `connect()` `ECONNREFUSED` in 0.1–0.7 ms; UDP `connect()` ok, first `send()` `EPERM`; 59,000 B datagram rejected the same way.
   ```
   [P1] PASS marked tcp -> 127.0.0.1:W wildcard service   reached_host_socket=False {'connect': 'ECONNREFUSED'}
   [P1] PASS marked udp -> 127.0.0.1:W wildcard service   reached_host_socket=False {'connect': 'ok', 'send': 'EPERM'}
   ```
2. **`gateway:53` allowed; off-host reachable — HOLDS** (TCP/UDP incl. 59,000 B; also in the composed ruleset and under `!= unicast`).
3. **Unmarked / other-mark sockets unaffected — HOLDS.** Owner's connect to leg-F must stay **unmarked** — a marked one is refused.
4. **Address added after the owner's check (R5-7) — HOLDS.** No-rule control reached the service; with the rule TCP `ECONNREFUSED`, UDP `EPERM`. Also a fresh dummy interface and an admin-down interface (still classed local).
5. **Bypasses — none worked:** v4-mapped IPv6, `SO_BINDTODEVICE`, 59,000 B datagram, the address-change race. Leaks found are F-1–F-6.
6. **Composition with the leg-S exemption, `ct state established,related accept` and the ADR-0172 prefix reject (D26 first) — HOLDS.**
7. **Reject → `DestinationUnreachable` — HOLDS for TCP, FALSIFIED as worded for UDP** (rejection at first `send()`, F-4).

## Findings DELIVER must pin

- **F-1 — Rule order vs `ct state established,related accept`.** With the conntrack accept first, a marked UDP socket reusing the source port of an earlier unmarked flow to a local destination matches ESTABLISHED and reaches the service:
  ```
  [P3] FAIL ordering ct-first:  ... reached_host_socket=True  {'send': 'ok', 'reply': True}
  [P3] PASS ordering d26-first: ... reached_host_socket=False {'send': 'EPERM'}
  ```
  Pin: the D26 rule precedes any `ct state established,related accept` in the output chain (or that accept excludes the owner mark).
- **F-2 — Replacement in one nft transaction.** Marked-connect storm during repeated replacement: `delete table` + `add table` in one `nft -f` → 0 leaked (326 Lima / 349 metal); separate delete then add → 559/1132 (Lima), 1446/2010 (metal) leaked.
- **F-3 — Verdict is plain `reject`.** `reject` → TCP `ECONNREFUSED`, UDP `EPERM`; `admin-prohibited` → `EHOSTUNREACH` / `EPERM`; `reject with tcp reset` → UDP **delivered and answered**; `drop` → TCP hangs to timeout.
- **F-4 — UDP reject invisible at `connect()`.** The first forwarded datagram fails `EPERM`; DELIVER treats `send()` `EPERM` on a forwarded datagram as a flow error with the `DestinationUnreachable` projection. The owner's `HostInternal` check refuses first; the rule is the backstop for the check-then-connect window.
- **F-5 — Broadcast and multicast escape `fib daddr type local`.** With `SO_BROADCAST`, marked UDP to `10.77.21.3` or `100.95.255.255`, and marked UDP to `224.0.0.1`, reached the wildcard host service. Without `SO_BROADCAST` `connect()` fails `EACCES`. `fib daddr type != unicast` closed all three (32/32) while still allowing `gateway:53`, `198.51.100.7`, `10.77.21.2` and rejecting every local case. Pin either "owner never sets `SO_BROADCAST`" or the broader match.
- **F-6 (minor) — exact mark.** Marks `0x5` and `0x104` reached `127.0.0.1:22022` under `meta mark 0x4`; the owner socket carries exactly the owner mark, or match on a mask.

## Design implications

Nothing falsified at the contract level; A-25 validated. Add: D26 rule position (F-1), single-transaction replacement (F-2), plain `reject` (F-3), UDP `send()` projection (F-4), a decision between "never `SO_BROADCAST`" and `fib daddr type != unicast` (F-5).

Cleanup: all `v21-` resources removed on both hosts.

**Gate recommendation:** PROMOTE A-25 / V-21 with F-1–F-6 pinned.
