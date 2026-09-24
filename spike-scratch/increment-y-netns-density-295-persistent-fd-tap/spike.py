#!/usr/bin/env python3
"""SPIKE increment-y: persistent named TAP + separate launcher queue fd -> CH v53.

Throwaway code. Not production. Not a test tier. Deleted when the
implementation it validates lands (.claude/rules/spike.md).

Adapted from spike-scratch/netns-density-295-fd-tap/spike.py (preserved,
unmodified). The prior probe used a NON-persistent fd-owned TAP; this probe
uses the production-shaped PERSISTENT named TAP:

  1. network owner creates a persistent TAP exactly like
     overdrive_netlink::create_persistent_tap (TUNSETIFF IFF_TAP|IFF_NO_PI,
     TUNSETOFFLOAD(0), TUNSETOWNER(4200), TUNSETPERSIST(1), close), attaches it
     to a bridge, leaves it administratively down;
  2. the VMM launcher opens its own /dev/net/tun queue fd, TUNSETIFF-attaches
     it to the existing TAP and passes it to CH via --net fd=[N] through the
     production prlimit/setpriv/seccomp/landlock/cgroup chain;
  3. CH reaches the real guest READY beacon while the TAP stays down;
  4. the launcher closes its copy; CH retains the queue;
  5. the network owner raises the TAP after the (spike-local) guard is live;
  6. CH exit closes its queue, the persistent TAP remains, nobody holds it;
  7. teardown deletes the TAP (production-equivalent RTM_DELLINK) with no
     residue.

Variants:
  --variant main          (default) launcher attaches WITH IFF_VNET_HDR
  --variant no-vnet-hdr   launcher attaches with the production creator's
                          flags only (IFF_TAP|IFF_NO_PI); observes what CH v53
                          does with such a queue
"""

import argparse
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
import time


# Linux tun ioctls/flags (include/uapi/linux/if_tun.h).
TUNSETIFF = 0x400454CA
TUNSETPERSIST = 0x400454CB
TUNSETOWNER = 0x400454CC
TUNSETOFFLOAD = 0x400454D0
TUNGETIFF = 0x800454D2
IFF_TAP = 0x0002
IFF_MULTI_QUEUE = 0x0100
IFF_PERSIST = 0x0800
IFF_NO_PI = 0x1000
IFF_VNET_HDR = 0x4000
SO_TIMESTAMPNS = 35

FD_MIN = 50
UID = 4200  # overdrive_core::vm::config::OVERDRIVE_VMM_UID
GID = 4200
GUEST_BYTES = 256 * 1024 * 1024
CGROUP_MAX = GUEST_BYTES + 8 * 1024 * 1024 + GUEST_BYTES // 400
STAT_NAMES = ("rx_packets", "tx_packets", "rx_bytes", "tx_bytes", "rx_dropped", "tx_dropped")
OFFLOAD_KEYS = (
    "rx-checksumming",
    "tx-checksumming",
    "tx-checksum-ip-generic",
    "scatter-gather",
    "tcp-segmentation-offload",
    "udp-fragmentation-offload",
    "generic-segmentation-offload",
    "generic-receive-offload",
    "tx-udp-segmentation",
)


parser = argparse.ArgumentParser()
parser.add_argument("--variant", choices=("main", "no-vnet-hdr"), default="main")
args = parser.parse_args()
VARIANT = args.variant
LAUNCHER_FLAGS = IFF_TAP | IFF_NO_PI | (IFF_VNET_HDR if VARIANT == "main" else 0)

started = time.perf_counter()
tag = f"{os.getpid() & 0xffff:04x}"
tap = f"pftap{tag}"
scratch = f"pfx{tag}"
bridge = f"pfbr{tag}"
nft_table = f"pfguard{tag}"
run_dir = pathlib.Path(f"/run/overdrive/vm/pf-tap-{tag}")
cgroup = pathlib.Path(f"/sys/fs/cgroup/overdrive.slice/workloads.slice/pf-tap-{tag}.scope")
staging_root = pathlib.Path(os.environ.get("OVERDRIVE_METAL_ROOTFS", "/missing")).parent
work = staging_root / f"pf-tap-{tag}"
kernel_master = pathlib.Path(os.environ.get("OVERDRIVE_METAL_KERNEL", "/missing"))
rootfs_master = pathlib.Path(os.environ.get("OVERDRIVE_METAL_ROOTFS", "/missing"))
guest_mac = f"02:00:00:95:{tag[:2]}:{tag[2:]}"

open_fds = set()  # every tun fd this process owns, for the cleanup owner
loop = ""
packet_capture = None
main_proc = None
fail_proc = None
listener = None
conn = None
stderr_handle = None
before_ch = set()
step_results = {}
current_step = "PREFLIGHT"


class ProbeFailure(Exception):
    pass


def emit(phase, text):
    print(f"[+{time.perf_counter() - started:9.6f}] {phase} {text}", flush=True)


def check(condition, message):
    if not condition:
        raise ProbeFailure(f"{current_step}: {message}")


def begin(step):
    global current_step
    current_step = step
    emit(step, "BEGIN")


def passed(step, summary):
    step_results[step] = f"PASS {summary}"
    emit(step, f"RESULT=PASS {summary}")


def run(argv, *, check_rc=True):
    return subprocess.run(argv, check=check_rc, text=True, capture_output=True)


def command_output(argv):
    return run(argv).stdout.strip()


def errno_text(error):
    return f"errno={error.errno}({errno.errorcode.get(error.errno, '?')}) {error.strerror}"


def attempt(fn):
    try:
        result = fn()
        return "ok" if result is None else f"ok {result}"
    except OSError as error:
        return errno_text(error)


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


def iface_exists(name):
    return pathlib.Path("/sys/class/net", name).exists()


def link_json(name):
    return json.loads(command_output(["ip", "-j", "-d", "link", "show", "dev", name]))[0]


def read_sys(name, attr):
    try:
        return pathlib.Path(f"/sys/class/net/{name}/{attr}").read_text().strip()
    except FileNotFoundError:
        return "ABSENT"
    except OSError as error:
        return errno_text(error)


def offloads(name):
    result = run(["ethtool", "-k", name], check_rc=False)
    table = {}
    for line in result.stdout.splitlines():
        if ":" in line and not line.startswith("Features for"):
            key, value = line.split(":", 1)
            table[key.strip()] = value.strip()
    digest = hashlib.sha256(result.stdout.encode()).hexdigest()[:16]
    return {key: table.get(key, "?") for key in OFFLOAD_KEYS}, digest, table


def tun_holders(name):
    """Every (pid, fd, comm) on the host whose fdinfo names `iff: <name>`."""
    holders = []
    scanned = 0
    for fdinfo in pathlib.Path("/proc").glob("[0-9]*/fdinfo/*"):
        scanned += 1
        try:
            text = fdinfo.read_text()
        except (FileNotFoundError, PermissionError, ProcessLookupError, OSError):
            continue
        for line in text.splitlines():
            parts = line.split()
            if len(parts) == 2 and parts[0] == "iff:" and parts[1] == name:
                pid = int(fdinfo.parts[2])
                try:
                    comm = pathlib.Path(f"/proc/{pid}/comm").read_text().strip()
                except OSError:
                    comm = "?"
                holders.append([pid, int(fdinfo.name), comm])
    return sorted(holders), scanned


