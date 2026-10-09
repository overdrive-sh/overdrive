#!/usr/bin/env python3
"""V-22 spike (netns-density-295): dead-VM vsock connection reset before CID reuse.

Native metal run, root, under the canonical lease:
  cargo xtask metal run -- python3 spike-scratch/netns-density-295-v22/increment-a/run.py [ROUNDS N]
Host listeners (AF_VSOCK, CID_ANY) on the design's ports 1234 1240 1241 1243.
Guest agent connects to host CID 2 from chosen local ports.
"""
import collections, errno, fcntl, hashlib, json, os, pathlib, re, resource, select, shutil, signal, socket, subprocess, sys, time

INC = pathlib.Path(__file__).resolve().parent
BASE = INC.parent
ROOT = BASE.parent.parent
EV = INC / "evidence"
OUT = BASE / "out" / INC.name
EV.mkdir(exist_ok=False)
OUT.mkdir(parents=True, exist_ok=True)
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
ROUNDS = int(sys.argv[1]) if len(sys.argv) > 1 else 4
N = int(sys.argv[2]) if len(sys.argv) > 2 else 250
PORTS = [1234, 1240, 1241, 1243]
CID = 3220          # VM under test (killed, then relaunched with the same CID)
CID_BYSTANDER = 3221
CTL = 4000
OLD_PREV = pathlib.Path(ROOT / "spike-scratch/netns-density-295-ch-vhost-vsock/out")

lock = open("/tmp/netns-density-295-v22.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
resource.setrlimit(resource.RLIMIT_NOFILE, (65536, 65536))
LOG = (EV / "run.log").open("a")
RESULTS = {}


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    LOG.write(line + "\n"); LOG.flush()


def save():
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))


def sh(cmd, check=True, timeout=600, env=None, quiet=False):
    t0 = time.time()
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, timeout=timeout, env=env)
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": p.returncode, "out": p.stdout[-20000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(map(str, cmd))} (rc={p.returncode})")
        for line in p.stdout.rstrip().splitlines()[-40:]:
            log("    " + line)
    if check and p.returncode != 0:
        raise RuntimeError(f"rc={p.returncode}: {cmd}")
    return p.stdout


def sha(p):
    return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


def modules():
    return sorted(l.split()[0] for l in pathlib.Path("/proc/modules").read_text().splitlines())


# ------------------------------------------------------------- preflight (fail closed)
log("=== preflight")
assert os.uname().machine == "x86_64"
virt = sh("systemd-detect-virt || true", check=False).strip()
assert virt == "none", f"virtualized host: {virt}"
assert os.path.exists("/dev/kvm")
sh("uname -a; grep -m1 'model name' /proc/cpuinfo; nproc; free -m; ls -l /dev/kvm /dev/vhost-vsock || true")
MODULES_BEFORE = modules()
(EV / "modules-before.txt").write_text("\n".join(MODULES_BEFORE) + "\n")
RESULTS["env"] = {"uname_r": os.uname().release, "virt": virt, "rounds": ROUNDS, "n_per_port_per_round": N,
                  "ports": PORTS, "cid": CID, "cid_bystander": CID_BYSTANDER}
pre = sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False)
RESULTS["env"]["other_ch_processes_at_start"] = pre.strip()
save()

# ------------------------------------------------------------- build
log("=== build")
env = os.environ.copy()
env["CARGO_HOME"] = str(OUT / "cargo-home")
env["CARGO_TARGET_DIR"] = str(OUT / "ch-target")
prev_home, prev_target = OLD_PREV / "cargo-home", OLD_PREV / "ch-target"
if prev_home.exists() and not (OUT / "cargo-home").exists():
    sh(["cp", "-a", str(prev_home), str(OUT / "cargo-home")])        # read-only copy of a cache
if prev_target.exists() and not (OUT / "ch-target").exists():
    sh(["cp", "-a", str(prev_target), str(OUT / "ch-target")])
sh(["cargo", "build", "--release", "--locked", "--manifest-path", str(ROOT / "vendors/cloud-hypervisor/Cargo.toml"),
    "--bin", "cloud-hypervisor"], env=env, timeout=3600)
