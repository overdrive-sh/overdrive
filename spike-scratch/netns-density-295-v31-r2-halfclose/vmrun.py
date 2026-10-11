#!/usr/bin/env python3
"""v31 native-metal VM driver (root, under the metal lease; launched by launch.py).

A real Cloud Hypervisor VM (fork 41a619d19, --vsock backend=vhost-kernel, stock 7.0.0-29 host AND
guest kernel). Guest PID 1 runs `drainprobe agent`; this driver starts the host-side processes and
asks the agent to run the guest-side processes. All work in /tmp/v31-work; nothing shared is touched.

Usage: vmrun.py <env> <label> <plan.json>   (env: loop | vm)
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, struct, subprocess, time

W = pathlib.Path("/tmp/v31-work"); OUT = W / "out"
ENVNAME = sys.argv[1]; LABEL = sys.argv[2]; PLAN = json.loads(pathlib.Path(sys.argv[3]).read_text()); STAGE = f"{ENVNAME}-{LABEL}"; ARGS = []
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{STAGE}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3131
CH = OUT / "cloud-hypervisor"; PROBE = pathlib.Path(os.environ.get("V31_BIN", OUT / "drainprobe")); OBJ = pathlib.Path(os.environ.get("V31_OBJ", OUT / "drain-bpf.o"))
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
    shutil.copy2(PROBE, root / "drainprobe"); shutil.copy2(OBJ, root / "vm-drain-bpf.o")
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
/drainprobe agent > /tmp/agent.out 2>&1 &
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
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "drainprobe": sha(PROBE), "bpf_obj": sha(OBJ), "cloud_hypervisor": sha(CH)})
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



import random, threading

HOST_OWNER_ARGS_BASE = []


def offs_args():
    f = W / "offs.args"
    if not f.exists() or not f.read_text().strip():
        o = json.loads(subprocess.check_output(["python3", str(W / "offsets.py")], text=True))
        f.write_text(" ".join(f"--off-{k} {o[v]}" for k, v in {"wseq": "tcp_write_seq", "trans": "vsk_trans", "txcnt": "vvs_tx_cnt", "fpriv": "file_private", "skt": "socket_sk", "cookie": "sk_cookie"}.items()))
    return f.read_text().strip()


def is_reset(e): return e in ("ECONNRESET", "EPIPE") or (isinstance(e, str) and (e.startswith("connect:ECONNRESET") or e.startswith("connect:EPIPE") or e.startswith("connect:Connection refused")))
def is_hang(e): return isinstance(e, str) and ("TIMEOUT" in e)


def judge_strict(row, srv):
    t, n, m, sent, serr, recv, rok, eof, rerr = row[:9]; rst = row[12]
    if srv is None: return "no_server_result"
    _, srecv, sok, seof, srerr, ssent, sserr, rstdone = srv
    if rst is not None: return "expected_reset"
    if srerr: return f"srv_recv_err:{srerr}"
    if srecv != n: return "srv_excess_bytes" if srecv > n else ("srv_truncated" if seof else "srv_short_no_eof")
    if not sok: return "srv_payload_corrupt"
    if not seof: return "srv_no_eof"
    if sserr: return f"srv_send_err:{sserr}"
    if serr: return f"cli_send_err:{serr}"
    if rerr: return f"cli_recv_err:{rerr}"
    if recv != m: return "cli_excess_bytes" if recv > m else ("cli_truncated" if eof else "cli_short_no_eof")
    if not rok: return "cli_payload_corrupt"
    if not eof: return "cli_no_eof"
    return "pass"


def judge_fault(row, srv):
    """pass | reset | violation:<label>. A trial may end complete or in a reset; never a clean short end-of-stream, a hang,
    excess or corrupt bytes."""
    t, n, m, sent, serr, recv, rok, eof, rerr = row[:9]
    errs = [e for e in (serr, rerr) if e]
    if srv is not None:
        errs += [e for e in (srv[4], srv[6]) if e]
    if row[12] is not None:
        # the destination reset the connection on purpose after reading part of the request: the client must see a reset,
        # never a clean end-of-stream and never a hang
        cerrs = [e for e in (serr, rerr) if e]
        if any(is_hang(e) for e in cerrs): return "violation:hang"
        if any(is_reset(e) for e in cerrs): return "reset"
        if eof and not rerr: return "violation:cli_clean_eof_after_dest_rst"
        return "violation:cli_no_reset_no_eof"
    if any(is_hang(e) for e in errs): return "violation:hang"
    other = [e for e in errs if not is_reset(e)]
    if other: return f"violation:other_error:{other[0]}"
    if srv is None:
        return "reset" if errs else "violation:no_server_result_no_reset"
    _, srecv, sok, seof, srerr, ssent, sserr, rstdone = srv
    if srecv > n: return "violation:srv_excess_bytes"
    if srecv > 0 and not sok: return "violation:srv_payload_corrupt"
    # a read that follows an error already reported by a send on the same socket returns 0: that is not a clean FIN
    s_reset = bool(srerr or sserr)
    if not s_reset and seof and srecv != n: return "violation:srv_clean_short_eof"
    if not s_reset and not seof and srecv != n: return "violation:srv_short_no_eof_no_error"
    if recv > m: return "violation:cli_excess_bytes"
    if recv > 0 and not rok: return "violation:cli_payload_corrupt"
    c_reset = bool(serr or rerr)
    if not c_reset and eof and recv != m: return "violation:cli_clean_short_eof"
    if not c_reset and not eof and recv != m: return "violation:cli_short_no_eof_no_error"
    complete = not serr and not rerr and eof and recv == m and seof and srecv == n and not srerr and not sserr
    return "pass" if complete else "reset"


def judge_crash(c, srv):
    """c: [trial, n, m, kind, lo, hi, done, kill_delay_us]. FIN-crash (nothing unread): the destination must receive every byte the
    application's send() returned (lo) up to the bytes it may have issued (hi), then a clean end-of-stream. RST-crash (unread banner):
    a reset, or the complete request when the application had finished sending before it died; never a clean short end."""
    t, n, m, kind, lo, hi, done, dly = c
    if srv is None:
        return "violation:no_server_result" if kind == "fin" and lo > 0 else "reset_or_unconnected"
    _, srecv, sok, seof, srerr, ssent, sserr, rstdone = srv
    if srecv > hi: return "violation:srv_excess_bytes"
    if not sok: return "violation:srv_payload_corrupt"
    if srerr and is_hang(srerr): return "violation:hang"
    if kind == "fin":
        if srerr: return f"violation:unexpected_reset_on_fin_crash:{srerr}"
        if not seof: return "violation:srv_no_eof"
        if srecv < lo: return "violation:srv_lost_acknowledged_bytes"
        return "pass_faithful_fin"
    # rst
    if srerr and is_reset(srerr): return "reset"
    if srerr: return f"violation:other_error:{srerr}"
    if seof and srecv == n and done: return "pass_complete_before_crash"
    if seof and srecv != n: return "violation:srv_clean_short_eof_after_rst_crash"
    return "violation:srv_no_eof"


def stage_boot(vm):
    rc, t = gx("uname -r; ls /sys/kernel/btf/vmlinux /sys/kernel/tracing/events/vsock/virtio_transport_recv_pkt 2>&1 | head -3; ip -br link; cat /proc/sys/net/ipv4/ip_local_port_range")
    log(t); return {"guest_uname": t.splitlines()[0] if t else None}


class Gx:
    """guest or host-local command execution, one interface"""
    def __init__(self, vm): self.vm = vm
    def run(self, cmd, timeout=600):
        if self.vm: return gx(cmd, timeout=timeout)
        p = subprocess.run(["bash", "-c", cmd], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, errors="replace")
        return p.returncode, p.stdout


def run_rep(env, vmobj, sc, rep, tag):
    """one repetition of a scenario. Returns a result dict."""
    host = env == "loop"   # guest-side processes run on the host (loopback vsock, CID 1) or in the VM (CID 2)
    G = Gx(vmobj is not None)
    peer_cid = 1 if host else 2
    gobj = str(OBJ) if host else "/vm-drain-bpf.o"
    gprobe = str(PROBE) if host else "/drainprobe"
    gdir = str(W) if host else "/tmp"
    seed = sc.get("seed", 1000) + rep
    rng = random.Random(seed * 7919 + 13)
    OFFS = offs_args()
    flows = sc.get("flows", 256)
    owners = sc.get("owners", "")
    clients = sc.get("clients", "")
    ev_args = f"--events {EV}/{tag}-ev --events-sample {sc.get('events_sample', 1)}" if (sc.get("events") and env == "loop") else ""
    stop_h, stop_d = W / f"stop-{tag}-h", W / f"stop-{tag}-d"
    for f in (stop_h, stop_d): f.unlink(missing_ok=True)
    stop_g = f"{gdir}/stop-{tag}-g"
    out_h, out_d = EV / f"{tag}-owner-host.json", EV / f"{tag}-dest.json"
    out_g, out_c = f"{gdir}/{tag}-owner-guest.json", f"{gdir}/{tag}-clients.json"
    res = {"tag": tag, "seed": seed, "action": sc["action"]}
    G.run(f"rm -f {stop_g} {out_g} {out_c} {gdir}/{tag}-og.out {gdir}/{tag}-og.pid {gdir}/{tag}-cl.out")
    hp = spawn(f"{tag}-owner-host", [str(PROBE), "owner-host", "--obj", str(OBJ), "--stop-file", str(stop_h), "--out", str(out_h), "--flows", str(flows), *OFFS.split(), *owners.split(), *ev_args.split()])
    if not wait_text(EV / f"proc-{tag}-owner-host.log", "OWNER_HOST_LISTENING", 30): res["error"] = "owner-host not listening"; reap(hp); return res
    dp = spawn(f"{tag}-dest", [str(PROBE), "dest", "--stop-file", str(stop_d), "--out", str(out_d)])
    if not wait_text(EV / f"proc-{tag}-dest.log", "DEST_READY", 30): res["error"] = "dest not ready"; reap(hp); reap(dp); return res
    gcmd = f"{gprobe} owner-guest --obj {gobj} --peer-cid {peer_cid} --intake-port 17001 --stop-file {stop_g} --out {out_g} --flows {flows} {OFFS} {owners} {ev_args}"
    if host:
        gp = spawn(f"{tag}-owner-guest", gcmd.split()); gpid = gp.pid
        ok = wait_text(EV / f"proc-{tag}-owner-guest.log", "OWNER_GUEST_READY", 60)
    else:
        G.run(f"({gcmd} > {gdir}/{tag}-og.out 2>&1 & echo $! > {gdir}/{tag}-og.pid); sleep 0.3; echo started", timeout=30)
        ok = False
        for _ in range(300):
            rc, t = G.run(f"cat {gdir}/{tag}-og.out 2>/dev/null", timeout=30)
            if "OWNER_GUEST_READY" in t: ok = True; break
            time.sleep(0.2)
        rc, t = G.run(f"cat {gdir}/{tag}-og.pid"); gpid = int(t.strip().splitlines()[0]) if t.strip() else None
    if not ok: res["error"] = "owner-guest not ready"; reap(hp); reap(dp); return res
    # clients
    ccmd = f"{gprobe} clients --intake-port 17001 --seed {seed} --trials {sc['trials']} --conc {sc.get('conc', 64)} --out {out_c} {clients}"
    t_cl = {}
    def run_clients():
        t_cl["start"] = time.time()
        rc, t = G.run(ccmd, timeout=sc.get("client_timeout", 900)); t_cl["rc"] = rc; t_cl["out"] = t[-300:]; t_cl["end"] = time.time()
    th = threading.Thread(target=run_clients); th.start()
    act = sc["action"]
    lo, hi = sc.get("kill_ms", [50, 500])
    delay = rng.uniform(lo, hi) / 1000.0
    time.sleep(delay)
    res["action_delay_ms"] = round(delay * 1000)
    if act == "kill-guest-owner":
        if host: os.killpg(gpid, signal.SIGKILL)
        else: G.run(f"kill -9 {gpid}")
    elif act == "kill-host-owner":
        os.killpg(hp.pid, signal.SIGKILL)
    elif act == "ctl-loss":
        os.kill(hp.pid, signal.SIGUSR1)
    elif act == "none": pass
    else: raise RuntimeError(act)
    res["action_done_at_ms_after_clients_start"] = round((time.time() - t_cl["start"]) * 1000)
    th.join(timeout=sc.get("client_timeout", 900) + 60)
    res["clients_wall_s"] = round(t_cl.get("end", time.time()) - t_cl["start"], 1)
    # stop survivors
    stop_h.touch(); stop_d.touch(); G.run(f"touch {stop_g}")
    settle = sc.get("settle_s", 30)
    end = time.time() + settle + 30
    while time.time() < end and not (out_d.exists() and (hp.poll() is not None or out_h.exists())): time.sleep(0.3)
    guest_out = None
    if act != "kill-guest-owner":
        for _ in range(int((settle + 30) / 0.5)):
            rc, t = G.run(f"cat {out_g} 2>/dev/null", timeout=30)
            if t.strip().startswith("{"): guest_out = json.loads(t); break
            time.sleep(0.5)
    rc, t = G.run(f"cat {out_c} 2>/dev/null", timeout=120)
    try: cl = json.loads(t)
    except Exception: cl = {"cli_results": [], "crash_results": [], "error": t[-300:]}
    dest = json.loads(out_d.read_text()) if out_d.exists() else {"srv_results": []}
    host_out = json.loads(out_h.read_text()) if out_h.exists() else None
    for p in (hp, dp):
        if p.poll() is None: reap(p, signal.SIGTERM)
        reap(p)
    if host:
        try: os.killpg(gpid, signal.SIGKILL)
        except Exception: pass
    else:
        G.run(f"kill -9 {gpid} 2>/dev/null; true")
    # cgroups left by killed processes
    for d in pathlib.Path("/sys/fs/cgroup").glob("v31-drain-*"):
        try: d.rmdir()
        except OSError: pass
    G.run("rmdir /sys/fs/cgroup/v31-drain-* 2>/dev/null; true")
    srv = {r[0]: r for r in dest.get("srv_results", [])}
    fn = judge_strict if sc.get("strict") else judge_fault
    cats = {}
    ex = []
    ends = {}
    def endcount(k): ends[k] = ends.get(k, 0) + 1
    for row in cl.get("cli_results", []):
        # per-end outcome (independent of the trial verdict): complete | reset | clean_short_eof | hang | none
        t_, n_, m_, sent_, serr_, recv_, rok_, eof_, rerr_ = row[:9]
        if row[12] is None:
            if serr_ or rerr_: endcount("cli:hang" if (is_hang(serr_) or is_hang(rerr_)) else "cli:reset")
            elif eof_ and recv_ == m_: endcount("cli:complete")
            elif eof_: endcount("cli:CLEAN_SHORT_EOF")
            else: endcount("cli:other")
            sr = srv.get(row[0])
            if sr is None: endcount("srv:no_result")
            elif sr[4] or sr[6]: endcount("srv:hang" if (is_hang(sr[4]) or is_hang(sr[6])) else "srv:reset")
            elif sr[3] and sr[1] == n_: endcount("srv:complete")
            elif sr[3]: endcount("srv:CLEAN_SHORT_EOF")
            else: endcount("srv:other")
        v = fn(row, srv.get(row[0]))
        cats[v] = cats.get(v, 0) + 1
        if v.startswith("violation") or (sc.get("strict") and v != "pass"):
            if len(ex) < 6: ex.append({"why": v, "row": row, "srv": srv.get(row[0])})
    for c in cl.get("crash_results", []):
        v = judge_crash(c, srv.get(c[0]))
        cats["crash:" + v] = cats.get("crash:" + v, 0) + 1
        if v.startswith("violation") and len(ex) < 6: ex.append({"why": v, "crash": c, "srv": srv.get(c[0])})
    res["trials"] = len(cl.get("cli_results", [])) + len(cl.get("crash_results", []))
    res["cats"] = cats
    res["ends"] = ends
    res["examples"] = ex
    res["clients_rc"] = t_cl.get("rc")
    def own(o, key):
        if o is None: return None
        st = o.get(key) or {}
        return {"live_at_end": o.get(f"{key}_live_flows_at_end"), "clean": st.get("flows_clean", 0), "aborted": st.get("flows_aborted", 0),
                "aborts": {k[6:]: v for k, v in st.items() if k.startswith("abort_")}, "viol": {k: v for k, v in st.items() if k.startswith("VIOLATION")},
                "shut": {k: st.get(k, 0) for k in ("shut_0x2", "shut_0x3", "shut_rst", "shut_other", "rdhup_v_without_record")},
                "ctl_lost": st.get("ctl_lost", 0), "paired_at_loss": st.get("paired_at_ctl_loss", 0), "stuck": o.get(f"{key}_stuck", [])[:3],
                "bpf": {k: (o.get("bpf_counters") or {}).get(k) for k in ("fexit_recursion_misses", "fexit_b_recursion_misses", "injected_misses", "poke_runs", "fire_poke", "fire_fexit", "level_unreadable")}}
    res["guest_owner"] = own(guest_out, "guest") if guest_out else None
    res["host_owner"] = own(host_out, "host") if host_out else None
    return res


def brief(r):
    g, h = r.get("guest_owner"), r.get("host_owner")
    def o(x): return None if x is None else f"live={x['live_at_end']} clean={x['clean']} abort={x['aborted']} {x['aborts']} viol={x['viol']}"
    return f"{r['tag']}: act={r['action']}@{r.get('action_done_at_ms_after_clients_start')}ms trials={r.get('trials')} cats={r.get('cats')} ends={r.get('ends')} | guest[{o(g)}] host[{o(h)}] clients_wall={r.get('clients_wall_s')}s{' ERROR '+r['error'] if r.get('error') else ''}"


def main():
    log(f"=== v31 {ENVNAME} {LABEL}: {len(PLAN)} scenarios")
    vm = None
    if ENVNAME == "vm":
        preflight()
        sh("modprobe vhost_vsock; modprobe vsock_loopback; lsmod | grep -E 'vsock|vhost' | head", check=False)
        build_image()
        vm = VM()
        if not vm.wait_ready(): log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        stage_boot(vm)
    else:
        sh("modprobe vhost_vsock; modprobe vsock_loopback", check=False)
        (EV / "substrate-probes.json").write_text(json.dumps({"uname_r": os.uname().release, "machine": os.uname().machine}))
    allres = []
    try:
        for sc in PLAN:
            reps = sc.get("repeat", 1)
            agg = {}
            aggends = {}
            viol_total = 0
            for rep in range(reps):
                tag = f"{sc['name']}-{rep}"
                r = run_rep(ENVNAME, vm, sc, rep, tag)
                allres.append(r)
                print(brief(r), flush=True)
                (EV / f"{tag}-result.json").write_text(json.dumps(r, indent=1))
                for k, v in (r.get("cats") or {}).items(): agg[k] = agg.get(k, 0) + v
                for k, v in (r.get("ends") or {}).items(): aggends[k] = aggends.get(k, 0) + v
            nviol = sum(v for k, v in agg.items() if "violation" in k)
            print(f"### {sc['name']} ({ENVNAME}) reps={reps} aggregate={agg} ends={aggends} violations={nviol}", flush=True)
    except Exception as e:
        import traceback; traceback.print_exc(); log(f"EXCEPTION {e!r}")
    finally:
        for p in list(PROCS): reap(p)
        if vm: vm.stop()
        sh("pgrep -af 'cloud-hypervisor|drainprobe' | grep -v pgrep | head; ls -d /sys/fs/cgroup/v31-* 2>&1 | head -3; true", check=False)
        for d in pathlib.Path("/sys/fs/cgroup").glob("v31-*"):
            try: d.rmdir()
            except OSError as e: log(f"cgroup {d}: {e}")
    (EV / "result.json").write_text(json.dumps(allres, indent=1))


main()
