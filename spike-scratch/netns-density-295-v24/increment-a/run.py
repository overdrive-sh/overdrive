#!/usr/bin/env python3
"""V-24 probe (netns-density-295) - native metal run.

Runs under the canonical metal lease via `cargo xtask metal run -- python3 ...` (root).
Builds the forked Cloud Hypervisor (kernel vhost-vsock backend) and the guest probe,
boots a real guest (CID 3240) on the physical host (no nesting), and drives the
host side of a TcpAccept flow: the host opens V_h (AF_VSOCK STREAM) to the guest's
flow port, writes the 16-byte request, then closes V_h for each close cause the V-24
row names (abort, quiescence, control-session loss), with the guest's V_g either
already accepted or still in the guest accept queue. Everything is observed from
inside the guest (reports over a vsock stream + vsock_diag + /proc/net/tcp).
"""
import collections, errno as errno_mod, fcntl, hashlib, json, os, pathlib, re, resource, shutil, socket, statistics
import struct, subprocess, sys, threading, time

INC = pathlib.Path(__file__).resolve().parent
BASE = INC.parent
ROOT = BASE.parent.parent
OLD = ROOT / "spike-scratch/netns-density-295-ch-vhost-vsock/out"
EV = INC / "evidence"
OUT = BASE / "out" / INC.name
EV.mkdir(exist_ok=False)
(EV / "cases").mkdir()
OUT.mkdir(parents=True, exist_ok=True)
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
FORK_CH_DIR = ROOT / "vendors/cloud-hypervisor"

ARGS = dict(a.split("=", 1) for a in sys.argv[1:])
CID = int(ARGS.get("cid", "3240"))
assert 3240 <= CID <= 3249
N = int(ARGS.get("n", "1000"))
SUITE = ARGS.get("suite", "full")

FLOW_PORT, CMD_PORT, HOST_CTL_PORT, HOST_RPT_PORT = 52421, 52420, 52423, 52424

lock = open("/tmp/v24-netns-density-295.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
resource.setrlimit(resource.RLIMIT_NOFILE, (65536, 65536))

LOG = (EV / "run.log").open("a")


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
record("preflight", "modules_at_start", MODULES_START)
assert os.uname().machine == "x86_64", "not x86_64"
fd = os.open("/dev/kvm", os.O_RDWR)
api = fcntl.ioctl(fd, 0xAE00)  # KVM_GET_API_VERSION
vmfd = fcntl.ioctl(fd, 0xAE01)  # KVM_CREATE_VM
os.close(vmfd)
os.close(fd)
assert api == 12, f"kvm api {api}"
cpuinfo = pathlib.Path("/proc/cpuinfo").read_text()
assert re.search(r"\b(vmx|svm)\b", cpuinfo), "no vmx/svm"
virt = subprocess.run(["systemd-detect-virt"], text=True, stdout=subprocess.PIPE).stdout.strip()
assert virt == "none", f"virtualized host: {virt}"
record("preflight", "kvm_api", api)
record("preflight", "systemd_detect_virt", virt)
record("preflight", "host_uname_r", os.uname().release)
sh("uname -a; grep -m1 'model name' /proc/cpuinfo; nproc; free -m | head -2; ls -l /dev/kvm /dev/vhost-vsock")
pre_ch = sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False, quiet=True)
record("preflight", "other_cloud_hypervisors_at_start", pre_ch.strip())
log(f"other CH processes at start (not touched): {pre_ch.strip() or 'none'}")

# ------------------------------------------------------------------ build
log("=== build")
env = os.environ.copy()
(BASE / "out").mkdir(exist_ok=True)
for name in ("cargo-home", "ch-target"):  # warm caches from the earlier V-1(b) build (copy, never modify)
    dst = BASE / "out" / name
    src = OLD / name
    if not dst.exists() and src.exists():
        sh(f"cp -a {src} {dst}")
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
GUEST = str(BASE / "out" / "guest-target/x86_64-unknown-linux-gnu/release/v24g")
sh(f"file {GUEST} {FORK_CH}; {FORK_CH} --version")

root = OUT / "v24-initramfs-root"
if root.exists():
    shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]:
    (root / d).mkdir(parents=True)