CH = str(OUT / "ch-target/release/cloud-hypervisor")
penv = env.copy()
penv["CARGO_TARGET_DIR"] = str(OUT / "probe-target")
penv["RUSTFLAGS"] = "-C target-feature=+crt-static"
sh(["cargo", "build", "--release", "--target", "x86_64-unknown-linux-gnu", "--manifest-path",
    str(BASE / "probe/Cargo.toml")], env=penv, timeout=900)
PROBE = str(OUT / "probe-target/x86_64-unknown-linux-gnu/release/v22-probe")
sh(f"file {PROBE} {CH}; {CH} --version")

root = OUT / "initramfs-root"
if root.exists():
    shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]:
    (root / d).mkdir(parents=True)
bb = shutil.which("busybox")
shutil.copy2(bb, root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "cut"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(PROBE, root / "probe")
shutil.copy2(BASE / "guest-init.sh", root / "init"); (root / "init").chmod(0o755)
pins = {}
for line in sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"]).splitlines():
    if line.startswith("insmod "):
        src = pathlib.Path(line.split()[1])
        data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
        (root / src.name.removesuffix(".zst")).write_bytes(data)
INITRD = OUT / "initramfs.cpio.gz"
sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -9 > {INITRD}")
pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(INITRD), "ch": sha(CH), "probe": sha(PROBE)})
(EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))
RESULTS["pins"] = pins
save()

# ------------------------------------------------------------- VM + guest control
VMS = []


class VM:
    def __init__(self, name, cid):
        self.name, self.cid = name, cid
        self.dir = OUT / f"vm-{name}-{int(time.time() * 1000)}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmd = [CH, "--kernel", GUEST_KERNEL, "--initramfs", str(INITRD),
               "--cmdline", "console=ttyS0 panic=-1 loglevel=4", "--cpus", "boot=1", "--memory", "size=512M",
               "--serial", f"file={self.serial}", "--console", "off", "--log-file", str(self.dir / "ch.log"),
               "--vsock", f"cid={cid},backend=vhost-kernel"]
        log(f"launch {name}: {' '.join(cmd)}")
        self.out = (self.dir / "ch.stdout").open("w")
        self.t_launch = time.time()
        self.proc = subprocess.Popen(cmd, stdout=self.out, stderr=subprocess.STDOUT)
        self.pid = self.proc.pid
        VMS.append(self)

    def ready(self, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            if self.proc.poll() is not None:
                raise RuntimeError(f"{self.name} exited early rc={self.proc.returncode}")
            try:
                r = ctl(self.cid, "ident", timeout=5)
                if "IDENT" in r:
                    assert f"local_cid={self.cid}" in r, r
                    self.ident = r.splitlines()[0]
                    self.t_ready = time.time()
                    log(f"{self.name} ready after {self.t_ready - self.t_launch:.2f}s: {self.ident}")
                    return self.ident
            except OSError:
                pass
            time.sleep(0.3)
        raise RuntimeError(f"{self.name} not ready")

    def kill9(self):
        t0 = time.time()
        self.proc.send_signal(signal.SIGKILL)
        rc = self.proc.wait()
        return {"rc": rc, "kill_to_reaped_ms": round((time.time() - t0) * 1000, 2), "t_reaped": time.time()}

    def collect(self):
        for f in [self.serial, self.dir / "ch.stdout", self.dir / "ch.log"]:
            if f.exists():
                dst = EV / "vms" / self.dir.name / f.name
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(f, dst)


def ctl(cid, cmd, timeout=60):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect((cid, CTL))
        s.sendall((cmd + "\n").encode())
        buf = b""
        while not buf.endswith(b"END\n"):
            d = s.recv(65536)
            if not d:
                break
            buf += d
        return buf.decode(errors="replace").replace("\nEND\n", "")
    finally:
        s.close()


def ec(e):
    return errno.errorcode.get(e.errno, f"errno{e.errno}") if isinstance(e, OSError) else repr(e)


def probe_dead(s):
    """Non-blocking: first recv then first send on a socket that should be dead."""
    s.setblocking(False)
    r = {}
    try:
        d = s.recv(4096)
        r["recv"] = "EOF" if d == b"" else "DATA:" + d[:40].decode(errors="replace")
    except BlockingIOError:
        r["recv"] = "EAGAIN"
    except OSError as e:
        r["recv"] = ec(e)
    try:
        s.send(b"x", socket.MSG_NOSIGNAL)
        r["send"] = "OK"
    except BlockingIOError:
        r["send"] = "EAGAIN"
    except OSError as e:
        r["send"] = ec(e)
    return r


def ss_snapshot(tag):
    out = sh("ss --vsock -a -n", check=False, quiet=True)
    (EV / f"ss-{tag}.txt").write_text(out)
    cnt = collections.Counter()
    for line in out.splitlines()[1:]:
        f = line.split()
        if len(f) >= 6:
            m = re.match(r"(\d+):(\d+)", f[5])
            if m:
                cnt[(f[1], int(m.group(1)))] += 1
    return {f"{st}/peer{cid}": n for (st, cid), n in sorted(cnt.items())}


def accept_all(listener, idle=2.0, hard=30.0):
    """Accept until the queue stays empty for `idle` seconds."""
    got, t_end, last = [], time.time() + hard, time.time()
    listener.setblocking(False)
    while time.time() < t_end:
        r, _, _ = select.select([listener], [], [], 0.2)
        if r:
            try:
                got.append(listener.accept())
                last = time.time()
                continue
            except BlockingIOError:
                pass
        if time.time() - last > idle:
            break
    return got


def first_read(c, timeout=3.0):
    c.settimeout(timeout)
    try:
        d = c.recv(4096)
        return ("EOF", b"") if d == b"" else ("DATA", d)
    except socket.timeout:
        return ("TIMEOUT", b"")
    except OSError as e:
        return (ec(e), b"")


def base(kind, pidx):
    return 10000 + kind * 4000 + pidx * 500


KE, KC, KCD, KQ = 0, 1, 2, 3
OUTCOMES = (EV / "outcomes.jsonl").open("a")


def out(**kw):
    OUTCOMES.write(json.dumps(kw) + "\n")


T = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))   # T[claim][port][class]
EX = collections.defaultdict(list)                                                  # examples of any non-HOLDS class


