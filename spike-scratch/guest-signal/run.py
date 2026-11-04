#!/usr/bin/env python3
"""guest-signal spike driver. Runs on the metal host as root (via `cargo xtask metal run --`), cwd = repo tree.

  run.py build
  run.py v36                       # lat (100 sessions x 1000 LISTEN + 1 concurrent EXIT each) then sat (100k+100k)
  run.py v34 <boots> <par> <drain> # <boots> real guest boots, <par> in parallel, drain-before-EXIT 0|1
"""
import sys; sys.dont_write_bytecode = True
import json, os, pathlib, shutil, signal, subprocess, threading, time, concurrent.futures as cf

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "out"; EV = OUT / "evidence-metal"
KERNEL = os.environ.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
CH = os.environ.get("GS_CH", "/usr/local/bin/cloud-hypervisor")
NIGHTLY = "nightly-2026-08-05"
ENV = dict(os.environ, PATH=f"/home/ubuntu/.cargo/bin:{os.environ.get('PATH','')}", HOME="/home/ubuntu")
GUEST_CID_BASE = 0x0100_0000  # the production adapter ignores connections from CIDs outside GuestCid::new range
CARGO_BUILD = "cargo " + "build"


def sh(cmd, check=True, **kw):
    print("+", cmd if isinstance(cmd, str) else " ".join(cmd), flush=True)
    r = subprocess.run(cmd, shell=isinstance(cmd, str), env=ENV, text=True, **kw)
    if check and r.returncode: sys.exit(f"FAILED rc={r.returncode}: {cmd}")
    return r


def preflight():
    virt = subprocess.run(["systemd-detect-virt"], text=True, capture_output=True).stdout.strip()
    ch_sha = subprocess.run(["sha256sum", CH], text=True, capture_output=True).stdout.split()[0]
    info = {"uname_r": os.uname().release, "machine": os.uname().machine, "systemd_detect_virt": virt, "kernel_file": KERNEL,
            "ch": CH, "ch_sha256": ch_sha,
            "ch_version": subprocess.run([CH, "--version"], text=True, capture_output=True).stdout.strip()}
    print("SUBSTRATE", json.dumps(info), flush=True)
    if os.uname().machine != "x86_64" or virt != "none" or not os.path.exists("/dev/kvm"):
        sys.exit("substrate preflight failed")
    return info


def build():
    OUT.mkdir(exist_ok=True)
    sh(f"cd {HERE}/bpf && rustup run {NIGHTLY} {CARGO_BUILD} --release --target bpfel-unknown-none -Z build-std=core --target-dir {HERE}/out/bpf-target")
    sh(f"cd {HERE} && RUSTFLAGS='-C target-feature=+crt-static' {CARGO_BUILD} --release --target x86_64-unknown-linux-gnu -p gsguest -p gs-work")
    sh(f"cd {HERE} && {CARGO_BUILD} --release -p gshost")
    sh(f"cd {HERE} && sha256sum out/bpf-target/bpfel-unknown-none/release/gs-bpf target/x86_64-unknown-linux-gnu/release/gsguest target/x86_64-unknown-linux-gnu/release/gswork target/release/gshost")
    build_image()


