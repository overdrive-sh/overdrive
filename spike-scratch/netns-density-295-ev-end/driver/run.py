#!/usr/bin/env python3
"""ev-end native-metal driver (root, under the metal lease, launched by launch.py).

Layout of one run (the physical host kernel never loads the module):
  L0  physical metal host (kernel 7.0.0-38-generic, systemd-detect-virt=none, KVM api 12)
  L1  private QEMU/KVM VM, host role, kernel 7.0.0-29-generic, nested KVM (kvm_amd nested=1): loads vhost_vsock and ovd_fwd.ko,
      runs `evend host` (the orchestrator, the remote peer R, the host context) and the Cloud Hypervisor fork
  L2  Cloud Hypervisor microVM (--vsock backend=vhost-kernel), guest kernel = the stock image, loads ovd_fwd.ko, runs `evend agent`
Usage: run.py <tag> [--hazard-host N] [--hazard-guest N] [--mem MB] [--timeout S] [--modparams-host ...] -- scenario[:N] ...
"""
import argparse, fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, sys, threading, time

W = pathlib.Path("/var/tmp/ev-end-work"); SRC = W / "src"; OUT = W / "out"
KVER = "7.0.0-29-generic"
ap = argparse.ArgumentParser()
ap.add_argument("tag"); ap.add_argument("--hazard-host", type=int, default=0); ap.add_argument("--hazard-guest", type=int, default=0)
ap.add_argument("--mem", type=int, default=3072); ap.add_argument("--timeout", type=int, default=1500); ap.add_argument("--cpus", type=int, default=4)
ap.add_argument("--build-only", action="store_true"); ap.add_argument("--wait-ms", type=int, default=4000)
ap.add_argument("scenarios", nargs="*")
args = ap.parse_args()
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EV = W / "evidence" / f"{args.tag}-{STAMP}"; EV.mkdir(parents=True, exist_ok=False)
LOG = (EV / "driver.log").open("a")
def log(m):
    l = f"[{time.strftime('%H:%M:%S')}] {m}"; print(l, flush=True); LOG.write(l + "\n"); LOG.flush()
def sh(cmd, check=True, cwd=None, timeout=900):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=cwd, timeout=timeout, errors="replace")
    with (EV / "commands.jsonl").open("a") as f: f.write(json.dumps({"cmd": cmd if isinstance(cmd, str) else " ".join(cmd), "rc": p.returncode, "out": p.stdout[-4000:]}) + "\n")
    if p.returncode and check:
        log(f"FAILED rc={p.returncode}: {cmd}\n{p.stdout[-3000:]}"); sys.exit(4)
    return p.stdout
