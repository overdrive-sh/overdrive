#!/usr/bin/env python3
"""Native-metal harness for the V-11 / VIP / V-14 spike (netns-density-295).

Derived from spike-scratch/netns-density-295-guest-vsock-capture/harness.py (reused, extended).

Runs as root under the canonical metal lease (`cargo xtask metal run -- python3
<increment>/run.py`). Builds patched CH (vendors/cloud-hypervisor,
overdrive/vhost-kernel-vsock), the Aya BPF ELF and the vfwd binary, assembles a
guest initramfs (stock 7.0.0-29 kernel + stock modules, no NIC), boots one VM with
`--vsock cid=N,backend=vhost-kernel`, runs the host owner, host servers and the
selected probes, records raw evidence, and restores host state.
"""
import fcntl, hashlib, json, os, pathlib, random, re, shutil, signal, socket, struct, subprocess, sys, time

INC = None
BASE = pathlib.Path(__file__).resolve().parent
ROOT = BASE.parent.parent
OUT = BASE / "out"
# Reuse the prior guest spike's Cloud Hypervisor build and cargo cache on the metal host.
PRIOR_OUT = BASE.parent / "netns-density-295-guest-vsock-capture" / "out"
EV = None
LOG = None
RESULTS = {}
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
W = "10.99.7.2"
PROCS = []
CLEANUPS = []  # callables run (in reverse) before the after-inventory, even on exception


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    LOG.write(line + "\n")
    LOG.flush()


def sh(cmd, check=True, timeout=600, env=None, quiet=False, tail=60):
    t0 = time.time()
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, timeout=timeout, env=env, errors="replace")
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": p.returncode, "wall_s": round(time.time() - t0, 3),
                            "out": p.stdout[-30000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(map(str, cmd))}  (rc={p.returncode})")
        for line in p.stdout.rstrip().splitlines()[-tail:]:
            log("    " + line)
    if check and p.returncode != 0:
        raise RuntimeError(f"command failed rc={p.returncode}: {cmd}")
    return p.stdout


def record(probe, key, value):
    RESULTS.setdefault(probe, {})[key] = value
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def modules():
    return sorted(l.split()[0] for l in pathlib.Path("/proc/modules").read_text().splitlines())


def inventory(tag):
    out = {}
    out["modules"] = modules()
    out["links"] = sh("ip -br link", quiet=True)
    out["lo_tcx"] = sh("bpftool net show dev lo 2>&1 || true", check=False, quiet=True)
    out["bpf_prog_count"] = len(json.loads(sh("bpftool -j prog show 2>/dev/null || echo []", check=False, quiet=True) or "[]"))
    out["bpf_links"] = sh("bpftool link show 2>&1 | head -200 || true", check=False, quiet=True)
    out["vsock_sockets"] = sh("ss --vsock -anp 2>&1 || true", check=False, quiet=True)
    out["ch_procs"] = sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", check=False, quiet=True)
    out["root_cgroup_bpf"] = sh("bpftool cgroup show /sys/fs/cgroup 2>&1 || true", check=False, quiet=True)
    out["net_tcx_all"] = sh("bpftool net show 2>&1 || true", check=False, quiet=True)
    out["netns"] = sh("ip netns list 2>&1 || true", check=False, quiet=True)
    out["routes"] = sh("ip -4 route; ip rule", check=False, quiet=True)
    out["bpf_stats_enabled"] = sh("sysctl -n kernel.bpf_stats_enabled", check=False, quiet=True)
    (EV / f"inventory-{tag}.json").write_text(json.dumps(out, indent=2))
    return out


