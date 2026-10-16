#!/usr/bin/env python3
"""V-4 probe (netns-density-295) - native metal run, under the metal lease (root).

Question: when the guest owner close()s V_g (the guest end of a flow vsock), what does the HOST see?
Boots a real guest (Cloud Hypervisor fork, vhost-kernel vsock). For each cell the harness
establishes one vsock connection (guest-opened `g2h` or host-opened `h2g`), arranges a pre-state
(clean exchange / unread bytes on either side / prior half-close), commands the guest to close(V_g),
and records, for THAT flow:
  * the host kernel's own packet record, from the vsock tracepoints
    virtio_transport_recv_pkt (every packet the host receives: op, flags) and
    virtio_transport_alloc_pkt (every packet the host sends), via tracefs with trace_clock=mono
    (same clock as CLOCK_MONOTONIC);
  * the host application's view: edge-triggered epoll wakes (time + mask), final poll mask, bytes
    still readable, drain result (EOF / errno), a write attempt, SO_ERROR.
A second phase ("long") holds the host end untouched for 10.5 s after the guest close, to see
whether anything arrives after the VSOCK_CLOSE_TIMEOUT (8 s) of the guest's zombie socket.
"""
import collections, fcntl, hashlib, json, os, pathlib, random, re, resource, select, shutil
import socket, statistics, struct, subprocess, sys, threading, time, errno as _errno

INC = pathlib.Path(__file__).resolve().parent
BASE = INC.parent
ROOT = BASE.parent.parent
EV = INC / "evidence"
OUT = BASE / "out" / INC.name
EV.mkdir(exist_ok=False)
OUT.mkdir(parents=True, exist_ok=True)
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
FORK_CH_DIR = ROOT / "vendors/cloud-hypervisor"
TRACEFS = pathlib.Path("/sys/kernel/tracing")

ARGS = dict(a.split("=", 1) for a in sys.argv[1:])
CID = int(ARGS.get("cid", "3270"))
N = int(ARGS.get("n", "100"))
LONG_M = int(ARGS.get("long_m", "50"))
LONG_WAIT = float(ARGS.get("long_wait", "10.5"))
WINDOW = float(ARGS.get("window", "0.15"))
SUITE = ARGS.get("suite", "full")
CMD_PORT = 52520
SMALL, LARGE = 1024, 131072

lock = open("/tmp/v4-netns-density-295.lock", "a+")
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
assert (TRACEFS / "events/vsock").exists(), "no vsock tracepoints in tracefs"

# ------------------------------------------------------------------ build
log("=== build")
env = os.environ.copy()
(BASE / "out").mkdir(exist_ok=True)
for name in ("cargo-home", "ch-target"):  # warm caches from the previous spike (copy, never modify)
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
GUEST = str(BASE / "out" / "guest-target/x86_64-unknown-linux-gnu/release/v4g")
sh(f"file {GUEST} {FORK_CH}; {FORK_CH} --version")

root = OUT / "initramfs-root"
if root.exists():
    shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]:
    (root / d).mkdir(parents=True)
