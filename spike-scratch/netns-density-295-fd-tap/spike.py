#!/usr/bin/env python3
"""SPIKE: pass one down TAP fd through the production CH confinement boundary.

Throwaway code. Not production. Deleted after validation.
"""

import fcntl
import errno
import json
import os
import pathlib
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time


TUNSETIFF = 0x400454CA
TUNSETOWNER = 0x400454CC
TUNGETIFF = 0x800454D2
IFF_TAP = 0x0002
IFF_NO_PI = 0x1000
IFF_VNET_HDR = 0x4000
FD_MIN = 50
UID = 4200
GID = 4200
GUEST_BYTES = 256 * 1024 * 1024
CGROUP_MAX = GUEST_BYTES + 8 * 1024 * 1024 + GUEST_BYTES // 400


started = time.perf_counter()
tag = f"{os.getpid() & 0xffff:04x}"
tap = f"fdtap{tag}"
bridge = f"fdbr{tag}"
nft_table = f"fdguard{tag}"
run_dir = pathlib.Path(f"/run/overdrive/vm/fd-tap-{tag}")
cgroup = pathlib.Path(f"/sys/fs/cgroup/overdrive.slice/workloads.slice/fd-tap-{tag}.scope")
staging_root = pathlib.Path(os.environ.get("OVERDRIVE_METAL_ROOTFS", "/missing")).parent
work = staging_root / f"fd-tap-{tag}"
kernel_master = pathlib.Path(os.environ.get("OVERDRIVE_METAL_KERNEL", "/missing"))
rootfs_master = pathlib.Path(os.environ.get("OVERDRIVE_METAL_ROOTFS", "/missing"))
tap_fd = -1
loop = ""
packet_capture = None
main_proc = None
fail_proc = None
listener = None
conn = None
stderr_handle = None
cleanup_checks = {}
before_ch = set()


def run(argv, *, check=True, capture_output=True):
    return subprocess.run(argv, check=check, text=True, capture_output=capture_output)


def command_output(argv):
    return run(argv).stdout.strip()


def ch_pids():
    found = set()
    for entry in pathlib.Path("/proc").glob("[0-9]*"):
        try:
            argv0 = (entry / "cmdline").read_bytes().split(b"\0", 1)[0]
            if pathlib.Path(os.fsdecode(argv0)).name == "cloud-hypervisor":
                found.add(int(entry.name))
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass
    return found


def link_json(name):
    raw = command_output(["ip", "-j", "-d", "link", "show", "dev", name])
    return json.loads(raw)[0]


def iface_exists(name):
    return pathlib.Path("/sys/class/net", name).exists()


def drain_capture(sock):
    records = []
    while True:
        try:
            frame, ancillary, _, address = sock.recvmsg(65535, 256)
        except OSError as error:
            if error.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.ENETDOWN):
                break
            raise
        timestamp_ns = None
        for level, kind, data in ancillary:
            if level == socket.SOL_SOCKET and kind == 35 and len(data) >= 16:
                seconds, nanos = struct.unpack("qq", data[:16])
                timestamp_ns = seconds * 1_000_000_000 + nanos
        records.append((timestamp_ns, address, frame))
    return records


def frame_summary(record):
    timestamp_ns, address, frame = record
    if len(frame) < 14:
        return f"ts={timestamp_ns} addr={address} truncated_len={len(frame)}"
    dst = ":".join(f"{octet:02x}" for octet in frame[0:6])
    src = ":".join(f"{octet:02x}" for octet in frame[6:12])
    ether_type = int.from_bytes(frame[12:14], "big")
    return f"ts={timestamp_ns} pkttype={address[2]} len={len(frame)} src={src} dst={dst} ethertype=0x{ether_type:04x}"


