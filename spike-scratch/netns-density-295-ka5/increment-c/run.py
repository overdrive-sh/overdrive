#!/usr/bin/env python3
"""K-A5 probe (netns-density-295) - native metal run, under the metal lease (root).

Boots a real guest (Cloud Hypervisor fork, vhost-kernel vsock) and, for each case, has the GUEST open
a connection to a HOST vsock listener (CID 2), then has the HOST end it in a specific way. The guest
waits with no timer (blocking recv() or epoll_wait(-1)); the harness measures what the guest observes
and the latency from the host's action to the guest's wake-up (clock offset calibrated by ping-pong).
"""
import collections, ctypes, fcntl, hashlib, json, os, pathlib, random, re, resource, shutil, signal
import socket, statistics, struct, subprocess, sys, threading, time

INC = pathlib.Path(__file__).resolve().parent
BASE = INC.parent
ROOT = BASE.parent.parent
EV = INC / "evidence"
OUT = BASE / "out" / INC.name
EV.mkdir(exist_ok=False)
OUT.mkdir(parents=True, exist_ok=True)
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
FORK_CH_DIR = ROOT / "vendors/cloud-hypervisor"

ARGS = dict(a.split("=", 1) for a in sys.argv[1:])
CID = int(ARGS.get("cid", "3260"))
N = int(ARGS.get("n", "200"))
SUITE = ARGS.get("suite", "full")
CMD_PORT, CTRL_PORT = 52520, 52521
TRIAL_TIMEOUT = 4.0

lock = open("/tmp/ka5-netns-density-295.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
resource.setrlimit(resource.RLIMIT_NOFILE, (65536, 65536))
LOG = (EV / "run.log").open("a")
TRIALS_LOG = (EV / "trials.jsonl").open("a")  # append-only: one line per trial, first failure preserved


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    LOG.write(line + "\n")
    LOG.flush()


def sh(cmd, check=True, timeout=600, env=None, quiet=False):
    t0 = time.time()
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, timeout=timeout, env=env)
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": p.returncode, "wall_s": round(time.time() - t0, 3),
                            "out": p.stdout[-20000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(map(str, cmd))}  (rc={p.returncode})")
        for line in p.stdout.rstrip().splitlines()[-40:]:
            log("    " + line)
    if check and p.returncode != 0:
        raise RuntimeError(f"command failed rc={p.returncode}: {cmd}")
    return p.stdout


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def modules():
    return sorted(l.split()[0] for l in pathlib.Path("/proc/modules").read_text().splitlines())


RESULTS = {}


def record(section, key, value):
    RESULTS.setdefault(section, {})[key] = value
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))


# ------------------------------------------------------------------ preflight (fail closed)
log("=== preflight (fail-closed substrate probes)")
MODULES_START = modules()
(EV / "modules-before.txt").write_text("\n".join(MODULES_START) + "\n")
assert os.uname().machine == "x86_64", "not x86_64"
fd = os.open("/dev/kvm", os.O_RDWR)
api = fcntl.ioctl(fd, 0xAE00)
vmfd = fcntl.ioctl(fd, 0xAE01)
os.close(vmfd)
os.close(fd)
assert api == 12, f"kvm api {api}"
assert re.search(r"\b(vmx|svm)\b", pathlib.Path("/proc/cpuinfo").read_text()), "no vmx/svm"
virt = subprocess.run(["systemd-detect-virt"], text=True, stdout=subprocess.PIPE).stdout.strip()
assert virt == "none", f"virtualized host: {virt}"
record("preflight", "kvm_api", api)
record("preflight", "systemd_detect_virt", virt)
record("preflight", "host_uname_r", os.uname().release)
sh("uname -a; grep -m1 'model name' /proc/cpuinfo; nproc; free -m | head -2; ls -l /dev/kvm /dev/vhost-vsock")
pre_ch = sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False, quiet=True)
record("preflight", "other_cloud_hypervisors_at_start", pre_ch.strip())