# ------------------------------------------------------------------ build
def build():
    log("=== build")
    env = os.environ.copy()
    env["CARGO_HOME"] = str(PRIOR_OUT / "cargo-home")
    env["PATH"] = f"{os.path.expanduser('~ubuntu')}/.cargo/bin:" + env["PATH"]
    env["RUSTUP_HOME"] = f"{os.path.expanduser('~ubuntu')}/.rustup"
    env["CARGO_TARGET_DIR"] = str(PRIOR_OUT / "ch-target")
    sh(["cargo", "build", "--release", "--locked", "--manifest-path", str(ROOT / "vendors/cloud-hypervisor/Cargo.toml"),
        "--bin", "cloud-hypervisor"], env=env, timeout=5400, tail=5)
    ch = PRIOR_OUT / "ch-target/release/cloud-hypervisor"
    benv = env.copy()
    benv["CARGO_TARGET_DIR"] = str(OUT / "bpf-target")
    benv["RUSTFLAGS"] = "-C linker=bpf-linker"
    sh(["cargo", "+nightly-2026-08-05", "build", "--release", "--target", "bpfel-unknown-none", "-Z", "build-std=core",
        "--manifest-path", str(BASE / "bpf/Cargo.toml")], env=benv, timeout=1800, tail=5)
    obj = OUT / "bpf-target/bpfel-unknown-none/release/vfwd-bpf"
    venv = env.copy()
    venv["CARGO_TARGET_DIR"] = str(OUT / "vfwd-target")
    venv["RUSTFLAGS"] = "-C target-feature=+crt-static"
    sh(["cargo", "build", "--release", "--target", "x86_64-unknown-linux-gnu", "--manifest-path",
        str(BASE / "vfwd/Cargo.toml")], env=venv, timeout=1800, tail=5)
    vfwd = OUT / "vfwd-target/x86_64-unknown-linux-gnu/release/vfwd"
    sh(f"file {vfwd} {ch} {obj}; {ch} --version; bpf-linker --version || true; "
       f"cargo +nightly-2026-08-05 --version; rustc --version", env=env)
    return ch, obj, vfwd


def copy_with_libs(root, binary):
    binary = os.path.realpath(binary) if not os.path.islink(binary) else binary
    dst = root / binary.lstrip("/")
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(os.path.realpath(binary), dst)
    for line in subprocess.run(["ldd", os.path.realpath(binary)], text=True, capture_output=True).stdout.splitlines():
        m = re.search(r"(/[^ ]+) \(0x", line)
        if m:
            lib = m.group(1)
            for cand in {lib, os.path.realpath(lib)}:
                d = root / cand.lstrip("/")
                d.parent.mkdir(parents=True, exist_ok=True)
                if not d.exists():
                    shutil.copy2(os.path.realpath(lib), d)