def open_fds_without_cloexec():
    result = []
    for entry in pathlib.Path("/proc/self/fd").iterdir():
        try:
            fd = int(entry.name)
            flags = fcntl.fcntl(fd, fcntl.F_GETFD)
            if not flags & fcntl.FD_CLOEXEC:
                result.append(fd)
        except (FileNotFoundError, OSError, ValueError):
            pass
    return sorted(result)


def confined_command(ch_binary, kernel, rootfs, console, api, vsock):
    kvm_gid = os.stat("/dev/kvm").st_gid
    rlimit_fsize = max(rootfs.stat().st_size, GUEST_BYTES)
    net = (
        f"fd=[{tap_fd}],mac=02:00:00:95:{tag[:2]}:{tag[2:]},"
        "offload_tso=off,offload_ufo=off,offload_csum=off"
    )
    cmdline = (
        "console=ttyS0 panic=1 root=/dev/vda rw "
        "overdrive.net=10.77.239.2/24,gw=10.77.239.1,dns=10.77.239.1"
    )
    return [
        "prlimit",
        f"--fsize={rlimit_fsize}",
        "--nofile=256",
        "--",
        "setpriv",
        f"--reuid={UID}",
        f"--regid={GID}",
        f"--groups={kvm_gid}",
        "--no-new-privs",
        "--",
        ch_binary,
        "--cpus",
        "boot=1",
        "--memory",
        "size=268435456",
        "--kernel",
        str(kernel),
        "--cmdline",
        cmdline,
        "--disk",
        f"path={rootfs},image_type=raw",
        "--serial",
        f"file={console}",
        "--console",
        "off",
        "--vsock",
        f"cid=3,socket={vsock}",
        "--api-socket",
        str(api),
        "--seccomp",
        "true",
        "--landlock",
        "--net",
        net,
        "--landlock-rules",
        f"path=/sys/class/net/{tap},access=r",
        "--landlock-rules",
        f"path={run_dir},access=rw",
    ]


def shell_join(argv):
    import shlex

    return shlex.join(argv)


def wait_for_exec(pid, binary, timeout=5):
    deadline = time.perf_counter() + timeout
    wanted = pathlib.Path(binary).resolve()
    while time.perf_counter() < deadline:
        try:
            actual = pathlib.Path(os.readlink(f"/proc/{pid}/exe")).resolve()
            if actual == wanted:
                return
        except FileNotFoundError:
            pass
        time.sleep(0.01)
    raise RuntimeError("child did not exec cloud-hypervisor")


def thread_security(pid):
    observed = []
    for task in sorted(pathlib.Path(f"/proc/{pid}/task").iterdir(), key=lambda path: int(path.name)):
        status = (task / "status").read_text()
        values = {
            line.split(":", 1)[0]: line.split(":", 1)[1].strip()
            for line in status.splitlines()
            if line.startswith(("Name:", "NoNewPrivs:", "Seccomp:", "Seccomp_filters:"))
        }
        observed.append({"tid": int(task.name), **values})
    return observed


def read_line(sock, timeout=35):
    sock.settimeout(timeout)
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = sock.recv(1)
        if not chunk:
            raise RuntimeError("beacon EOF before a complete line")
        data.extend(chunk)
    return data.decode("utf-8", errors="replace").rstrip("\r\n")


