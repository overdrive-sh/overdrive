#!/usr/bin/env python3
"""evmod native-metal VM driver (root, under the metal lease; launched by launch.py). Derived from the V-3 / valve vmrun.py.

A real Cloud Hypervisor VM (fork, --vsock backend=vhost-kernel). The guest kernel is the stock 7.0.0-29-generic image; the physical
host was rebooted onto 7.0.0-38-generic between increments, so the host-side vhost-vsock transport is now -38 (recorded). The module
under test (evmod.ko) is loaded INSIDE THE GUEST kernel only; nothing is loaded into the physical host kernel. Guest PID 1 starts
`evprobe agent`; this driver starts host-side processes and asks the agent to start guest-side ones.

Usage: vmrun.py <scenario>[,<scenario>...] [mem=MB]
Directions: A = host sender -> guest vsock source -> evmod -> guest TCP target app
            T = guest TCP source app -> evmod -> guest TCP target app
            G = guest TCP source app -> evmod -> vsock target -> host reader
            ctl-* use the V-3/valve BPF probe (sockmap redirect, no bound) as the same-session control.
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, statistics, struct, subprocess, time

W = pathlib.Path("/var/tmp/evmod2-work"); OUT = W / "out"
NAMES = sys.argv[1].split(","); KV = dict(a.split("=", 1) for a in sys.argv[2:])
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{sys.argv[1].replace(',', '+')[:50]}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3371
MEM_MB = int(KV.get("mem", "1024"))
CH = OUT / "cloud-hypervisor"
PROBE = OUT / "evprobe"
VALVE_PROBE, VALVE_OBJ = OUT / "valveprobe", OUT / "valve-bpf.o"
KVER = "7.0.0-29-generic"   # the guest kernel; every guest module is taken from this version, never from the (newer) running host kernel
MODVARIANT = KV.get("mod", "module")   # module = the final build from the previous increment (byte-identical source); module-ktls = kTLS-aware variant
KO = OUT / f"mod-{MODVARIANT}" / "evmod.ko"
PROCS = []
EV.mkdir(parents=True, exist_ok=False)
LOG = (EV / "run.log").open("a")
MIB = 1048576


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


def sha(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


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
    pre = {"host_uname_r": os.uname().release, "machine": os.uname().machine, "systemd_detect_virt": virt.stdout.strip(),
           "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "nproc": os.cpu_count(),
           "meminfo_head": pathlib.Path("/proc/meminfo").read_text().splitlines()[:3],
           "physical_modules_before": pathlib.Path("/proc/modules").read_text()}
    (EV / "substrate-probes.json").write_text(json.dumps({**pre, "failures": fail}, indent=1))
    log(f"substrate: {json.dumps({k: v for k, v in pre.items() if k != 'physical_modules_before'})}")
    if fail:
        log(f"FAILED CLOSED: {fail}"); sys.exit(3)
    return pre["physical_modules_before"]


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
    for n in ["sh", "mount", "insmod", "rmmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "touch", "true", "head", "tail", "wc", "date", "grep", "dmesg", "free", "ps", "pkill", "ip"]:
        (root / "bin" / n).symlink_to("busybox")
    for b in ["/usr/sbin/ip", "/usr/bin/ss"]:
        if os.path.exists(b): copy_with_libs(root, b)
    shutil.copy2(PROBE, root / "evprobe"); shutil.copy2(VALVE_PROBE, root / "valveprobe"); shutil.copy2(VALVE_OBJ, root / "valve-bpf.o")
    shutil.copy2(KO, root / "mods" / "evmod.ko")
    params = KV.get("modparams", "").replace("+", " ")
    (root / "init").write_text(f"""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /sys/fs/cgroup /sys/fs/bpf /tmp
