#!/usr/bin/env python3
"""V-1 native-metal VM driver (derived from the v27 drain driver) (root, under the metal lease; launched by launch.py).

A real Cloud Hypervisor VM (fork 41a619d19, --vsock backend=vhost-kernel, stock 7.0.0-29 host AND
guest kernel). Guest PID 1 runs `drainprobe agent`; this driver starts the host-side processes and
asks the agent to run the guest-side processes. All work in /tmp/v1-work; nothing shared is touched.

Usage: vmrun.py <stage> [stage args]
  boot | t3 | t5 | drain <label> <drainprobe args...>
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, struct, subprocess, time

W = pathlib.Path("/tmp/v1-work"); OUT = W / "out"
STAGE = sys.argv[1]; ARGS = sys.argv[2:]
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{STAGE}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3360
CH, PROBE, OBJ = OUT / "cloud-hypervisor", OUT / "drainprobe", OUT / "drain-bpf.o"
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


def judge(row, srv):
    """row: [trial, n, m, sent, send_err, recv, recv_ok, eof, recv_err, ms, srv_pause, fin_data, rst_after]; srv: [id, recv, recv_ok, eof, recv_err, sent, send_err, rst_done]"""
    t, n, m, sent, serr, recv, rok, eof, rerr = row[:9]; rst = row[12]
    if srv is None: return "no_server_result"
    _, srecv, sok, seof, srerr, ssent, sserr, rstdone = srv[:8]
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


def stage_boot(vm):
    rc, t = gx("uname -r; ls /sys/kernel/btf/vmlinux /sys/kernel/tracing/events/vsock/virtio_transport_recv_pkt 2>&1 | head -3; ip -br link; cat /proc/sys/net/ipv4/ip_local_port_range; for f in tcp_rmem tcp_wmem tcp_adv_win_scale tcp_moderate_rcvbuf; do echo $f=$(cat /proc/sys/net/ipv4/$f); done; for f in rmem_default wmem_default rmem_max wmem_max; do echo $f=$(cat /proc/sys/net/core/$f); done; echo lo_mtu=$(cat /sys/class/net/lo/mtu)")
    sh("for f in tcp_rmem tcp_wmem tcp_adv_win_scale tcp_moderate_rcvbuf; do echo host_$f=$(cat /proc/sys/net/ipv4/$f); done; for f in rmem_default wmem_default rmem_max wmem_max; do echo host_$f=$(cat /proc/sys/net/core/$f); done; echo host_lo_mtu=$(cat /sys/class/net/lo/mtu)", check=False)
    log(t); return {"guest_uname": t.splitlines()[0] if t else None}


def stage_t3(vm):
    res = {}
    # guest -> host: host receives, guest sends
    n = int(ARGS[0]) if ARGS else 3000
    for name, recv_side in (("guest_to_host", "host"), ("host_to_guest", "guest")):
        log(f"--- t3 {name} ({n} trials)")
        port = 7950 if name == "guest_to_host" else 7970
        if recv_side == "host":
            p = spawn(f"t3-recv-{name}", [str(PROBE), "t3", "--mode", "recv", "--port", str(port), "--trials", str(n), "--wait-s", "300"])
            if not wait_text(EV / f"proc-t3-recv-{name}.log", "T3_RECV_READY"): res[name] = "host recv not ready"; continue
            rc, t = gx(f"/drainprobe t3 --mode send --peer-cid 2 --port {port} --trials {n} --conc 32 --id0 {10_000_000 if name=='guest_to_host' else 20_000_000}", timeout=600)
            log(t[-500:]); p.wait(timeout=400)
            out = (EV / f"proc-t3-recv-{name}.log").read_text()
            res[name] = {"guest_sender": t.strip().splitlines()[-1] if t.strip() else None,
                         "host_receiver": json.loads([l for l in out.splitlines() if l.startswith("{")][-1])}
        else:
            # guest receives (listener in guest), host sends to the guest CID
            gx(f"rm -f /tmp/t3recv.out; (/drainprobe t3 --mode recv --port {port} --trials {n} --wait-s 300 > /tmp/t3recv.out 2>&1 &) ; sleep 1", timeout=30)
            time.sleep(1)
            out = sh([str(PROBE), "t3", "--mode", "send", "--peer-cid", str(CID), "--port", str(port), "--trials", str(n), "--conc", "32", "--id0", "30000000"], timeout=600, quiet=True)
            end = time.time() + 120; txt = ""
            while time.time() < end:
                rc, txt = gx("cat /tmp/t3recv.out");
                if '"cmd":"t3"' in txt: break
                time.sleep(1)
            res[name] = {"host_sender": out.strip().splitlines()[-1], "guest_receiver": json.loads([l for l in txt.splitlines() if l.startswith("{")][-1]) if '"cmd":"t3"' in txt else txt[-400:]}
    return res


def stage_t5(vm):
    res = {}
    n = int(ARGS[0]) if ARGS else 110
    # (1) host observes, guest peers are killed/closed/half-closed
    log("--- t5: host observer, guest peers")
    p = spawn("t5-obs-host", [str(PROBE), "t5", "--mode", "obs", "--obj", str(OBJ), "--port", "7960", "--trials", str(n), "--wait-s", "400"])
    if not wait_text(EV / "proc-t5-obs-host.log", "T5_OBS_READY"): return {"error": "host obs not ready"}
    rc, t = gx(f"/drainprobe t5 --mode runner --cid 2 --port 7960 --trials {n}", timeout=600); log(t[-300:])
    p.wait(timeout=120)
    out = (EV / "proc-t5-obs-host.log").read_text()
    res["host_observes_guest_peers"] = json.loads([l for l in out.splitlines() if l.startswith("{")][-1])
    # (2) guest observes, host peers
    log("--- t5: guest observer, host peers")
    gx(f"rm -f /tmp/t5obs.out; (/drainprobe t5 --mode obs --obj /vm-drain-bpf.o --port 7961 --trials {n} --wait-s 400 > /tmp/t5obs.out 2>&1 &) ; sleep 2", timeout=30)
    time.sleep(1)
    r = sh([str(PROBE), "t5", "--mode", "runner", "--cid", str(CID), "--port", "7961", "--trials", str(n)], timeout=900, quiet=True)
    end = time.time() + 120; txt = ""
    while time.time() < end:
        rc, txt = gx("cat /tmp/t5obs.out")
        if '"cmd":"t5-obs"' in txt: break
        time.sleep(1)
    res["guest_observes_host_peers"] = json.loads([l for l in txt.splitlines() if l.startswith("{")][-1]) if '"cmd":"t5-obs"' in txt else txt[-600:]
    res["runner_host"] = r.strip().splitlines()[-1]
    return res


def stage_sweep(vm):
    """ARGS[0]: path of a JSON list of {"label":..., "args":[...]}; one VM boot, one cell after another."""
    cells = json.loads(pathlib.Path(ARGS[0]).read_text())
    res = {}
    for c in cells:
        log(f"##### cell {c['label']}: {' '.join(c['args'])}")
        if c.get("guest_pre"):
            rc, t = gx(c["guest_pre"]); log(f"guest_pre rc={rc}: {t.strip()[-300:]}")
        res[c["label"]] = stage_drain(vm, c["label"], c["args"])
        for g in pathlib.Path("/sys/fs/cgroup").glob("v1-drain-*"):
            try: g.rmdir()
            except OSError: pass
    return {k: {"trials": v.get("trials"), "guest_live_at_end": v.get("guest_live_at_end"), "host_live_at_end": v.get("host_live_at_end")} for k, v in res.items()}


def stage_drain(vm, label=None, pargs=None):
    if label is None: label, pargs = ARGS[0], ARGS[1:]
    stopf = W / f"stop-{label}"; stopf.unlink(missing_ok=True)
    hout = EV / f"{label}-host.json"
    hp = spawn(f"{label}-host", [str(PROBE), "vm-host", "--obj", str(OBJ), "--stop-file", str(stopf), "--out", str(hout), *pargs])
    if not wait_text(EV / f"proc-{label}-host.log", "HOST_READY"): return {"error": "vm-host not ready"}
    t0 = time.time()
    rc, t = gx(f"rm -f /tmp/guest-out.json; /drainprobe vm-guest --obj /vm-drain-bpf.o --out /tmp/guest-out.json {' '.join(pargs)}", timeout=3000)
    log(f"guest rc={rc} {t[-300:]}")
    stopf.touch(); hp.wait(timeout=90)
    rc2, gj = gx("cat /tmp/guest-out.json", timeout=120)
    try: guest = json.loads(gj)
    except Exception: guest = {"error": gj[-300:]}
    host = json.loads(hout.read_text()) if hout.exists() else {"error": "no host out"}
    srv = {r[0]: r for r in host.get("srv_results", [])}
    fails, passn = {}, 0
    ex = []
    for row in guest.get("cli_results", []):
        v = judge(row, srv.get(row[0]))
        if v == "pass": passn += 1
        else:
            fails[v] = fails.get(v, 0) + 1
            if len(ex) < 8: ex.append({"why": v, "row": row, "srv": srv.get(row[0])})
    (EV / f"{label}-guest.json").write_text(json.dumps(guest))
    summary = {"label": label, "guest_settles": guest.get("guest_settles"), "host_settles": host.get("host_settles"),
               "cli_app": [[r[0], r[1], r[2], r[13] if len(r) > 13 else None, r[14] if len(r) > 14 else None, r[3], r[4]] for r in guest.get("cli_results", [])],
               "srv_app": [[r[0], r[8] if len(r) > 8 else None, r[9] if len(r) > 9 else None] for r in host.get("srv_results", [])], "args": pargs, "guest_uname": guest.get("uname"), "host_uname": host.get("uname"), "wall_s": time.time() - t0,
               "trials": {"total": len(guest.get("cli_results", [])), "pass": passn, "fail": fails, "examples": ex},
               "guest": guest.get("guest"), "host": host.get("host"), "guest_live_at_end": guest.get("guest_live_flows_at_end"), "host_live_at_end": host.get("host_live_flows_at_end"),
               "guest_stuck": guest.get("guest_stuck"), "host_stuck": host.get("host_stuck"), "guest_bpf": guest.get("bpf_counters"), "host_bpf": host.get("bpf_counters")}
    (EV / f"{label}-summary.json").write_text(json.dumps(summary, indent=1))
    compact(summary)
    return summary


def compact(s):
    t = s["trials"]
    print(f"=== VM {s['label']} args: {' '.join(s['args'])}")
    print(f"trials total={t['total']} pass={t['pass']} fail={t['fail']} wall_s={s['wall_s']:.1f} host_uname={s['host_uname']} guest_uname={s['guest_uname']}")
    for r in ("guest", "host"):
        c = s.get(r) or {}
        own, rf, rs = c.get("complete_owner_cas", 0), c.get("complete_record_fexit", 0), c.get("complete_record_state_cb", 0)
        viol = {k: v for k, v in c.items() if k.startswith("VIOLATION")}
        ab = {k: v for k, v in c.items() if k.startswith("abort_")}
        tcp = c.get("tcp_eof_events", 0); vs = c.get("vsock_eof_events", 0)
        print(f" {r}: started={c.get('flows_started',0)} clean={c.get('flows_clean',0)} aborted={c.get('flows_aborted',0)} completions={own+rf+rs} (owner_cas={own} rec_fexit={rf} rec_state_cb={rs}) records={c.get('records',0)} owner_lost_cas={c.get('owner_lost_cas',0)}")
        print(f"   gap@RDHUP tcp_src sent<goal {c.get('tcp_eof_sent_lt_goal',0)}/{tcp} vsock_src sent<goal {c.get('vsock_eof_sent_lt_goal',0)}/{vs} | tcp_fwd_ne_goal={c.get('tcp_fwd_ne_goal_at_completion',0)} vsock_fwd_ne_peer_count={c.get('vsock_fwd_ne_peer_count_at_completion',0)} VIOLATIONS={viol} aborts={ab}")
    print(f" live_at_end guest={s['guest_live_at_end']} host={s['host_live_at_end']} guest_bpf_send_err={ (s.get('guest_bpf') or {}).get('send_err_hist')} host_bpf_send_err={ (s.get('host_bpf') or {}).get('send_err_hist')}")
    for e in t["examples"][:2]: print(" EX", json.dumps(e)[:300])


def main():
    log(f"=== v27 vm stage {STAGE} {ARGS}")
    preflight()
    sh("modprobe vhost_vsock; modprobe vsock_loopback; lsmod | grep -E 'vsock|vhost' | head", check=False)
    build_image()
    vm = VM()
    out = {}
    try:
        if not vm.wait_ready(): log("guest not ready:\n" + vm.text()[-1500:]); sys.exit(4)
        log("guest ready: " + [l for l in vm.text().splitlines() if "GUEST_BOOT" in l][0])
        boot = stage_boot(vm); out["boot"] = boot
        fn = {"boot": lambda v: {}, "t3": stage_t3, "t5": stage_t5, "drain": lambda v: stage_drain(v), "sweep": stage_sweep}[STAGE]
        out["result"] = fn(vm)
    except Exception as e:
        import traceback; traceback.print_exc(); out["exception"] = repr(e)
    finally:
        for p in list(PROCS): reap(p)
        vm.stop()
        sh("pgrep -af cloud-hypervisor | grep -v pgrep; ls -d /sys/fs/cgroup/v1-* 2>&1; true", check=False)
        for d in pathlib.Path("/sys/fs/cgroup").glob("v1-*"):
            try: d.rmdir()
            except OSError as e: log(f"cgroup {d}: {e}")
    (EV / "result.json").write_text(json.dumps(out, indent=1))
    print("RESULT_JSON " + json.dumps(out)[:200000])


main()