try:
    if os.geteuid() != 0:
        raise RuntimeError("spike must run through cargo xtask metal run (root context)")
    if command_output(["uname", "-m"]) != "x86_64":
        raise RuntimeError("native qualification requires x86_64")
    detect_virt = run(["systemd-detect-virt"], check=False).stdout.strip()
    if detect_virt != "none":
        raise RuntimeError("native qualification requires systemd-detect-virt=none")
    if not kernel_master.is_file() or not rootfs_master.is_file():
        raise RuntimeError("configured remote guest artifacts are missing")
    ch_binary = shutil.which("cloud-hypervisor")
    if ch_binary is None:
        raise RuntimeError("cloud-hypervisor is absent")
    if not pathlib.Path("/dev/kvm").is_char_device():
        raise RuntimeError("/dev/kvm is absent")
    for path in (run_dir, cgroup, work):
        if path.exists():
            raise RuntimeError(f"refusing pre-existing exact resource: {path}")
    for name in (tap, bridge):
        if iface_exists(name):
            raise RuntimeError(f"refusing pre-existing exact link: {name}")
    if run(["nft", "list", "table", "bridge", nft_table], check=False).returncode == 0:
        raise RuntimeError(f"refusing pre-existing exact nft table: {nft_table}")

    before_ch = ch_pids()
    print("ENVIRONMENT")
    print(f"uname={command_output(['uname', '-srmo'])}")
    print(f"systemd_detect_virt={detect_virt}")
    print(f"cloud_hypervisor_path={ch_binary}")
    print(f"cloud_hypervisor_version={command_output([ch_binary, '--version'])}")
    print(f"kernel={kernel_master}")
    print(f"rootfs={rootfs_master}")
    print(f"parent_identity={command_output(['id'])}")
    print(f"before_cloud_hypervisor_pids={sorted(before_ch)}")

    work.mkdir(mode=0o710)
    run(["cp", "--reflink=auto", str(rootfs_master), str(work / "rootfs.ext4")])
    loop = command_output(["losetup", "--find", "--show", str(work / "rootfs.ext4")])
    (work / "mnt").mkdir()
    run(["mount", loop, str(work / "mnt")])
    busybox = pathlib.Path("/usr/bin/busybox")
    if not busybox.is_file():
        raise RuntimeError("host static busybox is absent")
    shutil.copy2(busybox, work / "mnt/sbin/busybox")
    os.chmod(work / "mnt/sbin/busybox", 0o755)
    run(["umount", str(work / "mnt")])
    run(["losetup", "-d", loop])
    loop = ""

    run_dir.mkdir(parents=True, mode=0o700)
    shutil.copy2(kernel_master, run_dir / "kernel")
    os.chown(run_dir / "kernel", UID, GID)
    os.chown(run_dir, UID, GID)
    os.chown(work / "rootfs.ext4", UID, GID)
    os.chmod(work, 0o710)
    os.chown(work, 0, GID)

    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    beacon_path = run_dir / "vsock_1234"
    listener.bind(str(beacon_path))
    listener.listen(1)
    os.chown(beacon_path, UID, GID)

    cgroup.mkdir()
    (cgroup / "cpu.weight").write_text("100")
    (cgroup / "memory.max").write_text(str(CGROUP_MAX))

    raw_fd = os.open("/dev/net/tun", os.O_RDWR | os.O_NONBLOCK)
    tap_fd = fcntl.fcntl(raw_fd, fcntl.F_DUPFD_CLOEXEC, FD_MIN)
    os.close(raw_fd)
    ifreq = struct.pack("16sH22x", tap.encode(), IFF_TAP | IFF_NO_PI | IFF_VNET_HDR)
    fcntl.ioctl(tap_fd, TUNSETIFF, ifreq)
    fcntl.ioctl(tap_fd, TUNSETOWNER, UID)
    fd_flags_before = fcntl.fcntl(tap_fd, fcntl.F_GETFD)
    fcntl.fcntl(tap_fd, fcntl.F_SETFD, fd_flags_before & ~fcntl.FD_CLOEXEC)
    tap_stat = os.fstat(tap_fd)
    tun_flags = struct.unpack("16sH22x", fcntl.ioctl(tap_fd, TUNGETIFF, bytes(40)))[1]

    run(["ip", "link", "add", bridge, "type", "bridge"])
    run(["ip", "addr", "add", "10.77.239.1/24", "dev", bridge])
    run(["ip", "link", "set", bridge, "up"])
    run(["ip", "link", "set", tap, "master", bridge])
    run(["ip", "link", "set", tap, "down"])
    pathlib.Path(f"/proc/sys/net/ipv6/conf/{tap}/disable_ipv6").write_text("1")
    pathlib.Path(f"/proc/sys/net/ipv4/conf/{tap}/arp_notify").write_text("0")

    run(["nft", "add", "table", "bridge", nft_table])
    run(["nft", "add", "chain", "bridge", nft_table, "input", "{ type filter hook input priority -300; policy drop; }"])
    run(["nft", "add", "rule", "bridge", nft_table, "input", "iifname", tap, "ether", "type", "arp", "counter", "accept"])
    run(["nft", "add", "rule", "bridge", nft_table, "input", "iifname", tap, "ip", "protocol", "icmp", "counter", "accept"])
    run(["nft", "add", "chain", "bridge", nft_table, "forward", "{ type filter hook forward priority -300; policy drop; }"])

    down = link_json(tap)
    print("TAP_PARENT_OPEN")
    print(f"tap={tap} fd={tap_fd} inode={tap_stat.st_ino} device={tap_stat.st_dev}")
    print(f"tap_ifindex={down['ifindex']} tap_flags={down['flags']} tap_operstate={down['operstate']}")
    print(f"tap_master={down.get('master')} tap_info={json.dumps(down.get('linkinfo', {}), sort_keys=True)}")
    print(f"tun_ifflags=0x{tun_flags:04x} single_queue={not bool(tun_flags & 0x0100)}")
    print(f"fd_flags_before=0x{fd_flags_before:x} fd_flags_after=0x{fcntl.fcntl(tap_fd, fcntl.F_GETFD):x}")
    print(f"parent_non_cloexec_fds={open_fds_without_cloexec()}")
    unexpected = [fd for fd in open_fds_without_cloexec() if fd > 2 and fd != tap_fd]
    if unexpected:
        raise RuntimeError(f"unexpected inherited parent fds: {unexpected}")
    if "UP" in down["flags"]:
        raise RuntimeError("TAP unexpectedly administratively UP before launch")

    packet_capture = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
    packet_capture.setsockopt(socket.SOL_SOCKET, 35, 1)
    packet_capture.bind((tap, 0))
    packet_capture.setblocking(False)
    print(f"capture_started_before_launch=AF_PACKET ifindex={socket.if_nametoindex(tap)} all_ethertypes=true")

    fail_cmd = confined_command(
        ch_binary,
        run_dir / "deliberately-missing-kernel",
        work / "rootfs.ext4",
        run_dir / "fail-console.log",
        run_dir / "fail-api",
        run_dir / "fail-vsock",
    )
    fail_started = time.perf_counter()
    with open(run_dir / "fail.stderr", "wb") as fail_stderr:
        fail_proc = subprocess.Popen(
            fail_cmd,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=fail_stderr,
            close_fds=True,
            pass_fds=(tap_fd,),
        )
        fail_rc = fail_proc.wait(timeout=15)
    fail_elapsed = time.perf_counter() - fail_started
    if fail_rc == 0:
        raise RuntimeError("deliberate CH start failure unexpectedly succeeded")
    if not iface_exists(tap) or "UP" in link_json(tap)["flags"]:
        raise RuntimeError("failed launch consumed parent ownership or raised TAP")
    if os.fstat(tap_fd).st_ino != tap_stat.st_ino:
        raise RuntimeError("parent TAP fd identity changed across failed launch")
    print("START_FAILURE")
    print(f"failure_cli={shell_join(fail_cmd)}")
    print(f"failure_exit={fail_rc} failure_elapsed_seconds={fail_elapsed:.6f}")
    print("failure_stderr_tail=" + " | ".join((run_dir / "fail.stderr").read_text(errors="replace").splitlines()[-8:]))
    print(f"parent_fd_survived_failure=true tap_still_down=true inode={os.fstat(tap_fd).st_ino}")

    main_cmd = confined_command(
        ch_binary,
        run_dir / "kernel",
        work / "rootfs.ext4",
        run_dir / "console.log",
        run_dir / "api",
        run_dir / "vsock",
    )
    print("MAIN_LAUNCH")
    print(f"main_cli={shell_join(main_cmd)}")
    main_started = time.perf_counter()
    stderr_handle = open(run_dir / "main.stderr", "wb")
    main_proc = subprocess.Popen(
        main_cmd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=stderr_handle,
        close_fds=True,
        pass_fds=(tap_fd,),
    )
    (cgroup / "cgroup.procs").write_text(str(main_proc.pid))
    wait_for_exec(main_proc.pid, ch_binary)
    exec_elapsed = time.perf_counter() - main_started
    proc_status = pathlib.Path(f"/proc/{main_proc.pid}/status").read_text()
    proc_cmdline = pathlib.Path(f"/proc/{main_proc.pid}/cmdline").read_bytes().replace(b"\0", b" ").decode()
    child_fd_stat = os.stat(f"/proc/{main_proc.pid}/fd/{tap_fd}")
    child_fd_link = os.readlink(f"/proc/{main_proc.pid}/fd/{tap_fd}")
    child_fdinfo = pathlib.Path(f"/proc/{main_proc.pid}/fdinfo/{tap_fd}").read_text().strip().replace("\n", "; ")
    status_lines = {
        line.split(":", 1)[0]: line.split(":", 1)[1].strip()
        for line in proc_status.splitlines()
        if line.startswith(("Uid:", "Gid:", "Groups:", "NoNewPrivs:", "Seccomp:", "Seccomp_filters:"))
    }
    if child_fd_stat.st_ino != tap_stat.st_ino:
        raise RuntimeError("child inherited fd inode differs from parent")
    print(f"exec_elapsed_seconds={exec_elapsed:.6f}")
    print(f"child_pid={main_proc.pid} child_cmdline={proc_cmdline}")
    print(f"child_status={json.dumps(status_lines, sort_keys=True)}")
    print(f"child_fd={tap_fd} child_fd_link={child_fd_link} child_fd_inode={child_fd_stat.st_ino}")
    print(f"child_fdinfo={child_fdinfo}")
    print(f"cgroup={cgroup} cgroup_procs={(cgroup / 'cgroup.procs').read_text().strip()}")
    if status_lines.get("Uid", "").split()[:2] != [str(UID), str(UID)]:
        raise RuntimeError("CH uid does not match production identity")
    if status_lines.get("Gid", "").split()[:2] != [str(GID), str(GID)]:
        raise RuntimeError("CH gid does not match production identity")
    if status_lines.get("NoNewPrivs") != "1":
        raise RuntimeError("CH leader did not inherit production no-new-privs")

    conn, _ = listener.accept()
    ready_line = read_line(conn)
    ready_elapsed = time.perf_counter() - main_started
    at_ready = link_json(tap)
    stats_at_ready = {
        name: int(pathlib.Path(f"/sys/class/net/{tap}/statistics/{name}").read_text())
        for name in ("rx_packets", "tx_packets", "rx_bytes", "tx_bytes")
    }
    if not ready_line.startswith("READY pid=1 port=1234"):
        raise RuntimeError(f"unexpected guest beacon: {ready_line}")
    if "UP" in at_ready["flags"]:
        raise RuntimeError("Cloud Hypervisor raised the TAP before READY")
    security_at_ready = thread_security(main_proc.pid)
    worker_security = [item for item in security_at_ready if item["tid"] != main_proc.pid]
    if not worker_security or any(
        item.get("NoNewPrivs") != "1" or item.get("Seccomp") != "2" for item in worker_security
    ):
        raise RuntimeError(f"CH worker-thread seccomp confinement mismatch: {worker_security}")
    print("GUEST_READY_WITH_TAP_DOWN")
    print(f"ready_line={ready_line}")
    print(f"ready_elapsed_seconds={ready_elapsed:.6f}")
    print(f"tap_at_ready_flags={at_ready['flags']} operstate={at_ready['operstate']} master={at_ready.get('master')}")
    print(f"tap_stats_at_ready={json.dumps(stats_at_ready, sort_keys=True)}")
    print(f"thread_security_at_ready={json.dumps(security_at_ready, sort_keys=True)}")

    pre_records = drain_capture(packet_capture)
    print(f"pre_activation_capture_packets={len(pre_records)} pre_activation_capture_bytes={sum(len(r[2]) for r in pre_records)}")
    if pre_records:
        raise RuntimeError("frames appeared on exact TAP before activation")
    packet_capture.close()
    packet_capture = None

    activation_wall_ns = time.time_ns()
    activation_started = time.perf_counter()
    run(["ip", "link", "set", tap, "up"])
    activation_elapsed = time.perf_counter() - activation_started
    packet_capture = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
    packet_capture.setsockopt(socket.SOL_SOCKET, 35, 1)
    packet_capture.bind((tap, 0))
    packet_capture.setblocking(False)
    active = link_json(tap)
    nft_readback = command_output(["nft", "-a", "list", "table", "bridge", nft_table])
    bridge_readback = command_output(["bridge", "-details", "link", "show", "dev", tap])
    if "UP" not in active["flags"] or active.get("master") != bridge:
        raise RuntimeError("privileged activation did not read back exact UP/master state")
    print("PARENT_ACTIVATION")
    print(f"activation_wall_ns={activation_wall_ns} activation_elapsed_seconds={activation_elapsed:.6f}")
    print(f"tap_active_flags={active['flags']} operstate={active['operstate']} master={active.get('master')}")
    print(f"bridge_readback={bridge_readback}")
    print("protection_readback_begin")
    print(nft_readback)
    print("protection_readback_end")

    inherited_fd = tap_fd
    os.close(tap_fd)
    tap_fd = -1
    child_fd_after_parent_close = os.stat(f"/proc/{main_proc.pid}/fd/{inherited_fd}")
    if child_fd_after_parent_close.st_ino != tap_stat.st_ino or not iface_exists(tap):
        raise RuntimeError("CH did not retain TAP after parent fd close")
    print("PARENT_FD_CLOSED")
    print(f"child_fd_survives=true child_fd_inode={child_fd_after_parent_close.st_ino} tap_exists=true")

    conn.sendall(b'EXEC ["/sbin/busybox","sh","-c","ping -c 1 -W 2 10.77.239.1; sleep 60"]\n')
    traffic_started = time.perf_counter()
    progress = False
    live_stats = stats_at_ready
    while time.perf_counter() - traffic_started < 8:
        if not iface_exists(tap):
            break
        live_stats = {
            name: int(pathlib.Path(f"/sys/class/net/{tap}/statistics/{name}").read_text())
            for name in ("rx_packets", "tx_packets", "rx_bytes", "tx_bytes")
        }
        if live_stats["rx_packets"] > stats_at_ready["rx_packets"] and live_stats["tx_packets"] > stats_at_ready["tx_packets"]:
            progress = True
            break
        time.sleep(0.02)
    traffic_elapsed = time.perf_counter() - traffic_started
    if not progress:
        raise RuntimeError(f"no bidirectional TAP traffic after activation: {live_stats}")
    print("POST_ACTIVATION_PROGRESS")
    print(f"traffic_elapsed_seconds={traffic_elapsed:.6f} tap_stats={json.dumps(live_stats, sort_keys=True)}")
    post_records = drain_capture(packet_capture)

    cancel_started = time.perf_counter()
    main_proc.terminate()
    try:
        main_rc = main_proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        (cgroup / "cgroup.kill").write_text("1")
        main_rc = main_proc.wait(timeout=5)
    cancel_elapsed = time.perf_counter() - cancel_started
    stderr_handle.close()
    stderr_handle = None
    pre_barrier_post_records = [record for record in post_records if record[0] is None or record[0] <= activation_wall_ns]
    print("CH_CANCELLATION_EXIT")
    print(f"main_exit={main_rc} cancellation_elapsed_seconds={cancel_elapsed:.6f}")
    print(f"post_activation_capture_packets={len(post_records)} post_activation_capture_bytes={sum(len(r[2]) for r in post_records)}")
    print(f"capture_records_at_or_before_activation={len(pre_barrier_post_records)}")
    print("post_activation_capture_decode=" + " | ".join(frame_summary(record) for record in post_records[:20]))
    print("main_stderr_tail=" + " | ".join((run_dir / "main.stderr").read_text(errors="replace").splitlines()[-12:]))
    if not post_records:
        raise RuntimeError("post-activation capture remained empty")
    if pre_barrier_post_records:
        raise RuntimeError("capture timestamp was absent or not strictly after activation")

    deadline = time.perf_counter() + 5
    while iface_exists(tap) and time.perf_counter() < deadline:
        time.sleep(0.02)
    if iface_exists(tap):
        raise RuntimeError("TAP survived after parent and CH closed all queue fds")

    print("QUEUE_IMPLICATIONS")
    print("queue_fds=1 virtio_num_queues=2 queue_pairs=1 IFF_MULTI_QUEUE=false")
    print("multi_queue_requires_one_TUNSETIFF_fd_per_queue_with_IFF_MULTI_QUEUE_and_CH_num_queues=2*fd_count")
    print(f"TOTAL_ELAPSED_SECONDS={time.perf_counter() - started:.6f}")
    print("VERDICT=WORKS")
