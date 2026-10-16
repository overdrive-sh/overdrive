#!/usr/bin/env python3
"""evmod benchmark runner (derived from kernel-vs-aya-benchmark increment-c/r run.py).

Runs the SAME endpoint harness (src/main.rs, peer.rs) against three TCP proxies, each in a fresh private stock
7.0.0-29-generic QEMU/KVM host-role VM (never the physical host kernel):
  kernel = shmproxy.ko (two kernel threads per peer)      aya = tcp-bpf sockhash stream verdict      evmod = evmod.ko
Cells: --cells variant-tcp:repeat,...   Called under the canonical metal lease by launch.py.
"""
import argparse, fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, threading, time

W = pathlib.Path("/tmp/evmod-work")
SRC = W / "src"
ap = argparse.ArgumentParser()
ap.add_argument("--max", type=int, default=64)
ap.add_argument("--cells", default="evmod-tcp:0")
ap.add_argument("--tag", default="run")
ap.add_argument("--no-build", action="store_true")
ap.add_argument("--evmod-params", default="")
ap.add_argument("--command", default="", help="override the probe command line (default: /probe <variant> <proto> <max>)")
ap.add_argument("--timeout", type=int, default=3600)
ap.add_argument("--bench", default="bench", help="source dir of the harness package (bench = frozen perf harness, bench-v3 = + v3 subcommand)")
args = ap.parse_args()

STAMP = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
EVIDENCE = W / "evidence" / f"bench-{args.tag}-{STAMP}"
EVIDENCE.mkdir(parents=True, exist_ok=False)
OUT = W / "out" / ("bench" if args.bench == "bench" else args.bench)
OUT.mkdir(parents=True, exist_ok=True)
lease = open("/tmp/netns-density-295-kernel-vs-aya-nested-qemu.lock", "a+")
fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)


def capture(cmd, **kw):
    start = time.perf_counter()
    p = subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kw)
    with (EVIDENCE / "commands.jsonl").open("a") as f:
        f.write(json.dumps({"argv": list(map(str, cmd)), "exit": p.returncode, "wall_s": time.perf_counter() - start, "output": p.stdout[-4000:]}) + "\n")
    if p.returncode:
        print(p.stdout)
        raise RuntimeError(f"{cmd} exited {p.returncode}")
    return p.stdout


def sha(p):
    return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


kernel = capture(["uname", "-r"]).strip()
assert kernel == "7.0.0-29-generic", kernel
physical = {"modules": pathlib.Path("/proc/modules").read_text(), "links": sorted(p.name for p in pathlib.Path("/sys/class/net").iterdir())}
(EVIDENCE / "physical-before.json").write_text(json.dumps(physical, indent=2))
(EVIDENCE / "host.json").write_text(json.dumps({"host_uname_r": kernel, "lscpu": capture(["lscpu"])[:3000], "args": vars(args)}, indent=2))

env = os.environ.copy()
env["CARGO_HOME"] = str(W / "out" / "cargo-home-bench")
if not (W / "out" / "cargo-home-bench").exists():
    shutil.copytree("/home/ubuntu/overdrive/spike-scratch/netns-density-295-aya-vsock-proxy/out/cargo-home", W / "out" / "cargo-home-bench")
if not args.no_build:
    e = env.copy()
    e["CARGO_TARGET_DIR"] = str(OUT / "target")
    capture(["cargo", "build", "--release", "--locked", "--offline", "--manifest-path", str(SRC / args.bench / "Cargo.toml")], env=e)
    e = env.copy()
    e["CARGO_TARGET_DIR"] = str(OUT / "tcp-target")
    e["RUSTFLAGS"] = "-C linker=bpf-linker"
    capture(["cargo", "+nightly", "build", "--release", "--locked", "--offline", "--target", "bpfel-unknown-none", "-Z", "build-std=core",
             "--manifest-path", str(SRC / args.bench / "tcp-bpf" / "Cargo.toml")], env=e)
    for name, d in [("shmproxy", SRC / "bench" / "shmproxy"), ("evmod", SRC / "module")]:
        dst = OUT / f"mod-{name}"
        if dst.exists():
            shutil.rmtree(dst)
        shutil.copytree(d, dst)
        capture(["make", "-C", f"/lib/modules/{kernel}/build", f"M={dst}", "modules", "-j4"])