def sha(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()

# ---- preflight (fail closed): native, non-virtualized, KVM usable ----
def preflight():
    fail = []
    if os.uname().machine != "x86_64": fail.append("not x86_64")
    v = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True)
    if v.returncode != 1 or v.stdout.strip() != "none": fail.append(f"systemd-detect-virt={v.stdout.strip()!r}")
    cpu = pathlib.Path("/proc/cpuinfo").read_text()
    if not re.search(r"^flags.*\b(vmx|svm)\b", cpu, re.M): fail.append("no vmx/svm")
    if re.search(r"^flags.*\bhypervisor\b", cpu, re.M): fail.append("hypervisor flag")
    try:
        import stat
        if not stat.S_ISCHR(os.stat("/dev/kvm").st_mode): fail.append("/dev/kvm not a chardev")
        k = os.open("/dev/kvm", os.O_RDWR); api = fcntl.ioctl(k, 0xAE00); vm = fcntl.ioctl(k, 0xAE01, 0); os.close(vm); os.close(k)
        if api != 12: fail.append(f"kvm api {api}")
    except Exception as e: fail.append(f"kvm probe {e!r}")
    nested = pathlib.Path("/sys/module/kvm_amd/parameters/nested"); nested = nested.read_text().strip() if nested.exists() else pathlib.Path("/sys/module/kvm_intel/parameters/nested").read_text().strip()
    info = {"host_uname_r": os.uname().release, "systemd_detect_virt": v.stdout.strip(), "nested": nested, "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "failures": fail}
    (EV / "substrate-probes.json").write_text(json.dumps(info, indent=1)); log(f"substrate: {json.dumps(info)}")
    if fail: log(f"FAILED CLOSED: {fail}"); sys.exit(3)
    if nested not in ("1", "Y"): log("FAILED CLOSED: nested KVM off"); sys.exit(3)
preflight()
def modsnap():
    return sorted((l.split()[0], l.split()[1]) for l in pathlib.Path("/proc/modules").read_text().splitlines())
phys_before = modsnap(); links_before = sorted(p.name for p in pathlib.Path("/sys/class/net").iterdir())
(EV / "physical-before.json").write_text(json.dumps({"modules": phys_before, "links": links_before}))
assert not any(n in ("ovd_fwd", "evmod", "shmproxy", "tls") for n, _ in phys_before), "foreign/own module already on the physical host"

# ---- build (short): module against the 7.0.0-29 build tree, harness static ----
B = OUT / "build"; shutil.rmtree(B, ignore_errors=True); (B / "module").mkdir(parents=True); (B / "harness").mkdir()
for f in ["ovd_fwd.c", "ovd_fwd.h", "Makefile"]: shutil.copy2(SRC / "module" / f, B / "module" / f)
shutil.copy2(SRC / "harness" / "evend.c", B / "harness" / "evend.c")
mb = sh(["make", "-C", f"/lib/modules/{KVER}/build", f"M={B/'module'}", "modules"])
log("module build:\n" + "\n".join(mb.splitlines()[-12:]))
if "warning" in mb.lower(): log("NOTE: compiler warnings in module build (see commands.jsonl)")
ko = B / "module" / "ovd_fwd.ko"; assert ko.exists()
cb = sh(["gcc", "-O1", "-g", "-static", "-pthread", "-Wall", "-Wno-unused-but-set-variable", "-Wno-unused-variable", "-o", str(B / "harness" / "evend"), str(B / "harness" / "evend.c")])
log("harness build:\n" + "\n".join(cb.splitlines()[-12:]))
evend = B / "harness" / "evend"
log("modinfo: " + sh(["modinfo", str(ko)]).replace("\n", " | ")[:600])
shutil.copy2(SRC / "module" / "ovd_fwd.c", EV / "ovd_fwd.c.executed"); shutil.copy2(SRC / "harness" / "evend.c", EV / "evend.c.executed")
if args.build_only: sys.exit(0)

# ---- images ----
def copy_with_libs(root, binary):
    dst = root / binary.lstrip("/"); dst.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(os.path.realpath(binary), dst)
    for line in subprocess.run(["ldd", os.path.realpath(binary)], text=True, capture_output=True).stdout.splitlines():
        m = re.search(r"(/[^ ]+) \(0x", line)
        if m:
            lib = m.group(1)
            for cand in {lib, os.path.realpath(lib)}:
                d = root / cand.lstrip("/"); d.parent.mkdir(parents=True, exist_ok=True)
                if not d.exists(): shutil.copy2(os.path.realpath(lib), d)
def add_mods(root, tops, tag):
    order = []
    for top in tops:
        for line in sh(["modprobe", "-S", KVER, "--show-depends", top], check=False).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); order.append(name)
    return order
def cpio(root, out):
    with open(out, "wb") as a:
        pr = subprocess.Popen(["find", ".", "-print0"], cwd=root, stdout=subprocess.PIPE)
        gz = subprocess.Popen(["gzip", "-1"], stdin=subprocess.PIPE, stdout=a)
        subprocess.run(["cpio", "--null", "-o", "--format=newc", "--owner=0:0", "--quiet"], cwd=root, stdin=pr.stdout, stdout=gz.stdin, check=True)
        gz.stdin.close(); gz.wait()

