#!/usr/bin/env python3
"""increment-a: development smoke for V-29 (not evidence). Boots the VM, sets the host owner to verdict-notify, sends 50
empties and 50 data+empty pairs, prints counters. Findings: docs/feature/netns-density-295/spike/findings-v29-udp-empty.md"""
import json, pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P
import p29 as Q

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"


def probes():
    cid = H.CTX["cid"]
    H.record("substrate", "guest_uname", H.gx(cid, "uname -r")[1].strip())
    H.record("substrate", "offhost", P.setup_offhost())
    H.set_modes(P.MODE, Q.VERDICT_NOTIFY)
    H.record("programs", "loaded", H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 2500", check=False, quiet=True))
    H.record("programs", "tcx_on_host", H.sh("bpftool net show 2>&1 | head -40", check=False, quiet=True))
    Q.phase_sizes("smoke_empties", [0], reps=50, pcap=True)
    for gap in (500, 100, 20, 0):
        Q.phase_seq(f"sweep_k1_gap{gap}", 2000, 1400, 1, gap_us=gap)
    Q.phase_seq("sweep_datanly_gap0", 2000, 1400, 0, gap_us=0)


H.run_increment(probes, P.MODE)