def tap_snapshot(name):
    holders, scanned = tun_holders(name)
    if not iface_exists(name):
        return {"exists": False, "holders": holders, "fdinfo_scanned": scanned}
    link = link_json(name)
    subset, digest, full = offloads(name)
    return {
        "exists": True,
        "ifindex": link["ifindex"],
        "flags": link["flags"],
        "operstate": link["operstate"],
        "carrier": read_sys(name, "carrier"),
        "master": link.get("master"),
        "mtu": link.get("mtu"),
        "address": link.get("address"),
        "tun_flags": read_sys(name, "tun_flags"),
        "owner": read_sys(name, "owner"),
        "group": read_sys(name, "group"),
        "info_data": link.get("linkinfo", {}).get("info_data"),
        "offload": subset,
        "offload_digest": digest,
        "offload_full": full,
        "stats": {stat: int(read_sys(name, f"statistics/{stat}")) for stat in STAT_NAMES},
        "holders": holders,
        "fdinfo_scanned": scanned,
    }


def show(phase, label, snap):
    printable = {key: value for key, value in snap.items() if key != "offload_full"}
    emit(phase, f"{label}={json.dumps(printable, sort_keys=True)}")


def state_diff(before, after, ignore=("stats", "holders", "fdinfo_scanned", "offload", "offload_digest")):
    changed = {}
    for key in sorted(set(before) | set(after)):
        if key in ignore:
            continue
        if before.get(key) != after.get(key):
            changed[key] = [before.get(key), after.get(key)]
    return changed


def tun_open(nonblock):
    # os.open is O_CLOEXEC (non-inheritable) by default, like Rust OpenOptions.
    fd = os.open("/dev/net/tun", os.O_RDWR | (os.O_NONBLOCK if nonblock else 0))
    open_fds.add(fd)
    return fd


def tun_close(fd):
    os.close(fd)
    open_fds.discard(fd)


def tun_setiff(fd, name, flags):
    fcntl.ioctl(fd, TUNSETIFF, struct.pack("16sH22x", name.encode(), flags))


def tun_getiff(fd):
    raw = fcntl.ioctl(fd, TUNGETIFF, bytes(40))
    name, flags = struct.unpack("16sH22x", raw)
    return f"name={name.rstrip(bytes(1)).decode()} flags=0x{flags:04x}"


def production_create_persistent_tap(name, owner, hold=False):
    """Mirror overdrive_netlink::create_persistent_tap ioctl-for-ioctl.

    `hold=True` returns the creator fd before its close (used only by the
    single-queue semantics sub-probe on a scratch TAP).
    """
    fd = tun_open(nonblock=False)
    try:
        tun_setiff(fd, name, IFF_TAP | IFF_NO_PI)
        fcntl.ioctl(fd, TUNSETOFFLOAD, 0)
        fcntl.ioctl(fd, TUNSETOWNER, owner)
        fcntl.ioctl(fd, TUNSETPERSIST, 1)
    except OSError:
        tun_close(fd)
        raise
    if hold:
        return fd
    tun_close(fd)
    return -1


def launcher_attach(name, flags):
    """VMM launcher: separate /dev/net/tun open + TUNSETIFF onto the existing TAP."""
    raw = tun_open(nonblock=True)
    fd = fcntl.fcntl(raw, fcntl.F_DUPFD_CLOEXEC, FD_MIN)
    open_fds.add(fd)
    tun_close(raw)
    try:
        tun_setiff(fd, name, flags)
    except OSError:
        tun_close(fd)
        raise
    return fd


def open_fds_without_cloexec():
    result = []
    for entry in pathlib.Path("/proc/self/fd").iterdir():
        try:
            fd = int(entry.name)
            if not fcntl.fcntl(fd, fcntl.F_GETFD) & fcntl.FD_CLOEXEC:
                result.append(fd)
        except (FileNotFoundError, OSError, ValueError):
            pass
    return sorted(result)


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
            if level == socket.SOL_SOCKET and kind == SO_TIMESTAMPNS and len(data) >= 16:
                seconds, nanos = struct.unpack("qq", data[:16])
                timestamp_ns = seconds * 1_000_000_000 + nanos
        records.append((timestamp_ns, address, frame))
    return records


def classify(frame):
    if len(frame) < 14:
        return "truncated"
    ether_type = int.from_bytes(frame[12:14], "big")
    if ether_type == 0x0806 and len(frame) >= 22:
        return {1: "arp-request", 2: "arp-reply"}.get(int.from_bytes(frame[20:22], "big"), "arp-other")
    if ether_type == 0x0800 and len(frame) >= 35:
        if frame[23] == 1:
            return {8: "icmp-echo-request", 0: "icmp-echo-reply"}.get(frame[34], "icmp-other")
        return f"ipv4-proto-{frame[23]}"
    if ether_type == 0x86DD:
        return "ipv6"
    return f"ethertype-0x{ether_type:04x}"


def frame_summary(record):
    timestamp_ns, address, frame = record
    if len(frame) < 14:
        return f"ts={timestamp_ns} truncated_len={len(frame)} hex={frame.hex()}"
    dst = ":".join(f"{octet:02x}" for octet in frame[0:6])
    src = ":".join(f"{octet:02x}" for octet in frame[6:12])
    return (
        f"ts={timestamp_ns} pkttype={address[2]} len={len(frame)} src={src} dst={dst} "
        f"kind={classify(frame)} head={frame[:24].hex()}"
    )


def new_capture():
    sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
    sock.setsockopt(socket.SOL_SOCKET, SO_TIMESTAMPNS, 1)
    sock.bind((tap, 0))
    sock.setblocking(False)
    return sock


def confined_command(ch_binary, net_fd, kernel, rootfs, console, api, vsock):
    kvm_gid = os.stat("/dev/kvm").st_gid
    rlimit_fsize = max(rootfs.stat().st_size, GUEST_BYTES)
    net = f"fd=[{net_fd}],mac={guest_mac},offload_tso=off,offload_ufo=off,offload_csum=off"
    cmdline = "console=ttyS0 panic=1 root=/dev/vda rw overdrive.net=10.77.239.2/24,gw=10.77.239.1,dns=10.77.239.1"
    return [
        "prlimit", f"--fsize={rlimit_fsize}", "--nofile=256", "--",
        "setpriv", f"--reuid={UID}", f"--regid={GID}", f"--groups={kvm_gid}", "--no-new-privs", "--",
        ch_binary,
        "--cpus", "boot=1",
        "--memory", "size=268435456",
        "--kernel", str(kernel),
        "--cmdline", cmdline,
        "--disk", f"path={rootfs},image_type=raw",
        "--serial", f"file={console}",
        "--console", "off",
        "--vsock", f"cid=3,socket={vsock}",
        "--api-socket", str(api),
        "--seccomp", "true",
        "--landlock",
        "--net", net,
        "--landlock-rules", f"path=/sys/class/net/{tap},access=r",
        "--landlock-rules", f"path={run_dir},access=rw",
    ]


