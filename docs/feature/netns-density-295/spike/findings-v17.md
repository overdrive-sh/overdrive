# V-17 — hybrid unframe under the appliance's interface set (D5a-IFACE, D5a-IFACE-BOOT)

**Verdict:** (a) independent interfaces with policy routing **HOLD**; (b) boot attach refusals observed and mapped **HOLD**; (c) interfaces **stacked** on an attached interface (macvlan, VLAN, bridge) with both upper and lower attached, as D5a-IFACE directs, are **FALSIFIED** — silent corruption and loss.

Probe and logs: `spike-scratch/netns-density-295-v17/` (`run.sh`, `r4.sh`, `build.sh`, `build-x86.sh`, `ebpf/src/main.rs`, `probe/src/main.rs`, `out/run-lima-aarch64.log`, `out/run-metal-x86_64.log`, `out/r4-*.log`, `out/kernel-tcx.c.excerpt-source.txt`).

## Substrate

Lima `7.0.0-34-generic` aarch64 (as root via `cargo xtask lima run --`); metal `7.0.0-29-generic` x86_64, `systemd-detect-virt` = none (scp + ssh to `/tmp/v17-run/`, uplink untouched). Not the 6.18 pin. Program: V-11's fragment-aware, tuple-gated `eg_decode_frag` (lenient), TCX egress with `LinkOrder::first()` as production does.

Topology: every peer netns holds `10.17.200.1/32`; the host main table has no route to it, so only policy routing selects egress (source rules 1001–1006, fwmark rule `0x173`). Interfaces at boot: `lo`; veth `v17-h1` (1500, source rule), `v17-h2` (1280), `v17-h3` (9000, fwmark); stacked `v17-mv4` (macvlan over `v17-h4`), `v17-h5.50` (VLAN over `v17-h5`), `v17-br6` (bridge, port `v17-h6`); pair `v17-cli`/`v17-bk`; dummy `v17-dum0`.

## (a) HOLDS

Control (no program): every frame-kept datagram arrived 8 bytes longer starting `ZUD1`. Boot attach: all 13 interfaces carry the one program (`boot: ALL 13 interfaces attached`). 46 sizes per interface (0…65,499 incl. MTU boundaries), variants A kept-frame, B bare registered, C frame-shaped (escape), D unregistered, E unconnected `sendto` — identical on both machines:
```
egress lo     A 17/17 B 16/16 C 12/12 D 17/17 E 17/17
egress v17-h1 A 46/46 B 45/45 C 41/41 D 46/46 E 46/46  frag_first=95  frag_shift=355 err=0
egress v17-h2 A 46/46 B 45/45 C 41/41 D 46/46 E 46/46  frag_first=101 frag_shift=422 err=0
egress v17-h3 A 46/46 B 45/45 C 41/41 D 46/46 E 46/46  frag_first=86  frag_shift=170 err=0
```

## (b) Refusals observed and mapped

| # | Provocation | Result (both machines) | Maps to |
|---|---|---|---|
| R1 | same program attached twice to one interface | `EEXIST` (`bpf_mprog_attach`) | `Attach` |
| R1b | a different load of the same program | OK | — |
| R2 | interface deleted between enumeration and attach | `ENODEV` (aya name→ifindex) | `Attach` |
| R3 | capabilities dropped after load, then attach | OK, no `EPERM` (a load without caps fails) | — |
| R4 | `RLIMIT_NOFILE` exhausted | metal `EMFILE`; Lima stale `errno 2`; both from aya name resolution | `Attach` |
| R5 | fill the TCX egress set | 63 accepted, 64th `ERANGE` | `Attach` |
| R6 | tun (no L2); interface down | attach OK | — |
| R7 | `ENOMEM` injected (Lima, kprobe `bpf_override_return` on `BPF_LINK_CREATE`) | `ENOMEM` → classified | `KernelMemory` |

R5 at boot: `UnframeAttach refused startup: [(<ifindex>, "v17-h1", ...)]`, others still attached. R7 proves the errno survives aya's `SyscallError` chain into the classification, not the kernel's own allocation failure (no `CONFIG_FAULT_INJECTION`; TCX allocations not memcg-accounted). Kernel source (`kernel/bpf/tcx.c`, kernel.org master): `tcx_link_attach` / `tcx_entry_fetch_or_create` return `-ENOMEM`; `ERANGE`/`EEXIST` from `bpf_mprog_attach` (`BPF_MPROG_MAX` = 64). TCX attach to an L3 device (tun) succeeds though the program reads Ethernet offsets — traffic through it not exercised.

## (c) Stacked interfaces: FALSIFIED

```
[both upper and lower attached (design)] egress v17-mv4 (macvlan over v17-h4)
  A 28/46  FAILURES(18): size 2945.. 4096..: LOST
  B 45/45
  C 0/41   size 8: got 0 bytes want 8; size 100: got 92 want 100
  D 46/46
  E 28/46
```
`v17-h5.50` (VLAN) and `v17-br6` (bridge) identical. Two causes, one skb through two attached egresses:
1. **Frame-shaped payloads (C) — silent corruption:** the upper removes the frame; the payload itself parses as a frame and the lower removes 8 more bytes.
2. **Fragmented datagrams of ≥ 3 fragments — loss:** the upper pass records `(src,dst,id)` and shifts later fragments; the lower pass shifts them again. Two-fragment datagrams survive.

Isolation — only upper attached, or only lower attached: 46/46 45/45 41/41 46/46 46/46 for all three stacks.

Mitigation probe (not a design change): marking `skb->cb[4]` after modification makes macvlan and VLAN correct (`idem_skip=488`) but not bridge (bridge transmit clears `skb->cb`); `skb->tc_index` did not survive; `skb->mark` not tried (policy routing and nft use it). Not a uniform fix.

## Design implications

Scope: an appliance image with a root-namespace interface stacked on another (VLAN, macvlan, bridge, likely bond). Independent interfaces hold. Options:
1. Image requirement: no stacked root-namespace interface (ADR-0068 § 5, A-21/A-31) plus a boot check that refuses startup on one — smallest.
2. Attach exactly one interface per transmit path (leaf-device rule); traffic sent straight to the lower device is unprotected on an upper-only attach.
3. Per-skb idempotence — macvlan/VLAN only, unproven for bridge.

Text to revisit: D5a-IFACE, D5a-IFACE-BOOT, ADR-0165 "Attachment set"/Consequences, A-21, R5-38 (add a stacked-interface scenario). Keying the fragment map per interface would remove cause 2 but not cause 1.

## Limits

Not tested: USO (V-16), real NIC drivers (veth only), bond, IPv6. R7 on Lima aarch64 only. All `v17-` resources cleaned on both hosts.

**Gate recommendation:** PROMOTE (a) and (b); (c) returns D5a-IFACE to DESIGN for the stacked-interface rule.
