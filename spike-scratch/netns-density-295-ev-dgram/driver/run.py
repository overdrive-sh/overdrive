#!/usr/bin/env python3
"""ev-dgram metal driver (root, under the metal lease; launched by launch.py).

  run.py qemu <cases>         Env 1: a private QEMU/KVM VM (stock 7.0.0-29-generic), module loaded INSIDE the VM, loopback vsock,
                              `dgprobe k <cases>` as PID 1's job. Chain A -> S -> fwd1 -> vsock -> fwd2 -> U -> D plus peer cases.
  run.py ch <case,case,...>   Env 2: a Cloud Hypervisor guest (fork, --vsock backend=vhost-kernel), module loaded INSIDE the guest,
                              `dgprobe guest <case>` in the guest; `dgprobe host <case>` (userspace framed peer) on the physical host.
Never loads anything into the physical host kernel (asserted before and after). Records uname -r.
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, socket, subprocess, threading, time

W = pathlib.Path("/var/tmp/ev-dgram-work"); OUT = W / "out"; SRC = W / "src"
MODE = sys.argv[1]; ARG = sys.argv[2]; KV = dict(a.split("=", 1) for a in sys.argv[3:])
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
TAG = KV.get("tag", ARG.replace(",", "+")[:40])
EV = W / "evidence" / f"{MODE}-{TAG}-{STAMP}"
KVER = "7.0.0-29-generic"
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CID = 3372
PROBE, BPFO, KO, CH = OUT / "dgprobe", OUT / "dg-bpf.o", OUT / "mod" / "evmod.ko", OUT / "cloud-hypervisor"
MODPARAMS = KV.get("modparams", "").replace("+", " ")
EV.mkdir(parents=True, exist_ok=False)
LOG = (EV / "run.log").open("a")


def log(m):
    line = f"[{time.strftime('%H:%M:%S')}] {m}"; print(line, flush=True); LOG.write(line + "\n"); LOG.flush()


def sh(cmd, check=True, timeout=600, quiet=False):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, errors="replace")
    with (EV / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"t": time.time(), "cmd": cmd, "rc": p.returncode, "out": p.stdout[-4000:]}) + "\n")
    if not quiet:
        log(f"$ {cmd if isinstance(cmd, str) else ' '.join(cmd)} (rc={p.returncode})")
    if check and p.returncode: raise RuntimeError(f"failed rc={p.returncode}: {cmd}\n{p.stdout[-2000:]}")
    return p.stdout


def sha(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


def phys_modules():
    names = {}
    for l in pathlib.Path("/proc/modules").read_text().splitlines():
        f = l.split(); names[f[0]] = f[1]
    return names


def preflight():
    import stat
    log("=== preflight (fail closed)")
    fail = []
    if os.uname().machine != "x86_64": fail.append("not x86_64")
    virt = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True)
    if virt.returncode != 1 or virt.stdout.strip() != "none": fail.append(f"systemd-detect-virt={virt.stdout.strip()!r}")
    cpu = pathlib.Path("/proc/cpuinfo").read_text()
    if not re.search(r"^flags.*\b(vmx|svm)\b", cpu, re.M): fail.append("no vmx/svm")
    if re.search(r"^flags.*\bhypervisor\b", cpu, re.M): fail.append("hypervisor flag")
    try:
        if not stat.S_ISCHR(os.stat("/dev/kvm").st_mode): fail.append("/dev/kvm not chardev")
        k = os.open("/dev/kvm", os.O_RDWR); api = fcntl.ioctl(k, 0xAE00); v = fcntl.ioctl(k, 0xAE01, 0); os.close(v); os.close(k)
        if api != 12: fail.append(f"kvm api {api}")
    except Exception as e:
        fail.append(f"kvm probe {e!r}")
    mods = phys_modules()
    bad = [m for m in mods if m in ("evmod", "shmproxy")]
    if bad: fail.append(f"module already loaded on the physical host: {bad}")
    pre = {"host_uname_r": os.uname().release, "vm_kernel_modules_from": KVER, "machine": os.uname().machine, "systemd_detect_virt": virt.stdout.strip(),
           "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "nproc": os.cpu_count(), "meminfo_head": pathlib.Path("/proc/meminfo").read_text().splitlines()[:3],
           "loadavg": pathlib.Path("/proc/loadavg").read_text().strip(), "failures": fail, "phys_modules_before": mods}
    (EV / "substrate-probes.json").write_text(json.dumps(pre, indent=1))
    log("substrate: " + json.dumps({k: v for k, v in pre.items() if k != "phys_modules_before"}))
    if fail:
        log(f"FAILED CLOSED: {fail}"); sys.exit(3)
    return mods


def check_phys_after(before):
    after = phys_modules()
    added = sorted(set(after) - set(before)); removed = sorted(set(before) - set(after))
    forbidden = [m for m in after if m in ("evmod", "shmproxy")]
    ok = not forbidden and all(m.startswith(("vhost", "vsock", "vmw_vsock", "tun", "tap")) for m in added)
    (EV / "physical-after.json").write_text(json.dumps({"added": added, "removed": removed, "forbidden_present": forbidden, "ok": ok}, indent=1))
    log(f"physical host modules after: added={added} removed={removed} forbidden={forbidden} ok={ok}")
    return ok


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


def mod_chain(root, tops, listname):
    """decompress the stock kernel modules (and dependencies) for `tops` into root/mods in dependency order"""
    seen = {p.name for p in (root / "mods").iterdir()}
    order = []
    for top in tops:
        for line in sh(["modprobe", "-S", KVER, "--show-depends", top], quiet=True, check=False).splitlines():
            if not line.startswith("insmod "): continue
            src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
            if name in order: continue
            data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
            (root / "mods" / name).write_bytes(data); order.append(name)
    (root / "mods" / listname).write_text("\n".join(order) + "\n")
    return order


def build_root(kind, init_body):
    root = OUT / f"rootfs-{kind}"
    if root.exists(): shutil.rmtree(root)
    for d in ["proc", "sys", "dev", "bin", "sbin", "usr/bin", "usr/sbin", "tmp", "mods", "lib64", "etc"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for n in ["sh", "mount", "insmod", "rmmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "touch", "true", "head", "tail", "wc", "date", "grep",
              "dmesg", "free", "ps", "pkill", "ip", "poweroff", "sync", "seq", "tr", "sed", "cut"]:
        (root / "bin" / n).symlink_to("busybox")
    shutil.copy2(PROBE, root / "dgprobe"); (shutil.copy2(BPFO, root / "dg-bpf.o") if BPFO.exists() else None); shutil.copy2(KO, root / "mods" / "evmod.ko")
    pins = {"dgprobe": sha(PROBE), "dg-bpf.o": (sha(BPFO) if BPFO.exists() else "missing"), "evmod.ko": sha(KO), "evmod.c": sha(SRC / "module" / "evmod.c")}
    if kind == "q":
        n1 = mod_chain(root, ["vsock_loopback"], "order")
        nf = mod_chain(root, ["nft_reject_ipv4"], "order-nf")
        n2 = mod_chain(root, ["dummy", "sch_tbf"], "order-net")
        for b in ["/usr/sbin/ip", "/usr/sbin/tc"]:
            if os.path.exists(os.path.realpath(b)):
                copy_with_libs(root, b)
            else:
                log(f"WARNING: {b} missing on the host")
        log(f"modules: net chain {n2}")
        for b in ["/usr/sbin/nft", "/usr/bin/nft", "/sbin/nft"]:
            if os.path.exists(b):
                copy_with_libs(root, b); (root / "sbin").mkdir(exist_ok=True)
                if not (root / "sbin/nft").exists(): (root / "sbin/nft").symlink_to(b)
                break
        log(f"modules: vsock chain {n1}; nf chain {nf}")
    else:
        mod_chain(root, ["vmw_vsock_virtio_transport"], "order"); (root / "mods/order-nf").write_text("")
    (root / "init").write_text(init_body); os.chmod(root / "init", 0o755)
    initrd = OUT / f"initramfs-{kind}.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}", quiet=True)
    pins["initramfs"] = sha(initrd)
    (EV / "artifact-pins.json").write_text(json.dumps(pins, indent=2))
    shutil.copy2(SRC / "module" / "evmod.c", EV / "evmod.c.executed")
    shutil.copy2(SRC / "probe" / "src" / "main.rs", EV / "dgprobe-main.rs.executed")
    shutil.copy2(SRC / "bpf" / "src" / "main.rs", EV / "dg-bpf-main.rs.executed")
    log("pins: " + json.dumps({k: v[:12] for k, v in pins.items()}))
    return initrd


COMMON_INIT = r"""#!/bin/sh
export PATH=/bin:/sbin:/usr/bin:/usr/sbin
mount -t proc proc /proc; mount -t sysfs sys /sys; mount -t devtmpfs dev /dev
mkdir -p /tmp /sys/fs/cgroup /sys/fs/bpf
mount -t tmpfs tmpfs /tmp; mount -t cgroup2 none /sys/fs/cgroup; mount -t bpf bpf /sys/fs/bpf
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
for m in $(cat /mods/order-nf); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL(nf) $m"; done
for m in $(cat /mods/order-net 2>/dev/null); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL(net) $m"; done
insmod /mods/evmod.ko MODPARAMS || echo "GUEST_INSMOD_FAIL evmod"
ip link set lo up
ulimit -n 65536
grep -E 'evmod|vsock' /proc/modules
echo "GUEST_MEM $(grep -E 'MemTotal|MemAvailable|SUnreclaim' /proc/meminfo | tr -s ' ' | tr '\n' ' ')"
"""


def jsonl(text):
    out = []
    for l in text.splitlines():
        i = l.find('{"ev"')
        if i < 0: continue
        try: out.append(json.loads(l[i:]))
        except Exception: pass
    return out


def run_qemu(cases):
    init = COMMON_INIT.replace("MODPARAMS", MODPARAMS) + f"""/dgprobe k {cases}
