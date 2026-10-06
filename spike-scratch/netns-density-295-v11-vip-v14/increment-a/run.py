#!/usr/bin/env python3
"""increment-a: read-only host inventory before building anything.
Questions: which interface carries off-host egress and its MTU; are the planned
fixture ranges (10.250.95.0/24 netns peer, 10.251.0.0/24 mock VIPs) unused; are the
V-14 hook targets (inet_csk_listen_start / inet_csk_listen_stop, sock:inet_sock_set_state)
present in this kernel's BTF/tracefs; tool availability. Changes nothing."""
import json, pathlib, sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
H.EV.mkdir()
H.LOG = (H.EV / "run.log").open("a")
sh = H.sh
H.record("inv", "uname", sh("uname -a; systemd-detect-virt || true; grep -m1 'model name' /proc/cpuinfo; nproc"))
H.record("inv", "links", sh("ip -d -br link; ip -br -4 addr; ip -4 route; ip rule", check=False))
H.record("inv", "route_get", sh("ip -4 route get 1.1.1.1; ip -4 route get 10.250.95.2; ip -4 route get 10.251.0.10", check=False))
H.record("inv", "ranges_in_use", sh("ip -4 addr | grep -E '10\\.25[01]\\.' || echo none; ip -4 route | grep -E '10\\.25[01]\\.' || echo none; ip netns list", check=False))
H.record("inv", "btf_symbols", sh("bpftool btf dump file /sys/kernel/btf/vmlinux format raw 2>/dev/null | grep -E \"FUNC '(inet_csk_listen_start|inet_csk_listen_stop|skb_send_sock|inet_sk_state_store|tcp_set_state)'\" ; "
                                  "grep -E ' (inet_csk_listen_start|inet_csk_listen_stop)$' /proc/kallsyms | head; "
                                  "cat /sys/kernel/tracing/events/sock/inet_sock_set_state/format 2>/dev/null | head -30", check=False))
H.record("inv", "tools", sh("which tcpdump bpftool strace ethtool nft ip; tcpdump --version 2>&1 | head -1; bpftool version", check=False))
H.record("inv", "sysctls", sh("sysctl kernel.bpf_stats_enabled net.ipv4.ip_forward net.ipv4.conf.all.rp_filter kernel.unprivileged_bpf_disabled", check=False))
dev = sh("ip -4 route get 1.1.1.1 | sed -n 's/.* dev \\([^ ]*\\).*/\\1/p'", check=False).strip()
H.record("inv", "egress_dev", dev)
H.record("inv", "egress_dev_detail", sh(f"ip -d link show {dev}; ethtool -k {dev} 2>/dev/null | grep -E 'segmentation|checksum|scatter|udp' ; bpftool net show dev {dev}; tc qdisc show dev {dev}", check=False))
H.record("inv", "tcx_all", sh("bpftool net show 2>&1 | head -60", check=False))
H.record("inv", "cgroup_root", sh("bpftool cgroup show /sys/fs/cgroup 2>&1; ls /sys/fs/cgroup | head -50", check=False))
H.record("inv", "other_spike_activity", sh("pgrep -af 'cloud-hypervisor|vfwd' | grep -v pgrep || echo none; ls /tmp/*.lock 2>/dev/null", check=False))
H.log("=== done")