busybox = shutil.which("busybox")
shutil.copy2(busybox, root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "sleep", "cut", "basename", "readlink"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(GUEST, root / "v24g")
shutil.copy2(BASE / "guest-init.sh", root / "init")
pins = {}
mods = sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"]).splitlines()
mods += sh(["modprobe", "--show-depends", "vsock_diag"], check=True).splitlines()
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
INITRD = OUT / "v24-initramfs.cpio.gz"
sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -9 > {INITRD}")
pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(INITRD), "fork_ch": sha(FORK_CH), "guest_probe": sha(GUEST)})
(EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))

# ------------------------------------------------------------------ host-side servers
EVENTS = []  # every guest report + host-side record: dicts with host monotonic time "h"
CV = threading.Condition()
RAW = (EV / "reports.jsonl").open("a")


def add_event(d):
    d["h"] = time.monotonic()
    with CV:
        EVENTS.append(d)
        CV.notify_all()
    RAW.write(json.dumps(d) + "\n")


def wait_for(pred, timeout, what):
    deadline = time.monotonic() + timeout
    with CV:
        while True:
            v = pred()
            if v:
                return v
            left = deadline - time.monotonic()
            if left <= 0:
                log(f"TIMEOUT waiting for {what}")
                return None
            CV.wait(min(left, 0.2))


def vsock_listener(port):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.bind((socket.VMADDR_CID_ANY, port))
    s.listen(4096)
    return s


def rpt_server(lsock):
    def reader(c):
        f = c.makefile("r")
        for line in f:
            try:
                add_event(json.loads(line))
            except Exception as e:
                add_event({"k": "bad_report", "line": line[:200], "err": str(e)})
    while True:
        c, _ = lsock.accept()
        threading.Thread(target=reader, args=(c,), daemon=True).start()


class Session:
    """Host side of the control session: one live session; messages are exactly 8 bytes; no replies (U-5)."""
    def __init__(self):
        self.sock = None
        self.lock = threading.Lock()
        self.open_flows = set()  # flows the host still holds open
        self.paired_open = 0
        self.paired_discarded = 0
        self.hello = 0

    def serve(self, lsock):
        while True:
            c, peer = lsock.accept()
            with self.lock:
                old, self.sock = self.sock, c
            if old:
                try:
                    old.close()
                except OSError:
                    pass
            threading.Thread(target=self.reader, args=(c, peer), daemon=True).start()

    def reader(self, c, peer):
        while True:
            buf = b""
            try:
                while len(buf) < 8:
                    d = c.recv(8 - len(buf))
                    if not d:
                        raise EOFError
                    buf += d
            except (EOFError, OSError):
                add_event({"k": "host_ctl_end", "peer": list(peer)})
                return
            ty, flow = buf[0], struct.unpack("<I", buf[4:8])[0]
            if ty == 1:
                self.hello += 1
                add_event({"k": "host_ctl_hello", "peer_cid": peer[0]})
            elif ty == 3:
                if flow in self.open_flows:
                    self.paired_open += 1
                else:
                    self.paired_discarded += 1  # discarded and counted, no reply

    def send_abort(self, flow):
        with self.lock:
            c = self.sock
        c.sendall(bytes([2, 0, 0, 0]) + struct.pack("<I", flow))

    def kill(self):
        deadline = time.monotonic() + 3
        while self.sock is None and time.monotonic() < deadline:
            time.sleep(0.01)
        with self.lock:
            c, self.sock = self.sock, None
        c.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
        c.close()


SESSION = Session()
threading.Thread(target=rpt_server, args=(vsock_listener(HOST_RPT_PORT),), daemon=True).start()
threading.Thread(target=SESSION.serve, args=(vsock_listener(HOST_CTL_PORT),), daemon=True).start()

# ------------------------------------------------------------------ VM
class VM:
    def __init__(self, cid):
        self.cid = cid
        self.dir = OUT / f"v24-vm-{cid}-{int(time.time() * 1000)}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmd = [FORK_CH, "--kernel", GUEST_KERNEL, "--initramfs", str(INITRD),
               "--cmdline", "console=ttyS0 panic=-1 loglevel=4",
               "--cpus", "boot=2", "--memory", "size=1024M",
               "--serial", f"file={self.serial}", "--console", "off",
               "--log-file", str(self.dir / "ch.log"), "-v",
               "--vsock", f"cid={cid},backend=vhost-kernel"]
        log("launch: " + " ".join(cmd))
        self.cmd = cmd
        self.out = (self.dir / "ch.stdout").open("w")
        self.proc = subprocess.Popen(cmd, stdout=self.out, stderr=subprocess.STDOUT)
        self.pid = self.proc.pid

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


