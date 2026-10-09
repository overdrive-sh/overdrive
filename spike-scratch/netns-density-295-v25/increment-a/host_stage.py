#!/usr/bin/env python3
"""V-25 host-only sub-claims (a) claim on an unowned instance and (b) exclusivity
while any reference is open + release at the last close. No VM. Root on metal."""
import sys; sys.dont_write_bytecode = True
import errno, json, multiprocessing as mp, os, pathlib, signal, socket, struct, subprocess, sys, time
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from common import *

N = int(os.environ.get("V25_N", "1000"))
EV = pathlib.Path(sys.argv[1])
R = {}


def rec(key, value):
    R[key] = value
    (EV / "host-results.json").write_text(json.dumps(R, indent=2, sort_keys=True))
    print(f"{key}: {json.dumps(value)}", flush=True)


def tally(rcs):
    t = {}
    for r in rcs:
        t[ename(r)] = t.get(ename(r), 0) + 1
    return t


def pct(vals):
    v = sorted(vals)
    return {"n": len(v), "min_ms": round(v[0] * 1e3, 3), "p50_ms": round(v[len(v) // 2] * 1e3, 3),
            "p99_ms": round(v[int(len(v) * 0.99)] * 1e3, 3), "max_ms": round(v[-1] * 1e3, 3)} if v else {}


# ---------------------------------------------------------------- (a)
def sub_a():
    rcs = []
    for i in range(N):
        fd = open_dev()
        rcs.append(claim(fd, CID_BASE))      # never SET_OWNER first
        os.close(fd)
    rec("a1_claim_unowned_fresh_instance", {"n": N, "outcomes": tally(rcs), "set_owner_issued": False})

    # invalid CIDs
    inv = {}
    for cid in (0, 1, 2, 0xFFFFFFFF, 1 << 32):
        fd = open_dev(); inv[str(cid)] = ename(claim(fd, cid)); os.close(fd)
    rec("a2_invalid_cids", inv)

    # claim after SET_OWNER also works (ordering is not required by the kernel)
    fd = open_dev(); o = set_owner(fd); c = claim(fd, CID_BASE + 1)
    rec("a3_claim_after_set_owner_same_process", {"set_owner": ename(o), "claim": ename(c)})
    os.close(fd)

    # idempotent re-claim and re-claim of a different CID on the same instance
    fd = open_dev(); first = claim(fd, CID_BASE + 2); same = claim(fd, CID_BASE + 2)
    other = claim(fd, CID_BASE + 3)
    old_free = claim_fresh(CID_BASE + 2)
    if old_free[1] is not None: os.close(old_free[1])
    new_held = claim_fresh(CID_BASE + 3)
    if new_held[1] is not None: os.close(new_held[1])
    rec("a4_reclaim_semantics", {"first": ename(first), "same_cid_again_same_instance": ename(same),
                                 "different_cid_same_instance": ename(other),
                                 "old_cid_claimable_by_other_instance_after_move": ename(old_free[0]),
                                 "new_cid_claimable_by_other_instance": ename(new_held[0])})
    os.close(fd)

    # ten distinct CIDs held at once on ten instances: each is exclusive
    held = []
    for k in range(10):
        fd = open_dev(); assert claim(fd, CID_BASE + k) == 0; held.append(fd)
    cross = []
    for k in range(10):
        r, f = claim_fresh(CID_BASE + k)
        cross.append(r)
        if f is not None: os.close(f)
    rec("a5_ten_cids_held_each_exclusive", tally(cross))
    for fd in held: os.close(fd)

    # a failed claim does not poison the instance
    a = open_dev(); claim(a, CID_BASE); b = open_dev(); fail = claim(b, CID_BASE)
    ok = claim(b, CID_BASE + 1)
    os.close(a); os.close(b)
    rec("a6_failed_claim_does_not_poison", {"conflicting_claim": ename(fail), "next_cid_on_same_instance": ename(ok)})

    # open(/proc/self/fd/N) is a NEW instance, not a shared reference
    a = open_dev(); claim(a, CID_BASE)
    r = os.open(f"/proc/self/fd/{a}", os.O_RDWR)
    rc = claim(r, CID_BASE)
    os.close(a)
    rc2 = claim(r, CID_BASE)
    os.close(r)
    rec("a7_proc_self_fd_reopen_is_new_instance", {"claim_same_cid_while_original_open": ename(rc),
                                                  "claim_same_cid_after_original_closed": ename(rc2)})


# ---------------------------------------------------------------- (b)
def check_held_then_released(hold_rc_list, release_lat, release_rc_list, name, extra=None):
    out = {"n": len(hold_rc_list), "claim_while_held": tally(hold_rc_list),
           "claim_after_last_close": tally(release_rc_list), "release_latency": pct(release_lat)}
    if extra: out.update(extra)
    rec(name, out)


def b1_parent_alone():
    held, lat, rel = [], [], []
    for i in range(N):
        c = CID_BASE + i % 10
        a = open_dev(); assert claim(a, c) == 0
        r, f = claim_fresh(c); held.append(r)
        if f is not None: os.close(f)
        os.close(a)
        l, n, f = claim_poll(c); lat.append(l if l is not None else 9.9); rel.append(0 if f is not None else errno.EADDRINUSE)
        if f is not None: os.close(f)
    check_held_then_released(held, lat, rel, "b1_parent_alone")


def b2_dup():
    held, lat, rel = [], [], []
    for i in range(N):
        c = CID_BASE + i % 10
        a = open_dev(); assert claim(a, c) == 0
        d = os.dup(a); os.close(a)
        r, f = claim_fresh(c); held.append(r)
        if f is not None: os.close(f)
        os.close(d)
        l, n, f = claim_poll(c); lat.append(l if l is not None else 9.9); rel.append(0 if f is not None else errno.EADDRINUSE)
        if f is not None: os.close(f)
    check_held_then_released(held, lat, rel, "b2_dup_then_close_original")


def b3_fork_no_exec():
    held_p, held_c, lat, rel = [], [], [], []
    for i in range(N):
        c = CID_BASE + i % 10
        a = open_dev(); assert claim(a, c) == 0
        pid = os.fork()
        if pid == 0:
            signal.pause(); os._exit(0)
        r, f = claim_fresh(c); held_p.append(r)          # parent + child both hold
        if f is not None: os.close(f)
        os.close(a)                                       # parent drops its copy; child alone holds
        r, f = claim_fresh(c); held_c.append(r)
        if f is not None: os.close(f)
        os.kill(pid, signal.SIGKILL); os.waitpid(pid, 0)
        l, n, f = claim_poll(c); lat.append(l if l is not None else 9.9); rel.append(0 if f is not None else errno.EADDRINUSE)
        if f is not None: os.close(f)
    rec("b3_fork_without_exec", {"n": N, "claim_while_parent_and_child_hold": tally(held_p),
                                 "claim_while_child_alone_holds": tally(held_c),
                                 "claim_after_child_killed_and_reaped": tally(rel), "release_latency": pct(lat)})


def b4_exec_inherited():
    held_p, held_c, lat_reaped, lat_unreaped, rel = [], [], [], [], []
    insp = None
    for i in range(N):
        c = CID_BASE + i % 10
        a = open_dev(); assert claim(a, c) == 0
        os.set_inheritable(a, True)
        p = subprocess.Popen(["sleep", "300"], pass_fds=[a], close_fds=True)
        if i == 0:
            fds = {f: os.readlink(f"/proc/{p.pid}/fd/{f}") for f in os.listdir(f"/proc/{p.pid}/fd")}
            insp = fds
        r, f = claim_fresh(c); held_p.append(r)
        if f is not None: os.close(f)
        os.close(a)
        r, f = claim_fresh(c); held_c.append(r)
        if f is not None: os.close(f)
        if i % 2 == 0:       # measure from SIGKILL send, before the zombie is reaped
            t0 = time.perf_counter(); p.send_signal(signal.SIGKILL)
            l, n, f = claim_poll(c); lat_unreaped.append(l if l is not None else 9.9)
            rel.append(0 if f is not None else errno.EADDRINUSE)
            p.wait()
        else:                # reaped first, then claim
            p.send_signal(signal.SIGKILL); p.wait()
            l, n, f = claim_poll(c); lat_reaped.append(l if l is not None else 9.9)
            rel.append(0 if f is not None else errno.EADDRINUSE)
        if f is not None: os.close(f)
    rec("b4_exec_inherited_fd", {"n": N, "child_proc_fd_example": insp,
                                 "claim_while_parent_and_child_hold": tally(held_p),
                                 "claim_while_child_alone_holds": tally(held_c),
                                 "claim_after_child_killed": tally(rel),
                                 "latency_from_sigkill_before_reap": pct(lat_unreaped),
                                 "latency_after_reap": pct(lat_reaped)})


def b5_scm_rights():
    held, lat, rel = [], [], []
    for i in range(N // 2):
        c = CID_BASE + i % 10
        a = open_dev(); assert claim(a, c) == 0
        s1, s2 = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        pid = os.fork()
        if pid == 0:
            s1.close()
            msg, anc, fl, ad = s2.recvmsg(1, socket.CMSG_LEN(4))
            s2.send(b"k"); signal.pause(); os._exit(0)
        s2.close()
        s1.sendmsg([b"x"], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, struct.pack("i", a))])
        s1.recv(1); os.close(a)                            # child alone holds, via SCM_RIGHTS
        r, f = claim_fresh(c); held.append(r)
        if f is not None: os.close(f)
        os.kill(pid, signal.SIGKILL); os.waitpid(pid, 0); s1.close()
        l, n, f = claim_poll(c); lat.append(l if l is not None else 9.9); rel.append(0 if f is not None else errno.EADDRINUSE)
        if f is not None: os.close(f)
    check_held_then_released(held, lat, rel, "b5_scm_rights_to_peer")


def b6_cloexec_not_inherited():
    after_parent_close = []
    for i in range(N // 5):
        c = CID_BASE + i % 10
        a = open_dev(cloexec=True); assert claim(a, c) == 0     # CLOEXEC, not passed
        p = subprocess.Popen(["sleep", "300"], close_fds=True)   # exec'd child does NOT get it
        os.close(a)
        r, f = claim_fresh(c); after_parent_close.append(r)     # free although child alive
        if f is not None: os.close(f)
        p.kill(); p.wait()
    rec("b6_cloexec_fd_not_carried_across_exec", {"n": N // 5, "claim_after_parent_close_child_alive": tally(after_parent_close)})


def _racer(rd, q, bar, bar2):
    for rnd in range(rd):
        fd = open_dev()
        bar.wait()
        rc = claim(fd, CID_BASE + 4)
        q.put((rnd, rc))
        bar2.wait()
        os.close(fd)
        bar2.wait()


def b7_race():
    W, ROUNDS = 8, 200
    ctx = mp.get_context("fork")
    q = ctx.Queue(); bar = ctx.Barrier(W); bar2 = ctx.Barrier(W)
    ps = [ctx.Process(target=_racer, args=(ROUNDS, q, bar, bar2)) for _ in range(W)]
    [p.start() for p in ps]
    res = {}
    for _ in range(W * ROUNDS):
        rnd, rc = q.get(); res.setdefault(rnd, []).append(rc)
    [p.join() for p in ps]
    winners = [sum(1 for r in v if r == 0) for v in res.values()]
    others = tally([r for v in res.values() for r in v if r != 0])
    rec("b7_race_8_processes_one_cid", {"rounds": ROUNDS, "workers": W, "rounds_with_exactly_one_winner": sum(1 for w in winners if w == 1),
                                        "winner_count_histogram": {str(k): winners.count(k) for k in set(winners)}, "losers": others})


# ---------------------------------------------------------------- main
if __name__ == "__main__":
    EV.mkdir(parents=True, exist_ok=True)
    for name, fn in [("a", sub_a), ("b1", b1_parent_alone), ("b2", b2_dup), ("b3", b3_fork_no_exec),
                     ("b4", b4_exec_inherited), ("b5", b5_scm_rights), ("b6", b6_cloexec_not_inherited), ("b7", b7_race)]:
        try:
            fn()
        except Exception as e:
            import traceback
            rec(f"{name}_EXCEPTION", traceback.format_exc())
