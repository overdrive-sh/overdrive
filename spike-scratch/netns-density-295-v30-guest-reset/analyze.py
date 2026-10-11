#!/usr/bin/env python3
"""aggregate evidence/*/result.json (matrix runs) into per-(mode, case) rows"""
import collections, json, pathlib, statistics, sys
EV = pathlib.Path(__file__).parent / "evidence"
rows = collections.defaultdict(list)
for d in sorted(EV.iterdir()):
    f = d / "result.json"
    if not f.exists() or "-sup" in d.name or d.name.startswith("v30-loop"): continue
    r = json.loads(f.read_text())
    if "error" in r: print("ERROR", d.name, r["error"]); continue
    rows[(r["mode"], r["case"])].append(r)


def rng(xs):
    xs = [x for x in xs if x is not None]
    return "-" if not xs else f"{min(xs):.3f}..{max(xs):.3f}"


order = ["stock", "fd-keep", "fd-close"]
cases = ["pid1exit", "sysrq-c", "reboot", "rebootf", "sysrq-b", "triple", "triple-panic", "poweroff"]
print(f"{'mode':9} {'case':13} {'n':>2} {'VMM exited':>10} {'rc':>6} {'exit t(s)':>13} {'2nd incarnation':>15} {'old flows end (t after GO, s)':>60} {'new inc via h2g':>15}")
for m in order:
    for c in cases:
        rs = rows.get((m, c), [])
        if not rs: continue
        ex = [r for r in rs if r["ch_exit_t_s"] is not None]
        rc = collections.Counter(str(r["ch_rc"]) for r in ex)
        inc2 = sum(1 for r in rs if r["second_incarnation_first_hello_s"] is not None)
        cls = collections.Counter(); times = []
        for r in rs:
            for h in r["after"]["held"]:
                if h["incarnation"] == r["before"]["incarnation_1"]:
                    cls[(h["detail"] or h["state"]).replace("recv: ", "")] += 1; times.append(h["t_end_s"])
        h2g_new = sum(1 for r in rs if any(a.get("result") == "connected" and a["greeting"].split()[-1] != r["before"]["incarnation_1"] for a in r["connect_attempts"]))
        print(f"{m:9} {c:13} {len(rs):>2} {len(ex):>7}/{len(rs):<2} {','.join(f'{k}x{v}' for k, v in rc.items()) or '-':>6} {rng([r['ch_exit_t_s'] for r in ex]):>13} {inc2:>10}/{len(rs):<4} {dict(cls)!s:>40} {rng(times):>19} {h2g_new:>10}/{len(rs)}")
print()
print("fd-keep: flows state while the launcher still holds its fd copy (VMM already exited), then after the launcher closes")
for c in cases:
    rs = rows.get(("fd-keep", c), [])
    if not rs: continue
    before = collections.Counter(h["state"] for r in rs for h in r["after"].get("fd_keep_flows_before_launcher_closes", []) if h["incarnation"] == r["before"]["incarnation_1"])
    after = collections.Counter((h["detail"] or h["state"]) for r in rs for h in r["after"].get("fd_keep_flows_after_launcher_closes", []) if h["incarnation"] == r["before"]["incarnation_1"])
    claim = collections.Counter((r["after"].get("claim_cid_while_launcher_holds"), r["after"].get("claim_cid_after_launcher_close")) for r in rs)
    print(f"  {c:13} while launcher holds: {dict(before)}  after launcher close: {dict(after)}  claim(while holds, after close)={dict(claim)}")
print()
print("fd-close: claim after VMM exit:", dict(collections.Counter(r["after"].get("claim_cid_after_vmm_exit") for c in cases for r in rows.get(("fd-close", c), []))))
print("stock: CH rebooting->rebooted window (CH event timestamps, ms):",
      rng([r["ch_rebooting_to_rebooted_s"] * 1000 for c in cases for r in rows.get(("stock", c), []) if r.get("ch_rebooting_to_rebooted_s") is not None]))
print("stock: before-trigger second-claim of the CID (same host):", dict(collections.Counter(r["before"]["claim_attempt_from_host_probe"] for c in cases for r in rows.get(("stock", c), []))))
print("stock: CH holds exactly one /dev/vhost-vsock fd before and after reboot:",
      all(len(r["before"]["ch_vhost_fds"]) == 1 and len(r["after"]["ch_vhost_fds"] or {1: 1}) == 1 for c in cases if c != "poweroff" for r in rows.get(("stock", c), [])))
