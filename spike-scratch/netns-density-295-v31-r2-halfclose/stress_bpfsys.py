#!/usr/bin/env python3
"""Causal stressor for the SHUTDOWN-record-loss hypothesis: N threads, each pinned to one CPU, spin BPF_MAP_UPDATE_ELEM on a private array
map. Each update runs under bpf_disable_instrumentation() (kernel/bpf/syscall.c @ v7.0), i.e. bpf_prog_active>0 on that CPU.
usage: stress_bpfsys.py <stopfile> [threads]"""
import ctypes, os, sys, threading, struct
libc = ctypes.CDLL(None, use_errno=True); SYS_bpf = 321
def bpf(cmd, attr):
    b = ctypes.create_string_buffer(attr, 128)
    return libc.syscall(SYS_bpf, cmd, b, 128)
stop = sys.argv[1]; n = int(sys.argv[2]) if len(sys.argv) > 2 else os.cpu_count()
cnt = [0] * n
def spin(i):
    os.sched_setaffinity(0, {i % os.cpu_count()})
    fd = bpf(0, struct.pack("IIIII", 2, 4, 8, 4, 0))  # BPF_MAP_TYPE_ARRAY
    assert fd >= 0, "map create"
    key = ctypes.create_string_buffer(struct.pack("I", 0)); val = ctypes.create_string_buffer(struct.pack("Q", 1))
    attr = struct.pack("IIQQQ", fd, 0, ctypes.addressof(key), ctypes.addressof(val), 0)
    k = 0
    while True:
        for _ in range(20000):
            libc.syscall(SYS_bpf, 2, ctypes.create_string_buffer(attr, 128), 128)  # 2 = BPF_MAP_UPDATE_ELEM
        cnt[i] += 20000
        if os.path.exists(stop): return
ts = [threading.Thread(target=spin, args=(i,), daemon=True) for i in range(n)]
[t.start() for t in ts]; [t.join() for t in ts]
print("updates", sum(cnt))
