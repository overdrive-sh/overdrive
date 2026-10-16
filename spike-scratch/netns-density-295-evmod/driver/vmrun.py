#!/usr/bin/env python3
"""evmod native-metal VM driver (root, under the metal lease; launched by launch.py). Derived from the V-3 / valve vmrun.py.

A real Cloud Hypervisor VM (fork, --vsock backend=vhost-kernel, stock 7.0.0-29 host AND guest kernel). The module under test
(evmod.ko) is loaded INSIDE THE GUEST kernel only; nothing is loaded into the physical host kernel. Guest PID 1 starts
`evprobe agent`; this driver starts host-side processes and asks the agent to start guest-side ones.

Usage: vmrun.py <scenario>[,<scenario>...] [mem=MB]
Directions: A = host sender -> guest vsock source -> evmod -> guest TCP target app
            T = guest TCP source app -> evmod -> guest TCP target app
            G = guest TCP source app -> evmod -> vsock target -> host reader
            ctl-* use the V-3/valve BPF probe (sockmap redirect, no bound) as the same-session control.
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, statistics, struct, subprocess, time

W = pathlib.Path("/tmp/evmod-work"); OUT = W / "out"
NAMES = sys.argv[1].split(","); KV = dict(a.split("=", 1) for a in sys.argv[2:])
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{sys.argv[1].replace(',', '+')[:50]}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3371
MEM_MB = int(KV.get("mem", "1024"))
CH = OUT / "cloud-hypervisor"
PROBE = OUT / "evprobe"
VALVE_PROBE, VALVE_OBJ = OUT / "valveprobe", OUT / "valve-bpf.o"
KO = OUT / "mod-evmod" / "evmod.ko"
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
    for top in ["vmw_vsock_virtio_transport"]:
        for line in sh(["modprobe", "--show-depends", top], quiet=True).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); order.append(name); pins[str(src)] = sha(src)
    (root / "mods/order").write_text("\n".join(order) + "\n")
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
CAP = "--abort-queued-mb 160"


def S(d, n, mb, app, sink="", send="", max_s=30, verbose=1, probe="ev", params=None):
    return dict(dir=d, n=n, bytes=mb * MIB, app=app, sink=sink, send=send, max_s=max_s, verbose=verbose, probe=probe, params=params or {})


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
    "ev-sclose-A": S("A", 5, 64, "f", CAP + " --await-gone 1", "--close-after-mb 8", max_s=20, verbose=0),
    # rmmod while a pair is attached (the controller closed /dev/evmod, so only the live pair is left)
    "ev-unload-T": S("T", 1, 64, "s60000", "--leave-attached 1", "", max_s=30, verbose=0, params={"unload": 1}),
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
PORT = {n: 7300 + i for i, n in enumerate(SCENARIOS)}


def run_scenario(vm, name):
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
    sink_common = f"--port {port} --trials {n} --bytes {nbytes} --label {name} --app {sc['app']} --max-s {sc['max_s']} --verbose-trials {sc['verbose']} {sc['sink']}"
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
        gx(f"{exe_g} sink --kind tcpsrc{objarg} {sink_common} > /dev/ttyS0 2>&1 < /dev/null &", 20)
        if sc["params"].get("unload"):
            time.sleep(4)
            rc, ut = gx("lsmod | grep evmod; cat /proc/evmod; rmmod evmod; echo rmmod_rc=$?; lsmod | grep evmod; echo ---; dmesg | tail -12")
            log("UNLOAD with a live pair:\n" + ut)
            unload_out = ut
        end = time.time() + horizon
        while time.time() < end:
            g, _ = jl(vm.text(), name)
            if any(e["ev"] == "sink_done" for e in g) or not vm.alive(): break
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
    return summ


def main():
    log(f"=== evmod vm {NAMES} {KV}")
    modules_before = preflight()
    sh("modprobe vhost_vsock; lsmod | grep -E 'vsock|vhost' | head", check=False)
    build_image()
    vm = VM()
    res = {"host_uname_r": os.uname().release, "scenarios": {}}
    try:
        if not vm.wait_ready():
            log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        rc, t = gx("uname -r; grep -E 'MemTotal|MemAvailable' /proc/meminfo; lsmod | grep -E 'evmod|vsock'; cat /proc/evmod; ls -la /dev/evmod; for f in tcp_rmem tcp_wmem tcp_mem rmem_max; do echo $f=$(cat /proc/sys/net/ipv4/$f 2>/dev/null || cat /proc/sys/net/core/$f); done; echo vsock_buf_default; nproc; cat /sys/devices/system/cpu/online")
        log(t); res["guest_boot"] = t
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
        res["physical_modules_unchanged"] = (after == modules_before)
        log(f"physical host /proc/modules unchanged: {res['physical_modules_unchanged']}")
    (EV / "result.json").write_text(json.dumps(res, indent=1))
    print("RESULT_JSON " + json.dumps(res)[:60000])


main()