binary = OUT / "target" / "release" / "aya-vsock-probe"
tcp_o = OUT / "tcp-target" / "bpfel-unknown-none" / "release" / "aya-vsock-bpf"
ko = {"kernel": OUT / "mod-shmproxy" / "shmproxy.ko", "evmod": OUT / "mod-evmod" / "evmod.ko"}

root = OUT / "rootfs"
if root.exists():
    shutil.rmtree(root)
root.mkdir()
for name in ["proc", "sys", "dev", "tmp", "bin", "lib", "sbin"]:
    (root / name).mkdir()
shutil.copy2("/bin/busybox", root / "bin/busybox")
for name in ["sh", "mount", "insmod", "rmmod", "cat", "echo", "uname", "dmesg", "poweroff", "sha256sum", "ls", "sleep", "ip", "kill", "date"]:
    (root / "bin" / name).symlink_to("busybox")
shutil.copy2(binary, root / "probe")
shutil.copy2(tcp_o, root / "tcp.o")
shutil.copy2(ko["kernel"], root / "shmproxy.ko")
shutil.copy2(ko["evmod"], root / "evmod.ko")
perf = pathlib.Path("/usr/bin/perf")
real_perf = next((p for p in list(pathlib.Path("/usr/lib/linux-tools").glob("*/perf")) if p.is_file() and p.read_bytes()[:4] == b"\x7fELF"), None)
if real_perf is None and perf.is_file() and perf.read_bytes()[:4] == b"\x7fELF":
    real_perf = perf
if real_perf:
    shutil.copy2(real_perf, root / "perf")
for p in [binary] + ([real_perf] if real_perf else []):
    for dep in re.findall(r"(/[^\s()]+)", capture(["ldd", str(p)])):
        dest = root / dep.lstrip("/")
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.exists():
            shutil.copy2(dep, dest)
insmods, pins = [], {}
for line in capture(["modprobe", "--show-depends", "vhost_vsock"]).splitlines():
    if not line.startswith("insmod "):
        continue
    src = pathlib.Path(line.split()[1])
    dest = root / src.name.removesuffix(".zst")
    data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
    dest.write_bytes(data)
    insmods.append("insmod /" + dest.name + " || poweroff -f")
    pins[str(src)] = sha(src)
with (OUT / "Image").open("wb") as f:
    subprocess.run(["cat", f"/boot/vmlinuz-{kernel}"], stdout=f, check=True)
assert b"zimg" not in (OUT / "Image").read_bytes()[:4096]
pins.update({p.name: sha(p) for p in [OUT / "Image", binary, root / "shmproxy.ko", root / "evmod.ko", root / "tcp.o"]})
(EVIDENCE / "artifact-pins.json").write_text(json.dumps(pins, indent=2))
shutil.copy2(SRC / "module" / "evmod.c", EVIDENCE / "evmod.c.executed")
shutil.copy2(SRC / args.bench / "src" / "main.rs", EVIDENCE / "main.rs.executed")
shutil.copy2(SRC / args.bench / "src" / "peer.rs", EVIDENCE / "peer.rs.executed")

