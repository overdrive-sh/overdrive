#!/usr/bin/env python3
"""V-25 native-metal driver. Usage (under the metal lease, root):
   cargo xtask metal run -- python3 spike-scratch/netns-density-295-v25/increment-a/run.py host|vm
Stages: preflight (fail-closed substrate probes) -> host sub-claims -> build -> VM sub-claims."""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, struct, subprocess, sys, time
INC = pathlib.Path(__file__).resolve().parent
BASE = INC.parent
ROOT = BASE.parent.parent
sys.path.insert(0, str(BASE))
from common import *
STAGE = sys.argv[1]
EV = INC / "evidence" / STAGE
OUT = BASE / "out"
EV.mkdir(parents=True, exist_ok=False)
OUT.mkdir(exist_ok=True)
OTHER = ROOT / "spike-scratch/netns-density-295-ch-vhost-vsock"   # linked by path, never edited
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
lock = open("/tmp/v25-netns-density-295.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
LOG = (EV / "run.log").open("a")


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True); LOG.write(line + "\n"); LOG.flush()


def sh(cmd, check=True, timeout=600, env=None, quiet=False):
    t0 = time.time()
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, env=env)
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": p.returncode, "wall_s": round(time.time() - t0, 3), "out": p.stdout[-20000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(map(str, cmd))}  (rc={p.returncode})")
        for line in p.stdout.rstrip().splitlines()[-40:]:
            log("    " + line)
    if check and p.returncode != 0:
        raise RuntimeError(f"command failed rc={p.returncode}: {cmd}")
    return p.stdout


RESULTS = {}


def record(probe, key, value):
    RESULTS.setdefault(probe, {})[key] = value
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))
    log(f"RESULT {probe}.{key} = {json.dumps(value)[:600]}")


def modules():
    return sorted(l.split()[0] for l in pathlib.Path("/proc/modules").read_text().splitlines())


# ------------------------------------------------------------------ preflight (fail closed)
log("=== preflight")
sh("uname -a; uname -r; systemd-detect-virt; echo detect_virt_rc=$?; grep -m1 'model name' /proc/cpuinfo; nproc; free -m", check=False)
fail = []
if os.uname().machine != "x86_64": fail.append("not x86_64")
virt = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True)
if virt.stdout.strip() != "none": fail.append(f"systemd-detect-virt={virt.stdout.strip()!r} rc={virt.returncode}")
if not re.search(r"\b(vmx|svm)\b", pathlib.Path("/proc/cpuinfo").read_text()): fail.append("no vmx/svm")
try:
    import stat
    st = os.stat("/dev/kvm")
    if not stat.S_ISCHR(st.st_mode): fail.append("/dev/kvm not chardev")
    kfd = os.open("/dev/kvm", os.O_RDWR)
    api = fcntl.ioctl(kfd, 0xAE00)           # KVM_GET_API_VERSION
    vmfd = fcntl.ioctl(kfd, 0xAE01, 0)       # KVM_CREATE_VM
    os.close(vmfd); os.close(kfd)
    if api != 12: fail.append(f"kvm api {api}")
    record("preflight", "kvm_api_version", api)
except Exception as e:
    fail.append(f"kvm probe: {e!r}")
record("preflight", "uname_r", os.uname().release)
record("preflight", "systemd_detect_virt", virt.stdout.strip())
if fail:
    record("preflight", "FAILED_CLOSED", fail)
    sys.exit(3)
record("preflight", "substrate_ok", True)

MODS_BEFORE = modules()
(EV / "modules-before.txt").write_text("\n".join(MODS_BEFORE) + "\n")
LOADED_BY_US = []
if "vhost_vsock" not in MODS_BEFORE:
    sh("modprobe vhost_vsock")
    LOADED_BY_US = [m for m in modules() if m not in MODS_BEFORE]
    record("preflight", "modules_loaded_by_us", LOADED_BY_US)
sh("ls -l /dev/vhost-vsock /dev/vsock; cat /sys/module/vhost_vsock/refcnt; ps -eLo pid,tid,comm | awk '$3 ~ /^vhost/' | head", check=False)
record("preflight", "vhost_vsock_refcnt_at_start", pathlib.Path("/sys/module/vhost_vsock/refcnt").read_text().strip())
record("preflight", "pre_existing_cloud_hypervisor", sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False, quiet=True).strip())