def build_image():
    root = OUT / "initramfs-root"
    if root.exists(): shutil.rmtree(root)
    for d in ["proc", "sys", "dev", "bin", "sbin", "tmp", "mods", "etc"]:
        (root / d).mkdir(parents=True, exist_ok=True)
    shutil.copy2("/usr/bin/busybox", root / "bin/busybox")
    for n in ["sh", "mount", "insmod", "cat", "echo", "uname", "sleep", "mkdir", "poweroff"]:
        (root / "bin" / n).symlink_to("busybox")
    t = HERE / "target/x86_64-unknown-linux-gnu/release"
    shutil.copy2(t / "gsguest", root / "gsguest"); shutil.copy2(t / "gswork", root / "gswork")
    (root / "init").write_text("""#!/bin/sh
export PATH=/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mount -t tmpfs tmpfs /tmp
mkdir -p /sys/fs/cgroup
mount -t cgroup2 none /sys/fs/cgroup || echo GS_CGROUP2_MOUNT_FAILED
for m in $(cat /mods/order); do insmod /mods/$m || echo "GS_INSMOD_FAIL $m"; done
exec /gsguest < /dev/console > /dev/console 2>&1
""")
    (root / "init").chmod(0o755)
    order = []
    for line in subprocess.run(["modprobe", "--show-depends", "vmw_vsock_virtio_transport"], text=True, capture_output=True).stdout.splitlines():
        if line.startswith("insmod "):
            src = pathlib.Path(line.split()[1]); name = src.name.removesuffix(".zst")
            if name in order: continue
            data = subprocess.check_output(["zstd", "-dc", str(src)]) if src.suffix == ".zst" else src.read_bytes()
            (root / "mods" / name).write_bytes(data); order.append(name)
    (root / "mods/order").write_text("\n".join(order) + "\n")
    sh(f"cd {root} && find . | cpio -o -H newc --quiet | gzip -1 > {OUT}/initramfs.cpio.gz")
    print("IMAGE built; modules:", order, flush=True)


def launch_ch(cid, name, port, drain, once, evdir, late=0):
    cmdline = f"console=ttyS0 panic=1 loglevel=3 gs.port={port} gs.drain={drain} gs.once={once} gs.late={late} rdinit=/init"
    argv = [CH, "--kernel", KERNEL, "--initramfs", str(OUT / "initramfs.cpio.gz"), "--cmdline", cmdline, "--cpus", "boot=2",
            "--memory", "size=256M", "--serial", f"file={evdir}/{name}.serial", "--console", "off",
            "--vsock", f"cid={cid},backend=vhost-kernel", "--log-file", f"{evdir}/{name}.ch.log"]
    return subprocess.Popen(argv, stdout=open(f"{evdir}/{name}.stdout", "w"), stderr=subprocess.STDOUT, start_new_session=True)


def kill(p):
    if p.poll() is None:
        try: os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError: pass
    p.wait()


def start_host(port, scenario, outfile):
    log = open(f"{outfile}.hostlog", "w")
    p = subprocess.Popen([str(HERE / "target/release/gshost"), str(port), scenario, str(outfile)], stdout=log, stderr=subprocess.STDOUT, env=ENV)
    for _ in range(100):
        if "GSHOST_LISTENING" in open(f"{outfile}.hostlog").read(): return p
        time.sleep(0.1)
    sys.exit("gshost did not start")


def pct(sorted_v, p): return sorted_v[int((len(sorted_v) - 1) * p)]


