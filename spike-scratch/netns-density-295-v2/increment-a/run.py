#!/usr/bin/env python3
"""V-2 probe (netns-density-295) - native metal run, under the metal lease (root).

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
CID = int(ARGS.get("cid", "3270"))
N = int(ARGS.get("n", "200"))
SUITE = ARGS.get("suite", "full")
CMD_PORT = 52520
TRIAL_TIMEOUT = 6.0

lock = open("/tmp/v2-netns-density-295.lock", "a+")
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
    for src in (ROOT / "spike-scratch/netns-density-295-ka5/out" / name,
                ROOT / "spike-scratch/netns-density-295-v24/out" / name,
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
GUEST = str(BASE / "out" / "guest-target/x86_64-unknown-linux-gnu/release/v2g")
sh(f"file {GUEST} {FORK_CH}; {FORK_CH} --version")

root = OUT / "initramfs-root"
if root.exists():
    shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]:
    (root / d).mkdir(parents=True)
shutil.copy2(shutil.which("busybox"), root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "sleep", "head"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(GUEST, root / "v2g")
shutil.copy2(BASE / "guest-init.sh", root / "init")
os.chmod(root / "init", 0o755)
os.chmod(root / "v2g", 0o755)
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
NEXT_ID = [5000]
CASES = {}   # case -> list of trial dicts
END_BITS = POLL_HUP = 0x10
EPOLL_BITS = {"IN": 0x1, "OUT": 0x4, "ERR": 0x8, "HUP": 0x10, "RDHUP": 0x2000}
END_MASK = 0x8 | 0x10 | 0x2000   # POLLERR | POLLHUP | POLLRDHUP
GUEST_CID_REAL = [None]
BURNERS = []


def new_id():
    NEXT_ID[0] += 1
    return NEXT_ID[0]


def bits(v):
    if v is None or v < 0:
        return str(v)
    return "|".join(k for k, m in EPOLL_BITS.items() if v & m) or "0"


def busy_us(us):
    end = time.monotonic_ns() + int(us * 1000)
    while time.monotonic_ns() < end:
        pass


def vconnect(port):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(5)
    s.connect((GUEST_CID_REAL[0], port))
    return s


def ts_us(ts):
    try:
        return float(ts) * 1e6
    except Exception:
        return None


def verdict(res, var, bmode, ids):
    """Exact oracle: the guest kernel's receive order of host->guest packets (tracepoint at the
    classifier site) + the user-space spinner's sample of A taken after B's first byte was readable."""
    lpa, lpb, hl = ids
    guest_opened = var in ("v3", "v4")
    tr = res.get("trace", [])   # [seq, ts, src_port, dst_port, dst_cid, len, op]
    a_end = b_req = b_data = None
    for i, (seq, ts, sp, dp, dcid, ln, op) in enumerate(tr):
        is_a = (sp == hl) if guest_opened else (dp == lpa)
        is_b = (dp == lpb) and not is_a
        if is_a and op in ("SHUTDOWN", "RST") and a_end is None:
            a_end = i
        if is_b and op == "REQUEST" and b_req is None:
            b_req = i
        if is_b and op == "RW" and ln > 0 and b_data is None:
            b_data = i
    out = {"trace_a_end_idx": a_end, "trace_b_req_idx": b_req, "trace_b_data_idx": b_data,
           "trace_len": len(tr)}
    if a_end is None or b_data is None:
        out["trace"] = "INCOMPLETE"
    else:
        ok = a_end < b_data and (bmode == "pre" or b_req is None or a_end < b_req)
        out["trace"] = "ORDERED" if ok else "REORDERED"
        ta, tb = ts_us(tr[a_end][1]), ts_us(tr[b_data][1])
        if ta is not None and tb is not None:
            out["margin_us"] = tb - ta
    abd = res.get("a_at_b_data")
    if res.get("timeout") or abd is None:
        out["user"] = "NO-OBS"
    elif abd == -1:
        out["user"] = "A-NOT-ACCEPTED-AT-B-DATA"
    else:
        out["user"] = "A-ENDED" if abd & END_MASK else "A-NOT-ENDED"
        out["a_at_b_data"] = bits(abd)
    out["a_late_accept"] = res.get("a_late_accept")
    if bmode == "fresh" and res.get("a_at_b_acc", -3) >= 0:
        out["user_at_b_accept"] = "A-ENDED" if res["a_at_b_acc"] & END_MASK else "A-NOT-ENDED"
    out["lost"] = res.get("lost")
    return out


def finish(case, rec):
    CASES.setdefault(case, []).append(rec)
    TRIALS_LOG.write(json.dumps({"case": case, **rec}) + "\n")
    TRIALS_LOG.flush()


def run_trial(case, var, bmode, gap, order="normal"):
    """var: v1 dial-drop | v2 paired-drop | v3 guest-opened host close | v4 guest-opened host linger0.
    bmode: fresh (connect B after A's release) | pre (B established before; request sent after A's release).
    gap: microseconds between A's release returning and the next step ('rand' = uniform 0..1000).
    order: normal | reversed (control: B's request first, A released after)."""
    id = new_id()
    lpa, lpb, hl = 300000 + 3 * id, 300001 + 3 * id, 300002 + 3 * id
    tag = f"{id:08d}".encode()
    g = random.uniform(0, 1000) if gap == "rand" else float(gap)
    rec = {"id": id, "var": var, "bmode": bmode, "order": order, "gap_req_us": round(g, 1)}
    lst_hl = vsock_listener(hl) if var in ("v3", "v4") else None
    A = B = None
    try:
        start = len(EVENTS)
        send_cmd(f"N {id} {var} {lpa} {lpb} {hl}")
        r = wait_for(lambda: ev_find("ready", id, start) or ev_find("setup_fail", id, start), 6)
        if not r or r["k"] != "ready":
            rec.update(result="SETUP-FAIL", detail=r)
            return finish(case, rec)
        if bmode == "pre":
            B = vconnect(lpb)
        if var in ("v1", "v2"):
            A = vconnect(lpa)
            A.sendall(b"A" + tag)
            if var == "v2":
                A.settimeout(3)
                assert A.recv(1) == b"k", "no pairing ack"
        else:
            lst_hl.settimeout(3)
            A, _ = lst_hl.accept()
            if var == "v4":
                set_linger0(A)
        if order == "normal":
            t0 = time.monotonic_ns()
            A.close()
            A = None
            t1 = time.monotonic_ns()
            busy_us(g)
            t2 = time.monotonic_ns()
            if bmode == "fresh":
                B = vconnect(lpb)
            t3 = time.monotonic_ns()
            B.sendall(b"B" + tag)
            t4 = time.monotonic_ns()
            rec.update(close_us=(t1 - t0) / 1e3, gap_actual_us=(t2 - t1) / 1e3, connect_b_us=(t3 - t2) / 1e3,
                       release_to_send_start_us=(t2 - t1) / 1e3 if bmode == "pre" else (t3 - t1) / 1e3)
        else:
            if bmode == "fresh":
                B = vconnect(lpb)
            B.sendall(b"B" + tag)
            busy_us(g)
            A.close()
            A = None
        res = wait_for(lambda: ev_find("res", id, start), TRIAL_TIMEOUT)
        if not res:
            rec.update(result="NO-RESULT")
            return finish(case, rec)
        v = verdict(res, var, bmode, (lpa, lpb, hl))
        rec.update(v)
        if order == "reversed":
            rec["result"] = "CONTROL-REORDER-DETECTED" if v["trace"] == "REORDERED" else f"CONTROL-MISSED-{v['trace']}"
        elif v["trace"] == "ORDERED" and v["user"] == "A-ENDED":
            rec["result"] = "PASS"
        elif v["trace"] == "REORDERED":
            rec["result"] = "VIOLATION-TRACE"
        elif v["user"] in ("A-NOT-ENDED", "A-NOT-ACCEPTED-AT-B-DATA"):
            rec["result"] = "VIOLATION-USER"
        else:
            rec["result"] = "INCOMPLETE"
        return finish(case, rec)
    except Exception as e:
        rec.update(result="HOST-EXC", exc=repr(e))
        return finish(case, rec)
    finally:
        for s in (A, B, lst_hl):
            if s:
                try:
                    s.close()
                except Exception:
                    pass


def pct(vals, p):
    vals = sorted(vals)
    if not vals:
        return None
    return vals[min(len(vals) - 1, int(round(p / 100.0 * (len(vals) - 1))))]


def summarize(case):
    t = CASES.get(case, [])
    res = collections.Counter(r.get("result") for r in t)
    s = {"trials": len(t), "results": dict(res),
         "trace": dict(collections.Counter(r.get("trace") for r in t)),
         "user": dict(collections.Counter(r.get("user") for r in t)),
         "a_late_accept": sum(1 for r in t if r.get("a_late_accept"))}
    for key in ("margin_us", "gap_actual_us", "release_to_send_start_us"):
        v = [r[key] for r in t if key in r]
        if v:
            s[key] = {"min": round(min(v), 1), "p50": round(pct(v, 50), 1), "p99": round(pct(v, 99), 1), "max": round(max(v), 1)}
    record("cases", case, s)
    log(f"{case}: {json.dumps(s)}")


def loop(case, n, *a, **kw):
    fails = 0
    for _ in range(n):
        run_trial(case, *a, **kw)
        last = CASES[case][-1]
        fails = fails + 1 if last.get("result") in ("SETUP-FAIL", "NO-RESULT", "HOST-EXC") else 0
        if fails >= 10:
            log(f"{case}: 10 consecutive infrastructure failures, aborting this case")
            break
    summarize(case)


VARS = ("v1", "v2", "v3", "v4")


def suite_pass(label, n_cell, n_grid, n_ctl):
    for var in VARS:
        for bmode in ("fresh", "pre"):
            loop(f"{label}-{var}-{bmode}-b2b", n_cell, var, bmode, 0)
            loop(f"{label}-{var}-{bmode}-jit0to1000", n_cell, var, bmode, "rand")
    if n_grid:
        for var, bmode in (("v1", "pre"), ("v3", "pre"), ("v1", "fresh")):
            for gus in (25, 100, 250, 500, 1000):
                loop(f"{label}-grid-{var}-{bmode}-gap{gus}us", n_grid, var, bmode, gus)
    if n_ctl:
        loop(f"{label}-CONTROL-reversed-v1-pre", n_ctl, "v1", "pre", "rand", order="reversed")
        loop(f"{label}-CONTROL-reversed-v3-fresh", n_ctl, "v3", "fresh", "rand", order="reversed")


def suite_full(vm):
    suite_pass("idle", N, max(N // 2, 1), max(N // 2, 1))
    # loaded pass: all-but-4 host cores burning, 2 burner threads in the 2-vCPU guest
    nburn = max((os.cpu_count() or 8) - 4, 2)
    for _ in range(nburn):
        BURNERS.append(subprocess.Popen([sys.executable, "-c", "while True: pass"]))
    send_cmd("BURN 2")
    assert wait_for(lambda: ev_find("burning"), 5)
    record("load", "host_burners", nburn)
    record("load", "guest_burners", 2)
    suite_pass("loaded", max(N * 3 // 5, 1), 0, 0)
    send_cmd("UNBURN")
    stop_burners()


def stop_burners():
    for p in BURNERS:
        p.kill()
        p.wait()
    BURNERS.clear()


def suite_smoke(vm):
    suite_pass("smoke", 3, 0, 3)


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
    assert hello["trace_ok"], f"guest tracepoint unavailable: {hello}"
    GUEST_CID_REAL[0] = CID
    sh(f"{FORK_CH} --version")
    {"full": suite_full, "smoke": suite_smoke}[SUITE](vm)
    allr = [r for rs in CASES.values() for r in rs]
    record("totals", "all_trials", {"n": len(allr), "results": dict(collections.Counter(r.get("result") for r in allr))})
    record("done", "ok", True)
finally:
    try:
        log("=== teardown")
        stop_burners()
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
