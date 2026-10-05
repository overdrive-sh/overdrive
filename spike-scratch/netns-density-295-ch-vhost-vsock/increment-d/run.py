#!/usr/bin/env python3
"""CH vhost-kernel vsock spike (netns-density-295, V-1(b)) — native metal run.

Runs under the canonical metal lease via `cargo xtask metal run -- python3 ...`
(root). Builds the patched CH and the probe, builds a guest initramfs, runs
probes P1..P9 on the physical host (no nesting), records raw evidence under
<increment>/evidence/ and restores the physical module set at the end.
"""
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, subprocess, sys, threading, time

INC = pathlib.Path(__file__).resolve().parent
BASE = INC.parent
ROOT = BASE.parent.parent
EV = INC / "evidence"
OUT = BASE / "out" / INC.name
EV.mkdir(exist_ok=False)
OUT.mkdir(parents=True, exist_ok=True)
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
STOCK_CH = "/usr/local/bin/cloud-hypervisor"
ONLY = set(sys.argv[1:])  # optional probe subset, e.g. P1 P2

lock = open("/tmp/netns-density-295-ch-vhost-vsock.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

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
        for line in p.stdout.rstrip().splitlines()[-60:]:
            log("    " + line)
    if check and p.returncode != 0:
        raise RuntimeError(f"command failed rc={p.returncode}: {cmd}")
    return p.stdout


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def modules():
    return sorted(l.split()[0] for l in pathlib.Path("/proc/modules").read_text().splitlines())


RESULTS = {}


def record(probe, key, value):
    RESULTS.setdefault(probe, {})[key] = value
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))


VSOCK_STACK = ["vsock_diag", "vsock_loopback", "vmw_vsock_vmci_transport", "vmw_vmci", "vhost_vsock",
               "vmw_vsock_virtio_transport_common", "vsock", "vhost", "vhost_iotlb"]
PRISTINE = (INC / "pristine-modules-before-increment-a.txt").read_text().split()


def unload_vsock_stack(tag):
    """Unload stock vsock/vhost modules that were not loaded on the pristine host."""
    for _ in range(3):
        for m in VSOCK_STACK:
            if m in modules() and m not in PRISTINE:
                sh(f"rmmod {m}", check=False)
    left = [m for m in modules() if m in VSOCK_STACK and m not in PRISTINE]
    record("cleanup", f"vsock_stack_left_{tag}", left)
    return left


# ------------------------------------------------------------------ preflight
log("=== preflight")
record("cleanup", "vsock_stack_present_at_start", [m for m in modules() if m in VSOCK_STACK])
unload_vsock_stack("start")
MODULES_BEFORE = modules()
(EV / "modules-before.txt").write_text("\n".join(MODULES_BEFORE) + "\n")
sh("uname -a; systemd-detect-virt || true; grep -m1 'model name' /proc/cpuinfo; nproc; free -m")
sh("cat /sys/module/kvm_amd/parameters/nested 2>/dev/null; ls -l /dev/kvm /dev/vhost-vsock")
sh(f"{STOCK_CH} --version; sha256sum {STOCK_CH} {GUEST_KERNEL}")
pre_ch = sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False)
record("preflight", "host_uname", os.uname().release)
record("preflight", "other_ch_processes_at_start", pre_ch.strip())
record("preflight", "vsock_modules_loaded_before", [m for m in MODULES_BEFORE if "vsock" in m or "vhost" in m])

# ------------------------------------------------------------------ build
log("=== build")
env = os.environ.copy()
env["CARGO_HOME"] = str(BASE / "out" / "cargo-home")
env["CARGO_TARGET_DIR"] = str(BASE / "out" / "ch-target")
sh(["cargo", "build", "--release", "--locked", "--manifest-path", str(ROOT / "vendors/cloud-hypervisor/Cargo.toml"),
    "--bin", "cloud-hypervisor", "--bin", "ch-remote"], env=env, timeout=3600)
PATCHED_CH = str(BASE / "out" / "ch-target/release/cloud-hypervisor")
CH_REMOTE = str(BASE / "out" / "ch-target/release/ch-remote")
penv = env.copy()
penv["CARGO_TARGET_DIR"] = str(BASE / "out" / "probe-target")
penv["RUSTFLAGS"] = "-C target-feature=+crt-static"
sh(["cargo", "build", "--release", "--target", "x86_64-unknown-linux-gnu", "--manifest-path",
    str(BASE / "probe/Cargo.toml")], env=penv, timeout=900)