def v36():
    preflight(); stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    evdir = EV / f"v36-{stamp}"; evdir.mkdir(parents=True)
    # ---- phase 1: latency + concurrent EXIT, 100 sessions x 1000 = 100,000 LISTEN
    cid, port = GUEST_CID_BASE + 1, 1234
    host = start_host(port, "lat:100:1000", evdir / "lat.jsonl")
    ch = launch_ch(cid, "lat", port, 1, 0, evdir)
    t0 = time.time()
    try: host.wait(timeout=900)
    except subprocess.TimeoutExpired: print("LAT TIMEOUT"); host.kill()
    time.sleep(1.0); kill(ch)
    print(f"lat phase wall={time.time()-t0:.1f}s ch_serial_tail:\n" + open(f"{evdir}/lat.serial", errors="replace").read()[-1500:])
    recs = [json.loads(l) for l in open(evdir / "lat.jsonl")]
    sess = [r for r in recs if r["scenario"] == "lat"]
    allr = sorted(x for r in sess for x in r["rtts"])
    summary = {
        "sessions": len(sess), "lines_total": sum(r["lines"] for r in sess),
        "listen_ok_in_order": sum(r["listen_ok"] for r in sess), "listen_out_of_order": sum(r["listen_out_of_order"] for r in sess),
        "malformed": sum(len(r["malformed"]) for r in sess), "malformed_samples": [m for r in sess for m in r["malformed"]][:5],
        "exits_total": sum(r["exits"] for r in sess),
        "sessions_with_exactly_one_exit": sum(1 for r in sess if r["exits"] == 1),
        "listen_after_exit_total": sum(r["listen_after_exit"] for r in sess),
        "sessions_with_listen_after_exit": sum(1 for r in sess if r["listen_after_exit"] > 0),
        "exit_position_in_stream_min_max": [min(r["exit_positions"][0][0] for r in sess if r["exit_positions"]), max(r["exit_positions"][0][0] for r in sess if r["exit_positions"])],
        "today_codec_unknown_kind": sum(r["today_codec_unknown_kind"] for r in sess),
        "rtt_n": len(allr),
        "rtt_us": {k: round(pct(allr, p) / 1000, 1) for k, p in [("min", 0), ("p50", .5), ("p90", .9), ("p99", .99), ("p999", .999), ("max", 1)]} if allr else None,
    }
    print("V36_LAT_SUMMARY", json.dumps(summary), flush=True)
    print("V36_LAT_PER_SESSION_P99_US", [round(pct(sorted(r["rtts"]), .99) / 1000, 1) for r in sess if r["rtts"]], flush=True)
    # ---- phase 2: saturation integrity, two concurrent writers: unsynchronised vs one user-space lock
    for i, mode in enumerate(["free", "lock"]):
        cid2 = GUEST_CID_BASE + 2 + i
        host = start_host(port, f"sat:100000:{mode}", evdir / f"sat-{mode}.jsonl")
        ch = launch_ch(cid2, f"sat-{mode}", port, 1, 0, evdir)
        t0 = time.time()
        try: host.wait(timeout=300)
        except subprocess.TimeoutExpired: print("SAT TIMEOUT", mode); host.kill()
        time.sleep(1.0); kill(ch)
        print(f"sat-{mode} phase wall={time.time()-t0:.1f}s")
        for l in open(evdir / f"sat-{mode}.jsonl"):
            r = json.loads(l); r.pop("rtts", None); r["malformed_n"] = len(r["malformed"]); r["malformed"] = r["malformed"][:3]
            print(f"V36_SAT_{mode.upper()}", json.dumps(r), flush=True)
    print("EVDIR", evdir)


