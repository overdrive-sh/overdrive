#!/usr/bin/env python3
"""V-3 native-metal VM driver (root, under the metal lease; launched by launch.py).

A real Cloud Hypervisor VM (fork, --vsock backend=vhost-kernel, stock 7.0.0-29 host AND guest kernel).
Guest PID 1 starts `v3probe agent`; this driver starts the host-side processes and asks the agent to start
guest-side ones. Everything lives in /tmp/v3-work; nothing shared is touched.

Usage: vmrun.py <stage> [key=value ...]
  boot | c0 | c1 | c1long | c3 | c4
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, statistics, struct, subprocess, time

W = pathlib.Path("/tmp/v3-work"); OUT = W / "out"
STAGE = sys.argv[1]; KV = dict(a.split("=", 1) for a in sys.argv[2:])
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{STAGE}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3370
MEM_MB = int(KV.get("mem", "1024"))
CH, PROBE, OBJ = OUT / "cloud-hypervisor", OUT / "v3probe", OUT / "v3-bpf.o"
PROCS = []
EV.mkdir(parents=True, exist_ok=False)
LOG = (EV / "run.log").open("a")


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
    shutil.copy2(PROBE, root / "v3probe"); shutil.copy2(OBJ, root / "v3-bpf.o")
    (root / "init").write_text("""#!/bin/sh
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
ip link set lo up
ulimit -n 65536
/v3probe agent > /dev/ttyS0 2>&1 &
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
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "v3probe": sha(PROBE), "bpf_obj": sha(OBJ), "cloud_hypervisor": sha(CH)})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=1, sort_keys=True))
    log(f"image pins: {json.dumps({k: v[:12] for k, v in pins.items() if not k.startswith('/')})}")


class VM:
    def __init__(self, cpus=4, mem=MEM_MB):
        self.dir = OUT / f"vm-{STAGE}-{STAMP}"; self.dir.mkdir()
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


def jl(text):
    ev, bad = [], 0
    for l in text.splitlines():
        i = l.find("{\"ev\"")
        if i < 0: continue
        try: ev.append(json.loads(l[i:]))
        except Exception: bad += 1
    return ev, bad


def wait_for(pred, timeout, vm=None):
    end = time.time() + timeout
    while time.time() < end:
        if pred(): return True
        if vm is not None and not vm.alive(): return False
        time.sleep(0.5)
    return False


MIB = 1048576
def mib(x): return round(x / MIB, 1)
def kb_mib(x): return round(x / 1024, 1)


def series_table(samples, every=1):
    rows = []
    for i, s in enumerate(samples):
        if i % every: continue
        rows.append(f"t={s['t_ms']/1000:7.2f}s fwd={mib(s['fwd']):9.1f}MiB sent={mib(s['sent']):9.1f}MiB queued={mib(s['queued']):9.1f}MiB "
                    f"avail={kb_mib(s['mem_avail_kb']):8.1f}MiB sunrec={kb_mib(s['sunreclaim_kb']):8.1f}MiB slab={kb_mib(s['slab_kb']):8.1f}MiB "
                    f"tcpmem={s['tcp_mem_pages']*4/1024:6.1f}MiB skb_head={s['skbuff_head_active']:8d} outq={s['tgt_outq']} peer_inq={s['peer_inq']} "
                    f"src_inq={s['src_inq']} sendfail={s['send_fail']} drops={s['drops']} errno={s['last_errno']} top={s['slab_top_mib']}"
                    + (f" CG cur={mib(s['cg_current'])}MiB anon={mib(s['cg_anon'])} kernel={mib(s['cg_kernel'])} slab={mib(s['cg_slab'])} sock={mib(s['cg_sock'])} oomk={s['cg_oom_kill']}" if 'cg_current' in s else ""))
    return rows


def stage_boot(vm):
    rc, t = gx("uname -r; ls /sys/kernel/btf/vmlinux; ip -br link; cat /proc/meminfo | head -3; ls -l /proc/slabinfo")
    log(t); return {"guest_uname": t.splitlines()[0] if t else None}