# ------------------------------------------------------------------ build
log("=== build")
env = os.environ.copy()
(BASE / "out").mkdir(exist_ok=True)
for name in ("cargo-home", "ch-target"):  # warm caches from earlier spikes (copy, never modify)
    dst = BASE / "out" / name
    if dst.exists():
        continue
    for src in (ROOT / "spike-scratch/netns-density-295-v24/out" / name,
                ROOT / "spike-scratch/netns-density-295-ch-vhost-vsock/out" / name):
        if src.exists():
            sh(f"cp -a {src} {dst}")
            break
env["CARGO_HOME"] = str(BASE / "out" / "cargo-home")
env["CARGO_TARGET_DIR"] = str(BASE / "out" / "ch-target")
sh(["cargo", "build", "--release", "--locked", "--manifest-path", str(FORK_CH_DIR / "Cargo.toml"),
    "--bin", "cloud-hypervisor"], env=env, timeout=3600)
FORK_CH = str(BASE / "out" / "ch-target/release/cloud-hypervisor")
genv = env.copy()
genv["CARGO_TARGET_DIR"] = str(BASE / "out" / "guest-target")
genv["RUSTFLAGS"] = "-C target-feature=+crt-static"
sh(["cargo", "build", "--release", "--target", "x86_64-unknown-linux-gnu", "--manifest-path",
    str(BASE / "guest/Cargo.toml")], env=genv, timeout=900)
GUEST = str(BASE / "out" / "guest-target/x86_64-unknown-linux-gnu/release/ka5g")
sh(f"file {GUEST} {FORK_CH}; {FORK_CH} --version")

root = OUT / "initramfs-root"
if root.exists():
    shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]:
    (root / d).mkdir(parents=True)
shutil.copy2(shutil.which("busybox"), root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "sleep"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(GUEST, root / "ka5g")
shutil.copy2(BASE / "guest-init.sh", root / "init")
os.chmod(root / "init", 0o755)
os.chmod(root / "ka5g", 0o755)
pins = {}
mods = sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"]).splitlines()
mods += sh(["modprobe", "--show-depends", "vsock_diag"]).splitlines()
seen = set()
for line in mods:
    if line.startswith("insmod "):
        src = pathlib.Path(line.split()[1])
        if src.name in seen:
            continue
        seen.add(src.name)
        data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
        (root / src.name.removesuffix(".zst")).write_bytes(data)
        pins[str(src)] = sha(src)
INITRD = OUT / "initramfs.cpio.gz"
sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -9 > {INITRD}")
pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(INITRD), "fork_ch": sha(FORK_CH), "guest_probe": sha(GUEST)})
(EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))

# ------------------------------------------------------------------ host-side servers
EVENTS = []
CV = threading.Condition()
RAW = (EV / "reports.jsonl").open("a")


def add_event(d):
    d["h"] = time.monotonic_ns()
    with CV:
        EVENTS.append(d)
        CV.notify_all()
    RAW.write(json.dumps(d) + "\n")


def wait_for(pred, timeout, what=None):
    deadline = time.monotonic() + timeout
    with CV:
        while True:
            v = pred()
            if v:
                return v
            left = deadline - time.monotonic()
            if left <= 0:
                return None
            CV.wait(min(left, 0.05))


def ev_find(kind, id=None, start=0):
    for e in EVENTS[start:]:
        if e.get("k") == kind and (id is None or e.get("id") == id):
            return e
    return None


_LIBC = ctypes.CDLL(None, use_errno=True)


def set_linger0(sock):
    l = struct.pack("ii", 1, 0)
    if _LIBC.setsockopt(sock.fileno(), socket.SOL_SOCKET, socket.SO_LINGER, l, len(l)) != 0:
        raise OSError(ctypes.get_errno(), "setsockopt SO_LINGER")


def vsock_listener(port, backlog=128):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.bind((socket.VMADDR_CID_ANY, port))
    s.listen(backlog)
    return s