shutil.copy2(shutil.which("busybox"), root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "sleep"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(GUEST, root / "v4g")
shutil.copy2(BASE / "guest-init.sh", root / "init")
os.chmod(root / "init", 0o755)
os.chmod(root / "v4g", 0o755)
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

# ------------------------------------------------------------------ host kernel packet record (tracefs)
TRACE_RAW = (EV / "host-vsock-trace.log").open("w")
TRACE = []   # parsed: dict(ts_us, ev, src_cid, src_port, dst_cid, dst_port, len, op, flags)
TRACE_LOCK = threading.Lock()
TRACE_RE = re.compile(r"\s(\d+\.\d+): (virtio_transport_recv_pkt|virtio_transport_alloc_pkt): "
                      r"(\d+):(\d+) -> (\d+):(\d+) len=(\d+) type=(\S+) op=(\S+) flags=(\S+)")
TRACE_STOP = threading.Event()


def tw(rel, val):
    (TRACEFS / rel).write_text(val)


def trace_setup():
    tw("tracing_on", "0")
    tw("trace_clock", "mono")
    tw("events/enable", "0")
    tw("events/vsock/virtio_transport_recv_pkt/enable", "1")
    tw("events/vsock/virtio_transport_alloc_pkt/enable", "1")
    tw("trace", "")
    tw("tracing_on", "1")
    record("trace", "trace_clock", (TRACEFS / "trace_clock").read_text().strip())
    record("trace", "buffer_size_kb", (TRACEFS / "buffer_size_kb").read_text().strip())


def trace_reader():
    f = open(TRACEFS / "trace_pipe", "rb", buffering=0)
    p = select.poll()
    p.register(f, select.POLLIN)
    buf = b""
    while not TRACE_STOP.is_set():
        if not p.poll(100):
            continue
        chunk = f.read(65536)
        if not chunk:
            continue
        buf += chunk
        *lines, buf = buf.split(b"\n")
        for l in lines:
            s = l.decode(errors="replace")
            TRACE_RAW.write(s + "\n")
            m = TRACE_RE.search(s)
            if m:
                d = dict(ts_us=int(m.group(1).replace(".", "")), ev="rx" if "recv" in m.group(2) else "tx",
                         src_cid=int(m.group(3)), src_port=int(m.group(4)), dst_cid=int(m.group(5)),
                         dst_port=int(m.group(6)), len=int(m.group(7)), op=m.group(9), flags=m.group(10))
                with TRACE_LOCK:
                    TRACE.append(d)
    f.close()


def trace_lost():
    txt = (TRACEFS / "per_cpu").exists() and "".join(
        (c / "stats").read_text() for c in sorted((TRACEFS / "per_cpu").iterdir()))
    return sum(int(x) for x in re.findall(r"overrun: (\d+)", txt or "")) if txt else None


def flow_events(gport, hport, since_us=0):
    with TRACE_LOCK:
        snap = list(TRACE)
    out = []
    for d in snap:
        if d["ts_us"] < since_us:
            continue
        ports = {d["src_port"], d["dst_port"]}
        if ports == {gport, hport} and {d["src_cid"], d["dst_cid"]} == {2, CID}:
            out.append(d)
    return out


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


def wait_for(pred, timeout):
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


def vsock_listener(port, backlog=128):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.bind((socket.VMADDR_CID_ANY, port))
    s.listen(backlog)
    return s


CMD = {"sock": None}
CMD_LOCK = threading.Lock()


def cmd_server(lsock):
    c, _ = lsock.accept()
    CMD["sock"] = c
    for line in c.makefile("r"):
        try:
            add_event(json.loads(line))
        except Exception as e:
            add_event({"k": "bad_report", "line": line[:200], "err": str(e)})
    add_event({"k": "cmd_channel_end"})


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


NEXT_ID = [1000]
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


# ------------------------------------------------------------------ flow set-up and pre-states
EPOLLRDHUP = select.EPOLLRDHUP
MASK_BITS = {"IN": select.EPOLLIN, "OUT": select.EPOLLOUT, "ERR": select.EPOLLERR, "HUP": select.EPOLLHUP,
             "RDHUP": EPOLLRDHUP}


def mbits(m):
    return "|".join(k for k, v in MASK_BITS.items() if m & v) or "0"


def errname(e):
    return _errno.errorcode.get(e, str(e)) if e else "0"


def host_peek(s):
    try:
        return len(s.recv(1 << 20, socket.MSG_PEEK | socket.MSG_DONTWAIT))
    except BlockingIOError:
        return 0
    except OSError as e:
        return -e.errno


def host_poll0(s):
    p = select.poll()
    p.register(s.fileno(), select.POLLIN | select.POLLOUT | select.POLLRDHUP)
    r = p.poll(0)
    return r[0][1] if r else 0


def guest_status(id):
    start = len(EVENTS)
    send_cmd(f"Q {id}")
    return wait_for(lambda: ev_find("status", id, start), 2)


def establish(est):
    """returns (id, host_sock, gport, hport, listener_or_None)"""
    id = new_id()
    if est == "g2h":
        P = 200000 + id
        lst = vsock_listener(P)
        lst.settimeout(5)
        send_cmd(f"OC {id} {P}")
        c = wait_for(lambda: ev_find("connected", id) or ev_find("connect_fail", id), 5)
        if not c or c["k"] != "connected":
            lst.close()
            return None
        s, _ = lst.accept()
        lst.close()
        return id, s, c["lport"], P
    PG = 300000 + id
    send_cmd(f"LS {id} {PG}")
    assert wait_for(lambda: ev_find("listening", id), 5)
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(5)
    s.connect((CID, PG))
    s.settimeout(None)
    a = wait_for(lambda: ev_find("accepted", id), 5)
    if not a:
        s.close()
        return None
    hport = s.getsockname()[1]
    return id, s, PG, hport


def wait_cond(fn, timeout=3.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        v = fn()
        if v:
            return v
        time.sleep(0.002)
    return None


def prestate(scen, size, id, s):
    """bring the flow to the scenario's pre-state. Returns dict of what was set up (None on failure)."""
    info = {}
    if scen == "clean":                       # request/response exchange fully consumed on both sides
        s.sendall(b"ping" * 4)
        assert wait_cond(lambda: guest_status(id)["peek"] == 16), "guest never got ping"
        start = len(EVENTS)
        send_cmd(f"RD {id}")
        assert wait_for(lambda: ev_find("read", id, start), 2)
        send_cmd(f"WR {id} 16")
        assert wait_for(lambda: ev_find("wrote", id), 2)
        got = b""
        s.settimeout(2)
        while len(got) < 16:
            got += s.recv(16 - len(got))
        s.settimeout(None)
    elif scen in ("guest-unread", "both-unread"):
        s.sendall(b"h" * size)
        assert wait_cond(lambda: guest_status(id)["peek"] == size), "guest never queued the bytes"
    if scen in ("host-unread", "both-unread"):
        start = len(EVENTS)
        send_cmd(f"WR {id} {size}")
        assert wait_for(lambda: ev_find("wrote", id, start), 5)
        assert wait_cond(lambda: host_peek(s) == size), "host never queued the bytes"
    if scen == "own-halfclose-first":         # guest shutdown(WR) -> host reads EOF -> then guest close
        send_cmd(f"SW {id}")
        assert wait_for(lambda: ev_find("shut_wr", id), 2)
        assert wait_cond(lambda: host_poll0(s) & EPOLLRDHUP), "host never saw RDHUP"
        info["host_eof_read"] = s.recv(16, socket.MSG_DONTWAIT) == b""
    if scen == "both-halfclosed":             # each side shutdown(WR) (0x2 both ways), then guest close
        s.shutdown(socket.SHUT_WR)
        send_cmd(f"SW {id}")
        assert wait_for(lambda: ev_find("shut_wr", id), 2)
        assert wait_cond(lambda: host_poll0(s) & EPOLLRDHUP), "host never saw RDHUP"
        assert wait_cond(lambda: guest_status(id)["poll"] & EPOLLRDHUP), "guest never saw RDHUP"
    if scen == "peer-halfclose-first":        # host shutdown(WR) -> guest sees RDHUP -> then guest close
        s.shutdown(socket.SHUT_WR)
        assert wait_cond(lambda: guest_status(id)["poll"] & EPOLLRDHUP), "guest never saw RDHUP"
    return info


SCENARIOS = [("clean", 0), ("guest-unread", SMALL), ("guest-unread", LARGE), ("host-unread", SMALL),
             ("host-unread", LARGE), ("both-unread", SMALL), ("both-unread", LARGE),
             ("own-halfclose-first", 0), ("peer-halfclose-first", 0), ("both-halfclosed", 0)]


def cell_name(est, scen, size):
    return f"{est}/{scen}" + (f"/{size // 1024}KiB" if size else "")


CASES = collections.defaultdict(list)


def seq(evs, ev, t_from_us):
    return [f"{d['op']}:{d['flags']}" if d["op"] in ("SHUTDOWN", "RST") else d["op"]
            for d in evs if d["ev"] == ev and d["ts_us"] >= t_from_us]


def run_trial(est, scen, size):
    cell = cell_name(est, scen, size)
    t_est_us = time.monotonic_ns() // 1000
    r = establish(est)
    rec = {"cell": cell}
    if not r:
        rec["result"] = "SETUP-FAIL"
        TRIALS_LOG.write(json.dumps(rec) + "\n")
        CASES[cell].append(rec)
        return
    id, s, gport, hport = r
    rec.update(id=id, gport=gport, hport=hport)
    try:
        info = prestate(scen, size, id, s)
        rec["prestate"] = info
        gst = guest_status(id)
        pre_mask = host_poll0(s)
        rec["pre"] = {"host_poll": mbits(pre_mask), "host_peek": host_peek(s), "guest_peek": gst["peek"],
                      "guest_poll": gst["poll"]}
        ep = select.epoll()
        ep.register(s.fileno(), select.EPOLLIN | select.EPOLLRDHUP | select.EPOLLHUP | select.EPOLLERR | select.EPOLLET)
        pre_wakes = ep.poll(0)   # consume the initial readiness report of the edge-triggered registration
        rec["pre"]["epoll_initial"] = [mbits(m) for _, m in pre_wakes]
        time.sleep(0.03)         # let in-flight credit updates / acks settle: anything later is attributable
        off = calibrate()
        pre_us = (time.monotonic_ns()) // 1000
        t_cmd = time.monotonic_ns()
        send_cmd(f"CL {id}")
        wakes = []
        deadline = t_cmd + int(WINDOW * 1e9)
        while True:
            left = (deadline - time.monotonic_ns()) / 1e9
            if left <= 0:
                break
            for _, m in ep.poll(left):
                wakes.append((time.monotonic_ns(), m))
        closed = ev_find("closed", id)
        # host application's state at the end of the window, BEFORE the host closes anything
        end = {"poll": mbits(host_poll0(s)), "peek": host_peek(s)}
        drained, how = 0, "again"
        while True:
            try:
                b = s.recv(1 << 20, socket.MSG_DONTWAIT)
                if not b:
                    how = "EOF"
                    break
                drained += len(b)
            except BlockingIOError:
                how = "EAGAIN"
                break
            except OSError as e:
                how = errname(e.errno)
                break
        end.update(drained=drained, drain_end=how)
        try:
            w = s.send(b"x", socket.MSG_NOSIGNAL | socket.MSG_DONTWAIT)
            end["write"] = "ok"
        except OSError as e:
            end["write"] = errname(e.errno)
        end["so_error"] = errname(s.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR))
        end["poll_after_drain"] = mbits(host_poll0(s))
        rec["end"] = end
        t_hclose = time.monotonic_ns()
        ep.close()
        s.close()
        s = None
        time.sleep(0.05)
        evs = flow_events(gport, hport, t_est_us - 1000)
        t0h = (closed["t0"] - off) if closed else None   # guest close() entry on the host clock
        rec["guest_close_rc"] = (closed["rc"], closed["t1"] - closed["t0"]) if closed else None
        rec["wakes"] = [{"dt_us": (t - t_cmd) / 1000.0, "mask": mbits(m)} for t, m in wakes]
        cmd_us = t_cmd // 1000
        rec["pre_ops"] = dict(collections.Counter(f"{d['ev']}:{d['op']}" + (f":{d['flags']}" if d["op"] in ("SHUTDOWN", "RST") else "")
                                                  for d in evs if d["ts_us"] < cmd_us))
        after_close = []
        for d in evs:
            if d["ts_us"] < cmd_us:
                continue
            ts_ns = d["ts_us"] * 1000
            after_close.append({"ev": d["ev"], "op": d["op"], "flags": d["flags"], "len": d["len"],
                                "rel_guest_close_us": (ts_ns - t0h) / 1000.0 if t0h else None,
                                "after_host_close": ts_ns >= t_hclose})
        rec["post_events"] = after_close
        rx_end = [a for a in after_close if a["ev"] == "rx" and a["op"] in ("SHUTDOWN", "RST") and not a["after_host_close"]]
        rec["host_rx_end_records"] = [f"{a['op']}:{a['flags']}" for a in rx_end]
        rec["host_tx_before_hclose"] = [f"{a['op']}:{a['flags']}" for a in after_close if a["ev"] == "tx" and not a["after_host_close"]
                                        and a["op"] in ("SHUTDOWN", "RST")]
        rec["host_tx_after_hclose"] = [f"{a['op']}:{a['flags']}" for a in after_close if a["ev"] == "tx" and a["after_host_close"]
                                       and a["op"] in ("SHUTDOWN", "RST")]
        if rx_end and t0h:
            rec["close_to_rx_record_us"] = rx_end[0]["rel_guest_close_us"]
        tx_rst = [a for a in after_close if a["ev"] == "tx" and a["op"] == "RST" and not a["after_host_close"]]
        if tx_rst and rx_end:
            rec["host_rst_tx_after_rx_record_us"] = tx_rst[0]["rel_guest_close_us"] - rx_end[0]["rel_guest_close_us"]
        hw = [w for w in wakes if w[1] & (select.EPOLLHUP | select.EPOLLERR)]
        any_w = wakes[0] if wakes else None
        if t0h:
            if any_w:
                rec["close_to_first_wake_us"] = (any_w[0] - t0h) / 1000.0
                if rx_end:
                    rec["rx_record_to_first_wake_us"] = rec["close_to_first_wake_us"] - rx_end[0]["rel_guest_close_us"]
            if hw:
                rec["close_to_hup_wake_us"] = (hw[0][0] - t0h) / 1000.0
        # end indications that reached the host BEFORE the close command (0x2 expected only in own-halfclose-first)
        rec["early_end_records"] = [f"{d['op']}:{d['flags']}" for d in evs if d["ev"] == "rx" and d["op"] in ("SHUTDOWN", "RST")
                                    and d["ts_us"] < cmd_us]
        rec["early_hup_pre_mask"] = bool(pre_mask & (select.EPOLLHUP | select.EPOLLERR))
        rec["result"] = "OK"
    except Exception as e:
        rec["result"] = f"EXC {e!r}"
    finally:
        if s:
            s.close()
        send_cmd(f"R {id}")
    TRIALS_LOG.write(json.dumps(rec) + "\n")
    TRIALS_LOG.flush()
    CASES[cell].append(rec)


def pct(vals, p):
    vals = sorted(vals)
    return vals[min(len(vals) - 1, int(round(p / 100.0 * (len(vals) - 1))))] if vals else None


def dist(vals):
    return {"n": len(vals), "min": min(vals), "p50": pct(vals, 50), "p99": pct(vals, 99), "max": max(vals)} if vals else None


def summarize(cell):
    t = CASES[cell]
    ok = [r for r in t if r.get("result") == "OK"]
    c = collections.Counter
    s = {
        "trials": len(t), "ok": len(ok), "not_ok": dict(c(r["result"] for r in t if r.get("result") != "OK")),
        "host_rx_end_records": dict(c(str(r["host_rx_end_records"]) for r in ok)),
        "host_tx_end_records_before_hostclose": dict(c(str(r["host_tx_before_hclose"]) for r in ok)),
        "host_tx_end_records_after_hostclose": dict(c(str(r["host_tx_after_hclose"]) for r in ok)),
        "early_end_records_pre_close": dict(c(str(r["early_end_records"]) for r in ok)),
        "pre_state_host_poll": dict(c(r["pre"]["host_poll"] for r in ok)),
        "wakes_in_window": dict(c(len(r["wakes"]) for r in ok)),
        "wake_mask_sequence": dict(c(" > ".join(w["mask"] for w in r["wakes"]) for r in ok)),
        "end_poll": dict(c(r["end"]["poll"] for r in ok)),
        "end_peek_bytes_unread": dict(c(r["end"]["peek"] for r in ok)),
        "drain": dict(c(f"{r['end']['drained']}B/{r['end']['drain_end']}" for r in ok)),
        "write": dict(c(r["end"]["write"] for r in ok)),
        "so_error": dict(c(r["end"]["so_error"] for r in ok)),
        "guest_close_rc_dur_us": dist([r["guest_close_rc"][1] / 1000.0 for r in ok if r.get("guest_close_rc")]),
        "close_to_rx_record_us": dist([r["close_to_rx_record_us"] for r in ok if "close_to_rx_record_us" in r]),
        "close_to_first_wake_us": dist([r["close_to_first_wake_us"] for r in ok if "close_to_first_wake_us" in r]),
        "rx_record_to_first_wake_us": dist([r["rx_record_to_first_wake_us"] for r in ok if "rx_record_to_first_wake_us" in r]),
        "record_not_before_wake": sum(1 for r in ok if r.get("rx_record_to_first_wake_us", 1) < 0),
        "host_rst_tx_after_rx_record_us": dist([r["host_rst_tx_after_rx_record_us"] for r in ok if "host_rst_tx_after_rx_record_us" in r]),
        "pre_close_ops_seen_by_host": sorted(set(k for r in ok for k in r["pre_ops"])),
        "close_rc": dict(c(str(r["guest_close_rc"][0]) for r in ok if r.get("guest_close_rc"))),
        "close_to_hup_wake_us": dist([r["close_to_hup_wake_us"] for r in ok if "close_to_hup_wake_us" in r]),
    }
    record("cells", cell, s)
    log(f"{cell}: {json.dumps(s)}")


# ------------------------------------------------------------------ long phase
def long_phase():
    """Hold M flows per cell with the host end untouched for LONG_WAIT s after the guest close."""
    log(f"=== long phase: {LONG_M} flows per cell, host end untouched for {LONG_WAIT}s")
    flows = []
    for est in ("g2h", "h2g"):
        for scen, size in SCENARIOS:
            for _ in range(LONG_M):
                r = establish(est)
                if not r:
                    continue
                id, s, gport, hport = r
                try:
                    prestate(scen, size, id, s)
                except Exception as e:
                    log(f"long setup failure {cell_name(est, scen, size)}: {e!r}")
                    s.close()
                    continue
                flows.append(dict(cell=cell_name(est, scen, size), id=id, s=s, gport=gport, hport=hport))
    log(f"long phase: {len(flows)} flows established")
    time.sleep(0.2)
    off = calibrate()
    t_close_start = time.monotonic_ns()
    for f in flows:
        f["t_cmd"] = time.monotonic_ns()
        send_cmd(f"CL {f['id']}")
    wait_for(lambda: sum(1 for e in EVENTS if e.get("k") == "closed" and e["id"] in {f["id"] for f in flows}) >= len(flows), 20)
    closed = {e["id"]: e for e in EVENTS if e.get("k") == "closed"}
    time.sleep(LONG_WAIT)
    t_end = time.monotonic_ns()
    per_cell = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))
    late = collections.defaultdict(list)
    for f in flows:
        evs = flow_events(f["gport"], f["hport"], t_close_start // 1000 - 100000)
        c = closed.get(f["id"])
        t0h = (c["t0"] - off) if c else f["t_cmd"]
        seqs = []
        for d in evs:
            rel = (d["ts_us"] * 1000 - t0h) / 1e6   # ms
            if d["op"] in ("SHUTDOWN", "RST"):
                seqs.append((d["ev"], f"{d['op']}:{d['flags']}", round(rel, 3)))
        s = f["s"]
        endst = {"poll": mbits(host_poll0(s)), "peek": host_peek(s)}
        b = b""
        how = "again"
        try:
            while True:
                x = s.recv(1 << 20, socket.MSG_DONTWAIT)
                if not x:
                    how = "EOF"
                    break
                b += x
        except BlockingIOError:
            how = "EAGAIN"
        except OSError as e:
            how = errname(e.errno)
        endst.update(drained=len(b), drain_end=how, so_error=errname(s.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR)))
        pc = per_cell[f["cell"]]
        pc["end_record_sequence_ops"][" , ".join(f"{e}:{o}" for e, o, _ in seqs)] += 1
        pc["host_state_at_10s"][json.dumps(endst, sort_keys=True)] += 1
        for e, o, rel in seqs:
            if rel > 1000:
                late[f["cell"]].append((e, o, rel))
        f["seqs"] = seqs
        f["endst"] = endst
    res = {}
    for cell, d in per_cell.items():
        rx = [rel for f in flows if f["cell"] == cell for e, o, rel in f["seqs"] if e == "rx" and rel < 1000]
        late_rel = [rel for (e, o, rel) in late.get(cell, [])]
        res[cell] = {k: dict(v) for k, v in d.items()}
        res[cell]["n_flows"] = sum(1 for f in flows if f["cell"] == cell)
        res[cell]["first_end_record_rx_ms_after_close"] = dist(rx)
        res[cell]["end_records_after_1s"] = {"n": len(late_rel), "ms_after_close": dist(late_rel),
                                              "which": dict(collections.Counter(f"{e}:{o}" for e, o, _ in late.get(cell, [])))}
    record("long", "per_cell", res)
    (EV / "long-flows.jsonl").write_text("\n".join(json.dumps({k: v for k, v in f.items() if k != "s"}, default=str) for f in flows))
    for cell in sorted(res):
        log(f"LONG {cell}: {json.dumps(res[cell])}")
    for f in flows:
        f["s"].close()
        send_cmd(f"R {f['id']}")


