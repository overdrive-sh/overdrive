#!/usr/bin/env python3
"""increment-c: V-29 item 3, the zero-length SK_PASS hazard, on the host (no VM).
Hypothesis: a stream verdict that SK_PASSes a zero-length skb into a psock enqueues a zero-size sk_msg; sk_msg_recvmsg then
returns -EFAULT (skmsg.c:437-446, 489-491) without dequeuing it, so every later recv on that socket returns -EFAULT, including
for data queued behind it (net/ipv4/udp_bpf.c:79-86 routes recvmsg to sk_msg_recvmsg while the psock has data).
Falsification: the recv after the empty datagram returns 0 (an empty datagram) or EAGAIN, and later data is readable.
Control: the same sequence on a UDP socket that is not in a sockmap."""
import json, pathlib, subprocess, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
H.EV.mkdir(exist_ok=False)
H.OUT.mkdir(exist_ok=True)
H.LOG = (H.EV / "run.log").open("a")
import fcntl
lock = open("/tmp/v29-netns-density-295.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
H.sh("uname -a; systemd-detect-virt || true")
ch, obj, vfwd = H.build()
for i in range(2):
    p = subprocess.run([str(vfwd), "hazard", "--obj", str(obj)], capture_output=True, text=True, timeout=120)
    (H.EV / f"hazard-run{i}.json").write_text(p.stdout)
    (H.EV / f"hazard-run{i}.stderr").write_text(p.stderr)
    H.log(f"hazard run {i} rc={p.returncode}")
    for l in p.stdout.splitlines():
        try:
            j = json.loads(l)
        except Exception:
            continue
        for case in j["report"]:
            H.log(f"--- {case['case']}")
            for st in case["steps"]:
                H.log(f"  {st['step']:<28} {st['call']:<14} ret={st['ret']:<4} errno={st['errno']} {st['errno_name']}")
        H.log(f"pass_verdict counters: all={j['pass_all']} zero_length={j['pass_zero']}")
H.log("bpf progs/maps left: " + H.sh("bpftool prog show 2>/dev/null | grep -c pass_verdict || true", quiet=True, check=False).strip())
