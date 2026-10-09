#!/usr/bin/env python3
"""increment-a: smoke. Build, boot, hybrid, a handful of V-18 cases, wire capture + reassembly. Fixes the harness."""
import json, pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"


def probes():
    cid = H.CTX["cid"]
    rec = H.record
    rec("substrate", "guest_uname", H.gx(cid, "uname -a; /usr/sbin/ip -br link")[1])
    rec("substrate", "sha_selftest_host", H.sh(f"{H.CTX['vfwd']} sha-selftest"))
    rec("substrate", "sha_selftest_guest", H.gx(cid, "/vfwd sha-selftest")[1])
    rec("substrate", "offhost", P.setup_offhost())
    H.set_modes(P.MODE, ["--unframe", "hybrid", "--egress-if", P.HV])
    rec("programs", "loaded", H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 3000", check=False, quiet=True))
    cap = P.Capture(P.HV, "smoke")
    res, sent, rx = P.run_phase("smoke", ["frame", "rand"], [8, 1464, 1465, 1472, 4096], reps=1)
    path = cap.stop()
    rec("smoke", "judge", res)
    wire = P.parse_pcap(path)
    rec("smoke", "wire", wire)


H.run_increment(probes, P.MODE)
