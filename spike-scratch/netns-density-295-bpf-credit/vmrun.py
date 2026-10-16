#!/usr/bin/env python3
"""bpf-credit native-metal VM driver (root, under the metal lease; launched by launch.py).

A real Cloud Hypervisor VM (fork, --vsock backend=vhost-kernel, stock 7.0.0-29 host AND guest kernel).
Guest PID 1 starts `valveprobe agent`; this driver starts the host-side processes and asks the agent to start
guest-side ones. Everything lives in /tmp/bpf-credit-work; nothing shared is touched.

Usage: vmrun.py <scenario>[,<scenario>...] [mem=MB]
Scenarios are rows of SCENARIOS below. dir A = host sender -> guest owner; B = guest sender -> host owner;
T = guest-local TCP source app -> guest owner.
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, statistics, struct, subprocess, time

W = pathlib.Path("/tmp/bpf-credit-work"); OUT = W / "out"
NAMES = sys.argv[1].split(","); KV = dict(a.split("=", 1) for a in sys.argv[2:])
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{KV.get('mod', 'stock')}-{sys.argv[1].replace(',', '+')[:50]}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3370
MEM_MB = int(KV.get("mem", "1024"))
MODV = KV.get("mod", "stock")   # guest vmw_vsock_virtio_transport_common.ko: stock | rebuilt | p1
CH, PROBE, OBJ, ELIG = OUT / "cloud-hypervisor", OUT / "valveprobe", OUT / "valve-bpf.o", OUT / "eligprobe"
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
           "meminfo_head": pathlib.Path("/proc/meminfo").read_text().splitlines()[:3]}
    (EV / "substrate-probes.json").write_text(json.dumps({**pre, "failures": fail}, indent=1))
    log(f"substrate: {json.dumps(pre)}")
    if fail:
        log(f"FAILED CLOSED: {fail}"); sys.exit(3)


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
    for n in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "touch", "true", "head", "tail", "wc", "date", "grep", "dmesg", "free"]:
        (root / "bin" / n).symlink_to("busybox")
    for b in ["/usr/sbin/ip", "/usr/bin/ss"]:
        if os.path.exists(b): copy_with_libs(root, b)
    shutil.copy2(PROBE, root / "valveprobe"); shutil.copy2(OBJ, root / "valve-bpf.o"); shutil.copy2(ELIG, root / "eligprobe")
    (root / "init").write_text("""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /sys/fs/cgroup /sys/fs/bpf /tmp