def wait_for_exec(pid, binary, timeout=5):
    deadline = time.perf_counter() + timeout
    wanted = pathlib.Path(binary).resolve()
    while time.perf_counter() < deadline:
        try:
            if pathlib.Path(os.readlink(f"/proc/{pid}/exe")).resolve() == wanted:
                return
        except FileNotFoundError:
            pass
        time.sleep(0.01)
    raise ProbeFailure("child did not exec cloud-hypervisor")


def thread_security(pid):
    observed = []
    for task in sorted(pathlib.Path(f"/proc/{pid}/task").iterdir(), key=lambda path: int(path.name)):
        values = {
            line.split(":", 1)[0]: line.split(":", 1)[1].strip()
            for line in (task / "status").read_text().splitlines()
            if line.startswith(("Name:", "NoNewPrivs:", "Seccomp:", "Seccomp_filters:"))
        }
        observed.append({"tid": int(task.name), **values})
    return observed


def process_tun_fds(pid):
    found = []
    for fdinfo in pathlib.Path(f"/proc/{pid}/fdinfo").iterdir():
        try:
            text = fdinfo.read_text()
        except OSError:
            continue
        fields = dict(line.split(":", 1) for line in text.splitlines() if ":" in line)
        if "iff" in fields:
            fd = int(fdinfo.name)
            found.append({
                "fd": fd,
                "iff": fields["iff"].strip(),
                "fdinfo_flags": fields.get("flags", "").strip(),
                "inode": os.stat(f"/proc/{pid}/fd/{fd}").st_ino,
            })
    return sorted(found, key=lambda item: item["fd"])


def read_line(sock, timeout=35):
    sock.settimeout(timeout)
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = sock.recv(1)
        if not chunk:
            raise ProbeFailure("beacon EOF before a complete line")
        data.extend(chunk)
    return data.decode("utf-8", errors="replace").rstrip("\r\n")


def stats(name):
    return {stat: int(read_sys(name, f"statistics/{stat}")) for stat in STAT_NAMES}


def single_queue_semantics():
    """Kernel attach semantics on a scratch persistent TAP (no VMM)."""
    begin("SQ")
    emit("SQ", "hypothesis=a single-queue persistent TAP admits exactly one attached queue fd; TUN_FEATURES follow the last attacher")
    emit("SQ", "prediction=attach while creator fd open -> EBUSY; after close ok; re-TUNSETIFF on an attached fd -> EEXIST; "
         "IFF_VNET_HDR in sysfs tun_flags equals the last attacher's request and survives detach")
    emit("SQ", "falsification=attach succeeds while another queue fd is attached, or tun_flags does not track the attacher's flags")
    creator = production_create_persistent_tap(scratch, UID, hold=True)
    show("SQ", "creator_held_snapshot", tap_snapshot(scratch))
    emit("SQ", f"creator_TUNGETIFF={tun_getiff(creator)}")
    contender = tun_open(nonblock=True)
    busy_with_creator = attempt(lambda: tun_setiff(contender, scratch, IFF_TAP | IFF_NO_PI | IFF_VNET_HDR))
    emit("SQ", f"launcher_attach_while_creator_fd_open={busy_with_creator}")
    tun_close(contender)
    tun_close(creator)
    after_creator = tap_snapshot(scratch)
    show("SQ", "after_creator_close_snapshot", after_creator)

    plain = tun_open(nonblock=True)
    plain_attach = attempt(lambda: tun_setiff(plain, scratch, IFF_TAP | IFF_NO_PI))
    emit("SQ", f"attach_without_vnet_hdr={plain_attach} TUNGETIFF={tun_getiff(plain)} sysfs_tun_flags={read_sys(scratch, 'tun_flags')}")
    second = tun_open(nonblock=True)
    busy_with_queue = attempt(lambda: tun_setiff(second, scratch, IFF_TAP | IFF_NO_PI | IFF_VNET_HDR))
    emit("SQ", f"second_attach_while_queue_attached={busy_with_queue}")
    tun_close(second)
    reissue = attempt(lambda: tun_setiff(plain, scratch, IFF_TAP | IFF_NO_PI | IFF_VNET_HDR))
    emit("SQ", f"ch_style_reissue_TUNSETIFF_with_vnet_on_attached_fd={reissue} sysfs_tun_flags={read_sys(scratch, 'tun_flags')}")
    tun_close(plain)
    emit("SQ", f"after_plain_close holders={tun_holders(scratch)[0]} sysfs_tun_flags={read_sys(scratch, 'tun_flags')}")

    vnet = tun_open(nonblock=True)
    vnet_attach = attempt(lambda: tun_setiff(vnet, scratch, IFF_TAP | IFF_NO_PI | IFF_VNET_HDR))
    emit("SQ", f"attach_with_vnet_hdr={vnet_attach} TUNGETIFF={tun_getiff(vnet)} sysfs_tun_flags={read_sys(scratch, 'tun_flags')}")
    tun_close(vnet)
    sticky_after_vnet = read_sys(scratch, "tun_flags")
    emit("SQ", f"after_vnet_close holders={tun_holders(scratch)[0]} sysfs_tun_flags={sticky_after_vnet}")
    plain_again = tun_open(nonblock=True)
    emit("SQ", f"reattach_without_vnet_hdr={attempt(lambda: tun_setiff(plain_again, scratch, IFF_TAP | IFF_NO_PI))} "
         f"sysfs_tun_flags={read_sys(scratch, 'tun_flags')}")
    tun_close(plain_again)
    emit("SQ", f"after_plain_again_close sysfs_tun_flags={read_sys(scratch, 'tun_flags')}")

    # Delete while a queue is attached: what happens to the holder?
    held = tun_open(nonblock=True)
    emit("SQ", f"attach_for_delete_probe={attempt(lambda: tun_setiff(held, scratch, IFF_TAP | IFF_NO_PI | IFF_VNET_HDR))} "
         f"holders={tun_holders(scratch)[0]}")
    deleted = run(["ip", "link", "delete", "dev", scratch], check_rc=False)
    emit("SQ", f"rtm_dellink_with_queue_attached rc={deleted.returncode} stderr={deleted.stderr.strip()!r} "
         f"exists_after={iface_exists(scratch)}")
    emit("SQ", f"holder_TUNGETIFF_after_delete={attempt(lambda: tun_getiff(held))} holders_after={tun_holders(scratch)[0]}")
    tun_close(held)
    check(not iface_exists(scratch), "scratch TAP survived delete")
    check(busy_with_creator.startswith("errno=16"), "attach while creator fd open was not EBUSY")
    check(busy_with_queue.startswith("errno=16"), "second attach while a queue is attached was not EBUSY")
    check(reissue.startswith("errno=17"), "re-TUNSETIFF on an attached fd was not EEXIST")
    passed("SQ", f"creator_open={busy_with_creator.split()[0]} second_queue={busy_with_queue.split()[0]} "
                 f"reissue={reissue.split()[0]} vnet_sticky_after_detach={sticky_after_vnet}")


