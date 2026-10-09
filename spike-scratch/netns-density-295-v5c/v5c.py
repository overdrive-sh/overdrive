#!/usr/bin/env python3
"""Scenario library for the V-5(c) spike (netns-density-295). Imported by increment-*/run.py
(root, on the native metal host, guest CID 3261, every resource prefixed v5c-).

Topology: one microVM (stock 7.0.0-29 kernel, no NIC, dummy device carrying the workload address),
the host owner and the guest owner (stand-in for overdrive-init) from the V-11 spike plus the V-5(c)
additions, and recording UDP destinations on the host's own address in the private port range 475xx."""
import json, pathlib, re, time
import harness as H

BASE = pathlib.Path(__file__).resolve().parent
MODE = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park"]
PORT = {"X": 47501, "Y": 47502, "Z": 47503}
# Private destination fixture: netns v5c-ns behind veth v5c-h (host side) / v5c-p (ns side). The host owner unframes
# with the fragment-aware TC program on v5c-h only (never on the shared root `lo`).
NS, HV, PV = "v5c-ns", "v5c-h", "v5c-p"
NET = "10.250.205"
HOST_IP, PEER_IP = f"{NET}.1", f"{NET}.2"
BASE_HOST = ["--unframe", "tc-frag", "--egress-if", HV]


def setup_fixture():
    """Create the private fixture; fail closed if the /24 is already routed or addressed by anyone."""
    seen = H.sh("ip -4 route; ip -4 addr; ip netns list", check=False, quiet=True)
    assert NET not in seen and NS not in seen and HV not in seen, "v5c fixture address/name already in use:\n" + seen
    for cmd in [f"ip netns add {NS}", f"ip link add {HV} type veth peer name {PV}", f"ip link set {PV} netns {NS}",
                f"ip addr add {HOST_IP}/24 dev {HV}", f"ip link set {HV} mtu 1500 up",
                f"ip -n {NS} addr add {PEER_IP}/24 dev {PV}", f"ip -n {NS} link set {PV} mtu 1500 up", f"ip -n {NS} link set lo up"]:
        H.sh(cmd)
    H.CLEANUPS.append(teardown_fixture)
    return H.sh(f"ip -d link show {HV}; ip -n {NS} -br addr; ip -n {NS} route", check=False)


def teardown_fixture():
    H.sh(f"ip link del {HV} 2>&1; ip netns del {NS} 2>&1", check=False)



def J(text):
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                out.append(json.loads(line))
            except Exception:
                pass
    return out


def dest(label, name, echo_skip=0, no_echo=False, no_echo_tag=None):
    """Recording UDP destination on the host address."""
    log = H.OUT / f"rec-{H.INC.name}-{label}-{name}.log"
    if log.exists():
        log.unlink()
    argv = ["ip", "netns", "exec", NS, str(H.CTX["vfwd"]), "udp-rec", "--bind", f"{PEER_IP}:{PORT[name]}", "--log", str(log)]
    if echo_skip:
        argv += ["--echo-skip", str(echo_skip)]
    if no_echo:
        argv += ["--no-echo"]
    if no_echo_tag:
        argv += ["--no-echo-tag", no_echo_tag]
    p = H.spawn(f"rec-{label}-{name}", argv)
    p.reclog = log
    time.sleep(0.3)
    return p


def addr(name):
    return f"{PEER_IP}:{PORT[name]}"


def rec_entries(p):
    out = []
    if p.reclog.exists():
        for line in p.reclog.read_text().splitlines():
            try:
                out.append(json.loads(line))
            except Exception:
                pass
    return out


def stop_dest(p):
    H.stop(p)
    if p.reclog.exists():
        (H.EV / p.reclog.name.replace(f"rec-{H.INC.name}-", "rec-")).write_text(p.reclog.read_text())


def app(cmd, timeout=240, quiet=True):
    """Run a guest test application; returns (rc, [json lines], raw)."""
    rc, t = H.gx(H.CTX["cid"], f"/vfwd {cmd}", timeout=timeout, quiet=quiet)
    return rc, J(t), t


def glog(label):
    """Guest owner log (JSON events) saved verbatim into evidence; returns parsed events."""
    t = H.gsave("/tmp/owner.log", f"guest-owner-{label}.log")
    H.gsave("/tmp/owner-stats.json", f"guest-stats-{label}.json")
    return J(t)


def hlog(label):
    p = H.CTX.get("host_owner")
    time.sleep(0.2)
    if p and p.owner_log.exists():
        txt = p.owner_log.read_text()
        (H.EV / f"host-owner-{label}.log").write_text(txt)
        return J(txt)
    return []


def count(events, ev, **kw):
    return sum(1 for e in events if e.get("ev") == ev and all(e.get(k) == v for k, v in kw.items()))


def tags(entries):
    """Application tag + sequence of every datagram a destination received (A000003 -> 'A000003')."""
    return [e["tag"] for e in entries]


def scenario(label, cells, fn, host_extra=None, guest_extra=None):
    """Restart both owners with the given cell policy, run fn, save both owner logs."""
    mode = MODE + ["--cells", cells, "--order", ("old" if cells == "none" else "cutfirst")] + (guest_extra or [])
    H.log(f"=== scenario {label} cells={cells}")
    H.set_modes(mode, BASE_HOST + (host_extra or []))
    r = {"cells": cells, "host_extra": host_extra or [], "guest_extra": guest_extra or [], "t_start": time.time()}
    try:
        r.update(fn() or {})
    except Exception as e:
        import traceback
        r["exception"] = traceback.format_exc()
        H.log("EXCEPTION " + r["exception"])
    ge = glog(label)
    he = hlog(label)
    r["guest_events"] = {k: count(ge, k) for k in ("udp_associate", "udp_paired", "udp_teardown", "cells_cleared", "refused", "abort")}
    r["host_events"] = {k: count(he, k) for k in ("host_dg_installed", "host_dg_teardown", "fault_refused", "fault_abort_sent", "fault_partial_frame")}
    H.record("scenario", f"{label}/{cells}", r)
    return r, ge, he


def save_raw(label, name, raw):
    (H.EV / f"app-{label}-{name}.jsonl").write_text(raw)


def summary_of(events, ev):
    xs = [e for e in events if e.get("ev") == ev]
    return xs[-1] if xs else None
