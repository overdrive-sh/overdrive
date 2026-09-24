#!/usr/bin/env python3
"""Native probe: launcher-installed seccomp deny-list around CH v53 fd= TAP.

Throwaway spike code. It is deliberately isolated from production crates.
The full VM path is adapted from increment-y's persistent-TAP fd-handoff
harness. Run only through `cargo xtask metal run --`.
"""

import argparse
import ctypes
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import shlex
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time


# Linux tun ioctls/flags (include/uapi/linux/if_tun.h, x86_64 ABI).
TUNSETDEBUG = 0x400454C9
TUNSETIFF = 0x400454CA
TUNSETPERSIST = 0x400454CB
TUNSETOWNER = 0x400454CC
TUNSETLINK = 0x400454CD
TUNSETGROUP = 0x400454CE
TUNSETOFFLOAD = 0x400454D0
TUNSETTXFILTER = 0x400454D1
TUNGETIFF = 0x800454D2
TUNATTACHFILTER = 0x401054D5
TUNDETACHFILTER = 0x401054D6
TUNSETVNETHDRSZ = 0x400454D8
TUNSETQUEUE = 0x400454D9
TUNSETSTEERINGEBPF = 0x800454E0
TUNSETFILTEREBPF = 0x800454E1
TUNSETCARRIER = 0x400454E2
SIOCSIFHWADDR = 0x8924

IFF_TAP = 0x0002
IFF_ATTACH_QUEUE = 0x0200
IFF_PERSIST = 0x0800
IFF_NO_PI = 0x1000
IFF_VNET_HDR = 0x4000
ARPHRD_ETHER = 1
SO_TIMESTAMPNS = 35

DENIED_IOCTL_ITEMS = (
    ("SIOCSIFHWADDR", SIOCSIFHWADDR),
    ("TUNSETOWNER", TUNSETOWNER),
    ("TUNSETGROUP", TUNSETGROUP),
    ("TUNSETPERSIST", TUNSETPERSIST),
    ("TUNSETCARRIER", TUNSETCARRIER),
    ("TUNSETDEBUG", TUNSETDEBUG),
    ("TUNSETLINK", TUNSETLINK),
    ("TUNSETTXFILTER", TUNSETTXFILTER),
    ("TUNATTACHFILTER", TUNATTACHFILTER),
    ("TUNDETACHFILTER", TUNDETACHFILTER),
    ("TUNSETSTEERINGEBPF", TUNSETSTEERINGEBPF),
    ("TUNSETFILTEREBPF", TUNSETFILTEREBPF),
    ("TUNSETQUEUE", TUNSETQUEUE),
)

FD_PATH_IOCTL_ITEMS = (
    ("TUNGETIFF", TUNGETIFF, "net_util/src/tap.rs:255-257"),
    ("TUNSETIFF", TUNSETIFF, "net_util/src/tap.rs:268-281; EEXIST accepted"),
    ("TUNSETVNETHDRSZ", TUNSETVNETHDRSZ, "net_util/src/tap.rs:284-286,478-482"),
    ("SIOCGIFMTU", 0x8921, "virtio-devices/src/net.rs:546-553 -> net_util/src/tap.rs:413-425"),
    ("SIOCSIFMTU", 0x8922, "conditional on fd=...,mtu=; virtio-devices/src/net.rs:732-734 -> net_util/src/tap.rs:434-442"),
    ("TUNSETOFFLOAD", TUNSETOFFLOAD, "virtio-devices/src/net.rs:959-965"),
)

UID = 4200
GID = 4200
FD_MIN = 50
GUEST_BYTES = 256 * 1024 * 1024
CGROUP_MAX = GUEST_BYTES + 8 * 1024 * 1024 + GUEST_BYTES // 400
CH_V53_COMMIT = "9ed824d6d08df3e96f7d5f50795d9449ac99f431"


class SockFilter(ctypes.Structure):
    _fields_ = [
        ("code", ctypes.c_ushort),
        ("jt", ctypes.c_ubyte),
        ("jf", ctypes.c_ubyte),
        ("k", ctypes.c_uint32),
    ]


class SockFprog(ctypes.Structure):
    _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.POINTER(SockFilter))]