def tally(claim, port, cls, example=None, bad=False):
    T[claim][port][cls] += 1
    if bad and len(EX[f"{claim}/{port}/{cls}"]) < 5:
        EX[f"{claim}/{port}/{cls}"].append(example)


def reuse_state_snapshot():
    RESULTS["tallies"] = {c: {str(p): dict(v) for p, v in d.items()} for c, d in T.items()}
    RESULTS["examples"] = dict(EX)
    save()


# ------------------------------------------------------------- scenario
def round_(r, listeners):
    log(f"########## round {r}")
    rr = {"round": r}
    byst = VM(f"r{r}-bystander", CID_BYSTANDER)
    byst.ready()
    a = VM(f"r{r}-old", CID)
    a.ready()
    # bystander: established connection per port, accepted by host
    bsock = []
    for pi, P in enumerate(PORTS):
        assert "ok=3" in ctl(CID_BYSTANDER, f"hold {P} 3 {base(0, pi)} BY"), "bystander hold"
        for c, peer in accept_all(listeners[P], idle=1.0):
            k, d = first_read(c)
            assert k == "DATA" and d.startswith(b"BY-"), (k, d)
            bsock.append((P, c))
    # ---- C1: established, accepted by host before the kill
    E = {}
    for pi, P in enumerate(PORTS):
        rep = ctl(CID, f"hold {P} {N} {base(KE, pi)} ET")
        assert f"ok={N}" in rep, rep
        acc = accept_all(listeners[P], idle=1.0)
        E[P] = []
        for c, peer in acc:
            k, d = first_read(c)
            ok = k == "DATA" and d.startswith(b"ET-") and peer[0] == CID
            tally("C1.setup_live", P, "live" if ok else f"notlive:{k}", (peer, k, d), bad=not ok)
            E[P].append((c, peer))
        assert len(E[P]) == N, f"accepted {len(E[P])} != {N} on {P}"
    # ---- C3: guest closes before the host accepts (VM alive)
    for pi, P in enumerate(PORTS):
        assert f"ok={N}" in ctl(CID, f"close {P} {N} {base(KC, pi)} CN 0")
        assert f"ok={N}" in ctl(CID, f"close {P} {N} {base(KCD, pi)} CD 1")
        time.sleep(0.2)
        acc = accept_all(listeners[P], idle=1.0)
        tally("C3.accepted_count", P, f"accepted={len(acc)}/{2 * N}")
        for c, peer in acc:
            kind = "nodata" if base(KC, pi) <= peer[1] < base(KC, pi) + 500 else "data" if base(KCD, pi) <= peer[1] < base(KCD, pi) + 500 else "other"
            k1, d1 = first_read(c)
            if kind == "nodata":
                good = k1 in ("EOF", "ECONNRESET")
                tally("C3.nodata.first_read", P, k1 if k1 != "DATA" else "DATA:" + d1[:20].decode(errors="replace"), (peer, k1, d1), bad=not good)
            elif kind == "data":
                k2, d2 = first_read(c) if k1 == "DATA" else (None, b"")
                tally("C3.data.first_read_then_second", P, f"{k1}:{d1[:3].decode(errors='replace')}/{k2}", (peer, k1, d1, k2), bad=(k2 not in ("EOF", "ECONNRESET", None)))
            else:
                tally("C3.other", P, "unexpected", (peer, k1, d1), bad=True)
            out(claim="C3", round=r, port=P, peer=peer, kind=kind, first=k1, data=d1[:16].decode(errors="replace"))
            c.close()
    # ---- C2: connected but NOT accepted by host (accept-queue entries)
    for pi, P in enumerate(PORTS):
        rep = ctl(CID, f"hold {P} {N} {base(KQ, pi)} QO")
        assert f"ok={N}" in rep, rep
    time.sleep(0.5)
    rr["ss_before_kill"] = ss_snapshot(f"r{r}-before-kill")
    rr["listener_readable_before_kill"] = {P: bool(select.select([listeners[P]], [], [], 0)[0]) for P in PORTS}
    # ---- kill -9 the VMM
    k = a.kill9()
    rr["kill"] = k
    log(f"killed old VM: {k}")
    # ---- observe BEFORE the CID is reused
    rr["ss_after_kill_before_reuse"] = ss_snapshot(f"r{r}-after-kill")
    for P in PORTS:
        for c, peer in E[P]:
            res = probe_dead(c)
            tally("C1.before_reuse.recv", P, res["recv"], (peer, res), bad=not (res["recv"] in ("EOF",) or res["recv"].startswith("E")) or res["recv"] == "EAGAIN")
            tally("C1.before_reuse.send", P, res["send"], (peer, res), bad=res["send"] in ("OK", "EAGAIN"))
            out(claim="C1", phase="before_reuse", round=r, port=P, peer=peer, **res)
    # ---- reuse the CID
    b = VM(f"r{r}-new", CID)
    b.ready()
    rr["kill_to_new_ready_s"] = round(b.t_ready - k["t_reaped"], 2)
    rr["new_launch_after_reap_ms"] = round((b.t_launch - k["t_reaped"]) * 1000, 1)
    # new VM connects from the SAME guest local ports (same 4-tuples) with distinct markers
    for pi, P in enumerate(PORTS):
        r1 = ctl(CID, f"hold {P} {N} {base(KE, pi)} NE")
        r2 = ctl(CID, f"hold {P} {N} {base(KQ, pi)} NQ")
        tally("new_vm.connect", P, f"{r1.split(' errs')[0]} | {r2.split(' errs')[0]}", (r1, r2), bad=(f"ok={N} fail=0" not in r1 or f"ok={N} fail=0" not in r2))
    rr["ss_after_reuse"] = ss_snapshot(f"r{r}-after-reuse")
    # ---- C1 after reuse: old established sockets
    for P in PORTS:
        for c, peer in E[P]:
            res = probe_dead(c)
            tally("C1.after_reuse.recv", P, res["recv"], (peer, res), bad=not (res["recv"] == "EOF" or res["recv"].startswith("E")) or res["recv"] == "EAGAIN")
            tally("C1.after_reuse.send", P, res["send"], (peer, res), bad=res["send"] in ("OK", "EAGAIN"))
            out(claim="C1", phase="after_reuse", round=r, port=P, peer=peer, **res)
            c.close()
    # ---- C2: accept everything now (old queued entries + new VM's connections)
    for pi, P in enumerate(PORTS):
        acc = accept_all(listeners[P], idle=2.0)
        cls = collections.Counter()
        newmarks = collections.Counter()
        for c, peer in acc:
            k1, d1 = first_read(c, timeout=2.0)
            txt = d1.decode(errors="replace").strip()
            if k1 == "DATA" and txt.startswith(("NE-", "NQ-")):
                cls["fresh_new_vm_connection"] += 1
                newmarks[txt] += 1
                continue
            if k1 == "DATA":
                cls["STALE_DATA:" + txt[:12]] += 1
                tally("C2.stale", P, "STALE_DATA_DELIVERED", (peer, txt), bad=True)
                continue
            # no data: stale/dead entry. Does a write succeed?
            c.setblocking(False)
            try:
                c.send(b"x", socket.MSG_NOSIGNAL); sres = "OK"
            except OSError as e:
                sres = ec(e)
            cls[f"dead_entry first_read={k1} send={sres}"] += 1
            tally("C2.dead_entry.send", P, sres, (peer, k1, sres), bad=(sres == "OK"))
            out(claim="C2", round=r, port=P, peer=peer, first=k1, send=sres)
            c.close()
        for kk, v in cls.items():
            tally("C2.accept_classes", P, kk)
            T["C2.accept_classes"][P][kk] += v - 1
        dup = [m for m, v in newmarks.items() if v != 1]
        missing = 2 * N - sum(newmarks.values())
        tally("C2.new_vm_markers", P, f"fresh={sum(newmarks.values())}/{2 * N} dup_markers={len(dup)} missing={missing}",
              (dup[:3], missing), bad=(len(dup) > 0 or missing != 0))
        tally("C2.accepted_total", P, f"accepted={len(acc)} (new 2N={2 * N}, old queued N={N})")
    # ---- bystander still live
    ctl(CID_BYSTANDER, "tick BYST2")
    for P, c in bsock:
        res = probe_dead(c)
        c.settimeout(2)
        # probe_dead already consumed the tick if present; re-check by liveness (send OK, recv DATA or EAGAIN)
        tally("bystander", P, f"recv={res['recv'][:9]} send={res['send']}", res, bad=(res["send"] != "OK" or res["recv"] == "EOF"))
        c.close()
    # drain + queue-empty check
    for P in PORTS:
        left = accept_all(listeners[P], idle=0.5)
        tally("queue_empty_at_round_end", P, f"left={len(left)}")
        for c, _ in left:
            c.close()
    for v in (VMS[-1], ):
        pass
    for vm in [byst, b]:
        vm.kill9()
    for vm in [byst, a, b]:
        vm.collect()
    RESULTS.setdefault("rounds", []).append(rr)
    reuse_state_snapshot()