echo "DGPROBE_EXIT=$?"
cat /proc/evmod
dmesg | grep -iE 'evmod|BUG|Oops|WARNING|panic|RIP' | tail -30
echo DG_ALL_DONE
poweroff -f
"""
    initrd = build_root("q", init)
    with (OUT / "Image").open("wb") as f:
        subprocess.run(["cat", f"/boot/vmlinuz-{KVER}"], stdout=f, check=True)
    mem = int(KV.get("mem", "8192")); smp = int(KV.get("smp", "4"))
    q = ["qemu-system-x86_64", "-machine", "q35", "-cpu", "host", "-accel", "kvm", "-m", str(mem), "-smp", str(smp), "-nodefaults", "-nographic", "-monitor", "none",
         "-serial", "stdio", "-no-reboot", "-kernel", str(OUT / "Image"), "-initrd", str(initrd), "-append", "console=ttyS0 rdinit=/init panic=-1 loglevel=4"]
    (EV / "qemu-command.json").write_text(json.dumps(q, indent=1))
    log("qemu: " + " ".join(q))
    timeout = int(KV.get("timeout", "1500"))
    p = subprocess.Popen(q, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, errors="replace", start_new_session=True)
    timer = threading.Timer(timeout, lambda: os.killpg(p.pid, signal.SIGKILL)); timer.start()
    lines = []
    with (EV / "serial.log").open("w") as f:
        for line in p.stdout:
            f.write(line); f.flush(); lines.append(line)
            if '{"ev":"case"' in line or "GUEST_" in line or "DG_" in line or "evmod" in line.lower():
                log(line.rstrip()[:700])
    rc = p.wait(); timer.cancel()
    text = "".join(lines)
    res = jsonl(text)
    cases = [r for r in res if r.get("ev") == "case"]
    summary = {"qemu_exit": rc, "done_marker": "DG_ALL_DONE" in text, "uname_r": re.search(r"uname_r=(\S+)", text).group(1) if "uname_r=" in text else None,
               "cases": [{"case": c["case"], "pass": c["pass"]} for c in cases], "all_pass": bool(cases) and all(c["pass"] for c in cases) and "DG_ALL_DONE" in text,
               "oops_lines": [l.strip() for l in lines if re.search(r"BUG:|Oops|WARNING:|Kernel panic|general protection|RIP:", l)][:10]}
    (EV / "results.jsonl").write_text("\n".join(json.dumps(r) for r in res) + "\n")
    (EV / "result.json").write_text(json.dumps(summary, indent=1))
    log("SUMMARY " + json.dumps({k: v for k, v in summary.items() if k != "cases"}))
    for c in cases: log(f"  {'PASS' if c['pass'] else 'FAIL'}  {c['case']}")
    return summary["all_pass"]


def run_ch(cases):
    cl = cases.split(",")
    seq = "\n".join(f"/dgprobe guest {c}; sleep 3" for c in cl)
    init = COMMON_INIT.replace("MODPARAMS", MODPARAMS) + f"""(while true; do echo "HB $(grep -E 'MemAvailable|SUnreclaim' /proc/meminfo | tr -s ' ' | tr '\\n' ' ')"; sleep 5; done) &
{seq}
cat /proc/evmod
dmesg | grep -iE 'evmod|BUG|Oops|WARNING|panic|RIP' | tail -30
echo DG_ALL_DONE
poweroff -f
"""
    initrd = build_root("c", init)
    vmdir = OUT / f"vm-{STAMP}"; vmdir.mkdir()
    serial = vmdir / "serial.log"
    cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(initrd), "--cmdline", "console=ttyS0 panic=-1 loglevel=4", "--cpus", f"boot={KV.get('cpus', '4')}",
           "--memory", f"size={KV.get('mem', '1024')}M", "--serial", f"file={serial}", "--console", "off", "--vsock", f"cid={CID},backend=vhost-kernel",
           "--log-file", str(vmdir / "ch.log"), "-v"]
    host_res = {}
    procs = []

    def start_host(c):
        f = (EV / f"host-{c}.log").open("w")
        p = subprocess.Popen([str(PROBE), "host", c], stdout=f, stderr=subprocess.STDOUT, start_new_session=True)
        procs.append(p); return p
    hp = start_host(cl[0])
    time.sleep(0.5)
    log("launch VM: " + " ".join(cmd))
    vm = subprocess.Popen(cmd, stdout=(vmdir / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True)
    per = int(KV.get("per_case_timeout", "150"))
    t_boot = time.time()
    for i, c in enumerate(cl):
        t = time.time()
        while hp.poll() is None and time.time() - t < per and vm.poll() is None:
            time.sleep(0.5)
        if hp.poll() is None:
            log(f"host peer for {c} timed out / VM gone; killing"); os.killpg(hp.pid, signal.SIGKILL)
        host_res[c] = jsonl((EV / f"host-{c}.log").read_text())
        log(f"host peer {c} done: " + json.dumps([r for r in host_res[c] if r.get('ev') == 'case'])[:600])
        if i + 1 < len(cl):
            hp = start_host(cl[i + 1])
    end = time.time() + 60
    while time.time() < end and "DG_ALL_DONE" not in serial.read_text(errors="replace") and vm.poll() is None:
        time.sleep(1)
    text = serial.read_text(errors="replace")
    if vm.poll() is None:
        try: os.killpg(vm.pid, signal.SIGKILL)
        except ProcessLookupError: pass
    shutil.copytree(vmdir, EV / "vm")
    gres = [r for r in jsonl(text) if r.get("ev") == "case"]
    out = []
    for c in cl:
        g = next((r for r in gres if r["case"] == c), None)
        h = next((r for r in host_res.get(c, []) if r.get("ev") == "case"), None)
        row = {"case": c, "guest": g, "host": h}
        row["pass"] = bool(g and h and g["pass"] and h["pass"])
        if c == "stall" and g and h:
            row["stall_cross_check_host_rx_ok_eq_guest_dg_ab"] = h["detail"].get("rx_ok") == g["detail"].get("dg_ab")
            row["pass"] = row["pass"] and row["stall_cross_check_host_rx_ok_eq_guest_dg_ab"]
        out.append(row)
    summary = {"uname_r_guest": re.search(r"uname_r=(\S+)", text).group(1) if "uname_r=" in text else None, "done_marker": "DG_ALL_DONE" in text,
               "oops_lines": [l.strip() for l in text.splitlines() if re.search(r"BUG:|Oops|WARNING:|Kernel panic|general protection|RIP:", l)][:10],
               "cases": [{"case": r["case"], "pass": r["pass"]} for r in out]}
    summary["all_pass"] = all(r["pass"] for r in out) and summary["done_marker"]
    (EV / "results.json").write_text(json.dumps(out, indent=1))
    (EV / "result.json").write_text(json.dumps(summary, indent=1))
    log("SUMMARY " + json.dumps(summary))
    for r in out: log(f"  {'PASS' if r['pass'] else 'FAIL'}  {r['case']}")
    return summary["all_pass"]


def main():
    before = preflight()
    for f in (PROBE, KO):
        assert f.exists(), f"missing {f} (run build.sh)"
    if MODE == "ch" and not pathlib.Path("/dev/vhost-vsock").exists():
        sh("modprobe vhost_vsock", check=False)
    try:
        ok = run_qemu(ARG) if MODE == "qemu" else run_ch(ARG)
    finally:
        for p in list(globals().get("PROCS", [])): pass
        phys_ok = check_phys_after(before)
    log(f"RESULT all_pass={ok} physical_ok={phys_ok}")
    sys.exit(0 if ok and phys_ok else 1)


main()