def install_ioctl_deny_filter():
    """Install an x86_64 classic-BPF seccomp filter returning EPERM by request."""
    bpf_ld_w_abs = 0x20
    bpf_jmp_jeq_k = 0x15
    bpf_ret_k = 0x06
    audit_arch_x86_64 = 0xC000003E
    sys_ioctl = 16
    seccomp_ret_kill_process = 0x80000000
    seccomp_ret_errno_eperm = 0x00050000 | errno.EPERM
    seccomp_ret_allow = 0x7FFF0000
    pr_set_no_new_privs = 38
    seccomp_set_mode_filter = 1
    sys_seccomp = 317

    denied = [request for _, request in DENIED_IOCTL_ITEMS]
    instructions = [
        SockFilter(bpf_ld_w_abs, 0, 0, 4),
        SockFilter(bpf_jmp_jeq_k, 1, 0, audit_arch_x86_64),
        SockFilter(bpf_ret_k, 0, 0, seccomp_ret_kill_process),
        SockFilter(bpf_ld_w_abs, 0, 0, 0),
        SockFilter(bpf_jmp_jeq_k, 0, len(denied) + 1, sys_ioctl),
        SockFilter(bpf_ld_w_abs, 0, 0, 24),  # seccomp_data.args[1], low dword
    ]
    for index, request in enumerate(denied):
        instructions.append(
            SockFilter(bpf_jmp_jeq_k, len(denied) - index, 0, request)
        )
    instructions.extend(
        [
            SockFilter(bpf_ret_k, 0, 0, seccomp_ret_allow),
            SockFilter(bpf_ret_k, 0, 0, seccomp_ret_errno_eperm),
        ]
    )
    array = (SockFilter * len(instructions))(*instructions)
    program = SockFprog(len(instructions), array)
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(pr_set_no_new_privs, 1, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    if libc.syscall(sys_seccomp, seccomp_set_mode_filter, 0, ctypes.byref(program)) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def ioctl_attempts(fd, tap_name):
    target_mac = bytes.fromhex("020000aa0001")
    ifreq_mac = struct.pack("16sH6s16x", tap_name.encode(), ARPHRD_ETHER, target_mac)
    ifreq_queue = struct.pack("16sH22x", tap_name.encode(), IFF_ATTACH_QUEUE)
    int_zero = struct.pack("i", 0)
    int_neg_one = struct.pack("i", -1)
    sock_fprog_empty = bytes(16)
    tx_filter_empty = bytes(256)
    calls = {
        "SIOCSIFHWADDR": lambda: fcntl.ioctl(fd, SIOCSIFHWADDR, ifreq_mac),
        "TUNSETOWNER": lambda: fcntl.ioctl(fd, TUNSETOWNER, UID),
        "TUNSETGROUP": lambda: fcntl.ioctl(fd, TUNSETGROUP, GID),
        "TUNSETPERSIST": lambda: fcntl.ioctl(fd, TUNSETPERSIST, 1),
        "TUNSETCARRIER": lambda: fcntl.ioctl(fd, TUNSETCARRIER, int_zero),
        "TUNSETDEBUG": lambda: fcntl.ioctl(fd, TUNSETDEBUG, 1),
        "TUNSETLINK": lambda: fcntl.ioctl(fd, TUNSETLINK, ARPHRD_ETHER),
        "TUNSETTXFILTER": lambda: fcntl.ioctl(fd, TUNSETTXFILTER, tx_filter_empty),
        "TUNATTACHFILTER": lambda: fcntl.ioctl(fd, TUNATTACHFILTER, sock_fprog_empty),
        "TUNDETACHFILTER": lambda: fcntl.ioctl(fd, TUNDETACHFILTER, sock_fprog_empty),
        "TUNSETSTEERINGEBPF": lambda: fcntl.ioctl(fd, TUNSETSTEERINGEBPF, int_neg_one),
        "TUNSETFILTEREBPF": lambda: fcntl.ioctl(fd, TUNSETFILTEREBPF, int_neg_one),
        "TUNSETQUEUE": lambda: fcntl.ioctl(fd, TUNSETQUEUE, ifreq_queue),
    }
    results = {}
    for name, _ in DENIED_IOCTL_ITEMS:
        try:
            calls[name]()
            results[name] = {"rc": 0, "errno": 0, "errname": "OK"}
        except OSError as error:
            results[name] = {
                "rc": -1,
                "errno": error.errno,
                "errname": errno.errorcode.get(error.errno, "?"),
            }
    return results


def ioctl_child(mode, fd, tap_name):
    wanted_threads = 4 if mode == "filtered" else 1
    lock = threading.Lock()
    rows = []

    def worker(label):
        row = {
            "label": label,
            "tid": threading.get_native_id(),
            "status_before": {
                line.split(":", 1)[0]: line.split(":", 1)[1].strip()
                for line in pathlib.Path("/proc/thread-self/status").read_text().splitlines()
                if line.startswith(("Name:", "NoNewPrivs:", "Seccomp:", "Seccomp_filters:"))
            },
            "ioctls": ioctl_attempts(fd, tap_name),
        }
        with lock:
            rows.append(row)

    worker("main")
    workers = [threading.Thread(target=worker, args=(f"worker-{i}",)) for i in range(wanted_threads - 1)]
    for thread in workers:
        thread.start()
    for thread in workers:
        thread.join()
    rows.sort(key=lambda row: row["label"])
    print("IOCTL_CHILD " + json.dumps(rows, sort_keys=True), flush=True)
    if mode == "filtered":
        ok = all(
            result["errno"] == errno.EPERM
            for row in rows
            for result in row["ioctls"].values()
        )
        ok = ok and all(
            row["status_before"].get("Seccomp") == "2"
            and int(row["status_before"].get("Seccomp_filters", "0")) >= 1
            for row in rows
        )
    else:
        ok = all(
            result["errno"] != errno.EPERM
            for row in rows
            for result in row["ioctls"].values()
        )
        ok = ok and rows[0]["status_before"].get("Seccomp") == "0"
    print(f"IOCTL_CHILD_RESULT mode={mode} pass={ok}", flush=True)
    return 0 if ok else 1


parser = argparse.ArgumentParser()
parser.add_argument("--mode", choices=("control", "filtered"))
parser.add_argument("--ioctl-child", action="store_true")
parser.add_argument("--fd", type=int)
parser.add_argument("--tap")
args = parser.parse_args()
if args.ioctl_child:
    sys.exit(ioctl_child(args.mode, args.fd, args.tap))
if args.mode is None:
    parser.error("--mode is required")


MODE = args.mode
started = time.perf_counter()
tag = f"{os.getpid() & 0xffff:04x}"
tap = f"aatap{tag}"
scratch = f"aascr{tag}"
bridge = f"aabr{tag}"
nft_table = f"aaguard{tag}"
run_dir = pathlib.Path(f"/run/overdrive/vm/aa-seccomp-{tag}")
cgroup = pathlib.Path(f"/sys/fs/cgroup/overdrive.slice/workloads.slice/aa-seccomp-{tag}.scope")
staging_root = pathlib.Path(os.environ.get("OVERDRIVE_METAL_ROOTFS", "/missing")).parent
work = staging_root / f"aa-seccomp-{tag}"
kernel_master = pathlib.Path(os.environ.get("OVERDRIVE_METAL_KERNEL", "/missing"))
rootfs_master = pathlib.Path(os.environ.get("OVERDRIVE_METAL_ROOTFS", "/missing"))
guest_mac = f"02:00:00:aa:{tag[:2]}:{tag[2:]}"

open_fds = set()
loop = ""
listener = None
conn = None
capture = None
main_proc = None
stderr_handle = None
before_ch = set()
current_step = "PREFLIGHT"
step_results = {}


class ProbeFailure(Exception):
    pass


def emit(phase, message):
    print(f"[+{time.perf_counter() - started:9.6f}] {phase} {message}", flush=True)


def check(condition, message):
    if not condition:
        raise ProbeFailure(f"{current_step}: {message}")


def begin(step, hypothesis, prediction, falsification):
    global current_step
    current_step = step
    emit(step, "BEGIN")
    emit(step, "hypothesis=" + hypothesis)
    emit(step, "prediction=" + prediction)
    emit(step, "falsification=" + falsification)


def passed(step, message):
    step_results[step] = "PASS " + message
    emit(step, "RESULT=PASS " + message)


def run(argv, check_rc=True):
    return subprocess.run(argv, check=check_rc, text=True, capture_output=True)


def output(argv):
    return run(argv).stdout.strip()


def iface_exists(name):
    return pathlib.Path("/sys/class/net", name).exists()


def ch_pids():
    found = set()
    for entry in pathlib.Path("/proc").glob("[0-9]*"):
        try:
            argv0 = (entry / "cmdline").read_bytes().split(bytes(1), 1)[0]
            if pathlib.Path(os.fsdecode(argv0)).name == "cloud-hypervisor":
                found.add(int(entry.name))
        except OSError:
            pass
    return found


def tun_holders(name):
    holders = []
    for fdinfo in pathlib.Path("/proc").glob("[0-9]*/fdinfo/*"):
        try:
            text = fdinfo.read_text()
        except OSError:
            continue
        if f"iff:\t{name}" in text or f"iff: {name}" in text:
            pid = int(fdinfo.parts[2])
            try:
                comm = pathlib.Path(f"/proc/{pid}/comm").read_text().strip()
            except OSError:
                comm = "?"
            holders.append([pid, int(fdinfo.name), comm])
    return sorted(holders)


def tun_open(nonblock=False):
    fd = os.open("/dev/net/tun", os.O_RDWR | (os.O_NONBLOCK if nonblock else 0))
    open_fds.add(fd)
    return fd


def tun_close(fd):
    os.close(fd)
    open_fds.discard(fd)


def tun_setiff(fd, name, flags):
    fcntl.ioctl(fd, TUNSETIFF, struct.pack("16sH22x", name.encode(), flags))


def create_persistent_tap(name, owner):
    fd = tun_open()
    try:
        tun_setiff(fd, name, IFF_TAP | IFF_NO_PI)
        fcntl.ioctl(fd, TUNSETOFFLOAD, 0)
        fcntl.ioctl(fd, TUNSETOWNER, owner)
        fcntl.ioctl(fd, TUNSETPERSIST, 1)
    finally:
        tun_close(fd)


def launcher_attach(name):
    raw = tun_open(nonblock=True)
    fd = fcntl.fcntl(raw, fcntl.F_DUPFD_CLOEXEC, FD_MIN)
    open_fds.add(fd)
    tun_close(raw)
    try:
        tun_setiff(fd, name, IFF_TAP | IFF_NO_PI | IFF_VNET_HDR)
    except BaseException:
        tun_close(fd)
        raise
    return fd


def stats(name):
    return {
        key: int(pathlib.Path(f"/sys/class/net/{name}/statistics/{key}").read_text())
        for key in ("rx_packets", "tx_packets", "rx_bytes", "tx_bytes", "rx_dropped", "tx_dropped")
    }


def thread_security(pid):
    rows = []
    for task in sorted(pathlib.Path(f"/proc/{pid}/task").iterdir(), key=lambda p: int(p.name)):
        try:
            values = {
                line.split(":", 1)[0]: line.split(":", 1)[1].strip()
                for line in (task / "status").read_text().splitlines()
                if line.startswith(("Name:", "NoNewPrivs:", "Seccomp:", "Seccomp_filters:"))
            }
        except OSError:
            continue
        rows.append({"tid": int(task.name), **values})
    return rows


def process_tun_fds(pid):
    rows = []
    for fdinfo in pathlib.Path(f"/proc/{pid}/fdinfo").iterdir():
        try:
            text = fdinfo.read_text()
        except OSError:
            continue
        fields = dict(line.split(":", 1) for line in text.splitlines() if ":" in line)
        if "iff" in fields:
            fd = int(fdinfo.name)
            rows.append({
                "fd": fd,
                "iff": fields["iff"].strip(),
                "inode": os.stat(f"/proc/{pid}/fd/{fd}").st_ino,
            })
    return sorted(rows, key=lambda row: row["fd"])


def wait_for_exec(pid, binary, timeout=5):
    deadline = time.perf_counter() + timeout
    wanted = pathlib.Path(binary).resolve()
    while time.perf_counter() < deadline:
        try:
            if pathlib.Path(os.readlink(f"/proc/{pid}/exe")).resolve() == wanted:
                return
        except OSError:
            pass
        time.sleep(0.01)
    raise ProbeFailure("child did not exec cloud-hypervisor")


def read_line(sock, timeout=40):
    sock.settimeout(timeout)
    data = bytearray()
    while not data.endswith(b"\n"):
        part = sock.recv(1)
        if not part:
            raise ProbeFailure("guest beacon closed before newline")
        data.extend(part)
    return data.decode(errors="replace").strip()


def new_capture(name):
    sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
    sock.setsockopt(socket.SOL_SOCKET, SO_TIMESTAMPNS, 1)
    sock.bind((name, 0))
    sock.setblocking(False)
    return sock


def drain_capture(sock):
    records = []
    while True:
        try:
            frame, ancillary, _, address = sock.recvmsg(65535, 256)
        except OSError as error:
            if error.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.ENETDOWN):
                break
            raise
        timestamp = None
        for level, kind, data in ancillary:
            if level == socket.SOL_SOCKET and kind == SO_TIMESTAMPNS and len(data) >= 16:
                seconds, nanos = struct.unpack("qq", data[:16])
                timestamp = seconds * 1_000_000_000 + nanos
        records.append((timestamp, address, frame))
    return records