finally:
    if conn is not None:
        conn.close()
    if listener is not None:
        listener.close()
    if packet_capture is not None:
        packet_capture.close()
    if fail_proc is not None and fail_proc.poll() is None:
        fail_proc.kill()
        fail_proc.wait()
    if main_proc is not None and main_proc.poll() is None:
        if cgroup.exists() and (cgroup / "cgroup.kill").exists():
            (cgroup / "cgroup.kill").write_text("1")
        else:
            main_proc.kill()
        main_proc.wait()
    if stderr_handle is not None:
        stderr_handle.close()
    if tap_fd >= 0:
        os.close(tap_fd)
        tap_fd = -1
    if loop:
        if run(["mountpoint", "-q", str(work / "mnt")], check=False).returncode == 0:
            run(["umount", str(work / "mnt")], check=False)
        run(["losetup", "-d", loop], check=False)
    if run(["nft", "list", "table", "bridge", nft_table], check=False).returncode == 0:
        run(["nft", "delete", "table", "bridge", nft_table], check=False)
    if iface_exists(tap):
        run(["ip", "link", "delete", tap], check=False)
    if iface_exists(bridge):
        run(["ip", "link", "delete", bridge], check=False)
    if cgroup.exists():
        if (cgroup / "cgroup.kill").exists():
            try:
                (cgroup / "cgroup.kill").write_text("1")
            except OSError:
                pass
        for _ in range(100):
            try:
                cgroup.rmdir()
                break
            except OSError:
                time.sleep(0.02)
    if run_dir.exists():
        shutil.rmtree(run_dir)
    if work.exists():
        shutil.rmtree(work)
    cleanup_checks = {
        "new_cloud_hypervisor_pids": sorted(ch_pids() - before_ch),
        "tap_exists": iface_exists(tap),
        "bridge_exists": iface_exists(bridge),
        "nft_table_exists": run(["nft", "list", "table", "bridge", nft_table], check=False).returncode == 0,
        "run_dir_exists": run_dir.exists(),
        "cgroup_exists": cgroup.exists(),
        "temp_exists": work.exists(),
    }
    print("CLEANUP")
    print(json.dumps(cleanup_checks, sort_keys=True))
    print(f"cleanup_complete={not any(bool(value) for value in cleanup_checks.values())}")
