# netns-density-295: off-host guest UDP (V-11, D5a), guest → service VIP, listen-state mirroring (V-14, D23)

Throwaway spike. Findings: `docs/feature/netns-density-295/spike/v11-vip-v14-findings.md`.

The spike starts from a copy of `spike-scratch/netns-density-295-guest-vsock-capture/`
(bpf, vfwd, guest init, harness, launcher) and extends it. On the metal host it reuses that
spike's Cloud Hypervisor build and cargo cache (`harness.PRIOR_OUT`).

## Files

- `bpf/src/main.rs`: the Aya eBPF ELF, loaded by both sides. Additions over the copy:
  - forward modes 3/5/6 (unframe in the host verdict, D5a b / hybrid);
  - `eg_decode_frag` (fragment-aware tuple-gated egress unframe, D5a a2), plus a lenient
    tuple mode for hybrid;
  - `vip_connect4` (mirror of production `cgroup_connect4_service`, ADR-0053 key/value layout);
  - `listen_start`/`listen_stop`: `fexit` on `inet_csk_listen_start`/`inet_csk_listen_stop`.
    They maintain the `LISTENERS` map (cookie → port) and write wake hints to the `LEV`
    ring buffer.
- `vfwd/src/owner.rs`: owners. Additions:
  - `--unframe tc|tc-frag|verdict|verdict-naive|hybrid`, `--egress-if`;
  - `--vip VIP:PORT/proto=BACKEND:PORT`;
  - `--tuple-from peer|requested`;
  - V-14: `--declare` (guest), `--d23 hostaddr=gport` (host),
    `--listen-mode map|event|level`, `ListenState` control message, guest control-session
    reconnect, `sock_diag` seeding;
  - fault injection `--pair-delay-ms/--pair-delay-dst`;
  - `RLIMIT_NOFILE` 65,536.
- `vfwd/src/main.rs`: test applications. Added `listen-cycle`, `listen-churn`, `udp-send`,
  `udp-framelike`, and `udp-server --mark-len`.
- `guest/init.sh`: guest PID 1. Adds `net.ipv4.fwmark_reflect=1`.
- `harness.py`: build, guest image (adds `tcp_diag` + `inet_diag`), VM, owners, cleanup
  registry, `gx_stream`, and inventories (TCX, netns, routes, `bpf_stats_enabled`).
- `probes.py`: netns/veth off-host fixture, TCX cost, NIC/veth `tcpdump` captures,
  matrices, strace accounting, and V-14 probe helpers.
- `launch.py <increment>`: holds the canonical metal lease (`cargo xtask metal run`), streams
  the redacted log, and retrieves `evidence/`.

| Increment | Role |
|---|---|
| a | read-only host inventory |
| b–d | development: found the `tc` fragmentation loss, the verdict-naive EPIPE, the hybrid escape rule, the `fwmark_reflect`/EMFILE stall, and the `/proc` enumeration cost |
| e | pre-registered evidence: V-11 (all options + missing-TC), VIP. Its V-14 section is invalid (no diag modules in the guest) |
| f | slot-recycle hypothesis (fault-injected), untraced first-datagram stresses, V-14 level mode |
| g | V-14 stale-dump check (level), event mode |
| h | V-14 level mode (uninstrumented, after strace), event-mode churn control, **map mode** (final) |

Every run restored modules, links, TCX, root cgroup BPF, netns, routes and
`bpf_stats_enabled` (see `cleanup` in each `results.json`).