try:
    begin("PREFLIGHT")
    check(os.geteuid() == 0, "spike must run through cargo xtask metal run (root context)")
    check(command_output(["uname", "-m"]) == "x86_64", "native qualification requires x86_64")
    detect_virt = run(["systemd-detect-virt"], check_rc=False).stdout.strip()
    check(detect_virt == "none", "native qualification requires systemd-detect-virt=none")
    check(kernel_master.is_file() and rootfs_master.is_file(), "configured remote guest artifacts are missing")
    ch_binary = shutil.which("cloud-hypervisor")
    check(ch_binary is not None, "cloud-hypervisor is absent")
    check(pathlib.Path("/dev/kvm").is_char_device(), "/dev/kvm is absent")
    check(shutil.which("ethtool") is not None, "ethtool is absent")
    for path in (run_dir, cgroup, work):
        check(not path.exists(), f"refusing pre-existing exact resource: {path}")
    for name in (tap, scratch, bridge):
        check(not iface_exists(name), f"refusing pre-existing exact link: {name}")
        check(not tun_holders(name)[0], f"refusing pre-existing queue holder for {name}")
    check(run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode != 0,
          f"refusing pre-existing exact nft table: {nft_table}")
    before_ch = ch_pids()
    emit("ENV", f"variant={VARIANT} launcher_flags=0x{LAUNCHER_FLAGS:04x}")
    emit("ENV", f"uname={command_output(['uname', '-srmo'])}")
    emit("ENV", f"systemd_detect_virt={detect_virt}")
    emit("ENV", f"cloud_hypervisor_path={ch_binary}")
    emit("ENV", f"cloud_hypervisor_version={command_output([ch_binary, '--version']).splitlines()[0]}")
    emit("ENV", f"ethtool_version={command_output(['ethtool', '--version'])}")
    emit("ENV", f"kernel={kernel_master} rootfs={rootfs_master}")
    emit("ENV", f"parent_identity={command_output(['id'])}")
    emit("ENV", f"names tap={tap} scratch={scratch} bridge={bridge} nft={nft_table} run_dir={run_dir} cgroup={cgroup} work={work}")
    emit("ENV", f"before_cloud_hypervisor_pids={sorted(before_ch)}")
    passed("PREFLIGHT", "native root x86_64 metal, no pre-existing exact resources")

    if VARIANT == "main":
        single_queue_semantics()

    # ---------------- guest/VMM staging (identical to the prior probe) ----------------
    begin("STAGING")
    work.mkdir(mode=0o710)
    run(["cp", "--reflink=auto", str(rootfs_master), str(work / "rootfs.ext4")])
    loop = command_output(["losetup", "--find", "--show", str(work / "rootfs.ext4")])
    (work / "mnt").mkdir()
    run(["mount", loop, str(work / "mnt")])
    busybox = pathlib.Path("/usr/bin/busybox")
    check(busybox.is_file(), "host static busybox is absent")
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
    passed("STAGING", "rootfs/kernel/run-dir/beacon/cgroup staged")

    # ---------------- STEP 1: network owner creates persistent TAP, leaves it down ----------------
    begin("STEP1")
    emit("STEP1", "hypothesis=the production creator sequence yields a persistent, down, owner-4200 TAP with zero queue holders after its fd closes")
    emit("STEP1", "prediction=tun_flags=0x1802 (TAP|NO_PI|PERSIST, no VNET_HDR) owner=4200 flags lack UP holders=[] master=bridge after attach")
    emit("STEP1", "falsification=TAP absent after creator close, UP, holders non-empty, or persistence/owner differ")
    run(["ip", "link", "add", bridge, "type", "bridge"])
    run(["ip", "addr", "add", "10.77.239.1/24", "dev", bridge])
    run(["ip", "link", "set", bridge, "up"])
    production_create_persistent_tap(tap, UID)
    after_create = tap_snapshot(tap)
    show("STEP1", "after_creator_close_snapshot", after_create)
    run(["ip", "link", "set", tap, "master", bridge])
    step1 = tap_snapshot(tap)
    show("STEP1", "after_bridge_attach_snapshot", step1)
    tap_ifindex = step1["ifindex"] if step1["exists"] else None
    # Spike-local guard (stand-in for "interception live"; NOT production TCX/D9).
    run(["nft", "add", "table", "bridge", nft_table])
    run(["nft", "add", "chain", "bridge", nft_table, "input", "{ type filter hook input priority -300; policy drop; }"])
    run(["nft", "add", "rule", "bridge", nft_table, "input", "iifname", tap, "ether", "type", "arp", "counter", "accept"])
    run(["nft", "add", "rule", "bridge", nft_table, "input", "iifname", tap, "ip", "protocol", "icmp", "counter", "accept"])
    run(["nft", "add", "chain", "bridge", nft_table, "forward", "{ type filter hook forward priority -300; policy drop; }"])
    check(after_create["exists"], "persistent TAP absent after creator fd close")
    check(step1["tun_flags"] == "0x1802", f"unexpected tun_flags {step1['tun_flags']}")
    check(step1["owner"] == str(UID), f"unexpected owner {step1['owner']}")
    check("UP" not in step1["flags"], "TAP administratively UP after create")
    check(step1["master"] == bridge, "TAP not attached to spike bridge")
    check(step1["holders"] == [], f"queue holders after creator close: {step1['holders']}")
    passed("STEP1", f"tap={tap} ifindex={tap_ifindex} tun_flags={step1['tun_flags']} owner={step1['owner']} "
                    f"flags={step1['flags']} master={step1['master']} holders=[]")

    # Capture on the exact ifindex, all EtherTypes, bound before any launch.
    packet_capture = new_capture()
    emit("CAPTURE", f"capture_started_before_launch=AF_PACKET ifindex={socket.if_nametoindex(tap)} all_ethertypes=true")

    # ---------------- STEP 2 / FAILURE ARM ----------------
    begin("STEP2")
    emit("STEP2", f"hypothesis=a separate launcher open+TUNSETIFF(0x{LAUNCHER_FLAGS:04x}) attaches to the existing persistent TAP without the creator fd")
    emit("STEP2", "prediction=attach ok; fdinfo iff names the TAP; TUNSETIFF updates TUN_FEATURES (tun_flags gains 0x4000 iff requested); TAP stays down")
    emit("STEP2", "falsification=attach errors, tun_flags does not track the request, or the TAP changes admin state")
    tap_fd = launcher_attach(tap, LAUNCHER_FLAGS)
    tap_stat = os.fstat(tap_fd)
    self_fdinfo = pathlib.Path(f"/proc/self/fdinfo/{tap_fd}").read_text().strip().replace("\n", "; ")
    emit("STEP2", f"launcher_fd={tap_fd} inode={tap_stat.st_ino} device={tap_stat.st_dev} TUNGETIFF={tun_getiff(tap_fd)}")
    emit("STEP2", f"launcher_fdinfo={self_fdinfo}")
    after_attach = tap_snapshot(tap)
    show("STEP2", "after_launcher_attach_snapshot", after_attach)
    emit("STEP2", f"diff_vs_step1={json.dumps(state_diff(step1, after_attach), sort_keys=True)}")
    expected_tun_flags = "0x5802" if VARIANT == "main" else "0x1802"
    check(after_attach["tun_flags"] == expected_tun_flags, f"launcher attach tun_flags {after_attach['tun_flags']} != {expected_tun_flags}")
    check([h[:2] for h in after_attach["holders"]] == [[os.getpid(), tap_fd]], f"holders {after_attach['holders']}")
    check("UP" not in after_attach["flags"], "TAP UP after launcher attach")

    fail_cmd = None
    if VARIANT == "main":
        begin("FAILURE_ARM")
        emit("FAILURE_ARM", "hypothesis=a failed CH launch leaves the launcher's queue fd and the persistent down TAP intact; after the launcher closes the fd the TAP persists and a fresh attach succeeds")
        emit("FAILURE_ARM", "prediction=CH exits non-zero; launcher fd inode/iff unchanged; TAP down, tun_flags unchanged; after close holders=[] TAP persists; reattach ok")
        emit("FAILURE_ARM", "falsification=TAP raised/removed, flags changed, or reattach refused")
        fd_flags_before = fcntl.fcntl(tap_fd, fcntl.F_GETFD)
        fcntl.fcntl(tap_fd, fcntl.F_SETFD, fd_flags_before & ~fcntl.FD_CLOEXEC)
        emit("FAILURE_ARM", f"fd_flags_before=0x{fd_flags_before:x} fd_flags_after=0x{fcntl.fcntl(tap_fd, fcntl.F_GETFD):x} "
             f"parent_non_cloexec_fds={open_fds_without_cloexec()}")
        check([fd for fd in open_fds_without_cloexec() if fd > 2] == [tap_fd], "unexpected inherited parent fds")
        fail_cmd = confined_command(ch_binary, tap_fd, run_dir / "deliberately-missing-kernel", work / "rootfs.ext4",
                                    run_dir / "fail-console.log", run_dir / "fail-api", run_dir / "fail-vsock")
        emit("FAILURE_ARM", f"failure_cli={shlex.join(fail_cmd)}")
        fail_started = time.perf_counter()
        with open(run_dir / "fail.stderr", "wb") as fail_stderr:
            fail_proc = subprocess.Popen(fail_cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                         stderr=fail_stderr, close_fds=True, pass_fds=(tap_fd,))
            fail_rc = fail_proc.wait(timeout=15)
        emit("FAILURE_ARM", f"failure_exit={fail_rc} failure_elapsed_seconds={time.perf_counter() - fail_started:.6f}")
        emit("FAILURE_ARM", "failure_stderr_tail=" + " | ".join((run_dir / "fail.stderr").read_text(errors="replace").splitlines()[-8:]))
        after_fail = tap_snapshot(tap)
        show("FAILURE_ARM", "after_failed_launch_snapshot", after_fail)
        emit("FAILURE_ARM", f"diff_vs_launcher_attach={json.dumps(state_diff(after_attach, after_fail), sort_keys=True)} "
             f"launcher_fd_inode={os.fstat(tap_fd).st_ino} launcher_TUNGETIFF={tun_getiff(tap_fd)}")
        check(fail_rc != 0, "deliberate CH start failure unexpectedly succeeded")
        check(os.fstat(tap_fd).st_ino == tap_stat.st_ino, "launcher fd identity changed across failed launch")
        check(state_diff(after_attach, after_fail) == {}, "failed launch changed TAP state")
        check([h[:2] for h in after_fail["holders"]] == [[os.getpid(), tap_fd]], f"holders after failure {after_fail['holders']}")
        tun_close(tap_fd)
        after_fail_close = tap_snapshot(tap)
        show("FAILURE_ARM", "after_launcher_close_snapshot", after_fail_close)
        check(after_fail_close["exists"] and after_fail_close["holders"] == [], "TAP absent or still held after failed-launch fd close")
        check(after_fail_close["tun_flags"] == expected_tun_flags and "UP" not in after_fail_close["flags"],
              "persistent TAP flags/admin changed after failed-launch fd close")
        tap_fd = launcher_attach(tap, LAUNCHER_FLAGS)
        reattach = tap_snapshot(tap)
        emit("FAILURE_ARM", f"subsequent_launcher_attach=ok fd={tap_fd} inode={os.fstat(tap_fd).st_ino} TUNGETIFF={tun_getiff(tap_fd)} "
             f"holders={reattach['holders']} tun_flags={reattach['tun_flags']}")
        check(tap_fd == FD_MIN, f"reattach fd numbering not deterministic: {tap_fd}")
        passed("FAILURE_ARM", f"exit={fail_rc} tap_persisted_down=true tun_flags={after_fail_close['tun_flags']} "
                              f"holders_after_close=[] reattach=ok fd={tap_fd}")
    tap_stat = os.fstat(tap_fd)
    passed("STEP2", f"launcher_fd={tap_fd} TUNGETIFF={tun_getiff(tap_fd)} tun_flags={tap_snapshot(tap)['tun_flags']}")

    # ---------------- STEP 3: CH reaches READY while TAP down ----------------
    begin("STEP3")
    emit("STEP3", "hypothesis=CH v53 consumes the inherited queue fd without raising the persistent TAP; guest READY with zero frames")
    emit("STEP3", "prediction=READY pid=1 port=1234; TAP flags lack UP; capture=0; all six counters=0; TAP state identical to post-attach")
    emit("STEP3", "falsification=TAP UP at READY, any frame/counter (incl. rx_dropped) non-zero, or CH altered tun_flags/owner/offload")
    fd_flags_before = fcntl.fcntl(tap_fd, fcntl.F_GETFD)
    fcntl.fcntl(tap_fd, fcntl.F_SETFD, fd_flags_before & ~fcntl.FD_CLOEXEC)
    emit("STEP3", f"fd_flags_before=0x{fd_flags_before:x} fd_flags_after=0x{fcntl.fcntl(tap_fd, fcntl.F_GETFD):x} "
         f"parent_non_cloexec_fds={open_fds_without_cloexec()}")
    check([fd for fd in open_fds_without_cloexec() if fd > 2] == [tap_fd], "unexpected inherited parent fds")
    pre_launch = tap_snapshot(tap)
    main_cmd = confined_command(ch_binary, tap_fd, run_dir / "kernel", work / "rootfs.ext4",
                                run_dir / "console.log", run_dir / "api", run_dir / "vsock")
    emit("STEP3", f"main_cli={shlex.join(main_cmd)}")
    main_started = time.perf_counter()
    stderr_handle = open(run_dir / "main.stderr", "wb")
    main_proc = subprocess.Popen(main_cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=stderr_handle, close_fds=True, pass_fds=(tap_fd,))
    (cgroup / "cgroup.procs").write_text(str(main_proc.pid))
    wait_for_exec(main_proc.pid, ch_binary)
    proc_status = pathlib.Path(f"/proc/{main_proc.pid}/status").read_text()
    status_lines = {
        line.split(":", 1)[0]: line.split(":", 1)[1].strip()
        for line in proc_status.splitlines()
        if line.startswith(("Uid:", "Gid:", "Groups:", "NoNewPrivs:", "Seccomp:", "Seccomp_filters:"))
    }
    emit("STEP3", f"exec_elapsed_seconds={time.perf_counter() - main_started:.6f} child_pid={main_proc.pid}")
    emit("STEP3", "child_cmdline=" + pathlib.Path(f"/proc/{main_proc.pid}/cmdline").read_bytes().replace(b"\0", b" ").decode())
    emit("STEP3", f"child_status={json.dumps(status_lines, sort_keys=True)}")
    emit("STEP3", f"cgroup={cgroup} cgroup_procs={(cgroup / 'cgroup.procs').read_text().strip()}")
    check(status_lines.get("Uid", "").split()[:2] == [str(UID), str(UID)], "CH uid mismatch")
    check(status_lines.get("Gid", "").split()[:2] == [str(GID), str(GID)], "CH gid mismatch")
    check(status_lines.get("NoNewPrivs") == "1", "CH leader lacks no-new-privs")

    listener.settimeout(40)
    conn, _ = listener.accept()
    ready_line = read_line(conn)
    ready_elapsed = time.perf_counter() - main_started
    at_ready = tap_snapshot(tap)
    ch_fds_at_ready = process_tun_fds(main_proc.pid)
    security_at_ready = thread_security(main_proc.pid)
    pre_records = drain_capture(packet_capture)
    emit("STEP3", f"ready_line={ready_line} ready_elapsed_seconds={ready_elapsed:.6f}")
    show("STEP3", "at_ready_snapshot", at_ready)
    emit("STEP3", f"diff_vs_pre_launch={json.dumps(state_diff(pre_launch, at_ready), sort_keys=True)} "
         f"offload_digest_pre={pre_launch['offload_digest']} offload_digest_ready={at_ready['offload_digest']}")
    emit("STEP3", f"ch_tun_fds_at_ready={json.dumps(ch_fds_at_ready, sort_keys=True)}")
    emit("STEP3", f"thread_security_at_ready={json.dumps(security_at_ready, sort_keys=True)}")
    emit("STEP3", f"pre_activation_capture_packets={len(pre_records)} bytes={sum(len(r[2]) for r in pre_records)}")
    for record in pre_records[:20]:
        emit("STEP3", "pre_activation_frame " + frame_summary(record))
    worker_security = [item for item in security_at_ready if item["tid"] != main_proc.pid]
    check(ready_line.startswith("READY pid=1 port=1234"), f"unexpected guest beacon: {ready_line}")
    check("UP" not in at_ready["flags"], "Cloud Hypervisor raised the TAP before READY")
    check(all(value == 0 for value in at_ready["stats"].values()), f"non-zero counters at READY: {at_ready['stats']}")
    check(not pre_records, "frames appeared on exact TAP before activation")
    check(state_diff(pre_launch, at_ready) == {}, "CH altered TAP state before READY")
    check(at_ready["offload_full"] == pre_launch["offload_full"], "CH altered TAP offload features before READY")
    check(any(item["fd"] == tap_fd and item["iff"] == tap and item["inode"] == tap_stat.st_ino for item in ch_fds_at_ready),
          "CH does not hold the inherited fd for the exact TAP")
    check(worker_security and all(item.get("NoNewPrivs") == "1" and item.get("Seccomp") == "2" for item in worker_security),
          "CH worker-thread seccomp confinement mismatch")
    packet_capture.close()
    packet_capture = None
    passed("STEP3", f"{ready_line!r} at {ready_elapsed:.3f}s flags={at_ready['flags']} capture=0 stats=all-zero "
                    f"tap_state_unchanged=true ch_tun_fds={[item['fd'] for item in ch_fds_at_ready]}")

    # ---------------- STEP 4: launcher closes its copy; CH retains the queue ----------------
    begin("STEP4")
    emit("STEP4", "hypothesis=CH's inherited fd (and its dup) are the only remaining queue holders once the launcher closes")
    emit("STEP4", "prediction=holders == CH pid only; CH fdinfo iff still names the TAP; TAP exists, down, state unchanged")
    emit("STEP4", "falsification=TAP disappears/changes, CH loses the iff association, or another holder remains")
    inherited_fd = tap_fd
    tun_close(tap_fd)
    tap_fd = -1
    after_parent_close = tap_snapshot(tap)
    ch_fds_after_close = process_tun_fds(main_proc.pid)
    show("STEP4", "after_launcher_close_snapshot", after_parent_close)
    emit("STEP4", f"ch_tun_fds_after_launcher_close={json.dumps(ch_fds_after_close, sort_keys=True)}")
    holder_pids = sorted({holder[0] for holder in after_parent_close["holders"]})
    check(after_parent_close["exists"], "TAP disappeared after launcher close")
    check(holder_pids == [main_proc.pid], f"unexpected queue holders {after_parent_close['holders']}")
    check(any(item["fd"] == inherited_fd and item["iff"] == tap for item in ch_fds_after_close), "CH lost the inherited fd")
    check(state_diff(at_ready, after_parent_close) == {}, "TAP state changed on launcher close")
    passed("STEP4", f"holders={after_parent_close['holders']} tap_exists=true flags={after_parent_close['flags']}")

    # ---------------- STEP 5: network owner raises the TAP after the guard is live ----------------
    begin("STEP5")
    emit("STEP5", "hypothesis=after guard read-back, owner-side offload disable + UP yields UP/LOWER_UP and bidirectional traffic entirely after the barrier")
    emit("STEP5", "prediction=nft read-back exact; UP,LOWER_UP master=bridge forwarding; ARP+ICMP both directions; every timestamp > barrier")
    emit("STEP5", "falsification=read-back mismatch, no bidirectional progress, or any capture timestamp <= barrier")
    nft_readback = command_output(["nft", "-a", "list", "table", "bridge", nft_table])
    emit("STEP5", "protection_readback_begin")
    for line in nft_readback.splitlines():
        emit("STEP5", "  " + line)
    emit("STEP5", "protection_readback_end")
    check(f'iifname "{tap}" ether type arp' in nft_readback and "policy drop" in nft_readback, "guard read-back mismatch")
    ethtool = run(["ethtool", "-K", tap, "tx-checksum-ip-generic", "off"], check_rc=False)
    emit("STEP5", f"owner_disable_tx_offload rc={ethtool.returncode} stdout={ethtool.stdout.strip()!r} stderr={ethtool.stderr.strip()!r}")
    pre_up = tap_snapshot(tap)
    show("STEP5", "pre_up_snapshot", pre_up)
    check(all(value == 0 for value in pre_up["stats"].values()), f"non-zero counters immediately before activation: {pre_up['stats']}")
    activation_wall_ns = time.time_ns()
    activation_started = time.perf_counter()
    run(["ip", "link", "set", tap, "up"])
    activation_elapsed = time.perf_counter() - activation_started
    packet_capture = new_capture()
    active = tap_snapshot(tap)
    bridge_readback = command_output(["bridge", "-details", "link", "show", "dev", tap])
    emit("STEP5", f"activation_wall_ns={activation_wall_ns} activation_elapsed_seconds={activation_elapsed:.6f}")
    show("STEP5", "active_snapshot", active)
    emit("STEP5", f"bridge_readback={bridge_readback}")
    check("UP" in active["flags"] and active["master"] == bridge, "activation did not read back UP/master")

    exec_line = b'EXEC ["/sbin/busybox","sh","-c","ping -c 1 -W 2 10.77.239.1; sleep 60"]\n'
    conn.sendall(exec_line)
    traffic_started = time.perf_counter()
    post_records = []
    progress = False
    saw_reply_to_guest = False
    while time.perf_counter() - traffic_started < 6:
        post_records.extend(drain_capture(packet_capture))
        live = stats(tap)
        progress = live["rx_packets"] > 0 and live["tx_packets"] > 0
        saw_reply_to_guest = any(
            classify(frame) == "icmp-echo-reply" and frame[0:6].hex(":") == guest_mac for _, _, frame in post_records
        )
        if progress and saw_reply_to_guest:
            break
        time.sleep(0.02)
    traffic_elapsed = time.perf_counter() - traffic_started
    time.sleep(0.1)
    post_records.extend(drain_capture(packet_capture))
    live = stats(tap)
    pre_barrier = [r for r in post_records if r[0] is None or r[0] <= activation_wall_ns]
    guest_originated = [r for r in post_records if r[2][6:12].hex(":") == guest_mac]
    well_formed_guest = [r for r in guest_originated if classify(r[2]) in ("arp-request", "arp-reply", "icmp-echo-request")]
    to_guest = [r for r in post_records if r[2][0:6].hex(":") == guest_mac or r[2][0:6] == b"\xff" * 6 and r[2][6:12].hex(":") != guest_mac]
    emit("STEP5", f"traffic_elapsed_seconds={traffic_elapsed:.6f} tap_stats={json.dumps(live, sort_keys=True)}")
    emit("STEP5", f"post_activation_capture_packets={len(post_records)} bytes={sum(len(r[2]) for r in post_records)} "
         f"records_at_or_before_barrier={len(pre_barrier)} guest_originated={len(guest_originated)} "
         f"well_formed_guest={len(well_formed_guest)} to_guest={len(to_guest)} icmp_reply_to_guest={saw_reply_to_guest}")
    if post_records:
        first_ts = min(r[0] for r in post_records if r[0] is not None)
        emit("STEP5", f"first_frame_after_barrier_ms={(first_ts - activation_wall_ns) / 1e6:.3f}")
    for record in post_records[:24]:
        emit("STEP5", "post_activation_frame " + frame_summary(record))
    check(not pre_barrier, "capture timestamp absent or not strictly after activation")
    if VARIANT == "main":
        check(progress and saw_reply_to_guest, f"no bidirectional ARP/ICMP after activation: {live}")
        check(well_formed_guest, "no well-formed guest-originated frame")
        passed("STEP5", f"guard_readback=exact UP={active['flags']} master={active['master']} rx={live['rx_packets']} "
                        f"tx={live['tx_packets']} frames={len(post_records)} all_after_barrier=true icmp_reply_to_guest=true")
    else:
        step_results["STEP5"] = (f"OBSERVED bidirectional_icmp={progress and saw_reply_to_guest} "
                                 f"well_formed_guest_frames={len(well_formed_guest)} guest_frames={len(guest_originated)} stats={live}")
        emit("STEP5", "RESULT=OBSERVED " + step_results["STEP5"])

    # ---------------- STEP 6: CH exit; persistent TAP remains, nobody holds it ----------------
    begin("STEP6")
    emit("STEP6", "hypothesis=CH exit closes its queue; the persistent TAP remains with no holder anywhere")
    emit("STEP6", "prediction=TAP exists, admin UP retained, NO-CARRIER/operstate DOWN, master=bridge, tun_flags persist bit set, owner=4200, holders=[]")
    emit("STEP6", "falsification=TAP disappears, a holder remains, or persistence/owner changed")
    main_proc.terminate()
    cancel_started = time.perf_counter()
    try:
        main_rc = main_proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        (cgroup / "cgroup.kill").write_text("1")
        main_rc = main_proc.wait(timeout=5)
    cancel_elapsed = time.perf_counter() - cancel_started
    stderr_handle.close()
    stderr_handle = None
    after_exit = tap_snapshot(tap)
    time.sleep(0.5)
    after_exit_settled = tap_snapshot(tap)
    emit("STEP6", f"main_exit={main_rc} cancellation_elapsed_seconds={cancel_elapsed:.6f} ch_pid_alive={pathlib.Path(f'/proc/{main_proc.pid}').exists()}")
    emit("STEP6", "main_stderr_tail=" + " | ".join((run_dir / "main.stderr").read_text(errors="replace").splitlines()[-12:]))
    show("STEP6", "after_ch_exit_snapshot", after_exit)
    show("STEP6", "after_ch_exit_settled_snapshot", after_exit_settled)
    emit("STEP6", f"diff_active_vs_exit={json.dumps(state_diff(active, after_exit_settled), sort_keys=True)}")
    emit("STEP6", f"tap_ip_detail={command_output(['ip', '-d', 'link', 'show', 'dev', tap]).replace(chr(10), ' | ')}")
    check(after_exit_settled["exists"], "persistent TAP disappeared on CH exit")
    check(after_exit_settled["holders"] == [], f"queue holders remain after CH exit: {after_exit_settled['holders']}")
    check(int(after_exit_settled["tun_flags"], 16) & IFF_PERSIST, "persistence bit lost")
    check(after_exit_settled["owner"] == str(UID), "owner changed")
    passed("STEP6", f"exists=true flags={after_exit_settled['flags']} operstate={after_exit_settled['operstate']} "
                    f"carrier={after_exit_settled['carrier']} master={after_exit_settled['master']} "
                    f"tun_flags={after_exit_settled['tun_flags']} owner={after_exit_settled['owner']} "
                    f"holders=[] (fdinfo scanned={after_exit_settled['fdinfo_scanned']})")

    # ---------------- RESTART OBSERVATION (extra arm; not one of steps 1-7) ----------------
    if VARIANT == "main":
        begin("RESTART_OBS")
        emit("RESTART_OBS", "hypothesis=tun_set_iff turns carrier on at attach, so a relaunch attach onto the persistent TAP left admin-UP "
             "after VMM exit makes it LOWER_UP before any VMM runs; a down-first reattach stays down")
        emit("RESTART_OBS", "prediction=attach while admin-UP -> LOWER_UP carrier=1 and host frames queue into the new fd; "
             "attach after set-down -> no UP, zero queued frames, counters unchanged")
        emit("RESTART_OBS", "falsification=attach onto admin-UP TAP stays NO-CARRIER with nothing queued")

        def read_queue(fd):
            records = []
            while True:
                try:
                    records.append(os.read(fd, 65536))
                except BlockingIOError:
                    return records

        def describe_queued(record):
            return (f"len={len(record)} head={record[:40].hex()} "
                    f"as_vnet12={classify(record[12:])} as_vnet10={classify(record[10:])}")

        up_fd = launcher_attach(tap, LAUNCHER_FLAGS)
        time.sleep(0.5)
        attached_up = tap_snapshot(tap)
        queued_up = read_queue(up_fd)
        show("RESTART_OBS", "reattach_while_admin_up_snapshot", attached_up)
        emit("RESTART_OBS", f"queued_frames_while_admin_up={len(queued_up)}")
        for record in queued_up[:10]:
            emit("RESTART_OBS", "queued_frame " + describe_queued(record))
        tun_close(up_fd)
        closed_up = tap_snapshot(tap)
        emit("RESTART_OBS", f"after_close flags={closed_up['flags']} carrier={closed_up['carrier']} holders={closed_up['holders']}")
        run(["ip", "link", "set", tap, "down"])
        before_down_attach = stats(tap)
        down_fd = launcher_attach(tap, LAUNCHER_FLAGS)
        time.sleep(0.5)
        attached_down = tap_snapshot(tap)
        queued_down = read_queue(down_fd)
        show("RESTART_OBS", "reattach_after_set_down_snapshot", attached_down)
        emit("RESTART_OBS", f"queued_frames_after_set_down={len(queued_down)} stats_before_attach={json.dumps(before_down_attach, sort_keys=True)}")
        tun_close(down_fd)
        after_obs = tap_snapshot(tap)
        emit("RESTART_OBS", f"after_obs flags={after_obs['flags']} holders={after_obs['holders']} tun_flags={after_obs['tun_flags']}")
        step_results["RESTART_OBS"] = (
            f"OBSERVED admin_up_reattach flags={attached_up['flags']} carrier={attached_up['carrier']} queued={len(queued_up)}; "
            f"down_first_reattach flags={attached_down['flags']} queued={len(queued_down)} "
            f"stats_delta={ {k: attached_down['stats'][k] - before_down_attach[k] for k in STAT_NAMES} }"
        )
        emit("RESTART_OBS", "RESULT=" + step_results["RESTART_OBS"])
        check(after_obs["holders"] == [], "restart observation left a queue holder")

    # ---------------- STEP 7: production-equivalent teardown ----------------
    begin("STEP7")
    emit("STEP7", "hypothesis=set-down + RTM_DELLINK removes the persistent TAP; the spike's remaining resources remove to an exact empty complement")
    emit("STEP7", "prediction=down read-back; delete rc=0; TAP absent (ip link show ENODEV); holders=[]; no bridge/nft/cgroup/run-dir/staging/CH residue")
    emit("STEP7", "falsification=TAP survives delete or any residue remains")
    run(["ip", "link", "set", tap, "down"])
    down = tap_snapshot(tap)
    emit("STEP7", f"after_set_down flags={down['flags']} operstate={down['operstate']} holders={down['holders']}")
    check("UP" not in down["flags"], "set down did not read back")
    deleted = run(["ip", "link", "delete", "dev", tap], check_rc=False)
    show_after = run(["ip", "link", "show", "dev", tap], check_rc=False)
    holders_after_delete, scanned = tun_holders(tap)
    emit("STEP7", f"rtm_dellink rc={deleted.returncode} stderr={deleted.stderr.strip()!r}")
    emit("STEP7", f"ip_link_show_after rc={show_after.returncode} stderr={show_after.stderr.strip()!r} sysfs_exists={iface_exists(tap)}")
    emit("STEP7", f"holders_after_delete={holders_after_delete} fdinfo_scanned={scanned}")
    check(deleted.returncode == 0 and not iface_exists(tap) and show_after.returncode != 0, "TAP survived production-equivalent delete")
    check(holders_after_delete == [], "queue holder remains after delete")
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
        "new_cloud_hypervisor_pids": sorted(ch_pids() - before_ch),
        "tap_exists": iface_exists(tap),
        "scratch_exists": iface_exists(scratch),
        "tap_queue_holders": tun_holders(tap)[0],
        "scratch_queue_holders": tun_holders(scratch)[0],
        "bridge_exists": iface_exists(bridge),
        "nft_table_exists": run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode == 0,
        "run_dir_exists": run_dir.exists(),
        "cgroup_exists": cgroup.exists(),
        "staging_exists": work.exists(),
        "probe_open_tun_fds": sorted(open_fds),
    }
    emit("STEP7", f"final_readback={json.dumps(final, sort_keys=True)}")
    check(not any(bool(value) for value in final.values()), "residue after teardown")
    passed("STEP7", "tap deleted, exact empty complement")

    emit("SUMMARY", "QUEUE_IMPLICATIONS queue_fds=1 virtio_num_queues=2 IFF_MULTI_QUEUE=false")
    for step, result in step_results.items():
        emit("SUMMARY", f"{step}: {result}")
    emit("SUMMARY", f"TOTAL_ELAPSED_SECONDS={time.perf_counter() - started:.6f}")
    if VARIANT == "main":
        print("VERDICT=WORKS", flush=True)
    else:
        print("VARIANT_COMPLETE=no-vnet-hdr", flush=True)
