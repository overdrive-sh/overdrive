#!/usr/bin/env python3
"""Print the kernel struct member byte offsets the v31 BPF programs read (from the running kernel's BTF, via bpftool)."""
import re, subprocess, sys, json
def raw(path):
    return subprocess.run(["bpftool", "btf", "dump", "file", path, "format", "raw"], capture_output=True, text=True).stdout
def member(text, struct, name):
    m = re.search(r"^\[\d+\] STRUCT '%s' size=\d+ vlen=\d+\n((?:\t.*\n)+)" % re.escape(struct), text, re.M)
    if not m: return None
    for l in m.group(1).splitlines():
        mm = re.match(r"\t'(\w+)' type_id=\d+ bits_offset=(\d+)", l)
        if mm and mm.group(1) == name:
            assert int(mm.group(2)) % 8 == 0
            return int(mm.group(2)) // 8
    return None
vm = raw("/sys/kernel/btf/vmlinux"); vs = raw("/sys/kernel/btf/vsock"); vc = raw("/sys/kernel/btf/vmw_vsock_virtio_transport_common")
o = {"tcp_write_seq": member(vm, "tcp_sock", "write_seq"), "file_private": member(vm, "file", "private_data"),
     "socket_sk": member(vm, "socket", "sk"), "sk_cookie": member(vm, "sock_common", "skc_cookie"),
     "vsk_trans": member(vs, "vsock_sock", "trans"), "vvs_tx_cnt": member(vc, "virtio_vsock_sock", "tx_cnt")}
print(json.dumps(o))
if any(v is None for v in o.values()): sys.exit(1)