mount -t cgroup2 none /sys/fs/cgroup
mount -t bpf bpf /sys/fs/bpf
mount -t debugfs debugfs /sys/kernel/debug
mount -t tmpfs tmpfs /tmp
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
ip link set lo up
ulimit -n 65536
/valveprobe agent > /dev/ttyS0 2>&1 &
( while true; do echo "HB up=$(cat /proc/uptime) $(grep -E 'MemAvailable|SUnreclaim' /proc/meminfo | head -2 | while read k v u; do echo -n "$k$v "; done)" > /dev/ttyS0; sleep 1; done ) &
echo "GUEST_READY"
while true; do sleep 3600; done
""")
    os.chmod(root / "init", 0o755)
    order, pins = [], {}
    for top in ["vmw_vsock_virtio_transport"]:
        for line in sh(["modprobe", "--show-depends", top], quiet=True).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); order.append(name); pins[str(src)] = sha(src)
    if MODV != "stock":
        kv = OUT / "kmod" / f"vmw_vsock_virtio_transport_common.{MODV}.ko"
        shutil.copy2(kv, root / "mods" / "vmw_vsock_virtio_transport_common.ko"); pins["guest_module_variant"] = MODV; pins["guest_module_sha"] = sha(kv)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}", quiet=True)
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "valveprobe": sha(PROBE), "bpf_obj": sha(OBJ), "eligprobe": sha(ELIG), "cloud_hypervisor": sha(CH)})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=1, sort_keys=True))
    log(f"image pins: {json.dumps({k: v[:12] for k, v in pins.items() if not k.startswith('/')})}")


class VM:
    def __init__(self, cpus=4, mem=MEM_MB):
        self.dir = OUT / f"vm-{STAMP}"; self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", "console=ttyS0 panic=-1 loglevel=4",
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
# dir, n trials, bytes, sink args (appended), send args (appended), max_s
HI = "--high-kb 4096"


def S(d, n, mb, app, sink="", send="", max_s=30, verbose=1):
    return dict(dir=d, n=n, bytes=mb * MIB, app=app, sink=sink, send=send, max_s=max_s, verbose=verbose)


SCENARIOS = {
    # --- bpf-credit P1 (credit returned when the redirect releases the skb; needs mod=p1 or mod=rebuilt) -- no valve, abort cap only
    "p1stall-A": S("A", 10, 256, "s3000,f", "--abort-queued-mb 160"),
    "p1long-A": S("A", 3, 64, "s10000,f", "--abort-queued-mb 160", max_s=40),
    "p1slow-A": S("A", 10, 128, "r20", "--abort-queued-mb 160", max_s=60),
    "p1slow2-A": S("A", 5, 16, "r2", "--abort-queued-mb 160", max_s=40),
    "p1cyc-A": S("A", 100, 256, "s300,f:150,s300,f:150,s300,f:150,s300,f", "--abort-queued-mb 160", max_s=30, verbose=0),
    "p1tp1m-A": S("A", 10, 1024, "f", "--high-kb 0 --verify 0 --vsock-buf 1048576", "--fast-gen 1 --vsock-buf 1048576", verbose=0),
    "p1tp4m-A": S("A", 10, 1024, "f", "--high-kb 0 --verify 0 --vsock-buf 4194304", "--fast-gen 1 --vsock-buf 4194304", verbose=0),
    "p1tp16m-A": S("A", 10, 1024, "f", "--high-kb 0 --verify 0 --vsock-buf 16777216", "--fast-gen 1 --vsock-buf 16777216", verbose=0),
    "p1stall1m-A": S("A", 5, 256, "s3000,f", "--abort-queued-mb 160 --vsock-buf 1048576", "--vsock-buf 1048576"),
    "p1stall4m-A": S("A", 5, 256, "s3000,f", "--abort-queued-mb 160 --vsock-buf 4194304", "--vsock-buf 4194304"),
    "p1ver1m-A": S("A", 10, 1024, "f", "--abort-queued-mb 160 --vsock-buf 1048576", "--vsock-buf 1048576", verbose=0),
    "p1cyc1m-A": S("A", 100, 256, "s300,f:150,s300,f:150,s300,f:150,s300,f", "--abort-queued-mb 160 --vsock-buf 1048576", "--vsock-buf 1048576", max_s=30, verbose=0),
    "p1slow1m-A": S("A", 10, 128, "r20", "--abort-queued-mb 160 --vsock-buf 1048576", "--vsock-buf 1048576", max_s=60),
    "lat1m-A": S("A", 10, 1, "f", "--high-kb 0 --lat 2000 --lat-interval-us 300 --vsock-buf 1048576", "--lat 2000 --lat-interval-us 300 --vsock-buf 1048576", max_s=20, verbose=0),
    "p1abort1m-A": S("A", 20, 64, "s10000,f", "--abort-queued-mb 160 --vsock-buf 1048576", "--vsock-buf 1048576", max_s=3, verbose=0),
    "p1abort-A": S("A", 20, 64, "s10000,f", "--abort-queued-mb 160", max_s=3, verbose=0),
    "p1ver-A": S("A", 10, 1024, "f", "--abort-queued-mb 160", verbose=0),
    # controls: no valve (reproduces V-3 with byte-verified streams); the abort cap protects the guest
    "ctl-A": S("A", 3, 256, "s3000,f", "--abort-queued-mb 160"),
    "ctl-B": S("B", 3, 256, "s3000,f", "--abort-queued-mb 160"),
    "ctl-T": S("T", 3, 256, "s3000,f", "--abort-queued-mb 160"),
    # candidate 1: valve (owner removes the source at HIGH) + owner drain in order (pump) at queued == 0
    "pump-A": S("A", 20, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump"),
    "pump-B": S("B", 20, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump"),
    "pump-T": S("T", 20, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump"),
    # candidate 1 with a forced wake (TCP only: SO_RCVLOWAT -> tcp_data_ready)
    "kick-T": S("T", 20, 256, "s3000,f", f"{HI} --low-kb 1024 --resume kick"),
    # candidate 1 naive re-insert (no residue handling, no wake): the V-3 deadlock
    "naive-A": S("A", 3, 256, "s3000,f", f"{HI} --low-kb 0 --resume naive", max_s=25),
    "naive-T": S("T", 3, 256, "s3000,f", f"{HI} --low-kb 1024 --resume naive", max_s=25),
    # slow but steady reader
    "slow-A": S("A", 10, 128, "r20", f"{HI} --low-kb 0 --resume pump --gate lowat", max_s=60),
    "slow-B": S("B", 10, 128, "r20", f"{HI} --low-kb 0 --resume pump --gate lowat", max_s=60),
    "slow-T": S("T", 10, 128, "r20", f"{HI} --low-kb 0 --resume pump", max_s=60),
    "slowk-T": S("T", 10, 128, "r20", "--high-kb 4096 --low-kb 1024 --resume kick", max_s=60),
    # repeated stall / resume cycles
    "cyc-A": S("A", 10, 256, "s1000,f:300,s1000,f:300,s1000,f", f"{HI} --low-kb 0 --resume pump --gate lowat"),
    "cyc-B": S("B", 10, 256, "s1000,f:300,s1000,f:300,s1000,f", f"{HI} --low-kb 0 --resume pump --gate lowat"),
    "cyc-T": S("T", 10, 256, "s1000,f:300,s1000,f:300,s1000,f", "--high-kb 4096 --low-kb 1024 --resume kick"),
    # fast reader: cost of having the valve armed vs not
    "fast-off-A": S("A", 10, 1024, "f", "--high-kb 0", verbose=0),
    "fast-on-A": S("A", 10, 1024, "f", f"{HI} --low-kb 0 --resume pump", verbose=0),
    "fast-off-B": S("B", 10, 1024, "f", "--high-kb 0", verbose=0),
    "fast-on-B": S("B", 10, 1024, "f", f"{HI} --low-kb 0 --resume pump", verbose=0),
    "fast-off-T": S("T", 10, 1024, "f", "--high-kb 0", verbose=0),
    "fast-on-T": S("T", 10, 1024, "f", f"{HI} --low-kb 0 --resume pump", verbose=0),
    # pump then re-join the fast path, with a bursty sender (tail stranding hazard)
    "rejoin-A": S("A", 10, 64, "s2000,f", f"{HI} --low-kb 0 --resume pump --rejoin 1", "--gap-ms 100 --burst-kb 1024", max_s=120),
    "rejoin-B": S("B", 10, 64, "s2000,f", f"{HI} --low-kb 0 --resume pump --rejoin 1", "--gap-ms 100 --burst-kb 1024", max_s=120),
    "burst-A": S("A", 10, 64, "f", f"{HI} --low-kb 0 --resume pump --rejoin 1", "--gap-ms 100 --burst-kb 1024", max_s=120),
    # candidate 2: verdict returns SK_PASS once queued >= HIGH; native knobs; ingress redirect; owner latency; hard drop
    "pass-A": S("A", 3, 256, "s3000,f", f"{HI} --mode 1 --resume none --abort-queued-mb 160", max_s=40),
    "pass-T": S("T", 3, 256, "s3000,f", f"{HI} --mode 1 --resume none --abort-queued-mb 160", max_s=40),
    "knobs-A": S("A", 3, 256, "s3000,f", "--abort-queued-mb 160 --vsock-buf 65536 --tgt-sndbuf 4096", "--vsock-buf 65536", max_s=40),
    "knobs-T": S("T", 3, 256, "s3000,f", "--abort-queued-mb 160 --src-rcvbuf 4096 --tgt-sndbuf 4096", max_s=40),
    "delay20-A": S("A", 5, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump --owner-delay-ms 20"),
    "delay100-A": S("A", 5, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump --owner-delay-ms 100"),
    "hard-A": S("A", 3, 256, "s3000,f", "--high-kb 4096 --mode 2 --hard-kb 8192 --resume none", max_s=40),
    "ingress-T": S("T", 1, 128, "f", "--redir-flags 1 --abort-queued-mb 0", max_s=12),
    # candidate 2 proper (TCP source): the verdict parks bytes with SK_PASS (they stay charged to the source socket, so the
    # TCP window closes) and the owner forwards them in order; the verdict resumes the redirect by itself when PARKED == FORWARDED
    "park-T": S("T", 20, 256, "s3000,f", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160"),
    "parkslow-T": S("T", 10, 128, "r20", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160", max_s=60),
    "parkcyc-T": S("T", 10, 256, "s1000,f:300,s1000,f:300,s1000,f", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160"),
    "park-A": S("A", 3, 256, "s3000,f", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160", max_s=40),
    # lowat gate + removal (vsock): SO_RCVLOWAT stops new reads (vsock_data_ready) before the source is removed
    "gate-A": S("A", 40, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump --gate lowat"),
    "gate-B": S("B", 40, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump --gate lowat"),
    "gate1m-A": S("A", 40, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump --gate lowat --vsock-buf 1048576", "--vsock-buf 1048576"),
    # control for the race: removal with no gate, with the read_skb return counter armed
    "nogate-A": S("A", 40, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump"),
    "nogate-B": S("B", 40, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump"),
    "nogate-T": S("T", 40, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump"),
    # the owner never puts the flow in a sockmap: it reads the source and writes the target itself
    "purepump-A": S("A", 10, 256, "s3000,f", "--force-pump 1"),
    "purepump-B": S("B", 10, 256, "s3000,f", "--force-pump 1"),
    "purepump-T": S("T", 10, 256, "s3000,f", "--force-pump 1"),
    "purepumpslow-A": S("A", 10, 128, "r20", "--force-pump 1", max_s=60),
    "purepumpslow-B": S("B", 10, 128, "r20", "--force-pump 1", max_s=60),
    "purepumpslow-T": S("T", 10, 128, "r20", "--force-pump 1", max_s=60),
    "detect-A": S("A", 20, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump", max_s=12),
    "detect-B": S("B", 20, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump", max_s=12),
    "detect-T": S("T", 20, 256, "s3000,f", f"{HI} --low-kb 0 --resume pump", max_s=12),
    "tp-valve-A": S("A", 10, 1024, "f", f"{HI} --low-kb 0 --resume pump --gate lowat --verify 0", "--fast-gen 1", verbose=0),
    "tp-valve-B": S("B", 10, 1024, "f", f"{HI} --low-kb 0 --resume pump --gate lowat --verify 0", "--fast-gen 1", verbose=0),
    "tp-park-T": S("T", 10, 1024, "f", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --verify 0 --fast-gen 1", verbose=0),
    "gatefast-A": S("A", 100, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --verify 0", "--fast-gen 1", max_s=12, verbose=0),
    "nogatefast-A": S("A", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --verify 0", "--fast-gen 1", max_s=12, verbose=0),
    "gatefast-B": S("B", 100, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --verify 0", "--fast-gen 1", max_s=12, verbose=0),
    "nogatefast-B": S("B", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --verify 0", "--fast-gen 1", max_s=12, verbose=0),
    "nogatefast-T": S("T", 100, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --verify 0 --fast-gen 1", max_s=12, verbose=0),
    "gd1ms-A": S("A", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --gate-delay-us 1000 --verify 0", "--fast-gen 1", max_s=12, verbose=0),
    "gd5ms-A": S("A", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --gate-delay-us 5000 --verify 0", "--fast-gen 1", max_s=12, verbose=0),
    "gd1ms-1m-A": S("A", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --gate-delay-us 1000 --verify 0 --vsock-buf 1048576", "--fast-gen 1 --vsock-buf 1048576", max_s=12, verbose=0),
    "gd5ms-1m-A": S("A", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --gate-delay-us 5000 --verify 0 --vsock-buf 1048576", "--fast-gen 1 --vsock-buf 1048576", max_s=12, verbose=0),
    "gd20ms-1m-A": S("A", 40, 512, "s300,f", f"{HI} --low-kb 0 --resume pump --gate lowat --gate-delay-us 20000 --verify 0 --vsock-buf 1048576", "--fast-gen 1 --vsock-buf 1048576", max_s=12, verbose=0),
    "rejoingate-A": S("A", 20, 64, "s2000,f", f"{HI} --low-kb 0 --resume pump --rejoin 1 --gate lowat", "--gap-ms 100 --burst-kb 1024", max_s=60),
    "rejoingate-B": S("B", 20, 64, "s2000,f", f"{HI} --low-kb 0 --resume pump --rejoin 1 --gate lowat", "--gap-ms 100 --burst-kb 1024", max_s=60),
    "parkstress-T": S("T", 100, 256, "s300,f:150,s300,f:150,s300,f:150,s300,f", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160", max_s=30, verbose=0),
    "parkslow2-T": S("T", 5, 16, "r2", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160", max_s=40),
    "parklong-T": S("T", 3, 64, "s10000,f", "--high-kb 4096 --low-kb 0 --mode 3 --resume park --abort-queued-mb 160", max_s=40),
    "lat-sockmap-A": S("A", 10, 1, "f", "--high-kb 0 --lat 2000 --lat-interval-us 300", "--lat 2000 --lat-interval-us 300", max_s=20, verbose=0),
    "lat-pump-A": S("A", 10, 1, "f", "--force-pump 1 --lat 2000 --lat-interval-us 300", "--lat 2000 --lat-interval-us 300", max_s=20, verbose=0),
    "lat-sockmap-B": S("B", 10, 1, "f", "--high-kb 0 --lat 2000 --lat-interval-us 300", "--lat 2000 --lat-interval-us 300", max_s=20, verbose=0),
    "lat-pump-B": S("B", 10, 1, "f", "--force-pump 1 --lat 2000 --lat-interval-us 300", "--lat 2000 --lat-interval-us 300", max_s=20, verbose=0),
    "lat-sockmap-T": S("T", 10, 1, "f", "--high-kb 0 --lat 2000 --lat-interval-us 300", "--lat 2000 --lat-interval-us 300", max_s=20, verbose=0),
    "lat-pump-T": S("T", 10, 1, "f", "--force-pump 1 --lat 2000 --lat-interval-us 300", "--lat 2000 --lat-interval-us 300", max_s=20, verbose=0),
    "tp-sockmap-A": S("A", 10, 1024, "f", "--high-kb 0 --verify 0", "--fast-gen 1", verbose=0),
    "tp-pump-A": S("A", 10, 1024, "f", "--force-pump 1 --verify 0", "--fast-gen 1", verbose=0),
    "tp-sockmap-B": S("B", 10, 1024, "f", "--high-kb 0 --verify 0", "--fast-gen 1", verbose=0),
    "tp-pump-B": S("B", 10, 1024, "f", "--force-pump 1 --verify 0", "--fast-gen 1", verbose=0),
    "tp-sockmap-T": S("T", 10, 1024, "f", "--high-kb 0 --verify 0 --fast-gen 1", verbose=0),
    "tp-pump-T": S("T", 10, 1024, "f", "--force-pump 1 --verify 0 --fast-gen 1", verbose=0),
}
PORT = {n: 7100 + i for i, n in enumerate(SCENARIOS)}


def run_scenario(vm, name):
    sc = SCENARIOS[name]; port = PORT[name]
    d, n = sc["dir"], sc["n"]
    log(f"##### scenario {name}: {json.dumps(sc)}")
    sink_common = f"--port {port} --trials {n} --bytes {sc['bytes']} --label {name} --app {sc['app']} --max-s {sc['max_s']} --verbose-trials {sc['verbose']} {sc['sink']}"
    t0 = time.time()
    if d == "A":
        gx(f"/valveprobe sink --kind vsock --obj /valve-bpf.o {sink_common} > /dev/ttyS0 2>&1 < /dev/null &", 20)
        time.sleep(1.5)
        p = spawn(f"{name}-send", [str(PROBE), "send", "--cid", str(CID), "--port", str(port), "--bytes", str(sc["bytes"]), "--trials", str(n), "--label", name] + sc["send"].split())
        horizon = n * (sc["max_s"] + 20) + 60
        try: p.wait(timeout=horizon)
        except subprocess.TimeoutExpired: log("sender timeout"); reap(p)
        end = time.time() + 40
        while time.time() < end:
            g, _ = jl(vm.text(), name)
            if any(e["ev"] == "sink_done" for e in g): break
            time.sleep(0.5)
        g, bad = jl(vm.text(), name); h, _ = jl((EV / f"proc-{name}-send.log").read_text(), name)
    elif d == "B":
        hp = spawn(f"{name}-sink", [str(PROBE), "sink", "--kind", "vsock", "--obj", str(OBJ)] + sink_common.split())
        time.sleep(2)
        sendv = [a for a in sc["send"].split()]
        gx(f"/valveprobe send --cid 2 --port {port} --bytes {sc['bytes']} --trials {n} --label {name} {sc['send']} > /dev/ttyS0 2>&1 < /dev/null &", 20)
        horizon = n * (sc["max_s"] + 20) + 60
        try: hp.wait(timeout=horizon)
        except subprocess.TimeoutExpired: log("host sink timeout"); reap(hp)
        time.sleep(2)
        g, bad = jl((EV / f"proc-{name}-sink.log").read_text(), name); h, _ = jl(vm.text(), name)
    else:
        gx(f"/valveprobe sink --kind tcpsrc --obj /valve-bpf.o {sink_common} > /dev/ttyS0 2>&1 < /dev/null &", 20)
        horizon = n * (sc["max_s"] + 20) + 60
        end = time.time() + horizon
        while time.time() < end:
            g, _ = jl(vm.text(), name)
            if any(e["ev"] == "sink_done" for e in g) or not vm.alive(): break
            time.sleep(0.5)
        g, bad = jl(vm.text(), name); h = []
    wall = time.time() - t0
    sink_text_src = (EV / f"proc-{name}-sink.log").read_text() if d == "B" else vm.text()
    oom = [l for l in vm.text().splitlines() if re.search(r"Out of memory|oom-kill|Kernel panic|invoked oom|Killed process|page allocation failure|BUG:|WARNING:|Oops|Call Trace|general protection|refcount_t|use-after-free|bpf-credit|lockdep|RCU stall|hung task", l)]
    te = [e for e in g if e["ev"] == "trial_end"]; se = [e for e in h if e["ev"] == "send_end"]
    samples = [e for e in g if e["ev"] == "sample" and e["trial"] == 0]
    (EV / f"{name}-trial_end.jsonl").write_text("\n".join(json.dumps(e) for e in te))
    (EV / f"{name}-send_end.jsonl").write_text("\n".join(json.dumps(e) for e in se))
    (EV / f"{name}-samples-trial0.jsonl").write_text("\n".join(json.dumps(e) for e in samples))
    # derived per-trial metrics
    lat, lat_src = [], []
    for i, t in enumerate(te):
        if not t.get("stall_ends"): continue
        t_res = t["stall_ends"][0][0]
        if d == "T" and t.get("writer"):
            bl = t["writer"]["blocks"]
            if bl:
                b = max(bl, key=lambda x: x[1]); lat.append((b[0] - t_res) / 1e6)
        elif d in "AB":
            s = next((x for x in se if x.get("trial") == t["trial"]), None)
            if s and s.get("blocks"):
                b = max(s["blocks"], key=lambda x: x[1]); lat.append((b[0] + s["off_ns"] - t_res) / 1e6)
    peaks = [mib(t["peak_queued"]) for t in te]
    summ = {
        "scenario": name, "dir": d, "wall_s": round(wall, 1), "trials_planned": n, "trial_end": len(te), "send_end": len(se),
        "outcomes": {k: sum(1 for t in te if t["outcome"] == k) for k in sorted({t["outcome"] for t in te})},
        "send_outcomes": {k: sum(1 for t in se if t.get("outcome") == k) for k in sorted({t.get("outcome") for t in se})},
        "app_ok": f"{sum(1 for t in te if t['app_ok'])}/{len(te)}",
        "bytes_verified_total_MiB": mib(sum(t["app_read"] for t in te if t["app_ok"])),
        "first_bad": [t["first_bad"] for t in te if t["first_bad"] >= 0][:5],
        "hash_eq": f"{sum(1 for t in te if t['hash'] == t['ehash'] and t['app_read'] == t['bytes'])}/{len(te)}",
        "peak_queued_MiB": st(peaks),
        "queued_at_remove_MiB": st([mib(q) for t in te for q in t["close_queued"]]),
        "unreclaim_rise_MiB(max-start)": st([kbm(t["max_sunreclaim_kb"] - t["sunreclaim_start_kb"]) for t in te]),
        "avail_drop_MiB(start-min)": st([kbm(t["avail_start_kb"] - t["min_avail_kb"]) for t in te]),
        "unreclaim_after_close_minus_start_MiB": st([kbm(t["sunreclaim_after_close_kb"] - t["sunreclaim_start_kb"]) for t in te]),
        "valve_closes": st([t["valve_closes"] for t in te]), "resumes": st([t["resumes"] for t in te]),
        "pump_MiB": st([mib(t["pump_recv"]) for t in te]), "pump_ms": st([t["pump_ms"] for t in te]),
        "rejoins": sum(t["rejoins"] for t in te), "rejoin_fail": sum(t["rejoin_fail"] for t in te),
        "refused": sum(t["refused"] for t in te), "hard_drops": sum(t["hard_drops"] for t in te), "passed": sum(t["passed"] for t in te),
        "rb_fail": sum(t["rb_fail"] for t in te),
        "rs_zero_total(kretprobe read_skb==0)": sum(t["rs_zero"] for t in te), "vr_zero_total(fexit sk_psock_verdict_recv==0)": sum(t["vr_zero"] for t in te), "vr_ok_total": sum(t["vr_ok"] for t in te), "trials_with_vr_zero": sum(1 for t in te if t["vr_zero"] > 0), "rs_ok_total": sum(t["rs_ok"] for t in te),
        "parked_MiB": st([mib(t["parked"]) for t in te]), "forwarded_MiB": st([mib(t["forwarded"]) for t in te]),
        "lost_bytes_per_trial": [t["bytes"] - t["app_read"] for t in te if t["bytes"] != t["app_read"]][:12],
        "queued_end_MiB": st([mib(t["queued_end"]) for t in te]),
        "src_inq_end": st([t["src_inq_end"] for t in te]),
        "reader_MiBps": st([t["reader_mibps"] for t in te]), "owner_cpu_ms": st([t["owner_cpu_ms"] for t in te]), "sys_busy_ms": st([t["sys_busy_ms"] for t in te]), "owner_trial_secs": st([t["secs"] for t in te]),
        "sender_MiBps": st([s.get("mibps") for s in se]), "sender_max_send_ms": st([s.get("max_send_ms") for s in se]),
        "sender_sent_at_longest_block_start_MiB": st([mib(s.get("sent_at_longest_block_start", 0)) for s in se]),
        "resume_latency_ms(unblock-resume)": st(lat),
        "final_state": {k: sum(1 for t in te if t["final_state"] == k) for k in sorted({t["final_state"] for t in te})},
        "oom_lines": oom[:6], "bad_json_lines": bad,
    }
    lp = {k: [] for k in ("min", "p50", "p90", "p99", "max")}
    for t in te:
        L = t.get("lat_raw_ns")
        if not L or not L["count"]: continue
        off = 0
        if d in "AB":
            sx = next((x for x in se if x.get("trial") == t["trial"]), None)
            off = sx["off_ns"] if sx else 0
        for k in lp: lp[k].append((L[k] - off) / 1e3)
    if lp["p50"]:
        summ["lat_us_(recv-send)"] = {k: st(v) for k, v in lp.items()}
        summ["lat_records"] = sum(t["lat_raw_ns"]["count"] for t in te)
    log("SUMMARY " + json.dumps(summ))
    # compact series of trial 0
    def row(s):
        return (f"t={s['t_ms']/1000:6.2f}s {s['state']:6s} fwd={mib(s['fwd']):8.1f} sent={mib(s['sent']):8.1f} q={mib(s['queued']):7.2f} peak={mib(s['peak']):7.2f} src_inq={s['src_inq']:7d} "
                f"outq={s['tgt_outq']:8d} peer_inq={s['peer_inq']:8d} parked={mib(s['parked']):6.2f} rs0={s['rs_zero']} app_read={mib(s['app_read']):8.1f} pumped={mib(s['pumped']):7.1f} avail={kbm(s['mem_avail_kb']):7.1f} sunrec={kbm(s['sunreclaim_kb']):7.1f} skb={s['skb_head']}")
    if samples:
        step = max(1, len(samples) // 22)
        log("series(trial 0):\n" + "\n".join(row(s) for s in samples[::step]) + "\n" + row(samples[-1]))
    if te:
        log("trial_end[0]: " + json.dumps(te[0]))
    return summ


def run_elig(vm):
    """Eligibility matrix on the host kernel and inside the guest (same stock 7.0.0-29), raw output kept."""
    log("##### elig: host")
    out = sh([str(ELIG)], check=False, timeout=300, quiet=True)
    (EV / "elig-host.jsonl").write_text(out)
    gl = sh("grep -c . /sys/kernel/debug/error_injection/list; grep -E 'vsock|psock|skmsg|virtio|sock_map|tcp_|skb_send|page_pool|net_realloc' /sys/kernel/debug/error_injection/list; grep -E 'BPF_KPROBE_OVERRIDE|BPF_LSM|FUNCTION_ERROR_INJECTION|FAILSLAB|FAIL_FUNCTION' /boot/config-$(uname -r); cat /sys/kernel/security/lsm; echo", check=False, quiet=True)
    (EV / "elig-host-context.txt").write_text(gl)
    log("##### elig: guest")
    rc, t = gx("uname -r; /eligprobe; echo '---ctx'; grep -c . /sys/kernel/debug/error_injection/list; grep -E 'vsock|psock|skmsg|virtio|sock_map|tcp_|skb_send|page_pool|net_realloc' /sys/kernel/debug/error_injection/list; cat /sys/kernel/security/lsm; ls /sys/kernel/btf", 300)
    (EV / "elig-guest.jsonl").write_text(t)
    log(f"elig host rows={len(out.splitlines())} guest rc={rc} bytes={len(t)}")
    return {"host_rows": len(out.splitlines()), "guest_rc": rc}


def main():
    log(f"=== valve vm {NAMES} {KV}")
    preflight()
    sh("modprobe vhost_vsock; lsmod | grep -E 'vsock|vhost' | head", check=False)
    build_image()
    vm = VM()
    res = {"host_uname_r": os.uname().release, "scenarios": {}}
    try:
        if not vm.wait_ready():
            log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        log("guest insmod failures: " + str([l for l in vm.text().splitlines() if "INSMOD_FAIL" in l]) + f" module variant={MODV}")
        rc, t = gx("uname -r; ls /sys/kernel/btf/vmlinux; grep -E 'MemTotal|MemAvailable' /proc/meminfo")
        log(t); res["guest_boot"] = t
        for nm in NAMES:
            if not vm.alive():
                log("VM died; stopping"); break
            try: res["scenarios"][nm] = run_elig(vm) if nm == "elig" else run_scenario(vm, nm)
            except Exception as e:
                import traceback; traceback.print_exc(); res["scenarios"][nm] = {"exception": repr(e)}
            gx("pkill valveprobe 2>/dev/null; true", 20) if False else None
    finally:
        for p in list(PROCS): reap(p)
        vm.stop()
        sh("pgrep -af 'cloud-hypervisor|valveprobe' | grep -v pgrep; true", check=False)
    (EV / "result.json").write_text(json.dumps(res, indent=1))
    print("RESULT_JSON " + json.dumps(res)[:60000])


main()
