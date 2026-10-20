#!/usr/bin/env python3
"""ev-ctl native-metal driver (root, run under the metal lease by launch.py).

Builds (if the sources changed) the spike module and the static test program, packs an initramfs, and boots one Cloud Hypervisor
guest per suite. The module is loaded INSIDE THE GUEST kernel only; nothing is loaded into the physical host kernel. The guest init
runs /ctltest <suite>, prints SUITE_DONE and powers off; this driver watches the serial log for the end, a panic or a timeout.

Usage: run.py build | run.py <suite>[+strict] ...      (suites: errno release storm ids watch unload tls misc)
"""
import sys; sys.dont_write_bytecode = True
import fcntl, hashlib, json, os, pathlib, re, shutil, signal, subprocess, time

W = pathlib.Path("/var/tmp/ev-ctl-work"); SRC = W / "src"; OUT = W / "out"
STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
KVER = "7.0.0-29-generic"          # the guest kernel image (and the module build tree): the pinned-class 7.0 kernel
GUEST_KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CH = OUT / "cloud-hypervisor"
CID = 4471
TIMEOUT = {"exec": 120, "chaos": 500, "virtio": 300, "errno": 180, "release": 420, "storm": 900, "ids": 900, "watch": 300, "unload": 180, "tls": 240, "misc": 240}
MEM = {"storm": 8192, "ids": 4096}
CPUS = {"watch": 4, "storm": 4}


def log(m):
    print(f"[{time.strftime('%H:%M:%S')}] {m}", flush=True)


def sh(cmd, check=True, timeout=900, cwd=None):
    p = subprocess.run(cmd, shell=isinstance(cmd, str), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, errors="replace", cwd=cwd)
    if check and p.returncode:
        log(f"FAILED rc={p.returncode}: {cmd}\n{p.stdout[-3000:]}")
        raise SystemExit(4)
    return p.stdout