def build_image(obj, vfwd):
    log("=== guest image")
    root = OUT / "initramfs-root"
    if root.exists():
        shutil.rmtree(root)
    for d in ["proc", "sys", "dev", "bin", "sbin", "usr/bin", "usr/sbin", "tmp", "etc/ssh", "root/.ssh", "run/sshd",
              "var/empty", "mods", "lib64"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "sleep", "cut", "basename", "readlink",
                 "mkdir", "sed", "tr", "awk", "grep", "head", "tail", "wc", "sha256sum", "dd", "date", "ps", "kill",
                 "sysctl", "nslookup", "rm", "cp", "env", "seq", "timeout", "true", "false", "printf", "id", "stat",
                 "sort", "uniq", "xargs", "hostname", "chmod", "tee", "nc", "test", "["]:
        (root / "bin" / name).symlink_to("busybox")
    for b in ["/usr/bin/curl", "/usr/sbin/sshd", "/usr/lib/openssh/sshd-session", "/usr/lib/openssh/sshd-auth",
              "/usr/lib/openssh/sftp-server", "/usr/bin/dig", "/usr/bin/getent", "/usr/bin/strace", "/usr/sbin/ip",
              "/usr/bin/ss", "/usr/sbin/bpftool"]:
        if os.path.exists(b):
            copy_with_libs(root, b)
        else:
            log(f"missing guest tool {b}")
    shutil.copy2(vfwd, root / "vfwd")
    shutil.copy2(obj, root / "vfwd-bpf.o")
    shutil.copy2(BASE / "guest/init.sh", root / "init")
    os.chmod(root / "init", 0o755)
    kver = os.uname().release
    order = []
    pins = {}
    for top in ["vmw_vsock_virtio_transport", "dummy", "tcp_diag"]:
        for line in sh(["modprobe", "--show-depends", top], quiet=True).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1])
                name = src.name.removesuffix(".zst")
                if name in order:
                    continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data)
                order.append(name)
                pins[str(src)] = sha(src)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    etc = root / "etc"
    (etc / "passwd").write_text("root:x:0:0:root:/root:/bin/sh\nsshd:x:110:65534::/run/sshd:/bin/false\nnobody:x:65534:65534:nobody:/nonexistent:/bin/false\n")
    (etc / "group").write_text("root:x:0:\nnogroup:x:65534:\n")
    (etc / "shadow").write_text("root:*:19000:0:99999:7:::\nsshd:*:19000:0:99999:7:::\n")
    os.chmod(etc / "shadow", 0o600)
    (etc / "nsswitch.conf").write_text("passwd: files\ngroup: files\nshadow: files\nhosts: files dns\n")
    (etc / "hosts").write_text("127.0.0.1 localhost\n")
    (etc / "resolv.conf").write_text("nameserver 127.0.0.53\n")
    keys = OUT / "keys"
    keys.mkdir(exist_ok=True)
    if not (keys / "client").exists():
        sh(f"ssh-keygen -q -t ed25519 -N '' -f {keys}/client -C spike-client")
        sh(f"ssh-keygen -q -t ed25519 -N '' -f {keys}/hostkey -C spike-guest")
    shutil.copy2(keys / "hostkey", etc / "ssh/ssh_host_ed25519_key")
    os.chmod(etc / "ssh/ssh_host_ed25519_key", 0o600)
    shutil.copy2(keys / "client.pub", root / "root/.ssh/authorized_keys")
    (etc / "ssh/sshd_config").write_text(
        "Port 22\nListenAddress 0.0.0.0\nHostKey /etc/ssh/ssh_host_ed25519_key\nPermitRootLogin prohibit-password\n"
        "PubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\n"
        "PidFile none\nLogLevel VERBOSE\nSubsystem sftp /usr/lib/openssh/sftp-server\nUseDNS no\n")
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}")
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "vfwd": sha(vfwd), "bpf_obj": sha(obj),
                 "guest_curl": sha("/usr/bin/curl"), "guest_sshd": sha("/usr/sbin/sshd")})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))
    return initrd


# ------------------------------------------------------------------ processes
class VM:
    def __init__(self, ch, initrd, cid, owner_args):
        self.dir = OUT / f"vm-{INC.name}-{int(time.time())}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        self.cid = cid
        cmdline = f"console=ttyS0 panic=-1 loglevel=4 vfwd_w={W} vfwd_owner={','.join(owner_args)}"
        self.cmd = [str(ch), "--kernel", GUEST_KERNEL, "--initramfs", str(initrd), "--cmdline", cmdline,
                    "--cpus", "boot=2", "--memory", "size=1024M", "--serial", f"file={self.serial}", "--console", "off",
                    "--vsock", f"cid={cid},backend=vhost-kernel", "--log-file", str(self.dir / "ch.log"), "-v"]
        log("launch VM: " + " ".join(self.cmd))
        self.out = (self.dir / "ch.stdout").open("w")
        self.proc = subprocess.Popen(self.cmd, stdout=self.out, stderr=subprocess.STDOUT)
        PROCS.append(self.proc)

    def text(self):
        return self.serial.read_text(errors="replace") if self.serial.exists() else ""

    def wait(self, pattern, timeout=90):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if re.search(pattern, self.text()):
                return True
            if self.proc.poll() is not None:
                return False
            time.sleep(0.2)
        return False

    def collect(self):
        d = EV / "vm"
        d.mkdir(exist_ok=True)
        for f in self.dir.iterdir():
            if f.is_file():
                shutil.copy2(f, d / f.name)


def spawn(name, argv, env=None):
    f = (OUT / f"proc-{INC.name}-{name}.log").open("w")
    p = subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, env=env)
    p.logpath = OUT / f"proc-{INC.name}-{name}.log"
    p.name = name
    PROCS.append(p)
    return p


def stop(p, sig=signal.SIGTERM):
    if p.poll() is None:
        p.send_signal(sig)
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
    if hasattr(p, "logpath") and p.logpath.exists():
        shutil.copy2(p.logpath, EV / p.logpath.name.replace(f"proc-{INC.name}-", "proc-"))