mount -t cgroup2 none /sys/fs/cgroup
mount -t bpf bpf /sys/fs/bpf
mount -t tmpfs tmpfs /tmp
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
insmod /mods/evmod.ko {params} || echo "GUEST_INSMOD_FAIL evmod"
ip link set lo up
ulimit -n 65536
/evprobe agent > /dev/ttyS0 2>&1 &
( while true; do echo "HB up=$(cat /proc/uptime) $(grep -E 'MemAvailable|SUnreclaim' /proc/meminfo | head -2 | while read k v u; do echo -n "$k$v "; done)" > /dev/ttyS0; sleep 1; done ) &
echo "GUEST_READY"
while true; do sleep 3600; done
""")
    os.chmod(root / "init", 0o755)
    order, pins = [], {}
    tls_order = []
    for top, dst in [("vmw_vsock_virtio_transport", order), ("tls", tls_order), ("aesni_intel", tls_order)]:
        for line in sh(["modprobe", "-S", KVER, "--show-depends", top], quiet=True).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order or name in tls_order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); dst.append(name); pins[str(src)] = sha(src)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    (root / "mods/order-tls").write_text("\n".join(tls_order) + "\n")
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}", quiet=True)
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "evprobe": sha(PROBE), "valveprobe": sha(VALVE_PROBE), "valve_bpf_obj": sha(VALVE_OBJ),
                 "evmod.ko": sha(KO), "cloud_hypervisor": sha(CH)})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))
    shutil.copy2(W / "src" / "module" / "evmod.c", EV / "evmod.c.executed")
    shutil.copy2(W / "src" / "probe" / "src" / "main.rs", EV / "evprobe-main.rs.executed")
    log(f"image pins: {json.dumps({k: v[:12] for k, v in pins.items() if not k.startswith('/')})}")


class VM:
    def __init__(self, cpus=4, mem=MEM_MB):
        self.dir = OUT / f"vm-{STAMP}"; self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", f"console=ttyS0 panic={KV.get('panic', '-1')} loglevel={KV.get('loglevel', '4')}",
               "--cpus", f"boot={cpus}", "--memory", f"size={mem}M", "--serial", f"file={self.serial}", "--console", "off",
               "--vsock", f"cid={CID},backend=vhost-kernel", "--log-file", str(self.dir / "ch.log"), "-v"]
        log("launch VM: " + " ".join(cmd))
        self.proc = subprocess.Popen(cmd, stdout=(self.dir / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True)

    def text(self): return self.serial.read_text(errors="replace") if self.serial.exists() else ""
    def alive(self): return self.proc.poll() is None

    def wait_ready(self, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            if "GUEST_READY" in self.text() and '"agent_ready"' in self.text(): return True
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


def gx(cmd, timeout=600):
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
    with (EV / "guest-commands.jsonl").open("a") as f: f.write(json.dumps({"t": time.time(), "cmd": cmd, "rc": rc, "out": t[-6000:]}) + "\n")
    return rc, t


def spawn(name, argv):
    f = (EV / f"proc-{name}.log").open("w")
    p = subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, start_new_session=True); p.name = name; PROCS.append(p); return p


def reap(p, sig=signal.SIGKILL):
    if p.poll() is None:
        try: os.killpg(p.pid, sig)
        except ProcessLookupError: pass
        try: p.wait(timeout=8)
        except subprocess.TimeoutExpired: pass


def jl(text, label=None):
    ev, bad = [], 0
    for l in text.splitlines():
        i = l.find("{\"ev\"")
        if i < 0: continue
        try: e = json.loads(l[i:])
        except Exception: bad += 1; continue
        if label is None or e.get("label") == label: ev.append(e)
    return ev, bad


def mib(x): return round(x / MIB, 2)
def kbm(x): return round(x / 1024, 1)
def st(xs):
    xs = [x for x in xs if x is not None]
    if not xs: return None
    return {"min": round(min(xs), 2), "p50": round(statistics.median(xs), 2), "max": round(max(xs), 2), "n": len(xs)}


# ----------------------------------------------------------------------------- scenarios
CAP = "--abort-queued-mb 160"


def S(d, n, mb, app, sink="", send="", max_s=30, verbose=1, probe="ev", params=None):
    return dict(dir=d, n=n, bytes=mb * MIB, app=app, sink=sink, send=send, max_s=max_s, verbose=verbose, probe=probe, params=params or {})


TLS_LOADED = []
SCENARIOS = {}
for d in "ATG":
    SCENARIOS.update({
        # (a) memory over time with a non-reading target; (b) byte-exact after resume
        f"ev-stall-{d}": S(d, 20, 256, "s3000,f", CAP),
        f"ev-long-{d}": S(d, 3, 512, "s10000,f", CAP, max_s=40),
        f"ev-slow-{d}": S(d, 5, 128, "r20", CAP, max_s=60),
        f"ev-cyc-{d}": S(d, 10, 256, "s1000,f:300,s1000,f:300,s1000,f", CAP),
        # both directions at once: the stalled direction must not block the opposite one; FIN must propagate both ways
        f"ev-bidir-{d}": S(d, 10, 128, "s2000,f", CAP + " --back 33554432 --eof 1", "--back 33554432 --eof 1"),
        f"ev-eof-{d}": S(d, 40, 1, "f", CAP + " --eof 1 --close-wait-ms 150", "--eof 1", max_s=20, verbose=0),
        f"ev-tiny-{d}": S(d, 60, 0, "f", CAP + " --eof 1 --close-wait-ms 150", "--eof 1", max_s=20, verbose=0),
        # negative controls: the harness must detect an injected fault (module param fault=1 loss, 2 duplicate, 3 reorder)
        f"ev-fault1-{d}": S(d, 3, 16, "f", CAP, params={"fault": 1}, max_s=8),
        f"ev-fault2-{d}": S(d, 3, 16, "f", CAP, params={"fault": 2}, max_s=8),
        f"ev-fault3-{d}": S(d, 3, 16, "f", CAP, params={"fault": 3}, max_s=8),
        # throughput / CPU of one flow, fast reader, no verification (byte count only)
        f"ev-tp-{d}": S(d, 10, 1024, "f", "--verify 0 --fast-gen 1", "--fast-gen 1", verbose=0, max_s=40),
        # reader that stalls then finishes with a bursty sender (tail stranding hazard of the sockmap re-insert design)
        f"ev-burst-{d}": S(d, 10, 64, "s2000,f", CAP, "--gap-ms 100 --burst-kb 1024", max_s=120),
    })
SCENARIOS.update({
    # lifecycle: abrupt end of one side mid-stream: no hang, no leak, no kernel BUG, the pair is gone (live=0) afterwards
    "ev-cut-A": S("A", 5, 64, "f", CAP + " --cut-after-kb 4096", "", max_s=20, verbose=0),
    "ev-cut-T": S("T", 5, 64, "f", CAP + " --cut-after-kb 4096", "", max_s=20, verbose=0),
    "ev-cut-G": S("G", 5, 64, "f", CAP + " --await-gone 1", "--cut-after-kb 4096", max_s=20, verbose=0),
    # the target PROCESS dies (RST) rather than half-closing: the abort must reach the source writer promptly
    "ev-kill-A": S("A", 10, 64, "f", CAP + " --cut-after-kb 4096 --cut-rst 1", "", max_s=20, verbose=0),
    "ev-kill-T": S("T", 10, 64, "f", CAP + " --cut-after-kb 4096 --cut-rst 1", "", max_s=20, verbose=0),
    "ev-kills-T": S("T", 10, 64, "s500,f", CAP + " --cut-after-kb 4096 --cut-rst 1", "", max_s=20, verbose=0),
    "ev-sclose-A": S("A", 5, 64, "f", CAP + " --await-gone 1", "--close-after-mb 8", max_s=20, verbose=0),
    # the bound is the sum of the explicit socket buffers: with the target legs set small the whole pipeline shrinks accordingly
    "ev-tight-A": S("A", 5, 128, "s2000,f", CAP + " --tgt-sndbuf 65536 --app-rcvbuf 65536", "", max_s=30),
    "ev-tight-T": S("T", 5, 128, "s2000,f", CAP + " --tgt-sndbuf 65536 --app-rcvbuf 65536 --src-rcvbuf 65536", "", max_s=30),
    # same-session controls with the V-3 / valve BPF probe (sockmap redirect, no bound; and the userspace pump)
    "ctl-A": S("A", 3, 256, "s3000,f", "--abort-queued-mb 160", probe="valve"),
    "ctl-T": S("T", 3, 256, "s3000,f", "--abort-queued-mb 160", probe="valve"),
    "tp-sockmap-A": S("A", 10, 1024, "f", "--high-kb 0 --verify 0", "--fast-gen 1", verbose=0, probe="valve"),
    "tp-pump-A": S("A", 10, 1024, "f", "--force-pump 1 --verify 0", "--fast-gen 1", verbose=0, probe="valve"),
    "tp-sockmap-T": S("T", 10, 1024, "f", "--high-kb 0 --verify 0 --fast-gen 1", verbose=0, probe="valve"),
    "tp-pump-T": S("T", 10, 1024, "f", "--force-pump 1 --verify 0 --fast-gen 1", verbose=0, probe="valve"),
})
EOF1 = " --eof 1"
SCENARIOS.update({"leak-mod": S("T", 1, 1, "f"), "leak-ctl": S("T", 1, 1, "f")})
for d in "ATG":
    snd = "--eof 1"   # sender / host reader args (A: sender, G: vread)
    SCENARIOS.update({
        # rmmod while a pair is LIVE. The controller closes /dev/evmod (so rmmod is not blocked by the open device), the sink then runs
        # `rmmod evmod` itself, and a plain userspace pump continues the stream on the very same sockets using only their own poll wakeups.
        # stalled: the target app is still stalled when the module goes; flowing: traffic is running at full speed when it goes.
        f"ev-unload-{d}": S(d, 3, 64, "s2500,f", CAP + EOF1 + " --unload-after-ms 1200", snd, max_s=30),
        f"ev-unloadf-{d}": S(d, 5, 256, "f", CAP + EOF1 + " --unload-after-ms 150", snd, max_s=40),
        # stress: unload at 0..60 ms into a flowing pair, 30 fresh insmod/attach/rmmod cycles; leak audit at the end
        f"ev-leakbase-{d}": S(d, 40, 1, "f", CAP + " --eof 1 --close-wait-ms 300 --leak-wait-ms 10000", "--eof 1", max_s=20, verbose=0),
        f"ev-unload-stress-{d}": S(d, 30, 64, "f", CAP + EOF1 + " --unload-after-ms 2 --unload-spread-ms 60 --leak-wait-ms 10000 --close-wait-ms 300", snd, max_s=40, verbose=0),
        # the bound with the target legs set small
        f"ev-tight-{d}": S(d, 5, 128, "s2000,f", CAP + (" --tgt-sndbuf 65536 --app-rcvbuf 65536" if d == "A" else " --tgt-sndbuf 65536 --app-rcvbuf 65536 --src-rcvbuf 65536" if d == "T" else " --src-rcvbuf 65536 --vsock-buf 65536"), "--vsock-buf 65536" if d == "G" else "", max_s=30),
    })
SCENARIOS.update({
    # abort propagation from the SOURCE side: the source app RSTs mid-stream; the target half must be aborted (TCP: RST; vsock: SHUT_RDWR)
    "ev-rst-T": S("T", 5, 64, "f", CAP + " --src-rst-after-kb 4096 --await-gone 1", "", max_s=20, verbose=0),
    "ev-rst-G": S("G", 5, 64, "f", CAP + " --src-rst-after-kb 4096 --await-gone 1", "", max_s=20, verbose=0),
})
# ---- kTLS coexistence (ek-*): the TCP leg(s) carry kTLS (TLS 1.3 AES-128-GCM, TX and RX, ULP installed). Mask: 1 = target pair, 2 = source pair.
KM = {"A": 1, "T": 3, "G": 2}
for d in "ATG":
    k = f" --ktls {KM[d]}"
    SCENARIOS.update({
        f"ek-stall-{d}": S(d, 10, 256, "s3000,f", CAP + k, "", max_s=30),
        f"ek-bidir-{d}": S(d, 5, 128, "s2000,f", CAP + k + " --back 33554432 --eof 1", "--back 33554432 --eof 1"),
        f"ek-eof-{d}": S(d, 20, 1, "f", CAP + k + " --eof 1 --close-wait-ms 150", "--eof 1", max_s=20, verbose=0),
        f"ek-alert-{d}": S(d, 5, 1, "f", CAP + k + " --eof 1 --ktls-close-notify 1 --close-wait-ms 150", "--eof 1", max_s=20, verbose=0),
        f"ek-after-{d}": S(d, 5, 64, "s1000,f", CAP + k + " --ktls-order after", "", max_s=30),
        f"ek-tp-{d}": S(d, 10, 1024, "f", "--verify 0 --fast-gen 1" + k, "--fast-gen 1", verbose=0, max_s=60),
        f"ek-ver-{d}": S(d, 5, 512, "f", CAP + k, "", max_s=40, verbose=0),
        f"ek-unload-{d}": S(d, 3, 64, "s2500,f", CAP + k + EOF1 + " --unload-after-ms 1200", "--eof 1", max_s=30),
        # kTLS installed AFTER the module hooked the socket (TLS saves ev_data_ready as saved_data_ready), then rmmod with live pairs
        f"ek-unloadaft-{d}": S(d, 3, 64, "s2500,f", CAP + k + " --ktls-order after" + EOF1 + " --unload-after-ms 1200", "--eof 1", max_s=30),
        f"ek-unloadaftf-{d}": S(d, 5, 256, "f", CAP + k + " --ktls-order after" + EOF1 + " --unload-after-ms 150", "--eof 1", max_s=40),
    })
PORT = {n: 7300 + i for i, n in enumerate(SCENARIOS)}


def run_leak(vm, name):
    ctl = name.endswith("ctl"); cycles = int(KV.get("cycles", "300"))
    log(f"##### leak audit {name}: {cycles} cycles, control={ctl}")
    gx(f"/evprobe leak --cycles {cycles} --control {int(ctl)} --label {name} --modparams '{KV.get('modparams', '')}' > /dev/ttyS0 2>&1 < /dev/null &", 20)
    end = time.time() + 900; ev_ = None
    while time.time() < end and vm.alive():
        for l in vm.text().splitlines():
            if '"leak_cycles"' in l and f'"label":"{name}"' in l:
                ev_ = json.loads(l[l.find("{"):])
        if ev_: break
        time.sleep(1)
    gx(f"[ -e /proc/evmod ] || insmod /mods/evmod.ko")
    if not ev_:
        return {"scenario": name, "exception": "no leak_cycles event"}
    sn = ev_["snaps_cycle_avail_sunreclaim_slab_kb"]
    def slope(xs, ys):
        n = len(xs); xm = sum(xs) / n; ym = sum(ys) / n
        return sum((x - xm) * (y - ym) for x, y in zip(xs, ys)) / max(sum((x - xm) ** 2 for x in xs), 1e-9)
    xs = [s[0] for s in sn]
    summ = {"scenario": name, "dir": "leak", "wall_s": 0, "outcomes": {}, "cycles": ev_["cycles"], "control": ev_["control"], "failures": ev_["failures"],
            "snaps": sn, "SUnreclaim_slope_KiB_per_cycle": round(slope(xs, [s[2] for s in sn]), 3), "Slab_slope_KiB_per_cycle": round(slope(xs, [s[3] for s in sn]), 3),
            "MemAvailable_slope_KiB_per_cycle": round(slope(xs, [s[1] for s in sn]), 3), "named_start_end": ev_["named_start_end"], "slab_objs_delta_top": ev_["slab_objs_delta_top"],
            "module_loaded_after": ev_["module_loaded"]}
    log("SUMMARY " + json.dumps(summ))
    return summ


def run_scenario(vm, name):
    if name.startswith("leak-"):
        return run_leak(vm, name)
    sc = SCENARIOS[name]; port = PORT[name]
    d, n, probe = sc["dir"], sc["n"], sc["probe"]
    log(f"##### scenario {name}: {json.dumps(sc)}")
    for k, v in sc["params"].items():
        if k != "unload":
            gx(f"echo {v} > /sys/module/evmod/parameters/{k}")
    exe_g = "/evprobe" if probe == "ev" else "/valveprobe"
    exe_h = str(PROBE if probe == "ev" else VALVE_PROBE)
    objarg = "" if probe == "ev" else " --obj /valve-bpf.o"
    nbytes = max(sc["bytes"], 8) if sc["bytes"] else 8
    if name.startswith("ev-tiny"):
        nbytes = 8  # exactly one 8-byte word
    sink_common = f"--port {port} --trials {n} --bytes {nbytes} --label {name} --app {sc['app']} --max-s {sc['max_s']} --verbose-trials {sc['verbose']} --modparams '{KV.get('modparams', '')}' {sc['sink']}"
    gx(f"[ -e /proc/evmod ] || insmod /mods/evmod.ko {KV.get('modparams', '').replace('+', ' ')}; true")
    if "--ktls" in sc["sink"] and not TLS_LOADED:
        rc, t = gx("for m in $(cat /mods/order-tls); do insmod /mods/$m 2>&1 || echo INSMOD_FAIL $m; done; lsmod | grep -E 'tls|aesni'; ls -l /usr/bin/ss; /usr/bin/ss -V 2>&1 | head -2; grep -B1 -A8 'name         : gcm(aes)' /proc/crypto | grep -E 'name|driver|priority' | head -12; grep -E 'cpu MHz|flags' /proc/cpuinfo | head -2 | cut -c1-200; cat /proc/net/tls_stat")
        log("kTLS modules:\n" + t); (EV / "ktls-modules.txt").write_text(t); TLS_LOADED.append(1)
    t0 = time.time()
    unload_out = None
    horizon = n * (sc["max_s"] + 20) + 60
    hlog = None
    if d == "A":
        gx(f"{exe_g} sink --kind vsock{objarg} {sink_common} > /dev/ttyS0 2>&1 < /dev/null &", 20)
        time.sleep(1.5)
        p = spawn(f"{name}-send", [exe_h, "send", "--cid", str(CID), "--port", str(port), "--bytes", str(nbytes), "--trials", str(n), "--label", name] + sc["send"].split())
        try: p.wait(timeout=horizon)
        except subprocess.TimeoutExpired: log("sender timeout"); reap(p)
        end = time.time() + 40
        while time.time() < end:
            g, _ = jl(vm.text(), name)
            if any(e["ev"] == "sink_done" for e in g): break
            time.sleep(0.5)
        g, bad = jl(vm.text(), name); h, _ = jl((EV / f"proc-{name}-send.log").read_text(), name)
    elif d == "G":
        hp = spawn(f"{name}-vread", [exe_h, "vread", "--port", str(port), "--trials", str(n), "--bytes", str(nbytes), "--label", name, "--app", sc["app"], "--max-s", str(sc["max_s"]),
                                    "--verbose-trials", str(sc["verbose"])] + [a for a in sc["send"].split()] + (["--verify", "0"] if "--verify 0" in sc["sink"] else []))
        time.sleep(2)
        gx(f"{exe_g} sink --kind tcp2vsock --host-port {port} {sink_common} > /dev/ttyS0 2>&1 < /dev/null &", 20)
        try: hp.wait(timeout=horizon)
        except subprocess.TimeoutExpired: log("host vread timeout"); reap(hp)
        end = time.time() + 40
        while time.time() < end:
            g, _ = jl(vm.text(), name)
            if any(e["ev"] == "sink_done" for e in g): break
            time.sleep(0.5)
        g, bad = jl(vm.text(), name); h, _ = jl((EV / f"proc-{name}-vread.log").read_text(), name)
    else:
        gx((f"({exe_g} sink --kind tcpsrc{objarg} {sink_common}; echo SINK_EXIT=$? ) > /tmp/sink.log 2>&1 < /dev/null &" if KV.get('diag') == '1' else f"{exe_g} sink --kind tcpsrc{objarg} {sink_common} > /dev/ttyS0 2>&1 < /dev/null &"), 20)
        end = time.time() + horizon
        diag_done = False
        while time.time() < end:
            g, _ = jl(vm.text(), name)
            if any(e["ev"] == "sink_done" for e in g) or not vm.alive(): break
            if KV.get("diag") == "1" and not diag_done and time.time() - t0 > 25 and not any(e["ev"] in ("trial_end", "sample") for e in g):
                diag_done = True
                rc_, dt = gx("ps; ls -la /tmp | head; for p in $(pidof evprobe); do echo == pid $p; cat /proc/$p/status | grep -E 'State|Threads'; for t in /proc/$p/task/*; do echo task $(basename $t) $(cat $t/comm) $(cat $t/wchan 2>/dev/null); cat $t/stack 2>/dev/null | head -14; done; done; echo == evmod; cat /proc/evmod; echo == tls; cat /proc/net/tls_stat | grep -v ' 0$'; echo == ss; ss -tniea 2>/dev/null | head -40; echo == sinklog; tail -c 3000 /tmp/sink.log; echo == dmesg; dmesg | tail -40", 60)
                (EV / "diag.txt").write_text(dt); log("DIAG captured:\n" + dt[-3500:])
            time.sleep(0.5)
        g, bad = jl(vm.text(), name); h = []
    for k in sc["params"]:
        if k != "unload":
            gx(f"echo 0 > /sys/module/evmod/parameters/{k}")
    wall = time.time() - t0
    oom = [l for l in vm.text().splitlines() if re.search(r"Out of memory|oom-kill|Kernel panic|invoked oom|Killed process|page allocation failure|BUG:|WARNING:|Oops|general protection|Call Trace", l)]
    te = [e for e in g if e["ev"] == "trial_end"]; se = [e for e in h if e["ev"] == "send_end"]; ve = [e for e in h if e["ev"] == "trial_end"]
    samples = [e for e in g if e["ev"] == "sample" and e["trial"] == 0 and e.get("side") != "host"]
    hsamples = [e for e in h if e["ev"] == "sample" and e["trial"] == 0]
    (EV / f"{name}-trial_end.jsonl").write_text("\n".join(json.dumps(e) for e in te))
    (EV / f"{name}-send_end.jsonl").write_text("\n".join(json.dumps(e) for e in se))
    (EV / f"{name}-vread_trial_end.jsonl").write_text("\n".join(json.dumps(e) for e in ve))
    (EV / f"{name}-samples-trial0.jsonl").write_text("\n".join(json.dumps(e) for e in samples))
    (EV / f"{name}-host-samples-trial0.jsonl").write_text("\n".join(json.dumps(e) for e in hsamples))
    ev_probe = probe == "ev"
    # who verified the main stream: the local reader (A, T) or the host vread (G)
    main_ok = ve if d == "G" else te
    lat = []
    for t in (ve if d == "G" else te):
        if not t.get("stall_ends"): continue
        t_res = t["stall_ends"][0][0]
        if d == "A":
            s = next((x for x in se if x.get("trial") == t["trial"]), None)
            if s and s.get("blocks"):
                b = max(s["blocks"], key=lambda x: x[1]); lat.append((b[0] + s["off_ns"] - t_res) / 1e6)
        elif d == "T":
            w = t.get("writer")
            if isinstance(w, dict) and w.get("blocks"):
                b = max(w["blocks"], key=lambda x: x[1]); lat.append((b[0] - t_res) / 1e6)
        else:  # G: guest writer clock + off -> host clock
            gt = next((x for x in te if x["trial"] == t["trial"]), None)
            w = gt.get("writer") if gt else None
            if isinstance(w, dict) and w.get("blocks"):
                b = max(w["blocks"], key=lambda x: x[1]); lat.append((b[0] + gt["off_ns"] - t_res) / 1e6)
    summ = {"unload_output": (unload_out if sc["params"].get("unload") else None), "scenario": name, "dir": d, "probe": probe, "wall_s": round(wall, 1), "trials_planned": n, "trial_end": len(te), "send_end": len(se), "vread_trial_end": len(ve),
            "outcomes": {k: sum(1 for t in te if t["outcome"] == k) for k in sorted({t["outcome"] for t in te})},
            "oom_or_bug_lines": oom[:8], "bad_json_lines": bad}
    if ev_probe:
        ok_main = sum(1 for t in main_ok if t["app_ok"])
        summ.update({
            "main_stream_ok": f"{ok_main}/{len(main_ok)}",
            "main_stream_first_bad": [t["first_bad"] for t in main_ok if t["first_bad"] >= 0][:5],
            "main_hash_eq": f"{sum(1 for t in main_ok if t['hash'] == t['ehash'] and t['app_read'] == t['bytes'])}/{len(main_ok)}",
            "eof_seen": f"{sum(1 for t in main_ok if t.get('eof_seen'))}/{len(main_ok)}", "extra_after_eof": sum(t.get("extra", 0) for t in main_ok),
            "bytes_verified_MiB": mib(sum(t["app_read"] for t in main_ok if t["app_ok"])),
            "peak_pipe_MiB(src_inq+tgt_outq+peer_inq)": st([mib(t["peak_pipe"]) for t in te]),
            "peak_src_inq_KiB": st([round(t["peak_src_inq"] / 1024, 1) for t in te]),
            "peak_tgt_outq_KiB": st([round(t["peak_tgt_outq"] / 1024, 1) for t in te]),
            "peak_peer_inq_KiB": st([round(t["peak_peer_inq"] / 1024, 1) for t in te]),
            "peak_ap_outq_KiB": st([round(t["peak_ap_outq"] / 1024, 1) for t in te]),
            "guest_unreclaim_rise_MiB(max-start)": st([kbm(t["max_sunreclaim_kb"] - t["sunreclaim_start_kb"]) for t in te]),
            "guest_avail_drop_MiB(start-min)": st([kbm(t["avail_start_kb"] - t["min_avail_kb"]) for t in te]),
            "guest_unreclaim_after_close_minus_start_MiB": st([kbm(t["sunreclaim_after_close_kb"] - t["sunreclaim_start_kb"]) for t in te]),
            "max_skb_head": st([t["max_skb_head"] for t in te]),
            "module_moved_MiB": st([mib(t["moved"]) for t in te]), "module_stall_dst": st([t["stall_dst"] for t in te]), "module_stall_src": st([t["stall_src"] for t in te]),
            "module_runs": st([t["runs"] for t in te]), "module_requeue": st([t["requeue"] for t in te]), "module_err": sum(t["err"] for t in te), "pairs_aborted": sum(t.get("aborted", 0) for t in te), "pairs_completed": sum(t.get("completed", 0) for t in te), "live_end": [t.get("live_end") for t in te][:6], "short_consume": sum(t["short_consume"] for t in te),
            "fault_hits": sum(t["fault_hits"] for t in te), "pair_self_completed_before_detach": sum(1 for t in te if not t["detached"]),
            "tasks_start": st([t["tasks_start"] for t in te]), "tasks_max": st([t["tasks_max"] for t in te]),
            "reader_MiBps": st([t["reader_mibps"] for t in main_ok]),
            "sys_busy_ms": st([t["sys_busy_ms"] for t in te]), "trial_secs": st([t["secs"] for t in te]),
            "cpu_core_s_per_GiB": st([(t["sys_busy_ms"] / 1000.0) / max(t["app_read"] / 2**30, 1e-9) for t in te]),
            "back_ok": f"{sum(1 for t in te if isinstance(t.get('back'), dict) and t['back']['ok'])}/{sum(1 for t in te if isinstance(t.get('back'), dict))}",
            "sender_back_ok": f"{sum(1 for s in se if isinstance(s.get('back'), dict) and s['back']['ok'])}/{sum(1 for s in se if isinstance(s.get('back'), dict))}",
            "sender_outcomes": {k: sum(1 for t in se if t.get("outcome") == k) for k in sorted({t.get("outcome") for t in se})},
            "sender_MiBps": st([s.get("mibps") for s in se]), "sender_max_send_ms": st([s.get("max_send_ms") for s in se]),
            "sender_sent_at_longest_block_start_MiB": st([mib(s.get("sent_at_longest_block_start", 0)) for s in se]),
            "writer_max_send_ms": st([t["writer"].get("max_send_ms") for t in te if isinstance(t.get("writer"), dict)]),
            "writer_sent_at_longest_block_start_MiB": st([mib(t["writer"].get("sent_at_longest_block_start", 0)) for t in te if isinstance(t.get("writer"), dict)]),
            "writer_outcomes": {k: sum(1 for t in te if isinstance(t.get("writer"), dict) and t["writer"]["outcome"] == k) for k in sorted({t["writer"]["outcome"] for t in te if isinstance(t.get("writer"), dict)})},
            "host_unreclaim_rise_MiB(max-start)": st([kbm(t["max_sunreclaim_kb"] - t["sunreclaim_start_kb"]) for t in ve]),
            "host_peak_vsock_inq_KiB": st([round(t["peak_src_inq"] / 1024, 1) for t in ve]),
            "resume_latency_ms(stall_end->sender_unblocked)": st(lat),
        })
        # ---- inc2 additions: how each side ended, abort-propagation latency, kTLS evidence, unload + pump, leak audit
        mt = ve if d == "G" else te
        summ["reader_end"] = {k: sum(1 for t in mt if t.get("reader_end") == k) for k in sorted({t.get("reader_end") for t in mt})}
        sender_err = [(t.get("writer") or {}).get("outcome") for t in te if isinstance(t.get("writer"), dict)] + [x.get("outcome") for x in se]
        summ["writer_outcomes_all"] = {k: sender_err.count(k) for k in sorted(set(sender_err), key=str)}
        al = []
        for t in te:
            tr = t["trial"]
            if name.startswith("ev-cut-A") or name.startswith("ev-kill-A"):
                x = next((q for q in se if q.get("trial") == tr), None)
                if x and x.get("end_ns"): al.append((x["end_ns"] + x["off_ns"] - t["reader_done_ns"]) / 1e6)
            elif name.startswith("ev-cut-T") or name.startswith("ev-kill") or name.startswith("ev-rst-T"):
                w = t.get("writer")
                if isinstance(w, dict) and w.get("end_ns"):
                    al.append((w["end_ns"] - t["reader_done_ns"]) / 1e6 if name.startswith(("ev-cut", "ev-kill")) else (t["reader_done_ns"] - w["end_ns"]) / 1e6)
            elif name.startswith("ev-cut-G") or name.startswith("ev-rst-G"):
                w = t.get("writer"); v = next((q for q in ve if q.get("trial") == tr), None)
                if isinstance(w, dict) and w.get("end_ns") and v:
                    gw = (w["end_ns"] + t["off_ns"])  # guest writer end on the host clock
                    al.append((gw - v["reader_done_ns"]) / 1e6 if name.startswith("ev-cut") else (v["reader_done_ns"] - gw) / 1e6)
        if al:
            summ["abort_propagation_ms" + ("(cut: target-app cut -> source writer end)" if name.startswith(("ev-cut", "ev-kill")) else "(rst: source RST -> target reader end)")] = st(al)
        xs = [t.get("x") or {} for t in te]
        if xs and xs[0].get("ktls_mask"):
            def dl(x):
                a, b = x.get("tls_stat_after") or {}, x.get("tls_stat_before") or {}
                return {k: a[k] - b.get(k, 0) for k in a if a[k] != b.get(k, 0)}
            summ["ktls"] = {"mask": xs[0]["ktls_mask"], "order": xs[0]["ktls_order"], "install_errors": [x.get("ktls_err") for x in xs if x.get("ktls_err")][:3],
                            "ss_ulp_trial0": xs[0].get("ss_ulp"), "tls_stat_delta_trial0": dl(xs[0]), "tls_stat_after_last": xs[-1].get("tls_stat_after")}
        uls = [x["unload"] for x in xs if x.get("unload")]
        if uls:
            live = [int(m.group(1)) for u in uls for m in [re.search(r"live=(\d+)", u.get("info", ""))] if m]
            pumps = [x["pump"] for x in xs if x.get("pump")]
            summ["unload"] = {"trials": len(uls), "rmmod_rc": [u["rc"] for u in uls], "rmmod_ms": st([u["ms"] for u in uls]), "pairs_live_at_rmmod": f"{sum(1 for l in live if l >= 1)}/{len(live)}",
                              "rmmod_stderr": [u["info"].split("|", 1)[1] for u in uls if "|" in u["info"] and u["info"].split("|", 1)[1]][:3],
                              "pump_moved_MiB": st([mib(p["moved"]) for p in pumps]), "pump_lost_wakeups": sum(p["lost_wakeups"] for p in pumps), "pump_eof": f"{sum(1 for p in pumps if p['eof'])}/{len(pumps)}",
                              "pump_errors": [p["err"] for p in pumps if p["err"]][:3]}
        lk = [e for e in g if e["ev"] == "leak_check"]
        if lk: summ["leak_check"] = lk[-1]
        if len(te) >= 8:
            def slope(ys):
                n = len(ys); xm = (n - 1) / 2; ym = sum(ys) / n
                return sum((i - xm) * (y - ym) for i, y in enumerate(ys)) / max(sum((i - xm) ** 2 for i in range(n)), 1e-9)
            su = [t["sunreclaim_after_close_kb"] for t in te]; av = [t["avail_after_close_kb"] for t in te]
            summ["per_trial_series_after_close"] = {"SUnreclaim_KiB_first_last": [su[0], su[-1]], "SUnreclaim_slope_KiB_per_trial": round(slope(su), 2), "MemAvailable_KiB_first_last": [av[0], av[-1]], "MemAvailable_slope_KiB_per_trial": round(slope(av), 2), "n": len(su)}
    else:  # valve probe control
        summ.update({
            "app_ok": f"{sum(1 for t in te if t['app_ok'])}/{len(te)}", "peak_queued_MiB": st([mib(t["peak_queued"]) for t in te]),
            "unreclaim_rise_MiB(max-start)": st([kbm(t["max_sunreclaim_kb"] - t["sunreclaim_start_kb"]) for t in te]),
            "avail_drop_MiB(start-min)": st([kbm(t["avail_start_kb"] - t["min_avail_kb"]) for t in te]),
            "reader_MiBps": st([t["reader_mibps"] for t in te]), "sys_busy_ms": st([t["sys_busy_ms"] for t in te]), "trial_secs": st([t["secs"] for t in te]),
            "cpu_core_s_per_GiB": st([(t["sys_busy_ms"] / 1000.0) / max(t["app_read"] / 2**30, 1e-9) for t in te]),
            "sender_max_send_ms": st([s.get("max_send_ms") for s in se]), "sender_outcomes": {k: sum(1 for t in se if t.get("outcome") == k) for k in sorted({t.get("outcome") for t in se})}})
    log("SUMMARY " + json.dumps(summ))
    # compact memory-over-time series of trial 0
    def row(s):
        if "pipe" in s:
            return (f"t={s['t_ms']/1000:6.2f}s pipe={mib(s['pipe']):7.3f}MiB src_inq={s['src_inq']:7d} tgt_outq={s['tgt_outq']:8d} peer_inq={s['peer_inq']:8d} ap_outq={s['ap_outq']:8d} "
                    f"moved={mib(s['moved']):8.1f} app_read={mib(s['app_read']):8.1f} stall_dst={s['stall_dst']} avail={kbm(s['mem_avail_kb']):7.1f} sunrec={kbm(s['sunreclaim_kb']):7.1f} skb={s['skb_head']}")
        return (f"t={s['t_ms']/1000:6.2f}s fwd={mib(s['fwd']):8.1f} sent={mib(s['sent']):8.1f} q={mib(s['queued']):7.2f} src_inq={s['src_inq']:7d} outq={s['tgt_outq']:8d} "
                f"peer_inq={s['peer_inq']:8d} app_read={mib(s['app_read']):8.1f} avail={kbm(s['mem_avail_kb']):7.1f} sunrec={kbm(s['sunreclaim_kb']):7.1f} skb={s['skb_head']}")
    if samples:
        step = max(1, len(samples) // 24)
        log("series(trial 0, guest):\n" + "\n".join(row(s) for s in samples[::step]) + "\n" + row(samples[-1]))
    if hsamples:
        step = max(1, len(hsamples) // 12)
        log("series(trial 0, host side):\n" + "\n".join(f"t={s['t_ms']/1000:6.2f}s src_inq={s['src_inq']:7d} app_read={mib(s['app_read']):8.1f} avail={kbm(s['mem_avail_kb']):7.1f} sunrec={kbm(s['sunreclaim_kb']):7.1f}" for s in hsamples[::step]))
    if te:
        log("trial_end[0]: " + json.dumps(te[0]))
    log("module stats after: " + gx("cat /proc/evmod")[1].strip())
    if "audit" in MODVARIANT:
        # the audit build prints one line per rmmod: every pair, socket reference, scratch buffer and hook taken must have been released
        _, dm = gx("dmesg | grep 'evmod: AUDIT'")
        lines = [l for l in dm.splitlines() if "AUDIT" in l]
        def parse(l):
            return {k: int(v) for k, v in re.findall(r"(\w+ \w+)=(-?\d+)", l.split("AUDIT", 1)[1].replace("pair alloc", "pair_alloc").replace("sock get", "sock_get"))} if False else dict((m.group(1), int(m.group(2))) for m in re.finditer(r"(pair alloc|pair free|sock get|sock put|buf alloc|buf free|hook|unhook)=(-?\d+)", l.replace("free=", "free_=") if False else l))
        bad = []
        for l in lines:
            nums = [int(x) for x in re.findall(r"=(-?\d+)", l.split("AUDIT", 1)[1])]
            # order: pair alloc,free ; sock get,put ; buf alloc,free ; hook,unhook
            if len(nums) == 8 and not (nums[0] == nums[1] and nums[2] == nums[3] and nums[4] == nums[5] and nums[6] == nums[7]):
                bad.append(l.strip())
        summ["audit"] = {"rmmod_lines": len(lines), "unbalanced": bad[:5], "last": lines[-1].strip() if lines else None}
        log("AUDIT " + json.dumps(summ["audit"]))
    return summ


def main():
    log(f"=== evmod vm {NAMES} {KV}")
    modules_before = preflight()
    # The physical host rebooted onto a fresh kernel, so the STOCK vhost_vsock transport is loaded here on demand (as the previous
    # increment's long-lived host already had it). The snapshot for the "unchanged" assertion is taken after that, and evmod/tls are
    # asserted absent from the physical host separately.
    sh("modprobe vhost_vsock; lsmod | grep -E 'vsock|vhost' | head", check=False)
    modules_before = pathlib.Path("/proc/modules").read_text()
    assert not any(l.split()[0] in ("evmod", "tls") for l in modules_before.splitlines()), "evmod/tls must never be loaded into the physical host"
    build_image()
    vm = VM()
    res = {"host_uname_r": os.uname().release, "scenarios": {}}
    try:
        if not vm.wait_ready():
            log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        rc, t = gx("uname -r; grep -E 'MemTotal|MemAvailable' /proc/meminfo; lsmod | grep -E 'evmod|vsock'; cat /proc/evmod; ls -la /dev/evmod; for f in tcp_rmem tcp_wmem tcp_mem rmem_max; do echo $f=$(cat /proc/sys/net/ipv4/$f 2>/dev/null || cat /proc/sys/net/core/$f); done; echo vsock_buf_default; nproc; cat /sys/devices/system/cpu/online")
        log(t); res["guest_boot"] = t
        if KV.get("adhoc"):
            # adhoc diagnostics: '@@'-separated guest commands, each run in the foreground, output logged (no scenario)
            for c in KV["adhoc"].split("@@"):
                rc_, t_ = gx(c, int(KV.get("adhoc_timeout", "180"))); log(f"ADHOC $ {c}\n{t_}")
            time.sleep(3); sv = [l for l in vm.text().splitlines() if not l.startswith("HB ")]; log("SERIAL tail after adhoc:\n" + "\n".join(sv[-90:]))
            NAMES.clear()
        for nm in NAMES:
            if not vm.alive():
                log("VM died; stopping"); break
            try: res["scenarios"][nm] = run_scenario(vm, nm)
            except Exception as e:
                import traceback; traceback.print_exc(); res["scenarios"][nm] = {"exception": repr(e)}
    finally:
        for p in list(PROCS): reap(p)
        vm.stop()
        sh("pgrep -af 'cloud-hypervisor|evprobe|valveprobe' | grep -v pgrep; true", check=False)
        after = pathlib.Path("/proc/modules").read_text()
        # compare module NAMES and sizes only: field 3 of /proc/modules is a use count that legitimately moves while a VM holds vhost fds
        norm = lambda t: sorted(" ".join(l.split()[:2]) for l in t.splitlines())
        res["physical_modules_unchanged"] = (norm(after) == norm(modules_before))
        res["physical_modules_name_size_diff"] = sorted(set(norm(after)) ^ set(norm(modules_before)))
        res["physical_modules_raw_equal"] = (after == modules_before)
        res["physical_has_evmod_or_tls"] = [l.split()[0] for l in after.splitlines() if l.split()[0] in ("evmod", "tls", "shmproxy")]
        log(f"physical host module names+sizes unchanged: {res['physical_modules_unchanged']} diff={res['physical_modules_name_size_diff']} evmod/tls/shmproxy={res['physical_has_evmod_or_tls']} raw_equal={res['physical_modules_raw_equal']}")
    (EV / "result.json").write_text(json.dumps(res, indent=1))
    print("RESULT_JSON " + json.dumps(res)[:60000])


main()
