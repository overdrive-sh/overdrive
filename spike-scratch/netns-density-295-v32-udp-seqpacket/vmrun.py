#!/usr/bin/env python3
"""v32 native-metal VM driver (root, under the metal lease; launched by launch.py).

A real Cloud Hypervisor VM (fork 41a619d19, --vsock backend=vhost-kernel, stock 7.0.0-29 host AND
guest kernel). Guest PID 1 runs `v32probe agent`; this driver starts the host-side processes and
asks the agent to run the guest-side processes. All work in /tmp/v32-work; nothing shared is touched.

Usage: vmrun.py <stage> [stage args]
  boot | matrix <cells.json>
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, struct, subprocess, time

W = pathlib.Path("/tmp/v32-work"); OUT = W / "out"
STAGE = sys.argv[1]; ARGS = sys.argv[2:]
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{STAGE}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3232
CH, PROBE, OBJ = OUT / "cloud-hypervisor", OUT / "v32probe", OUT / "v32-bpf.o"
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
           "pkg": sh("dpkg -l 'linux-image-*' 2>/dev/null | grep -E '^ii' | awk '{print $2, $3}' | head -5", check=False, quiet=True),
           "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "nproc": os.cpu_count()}
    (EV / "substrate-probes.json").write_text(json.dumps({**pre, "failures": fail}, indent=1))
    log(f"substrate: {json.dumps(pre)}")
    if fail:
        log(f"FAILED CLOSED: {fail}"); sys.exit(3)


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
    for n in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "touch", "env", "true", "false", "ps", "id", "head", "tail", "wc", "date", "pkill", "grep"]:
        (root / "bin" / n).symlink_to("busybox")
    for b in ["/usr/sbin/ip", "/usr/bin/ss"]:
        if os.path.exists(b): copy_with_libs(root, b)
    shutil.copy2(PROBE, root / "v32probe"); shutil.copy2(OBJ, root / "v32-bpf.o")
    (root / "init").write_text("""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /sys/fs/cgroup /sys/fs/bpf /tmp /sys/kernel/debug /sys/kernel/tracing
mount -t cgroup2 none /sys/fs/cgroup
mount -t bpf bpf /sys/fs/bpf
mount -t tmpfs tmpfs /tmp
mount -t debugfs none /sys/kernel/debug 2>/dev/null
mount -t tracefs none /sys/kernel/tracing 2>/dev/null
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
ip link set lo up
ulimit -n 1048576
echo "1024 65535" > /proc/sys/net/ipv4/ip_local_port_range 2>/dev/null
/v32probe agent > /tmp/agent.out 2>&1 &
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
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "v32probe": sha(PROBE), "bpf_obj": sha(OBJ), "cloud_hypervisor": sha(CH)})
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


def spawn(name, argv, env=None):
    f = (EV / f"proc-{name}.log").open("w")
    p = subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, start_new_session=True, env=env); p.name = name; PROCS.append(p); return p


def reap(p, sig=signal.SIGKILL):
    if p.poll() is None:
        try: os.killpg(p.pid, sig)
        except ProcessLookupError: pass
        try: p.wait(timeout=8)
        except subprocess.TimeoutExpired: pass


