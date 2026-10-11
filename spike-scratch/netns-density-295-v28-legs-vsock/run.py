#!/usr/bin/env python3
"""v28 native-metal driver (root, under the metal lease; launched by launch.py).

Real Cloud Hypervisor microVM (fork 41a619d19, --vsock backend=vhost-kernel, stock host AND guest kernel).
Guest PID 1 starts `legs agent`; this driver configures the guest like ADR-0150 (workload address on lo,
fwmark rule -> local table, fwmark_reflect, tcp_fwmark_accept), starts the guest receiver `legs gagent`
and per-scenario `legs gapp`, and the host `legs relay` / `legs client` / `legs probe`.
All state in /tmp/v28-work; every name is v28-prefixed or a unique port/CID.

Usage: run.py <stage>   (stage: smoke | half | peer | probes | bench | death)
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, struct, subprocess, time

W = pathlib.Path("/tmp/v28-work"); OUT = W / "out"
STAGE = sys.argv[1]; ARGS = sys.argv[2:]
EOF = os.environ.get("V28_EOF", "clean")
PAUSE = int(os.environ.get("V28_PAUSE", "5"))
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"{STAGE}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 4280
LEGS, CH = OUT / "legs", OUT / "cloud-hypervisor"
CERT = str(OUT)
WL = "10.77.0.2"            # guest workload address (on lo, like the dummy device of ADR-0150)
GW = "198.18.28.254"        # "guest gateway" address a host-local platform client presents (non-local in the guest)
CLI_SRC = "198.18.28.1"     # source of the stand-in originating agent (leg B), local on host lo
RELAY_PORT = 18428
HOSTAPP_PORT = 19428
PROCS = []
EV.mkdir(parents=True, exist_ok=False)
LOG = (EV / "run.log").open("a")
RESULTS = {}


def log(m):
    line = f"[{time.strftime('%H:%M:%S')}] {m}"; print(line, flush=True); LOG.write(line + "\n"); LOG.flush()


def sh(cmd, check=True, timeout=600, quiet=False):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, errors="replace")
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": time.time(), "cmd": cmd, "rc": p.returncode, "out": p.stdout[-6000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(cmd)} (rc={p.returncode})")
        for l in p.stdout.rstrip().splitlines()[-25:]: log("    " + l)
    if check and p.returncode: raise RuntimeError(f"failed rc={p.returncode}: {cmd}")
    return p.stdout


def preflight():
    log("=== preflight (fail closed)")
    fail = []
    if os.uname().machine != "x86_64": fail.append("not x86_64")
    virt = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True)
    if virt.returncode != 1 or virt.stdout.strip() != "none": fail.append(f"systemd-detect-virt={virt.stdout.strip()!r}")
    cpu = pathlib.Path("/proc/cpuinfo").read_text()
    if not re.search(r"^flags.*\b(vmx|svm)\b", cpu, re.M): fail.append("no vmx/svm")
    if re.search(r"^flags.*\bhypervisor\b", cpu, re.M): fail.append("hypervisor flag")
    try:
        import stat
        if not stat.S_ISCHR(os.stat("/dev/kvm").st_mode): fail.append("/dev/kvm not chardev")
        k = os.open("/dev/kvm", os.O_RDWR); api = fcntl.ioctl(k, 0xAE00); v = fcntl.ioctl(k, 0xAE01, 0); os.close(v); os.close(k)
        if api != 12: fail.append(f"kvm api {api}")
    except Exception as e:
        fail.append(f"kvm probe {e!r}")
    pre = {"uname_r": os.uname().release, "machine": os.uname().machine, "systemd_detect_virt": virt.stdout.strip(),
           "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "nproc": os.cpu_count()}
    (EV / "substrate-probes.json").write_text(json.dumps({**pre, "failures": fail}, indent=1))
    log(f"substrate: {json.dumps(pre)}")
    if fail:
        log(f"FAILED CLOSED: {fail}"); sys.exit(3)
    return pre


def sha(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


def copy_with_libs(root, binary):
    dst = root / binary.lstrip("/"); dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(os.path.realpath(binary), dst)
    for line in subprocess.run(["ldd", os.path.realpath(binary)], text=True, capture_output=True).stdout.splitlines():
        m = re.search(r"(/[^ ]+) \(0x", line)
        if m:
            lib = m.group(1)
            for cand in {lib, os.path.realpath(lib)}:
                d = root / cand.lstrip("/"); d.parent.mkdir(parents=True, exist_ok=True)
                if not d.exists(): shutil.copy2(os.path.realpath(lib), d)


def build_image():
    log("=== guest image")
    root = OUT / "initramfs-root"
    if root.exists(): shutil.rmtree(root)
    for d in ["proc", "sys", "dev", "bin", "sbin", "usr/bin", "usr/sbin", "tmp", "mods", "lib64", "etc"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for n in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "touch", "env", "true", "false", "ps", "id", "head", "tail", "wc", "date", "pkill", "grep", "pidof"]:
        (root / "bin" / n).symlink_to("busybox")
    for b in ["/usr/sbin/ip", "/usr/bin/ss"]:
        if os.path.exists(b): copy_with_libs(root, b)
    shutil.copy2(LEGS, root / "legs")
    (root / "init").write_text("""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /tmp /sys/fs/bpf /sys/kernel/debug /sys/kernel/tracing
