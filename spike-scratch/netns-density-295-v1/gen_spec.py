#!/usr/bin/env python3
"""Generate the V-1 sweep specs (cells run one after another in ONE microVM boot). usage: gen_spec.py pilot|A|B|C"""
import json, sys
mode = sys.argv[1]
K = 1024
def conc(sz): return 50 if sz <= 1024*K else 25 if sz <= 4096*K else 8
common = ["--arm", "owner", "--flows", "256", "--hold-ms", "7000", "--settle-ms", "2500", "--io-timeout", "40", "--events-sample", "1", "--diag-seq", "--fin", "mix"]
cells = []
def pair(sz, extra=(), tag="", T=100, pre=None, c=None):
    c = c or conc(sz)
    g = {"label": f"G{tag}-{sz}", "args": ["--n", "0", "--m", str(sz), "--srv-pause", "0", "--cli-pause", "never", "--trials", str(T), "--conc", str(c), *common, *extra]}
    h = {"label": f"H{tag}-{sz}", "args": ["--n", str(sz), "--m", "0", "--srv-pause", "never", "--cli-pause", "0", "--trials", str(T), "--conc", str(c), *common, *extra]}
    if pre: g["guest_pre"] = pre; h["guest_pre"] = pre
    return [g, h]
if mode == "pilot":
    for sz in (4096*K,): cells += pair(sz, T=3, c=3)
    pre = "ip link set lo mtu 1500; echo lo_mtu=$(cat /sys/class/net/lo/mtu)"
    for sz in (64*K, 256*K, 1024*K): cells += pair(sz, extra=["--dest-mss", "1448"], tag="m1500", pre=pre, T=3, c=3)
elif mode == "A":   # default stack: guest lo MTU 65536, host dest MSS unclamped
    for sz in [4*K, 16*K, 64*K, 96*K, 128*K, 192*K, 256*K, 512*K, 1024*K, 2048*K, 2432*K, 2560*K, 2688*K, 2816*K, 3072*K, 4096*K, 16384*K]:
        cells += pair(sz)
elif mode == "B":   # 1500-MTU path: guest lo mtu 1500, host destination leg TCP_MAXSEG 1448
    pre = "ip link set lo mtu 1500; echo lo_mtu=$(cat /sys/class/net/lo/mtu)"
    for sz in [4*K, 16*K, 64*K, 96*K, 128*K, 160*K, 192*K, 224*K, 256*K, 320*K, 384*K, 512*K, 1024*K, 4096*K]:
        cells += pair(sz, extra=["--dest-mss", "1448"], tag="m1500", pre=pre)
elif mode == "C":   # controls (readers) and the application receive-buffer sweep at an 8 MiB tail
    for sz in (1024*K, 16384*K):
        cells.append({"label": f"C-read-{sz}", "args": ["--n", str(sz), "--m", str(sz), "--srv-pause", "0", "--cli-pause", "0", "--trials", "100", "--conc", str(conc(sz)), *common]})
    for rb in (16*K, 128*K, 1024*K, 4096*K):
        cells += pair(8192*K, extra=["--app-rcvbuf", str(rb)], tag=f"rb{rb}", T=50, c=10)
print(json.dumps(cells, indent=1))