except BaseException as error:
    emit("FAILURE", f"step={current_step} error={type(error).__name__}: {error}")
    for step, result in step_results.items():
        emit("SUMMARY", f"{step}: {result}")
    print("VERDICT=DOESNT_WORK_OR_INCOMPLETE", flush=True)
    raise
finally:
    residue_found_by_fallback = []
    if conn is not None:
        conn.close()
    if listener is not None:
        listener.close()
    if packet_capture is not None:
        packet_capture.close()
    for proc in (fail_proc, main_proc):
        if proc is not None and proc.poll() is None:
            residue_found_by_fallback.append(f"live_ch_pid={proc.pid}")
            if cgroup.exists() and (cgroup / "cgroup.kill").exists():
                (cgroup / "cgroup.kill").write_text("1")
            else:
                proc.kill()
            proc.wait()
    if stderr_handle is not None:
        stderr_handle.close()
    for fd in sorted(open_fds):
        residue_found_by_fallback.append(f"open_tun_fd={fd}")
        os.close(fd)
    open_fds.clear()
    if loop:
        residue_found_by_fallback.append(f"loop={loop}")
        if run(["mountpoint", "-q", str(work / "mnt")], check_rc=False).returncode == 0:
            run(["umount", str(work / "mnt")], check_rc=False)
        run(["losetup", "-d", loop], check_rc=False)
    if run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode == 0:
        residue_found_by_fallback.append(f"nft={nft_table}")
        run(["nft", "delete", "table", "bridge", nft_table], check_rc=False)
    for name in (tap, scratch, bridge):
        if iface_exists(name):
            residue_found_by_fallback.append(f"link={name}")
            run(["ip", "link", "delete", "dev", name], check_rc=False)
    if cgroup.exists():
        residue_found_by_fallback.append(f"cgroup={cgroup}")
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
            residue_found_by_fallback.append(f"dir={path}")
            shutil.rmtree(path)
    cleanup_checks = {
        "new_cloud_hypervisor_pids": sorted(ch_pids() - before_ch),
        "tap_exists": iface_exists(tap),
        "scratch_exists": iface_exists(scratch),
        "bridge_exists": iface_exists(bridge),
        "tap_queue_holders": tun_holders(tap)[0],
        "nft_table_exists": run(["nft", "list", "table", "bridge", nft_table], check_rc=False).returncode == 0,
        "run_dir_exists": run_dir.exists(),
        "cgroup_exists": cgroup.exists(),
        "staging_exists": work.exists(),
    }
    print(f"CLEANUP residue_found_by_fallback={residue_found_by_fallback}", flush=True)
    print("CLEANUP " + json.dumps(cleanup_checks, sort_keys=True), flush=True)
    print(f"cleanup_complete={not any(bool(value) for value in cleanup_checks.values())}", flush=True)