def mkroot(name):
    r = OUT / name; shutil.rmtree(r, ignore_errors=True)
    for d in ["proc", "sys", "dev", "bin", "sbin", "tmp", "mods", "etc", "usr/bin", "usr/sbin", "lib64", "ch"]: (r / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", r / "bin/busybox")
    for n in ["sh", "mount", "insmod", "rmmod", "cat", "echo", "uname", "ls", "sleep", "kill", "rm", "mkdir", "true", "head", "tail", "wc", "date", "grep", "dmesg", "free", "ps", "ip", "poweroff", "sync", "mknod"]:
        (r / "bin" / n).symlink_to("busybox")
    return r

# guest (L2) initramfs
g = mkroot("guest-root"); shutil.copy2(evend, g / "evend"); shutil.copy2(ko, g / "mods" / "ovd_fwd.ko")
gorder = add_mods(g, ["vmw_vsock_virtio_transport"], "guest")
(g / "init").write_text(f"""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc; mount -t sysfs sysfs /sys; mount -t devtmpfs devtmpfs /dev; mkdir -p /tmp; mount -t tmpfs tmpfs /tmp
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in {' '.join(gorder)}; do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
insmod /mods/ovd_fwd.ko hazard={args.hazard_guest} || echo "GUEST_INSMOD_FAIL ovd_fwd"
ip link set lo up
ulimit -n 65536
echo "GUEST_READY"
/evend agent > /dev/ttyS0 2>&1 &
while true; do sleep 3600; done
""")
(g / "init").chmod(0o755)
gimg = OUT / "guest.cpio.gz"; cpio(g, gimg)

# host (L1) initramfs
h = mkroot("host-root"); shutil.copy2(evend, h / "evend"); shutil.copy2(ko, h / "mods" / "ovd_fwd.ko")
CH = next(p for p in [pathlib.Path("/var/tmp/evmod2-work/out/cloud-hypervisor"), pathlib.Path("/home/ubuntu/overdrive/spike-scratch/netns-density-295-v24/out/ch-target/release/cloud-hypervisor")] if p.exists())
copy_with_libs(h, str(CH)); (h / "ch").mkdir(exist_ok=True)
shutil.copy2(os.path.realpath(str(CH)), h / "ch" / "cloud-hypervisor"); shutil.copy2(os.environ["OVERDRIVE_METAL_KERNEL"], h / "ch" / "kernel"); shutil.copy2(gimg, h / "ch" / "initramfs.cpio.gz")
horder = add_mods(h, ["vhost_vsock", "kvm_amd"], "host")
scen = " ".join(args.scenarios)
(h / "init").write_text(f"""#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc; mount -t sysfs sysfs /sys; mount -t devtmpfs devtmpfs /dev; mkdir -p /tmp; mount -t tmpfs tmpfs /tmp
echo "L1_BOOT uname_r=$(uname -r)"
for m in {' '.join(horder)}; do insmod /mods/$m || echo "L1_INSMOD_FAIL $m"; done
cat /sys/module/kvm_amd/parameters/nested; ls -la /dev/kvm /dev/vhost-vsock
insmod /mods/ovd_fwd.ko hazard={args.hazard_host} || echo "L1_INSMOD_FAIL ovd_fwd"
ls -la /dev/ovd_fwd; grep -E "ovd_fwd" /proc/modules
ip link set lo up
ulimit -n 65536
EVEND_WAIT_MS={args.wait_ms} /evend host {scen}
echo "L1_EVEND_EXIT=$?"
echo "--- L1 dmesg tail"; dmesg | tail -40
echo "--- guest serial tail"; tail -60 /tmp/guest-serial.log
echo "--- ch.log tail"; tail -5 /tmp/ch.log
cat /proc/modules | grep -E "ovd_fwd"
poweroff -f
""")
(h / "init").chmod(0o755)
himg = OUT / "host.cpio.gz"; cpio(h, himg)
(EV / "init-l1.sh").write_text((h / "init").read_text()); (EV / "init-guest.sh").write_text((g / "init").read_text())
img = OUT / "Image"; shutil.copyfile(f"/boot/vmlinuz-{KVER}", img)
assert b"zimg" not in img.read_bytes()[:4096]
pins = {"ovd_fwd.ko": sha(ko), "evend": sha(evend), "L1_kernel": sha(img), "guest_kernel": sha(os.environ["OVERDRIVE_METAL_KERNEL"]), "cloud-hypervisor": sha(CH), "host_initramfs": sha(himg), "guest_initramfs": sha(gimg)}
(EV / "artifact-pins.json").write_text(json.dumps(pins, indent=1)); log(f"pins {json.dumps({k: v[:12] for k, v in pins.items()})}")
log(f"L1 size: {himg.stat().st_size>>20} MiB")

# ---- run L1 ----
avail = int(next(l.split()[1] for l in pathlib.Path("/proc/meminfo").read_text().splitlines() if l.startswith("MemAvailable:")))
assert avail > (args.mem + 4096) * 1024, avail
q = ["qemu-system-x86_64", "-machine", "q35", "-cpu", "host", "-accel", "kvm", "-m", str(args.mem), "-smp", str(args.cpus), "-nodefaults", "-nographic", "-monitor", "none",
     "-serial", "stdio", "-no-reboot", "-kernel", str(img), "-initrd", str(himg), "-append", "console=ttyS0 rdinit=/init panic=-1 loglevel=4"]
(EV / "qemu-command.json").write_text(json.dumps(q, indent=1)); log("QEMU: " + " ".join(q))
t0 = time.time()
p = subprocess.Popen(q, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, errors="replace")
timer = threading.Timer(args.timeout, p.terminate); timer.start()
done = False
with (EV / "serial.log").open("x") as f:
    for line in p.stdout:
        f.write(line); f.flush(); print(line, end="", flush=True)
        if "HOST_DONE" in line: done = True
rc = p.wait(); timer.cancel()
log(f"QEMU exit {rc} after {time.time()-t0:.0f}s; HOST_DONE={done}")
phys_after = modsnap(); links_after = sorted(x.name for x in pathlib.Path("/sys/class/net").iterdir())
bad = [n for n, _ in phys_after if n in ("ovd_fwd", "evmod", "shmproxy", "tls")]
(EV / "physical-after.json").write_text(json.dumps({"modules": phys_after, "links": links_after, "foreign_or_own_module_entries": bad}))
names_b = sorted(n for n, _ in phys_before); names_a = sorted(n for n, _ in phys_after)
log(f"physical host module names unchanged: {names_b == names_a}; ovd_fwd/evmod/tls/shmproxy on physical host: {bad}; links unchanged: {links_before == links_after}")
(EV / "result.json").write_text(json.dumps({"qemu_exit": rc, "host_done": done, "elapsed_s": time.time() - t0, "scenarios": args.scenarios, "hazard_host": args.hazard_host, "hazard_guest": args.hazard_guest, "physical_module_names_unchanged": names_b == names_a, "module_on_physical_host": bad}, indent=1))
sys.exit(0 if done else 1)
