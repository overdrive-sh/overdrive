#!/usr/bin/env python3
"""A-19 / V-15: prove cgroup-connect4 ancestor inheritance in a disposable CH guest."""
import json
import pathlib
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
H.SKIP_HOST_OWNER = True


def probes():
    cid = H.CTX["cid"]
    # The stock guest init starts the reference guest owner so the image is
    # identical to the working V-11 configuration. Stop it before the litmus:
    # only the litmus attaches the host-side VIP program in the guest.
    rc, stopped = H.gx(cid, "pid=$(cat /tmp/owner.pid); if [ \"$pid\" -gt 1 ]; then kill \"$pid\"; fi; sleep 1; cat /proc/sys/kernel/osrelease")
    H.record("probe", "guest_owner_stop", {"rc": rc, "output": stopped})
    rc, output = H.gx(
        cid,
        "/vfwd ancestor-vip --obj /vfwd-bpf.o",
        timeout=90,
    )
    H.record("probe", "raw_litmus_output", output)
    H.record("probe", "litmus_exit", rc)
    result = None
    for line in reversed(output.splitlines()):
        try:
            candidate = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(candidate, dict) and ("ok" in candidate or "error" in candidate):
            result = candidate
            break
    H.record("probe", "result", result or {"ok": False, "raw": output})
    if rc != 0 or not result or result.get("ok") is not True:
        raise RuntimeError(f"ancestor-vip litmus failed: rc={rc}, result={result!r}")


H.run_increment(probes, [])
