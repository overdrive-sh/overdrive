#!/usr/bin/env python3
"""increment-a: build + boot + smoke (one connected cycle per cell policy). Development increment."""
import pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import v5c as V

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"


def probes():
    cid = H.CTX["cid"]
    H.record("substrate", "guest_uname", H.gx(cid, "uname -a; cat /proc/modules | cut -d' ' -f1 | tr '\\n' ' '")[1])
    for cells in ("none", "recreate"):
        def fn():
            x = V.dest(f"smoke-{cells}", "X")
            rc, ev, raw = V.app(f"udp-cycle --dst {V.addr('X')} --connect --cycles 3 --gap-ms 300 --tag S --size 64")
            V.stop_dest(x)
            return {"app": ev, "rec": V.rec_entries(x)}
        V.scenario(f"smoke-{cells}", cells, fn, host_extra=["--abort-dst", V.addr("X"), "--abort-after-ms", "80"])


H.run_increment(probes, V.MODE + ["--cells", "none"])