PROBE = str(BASE / "out" / "probe-target/x86_64-unknown-linux-gnu/release/ch-vsock-probe")
sh(f"file {PROBE} {PATCHED_CH}; {PATCHED_CH} --version")

# guest initramfs
root = OUT / "initramfs-root"
if root.exists():
    shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]:
    (root / d).mkdir(parents=True)
busybox = shutil.which("busybox")
sh(f"file {busybox}")
shutil.copy2(busybox, root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "reboot", "sleep", "cut", "basename", "readlink"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(PROBE, root / "probe")
shutil.copy2(BASE / "guest-init.sh", root / "init")
kver = os.uname().release
pins = {}
for line in sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"]).splitlines():
    if line.startswith("insmod "):
        src = pathlib.Path(line.split()[1])
        data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
        (root / src.name.removesuffix(".zst")).write_bytes(data)
        pins[str(src)] = sha(src)
INITRD = OUT / "initramfs.cpio.gz"
sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -9 > {INITRD}")
pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(INITRD), "patched_ch": sha(PATCHED_CH),
             "stock_ch": sha(STOCK_CH), "probe": sha(PROBE), "ch_remote": sha(CH_REMOTE)})
(EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))

# ------------------------------------------------------------------ helpers
VMS = []


class VM:
    def __init__(self, name, binary, vsock, api=False):
        self.name = name
        self.dir = OUT / f"vm-{name}-{int(time.time() * 1000)}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        self.stdout = self.dir / "ch.stdout"
        self.api = pathlib.Path(f"/tmp/chvv-{os.getpid()}-{len(VMS)}.sock") if api else None
        cmd = [binary, "--kernel", GUEST_KERNEL, "--initramfs", str(INITRD),
               "--cmdline", "console=ttyS0 panic=-1 loglevel=4",
               "--cpus", "boot=1", "--memory", "size=512M",
               "--serial", f"file={self.serial}", "--console", "off",
               "--log-file", str(self.dir / "ch.log"), "-v"]
        if vsock:
            cmd += ["--vsock", vsock]
        if self.api:
            cmd += ["--api-socket", f"path={self.api}"]
        self.cmd = cmd
        log(f"launch {name}: {' '.join(cmd)}")
        self.out = self.stdout.open("w")
        self.proc = subprocess.Popen(cmd, stdout=self.out, stderr=subprocess.STDOUT)
        self.pid = self.proc.pid
        VMS.append(self)

    def serial_text(self):
        return self.serial.read_text(errors="replace") if self.serial.exists() else ""

    def wait_serial(self, pattern, count=1, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if len(re.findall(pattern, self.serial_text())) >= count:
                return True
            if self.proc.poll() is not None:
                return False
            time.sleep(0.1)
        return False

    def wait_exit(self, timeout=30):
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            return None

    def snapshot(self, tag):
        pid = self.pid
        snap = {"tag": tag, "pid": pid}
        try:
            tasks = sorted(pathlib.Path(f"/proc/{pid}/task").iterdir(), key=lambda p: int(p.name))
            snap["threads"] = [f"{t.name}:{(t / 'comm').read_text().strip()}" for t in tasks]
            snap["seccomp"] = sorted({re.search(r"Seccomp:\s+(\d+)", (t / 'status').read_text()).group(1) for t in tasks})
            fds = []
            for fd in sorted(pathlib.Path(f"/proc/{pid}/fd").iterdir(), key=lambda p: int(p.name)):
                try:
                    fds.append(f"{fd.name}->{os.readlink(fd)}")
                except OSError:
                    pass
            snap["fds"] = fds
        except FileNotFoundError:
            snap["gone"] = True
        (self.dir / f"snapshot-{tag}.json").write_text(json.dumps(snap, indent=2))
        return snap

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()

    def collect(self):
        for f in [self.serial, self.stdout, self.dir / "ch.log"]:
            if f.exists():
                dst = EV / "vms" / self.dir.name / f.name
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(f, dst)
        for f in self.dir.glob("snapshot-*.json"):
            dst = EV / "vms" / self.dir.name / f.name
            shutil.copy2(f, dst)


def probe(*args, timeout=120, check=False):
    return sh([PROBE, *map(str, args)], check=check, timeout=timeout)


def bg_probe(name, *args):
    f = (OUT / f"bg-{name}.log").open("w")
    p = subprocess.Popen([PROBE, *map(str, args)], stdout=f, stderr=subprocess.STDOUT)
    time.sleep(0.3)
    return p, OUT / f"bg-{name}.log"


def finish_bg(p, path, timeout=60, name=None):
    try:
        p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
    text = path.read_text()
    shutil.copy2(path, EV / path.name)
    log(f"--- {path.name}:\n" + text)
    return text


def vhost_state(tag):
    out = sh("cat /sys/module/vhost_vsock/refcnt 2>/dev/null || echo not-loaded; "
             "ss --vsock -a -n -p 2>&1; pgrep -af cloud-hypervisor | grep -v pgrep || true; "
             "ps -eLo pid,tid,comm | awk '$3 ~ /^vhost/' || true", check=False, quiet=True)
    (EV / f"host-vsock-state-{tag}.txt").write_text(out)
    log(f"host vsock state [{tag}]:\n" + out)
    return out


SYSCALLS = "read,write,readv,writev,pread64,pwrite64,preadv,pwritev,preadv2,pwritev2,recvfrom,sendto,recvmsg,sendmsg,recvmmsg,sendmmsg,splice,sendfile,vmsplice,copy_file_range"
RE_FULL = re.compile(r"^(\d+)\s+(\w+)\((\d+)(<[^>]*>)?.*=\s+(-?\d+)")
RE_UNF = re.compile(r"^(\d+)\s+(\w+)\((\d+)(<[^>]*>)?.*<unfinished \.\.\.>")
RE_RES = re.compile(r"^(\d+)\s+<\.\.\. (\w+) resumed>.*=\s+(-?\d+)")


def strace_start(vm, tag):
    path = OUT / f"strace-{tag}.txt"
    p = subprocess.Popen(["strace", "-f", "-qq", "-yy", "-s", "0", "-e", f"trace={SYSCALLS}", "-p", str(vm.pid),
                          "-o", str(path)], stdout=subprocess.DEVNULL, stderr=(OUT / f"strace-{tag}.err").open("w"))
    time.sleep(1.0)
    return p, path


def strace_stop(p, path, tag):
    p.send_signal(signal.SIGINT)
    p.wait(timeout=30)
    pending, per_fd, calls = {}, {}, 0
    for line in path.read_text(errors="replace").splitlines():
        m = RE_FULL.match(line)
        if m and "unfinished" not in line:
            pid, sc, fd, desc, rc = m.groups()
        else:
            m = RE_UNF.match(line)
            if m:
                pending[m.group(1)] = (m.group(2), m.group(3), m.group(4))
                continue
            m = RE_RES.match(line)
            if not m or m.group(1) not in pending:
                continue
            sc, fd, desc = pending.pop(m.group(1))
            rc = m.group(3)
        calls += 1
        rc = int(rc)
        key = f"{sc} fd={fd}{desc or ''}"
        if rc > 0:
            per_fd[key] = per_fd.get(key, 0) + rc
    per_tid = {}
    for line in path.read_text(errors="replace").splitlines():
        tid = line.split(" ", 1)[0]
        per_tid[tid] = per_tid.get(tid, 0) + 1
    total = sum(per_fd.values())
    summary = {"calls": calls, "bytes_total": total, "lines_per_tid": per_tid, "by_syscall_fd": dict(sorted(per_fd.items(), key=lambda kv: -kv[1])[:40])}
    shutil.copy2(path, EV / path.name)
    err = OUT / f"strace-{tag}.err"
    if err.exists():
        shutil.copy2(err, EV / err.name)
    (EV / f"strace-{tag}-summary.json").write_text(json.dumps(summary, indent=2))
    log(f"strace[{tag}] calls={calls} bytes_total={total} top={list(summary['by_syscall_fd'].items())[:6]}")
    return summary


def held_connection_ss(cid, port, tag):
    """Open a host AF_VSOCK stream to the guest, start a request, sample ss, finish."""
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(20)
    s.connect((cid, port))
    s.sendall(b"LITMUS-REQ label=held len=0\n")
    time.sleep(0.5)
    ss = sh("ss --vsock -a -n -p", check=False, quiet=True)
    (EV / f"ss-held-{tag}.txt").write_text(ss)
    log(f"ss --vsock during held connection [{tag}] (python pid {os.getpid()}):\n{ss}")
    s.shutdown(socket.SHUT_WR)
    data = b""
    while True:
        chunk = s.recv(65536)
        if not chunk:
            break
        data += chunk
    s.close()
    return ss, data.decode(errors="replace").splitlines()[0] if data else ""


def wait_ident(cid, timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        p = subprocess.run([PROBE, "ctl", f"vsock:{cid}:4000", "ident"], text=True, capture_output=True, timeout=30)
        if "IDENT" in p.stdout:
            return p.stdout.strip().splitlines()[0]
        time.sleep(0.5)
    return None


def uptime_of(ident):
    m = re.search(r"uptime_s=([\d.]+)", ident or "")
    return float(m.group(1)) if m else None


def run_probe(name, fn):
    if ONLY and name not in ONLY:
        return
    log(f"========== {name}")
    try:
        fn()
        record(name, "completed", True)
    except Exception as e:  # keep going; record the failure
        log(f"{name} EXCEPTION {e!r}")
        record(name, "exception", repr(e))
    finally:
        for vm in list(VMS):
            vm.kill()
            vm.collect()
            VMS.remove(vm)


# ------------------------------------------------------------------ probes
def unix_baseline(name, binary, vsock_extra=""):
    sock = OUT / f"{name}.vsock"
    srv, srv_log = bg_probe(f"{name}-g2h-unix", "serve", f"unix:{sock}:6000", "stream", 1)
    vm = VM(name, binary, f"cid=3,socket={sock}{vsock_extra}")
    ready = vm.wait_serial(r"AGENT_READY")
    record(name, "agent_ready", ready)
    record(name, "guest_boot_lines", re.findall(r"GUEST_\w+.*|LOCAL_CID=\d+", vm.serial_text()))
    snap = vm.snapshot("running")
    record(name, "ch_threads", snap.get("threads"))
    record(name, "ch_fds", snap.get("fds"))
    record(name, "ch_seccomp_modes", snap.get("seccomp"))
    sh(f"ss -x -a -p | grep -F '{sock}' || true", check=False)
    record(name, "ctl_ident", probe("ctl", f"unix:{sock}:4000", "ident").strip())
    st, st_path = strace_start(vm, name)
    r = probe("client", f"unix:{sock}:5000", "stream", 4 * 1024 * 1024, f"h2g-{name}")
    record(name, "h2g_stream", r.strip())
    record(name, "strace_h2g", strace_stop(st, st_path, name))
    r = probe("ctl", f"unix:{sock}:4000", "unix-client", 6000, 1024 * 1024, timeout=120)
    record(name, "g2h_stream_guest_view", r.strip())
    record(name, "g2h_stream_host_view", finish_bg(srv, srv_log).strip())
    vhost_state(f"{name}-running")
    probe("ctl", f"unix:{sock}:4000", "poweroff")
    record(name, "ch_exit_code_after_poweroff", vm.wait_exit(30))


def p2_p3_p4_p6():
    vhost_state("before-vhost")
    srv_s, log_s = bg_probe("g2h-stream", "serve", "vsock:any:6000", "stream", 1)
    srv_q, log_q = bg_probe("g2h-seq", "serve", "vsock:any:6001", "seqpacket", 1)
    rec, log_r = bg_probe("spoof-recorder", "recorder", "vsock:any:7000", 6)
    vm = VM("p2", PATCHED_CH, "cid=42,backend=vhost-kernel")
    ready = vm.wait_serial(r"AGENT_READY")
    record("P2", "agent_ready", ready)
    record("P2", "guest_boot_lines", re.findall(r"GUEST_\w+.*|LOCAL_CID=\d+", vm.serial_text()))
    snap = vm.snapshot("running")
    record("P2", "ch_threads", snap.get("threads"))
    record("P2", "ch_fds", snap.get("fds"))
    record("P2", "ch_seccomp_modes", snap.get("seccomp"))
    record("P2", "ctl_ident", probe("ctl", "vsock:42:4000", "ident").strip())
    record("P2", "host_state", vhost_state("p2-running"))
    sh(f"grep -iE 'vsock|vhost' {vm.dir / 'ch.log'} | head -40 || true", check=False)

    st, st_path = strace_start(vm, "p3-vhost")
    record("P3", "h2g_stream", probe("client", "vsock:42:5000", "stream", 4 * 1024 * 1024, "h2g-stream").strip())
    record("P4", "strace_h2g", strace_stop(st, st_path, "p3-vhost"))
    record("P3", "h2g_seqpacket", probe("client", "vsock:42:5001", "seqpacket", 0, "h2g-seq").strip())
    st, st_path = strace_start(vm, "p3-vhost-g2h")
    record("P3", "g2h_guest_view", probe("ctl", "vsock:42:4000", "client", 2, 6000, 6001, 4 * 1024 * 1024, timeout=180).strip())
    record("P4", "strace_g2h", strace_stop(st, st_path, "p3-vhost-g2h"))
    record("P3", "g2h_stream_host_view", finish_bg(srv_s, log_s).strip())
    record("P3", "g2h_seqpacket_host_view", finish_bg(srv_q, log_q).strip())
    ss, rsp = held_connection_ss(42, 5000, "p4")
    record("P4", "held_ss", ss)
    record("P4", "held_rsp_header", rsp)

    record("P6", "guest_view", probe("ctl", "vsock:42:4000", "spoof", 2, 7000, timeout=120).strip())
    record("P6", "host_recorder", finish_bg(rec, log_r, timeout=10).strip())

    # P7(c): reboot twice, in-process VM re-creation
    for i in (1, 2):
        before = wait_ident(42, 10)
        snap0 = vm.snapshot(f"pre-reboot-{i}")
        (vm.dir / f"serial-before-reboot-{i}.log").write_text(vm.serial_text())
        shutil.copy2(vm.dir / f"serial-before-reboot-{i}.log", EV / f"p7c-serial-before-reboot-{i}.log")
        probe("ctl", "vsock:42:4000", "reboot")
        time.sleep(2)
        after = wait_ident(42, 120)
        snap = vm.snapshot(f"after-reboot-{i}")
        record("P7c", f"reboot{i}_ident_before", before)
        record("P7c", f"reboot{i}_ident_after", after)
        record("P7c", f"reboot{i}_uptime_reset", (uptime_of(after) or 1e9) < (uptime_of(before) or 0))
        record("P7c", f"reboot{i}_ch_pid_alive_same_pid", vm.proc.poll() is None)
        record("P7c", f"reboot{i}_vhost_vsock_fds_before", [f for f in snap0.get("fds", []) if "vhost-vsock" in f])
        record("P7c", f"reboot{i}_vhost_vsock_fds_after", [f for f in snap.get("fds", []) if "vhost-vsock" in f])
        record("P7c", f"reboot{i}_fd_count_before_after", [len(snap0.get("fds", [])), len(snap.get("fds", []))])
        record("P7c", f"reboot{i}_threads_after", snap.get("threads"))
        if after:
            record("P7c", f"reboot{i}_h2g", probe("client", "vsock:42:5000", "stream", 65536, f"post-reboot-{i}").strip())
            record("P7c", f"reboot{i}_h2g_seq", probe("client", "vsock:42:5001", "seqpacket", 0, f"post-reboot-seq-{i}").strip())
    record("P7c", "ch_log_errors", sh(f"grep -iE 'error|EADDRINUSE|panic' {vm.dir / 'ch.log'} | head -20 || true", check=False, quiet=True))

    # P7(a): poweroff then immediate relaunch with the same CID
    probe("ctl", "vsock:42:4000", "poweroff")
    record("P7a", "ch_exit_code_after_poweroff", vm.wait_exit(30))
    record("P7a", "host_state_after_exit", vhost_state("p7a-after-exit"))
    vm2 = VM("p7a-relaunch", PATCHED_CH, "cid=42,backend=vhost-kernel")
    ok = vm2.wait_serial(r"AGENT_READY")
    record("P7a", "relaunch_agent_ready", ok)
    if ok:
        record("P7a", "relaunch_ident", probe("ctl", "vsock:42:4000", "ident").strip())

    # P7(b): kill -9 with a held host connection, then relaunch
    held = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    held.settimeout(15)
    held.connect((42, 5000))
    held.sendall(b"LITMUS-REQ label=killed len=1048576\n")
    vhost_state("p7b-before-kill")
    os.kill(vm2.pid, signal.SIGKILL)
    rc = vm2.wait_exit(30)
    t0 = time.time()
    try:
        data = held.recv(16)
        held_outcome = f"recv returned {len(data)} bytes"
    except OSError as e:
        held_outcome = f"recv error {e!r}"
    record("P7b", "held_socket_outcome", f"{held_outcome} after {time.time() - t0:.3f}s")
    held.close()
    record("P7b", "ch_exit_after_sigkill", rc)
    record("P7b", "proc_gone", not pathlib.Path(f"/proc/{vm2.pid}").exists())
    record("P7b", "host_state_after_kill", vhost_state("p7b-after-kill"))
    vm3 = VM("p7b-relaunch", PATCHED_CH, "cid=42,backend=vhost-kernel")
    ok = vm3.wait_serial(r"AGENT_READY")
    record("P7b", "relaunch_agent_ready", ok)
    if ok:
        record("P7b", "relaunch_ident", probe("ctl", "vsock:42:4000", "ident").strip())
        record("P7b", "relaunch_h2g", probe("client", "vsock:42:5000", "stream", 1048576, "after-kill").strip())
    probe("ctl", "vsock:42:4000", "poweroff")
    record("P7b", "relaunch_exit", vm3.wait_exit(30))
    record("P7", "host_state_final", vhost_state("p7-final"))


def refcnt():
    try:
        return int(pathlib.Path("/sys/module/vhost_vsock/refcnt").read_text())
    except (FileNotFoundError, ValueError):
        return None


def release_timeline(limit_s=3.0):
    t0 = time.time()
    samples = []
    while time.time() - t0 < limit_s:
        rc = refcnt()
        samples.append((round(time.time() - t0, 4), rc))
        if rc == 0:
            break
        time.sleep(0.002)
    return {"first": samples[0], "last": samples[-1], "n": len(samples)}


def p7d():
    """Rapid same-CID relaunch: does a just-exited CH release CID 42 before the next claim?"""
    cycles = []
    vm = VM("p7d-0", PATCHED_CH, "cid=42,backend=vhost-kernel")
    assert vm.wait_serial(r"AGENT_READY")
    for i in range(1, 21):
        mode = "sigkill" if i % 2 else "poweroff"
        if mode == "sigkill":
            os.kill(vm.pid, signal.SIGKILL)
        else:
            probe("ctl", "vsock:42:4000", "poweroff")
        rc = vm.wait_exit(30)
        rc_at_exit = refcnt()
        vhost_tasks = sh("ps -eLo comm | grep -c '^vhost-' || true", check=False, quiet=True).strip()
        # immediate relaunch: no wait for the release
        nxt = VM(f"p7d-{i}", PATCHED_CH, "cid=42,backend=vhost-kernel")
        ready = nxt.wait_serial(r"AGENT_READY", timeout=30)
        exitcode = nxt.proc.poll()
        err = nxt.stdout.read_text(errors="replace")[-400:] if exitcode is not None else ""
        cycles.append({"i": i, "mode": mode, "prev_exit": rc, "refcnt_at_prev_exit": rc_at_exit,
                       "vhost_tasks_at_prev_exit": vhost_tasks, "relaunch_ready": ready,
                       "relaunch_exit": exitcode, "relaunch_err": err})
        log(f"P7d cycle {cycles[-1]}")
        vm.collect()
        VMS.remove(vm)
        if not ready:
            nxt.kill()
            nxt.collect()
            VMS.remove(nxt)
            # measure how long the release takes, then retry once
            tl = release_timeline()
            cycles[-1]["release_timeline_after_failure"] = tl
            vm = VM(f"p7d-{i}-retry", PATCHED_CH, "cid=42,backend=vhost-kernel")
            cycles[-1]["retry_ready"] = vm.wait_serial(r"AGENT_READY", timeout=30)
            if not cycles[-1]["retry_ready"]:
                break
        else:
            vm = nxt
    record("P7d", "cycles", cycles)
    record("P7d", "relaunch_failures", sum(1 for c in cycles if not c["relaunch_ready"]))
    probe("ctl", "vsock:42:4000", "poweroff")
    vm.wait_exit(30)
    t0 = time.time()
    record("P7d", "final_release_timeline", release_timeline())
    record("P7d", "final_host_state", vhost_state("p7d-final"))


def p5():
    srv, srv_log = bg_probe("p5-g2h", "serve", "vsock:any:6100", "stream", 2)
    a = VM("p5-a", PATCHED_CH, "cid=101,backend=vhost-kernel")
    b = VM("p5-b", PATCHED_CH, "cid=102,backend=vhost-kernel")
    record("P5", "a_ready", a.wait_serial(r"AGENT_READY"))
    record("P5", "b_ready", b.wait_serial(r"AGENT_READY"))
    record("P5", "h2g_a", probe("client", "vsock:101:5000", "stream", 1048576, "multi-a").strip())
    record("P5", "h2g_b", probe("client", "vsock:102:5000", "stream", 1048576, "multi-b").strip())
    record("P5", "g2h_a", probe("ctl", "vsock:101:4000", "client", 2, 6100, 0, 65536).strip())
    record("P5", "g2h_b", probe("ctl", "vsock:102:4000", "client", 2, 6100, 0, 65536).strip())
    record("P5", "g2h_host_view", finish_bg(srv, srv_log).strip())
    record("P5", "guest_a_to_guest_b", probe("ctl", "vsock:101:4000", "peer", 102, 5000, timeout=60).strip())
    record("P5", "host_state_two_vms", vhost_state("p5-two-vms"))
    dup = VM("p5-dup", PATCHED_CH, "cid=101,backend=vhost-kernel")
    rc = dup.wait_exit(30)
    record("P5", "duplicate_exit_code", rc)
    record("P5", "duplicate_output", dup.stdout.read_text(errors="replace")[-3000:])
    record("P5", "duplicate_log_tail", (dup.dir / "ch.log").read_text(errors="replace")[-3000:] if (dup.dir / "ch.log").exists() else "")
    record("P5", "a_after_duplicate", probe("ctl", "vsock:101:4000", "ident").strip())
    record("P5", "a_h2g_after_duplicate", probe("client", "vsock:101:5000", "stream", 65536, "after-dup").strip())
    for cid, vm in ((101, a), (102, b)):
        probe("ctl", f"vsock:{cid}:4000", "poweroff")
        record("P5", f"exit_{cid}", vm.wait_exit(30))


def p9():
    vm = VM("p9", PATCHED_CH, None, api=True)
    vm.wait_serial(r"AGENT_READY|AGENT_WAIT", timeout=60)
    time.sleep(2)
    record("P9", "serial_before_add", vm.serial_text()[-1500:])
    remote = [CH_REMOTE, "--api-socket", str(vm.api)]
    for round_ in (1, 2):
        r = sh(remote + ["add-vsock", "cid=43,backend=vhost-kernel"], check=False)
        record("P9", f"round{round_}_add_vsock", r.strip())
        ident = wait_ident(43, 60)
        record("P9", f"round{round_}_ident", ident)
        if ident:
            record("P9", f"round{round_}_h2g", probe("client", "vsock:43:5000", "stream", 1048576, f"hotplug-{round_}").strip())
        info = sh(remote + ["info"], check=False, quiet=True)
        m = re.search(r'"vsock":\s*\{[^}]*"id":\s*"([^"]+)"', info)
        record("P9", f"round{round_}_vm_info_vsock", m.group(0) if m else info[-800:])
        record("P9", f"round{round_}_host_state_after_add", vhost_state(f"p9-r{round_}-after-add"))
        if m:
            r = sh(remote + ["remove-device", m.group(1)], check=False)
            record("P9", f"round{round_}_remove_device", r.strip())
            time.sleep(3)
            record("P9", f"round{round_}_host_state_after_remove", vhost_state(f"p9-r{round_}-after-remove"))
            record("P9", f"round{round_}_ch_vhost_fds_after_remove", [f for f in vm.snapshot(f"p9-r{round_}-removed").get("fds", []) if "vhost-vsock" in f])
    record("P9", "serial_tail", vm.serial_text()[-2500:])
    vm.kill()


def gates():
    """x86_64 lint + unit tests for the touched CH crates (CH's own tooling)."""
    genv = env.copy()
    genv["CARGO_TARGET_DIR"] = str(BASE / "out" / "ch-gates-target")
    chdir = str(ROOT / "vendors/cloud-hypervisor")
    lint = "cargo clippy".split()
    runs = {
        "lint_x86_64_default_features": lint + ["--locked", "--all", "--all-targets", "--tests", "--examples", "--", "-D", "warnings"],
        "lint_x86_64_kvm_only": lint + ["--locked", "--all", "--all-targets", "--no-default-features", "--tests", "--examples", "--features", "kvm", "--", "-D", "warnings"],
        "unit_tests_x86_64_vsock": ["cargo", "test", "--locked", "--workspace", "--lib", "--", "vsock"],
        "unit_tests_x86_64_config": ["cargo", "test", "--locked", "--workspace", "--lib", "--", "config::"],
    }
    for key, cmd in runs.items():
        t0 = time.time()
        p = subprocess.run(cmd, cwd=chdir, env=genv, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=3600)
        (EV / f"gate-{key}.log").write_text(p.stdout)
        keep = [l for l in p.stdout.splitlines() if l.startswith(("test ", "test result", "error", "warning", "    Finished")) or "panicked" in l]
        record("GATES", key, {"cmd": " ".join(cmd), "rc": p.returncode, "wall_s": round(time.time() - t0, 1), "summary": keep[-150:]})
        log(f"GATE {key} rc={p.returncode}")


def p10():
    """pause/resume + snapshot refusal on a live vhost-kernel VM."""
    vm = VM("p10", PATCHED_CH, "cid=44,backend=vhost-kernel", api=True)
    assert vm.wait_serial(r"AGENT_READY")
    remote = [CH_REMOTE, "--api-socket", str(vm.api)]
    record("P10", "h2g_before", probe("client", "vsock:44:5000", "stream", 1048576, "before-pause").strip())
    record("P10", "pause", sh(remote + ["pause"], check=False).strip())
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(3)
    try:
        s.connect((44, 4000))
        s.sendall(b"ident\n")
        paused_reply = s.recv(200).decode(errors="replace")
    except OSError as e:
        paused_reply = f"error {e!r}"
    record("P10", "ctl_while_paused_3s", paused_reply)
    snapdir = OUT / "p10-snapshot"
    snapdir.mkdir(exist_ok=True)
    record("P10", "snapshot_attempt", sh(remote + ["snapshot", f"file://{snapdir}"], check=False).strip())
    record("P10", "snapshot_dir_contents", sorted(x.name for x in snapdir.iterdir()))
    record("P10", "ch_log_snapshot_lines", sh(f"grep -iE 'snapshot|vhost-kernel' {vm.dir / 'ch.log'} | tail -8 || true", check=False, quiet=True))
    record("P10", "resume", sh(remote + ["resume"], check=False).strip())
    tail = b""
    try:
        s.settimeout(10)
        if paused_reply.startswith("error"):
            s.close()
        else:
            while b"END" not in tail:
                chunk = s.recv(200)
                if not chunk:
                    break
                tail += chunk
    except OSError as e:
        tail += f" error {e!r}".encode()
    record("P10", "ctl_reply_after_resume", (paused_reply + tail.decode(errors="replace")).strip())
    s.close()
    record("P10", "ident_after_resume", wait_ident(44, 20))
    record("P10", "h2g_after_resume", probe("client", "vsock:44:5000", "stream", 1048576, "after-resume").strip())
    record("P10", "h2g_seq_after_resume", probe("client", "vsock:44:5001", "seqpacket", 0, "after-resume-seq").strip())
    record("P10", "ch_alive", vm.proc.poll() is None)
    probe("ctl", "vsock:44:4000", "poweroff")
    record("P10", "exit", vm.wait_exit(30))


run_probe("P10", p10)
run_probe("GATES", gates)
run_probe("P7d", p7d)
run_probe("P1", lambda: unix_baseline("P1", STOCK_CH))
run_probe("P2-P7", p2_p3_p4_p6)
run_probe("P5", p5)
run_probe("P8", lambda: unix_baseline("P8", PATCHED_CH))
run_probe("P9", p9)

# ------------------------------------------------------------------ restore
log("=== restore physical module set")
vhost_state("end")
for vm in VMS:
    vm.kill()
time.sleep(1)
added = [m for m in modules() if m not in MODULES_BEFORE]
record("cleanup", "modules_added_by_run", added)
unload_vsock_stack("end")
after = modules()
record("cleanup", "vsock_stack_vs_pristine_increment_a", {
    "pristine": [m for m in PRISTINE if m in VSOCK_STACK],
    "now": [m for m in after if m in VSOCK_STACK]})
(EV / "modules-after.txt").write_text("\n".join(after) + "\n")
record("cleanup", "modules_still_added", [m for m in after if m not in MODULES_BEFORE])
record("cleanup", "modules_removed_vs_before", [m for m in MODULES_BEFORE if m not in after])
record("cleanup", "ch_processes_at_end", sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False).strip())
log("=== done")
fcntl.flock(lock, fcntl.LOCK_UN)
