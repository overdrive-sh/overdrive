#!/usr/bin/env python3
# Ad-hoc evidence-capture helper for E-2 (not a test, not part of any crate): hold one
# BPF_ENABLE_STATS(BPF_STATS_RUN_TIME) descriptor, which `kernel.bpf_stats_enabled` cannot see.
import ctypes, os, sys, time
libc = ctypes.CDLL(None, use_errno=True)
SYS_BPF, BPF_ENABLE_STATS, BPF_STATS_RUN_TIME = 321, 32, 0  # x86_64
attr = ctypes.c_uint32(BPF_STATS_RUN_TIME)
fd = libc.syscall(SYS_BPF, BPF_ENABLE_STATS, ctypes.byref(attr), ctypes.sizeof(attr))
if fd < 0:
    sys.exit("BPF_ENABLE_STATS failed: errno %d" % ctypes.get_errno())
print("holder pid=%d fd=%d link=%s" % (os.getpid(), fd, os.readlink("/proc/self/fd/%d" % fd)), flush=True)
time.sleep(900)