all_pass = True
for index, cell in enumerate(args.cells.split(",")):
    head, _, rep = cell.partition(":")
    variant, protocol = head.split("-")
    repeat = int(rep or 0)
    runid = f"{index:02d}-{variant}-{protocol}-r{repeat}"
    ev = EVIDENCE / runid
    ev.mkdir()
    vo = OUT / runid
    if vo.exists():
        shutil.rmtree(vo)
    vo.mkdir()
    avail = int(next(l.split()[1] for l in pathlib.Path("/proc/meminfo").read_text().splitlines() if l.startswith("MemAvailable:")))
    assert avail > (16 + 8) * 1024 * 1024, avail
    (ev / "host-admission-budget.json").write_text(json.dumps({"host_memavailable_kib": avail, "private_memory_mib": 16384, "online_vcpus": 4, "possible_vcpus": 64,
                                                                 "physical_loadavg": pathlib.Path("/proc/loadavg").read_text()}))
    modload = ""
    if variant == "kernel":
        modload = "insmod /shmproxy.ko || poweroff -f\n"
    elif variant == "evmod":
        modload = f"insmod /evmod.ko {args.evmod_params} || poweroff -f\n"
    cmdline = args.command or f"/probe {variant} {protocol} {args.max}"
    unload = {"kernel": "if [ -e /proc/shmproxy ]; then rmmod shmproxy; fi\n", "evmod": "if [ -e /proc/evmod ]; then rmmod evmod; fi\n"}.get(variant, "")
    init = ("#!/bin/sh\nmount -t proc proc /proc\nmount -t sysfs sys /sys\nmount -t devtmpfs dev /dev\nip link set lo up\nuname -a\n"
            "cat /proc/sys/kernel/pid_max /proc/sys/kernel/threads-max\ncat /sys/devices/system/cpu/online /sys/devices/system/cpu/possible\n"
            + "\n".join(insmods) + "\n"
            "echo BENCH_PRE_VARIANT_MEMINFO\ncat /proc/meminfo\necho BENCH_PRE_VARIANT_SLABINFO\ncat /proc/slabinfo\necho BENCH_PRE_VARIANT_END\n"
            + modload +
            "echo BENCH_POST_LOAD_MEMINFO\ncat /proc/meminfo\necho BENCH_POST_LOAD_END\n"
            "echo BENCH_PERF_PMU_PROBE\n/perf stat -a -e cycles,instructions -- sleep 0.1 2>&1\n"
            "echo BENCH_PERF_SOFTWARE_PROBE\n/perf stat -a -e cpu-clock,task-clock,context-switches,cpu-migrations -- sleep 0.1 2>&1\n"
            f"BENCH_REPEAT={repeat} {cmdline}\nrc=$?\necho BENCH_HARNESS_EXIT=$rc\ncat /proc/modules\n"
            + unload +
            "echo BENCH_MODULE_UNLOAD_EXIT=$?\necho BENCH_LIVE_MODULES_AFTER_UNLOAD\ncat /proc/modules\ndmesg | tail -60\necho BENCH_COMPLETE_EXIT=$rc\npoweroff -f\n")
    (root / "init").write_text(init)
    (root / "init").chmod(0o755)
    (ev / "init.executed").write_text(init)
    with (vo / "initramfs").open("wb") as archive:
        pr = subprocess.Popen(["find", ".", "-print0"], cwd=root, stdout=subprocess.PIPE)
        subprocess.run(["cpio", "--null", "-o", "--format=newc", "--owner=0:0"], cwd=root, stdin=pr.stdout, stdout=archive, check=True)
        assert pr.wait() == 0
    q = ["qemu-system-x86_64", "-machine", "q35", "-cpu", "host", "-accel", "kvm", "-m", "16384", "-smp", "4,maxcpus=64", "-nodefaults", "-nographic", "-monitor", "none",
         "-serial", "stdio", "-no-reboot", "-kernel", str(OUT / "Image"), "-initrd", str(vo / "initramfs"), "-append", "console=ttyS0 rdinit=/init panic=-1"]
    (ev / "qemu-command.json").write_text(json.dumps(q, indent=2))
    started = time.perf_counter()
    p = subprocess.Popen(q, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, errors="replace")
    timer = threading.Timer(args.timeout, p.terminate)
    timer.start()
    serial = []
    with (ev / "native-serial.log").open("x") as f:
        for line in p.stdout:
            f.write(line)
            f.flush()
            serial.append(line)
            if not line.startswith("{"):
                print(line, end="", flush=True)
            else:
                try:
                    v = json.loads(line)
                    print(json.dumps({k: v[k] for k in ["event", "id", "population", "duration_s", "delivered_roundtrips", "confirmed_zero", "usage"] if k in v}), flush=True)
                except json.JSONDecodeError:
                    print(line[:200], flush=True)
    rc = p.wait()
    timer.cancel()
    good = rc == 0 and any("BENCH_COMPLETE_EXIT=0" in x for x in serial)
    (ev / "native-result.json").write_text(json.dumps({"qemu_exit": rc, "passed": good, "elapsed_s": time.perf_counter() - started, "variant": variant, "protocol": protocol,
                                                       "max_population": args.max, "repeat": repeat, "private_kernel_only": True}, indent=2))
    all_pass &= good
    if not good:
        break
after = {"modules": pathlib.Path("/proc/modules").read_text(), "links": sorted(p.name for p in pathlib.Path("/sys/class/net").iterdir())}
(EVIDENCE / "physical-after.json").write_text(json.dumps(after, indent=2))
assert after["modules"] == physical["modules"] and after["links"] == physical["links"], "physical host modules/links changed"
(EVIDENCE / "result.json").write_text(json.dumps({"all_pass": all_pass, "cells": args.cells, "maximum": args.max, "physical_modules_links_preserved": True}, indent=2))
fcntl.flock(lease, fcntl.LOCK_UN)
lease.close()
raise SystemExit(0 if all_pass else 1)