class Cmd:
    def __init__(self, cid):
        self.s = None
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
                s.settimeout(5)
                s.connect((cid, CMD_PORT))
                self.s = s
                break
            except OSError:
                time.sleep(0.2)
        assert self.s, "command channel never came up"
        self.f = self.s.makefile("rw", newline="\n")
        self.s.settimeout(60)

    def __call__(self, line):
        self.f.write(line + "\n")
        self.f.flush()
        return self.f.readline().strip()


def ename(e):
    return errno_mod.errorcode.get(e, str(e)) if e else "0"


# ------------------------------------------------------------------ case machinery
TRIAL = [1000]


def open_flow(cid, trial, request=True, linger=True, payload=b""):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(20)
    s.connect((cid, FLOW_PORT))
    if linger:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    if request:
        s.sendall(b"RQ24" + struct.pack("<III", trial, 8080, 0))
    if payload:
        s.sendall(payload)
    return s


def close_v(s):
    s.close()  # SO_LINGER{1,0} was set at creation when linger=True


def diag(cmd):
    return json.loads(cmd("diag"))


def hist(vals):
    c = collections.Counter(vals)
    return dict(sorted(c.items(), key=lambda kv: (-kv[1], str(kv[0]))))


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else None


def run_case(cmd, name, cause, queued, n, ctl_ignore, settle_ms=0, hold_ms=600, linger=True, request=True,
             payload=b"", paired_blind=False, sequential=False):
    log(f"=== case {name}: cause={cause} queued={queued} n={n} ctl_ignore={ctl_ignore} settle_ms={settle_ms} "
        f"hold_ms={hold_ms} linger={linger} request={request} payload={len(payload)} paired_blind={paired_blind} sequential={sequential}")
    cmd("hold off")
    cmd("reset")
    SESSION.paired_open = SESSION.paired_discarded = 0
    cmd(f"cfg label={name} hold_ms={hold_ms} paired_blind={int(paired_blind)} ctl_ignore={int(ctl_ignore)}")
    with CV:
        mark = len(EVENTS)
    ups0 = sum(1 for e in EVENTS if e.get("k") == "session_up")
    case = {"name": name, "cause": cause, "queued": queued, "n": n, "ctl_ignore": ctl_ignore, "settle_ms": settle_ms,
            "hold_ms": hold_ms, "linger": linger, "request": request, "payload_bytes": len(payload),
            "paired_blind": paired_blind, "sequential": sequential}
    trials, close_t = [], {}
    diag_before = diag_after_close = None

    def do_cause(batch, socks):
        t = time.monotonic()
        if cause == "abort":
            for tr, s in zip(batch, socks):
                SESSION.send_abort(tr)
                SESSION.open_flows.discard(tr)
                close_t[tr] = time.monotonic()
                close_v(s)
        elif cause == "quiescence":
            for tr, s in zip(batch, socks):
                SESSION.open_flows.discard(tr)
                close_t[tr] = time.monotonic()
                close_v(s)
        elif cause == "session-loss":
            SESSION.kill()
            for tr, s in zip(batch, socks):
                SESSION.open_flows.discard(tr)
                close_t[tr] = time.monotonic()
                close_v(s)
            want = ups0 + 1 + case.setdefault("_ups", 0)
            case["_ups"] += 1
            wait_for(lambda: sum(1 for e in EVENTS if e.get("k") == "session_up") >= want, 15, "session_up after loss")
        elif cause == "loss-reconnect-close":
            SESSION.kill()
            want = ups0 + 1 + case.setdefault("_ups", 0)
            case["_ups"] += 1
            wait_for(lambda: sum(1 for e in EVENTS if e.get("k") == "session_up") >= want, 15, "session_up after loss")
            for tr, s in zip(batch, socks):
                SESSION.open_flows.discard(tr)
                close_t[tr] = time.monotonic()
                close_v(s)
        elif cause == "none":
            pass
        else:
            raise ValueError(cause)
        return t

    if not sequential:
        batch = list(range(TRIAL[0], TRIAL[0] + n))
        TRIAL[0] += n
        trials = batch
        if queued:
            cmd("hold on")
        socks = []
        for tr in batch:
            SESSION.open_flows.add(tr)
            socks.append(open_flow(CID, tr, request=request, linger=linger, payload=payload if cause != "none" else b""))
        if queued:
            # wait until the guest kernel shows n queued V_g (never accepted)
            t0 = time.monotonic()
            while time.monotonic() - t0 < 15:
                d = diag(cmd)
                if d.get("flow_vsock_total", 0) >= n:
                    break
                time.sleep(0.1)
            diag_before = {k: d.get(k) for k in ("flow_vsock_total", "flow_vsock_by_state", "tcp_app_conns")}
        else:
            wait_for(lambda: sum(1 for e in EVENTS[mark:] if e.get("k") == "stage" and e.get("label") == name) >= n, 30, "stage events")
        if cause == "none":
            if queued:
                time.sleep(1.0)
                cmd("hold off")
                wait_for(lambda: SESSION.paired_open >= n, 30, "Paired for positive control")
            else:
                wait_for(lambda: SESSION.paired_open >= n, 30, "Paired for positive control")
            for s in socks:
                s.sendall(b"PAYLOAD-AFTER-PAIRED-0123456789")
            time.sleep(0.5)
            for tr, s in zip(batch, socks):
                SESSION.open_flows.discard(tr)
                close_t[tr] = time.monotonic()
                close_v(s)
        else:
            do_cause(batch, socks)
            time.sleep(settle_ms / 1000)
            if queued:
                d = diag(cmd)
                diag_after_close = {k: d.get(k) for k in ("flow_vsock_total", "flow_vsock_by_state", "tcp_app_conns")}
                cmd("hold off")
    else:
        cmd("hold on")
        for i in range(n):
            tr = TRIAL[0]
            TRIAL[0] += 1
            trials.append(tr)
            SESSION.open_flows.add(tr)
            s = open_flow(CID, tr, request=request, linger=linger, payload=payload)
            t0 = time.monotonic()
            while time.monotonic() - t0 < 5 and diag(cmd).get("flow_vsock_total", 0) < 1:
                time.sleep(0.005)
            do_cause([tr], [s])
            time.sleep(settle_ms / 1000)
            cmd("hold off")
            wait_for(lambda: any(e.get("k") == "flow" and e.get("trial") == tr for e in EVENTS[mark:]), 20, f"flow {tr}")
            cmd("hold on")
        cmd("hold off")
    # collect: wait for n flow reports and the app events
    wait_for(lambda: sum(1 for e in EVENTS[mark:] if e.get("k") == "flow" and e.get("label") == name) >= n, hold_ms / 1000 + 40, "flow reports")
    time.sleep(0.5)
    flows = [e for e in EVENTS[mark:] if e.get("k") == "flow" and e.get("label") == name]
    ports = {f["c_port"] for f in flows if "c_port" in f}
    wait_for(lambda: len({e["c_port"] for e in EVENTS[mark:] if e.get("k") == "app"} & ports) >= len(ports), 15, "app events")
    time.sleep(1.0)  # the row's "1 s after the case ends"
    d_end = diag(cmd)
    counters = json.loads(cmd("counters"))
    with CV:
        evs = list(EVENTS[mark:])
    apps = {e["c_port"]: e for e in evs if e.get("k") == "app"}
    flows = [e for e in evs if e.get("k") == "flow" and e.get("label") == name]
    stages = [e for e in evs if e.get("k") == "stage" and e.get("label") == name]
    # clock offset guest->host (min over stage events: the least-delayed report)
    off = min((s["h"] - s["t"] / 1e6 for s in stages), default=None)
    (EV / "cases" / f"{name}.jsonl").write_text("\n".join(json.dumps(e) for e in evs) + "\n")

    sig_obs, sig_pre, lat_app, lat_prop = [], [], [], []
    dec = []
    teardown_ok = 0
    bytes_on_c = 0
    violations = []
    for f in flows:
        if "error" in f:
            violations.append({"flow": f, "why": "worker error"})
            continue
        o, p = f["obs"], f["pre"]
        sig_obs.append(f"read={o['rd_rc']}/{ename(o['rd_err'])} write={o['wr_rc']}/{ename(o['wr_err'])} so_err={ename(o['so_err'])} rev={o['rev']}")
        sig_pre.append(f"rev={p['rev']} peek={p['peek_rc']}/{ename(p['peek_err'])} request_read={f['req']['n']}B/{ename(f['req']['err'])}/eof={f['req']['eof']}")
        dec.append(f["decision"])
        a = apps.get(f["c_port"])
        if a:
            bytes_on_c += a["bytes"]
            lat_app.append((a["first_t"] - f["t_accept"]) / 1000.0)
        if f["decision"].startswith("teardown") and a and a["first"] in ("reset", "eof") and a["bytes"] == 0:
            teardown_ok += 1
        elif cause != "none":
            violations.append({"trial": f["trial"], "decision": f["decision"], "app": a, "obs": o})
        ho = f["hold_obs"]
        if off is not None and ho["vg_event_t"] >= 0 and f["trial"] in close_t:
            lat_prop.append((ho["vg_event_t"] / 1e6 + off - close_t[f["trial"]]) * 1000.0)
    summary = {
        **{k: v for k, v in case.items() if not k.startswith("_")},
        "flow_reports": len(flows), "app_events": len(apps),
        "decisions": hist(dec),
        "state_at_accept_and_request_read": hist(sig_pre),
        "observation_after_hold": hist(sig_obs),
        "teardown_ok(decision=teardown & app reset/eof & 0 bytes on C)": teardown_ok,
        "bytes_read_by_app_on_C": bytes_on_c,
        "app_first_event": hist(a["first"] for a in apps.values()),
        "latency_guest_accept_to_app_first_event_ms": {
            "min": min(lat_app, default=None), "p50": pct(lat_app, .5), "p99": pct(lat_app, .99), "max": max(lat_app, default=None)},
        "vg_event_during_hold_count": sum(1 for f in flows if f.get("hold_obs", {}).get("vg_event_t", -1) >= 0),
        "latency_host_close_to_vg_event_ms(approx, clock offset ~ms)": {
            "n": len(lat_prop), "min": min(lat_prop, default=None), "p50": pct(lat_prop, .5), "p99": pct(lat_prop, .99), "max": max(lat_prop, default=None)},
        "host_paired_for_open_flow": SESSION.paired_open, "host_paired_discarded_no_reply": SESSION.paired_discarded,
        "guest_counters": counters,
        "diag_queued_before_close": diag_before, "diag_queued_after_close_before_accept": diag_after_close,
        "leftover_1s_after": {"flow_vsock_total": d_end.get("flow_vsock_total"), "flow_vsock_by_state": d_end.get("flow_vsock_by_state"),
                              "tcp_app_conns": d_end.get("tcp_app_conns"), "tcp_app_list": d_end.get("tcp_app_list"),
                              "all_vsock_total": d_end.get("all_vsock_total")},
        "violations_count": len(violations), "violations_first3": violations[:3],
    }
    CASES[name] = summary
    (EV / "case-summaries.json").write_text(json.dumps(CASES, indent=2, sort_keys=True))
    log(json.dumps({k: summary[k] for k in ("name", "flow_reports", "decisions", "state_at_accept_and_request_read",
                                              "observation_after_hold", "bytes_read_by_app_on_C", "app_first_event",
                                              "violations_count", "leftover_1s_after")}, indent=1))
    return summary