def sha(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


def preflight(ev):
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
    names = lambda t: sorted(l.split()[0] for l in t.splitlines())
    mods = pathlib.Path("/proc/modules").read_text()
    pre = {"host_uname_r": os.uname().release, "guest_kernel_image": GUEST_KERNEL, "guest_module_build_tree": KVER, "machine": os.uname().machine,
           "systemd_detect_virt": virt.stdout.strip(), "cpu": re.search(r"model name\s*:\s*(.*)", cpu).group(1), "nproc": os.cpu_count(),
           "physical_module_names_before": names(mods), "ovd_fwd_in_physical_kernel_before": "ovd_fwd" in names(mods), "failures": fail}
    if "ovd_fwd" in names(mods): fail.append("ovd_fwd is loaded in the PHYSICAL kernel")
    (ev / "substrate-probes.json").write_text(json.dumps(pre, indent=1))
    cfg = sh(f"grep -E 'CONFIG_MODULE_FORCE_UNLOAD|CONFIG_MODULE_UNLOAD|CONFIG_MODULE_SIG|CONFIG_VSOCKETS_LOOPBACK|CONFIG_TLS=' /boot/config-{KVER}", check=False)
    (ev / "guest-kernel-config-grep.txt").write_text(cfg)
    log("guest kernel config: " + cfg.replace("\n", " | "))
    log(f"substrate: host uname -r {pre['host_uname_r']}  virt={pre['systemd_detect_virt']}  cpu={pre['cpu']}  nproc={pre['nproc']}")
    if fail:
        log(f"FAILED CLOSED: {fail}"); raise SystemExit(3)
    return names(mods)


def build(ev):
    h = hashlib.sha256()
    for f in sorted(list((SRC / "module").glob("*")) + list((SRC / "test").glob("*"))):
        h.update(f.name.encode()); h.update(f.read_bytes())
    want = h.hexdigest()
    stamp = OUT / "build.hash"
    if stamp.exists() and stamp.read_text() == want and (OUT / "ovd_fwd.ko").exists() and (OUT / "ctltest").exists():
        log("build up to date " + want[:12]); return
    log("=== build (module against " + KVER + ", static ctltest)")
    bm = OUT / "modbuild"
    shutil.rmtree(bm, ignore_errors=True); bm.mkdir(parents=True)
    for f in (SRC / "module").glob("*"): shutil.copy2(f, bm / f.name)
    o = sh(f"make -C /lib/modules/{KVER}/build M={bm} W=1 modules 2>&1 | tail -25", check=False)
    (ev / "build-module.log").write_text(o)
    log(o[-1500:])
    if not (bm / "ovd_fwd.ko").exists(): raise SystemExit(4)
    shutil.copy2(bm / "ovd_fwd.ko", OUT / "ovd_fwd.ko")
    o = sh(f"gcc -O2 -Wall -static -o {OUT}/ctltest {SRC}/test/ctltest.c -lpthread -I{SRC}/module 2>&1 | tail -30", check=False)
    (ev / "build-ctltest.log").write_text(o)
    if not (OUT / "ctltest").exists():
        log(o); raise SystemExit(4)
    if not CH.exists():
        shutil.copy2("/var/tmp/evmod2-work/out/cloud-hypervisor", CH)    # read-only use of the previous spike's pinned CH fork binary
    stamp.write_text(want)
    log("modinfo: " + sh(f"modinfo {OUT}/ovd_fwd.ko | grep -E 'vermagic|srcversion|license'", check=False).replace("\n", " | "))
    log(f"sha256 ovd_fwd.ko {sha(OUT/'ovd_fwd.ko')[:16]} ctltest {sha(OUT/'ctltest')[:16]} CH {sha(CH)[:16]}")


def image(ev):
    root = OUT / "initramfs-root"
    shutil.rmtree(root, ignore_errors=True)
    for d in ["proc", "sys", "dev", "bin", "sbin", "tmp", "mods", "etc"]: (root / d).mkdir(parents=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for n in ["sh", "mount", "insmod", "cat", "echo", "uname", "ls", "sleep", "sed", "poweroff", "mknod", "mkdir", "dmesg", "grep", "true", "ulimit", "free", "ps", "kill", "reboot", "head", "tr", "wc"]:
        (root / "bin" / n).symlink_to("busybox")
    shutil.copy2(OUT / "ctltest", root / "ctltest")
    shutil.copy2(OUT / "ovd_fwd.ko", root / "mods/ovd_fwd.ko")
    order, tls_order, pins = [], [], {}
    for top, dst in [("vmw_vsock_virtio_transport", order), ("vsock_loopback", order), ("vsock_diag", order), ("tls", tls_order), ("aesni_intel", tls_order)]:
        for line in sh(["modprobe", "-S", KVER, "--show-depends", top]).splitlines():
            if line.startswith("insmod "):
                src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
                if name in order or name in tls_order: continue
                data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
                (root / "mods" / name).write_bytes(data); dst.append(name); pins[name] = hashlib.sha256(data).hexdigest()[:16]
    (root / "mods/order").write_text("\n".join(order + tls_order) + "\n")
    (root / "init").write_text("""#!/bin/sh
export PATH=/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /tmp
mount -t tmpfs tmpfs /tmp
CMD=$(cat /proc/cmdline)
SUITE=$(echo "$CMD" | sed -n 's/.*ctl\\.suite=\\([^ ]*\\).*/\\1/p')
ARG=$(echo "$CMD" | sed -n 's/.*ctl\\.arg=\\([^ ]*\\).*/\\1/p')
MODP=$(echo "$CMD" | sed -n 's/.*ctl\\.modp=\\([^ ]*\\).*/\\1/p' | tr '+' ' ')
[ "$MODP" = none ] && MODP=''
[ "$ARG" = none ] && ARG=''

echo "GUEST_BOOT uname_r=$(uname -r) suite=$SUITE arg=$ARG modparams=$MODP"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
insmod /mods/ovd_fwd.ko $MODP || echo "GUEST_INSMOD_FAIL ovd_fwd"
echo "GUEST_MODULES:"; cat /proc/modules
ls -l /dev/ovd_fwd
ulimit -n 1048576
/ctltest $SUITE $ARG
RC=$?
echo "SUITE_DONE suite=$SUITE rc=$RC"
dmesg | grep -E 'ovd_fwd|BUG|Oops|WARNING|Call Trace|RCU|hung|lockdep|taint' | head -80
echo "GUEST_END"
poweroff -f
while true; do sleep 3600; done
""")
    (root / "init").chmod(0o755)
    initrd = OUT / "initramfs.cpio.gz"
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {initrd}")
    (ev / "artifact-pins.json").write_text(json.dumps({"guest_kernel": sha(GUEST_KERNEL), "initramfs": sha(initrd), "ctltest": sha(OUT / "ctltest"), "ovd_fwd.ko": sha(OUT / "ovd_fwd.ko"),
                                                       "cloud_hypervisor": sha(CH), "guest_modules": pins}, indent=1))
    shutil.copy2(SRC / "module/ovd_fwd.c", ev / "ovd_fwd.c.executed"); shutil.copy2(SRC / "test/ctltest.c", ev / "ctltest.c.executed")
    shutil.copy2(SRC / "module/ovd_fwd.h", ev / "ovd_fwd.h.executed")



def host_virtio(d):
    """Real host peers over the vhost-vsock device: N connections per round to the guest, a release action in the guest, observe the host side."""
    import socket
    out = []
    def wait_line(s):
        buf = b""
        while not buf.endswith(b"\n"):
            c = s.recv(1)
            if not c: raise RuntimeError("control closed")
            buf += c
        return buf.decode().strip()
    for traffic in (0, 1):
        for how in (0, 1, 2):
            N = 32
            rec = {"how": ["close", "kill-9", "exit"][how], "traffic": traffic, "N": N}
            try:
                socks = []
                for _ in range(N):
                    s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(15); s.connect((CID, 6000)); socks.append(s)
                ctl = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); ctl.settimeout(60); ctl.connect((CID, 6001))
                ctl.sendall(f"R {how} {traffic} {N}\n".encode())
                assert wait_line(ctl) == "READY"
                for s in socks: s.setblocking(False)
                if traffic:
                    chunk = b"h" * 65536
                    for s in socks:
                        sent = 0
                        while sent < (8 << 20):
                            try: sent += s.send(chunk)
                            except BlockingIOError: break
                pre_open = 0
                for s in socks:
                    try:
                        while s.recv(65536): pass
                        # b"" would mean EOF
                    except BlockingIOError: pre_open += 1
                    except OSError: pass
                rec["host_peers_open_before"] = pre_open
                ctl.sendall(b"GO\n"); t0 = time.time()
                rec["guest_report"] = wait_line(ctl)
                rec["guest_report_after_ms"] = round((time.time() - t0) * 1000, 1)
                state = ["open"] * N; lat = [None] * N
                deadline = time.time() + 6
                while time.time() < deadline and "open" in state:
                    for i, s in enumerate(socks):
                        if state[i] != "open": continue
                        try:
                            while True:
                                dd = s.recv(65536)
                                if dd == b"": state[i] = "eof"; break
                        except BlockingIOError: pass
                        except ConnectionResetError: state[i] = "reset"
                        except OSError as e: state[i] = f"err{e.errno}"
                        if state[i] != "open": lat[i] = round((time.time() - t0) * 1000, 1)
                    time.sleep(0.0005)
                rec["host_states"] = {k: state.count(k) for k in sorted(set(state))}
                ls = sorted(x for x in lat if x is not None)
                rec["host_peer_end_latency_ms"] = {"p50": ls[len(ls) // 2], "max": ls[-1], "min": ls[0]} if ls else None
                for s in socks: s.close()
                ctl.close()
            except Exception as e:
                rec["error"] = repr(e)
            log("HOSTDATA " + json.dumps(rec))
            out.append(rec)
            time.sleep(0.5)
    try:
        q = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); q.settimeout(15); q.connect((CID, 6001)); q.sendall(b"Q\n"); q.close()
    except Exception as e:
        log(f"quit failed {e!r}")
    (d / "host-virtio.json").write_text(json.dumps(out, indent=1))
    return out

def run_suite(item, ev):
    suite, _, extra = item.partition("+")
    arg = "strict" if extra == "strict" else ""
    modp = "strict_established=1" if extra == "strict" else ""
    d = ev / f"suite-{item}"; d.mkdir()
    serial = d / "serial.log"
    cpus, mem = CPUS.get(suite, 4), MEM.get(suite, 4096)
    cmdline = f"console=ttyS0 panic=0 loglevel=4 ctl.suite={suite} ctl.arg={arg} ctl.modp={modp or 'none'}"
    cmd = [str(CH), "--kernel", GUEST_KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", cmdline, "--cpus", f"boot={cpus}", "--memory", f"size={mem}M",
           "--serial", f"file={serial}", "--console", "off", "--vsock", f"cid={CID},backend=vhost-kernel", "--log-file", str(d / "ch.log"), "-v"]
    log(f"=== suite {item}: {cpus} vCPU, {mem} MiB; timeout {TIMEOUT.get(suite, 300)} s")
    (d / "cmd.txt").write_text(" ".join(cmd))
    proc = subprocess.Popen(cmd, stdout=(d / "ch.stdout").open("w"), stderr=subprocess.STDOUT, start_new_session=True)
    t0 = time.time(); verdict = "timeout"; seen = 0; host_done = suite != "virtio"
    while time.time() - t0 < TIMEOUT.get(suite, 300):
        time.sleep(1.0)
        txt = serial.read_text(errors="replace") if serial.exists() else ""
        if not host_done and "VIRTIO_READY" in txt:
            host_done = True
            time.sleep(1.0)
            host_virtio(d)
        if "GUEST_END" in txt: verdict = "done"; break
        if re.search(r"Kernel panic|BUG: |Oops: |general protection fault|kernel BUG", txt): verdict = "kernel-fault"; time.sleep(3); break
        if proc.poll() is not None: verdict = "vm-exited"; break
        lines = txt.splitlines()
        for l in lines[seen:]:
            if l.startswith(("CHECK", "DATA", "SUITE_DONE", "RESULT", "GUEST_", "DMESG")) or "ovd_fwd" in l: print("    " + l[:300], flush=True)
        seen = len(lines)
    elapsed = time.time() - t0
    try: os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError: pass
    try: proc.wait(timeout=10)
    except subprocess.TimeoutExpired: pass
    txt = serial.read_text(errors="replace") if serial.exists() else ""
    for l in txt.splitlines()[seen:]:
        if l.startswith(("CHECK", "DATA", "SUITE_DONE", "RESULT", "GUEST_", "DMESG")) or "ovd_fwd" in l: print("    " + l[:300], flush=True)
    chk = re.findall(r"^CHECK (PASS|FAIL) (.*?) :: (.*)$", txt, re.M)
    p = sum(1 for c in chk if c[0] == "PASS"); f = sum(1 for c in chk if c[0] == "FAIL")
    res = {"item": item, "verdict": verdict, "elapsed_s": round(elapsed, 1), "pass": p, "fail": f, "kernel_fault": bool(re.search(r"Kernel panic|BUG: |Oops: |general protection fault|kernel BUG", txt)),
           "suite_done": re.findall(r"SUITE_DONE.*", txt), "guest_uname": re.findall(r"GUEST_BOOT uname_r=(\S+)", txt)}
    (d / "result.json").write_text(json.dumps(res, indent=1))
    log(f"=== suite {item}: {verdict} in {elapsed:.0f}s  PASS={p} FAIL={f}")
    for c in chk:
        if c[0] == "FAIL": log(f"    FAIL {c[1]} :: {c[2][:300]}")
    return res


def main():
    args = sys.argv[1:]
    ev = W / "evidence" / f"run-{STAMP}"; ev.mkdir(parents=True)
    pre = preflight(ev)
    build(ev)
    if args == ["build"]:
        return
    image(ev)
    results = [run_suite(a, ev) for a in args]
    after = pathlib.Path("/proc/modules").read_text()
    names = sorted(l.split()[0] for l in after.splitlines())
    ok = names == sorted(pre) or set(names) - set(pre) <= {"vhost_vsock", "vhost", "vhost_iotlb", "vsock", "vmw_vsock_virtio_transport_common", "vmw_vsock_virtio_transport", "vhost_net"}
    summary = {"results": results, "ovd_fwd_in_physical_kernel_after": "ovd_fwd" in names, "physical_modules_added": sorted(set(names) - set(pre)), "physical_modules_ok": ok}
    (ev / "summary.json").write_text(json.dumps(summary, indent=1))
    log("physical kernel: ovd_fwd loaded? " + str("ovd_fwd" in names) + "; modules added since start: " + str(sorted(set(names) - set(pre))))
    leftover = sh("pgrep -af '[e]v-ctl-work/out/initramfs|[c]tltest' || true", check=False)
    log("leftover processes: " + (leftover.strip() or "none"))


main()