CMD = {"sock": None}
CTRL = {"sock": None}
CMD_LOCK = threading.Lock()


def cmd_server(lsock):
    c, _ = lsock.accept()
    CMD["sock"] = c
    f = c.makefile("r")
    for line in f:
        try:
            add_event(json.loads(line))
        except Exception as e:
            add_event({"k": "bad_report", "line": line[:200], "err": str(e)})
    add_event({"k": "cmd_channel_end"})


def ctrl_server(lsock):
    c, _ = lsock.accept()
    CTRL["sock"] = c  # held open and silent until the final check


def send_cmd(line):
    with CMD_LOCK:
        CMD["sock"].sendall((line + "\n").encode())


threading.Thread(target=cmd_server, args=(vsock_listener(CMD_PORT),), daemon=True).start()
threading.Thread(target=ctrl_server, args=(vsock_listener(CTRL_PORT),), daemon=True).start()


class VM:
    def __init__(self, cid):
        self.cid = cid
        self.dir = OUT / f"vm-{cid}-{int(time.time() * 1000)}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmd = [FORK_CH, "--kernel", GUEST_KERNEL, "--initramfs", str(INITRD),
               "--cmdline", "console=ttyS0 panic=-1 loglevel=4",
               "--cpus", "boot=2", "--memory", "size=1024M",
               "--serial", f"file={self.serial}", "--console", "off",
               "--log-file", str(self.dir / "ch.log"), "-v",
               "--vsock", f"cid={cid},backend=vhost-kernel"]
        log("launch: " + " ".join(cmd))
        self.out = (self.dir / "ch.stdout").open("w")
        self.proc = subprocess.Popen(cmd, stdout=self.out, stderr=subprocess.STDOUT)

    def serial_text(self):
        return self.serial.read_text(errors="replace") if self.serial.exists() else ""

    def collect(self):
        for f in [self.serial, self.dir / "ch.stdout", self.dir / "ch.log"]:
            if f.exists():
                shutil.copy2(f, EV / f"vm{self.cid}-{f.name}")

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()


# ------------------------------------------------------------------ trial machinery
NEXT_ID = [1000]
CASES = {}   # case -> list of trial dicts
CAL_ERR = []


def new_id():
    NEXT_ID[0] += 1
    return NEXT_ID[0]