CASES = {}


def main_suite(vm, cmd):
    # positive controls first (prove the harness can see a live flow and bytes on C)
    run_case(cmd, "P-accepted-live", "none", False, 100, False, hold_ms=1000)
    run_case(cmd, "P-queued-live", "none", True, 100, False, hold_ms=0)
    for mode, ign in (("vgonly", True), ("ctl", False)):
        for cause in ("abort", "quiescence", "session-loss"):
            run_case(cmd, f"A-{cause}-{mode}", cause, False, N, ign)
        for cause in ("abort", "quiescence", "session-loss", "loss-reconnect-close"):
            run_case(cmd, f"B-{cause}-{mode}", cause, True, N, ign, settle_ms=200, hold_ms=0)
    # U-5: a Paired already in flight for a flow the host closed is discarded, no reply
    run_case(cmd, "R-paired-blind-quiescence", "quiescence", False, N, False, paired_blind=True)
    run_case(cmd, "R-paired-blind-abort", "abort", False, N, False, paired_blind=True)
    # edge cases (guest ignores control so only the V_g signal can end the flow)
    run_case(cmd, "E1-no-request-queued", "quiescence", True, 100, True, settle_ms=200, hold_ms=0, request=False)
    run_case(cmd, "E1b-no-request-accepted", "quiescence", False, 100, True, request=False)
    run_case(cmd, "E2-no-linger-queued", "quiescence", True, 100, True, settle_ms=200, hold_ms=0, linger=False)
    run_case(cmd, "E2b-no-linger-accepted", "quiescence", False, 100, True, linger=False)
    run_case(cmd, "E3-payload-before-close-queued", "quiescence", True, 100, True, settle_ms=200, hold_ms=0, payload=b"PAYLOAD-BEFORE-PAIRED-0123456789")
    run_case(cmd, "E4-queued-10s-settle", "quiescence", True, 100, True, settle_ms=10000, hold_ms=0)
    run_case(cmd, "E5-sequential-queued", "quiescence", True, 100, True, settle_ms=20, hold_ms=0, sequential=True)
    run_case(cmd, "E5b-sequential-queued-ctl", "session-loss", True, 30, False, settle_ms=20, hold_ms=0, sequential=True)