def restore_modules():
    for _ in range(3):
        for m in LOADED_BY_US:
            if m in modules():
                subprocess.run(["rmmod", m], capture_output=True)
    left = [m for m in modules() if m not in MODS_BEFORE]
    record("cleanup", "modules_not_in_pristine_after_restore", left)


# ------------------------------------------------------------------ host stage
if STAGE == "host":
    try:
        sh(f"python3 {INC}/host_stage.py {EV} 2>&1 | tee {EV}/host-stage.log", timeout=3000)
    finally:
        restore_modules()
    sys.exit(0)

# ------------------------------------------------------------------ build (vm stage)
log("=== build")
env = os.environ.copy()
env["CARGO_HOME"] = str(OUT / "cargo-home")
env["CARGO_TARGET_DIR"] = str(OUT / "ch-target")
src = OUT / "ch-src"
if src.exists(): shutil.rmtree(src)
sh(f"mkdir -p {src} && cd {ROOT}/vendors/cloud-hypervisor && tar --exclude=.git --exclude=target -cf - . | tar -xf - -C {src}")
sh(f"cd {src} && patch -p1 < {BASE}/fd-mode.patch")
sh(["cargo", "build", "--release", "--locked", "--manifest-path", str(src / "Cargo.toml"), "--bin", "cloud-hypervisor"], env=env, timeout=3600)
PATCHED_CH = str(OUT / "ch-target/release/cloud-hypervisor")
penv = env.copy(); penv["CARGO_TARGET_DIR"] = str(OUT / "probe-target"); penv["RUSTFLAGS"] = "-C target-feature=+crt-static"
sh(["cargo", "build", "--release", "--target", "x86_64-unknown-linux-gnu", "--manifest-path", str(OTHER / "probe/Cargo.toml")], env=penv, timeout=900)
PROBE = str(OUT / "probe-target/x86_64-unknown-linux-gnu/release/ch-vsock-probe")
sh(f"file {PATCHED_CH} {PROBE}; sha256sum {PATCHED_CH} {PROBE} {GUEST_KERNEL}; {PATCHED_CH} --version")

