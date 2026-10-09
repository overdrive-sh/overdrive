#!/usr/bin/env python3
"""V-13 native-metal driver (root, under the canonical metal lease).

SO_LINGER{1,0} on owner-held sockets: kill -9 the guest owner / the host owner mid-transfer and
record, per observer, whether the connection ended in a reset (ECONNRESET) or a clean EOF.

Topology (the design's, not a toy): a real Cloud Hypervisor VM (fork 41a619d19, --vsock
backend=vhost-kernel, stock 7.0.0-29 host + guest kernel); guest owner (vfwd guest, stand-in for
overdrive-init) and host owner (vfwd host, stand-in for the guest-flow owner) pair sockets with
SockHash + SK_SKB verdict programs (payload never read by an owner); remote endpoints live in a
separate network namespace (v13-rem) behind a veth pair.

Usage: run.py <stage> [N]   stages: smoke | matrix | controls | orders
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, struct, subprocess, time, traceback

W = pathlib.Path("/tmp/v13-work")
OUT = W / "out"
STAGE = sys.argv[1]
NTRIALS = int(sys.argv[2]) if len(sys.argv) > 2 else None
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"{STAGE}-{STAMP}"
OBS = EV / "obs"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3280                      # this spike owns guest CIDs 3280..3289
NS, HV, RV = "v13-rem", "v13-h", "v13-r"
HIP, REM_NM, REM_MESH = "172.29.131.1", "172.29.131.2", "172.29.131.3"
GW = "10.99.7.2"                # guest workload address (inside the guest only)
VFWD, OBJ, CH = OUT / "vfwd", OUT / "vfwd-bpf.o", OUT / "cloud-hypervisor"
PROCS = []
RESULTS = {}
VSOCK_MODS = ["vsock_diag", "vhost_vsock", "vsock_loopback", "vmw_vsock_vmci_transport", "vmw_vmci",
              "vmw_vsock_virtio_transport_common", "vsock", "vhost", "vhost_iotlb", "vmw_vsock_virtio_transport"]

EV.mkdir(parents=True, exist_ok=False)
OBS.mkdir()
LOG = (EV / "run.log").open("a")


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    LOG.write(line + "\n"); LOG.flush()


def sh(cmd, check=True, timeout=300, quiet=False, env=None):
    t0 = time.time()
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                       timeout=timeout, errors="replace", env=env)
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": p.returncode, "wall_s": round(time.time() - t0, 3), "out": p.stdout[-20000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(map(str, cmd))}  (rc={p.returncode})")
        for line in p.stdout.rstrip().splitlines()[-40:]:
            log("    " + line)
    if check and p.returncode != 0:
        raise RuntimeError(f"command failed rc={p.returncode}: {cmd}")
    return p.stdout


def record(probe, key, value):
    RESULTS.setdefault(probe, {})[key] = value
    (EV / "results.json").write_text(json.dumps(RESULTS, indent=2, sort_keys=True))
    log(f"RESULT {probe}.{key} = {json.dumps(value)[:500]}")


def sha(p):
    return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


def modules():
    return sorted(l.split()[0] for l in pathlib.Path("/proc/modules").read_text().splitlines())


# ------------------------------------------------------------------ preflight (fail closed)
def preflight():
    log("=== preflight")
    sh("uname -a; uname -r; systemd-detect-virt; echo detect_virt_rc=$?; grep -m1 'model name' /proc/cpuinfo; nproc; free -m", check=False)
    fail = []
    if os.uname().machine != "x86_64": fail.append("not x86_64")
    virt = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True)
    if virt.returncode != 1 or virt.stdout.strip() != "none": fail.append(f"systemd-detect-virt={virt.stdout.strip()!r} rc={virt.returncode}")
    cpu = pathlib.Path("/proc/cpuinfo").read_text()
    if not re.search(r"^flags.*\b(vmx|svm)\b", cpu, re.M): fail.append("no vmx/svm")
    if re.search(r"^flags.*\bhypervisor\b", cpu, re.M): fail.append("hypervisor cpu flag")
    try:
        import stat
        st = os.stat("/dev/kvm")
        if not stat.S_ISCHR(st.st_mode): fail.append("/dev/kvm not chardev")
        kfd = os.open("/dev/kvm", os.O_RDWR)
        api = fcntl.ioctl(kfd, 0xAE00)
        vmfd = fcntl.ioctl(kfd, 0xAE01, 0)
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


def inventory(tag):
    out = {"modules": modules()}
    out["links"] = sh("ip -br link", quiet=True)
    out["netns"] = sh("ip netns list", quiet=True, check=False)
    out["nft_tables"] = sh("nft list tables", quiet=True, check=False)
    out["ch_procs"] = sh("pgrep -af cloud-hypervisor | grep -v pgrep || true", quiet=True, check=False)
    out["v13_procs"] = sh("pgrep -af 'v13-work' | grep -v pgrep | grep -v 'run.py' || true", quiet=True, check=False)
    out["cgroups"] = sorted(p.name for p in pathlib.Path("/sys/fs/cgroup").glob("v13-*"))
    out["routes"] = sh("ip route", quiet=True)
    (EV / f"inventory-{tag}.json").write_text(json.dumps(out, indent=2))
    return out


# ------------------------------------------------------------------ fixture: remote endpoints in their own netns
def net_up():
    r = sh(f"ip route show 172.29.131.0/24; ip netns list | grep -c '^{NS}' || true", quiet=True, check=False)
    assert r.strip() in ("0", ""), f"172.29.131.0/24 or netns {NS} already present: {r!r}"
    sh(f"ip netns add {NS}")
    sh(f"ip link add {HV} type veth peer name {RV}")
    sh(f"ip link set {RV} netns {NS}")
    sh(f"ip addr add {HIP}/29 dev {HV}; ip link set {HV} up")
    sh(f"ip netns exec {NS} ip addr add {REM_NM}/29 dev {RV}")
    sh(f"ip netns exec {NS} ip addr add {REM_MESH}/29 dev {RV}")
    sh(f"ip netns exec {NS} ip link set {RV} up; ip netns exec {NS} ip link set lo up")
    sh(f"ping -c1 -W2 {REM_NM}; ping -c1 -W2 {REM_MESH}; ip route get {REM_NM}", check=False)


def net_down():
    sh(f"ip netns del {NS}", check=False)
    sh(f"ip link del {HV}", check=False, quiet=True)


# ------------------------------------------------------------------ processes
def spawn(name, argv):
    f = (OBS / f"proc-{name}.log").open("w")
    p = subprocess.Popen(argv, stdout=f, stderr=subprocess.STDOUT, start_new_session=True)
    p.name = name
    PROCS.append(p)
    return p


def reap(p, sig=signal.SIGKILL):
    if p.poll() is None:
        try:
            os.killpg(p.pid, sig)
        except ProcessLookupError:
            pass
        try:
            p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
    if p in PROCS:
        PROCS.remove(p)


class VM:
    def __init__(self, owner_args):
        self.dir = OUT / f"vm-{STAGE}-{STAMP}"
        self.dir.mkdir()
        self.serial = self.dir / "serial.log"
        cmdline = f"console=ttyS0 panic=-1 loglevel=4 vfwd_w={GW} vfwd_owner={','.join(owner_args)}"
        self.cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", cmdline,
                    "--cpus", "boot=2", "--memory", "size=1024M", "--serial", f"file={self.serial}", "--console", "off",
                    "--vsock", f"cid={CID},backend=vhost-kernel", "--log-file", str(self.dir / "ch.log"), "-v"]
        log("launch VM: " + " ".join(self.cmd))
        self.out = (self.dir / "ch.stdout").open("w")
        self.proc = subprocess.Popen(self.cmd, stdout=self.out, stderr=subprocess.STDOUT, start_new_session=True)

    def text(self):
        return self.serial.read_text(errors="replace") if self.serial.exists() else ""

    def wait(self, pattern, timeout=90):
        end = time.time() + timeout
        while time.time() < end:
            if re.search(pattern, self.text()): return True
            if self.proc.poll() is not None: return False
            time.sleep(0.2)
        return False

    def collect(self):
        d = EV / "vm"; d.mkdir(exist_ok=True)
        for f in self.dir.iterdir():
            if f.is_file(): shutil.copy2(f, d / f.name)


def gx(cmd, timeout=60, quiet=True):
    """Run a shell command in the guest via the test agent (vsock 4000)."""
    t0 = time.time()
    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    s.settimeout(timeout)
    out, rc = b"", None
    try:
        s.connect((CID, 4000))
        b = cmd.encode()
        s.sendall(struct.pack(">I", len(b)) + b)
        while True:
            chunk = s.recv(65536)
            if not chunk: break
            out += chunk
            m = re.search(rb"\nAGENT_RC=(-?\d+)\n", out)
            if m:
                rc = int(m.group(1)); out = out[:m.start()]; break
    except Exception as e:
        out += f"\nHARNESS_EXCEPTION {e!r}".encode()
    finally:
        s.close()
    text = out.decode(errors="replace")
    with (EV / "guest-commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": t0, "cmd": cmd, "rc": rc, "wall_s": round(time.time() - t0, 3), "out": text[-8000:]}) + "\n")
    return rc, text


# ------------------------------------------------------------------ guest image (as the capture spike's harness)
def copy_with_libs(root, binary):
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
                if not d.exists(): shutil.copy2(os.path.realpath(lib), d)


def build_image():
    log("=== guest image")
    root = OUT / "initramfs-root"
    if root.exists(): shutil.rmtree(root)
    for d in ["proc", "sys", "dev", "bin", "sbin", "usr/bin", "usr/sbin", "tmp", "mods", "lib64", "etc"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for name in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "poweroff", "sleep", "cut", "basename", "readlink", "mkdir",
                 "sed", "tr", "awk", "grep", "head", "tail", "wc", "date", "ps", "kill", "sysctl", "rm", "cp", "env", "seq",
                 "timeout", "true", "false", "printf", "id", "stat", "xargs", "chmod", "tee", "nc", "test", "pkill", "pgrep", "["]:
        (root / "bin" / name).symlink_to("busybox")
    for b in ["/usr/sbin/ip", "/usr/bin/ss", "/usr/bin/strace"]:
        if os.path.exists(b): copy_with_libs(root, b)
    shutil.copy2(VFWD, root / "vfwd"); shutil.copy2(OBJ, root / "vfwd-bpf.o")
    init = (W / "src/guest/init.sh").read_text()
    # no sshd / tcp-server litmus in this spike's guest; keep the agent and the owner
    init = re.sub(r"/vfwd tcp-server.*\n", "", init)
    init = re.sub(r"/usr/sbin/sshd.*\n", "", init)
    (root / "init").write_text(init); os.chmod(root / "init", 0o755)
    order, pins = [], {}
    for top in ["vmw_vsock_virtio_transport", "dummy"]:
        for line in sh(["modprobe", "--show-depends", top], quiet=True).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); order.append(name); pins[str(src)] = sha(src)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    (root / "etc/passwd").write_text("root:x:0:0:root:/root:/bin/sh\n")
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}")
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "vfwd": sha(VFWD), "bpf_obj": sha(OBJ), "cloud_hypervisor": sha(CH)})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))


# ------------------------------------------------------------------ owners
CTX = {"linger": "yes", "hup_abort": "no", "half_msg": "no"}


def host_args():
    return ["host", "--cid", str(CID), "--obj", str(OBJ), "--intake", f"{HIP}:2720=7300", "--allow-foreign", "yes", "--no-lo", "yes",
            "--mesh", f"{REM_MESH}:7201", "--linger", CTX["linger"], "--hup-abort", CTX["hup_abort"], "--half-msg", CTX["half_msg"], "--log", str(OUT / "host-owner.log"), "--stats", str(OUT / "host-stats.json")]


def start_host_owner():
    for f in ("host-owner.log", "host-stats.json"):
        (OUT / f).unlink(missing_ok=True)
    p = spawn("host-owner", [str(VFWD), *host_args()])
    end = time.time() + 15
    while time.time() < end and not (OUT / "host-stats.json").exists():
        if p.poll() is not None: raise RuntimeError("host owner died at start")
        time.sleep(0.1)
    CTX["host"] = p
    return p


def stop_host_owner():
    p = CTX.get("host")
    if p: reap(p, signal.SIGTERM)
    CTX["host"] = None
    for d in pathlib.Path("/sys/fs/cgroup").glob("v13-spike-*"):
        try: d.rmdir()
        except OSError as e: log(f"cgroup rmdir {d}: {e}")


def hello_count():
    try:
        return (OUT / "host-owner.log").read_text().count('"ev":"hello"')
    except FileNotFoundError:
        return 0


def restart_owners():
    """Fresh host owner then fresh guest owner; wait until the control session is live."""
    stop_host_owner()
    start_host_owner()
    n0 = hello_count()
    rc, t = gx(f"kill -9 $(cat /tmp/owner.pid) 2>/dev/null; sleep 0.3; RUST_BACKTRACE=1 /vfwd guest --linger {CTX['linger']} --hup-abort {CTX['hup_abort']} --half-msg {CTX['half_msg']} > /tmp/owner.out 2>&1 </dev/null & echo $! > /tmp/owner.pid; sleep 1.2; cat /tmp/owner.out")
    ok = "OWNER_READY" in t
    end = time.time() + 8
    while time.time() < end and hello_count() <= n0:
        time.sleep(0.1)
    if not ok or hello_count() <= n0:
        raise RuntimeError(f"owners not ready: guest_ready={ok} hello={hello_count()} out={t[-300:]}")


# ------------------------------------------------------------------ observers
def read_json(path):
    try:
        return json.loads(pathlib.Path(path).read_text())
    except Exception:
        return None


def guest_json(path):
    rc, t = gx(f"cat {path} 2>/dev/null")
    try:
        return json.loads(t.strip().splitlines()[-1])
    except Exception:
        return None


def netns_cmd(*a):
    return ["ip", "netns", "exec", NS, str(VFWD), *a]


def start_flow(kind, tag, finite=None):
    """Start one flow of `kind`; return {'obs': {name: getter}, 'kind': kind}."""
    hp = OBS / f"{tag}-host.json"
    fin = ["--finite", str(finite)] if finite else []
    finS = " ".join(fin)
    gp = f"/tmp/{tag}-guest.json"
    host_getter = lambda: read_json(hp)
    guest_getter = lambda: guest_json(gp)
    if kind == "out_nonmesh":
        spawn(f"{tag}-srv", netns_cmd("stream-server", "--bind", f"{REM_NM}:7200", "--out", str(hp), *fin))
        time.sleep(0.4)
        gx(f"/vfwd stream-client --dst {REM_NM}:7200 --out {gp} {finS} </dev/null >/tmp/{tag}-guest.log 2>&1 &")
        return {"kind": kind, "obs": {"guest_app": guest_getter, "remote_nonmesh": host_getter}}
    if kind == "out_mesh":
        spawn(f"{tag}-srv", netns_cmd("stream-server", "--bind", f"{REM_MESH}:7201", "--out", str(hp)))
        time.sleep(0.4)
        gx(f"/vfwd stream-client --dst {REM_MESH}:7201 --out {gp} </dev/null >/tmp/{tag}-guest.log 2>&1 &")
        return {"kind": kind, "obs": {"guest_app": guest_getter, "remote_mesh": host_getter}}
    if kind == "in":
        gx(f"/vfwd stream-server --bind 0.0.0.0:7300 --out {gp} {finS} </dev/null >/tmp/{tag}-guest.log 2>&1 &")
        time.sleep(0.6)
        spawn(f"{tag}-cli", netns_cmd("stream-client", "--dst", f"{HIP}:2720", "--out", str(hp), *fin))
        return {"kind": kind, "obs": {"guest_app": guest_getter, "external_client": host_getter}}
    raise ValueError(kind)


def wait_midtransfer(flows, timeout=15):
    end = time.time() + timeout
    while time.time() < end:
        ok = True
        for f in flows:
            for name, g in f["obs"].items():
                j = g()
                if not j or j["bytes_rx"] < 200_000 or j["bytes_tx"] < 200_000:
                    ok = False
        if ok: return True
        time.sleep(0.3)
    return False


def kill_guest_owner():
    t = time.time_ns()
    rc, out = gx("kill -9 $(cat /tmp/owner.pid)")
    return t, rc


def kill_host_owner():
    t = time.time_ns()
    os.kill(CTX["host"].pid, signal.SIGKILL)
    return t


def wait_done(flows, timeout=12):
    end = time.time() + timeout
    res = {}
    while time.time() < end:
        alldone = True
        for i, f in enumerate(flows):
            for name, g in f["obs"].items():
                j = g()
                res[(i, name)] = j
                if not j or not j.get("done"): alldone = False
        if alldone: break
        time.sleep(0.25)
    return res


def classify(j):
    if not j: return "NO_RESULT"
    if not j.get("done"): return "NOT_DONE(" + j.get("rx_verdict", "?") + ")"
    return j["rx_verdict"]


def one_trial(cell, idx, kinds, order, jitter_ms=0):
    tag = f"{cell}-{idx:03d}"
    rec = {"cell": cell, "trial": idx, "kinds": kinds, "order": order, "linger": CTX["linger"]}
    for p in list(PROCS):
        if p.name.endswith("-srv") or p.name.endswith("-cli"): reap(p)
    gx("pkill -f 'vfwd stream-' 2>/dev/null; rm -f /tmp/*-guest.json /tmp/*-guest.log; true")
    restart_owners()
    ta = time.time_ns(); rc, gt = gx("/vfwd now"); tb = time.time_ns()
    try: offset = int(gt.strip().splitlines()[-1]) - (ta + tb) // 2     # guest clock minus host clock, +-~1 ms
    except Exception: offset = 0
    rec["guest_clock_offset_ns"] = offset
    flows = [start_flow(k, f"{tag}-{k}") for k in kinds]
    rec["midtransfer"] = wait_midtransfer(flows)
    pre = {f"{i}:{n}": {k: v for k, v in (g() or {}).items() if k in ("bytes_rx", "bytes_tx")} for i, f in enumerate(flows) for n, g in f["obs"].items()}
    rec["pre_kill_bytes"] = pre
    if not rec["midtransfer"]:
        rec["verdicts"] = {"INVALID": "no mid-transfer traffic before kill"}
        return rec, flows
    kills = []
    for j, who in enumerate(order):
        if j and jitter_ms: time.sleep(jitter_ms / 1000)
        if who == "G":
            t, rc = kill_guest_owner(); kills.append({"who": "guest_owner", "t_ns": t, "agent_rc": rc})
        else:
            t = kill_host_owner(); kills.append({"who": "host_owner", "t_ns": t})
    rec["kills"] = kills
    res = wait_done(flows)
    rec["verdicts"] = {}
    rec["observers"] = {}
    for (i, name), j in res.items():
        key = f"{kinds[i]}:{name}"
        rec["verdicts"][key] = classify(j)
        rec["observers"][key] = j
        fe = (j or {}).get("first_event")
        if fe:
            base = kills[0]["t_ns"] - (0 if name != "guest_app" else offset)
            rec["observers"][key]["first_event_after_first_kill_ms"] = round((int(fe["t_ns"]) - base) / 1e6, 2)
    return rec, flows


def clean_trial(cell, idx, kind, nbytes):
    """No kill: a finite bidirectional transfer with a clean half-close from both applications."""
    tag = f"{cell}-{idx:03d}"
    rec = {"cell": cell, "trial": idx, "kinds": [kind], "order": "", "linger": CTX["linger"], "finite": nbytes}
    for p in list(PROCS):
        if p.name.endswith("-srv") or p.name.endswith("-cli"): reap(p)
    gx("pkill -f 'vfwd stream-' 2>/dev/null; rm -f /tmp/*-guest.json /tmp/*-guest.log; true")
    restart_owners()
    f = start_flow(kind, f"{tag}-{kind}", finite=nbytes)
    res = wait_done([f], timeout=25)
    rec["observers"] = {}; rec["verdicts"] = {}
    for (i, name), j in res.items():
        ok = bool(j) and j.get("done") and [e["kind"] for e in j["events"]] == ["rx_eof"] and j["bytes_rx"] == nbytes and j["bytes_tx"] == nbytes
        rec["observers"][f"{kind}:{name}"] = j
        rec["verdicts"][f"{kind}:{name}"] = "CLEAN" if ok else "NOT_CLEAN"
    return rec, [f]


def run_clean_cell(cell, kind, n, half_msg, nbytes=2_000_000):
    CTX["linger"] = "yes"; CTX["hup_abort"] = "no"; CTX["half_msg"] = half_msg
    tally = {}
    log(f"=== clean cell {cell}: kind={kind} half_msg={half_msg} n={n} bytes={nbytes}")
    for i in range(n):
        try:
            rec, _ = clean_trial(cell, i, kind, nbytes)
        except Exception:
            rec = {"cell": cell, "trial": i, "harness_exception": traceback.format_exc()}
            log("EXCEPTION " + traceback.format_exc())
        save_trial(rec, keep_logs=True)
        for k, v in rec.get("verdicts", {}).items():
            tally.setdefault(k, {}).setdefault(v, 0); tally[k][v] += 1
        log(f"  trial {i}: {rec.get('verdicts')}")
    record("cells", cell, {"kind": kind, "half_msg": half_msg, "n": n, "finite_bytes": nbytes, "tally": tally})


def save_trial(rec, keep_logs):
    with (EV / "results.jsonl").open("a") as f:
        f.write(json.dumps(rec, sort_keys=True) + "\n")
    if keep_logs:
        d = EV / "trial-logs" / f"{rec['cell']}-{rec['trial']:03d}"
        d.mkdir(parents=True, exist_ok=True)
        if (OUT / "host-owner.log").exists(): shutil.copy2(OUT / "host-owner.log", d / "host-owner.log")
        rc, t = gx("cat /tmp/owner.log 2>/dev/null | tail -c 20000; echo ---; cat /tmp/owner.out | tail -c 2000")
        (d / "guest-owner.log").write_text(t)


def run_cell(cell, kinds, order, n, linger="yes", jitter_ms=0, hup_abort="no", half_msg="no"):
    CTX["linger"] = linger
    CTX["hup_abort"] = hup_abort
    CTX["half_msg"] = half_msg
    tally = {}
    log(f"=== cell {cell}: kinds={kinds} order={order} linger={linger} n={n}")
    for i in range(n):
        try:
            rec, flows = one_trial(cell, i, kinds, order, jitter_ms)
        except Exception:
            rec = {"cell": cell, "trial": i, "kinds": kinds, "order": order, "linger": linger, "harness_exception": traceback.format_exc()}
            log("EXCEPTION " + traceback.format_exc())
            flows = []
        save_trial(rec, keep_logs=True)
        for k, v in rec.get("verdicts", {}).items():
            tally.setdefault(k, {}).setdefault(v, 0)
            tally[k][v] += 1
        log(f"  trial {i}: {rec.get('verdicts')}")
    record("cells", cell, {"kinds": kinds, "order": order, "linger": linger, "hup_abort": hup_abort, "half_msg": half_msg, "n": n, "tally": tally})
    return tally


def ktls_stage():
    """Edge case: a kTLS-ULP socket with SO_LINGER{1,0} held by a process that is SIGKILLed."""
    n = NTRIALS or 20
    sh("lsmod | grep -E '^tls ' || true", check=False)
    shutil.copy2(W / "ktls_holder.py", OUT / "ktls_holder.py")
    for linger, tls in (("yes", "yes"), ("no", "yes"), ("yes", "no")):
        cell = f"ktls-linger{linger}-tls{tls}"
        tally = {}
        for i in range(n):
            hp = OBS / f"{cell}-{i:03d}-host.json"
            srv = spawn(f"{cell}-{i}-srv", netns_cmd("stream-server", "--bind", f"{REM_MESH}:7201", "--out", str(hp), "--sleep-us", "1000000000"))   # receive-only: no unread data can force an RST at the holder
            time.sleep(0.4)
            h = spawn(f"{cell}-{i}-holder", ["python3", str(OUT / "ktls_holder.py"), f"{REM_MESH}:7201", linger, tls])
            end = time.time() + 10
            while time.time() < end:
                j = read_json(hp)
                if j and j["bytes_rx"] > 200_000: break
                time.sleep(0.2)
            else:
                log(f"  {cell} {i}: no traffic"); reap(h); reap(srv); continue
            t = time.time_ns(); os.kill(h.pid, signal.SIGKILL)
            end = time.time() + 8
            while time.time() < end:
                j = read_json(hp)
                if j and j.get("done"): break
                time.sleep(0.1)
            v = classify(j)
            rec = {"cell": cell, "trial": i, "linger": linger, "tls": tls, "verdict": v, "kill_t_ns": t, "observer": j}
            with (EV / "results.jsonl").open("a") as f: f.write(json.dumps(rec, sort_keys=True) + "\n")
            tally[v] = tally.get(v, 0) + 1
            reap(h); reap(srv)
        log(f"  {cell}: {tally}")
        record("cells", cell, {"linger": linger, "tls_tx": tls, "n": n, "tally": tally})


def main():
    preflight()
    before = inventory("before")
    record("preflight", "modules_before", [m for m in before["modules"] if m in VSOCK_MODS])
    record("preflight", "ch_version", sh(f"{CH} --version", quiet=True).strip())
    lock = open("/tmp/v13-netns-density-295.lock", "a+")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    vm = None
    try:
        if "vhost_vsock" not in before["modules"]:
            sh("modprobe vhost_vsock")
        sh("ls -l /dev/vhost-vsock /dev/vsock", check=False)
        build_image()
        net_up()
        if STAGE == "ktls":
            return ktls_stage()
        # `vm` boots with the owner mode; trials restart both owners anyway
        start_host_owner()
        vm = VM(["--linger", "yes"])
        ok = vm.wait(r"GUEST_READY", 90)
        record("boot", "guest_ready", ok)
        log(vm.text()[-3000:])
        if not ok: raise RuntimeError("guest did not become ready")
        record("boot", "guest_uname", gx("uname -r")[1].strip())
        time.sleep(1)
        S = STAGE
        if S == "smoke":
            for kinds, order in ((["out_nonmesh"], "G"), (["out_nonmesh"], "H"), (["out_mesh"], "G"), (["out_mesh"], "H"), (["in"], "G"), (["in"], "H")):
                run_cell(f"smoke-{kinds[0]}-{order}", kinds, order, NTRIALS or 1)
        elif S == "matrix":
            n = NTRIALS or 20
            for kind in ("out_nonmesh", "out_mesh", "in"):
                for order in ("G", "H"):
                    run_cell(f"{kind}-kill{order}", [kind], order, n)
        elif S == "controls":
            n = NTRIALS or 10
            for kind in ("out_nonmesh", "out_mesh", "in"):
                for order in ("G", "H"):
                    run_cell(f"ctl-nolinger-{kind}-kill{order}", [kind], order, n, linger="no")
        elif S == "variantB":
            n = NTRIALS or 30
            for kind in ("out_nonmesh", "out_mesh", "in"):
                for order in ("G", "H"):
                    run_cell(f"hupabort-{kind}-kill{order}", [kind], order, n, hup_abort="yes")
        elif S == "variantC":
            n = NTRIALS or 30
            for kind in ("out_nonmesh", "out_mesh", "in"):
                for order in ("G", "H"):
                    run_cell(f"halfmsg-{kind}-kill{order}", [kind], order, n, half_msg="yes")
        elif S == "clean":
            n = NTRIALS or 10
            for kind in ("out_nonmesh", "in"):
                for hm in ("no", "yes"):
                    run_clean_cell(f"clean-{kind}-halfmsg{hm}", kind, n, hm)
        elif S == "orders":
            n = NTRIALS or 10
            for order, jit in (("GH", 0), ("HG", 0), ("GH", 40), ("HG", 40)):
                run_cell(f"all3-order{order}-jit{jit}", ["out_nonmesh", "out_mesh", "in"], order, n, jitter_ms=jit)
        else:
            raise SystemExit(f"unknown stage {S}")
    except SystemExit:
        raise
    except Exception:
        log("EXCEPTION " + traceback.format_exc())
        record("harness", "exception", traceback.format_exc())
    finally:
        if vm:
            try:
                (EV / "guest-owner-final.log").write_text(gx("cat /tmp/owner.out; tail -c 20000 /tmp/owner.log")[1])
            except Exception:
                pass
            try: os.killpg(vm.proc.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            vm.proc.wait()
            vm.collect()
        stop_host_owner()
        for p in list(PROCS): reap(p)
        if (OUT / "host-owner.log").exists(): shutil.copy2(OUT / "host-owner.log", EV / "host-owner-final.log")
        net_down()
        caused = [m for m in modules() if m not in before["modules"]]
        record("cleanup", "modules_caused", caused)
        for _ in range(10):
            left = [m for m in VSOCK_MODS if m in caused and m in modules()]
            if not left: break
            for m in left: subprocess.run(["rmmod", m], capture_output=True)
            time.sleep(1)
        after = inventory("after")
        record("cleanup", "modules_diff", sorted(set(after["modules"]) ^ set(before["modules"])))
        for k in ("links", "netns", "nft_tables", "routes"):
            a, b = set(after[k].splitlines()), set(before[k].splitlines())
            record("cleanup", f"{k}_added_removed", {"added": sorted(a - b), "removed": sorted(b - a)})
        record("cleanup", "v13_links_left", [l for l in after["links"].splitlines() if "v13" in l])
        record("cleanup", "v13_netns_left", [l for l in after["netns"].splitlines() if "v13" in l])
        record("cleanup", "v13_cgroups_left", after["cgroups"])
        record("cleanup", "v13_procs_left", after["v13_procs"])
        log("=== done")


main()