def calibrate():
    """guest_monotonic - host_monotonic offset (ns), from the min-RTT of 5 ping-pongs; error <= rtt/2."""
    best = None
    for _ in range(5):
        start = len(EVENTS)
        t1 = time.monotonic_ns()
        send_cmd("P")
        p = wait_for(lambda: ev_find("pong", None, start), 2)
        if not p:
            continue
        t2 = p["h"]
        rtt = t2 - t1
        off = p["t"] - (t1 + t2) // 2
        if best is None or rtt < best[0]:
            best = (rtt, off)
    assert best, "no pong"
    CAL_ERR.append(best[0] // 2)
    return best[1]


EPOLL = {"IN": 0x1, "OUT": 0x4, "ERR": 0x8, "HUP": 0x10, "RDHUP": 0x2000}
POLL = {"IN": 0x1, "OUT": 0x4, "ERR": 0x8, "HUP": 0x10, "RDHUP": 0x2000}


def bits(v, table):
    return "|".join(k for k, m in table.items() if v & m) or "0"


def errname(e):
    import errno as _e
    return _e.errorcode.get(e, str(e)) if e else "0"


def classify_end(e):
    """Human-readable observation from an 'end' report."""
    f = e["first"]
    if e["mode"] == "read":
        first = "read=EOF" if f["ret"] == 0 else f"read=-1/{errname(f['errno'])}"
    else:
        first = "epoll=" + bits(f["epoll_events"], EPOLL)
    return {
        "first": first,
        "drain": f"{e['drain_end']}" + (f"/{errname(e['drain_errno'])}" if e["drain_errno"] else ""),
        "poll": bits(e["poll"], POLL),
        "write": "ok" if e["write_rc"] >= 0 else errname(e["write_errno"]),
        "so_error": errname(e["so_error"]),
        "data_before": f.get("data_before", 0),
    }


def finish(case, rec):
    CASES.setdefault(case, []).append(rec)
    TRIALS_LOG.write(json.dumps({"case": case, **rec}) + "\n")
    TRIALS_LOG.flush()


def reap(id):
    send_cmd(f"R {id}")


def await_end(id, timeout=TRIAL_TIMEOUT):
    """The guest's observation: an 'end' report, or a connect_fail. Returns (event, kind) or (None,'NO-END')."""
    e = wait_for(lambda: ev_find("end", id) or ev_find("connect_fail", id), timeout)
    if not e:
        send_cmd(f"X {id}")
        wait_for(lambda: ev_find("abandoned", id), 2)
        return None
    return e


def run_accepted(case, mode, how):
    """Guest connects; host accepts (or a separate holder process does); host ends it per `how`."""
    id = new_id()
    port = 200000 + id
    rec = {"id": id, "mode": mode, "how": how}
    holder = lst = c = None
    try:
        if how.startswith("sigkill"):
            holder = subprocess.Popen([sys.executable, str(BASE / "holder.py"), str(port),
                                       "accept" if how == "sigkill" else "noaccept"],
                                      stdout=subprocess.PIPE, text=True)
            assert holder.stdout.readline().strip() == "L"
        else:
            lst = vsock_listener(port)
        off = calibrate()
        send_cmd(f"T {id} {mode} {port}")
        conn = wait_for(lambda: ev_find("connected", id) or ev_find("connect_fail", id), 5)
        if not conn or conn["k"] != "connected":
            rec.update(result="SETUP-FAIL", detail=conn)
            return finish(case, rec)
        if how == "sigkill":
            assert holder.stdout.readline().strip() == "A"
        elif lst is not None:
            lst.settimeout(5)
            c, _ = lst.accept()
        assert wait_for(lambda: ev_find("waiting", id), 5), "guest never reported waiting"
        time.sleep(0.02)  # let the guest thread block in recv()/epoll_wait()
        if how == "linger0":
            set_linger0(c)
        t_h = time.monotonic_ns()
        if how in ("close", "linger0"):
            c.close()
            c = None
        elif how == "data_close":
            c.sendall(b"x" * 16)
            c.close()
            c = None
        else:
            os.kill(holder.pid, signal.SIGKILL)
        e = await_end(id)
        if not e:
            rec.update(result="NO-END")
        else:
            lat = (e["t"] - off - t_h) / 1000.0
            rec.update(result="END", latency_us=lat, **classify_end(e))
        return finish(case, rec)
    finally:
        if holder:
            holder.kill()
            holder.wait()
        for s in (c, lst):
            if s:
                s.close()
        reap(id)


def run_ctl_idle(case, mode):
    """Control: an unrelated connection stays open; the guest must NOT see an end."""
    id = new_id()
    port = 200000 + id
    rec = {"id": id, "mode": mode, "how": "idle"}
    lst = vsock_listener(port)
    c = None
    try:
        send_cmd(f"T {id} {mode} {port}")
        assert wait_for(lambda: ev_find("connected", id), 5)
        lst.settimeout(5)
        c, _ = lst.accept()
        assert wait_for(lambda: ev_find("waiting", id), 5)
        time.sleep(0.2)
        start = len(EVENTS)
        send_cmd(f"Q {id}")
        st = wait_for(lambda: ev_find("status", id, start), 2)
        ended = ev_find("end", id)
        bad = bool(ended) or (st and (st["ended"] or st["poll"] & (POLL["HUP"] | POLL["RDHUP"] | POLL["ERR"])))
        rec.update(result="FALSE-END" if bad else "STILL-OPEN", status=st)
        finish(case, rec)
    finally:
        if c:
            c.close()  # now end it (graceful) so the guest thread finishes
        lst.close()
        wait_for(lambda: ev_find("end", id), 3)
        reap(id)


def run_nolistener(case, mode):
    id = new_id()
    port = 200000 + id
    rec = {"id": id, "mode": mode, "how": "no-listener"}
    send_cmd(f"T {id} {mode} {port}")
    e = await_end(id)
    if not e:
        rec.update(result="NO-END")
    elif e["k"] == "connect_fail":
        rec.update(result="CONNECT-FAIL", errno=errname(e["errno"]), connect_latency_us=(e["t"] - e["t_start"]) / 1000.0)
    else:
        rec.update(result="UNEXPECTED-CONNECTED-THEN-END", **classify_end(e))
    finish(case, rec)
    reap(id)


def run_backlog_full(case_refuse, case_filler, mode):
    """listen(0): one queued filler connection, then a second connect() is refused; then the listener is
    closed while the filler is still queued (host never accept()s)."""
    id, idf = new_id(), new_id()
    port = 200000 + id
    lst = vsock_listener(port, backlog=0)
    try:
        send_cmd(f"T {idf} {mode} {port}")
        fc = wait_for(lambda: ev_find("connected", idf) or ev_find("connect_fail", idf), 5)
        rec_f = {"id": idf, "mode": mode, "how": "filler"}
        if not fc or fc["k"] != "connected":
            rec_f.update(result="SETUP-FAIL", detail=fc)
            finish(case_filler, rec_f)
            return
        assert wait_for(lambda: ev_find("waiting", idf), 5)
        send_cmd(f"T {id} {mode} {port}")
        e = await_end(id)
        rec = {"id": id, "mode": mode, "how": "backlog-full"}
        if not e:
            rec.update(result="NO-END")
        elif e["k"] == "connect_fail":
            rec.update(result="CONNECT-FAIL", errno=errname(e["errno"]), connect_latency_us=(e["t"] - e["t_start"]) / 1000.0)
        else:
            rec.update(result="UNEXPECTED-CONNECTED-THEN-END", **classify_end(e))
        finish(case_refuse, rec)
        # now close the listener with the filler still queued
        off = calibrate()
        time.sleep(0.02)
        t_h = time.monotonic_ns()
        lst.close()
        lst = None
        e = await_end(idf)
        if not e:
            rec_f.update(result="NO-END")
        else:
            rec_f.update(result="END", latency_us=(e["t"] - off - t_h) / 1000.0, **classify_end(e))
        finish(case_filler, rec_f)
    finally:
        if lst:
            lst.close()
        reap(id)
        reap(idf)


def run_listener_close_queued(case, mode):
    id = new_id()
    port = 200000 + id
    lst = vsock_listener(port)
    rec = {"id": id, "mode": mode, "how": "listener-close-queued"}
    try:
        off = calibrate()
        send_cmd(f"T {id} {mode} {port}")
        conn = wait_for(lambda: ev_find("connected", id) or ev_find("connect_fail", id), 5)
        if not conn or conn["k"] != "connected":
            rec.update(result="SETUP-FAIL", detail=conn)
            return finish(case, rec)
        assert wait_for(lambda: ev_find("waiting", id), 5)
        time.sleep(0.02)
        t_h = time.monotonic_ns()
        lst.close()
        lst = None
        e = await_end(id)
        if not e:
            rec.update(result="NO-END")
        else:
            rec.update(result="END", latency_us=(e["t"] - off - t_h) / 1000.0, **classify_end(e))
        finish(case, rec)
    finally:
        if lst:
            lst.close()
        reap(id)


def run_race(case, mode):
    """Guest connect() in flight while the host closes the listener (jitter 0..1000 us)."""
    id = new_id()
    port = 200000 + id
    lst = vsock_listener(port)
    jitter = random.randint(0, 1000)
    rec = {"id": id, "mode": mode, "how": "race", "jitter_us": jitter}
    try:
        off = calibrate()
        send_cmd(f"T {id} {mode} {port}")
        t_end = time.monotonic_ns() + jitter * 1000
        while time.monotonic_ns() < t_end:
            pass
        t_h = time.monotonic_ns()
        lst.close()
        lst = None
        e = wait_for(lambda: ev_find("connect_fail", id) or ev_find("end", id), TRIAL_TIMEOUT)
        if not e:
            send_cmd(f"X {id}")
            c1 = ev_find("connected", id)
            rec.update(result="NO-END", connected=bool(c1))
        elif e["k"] == "connect_fail":
            rec.update(result="CONNECT-FAIL", errno=errname(e["errno"]), connect_latency_us=(e["t"] - e["t_start"]) / 1000.0)
        else:
            rec.update(result="END", via="connected-then-end", latency_us=(e["t"] - off - t_h) / 1000.0, **classify_end(e))
        finish(case, rec)
    finally:
        if lst:
            lst.close()
        reap(id)


def loop(case, n, fn, *a):
    fails = 0
    for i in range(n):
        before = len(CASES.get(case, []))
        fn(case, *a)
        last = CASES[case][-1] if case in CASES else None
        if last and last.get("result") in ("NO-END", "SETUP-FAIL"):
            fails += 1
        else:
            fails = 0
        if fails >= 10:
            log(f"{case}: 10 consecutive failures, aborting this case")
            break
    summarize(case)


def pct(vals, p):
    vals = sorted(vals)
    if not vals:
        return None
    return vals[min(len(vals) - 1, int(round(p / 100.0 * (len(vals) - 1))))]


def summarize(case):
    t = CASES.get(case, [])
    res = collections.Counter(r.get("result") for r in t)
    obs = collections.Counter((r.get("first"), r.get("drain"), r.get("poll"), r.get("write"), r.get("errno")) for r in t)
    lat = [r["latency_us"] for r in t if "latency_us" in r]
    clat = [r["connect_latency_us"] for r in t if "connect_latency_us" in r]
    s = {"trials": len(t), "results": dict(res), "observations": {str(k): v for k, v in obs.items()}}
    if lat:
        s["latency_us"] = {"min": min(lat), "p50": pct(lat, 50), "p99": pct(lat, 99), "max": max(lat)}
    if clat:
        s["connect_latency_us"] = {"min": min(clat), "p50": pct(clat, 50), "p99": pct(clat, 99), "max": max(clat)}
    record("cases", case, s)
    log(f"{case}: {json.dumps(s)}")


def suite_full(vm):
    for mode in ("read", "epoll"):
        loop(f"ctl-idle-{mode}", N, run_ctl_idle, mode)
    for mode in ("read", "epoll"):
        loop(f"c1-host-close-{mode}", N, run_accepted, mode, "close")
        loop(f"c2-host-linger0-{mode}", N, run_accepted, mode, "linger0")
        loop(f"c3-host-sigkill-{mode}", N, run_accepted, mode, "sigkill")
        loop(f"c5-data-then-close-{mode}", N, run_accepted, mode, "data_close")
    loop("c3b-host-sigkill-unaccepted-epoll", N, run_accepted, "epoll", "sigkill_queued")
    loop("c3b-host-sigkill-unaccepted-read", N, run_accepted, "read", "sigkill_queued")
    loop("c4a-no-listener", N, run_nolistener, "epoll")
    for mode in ("epoll",):
        for i in range(N):
            run_backlog_full("c4b-backlog-full-refused", "c4b-filler-after-listener-close", mode)
        summarize("c4b-backlog-full-refused")
        summarize("c4b-filler-after-listener-close")
    loop("c4c-listener-close-queued-epoll", N, run_listener_close_queued, "epoll")
    loop("c4c-listener-close-queued-read", N, run_listener_close_queued, "read")
    loop("c4d-connect-vs-listener-close-race", N * 2, run_race, "epoll")


def suite_smoke(vm):
    for mode in ("read", "epoll"):
        loop(f"ctl-idle-{mode}", 3, run_ctl_idle, mode)
        loop(f"c1-host-close-{mode}", 3, run_accepted, mode, "close")
        loop(f"c2-host-linger0-{mode}", 3, run_accepted, mode, "linger0")
        loop(f"c3-host-sigkill-{mode}", 3, run_accepted, mode, "sigkill")
        loop(f"c5-data-then-close-{mode}", 3, run_accepted, mode, "data_close")
    loop("c3b-host-sigkill-unaccepted-epoll", 3, run_accepted, "epoll", "sigkill_queued")
    loop("c4a-no-listener", 3, run_nolistener, "epoll")
    run_backlog_full("c4b-backlog-full-refused", "c4b-filler-after-listener-close", "epoll")
    summarize("c4b-backlog-full-refused")
    summarize("c4b-filler-after-listener-close")
    loop("c4c-listener-close-queued-epoll", 3, run_listener_close_queued, "epoll")
    loop("c4d-connect-vs-listener-close-race", 10, run_race, "epoll")


vm = None
try:
    log(f"=== boot guest CID {CID}")
    vm = VM(CID)
    hello = wait_for(lambda: ev_find("hello"), 90)
    assert hello, "guest never reported hello; serial:\n" + vm.serial_text()[-3000:]
    log(f"guest hello: {hello}")
    record("guest", "hello", hello)
    record("guest", "serial_boot", vm.serial_text()[:3000])
    assert hello["local_cid"] == CID, hello
    assert wait_for(lambda: ev_find("ctrl_connected"), 30), "ctrl connection never came up"
    assert wait_for(lambda: CTRL["sock"], 5)
    sh(f"{FORK_CH} --version")
    {"full": suite_full, "smoke": suite_smoke}[SUITE](vm)
    # final unrelated-connection check: no false end during the whole run, and it is still usable
    start = len(EVENTS)
    CTRL["sock"].sendall(b"Z")
    got = wait_for(lambda: ev_find("ctrl_byte", None, start), 5)
    false_ends = [e for e in EVENTS if e.get("k") == "ctrl_false_end"]
    record("control", "unrelated_idle_connection", {"false_end_events": len(false_ends), "final_byte_received": bool(got),
                                                      "false_end_detail": false_ends[:5]})
    log(f"unrelated idle connection: false_ends={len(false_ends)} final_byte={bool(got)}")
    record("calibration", "clock_offset_error_bound_us", {
        "median": statistics.median(CAL_ERR) / 1000.0, "max": max(CAL_ERR) / 1000.0, "n": len(CAL_ERR)})
    record("done", "ok", True)
finally:
    try:
        log("=== teardown")
        sh(f"ss --vsock -a -n -p > {EV}/host-ss-vsock-end.txt 2>&1; head -20 {EV}/host-ss-vsock-end.txt", check=False)
        try:
            send_cmd("POWEROFF")
        except Exception:
            pass
        if vm:
            try:
                vm.proc.wait(timeout=20)
            except Exception:
                pass
            vm.kill()
            vm.collect()
        sh("pgrep -af 'cloud-hypervisor' | grep -v pgrep || true", check=False)
        for _ in range(3):
            for m in ["vsock_diag", "vhost_vsock", "vmw_vsock_virtio_transport", "vmw_vsock_virtio_transport_common",
                      "vsock_loopback", "vsock", "vhost", "vhost_iotlb"]:
                if m in modules() and m not in MODULES_START:
                    sh(f"rmmod {m}", check=False, quiet=True)
        (EV / "modules-after.txt").write_text("\n".join(modules()) + "\n")
        record("cleanup", "modules_added_and_left", sorted(set(modules()) - set(MODULES_START)))
    except Exception as e:
        log(f"teardown error: {e!r}")