root = OUT / "v25-initramfs-root"
if root.exists(): shutil.rmtree(root)
for d in ["proc", "sys", "dev", "bin", "sbin", "tmp"]: (root / d).mkdir(parents=True)
busybox = shutil.which("busybox")
shutil.copy2(busybox, root / "bin/busybox")
for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "reboot", "sleep", "cut", "basename", "readlink"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(PROBE, root / "probe")
shutil.copy2(OTHER / "guest-init.sh", root / "init")
for line in sh(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"]).splitlines():
    if line.startswith("insmod "):
        s = pathlib.Path(line.split()[1])
        data = subprocess.check_output(["zstd", "-dc", str(s)]) if s.suffix == ".zst" else s.read_bytes()
        (root / s.name.removesuffix(".zst")).write_bytes(data)
INITRD = OUT / "v25-initramfs.cpio.gz"
sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -9 > {INITRD}")

# ------------------------------------------------------------------ VM helpers
VMS = []


class VM:
    def __init__(self, name, cid, fd=None, keep_fd=False, extra=(), vsock_override=None, wrap=None):
        self.name = name
        self.dir = OUT / f"v25-vm-{name}-{int(time.time() * 1000)}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        self.stdout = self.dir / "ch.stdout"
        vs = vsock_override or (f"cid={cid},backend=vhost-kernel,fd={fd}" if fd is not None else f"cid={cid},backend=vhost-kernel")
        cmd = [PATCHED_CH, "--kernel", GUEST_KERNEL, "--initramfs", str(INITRD), "--cmdline", "console=ttyS0 panic=-1 loglevel=4",
               "--cpus", "boot=1", "--memory", "size=512M", "--serial", f"file={self.serial}", "--console", "off",
               "--log-file", str(self.dir / "ch.log"), "-v", "--vsock", vs, *extra]
        if wrap: cmd = [*wrap, *cmd]
        self.cmd = cmd
        log(f"launch {name}: {' '.join(cmd)}")
        self.out = self.stdout.open("w")
        pass_fds = [fd] if fd is not None and os.path.exists(f"/proc/self/fd/{fd}") else []
        if pass_fds: os.set_inheritable(fd, True)
        self.proc = subprocess.Popen(cmd, stdout=self.out, stderr=subprocess.STDOUT, pass_fds=pass_fds)
        self.pid = self.proc.pid
        if pass_fds and not keep_fd:
            os.close(fd)               # serve closes its copy once the VMM holds it
        VMS.append(self)

    def serial_text(self):
        return self.serial.read_text(errors="replace") if self.serial.exists() else ""

    def wait_serial(self, pattern, count=1, timeout=60):
        end = time.time() + timeout
        while time.time() < end:
            if len(re.findall(pattern, self.serial_text())) >= count: return True
            if self.proc.poll() is not None: return False
            time.sleep(0.1)
        return False

    def wait_exit(self, timeout=30):
        try: return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired: return None

    def snapshot(self, tag):
        pid = self.pid
        snap = {"tag": tag, "pid": pid}
        try:
            snap["fds"] = [f"{f}->{os.readlink(f'/proc/{pid}/fd/{f}')}" for f in sorted(os.listdir(f"/proc/{pid}/fd"), key=int)]
            tasks = sorted(os.listdir(f"/proc/{pid}/task"), key=int)
            snap["threads"] = [f"{t}:{pathlib.Path(f'/proc/{pid}/task/{t}/comm').read_text().strip()}" for t in tasks]
            snap["seccomp"] = sorted({re.search(r"Seccomp:\s+(\d+)", pathlib.Path(f"/proc/{pid}/task/{t}/status").read_text()).group(1) for t in tasks})
        except (FileNotFoundError, ProcessLookupError):
            snap["gone"] = True
        (self.dir / f"snapshot-{tag}.json").write_text(json.dumps(snap, indent=2))
        return snap

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill(); self.proc.wait()

    def collect(self):
        for f in [self.serial, self.stdout, self.dir / "ch.log", *self.dir.glob("snapshot-*.json")]:
            if f.exists():
                dst = EV / "vms" / self.dir.name / f.name
                dst.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(f, dst)


def probe(*args, timeout=120):
    return sh([PROBE, *map(str, args)], check=False, timeout=timeout, quiet=True)


def bg_probe(name, *args):
    f = (OUT / f"v25-bg-{name}.log").open("w")
    p = subprocess.Popen([PROBE, *map(str, args)], stdout=f, stderr=subprocess.STDOUT)
    time.sleep(0.3)
    return p, OUT / f"v25-bg-{name}.log"


def finish_bg(p, path, timeout=60):
    try: p.wait(timeout=timeout)
    except subprocess.TimeoutExpired: p.kill(); p.wait()
    text = path.read_text(); shutil.copy2(path, EV / path.name); return text


def wait_ident(cid, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        out = probe("ctl", f"vsock:{cid}:4000", "ident", timeout=30)
        if "IDENT" in out: return out.strip().splitlines()[0]
        time.sleep(0.5)
    return None


def run(name, fn):
    log(f"========== {name}")
    try:
        fn(); record(name, "completed", True)
    except Exception:
        import traceback; record(name, "exception", traceback.format_exc())
    finally:
        for vm in list(VMS):
            vm.kill(); vm.collect(); VMS.remove(vm)


def claim_cid_held_check(cid):
    rc, fd = claim_fresh(cid)
    if fd is not None: os.close(fd)
    return ename(rc)


def lat_rec(lat, n, f):
    d = {"claimed": f is not None, "attempts": n, "latency_ms": None if lat is None else round(lat * 1e3, 3)}
    if f is not None: os.close(f)
    return d


# ------------------------------------------------------------------ C: handoff to fd= mode
def c1_handoff():
    CID = CID_BASE + 1
    fd = open_dev(); rc = claim(fd, CID)
    record("C1", "claim_before_launch", ename(rc))
    record("C1", "second_claim_while_serve_holds", claim_cid_held_check(CID))
    vm = VM("c1", CID, fd=fd, extra=("--landlock",))     # serve closes its copy right after Popen
    ready = vm.wait_serial(r"AGENT_READY")
    record("C1", "agent_ready", ready)
    record("C1", "guest_boot_lines", re.findall(r"GUEST_\w+.*|LOCAL_CID=\d+|AGENT_READY.*", vm.serial_text()))
    snap = vm.snapshot("running")
    record("C1", "ch_fds", snap.get("fds")); record("C1", "ch_threads", snap.get("threads")); record("C1", "ch_seccomp", snap.get("seccomp"))
    record("C1", "vhost_vsock_fds_in_vmm", [f for f in snap.get("fds", []) if "vhost-vsock" in f])
    record("C1", "ident_via_cid", wait_ident(CID, 20))
    record("C1", "second_claim_while_vmm_alone_holds", claim_cid_held_check(CID))
    record("C1", "other_cid_claimable_control", claim_cid_held_check(CID_BASE + 7))
    record("C1", "h2g_stream", probe("client", f"vsock:{CID}:5000", "stream", 4 * 1024 * 1024, "v25-h2g-stream").strip())
    record("C1", "h2g_seqpacket", probe("client", f"vsock:{CID}:5001", "seqpacket", 0, "v25-h2g-seq").strip())
    srv_s, ls = bg_probe("g2h-stream", "serve", "vsock:any:6000", "stream", 1)
    srv_q, lq = bg_probe("g2h-seq", "serve", "vsock:any:6001", "seqpacket", 1)
    record("C1", "g2h_guest_view", probe("ctl", f"vsock:{CID}:4000", "client", 2, 6000, 6001, 4 * 1024 * 1024, timeout=180).strip())
    record("C1", "g2h_stream_host_view", finish_bg(srv_s, ls).strip())
    record("C1", "g2h_seqpacket_host_view", finish_bg(srv_q, lq).strip())
    record("C1", "ch_log_vsock_lines", sh(f"grep -iE 'vsock|vhost|landlock|error' {vm.dir / 'ch.log'} | head -30 || true", check=False, quiet=True))
    probe("ctl", f"vsock:{CID}:4000", "poweroff")
    record("C1", "exit_after_poweroff", vm.wait_exit(30))
    record("C1", "reclaim_after_vmm_exit", lat_rec(*claim_poll(CID, 1.0)))


def c2_strace_no_open():
    CID = CID_BASE + 2
    fd = open_dev(); claim(fd, CID)
    sp = OUT / "v25-strace-c2.txt"
    vm = VM("c2", CID, fd=fd, wrap=["strace", "-f", "-qq", "-yy", "-s", "0", "-e", "trace=openat,open,ioctl", "-o", str(sp)])
    record("C2", "agent_ready_under_strace", vm.wait_serial(r"AGENT_READY", timeout=90))
    probe("ctl", f"vsock:{CID}:4000", "poweroff"); vm.wait_exit(30)
    text = sp.read_text(errors="replace") if sp.exists() else ""
    if sp.exists(): shutil.copy2(sp, EV / sp.name)
    record("C2", "openat_of_dev_vhost_vsock_by_vmm", [l for l in text.splitlines() if "openat" in l and "vhost-vsock" in l])
    record("C2", "vhost_ioctls_on_fd", [l[:200] for l in text.splitlines() if "VHOST" in l][:20])


def c3_keep_copy():
    CID = CID_BASE + 3
    fd = open_dev(); claim(fd, CID)
    vm = VM("c3", CID, fd=fd, keep_fd=True)
    record("C3", "agent_ready", vm.wait_serial(r"AGENT_READY"))
    record("C3", "parent_set_owner_while_vmm_owns", ename(set_owner(fd)))
    probe("ctl", f"vsock:{CID}:4000", "poweroff")
    record("C3", "exit_after_poweroff", vm.wait_exit(30))
    record("C3", "claim_after_vmm_exit_parent_copy_still_open", claim_cid_held_check(CID))
    os.close(fd)
    record("C3", "claim_after_parent_close_last_close", lat_rec(*claim_poll(CID, 1.0)))


def cycles(mode, count):
    CID = CID_BASE + 5
    res = []
    for i in range(count):
        fd = open_dev(); assert claim(fd, CID) == 0
        vm = VM(f"cyc-{mode}-{i}", CID, fd=fd)
        if not vm.wait_serial(r"AGENT_READY", timeout=60):
            res.append({"i": i, "ready": False, "exit": vm.proc.poll(), "tail": vm.stdout.read_text(errors='replace')[-300:]})
            vm.kill(); vm.collect(); VMS.remove(vm); continue
        held = claim_cid_held_check(CID)
        if mode == "sigkill":
            vm.proc.send_signal(signal.SIGKILL)
            lat, n, f = claim_poll(CID, 1.0)             # measured from SIGKILL send; zombie not yet reaped
            rc = vm.wait_exit(30)
        else:
            probe("ctl", f"vsock:{CID}:4000", "poweroff")
            rc = vm.wait_exit(30)
            lat, n, f = claim_poll(CID, 1.0)             # measured from reap of the exited VMM
        res.append({"i": i, "ready": True, "held_while_running": held, "exit": rc, **lat_rec(lat, n, f)})
        vm.collect(); VMS.remove(vm)
    ok = sum(1 for r in res if r.get("claimed"))
    lats = sorted(r["latency_ms"] for r in res if r.get("latency_ms") is not None)
    record(f"D_{mode}", "summary", {"cycles": count, "claimed_ok": ok,
                                    "held_while_running_all_EADDRINUSE": all(r.get("held_while_running") == "EADDRINUSE" for r in res if r.get("ready")),
                                    "latency_ms_max": lats[-1] if lats else None, "latency_ms_p50": lats[len(lats) // 2] if lats else None,
                                    "first_attempt_failures": sum(1 for r in res if r.get("attempts", 1) > 1)})
    record(f"D_{mode}", "cycles", res)


def e_held_conn_reset():
    CID = CID_BASE + 6
    fd = open_dev(); claim(fd, CID)
    vm = VM("e", CID, fd=fd)
    record("E", "agent_ready", vm.wait_serial(r"AGENT_READY"))
    held = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); held.settimeout(15); held.connect((CID, 5000))
    held.sendall(b"LITMUS-REQ label=v25-killed len=1048576\n")
    os.kill(vm.pid, signal.SIGKILL); vm.wait_exit(30)
    t0 = time.time()
    try:
        data = held.recv(16); out = f"recv returned {len(data)} bytes"
    except OSError as e:
        out = f"recv error {e!r}"
    record("E", "held_connection_after_sigkill", f"{out} after {time.time() - t0:.3f}s")
    held.close()
    record("E", "reclaim_after_kill", lat_rec(*claim_poll(CID, 1.0)))


def f_reboot():
    CID = CID_BASE + 8
    fd = open_dev(); claim(fd, CID)
    vm = VM("f", CID, fd=fd)
    record("F", "agent_ready", vm.wait_serial(r"AGENT_READY"))
    record("F", "ident_before", wait_ident(CID, 10))
    probe("ctl", f"vsock:{CID}:4000", "reboot")
    t0 = time.time(); rc = vm.wait_exit(90)
    record("F", "vmm_exit_after_guest_reboot", {"exit_code": rc, "alive_after_90s": rc is None, "waited_s": round(time.time() - t0, 1)})
    st = vm.serial_text()
    record("F", "guest_boot_count_in_serial", len(re.findall("GUEST_BOOT", st)))
    record("F", "agent_ready_count_in_serial", len(re.findall("AGENT_READY", st)))
    record("F", "serial_tail_after_reboot", st[-1500:])
    record("F", "ch_log_errors", sh(f"grep -iE 'error|consumed|panic|reboot|vsock' {vm.dir / 'ch.log'} | tail -20 || true", check=False, quiet=True))
    record("F", "ch_stdout_tail", vm.stdout.read_text(errors="replace")[-800:])
    if rc is None:
        record("F", "ident_after_reboot_if_alive", wait_ident(CID, 10))
        record("F", "ch_fds_if_alive", vm.snapshot("alive-after-reboot").get("fds"))
    else:
        record("F", "reclaim_after_exit", lat_rec(*claim_poll(CID, 1.0)))


def g_bad_fds():
    res = {}
    nul = os.open("/dev/null", os.O_RDWR)
    vm = VM("g-null", CID_BASE + 9, fd=nul)
    res["devnull_fd_exit"] = vm.wait_exit(30)
    res["devnull_fd_log"] = sh(f"tail -c 500 {vm.dir / 'ch.log'}; tail -c 400 {vm.stdout}", check=False, quiet=True)
    vm.collect(); VMS.remove(vm)
    vm = VM("g-closed", CID_BASE + 9, vsock_override=f"cid={CID_BASE + 9},backend=vhost-kernel,fd=987")
    res["closed_fd_exit"] = vm.wait_exit(30)
    res["closed_fd_log"] = sh(f"tail -c 500 {vm.dir / 'ch.log'}; tail -c 400 {vm.stdout}", check=False, quiet=True)
    res["cid_left_claimable"] = claim_cid_held_check(CID_BASE + 9)
    record("G", "bad_fd_results", res)


try:
    for name, fn in [("C1", c1_handoff), ("C2", c2_strace_no_open), ("C3", c3_keep_copy), ("E", e_held_conn_reset),
                     ("F", f_reboot), ("G", g_bad_fds),
                     ("D_poweroff", lambda: cycles("poweroff", int(os.environ.get("V25_CYCLES", "20")))),
                     ("D_sigkill", lambda: cycles("sigkill", int(os.environ.get("V25_CYCLES", "20"))))]:
        run(name, fn)
finally:
    for vm in list(VMS): vm.kill()
    restore_modules()