def v34(boots, par, drain, late=0):
    preflight(); stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    evdir = EV / f"v34-drain{drain}-late{late}-{stamp}"; evdir.mkdir(parents=True)
    port = 1234 + (0 if drain else 1) + (2 if late else 0)
    outf = evdir / "wl.jsonl"
    host = start_host(port, "wl", outf)
    base = GUEST_CID_BASE + (1000 if drain else 4000) + (8000 if late else 0)   # cid; tcp/udp ports derive from cid % 20000
    results = {}
    lock = threading.Lock()
    stop = threading.Event()

    def reader():
        pos = 0
        while not stop.is_set():
            try:
                with open(outf) as f:
                    f.seek(pos); data = f.read()
                    lines = data.split("\n")
                    for l in lines[:-1]:
                        if l:
                            r = json.loads(l)
                            with lock: results[r["cid"]] = r
                    pos += len(data) - len(lines[-1])
            except FileNotFoundError: pass
            time.sleep(0.05)
    threading.Thread(target=reader, daemon=True).start()

    def one(i):
        cid = base + i; name = f"b{i}"
        p = launch_ch(cid, name, port, drain, 1, evdir, late)
        t0 = time.time(); natural = False
        while time.time() - t0 < 60:
            with lock:
                done = cid in results
            if done:
                for _ in range(50):          # let the guest power off by itself for up to 5 s
                    if p.poll() is not None: natural = True; break
                    time.sleep(0.1)
                break
            if p.poll() is not None: break
            time.sleep(0.05)
        wall = time.time() - t0
        kill(p)
        with lock: r = results.get(cid)
        if r is None:
            keep = open(f"{evdir}/{name}.serial", errors="replace").read()[-1200:]
            return {"i": i, "cid": cid, "error": "no record", "wall": wall, "serial_tail": keep}
        if (r.get("tcp_reported") and r.get("udp_reported")) or late:
            for ext in ("serial", "ch.log", "stdout"):
                try: os.unlink(f"{evdir}/{name}.{ext}")
                except FileNotFoundError: pass
        return {"i": i, "cid": cid, "wall": wall, "natural_poweroff": natural}
    t_all = time.time()
    with cf.ThreadPoolExecutor(par) as ex:
        res = list(ex.map(one, range(boots)))
    time.sleep(1.0); stop.set(); host.kill()
    recs = list(results.values())
    firsts = sorted(r["t_exec_write_to_first_listen_us"] for r in recs if r["t_exec_write_to_first_listen_us"] is not None)
    walls = sorted(r["wall"] for r in res)
    summ = {
        "drain_before_exit": drain, "boots": boots, "parallel": par, "wall_s": round(time.time() - t_all, 1),
        "records": len(recs), "no_record_n": sum(1 for r in res if "error" in r), "no_record_samples": [r for r in res if "error" in r][:3],
        "tcp_reported": sum(1 for r in recs if r["tcp_reported"]), "udp_reported": sum(1 for r in recs if r["udp_reported"]),
        "missed_tcp_cids": [r["cid"] for r in recs if not r["tcp_reported"]][:20],
        "missed_udp_cids": [r["cid"] for r in recs if not r["udp_reported"]][:20],
        "any_listen_before_exec_write": sum(1 for r in recs if r["any_listen_before_exec_write"]),
        "listens_total": sum(r["listens"] for r in recs),
        "listens_per_boot_distribution": {str(k): sum(1 for r in recs if r["listens"] == k) for k in sorted({r["listens"] for r in recs})},
        "listen_after_exit_total": sum(r["listen_after_exit"] for r in recs),
        "boots_with_listen_after_exit": sum(1 for r in recs if r["listen_after_exit"] > 0),
        "both_reported_before_exit": sum(1 for r in recs if r["tcp_reported"] and r["udp_reported"] and r["listens_before_exit"] >= 2),
        "exit_status_counts": {str(k): sum(1 for r in recs if r["exit"] == k) for k in sorted({r["exit"] for r in recs if r["exit"] is not None})},
        "boots_with_preexec_zero_note": sum(1 for r in recs if any("preexec=0 " in n for n in r.get("notes", []))),
        "boots_with_preexec_nonzero_note": sum(1 for r in recs if any(("preexec=" in n and "preexec=0 " not in n) for n in r.get("notes", []))),
        "malformed_total": sum(len(r["malformed"]) for r in recs),
        "today_codec_unknown_kind_total": sum(r["today_codec_unknown_kind"] for r in recs),
        "natural_poweroff": sum(1 for r in res if r.get("natural_poweroff")),
        "boot_wall_s": {"p50": round(pct(walls, .5), 2), "p99": round(pct(walls, .99), 2), "max": round(walls[-1], 2)},
        "exec_write_to_first_listen_us": {"p50": pct(firsts, .5), "p99": pct(firsts, .99), "max": firsts[-1]} if firsts else None,
    }
    print("V34_SUMMARY", json.dumps(summ), flush=True)
    print("EVDIR", evdir)


if __name__ == "__main__":
    c = sys.argv[1]
    if c == "build": preflight(); build()
    elif c == "image": build_image()
    elif c == "v36": v36()
    elif c == "v34": v34(int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]) if len(sys.argv) > 5 else 0)