vm = None
try:
    log(f"=== boot guest CID {CID}")
    trace_setup()
    threading.Thread(target=trace_reader, daemon=True).start()
    vm = VM(CID)
    hello = wait_for(lambda: ev_find("hello"), 90)
    assert hello, "guest never reported hello; serial:\n" + vm.serial_text()[-3000:]
    log(f"guest hello: {hello}")
    record("guest", "hello", hello)
    record("guest", "serial_boot", vm.serial_text()[:3000])
    record("guest", "host_uname_r", os.uname().release)
    sh(f"{FORK_CH} --version")
    n = 3 if SUITE == "smoke" else N
    for est in ("g2h", "h2g"):
        for scen, size in SCENARIOS:
            cell = cell_name(est, scen, size)
            for _ in range(n):
                run_trial(est, scen, size)
            summarize(cell)
    if SUITE != "nolong":
        if SUITE == "smoke":
            LONG_M = 2
            LONG_WAIT = 9.0
        long_phase()
    record("calibration", "clock_offset_error_bound_us", {
        "median": statistics.median(CAL_ERR) / 1000.0, "max": max(CAL_ERR) / 1000.0, "n": len(CAL_ERR)})
    record("trace", "total_parsed_events", len(TRACE))
    record("trace", "overrun_events_lost", trace_lost())
    record("done", "ok", True)
finally:
    try:
        log("=== teardown")
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
        TRACE_STOP.set()
        try:
            tw("tracing_on", "0")
            tw("events/vsock/virtio_transport_recv_pkt/enable", "0")
            tw("events/vsock/virtio_transport_alloc_pkt/enable", "0")
            tw("trace_clock", "local")
        except Exception as e:
            log(f"trace restore error: {e!r}")
        sh(f"ss --vsock -a -n -p > {EV}/host-ss-vsock-end.txt 2>&1; head -5 {EV}/host-ss-vsock-end.txt", check=False)
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