def frame_kind(frame):
    if len(frame) < 35:
        return "short"
    ethertype = int.from_bytes(frame[12:14], "big")
    if ethertype == 0x0806:
        return "arp"
    if ethertype == 0x0800 and frame[23] == 1:
        return {8: "icmp-request", 0: "icmp-reply"}.get(frame[34], "icmp-other")
    return f"ethertype-0x{ethertype:04x}"


def summarize_frame(record):
    timestamp, address, frame = record
    return {
        "ts": timestamp,
        "pkttype": address[2],
        "len": len(frame),
        "src": frame[6:12].hex(":") if len(frame) >= 12 else "short",
        "dst": frame[0:6].hex(":") if len(frame) >= 6 else "short",
        "kind": frame_kind(frame),
    }


def confined_command(ch_binary, net_fd):
    kvm_gid = os.stat("/dev/kvm").st_gid
    rlimit_fsize = max((work / "rootfs.ext4").stat().st_size, GUEST_BYTES)
    net = f"fd=[{net_fd}],mac={guest_mac},offload_tso=off,offload_ufo=off,offload_csum=off"
    cmdline = "console=ttyS0 panic=1 root=/dev/vda rw overdrive.net=10.77.239.2/24,gw=10.77.239.1,dns=10.77.239.1"
    return [
        "prlimit", f"--fsize={rlimit_fsize}", "--nofile=256", "--",
        "setpriv", f"--reuid={UID}", f"--regid={GID}", f"--groups={kvm_gid}", "--no-new-privs", "--",
        ch_binary,
        "--cpus", "boot=1",
        "--memory", "size=268435456",
        "--kernel", str(run_dir / "kernel"),
        "--cmdline", cmdline,
        "--disk", f"path={work / 'rootfs.ext4'},image_type=raw",
        "--serial", f"file={run_dir / 'console.log'}",
        "--console", "off",
        "--vsock", f"cid=3,socket={run_dir / 'vsock'}",
        "--api-socket", str(run_dir / "api"),
        "--seccomp", "true",
        "--landlock",
        "--net", net,
        "--landlock-rules", f"path=/sys/class/net/{tap},access=r",
        "--landlock-rules", f"path={run_dir},access=rw",
    ]


