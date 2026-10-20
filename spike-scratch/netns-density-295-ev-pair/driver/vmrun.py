#!/usr/bin/env python3
"""ev-pair native-metal VM driver (root, under the metal lease; launched by launch.py).

A real Cloud Hypervisor VM (fork, --vsock backend=vhost-kernel), guest kernel = the stock 7.0.0-29-generic image. ovdpair.ko is loaded
INSIDE THE GUEST only. The host runs `pairprobe peer` (plain userspace, no module) for the real-vhost (G) runs; the L runs need no host
participation (vsock loopback inside the guest). The guest PID 1 runs the runlist and powers off.

Usage: vmrun.py <runlist-file> [mem=MB] [timeout=SEC] [peer=1] [name=label]
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, subprocess, time

W = pathlib.Path("/var/tmp/ev-pair-work"); OUT = W / "out"
RUNLIST = pathlib.Path(sys.argv[1]).read_text(); KV = dict(a.split("=", 1) for a in sys.argv[2:])
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"vm-{KV.get('name', 'run')}-{STAMP}"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3371
MEM_MB = int(KV.get("mem", "1536")); TIMEOUT = int(KV.get("timeout", "1500")); IDLE = int(KV.get("idle", "150"))
CH = OUT / "cloud-hypervisor"; PROBE = OUT / "pairprobe"; KO = OUT / "mod" / "ovdpair.ko"
KVER = "7.0.0-29-generic"
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


def modules_snapshot():
    out = {}
    for l in pathlib.Path("/proc/modules").read_text().splitlines():
        f = l.split(); out[f[0]] = f[1]
    return out


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
    mods = modules_snapshot()
    if "ovdpair" in mods: fail.append("ovdpair already loaded on the PHYSICAL host")
    pre = {"host_uname_r": os.uname().release, "machine": os.uname().machine, "systemd_detect_virt": virt.stdout.strip(),
           "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "nproc": os.cpu_count(), "physical_module_names_before": sorted(mods)}
    (EV / "substrate-probes.json").write_text(json.dumps({**pre, "failures": fail}, indent=1))
    log(f"substrate: {json.dumps({k: v for k, v in pre.items() if k != 'physical_module_names_before'})}")
    if fail:
        log(f"FAILED CLOSED: {fail}"); sys.exit(3)
    return mods


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
    for n in ["sh", "mount", "insmod", "rmmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "touch", "true", "head", "tail", "wc", "date", "grep", "dmesg", "free", "ps", "pkill", "ip", "poweroff"]:
        (root / "bin" / n).symlink_to("busybox")
    if subprocess.run(["ldd", str(PROBE)], text=True, capture_output=True).returncode == 0:
        copy_with_libs(root, str(PROBE)); (root / "pairprobe").symlink_to(str(PROBE))
    else:
        shutil.copy2(PROBE, root / "pairprobe")
    shutil.copy2(KO, root / "mods" / "ovdpair.ko")
    (root / "runlist.sh").write_text("#!/bin/sh\n" + RUNLIST + "\n")
    os.chmod(root / "runlist.sh", 0o755)
    (root / "init").write_text("""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /tmp