def smoke_suite(vm, cmd):
    run_case(cmd, "P-queued-live", "none", True, 5, False, hold_ms=0)
    run_case(cmd, "A-quiescence-vgonly", "quiescence", False, 5, True)
    run_case(cmd, "B-quiescence-vgonly", "quiescence", True, 5, True, settle_ms=200, hold_ms=0)
    run_case(cmd, "B-session-loss-ctl", "session-loss", True, 5, False, settle_ms=200, hold_ms=0)


# ------------------------------------------------------------------ run
vm = None
try:
    log(f"=== boot guest CID {CID}")
    vm = VM(CID)
    hello = wait_for(lambda: next((e for e in EVENTS if e.get("k") == "hello"), None), 90, "guest hello")
    assert hello, "guest never reported hello; serial:\n" + vm.serial_text()[-3000:]
    log(f"guest hello: {hello}")
    record("guest", "hello", hello)
    record("guest", "serial_boot", vm.serial_text()[:3000])
    assert hello["local_cid"] == CID, hello
    wait_for(lambda: SESSION.hello >= 1, 30, "control session")
    cmd = Cmd(CID)
    assert cmd("ping") == "pong"
    sh(f"{FORK_CH} --version; ss --vsock -a -n -p > {EV}/host-ss-vsock-start.txt; cat {EV}/host-ss-vsock-start.txt | head -20")
    record("guest", "diag_idle", diag(cmd))
    {"full": main_suite, "smoke": smoke_suite}[SUITE](vm, cmd)
    record("done", "ok", True)
finally:
    try:
        log("=== teardown")
        sh(f"ss --vsock -a -n -p > {EV}/host-ss-vsock-end.txt 2>&1; head -20 {EV}/host-ss-vsock-end.txt", check=False)
        try:
            cmd("poweroff")
        except Exception:
            pass
        if vm:
            try:
                vm.proc.wait(timeout=20)
            except Exception:
                pass
            vm.kill()
            vm.collect()
        RESULTS["cases"] = CASES
        (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))
        sh("pgrep -af 'cloud-hypervisor' | grep -v pgrep || true", check=False)
        # restore the physical module set: unload only modules this run caused to load
        for _ in range(3):
            for m in ["vsock_diag", "vhost_vsock", "vmw_vsock_virtio_transport", "vmw_vsock_virtio_transport_common",
                      "vsock_loopback", "vsock", "vhost", "vhost_iotlb"]:
                if m in modules() and m not in MODULES_START:
                    sh(f"rmmod {m}", check=False, quiet=True)
        (EV / "modules-after.txt").write_text("\n".join(modules()) + "\n")
        record("cleanup", "modules_added_and_left", sorted(set(modules()) - set(MODULES_START)))
    except Exception as e:  # keep the original failure visible
        log(f"teardown error: {e!r}")