try:
    boot = VM("listener-anchor", 3229)       # loads the vsock stack before the listeners are created
    boot.ready()
    listeners = {}
    for P in PORTS:
        s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
        s.bind((socket.VMADDR_CID_ANY, P))
        s.listen(4096)
        listeners[P] = s
    log(f"listeners up on {PORTS}")
    for r in range(ROUNDS):
        round_(r, listeners)
    boot.kill9(); boot.collect()
finally:
    for vm in VMS:
        if vm.proc.poll() is None:
            vm.proc.kill(); vm.proc.wait()
    reuse_state_snapshot()
    after = modules()
    (EV / "modules-after.txt").write_text("\n".join(after) + "\n")
    mine = [m for m in ("vsock_diag", "vhost_vsock", "vmw_vsock_virtio_transport", "vmw_vsock_virtio_transport_common", "vsock", "vhost", "vhost_iotlb")
            if m in after and m not in MODULES_BEFORE]
    for _ in range(3):
        for m in mine:
            sh(f"rmmod {m}", check=False, quiet=True)
    RESULTS["modules_unloaded_by_me"] = mine
    RESULTS["modules_left"] = [m for m in modules() if m in mine]
    save()
    log("TALLIES " + json.dumps(RESULTS.get("tallies"), indent=1, sort_keys=True))
    log("EXAMPLES " + json.dumps(RESULTS.get("examples"), indent=1, sort_keys=True))
    log("done")
