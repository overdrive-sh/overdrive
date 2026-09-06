#!/usr/bin/env python3
import os
import pathlib
import selectors
import socket
import subprocess
import sys
import time

root = pathlib.Path(sys.argv[1])
kernel = pathlib.Path(sys.argv[2])
rootfs = pathlib.Path(sys.argv[3])
run_dir = root / "run"
run_dir.mkdir(parents=True, exist_ok=True)

def listener(path):
    try: path.unlink()
    except FileNotFoundError: pass
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.bind(str(path))
    os.chmod(path, 0o777)
    sock.listen(4)
    sock.settimeout(45)
    return sock

beacon_listener = listener(run_dir / "vsock_1234")
control_listener = listener(run_dir / "vsock_1235")
console = open(run_dir / "console.log", "wb")
cmd = [
    "cloud-hypervisor", "--cpus", "boot=1", "--memory", "size=134217728",
    "--kernel", str(kernel), "--cmdline", "console=ttyS0 panic=1 root=/dev/vda rw",
    "--disk", f"path={rootfs}", "--serial", f"file={run_dir / 'guest-console.log'}",
    "--console", "off", "--vsock", f"cid=3,socket={run_dir / 'vsock'}",
    "--api-socket", str(run_dir / "api"), "--seccomp", "true",
]
vmm = subprocess.Popen(cmd, stdout=console, stderr=subprocess.STDOUT)

buffers = {}

def recv_line(sock, timeout=20):
    sock.settimeout(timeout)
    data = buffers.pop(sock, b"")
    while b"\n" not in data:
        chunk = sock.recv(4096)
        if not chunk: raise RuntimeError("unexpected EOF")
        data += chunk
    line, remainder = data.split(b"\n", 1)
    if remainder:
        buffers[sock] = remainder
    return line.decode().strip()

def send(sock, line):
    sock.sendall((line + "\n").encode())

def collect(sock, wanted, timeout=20):
    results = []
    deadline = time.monotonic() + timeout
    while len(results) < wanted:
        results.append(recv_line(sock, max(0.1, deadline - time.monotonic())))
    return results

try:
    beacon, _ = beacon_listener.accept()
    ready = recv_line(beacon)
    assert ready.startswith("READY "), ready
    send(beacon, 'EXEC ["/sbin/spike-workload","service"]')
    control, _ = control_listener.accept()
    hello1 = recv_line(control)
    assert "session=1" in hello1, hello1
    print("H1 PASS beacon=ready distinct_control=session-1 vmm=alive")

    for line in ["RUN c-ok ok 2000", "RUN c-fail fail 2000", "RUN c-signal signal 2000"]:
        send(control, line)
    h2 = collect(control, 3)
    joined = "\n".join(h2)
    assert "c-ok outcome=exit:0" in joined, h2
    assert "c-fail outcome=exit:7" in joined, h2
    assert "c-signal outcome=signal:15" in joined, h2
    send(control, "STATUS after-h2")
    status = recv_line(control)
    assert "primary=alive active=0" in status, status
    print("H2 PASS correlated=3 exit0=1 exit7=1 signal15=1 primary=alive")

    h3_started = time.monotonic()
    send(control, "RUN tree fork 500")
    h3 = recv_line(control, 10)
    h3_ms = round((time.monotonic() - h3_started) * 1000)
    assert "tree outcome=timeout residual=0" in h3, h3
    send(control, "STATUS after-h3")
    assert "primary=alive active=0" in recv_line(control), "primary not alive after H3"
    assert vmm.poll() is None
    print(f"H3 PASS timeout=bounded elapsed_ms={h3_ms} process_group_residual=0 primary=alive vmm=alive")

    h4_started = time.monotonic()
    for i in range(1, 5): send(control, f"RUN load-{i} sleep 1200")
    h4 = collect(control, 4, 10)
    h4_ms = round((time.monotonic() - h4_started) * 1000)
    assert sum("OVERLOAD" in line for line in h4) == 1, h4
    assert sum("outcome=timeout" in line for line in h4) == 3, h4
    assert all("residual=0" in line for line in h4 if "RESULT" in line), h4
    print(f"H4 PASS accepted=3 overloaded=1 elapsed_ms={h4_ms} active_residual=0 provisional_capacity=3")

    send(control, "RUN ambiguous count 10000")
    time.sleep(0.25)
    disconnect_started = time.monotonic()
    buffers.pop(control, None)
    control.close()
    control2, _ = control_listener.accept()
    hello2 = recv_line(control2)
    reconnect_ms = round((time.monotonic() - disconnect_started) * 1000)
    assert "session=2" in hello2, hello2
    send(control2, "RUN verify-one check1 2000")
    check1 = recv_line(control2)
    assert "verify-one outcome=exit:0" in check1, check1
    send(control2, "RUN fresh count 5000")
    fresh = recv_line(control2, 10)
    assert "fresh outcome=exit:0" in fresh, fresh
    send(control2, "RUN verify-two check2 2000")
    check2 = recv_line(control2)
    assert "verify-two outcome=exit:0" in check2, check2
    send(control2, "STATUS after-h5")
    assert "primary=alive active=0" in recv_line(control2), "primary not alive after H5"
    print(f"H5 PASS ambiguous_count=1 replay=0 reconnect=session-2 reconnect_ms={reconnect_ms} fresh_tick_count=2 cleanup=bounded")
finally:
    if vmm.poll() is None:
        vmm.terminate()
        try: vmm.wait(timeout=5)
        except subprocess.TimeoutExpired:
            vmm.kill(); vmm.wait(timeout=5)
    console.close()
    print(f"CLEANUP vmm_exit={vmm.returncode} owned_run_dir={run_dir}")