try:
    begin(
        "PREFLIGHT",
        "the selected runner is native x86_64 metal with CH v53.0 and clean exact probe names",
        "root, systemd-detect-virt=none, uname recorded, CH reports v53.0, and no exact resource exists",
        "any virtualization, version mismatch, missing artifact, or pre-existing exact resource",
    )
    check(os.geteuid() == 0, "requires root via cargo xtask metal run")
    check(output(["uname", "-m"]) == "x86_64", "runner is not x86_64")
    detect_virt = run(["systemd-detect-virt"], check_rc=False).stdout.strip()
    check(detect_virt == "none", f"runner is virtualized: {detect_virt}")
    ch_binary = shutil.which("cloud-hypervisor")
    check(ch_binary is not None, "cloud-hypervisor absent")
    ch_version = output([ch_binary, "--version"]).splitlines()[0]
    check("v53.0" in ch_version, f"wrong CH version: {ch_version}")
    check(kernel_master.is_file() and rootfs_master.is_file(), "guest artifacts absent")
    for name in (tap, scratch, bridge):
        check(not iface_exists(name) and not tun_holders(name), f"pre-existing link/holder: {name}")
    for path in (run_dir, cgroup, work):
        check(not path.exists(), f"pre-existing path: {path}")
    before_ch = ch_pids()
    emit("ENV", f"mode={MODE}")
    emit("ENV", f"uname_r={output(['uname', '-r'])}")
    emit("ENV", f"uname_full={output(['uname', '-srmo'])}")
    emit("ENV", f"systemd_detect_virt={detect_virt}")
    emit("ENV", f"cloud_hypervisor={ch_binary} version={ch_version}")
    emit("ENV", f"ch_v53_source_commit={CH_V53_COMMIT}")
    emit("ENV", "fd_path_ioctls=" + json.dumps(FD_PATH_IOCTL_ITEMS))
    emit("ENV", "deny_list=" + json.dumps(DENIED_IOCTL_ITEMS))
    passed("PREFLIGHT", "native metal and exact resource complement clean")

    begin(
        "IOCTL_ARM",
        "the launcher filter returns EPERM for every deny-listed TAP mutation on every descendant thread; without it the kernel decides each request",
        "filtered: four inherited-filter threads report EPERM for all 13 requests; control: one unfiltered thread reports no EPERM",
        "any filtered request reaches the kernel, any filtered thread lacks seccomp mode 2, or any control request is preempted by EPERM",
    )
    create_persistent_tap(scratch, UID)
    scratch_fd = launcher_attach(scratch)
    fd_flags = fcntl.fcntl(scratch_fd, fcntl.F_GETFD)
    fcntl.fcntl(scratch_fd, fcntl.F_SETFD, fd_flags & ~fcntl.FD_CLOEXEC)
    # The metal checkout's parent is intentionally not traversable by uid 4200.
    # Hand the already-open probe source across exec just like the TAP fd, so
    # the helper exercises the intended uid without changing workspace modes.
    helper_source = open(__file__, "rb")
    helper_cmd = [
        "setpriv", f"--reuid={UID}", f"--regid={GID}", "--clear-groups", "--no-new-privs", "--",
        sys.executable, f"/proc/self/fd/{helper_source.fileno()}", "--ioctl-child", "--mode", MODE,
        "--fd", str(scratch_fd), "--tap", scratch,
    ]
    emit("IOCTL_ARM", "helper_cli=" + shlex.join(helper_cmd))
    helper = subprocess.run(
        helper_cmd,
        text=True,
        capture_output=True,
        pass_fds=(scratch_fd, helper_source.fileno()),
        preexec_fn=install_ioctl_deny_filter if MODE == "filtered" else None,
    )
    helper_source.close()
    emit("IOCTL_ARM", f"helper_rc={helper.returncode}")
    emit("IOCTL_ARM", "helper_stdout_begin")
    for line in helper.stdout.splitlines():
        emit("IOCTL_ARM", "  " + line)
    emit("IOCTL_ARM", "helper_stdout_end")
    emit("IOCTL_ARM", "helper_stderr=" + repr(helper.stderr))
    check(helper.returncode == 0, "ioctl helper failed")
    tun_close(scratch_fd)
    run(["ip", "link", "delete", "dev", scratch])
    check(not iface_exists(scratch) and not tun_holders(scratch), "scratch TAP residue")
    passed("IOCTL_ARM", f"mode={MODE} deny-list causal control passed; scratch removed")

    begin(
        "STAGING",
        "the increment-y guest and artifacts can be staged without altering shared masters",
        "private reflink rootfs, busybox beacon support, kernel, run directory and cgroup are ready",
        "any private staging step fails or touches a non-probe resource",
    )
    work.mkdir(mode=0o710)
    run(["cp", "--reflink=auto", str(rootfs_master), str(work / "rootfs.ext4")])
    loop = output(["losetup", "--find", "--show", str(work / "rootfs.ext4")])
    (work / "mnt").mkdir()
    run(["mount", loop, str(work / "mnt")])
    shutil.copy2("/usr/bin/busybox", work / "mnt/sbin/busybox")
    os.chmod(work / "mnt/sbin/busybox", 0o755)
    run(["umount", str(work / "mnt")])
    run(["losetup", "-d", loop])
    loop = ""
    run_dir.mkdir(parents=True, mode=0o700)
    shutil.copy2(kernel_master, run_dir / "kernel")
    os.chown(run_dir / "kernel", UID, GID)
    os.chown(run_dir, UID, GID)
    os.chown(work / "rootfs.ext4", UID, GID)
    os.chown(work, 0, GID)
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    beacon_path = run_dir / "vsock_1234"
    listener.bind(str(beacon_path))
    listener.listen(1)
    os.chown(beacon_path, UID, GID)
    cgroup.mkdir()
    (cgroup / "cpu.weight").write_text("100")
    (cgroup / "memory.max").write_text(str(CGROUP_MAX))
    passed("STAGING", "private guest staging ready")

    begin(
        "BOOT",
        "CH v53's fd= construction ioctls are disjoint from the deny-list, so CH boots with the launcher filter inherited",
        "guest READY, exact TAP fd held by CH, and filtered mode shows seccomp mode 2 plus at least one filter on every CH thread including leader",
        "CH exits, guest misses READY, required ioctl is denied, exact TAP fd is absent, or any filtered CH thread lacks the launcher filter",
    )
    run(["ip", "link", "add", bridge, "type", "bridge"])
    run(["ip", "addr", "add", "10.77.239.1/24", "dev", bridge])
    run(["ip", "link", "set", bridge, "up"])
    create_persistent_tap(tap, 0)
    run(["ip", "link", "set", tap, "master", bridge])
    run(["nft", "add", "table", "bridge", nft_table])
    run(["nft", "add", "chain", "bridge", nft_table, "input", "{ type filter hook input priority -300; policy drop; }"])
    run(["nft", "add", "rule", "bridge", nft_table, "input", "iifname", tap, "ether", "type", "arp", "counter", "accept"])
    run(["nft", "add", "rule", "bridge", nft_table, "input", "iifname", tap, "ip", "protocol", "icmp", "counter", "accept"])
    tap_fd = launcher_attach(tap)
    tap_inode = os.fstat(tap_fd).st_ino
    fd_flags = fcntl.fcntl(tap_fd, fcntl.F_GETFD)
    fcntl.fcntl(tap_fd, fcntl.F_SETFD, fd_flags & ~fcntl.FD_CLOEXEC)
    capture = new_capture(tap)
    command = confined_command(ch_binary, tap_fd)
    emit("BOOT", "ch_cli=" + shlex.join(command))
    stderr_handle = open(run_dir / "ch.stderr", "wb")
    main_proc = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=stderr_handle,
        close_fds=True,
        pass_fds=(tap_fd,),
        preexec_fn=install_ioctl_deny_filter if MODE == "filtered" else None,
    )
    (cgroup / "cgroup.procs").write_text(str(main_proc.pid))
    wait_for_exec(main_proc.pid, ch_binary)
    listener.settimeout(40)
    conn, _ = listener.accept()
    ready_line = read_line(conn)
    ready_security = thread_security(main_proc.pid)
    ready_fds = process_tun_fds(main_proc.pid)
    emit("BOOT", f"ch_pid={main_proc.pid} ready_line={ready_line!r}")
    emit("BOOT", "thread_security_ready=" + json.dumps(ready_security, sort_keys=True))
    emit("BOOT", "ch_tun_fds_ready=" + json.dumps(ready_fds, sort_keys=True))
    emit("BOOT", "tap_state_ready=" + output(["ip", "-j", "-d", "link", "show", "dev", tap]))
    emit("BOOT", "tap_stats_ready=" + json.dumps(stats(tap), sort_keys=True))
    check(ready_line.startswith("READY pid=1 port=1234"), "unexpected READY beacon")
    check(any(row["fd"] == tap_fd and row["iff"] == tap and row["inode"] == tap_inode for row in ready_fds), "CH lacks inherited exact TAP fd")
    leader = next(row for row in ready_security if row["tid"] == main_proc.pid)
    if MODE == "filtered":
        check(all(row.get("Seccomp") == "2" and int(row.get("Seccomp_filters", "0")) >= 1 for row in ready_security), "a READY CH thread lacks launcher filter")
        check(leader.get("Seccomp") == "2" and int(leader.get("Seccomp_filters", "0")) >= 1, "CH main thread unfiltered")
    else:
        check(leader.get("Seccomp") == "0" and int(leader.get("Seccomp_filters", "0")) == 0, "control CH main unexpectedly filtered")
    tun_close(tap_fd)
    passed("BOOT", f"mode={MODE} READY; threads={len(ready_security)} leader_seccomp={leader.get('Seccomp')} leader_filters={leader.get('Seccomp_filters')}")

    begin(
        "TRAFFIC",
        "after guard read-back and TAP activation, CH under the selected launcher-filter mode passes traffic in both directions",
        "guest ping reaches host, host ping reaches guest, RX and TX counters advance, and capture shows guest-originated plus host-originated ICMP after activation",
        "either ping fails, either direction is absent from capture/counters, or a frame predates activation",
    )
    nft_readback = output(["nft", "-a", "list", "table", "bridge", nft_table])
    emit("TRAFFIC", "guard_readback_begin")
    for line in nft_readback.splitlines():
        emit("TRAFFIC", "  " + line)
    emit("TRAFFIC", "guard_readback_end")
    check(f'iifname "{tap}" ether type arp' in nft_readback and "policy drop" in nft_readback, "guard read-back mismatch")
    run(["ethtool", "-K", tap, "tx-checksum-ip-generic", "off"], check_rc=False)
    before_stats = stats(tap)
    activation_ns = time.time_ns()
    run(["ip", "link", "set", tap, "up"])
    conn.sendall(b'EXEC ["/sbin/busybox","sh","-c","ping -c 1 -W 2 10.77.239.1; sleep 60"]\n')
    guest_progress = False
    host_ping = None
    records = []
    deadline = time.perf_counter() + 8
    while time.perf_counter() < deadline:
        records.extend(drain_capture(capture))
        kinds = [frame_kind(record[2]) for record in records]
        guest_progress = "icmp-request" in kinds and "icmp-reply" in kinds
        if guest_progress and host_ping is None:
            host_ping = run(["ping", "-c", "1", "-W", "2", "10.77.239.2"], check_rc=False)
            emit("TRAFFIC", f"host_ping_rc={host_ping.returncode} stdout={host_ping.stdout!r} stderr={host_ping.stderr!r}")
        if guest_progress and host_ping is not None and host_ping.returncode == 0:
            break
        time.sleep(0.02)
    time.sleep(0.1)
    records.extend(drain_capture(capture))
    after_stats = stats(tap)
    pre_activation = [row for row in records if row[0] is None or row[0] <= activation_ns]
    guest_src = [row for row in records if row[2][6:12].hex(":") == guest_mac]
    host_src = [row for row in records if row[2][6:12].hex(":") != guest_mac]
    emit("TRAFFIC", f"activation_ns={activation_ns} before_stats={json.dumps(before_stats, sort_keys=True)} after_stats={json.dumps(after_stats, sort_keys=True)}")
    emit("TRAFFIC", f"capture_count={len(records)} pre_activation={len(pre_activation)} guest_src={len(guest_src)} host_src={len(host_src)}")
    for row in records[:30]:
        emit("TRAFFIC", "frame=" + json.dumps(summarize_frame(row), sort_keys=True))
    check(guest_progress, "guest->host ping exchange absent")
    check(host_ping is not None and host_ping.returncode == 0, "host->guest ping failed")
    check(not pre_activation, "capture contains pre-activation frame")
    check(guest_src and host_src, "capture lacks one traffic direction")
    check(after_stats["rx_packets"] > before_stats["rx_packets"] and after_stats["tx_packets"] > before_stats["tx_packets"], "TAP counters lack bidirectional progress")
    live_security = thread_security(main_proc.pid)
    emit("TRAFFIC", "thread_security_live=" + json.dumps(live_security, sort_keys=True))
    if MODE == "filtered":
        check(all(row.get("Seccomp") == "2" and int(row.get("Seccomp_filters", "0")) >= 1 for row in live_security), "a live CH thread lacks launcher filter")
    passed("TRAFFIC", f"mode={MODE} guest_ping=true host_ping=true rx_delta={after_stats['rx_packets'] - before_stats['rx_packets']} tx_delta={after_stats['tx_packets'] - before_stats['tx_packets']} all_threads={len(live_security)}")

    begin(
        "CLEANUP",
        "terminating CH then removing only tag-scoped resources leaves the exact pre-run complement",
        "CH gone; TAP, bridge, nft table, cgroup, run directory, staging directory and holders absent",
        "any exact resource or new CH process remains",
    )
    main_proc.terminate()
    try:
        main_proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        (cgroup / "cgroup.kill").write_text("1")
        main_proc.wait(timeout=5)
    stderr_handle.close()
    stderr_handle = None
    ch_stderr = (run_dir / "ch.stderr").read_text(errors="replace")
    emit("CLEANUP", "ch_stderr_complete_begin")
    for line in ch_stderr.splitlines():
        emit("CLEANUP", "  " + line)
    emit("CLEANUP", "ch_stderr_complete_end")
    emit("CLEANUP", f"ch_exit={main_proc.returncode}")
    main_proc = None
    if capture is not None:
        capture.close()
        capture = None
    if conn is not None:
        conn.close()
        conn = None
    if listener is not None:
        listener.close()
        listener = None
    run(["ip", "link", "set", tap, "down"], check_rc=False)
    run(["ip", "link", "delete", "dev", tap])
    run(["nft", "delete", "table", "bridge", nft_table])
    run(["ip", "link", "delete", "dev", bridge])
    for _ in range(100):
        try:
            cgroup.rmdir()
            break
        except OSError:
            time.sleep(0.02)
    shutil.rmtree(run_dir)
    shutil.rmtree(work)
    final = {
        "new_ch_pids": sorted(ch_pids() - before_ch),
        "tap_exists": iface_exists(tap),
        "scratch_exists": iface_exists(scratch),
        "bridge_exists": iface_exists(bridge),
        "tap_holders": tun_holders(tap),
        "scratch_holders": tun_holders(scratch),
        "nft_exists": run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode == 0,
        "cgroup_exists": cgroup.exists(),
        "run_dir_exists": run_dir.exists(),
        "work_exists": work.exists(),
        "open_tun_fds": sorted(open_fds),
    }
    emit("CLEANUP", "final_readback=" + json.dumps(final, sort_keys=True))
    check(not any(bool(value) for value in final.values()), "residue remains")
    passed("CLEANUP", "exact empty complement")

    for step, result in step_results.items():
        emit("SUMMARY", f"{step}: {result}")
    emit("SUMMARY", f"elapsed_seconds={time.perf_counter() - started:.6f}")
    print(f"VERDICT=WORKS mode={MODE}", flush=True)
