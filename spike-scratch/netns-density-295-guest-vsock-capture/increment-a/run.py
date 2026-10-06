#!/usr/bin/env python3
"""increment-a: build + boot + smoke of every path (one shot each). Native metal, root, lease held."""
import fcntl, json, os, pathlib, random, sys, time, traceback

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
H.EV.mkdir(exist_ok=False)
H.OUT.mkdir(exist_ok=True)
H.LOG = (H.EV / "run.log").open("a")
lock = open("/tmp/netns-density-295-guest-vsock-capture.lock", "a+")
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
log, sh, record, gx = H.log, H.sh, H.record, H.gx

log("=== preflight")
sh("uname -a; systemd-detect-virt || true; grep -m1 'model name' /proc/cpuinfo; nproc; ls -l /dev/kvm")
BEFORE = H.inventory("before")
record("preflight", "host_uname", os.uname().release)
CH, OBJ, VFWD = H.build()
INITRD = H.build_image(OBJ, VFWD)
PUB = H.public_ip()
CID = random.randint(30000, 60000)
record("preflight", "cid", CID)
vm = None
try:
    blob = H.OUT / "www" / "blob.bin"
    blob.parent.mkdir(exist_ok=True)
    if not blob.exists():
        blob.write_bytes(os.urandom(2 * 1024 * 1024))
    record("preflight", "blob_sha256", H.sha(blob))
    srv = [H.spawn("tcp7000", [str(VFWD), "tcp-server", "--bind", f"{PUB}:7000"]),
           H.spawn("tcp7001", [str(VFWD), "tcp-server", "--bind", f"{PUB}:7001", "--banner"]),
           H.spawn("tcp7002", [str(VFWD), "tcp-server", "--bind", f"{PUB}:7002", "--rst"]),
           H.spawn("udp7010", [str(VFWD), "udp-server", "--bind", f"{PUB}:7010"]),
           H.spawn("http8080", ["python3", "-m", "http.server", "8080", "--bind", PUB, "--directory", str(blob.parent)])]
    owner = H.spawn("host-owner", [str(VFWD), "host", "--cid", str(CID), "--obj", str(OBJ),
                                   "--intake", f"{PUB}:2222=22", "--intake", f"{PUB}:2710=7100", "--intake", f"{PUB}:2711=7101",
                                   "--log", str(H.OUT / f"host-owner-{H.INC.name}.log"), "--stats", str(H.OUT / "host-stats.json")])
    time.sleep(1.5)
    sh(f"cat {owner.logpath}; cat {H.OUT}/host-owner-{H.INC.name}.log | head -5", check=False)
    vm = H.VM(CH, INITRD, CID, ["--ack", "oob", "--flush", "yes", "--udp", "park"])
    ok = vm.wait(r"GUEST_READY", 90)
    record("boot", "guest_ready", ok)
    log(vm.text()[-6000:])
    if not ok:
        raise RuntimeError("guest did not become ready")
    time.sleep(1)
    rc, t = gx(CID, "uname -r; ip -br link; ip -4 addr; cat /tmp/owner.out; head -c 3000 /tmp/owner.log")
    record("d15", "guest_net", t)
    rc, t = gx(CID, f"/vfwd sockname --dst {PUB}:7000; /vfwd sockname --bind {H.W}:0 --dst {PUB}:7000")
    record("d15", "sockname", t)
    rc, t = gx(CID, f"/vfwd tcp-client --dst {PUB}:7000 --size 1048576")
    record("v2", "tcp_1mib", H.last_json(t))
    rc, t = gx(CID, f"/vfwd tcp-client --dst {PUB}:7001 --size 4096 --banner")
    record("v3", "banner_once", H.last_json(t))
    rc, t = gx(CID, f"/vfwd tcp-client --dst {PUB}:7002 --size 100 --expect-rst")
    record("v2", "rst_once", H.last_json(t))
    rc, t = gx(CID, f"curl -sS -o /tmp/blob -w '%{{http_code}} %{{size_download}} %{{local_ip}}:%{{local_port}} %{{remote_ip}}:%{{remote_port}}\\n' http://{PUB}:8080/blob.bin; sha256sum /tmp/blob")
    record("v2", "curl_host", t)
    rc, t = gx(CID, "getent hosts example.com; echo rc=$?")
    record("v5", "getent", t)
    rc, t = gx(CID, f"/vfwd udp-client --dst {PUB}:7010 --sizes 0,1,1431,59000")
    record("v5", "udp_unconnected", H.last_json(t))
    rc, t = gx(CID, "head -c 6000 /tmp/owner.log; cat /tmp/owner-stats.json")
    record("owner", "guest_log_head", t)
    o = H.sh(f"cat {H.OUT}/host-stats.json; head -c 4000 {H.OUT}/host-owner-{H.INC.name}.log", check=False)
    record("owner", "host_log_head", o)
    r = H.sh(f"timeout 20 ssh -i {H.OUT}/keys/client -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=no "
             f"-o UserKnownHostsFile=/dev/null root@{PUB} 'echo SSH_CONNECTION=$SSH_CONNECTION; uname -r'", check=False)
    record("v4", "ssh_once", r)
    r = H.sh(f"timeout 20 {VFWD} tcp-client --dst {PUB}:2711 --size 4096 --banner", check=False)
    record("v4", "inbound_banner_once", r)
except Exception:
    log("EXCEPTION " + traceback.format_exc())
    record("harness", "exception", traceback.format_exc())
finally:
    if vm:
        try:
            gx(vm.cid, "cat /tmp/owner.log | tail -c 20000; cat /tmp/sshd.log | tail -40; cat /tmp/srv7101.log | tail", timeout=20)
        except Exception:
            pass
        vm.proc.kill()
        vm.proc.wait()
        vm.collect()
    for p in list(H.PROCS):
        if p.poll() is None:
            H.stop(p)
    for f in H.OUT.glob(f"host-owner-{H.INC.name}.log"):
        H.shutil.copy2(f, H.EV / f.name)
    time.sleep(1)
    caused = [m for m in H.modules() if m not in BEFORE["modules"]]
    record("cleanup", "modules_caused", caused)
    for m in ["vhost_vsock", "vsock_diag", "vmw_vsock_virtio_transport_common", "vsock", "vhost", "vhost_iotlb"]:
        if m in caused:
            sh(f"rmmod {m}", check=False)
    AFTER = H.inventory("after")
    record("cleanup", "modules_restored", AFTER["modules"] == BEFORE["modules"])
    record("cleanup", "links_unchanged", AFTER["links"] == BEFORE["links"])
    record("cleanup", "lo_tcx_unchanged", AFTER["lo_tcx"] == BEFORE["lo_tcx"])
    record("cleanup", "bpf_prog_count", [BEFORE["bpf_prog_count"], AFTER["bpf_prog_count"]])
    log("=== done")