def wait_text(path, pat, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if pat in pathlib.Path(path).read_text(): return True
        except FileNotFoundError: pass
        time.sleep(0.2)
    return False


def stage_boot(vm):
    rc, t = gx("uname -r; ls /sys/kernel/btf/vmlinux /sys/kernel/tracing/events/vsock/virtio_transport_alloc_pkt 2>&1 | head -3; ip -br link; cat /proc/sys/net/ipv4/udp_mem /proc/sys/net/core/rmem_max")
    log(t); return {"guest_uname": t.splitlines()[0] if t else None, "detail": t}


def run_cell(idx, cell):
    label = cell["label"]; port = 7700 + 2 * idx
    v, size, n = cell.get("variant", "seq"), str(cell.get("size", 1024)), int(cell.get("n", 100000))
    common = ["--variant", v, "--size", size, "--n", str(n), "--port", str(port)]
    hargs = common + ["--obj", str(OBJ), "--out", str(EV / f"{label}-host.json"), "--hostbuf", str(cell.get("hostbuf", 0)), "--nodelay", str(cell.get("nodelay", 0)), "--h1mode", str(cell.get("h1mode", 4)),
                      "--pause-ms", str(cell.get("pause_ms", 0)), "--check", str(cell.get("check", 1))]
    if cell.get("synth"): hargs += ["--size", "any"] if False else []
    gout = f"/tmp/{label}-guest.json"
    gargs = common + ["--obj", "/v32-bpf.o", "--out", gout, "--synth", str(cell.get("synth", 0)), "--bound", str(cell.get("bound", 59000)),
                      "--cell-bytes", str(cell.get("cell_bytes", 8 << 20)), "--p2mode", str(cell.get("p2mode", 4)), "--nodelay", str(cell.get("nodelay", 0)),
                      "--cell", str(cell.get("cell", "tcp")), "--k1", str(cell.get("k1", 0)), "--k1rearm", str(cell.get("k1rearm", 0))]
    if cell.get("synth"):
        hargs[hargs.index("--size") + 1] = "any"; gargs[gargs.index("--size") + 1] = "1024"
    log(f"--- cell {label}: {cell}")
    hp = spawn(f"{label}-host", [str(PROBE), "host", *hargs])
    t0 = time.time()
    if not wait_text(EV / f"proc-{label}-host.log", "HOST_READY", 60):
        reap(hp); return {"label": label, "error": "host not ready", "host_log": (EV / f"proc-{label}-host.log").read_text()[-800:]}
    rc, t = gx(f"rm -f {gout}; /v32probe guest {' '.join(gargs)}", timeout=1500)
    log(f"guest rc={rc} {t[-400:]}")
    try: hp.wait(timeout=600)
    except subprocess.TimeoutExpired: reap(hp)
    rc2, gj = gx(f"cat {gout}", timeout=120)
    try: guest = json.loads(gj[:gj.rindex("}") + 1])
    except Exception: guest = {"error": gj[-400:]}
    hf = EV / f"{label}-host.json"
    host = json.loads(hf.read_text()) if hf.exists() else {"error": "no host json", "host_log": (EV / f"proc-{label}-host.log").read_text()[-800:]}
    res = {"label": label, "cell": cell, "wall_s": time.time() - t0, "host": host, "guest": guest, "guest_stdout_tail": t[-300:]}
    (EV / f"{label}-summary.json").write_text(json.dumps(res, indent=1))
    compact(res)
    return res


def compact(r):
    h, g, c = r["host"], r["guest"], r["cell"]
    print(f"=== {r['label']}: variant={c.get('variant','seq')} size={c.get('size')} n={c.get('n',100000)} hostbuf={c.get('hostbuf',0)} pause_ms={c.get('pause_ms',0)} check={c.get('check',1)} nodelay={c.get('nodelay',0)} h1mode={c.get('h1mode',4)} cell={c.get('cell','tcp')} k1={c.get('k1',0)} p2mode={c.get('p2mode',4)} wall={r['wall_s']:.1f}s")
    if "outcome" not in h: print("  HOST ERROR", json.dumps(h)[:400]); return
    o = h["outcome"]; hc = h.get("bpf_counters", {}); gc = g.get("bpf_counters", {}); s = h["sink"]; q = h["seqrecv"]
    print(f"  OUTCOME once={o['delivered_once']} dup_seqs={o['duplicated_seqs']} dup_extra={o['duplicate_extra_copies']} lost={o['lost']}  (guest app_sent_ok={g.get('app_sent_ok')} stalled={g.get('sender_stalled')} udp_rcvbuf_err={g.get('udp_snmp_delta',{}).get('RcvbufErrors',0)})")
    print(f"  HOST verdict ok={hc.get('rx_ok',0)} short={hc.get('rx_short',0)} long={hc.get('rx_long',0)} (2nd-magic {hc.get('rx_long_second_magic',0)}) lt8={hc.get('rx_lt8',0)} badmagic={hc.get('rx_badmagic',0)} passed_bad={hc.get('rx_mismatch_passed_check_off',0)} | sink frames={s['frames_parsed']} ok={s['frames_verified_ok']} bad={s['bad']} {s['bad_reasons']} desync_bytes={s['desync_bytes_skipped']}")
    if q["records"]: print(f"  HOST seqrecv records={q['records']} ok={q['ok']} short={q['short']} long={q['long']} (2nd-magic {q['long_second_magic']}) lt8={q['lt8']} badmagic={q['badmagic']}")
    print(f"  GUEST strp_bad_frame={gc.get('strp_bad_frame',0)} host_strp_bad_frame={hc.get('strp_bad_frame',0)} lost_of_sent={g.get('app_sent_ok',0)-o['delivered_once']-o['duplicated_seqs']}")
    print(f"  GUEST encoded={gc.get('guest_encoded',0)} over_bound={gc.get('guest_over_bound_drop',0)} strp_frames={gc.get('strp_frames',0)} send_outcomes={g.get('skb_send_sock_outcomes_on_vsock_leg')} tx_pkts(alloc, EOM flag not yet set at this tracepoint) seq={gc.get('tx_alloc_seq_eom',0)+gc.get('tx_alloc_seq_noeom',0)} stream={gc.get('tx_alloc_stream',0)}")
    print(f"  HOST rx_pkts (flags as received) seq_eom={hc.get('rx_pkt_seq_eom',0)} seq_noeom={hc.get('rx_pkt_seq_noeom',0)} stream={hc.get('rx_pkt_stream',0)} peer_buf_alloc={h.get('peer_buf_alloc_seen_by_host_recv_pkt')} host_child_buf={h.get('accepted_child_buffer_size')} guest_buf={g.get('guest_vsock_buffer_size')} | send_s={g.get('send_wall_s',0):.1f} drain_s={g.get('drain_wall_s',0):.1f}")
    if h["events"]["sample"]: print("  EVENTS", json.dumps(h["events"]["sample"][:4]))


def main():
    log(f"=== v32 vm stage {STAGE} {ARGS}")
    preflight()
    sh("modprobe vhost_vsock; modprobe vsock_loopback; lsmod | grep -E 'vsock|vhost' | head", check=False)
    build_image()
    vm = VM()
    out = {}
    try:
        if not vm.wait_ready(): log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        out["boot"] = stage_boot(vm)
        if STAGE == "matrix":
            cells = json.loads(pathlib.Path(ARGS[0]).read_text())
            out["cells"] = []
            for i, cell in enumerate(cells):
                out["cells"].append(run_cell(i + int(os.environ.get("V32_IDX0", "0")), cell))
                (EV / "result.partial.json").write_text(json.dumps(out, indent=1))
    except Exception as e:
        import traceback; traceback.print_exc(); out["exception"] = repr(e)
    finally:
        for p in list(PROCS): reap(p)
        vm.stop()
        sh("pgrep -af cloud-hypervisor | grep -v pgrep; true", check=False)
    (EV / "result.json").write_text(json.dumps(out, indent=1))
    print("RESULT_JSON_WRITTEN", EV / "result.json")


main()