def stage_c0(vm):
    """credit control: vsock flow, application never reads, NO sockmap. Sender must stop at the credit window."""
    gx("/v3probe sink --kind plain --port 7001 --idle-s 10 --max-s 60 --sample-ms 500 --label c0 > /dev/ttyS0 2>&1 < /dev/null &", 20)
    time.sleep(1.5)
    p = spawn("c0-send", [str(PROBE), "send", "--cid", str(CID), "--port", "7001", "--cap-bytes", str(256 * MIB), "--stall-s", "5", "--label", "c0"])
    p.wait(timeout=120)
    wait_for(lambda: any(e["ev"] == "trial_end" and e["label"] == "c0" for e in jl(vm.text())[0]), 30, vm)
    g, bad = jl(vm.text()); h, _ = jl((EV / "proc-c0-send.log").read_text())
    te = [e for e in g if e["ev"] == "trial_end" and e["label"] == "c0"]
    se = [e for e in h if e["ev"] == "send_end"]
    samples = [e for e in g if e["ev"] == "sample" and e["label"] == "c0"]
    log("\n".join(series_table(samples, 2)))
    return {"sink_trial_end": te, "send_end": se, "max_src_inq": max([s["src_inq"] for s in samples], default=None), "bad_json_lines": bad}


def stage_c1(vm):
    n = int(KV.get("n", "100")); b = int(KV.get("bytes", str(16 * MIB)))
    gx(f"/v3probe sink --kind vsock --port 7002 --trials {n} --bytes {b} --hold-ms 200 --sample-ms 100 --samples 0 --idle-s 20 --label c1 > /dev/ttyS0 2>&1 < /dev/null &", 20)
    time.sleep(1.5)
    p = spawn("c1-send", [str(PROBE), "send", "--cid", str(CID), "--port", "7002", "--trials", str(n), "--bytes", str(b), "--stall-s", "10", "--verbose", "0", "--label", "c1"])
    try: p.wait(timeout=1500)
    except subprocess.TimeoutExpired: log("sender timeout"); reap(p)
    wait_for(lambda: sum(1 for e in jl(vm.text())[0] if e["ev"] == "trial_end" and e["label"] == "c1") >= n, 60, vm)
    g, bad = jl(vm.text()); h, _ = jl((EV / "proc-c1-send.log").read_text())
    te = [e for e in g if e["ev"] == "trial_end" and e["label"] == "c1"]
    se = [e for e in h if e["ev"] == "send_end"]
    (EV / "c1-trial_end.jsonl").write_text("\n".join(json.dumps(e) for e in te))
    (EV / "c1-send_end.jsonl").write_text("\n".join(json.dumps(e) for e in se))
    def st(xs): return {"min": min(xs), "median": statistics.median(xs), "max": max(xs)} if xs else None
    out = {"n_requested": n, "bytes_per_trial": b, "trial_end_count": len(te), "send_end_count": len(se),
           "sink_outcomes": {k: sum(1 for e in te if e["outcome"] == k) for k in {e["outcome"] for e in te}},
           "send_outcomes": {k: sum(1 for e in se if e["outcome"] == k) for k in {e["outcome"] for e in se}},
           "fwd_eq_bytes": sum(1 for e in te if e["fwd"] >= b),
           "queued_end_MiB": st([mib(e["queued_end"]) for e in te]),
           "peak_queued_MiB": st([mib(e["peak_queued"]) for e in te]),
           "sent_MiB_at_end": st([mib(e["sent"]) for e in te]),
           "drops_total": sum(e["drops"] for e in te),
           "avail_drop_MiB(start-end)": st([kb_mib(e["avail_start_kb"] - e["avail_end_kb"]) for e in te]),
           "sunreclaim_rise_MiB(end-start)": st([kb_mib(e["sunreclaim_end_kb"] - e["sunreclaim_start_kb"]) for e in te]),
           "avail_after_close_minus_start_MiB": st([kb_mib(e["avail_after_close_kb"] - e["avail_start_kb"]) for e in te]),
           "sunreclaim_after_close_minus_start_MiB": st([kb_mib(e["sunreclaim_after_close_kb"] - e["sunreclaim_start_kb"]) for e in te]),
           "sender_mbps": st([e.get("mbps", 0) for e in se if "mbps" in e]),
           "sender_max_send_ms": st([e.get("max_send_ms", 0) for e in se if "max_send_ms" in e]),
           "sender_slow_sends_total": sum(e.get("slow_sends", 0) for e in se), "bad_json_lines": bad,
           "first_trial": te[0] if te else None, "last_trial": te[-1] if te else None}
    return out