mount -t tmpfs tmpfs /tmp
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
insmod /mods/ovdpair.ko || echo "GUEST_INSMOD_FAIL ovdpair"
ip link set lo up
ulimit -n 65536
echo 1 > /proc/sys/kernel/sysrq
echo "GUEST_READY"
/bin/sh /runlist.sh
echo "GUEST_DONE"
echo o > /proc/sysrq-trigger
while true; do sleep 3600; done
""")
    os.chmod(root / "init", 0o755)
    order, pins = [], {}
    for top in ["vmw_vsock_virtio_transport", "vsock_loopback"]:
        for line in sh(["modprobe", "-S", KVER, "--show-depends", top], quiet=True).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); order.append(name); pins[str(src)] = sha(src)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}", quiet=True)
    pins.update({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "pairprobe": sha(PROBE), "ovdpair.ko": sha(KO), "cloud_hypervisor": sha(CH)})
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2, sort_keys=True))
    shutil.copy2(W / "src" / "module" / "ovdpair.c", EV / "ovdpair.c.executed")
    shutil.copy2(W / "src" / "probe" / "pairprobe.c", EV / "pairprobe.c.executed")
    shutil.copy2(W / "src" / "module" / "ovd_fwd_uapi.h", EV / "ovd_fwd_uapi.h.executed")
    (EV / "runlist.executed").write_text(RUNLIST)
    log(f"image pins: {json.dumps({k: v[:12] for k, v in pins.items() if not k.startswith('/')})}")
    log(f"modules: {order}")


def main():
    mods_before = preflight()
    build_image()
    peer = None
    if KV.get("peer") == "1":
        sh("modprobe vhost_vsock", check=False, quiet=True)
        peer = subprocess.Popen([str(PROBE), "peer", str(EV)], stdout=(EV / "peer.stdout").open("w"), stderr=(EV / "peer.stderr").open("w"), start_new_session=True)
        for _ in range(50):
            time.sleep(0.1)
            if "PEER_READY" in (EV / "peer.stdout").read_text(): break
        else:
            log("FAILED: host peer not ready: " + (EV / "peer.stderr").read_text()[-500:]); sys.exit(4)
        log("host peer ready (userspace only)")
    vmdir = OUT / f"vm-{STAMP}"; vmdir.mkdir()
    serial = vmdir / "serial.log"
    cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", "console=ttyS0 panic=0 loglevel=4 log_buf_len=16M",
           "--cpus", "boot=4", "--memory", f"size={MEM_MB}M", "--serial", f"file={serial}", "--console", "off",
           "--vsock", f"cid={CID},backend=vhost-kernel", "--log-file", str(vmdir / "ch.log"), "-v"]
    log("launch VM: " + " ".join(cmd))
    proc = subprocess.Popen(cmd, stdout=(vmdir / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True)
    t0 = time.time(); status = "timeout"; last_sz = -1; last_change = time.time()
    while time.time() - t0 < TIMEOUT:
        txt = serial.read_text(errors="replace") if serial.exists() else ""
        if "GUEST_DONE" in txt: status = "done"; break
        if proc.poll() is not None: status = "vm-exited"; break
        sz = len(txt)
        if sz != last_sz: last_sz = sz; last_change = time.time()
        if time.time() - last_change > IDLE: status = "idle-no-serial-output"; break
        if re.search(r"Kernel panic|BUG:|Oops:|general protection fault", txt): status = "kernel-fault"; time.sleep(3); break
        time.sleep(1)
    wall = time.time() - t0
    log(f"VM status={status} wall={wall:.0f}s")
    time.sleep(2)
    if proc.poll() is None:
        try: os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: proc.wait(timeout=10)
        except subprocess.TimeoutExpired: pass
    if peer is not None:
        peer.send_signal(signal.SIGTERM)
        try: peer.wait(timeout=10)
        except subprocess.TimeoutExpired: peer.kill()
    d = EV / "vm"; d.mkdir(exist_ok=True)
    for f in vmdir.iterdir():
        if f.is_file(): shutil.copy2(f, d / f.name)
    txt = serial.read_text(errors="replace") if serial.exists() else ""
    results = [json.loads(l.split("RUN_RESULT ", 1)[1]) for l in txt.splitlines() if "RUN_RESULT " in l]
    probes = [l.split("PROBE_RESULT ", 1)[1] for l in txt.splitlines() if "PROBE_RESULT " in l]
    faults = [l for l in txt.splitlines() if re.search(r"Kernel panic|BUG:|Oops|WARNING:|Call Trace|general protection|unbalanced|lockdep|GUEST_INSMOD_FAIL", l)]
    (EV / "run-results.jsonl").write_text("\n".join(json.dumps(r) for r in results) + "\n")
    mods_after = modules_snapshot()
    boot = re.search(r"GUEST_BOOT uname_r=(\S+)", txt)
    summary = {"status": status, "wall_s": round(wall), "host_uname_r": os.uname().release, "guest_uname_r": boot.group(1) if boot else None,
               "physical_module_names_unchanged": sorted(mods_before) == sorted(mods_after), "physical_ovdpair_loaded": "ovdpair" in mods_after,
               "runs": len(results), "fault_lines": faults[:20], "probe_results": probes}
    (EV / "result.json").write_text(json.dumps(summary, indent=1))
    log("RESULT " + json.dumps(summary))
    for r in results:
        log(f"  run {r['run']} {r['mode']} {r['shape']} lanes={r['lanes']}x{r['trials_per_lane']}: ok {r['ok']}/{r['trials']} reasons={r['reasons']} wall={r['wall_s']}s")
    if (EV / "peer-summary.json").exists():
        log("PEER " + (EV / "peer-summary.json").read_text().strip()[:3000])
    log(f"evidence dir: {EV}")


main()