mount -t tmpfs tmpfs /tmp
mount -t bpf bpf /sys/fs/bpf
mount -t debugfs none /sys/kernel/debug 2>/dev/null
mount -t tracefs none /sys/kernel/tracing 2>/dev/null
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
ip link set lo up
ulimit -n 1048576
/legs agent > /tmp/agent.out 2>&1 &
echo "GUEST_READY"
while true; do sleep 3600; done
""")
    os.chmod(root / "init", 0o755)
    order, pins = [], {}
    for line in sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"], quiet=True).splitlines():
        if line.startswith("insmod "):
            src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
            if name in order: continue
            data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
            (root / "mods" / name).write_bytes(data); order.append(name); pins[str(src)] = sha(src)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}", quiet=True)
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "legs": sha(LEGS), "cloud_hypervisor": sha(CH)})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=1, sort_keys=True))
    log(f"image pins: {json.dumps({k: v[:12] for k, v in pins.items() if not k.startswith('/')})}")


class VM:
    def __init__(self, cpus=4, mem=2048):
        self.dir = OUT / f"vm-{STAGE}-{STAMP}"; self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", "console=ttyS0 panic=-1 loglevel=4",
               "--cpus", f"boot={cpus}", "--memory", f"size={mem}M", "--serial", f"file={self.serial}", "--console", "off",
               "--vsock", f"cid={CID},backend=vhost-kernel", "--log-file", str(self.dir / "ch.log"), "-v"]
        log("launch VM: " + " ".join(cmd))
        self.proc = subprocess.Popen(cmd, stdout=(self.dir / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True)

    def text(self): return self.serial.read_text(errors="replace") if self.serial.exists() else ""

    def wait_ready(self, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            if "GUEST_READY" in self.text(): return True
            if self.proc.poll() is not None: return False
            time.sleep(0.2)
        return False

    def stop(self):
        if self.proc.poll() is None:
            try: os.killpg(self.proc.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            try: self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired: pass
        d = EV / "vm"; d.mkdir(exist_ok=True)
        for f in self.dir.iterdir():
            if f.is_file(): shutil.copy2(f, d / f.name)


def gx(cmd, timeout=120):
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(timeout)
    out, rc = b"", None
    try:
        s.connect((CID, 4000)); b = cmd.encode(); s.sendall(struct.pack(">I", len(b)) + b)
        while True:
            c = s.recv(65536)
            if not c: break
            out += c
            m = re.search(rb"\nAGENT_RC=(-?\d+)\n", out)
            if m: rc = int(m.group(1)); out = out[:m.start()]; break
    except Exception as e:
        out += f"\nHARNESS_EXCEPTION {e!r}".encode()
    finally: s.close()
    t = out.decode(errors="replace")
    with (EV / "guest-commands.jsonl").open("a") as f: f.write(json.dumps({"t": time.time(), "cmd": cmd, "rc": rc, "out": t[-4000:]}) + "\n")
    return rc, t


def spawn(name, argv, env=None):
    f = (EV / f"proc-{name}.log").open("w")
    p = subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, start_new_session=True, env=env); p.name = name; PROCS.append(p); return p


def reap(p, sig=signal.SIGKILL):
    if p.poll() is None:
        try: os.killpg(p.pid, sig)
        except ProcessLookupError: pass
        try: p.wait(timeout=8)
        except subprocess.TimeoutExpired: pass


def wait_text(path, pat, timeout=30):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if pat in pathlib.Path(path).read_text(): return True
        except FileNotFoundError: pass
        time.sleep(0.1)
    return False


def jl(text):
    out = []
    for l in text.splitlines():
        l = l.strip()
        if l.startswith("{"):
            try: out.append(json.loads(l))
            except Exception: pass
    return out


# ---------------------------------------------------------------- guest/host service helpers
def gagent_start(eof):
    gx("/legs pk 9 gagent; sleep 0.2; true")
    rc, t = gx(f"(setsid /legs gagent --eof-class {eof} --log /tmp/gagent.jsonl > /tmp/gagent.out 2>&1 < /dev/null &) ; sleep 0.5; : > /tmp/gagent.jsonl; cat /tmp/gagent.out")
    log(f"gagent eof-class={eof} started {t.strip()[:100]}")


def guest_setup():
    cmds = [
        f"ip addr add {WL}/32 dev lo",
        "echo 1 > /proc/sys/net/ipv4/tcp_fwmark_accept",
        "echo 1 > /proc/sys/net/ipv4/fwmark_reflect",
        "for f in /proc/sys/net/ipv4/conf/*/rp_filter; do echo 0 > $f; done",
        "ip rule add fwmark 1 lookup 100",
        "ip route add local 0.0.0.0/0 dev lo table 100",
        "ip -br addr; ip rule; ip route show table 100; uname -r",
    ]
    for c in cmds:
        rc, t = gx(c); log(f"guest$ {c} rc={rc} {t.strip()[:300]}")
    gagent_start(EOF)


def gapp_start(label, *args, port=8080, listen=None):
    gapp_stop()
    listen = listen or f"{WL}:{port}"
    gx(f"rm -f /tmp/app.jsonl; (setsid /legs gapp --listen {listen} --log /tmp/app.jsonl {' '.join(args)} > /tmp/app.out 2>&1 < /dev/null &) ; sleep 0.3")
    rc, t = gx("cat /tmp/app.jsonl /tmp/app.out")
    ok = '"listening"' in t
    log(f"gapp[{label}] {'listening' if ok else 'NOT LISTENING: ' + t[:200]}")
    return ok


def gapp_stop(sig="9"):
    gx(f"/legs pk {sig} gapp; sleep 0.2; true")


def gapp_log():
    rc, t = gx("cat /tmp/app.jsonl"); return jl(t)


def gagent_log():
    rc, t = gx("cat /tmp/gagent.jsonl"); return jl(t)


class Relay:
    def __init__(self, label, legs="vsock", fwd="splice", peer_mode="gateway", native=False, up=0, down=0, dst=None, port=RELAY_PORT):
        eof = EOF
        self.label = label; self.port = port
        self.events = EV / f"relay-{label}.jsonl"
        dst = dst or f"{WL}:8080"
        argv = [str(LEGS), "relay", "--dir", CERT, "--listen", f"127.0.0.1:{port}", "--legs", legs, "--cid", str(CID), "--dst", dst,
                "--fwd", fwd, "--eof-class", eof, "--peer-mode", peer_mode, "--gateway", GW, "--events", str(self.events)]
        if native: argv += ["--native", "--up", str(up), "--down", str(down)]
        self.p = spawn(f"relay-{label}", argv)
        if not wait_text(EV / f"proc-relay-{label}.log", "RELAY_READY"): raise RuntimeError("relay not ready")

    def cpu_s(self):
        try:
            f = pathlib.Path(f"/proc/{self.p.pid}/stat").read_text().rsplit(")", 1)[1].split()
            return (int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK")
        except Exception:
            return None

    def stop(self):
        self.cpu_final = self.cpu_s()
        reap(self.p)

    def ev(self):
        try: return jl(self.events.read_text())
        except FileNotFoundError: return []


def client(*args, tag="", timeout=60, src=CLI_SRC, port=RELAY_PORT, bg=False):
    argv = [str(LEGS), "client", "--dir", CERT, "--connect", f"127.0.0.1:{port}", "--tag", tag, *map(str, args)]
    if src: argv += ["--src", src]
    if bg:
        return subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True)
    t0 = time.time()
    try:
        p = subprocess.run(argv, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        r = jl(p.stdout); res = r[-1] if r else {"error": p.stdout[-400:]}
    except subprocess.TimeoutExpired:
        res = {"HANG": True, "tag": tag}
    res["wall_s"] = round(time.time() - t0, 3)
    return res


def collect(p, timeout=60):
    try:
        out, _ = p.communicate(timeout=timeout)
        r = jl(out); return r[-1] if r else {"error": out[-400:]}
    except subprocess.TimeoutExpired:
        try: os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        p.communicate()
        return {"HANG": True}


def probe(kind, port, timeout_ms=3000):
    p = subprocess.run([str(LEGS), "probe", "--cid", str(CID), "--dst", f"{WL}:{port}", "--kind", kind, "--timeout-ms", str(timeout_ms), "--gateway", GW],
                       text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    r = jl(p.stdout); return r[-1] if r else {"error": p.stdout[-300:]}


def record(name, **kw):
    RESULTS[name] = kw
    log(f"RESULT {name}: {json.dumps(kw, default=str)[:1500]}")
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=1, default=str))


# ---------------------------------------------------------------- stages
N = 3_000_001
M = 2_000_003
BIG = 16 * 1024 * 1024


def one(name, relay_kw, app_args, cli_args, tcp=False, settle=0.4):
    """run one scenario; returns (client result, app log, relay events, gagent log)"""
    if tcp:
        app = spawn(f"hostapp-{name}", [str(LEGS), "gapp", "--listen", f"127.0.0.1:{HOSTAPP_PORT}", "--log", str(EV / f"hostapp-{name}.jsonl"), *app_args])
        wait_text(EV / f"hostapp-{name}.jsonl", "listening")
        rl = Relay(name, legs="tcp", dst=f"127.0.0.1:{HOSTAPP_PORT}", **relay_kw)
    else:
        gx(": > /tmp/gagent.jsonl")
        if not gapp_start(name, *app_args): return None
        rl = Relay(name, **relay_kw)
    c = client(*cli_args, tag=name)
    time.sleep(settle)
    if tcp:
        try: al = jl((EV / f"hostapp-{name}.jsonl").read_text())
        except FileNotFoundError: al = []
        reap(app)
        gl = []
    else:
        al = gapp_log(); gl = gagent_log(); gapp_stop()
    ev = rl.ev(); rl.stop()
    return c, al, ev, gl


def conn_ev(al):
    return [x for x in al if x.get("ev") == "conn"]


def stage_smoke():
    for tcp in (True, False):
        r = one("smoke-" + ("tcp" if tcp else "vsock"), {}, ["--send", str(M), "--end", "fin"], ["--up", N, "--down", M, "--end", "fin"], tcp=tcp)
        c, al, ev, gl = r
        record("smoke-" + ("tcp" if tcp else "vsock"), client=c, app=conn_ev(al), relay=[e for e in ev if e.get("ev") in ("relay_paired", "relay_done", "pump_end", "vsock_eof_classify", "tp_classify")], gagent=gl)


def stage_half():
    sizes = dict(N=N, M=M)
    for tcp in (False, True):
        sfx = "tcp" if tcp else "vsock"
        def go(name, app_args, cli_args, **kw):
            r = one(f"{name}-{sfx}", {}, app_args, cli_args, tcp=tcp, **kw)
            if r is None: record(f"{name}-{sfx}", error="app not listening"); return
            c, al, ev, gl = r
            record(f"{name}-{sfx}", client=c, app=conn_ev(al), relay=[e for e in ev if e.get("ev") in ("relay_paired", "relay_done", "relay_legs_not_paired", "pump_end", "vsock_eof_classify", "tp_classify")],
                   gagent=[g for g in gl if g.get("ev") in ("gagent_paired", "gagent_done", "gagent_not_paired", "pump_end", "vsock_eof_classify", "tp_classify")])
        # H1 client half-closes (FIN, no close_notify) before the app answers
        go("H1-client-fin-then-reply", ["--send", str(M), "--send-after-read", "--end", "close"], ["--up", N, "--down", M, "--end", "fin"])
        # H1b client sends TLS close_notify then FIN
        go("H1b-client-close-notify", ["--send", str(M), "--send-after-read", "--end", "close"], ["--up", N, "--down", M, "--end", "notify"])
        # H2 app half-closes first (FIN after sending), client keeps sending after it sees EOF
        go("H2-app-fin-first-client-continues", ["--send", str(M), "--end", "fin"], ["--up", N, "--down", M, "--end", "fin", "--send-after-eof"])
        # H3 simultaneous both directions then both fin
        go("H3-simultaneous", ["--send", str(M), "--end", "fin"], ["--up", N, "--down", M, "--end", "fin"])
        # H4 app close() after complete exchange (HTTP/1.0 shape)
        go("H4-app-close-after-reply", ["--send", str(M), "--send-after-read", "--end", "close"], ["--up", 1000, "--down", M, "--end", "fin"])
        # H5 empty request, server speaks first, client only reads
        go("H5-server-speaks-first", ["--send", str(M), "--end", "fin"], ["--up", 0, "--down", M, "--end", "none"])
        # A1 remote TLS peer RST mid-upload
        go("A1-tls-peer-rst-mid-upload", ["--send", "0", "--end", "fin"], ["--up", BIG, "--rst-at", 1_000_000, "--end", "rst"])
        # A2 guest app RST mid-download
        go("A2-app-rst-mid-download", ["--send", str(BIG), "--rst-after", "1000000", "--end", "rst", "--send-after-read"], ["--up", 1000, "--down", BIG, "--end", "fin"], )
        # A2b guest app RST while client is idle (no unread data, app sends part then resets)
        go("A2b-app-rst-after-partial-reply", ["--send", str(BIG), "--rst-after", "500000", "--end", "rst"], ["--up", 0, "--down", BIG, "--end", "none"])
        # A3 app closes with unread data (close() before reading the whole upload) -> RST
        go("A3-app-close-with-unread", ["--send", "1000", "--end", "close"], ["--up", BIG, "--down", 1000, "--end", "fin"])
    # A4 / A5 app crash (kill -9): with unread data, and idle
    for tcp in (False, True):
        sfx = "tcp" if tcp else "vsock"
        for name, up, hang, mode in (("A4-app-crash-with-unread", 64 * 1024 * 1024, 100000, "unread"), ("A5-app-crash-idle-no-unread", 100000, 100000, "idle")):
            nm = f"{name}-{sfx}"
            if tcp:
                app = spawn(f"hostapp-{nm}", [str(LEGS), "gapp", "--listen", f"127.0.0.1:{HOSTAPP_PORT}", "--log", str(EV / f"hostapp-{nm}.jsonl"), "--send", "0", "--end", "fin", "--hang-after", str(hang)])
                wait_text(EV / f"hostapp-{nm}.jsonl", "listening"); rl = Relay(nm, legs="tcp", dst=f"127.0.0.1:{HOSTAPP_PORT}")
            else:
                gx(": > /tmp/gagent.jsonl")
                gapp_start(nm, "--send", "0", "--end", "fin", "--hang-after", str(hang)); rl = Relay(nm)
            pcap = EV / f"{nm}.pcap"
            td = subprocess.Popen(["tcpdump", "-i", "lo", "-nn", "-U", "-s", "96", "-w", str(pcap), f"tcp port {RELAY_PORT}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            time.sleep(0.5)
            cp = client("--up", up, "--down", 0, "--end", "none", "--timeout-ms", 25000, tag=nm, bg=True)
            # wait for the app to report 'hang'
            t0 = time.time(); hung = False
            while time.time() - t0 < 15:
                if tcp: al = jl((EV / f"hostapp-{nm}.jsonl").read_text())
                else: al = gapp_log()
                if any(x.get("ev") == "hang" for x in al): hung = True; break
                time.sleep(0.2)
            time.sleep(0.5)
            tk = time.time()
            if tcp: os.killpg(app.pid, signal.SIGKILL)
            else: gx("/legs pk 9 gapp")
            c = collect(cp, 40)
            c["killed_at_to_client_done_s"] = round(time.time() - tk, 2)
            time.sleep(0.5); reap(td)
            flags = sh(f"tcpdump -nn -r {pcap} 2>/dev/null | awk '{{for(i=1;i<=NF;i++) if($i==\"Flags\") print $(i+1), $3\" > \"$5}}' | sort | uniq -c | sort -rn | head -12", check=False, quiet=True)
            c["pcap_flag_summary"] = flags.strip().splitlines()
            gl = [] if tcp else gagent_log()
            ev = rl.ev(); rl.stop()
            if not tcp: gapp_stop()
            else: reap(app)
            record(nm, hung=hung, client=c, relay=[e for e in ev if e.get("ev") in ("relay_paired", "relay_done", "pump_end", "vsock_eof_classify", "tp_classify")],
                   gagent=[g for g in gl if g.get("ev") in ("gagent_paired", "gagent_done", "pump_end", "vsock_eof_classify", "tp_classify")])


def stage_peer():
    # Q3: what does the guest application see as the peer, per the address leg-S supplies
    for mode in ("legc", "gateway", "none"):
        r = one(f"P-{mode}", {"peer_mode": mode}, ["--send", "1000", "--end", "fin"], ["--up", 1000, "--down", 1000, "--end", "fin"])
        c, al, ev, gl = r
        pc = [e for e in ev if e.get("ev") == "relay_paired"]
        record(f"P-{mode}", client_local=c.get("local"), client_ok=c.get("down_ok"), client_down=c.get("down_recv"),
               relay_peer_c=pc[0]["peer_c"] if pc else None, app_peer=[x.get("peer") for x in conn_ev(al)], app_local=[x.get("local") for x in conn_ev(al)],
               gagent=[g for g in gl if g.get("ev") == "gagent_paired"])
    # port-reuse / concurrency on the transparent bind: 40 concurrent flows from the same peer ip, different ports
    gapp_start("P-conc", "--send", "1000", "--end", "fin")
    rl = Relay("P-conc", peer_mode="legc")
    procs = [client("--up", 1000, "--down", 1000, "--end", "fin", tag=f"c{i}", bg=True) for i in range(40)]
    res = [collect(p, 60) for p in procs]
    time.sleep(0.5)
    al = conn_ev(gapp_log()); rl.stop(); gapp_stop()
    peers = sorted({x["peer"] for x in al})
    record("P-concurrent-40", ok=sum(1 for r in res if r.get("down_ok") and r.get("down_end") == "eof"), app_conns=len(al), distinct_peers=len(peers),
           peers_match_client_locals=sorted(r.get("local") for r in res) == peers)
    # a TLS peer whose source address is not local anywhere (what leg-C getpeername shows for a remote node) is covered by CLI_SRC (198.18.28.1)


def stage_probes():
    gx(": > /tmp/gagent.jsonl")
    out = {}
    # R1/R2 http listener present
    gapp_start("http200", "--mode", "http", "--status", "200", port=8081)
    out["http200_http"] = probe("http", 8081); out["http200_tcp"] = probe("tcp", 8081)
    gapp_stop()
    gapp_start("http500", "--mode", "http", "--status", "500", port=8081)
    out["http500_http"] = probe("http", 8081); out["http500_tcp"] = probe("tcp", 8081)
    gapp_stop()
    # R3 no listener
    out["nolistener_tcp"] = probe("tcp", 8081); out["nolistener_http"] = probe("http", 8081)
    # R4 listens but never accepts, backlog roomy
    gapp_start("noaccept", "--mode", "noaccept", "--backlog", "128", port=8082)
    out["noaccept_tcp"] = probe("tcp", 8082); out["noaccept_http"] = probe("http", 8082, 2000)
    gapp_stop()
    # R5 backlog=1 never accepting: fill the accept queue then probe
    gapp_start("noaccept1", "--mode", "noaccept", "--backlog", "1", port=8083)
    seq = []
    for i in range(6):
        seq.append(probe("tcp", 8083, 2500))
    out["noaccept_backlog1_tcp_sequence"] = seq
    gapp_stop()
    # R7 accepts but never answers
    gapp_start("silent", "--mode", "silent", port=8084)
    out["silent_tcp"] = probe("tcp", 8084); out["silent_http"] = probe("http", 8084, 2000)
    gapp_stop()
    # R6 listener goes away after being healthy
    gapp_start("http200b", "--mode", "http", "--status", "200", port=8085)
    a = probe("http", 8085); gapp_stop(); time.sleep(0.3); b = probe("http", 8085)
    out["goes_away"] = {"before": a, "after": b}
    record("probes", **out)
    record("probes-gagent", gagent=[g for g in gagent_log()][:60])


BENCH = {
    "tcp-splice": dict(legs="tcp", fwd="splice"),
    "tcp-copy": dict(legs="tcp", fwd="copy"),
    "vsock-native-splice": dict(legs="vsock", fwd="splice", native=True),
    "vsock-native-copy": dict(legs="vsock", fwd="copy", native=True),
    "vsock-app-splice": dict(legs="vsock", fwd="splice"),
    "vsock-app-copy": dict(legs="vsock", fwd="copy"),
}
REPS = {1024: 300, 65536: 200, 1048576: 60, 67108864: 8}


def stage_bench():
    only = set(ARGS[0].split(",")) if ARGS else set(BENCH)
    sizes = [int(x) for x in ARGS[1].split(",")] if len(ARGS) > 1 else list(REPS)
    rounds = int(os.environ.get("V28_ROUNDS", "1"))
    reps_env = os.environ.get("V28_REPS")
    reps_map = dict(REPS)
    if reps_env: reps_map.update({int(a): int(b) for a, b in (x.split(":") for x in reps_env.split(","))})
    names = [c for c in BENCH if c in only]
    for rnd in range(rounds):
        order = names[rnd % len(names):] + names[:rnd % len(names)] if rounds > 1 else names
        if rnd % 2 == 1: order = list(reversed(order))
        for cfgname in order:
            kw = BENCH[cfgname]
            for size in sizes:
                for d in ("up", "down", "both"):
                    up = size if d in ("up", "both") else 0
                    down = size if d in ("down", "both") else 0
                    nm = f"{cfgname}-{size}-{d}" + (f"-r{rnd}" if rounds > 1 else "") + (f"-p{PAUSE}" if PAUSE != 5 else "")
                    legs = kw["legs"]
                    if legs == "tcp":
                        app = spawn(f"hostapp-{nm}", [str(LEGS), "gapp", "--listen", f"127.0.0.1:{HOSTAPP_PORT}", "--log", str(EV / f"hostapp-{nm}.jsonl"), "--send", str(down), "--end", "fin", "--end-after-read"])
                        wait_text(EV / f"hostapp-{nm}.jsonl", "listening")
                        rl = Relay(nm, legs="tcp", fwd=kw["fwd"], dst=f"127.0.0.1:{HOSTAPP_PORT}")
                    elif kw.get("native"):
                        rl = Relay(nm, legs="vsock", fwd=kw["fwd"], native=True, up=up, down=down, peer_mode="gateway")
                    else:
                        gapp_start(nm, "--send", str(down), "--end", "fin", "--end-after-read")
                        rl = Relay(nm, legs="vsock", fwd=kw["fwd"], peer_mode="gateway")
                    reps = reps_map[size]
                    c = client("--up", up, "--down", down, "--end", "fin", "--reps", reps, "--pause-ms", PAUSE, tag=nm, timeout=900)
                    rl.stop()
                    if legs == "tcp": reap(app)
                    elif not kw.get("native"): gapp_stop()
                    if c.get("bench"):
                        ts = sorted(c["t_total_us"])
                        med = ts[len(ts) // 2] if ts else None
                        mbps = (max(up, down) if d != "both" else up + down) / (med / 1e6) / 1e6 if med else None
                        log(f"BENCH {nm}: ok={c['ok']}/{reps} median_ms={med/1000 if med else None} MB/s={mbps and round(mbps,1)}")
                        record(nm, ok=c["ok"], fail=c["fail"], fail_examples=c["fail_examples"], t_total_us=c["t_total_us"], t_first_us=c["t_first_us"], wall_us=c["wall_us"], bytes=up + down,
                               relay_cpu_s=rl.cpu_final, reps=reps, pause_ms=PAUSE,
                               legs_open_us=sorted(e["legs_open_us"] for e in rl.ev() if e.get("ev") == "relay_paired")[::max(1, reps // 50)])
                    else:
                        record(nm, error=c)


def stage_death():
    """Last stage: kill the VMM mid-transfer; the host flows must not hang."""
    gapp_start("death", "--send", "0", "--end", "fin", "--hang-after", "100000")
    rl = Relay("death")
    cp = client("--up", 256 * 1024 * 1024, "--down", 0, "--end", "none", "--timeout-ms", 25000, tag="death", bg=True)
    t0 = time.time()
    while time.time() - t0 < 15:
        if any(x.get("ev") == "hang" for x in gapp_log()): break
        time.sleep(0.2)
    time.sleep(0.5)
    tk = time.time()
    os.killpg(VMREF[0].proc.pid, signal.SIGKILL)
    c = collect(cp, 40)
    c["killed_at_to_client_done_s"] = round(time.time() - tk, 2)
    time.sleep(0.5)
    ev = rl.ev(); rl.stop()
    pr = {}
    for kind in ("tcp", "http"):
        t0 = time.time(); pr[kind] = probe(kind, 8080, 4000); pr[kind]["wall_s"] = round(time.time() - t0, 2)
    record("D2-probe-after-vmm-death", **pr)
    record("D1-vmm-killed-mid-upload", client=c, relay=[e for e in ev if e.get("ev") in ("relay_paired", "relay_done", "pump_end", "vsock_eof_classify", "tp_classify")])


def stress_group(name, tcp, app_args, cli_args, judge, rounds=12, par=16):
    nm = f"{name}-{'tcp' if tcp else 'vsock'}"
    if tcp:
        app = spawn(f"hostapp-{nm}", [str(LEGS), "gapp", "--listen", f"127.0.0.1:{HOSTAPP_PORT}", "--log", str(EV / f"hostapp-{nm}.jsonl"), *app_args])
        wait_text(EV / f"hostapp-{nm}.jsonl", "listening"); rl = Relay(nm, legs="tcp", dst=f"127.0.0.1:{HOSTAPP_PORT}")
    else:
        gx(": > /tmp/gagent.jsonl")
        gapp_start(nm, *app_args); rl = Relay(nm)
    res = []
    t0 = time.time()
    for _ in range(rounds):
        ps = [client(*cli_args, "--timeout-ms", 15000, tag=nm, bg=True) for _ in range(par)]
        res += [collect(p, 40) for p in ps]
    time.sleep(1.0)
    if tcp:
        al = conn_ev(jl((EV / f"hostapp-{nm}.jsonl").read_text())); reap(app); gl = []
    else:
        al = conn_ev(gapp_log()); gl = gagent_log(); gapp_stop()
    ev = rl.ev(); rl.stop()
    v = judge(res, al)
    v["flows"] = len(res); v["app_conns_logged"] = len(al); v["wall_s"] = round(time.time() - t0, 1)
    v["hang_clients"] = sum(1 for x in res if x.get("HANG"))
    v["setup_errs"] = sum(1 for x in res if x.get("setup_err"))
    v["relay_not_paired"] = sum(1 for e in ev if e.get("ev") == "relay_legs_not_paired")
    v["gagent_bind_fail"] = sum(1 for g in gl if g.get("ev") == "gagent_bind_fail")
    v["gagent_not_paired"] = sum(1 for g in gl if g.get("ev") == "gagent_not_paired")
    v["tp_none_records"] = sum(1 for e in ev + gl if e.get("ev") == "tp_classify" and "None" in e.get("first", ""))
    record(nm, **v)


def j_clean(Nn, Mm):
    def j(res, al):
        cb = sum(1 for x in res if not (x.get("down_end") == "eof" and x.get("down_recv") == Mm and x.get("down_ok") and x.get("up_err") == 0))
        ab = sum(1 for a in al if not (a["up_end"] == "eof" and a["up_bytes"] == Nn and a["up_ok"]))
        return {"expect": "all clean", "client_bad": cb, "app_bad": ab}
    return j


def j_client_rst(res, al):
    kinds = {}
    for a in al: kinds[a["up_end"]] = kinds.get(a["up_end"], 0) + 1
    return {"expect": "app sees reset, never clean EOF", "app_up_end": kinds, "silent_truncation": kinds.get("eof", 0)}


def j_app_rst(Mm):
    def j(res, al):
        kinds = {}
        for x in res: kinds[x.get("down_end")] = kinds.get(x.get("down_end"), 0) + 1
        trunc = sum(1 for x in res if x.get("down_end") == "eof" and x.get("down_recv", 0) < Mm)
        return {"expect": "client sees reset, never clean EOF short of M", "client_down_end": kinds, "silent_truncation": trunc}
    return j


def j_app_close_unread(res, al):
    kinds = {}
    for x in res: k = f"up_err={x.get('up_err')} down_end={x.get('down_end')} recv={x.get('down_recv')}"; kinds[k] = kinds.get(k, 0) + 1
    return {"expect": "same distribution as tcp baseline", "outcomes": kinds}


def stage_stress():
    for tcp in (False, True):
        sm = 300_001; mm = 200_003
        stress_group("S-H1-fin-then-reply", tcp, ["--send", str(mm), "--send-after-read", "--end", "close"], ["--up", sm, "--down", mm, "--end", "fin"], j_clean(sm, mm))
        stress_group("S-H3-simultaneous", tcp, ["--send", str(mm), "--end", "fin"], ["--up", sm, "--down", mm, "--end", "fin"], j_clean(sm, mm))
        stress_group("S-H2-app-fin-first", tcp, ["--send", str(mm), "--end", "fin"], ["--up", sm, "--down", mm, "--end", "fin", "--send-after-eof"], j_clean(sm, mm))
        stress_group("S-H4-small", tcp, ["--send", "2000", "--send-after-read", "--end", "close"], ["--up", 100, "--down", 2000, "--end", "fin"], j_clean(100, 2000), rounds=30)
        stress_group("S-A1-client-rst", tcp, ["--send", "0", "--end", "fin"], ["--up", 4_000_000, "--rst-at", 300_000, "--end", "rst"], j_client_rst)
        stress_group("S-A2-app-rst", tcp, ["--send", "4000000", "--rst-after", "300000", "--end", "rst", "--send-after-read"], ["--up", 1000, "--down", 4_000_000, "--end", "fin"], j_app_rst(4_000_000))
        stress_group("S-A2b-app-rst-idle", tcp, ["--send", "4000000", "--rst-after", "300000", "--end", "rst"], ["--up", 0, "--down", 4_000_000, "--end", "none"], j_app_rst(4_000_000))
        stress_group("S-A3-app-close-unread", tcp, ["--send", "1000", "--end", "close"], ["--up", 4_000_000, "--down", 1000, "--end", "fin"], j_app_close_unread)


VMREF = []


def main():
    log(f"=== v28 stage {STAGE} {ARGS}")
    pre = preflight()
    sh("modprobe vhost_vsock; modprobe vsock_loopback; modprobe tls; lsmod | grep -E '^(tls|vsock|vhost_vsock)' | head", check=False)
    sh(f"ip addr show dev lo | grep -q {CLI_SRC} || ip addr add {CLI_SRC}/32 dev lo", check=False)
    build_image()
    vm = VM(); VMREF.append(vm)
    try:
        if not vm.wait_ready(): log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        rc, t = gx("uname -r"); log(f"guest uname -r: {t.strip()}  host uname -r: {pre['uname_r']}")
        RESULTS["substrate"] = {"host_uname": pre["uname_r"], "guest_uname": t.strip()}
        guest_setup()
        {"smoke": stage_smoke, "half": stage_half, "peer": stage_peer, "probes": stage_probes, "bench": stage_bench, "death": stage_death, "stress": stage_stress}[STAGE]()
    except Exception as e:
        import traceback; traceback.print_exc(); RESULTS["exception"] = repr(e)
    finally:
        for p in list(PROCS): reap(p)
        vm.stop()
        sh(f"ip addr del {CLI_SRC}/32 dev lo", check=False, quiet=True)
        sh("pgrep -af cloud-hypervisor | grep -v pgrep; true", check=False)
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=1, default=str))
    print("DONE results at", EV)


main()