def stage_c1long(vm):
    cap = int(KV.get("cap", str(4096 * MIB)))
    gx(f"/v3probe sink --kind vsock --port 7003 --trials 1 --bytes 0 --idle-s 20 --max-s 600 --sample-ms {KV.get('sample_ms', '50')} --abort-queued-mb {KV.get('abort', '0')} {KV.get('extra', '')} --label c1long > /dev/ttyS0 2>&1 < /dev/null &", 20)
    time.sleep(1.5)
    p = spawn("c1long-send", [str(PROBE), "send", "--cid", str(CID), "--port", "7003", "--cap-bytes", str(cap), "--stall-s", "30", "--label", "c1long"])
    t0 = time.time(); died_at = None; sender_done_at = None
    while time.time() - t0 < 720:
        if p.poll() is not None:
            sender_done_at = sender_done_at or time.time()
            if any(e["ev"] == "trial_end" for e in jl(vm.text())[0]) or time.time() - sender_done_at > 25: break
        if not vm.alive():
            died_at = time.time() - t0; log(f"VM process exited at +{died_at:.1f}s"); break
        time.sleep(1)
    reap(p)
    rc, post = gx("grep -E 'MemTotal|MemAvailable|^Slab|SUnreclaim' /proc/meminfo; ls /sys/fs/cgroup | head; cat /sys/fs/cgroup/v3sink/memory.events 2>&1 | head -8", 20)
    log("post-run guest state:\n" + post)
    g, bad = jl(vm.text()); h, _ = jl((EV / "proc-c1long-send.log").read_text())
    samples = [e for e in g if e["ev"] == "sample"]
    te = [e for e in g if e["ev"] == "trial_end"]
    se = [e for e in h if e["ev"] == "send_end"]
    sp = [e for e in h if e["ev"] == "send_prog"]
    log("\n".join(series_table(samples, max(1, len(samples) // 40))))
    log("last 5 samples:\n" + "\n".join(series_table(samples[-5:])))
    oom = [l for l in vm.text().splitlines() if re.search(r"Out of memory|oom-kill|Kernel panic|invoked oom|Killed process|page allocation failure", l)]
    return {"post_run_guest_meminfo": post, "vm_died_after_s": died_at, "vm_alive_at_end": vm.alive(), "n_samples": len(samples), "sink_trial_end": te, "send_end": se,
            "send_prog_last": sp[-3:], "oom_lines": oom[:12], "peak_queued_MiB": mib(max([s["queued"] for s in samples], default=0)),
            "min_avail_MiB": kb_mib(min([s["mem_avail_kb"] for s in samples], default=0)), "mem_total_MiB": kb_mib(samples[0]["mem_total_kb"]) if samples else None,
            "last_sample": samples[-1] if samples else None, "bad_json_lines": bad}


def stage_c3(vm):
    ab = KV.get("abort", "0"); sm = KV.get("sample_ms", "50"); app = KV.get("app", str(1024 * MIB))
    gx(f"/v3probe sink --kind tcpsrc --trials 1 --bytes 0 --app-bytes {app} --stall-s 10 --idle-s 15 --max-s 120 --sample-ms {sm} --abort-queued-mb {ab} --label c3 > /dev/ttyS0 2>&1 < /dev/null &", 20)
    wait_for(lambda: any(e["ev"] == "trial_end" and e["label"] == "c3" for e in jl(vm.text())[0]) or "Out of memory" in vm.text(), 180, vm)
    time.sleep(2)
    rc, post = gx("grep -E 'MemAvailable|SUnreclaim' /proc/meminfo", 20); log("post-run guest state: " + post)
    g, bad = jl(vm.text())
    samples = [e for e in g if e["ev"] == "sample"]; te = [e for e in g if e["ev"] == "trial_end"]
    log("\n".join(series_table(samples, max(1, len(samples) // 30))))
    return {"sink_trial_end": te, "n_samples": len(samples), "peak_queued_MiB": mib(max([s["queued"] for s in samples], default=0)),
            "peak_tcp_mem_MiB": round(max([s["tcp_mem_pages"] for s in samples], default=0) * 4 / 1024, 1),
            "last_sample": samples[-1] if samples else None, "bad_json_lines": bad}


def stage_c5(vm):
    """valve: close the verdict valve (remove the vsock source from the sockmap) at 8 MiB queued; does vsock credit stop the sender,
    does the queue stop growing, and does re-inserting the source resume forwarding?"""
    vmb = KV.get("valve_mb", "8"); ro = KV.get("reopen_ms", "3000")
    gx(f"/v3probe sink --kind vsock --port 7005 --trials 1 --bytes 0 --valve-mb {vmb} --valve-reopen-ms {ro} --valve-kick {KV.get('kick', '0')} --abort-queued-mb 64 --idle-s 10 --max-s 60 --sample-ms 100 --label c5 > /dev/ttyS0 2>&1 < /dev/null &", 20)
    time.sleep(1.5)
    p = spawn("c5-send", [str(PROBE), "send", "--cid", str(CID), "--port", "7005", "--cap-bytes", str(256 * MIB), "--stall-s", "20", "--label", "c5"])
    try: p.wait(timeout=120)
    except subprocess.TimeoutExpired: reap(p)
    wait_for(lambda: any(e["ev"] == "trial_end" and e["label"] == "c5" for e in jl(vm.text())[0]), 40, vm)
    g, bad = jl(vm.text()); h, _ = jl((EV / "proc-c5-send.log").read_text())
    samples = [e for e in g if e["ev"] == "sample"]
    log("\n".join(series_table(samples, max(1, len(samples) // 12))))
    return {"events": [e for e in g if e["ev"] in ("valve_closed", "valve_reopened", "valve_kick", "trial_end")], "send": [e for e in h if e["ev"] in ("send_end", "send_prog")], "bad_json_lines": bad}


def stage_c4(vm):
    """host-side sink: guest sends, host redirects into a non-reading host TCP socket. Hard-capped at 256 MiB queued."""
    hp = spawn("c4-sink", [str(PROBE), "sink", "--kind", "vsock", "--port", "7004", "--obj", str(OBJ), "--abort-queued-mb", "256", "--idle-s", "10",
                           "--max-s", "120", "--sample-ms", "250", "--label", "c4host"])
    time.sleep(2)
    gx("/v3probe send --cid 2 --port 7004 --cap-bytes 402653184 --stall-s 20 --label c4guest > /dev/ttyS0 2>&1 < /dev/null &", 20)
    wait_for(lambda: hp.poll() is not None, 200, vm)
    reap(hp)
    h, bad = jl((EV / "proc-c4-sink.log").read_text()); g, _ = jl(vm.text())
    samples = [e for e in h if e["ev"] == "sample"]; te = [e for e in h if e["ev"] == "trial_end"]
    log("\n".join(series_table(samples, max(1, len(samples) // 30))))
    return {"host_trial_end": te, "guest_send_end": [e for e in g if e["ev"] == "send_end"], "n_samples": len(samples),
            "peak_queued_MiB": mib(max([s["queued"] for s in samples], default=0)), "bad_json_lines": bad,
            "host_mem_total_MiB": kb_mib(samples[0]["mem_total_kb"]) if samples else None,
            "host_min_avail_MiB": kb_mib(min([s["mem_avail_kb"] for s in samples], default=0)), "last_sample": samples[-1] if samples else None}


def main():
    log(f"=== v3 vm stage {STAGE} {KV}")
    preflight()
    sh("modprobe vhost_vsock; lsmod | grep -E 'vsock|vhost' | head", check=False)
    build_image()
    vm = VM()
    out = {"host_uname_r": os.uname().release}
    try:
        if not vm.wait_ready():
            log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        out["boot"] = stage_boot(vm)
        fn = {"boot": lambda v: {}, "c0": stage_c0, "c1": stage_c1, "c1long": stage_c1long, "c3": stage_c3, "c4": stage_c4, "c5": stage_c5}[STAGE]
        out["result"] = fn(vm)
    except Exception as e:
        import traceback; traceback.print_exc(); out["exception"] = repr(e)
    finally:
        for p in list(PROCS): reap(p)
        vm.stop()
        sh("pgrep -af 'cloud-hypervisor|v3probe' | grep -v pgrep; true", check=False)
    (EV / "result.json").write_text(json.dumps(out, indent=1))
    print("RESULT_JSON " + json.dumps(out)[:60000])


main()