except BaseException as error:
    emit("FAILURE", f"step={current_step} error={type(error).__name__}: {error}")
    for step, result in step_results.items():
        emit("SUMMARY", f"{step}: {result}")
    print(f"VERDICT=DOESNT_WORK_OR_INCOMPLETE mode={MODE}", flush=True)
    raise
finally:
    fallback = []
    if conn is not None:
        conn.close()
    if listener is not None:
        listener.close()
    if capture is not None:
        capture.close()
    if main_proc is not None and main_proc.poll() is None:
        fallback.append(f"ch_pid={main_proc.pid}")
        if cgroup.exists() and (cgroup / "cgroup.kill").exists():
            (cgroup / "cgroup.kill").write_text("1")
        else:
            main_proc.kill()
        main_proc.wait()
    if stderr_handle is not None:
        stderr_handle.close()
    for fd in sorted(open_fds):
        fallback.append(f"tun_fd={fd}")
        os.close(fd)
    open_fds.clear()
    if loop:
        fallback.append(f"loop={loop}")
        if run(["mountpoint", "-q", str(work / "mnt")], check_rc=False).returncode == 0:
            run(["umount", str(work / "mnt")], check_rc=False)
        run(["losetup", "-d", loop], check_rc=False)
    if run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode == 0:
        fallback.append(f"nft={nft_table}")
        run(["nft", "delete", "table", "bridge", nft_table], check_rc=False)
    for name in (tap, scratch, bridge):
        if iface_exists(name):
            fallback.append(f"link={name}")
            run(["ip", "link", "delete", "dev", name], check_rc=False)
    if cgroup.exists():
        fallback.append(f"cgroup={cgroup}")
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
    for path in (run_dir, work):
        if path.exists():
            fallback.append(f"path={path}")
            shutil.rmtree(path)
    final_cleanup = {
        "new_ch_pids": sorted(ch_pids() - before_ch),
        "tap": iface_exists(tap),
        "scratch": iface_exists(scratch),
        "bridge": iface_exists(bridge),
        "tap_holders": tun_holders(tap),
        "nft": run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode == 0,
        "cgroup": cgroup.exists(),
        "run_dir": run_dir.exists(),
        "work": work.exists(),
    }
    print("CLEANUP_FALLBACK " + json.dumps(fallback, sort_keys=True), flush=True)
    print("CLEANUP_FINAL " + json.dumps(final_cleanup, sort_keys=True), flush=True)
    print(f"cleanup_complete={not any(bool(value) for value in final_cleanup.values())}", flush=True)