def gx(cid, cmd, timeout=180, quiet=False):
    """Run a shell command inside the guest via the test agent (vsock 4000)."""
    t0 = time.time()
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(timeout)
    out = b""
    rc = None
    try:
        s.connect((cid, 4000))
        b = cmd.encode()
        s.sendall(struct.pack(">I", len(b)) + b)
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            out += chunk
            m = re.search(rb"\nAGENT_RC=(-?\d+)\n", out)
            if m:
                rc = int(m.group(1))
                out = out[:m.start()]
                break
    except Exception as e:
        out += f"\nHARNESS_EXCEPTION {e!r}".encode()
    finally:
        s.close()
    text = out.decode(errors="replace")
    with (EV / "guest-commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": rc, "wall_s": round(time.time() - t0, 3), "out": text[-30000:]}) + "\n")
    if not quiet:
        log(f"guest$ {cmd}  (rc={rc}, {time.time()-t0:.1f}s)")
        for line in text.rstrip().splitlines()[-40:]:
            log("    " + line)
    return rc, text


def last_json(text, ev=None):
    for line in reversed(text.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                j = json.loads(line)
            except Exception:
                continue
            if ev is None or j.get("ev") == ev:
                return j
    return None


def public_ip():
    out = sh("ip -4 route get 1.1.1.1", quiet=True)
    return re.search(r"src (\S+)", out).group(1)


# ------------------------------------------------------------------ strace payload accounting
SYSCALLS = "read,write,readv,writev,recvfrom,sendto,recvmsg,sendmsg,recvmmsg,sendmmsg,splice,sendfile,pread64,pwrite64"
RE_LINE = re.compile(r"^(?:(\d+)\s+)?(?:\d+\.\d+\s+)?(\w+)\((\d+)<([^>]*)>.*\)\s+=\s+(-?\d+)")


def strace_summary(text):
    per = {}
    for line in text.splitlines():
        m = RE_LINE.match(line)
        if not m:
            continue
        sc, fdkind, ret = m.group(2), m.group(4), int(m.group(5))
        kind = fdkind.split(":")[0]
        if ret <= 0:
            continue
        k = f"{sc}:{kind}"
        per.setdefault(k, [0, 0])
        per[k][0] += 1
        per[k][1] += ret
    return {k: {"calls": v[0], "bytes": v[1]} for k, v in sorted(per.items())}


# ------------------------------------------------------------------ increment driver
VSOCK_MODS = ["vsock_diag", "vhost_vsock", "vsock_loopback", "vmw_vsock_vmci_transport", "vmw_vmci",
              "vmw_vsock_virtio_transport_common", "vsock", "vhost", "vhost_iotlb"]
CTX = {}


def host_owner_args(cid, obj, mode, pub):
    return ["host", "--cid", str(cid), "--obj", str(obj), "--intake", f"{pub}:2222=22", "--intake", f"{pub}:2710=7100",
            "--intake", f"{pub}:2711=7101", "--log", str(OUT / f"host-owner-{INC.name}-{len(PROCS)}.log"),
            "--stats", str(OUT / "host-stats.json")] + mode + CTX.get("host_extra", [])


def start_host_owner(mode, extra=None):
    c = CTX
    if extra is not None:
        c["host_extra"] = extra
    p = spawn(f"host-owner-{len(PROCS)}", [str(c["vfwd"]), *host_owner_args(c["cid"], c["obj"], mode, c["pub"])])
    p.owner_log = OUT / f"host-owner-{INC.name}-{len(PROCS) - 1}.log"
    deadline = time.time() + 10
    while time.time() < deadline and not (OUT / "host-stats.json").exists():
        time.sleep(0.1)
    time.sleep(0.3)
    c["host_owner"] = p
    return p


def stop_host_owner(sig=signal.SIGTERM):
    p = CTX.get("host_owner")
    if p:
        stop(p, sig)
        if hasattr(p, "owner_log") and p.owner_log.exists():
            shutil.copy2(p.owner_log, EV / p.owner_log.name)
        CTX["host_owner"] = None
    try:
        (OUT / "host-stats.json").unlink()
    except FileNotFoundError:
        pass
    for d in pathlib.Path("/sys/fs/cgroup").glob("gvc-spike-*"):
        try:
            d.rmdir()
        except OSError as e:
            log(f"cgroup rmdir {d}: {e}")


def host_stats():
    try:
        return json.loads((OUT / "host-stats.json").read_text())
    except Exception:
        return None


def guest_owner_restart(mode):
    """Restart the guest owner (stand-in for overdrive-init's adaptation) with the given mode."""
    cid = CTX["cid"]
    args = " ".join(mode)
    rc, t = gx(cid, f"kill $(cat /tmp/owner.pid) 2>/dev/null; sleep 0.5; "
                    f"RUST_BACKTRACE=1 /vfwd guest {args} > /tmp/owner.out 2>&1 </dev/null & echo $! > /tmp/owner.pid; sleep 1.5; cat /tmp/owner.out")
    return t


def set_modes(mode, extra=None):
    """Both owners must agree on the ordering mode. Host first (guest ctl connects to it).
    `extra` are host-only owner args (unframe option, egress interfaces, VIPs, D23 intakes)."""
    stop_host_owner()
    start_host_owner(mode, extra)
    out = guest_owner_restart(mode)
    record("modes", " ".join(mode) or "default", out[-500:])
    time.sleep(0.5)


def run_increment(probes, first_mode, vm_cpus=2):
    import fcntl as _f, traceback
    EV.mkdir(exist_ok=False)
    OUT.mkdir(exist_ok=True)
    global LOG
    LOG = (EV / "run.log").open("a")
    lock = open("/tmp/netns-density-295-v11-vip-v14.lock", "a+")
    _f.flock(lock, _f.LOCK_EX | _f.LOCK_NB)
    log("=== preflight")
    sh("uname -a; systemd-detect-virt || true; grep -m1 'model name' /proc/cpuinfo; nproc; ls -l /dev/kvm")
    before = inventory("before")
    record("preflight", "host_uname", os.uname().release)
    record("preflight", "host_uname_v", os.uname().version)
    stale = [m for m in before["modules"] if m in VSOCK_MODS]
    record("preflight", "vsock_modules_loaded_before", stale)
    ch, obj, vfwd = build()
    initrd = build_image(obj, vfwd)
    pub = public_ip()
    cid = random.randint(30000, 60000)
    CTX.update({"ch": ch, "obj": obj, "vfwd": vfwd, "initrd": initrd, "pub": pub, "cid": cid})
    record("preflight", "cid", cid)
    vm = None
    try:
        blob = OUT / "www" / "blob.bin"
        blob.parent.mkdir(exist_ok=True)
        if not blob.exists():
            blob.write_bytes(os.urandom(2 * 1024 * 1024))
        record("preflight", "blob_sha256", sha(blob))
        V = str(vfwd)
        spawn("tcp7000", [V, "tcp-server", "--bind", f"{pub}:7000"])
        spawn("tcp7001", [V, "tcp-server", "--bind", f"{pub}:7001", "--banner"])
        spawn("tcp7002", [V, "tcp-server", "--bind", f"{pub}:7002", "--rst"])
        spawn("tcp7003", [V, "tcp-server", "--bind", f"{pub}:7003", "--read-delay", "300"])
        spawn("tcp7004", [V, "tcp-server", "--bind", f"{pub}:7004", "--early-ack"])
        spawn("udp7010", [V, "udp-server", "--bind", f"{pub}:7010"])
        spawn("http8080", ["python3", "-m", "http.server", "8080", "--bind", pub, "--directory", str(blob.parent)])
        start_host_owner(first_mode)
        vm = VM(ch, initrd, cid, first_mode)
        CTX["vm"] = vm
        ok = vm.wait(r"GUEST_READY", 90)
        record("boot", "guest_ready", ok)
        log(vm.text()[-5000:])
        if not ok:
            raise RuntimeError("guest did not become ready")
        time.sleep(1)
        probes()
    except Exception:
        log("EXCEPTION " + traceback.format_exc())
        record("harness", "exception", traceback.format_exc())
    finally:
        if vm:
            try:
                gsave("/tmp/owner.log", "guest-owner-final.log")
                gsave("/tmp/owner.out", "guest-owner-final.out")
            except Exception:
                pass
            try:
                gx(vm.cid, "cat /tmp/owner.out; tail -c 30000 /tmp/owner.log; cat /tmp/owner-stats.json; tail -60 /tmp/sshd.log; "
                           "tail -20 /tmp/srv7100.log /tmp/srv7101.log", timeout=20)
            except Exception:
                pass
            vm.proc.kill()
            vm.proc.wait()
            vm.collect()
        stop_host_owner()
        for p in list(PROCS):
            if p.poll() is None:
                stop(p)
        for fn in reversed(CLEANUPS):
            try:
                fn()
            except Exception as e:
                log(f"cleanup error {e!r}")
        for p in PROCS:
            if hasattr(p, "logpath") and p.logpath.exists():
                shutil.copy2(p.logpath, EV / p.logpath.name.replace(f"proc-{INC.name}-", "proc-"))
        for f in OUT.glob(f"host-owner-{INC.name}-*.log"):
            shutil.copy2(f, EV / f.name)
        caused = [m for m in modules() if m not in before["modules"]]
        record("cleanup", "modules_caused", caused)
        for attempt in range(10):
            left = [m for m in VSOCK_MODS if m in caused and m in modules()]
            if not left:
                break
            for m in left:
                subprocess.run(["rmmod", m], capture_output=True)
            time.sleep(1)
        after = inventory("after")
        record("cleanup", "modules_restored", after["modules"] == before["modules"])
        record("cleanup", "modules_diff", sorted(set(after["modules"]) ^ set(before["modules"])))
        record("cleanup", "links_unchanged", after["links"] == before["links"])
        record("cleanup", "lo_tcx_unchanged", after["lo_tcx"] == before["lo_tcx"])
        record("cleanup", "root_cgroup_bpf_unchanged", after["root_cgroup_bpf"] == before["root_cgroup_bpf"])
        record("cleanup", "bpf_prog_count", [before["bpf_prog_count"], after["bpf_prog_count"]])
        record("cleanup", "ch_procs_after", after["ch_procs"])
        record("cleanup", "spike_cgroups_left", [str(d) for d in pathlib.Path("/sys/fs/cgroup").glob("gvc-spike-*")])
        record("cleanup", "tcx_unchanged", after["net_tcx_all"] == before["net_tcx_all"])
        record("cleanup", "netns_unchanged", after["netns"] == before["netns"])
        record("cleanup", "routes_unchanged", after["routes"] == before["routes"])
        record("cleanup", "bpf_stats_enabled", [before["bpf_stats_enabled"].strip(), after["bpf_stats_enabled"].strip()])
        log("=== done")


def gsave(path, name):
    """Copy a guest file verbatim into evidence (via the test agent)."""
    rc, t = gx(CTX["cid"], f"cat {path}", quiet=True, timeout=120)
    (EV / name).write_text(t)
    return t


def gx_stream(cid, cmd, on_line, timeout=300):
    """Run a guest command via the agent and call on_line(line, host_monotonic_s) for each
    stdout line as it arrives (the agent streams the child's stdout over vsock)."""
    t0 = time.time()
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(timeout)
    buf = b""
    lines = []
    rc = None
    try:
        s.connect((cid, 4000))
        b = cmd.encode()
        s.sendall(struct.pack(">I", len(b)) + b)
        done = False
        while not done:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                text = line.decode(errors="replace")
                m = re.match(r"AGENT_RC=(-?\d+)", text)
                if m:
                    rc = int(m.group(1))
                    done = True
                    break
                lines.append(text)
                on_line(text, time.monotonic())
    except Exception as e:
        lines.append(f"HARNESS_EXCEPTION {e!r}")
    finally:
        s.close()
    with (EV / "guest-commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": rc, "wall_s": round(time.time() - t0, 3), "out": "\n".join(lines)[-30000:], "stream": True}) + "\n")
    log(f"guest(stream)$ {cmd}  (rc={rc}, {time.time()-t0:.1f}s, {len(lines)} lines)")
    return rc, lines
